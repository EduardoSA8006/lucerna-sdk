mod common;

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use common::{demo, start};
use lucerna_ipc::MAX_MESSAGE;
use lucerna_ipc::client::{Client, ClientError};
use lucerna_ipc::raw::RawConn;
use lucerna_protocol::codes;
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::net::UnixListener;
use tokio::task::{JoinHandle, JoinSet};

/// Servidor falso: aceita uma conexão e entrega a `script` como `RawConn`.
fn fake<F, Fut>(script: F) -> (TempDir, std::path::PathBuf, JoinHandle<()>)
where
    F: FnOnce(RawConn) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send,
{
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("f.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let handle = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        script(RawConn::from_stream(stream)).await;
    });
    (dir, path, handle)
}

async fn expect(c: &mut RawConn, method: &str) -> Value {
    let m = c.recv().await.unwrap().expect("o cliente fechou");
    assert_eq!(m["method"], method, "mensagem inesperada: {m}");
    m
}

async fn answer(c: &mut RawConn, req: &Value, result: Value) {
    c.send(&json!({"jsonrpc": "2.0", "id": req["id"], "result": result}))
        .await
        .unwrap();
}

/// Responde ao `hello` como o app `demo` com os tópicos `topics`.
async fn fake_hello(c: &mut RawConn, topics: &[&str]) {
    let h = expect(c, "lucerna.hello").await;
    answer(
        c,
        &h,
        json!({"protocol": "1.0", "app": "demo", "version": "0.1.0",
               "topics": topics, "streams": [], "methods": []}),
    )
    .await;
}

async fn state(c: &mut RawConn, topic: &str, seq: u64) {
    c.send(&json!({"jsonrpc": "2.0", "method": "lucerna.state",
                   "params": {"topic": topic, "seq": seq, "data": {"v": seq}}}))
        .await
        .unwrap();
}

/// Segura a conexão até o cliente fechar (ou 5 s).
async fn hold(mut c: RawConn) {
    while let Ok(Some(_)) = c.recv().await {}
}

