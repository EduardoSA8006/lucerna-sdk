mod common;

use std::time::Duration;

use common::{demo, req, start};
use lucerna_ipc::raw::RawConn;
use lucerna_ipc::server::ServerBuilder;
use lucerna_ipc::{LineReader, MAX_MESSAGE, write_message};
use lucerna_protocol::{Id, Message, Request};
use serde_json::{Value, json};

const QUIET: Duration = Duration::from_millis(300);

async fn ready(p: &std::path::Path) -> RawConn {
    let mut c = RawConn::connect(p).await.unwrap();
    let r = c.hello("demo").await.unwrap();
    assert!(r.get("result").is_some(), "hello falhou: {r}");
    c
}

async fn next(c: &mut RawConn) -> Value {
    c.recv().await.unwrap().expect("conexão fechou")
}

/// Nada chega em 300 ms.
async fn assert_silent(c: &mut RawConn) {
    if let Ok(r) = tokio::time::timeout(QUIET, c.recv()).await {
        panic!("esperava silêncio, chegou {r:?}");
    }
}

#[tokio::test]
async fn hello_answers_with_topics_streams_and_methods() {
    let (_t, p, _s) = start(demo()).await;
    let mut c = RawConn::connect(&p).await.unwrap();
    let r = c.hello("demo").await.unwrap();
    assert_eq!(r["id"], 0);
    assert_eq!(r["result"]["protocol"], "1.0");
    assert_eq!(r["result"]["app"], "demo");
    assert_eq!(r["result"]["version"], "0.1.0");
    assert_eq!(r["result"]["topics"], json!(["counter"]));
    assert_eq!(r["result"]["streams"], json!(["ticks"]));
    assert!(
        r["result"]["methods"]
            .as_array()
            .unwrap()
            .contains(&json!("counter.add"))
    );
}

#[tokio::test]
async fn incompatible_major_gets_32010_and_closes() {
    let (_t, p, _s) = start(demo()).await;
    let mut c = RawConn::connect(&p).await.unwrap();
    c.send(&json!({"jsonrpc":"2.0","id":1,"method":"lucerna.hello","params":{"protocol":"2.0","client":"panel","app":"demo"}})).await.unwrap();
    let r = next(&mut c).await;
    assert_eq!(r["id"], 1);
    assert_eq!(r["error"]["code"], -32010);
    assert!(c.recv().await.unwrap().is_none());
}

#[tokio::test]
async fn compatible_minor_is_accepted() {
    let (_t, p, _s) = start(demo()).await;
    let mut c = RawConn::connect(&p).await.unwrap();
    c.send(&req(
        1,
        "lucerna.hello",
        json!({"protocol":"1.7","client":"panel","app":"demo"}),
    ))
    .await
    .unwrap();
    let r = next(&mut c).await;
    assert_eq!(r["result"]["protocol"], "1.0");
}

#[tokio::test]
async fn hello_for_another_app_is_invalid_params() {
    let (_t, p, _s) = start(demo()).await;
    let mut c = RawConn::connect(&p).await.unwrap();
    let r = c.hello("routine").await.unwrap();
    assert_eq!(r["error"]["code"], -32602);
    assert_eq!(
        r["error"]["message"],
        "Parâmetros inválidos: este serviço é o app demo, não routine."
    );
    // a conexão continua
    let r = c.hello("demo").await.unwrap();
    assert_eq!(r["result"]["app"], "demo");
}

#[tokio::test]
async fn anything_before_hello_gets_32011() {
    let (_t, p, _s) = start(demo()).await;
    let mut c = RawConn::connect(&p).await.unwrap();
    c.send(&req(2, "counter.add", json!({"n":1})))
        .await
        .unwrap();
    let r = next(&mut c).await;
    assert_eq!(
        (r["id"].clone(), r["error"]["code"].clone()),
        (json!(2), json!(-32011))
    );
    let r = c.hello("demo").await.unwrap(); // a conexão continua usável
    assert_eq!(r["result"]["app"], "demo");
}

