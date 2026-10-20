//! Servidor do protocolo: aperto de mão, métodos do app e assinaturas coalescidas.
//!
//! Cada conexão tem um leitor (trata `lucerna.*` em linha, na ordem, e põe cada
//! método do app numa tarefa própria), uma fila de respostas limitada e um escritor
//! único que dá prioridade às respostas e depois manda os retratos pendentes (no
//! máximo um por tópico, o mais novo).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::future::Future;
use std::io;
use std::ops::ControlFlow;
use std::panic::AssertUnwindSafe;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::{Context, Poll};
use std::time::Duration;

use lucerna_protocol::methods::{HELLO, STATE, SUBSCRIBE, UNSUBSCRIBE};
use lucerna_protocol::{
    HelloParams, HelloResult, Id, Message, Notification, PROTOCOL_VERSION, Request, Response,
    RpcError, State, SubscribeResult, TopicsParams, codes, is_reserved,
};
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::UnixListener;
use tokio::sync::{mpsc, watch};
use tokio::task::{AbortHandle, JoinSet};

use crate::framing::{FrameError, LineReader, MAX_MESSAGE, write_message};
use crate::topic::{SubscriberGuard, Topic, TopicKind};

/// Tamanho da fila de respostas de cada conexão.
const RESPONSE_QUEUE: usize = 64;
/// Quanto a conexão ainda espera, ao fechar, os métodos em andamento e o escritor.
const FLUSH_TIMEOUT: Duration = Duration::from_secs(5);

// `errno` do Linux com que o `accept` pode falhar por falta de recurso.
/// Sem memória.
const ENOMEM: i32 = 12;
/// Tabela de arquivos do sistema cheia.
const ENFILE: i32 = 23;
/// Limite de descritores do processo.
const EMFILE: i32 = 24;
/// Sem espaço de buffer.
const ENOBUFS: i32 = 105;

type BoxFuture = Pin<Box<dyn Future<Output = Result<Value, RpcError>> + Send>>;
type Handler = Arc<dyn Fn(Option<Value>) -> BoxFuture + Send + Sync>;

/// Monta um [`Server`]: tópicos, tópicos contínuos e métodos do app.
pub struct ServerBuilder {
    app: String,
    version: String,
    topics: Vec<Topic>,
    methods: Vec<(String, Handler)>,
    max_message: usize,
}

impl ServerBuilder {
    /// Começa o servidor do app `app`, na versão `version`.
    pub fn new(app: &str, version: &str) -> Self {
        ServerBuilder {
            app: app.to_string(),
            version: version.to_string(),
            topics: Vec::new(),
            methods: Vec::new(),
            max_message: MAX_MESSAGE,
        }
    }

    fn new_topic(&mut self, name: &str, kind: TopicKind, initial: Value) -> Topic {
        assert!(
            !is_reserved(name),
            "o nome {name:?} é reservado ao protocolo (lucerna.)"
        );
        assert!(
            self.topics.iter().all(|t| t.name() != name),
            "tópico repetido: {name:?}"
        );
        let topic = Topic::new(name, kind, initial);
        self.topics.push(topic.clone());
        topic
    }

    /// Declara um tópico comum com o retrato inicial `initial` (`seq` 0).
    ///
    /// Entra em pânico com nome reservado (`lucerna.`) ou repetido.
    pub fn topic(&mut self, name: &str, initial: Value) -> Topic {
        self.new_topic(name, TopicKind::Common, initial)
    }

    /// Declara um tópico contínuo ("só o último"), com retrato inicial `null`.
    ///
    /// Entra em pânico com nome reservado (`lucerna.`) ou repetido.
    pub fn stream(&mut self, name: &str) -> Topic {
        self.new_topic(name, TopicKind::Stream, Value::Null)
    }

