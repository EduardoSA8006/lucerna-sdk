#![warn(missing_docs)]
//! Enquadramento por linha, servidor e cliente do protocolo do Lucerna.

pub mod framing;

pub use framing::{FrameError, LineReader, MAX_MESSAGE, write_message};
