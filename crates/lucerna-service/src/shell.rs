//! A conexão ao `shell.sock`: itens da barra, não perturbe e pedidos avulsos.
//!
//! O app diz o que quer agora ([`Shell::bar_set`], [`Shell::dnd_request`]...) e
//! o [`Shell`] guarda isso como estado desejado. A tarefa de conexão
//! ([`Shell::spawn`]) conecta, aplica o desejado inteiro e depois manda só a
//! diferença a cada mudança. Se o shell não existe, cai ou trava, ela reconecta
//! com espera crescente ([`backoff`]) e reaplica tudo: mudanças feitas enquanto o
//! shell estava fora chegam só como o estado atual, sem fila de chamadas velhas.
//!
//! Os pedidos saem um por vez, cada um esperando a resposta (com prazo) antes do
//! próximo, para chegarem ao shell na ordem. Sem shell, nada dá erro para o app.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use lucerna_ipc::{Client, ClientError};
use lucerna_protocol::methods::{
    BAR_CLEAR, BAR_SET, DND_RELEASE, DND_REQUEST, PANEL_OPEN, TUI_OPEN,
};
use lucerna_protocol::{BarClearParams, BarItem, DndRequestParams, PanelOpenParams, TuiOpenParams};
use serde_json::Value;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

use crate::idle::{WorkGuard, WorkTracker};

/// Id do app do lado do shell no aperto de mão.
const SHELL_APP: &str = "shell";

/// Prazo para conectar (com o aperto de mão) e para cada resposta do shell.
/// Passou disso, a conexão conta como caída.
const SHELL_TIMEOUT: Duration = Duration::from_secs(2);

/// Teto da espera entre tentativas de conexão.
const MAX_BACKOFF: Duration = Duration::from_secs(5);

/// Espera antes da tentativa `attempt` (a partir de `0`): `min(100 ms × 2^attempt, 5 s)`.
pub fn backoff(attempt: u32) -> Duration {
    let base = Duration::from_millis(100);
    2u32.checked_pow(attempt)
        .and_then(|f| base.checked_mul(f))
        .map_or(MAX_BACKOFF, |d| d.min(MAX_BACKOFF))
}

/// O que o app quer que o shell mostre agora.
#[derive(Clone, Debug, Default, PartialEq)]
struct Desired {
    bar: BTreeMap<String, BarItem>,
    /// Motivo do não perturbe pedido, se houver.
    dnd: Option<String>,
}

