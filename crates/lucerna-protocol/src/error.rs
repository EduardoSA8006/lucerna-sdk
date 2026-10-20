//! Erros do protocolo.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Version;

/// Códigos de erro do protocolo.
pub mod codes {
    /// A linha não é JSON.
    pub const PARSE_ERROR: i32 = -32700;
    /// O JSON não é um pedido válido.
    pub const INVALID_REQUEST: i32 = -32600;
    /// Método inexistente.
    pub const METHOD_NOT_FOUND: i32 = -32601;
    /// Parâmetros inválidos.
    pub const INVALID_PARAMS: i32 = -32602;
    /// Erro interno do serviço.
    pub const INTERNAL_ERROR: i32 = -32603;
    /// Mensagem maior que o limite.
    pub const MESSAGE_TOO_LARGE: i32 = -32000;
    /// Versões do protocolo incompatíveis.
    pub const INCOMPATIBLE_VERSION: i32 = -32010;
    /// Método chamado antes do `lucerna.hello`.
    pub const HELLO_REQUIRED: i32 = -32011;
    /// Tópico desconhecido.
    pub const UNKNOWN_TOPIC: i32 = -32012;
}

/// Erro JSON-RPC, com `message` em português pronta para o usuário.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, thiserror::Error)]
#[error("{message}")]
pub struct RpcError {
    /// Código do erro.
    pub code: i32,
    /// Texto para o usuário.
    pub message: String,
    /// Dados extras opcionais.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl RpcError {
    fn new(code: i32, message: String) -> Self {
        RpcError {
            code,
            message,
            data: None,
        }
    }

    /// `-32700`: a linha não é JSON.
    pub fn parse_error() -> Self {
        Self::new(codes::PARSE_ERROR, "Mensagem inválida: não é JSON.".into())
    }

    /// `-32600`: pedido inválido.
    pub fn invalid_request() -> Self {
        Self::new(codes::INVALID_REQUEST, "Pedido inválido.".into())
    }

    /// `-32601`: método desconhecido.
    pub fn method_not_found(method: &str) -> Self {
        Self::new(
            codes::METHOD_NOT_FOUND,
            format!("Método desconhecido: {method}."),
        )
    }

    /// `-32602`: parâmetros inválidos.
    pub fn invalid_params(detail: &str) -> Self {
        Self::new(
            codes::INVALID_PARAMS,
            format!("Parâmetros inválidos: {detail}."),
        )
    }

    /// `-32603`: erro interno.
    pub fn internal() -> Self {
        Self::new(codes::INTERNAL_ERROR, "Erro interno do serviço.".into())
    }

    /// `-32000`: mensagem grande demais.
    pub fn message_too_large(limit: usize) -> Self {
        Self::new(
            codes::MESSAGE_TOO_LARGE,
            format!("Mensagem grande demais (limite de {limit} bytes)."),
        )
    }

    /// `-32010`: versão do protocolo incompatível.
    pub fn incompatible_version(client: Version, server: Version) -> Self {
        Self::new(
            codes::INCOMPATIBLE_VERSION,
            format!(
                "Versão do protocolo incompatível: o cliente fala {client} e o serviço fala {server}."
            ),
        )
    }

    /// `-32011`: método antes do `lucerna.hello`.
    pub fn hello_required() -> Self {
        Self::new(
            codes::HELLO_REQUIRED,
            "Faça o aperto de mão (lucerna.hello) antes dos outros pedidos.".into(),
        )
    }

    /// `-32012`: tópico desconhecido.
    pub fn unknown_topic(topic: &str) -> Self {
        Self::new(
            codes::UNKNOWN_TOPIC,
            format!("Tópico desconhecido: {topic}."),
        )
    }

    /// Erro do app (`code >= 1`).
    pub fn app(code: i32, message: impl Into<String>) -> Self {
        debug_assert!(code >= 1, "códigos do app começam em 1");
        Self::new(code, message.into())
    }

    /// Anexa dados extras ao erro.
    pub fn with_data(mut self, data: Value) -> Self {
        self.data = Some(data);
        self
    }
}
