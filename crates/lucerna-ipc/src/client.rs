//! Cliente do protocolo: aperto de mão, chamadas concorrentes e assinaturas.
//!
//! Uma tarefa leitora por conexão despacha as respostas pelo `id` e põe os
//! `lucerna.state` numa fila limitada (256). Com a fila cheia a leitora espera
//! (e o servidor coalesce os retratos); por isso quem assina tópicos deve
//! consumir [`Client::next_state`], senão as respostas das chamadas também param.
//!
//! O `seq` volta a `0` quando o serviço reinicia: o último `seq` visto é por
//! conexão, e um cliente que reconecta (um `Client` novo) começa do zero.

use std::collections::HashMap;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use lucerna_protocol::methods::{HELLO, STATE, SUBSCRIBE, UNSUBSCRIBE};
use lucerna_protocol::{
    HelloParams, HelloResult, Id, Message, Notification, PROTOCOL_VERSION, Request, RpcError,
    State, SubscribeResult, TopicsParams,
};
use serde_json::Value;
use tokio::io::AsyncWriteExt;
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;

use crate::framing::{FrameError, LineReader, MAX_MESSAGE};

/// Tamanho da fila de `lucerna.state` recebidos.
const STATE_QUEUE: usize = 256;

/// Quanto uma escrita que falhou com o socket quebrado espera pelo erro que o
/// serviço tenha mandado antes de fechar.
const FATAL_GRACE: Duration = Duration::from_secs(1);

/// Erro do cliente.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// Não deu para abrir o socket (o serviço não está lá).
    #[error("não foi possível conectar ao serviço: {0}")]
    Connect(io::Error),
    /// Erro de E/S com a conexão aberta.
    ///
    /// Inclui o `EPIPE` de uma escrita quando o serviço fechou sem mandar erro.
    #[error("erro de comunicação com o serviço: {0}")]
    Io(io::Error),
    /// O serviço respondeu com erro. Um `-32010` no aperto de mão também quer
    /// dizer que não dá para falar com o serviço.
    #[error("{0}")]
    Rpc(RpcError),
    /// A conexão fechou antes da resposta.
    #[error("o serviço fechou a conexão")]
    Closed,
}

/// Quem espera uma resposta.
struct Waiter {
    tx: oneshot::Sender<Result<Value, RpcError>>,
    /// Resposta de um `lucerna.subscribe`: o retrato atualiza o último `seq` visto.
    subscribe: bool,
}

/// Estado compartilhado entre o `Client` e a tarefa leitora.
struct Calls {
    /// `false` depois que a leitora terminou: ninguém mais vai responder.
    open: bool,
    waiting: HashMap<i64, Waiter>,
    /// Último erro sem `id` (ex.: `-32000` antes de o serviço fechar).
    fatal: Option<RpcError>,
}

type Shared = Arc<Mutex<Calls>>;

fn lock(s: &Shared) -> MutexGuard<'_, Calls> {
    s.lock().unwrap_or_else(|e| e.into_inner())
}

/// Tira o pedido do mapa se a chamada for cancelada antes da resposta.
struct Forget<'a> {
    calls: &'a Shared,
    id: i64,
}

impl Drop for Forget<'_> {
    fn drop(&mut self) {
        lock(self.calls).waiting.remove(&self.id);
    }
}

/// Cliente de um serviço do Lucerna, por conexão.
pub struct Client {
    hello: HelloResult,
    writer: Arc<tokio::sync::Mutex<OwnedWriteHalf>>,
    next_id: AtomicI64,
    calls: Shared,
    states: mpsc::Receiver<State>,
    closed: watch::Receiver<bool>,
    reader: JoinHandle<()>,
}

impl Drop for Client {
    fn drop(&mut self) {
        self.reader.abort();
    }
}