/// Pedido avulso (só vai com o shell conectado).
type Action = (&'static str, Value);

struct Inner {
    app: String,
    desired: watch::Sender<Desired>,
    work: WorkTracker,
    /// Seguro enquanto houver item de barra desejado.
    guard: Mutex<Option<WorkGuard>>,
    connected: watch::Sender<bool>,
    /// Fila de pedidos avulsos da tarefa de conexão atual.
    actions: Mutex<Option<mpsc::UnboundedSender<Action>>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Pedidos do app ao shell. Clonar dá outra alça para o mesmo estado.
///
/// Nenhum método é `async` nem devolve erro: sem shell, os pedidos viram nada
/// (os itens da barra e o não perturbe ficam guardados para quando ele voltar).
#[derive(Clone)]
pub struct Shell {
    inner: Arc<Inner>,
}

impl Shell {
    /// Um shell sem conexão, para o app `app`; os itens da barra contam em `work`.
    pub fn new(app: &str, work: WorkTracker) -> Shell {
        Shell {
            inner: Arc::new(Inner {
                app: app.to_string(),
                desired: watch::Sender::new(Desired::default()),
                work,
                guard: Mutex::new(None),
                connected: watch::Sender::new(false),
                actions: Mutex::new(None),
            }),
        }
    }

    /// Começa a tarefa que mantém a conexão ao shell em `socket`.
    ///
    /// A tarefa para quando o [`ShellTask`] devolvido cai.
    pub fn spawn(&self, socket: PathBuf) -> ShellTask {
        let (tx, rx) = mpsc::unbounded_channel();
        *lock(&self.inner.actions) = Some(tx);
        let inner = self.inner.clone();
        ShellTask {
            handle: tokio::spawn(connection_loop(inner, socket, rx)),
        }
    }

    /// Põe ou atualiza o item `item.id` na barra.
    pub fn bar_set(&self, item: BarItem) {
        self.update_bar(|bar| {
            bar.insert(item.id.clone(), item);
        });
    }

    /// Tira o item `id` da barra.
    pub fn bar_clear(&self, id: &str) {
        self.update_bar(|bar| {
            bar.remove(id);
        });
    }

    /// Pede o não perturbe, com o motivo `reason`.
    pub fn dnd_request(&self, reason: &str) {
        let reason = reason.to_string();
        self.inner.desired.send_if_modified(|d| {
            let new = Some(reason);
            let changed = d.dnd != new;
            d.dnd = new;
            changed
        });
    }

    /// Devolve o não perturbe.
    pub fn dnd_release(&self) {
        self.inner
            .desired
            .send_if_modified(|d| d.dnd.take().is_some());
    }

    /// Pede ao shell para abrir a TUI do app com `args`. Sem shell, some.
    pub fn tui_open(&self, args: Vec<String>) {
        self.action(TUI_OPEN, to_value(&TuiOpenParams { args }));
    }

    /// Pede ao shell para abrir o painel do app, na página `page`. Sem shell, some.
    pub fn panel_open(&self, page: Option<String>) {
        self.action(PANEL_OPEN, to_value(&PanelOpenParams { page }));
    }

    /// Se o shell está conectado agora (aperto de mão feito).
    pub fn is_connected(&self) -> bool {
        *self.inner.connected.borrow()
    }

    fn update_bar(&self, f: impl FnOnce(&mut BTreeMap<String, BarItem>)) {
        // O guarda e o desejado mudam juntos, sob a mesma trava.
        let mut guard = lock(&self.inner.guard);
        self.inner.desired.send_if_modified(|d| {
            let before = d.bar.clone();
            f(&mut d.bar);
            d.bar != before
        });
        let empty = self.inner.desired.borrow().bar.is_empty();
        match (empty, guard.is_some()) {
            (false, false) => *guard = Some(self.inner.work.hold()),
            (true, true) => *guard = None,
            _ => {}
        }
    }

    fn action(&self, method: &'static str, params: Value) {
        if !self.is_connected() {
            return;
        }
        if let Some(tx) = lock(&self.inner.actions).as_ref() {
            let _ = tx.send((method, params));
        }
    }
}

/// A tarefa de conexão ao shell; largá-la para a tarefa e fecha a conexão.
#[must_use = "a conexão ao shell para quando o ShellTask cai"]
pub struct ShellTask {
    handle: JoinHandle<()>,
}

impl Drop for ShellTask {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

fn to_value<T: serde::Serialize>(v: &T) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}

async fn connection_loop(
    inner: Arc<Inner>,
    socket: PathBuf,
    mut actions: mpsc::UnboundedReceiver<Action>,
) {
    let mut desired = inner.desired.subscribe();
    let mut attempt: u32 = 0;
    loop {
        if let Ok(client) =
            Client::connect_timeout(&socket, &inner.app, SHELL_APP, SHELL_TIMEOUT).await
        {
            attempt = 0;
            // Pedidos avulsos de antes da conexão não valem.
            while actions.try_recv().is_ok() {}
            inner.connected.send_replace(true);
            session(&client, &mut desired, &mut actions).await;
            inner.connected.send_replace(false);
            while actions.try_recv().is_ok() {}
        }
        tokio::time::sleep(backoff(attempt)).await;
        attempt = attempt.saturating_add(1);
    }
}

/// Mantém o shell igual ao desejado até a conexão cair.
async fn session(
    client: &Client,
    desired: &mut watch::Receiver<Desired>,
    actions: &mut mpsc::UnboundedReceiver<Action>,
) {
    // O shell novo não tem nada nosso: o aplicado começa vazio.
    let mut applied = Desired::default();
    loop {
        let want = desired.borrow_and_update().clone();
        if apply(client, &mut applied, &want).await.is_err() {
            return;
        }
        tokio::select! {
            r = desired.changed() => {
                if r.is_err() {
                    return;
                }
            }
            Some((method, params)) = actions.recv() => {
                if call(client, method, Some(params)).await.is_err() {
                    return;
                }
            }
            _ = client.closed() => return,
        }
    }
}

/// Manda a diferença entre `applied` e `want`, um pedido por vez.
async fn apply(client: &Client, applied: &mut Desired, want: &Desired) -> Result<(), ClientError> {
    for (id, item) in &want.bar {
        if applied.bar.get(id) != Some(item) {
            call(client, BAR_SET, Some(to_value(item))).await?;
            applied.bar.insert(id.clone(), item.clone());
        }
    }
    let gone: Vec<String> = applied
        .bar
        .keys()
        .filter(|id| !want.bar.contains_key(*id))
        .cloned()
        .collect();
    for id in gone {
        call(
            client,
            BAR_CLEAR,
            Some(to_value(&BarClearParams { id: id.clone() })),
        )
        .await?;
        applied.bar.remove(&id);
    }
    if applied.dnd != want.dnd {
        match &want.dnd {
            Some(reason) => {
                let p = DndRequestParams {
                    reason: reason.clone(),
                };
                call(client, DND_REQUEST, Some(to_value(&p))).await?;
            }
            None => call(client, DND_RELEASE, None).await?,
        }
        applied.dnd = want.dnd.clone();
    }
    Ok(())
}

/// Um pedido ao shell, com prazo. Erro de conexão (ou prazo vencido) volta como
/// `Err` e derruba a sessão; um erro do próprio shell (`Rpc`, resposta estranha)
/// conta como aplicado, para não repetir o mesmo pedido sem fim.
async fn call(client: &Client, method: &str, params: Option<Value>) -> Result<(), ClientError> {
    match client.call_timeout(method, params, SHELL_TIMEOUT).await {
        Ok(_) | Err(ClientError::Rpc(_) | ClientError::Decode(_)) => Ok(()),
        Err(e) => Err(e),
    }
}