    /// Declara um método do app. Cada pedido roda numa tarefa própria; um pânico
    /// vira o erro `-32603`.
    ///
    /// Entra em pânico com nome reservado (`lucerna.`) ou repetido.
    pub fn method<F, Fut>(&mut self, name: &str, f: F)
    where
        F: Fn(Option<Value>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Value, RpcError>> + Send + 'static,
    {
        assert!(
            !is_reserved(name),
            "o nome {name:?} é reservado ao protocolo (lucerna.)"
        );
        assert!(
            self.methods.iter().all(|(n, _)| n != name),
            "método repetido: {name:?}"
        );
        let handler: Handler = Arc::new(move |params| Box::pin(f(params)));
        self.methods.push((name.to_string(), handler));
    }

    /// Troca o limite de tamanho de uma mensagem (padrão [`MAX_MESSAGE`]).
    pub fn max_message(&mut self, bytes: usize) {
        self.max_message = bytes;
    }

    /// Termina o servidor.
    pub fn build(self) -> Server {
        let names = self.methods.iter().map(|(n, _)| n.clone()).collect();
        Server {
            inner: Arc::new(Inner {
                app: self.app,
                version: self.version,
                topics: self.topics,
                method_names: names,
                methods: self.methods.into_iter().collect(),
                max_message: self.max_message,
                connections: watch::Sender::new(0),
            }),
        }
    }
}

struct Inner {
    app: String,
    version: String,
    topics: Vec<Topic>,
    method_names: Vec<String>,
    methods: HashMap<String, Handler>,
    max_message: usize,
    connections: watch::Sender<usize>,
}

impl Inner {
    fn topic(&self, name: &str) -> Option<&Topic> {
        self.topics.iter().find(|t| t.name() == name)
    }
}

/// O servidor do protocolo. Clonar dá outra alça para o mesmo servidor.
#[derive(Clone)]
pub struct Server {
    inner: Arc<Inner>,
}

impl Server {
    /// Aceita conexões de `listener` e atende cada uma numa tarefa.
    ///
    /// Largar o future fecha todas as conexões.
    pub async fn serve(&self, listener: UnixListener) -> io::Result<()> {
        let mut conns = JoinSet::new();
        loop {
            tokio::select! {
                r = listener.accept() => match r {
                    Ok((stream, _)) => {
                        let server = self.clone();
                        conns.spawn(async move { server.serve_connection(stream).await });
                    }
                    Err(e) => match e.kind() {
                        io::ErrorKind::ConnectionAborted
                        | io::ErrorKind::ConnectionReset
                        | io::ErrorKind::Interrupted => {}
                        // Sem descritores ou sem memória: espera um pouco e tenta de novo.
                        _ if matches!(
                            e.raw_os_error(),
                            Some(ENOMEM | ENFILE | EMFILE | ENOBUFS)
                        ) => {
                            tokio::time::sleep(Duration::from_millis(100)).await;
                        }
                        _ => return Err(e),
                    },
                },
                Some(_) = conns.join_next(), if !conns.is_empty() => {}
            }
        }
    }

    /// Atende uma conexão até ela fechar.
    pub async fn serve_connection<S>(&self, stream: S)
    where
        S: AsyncRead + AsyncWrite + Send + 'static,
    {
        let _count = ConnectionCount::new(self.inner.clone());
        let (rd, wr) = tokio::io::split(stream);
        let (resp_tx, resp_rx) = mpsc::channel(RESPONSE_QUEUE);
        let (dirty_tx, dirty_rx) = mpsc::channel(1);
        let pending = Pending::default();
        let mut conn = Conn {
            server: self.inner.clone(),
            hello: false,
            resp: resp_tx,
            dirty: dirty_tx,
            pending: pending.clone(),
            subs: HashMap::new(),
            sub_tasks: JoinSet::new(),
            tasks: JoinSet::new(),
        };

        let writer = write_loop(wr, resp_rx, dirty_rx, pending);
        tokio::pin!(writer);
        let end = {
            let reader = conn.read_loop(LineReader::new(rd, self.inner.max_message));
            tokio::pin!(reader);
            tokio::select! {
                end = &mut reader => end,
                _ = &mut writer => return, // o escritor falhou: larga tudo
            }
        };
        let deadline = tokio::time::Instant::now() + FLUSH_TIMEOUT;
        if end == End::Eof {
            // Fim limpo (pode ser só um meio fechamento, como no `echo … | socat`):
            // as assinaturas saem já, mas os métodos em andamento terminam e
            // respondem, dentro do limite.
            conn.subs.clear();
            conn.sub_tasks.abort_all();
            let methods = async { while conn.tasks.join_next().await.is_some() {} };
            tokio::select! {
                _ = tokio::time::timeout_at(deadline, methods) => {}
                _ = &mut writer => return,
            }
        }
        // Aborta o que sobrou e devolve os assinantes.
        drop(conn);
        // Manda o que já estava na fila (o erro de quem causou o fechamento, as
        // respostas dos métodos que terminaram).
        let _ = tokio::time::timeout_at(deadline, writer).await;
    }

