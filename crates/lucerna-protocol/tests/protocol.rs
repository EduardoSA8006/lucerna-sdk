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

#[test]
fn error_constructors_table() {
    let v = |c, s| Version { major: c, minor: s };
    let cases = [
        (
            RpcError::parse_error(),
            -32700,
            "Mensagem inválida: não é JSON.",
        ),
        (RpcError::invalid_request(), -32600, "Pedido inválido."),
        (
            RpcError::method_not_found("a.b"),
            -32601,
            "Método desconhecido: a.b.",
        ),
        (
            RpcError::invalid_params("falta x"),
            -32602,
            "Parâmetros inválidos: falta x.",
        ),
        (RpcError::internal(), -32603, "Erro interno do serviço."),
        (
            RpcError::message_too_large(1_048_576),
            -32000,
            "Mensagem grande demais (limite de 1048576 bytes).",
        ),
        (
            RpcError::incompatible_version(v(2, 0), v(1, 0)),
            -32010,
            "Versão do protocolo incompatível: o cliente fala 2.0 e o serviço fala 1.0.",
        ),
        (
            RpcError::hello_required(),
            -32011,
            "Faça o aperto de mão (lucerna.hello) antes dos outros pedidos.",
        ),
        (
            RpcError::unknown_topic("x"),
            -32012,
            "Tópico desconhecido: x.",
        ),
    ];
    for (e, code, msg) in cases {
        assert_eq!((e.code, e.message.as_str()), (code, msg));
        assert_eq!(e.to_string(), msg);
        assert!(e.data.is_none());
    }
}

#[test]
fn with_data_attaches_data() {
    let e = RpcError::app(3, "x").with_data(json!({"k": 1}));
    assert_eq!(e.data, Some(json!({"k": 1})));
    assert_eq!(
        serde_json::to_value(&e).unwrap(),
        json!({"code":3,"message":"x","data":{"k":1}})
    );
}

#[test]
fn round_trips_all_variants() {
    let msgs = [
        Message::Request(Request {
            id: Id::Str("a".into()),
            method: "a.b".into(),
            params: Some(json!({"x": 1})),
        }),
        Message::Request(Request {
            id: Id::Num(2),
            method: "a.b".into(),
            params: None,
        }),
        Message::Notification(Notification {
            method: "lucerna.state".into(),
            params: Some(json!([1, 2])),
        }),
        Message::Response(Response {
            id: Some(Id::Num(1)),
            outcome: Ok(json!(null)),
        }),
        Message::Response(Response {
            id: None,
            outcome: Err(RpcError::app(2, "ruim").with_data(json!(true))),
        }),
    ];
    for m in msgs {
        assert_eq!(Message::parse(&m.to_json()).unwrap(), m);
    }
}

#[test]
fn version_serializes_as_string() {
    let v = Version { major: 1, minor: 0 };
    assert_eq!(serde_json::to_value(v).unwrap(), json!("1.0"));
    assert_eq!(
        serde_json::from_value::<Version>(json!("1.10")).unwrap(),
        Version {
            major: 1,
            minor: 10
        }
    );
    assert!(serde_json::from_value::<Version>(json!(1.0)).is_err());
}

#[test]
fn version_rejects_leading_zeros() {
    for bad in ["01.00", "1.00", "00.1", "1.01"] {
        assert!(bad.parse::<Version>().is_err(), "{bad}");
    }
    assert_eq!(
        "0.0".parse::<Version>().unwrap(),
        Version { major: 0, minor: 0 }
    );
    assert_eq!(
        "0.10".parse::<Version>().unwrap(),
        Version {
            major: 0,
            minor: 10
        }
    );
}

#[test]
fn key_order_is_stable() {
    let req = Message::Request(Request {
        id: Id::Num(1),
        method: "a.b".into(),
        params: Some(json!({})),
    });
    assert_eq!(
        req.to_json(),
        r#"{"jsonrpc":"2.0","id":1,"method":"a.b","params":{}}"#
    );
    let ok = Message::Response(Response {
        id: Some(Id::Num(1)),
        outcome: Ok(json!(1)),
    });
    assert_eq!(ok.to_json(), r#"{"jsonrpc":"2.0","id":1,"result":1}"#);
    let err = Message::Response(Response {
        id: None,
        outcome: Err(RpcError::internal()),
    });
    assert_eq!(
        err.to_json(),
        r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32603,"message":"Erro interno do serviço."}}"#
    );
}

#[test]
fn null_id_only_valid_on_error_response() {
    let bad = r#"{"jsonrpc":"2.0","id":null,"result":1}"#;
    assert_eq!(Message::parse(bad).unwrap_err().code, -32600);
    let ok = r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"x"}}"#;
    assert!(matches!(
        Message::parse(ok),
        Ok(Message::Response(Response { id: None, .. }))
    ));
}