impl Client {
    /// Conecta ao socket em `path` e faz o aperto de mão como `client` com o app `app`.
    pub async fn connect(path: &Path, client: &str, app: &str) -> Result<Client, ClientError> {
        let stream = UnixStream::connect(path)
            .await
            .map_err(ClientError::Connect)?;
        let (rd, wr) = stream.into_split();
        let calls: Shared = Arc::new(Mutex::new(Calls {
            open: true,
            waiting: HashMap::new(),
            fatal: None,
        }));
        let (states_tx, states) = mpsc::channel(STATE_QUEUE);
        let (closed_tx, closed) = watch::channel(false);
        let reader = tokio::spawn(read_loop(
            LineReader::new(rd, MAX_MESSAGE),
            calls.clone(),
            states_tx,
            closed_tx,
        ));
        let mut c = Client {
            hello: HelloResult {
                protocol: PROTOCOL_VERSION,
                app: String::new(),
                version: String::new(),
                topics: Vec::new(),
                streams: Vec::new(),
                methods: Vec::new(),
            },
            writer: Arc::new(tokio::sync::Mutex::new(wr)),
            next_id: AtomicI64::new(1),
            calls,
            states,
            closed,
            reader,
        };
        let params = HelloParams {
            protocol: PROTOCOL_VERSION,
            client: client.to_string(),
            app: app.to_string(),
        };
        let result = c.request(HELLO, Some(to_value(&params)), false).await?;
        c.hello = from_value(result)?;
        Ok(c)
    }

    /// Resposta do aperto de mão.
    pub fn hello(&self) -> &HelloResult {
        &self.hello
    }

    /// Chama `method` e espera a resposta. Pode ser chamado de várias tarefas ao mesmo tempo.
    pub async fn call(&self, method: &str, params: Option<Value>) -> Result<Value, ClientError> {
        self.request(method, params, false).await
    }

    /// Manda uma notificação (sem resposta).
    pub async fn notify(&self, method: &str, params: Option<Value>) -> Result<(), ClientError> {
        if !lock(&self.calls).open {
            return Err(ClientError::Closed);
        }
        let msg = Message::Notification(Notification {
            method: method.to_string(),
            params,
        });
        self.send(&msg).await.map_err(ClientError::Io)
    }

    /// Assina `topics` e devolve o retrato atual de cada um.
    ///
    /// As mudanças seguintes chegam por [`Client::next_state`].
    pub async fn subscribe(&self, topics: &[&str]) -> Result<Vec<State>, ClientError> {
        let result = self
            .request(SUBSCRIBE, Some(topics_params(topics)), true)
            .await?;
        let r: SubscribeResult = from_value(result)?;
        Ok(r.states)
    }

    /// Cancela as assinaturas de `topics`.
    pub async fn unsubscribe(&self, topics: &[&str]) -> Result<(), ClientError> {
        self.request(UNSUBSCRIBE, Some(topics_params(topics)), false)
            .await
            .map(|_| ())
    }

    /// Próximo retrato de um tópico assinado, em ordem de `seq` por tópico
    /// (os que chegam com `seq` já visto são descartados). `None` quando a conexão fecha.
    pub async fn next_state(&mut self) -> Option<State> {
        self.states.recv().await
    }

    /// Volta quando a conexão fecha.
    pub async fn closed(&self) {
        let mut rx = self.closed.clone();
        // Erro aqui é a leitora ter terminado: a conexão também fechou.
        let _ = rx.wait_for(|closed| *closed).await;
    }