#[tokio::test]
async fn request_response_and_errors() {
    let (_t, p, _s) = start(demo()).await;
    let mut c = ready(&p).await;

    c.send(&req(1, "counter.add", json!({"n":2})))
        .await
        .unwrap();
    let r = next(&mut c).await;
    assert_eq!(r["id"], 1);
    assert_eq!(r["result"], json!({"value":2}));

    c.send(&req(2, "nada.disso", json!({}))).await.unwrap();
    let r = next(&mut c).await;
    assert_eq!(r["id"], 2);
    assert_eq!(r["error"]["code"], -32601);

    c.send(&req(3, "app.fail", json!({}))).await.unwrap();
    let r = next(&mut c).await;
    assert_eq!(r["id"], 3);
    assert_eq!(r["error"], json!({"code":1,"message":"Falhou."}));

    c.send_raw(b"{oops\n").await.unwrap();
    let r = next(&mut c).await;
    assert_eq!(r["id"], Value::Null);
    assert_eq!(r["error"]["code"], -32700);

    // a conexão segue
    c.send(&req(4, "counter.add", json!({"n":1})))
        .await
        .unwrap();
    assert_eq!(next(&mut c).await["result"], json!({"value":3}));
}

#[tokio::test]
async fn subscribe_snapshot_then_changes_with_growing_seq() {
    let (_t, p, _s) = start(demo()).await;
    let mut c = RawConn::connect(&p).await.unwrap();
    c.hello("demo").await.unwrap();
    c.send(&json!({"jsonrpc":"2.0","id":3,"method":"lucerna.subscribe","params":{"topics":["counter"]}})).await.unwrap();
    let r = c.recv().await.unwrap().unwrap();
    assert_eq!(
        r["result"]["states"],
        json!([{"topic":"counter","seq":0,"data":{"value":0}}])
    );
    c.send(&json!({"jsonrpc":"2.0","id":4,"method":"counter.add","params":{"n":5}}))
        .await
        .unwrap();
    let mut got = [
        c.recv().await.unwrap().unwrap(),
        c.recv().await.unwrap().unwrap(),
    ];
    got.sort_by_key(|m| m.get("method").is_some());
    assert_eq!(got[0]["result"], json!({"value":5}));
    assert_eq!(got[1]["method"], "lucerna.state");
    assert_eq!(
        got[1]["params"],
        json!({"topic":"counter","seq":1,"data":{"value":5}})
    );
}

#[tokio::test]
async fn unknown_topic_is_32012_and_nothing_is_subscribed() {
    let (_t, p, _s) = start(demo()).await;
    let mut c = ready(&p).await;
    c.send(&req(
        1,
        "lucerna.subscribe",
        json!({"topics":["counter","x"]}),
    ))
    .await
    .unwrap();
    let r = next(&mut c).await;
    assert_eq!(r["id"], 1);
    assert_eq!(r["error"]["code"], -32012);

    c.send(&req(2, "counter.add", json!({"n":1})))
        .await
        .unwrap();
    let r = next(&mut c).await;
    assert_eq!(r["id"], 2);
    assert_silent(&mut c).await;
}

#[tokio::test]
async fn unsubscribe_stops_updates() {
    let (_t, p, _s) = start(demo()).await;
    let mut c = ready(&p).await;
    c.send(&req(1, "lucerna.subscribe", json!({"topics":["counter"]})))
        .await
        .unwrap();
    assert!(next(&mut c).await["result"]["states"].is_array());
    c.send(&req(
        2,
        "lucerna.unsubscribe",
        json!({"topics":["counter"]}),
    ))
    .await
    .unwrap();
    let r = next(&mut c).await;
    assert_eq!(
        (r["id"].clone(), r["result"].clone()),
        (json!(2), json!({}))
    );

    c.send(&req(3, "counter.add", json!({"n":1})))
        .await
        .unwrap();
    let r = next(&mut c).await;
    assert_eq!(r["id"], 3);
    assert_silent(&mut c).await;
}

