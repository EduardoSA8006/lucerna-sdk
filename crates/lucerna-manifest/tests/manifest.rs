use lucerna_manifest::*;
use serde_json::Value;

fn cases() -> Value {
    let p = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../testdata/manifest-cases.json"
    );
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

#[test]
fn shared_api_cases() {
    let cases = cases();
    let list = cases["api"].as_array().unwrap();
    assert!(!list.is_empty());
    for c in list {
        let (p, s) = (
            c["panel"].as_str().unwrap().parse().unwrap(),
            c["shell"].as_str().unwrap().parse().unwrap(),
        );
        assert_eq!(api_compatible(p, s), c["load"].as_bool().unwrap(), "{c}");
    }
}

#[test]
fn shared_manifest_cases() {
    let cases = cases();
    let list = cases["manifests"].as_array().unwrap();
    assert!(!list.is_empty());
    for c in list {
        let r = Manifest::from_json(&c["manifest"].to_string());
        if c["valid"].as_bool().unwrap() {
            assert!(r.is_ok(), "{}: {:?}", c["name"], r.err());
        } else {
            assert_eq!(
                r.unwrap_err().kind(),
                c["error"].as_str().unwrap(),
                "{}",
                c["name"]
            );
        }
    }
}

#[test]
fn round_trip_omits_empty_parts() {
    let src = r#"{"schema":1,"app":{"id":"hello","name":"Olá","icon":"waving_hand","socket":"hello.sock"}}"#;
    let m = Manifest::from_json(src).unwrap();
    let out: Value = serde_json::from_str(&m.to_json_pretty()).unwrap();
    assert_eq!(out, serde_json::from_str::<Value>(src).unwrap());
    assert_eq!(Manifest::from_json(&m.to_json_pretty()).unwrap(), m);
}

fn base() -> Manifest {
    Manifest {
        schema: 1,
        app: AppInfo {
            id: "a".into(),
            name: "A".into(),
            icon: "x".into(),
            socket: "a.sock".into(),
            tui: None,
        },
        panel: None,
        card: None,
        shortcuts: Default::default(),
    }
}

#[test]
fn socket_error_carries_expected_and_found() {
    let mut m = base();
    m.app.socket = "b.sock".into();
    assert_eq!(
        m.validate().unwrap_err(),
        ManifestError::Socket {
            expected: "a.sock".into(),
            found: "b.sock".into()
        }
    );
}

#[test]
fn validate_checks_hand_built_manifest() {
    assert_eq!(base().validate(), Ok(()));
    let mut m = base();
    m.schema = 3;
    assert_eq!(m.validate(), Err(ManifestError::Schema(3)));
    let mut m = base();
    m.app.icon.clear();
    assert_eq!(m.validate(), Err(ManifestError::EmptyField("icon")));
    let mut m = base();
    m.card = Some(CardInfo {
        qml: "/c/Card.qml".into(),
    });
    assert_eq!(m.validate(), Err(ManifestError::CardWithoutPanel));
}

#[test]
fn round_trip_with_everything() {
    let mut m = base();
    m.app.tui = Some("lucerna-a".into());
    m.panel = Some(PanelInfo {
        qml: "/p/Panel.qml".into(),
        api: "1.2".parse().unwrap(),
    });
    m.card = Some(CardInfo {
        qml: "/p/Card.qml".into(),
    });
    m.shortcuts.insert("panel".into(), "SUPER+A".into());
    let json = m.to_json_pretty();
    let v: Value = serde_json::from_str(&json).unwrap();
    assert_eq!(v["panel"]["api"], "1.2");
    assert_eq!(v["shortcuts"]["panel"], "SUPER+A");
    assert_eq!(Manifest::from_json(&json).unwrap(), m);
}

#[test]
fn error_messages_are_sentences() {
    let cases = [
        (ManifestError::Json("x".into()), "Manifesto inválido: x."),
        (
            ManifestError::Schema(2),
            "Schema do manifesto não suportado: 2 (esperado 1).",
        ),
        (
            ManifestError::AppId("A".into()),
            "Id de app inválido: \"A\".",
        ),
        (
            ManifestError::Socket {
                expected: "a.sock".into(),
                found: "b.sock".into(),
            },
            "O socket do manifesto é \"b.sock\", esperado \"a.sock\".",
        ),
        (
            ManifestError::EmptyField("app.name"),
            "O campo app.name não pode ser vazio.",
        ),
        (
            ManifestError::RelativePath("p/Panel.qml".into()),
            "O caminho p/Panel.qml precisa ser absoluto.",
        ),
        (ManifestError::CardWithoutPanel, "O cartão exige um painel."),
    ];
    for (e, want) in cases {
        assert_eq!(e.to_string(), want);
    }
}
