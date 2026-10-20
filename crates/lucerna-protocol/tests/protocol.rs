use lucerna_protocol::*;
use serde_json::json;

#[test]
fn version_parses_and_compares() {
    let v: Version = "1.10".parse().unwrap();
    assert_eq!(
        v,
        Version {
            major: 1,
            minor: 10
        }
    );
    assert_eq!(v.to_string(), "1.10");
    for bad in ["1", "1.0.0", "", "a.b", "+1.0", "1.-1", " 1.0", "1.0 "] {
        assert!(bad.parse::<Version>().is_err(), "{bad}");
    }
    assert!(Version { major: 1, minor: 0 }.is_compatible_with(&Version { major: 1, minor: 9 }));
    assert!(!Version { major: 2, minor: 0 }.is_compatible_with(&PROTOCOL_VERSION));
    assert_eq!(PROTOCOL_VERSION.to_string(), "1.0");
}

#[test]
fn parses_spec_hello_request() {
    let m = Message::parse(r#"{"jsonrpc":"2.0","id":1,"method":"lucerna.hello","params":{"protocol":"1.0","client":"panel","app":"routine"}}"#).unwrap();
    let Message::Request(r) = m else { panic!() };
    assert_eq!(r.id, Id::Num(1));
    assert_eq!(r.method, methods::HELLO);
    let p: HelloParams = serde_json::from_value(r.params.unwrap()).unwrap();
    assert_eq!(p.protocol, PROTOCOL_VERSION);
    assert_eq!((p.client.as_str(), p.app.as_str()), ("panel", "routine"));
}

#[test]
fn classifies_notification_and_response() {
    let n = Message::parse(r#"{"jsonrpc":"2.0","method":"lucerna.state","params":{"topic":"focus","seq":12,"data":{"running":true}}}"#).unwrap();
    let Message::Notification(n) = n else {
        panic!()
    };
    let s: State = serde_json::from_value(n.params.unwrap()).unwrap();
    assert_eq!((s.topic.as_str(), s.seq), ("focus", 12));
    let r = Message::parse(r#"{"jsonrpc":"2.0","id":"a","error":{"code":-32012,"message":"x"}}"#)
        .unwrap();
    let Message::Response(r) = r else { panic!() };
    assert_eq!(r.id, Some(Id::Str("a".into())));
    assert_eq!(r.outcome.unwrap_err().code, codes::UNKNOWN_TOPIC);
}

#[test]
fn response_serializes_as_spec() {
    let r = Message::Response(Response {
        id: Some(Id::Num(7)),
        outcome: Ok(json!({"ok": true})),
    });
    let v: serde_json::Value = serde_json::from_str(&r.to_json()).unwrap();
    assert_eq!(v, json!({"jsonrpc":"2.0","id":7,"result":{"ok":true}}));
    assert!(!r.to_json().contains('\n'));
    let e = Message::Response(Response {
        id: None,
        outcome: Err(RpcError::parse_error()),
    });
    let v: serde_json::Value = serde_json::from_str(&e.to_json()).unwrap();
    assert_eq!(
        v,
        json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Mensagem inválida: não é JSON."}})
    );
}

#[test]
fn rejects_bad_messages_with_the_right_code() {
    assert_eq!(Message::parse("{oops").unwrap_err().code, -32700);
    for bad in [
        r#"[{"jsonrpc":"2.0","id":1,"method":"a.b"}]"#, // lote
        r#"{"jsonrpc":"1.0","id":1,"method":"a.b"}"#,   // versão errada
        r#"{"id":1,"method":"a.b"}"#,                   // sem jsonrpc
        r#"{"jsonrpc":"2.0","id":null,"method":"a.b"}"#, // id nulo em pedido
        r#"{"jsonrpc":"2.0","id":1.5,"method":"a.b"}"#, // id fracionário
        r#"{"jsonrpc":"2.0","id":1,"method":"a.b","params":3}"#, // params escalar
        r#"{"jsonrpc":"2.0","id":1}"#,                  // nem método nem resultado
        r#"{"jsonrpc":"2.0","id":1,"result":1,"error":{"code":1,"message":"x"}}"#,
        r#""texto""#,
    ] {
        assert_eq!(Message::parse(bad).unwrap_err().code, -32600, "{bad}");
    }
}

#[test]
fn error_messages_are_portuguese_and_exact() {
    let v = |c, s| Version { major: c, minor: s };
    assert_eq!(
        RpcError::incompatible_version(v(2, 0), v(1, 0)).message,
        "Versão do protocolo incompatível: o cliente fala 2.0 e o serviço fala 1.0."
    );
    assert_eq!(
        RpcError::incompatible_version(v(2, 0), v(1, 0)).code,
        -32010
    );
    assert_eq!(RpcError::hello_required().code, -32011);
    assert_eq!(
        RpcError::unknown_topic("x").message,
        "Tópico desconhecido: x."
    );
    assert_eq!(RpcError::message_too_large(1_048_576).code, -32000);
    assert_eq!(RpcError::app(3, "Sem tarefa.").code, 3);
}

#[test]
fn reserved_names() {
    assert!(is_reserved("lucerna.hello"));
    assert!(is_reserved("lucerna.qualquer"));
    assert!(!is_reserved("tasks.complete"));
}

#[test]
fn bar_item_defaults_and_omits() {
    let b: BarItem = serde_json::from_value(json!({"id":"focus","icon":"timer"})).unwrap();
    assert!(!b.panel);
    assert_eq!(
        serde_json::to_value(&b).unwrap(),
        json!({"id":"focus","icon":"timer","panel":false})
    );
    let t: TuiOpenParams = serde_json::from_value(json!({})).unwrap();
    assert!(t.args.is_empty());
}
