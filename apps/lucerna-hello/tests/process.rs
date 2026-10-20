//! O binário `lucerna-hello` de verdade: `serve` e os subcomandos em processos.
//!
//! Cada teste usa a própria pasta temporária como `HOME` e `XDG_RUNTIME_DIR`;
//! nenhum processo sobra, nem quando um teste entra em pânico.

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

/// O binário com um ambiente limpo: só `HOME`, `XDG_RUNTIME_DIR` e `PATH`.
fn hello(dir: &Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_lucerna-hello"));
    c.env_clear()
        .env("HOME", dir)
        .env("XDG_RUNTIME_DIR", dir.join("run"))
        .env("PATH", std::env::var_os("PATH").unwrap_or_default());
    c
}

/// Um processo filho que é morto e esperado ao cair.
struct Serve(Child);

impl Serve {
    fn child(&mut self) -> &mut Child {
        &mut self.0
    }
}

impl Drop for Serve {
    fn drop(&mut self) {
        if let Ok(None) = self.0.try_wait() {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}

/// Sobe `lucerna-hello serve` com a ociosidade `idle_ms` e espera o socket.
fn serve(dir: &Path, idle_ms: u64) -> Serve {
    std::fs::create_dir_all(dir.join("run")).unwrap();
    let mut srv = Serve(
        hello(dir)
            .arg("serve")
            .env("LUCERNA_IDLE_TIMEOUT_MS", idle_ms.to_string())
            .stdout(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let sock = dir.join("run/lucerna/hello.sock");
    let start = Instant::now();
    while !sock.exists() {
        assert!(
            srv.child().try_wait().unwrap().is_none(),
            "o serve saiu antes de criar o socket"
        );
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "o socket não apareceu"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    srv
}

/// Espera o processo sair, até `limite`.
fn wait_exit(child: &mut Child, limite: Duration) -> Option<ExitStatus> {
    let start = Instant::now();
    loop {
        if let Some(s) = child.try_wait().unwrap() {
            return Some(s);
        }
        if start.elapsed() >= limite {
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn cli_json_and_exit_codes() {
    let t = tempfile::tempdir().unwrap();
    let out = hello(t.path()).args(["status"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2)); // ainda sem serviço
    assert!(
        String::from_utf8_lossy(&out.stderr).starts_with("Sem conexão com o serviço hello"),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let _srv = serve(t.path(), 60_000);
    let out = hello(t.path()).args(["add", "3"]).output().unwrap();
    assert_eq!(
        (out.status.code(), String::from_utf8(out.stdout).unwrap()),
        (Some(0), "Contador: 3\n".into())
    );
    let out = hello(t.path()).args(["--json", "status"]).output().unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&out.stdout).unwrap(),
        json!({"counter":{"value":3}})
    );
    let big = i64::MAX.to_string();
    hello(t.path()).args(["add", &big]).output().unwrap();
    let out = hello(t.path()).args(["add", &big]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(out.stderr).unwrap(),
        "erro: O contador passou do limite.\n"
    );
    assert_eq!(
        hello(t.path())
            .args(["add", "três"])
            .output()
            .unwrap()
            .status
            .code(),
        Some(64)
    );
    // Sem TUI: a ajuda, com 0.
    let out = hello(t.path()).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&out.stdout).contains("add"));
    let out = hello(t.path()).arg("--version").output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&out.stdout).contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn watch_prints_json_lines() {
    let t = tempfile::tempdir().unwrap();
    let _srv = serve(t.path(), 60_000);
    let mut watch = Serve(
        hello(t.path())
            .args(["--json", "watch", "counter"])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let stdout = watch.child().stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    // A leitura fica numa thread: o `recv_timeout` impede o teste de travar.
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if tx.send(line.unwrap()).is_err() {
                break;
            }
        }
    });
    let line = |rx: &mpsc::Receiver<String>| -> Value {
        serde_json::from_str(&rx.recv_timeout(Duration::from_secs(5)).unwrap()).unwrap()
    };
    assert_eq!(
        line(&rx),
        json!({"topic":"counter","seq":0,"data":{"value":0}})
    );
    let out = hello(t.path()).args(["add", "2"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        line(&rx),
        json!({"topic":"counter","seq":1,"data":{"value":2}})
    );
}

#[test]
fn idle_exit() {
    let t = tempfile::tempdir().unwrap();
    let mut srv = serve(t.path(), 200);
    let watch = Serve(
        hello(t.path())
            .args(["watch", "counter"])
            .stdout(Stdio::null())
            .spawn()
            .unwrap(),
    );
    std::thread::sleep(Duration::from_millis(500)); // > 2× a ociosidade
    assert!(
        srv.child().try_wait().unwrap().is_none(),
        "com cliente conectado não sai"
    );
    drop(watch); // kill + wait
    assert!(
        wait_exit(srv.child(), Duration::from_secs(2))
            .expect("o serviço não saiu")
            .success(),
        "sai sozinho e com 0"
    );
    assert!(
        !t.path().join("run/lucerna/hello.sock").exists(),
        "o socket manual é apagado"
    );
}

#[test]
fn bar_item_holds_the_service_even_without_shell() {
    let t = tempfile::tempdir().unwrap();
    let mut srv = serve(t.path(), 200);
    let out = hello(t.path()).args(["add", "1"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    std::thread::sleep(Duration::from_millis(500));
    assert!(
        srv.child().try_wait().unwrap().is_none(),
        "o item na barra segura o serviço"
    );
    let out = hello(t.path()).args(["reset"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert!(
        wait_exit(srv.child(), Duration::from_secs(2))
            .expect("o serviço não saiu depois do reset")
            .success()
    );
}

#[test]
fn second_serve_is_refused() {
    let t = tempfile::tempdir().unwrap();
    let mut srv = serve(t.path(), 60_000);
    let out = hello(t.path()).arg("serve").output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("Já há um serviço atendendo em"),
        "stderr: {err}"
    );
    assert!(
        srv.child().try_wait().unwrap().is_none(),
        "o primeiro continua"
    );
}
