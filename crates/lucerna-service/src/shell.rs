//! A conexão ao `shell.sock`: itens da barra, não perturbe e pedidos avulsos.
//!
//! O app diz o que quer agora ([`Shell::bar_set`], [`Shell::dnd_request`]...) e
//! o [`Shell`] guarda isso como estado desejado. A tarefa de conexão
//! ([`Shell::spawn`]) conecta, aplica o desejado inteiro e depois manda só a
//! diferença a cada mudança. Se o shell não existe, cai ou trava, ela reconecta
//! com espera crescente (de 100 ms a 5 s) e reaplica tudo: mudanças feitas enquanto o
//! shell estava fora chegam só como o estado atual, sem fila de chamadas velhas.
//!
//! Os pedidos saem um por vez, cada um esperando a resposta (com prazo) antes do
//! próximo, para chegarem ao shell na ordem. Sem shell, nada dá erro para o app.
//!
//! Um pedido recusado pelo shell (erro na resposta) vai para o stderr e não é
//! repetido até o app mudar aquele item ou o shell reconectar.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use lucerna_ipc::{Client, ClientError};
use lucerna_protocol::methods::{
    BAR_CLEAR, BAR_SET, DND_RELEASE, DND_REQUEST, PANEL_OPEN, TUI_OPEN,
};
use lucerna_protocol::{
    BarClearParams, BarItem, DndRequestParams, PanelOpenParams, TuiOpenParams, to_value,
};
use serde_json::Value;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

use crate::idle::{WorkGuard, WorkTracker};

/// Id do app do lado do shell no aperto de mão.
const SHELL_APP: &str = "shell";

/// Prazo para conectar (com o aperto de mão) e para cada resposta do shell.
/// Passou disso, a conexão conta como caída.
const SHELL_TIMEOUT: Duration = Duration::from_secs(2);

/// Quanto uma sessão precisa durar para a espera crescente voltar ao início,
/// quando o shell não respondeu nenhum pedido dela.
const HEALTHY_SESSION: Duration = Duration::from_secs(1);

/// Teto da espera entre tentativas de conexão.
const MAX_BACKOFF: Duration = Duration::from_secs(5);

/// Espera antes da tentativa `attempt` (a partir de `0`): `min(100 ms × 2^attempt, 5 s)`.
///
/// Detalhe de implementação, público só para os testes; pode mudar sem aviso.
#[doc(hidden)]
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
    /// Seguro enquanto houver item de barra ou não perturbe desejado.
    guard: Mutex<Option<WorkGuard>>,
    connected: watch::Sender<bool>,
    /// Fila de pedidos avulsos da tarefa de conexão atual.
    actions: Mutex<Option<mpsc::UnboundedSender<Action>>>,
}

/// Trava `m` ignorando o envenenamento (o estado sob a trava é coerente a cada
/// passo). A mesma regra do `lucerna-ipc`, que não a exporta.
pub(crate) fn lock<T: ?Sized>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Pedidos do app ao shell. Clonar dá outra alça para o mesmo estado.
///
/// Os itens da barra e o não perturbe pedido contam como trabalho pendente no
/// [`WorkTracker`] do [`Shell::new`]: o serviço não sai por ociosidade enquanto
/// houver um deles, mesmo com o shell fora.
///
/// Nenhum método é `async` nem devolve erro: sem shell, os pedidos viram nada
/// (os itens da barra e o não perturbe ficam guardados para quando ele voltar).
/// Um pedido recusado pelo shell não é repetido até o app mudar aquele item
/// (ou o não perturbe) ou o shell reconectar.
#[derive(Clone)]
pub struct Shell {
    inner: Arc<Inner>,
}

impl Shell {
    /// Um shell sem conexão, para o app `app`; a barra e o não perturbe contam em `work`.
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
    ///
    /// Enquanto ele estiver pedido, o serviço não sai por ociosidade (o shell o
    /// desligaria quando a conexão fecha).
    pub fn dnd_request(&self, reason: &str) {
        let new = Some(reason.to_string());
        self.update(|d| {
            let changed = d.dnd != new;
            d.dnd = new;
            changed
        });
    }

    /// Devolve o não perturbe.
    pub fn dnd_release(&self) {
        self.update(|d| d.dnd.take().is_some());
    }

    /// Pede ao shell para abrir a TUI do app com `args`. Sem shell, some.
    pub fn tui_open(&self, args: Vec<String>) {
        self.action(TUI_OPEN, to_value(&TuiOpenParams { args }));
    }

    /// Pede ao shell para abrir o painel do app, na página `page`. Sem shell, some.
    pub fn panel_open(&self, page: Option<String>) {
        self.action(PANEL_OPEN, to_value(&PanelOpenParams { page }));
    }