    /// Quantas conexões estão abertas agora.
    pub fn connections(&self) -> watch::Receiver<usize> {
        self.inner.connections.subscribe()
    }

    /// A resposta do `lucerna.hello` deste servidor.
    pub fn hello_result(&self) -> HelloResult {
        let names = |kind| {
            self.inner
                .topics
                .iter()
                .filter(|t| t.kind() == kind)
                .map(|t| t.name().to_string())
                .collect()
        };
        HelloResult {
            protocol: PROTOCOL_VERSION,
            app: self.inner.app.clone(),
            version: self.inner.version.clone(),
            topics: names(TopicKind::Common),
            streams: names(TopicKind::Stream),
            methods: self.inner.method_names.clone(),
        }
    }
}

/// Conta a conexão enquanto viver.
struct ConnectionCount(Arc<Inner>);

impl ConnectionCount {
    fn new(inner: Arc<Inner>) -> Self {
        inner.connections.send_modify(|n| *n += 1);
        ConnectionCount(inner)
    }
}

impl Drop for ConnectionCount {
    fn drop(&mut self) {
        self.0.connections.send_modify(|n| *n -= 1);
    }
}

/// Retratos esperando o escritor: no máximo um por tópico, o mais novo.
#[derive(Clone, Default)]
struct Pending(Arc<Mutex<BTreeMap<String, State>>>);

impl Pending {
    fn lock(&self) -> MutexGuard<'_, BTreeMap<String, State>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Uma assinatura da conexão: largar aborta a tarefa e devolve o assinante.
struct Subscription {
    task: AbortHandle,
    _guard: SubscriberGuard,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Como o leitor terminou.
#[derive(PartialEq, Eq)]
enum End {
    /// Fim do fluxo, sem erro: os métodos em andamento ainda respondem.
    Eof,
    /// Erro de E/S, mensagem grande demais ou versão incompatível: fecha já.
    Abort,
}

/// Estado do leitor de uma conexão.
struct Conn {
    server: Arc<Inner>,
    hello: bool,
    resp: mpsc::Sender<Message>,
    dirty: mpsc::Sender<()>,
    pending: Pending,
    subs: HashMap<String, Subscription>,
    /// Tarefas das assinaturas (uma por tópico assinado).
    sub_tasks: JoinSet<()>,
    /// Tarefas dos métodos do app.
    tasks: JoinSet<()>,
}

impl Conn {
    async fn read_loop<R: AsyncRead + Unpin>(&mut self, mut lines: LineReader<R>) -> End {
        loop {
            while self.tasks.try_join_next().is_some() {}
            while self.sub_tasks.try_join_next().is_some() {}
            let line = match lines.next_line().await {
                Ok(Some(line)) => line,
                Ok(None) => return End::Eof,
                Err(FrameError::Io(_)) => return End::Abort,
                Err(FrameError::TooLarge) => {
                    // O resto da linha não foi lido: a conexão fecha.
                    let e = RpcError::message_too_large(self.server.max_message);
                    self.reply(None, Err(e)).await;
                    return End::Abort;
                }
                Err(FrameError::InvalidUtf8) => {
                    self.reply(None, Err(RpcError::parse_error())).await;
                    continue;
                }
            };
            match Message::parse(&line) {
                Ok(Message::Request(r)) => {
                    if self.request(r).await.is_break() {
                        return End::Abort;
                    }
                }
                Ok(Message::Notification(n)) => self.notification(n).await,
                // O servidor não faz pedidos ao cliente: resposta solta é ignorada.
                Ok(Message::Response(_)) => {}
                Err(e) => {
                    let id = if e.code == codes::INVALID_REQUEST {
                        readable_id(&line)
                    } else {
                        None
                    };
                    self.reply(id, Err(e)).await;
                }
            }
        }
    }