    async fn request(
        &self,
        method: &str,
        params: Option<Value>,
        subscribe: bool,
    ) -> Result<Value, ClientError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        {
            let mut calls = lock(&self.calls);
            if !calls.open {
                return Err(ClientError::Closed);
            }
            calls.waiting.insert(id, Waiter { tx, subscribe });
        }
        let _forget = Forget {
            calls: &self.calls,
            id,
        };
        let msg = Message::Request(Request {
            id: Id::Num(id),
            method: method.to_string(),
            params,
        });
        if let Err(e) = self.send(&msg).await {
            // Uma mensagem grande demais pode dar `EPIPE` antes de lermos o
            // `-32000` que o serviço mandou: espera a leitora chegar ao fim.
            if is_broken(&e) {
                let _ = tokio::time::timeout(FATAL_GRACE, self.closed()).await;
                if let Some(f) = lock(&self.calls).fatal.clone() {
                    return Err(ClientError::Rpc(f));
                }
            }
            return Err(ClientError::Io(e));
        }
        match rx.await {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(e)) => Err(ClientError::Rpc(e)),
            Err(_) => Err(lock(&self.calls)
                .fatal
                .clone()
                .map_or(ClientError::Closed, ClientError::Rpc)),
        }
    }

    /// Escreve uma linha numa tarefa própria: cancelar quem chamou não deixa
    /// uma linha pela metade no fluxo.
    async fn send(&self, msg: &Message) -> io::Result<()> {
        let mut bytes = msg.to_json().into_bytes();
        bytes.push(b'\n');
        let writer = self.writer.clone();
        tokio::spawn(async move {
            let mut w = writer.lock().await;
            w.write_all(&bytes).await?;
            w.flush().await
        })
        .await
        .map_err(io::Error::other)?
    }
}

fn is_broken(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::BrokenPipe | io::ErrorKind::ConnectionReset
    )
}

fn topics_params(topics: &[&str]) -> Value {
    to_value(&TopicsParams {
        topics: topics.iter().map(|t| t.to_string()).collect(),
    })
}

fn to_value<T: serde::Serialize>(v: &T) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}

fn from_value<T: serde::de::DeserializeOwned>(v: Value) -> Result<T, ClientError> {
    serde_json::from_value(v)
        .map_err(|e| ClientError::Io(io::Error::new(io::ErrorKind::InvalidData, e)))
}

/// Lê as mensagens do serviço até a conexão fechar.
async fn read_loop(
    mut lines: LineReader<OwnedReadHalf>,
    calls: Shared,
    states: mpsc::Sender<State>,
    closed: watch::Sender<bool>,
) {
    let mut seen: HashMap<String, u64> = HashMap::new();
    loop {
        let line = match lines.next_line().await {
            Ok(Some(line)) => line,
            Err(FrameError::InvalidUtf8) => continue,
            // Fim do fluxo, `ECONNRESET` (o serviço fechou com dados não lidos),
            // outro erro de E/S ou linha grande demais: a conexão acabou.
            Ok(None) | Err(_) => break,
        };
        match Message::parse(&line) {
            Ok(Message::Response(r)) => match r.id {
                Some(Id::Num(id)) => {
                    let Some(w) = lock(&calls).waiting.remove(&id) else {
                        continue;
                    };
                    if w.subscribe
                        && let Ok(v) = &r.outcome
                        && let Ok(s) = serde_json::from_value::<SubscribeResult>(v.clone())
                    {
                        for st in s.states {
                            let last = seen.entry(st.topic).or_insert(st.seq);
                            *last = (*last).max(st.seq);
                        }
                    }
                    let _ = w.tx.send(r.outcome);
                }
                None => {
                    if let Err(e) = r.outcome {
                        lock(&calls).fatal = Some(e);
                    }
                }
                Some(Id::Str(_)) => {}
            },
            Ok(Message::Notification(n)) if n.method == STATE => {
                let Some(st) = n
                    .params
                    .and_then(|p| serde_json::from_value::<State>(p).ok())
                else {
                    continue;
                };
                if seen.get(&st.topic).is_some_and(|&last| st.seq <= last) {
                    continue;
                }
                seen.insert(st.topic.clone(), st.seq);
                if states.send(st).await.is_err() {
                    break;
                }
            }
            _ => {}
        }
    }
    {
        let mut c = lock(&calls);
        c.open = false;
        // Largar os `oneshot` acorda quem espera com "conexão fechou".
        c.waiting.clear();
    }
    closed.send_replace(true);
}
