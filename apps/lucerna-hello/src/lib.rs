//! Lucerna Hello: o app de exemplo, um contador. É o molde que os apps copiam.
//!
//! - tópico `counter`: `{"value": n}`;
//! - tópico contínuo `ticks`: `{"tick": k, "bands": [64 × f32]}` a cada 1 ms,
//!   só com assinante (imita o espectro de um analisador de áudio);
//! - métodos `counter.add {"n"}`, `counter.reset` e `quiet.set {"on"}`;
//! - na barra do shell, o item `counter` com o valor (some quando zera).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use lucerna_cli::clap::{Arg, Command, value_parser};
use lucerna_cli::{Call, CliApp, CliCommand};
use lucerna_ipc::Topic;
use lucerna_service::{BarItem, RpcError, ServiceBuilder, Shell};
use serde_json::{Value, json};
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;

/// Id do app.
pub const APP: &str = "hello";

/// Número de faixas do `ticks`.
const BANDS: usize = 64;

/// Período do `ticks`.
const TICK: Duration = Duration::from_millis(1);

/// O serviço montado e os tópicos dele.
///
/// O produtor do `ticks` vive enquanto [`Hello::producer`] viver: ao usar só o
/// `service`, mantenha o resto do `Hello` no escopo (um `let h = hello();`
/// seguido de `h.service.run()` já faz isso).
pub struct Hello {
    /// O serviço, pronto para `run` ou `run_with`.
    pub service: ServiceBuilder,
    /// O tópico `counter`.
    pub counter: Topic,
    /// O tópico contínuo `ticks`.
    pub ticks: Topic,
    /// A tarefa que produz o `ticks`; largá-la para a produção.
    pub producer: Producer,
}

/// A tarefa que produz o `ticks`. Abortada quando cai.
#[must_use = "o ticks para de ser produzido quando o Producer cai"]
pub struct Producer(JoinHandle<()>);

impl Drop for Producer {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// O contador e para onde cada mudança vai.
struct Counter {
    value: Mutex<i64>,
    topic: Topic,
    shell: Shell,
}

impl Counter {
    /// Troca o valor por `f(valor)`; `None` deixa como está e dá o erro de estouro.
    fn update(&self, f: impl FnOnce(i64) -> Option<i64>) -> Result<Value, RpcError> {
        // Publica sob a trava: o `seq` segue a ordem dos valores.
        let mut value = self.value.lock().unwrap_or_else(|e| e.into_inner());
        let v = f(*value).ok_or_else(|| RpcError::app(1, "O contador passou do limite."))?;
        *value = v;
        let data = json!({"value": v});
        self.topic.publish(data.clone());
        if v == 0 {
            self.shell.bar_clear("counter");
        } else {
            self.shell.bar_set(BarItem {
                id: "counter".into(),
                icon: "exposure_plus_1".into(),
                text: Some(v.to_string()),
                ends_at: None,
                panel: false,
            });
        }
        Ok(data)
    }
}

/// Lê o campo `name` de `params` com `get`; ausente ou de outro tipo dá `-32602`.
fn param<T>(
    params: &Option<Value>,
    name: &str,
    what: &str,
    get: impl FnOnce(&Value) -> Option<T>,
) -> Result<T, RpcError> {
    params
        .as_ref()
        .and_then(|p| p.get(name))
        .and_then(get)
        .ok_or_else(|| RpcError::invalid_params(&format!("{name} precisa ser {what}")))
}

/// Monta o serviço. Chame dentro do runtime: começa a tarefa que produz o `ticks`.
pub fn hello() -> Hello {
    let mut service = ServiceBuilder::new(APP, env!("CARGO_PKG_VERSION"));
    let shell = service.shell();
    let server = service.server();
    let counter = server.topic("counter", json!({"value": 0}));
    let ticks = server.stream("ticks");

    let state = Arc::new(Counter {
        value: Mutex::new(0),
        topic: counter.clone(),
        shell: shell.clone(),
    });
    let c = state.clone();
    server.method("counter.add", move |params| {
        let r = param(&params, "n", "um número inteiro", Value::as_i64)
            .and_then(|n| c.update(|v| v.checked_add(n)));
        async move { r }
    });
    let c = state;
    server.method("counter.reset", move |_| {
        let r = c.update(|_| Some(0));
        async move { r }
    });
    server.method("quiet.set", move |params| {
        let r = param(&params, "on", "true ou false", Value::as_bool).map(|on| {
            if on {
                shell.dnd_request("Lucerna Hello");
            } else {
                shell.dnd_release();
            }
            json!({})
        });
        async move { r }
    });

    let producer = Producer(tokio::spawn(produce_ticks(ticks.clone())));
    Hello {
        service,
        counter,
        ticks,
        producer,
    }
}

/// Produz o `ticks` enquanto houver assinante; sem assinante, espera.
async fn produce_ticks(ticks: Topic) {
    let mut k: u64 = 0;
    loop {
        ticks.wait_subscribed().await;
        let mut every = tokio::time::interval(TICK);
        every.set_missed_tick_behavior(MissedTickBehavior::Skip);
        // Um future só para o ciclo inteiro, em vez de um novo a cada quadro.
        let stop = ticks.wait_unsubscribed();
        tokio::pin!(stop);
        loop {
            tokio::select! {
                _ = every.tick() => {}
                _ = &mut stop => break,
            }
            k += 1;
            ticks.publish(json!({"tick": k, "bands": bands(k)}));
        }
    }
}

/// Um espectro de mentira: ondas que andam com `k`, entre 0 e 1.
fn bands(k: u64) -> Vec<f32> {
    let t = k as f32 * 0.01;
    (0..BANDS)
        .map(|i| {
            let x = i as f32 / BANDS as f32;
            0.5 + 0.5 * (t + x * 6.0).sin() * (1.0 - x)
        })
        .collect()
}

/// Texto do resultado do contador.
fn render_counter(v: &Value) -> String {
    format!("Contador: {}", v["value"])
}

/// A linha de comando do app.
pub fn cli() -> CliApp {
    CliApp::new(
        APP,
        "lucerna-hello",
        env!("CARGO_PKG_VERSION"),
        "Exemplo do Lucerna: um contador.",
    )
    .command(CliCommand {
        command: Command::new("add").about("Soma n ao contador.").arg(
            Arg::new("n")
                .value_name("n")
                .required(true)
                .value_parser(value_parser!(i64))
                .help("Quanto somar (pode ser negativo)."),
        ),
        to_call: |m| Call {
            method: "counter.add".into(),
            params: Some(json!({"n": *m.get_one::<i64>("n").expect("n é obrigatório")})),
        },
        render: Some(render_counter),
    })
    .command(CliCommand {
        command: Command::new("reset").about("Zera o contador."),
        to_call: |_| Call {
            method: "counter.reset".into(),
            params: None,
        },
        render: Some(render_counter),
    })
    .command(CliCommand {
        command: Command::new("quiet")
            .about("Liga ou desliga o não perturbe.")
            .arg(
                Arg::new("state")
                    .value_name("on|off")
                    .required(true)
                    .value_parser(["on", "off"])
                    .help("on liga, off desliga."),
            ),
        to_call: |m| Call {
            method: "quiet.set".into(),
            params: Some(json!({"on": m.get_one::<String>("state").is_some_and(|s| s == "on")})),
        },
        render: None,
    })
}
