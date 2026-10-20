use std::future::Future;
use std::task::Poll;
use std::time::Duration;

use lucerna_ipc::raw::RawConn;
use lucerna_service::ServiceBuilder;
use serde_json::json;

#[tokio::test]
async fn run_with_serves_then_exits_when_idle() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("s.sock");
    let mut b = ServiceBuilder::new("demo", "0.1.0");
    b.idle_timeout(Duration::from_millis(200));
    b.server().method("ping", |_| async { Ok(json!("pong")) });
    let l = tokio::net::UnixListener::bind(&p).unwrap();
    let h = tokio::spawn(b.run_with(l, None));
    let c = lucerna_ipc::Client::connect(&p, "test", "demo")
        .await
        .unwrap();
    assert_eq!(c.call("ping", None).await.unwrap(), json!("pong"));
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(!h.is_finished(), "não sai com cliente conectado");
    drop(c);
    tokio::time::timeout(Duration::from_secs(2), h)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn pending_work_keeps_run_with_alive() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("s.sock");
    let mut b = ServiceBuilder::new("demo", "0.1.0");
    b.idle_timeout(Duration::from_millis(100));
    let work = b.work();
    let guard = work.hold();
    let l = tokio::net::UnixListener::bind(&p).unwrap();
    let h = tokio::spawn(b.run_with(l, None));
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!h.is_finished(), "não sai com trabalho pendente");
    drop(guard);
    tokio::time::timeout(Duration::from_secs(2), h)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_client_accepted_at_the_deadline_is_served() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("s.sock");
    let mut b = ServiceBuilder::new("demo", "0.1.0");
    b.idle_timeout(Duration::from_secs(60));
    b.server().method("ping", |_| async { Ok(json!("pong")) });
    let l = tokio::net::UnixListener::bind(&p).unwrap();
    let mut run = Box::pin(b.run_with(l, None));

    // Um poll: o prazo da ociosidade começa a contar agora.
    let first = std::future::poll_fn(|cx| Poll::Ready(run.as_mut().poll(cx).is_ready())).await;
    assert!(!first);
    // O prazo vence e, no mesmo instante, um cliente entra na fila do accept.
    tokio::time::advance(Duration::from_secs(60)).await;
    let std_c = std::os::unix::net::UnixStream::connect(&p).unwrap();
    // Deixa o driver de I/O ver o listener pronto antes de o serviço rodar.
    tokio::task::yield_now().await;

    let h = tokio::spawn(run);
    std_c.set_nonblocking(true).unwrap();
    let mut raw = RawConn::from_stream(tokio::net::UnixStream::from_std(std_c).unwrap());
    raw.hello("demo").await.unwrap();
    raw.send(&json!({"jsonrpc": "2.0", "id": 1, "method": "ping"}))
        .await
        .unwrap();
    let resp = raw.recv().await.unwrap().unwrap();
    assert_eq!(resp["result"], json!("pong"));
    assert!(!h.is_finished());
    drop(raw);
    h.await.unwrap().unwrap();
}
