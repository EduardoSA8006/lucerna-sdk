#![warn(missing_docs)]
//! Esqueleto do serviço de um app do Lucerna.
//!
//! Compõe o servidor do [`lucerna_ipc`] com a ativação pelo systemd (ou o socket
//! criado à mão, no desenvolvimento), a saída por ociosidade e a conexão ao
//! `shell.sock` ([`Shell`]) e os timers do systemd do usuário ([`schedule`]).
//! [`testing::FakeShell`] e [`testing::FakeScheduler`] servem aos testes dos apps.

pub mod activation;
pub mod idle;
pub mod schedule;
pub mod service;
pub mod shell;
pub mod testing;

use std::io;
use std::path::PathBuf;

use lucerna_paths::PathsError;

/// Tipos do protocolo que os apps usam com o serviço.
pub use lucerna_protocol::{BarItem, RpcError};

pub use activation::{Activation, SocketGuard};
pub use idle::{WorkGuard, WorkTracker};
pub use schedule::{
    ScheduleError, Scheduler, SystemdScheduler, TimerSpec, calendar_utc, render_timer,
    timer_unit_name,
};
pub use service::{DEFAULT_IDLE, ServiceBuilder};
pub use shell::{Shell, ShellTask, backoff};

/// Erros ao subir ou rodar o serviço.
#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    /// Um caminho XDG que falta ou é inválido.
    #[error("{0}")]
    Paths(#[from] PathsError),
    /// Outro serviço já atende no socket.
    #[error("Já há um serviço atendendo em {}.", .0.display())]
    AlreadyRunning(PathBuf),
    /// O que o systemd passou não serve.
    #[error("{0}")]
    Activation(String),
    /// Erro de entrada e saída.
    #[error("{0}")]
    Io(#[from] io::Error),
}
