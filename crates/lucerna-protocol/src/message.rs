//! Mensagens JSON-RPC 2.0.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::RpcError;

/// Identificador de um pedido: inteiro ou string.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Id {
    /// Inteiro.
    Num(i64),
    /// String.
    Str(String),
}

/// Pedido (tem `id`, espera resposta).
#[derive(Clone, PartialEq, Debug)]
pub struct Request {
    /// Id do pedido.
    pub id: Id,
    /// Método.
    pub method: String,
    /// Parâmetros (objeto ou lista).
    pub params: Option<Value>,
}

/// Notificação (sem `id`, nunca tem resposta).
#[derive(Clone, PartialEq, Debug)]
pub struct Notification {
    /// Método.
    pub method: String,
    /// Parâmetros (objeto ou lista).
    pub params: Option<Value>,
}

/// Resposta a um pedido.
#[derive(Clone, PartialEq, Debug)]
pub struct Response {
    /// Id do pedido; `None` sai como `null`.
    pub id: Option<Id>,
    /// Resultado ou erro.
    pub outcome: Result<Value, RpcError>,
}

/// Uma mensagem do protocolo.
#[derive(Clone, PartialEq, Debug)]
pub enum Message {
    /// Pedido.
    Request(Request),
    /// Notificação.
    Notification(Notification),
    /// Resposta.
    Response(Response),
}

impl Message {
    /// Lê uma linha. `-32700` se não for JSON, `-32600` se não for uma mensagem válida.
    pub fn parse(line: &str) -> Result<Message, RpcError> {
        let value: Value = serde_json::from_str(line).map_err(|_| RpcError::parse_error())?;
        let Value::Object(mut obj) = value else {
            return Err(RpcError::invalid_request());
        };
        if obj.get("jsonrpc") != Some(&Value::String("2.0".into())) {
            return Err(RpcError::invalid_request());
        }
        let id = match obj.remove("id") {
            None => None,
            Some(Value::Null) => Some(None),
            Some(v) => Some(Some(
                serde_json::from_value::<Id>(v).map_err(|_| RpcError::invalid_request())?,
            )),
        };

        if let Some(method) = obj.remove("method") {
            let Value::String(method) = method else {
                return Err(RpcError::invalid_request());
            };
            if obj.contains_key("result") || obj.contains_key("error") {
                return Err(RpcError::invalid_request());
            }
            let params = match obj.remove("params") {
                None => None,
                Some(p @ (Value::Object(_) | Value::Array(_))) => Some(p),
                Some(_) => return Err(RpcError::invalid_request()),
            };
            return match id {
                None => Ok(Message::Notification(Notification { method, params })),
                Some(None) => Err(RpcError::invalid_request()),
                Some(Some(id)) => Ok(Message::Request(Request { id, method, params })),
            };
        }

        let Some(id) = id else {
            return Err(RpcError::invalid_request());
        };
        let outcome = match (obj.remove("result"), obj.remove("error")) {
            (Some(result), None) => {
                if id.is_none() {
                    return Err(RpcError::invalid_request());
                }
                Ok(result)
            }
            (None, Some(error)) => Err(serde_json::from_value::<RpcError>(error)
                .map_err(|_| RpcError::invalid_request())?),
            _ => return Err(RpcError::invalid_request()),
        };
        Ok(Message::Response(Response { id, outcome }))
    }

    /// Serializa em uma linha, sem `\n`.
    pub fn to_json(&self) -> String {
        let mut m = Map::new();
        m.insert("jsonrpc".into(), "2.0".into());
        let id_value = |id: &Id| serde_json::to_value(id).unwrap_or(Value::Null);
        match self {
            Message::Request(r) => {
                m.insert("id".into(), id_value(&r.id));
                m.insert("method".into(), r.method.clone().into());
                if let Some(p) = &r.params {
                    m.insert("params".into(), p.clone());
                }
            }
            Message::Notification(n) => {
                m.insert("method".into(), n.method.clone().into());
                if let Some(p) = &n.params {
                    m.insert("params".into(), p.clone());
                }
            }
            Message::Response(r) => {
                m.insert("id".into(), r.id.as_ref().map_or(Value::Null, id_value));
                match &r.outcome {
                    Ok(v) => m.insert("result".into(), v.clone()),
                    Err(e) => m.insert(
                        "error".into(),
                        serde_json::to_value(e).unwrap_or(Value::Null),
                    ),
                };
            }
        }
        Value::Object(m).to_string()
    }
}
