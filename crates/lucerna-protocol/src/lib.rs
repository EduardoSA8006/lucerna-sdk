#![warn(missing_docs)]
//! Tipos das mensagens do protocolo do Lucerna (JSON-RPC 2.0), sem I/O.
//!
//! Este crate fala o protocolo 1.0.

mod error;
mod message;
pub mod methods;
pub mod params;
mod version;

pub use error::{RpcError, codes};
pub use message::{Id, Message, Notification, Request, Response};
pub use methods::is_reserved;
pub use params::*;
pub use version::{PROTOCOL_VERSION, Version, VersionError};