#[tokio::test]
async fn connect_call_subscribe_and_follow() {
    let (_t, p, _s) = start(demo()).await;
    let c = Client::connect(&p, "tui", "demo").await.unwrap();
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
async fn one_task_reads_states_while_another_calls() {
    let (_t, p, _s) = start(demo()).await;
    let c = Arc::new(Client::connect(&p, "tui", "demo").await.unwrap());
    c.subscribe(&["counter"]).await.unwrap();
    let reader = {
        let c = c.clone();
        tokio::spawn(async move {
            // Estados podem ser coalescidos: lê até ver o último valor.
            let mut seen = Vec::new();
            while let Some(s) = c.next_state().await {
                let v = s.data["value"].as_i64().unwrap();
                seen.push(v);
                if v == 5 {
                    break;
                }
            }
            seen
        })
    };
    for _ in 0..5 {
        c.call("counter.add", Some(json!({"n": 1}))).await.unwrap();
    }
    let seen = tokio::time::timeout(Duration::from_secs(5), reader)
        .await
        .expect("a leitora não viu o valor 5")
        .unwrap();
    assert_eq!(seen.last(), Some(&5));
    assert!(seen.windows(2).all(|w| w[0] < w[1]), "{seen:?}");
}

#[tokio::test]
async fn every_waiting_reader_sees_the_close() {
    let (_t, p, s) = start(demo()).await;
    let c = Arc::new(Client::connect(&p, "tui", "demo").await.unwrap());
    let mut set = JoinSet::new();
    for _ in 0..3 {
        let c = c.clone();
        set.spawn(async move { c.next_state().await });
    }
    tokio::time::sleep(Duration::from_millis(50)).await; // as três esperando
    s.abort();
    while let Some(r) = tokio::time::timeout(Duration::from_secs(2), set.join_next())
        .await
        .expect("uma leitora não acordou com o fechamento")
    {
        assert!(r.unwrap().is_none());
    }
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
    let c = Client::connect(&p, "tui", "demo").await.unwrap();
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

/// O servidor falso responde ao subscribe com `seq` 5 e depois manda
/// `lucerna.state` com `seq` 4, 5 e 6: só o 6 passa.
#[tokio::test]
async fn out_of_order_seq_is_dropped() {
    let (_d, path, f) = fake(|mut c| async move {
        fake_hello(&mut c, &["counter"]).await;
        let sub = expect(&mut c, "lucerna.subscribe").await;
        answer(
            &mut c,
            &sub,
            json!({"states": [{"topic": "counter", "seq": 5, "data": {"v": 5}}]}),
        )
        .await;
        for seq in [4, 5, 6] {
            state(&mut c, "counter", seq).await;
        }
        hold(c).await;
    });
    let c = Client::connect(&path, "tui", "demo").await.unwrap();
    let snap = c.subscribe(&["counter"]).await.unwrap();
    assert_eq!(snap[0].seq, 5);
    let s = c.next_state().await.unwrap();
    assert_eq!((s.seq, s.data), (6, json!({"v": 6})));
    drop(c);
    f.await.unwrap();
}

/// Um estado por tópico (o mais novo), na ordem em que os tópicos chegaram;
/// o `unsubscribe` tira o tópico do que está pendente.
#[tokio::test]
async fn states_are_coalesced_per_topic_in_arrival_order() {
    let (_d, path, f) = fake(|mut c| async move {
        fake_hello(&mut c, &["a", "b", "c"]).await;
        let sub = expect(&mut c, "lucerna.subscribe").await;
        let zero = |t: &str| json!({"topic": t, "seq": 0, "data": null});
        answer(
            &mut c,
            &sub,
            json!({"states": [zero("a"), zero("b"), zero("c")]}),
        )
        .await;
        state(&mut c, "a", 1).await;
        state(&mut c, "b", 1).await;
        state(&mut c, "a", 2).await;
        state(&mut c, "c", 1).await;
        // A resposta vem depois dos estados: quando o `call` volta, todos chegaram.
        let sync = expect(&mut c, "sync").await;
        answer(&mut c, &sync, json!({})).await;
        let unsub = expect(&mut c, "lucerna.unsubscribe").await;
        answer(&mut c, &unsub, json!({})).await;
        // Fecha a conexão.
    });
    let c = Client::connect(&path, "tui", "demo").await.unwrap();
    c.subscribe(&["a", "b", "c"]).await.unwrap();
    c.call("sync", None).await.unwrap();
    c.unsubscribe(&["c"]).await.unwrap();
    let got: Vec<(String, u64)> = [c.next_state().await, c.next_state().await]
        .into_iter()
        .map(|s| {
            let s = s.unwrap();
            (s.topic, s.seq)
        })
        .collect();
    assert_eq!(got, vec![("a".into(), 2), ("b".into(), 1)]);
    assert!(c.next_state().await.is_none());
    f.await.unwrap();
}

/// Respostas com `id` desconhecido ou string são ignoradas; a certa chega.
#[tokio::test]
async fn unknown_or_string_ids_are_ignored() {
    let (_d, path, f) = fake(|mut c| async move {
        fake_hello(&mut c, &[]).await;
        let r = expect(&mut c, "app.x").await;
        for id in [json!(999), json!("1"), json!("x")] {
            c.send(&json!({"jsonrpc": "2.0", "id": id, "result": "errado"}))
                .await
                .unwrap();
        }
        answer(&mut c, &r, json!("certo")).await;
        hold(c).await;
    });
    let c = Client::connect(&path, "tui", "demo").await.unwrap();
    assert_eq!(c.call("app.x", None).await.unwrap(), json!("certo"));
    drop(c);
    f.await.unwrap();
}

/// Uma resposta que não tem a forma esperada vira `Decode`, não `Io`.
#[tokio::test]
async fn malformed_hello_result_is_decode_error() {
    let (_d, path, f) = fake(|mut c| async move {
        let h = expect(&mut c, "lucerna.hello").await;
        answer(&mut c, &h, json!({"app": 3})).await;
        hold(c).await;
    });
    match Client::connect(&path, "tui", "demo").await {
        Err(ClientError::Decode(_)) => {}
        r => panic!("esperava Decode, veio {:?}", r.map(|_| ())),
    }
    f.await.unwrap();
}

/// Uma chamada em andamento acorda com `Closed` quando o servidor cai.
#[tokio::test]
async fn pending_call_wakes_with_closed_when_server_dies() {
    let mut b = demo();
    // Bem mais longo que a espera do teste: só o fechamento acorda a chamada.
    b.method("hang", |_| async {
        tokio::time::sleep(Duration::from_secs(30)).await;
        Ok(json!({}))
    });
    let (_t, p, s) = start(b).await;
    let c = Arc::new(Client::connect(&p, "tui", "demo").await.unwrap());
    let call = tokio::spawn({
        let c = c.clone();
        async move { c.call("hang", None).await }
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    s.abort();
    let r = tokio::time::timeout(Duration::from_secs(1), call)
        .await
        .expect("a chamada não acordou em 1 s")
        .unwrap();
    assert!(matches!(r, Err(ClientError::Closed)), "veio {r:?}");
}

/// Um servidor que aceita e não responde: `connect_timeout` e `call_timeout`
/// voltam com `Timeout` no prazo.
#[tokio::test]
async fn timeouts_against_silent_server() {
    let (_d, path, f) = fake(hold);
    let t0 = std::time::Instant::now();
    match Client::connect_timeout(&path, "tui", "demo", Duration::from_millis(200)).await {
        Err(ClientError::Timeout) => {}
        r => panic!("esperava Timeout, veio {:?}", r.map(|_| ())),
    }
    assert!(t0.elapsed() < Duration::from_secs(1));
    f.await.unwrap();

    let (_d, path, f) = fake(|mut c| async move {
        fake_hello(&mut c, &[]).await;
        let late = expect(&mut c, "app.x").await;
        // Responde só depois do prazo do cliente.
        tokio::time::sleep(Duration::from_millis(400)).await;
        answer(&mut c, &late, json!("atrasada")).await;
        let r = expect(&mut c, "app.x").await;
        answer(&mut c, &r, json!("certo")).await;
        hold(c).await;
    });
    let c = Client::connect_timeout(&path, "tui", "demo", Duration::from_secs(2))
        .await
        .unwrap();
    let t0 = std::time::Instant::now();
    match c
        .call_timeout("app.x", None, Duration::from_millis(200))
        .await
    {
        Err(ClientError::Timeout) => {}
        r => panic!("esperava Timeout, veio {r:?}"),
    }
    assert!(t0.elapsed() < Duration::from_secs(1));
    // A resposta atrasada é ignorada e a conexão continua usável.
    assert_eq!(
        c.call_timeout("app.x", None, Duration::from_secs(2))
            .await
            .unwrap(),
        json!("certo")
    );
    drop(c);
    f.await.unwrap();
}

/// Ao reassinar, um estado pendente com `seq` até o do retrato novo é
/// descartado: o velho não sai depois do novo.
#[tokio::test]
async fn resubscribe_drops_pending_states_older_than_snapshot() {
    let (_d, path, f) = fake(|mut c| async move {
        fake_hello(&mut c, &["a", "b"]).await;
        let sub = expect(&mut c, "lucerna.subscribe").await;
        let st = |t: &str, seq: u64| json!({"topic": t, "seq": seq, "data": {"v": seq}});
        answer(&mut c, &sub, json!({"states": [st("a", 0), st("b", 0)]})).await;
        state(&mut c, "a", 3).await;
        state(&mut c, "b", 2).await;
        let sync = expect(&mut c, "sync").await;
        answer(&mut c, &sync, json!({})).await;
        let sub = expect(&mut c, "lucerna.subscribe").await;
        answer(&mut c, &sub, json!({"states": [st("a", 5)]})).await;
        // Fecha a conexão.
    });
    let c = Client::connect(&path, "tui", "demo").await.unwrap();
    c.subscribe(&["a", "b"]).await.unwrap();
    c.call("sync", None).await.unwrap();
    let snap = c.subscribe(&["a"]).await.unwrap();
    assert_eq!(snap[0].seq, 5);
    // O `a` 3 saiu; o `b` 2 (outro tópico) fica.
    let s = c.next_state().await.unwrap();
    assert_eq!((s.topic.as_str(), s.seq), ("b", 2));
    assert!(c.next_state().await.is_none());
    f.await.unwrap();
}

/// Com assinatura ativa e ninguém lendo `next_state`, as chamadas não travam:
/// os estados são coalescidos por tópico no cliente.
#[tokio::test]
async fn sequential_calls_without_draining_states_do_not_block() {
    let (_t, p, _s) = start(demo()).await;
    let c = Client::connect(&p, "tui", "demo").await.unwrap();
    c.subscribe(&["counter"]).await.unwrap();
    for i in 0..400 {
        let r = tokio::time::timeout(
            Duration::from_secs(2),
            c.call("counter.add", Some(json!({"n": 1}))),
        )
        .await;
        r.unwrap_or_else(|_| panic!("travou na chamada {i}"))
            .unwrap();
    }
    // O que ficou pendente foi coalescido: poucos estados, em ordem, até o 400.
    let mut got = Vec::new();
    while got.last() != Some(&400) {
        let s = tokio::time::timeout(Duration::from_secs(2), c.next_state())
            .await
            .unwrap()
            .unwrap();
        got.push(s.seq);
    }
    assert!(got.len() <= 3 && got.is_sorted(), "estados: {got:?}");
}