    /// Se o shell está conectado agora (aperto de mão feito). Volta a `false`
    /// quando a conexão cai ou a tarefa de conexão para.
    pub fn is_connected(&self) -> bool {
        *self.inner.connected.borrow()
    }

    fn update_bar(&self, f: impl FnOnce(&mut BTreeMap<String, BarItem>)) {
        self.update(|d| {
            let before = d.bar.clone();
            f(&mut d.bar);
            d.bar != before
        });
    }

    /// Muda o desejado com `f` (que diz se mudou) e segura o trabalho pendente
    /// enquanto houver item na barra ou não perturbe pedido.
    fn update(&self, f: impl FnOnce(&mut Desired) -> bool) {
        // O guarda e o desejado mudam juntos, sob a mesma trava.
        let mut guard = lock(&self.inner.guard);
        self.inner.desired.send_if_modified(f);
        let busy = {
            let d = self.inner.desired.borrow();
            !d.bar.is_empty() || d.dnd.is_some()
        };
        match (busy, guard.is_some()) {
            (true, false) => *guard = Some(self.inner.work.hold()),
            (false, true) => *guard = None,
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
            let started = tokio::time::Instant::now();
            // Pedidos avulsos de antes da conexão não valem.
            while actions.try_recv().is_ok() {}
            let answered = {
                // Volta a `false` também se a tarefa for abortada no meio.
                let _connected = Connected::new(&inner.connected);
                session(&client, &mut desired, &mut actions).await
            };
            while actions.try_recv().is_ok() {}
            // Um shell que aceita e fecha logo não pode fazer a tarefa
            // reconectar a cada 100 ms: a espera só volta ao início se a sessão
            // valeu (o shell respondeu um pedido, ou ela durou o bastante).
            if answered || started.elapsed() >= HEALTHY_SESSION {
                attempt = 0;
            }
        }
        tokio::time::sleep(backoff(attempt)).await;
        attempt = attempt.saturating_add(1);
    }
}

/// Marca o shell como conectado enquanto vive.
struct Connected<'a>(&'a watch::Sender<bool>);

impl<'a> Connected<'a> {
    fn new(tx: &'a watch::Sender<bool>) -> Self {
        tx.send_replace(true);
        Connected(tx)
    }
}

impl Drop for Connected<'_> {
    fn drop(&mut self) {
        self.0.send_replace(false);
    }
}

/// Mantém o shell igual ao desejado até a conexão cair. Devolve se o shell
/// respondeu algum pedido nesta sessão.
async fn session(
    client: &Client,
    desired: &mut watch::Receiver<Desired>,
    actions: &mut mpsc::UnboundedReceiver<Action>,
) -> bool {
    // O shell novo não tem nada nosso: o aplicado começa vazio.
    let mut applied = Desired::default();
    let mut answered = false;
    loop {
        let want = desired.borrow_and_update().clone();
        if apply(client, &mut applied, &want, &mut answered)
            .await
            .is_err()
        {
            return answered;
        }
        tokio::select! {
            // `biased`: uma mudança do estado desejado sai antes de um pedido
            // avulso que chegou no mesmo despertar.
            biased;
            r = desired.changed() => {
                if r.is_err() {
                    return answered;
                }
            }
            Some((method, params)) = actions.recv() => {
                if call(client, method, Some(params), &mut answered).await.is_err() {
                    return answered;
                }
            }
            _ = client.closed() => return answered,
        }
    }
}

/// Manda a diferença entre `applied` e `want`, um pedido por vez.
async fn apply(
    client: &Client,
    applied: &mut Desired,
    want: &Desired,
    answered: &mut bool,
) -> Result<(), ClientError> {
    for (id, item) in &want.bar {
        if applied.bar.get(id) != Some(item) {
            call(client, BAR_SET, Some(to_value(item)), answered).await?;
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
            answered,
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
                call(client, DND_REQUEST, Some(to_value(&p)), answered).await?;
            }
            None => call(client, DND_RELEASE, None, answered).await?,
        }
        applied.dnd = want.dnd.clone();
    }
    Ok(())
}

/// Um pedido ao shell, com prazo. Erro de conexão (ou prazo vencido) volta como
/// `Err` e derruba a sessão; um erro do próprio shell (`Rpc`, resposta estranha)
/// conta como aplicado (e vai para o stderr), para não repetir o mesmo pedido
/// sem fim. `answered` marca que o shell respondeu.
async fn call(
    client: &Client,
    method: &str,
    params: Option<Value>,
    answered: &mut bool,
) -> Result<(), ClientError> {
    match client.call_timeout(method, params, SHELL_TIMEOUT).await {
        Ok(_) => {
            *answered = true;
            Ok(())
        }
        Err(e @ (ClientError::Rpc(_) | ClientError::Decode(_))) => {
            *answered = true;
            eprintln!("lucerna: o shell recusou {method}: {e}");
            Ok(())
        }
        Err(e) => Err(e),
    }
}
