//! Ferramentas para os testes dos apps.

use std::path::{Path, PathBuf};
use std::time::Duration;

use lucerna_ipc::ServerBuilder;
use lucerna_protocol::methods::{
    BAR_CLEAR, BAR_SET, DND_RELEASE, DND_REQUEST, PANEL_OPEN, TUI_OPEN,
};
use serde_json::{Value, json};
use tokio::net::UnixListener;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

/// Quanto o [`FakeShell`] espera por uma chamada ou por uma conexão: cobre o
/// teto de 5 s da reconexão com folga.
const WAIT: Duration = Duration::from_secs(7);

/// Um shell falso num socket: atende os seis métodos do shell, registra cada
/// chamada e responde `{}`.
pub struct FakeShell {
    path: PathBuf,
    calls: mpsc::UnboundedReceiver<(String, Value)>,
    connections: watch::Receiver<usize>,
    task: JoinHandle<()>,
}

impl FakeShell {
    /// Sobe o shell falso em `path` (o arquivo não pode existir).
    pub async fn start(path: &Path) -> FakeShell {
        let (tx, calls) = mpsc::unbounded_channel();
        let mut b = ServerBuilder::new("shell", env!("CARGO_PKG_VERSION"));
        for name in [
            BAR_SET,
            BAR_CLEAR,
            DND_REQUEST,
            DND_RELEASE,
            TUI_OPEN,
            PANEL_OPEN,
        ] {
            let tx = tx.clone();
            b.method(name, move |params| {
                let _ = tx.send((name.to_string(), params.unwrap_or(Value::Null)));
                async { Ok(json!({})) }
            });
        }
        let server = b.build();
        let connections = server.connections();
        let listener = UnixListener::bind(path).unwrap_or_else(|e| {
            panic!("FakeShell: não deu para escutar em {}: {e}", path.display())
        });
        let task = tokio::spawn(async move {
            let _ = server.serve(listener).await;
        });
        FakeShell {
            path: path.to_path_buf(),
            calls,
            connections,
            task,
        }
    }

    /// Próxima chamada recebida, `(método, params)`; `params` ausente vira `null`.
    ///
    /// Entra em pânico se nada chegar em 7 s.
    pub async fn next_call(&mut self) -> (String, Value) {
        match tokio::time::timeout(WAIT, self.calls.recv()).await {
            Ok(Some(c)) => c,
            Ok(None) => panic!("FakeShell: o servidor parou"),
            Err(_) => panic!("FakeShell: nenhuma chamada em {WAIT:?}"),
        }
    }

    /// Entra em pânico se alguma chamada chegar em `d`.
    pub async fn no_call_within(&mut self, d: Duration) {
        if let Ok(Some(c)) = tokio::time::timeout(d, self.calls.recv()).await {
            panic!("FakeShell: chamada inesperada {c:?}");
        }
    }

    /// Volta quando há ao menos uma conexão aberta (pânico depois de 7 s).
    pub async fn wait_connected(&self) {
        self.wait_for(|n| n > 0, "nenhuma conexão").await;
    }

    /// Volta quando não há conexão aberta (pânico depois de 7 s).
    pub async fn wait_disconnected(&self) {
        self.wait_for(|n| n == 0, "a conexão não fechou").await;
    }

    async fn wait_for(&self, f: impl Fn(usize) -> bool, what: &str) {
        let mut rx = self.connections.clone();
        match tokio::time::timeout(WAIT, rx.wait_for(|n| f(*n))).await {
            Ok(Ok(_)) => {}
            Ok(Err(_)) => panic!("FakeShell: o servidor parou"),
            Err(_) => panic!("FakeShell: {what} em {WAIT:?}"),
        }
    }

    /// Para o shell falso: fecha as conexões e apaga o socket.
    pub async fn stop(mut self) {
        self.task.abort();
        let _ = (&mut self.task).await;
        let _ = std::fs::remove_file(&self.path);
    }
}

impl Drop for FakeShell {
    fn drop(&mut self) {
        self.task.abort();
    }
}
