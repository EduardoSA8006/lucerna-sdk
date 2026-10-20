use lucerna_paths::*;
use std::{os::unix::fs::PermissionsExt, path::PathBuf};

fn env(home: &str) -> Env {
    Env {
        home: Some(home.into()),
        xdg_runtime_dir: Some("/run/user/1000".into()),
        ..Env::default()
    }
}

#[test]
fn xdg_defaults_follow_ecossistema() {
    let p = AppPaths::new("routine", &env("/home/u")).unwrap();
    assert_eq!(
        p.data_dir().unwrap(),
        PathBuf::from("/home/u/.local/share/lucerna/routine")
    );
    assert_eq!(
        p.config_file().unwrap(),
        PathBuf::from("/home/u/.config/lucerna/routine.toml")
    );
    assert_eq!(
        p.cache_dir().unwrap(),
        PathBuf::from("/home/u/.cache/lucerna/routine")
    );
    assert_eq!(
        p.socket().unwrap(),
        PathBuf::from("/run/user/1000/lucerna/routine.sock")
    );
    assert_eq!(
        shell_socket(&env("/h")).unwrap(),
        PathBuf::from("/run/user/1000/lucerna/shell.sock")
    );
}

#[test]
fn xdg_variables_win_and_relative_ones_are_ignored() {
    let mut e = env("/home/u");
    e.xdg_data_home = Some("/dados".into());
    e.xdg_config_home = Some("relativo/config".into());
    let p = AppPaths::new("notes", &e).unwrap();
    assert_eq!(p.data_dir().unwrap(), PathBuf::from("/dados/lucerna/notes"));
    assert_eq!(
        p.config_file().unwrap(),
        PathBuf::from("/home/u/.config/lucerna/notes.toml")
    );
}

#[test]
fn missing_or_relative_runtime_dir_is_an_error() {
    let mut e = env("/home/u");
    e.xdg_runtime_dir = None;
    let err = AppPaths::new("a", &e).unwrap().socket().unwrap_err();
    assert!(matches!(err, PathsError::NoRuntimeDir));
    assert_eq!(err.to_string(), "XDG_RUNTIME_DIR não está definido.");
    e.xdg_runtime_dir = Some("run".into());
    assert!(matches!(
        AppPaths::new("a", &e).unwrap().socket(),
        Err(PathsError::NoRuntimeDir)
    ));
}

#[test]
fn app_ids() {
    for ok in ["hello", "routine", "phone-android", "a1"] {
        assert!(validate_app_id(ok).is_ok(), "{ok}");
    }
    for bad in [
        "",
        "Hello",
        "1app",
        "-a",
        "a_b",
        "a/b",
        "..",
        "a.sock",
        &"a".repeat(33),
    ] {
        assert!(validate_app_id(bad).is_err(), "{bad}");
    }
    assert!(AppPaths::new("../x", &env("/h")).is_err());
}

#[test]
fn manifest_dirs_user_first() {
    assert_eq!(
        manifest_dirs(&env("/home/u")),
        vec![
            PathBuf::from("/home/u/.local/share/lucerna/apps"),
            PathBuf::from("/usr/share/lucerna/apps")
        ]
    );
}

#[test]
fn private_dir_is_0700_even_if_it_existed() {
    let t = tempfile::tempdir().unwrap();
    let d = t.path().join("lucerna");
    std::fs::create_dir(&d).unwrap();
    std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o755)).unwrap();
    create_private_dir(&d).unwrap();
    assert_eq!(
        std::fs::metadata(&d).unwrap().permissions().mode() & 0o777,
        0o700
    );
}