    /// Põe uma resposta na fila (espera vaga).
    async fn reply(&self, id: Option<Id>, outcome: Result<Value, RpcError>) {
        // Só falha se o escritor acabou; aí a conexão já está fechando.
        let _ = self
            .resp
            .send(Message::Response(Response { id, outcome }))
            .await;
    }

    async fn request(&mut self, r: Request) -> ControlFlow<()> {
        let Request { id, method, params } = r;
        if method == HELLO {
            return self.hello(id, params).await;
        }
        if !self.hello {
            self.reply(Some(id), Err(RpcError::hello_required())).await;
            return ControlFlow::Continue(());
        }
        match method.as_str() {
            SUBSCRIBE => self.subscribe(id, params).await,
            UNSUBSCRIBE => {
                let outcome = self.unsubscribe(params);
                self.reply(Some(id), outcome).await;
            }
            m if is_reserved(m) => {
                self.reply(Some(id), Err(RpcError::method_not_found(m)))
                    .await
            }
            m => match self.server.methods.get(m).cloned() {
                Some(h) => self.call(Some(id), h, params).await,
                None => {
                    self.reply(Some(id), Err(RpcError::method_not_found(m)))
                        .await
                }
            },
        }
        ControlFlow::Continue(())
    }

    async fn notification(&mut self, n: Notification) {
        // Notificação nunca tem resposta: desconhecida ou antes do hello, some em silêncio.
        if !self.hello || is_reserved(&n.method) {
            return;
        }
        if let Some(h) = self.server.methods.get(&n.method).cloned() {
            self.call(None, h, n.params).await;
        }
    }

    /// Roda um método do app numa tarefa; com `id`, a resposta vai para a fila.
    async fn call(&mut self, id: Option<Id>, h: Handler, params: Option<Value>) {
        // A vaga na fila é reservada antes: fila cheia, o leitor para de ler.
        let Ok(permit) = self.resp.clone().reserve_owned().await else {
            return;
        };
        self.tasks.spawn(async move {
            let outcome = CatchUnwind(Box::pin(async move { h(params).await })).await;
            if let Some(id) = id {
                permit.send(Message::Response(Response {
                    id: Some(id),
                    outcome,
                }));
            }
        });
    }

    async fn hello(&mut self, id: Id, params: Option<Value>) -> ControlFlow<()> {
        let parsed = params.and_then(|p| serde_json::from_value::<HelloParams>(p).ok());
        let outcome = match parsed {
            None => Err(RpcError::invalid_params(
                "o lucerna.hello pede protocol (MAJOR.MINOR), client e app",
            )),
            Some(p) if !p.protocol.is_compatible_with(&PROTOCOL_VERSION) => {
                let e = RpcError::incompatible_version(p.protocol, PROTOCOL_VERSION);
                self.reply(Some(id), Err(e)).await;
                return ControlFlow::Break(());
            }
            Some(p) if p.app != self.server.app => Err(RpcError::invalid_params(&format!(
                "este serviço é o app {}, não {}",
                self.server.app, p.app
            ))),
            Some(_) => {
                self.hello = true;
                let result = Server {
                    inner: self.server.clone(),
                }
                .hello_result();
                Ok(serde_json::to_value(result).unwrap_or(Value::Null))
            }
        };
        self.reply(Some(id), outcome).await;
        ControlFlow::Continue(())
    }

    /// Os tópicos pedidos, todos conhecidos (ou `-32012` com o primeiro desconhecido).
    fn topics(&self, params: Option<Value>) -> Result<Vec<Topic>, RpcError> {
        let p = params
            .and_then(|p| serde_json::from_value::<TopicsParams>(p).ok())
            .ok_or_else(|| RpcError::invalid_params("informe topics, uma lista de nomes"))?;
        let mut seen = HashSet::new();
        let mut topics = Vec::new();
        for name in &p.topics {
            let t = self
                .server
                .topic(name)
                .ok_or_else(|| RpcError::unknown_topic(name))?;
            if seen.insert(name.clone()) {
                topics.push(t.clone());
            }
        }
        Ok(topics)
    }

