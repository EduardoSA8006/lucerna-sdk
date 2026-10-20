use std::time::Duration;

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
