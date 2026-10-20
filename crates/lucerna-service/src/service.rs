//! O serviço: o servidor do protocolo, a ativação e a saída por ociosidade.

use std::path::PathBuf;
use std::time::Duration;

use lucerna_ipc::ServerBuilder;
use lucerna_paths::{AppPaths, Env, shell_socket};
use tokio::signal::unix::{SignalKind, signal};

use crate::ServiceError;
use crate::activation::listener_from_env;
use crate::idle::{WorkTracker, wait_idle};
use crate::shell::Shell;

/// Ociosidade padrão: 60 s sem clientes e sem trabalho pendente.
pub const DEFAULT_IDLE: Duration = Duration::from_secs(60);

/// Variável que sobrepõe a ociosidade, em milissegundos (desenvolvimento e testes).
const IDLE_ENV: &str = "LUCERNA_IDLE_TIMEOUT_MS";

/// Monta e roda o serviço de um app.
pub struct ServiceBuilder {
    app: String,
    server: ServerBuilder,
    idle: Duration,
    work: WorkTracker,
    shell: Shell,
}

impl ServiceBuilder {
    /// Começa o serviço do app `app`, na versão `version`.
    pub fn new(app: &str, version: &str) -> Self {
        let work = WorkTracker::new();
        ServiceBuilder {
            app: app.to_string(),
            server: ServerBuilder::new(app, version),
            idle: DEFAULT_IDLE,
            shell: Shell::new(app, work.clone()),
            work,
        }
    }

    /// O servidor do protocolo, para declarar tópicos e métodos.
    pub fn server(&mut self) -> &mut ServerBuilder {
        &mut self.server
    }

    /// Quanto tempo sem clientes e sem trabalho até o serviço sair.
    ///
    /// `LUCERNA_IDLE_TIMEOUT_MS`, se definida, vence este valor em [`run`](Self::run).
    pub fn idle_timeout(&mut self, d: Duration) {
        self.idle = d;
    }

    /// O contador de trabalho pendente do serviço.
    pub fn work(&self) -> WorkTracker {
        self.work.clone()
    }

    /// Os pedidos ao shell (barra, não perturbe, TUI, painel).
    ///
    /// Os itens da barra e o não perturbe pedido contam como trabalho pendente
    /// em [`work`](Self::work).
    /// A conexão ao `shell.sock` existe enquanto o serviço roda.
    pub fn shell(&self) -> Shell {
        self.shell.clone()
    }

    /// Roda o serviço com o ambiente do processo.
    ///
    /// Usa o socket do systemd ou cria um à mão, e sai em `Ok(())` por
    /// ociosidade ou com `SIGTERM`/`SIGINT`.
    pub async fn run(mut self) -> Result<(), ServiceError> {
        let env = Env::from_process();
        if let Some(ms) = std::env::var(IDLE_ENV)
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
        {
            self.idle = Duration::from_millis(ms);
        }
        let paths = AppPaths::new(&self.app, &env)?;
        let (listener, activation) = listener_from_env(&paths)?;
        let shell = shell_socket(&env).ok();
        // Os sinais são registrados só aqui: o `run_with` não mexe no
        // tratamento de SIGTERM/SIGINT do processo.
        let mut term = signal(SignalKind::terminate())?;
        let mut int = signal(SignalKind::interrupt())?;
        let stop = async move {
            tokio::select! {
                _ = term.recv() => {}
                _ = int.recv() => {}
            }
        };
        let r = self.serve_until(listener, shell, stop).await;
        drop(activation); // o socket à mão some só depois de parar de atender
        r
    }

    /// Roda o serviço em `listener` até a ociosidade.
    ///
    /// Não trata sinais (isso é do [`run`](Self::run)); para parar antes, largue
    /// o future. Com `shell_socket`, mantém a conexão ao shell (ver
    /// [`shell`](Self::shell)) até sair.
    pub async fn run_with(
        self,
        listener: tokio::net::UnixListener,
        shell_socket: Option<PathBuf>,
    ) -> Result<(), ServiceError> {
        self.serve_until(listener, shell_socket, std::future::pending())
            .await
    }

    async fn serve_until(
        self,
        listener: tokio::net::UnixListener,
        shell_socket: Option<PathBuf>,
        stop: impl Future<Output = ()>,
    ) -> Result<(), ServiceError> {
        // A conexão ao shell fecha quando o serviço para (o guarda cai aqui).
        let _shell_task = shell_socket.map(|p| self.shell.spawn(p));
        let server = self.server.build();
        let idle = wait_idle(server.connections(), self.work.watch(), self.idle);
        tokio::select! {
            // `biased`: um cliente que já está na fila do `accept` é aceito (e
            // contado) antes de a ociosidade ser conferida no mesmo despertar.
            biased;
            r = server.serve(listener) => r.map_err(ServiceError::Io),
            _ = idle => Ok(()),
            _ = stop => Ok(()),
        }
    }
}
