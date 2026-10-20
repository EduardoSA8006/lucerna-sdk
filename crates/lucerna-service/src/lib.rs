#![warn(missing_docs)]
//! Esqueleto do serviço de um app do Lucerna.
//!
//! Compõe o servidor do [`lucerna_ipc`] com a ativação pelo systemd (ou o socket
//! criado à mão, no desenvolvimento) e a saída por ociosidade.

pub mod activation;
pub mod idle;
pub mod service;

use std::io;
use std::path::PathBuf;

use lucerna_paths::PathsError;

pub use activation::{Activation, SocketGuard};
pub use idle::{WorkGuard, WorkTracker};
pub use service::{DEFAULT_IDLE, ServiceBuilder};

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
