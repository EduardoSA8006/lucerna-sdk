mod common;

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use common::{demo, start};
use lucerna_ipc::client::{Client, ClientError};
use lucerna_ipc::{LineReader, MAX_MESSAGE};
use lucerna_protocol::codes;
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;
use tokio::net::UnixListener;
use tokio::task::JoinSet;

#[tokio::test]
async fn connect_call_subscribe_and_follow() {
    let (_t, p, _s) = start(demo()).await;
    let mut c = Client::connect(&p, "tui", "demo").await.unwrap();
    assert_eq!(c.hello().topics, vec!["counter"]);
    let snap = c.subscribe(&["counter"]).await.unwrap();
    assert_eq!(snap[0].data, json!({"value":0}));
    assert_eq!(
        c.call("counter.add", Some(json!({"n":2}))).await.unwrap(),
        json!({"value":2})
    );
    let s = c.next_state().await.unwrap();
    assert_eq!(
        (s.topic.as_str(), s.seq, s.data.clone()),
        ("counter", 1, json!({"value":2}))
    );
}

#[tokio::test]
async fn concurrent_calls_get_their_own_answers() {
    let (_t, p, _s) = start(demo()).await;
    let c = Arc::new(Client::connect(&p, "tui", "demo").await.unwrap());
    let mut set = JoinSet::new();
    for _ in 0..20 {
        let c = c.clone();
        set.spawn(async move { c.call("counter.add", Some(json!({"n": 1}))).await });
    }
    let mut got = BTreeSet::new();
    while let Some(r) = set.join_next().await {
        let v = r.unwrap().unwrap();
        got.insert(v["value"].as_i64().unwrap());
    }
    assert_eq!(got, (1..=20).collect::<BTreeSet<_>>());
}

#[tokio::test]
async fn rpc_errors_and_missing_socket() {
    let (t, p, _s) = start(demo()).await;
    let c = Client::connect(&p, "tui", "demo").await.unwrap();
    match c.call("app.fail", None).await {
        Err(ClientError::Rpc(e)) => assert_eq!(e.code, 1),
        r => panic!("esperava Rpc(1), veio {r:?}"),
    }
    match Client::connect(&t.path().join("nada.sock"), "tui", "demo").await {
        Err(ClientError::Connect(_)) => {}
        r => panic!("esperava Connect, veio {:?}", r.map(|_| ())),
    }
    match Client::connect(&p, "tui", "outro").await {
        Err(ClientError::Rpc(e)) => assert_eq!(e.code, codes::INVALID_PARAMS),
        r => panic!("esperava Rpc(-32602), veio {:?}", r.map(|_| ())),
    }
}

#[tokio::test]
async fn closed_resolves_when_server_goes_away() {
    let (_t, p, s) = start(demo()).await;
    let mut c = Client::connect(&p, "tui", "demo").await.unwrap();
    s.abort();
    tokio::time::timeout(Duration::from_secs(1), c.closed())
        .await
        .expect("closed() não voltou em 1 s");
    assert!(c.next_state().await.is_none());
    match c.call("counter.add", Some(json!({"n": 1}))).await {
        Err(ClientError::Closed | ClientError::Io(_)) => {}
        r => panic!("esperava Closed ou Io, veio {r:?}"),
    }
}

/// Pouco acima do limite, o `-32000` chega pela leitura; com 8 MiB a escrita
/// dá `EPIPE` antes, e o cliente ainda devolve o `-32000` que ficou para ler.
#[tokio::test]
async fn oversized_call_reports_message_too_large() {
    let (_t, p, _s) = start(demo()).await;
    for size in [MAX_MESSAGE + 10, 8 * MAX_MESSAGE] {
        let c = Client::connect(&p, "tui", "demo").await.unwrap();
        let big = "x".repeat(size);
        match c.call("counter.add", Some(json!({"n": big}))).await {
            Err(ClientError::Rpc(e)) => assert_eq!(e.code, codes::MESSAGE_TOO_LARGE),
            r => panic!("{size} bytes: esperava Rpc(-32000), veio {r:?}"),
        }
    }
}

/// Servidor falso: responde ao hello e ao subscribe (com `seq` 5) e depois
/// manda `lucerna.state` com `seq` 4 e 6.
#[tokio::test]
async fn out_of_order_seq_is_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("f.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let fake = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (rd, mut wr) = stream.into_split();
        let mut lines = LineReader::new(rd, MAX_MESSAGE);
        let mut send = async |v: Value| {
            let mut b = v.to_string().into_bytes();
            b.push(b'\n');
            wr.write_all(&b).await.unwrap();
        };
        let hello: Value =
            serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        assert_eq!(hello["method"], "lucerna.hello");
        send(json!({"jsonrpc":"2.0","id":hello["id"],"result":{
            "protocol":"1.0","app":"demo","version":"0.1.0",
            "topics":["counter"],"streams":[],"methods":[]}}))
        .await;
        let sub: Value = serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        assert_eq!(sub["method"], "lucerna.subscribe");
        send(json!({"jsonrpc":"2.0","id":sub["id"],"result":{"states":[
            {"topic":"counter","seq":5,"data":{"value":5}}]}}))
        .await;
        for seq in [4, 5, 6] {
            send(json!({"jsonrpc":"2.0","method":"lucerna.state",
                "params":{"topic":"counter","seq":seq,"data":{"value":seq}}}))
            .await;
        }
        // Segura a conexão até o cliente fechar.
        while let Ok(Some(_)) = lines.next_line().await {}
    });
    let mut c = Client::connect(&path, "tui", "demo").await.unwrap();
    let snap = c.subscribe(&["counter"]).await.unwrap();
    assert_eq!(snap[0].seq, 5);
    let s = c.next_state().await.unwrap();
    assert_eq!((s.seq, s.data), (6, json!({"value": 6})));
    drop(c);
    fake.await.unwrap();
}
