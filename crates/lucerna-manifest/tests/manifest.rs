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
    for c in cases()["api"].as_array().unwrap() {
        let (p, s) = (
            c["panel"].as_str().unwrap().parse().unwrap(),
            c["shell"].as_str().unwrap().parse().unwrap(),
        );
        assert_eq!(api_compatible(p, s), c["load"].as_bool().unwrap(), "{c}");
    }
}

#[test]
fn shared_manifest_cases() {
    for c in cases()["manifests"].as_array().unwrap() {
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
