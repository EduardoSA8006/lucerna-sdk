//! Trabalho pendente e a espera pela ociosidade.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;

/// Conta o trabalho pendente do app (um timer rodando, um item na barra).
///
/// Enquanto houver um [`WorkGuard`] vivo, o serviço não sai por ociosidade.
/// Clonar dá outra alça para o mesmo contador.
#[derive(Clone, Debug)]
pub struct WorkTracker {
    count: Arc<watch::Sender<usize>>,
}

impl Default for WorkTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkTracker {
    /// Um contador sem trabalho pendente.
    pub fn new() -> Self {
        WorkTracker {
            count: Arc::new(watch::Sender::new(0)),
        }
    }

    /// Marca um trabalho pendente até o guarda devolvido cair.
    pub fn hold(&self) -> WorkGuard {
        self.count.send_modify(|n| *n += 1);
        WorkGuard {
            count: self.count.clone(),
        }
    }

    /// Quantos trabalhos estão pendentes agora.
    pub fn pending(&self) -> usize {
        *self.count.borrow()
    }

    /// Acompanha o número de trabalhos pendentes.
    pub fn watch(&self) -> watch::Receiver<usize> {
        self.count.subscribe()
    }
}

/// Um trabalho pendente; largá-lo encerra o trabalho.
#[derive(Debug)]
#[must_use = "o trabalho acaba quando o guarda cai"]
pub struct WorkGuard {
    count: Arc<watch::Sender<usize>>,
}

impl Drop for WorkGuard {
    fn drop(&mut self) {
        self.count.send_modify(|n| *n -= 1);
    }
}

/// Volta quando `connections` e `work` ficam em `0` por `timeout` seguidos.
///
/// Qualquer mudança em um dos dois recomeça a contagem. Um emissor largado
/// conta como fixo no último valor.
pub async fn wait_idle(
    mut connections: watch::Receiver<usize>,
    mut work: watch::Receiver<usize>,
    timeout: Duration,
) {
    let mut conns_open = true;
    let mut work_open = true;
    loop {
        let idle = *connections.borrow_and_update() == 0 && *work.borrow_and_update() == 0;
        let sleep = async {
            if idle {
                tokio::time::sleep(timeout).await
            } else {
                std::future::pending().await
            }
        };
        tokio::select! {
            _ = sleep => return,
            r = connections.changed(), if conns_open => conns_open = r.is_ok(),
            r = work.changed(), if work_open => work_open = r.is_ok(),
        }
    }
}