#[tokio::test]
async fn two_clients_see_the_same_changes() {
    let (_t, p, _s) = start(demo()).await;
    let mut a = ready(&p).await;
    let mut b = ready(&p).await;
    for c in [&mut a, &mut b] {
        c.send(&req(1, "lucerna.subscribe", json!({"topics":["counter"]})))
            .await
            .unwrap();
        assert_eq!(next(c).await["id"], 1);
    }
    a.send(&req(2, "counter.add", json!({"n":1})))
        .await
        .unwrap();
    let want = json!({"topic":"counter","seq":1,"data":{"value":1}});

    let mut got = [next(&mut a).await, next(&mut a).await];
    got.sort_by_key(|m| m.get("method").is_some());
    assert_eq!(got[0]["result"], json!({"value":1}));
    assert_eq!(got[1]["params"], want);

    let r = next(&mut b).await;
    assert_eq!(r["method"], "lucerna.state");
    assert_eq!(r["params"], want);
}

#[tokio::test]
async fn slow_client_gets_only_the_latest() {
    let mut b = ServerBuilder::new("demo", "0.1.0");
    let ticks = b.stream("ticks");
    let server = b.build();
    let (client, srv) = tokio::io::duplex(256); // buffer minúsculo: o escritor trava logo
    tokio::spawn({
        let s = server.clone();
        async move { s.serve_connection(srv).await }
    });
    let (rd, mut wr) = tokio::io::split(client);
    let mut rd = LineReader::new(rd, MAX_MESSAGE);
    for (id, m, p) in [
        (
            0,
            "lucerna.hello",
            json!({"protocol":"1.0","client":"t","app":"demo"}),
        ),
        (1, "lucerna.subscribe", json!({"topics":["ticks"]})),
    ] {
        write_message(
            &mut wr,
            &Message::Request(Request {
                id: Id::Num(id),
                method: m.into(),
                params: Some(p),
            }),
        )
        .await
        .unwrap();
    }
    ticks.wait_subscribed().await;
    for k in 1..=1000 {
        ticks.publish(json!({ "tick": k }));
        tokio::task::yield_now().await;
    }
    let mut seqs = vec![];
    while let Ok(Ok(Some(line))) =
        tokio::time::timeout(Duration::from_millis(300), rd.next_line()).await
    {
        let v: Value = serde_json::from_str(&line).unwrap();
        if v["method"] == "lucerna.state" {
            seqs.push(v["params"]["seq"].as_u64().unwrap());
        }
    }
    assert_eq!(*seqs.last().unwrap(), 1000, "o último sempre chega");
    assert!(seqs.len() < 20, "coalescido, recebeu {}", seqs.len());
    assert!(seqs.windows(2).all(|w| w[0] < w[1]));
}

#[tokio::test]
async fn stream_counts_subscribers() {
    let mut b = demo();
    let ticks = b.stream("ticks2");
    let (_t, p, _s) = start(b).await;
    assert_eq!(ticks.subscribers(), 0);
    assert_eq!(ticks.current().seq, 0);
    assert_eq!(ticks.current().data, Value::Null);

    let mut c = ready(&p).await;
    c.send(&req(1, "lucerna.subscribe", json!({"topics":["ticks2"]})))
        .await
        .unwrap();
    let r = next(&mut c).await;
    assert_eq!(
        r["result"]["states"],
        json!([{"topic":"ticks2","seq":0,"data":null}])
    );
    tokio::time::timeout(Duration::from_secs(1), ticks.wait_subscribed())
        .await
        .unwrap();
    assert_eq!(ticks.subscribers(), 1);

    c.send(&req(2, "lucerna.unsubscribe", json!({"topics":["ticks2"]})))
        .await
        .unwrap();
    assert_eq!(next(&mut c).await["result"], json!({}));
    tokio::time::timeout(Duration::from_secs(1), ticks.wait_unsubscribed())
        .await
        .unwrap();
    assert_eq!(ticks.subscribers(), 0);
}

