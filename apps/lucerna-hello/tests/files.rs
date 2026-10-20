//! Os arquivos que acompanham o app: manifesto e unidades do systemd.

use lucerna_manifest::Manifest;

#[test]
fn manifest_is_valid_and_has_no_panel() {
    let m = Manifest::from_json(include_str!("../data/hello.json")).unwrap();
    m.validate().unwrap();
    assert_eq!(
        (m.app.id.as_str(), m.app.socket.as_str()),
        ("hello", "hello.sock")
    );
    assert!(m.panel.is_none() && m.card.is_none() && m.app.tui.is_none());
}

#[test]
fn units_match_the_paths_rules() {
    let s = include_str!("../systemd/lucerna-hello.socket");
    assert!(
        s.contains("ListenStream=%t/lucerna/hello.sock\n") && s.contains("DirectoryMode=0700\n")
    );
    let svc = include_str!("../systemd/lucerna-hello.service");
    assert!(
        svc.lines()
            .any(|l| l.starts_with("ExecStart=") && l.ends_with(" serve"))
    );
}
