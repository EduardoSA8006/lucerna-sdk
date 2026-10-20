//! O serviço do `lucerna-hello` em processo, num socket temporário.

// Cada arquivo de teste usa uma parte das ferramentas.
#![allow(dead_code)]

use std::path::PathBuf;
use std::time::Duration;

use lucerna_hello::{Producer, hello};
use lucerna_ipc::Client;
use lucerna_ipc::raw::RawConn;
use lucerna_service::Topic;
use tempfile::TempDir;
use tokio::net::UnixListener;
use tokio::task::JoinHandle;

/// O serviço rodando numa tarefa.
pub struct Running {
    _dir: TempDir,
    pub path: PathBuf,
    ticks: Topic,
    _producer: Producer,
    task: JoinHandle<()>,
}

/// Sobe o serviço num socket temporário; com `shell`, conecta ao `shell.sock` ali.
pub async fn start(shell: Option<PathBuf>) -> Running {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hello.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let mut h = hello();
    h.service.idle_timeout(Duration::from_secs(60));
    let task = tokio::spawn(async move {
        h.service.run_with(listener, shell).await.unwrap();
    });
    Running {
        _dir: dir,
        path,
        ticks: h.ticks,
        _producer: h.producer,
        task,
    }
}

impl Running {
    /// Conexão crua, já com o aperto de mão feito.
    pub async fn raw(&self) -> RawConn {
        let mut c = RawConn::connect(&self.path).await.unwrap();
        let r = c.hello("hello").await.unwrap();
        assert!(r.get("result").is_some(), "aperto de mão falhou: {r}");
        c
    }

    /// Cliente com o aperto de mão feito.
    pub async fn client(&self) -> Client {
        Client::connect(&self.path, "test", "hello").await.unwrap()
    }

    /// O tópico contínuo `ticks`.
    pub fn ticks(&self) -> &Topic {
        &self.ticks
    }

    /// Para o serviço e espera a tarefa terminar.
    pub async fn stop(mut self) {
        self.task.abort();
        let _ = (&mut self.task).await;
    }
}
