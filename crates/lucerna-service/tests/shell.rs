use std::time::Duration;

use lucerna_protocol::BarItem;
use lucerna_service::testing::FakeShell;
use lucerna_service::{ServiceBuilder, Shell, WorkTracker, backoff};
use serde_json::{Value, json};

fn item(text: &str) -> BarItem {
    BarItem {
        id: "c".into(),
        icon: "tag".into(),
        text: Some(text.into()),
        ends_at: None,
        panel: false,
    }
}

#[test]
fn backoff_grows_and_caps() {
    let ms = |a| backoff(a).as_millis();
    assert_eq!(
        (ms(0), ms(1), ms(2), ms(5), ms(6), ms(30)),
        (100, 200, 400, 3200, 5000, 5000)
    );
    assert_eq!(backoff(u32::MAX).as_millis(), 5000);
}

#[tokio::test]
async fn bar_and_dnd_reach_the_shell() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("shell.sock");
    let mut fake = FakeShell::start(&p).await;
    let shell = Shell::new("demo", WorkTracker::new());
    let _task = shell.spawn(p.clone());
    fake.wait_connected().await;
    shell.bar_set(item("3"));
    assert_eq!(
        fake.next_call().await,
        (
            "bar.set".into(),
            json!({"id":"c","icon":"tag","text":"3","panel":false})
        )
    );
    shell.dnd_request("foco");
    assert_eq!(
        fake.next_call().await,
        ("dnd.request".into(), json!({"reason":"foco"}))
    );
    shell.bar_clear("c");
    assert_eq!(
        fake.next_call().await,
        ("bar.clear".into(), json!({"id":"c"}))
    );
    shell.dnd_release();
    assert_eq!(fake.next_call().await.0, "dnd.release");
}

#[tokio::test]
async fn without_shell_calls_are_silent() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("shell.sock"); // ninguém escuta aqui
    let shell = Shell::new("demo", WorkTracker::new());
    let _task = shell.spawn(p);
    shell.bar_set(item("1"));
    shell.dnd_request("foco");
    shell.tui_open(vec!["t_42".into()]);
    shell.panel_open(None);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!shell.is_connected());
    shell.bar_clear("c");
    shell.dnd_release();
    // Sem tarefa de conexão nenhuma, também nada acontece.
    let alone = Shell::new("demo", WorkTracker::new());
    alone.tui_open(vec![]);
    alone.panel_open(Some("hoje".into()));
    assert!(!alone.is_connected());
}

#[tokio::test]
async fn reconnect_reapplies_desired_state() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("shell.sock");
    let mut fake = FakeShell::start(&p).await;
    let shell = Shell::new("demo", WorkTracker::new());
    let _task = shell.spawn(p.clone());
    fake.wait_connected().await;
    shell.bar_set(item("3"));
    assert_eq!(fake.next_call().await.0, "bar.set");
    shell.dnd_request("foco");
    assert_eq!(fake.next_call().await.0, "dnd.request");
    fake.stop().await; // o shell desfaria tudo aqui

    let mut fake = FakeShell::start(&p).await;
    let mut got = vec![fake.next_call().await, fake.next_call().await];
    got.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        got,
        vec![
            (
                "bar.set".to_string(),
                json!({"id":"c","icon":"tag","text":"3","panel":false})
            ),
            ("dnd.request".to_string(), json!({"reason":"foco"})),
        ]
    );
    fake.no_call_within(Duration::from_millis(300)).await;
}

#[tokio::test]
async fn changes_while_offline_collapse_to_the_latest() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("shell.sock");
    let shell = Shell::new("demo", WorkTracker::new());
    let _task = shell.spawn(p.clone());
    shell.bar_set(item("1"));
    shell.bar_set(item("2"));
    shell.bar_set(item("3"));
    shell.bar_set(BarItem {
        id: "x".into(),
        ..item("9")
    });
    shell.bar_clear("x");
    shell.dnd_request("a");
    shell.dnd_release();
    tokio::time::sleep(Duration::from_millis(150)).await;

    let mut fake = FakeShell::start(&p).await;
    assert_eq!(
        fake.next_call().await,
        (
            "bar.set".into(),
            json!({"id":"c","icon":"tag","text":"3","panel":false})
        )
    );
    fake.no_call_within(Duration::from_millis(500)).await;
}

#[tokio::test]
async fn shell_that_goes_away_many_times_gets_only_the_current_state() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("shell.sock");
    let shell = Shell::new("demo", WorkTracker::new());
    let _task = shell.spawn(p.clone());
    for round in 0..3 {
        let mut fake = FakeShell::start(&p).await;
        fake.wait_connected().await;
        let text = round.to_string();
        shell.bar_set(item(&text));
        let (m, params) = fake.next_call().await;
        assert_eq!((m.as_str(), &params["text"]), ("bar.set", &json!(text)));
        fake.stop().await;
        // Fora: muda de novo e tira o não perturbe que nunca chegou.
        shell.bar_set(item(&format!("{text}b")));
        shell.dnd_request("x");
        shell.dnd_release();
    }
    let mut fake = FakeShell::start(&p).await;
    let (m, params) = fake.next_call().await;
    assert_eq!((m.as_str(), &params["text"]), ("bar.set", &json!("2b")));
    fake.no_call_within(Duration::from_millis(300)).await;
}