#[tokio::test]
async fn abrupt_close_releases_subscriptions() {
    let mut b = demo();
    let ticks = b.stream("ticks2");
    let server = b.build();
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("t.sock");
    let listener = tokio::net::UnixListener::bind(&p).unwrap();
    let mut conns = server.connections();
    tokio::spawn({
        let s = server.clone();
        async move { s.serve(listener).await.unwrap() }
    });

    let mut c = ready(&p).await;
    c.send(&req(1, "lucerna.subscribe", json!({"topics":["ticks2"]})))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), ticks.wait_subscribed())
        .await
        .unwrap();
    c.send(&req(2, "slow.op", json!({}))).await.unwrap();
    drop(c); // larga sem ler

    tokio::time::timeout(Duration::from_secs(1), ticks.wait_unsubscribed())
        .await
        .expect("assinatura não foi devolvida");
    tokio::time::timeout(Duration::from_secs(1), conns.wait_for(|n| *n == 0))
        .await
        .unwrap()
        .unwrap();

    // um cliente novo ainda é atendido
    let mut c = ready(&p).await;
    c.send(&req(3, "counter.add", json!({"n":1})))
        .await
        .unwrap();
    assert_eq!(next(&mut c).await["result"], json!({"value":1}));
}

#[tokio::test]
async fn handler_panic_becomes_internal_error() {
    let (_t, p, _s) = start(demo()).await;
    let mut c = ready(&p).await;
    c.send(&req(1, "boom", json!({}))).await.unwrap();
    let r = next(&mut c).await;
    assert_eq!(r["id"], 1);
    assert_eq!(
        r["error"],
        json!({"code":-32603,"message":"Erro interno do serviço."})
    );
    c.send(&req(2, "counter.add", json!({"n":1})))
        .await
        .unwrap();
    let r = next(&mut c).await;
    assert_eq!(
        (r["id"].clone(), r["result"].clone()),
        (json!(2), json!({"value":1}))
    );
    // outra conexão também
    let mut d = ready(&p).await;
    d.send(&req(3, "counter.add", json!({"n":1})))
        .await
        .unwrap();
    assert_eq!(next(&mut d).await["result"], json!({"value":2}));
}

#[tokio::test]
async fn too_large_message_gets_32000_and_closes() {
    let mut b = demo();
    b.max_message(1000);
    let (_t, p, _s) = start(b).await;
    let mut c = ready(&p).await;
    let mut big = vec![b'x'; 2000];
    big.push(b'\n');
    c.send_raw(&big).await.unwrap();
    let r = next(&mut c).await;
    assert_eq!(r["id"], Value::Null);
    assert_eq!(r["error"]["code"], -32000);
    assert!(c.recv().await.unwrap().is_none());
}

#[tokio::test]
async fn client_notifications_get_no_response() {
    let (_t, p, _s) = start(demo()).await;
    let mut c = ready(&p).await;
    c.send(&json!({"jsonrpc":"2.0","method":"counter.add","params":{"n":4}}))
        .await
        .unwrap();
    c.send(&json!({"jsonrpc":"2.0","method":"nada.disso"}))
        .await
        .unwrap();
    c.send(&req(9, "counter.add", json!({"n":1})))
        .await
        .unwrap();
    let r = next(&mut c).await;
    assert_eq!(
        (r["id"].clone(), r["result"].clone()),
        (json!(9), json!({"value":5}))
    );
    assert_silent(&mut c).await;
}

#[tokio::test]
async fn invalid_request_keeps_the_id_when_readable() {
    let (_t, p, _s) = start(demo()).await;
    let mut c = ready(&p).await;
    c.send(&json!({"jsonrpc":"1.0","id":8,"method":"a.b"}))
        .await
        .unwrap();
    let r = next(&mut c).await;
    assert_eq!(
        (r["id"].clone(), r["error"]["code"].clone()),
        (json!(8), json!(-32600))
    );
    c.send(&json!({"jsonrpc":"1.0","id":"x","method":"a.b"}))
        .await
        .unwrap();
    assert_eq!(next(&mut c).await["id"], "x");
    c.send(&json!([1, 2])).await.unwrap();
    let r = next(&mut c).await;
    assert_eq!(
        (r["id"].clone(), r["error"]["code"].clone()),
        (Value::Null, json!(-32600))
    );
}

#[test]
#[should_panic(expected = "reservado")]
fn reserved_method_names_are_refused() {
    ServerBuilder::new("d", "0").method("lucerna.x", |_| async { Ok(Value::Null) });
}

#[test]
#[should_panic(expected = "repetido")]
fn repeated_topic_names_are_refused() {
    let mut b = ServerBuilder::new("d", "0");
    b.topic("a", Value::Null);
    b.stream("a");
}
