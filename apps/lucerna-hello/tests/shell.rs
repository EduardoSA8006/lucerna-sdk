//! O que o `lucerna-hello` manda ao shell, com o `FakeShell`.

mod common;

use common::start;
use lucerna_service::testing::FakeShell;
use serde_json::json;

#[tokio::test]
async fn bar_and_dnd_undone_on_close_and_reapplied() {
    let t = tempfile::tempdir().unwrap();
    let sp = t.path().join("shell.sock");
    let mut fake = FakeShell::start(&sp).await;
    let r = start(Some(sp.clone())).await;
    let c = r.client().await;
    c.call("counter.add", Some(json!({"n": 3}))).await.unwrap();
    assert_eq!(
        fake.next_call().await,
        (
            "bar.set".into(),
            json!({"id":"counter","icon":"exposure_plus_1","text":"3","panel":false})
        )
    );
    c.call("quiet.set", Some(json!({"on": true})))
        .await
        .unwrap();
    assert_eq!(
        fake.next_call().await,
        ("dnd.request".into(), json!({"reason":"Lucerna Hello"}))
    );
    fake.stop().await; // shell cai: ele desfaria barra e não perturbe
    let mut fake = FakeShell::start(&sp).await; // shell volta
    let mut got = [fake.next_call().await, fake.next_call().await];
    got.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(got[0].0, "bar.set");
    assert_eq!(got[0].1["text"], "3");
    assert_eq!(got[1].0, "dnd.request");
    // Zerar tira o item; desligar devolve o não perturbe.
    c.call("counter.reset", None).await.unwrap();
    assert_eq!(
        fake.next_call().await,
        ("bar.clear".into(), json!({"id":"counter"}))
    );
    c.call("quiet.set", Some(json!({"on": false})))
        .await
        .unwrap();
    assert_eq!(fake.next_call().await.0, "dnd.release");
    r.stop().await; // serviço sai: o shell vê a conexão fechar
    fake.wait_disconnected().await;
}

#[tokio::test]
async fn works_without_a_shell() {
    let t = tempfile::tempdir().unwrap();
    let r = start(Some(t.path().join("nao-existe.sock"))).await;
    let c = r.client().await;
    let v = c.call("counter.add", Some(json!({"n": 1}))).await.unwrap();
    assert_eq!(v, json!({"value": 1}));
    let v = c
        .call("quiet.set", Some(json!({"on": true})))
        .await
        .unwrap();
    assert_eq!(v, json!({}));
}
