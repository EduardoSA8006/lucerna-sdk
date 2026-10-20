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

/// Serializa `v` num [`serde_json::Value`], para os `params` e os `result`.
///
/// Os tipos deste crate sempre serializam; um tipo que falhe (um mapa com
/// chave que não é texto, por exemplo) vira `null`, em vez de um erro que
/// ninguém tem como tratar no meio de uma mensagem.
pub fn to_value<T: serde::Serialize + ?Sized>(v: &T) -> serde_json::Value {
    serde_json::to_value(v).unwrap_or(serde_json::Value::Null)
}