    async fn subscribe(&mut self, id: Id, params: Option<Value>) {
        let topics = match self.topics(params) {
            Ok(t) => t,
            Err(e) => return self.reply(Some(id), Err(e)).await,
        };
        let mut states = Vec::new();
        let mut fresh = Vec::new();
        for t in topics {
            // Antes do retrato e da tarefa nova: tirar depois apagaria um retrato
            // mais novo que a tarefa (noutra thread) já tivesse deixado ali.
            self.pending.lock().remove(t.name());
            let mut rx = t.receiver();
            states.push(rx.borrow_and_update().clone());
            let guard = t.guard();
            fresh.push((t, rx, guard));
        }
        // O retrato entra na fila antes de a tarefa da assinatura nascer: nenhum
        // `lucerna.state` chega antes da resposta.
        let result = serde_json::to_value(SubscribeResult { states }).unwrap_or(Value::Null);
        self.reply(Some(id), Ok(result)).await;
        for (t, rx, guard) in fresh {
            let name = t.name().to_string();
            let task = self.sub_tasks.spawn(watch_topic(
                name.clone(),
                rx,
                self.pending.clone(),
                self.dirty.clone(),
            ));
            // Reassinar troca a assinatura; a guarda nova já conta antes de a velha sair.
            self.subs.insert(
                name.clone(),
                Subscription {
                    task,
                    _guard: guard,
                },
            );
        }
    }

    fn unsubscribe(&mut self, params: Option<Value>) -> Result<Value, RpcError> {
        for t in self.topics(params)? {
            self.subs.remove(t.name());
            self.pending.lock().remove(t.name());
        }
        Ok(json!({}))
    }
}

/// Espera mudanças do tópico e deixa o retrato mais novo no mapa pendente.
async fn watch_topic(
    name: String,
    mut rx: watch::Receiver<State>,
    pending: Pending,
    dirty: mpsc::Sender<()>,
) {
    while rx.changed().await.is_ok() {
        let state = rx.borrow_and_update().clone();
        pending.lock().insert(name.clone(), state);
        // Cheio quer dizer que o escritor já vai olhar o mapa.
        let _ = dirty.try_send(());
    }
}

/// Escritor único da conexão: respostas primeiro, depois os retratos pendentes.
///
/// `write_message` não é segura para cancelar, por isso só roda no corpo dos braços.
async fn write_loop<W: AsyncWrite + Unpin>(
    mut wr: W,
    mut resp: mpsc::Receiver<Message>,
    mut dirty: mpsc::Receiver<()>,
    pending: Pending,
) -> io::Result<()> {
    loop {
        tokio::select! {
            biased;
            m = resp.recv() => match m {
                Some(m) => write_message(&mut wr, &m).await?,
                None => return Ok(()),
            },
            Some(()) = dirty.recv() => {
                let states = std::mem::take(&mut *pending.lock());
                // Respostas que chegaram enquanto isso (noutra thread) saem antes:
                // a resposta de um `subscribe` nunca fica atrás de um estado novo.
                while let Ok(m) = resp.try_recv() {
                    write_message(&mut wr, &m).await?;
                }
                for state in states.into_values() {
                    let n = Notification {
                        method: STATE.to_string(),
                        params: Some(serde_json::to_value(state).unwrap_or(Value::Null)),
                    };
                    write_message(&mut wr, &Message::Notification(n)).await?;
                }
            }
        }
    }
}

/// O `id` de uma mensagem inválida, se der para lê-lo (inteiro ou string).
fn readable_id(line: &str) -> Option<Id> {
    match serde_json::from_str::<Value>(line).ok()?.get("id")? {
        Value::Number(n) => n.as_i64().map(Id::Num),
        Value::String(s) => Some(Id::Str(s.clone())),
        _ => None,
    }
}

/// Transforma um pânico do método do app em `-32603`.
struct CatchUnwind(BoxFuture);

impl Future for CatchUnwind {
    type Output = Result<Value, RpcError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match std::panic::catch_unwind(AssertUnwindSafe(|| self.0.as_mut().poll(cx))) {
            Ok(poll) => poll,
            Err(_) => Poll::Ready(Err(RpcError::internal())),
        }
    }
}
