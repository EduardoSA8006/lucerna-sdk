use std::os::unix::fs::PermissionsExt;

use lucerna_service::ServiceError;
use lucerna_service::activation::{bind_manual, listen_fds, listener_from_fd};

#[test]
fn listen_fds_rules() {
    assert_eq!(listen_fds(Some("42"), Some("1"), 42), 1);
    assert_eq!(listen_fds(Some("41"), Some("1"), 42), 0);
    assert_eq!(listen_fds(None, Some("1"), 42), 0);
    assert_eq!(listen_fds(Some("42"), Some("x"), 42), 0);
}

#[tokio::test]
async fn listener_from_fd_accepts_connections() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("a.sock");
    let std_l = std::os::unix::net::UnixListener::bind(&p).unwrap();
    let l = tokio::net::UnixListener::from_std(listener_from_fd(std_l.into()).unwrap()).unwrap();
    let c = tokio::net::UnixStream::connect(&p);
    let (acc, con) = tokio::join!(l.accept(), c);
    assert!(acc.is_ok() && con.is_ok());
}

#[test]
fn listener_from_fd_rejects_non_sockets() {
    let f = tempfile::tempfile().unwrap();
    assert!(listener_from_fd(f.into()).is_err());
}

#[tokio::test]
async fn manual_bind_makes_private_dir_and_replaces_stale_socket() {
    let t = tempfile::tempdir().unwrap();
    let dir = t.path().join("run").join("lucerna");
    let p = dir.join("hello.sock");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    // Uma execução à mão que caiu: o arquivo fica, ninguém escuta.
    drop(std::os::unix::net::UnixListener::bind(&p).unwrap());
    assert!(p.exists());

    let (l, guard) = bind_manual(&p).unwrap();
    let mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o700);

    let l = tokio::net::UnixListener::from_std(l).unwrap();
    let (acc, con) = tokio::join!(l.accept(), tokio::net::UnixStream::connect(&p));
    assert!(acc.is_ok() && con.is_ok());

    drop(guard);
    assert!(!p.exists(), "largar o SocketGuard apaga o arquivo");
}

#[tokio::test]
async fn manual_bind_refuses_a_live_service() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("run").join("lucerna").join("hello.sock");
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    let _live = std::os::unix::net::UnixListener::bind(&p).unwrap();

    let err = bind_manual(&p).unwrap_err();
    assert!(matches!(&err, ServiceError::AlreadyRunning(q) if *q == p));
    assert_eq!(
        err.to_string(),
        format!("Já há um serviço atendendo em {}.", p.display())
    );
    assert!(p.exists(), "o socket do serviço vivo continua lá");
}

#[test]
fn manual_bind_does_not_delete_a_regular_file() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("hello.sock");
    std::fs::write(&p, b"nao sou um socket").unwrap();
    assert!(bind_manual(&p).is_err());
    assert!(p.exists());
}
