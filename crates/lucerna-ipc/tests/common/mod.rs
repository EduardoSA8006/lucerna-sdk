//! Ajudantes dos testes do servidor (a Tarefa 6 reusa com `mod common;`).
#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use lucerna_ipc::server::ServerBuilder;
use lucerna_protocol::RpcError;
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::net::UnixListener;
use tokio::task::JoinHandle;

/// Faz `bind` em `<tmp>/t.sock` e roda `serve` numa tarefa.
pub async fn start(b: ServerBuilder) -> (TempDir, PathBuf, JoinHandle<()>) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let server = b.build();
    let handle = tokio::spawn(async move {
        server.serve(listener).await.unwrap();
    });
    (dir, path, handle)
}

/// Builder do app `demo`: tópico `counter`, stream `ticks` e os métodos dos testes.
pub fn demo() -> ServerBuilder {
    let mut b = ServerBuilder::new("demo", "0.1.0");
    let counter = b.topic("counter", json!({"value": 0}));
    b.stream("ticks");
    let value = Arc::new(Mutex::new(0i64));
    b.method("counter.add", move |params: Option<Value>| {
        let counter = counter.clone();
        let value = value.clone();
        async move {
            let n = params
                .as_ref()
                .and_then(|p| p.get("n"))
                .and_then(Value::as_i64)
                .ok_or_else(|| RpcError::invalid_params("n deve ser um inteiro"))?;
            let mut v = value.lock().unwrap();
            *v += n;
            counter.publish(json!({"value": *v}));
            Ok(json!({"value": *v}))
        }
    });
    b.method("slow.op", |_| async {
        tokio::time::sleep(Duration::from_millis(200)).await;
        Ok(json!({}))
    });
    b.method("boom", |_| async { explode() });
    b.method("app.fail", |_| async { Err(RpcError::app(1, "Falhou.")) });
    b
}

fn explode() -> Result<Value, RpcError> {
    panic!("boom de propósito")
}

/// Monta um pedido JSON-RPC.
pub fn req(id: i64, method: &str, params: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
}