#[tokio::test]
async fn tui_open_is_dropped_while_offline() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("shell.sock");
    let shell = Shell::new("demo", WorkTracker::new());
    let _task = shell.spawn(p.clone());
    shell.tui_open(vec!["t_42".into()]);
    shell.panel_open(None);
    tokio::time::sleep(Duration::from_millis(150)).await;

    let mut fake = FakeShell::start(&p).await;
    fake.wait_connected().await;
    fake.no_call_within(Duration::from_millis(500)).await;
    assert!(shell.is_connected());
    shell.panel_open(Some("hoje".into()));
    assert_eq!(
        fake.next_call().await,
        ("panel.open".into(), json!({"page":"hoje"}))
    );
    shell.tui_open(vec!["t_42".into()]);
    assert_eq!(
        fake.next_call().await,
        ("tui.open".into(), json!({"args":["t_42"]}))
    );
}

#[tokio::test(start_paused = true)]
async fn bar_items_count_as_pending_work() {
    let w = WorkTracker::new();
    let shell = Shell::new("demo", w.clone()); // sem spawn: o shell nem existe
    shell.bar_set(item("1"));
    assert_eq!(w.pending(), 1);
    shell.bar_set(BarItem {
        id: "d".into(),
        ..item("2")
    });
    assert_eq!(w.pending(), 1);
    shell.bar_clear("c");
    assert_eq!(w.pending(), 1);
    shell.bar_clear("d");
    assert_eq!(w.pending(), 0);
    shell.bar_clear("d"); // tirar o que não está lá não faz nada
    assert_eq!(w.pending(), 0);
}

#[tokio::test]
async fn service_shell_uses_the_builder_work_tracker() {
    let b = ServiceBuilder::new("demo", "0.1.0");
    let shell = b.shell();
    shell.bar_set(item("1"));
    assert_eq!(b.work().pending(), 1);
    shell.bar_clear("c");
    assert_eq!(b.work().pending(), 0);
}

#[tokio::test]
async fn shell_sees_the_connection_close_when_the_service_ends() {
    let t = tempfile::tempdir().unwrap();
    let shell_path = t.path().join("shell.sock");
    let p = t.path().join("demo.sock");
    let mut fake = FakeShell::start(&shell_path).await;
    let mut b = ServiceBuilder::new("demo", "0.1.0");
    b.idle_timeout(Duration::from_millis(200));
    let l = tokio::net::UnixListener::bind(&p).unwrap();
    let h = tokio::spawn(b.run_with(l, Some(shell_path.clone())));
    fake.wait_connected().await;
    // Sem itens na barra: o serviço sai por ociosidade e a conexão fecha.
    tokio::time::timeout(Duration::from_secs(3), h)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    fake.wait_disconnected().await;
    fake.no_call_within(Duration::from_millis(100)).await;
}

#[tokio::test]
async fn hung_shell_times_out_and_reconnects() {
    // Um shell que aceita e nunca responde: o aperto de mão vence o prazo e a
    // tarefa tenta de novo; quando um shell de verdade aparece, o estado chega.
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("shell.sock");
    let hung = tokio::net::UnixListener::bind(&p).unwrap();
    let keep = tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((s, _)) = hung.accept().await {
            held.push(s);
        }
    });
    let shell = Shell::new("demo", WorkTracker::new());
    let _task = shell.spawn(p.clone());
    shell.bar_set(item("7"));
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(!shell.is_connected());
    keep.abort();
    let _ = keep.await;
    std::fs::remove_file(&p).unwrap();
    let mut fake = FakeShell::start(&p).await;
    let (m, params): (String, Value) = fake.next_call().await;
    assert_eq!((m.as_str(), &params["text"]), ("bar.set", &json!("7")));
}

#[tokio::test]
async fn shell_that_stops_answering_counts_as_down() {
    // O shell responde ao aperto de mão e trava no bar.set: o prazo da chamada
    // vence, a tarefa reconecta e reaplica o estado.
    use lucerna_ipc::raw::RawConn;
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("shell.sock");
    let l = tokio::net::UnixListener::bind(&p).unwrap();
    let shell = Shell::new("demo", WorkTracker::new());
    let _task = shell.spawn(p.clone());
    shell.bar_set(item("5"));
    let mut seen = Vec::new();
    let mut conns = Vec::new();
    for _ in 0..2 {
        let (s, _) = tokio::time::timeout(Duration::from_secs(7), l.accept())
            .await
            .unwrap()
            .unwrap();
        let mut raw = RawConn::from_stream(s);
        let hello = raw.recv().await.unwrap().unwrap();
        assert_eq!(hello["method"], "lucerna.hello");
        assert_eq!(hello["params"]["client"], "demo");
        assert_eq!(hello["params"]["app"], "shell");
        raw.send(&json!({"jsonrpc": "2.0", "id": hello["id"], "result": {
            "protocol": "1.0", "app": "shell", "version": "0",
            "topics": [], "streams": [], "methods": []}}))
            .await
            .unwrap();
        let call = raw.recv().await.unwrap().unwrap();
        seen.push((call["method"].clone(), call["params"]["text"].clone()));
        conns.push(raw); // nunca responde
    }
    assert_eq!(seen, vec![(json!("bar.set"), json!("5")); 2]);
}
