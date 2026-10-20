//! Tópicos: o estado que o serviço publica e os clientes assinam.

use std::sync::Arc;

use lucerna_protocol::State;
use serde_json::Value;
use tokio::sync::watch;

/// Tipo do tópico.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TopicKind {
    /// Tópico comum: retratos coalescidos por cliente.
    Common,
    /// Tópico contínuo: "só o último", produzido só com assinante.
    Stream,
}

struct Inner {
    name: String,
    kind: TopicKind,
    state: watch::Sender<State>,
    subscribers: watch::Sender<usize>,
}

/// Um tópico do serviço. Clonar dá outra alça para o mesmo tópico.
///
/// O tópico guarda sempre o último retrato publicado, mesmo sem assinantes. Num
/// tópico contínuo, "sem dado" (retrato `null`, `seq` 0) vale só antes do primeiro
/// [`publish`](Topic::publish): quem assina depois de um ciclo de assinatura recebe o
/// último quadro produzido. Se o próximo assinante deve ver vazio, o app publica
/// `null` ao parar de produzir.
#[derive(Clone)]
pub struct Topic {
    inner: Arc<Inner>,
}

impl Topic {
    pub(crate) fn new(name: &str, kind: TopicKind, initial: Value) -> Self {
        let state = State {
            topic: name.to_string(),
            seq: 0,
            data: initial,
        };
        Topic {
            inner: Arc::new(Inner {
                name: name.to_string(),
                kind,
                state: watch::Sender::new(state),
                subscribers: watch::Sender::new(0),
            }),
        }
    }

    /// Nome do tópico.
    pub fn name(&self) -> &str {
        &self.inner.name
    }

    /// Tipo do tópico.
    pub fn kind(&self) -> TopicKind {
        self.inner.kind
    }

    /// Troca o retrato do tópico e soma 1 ao `seq`. Guarda o valor mesmo sem assinantes.
    pub fn publish(&self, data: Value) {
        self.inner.state.send_modify(|s| {
            s.seq += 1;
            s.data = data;
        });
    }

    /// Retrato atual.
    pub fn current(&self) -> State {
        self.inner.state.borrow().clone()
    }

    /// Quantas assinaturas (de todas as conexões) o tópico tem agora.
    pub fn subscribers(&self) -> usize {
        *self.inner.subscribers.borrow()
    }

    /// Volta quando o tópico tem pelo menos um assinante.
    pub async fn wait_subscribed(&self) {
        let mut rx = self.inner.subscribers.subscribe();
        // O emissor vive enquanto `self` existir: o `wait_for` não falha.
        let _ = rx.wait_for(|n| *n > 0).await;
    }

    /// Volta quando o tópico não tem assinantes.
    pub async fn wait_unsubscribed(&self) {
        let mut rx = self.inner.subscribers.subscribe();
        let _ = rx.wait_for(|n| *n == 0).await;
    }

    pub(crate) fn receiver(&self) -> watch::Receiver<State> {
        self.inner.state.subscribe()
    }

    /// Conta um assinante enquanto a guarda viver.
    pub(crate) fn guard(&self) -> SubscriberGuard {
        self.inner.subscribers.send_modify(|n| *n += 1);
        SubscriberGuard(self.clone())
    }
}

/// Devolve o assinante ao ser largada.
pub(crate) struct SubscriberGuard(Topic);

impl Drop for SubscriberGuard {
    fn drop(&mut self) {
        self.0.inner.subscribers.send_modify(|n| *n -= 1);
    }
}
