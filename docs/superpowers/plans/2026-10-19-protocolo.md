# lucerna-sdk — plano de implementação

> **Para agentes:** SUB-SKILL OBRIGATÓRIO: use superpowers:subagent-driven-development (recomendado) ou superpowers:executing-plans para implementar este plano tarefa por tarefa. Os passos usam checkbox (`- [ ]`) para acompanhamento.

**Objetivo:** um workspace Rust com os seis crates do protocolo do Lucerna (`lucerna-protocol`, `lucerna-paths`, `lucerna-manifest`, `lucerna-ipc`, `lucerna-service`, `lucerna-cli`), o app de exemplo `lucerna-hello` testado de ponta a ponta sem shell, e o CI com checks obrigatórios.

**Arquitetura:** camadas de baixo para cima. `lucerna-protocol` só tem tipos (sem I/O); `lucerna-paths` e `lucerna-manifest` são regras puras de caminho e de manifesto; `lucerna-ipc` faz o enquadramento por linha, o servidor (aperto de mão, métodos, assinaturas com coalescência por cliente via `tokio::sync::watch`) e o cliente; `lucerna-service` compõe o servidor com a ativação do systemd, a ociosidade, a conexão ao `shell.sock` com reconexão e os timers; `lucerna-cli` transforma subcomandos em métodos. O `lucerna-hello` é lib + bin (`lucerna-hello serve` / subcomandos), o molde que os apps copiam.

**Tecnologia:** Rust 1.95, edição 2024; `tokio` 1, `serde` 1 + `serde_json` 1, `clap` 4 (API builder), `thiserror` 2; `tempfile` 3 só nos testes; GitHub Actions; `gh api`.

**Spec:** `docs/superpowers/specs/2026-10-19-protocolo-design.md` (o contrato; leia inteiro antes da primeira tarefa). Contexto: `docs/ecossistema.md`.

## Global Constraints

- Toolchain: `cargo`/`rustc` em `~/develop/cargo/bin` (ponha no `PATH`); `rust-toolchain.toml` com `channel = "1.95"` e `components = ["rustfmt", "clippy"]`.
- Workspace: `resolver = "3"`, `edition = "2024"`, `rust-version = "1.95"`, `license = "GPL-3.0-only"`, `version = "0.1.0"`, `repository = "https://github.com/lucerna-desktop/lucerna-sdk"`; dependências só em `[workspace.dependencies]`; `Cargo.lock` versionado.
- Lints do workspace: `unsafe_code = "deny"` (a única exceção é a leitura do fd do systemd, com `#[allow(unsafe_code)]` e comentário `// SAFETY:`), `missing_docs = "warn"` nos crates publicáveis; `clippy::all` em `warn`. O CI roda com `-D warnings`: nenhum aviso passa.
- Transporte: socket Unix `SOCK_STREAM` em `$XDG_RUNTIME_DIR/lucerna/<app>.sock`, pasta `0700`; uma mensagem JSON por linha, UTF-8, terminada em `\n`; limite padrão `1 MiB` = `1_048_576` bytes por mensagem (sem contar o `\n`), ajustável no `ServerBuilder`.
- JSON-RPC 2.0: `"jsonrpc":"2.0"` obrigatório; `id` inteiro ou string; sem lotes (array → `-32600`).
- Versão do protocolo: `1.0` (`MAJOR.MINOR`, só dígitos ASCII, sem sinal); compatível = mesmo `MAJOR`.
- Códigos de erro: `-32700`, `-32600`, `-32601`, `-32602`, `-32603` (JSON-RPC); `-32000` mensagem grande demais (decisão deste plano, dentro da faixa reservada `-32000..=-32009`); `-32010` versão incompatível; `-32011` método antes do `hello`; `-32012` tópico desconhecido; app: `>= 1`.
- `message` dos erros sempre em português, pronta para o usuário, com o texto exato da Tarefa 1.
- Métodos reservados: começam com `lucerna.` (`lucerna.hello`, `lucerna.subscribe`, `lucerna.unsubscribe`, notificação `lucerna.state`). Shell: `bar.set`, `bar.clear`, `dnd.request`, `dnd.release`, `tui.open`, `panel.open`.
- Ociosidade padrão: 60 s sem clientes e sem trabalho pendente; `LUCERNA_IDLE_TIMEOUT_MS` sobrepõe (desenvolvimento e testes).
- Reconexão ao shell: espera `min(100 ms × 2^tentativa, 5 s)`.
- Códigos de saída da linha de comando: `0` sucesso, `1` erro do app, `2` sem falar com o serviço, `64` uso errado (decisão deste plano, para não colidir com o `2`); `--help`/`--version` saem com `0`.
- Manifesto: JSON, `schema = 1`, em `/usr/share/lucerna/apps/<app>.json` e `$XDG_DATA_HOME/lucerna/apps/` (o do usuário vence).
- Nada de rede, nada de `/tmp` como reserva para o `XDG_RUNTIME_DIR`. Testes nunca dependem do ambiente da máquina: cada teste cria as próprias pastas com `tempfile` e passa um `Env` explícito (ou define as variáveis só no processo filho).
- Commits em português, um ou poucos por tarefa, no estilo `Protocolo: tipos das mensagens`. Conta do `gh`: `EduardoSA8006`; remote `origin` = `github-pessoal:lucerna-desktop/lucerna-sdk.git`. Merge só por merge commit, pelo usuário.
- Verificação de toda tarefa (além do teste da própria tarefa): `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings` e `cargo test --workspace` sem falhas.

## Review Focus

1. **Cliente que cai no meio**: o cliente fecha a conexão com um pedido em andamento ou com assinaturas ativas — o esperado é o servidor não entrar em pânico, descartar a resposta em silêncio e decrementar os assinantes (um tópico contínuo para de ser produzido). Teste na Tarefa 5 (`abrupt_close_releases_subscriptions`).
2. **Método do app que entra em pânico**: o esperado é o cliente receber `-32603` e o serviço continuar atendendo esta e as outras conexões. Teste na Tarefa 5 (`handler_panic_becomes_internal_error`).
3. **Socket velho de uma execução à mão que caiu**: o arquivo `hello.sock` existe sem ninguém escutando — o esperado é o `serve` apagá-lo e subir; com outro serviço vivo no mesmo socket, recusar com mensagem clara em vez de roubar o socket. Testes na Tarefa 7.
4. **`XDG_RUNTIME_DIR` ausente ou relativo**: o esperado é um erro claro ("XDG_RUNTIME_DIR não está definido"), nunca cair em `/tmp`; na linha de comando, código `2` com a mensagem. Testes nas Tarefas 2 e 10.
5. **Shell que cai e volta várias vezes, com mudanças enquanto está fora**: várias `bar_set` seguidas, um `bar_clear` ou um `dnd_release` com o shell fora — o esperado é o shell receber só o estado desejado atual ao voltar (um `bar.set` com o último texto, nenhum item já tirado, nenhum `dnd.request` devolvido), sem fila de chamadas velhas. Testes na Tarefa 8.

---

## Decisões deste plano (o que o spec deixava ao construtor)

| Tema | Decisão | Por quê |
| --- | --- | --- |
| Runtime e bibliotecas | `tokio`, `serde`/`serde_json`, `clap` 4 (builder), `thiserror` 2 | padrão do ecossistema, licenças MIT/Apache compatíveis com GPL-3.0; `clap` builder porque os subcomandos são declarados pelo app em tempo de execução |
| `LISTEN_FDS` | à mão, sem crate | são ~30 linhas (conferir `LISTEN_PID` = `std::process::id()`, `LISTEN_FDS = 1`, fd 3); `OwnedFd::try_clone()` reabre o fd com `CLOEXEC` sem `libc` (o fd 3 do systemd vem sem `CLOEXEC` e vazaria para o `systemctl` do agendador); `listenfd` traria mais do que usamos; `sd-notify` não é preciso (`Type=simple`) |
| Coalescência | `tokio::sync::watch` por tópico + um mapa "pendente" por conexão (no máximo um retrato por tópico) | "só o último" e "coalescido por cliente" de graça, memória limitada por construção |
| Resposta do `lucerna.subscribe` | `{"states":[{"topic","seq","data"}, ...]}`, a mesma forma do `lucerna.state` | o cliente trata retrato e mudança com o mesmo código |
| `seq` | começa em `0` (estado inicial) e soma 1 a cada `publish`, por tópico, por vida do serviço | o cliente descarta `seq <= último visto` |
| Tópico contínuo sem dado | retrato com `data: null` e `seq: 0` | o produtor só roda com assinante |
| Resposta do `lucerna.unsubscribe` | `{}` | |
| `hello` de outro app | `-32602` ("este serviço é o app X, não Y") | o painel detecta socket trocado |
| `hello` repetido | idempotente (responde de novo) | |
| `hello` no `shell.sock` | `client` = id do app, `app` = `"shell"` | coerente com a regra acima |
| Mensagem > limite | `-32000` e a conexão fecha | faixa reservada ao protocolo |
| Linha que não é JSON | `-32700` com `id: null`, a conexão continua | comportamento JSON-RPC |
| Pedidos do app | cada um numa tarefa própria (respostas podem sair fora de ordem, pelo `id`); `lucerna.*` tratado em linha | um método lento não trava assinaturas |
| `bar.set` | só os campos do spec: `id`, `icon`, `text?`, `ends_at?` (Unix s), `panel` (bool, padrão `false`) | YAGNI |
| Timer agendado | arquivos de unidade `.timer` + `systemctl --user` (não `systemd-run`): persistente em `$XDG_CONFIG_HOME/systemd/user/` com `Persistent=true` e `enable --now`; não persistente em `$XDG_RUNTIME_DIR/systemd/user/` com `start` | unidade transitória vive em `/run` e some no reinício; um mecanismo só para os dois casos. Atrás do trait `Scheduler`; testes com `FakeScheduler` e com um `systemctl` falso (script que registra os argumentos); o teste real é `#[ignore]` |
| `OnCalendar` | `"AAAA-MM-DD HH:MM:SS UTC"` a partir do Unix s, com o algoritmo civil de Hinnant (sem `chrono`) | sem dependência por uma conversão |
| Horário no passado | `ScheduleError::InPast` | o app faz o trabalho na hora |
| Saída por uso errado | `64` (`EX_USAGE`) | o `2` do clap colidiria com "sem serviço" |
| Saída legível padrão | string → ela mesma; objeto → `chave: valor` por linha; lista → um item por linha; `null` → nada | o app pode trocar por um `render` próprio |
| `--json` com erro | stdout vazio, stderr `{"error":{code,message,data?}}` | scripts leem um JSON só |
| `status` | assina todos os `topics` (não os `streams`) e imprime; `--json` → `{"<tópico>": data}` | |
| `watch` | retrato e cada mudança; `--json` → uma linha `{"topic","seq","data"}` por estado (JSON Lines); conexão perdida → `2` | |
| Sem argumentos no `lucerna-hello` | imprime a ajuda e sai `0` (não há TUI) | |
| `lucerna-hello` | id `hello`; `counter` = `{"value": n}`; `ticks` = `{"tick": k, "bands": [64 × f32]}` a cada 1 ms (imita o espectro); métodos `counter.add {"n"}`, `counter.reset`, `quiet.set {"on"}` (não perturbe, para o teste do shell) | `quiet.set` é o único acréscimo ao spec, para exercitar `dnd.*` |
| `FakeShell` | público em `lucerna_service::testing` | todo app vai querer testar o que manda ao shell |
| `RawConn` | público em `lucerna_ipc::raw` | testes e depuração, linha por linha como no `socat` |
| Exemplos compartilhados | `testdata/manifest-cases.json` (casos de `api` e de manifestos válidos/inválidos com o `kind` do erro) | o shell lê o mesmo arquivo |
| Ruleset | checks `fmt`, `clippy`, `test` (`integration_id: 15368`), `strict: true`, aplicados depois que o CI do PR passar | |

## Mapa de arquivos

| Arquivo | Responsabilidade |
| --- | --- |
| `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml` | workspace, versões, lints |
| `crates/lucerna-protocol/src/{lib,version,message,error,methods,params}.rs` | tipos e constantes, sem I/O |
| `crates/lucerna-paths/src/lib.rs` | `Env`, `AppPaths`, pastas XDG, validação do id |
| `crates/lucerna-manifest/src/lib.rs` | `Manifest`, leitura, validação, `api_compatible` |
| `testdata/manifest-cases.json` | exemplos compartilhados com o shell |
| `crates/lucerna-ipc/src/{lib,framing,server,topic,client,raw}.rs` | enquadramento, servidor, tópicos, cliente, conexão crua |
| `crates/lucerna-service/src/{lib,activation,idle,shell,schedule,service,testing}.rs` | esqueleto do serviço |
| `crates/lucerna-cli/src/{lib,render}.rs` | esqueleto da linha de comando |
| `apps/lucerna-hello/src/{lib,main}.rs` | o app de exemplo |
| `apps/lucerna-hello/systemd/lucerna-hello.{socket,service}` | unidades do systemd do usuário |
| `apps/lucerna-hello/data/hello.json` | manifesto sem painel |
| `apps/lucerna-hello/tests/{protocol,shell,process}.rs` | integração de ponta a ponta |
| `.github/workflows/ci.yml`, `.github/dependabot.yml` | CI |

---

### Tarefa 1: workspace e `lucerna-protocol`

**Files:**
- Create: `Cargo.toml`, `rust-toolchain.toml`, `.gitignore` (`/target`)
- Create: `crates/lucerna-protocol/Cargo.toml`, `crates/lucerna-protocol/src/{lib,version,message,error,methods,params}.rs`
- Test: `crates/lucerna-protocol/tests/protocol.rs`

**Interfaces:**
- Consumes: nada.
- Produces:
  - `pub struct Version { pub major: u32, pub minor: u32 }` — `Copy, Eq, Debug`, `FromStr` (`Err = VersionError`), `Display` (`"1.0"`), serde como string; `pub fn is_compatible_with(&self, other: &Version) -> bool` (mesmo major).
  - `pub const PROTOCOL_VERSION: Version` (`1.0`).
  - `pub enum Id { Num(i64), Str(String) }` — serde `untagged`, `Clone, Eq, Hash, Debug`.
  - `pub enum Message { Request(Request), Notification(Notification), Response(Response) }`; `pub struct Request { pub id: Id, pub method: String, pub params: Option<Value> }`; `pub struct Notification { pub method: String, pub params: Option<Value> }`; `pub struct Response { pub id: Option<Id>, pub outcome: Result<Value, RpcError> }`.
  - `impl Message { pub fn parse(line: &str) -> Result<Message, RpcError>; pub fn to_json(&self) -> String }` (`to_json` sem `\n`; `Response` com `id: None` sai como `"id":null`).
  - `pub struct RpcError { pub code: i32, pub message: String, pub data: Option<Value> }` — serde, `thiserror::Error` (`Display` = `message`); construtores: `parse_error()`, `invalid_request()`, `method_not_found(method: &str)`, `invalid_params(detail: &str)`, `internal()`, `message_too_large(limit: usize)`, `incompatible_version(client: Version, server: Version)`, `hello_required()`, `unknown_topic(topic: &str)`, `app(code: i32, message: impl Into<String>)` (`debug_assert!(code >= 1)`), `with_data(self, data: Value) -> Self`.
  - `pub mod codes { PARSE_ERROR=-32700, INVALID_REQUEST=-32600, METHOD_NOT_FOUND=-32601, INVALID_PARAMS=-32602, INTERNAL_ERROR=-32603, MESSAGE_TOO_LARGE=-32000, INCOMPATIBLE_VERSION=-32010, HELLO_REQUIRED=-32011, UNKNOWN_TOPIC=-32012 }` (`i32`).
  - `pub mod methods { HELLO, SUBSCRIBE, UNSUBSCRIBE, STATE, BAR_SET, BAR_CLEAR, DND_REQUEST, DND_RELEASE, TUI_OPEN, PANEL_OPEN }` (`&str`) e `pub fn is_reserved(method: &str) -> bool`.
  - `params`: `HelloParams { protocol: Version, client: String, app: String }`, `HelloResult { protocol: Version, app: String, version: String, topics: Vec<String>, streams: Vec<String>, methods: Vec<String> }`, `TopicsParams { topics: Vec<String> }` (subscribe e unsubscribe), `State { topic: String, seq: u64, data: Value }` (params do `lucerna.state`), `SubscribeResult { states: Vec<State> }`, `BarItem { id: String, icon: String, text: Option<String>, ends_at: Option<u64>, panel: bool }` (`panel` `#[serde(default)]`, opcionais com `skip_serializing_if`), `BarClearParams { id }`, `DndRequestParams { reason }`, `TuiOpenParams { args: Vec<String> }` (`default`), `PanelOpenParams { page: Option<String> }`. Todos `Serialize, Deserialize, Clone, Debug, PartialEq`.

Textos exatos das mensagens de erro:

| Construtor | `message` |
| --- | --- |
| `parse_error` | `Mensagem inválida: não é JSON.` |
| `invalid_request` | `Pedido inválido.` |
| `method_not_found(m)` | `Método desconhecido: {m}.` |
| `invalid_params(d)` | `Parâmetros inválidos: {d}.` |
| `internal` | `Erro interno do serviço.` |
| `message_too_large(l)` | `Mensagem grande demais (limite de {l} bytes).` |
| `incompatible_version(c, s)` | `Versão do protocolo incompatível: o cliente fala {c} e o serviço fala {s}.` |
| `hello_required` | `Faça o aperto de mão (lucerna.hello) antes dos outros pedidos.` |
| `unknown_topic(t)` | `Tópico desconhecido: {t}.` |

- [ ] **Passo 1: workspace.** `Cargo.toml` raiz com os valores das Global Constraints, `members = ["crates/*", "apps/*"]`, `[workspace.dependencies]`: `tokio = { version = "1", features = ["rt-multi-thread", "macros", "net", "io-util", "sync", "time", "signal"] }`, `serde = { version = "1", features = ["derive"] }`, `serde_json = "1"`, `clap = "4"`, `thiserror = "2"`, `tempfile = "3"` e os seis crates por `path` + `version = "0.1.0"`; `[workspace.lints]` como nas Global Constraints. Cada crate usa `[lints] workspace = true` e `edition.workspace = true` etc. A doc do crate (`//!` no `lib.rs`) diz "fala o protocolo 1.0".

- [ ] **Passo 2: testes que falham** em `crates/lucerna-protocol/tests/protocol.rs`:

```rust
use lucerna_protocol::*;
use serde_json::json;

#[test]
fn version_parses_and_compares() {
    let v: Version = "1.10".parse().unwrap();
    assert_eq!(v, Version { major: 1, minor: 10 });
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
    let Message::Notification(n) = n else { panic!() };
    let s: State = serde_json::from_value(n.params.unwrap()).unwrap();
    assert_eq!((s.topic.as_str(), s.seq), ("focus", 12));
    let r = Message::parse(r#"{"jsonrpc":"2.0","id":"a","error":{"code":-32012,"message":"x"}}"#).unwrap();
    let Message::Response(r) = r else { panic!() };
    assert_eq!(r.id, Some(Id::Str("a".into())));
    assert_eq!(r.outcome.unwrap_err().code, codes::UNKNOWN_TOPIC);
}

#[test]
fn response_serializes_as_spec() {
    let r = Message::Response(Response { id: Some(Id::Num(7)), outcome: Ok(json!({"ok": true})) });
    let v: serde_json::Value = serde_json::from_str(&r.to_json()).unwrap();
    assert_eq!(v, json!({"jsonrpc":"2.0","id":7,"result":{"ok":true}}));
    assert!(!r.to_json().contains('\n'));
    let e = Message::Response(Response { id: None, outcome: Err(RpcError::parse_error()) });
    let v: serde_json::Value = serde_json::from_str(&e.to_json()).unwrap();
    assert_eq!(v, json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Mensagem inválida: não é JSON."}}));
}

#[test]
fn rejects_bad_messages_with_the_right_code() {
    assert_eq!(Message::parse("{oops").unwrap_err().code, -32700);
    for bad in [
        r#"[{"jsonrpc":"2.0","id":1,"method":"a.b"}]"#,     // lote
        r#"{"jsonrpc":"1.0","id":1,"method":"a.b"}"#,       // versão errada
        r#"{"id":1,"method":"a.b"}"#,                      // sem jsonrpc
        r#"{"jsonrpc":"2.0","id":null,"method":"a.b"}"#,    // id nulo em pedido
        r#"{"jsonrpc":"2.0","id":1.5,"method":"a.b"}"#,     // id fracionário
        r#"{"jsonrpc":"2.0","id":1,"method":"a.b","params":3}"#, // params escalar
        r#"{"jsonrpc":"2.0","id":1}"#,                      // nem método nem resultado
        r#"{"jsonrpc":"2.0","id":1,"result":1,"error":{"code":1,"message":"x"}}"#,
        r#""texto""#,
    ] {
        assert_eq!(Message::parse(bad).unwrap_err().code, -32600, "{bad}");
    }
}

#[test]
fn error_messages_are_portuguese_and_exact() {
    let v = |c, s| Version { major: c, minor: s };
    assert_eq!(RpcError::incompatible_version(v(2, 0), v(1, 0)).message,
        "Versão do protocolo incompatível: o cliente fala 2.0 e o serviço fala 1.0.");
    assert_eq!(RpcError::incompatible_version(v(2, 0), v(1, 0)).code, -32010);
    assert_eq!(RpcError::hello_required().code, -32011);
    assert_eq!(RpcError::unknown_topic("x").message, "Tópico desconhecido: x.");
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
    assert_eq!(serde_json::to_value(&b).unwrap(), json!({"id":"focus","icon":"timer","panel":false}));
    let t: TuiOpenParams = serde_json::from_value(json!({})).unwrap();
    assert!(t.args.is_empty());
}
```

- [ ] **Passo 3: rodar e ver falhar.** `cargo test -p lucerna-protocol` → erro de compilação (itens inexistentes).

- [ ] **Passo 4: implementar** os itens das Interfaces. `Message::parse`: `serde_json::from_str::<Value>` (falha → `parse_error`), depois classificar o objeto à mão pelas regras dos testes; `to_json` monta um `serde_json::Map` na ordem `jsonrpc, id, method/result/error, params`. `Version` em serde como string: `impl Serialize` com `collect_str` e `Deserialize` por `String` + `FromStr`.

- [ ] **Passo 5: verificar.** `cargo test -p lucerna-protocol` → 8 passed; `cargo fmt --check` e `cargo clippy --workspace --all-targets -- -D warnings` sem saída de erro.

- [ ] **Passo 6: commit.** `git add Cargo.toml Cargo.lock rust-toolchain.toml .gitignore crates/lucerna-protocol && git commit -m "Protocolo: workspace e tipos das mensagens"`

---

### Tarefa 2: `lucerna-paths`

**Files:**
- Create: `crates/lucerna-paths/{Cargo.toml,src/lib.rs}`
- Test: `crates/lucerna-paths/tests/paths.rs`

**Interfaces:**
- Consumes: nada.
- Produces:
  - `pub struct Env { pub home: Option<PathBuf>, pub xdg_data_home: Option<PathBuf>, pub xdg_config_home: Option<PathBuf>, pub xdg_cache_home: Option<PathBuf>, pub xdg_runtime_dir: Option<PathBuf> }` — `Default, Clone, Debug`; `pub fn from_process() -> Env` (variáveis vazias contam como ausentes).
  - `pub enum PathsError { InvalidAppId(String), NoHome, NoRuntimeDir }` (`thiserror`; textos: `Nome de app inválido: "{0}" (use a-z, 0-9 e "-", começando por letra, até 32).`, `HOME não está definido.`, `XDG_RUNTIME_DIR não está definido.`).
  - `pub fn validate_app_id(id: &str) -> Result<(), PathsError>` — `^[a-z][a-z0-9-]{0,31}$`.
  - `pub struct AppPaths` com `pub fn new(app: &str, env: &Env) -> Result<AppPaths, PathsError>` e `app() -> &str`, `data_dir() -> Result<PathBuf, PathsError>`, `config_file()`, `cache_dir()`, `runtime_dir()` (`$XDG_RUNTIME_DIR/lucerna`), `socket()` (`…/lucerna/<app>.sock`), todos `Result<PathBuf, PathsError>`.
  - `pub fn shell_socket(env: &Env) -> Result<PathBuf, PathsError>`; `pub fn manifest_dirs(env: &Env) -> Vec<PathBuf>` (usuário primeiro, depois `/usr/share/lucerna/apps`); `pub fn create_private_dir(dir: &Path) -> io::Result<()>` (cria com pais e força `0700`).

- [ ] **Passo 1: testes que falham.**

```rust
use lucerna_paths::*;
use std::{os::unix::fs::PermissionsExt, path::PathBuf};

fn env(home: &str) -> Env { Env { home: Some(home.into()), xdg_runtime_dir: Some("/run/user/1000".into()), ..Env::default() } }

#[test]
fn xdg_defaults_follow_ecossistema() {
    let p = AppPaths::new("routine", &env("/home/u")).unwrap();
    assert_eq!(p.data_dir().unwrap(), PathBuf::from("/home/u/.local/share/lucerna/routine"));
    assert_eq!(p.config_file().unwrap(), PathBuf::from("/home/u/.config/lucerna/routine.toml"));
    assert_eq!(p.cache_dir().unwrap(), PathBuf::from("/home/u/.cache/lucerna/routine"));
    assert_eq!(p.socket().unwrap(), PathBuf::from("/run/user/1000/lucerna/routine.sock"));
    assert_eq!(shell_socket(&env("/h")).unwrap(), PathBuf::from("/run/user/1000/lucerna/shell.sock"));
}

#[test]
fn xdg_variables_win_and_relative_ones_are_ignored() {
    let mut e = env("/home/u");
    e.xdg_data_home = Some("/dados".into());
    e.xdg_config_home = Some("relativo/config".into());
    let p = AppPaths::new("notes", &e).unwrap();
    assert_eq!(p.data_dir().unwrap(), PathBuf::from("/dados/lucerna/notes"));
    assert_eq!(p.config_file().unwrap(), PathBuf::from("/home/u/.config/lucerna/notes.toml"));
}

#[test]
fn missing_or_relative_runtime_dir_is_an_error() {
    let mut e = env("/home/u");
    e.xdg_runtime_dir = None;
    let err = AppPaths::new("a", &e).unwrap().socket().unwrap_err();
    assert!(matches!(err, PathsError::NoRuntimeDir));
    assert_eq!(err.to_string(), "XDG_RUNTIME_DIR não está definido.");
    e.xdg_runtime_dir = Some("run".into());
    assert!(matches!(AppPaths::new("a", &e).unwrap().socket(), Err(PathsError::NoRuntimeDir)));
}

#[test]
fn app_ids() {
    for ok in ["hello", "routine", "phone-android", "a1"] { assert!(validate_app_id(ok).is_ok(), "{ok}"); }
    for bad in ["", "Hello", "1app", "-a", "a_b", "a/b", "..", "a.sock", &"a".repeat(33)] {
        assert!(validate_app_id(bad).is_err(), "{bad}");
    }
    assert!(AppPaths::new("../x", &env("/h")).is_err());
}

#[test]
fn manifest_dirs_user_first() {
    assert_eq!(manifest_dirs(&env("/home/u")), vec![
        PathBuf::from("/home/u/.local/share/lucerna/apps"), PathBuf::from("/usr/share/lucerna/apps")]);
}

#[test]
fn private_dir_is_0700_even_if_it_existed() {
    let t = tempfile::tempdir().unwrap();
    let d = t.path().join("lucerna");
    std::fs::create_dir(&d).unwrap();
    std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o755)).unwrap();
    create_private_dir(&d).unwrap();
    assert_eq!(std::fs::metadata(&d).unwrap().permissions().mode() & 0o777, 0o700);
}
```

- [ ] **Passo 2: rodar e ver falhar.** `cargo test -p lucerna-paths` → erro de compilação.
- [ ] **Passo 3: implementar** as Interfaces (só `std` + `thiserror`).
- [ ] **Passo 4: verificar.** `cargo test -p lucerna-paths` → 6 passed; fmt e clippy limpos.
- [ ] **Passo 5: commit.** `git commit -m "Caminhos: pastas XDG e socket de cada app"`

---

### Tarefa 3: `lucerna-manifest` e os exemplos compartilhados

**Files:**
- Create: `crates/lucerna-manifest/{Cargo.toml,src/lib.rs}`, `testdata/manifest-cases.json`, `testdata/README.md` (três linhas: para que serve, quem lê, que o shell testa contra ele)
- Test: `crates/lucerna-manifest/tests/manifest.rs`

**Interfaces:**
- Consumes: `lucerna_protocol::Version`; `lucerna_paths::validate_app_id`.
- Produces:
  - `pub const SCHEMA: u32 = 1;`
  - `pub struct Manifest { pub schema: u32, pub app: AppInfo, pub panel: Option<PanelInfo>, pub card: Option<CardInfo>, pub shortcuts: BTreeMap<String, String> }`; `AppInfo { id, name, icon, socket: String, tui: Option<String> }`; `PanelInfo { qml: PathBuf, api: Version }`; `CardInfo { qml: PathBuf }`. Campos desconhecidos são tolerados; opcionais e mapa vazio omitidos na escrita.
  - `impl Manifest { pub fn from_json(s: &str) -> Result<Manifest, ManifestError>; pub fn validate(&self) -> Result<(), ManifestError>; pub fn to_json_pretty(&self) -> String }` (`from_json` já valida).
  - `pub enum ManifestError { Json(String), Schema(u32), AppId(String), Socket { expected: String, found: String }, EmptyField(&'static str), RelativePath(PathBuf), CardWithoutPanel }` com `pub fn kind(&self) -> &'static str` = `"json" | "schema" | "app-id" | "socket" | "field" | "path" | "card-without-panel"` (versão de `api` malformada é `"json"`).
  - `pub fn api_compatible(panel: Version, shell: Version) -> bool` — mesmo major e `panel.minor <= shell.minor`.

Regras de `validate`: `schema == 1`; id válido; `name`, `icon` e `tui` (se houver) não vazios; `socket == "<id>.sock"`; `qml` absolutos; `card` exige `panel`.

`testdata/manifest-cases.json` (formato fixo; o shell lê o mesmo):

```json
{
  "api": [
    {"panel": "1.0", "shell": "1.0", "load": true},
    {"panel": "1.2", "shell": "1.3", "load": true},
    {"panel": "1.3", "shell": "1.2", "load": false},
    {"panel": "1.10", "shell": "1.9", "load": false},
    {"panel": "1.9", "shell": "1.10", "load": true},
    {"panel": "2.0", "shell": "1.9", "load": false},
    {"panel": "1.0", "shell": "2.0", "load": false}
  ],
  "manifests": [
    {"name": "completo (o do spec)", "valid": true, "manifest": {"schema": 1, "app": {"id": "routine", "name": "Rotina", "icon": "event_available", "socket": "routine.sock", "tui": "lucerna-routine"}, "panel": {"qml": "/usr/share/lucerna/panels/routine/Panel.qml", "api": "1.0"}, "card": {"qml": "/usr/share/lucerna/panels/routine/Card.qml"}, "shortcuts": {"panel": "SUPER+R"}}},
    {"name": "só serviço", "valid": true, "manifest": {"schema": 1, "app": {"id": "lucernad", "name": "Lucerna", "icon": "graphic_eq", "socket": "lucernad.sock"}}},
    {"name": "campo novo tolerado", "valid": true, "manifest": {"schema": 1, "futuro": 1, "app": {"id": "a", "name": "A", "icon": "x", "socket": "a.sock"}}},
    {"name": "schema desconhecido", "valid": false, "error": "schema", "manifest": {"schema": 2, "app": {"id": "a", "name": "A", "icon": "x", "socket": "a.sock"}}},
    {"name": "socket de outro app", "valid": false, "error": "socket", "manifest": {"schema": 1, "app": {"id": "a", "name": "A", "icon": "x", "socket": "b.sock"}}},
    {"name": "id inválido", "valid": false, "error": "app-id", "manifest": {"schema": 1, "app": {"id": "Rotina", "name": "A", "icon": "x", "socket": "Rotina.sock"}}},
    {"name": "nome vazio", "valid": false, "error": "field", "manifest": {"schema": 1, "app": {"id": "a", "name": "", "icon": "x", "socket": "a.sock"}}},
    {"name": "qml relativo", "valid": false, "error": "path", "manifest": {"schema": 1, "app": {"id": "a", "name": "A", "icon": "x", "socket": "a.sock"}, "panel": {"qml": "Panel.qml", "api": "1.0"}}},
    {"name": "card sem painel", "valid": false, "error": "card-without-panel", "manifest": {"schema": 1, "app": {"id": "a", "name": "A", "icon": "x", "socket": "a.sock"}, "card": {"qml": "/c/Card.qml"}}},
    {"name": "api malformada", "valid": false, "error": "json", "manifest": {"schema": 1, "app": {"id": "a", "name": "A", "icon": "x", "socket": "a.sock"}, "panel": {"qml": "/p/Panel.qml", "api": "1"}}},
    {"name": "sem app", "valid": false, "error": "json", "manifest": {"schema": 1}}
  ]
}
```

- [ ] **Passo 1: testes que falham.**

```rust
use lucerna_manifest::*;
use serde_json::Value;

fn cases() -> Value {
    let p = concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/manifest-cases.json");
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

#[test]
fn shared_api_cases() {
    for c in cases()["api"].as_array().unwrap() {
        let (p, s) = (c["panel"].as_str().unwrap().parse().unwrap(), c["shell"].as_str().unwrap().parse().unwrap());
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
            assert_eq!(r.unwrap_err().kind(), c["error"].as_str().unwrap(), "{}", c["name"]);
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
```

- [ ] **Passo 2: rodar e ver falhar.** `cargo test -p lucerna-manifest` → erro de compilação.
- [ ] **Passo 3: implementar** (serde + as regras acima; `Manifest: PartialEq, Debug, Clone`).
- [ ] **Passo 4: verificar.** `cargo test -p lucerna-manifest` → 3 passed.
- [ ] **Passo 5: commit.** `git commit -m "Manifesto: tipos, validação e exemplos compartilhados com o shell"`

---

### Tarefa 4: `lucerna-ipc` — enquadramento por linha

**Files:**
- Create: `crates/lucerna-ipc/{Cargo.toml,src/lib.rs,src/framing.rs}`
- Test: `crates/lucerna-ipc/tests/framing.rs`

**Interfaces:**
- Consumes: `lucerna_protocol::Message`.
- Produces:
  - `pub const MAX_MESSAGE: usize = 1_048_576;`
  - `pub struct LineReader<R> { .. }` com `pub fn new(inner: R, max: usize) -> Self` e `pub async fn next_line(&mut self) -> Result<Option<String>, FrameError>` (`R: AsyncRead + Unpin`): devolve a linha sem `\n` (e sem `\r` final); pula linhas vazias; `Ok(None)` no fim do fluxo (um resto sem `\n` no fim é descartado).
  - `pub enum FrameError { TooLarge, InvalidUtf8, Io(io::Error) }` (`thiserror`). Depois de `TooLarge` o leitor não é mais usado (a conexão fecha); depois de `InvalidUtf8` a linha inteira foi consumida e o leitor continua.
  - `pub async fn write_message<W: AsyncWrite + Unpin>(w: &mut W, msg: &Message) -> io::Result<()>` (escreve `to_json()` + `\n` e dá `flush`).

- [ ] **Passo 1: testes que falham.**

```rust
use lucerna_ipc::framing::*;
use tokio::io::AsyncWriteExt;

async fn reader_over(chunks: Vec<Vec<u8>>, max: usize) -> LineReader<tokio::io::DuplexStream> {
    let (mut a, b) = tokio::io::duplex(64);
    tokio::spawn(async move { for c in chunks { a.write_all(&c).await.unwrap(); tokio::task::yield_now().await; } });
    LineReader::new(b, max)
}

#[tokio::test]
async fn joins_split_lines_and_splits_joined_ones() {
    let mut r = reader_over(vec![b"{\"a\":".to_vec(), b"1}\n{\"b\":2}\n\n{\"c\"".to_vec(), b":3}\r\n".to_vec()], 1024).await;
    assert_eq!(r.next_line().await.unwrap().unwrap(), r#"{"a":1}"#);
    assert_eq!(r.next_line().await.unwrap().unwrap(), r#"{"b":2}"#);
    assert_eq!(r.next_line().await.unwrap().unwrap(), r#"{"c":3}"#);
    assert!(r.next_line().await.unwrap().is_none());
}

#[tokio::test]
async fn utf8_split_across_reads() {
    let s = "{\"m\":\"não perturbe ☕\"}\n".as_bytes().to_vec();
    let cut = s.iter().position(|&b| b >= 0x80).unwrap() + 1; // no meio de "ã"
    let mut r = reader_over(vec![s[..cut].to_vec(), s[cut..].to_vec()], 1024).await;
    assert_eq!(r.next_line().await.unwrap().unwrap(), "{\"m\":\"não perturbe ☕\"}");
}

#[tokio::test]
async fn limit_is_exact() {
    let ok = format!("\"{}\"\n", "x".repeat(98)); // 100 bytes + \n
    let mut r = reader_over(vec![ok.into_bytes()], 100).await;
    assert_eq!(r.next_line().await.unwrap().unwrap().len(), 100);
    let big = format!("\"{}\"\n", "x".repeat(99));
    let mut r = reader_over(vec![big.into_bytes()], 100).await;
    assert!(matches!(r.next_line().await, Err(FrameError::TooLarge)));
}

#[tokio::test]
async fn huge_line_without_newline_does_not_buffer_forever() {
    let mut r = reader_over(vec![vec![b'x'; 300_000]], 1000).await;
    assert!(matches!(r.next_line().await, Err(FrameError::TooLarge)));
}

#[tokio::test]
async fn invalid_utf8_skips_only_that_line() {
    let mut r = reader_over(vec![b"\xff\xfe\n{}\n".to_vec()], 100).await;
    assert!(matches!(r.next_line().await, Err(FrameError::InvalidUtf8)));
    assert_eq!(r.next_line().await.unwrap().unwrap(), "{}");
}

#[tokio::test]
async fn trailing_fragment_at_eof_is_dropped() {
    let mut r = reader_over(vec![b"{}\n{\"meio".to_vec()], 100).await;
    assert_eq!(r.next_line().await.unwrap().unwrap(), "{}");
    assert!(r.next_line().await.unwrap().is_none());
}
```

- [ ] **Passo 2: rodar e ver falhar.** `cargo test -p lucerna-ipc --test framing` → erro de compilação.
- [ ] **Passo 3: implementar.** `LineReader` sobre `BufReader` com `fill_buf`/`consume`, acumulando bytes num `Vec` até o `\n`; passou de `max` sem `\n` → `TooLarge` sem continuar lendo; UTF-8 conferido só na linha completa.
- [ ] **Passo 4: verificar.** `cargo test -p lucerna-ipc --test framing` → 6 passed.
- [ ] **Passo 5: commit.** `git commit -m "IPC: enquadramento por linha com limite de tamanho"`

---

### Tarefa 5: `lucerna-ipc` — servidor e tópicos

**Files:**
- Create: `crates/lucerna-ipc/src/{server,topic,raw}.rs`
- Test: `crates/lucerna-ipc/tests/server.rs`

**Interfaces:**
- Consumes: Tarefas 1 e 4.
- Produces:
  - `pub enum TopicKind { Common, Stream }`.
  - `#[derive(Clone)] pub struct Topic` com `name(&self) -> &str`, `kind(&self) -> TopicKind`, `publish(&self, data: Value)` (soma 1 ao `seq`), `current(&self) -> State`, `subscribers(&self) -> usize`, `async fn wait_subscribed(&self)` (volta quando `subscribers() > 0`), `async fn wait_unsubscribed(&self)` (quando `== 0`).
  - `pub struct ServerBuilder` com `pub fn new(app: &str, version: &str) -> Self`, `pub fn topic(&mut self, name: &str, initial: Value) -> Topic`, `pub fn stream(&mut self, name: &str) -> Topic` (estado inicial `null`), `pub fn method<F, Fut>(&mut self, name: &str, f: F)` com `F: Fn(Option<Value>) -> Fut + Send + Sync + 'static`, `Fut: Future<Output = Result<Value, RpcError>> + Send + 'static`, `pub fn max_message(&mut self, bytes: usize)`, `pub fn build(self) -> Server`. `method`/`topic`/`stream` entram em pânico com nome reservado (`lucerna.`) ou repetido (erro de programação).
  - `#[derive(Clone)] pub struct Server` com `pub async fn serve(&self, listener: UnixListener) -> io::Result<()>` (laço de `accept` com as conexões num `JoinSet`: largar o future fecha todas), `pub async fn serve_connection<S: AsyncRead + AsyncWrite + Send + 'static>(&self, stream: S)`, `pub fn connections(&self) -> watch::Receiver<usize>`, `pub fn hello_result(&self) -> HelloResult`.
  - `raw`: `pub struct RawConn` com `pub async fn connect(path: &Path) -> io::Result<RawConn>`, `pub async fn send(&mut self, v: &Value) -> io::Result<()>`, `pub async fn send_raw(&mut self, bytes: &[u8]) -> io::Result<()>`, `pub async fn recv(&mut self) -> io::Result<Option<Value>>` (espera até 5 s; estourou → `ErrorKind::TimedOut`), `pub async fn hello(&mut self, app: &str) -> io::Result<Value>` (manda `{"jsonrpc":"2.0","id":0,"method":"lucerna.hello","params":{"protocol":"1.0","client":"test","app":app}}` e devolve a resposta inteira).

Comportamento da conexão (o que os testes fixam): `lucerna.*` tratado em linha, na ordem; métodos do app numa tarefa por pedido (pânico → `-32603`); escritor único por conexão com `tokio::select! { biased; }` dando prioridade às respostas e depois ao mapa pendente (`topic → State`, um por tópico) alimentado por uma tarefa por assinatura que espera `watch::Receiver::changed()`; ao assinar, o retrato vem de `borrow_and_update()` e entra na fila de respostas antes de a tarefa da assinatura nascer (nenhum `lucerna.state` chega antes da resposta). Fechar a conexão aborta as tarefas da conexão e devolve os assinantes.

- [ ] **Passo 1: testes que falham.** Ajudante do arquivo: `async fn start(b: ServerBuilder) -> (TempDir, PathBuf, JoinHandle<()>)` que faz `bind` em `<tmp>/t.sock` e roda `serve`. O builder padrão `demo()` tem tópico `counter` (`{"value":0}`), stream `ticks`, método `counter.add` (soma `params.n`, publica e devolve `{"value":v}`), `slow.op` (dorme 200 ms e devolve `{}`), `boom` (`panic!`) e `app.fail` (`Err(RpcError::app(1, "Falhou."))`).

```rust
#[tokio::test]
async fn hello_answers_with_topics_streams_and_methods() {
    let (_t, p, _s) = start(demo()).await;
    let mut c = RawConn::connect(&p).await.unwrap();
    let r = c.hello("demo").await.unwrap();
    assert_eq!(r["result"]["protocol"], "1.0");
    assert_eq!(r["result"]["app"], "demo");
    assert_eq!(r["result"]["topics"], json!(["counter"]));
    assert_eq!(r["result"]["streams"], json!(["ticks"]));
    assert!(r["result"]["methods"].as_array().unwrap().contains(&json!("counter.add")));
}

#[tokio::test]
async fn incompatible_major_gets_32010_and_closes() {
    let (_t, p, _s) = start(demo()).await;
    let mut c = RawConn::connect(&p).await.unwrap();
    c.send(&json!({"jsonrpc":"2.0","id":1,"method":"lucerna.hello","params":{"protocol":"2.0","client":"panel","app":"demo"}})).await.unwrap();
    assert_eq!(c.recv().await.unwrap().unwrap()["error"]["code"], -32010);
    assert!(c.recv().await.unwrap().is_none());
}

#[tokio::test]
async fn compatible_minor_is_accepted() { /* hello com "1.7" → result.protocol == "1.0" */ }

#[tokio::test]
async fn hello_for_another_app_is_invalid_params() { /* app "routine" → -32602, conexão continua */ }

#[tokio::test]
async fn anything_before_hello_gets_32011() {
    let (_t, p, _s) = start(demo()).await;
    let mut c = RawConn::connect(&p).await.unwrap();
    c.send(&json!({"jsonrpc":"2.0","id":2,"method":"counter.add","params":{"n":1}})).await.unwrap();
    let r = c.recv().await.unwrap().unwrap();
    assert_eq!((r["id"].clone(), r["error"]["code"].clone()), (json!(2), json!(-32011)));
    c.hello("demo").await.unwrap(); // a conexão continua usável
}

#[tokio::test]
async fn request_response_and_errors() {
    // após hello: counter.add {"n":2} → result {"value":2}; "nada.disso" → -32601;
    // app.fail → error {"code":1,"message":"Falhou."}; "{oops" cru → -32700 com id null e a conexão segue.
}

#[tokio::test]
async fn subscribe_snapshot_then_changes_with_growing_seq() {
    let (_t, p, _s) = start(demo()).await;
    let mut c = RawConn::connect(&p).await.unwrap();
    c.hello("demo").await.unwrap();
    c.send(&json!({"jsonrpc":"2.0","id":3,"method":"lucerna.subscribe","params":{"topics":["counter"]}})).await.unwrap();
    let r = c.recv().await.unwrap().unwrap();
    assert_eq!(r["result"]["states"], json!([{"topic":"counter","seq":0,"data":{"value":0}}]));
    c.send(&json!({"jsonrpc":"2.0","id":4,"method":"counter.add","params":{"n":5}})).await.unwrap();
    let mut got = vec![c.recv().await.unwrap().unwrap(), c.recv().await.unwrap().unwrap()];
    got.sort_by_key(|m| m.get("method").is_some());
    assert_eq!(got[0]["result"], json!({"value":5}));
    assert_eq!(got[1]["method"], "lucerna.state");
    assert_eq!(got[1]["params"], json!({"topic":"counter","seq":1,"data":{"value":5}}));
}

#[tokio::test]
async fn unknown_topic_is_32012_and_nothing_is_subscribed() { /* ["counter","x"] → -32012; um add depois não gera lucerna.state */ }

#[tokio::test]
async fn unsubscribe_stops_updates() { /* subscribe, unsubscribe → result {} ; add → só a resposta chega (recv seguinte dá TimedOut com timeout curto de 300 ms via tokio::time::timeout) */ }

#[tokio::test]
async fn two_clients_see_the_same_changes() { /* A e B assinam counter; A chama add 1; os dois recebem seq 1 value 1 */ }

#[tokio::test]
async fn slow_client_gets_only_the_latest() {
    let mut b = ServerBuilder::new("demo", "0.1.0");
    let ticks = b.stream("ticks");
    let server = b.build();
    let (client, srv) = tokio::io::duplex(256); // buffer minúsculo: o escritor trava logo
    tokio::spawn({ let s = server.clone(); async move { s.serve_connection(srv).await } });
    let (rd, mut wr) = tokio::io::split(client);
    let mut rd = LineReader::new(rd, MAX_MESSAGE);
    for (id, m, p) in [(0, "lucerna.hello", json!({"protocol":"1.0","client":"t","app":"demo"})),
                       (1, "lucerna.subscribe", json!({"topics":["ticks"]}))] {
        write_message(&mut wr, &Message::Request(Request { id: Id::Num(id), method: m.into(), params: Some(p) })).await.unwrap();
    }
    ticks.wait_subscribed().await;
    for k in 1..=1000 { ticks.publish(json!({"tick": k})); tokio::task::yield_now().await; }
    let mut seqs = vec![];
    while let Ok(Ok(Some(line))) = tokio::time::timeout(Duration::from_millis(300), rd.next_line()).await {
        let v: Value = serde_json::from_str(&line).unwrap();
        if v["method"] == "lucerna.state" { seqs.push(v["params"]["seq"].as_u64().unwrap()); }
    }
    assert_eq!(*seqs.last().unwrap(), 1000, "o último sempre chega");
    assert!(seqs.len() < 20, "coalescido, recebeu {}", seqs.len());
    assert!(seqs.windows(2).all(|w| w[0] < w[1]));
}

#[tokio::test]
async fn stream_counts_subscribers() { /* subscribers() 0 → assina → 1 → unsubscribe → 0; wait_subscribed/wait_unsubscribed voltam */ }

#[tokio::test]
async fn abrupt_close_releases_subscriptions() {
    // Review Focus 1: assina ticks, manda slow.op e larga a conexão sem ler;
    // ticks.wait_unsubscribed() volta em < 1 s; connections() volta a 0; um cliente novo ainda é atendido.
}

#[tokio::test]
async fn handler_panic_becomes_internal_error() {
    // Review Focus 2: "boom" → -32603 "Erro interno do serviço."; na mesma conexão counter.add ainda responde.
}

#[tokio::test]
async fn too_large_message_gets_32000_and_closes() {
    // builder com max_message(1000); send_raw de 2000 bytes + \n → error -32000, depois recv → None.
}

#[test]
#[should_panic(expected = "reservado")]
fn reserved_method_names_are_refused() { ServerBuilder::new("d", "0").method("lucerna.x", |_| async { Ok(Value::Null) }); }
```

Os testes com corpo em comentário seguem o mesmo molde dos vizinhos; o comentário fixa a entrada e as asserções.

- [ ] **Passo 2: rodar e ver falhar.** `cargo test -p lucerna-ipc --test server` → erro de compilação.
- [ ] **Passo 3: implementar** `topic.rs` (`watch::Sender<State>` + `watch::Sender<usize>` de assinantes), `server.rs` e `raw.rs` como descrito. Mensagens de pânico do builder contêm "reservado" / "repetido".
- [ ] **Passo 4: verificar.** `cargo test -p lucerna-ipc` → todos passam (framing + 16 do servidor); rode 5 vezes seguidas (`for i in 1 2 3 4 5; do cargo test -p lucerna-ipc -q || break; done`) sem falha intermitente.
- [ ] **Passo 5: commit.** `git commit -m "IPC: servidor com aperto de mão, métodos e assinaturas coalescidas"`

---

### Tarefa 6: `lucerna-ipc` — cliente

**Files:**
- Create: `crates/lucerna-ipc/src/client.rs`
- Test: `crates/lucerna-ipc/tests/client.rs`

**Interfaces:**
- Consumes: Tarefas 1, 4 e 5.
- Produces:
  - `pub enum ClientError { Connect(io::Error), Io(io::Error), Rpc(RpcError), Closed }` (`thiserror`; `Connect` = não abriu o socket; `Rpc` com `-32010` no `hello` também é "sem falar com o serviço" para a CLI).
  - `pub struct Client` com `pub async fn connect(path: &Path, client: &str, app: &str) -> Result<Client, ClientError>` (faz o `hello` com `PROTOCOL_VERSION`), `pub fn hello(&self) -> &HelloResult`, `pub async fn call(&self, method: &str, params: Option<Value>) -> Result<Value, ClientError>`, `pub async fn notify(&self, method: &str, params: Option<Value>) -> Result<(), ClientError>`, `pub async fn subscribe(&self, topics: &[&str]) -> Result<Vec<State>, ClientError>`, `pub async fn unsubscribe(&self, topics: &[&str]) -> Result<(), ClientError>`, `pub async fn next_state(&mut self) -> Option<State>` (`None` quando a conexão fecha), `pub async fn closed(&self)` (volta quando a conexão fecha).
  - Uma tarefa leitora despacha respostas por `id` (`oneshot`) e põe os `lucerna.state` num `mpsc` limitado (256; cheio → a leitora espera, e o servidor coalesce); descarta `seq <= último visto` por tópico (o retrato do `subscribe` atualiza o último visto).

- [ ] **Passo 1: testes que falham** (reusam o `demo()`/`start()` da Tarefa 5 copiados para `tests/common/mod.rs`, que os dois arquivos usam com `mod common;`):

```rust
#[tokio::test]
async fn connect_call_subscribe_and_follow() {
    let (_t, p, _s) = start(demo()).await;
    let mut c = Client::connect(&p, "tui", "demo").await.unwrap();
    assert_eq!(c.hello().topics, vec!["counter"]);
    let snap = c.subscribe(&["counter"]).await.unwrap();
    assert_eq!(snap[0].data, json!({"value":0}));
    assert_eq!(c.call("counter.add", Some(json!({"n":2}))).await.unwrap(), json!({"value":2}));
    let s = c.next_state().await.unwrap();
    assert_eq!((s.topic.as_str(), s.seq, s.data.clone()), ("counter", 1, json!({"value":2})));
}

#[tokio::test]
async fn concurrent_calls_get_their_own_answers() {
    // 20 chamadas counter.add {"n":1} em paralelo (join_all via JoinSet) num Client compartilhado por Arc;
    // os 20 resultados distintos são exatamente {1..=20}.
}

#[tokio::test]
async fn rpc_errors_and_missing_socket() {
    // call("app.fail") → ClientError::Rpc com code 1; connect em caminho inexistente → ClientError::Connect;
    // connect com app "outro" → ClientError::Rpc com code -32602.
}

#[tokio::test]
async fn closed_resolves_when_server_goes_away() {
    // aborta o JoinHandle do serve; c.closed() volta em < 1 s; next_state() → None; call → Err(Closed ou Io).
}

#[tokio::test]
async fn out_of_order_seq_is_dropped() {
    // servidor falso com RawConn do outro lado (UnixListener à mão): responde ao hello e ao subscribe com seq 5,
    // depois manda lucerna.state seq 4 e seq 6; next_state() devolve só o 6.
}
```

- [ ] **Passo 2: rodar e ver falhar.** `cargo test -p lucerna-ipc --test client` → erro de compilação.
- [ ] **Passo 3: implementar** o `Client` (escrita atrás de `tokio::sync::Mutex`, ids `AtomicI64` a partir de 1, `closed` por `watch<bool>`).
- [ ] **Passo 4: verificar.** `cargo test -p lucerna-ipc` → tudo passa.
- [ ] **Passo 5: commit.** `git commit -m "IPC: cliente com chamadas concorrentes e estados em ordem"`

---

### Tarefa 7: `lucerna-service` — ativação, ociosidade e o serviço

**Files:**
- Create: `crates/lucerna-service/{Cargo.toml,src/lib.rs,src/activation.rs,src/idle.rs,src/service.rs}`
- Test: `crates/lucerna-service/tests/{activation,idle,service}.rs`

**Interfaces:**
- Consumes: `lucerna_ipc::{ServerBuilder, Server}`; `lucerna_paths::{Env, AppPaths, create_private_dir, shell_socket}`.
- Produces:
  - `activation`: `pub fn listen_fds(listen_pid: Option<&str>, listen_fds: Option<&str>, own_pid: u32) -> usize` (0 se o pid não bate ou falta algo); `pub fn listener_from_fd(fd: OwnedFd) -> io::Result<std::os::unix::net::UnixListener>` (reabre com `try_clone()` para ter `CLOEXEC`, confere `local_addr()`, põe não bloqueante); `pub enum Activation { Systemd, Manual(SocketGuard) }` (`SocketGuard` apaga o arquivo ao cair); `pub fn bind_manual(socket: &Path) -> Result<(UnixListener, SocketGuard), ServiceError>` (cria a pasta `0700`; arquivo existente: se um `connect` funciona → `ServiceError::AlreadyRunning(path)`, senão apaga e faz `bind`); `pub fn listener_from_env(paths: &AppPaths) -> Result<(tokio::net::UnixListener, Activation), ServiceError>` (lê `LISTEN_PID`/`LISTEN_FDS`, com `1` usa o fd 3 — único `unsafe`, `OwnedFd::from_raw_fd(3)`; com mais de 1 → erro).
  - `idle`: `#[derive(Clone)] pub struct WorkTracker` com `new()`, `hold(&self) -> WorkGuard` (RAII), `pending(&self) -> usize`, `watch(&self) -> watch::Receiver<usize>`; `pub async fn wait_idle(connections: watch::Receiver<usize>, work: watch::Receiver<usize>, timeout: Duration)` — volta quando os dois ficam em 0 por `timeout` seguidos.
  - `service`: `pub const DEFAULT_IDLE: Duration = Duration::from_secs(60);` `pub struct ServiceBuilder` com `pub fn new(app: &str, version: &str) -> Self`, `pub fn server(&mut self) -> &mut ServerBuilder`, `pub fn idle_timeout(&mut self, d: Duration)`, `pub fn work(&self) -> WorkTracker`, `pub async fn run(self) -> Result<(), ServiceError>` (`Env::from_process()`, `LUCERNA_IDLE_TIMEOUT_MS` se houver, `listener_from_env`, `shell_socket`; sai em `Ok(())` por ociosidade ou `SIGTERM`/`SIGINT`), `pub async fn run_with(self, listener: tokio::net::UnixListener, shell_socket: Option<PathBuf>) -> Result<(), ServiceError>` (nesta tarefa `shell_socket` é ignorado; a Tarefa 8 o liga).
  - `pub enum ServiceError { Paths(PathsError), AlreadyRunning(PathBuf), Activation(String), Io(io::Error) }` — texto de `AlreadyRunning`: `Já há um serviço atendendo em {path}.`

- [ ] **Passo 1: testes que falham.**

```rust
// activation.rs
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
    // Review Focus 3: <tmp>/run/lucerna/hello.sock criado por um UnixListener já largado (arquivo velho);
    // bind_manual funciona; a pasta tem modo 0700; largar o SocketGuard apaga o arquivo.
}

#[tokio::test]
async fn manual_bind_refuses_a_live_service() {
    // Review Focus 3: um listener vivo no caminho → Err(ServiceError::AlreadyRunning(p)),
    // to_string() == format!("Já há um serviço atendendo em {}.", p.display()); o arquivo continua lá.
}

// idle.rs — #[tokio::test(start_paused = true)] em todos (tokio "test-util" nas dev-dependencies)
#[tokio::test(start_paused = true)]
async fn idle_after_timeout_with_nobody() {
    let (_c, cr) = watch::channel(0usize);
    let w = WorkTracker::new();
    let t0 = Instant::now();
    wait_idle(cr, w.watch(), Duration::from_secs(60)).await;
    assert_eq!(t0.elapsed(), Duration::from_secs(60));
}

#[tokio::test(start_paused = true)]
async fn a_client_resets_the_clock() {
    // conexões 1 até t=30 s, 0 em t=30, 1 em t=40, 0 em t=50 → wait_idle volta em t=110 s.
}

#[tokio::test(start_paused = true)]
async fn pending_work_holds_the_service() {
    // guard = w.hold(); wait_idle não volta em 10 min (timeout externo); drop(guard) em t=10 min → volta em t=11 min.
}

// service.rs
#[tokio::test]
async fn run_with_serves_then_exits_when_idle() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("s.sock");
    let mut b = ServiceBuilder::new("demo", "0.1.0");
    b.idle_timeout(Duration::from_millis(200));
    b.server().method("ping", |_| async { Ok(json!("pong")) });
    let l = tokio::net::UnixListener::bind(&p).unwrap();
    let h = tokio::spawn(b.run_with(l, None));
    let c = lucerna_ipc::Client::connect(&p, "test", "demo").await.unwrap();
    assert_eq!(c.call("ping", None).await.unwrap(), json!("pong"));
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(!h.is_finished(), "não sai com cliente conectado");
    drop(c);
    tokio::time::timeout(Duration::from_secs(2), h).await.unwrap().unwrap().unwrap();
}
```

- [ ] **Passo 2: rodar e ver falhar.** `cargo test -p lucerna-service` → erro de compilação.
- [ ] **Passo 3: implementar** as Interfaces; `run_with` faz `select!` entre `server.serve(listener)`, `wait_idle(...)` e os sinais.
- [ ] **Passo 4: verificar.** `cargo test -p lucerna-service` → 9 passed.
- [ ] **Passo 5: commit.** `git commit -m "Serviço: ativação pelo systemd, socket manual e saída por ociosidade"`

---

### Tarefa 8: `lucerna-service` — conexão ao shell com reconexão

**Files:**
- Create: `crates/lucerna-service/src/{shell,testing}.rs`; Modify: `crates/lucerna-service/src/service.rs` (`ServiceBuilder::shell(&self) -> Shell`, criado no `new` com o `WorkTracker` do builder; `run_with` chama `shell.spawn(path)` quando `shell_socket` é `Some` e segura o `ShellTask` até sair)
- Test: `crates/lucerna-service/tests/shell.rs`

**Interfaces:**
- Consumes: `lucerna_ipc::{Client, ServerBuilder, Server}`; `lucerna_protocol::{BarItem, methods::*}`; `WorkTracker`.
- Produces:
  - `#[derive(Clone)] pub struct Shell` com `pub fn new(app: &str, work: WorkTracker) -> Shell`, `pub fn spawn(&self, socket: PathBuf) -> ShellTask` (`ShellTask` aborta ao cair), `bar_set(&self, item: BarItem)`, `bar_clear(&self, id: &str)`, `dnd_request(&self, reason: &str)`, `dnd_release(&self)`, `tui_open(&self, args: Vec<String>)`, `panel_open(&self, page: Option<String>)`, `is_connected(&self) -> bool`. Nenhum método é `async` nem devolve erro.
  - Estado desejado: `BTreeMap<String, BarItem>` + `Option<String>` (motivo do não perturbe). A tarefa de conexão: conecta (`client` = id do app, `app` = `"shell"`), aplica o desejado inteiro (`bar.set` de cada item, `dnd.request` se houver), depois a cada mudança manda só a diferença em relação ao aplicado (`bar.set` novo/alterado, `bar.clear` removido, `dnd.request`/`dnd.release` na troca); caiu → espera `backoff(tentativa)` e tenta de novo (zera a tentativa ao conectar). `tui_open`/`panel_open` só vão se conectado; senão somem.
  - Enquanto houver item de barra desejado, o `Shell` segura um `WorkGuard`.
  - `pub fn backoff(attempt: u32) -> Duration` = `min(100 ms × 2^attempt, 5 s)`.
  - `testing::FakeShell` com `pub async fn start(path: &Path) -> FakeShell` (um `Server` do `lucerna-ipc` com `app = "shell"` e os seis métodos, cada um registrando `(method, params)` e devolvendo `{}`), `pub async fn next_call(&mut self) -> (String, Value)` (até 5 s, senão pânico), `pub async fn no_call_within(&mut self, d: Duration)` (pânico se chegar algo), `pub async fn wait_connected(&self)`, `pub async fn wait_disconnected(&self)`, `pub async fn stop(self)` (fecha as conexões e apaga o socket).

- [ ] **Passo 1: testes que falham.**

```rust
fn item(text: &str) -> BarItem { BarItem { id: "c".into(), icon: "tag".into(), text: Some(text.into()), ends_at: None, panel: false } }

#[test]
fn backoff_grows_and_caps() {
    let ms = |a| backoff(a).as_millis();
    assert_eq!((ms(0), ms(1), ms(2), ms(5), ms(6), ms(30)), (100, 200, 400, 3200, 5000, 5000));
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
    assert_eq!(fake.next_call().await, ("bar.set".into(), json!({"id":"c","icon":"tag","text":"3","panel":false})));
    shell.dnd_request("foco");
    assert_eq!(fake.next_call().await, ("dnd.request".into(), json!({"reason":"foco"})));
    shell.bar_clear("c");
    assert_eq!(fake.next_call().await, ("bar.clear".into(), json!({"id":"c"})));
}

#[tokio::test]
async fn without_shell_calls_are_silent() {
    // socket inexistente: bar_set, dnd_request, tui_open não entram em pânico; is_connected() == false.
}

#[tokio::test]
async fn reconnect_reapplies_desired_state() {
    // fake A; bar_set("3") + dnd_request("foco") chegam; fake.stop(); (o shell desfaria tudo aqui)
    // fake B no mesmo caminho: em até 6 s chegam bar.set text "3" e dnd.request "foco" (em qualquer ordem), nada mais.
}

#[tokio::test]
async fn changes_while_offline_collapse_to_the_latest() {
    // Review Focus 5: com o shell fora: bar_set("1"), bar_set("2"), bar_set("3"); bar_set de id "x" e bar_clear("x");
    // dnd_request("a") e dnd_release(). Sobe o fake: chega exatamente um bar.set (id "c", text "3"); no_call_within(500 ms).
}

#[tokio::test]
async fn tui_open_is_dropped_while_offline() {
    // shell fora: tui_open(vec!["t_42".into()]); sobe o fake; no_call_within(500 ms) depois do hello.
    // conectado: panel_open(Some("hoje".into())) → ("panel.open", {"page":"hoje"}).
}

#[tokio::test]
async fn bar_items_count_as_pending_work() {
    let w = WorkTracker::new();
    let shell = Shell::new("demo", w.clone()); // sem spawn: o shell nem existe
    shell.bar_set(item("1"));
    assert_eq!(w.pending(), 1);
    shell.bar_set(BarItem { id: "d".into(), ..item("2") });
    assert_eq!(w.pending(), 1);
    shell.bar_clear("c"); shell.bar_clear("d");
    assert_eq!(w.pending(), 0);
}

#[tokio::test]
async fn shell_sees_the_connection_close_when_the_service_ends() {
    // fake; ServiceBuilder com idle 200 ms e run_with(listener, Some(shell_path)); fake.wait_connected();
    // a tarefa do serviço termina por ociosidade (sem itens na barra) → fake.wait_disconnected() volta.
}
```

- [ ] **Passo 2: rodar e ver falhar.** `cargo test -p lucerna-service --test shell` → erro de compilação.
- [ ] **Passo 3: implementar** `shell.rs` (estado desejado num `watch::Sender<Desired>`; a tarefa compara com o aplicado) e `testing.rs`.
- [ ] **Passo 4: verificar.** `cargo test -p lucerna-service` → tudo passa; repetir 5 vezes sem falha intermitente.
- [ ] **Passo 5: commit.** `git commit -m "Serviço: barra e não perturbe no shell, com reconexão"`

---

### Tarefa 9: `lucerna-service` — timers agendados

**Files:**
- Create: `crates/lucerna-service/src/schedule.rs`; Modify: `crates/lucerna-service/src/testing.rs` (`FakeScheduler`)
- Test: `crates/lucerna-service/tests/schedule.rs`

**Interfaces:**
- Consumes: `lucerna_paths::{Env, validate_app_id}`.
- Produces:
  - `pub struct TimerSpec { pub name: String, pub at_unix: u64, pub persistent: bool }`.
  - `pub trait Scheduler: Send + Sync { fn schedule(&self, spec: &TimerSpec) -> Result<(), ScheduleError>; fn cancel(&self, name: &str) -> Result<(), ScheduleError>; }`
  - `pub enum ScheduleError { InvalidName(String), InPast, NoDir(PathsError), Io(io::Error), Systemctl { args: String, status: Option<i32>, stderr: String } }`. Nome válido: `^[a-z0-9][a-z0-9-]{0,31}$`.
  - `pub fn calendar_utc(at_unix: u64) -> String` (`"AAAA-MM-DD HH:MM:SS UTC"`, algoritmo `civil_from_days` de Howard Hinnant).
  - `pub fn timer_unit_name(app: &str, name: &str) -> String` = `lucerna-<app>-<name>.timer`.
  - `pub fn render_timer(app: &str, spec: &TimerSpec) -> String` — exatamente:

```ini
[Unit]
Description=Lucerna: <app> (<name>)

[Timer]
OnCalendar=<calendar_utc(at_unix)>
AccuracySec=1s
Unit=lucerna-<app>.service
Persistent=true            ← só se persistent

[Install]                  ← só se persistent
WantedBy=timers.target     ← só se persistent
```

  - `pub struct SystemdScheduler` com `pub fn new(app: &str, env: &Env) -> Result<SystemdScheduler, ScheduleError>` e `pub fn with_systemctl(self, program: PathBuf) -> Self` (padrão `"systemctl"`). `schedule`: confere nome e `at_unix > agora` (`InPast`), escreve o `.timer` (persistente em `$XDG_CONFIG_HOME/systemd/user/`, senão em `$XDG_RUNTIME_DIR/systemd/user/`; cria a pasta), roda `systemctl --user daemon-reload` e depois `systemctl --user enable --now <unit>` (persistente) ou `systemctl --user restart <unit>` (não persistente; `restart` para reagendar um que já existe). `cancel`: `systemctl --user disable --now <unit>` (falha ignorada: pode não existir), apaga os dois arquivos possíveis, `daemon-reload`.
  - `pub struct FakeScheduler` (em `testing`): guarda `Mutex<BTreeMap<String, TimerSpec>>`, `pub fn timers(&self) -> BTreeMap<String, TimerSpec>`.
  - Doc do módulo: o timer sobe `lucerna-<app>.service`; o app confere o trabalho vencido ao subir e chama `cancel` depois de fazê-lo (um `OnCalendar` com data fixa não volta a disparar, mas o arquivo ficaria).

- [ ] **Passo 1: testes que falham.**

```rust
#[test]
fn calendar_values() {
    assert_eq!(calendar_utc(0), "1970-01-01 00:00:00 UTC");
    assert_eq!(calendar_utc(951_782_400), "2000-02-29 00:00:00 UTC");
    assert_eq!(calendar_utc(1_700_000_000), "2023-11-14 22:13:20 UTC");
}

#[test]
fn renders_persistent_and_runtime_timers() {
    let p = render_timer("routine", &TimerSpec { name: "lembrete".into(), at_unix: 1_700_000_000, persistent: true });
    assert!(p.contains("OnCalendar=2023-11-14 22:13:20 UTC\n"));
    assert!(p.contains("Unit=lucerna-routine.service\n"));
    assert!(p.contains("Persistent=true\n") && p.contains("WantedBy=timers.target\n"));
    let r = render_timer("routine", &TimerSpec { name: "x".into(), at_unix: 0, persistent: false });
    assert!(!r.contains("Persistent") && !r.contains("[Install]"));
}

fn fake_systemctl(dir: &Path) -> (PathBuf, PathBuf) {
    let log = dir.join("log");
    let bin = dir.join("systemctl");
    std::fs::write(&bin, format!("#!/bin/sh\necho \"$@\" >> '{}'\n", log.display())).unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    (bin, log)
}

fn env_in(dir: &Path) -> Env {
    Env { home: Some(dir.into()), xdg_config_home: Some(dir.join("config")), xdg_runtime_dir: Some(dir.join("run")), ..Env::default() }
}

#[test]
fn persistent_schedule_writes_config_unit_and_enables() {
    let t = tempfile::tempdir().unwrap();
    let (bin, log) = fake_systemctl(t.path());
    let s = SystemdScheduler::new("routine", &env_in(t.path())).unwrap().with_systemctl(bin);
    s.schedule(&TimerSpec { name: "lembrete".into(), at_unix: now() + 3600, persistent: true }).unwrap();
    assert!(t.path().join("config/systemd/user/lucerna-routine-lembrete.timer").exists());
    assert_eq!(std::fs::read_to_string(&log).unwrap(),
        "--user daemon-reload\n--user enable --now lucerna-routine-lembrete.timer\n");
}

#[test]
fn runtime_schedule_and_cancel() {
    // persistent false → arquivo em run/systemd/user/, log "--user daemon-reload\n--user restart <unit>\n";
    // cancel("lembrete") → arquivo apagado; log termina com "--user disable --now <unit>\n--user daemon-reload\n".
}

#[test]
fn refuses_past_times_and_bad_names() {
    // at_unix 1 → Err(InPast); name "Lembrete" ou "a/b" → Err(InvalidName); nada escrito, log inexistente.
}

#[test]
fn systemctl_failure_is_reported() {
    // script que faz `exit 3` → Err(ScheduleError::Systemctl { status: Some(3), .. }).
}

#[test]
fn fake_scheduler_records() { /* schedule duas vezes o mesmo nome → timers() tem o último; cancel → vazio */ }

#[test]
#[ignore = "precisa de um systemd --user de verdade"]
fn real_systemd_round_trip() {
    // SystemdScheduler::new("sdktest", &Env::from_process()); schedule não persistente para daqui a 1 h;
    // `systemctl --user list-timers --all` contém "lucerna-sdktest-teste.timer"; cancel; não contém mais.
}
```

- [ ] **Passo 2: rodar e ver falhar.** `cargo test -p lucerna-service --test schedule` → erro de compilação.
- [ ] **Passo 3: implementar** (`std::process::Command`, sem `tokio::process`: chamadas raras e curtas; doc avisa para chamar de `spawn_blocking` se o app quiser).
- [ ] **Passo 4: verificar.** `cargo test -p lucerna-service --test schedule` → 7 passed, 1 ignored. Na máquina do usuário (não no CI): `cargo test -p lucerna-service --test schedule -- --ignored` → 1 passed.
- [ ] **Passo 5: commit.** `git commit -m "Serviço: timers agendados do systemd do usuário"`

---

### Tarefa 10: `lucerna-cli`

**Files:**
- Create: `crates/lucerna-cli/{Cargo.toml,src/lib.rs,src/render.rs}`
- Test: `crates/lucerna-cli/tests/cli.rs`

**Interfaces:**
- Consumes: `lucerna_ipc::{Client, ClientError}`; `lucerna_paths::{Env, AppPaths}`; `clap::{Command, ArgMatches}`.
- Produces:
  - `pub const EXIT_OK: i32 = 0; EXIT_APP: i32 = 1; EXIT_NO_SERVICE: i32 = 2; EXIT_USAGE: i32 = 64;`
  - `pub struct Call { pub method: String, pub params: Option<Value> }`.
  - `pub struct CliCommand { pub command: clap::Command, pub to_call: fn(&ArgMatches) -> Call, pub render: Option<fn(&Value) -> String> }`.
  - `pub struct CliApp` com `pub fn new(app: &str, bin: &str, about: &str) -> Self`, `pub fn command(self, c: CliCommand) -> Self` (pânico com nome `status`, `watch`, `serve` ou repetido), `pub fn clap(&self) -> clap::Command` (os do app + `status` "Mostra o estado de todos os tópicos." + `watch <tópico>` "Acompanha as mudanças de um tópico." + `serve` oculto + `--json` global "Imprime o resultado em JSON, para scripts."), `pub fn parse_from<I, T>(&self, args: I) -> Result<Invocation, clap::Error>` (`I: IntoIterator<Item = T>, T: Into<OsString> + Clone`), `pub async fn run(&self, run: CliRun, env: &Env, out: &mut (dyn Write + Send), err: &mut (dyn Write + Send)) -> i32`.
  - `pub enum Invocation { Tui, Serve, Cli(CliRun) }`; `pub struct CliRun { .. }` (opaco: o subcomando escolhido e `json`).
  - `pub fn usage_exit(e: &clap::Error) -> i32` (imprime com `e.print()`; `0` para ajuda/versão, `64` para o resto).
  - `render::render_default(v: &Value) -> String` (regras da tabela de decisões; objeto em ordem de chave, valores aninhados em JSON compacto).
  - Saídas fixas: sem serviço → stderr `Sem conexão com o serviço {app}: {detalhe}` e `2`; erro do app → stderr `erro: {message}` e `1` (com `--json`: stderr `{"error":{...}}`); `status` legível → para cada tópico, `<tópico>:` e as linhas do `render_default` com dois espaços na frente; `watch` legível → `<tópico> #<seq>` e as linhas indentadas, um bloco por estado, com `flush` a cada um.

- [ ] **Passo 1: testes que falham.** Ajudante: `async fn service(dir) -> (Env, Topic, JoinHandle)` sobe um `ServerBuilder::new("demo", "0.1.0")` com tópico `counter` e métodos `counter.add` e `app.fail` em `<dir>/run/lucerna/demo.sock`, e devolve `Env { xdg_runtime_dir: Some(<dir>/run), home: Some(<dir>), .. }`. `fn app() -> CliApp` declara `add <n>` (`value_parser!(i64)`, ajuda "Soma ao contador.") → `counter.add {"n": n}` com `render` `|v| format!("Contador: {}", v["value"])`, e `fail` → `app.fail`. `async fn run(args: &[&str], env) -> (i32, String, String)`.

```rust
#[test]
fn parse_dispatch() {
    let a = app();
    assert!(matches!(a.parse_from(["demo"]).unwrap(), Invocation::Tui));
    assert!(matches!(a.parse_from(["demo", "serve"]).unwrap(), Invocation::Serve));
    assert!(matches!(a.parse_from(["demo", "add", "3"]).unwrap(), Invocation::Cli(_)));
    let e = a.parse_from(["demo", "add", "três"]).unwrap_err();
    assert_eq!(usage_exit(&e), 64);
    assert_eq!(usage_exit(&a.parse_from(["demo", "--help"]).unwrap_err()), 0);
    assert!(!a.clap().render_help().to_string().contains("serve"));
}

#[tokio::test]
async fn call_readable_and_json() {
    let t = tempfile::tempdir().unwrap();
    let (env, _topic, _s) = service(t.path()).await;
    assert_eq!(run(&["demo", "add", "3"], &env).await, (0, "Contador: 3\n".into(), String::new()));
    let (code, out, _) = run(&["demo", "--json", "add", "2"], &env).await;
    assert_eq!(code, 0);
    assert_eq!(serde_json::from_str::<Value>(&out).unwrap(), json!({"value":5}));
}

#[tokio::test]
async fn app_error_exits_1() {
    // fail → (1, "", "erro: Falhou.\n"); --json fail → stderr parseia como {"error":{"code":1,"message":"Falhou."}}, stdout "".
}

#[tokio::test]
async fn no_service_exits_2() {
    // Env com runtime vazio → status → código 2, stderr começa com "Sem conexão com o serviço demo".
    // Review Focus 4: Env com xdg_runtime_dir None → código 2, stderr contém "XDG_RUNTIME_DIR não está definido."
}

#[tokio::test]
async fn status_prints_every_common_topic() {
    // status → (0, "counter:\n  value: 0\n", ""); --json status → {"counter":{"value":0}}.
}

#[tokio::test]
async fn watch_follows_until_the_service_goes_away() {
    // run(["demo","--json","watch","counter"]) numa tarefa; topic.publish({"value":1}); aborta o serviço;
    // código 2; stdout tem 2 linhas JSON: seq 0 value 0 e seq 1 value 1. watch "x" → código 1 (Tópico desconhecido: x.).
}

#[test]
fn render_default_rules() {
    assert_eq!(render_default(&json!("oi")), "oi");
    assert_eq!(render_default(&json!(null)), "");
    assert_eq!(render_default(&json!({"b":1,"a":{"x":[1]}})), "a: {\"x\":[1]}\nb: 1");
    assert_eq!(render_default(&json!(["um","dois"])), "um\ndois");
}
```

- [ ] **Passo 2: rodar e ver falhar.** `cargo test -p lucerna-cli` → erro de compilação.
- [ ] **Passo 3: implementar.** `run` conecta com `client = "cli"`; `ClientError::Connect`, `Io`, `Closed` e `Rpc(-32010)` no connect → `2`; `Rpc` nas chamadas → `1`; `PathsError` → `2`.
- [ ] **Passo 4: verificar.** `cargo test -p lucerna-cli` → 7 passed.
- [ ] **Passo 5: commit.** `git commit -m "Linha de comando: subcomandos, status, watch, --json e códigos de saída"`

---

### Tarefa 11: `lucerna-hello` — o serviço, as unidades e o manifesto

**Files:**
- Create: `apps/lucerna-hello/Cargo.toml` (`publish = false`, `[[bin]] name = "lucerna-hello"`), `apps/lucerna-hello/src/lib.rs`, `apps/lucerna-hello/systemd/lucerna-hello.socket`, `apps/lucerna-hello/systemd/lucerna-hello.service`, `apps/lucerna-hello/data/hello.json`, `apps/lucerna-hello/README.md` (o molde: lib + bin, como instalar as unidades e o manifesto)
- Test: `apps/lucerna-hello/tests/{protocol,shell,files}.rs`

**Interfaces:**
- Consumes: `lucerna_service::{ServiceBuilder, Shell, testing::FakeShell}`, `lucerna_cli::*`, `lucerna_manifest::Manifest`, `lucerna_ipc::{Client, raw::RawConn}`.
- Produces:
  - `pub const APP: &str = "hello";`
  - `pub struct Hello { pub service: ServiceBuilder, pub counter: Topic, pub ticks: Topic }` e `pub fn hello() -> Hello` (chamado dentro do runtime: faz `tokio::spawn` do produtor de `ticks`) — tópico `counter` `{"value":0}`; stream `ticks`: uma tarefa que espera `wait_subscribed`, publica `{"tick":k,"bands":[64 × f32]}` a cada 1 ms (`MissedTickBehavior::Skip`) enquanto `subscribers() > 0`; métodos `counter.add {"n": i64}` (sem `n` inteiro → `-32602`; estouro → `RpcError::app(1, "O contador passou do limite.")`; publica e devolve `{"value":v}`), `counter.reset` (→ `{"value":0}`), `quiet.set {"on": bool}` (`dnd_request("Lucerna Hello")` / `dnd_release()`, devolve `{}`). Toda mudança do contador: valor ≠ 0 → `shell.bar_set(BarItem { id: "counter", icon: "exposure_plus_1", text: Some(v.to_string()), ends_at: None, panel: false })`; valor 0 → `bar_clear("counter")`.
  - `pub fn cli() -> CliApp` — `add <n>` (`counter.add`, "Soma n ao contador.", `render` → `Contador: {v}`), `reset` (`counter.reset`, "Zera o contador.", mesmo `render`), `quiet <on|off>` (`quiet.set {"on": arg == "on"}`, `value_parser(["on", "off"])`, "Liga ou desliga o não perturbe.").
  - Unidades:

```ini
# lucerna-hello.socket
[Unit]
Description=Lucerna Hello (socket)

[Socket]
ListenStream=%t/lucerna/hello.sock
SocketMode=0600
DirectoryMode=0700

[Install]
WantedBy=sockets.target
```

```ini
# lucerna-hello.service
[Unit]
Description=Lucerna Hello (serviço de exemplo)
Requires=lucerna-hello.socket
After=lucerna-hello.socket

[Service]
ExecStart=/usr/bin/lucerna-hello serve
Restart=on-failure
```

  - Manifesto `data/hello.json`: `{"schema":1,"app":{"id":"hello","name":"Olá","icon":"waving_hand","socket":"hello.sock"}}` (sem `panel`, `card` nem `tui`).

- [ ] **Passo 1: testes que falham.** `tests/common/mod.rs`: `async fn start(shell: Option<PathBuf>) -> Running` sobe `hello().service` com `run_with` num socket temporário (`idle_timeout` 60 s) numa tarefa; `Running` guarda a `TempDir`, o caminho, o `ticks` e o `JoinHandle`, com `raw()` (`RawConn` já com `hello("hello")`), `client()` (`Client::connect(.., "test", "hello")`), `ticks() -> &Topic` e `stop()` (aborta a tarefa e espera ela terminar).

```rust
// files.rs
#[test]
fn manifest_is_valid_and_has_no_panel() {
    let m = Manifest::from_json(include_str!("../data/hello.json")).unwrap();
    assert_eq!((m.app.id.as_str(), m.app.socket.as_str()), ("hello", "hello.sock"));
    assert!(m.panel.is_none() && m.card.is_none());
}

#[test]
fn units_match_the_paths_rules() {
    let s = include_str!("../systemd/lucerna-hello.socket");
    assert!(s.contains("ListenStream=%t/lucerna/hello.sock\n") && s.contains("DirectoryMode=0700\n"));
    let svc = include_str!("../systemd/lucerna-hello.service");
    assert!(svc.lines().any(|l| l.starts_with("ExecStart=") && l.ends_with(" serve")));
}

// protocol.rs (in-process, socket temporário)
#[tokio::test] async fn handshake() { /* raw hello → topics ["counter"], streams ["ticks"], methods contém counter.add, counter.reset, quiet.set; version == env!("CARGO_PKG_VERSION") */ }
#[tokio::test] async fn incompatible_version() { /* hello "2.0" → -32010 e a conexão fecha */ }
#[tokio::test] async fn request() {
    // counter.add {"n":3} → {"value":3}; {"n":"3"} → -32602; add i64::MAX duas vezes → error {"code":1,"message":"O contador passou do limite."}
}
#[tokio::test] async fn subscription_snapshot_and_changes() { /* Client: subscribe counter → seq 0 value 0; add 2 → next_state seq 1 value 2; reset → seq 2 value 0 */ }
#[tokio::test] async fn two_clients() { /* A assina; B chama add 1; A recebe value 1; B assina depois e recebe o retrato com value 1 */ }

#[tokio::test]
async fn slow_client_on_a_stream() {
    let r = start(None).await;
    let mut c = r.raw().await;
    c.send(&json!({"jsonrpc":"2.0","id":1,"method":"lucerna.subscribe","params":{"topics":["ticks"]}})).await.unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await; // não lê: ~2000 quadros produzidos
    let mut ticks = vec![];
    while let Ok(Ok(Some(v))) = tokio::time::timeout(Duration::from_millis(200), c.recv()).await {
        if v["method"] == "lucerna.state" { ticks.push(v["params"]["data"]["tick"].as_u64().unwrap()); }
    }
    assert!(ticks.windows(2).all(|w| w[0] < w[1]));
    assert!(ticks.windows(2).any(|w| w[1] - w[0] > 1), "houve descarte de quadros intermediários");
    // o produtor para sem assinante
    drop(c);
    tokio::time::timeout(Duration::from_secs(1), r.ticks().wait_unsubscribed()).await.unwrap();
}

// shell.rs (FakeShell)
#[tokio::test]
async fn bar_and_dnd_undone_on_close_and_reapplied() {
    let t = tempfile::tempdir().unwrap();
    let sp = t.path().join("shell.sock");
    let mut fake = FakeShell::start(&sp).await;
    let r = start(Some(sp.clone())).await;
    let c = r.client().await;
    c.call("counter.add", Some(json!({"n":3}))).await.unwrap();
    assert_eq!(fake.next_call().await, ("bar.set".into(), json!({"id":"counter","icon":"exposure_plus_1","text":"3","panel":false})));
    c.call("quiet.set", Some(json!({"on":true}))).await.unwrap();
    assert_eq!(fake.next_call().await, ("dnd.request".into(), json!({"reason":"Lucerna Hello"})));
    fake.stop().await;                       // shell cai: ele desfaria barra e não perturbe
    let mut fake = FakeShell::start(&sp).await; // shell volta
    let mut got = vec![fake.next_call().await, fake.next_call().await];
    got.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(got[0].0, "bar.set");
    assert_eq!(got[1].0, "dnd.request");
    r.stop().await;                          // serviço sai: o shell vê a conexão fechar
    fake.wait_disconnected().await;
}

#[tokio::test]
async fn works_without_a_shell() { /* start(Some(caminho inexistente)); add 1 → {"value":1}; nenhum erro */ }
```

- [ ] **Passo 2: rodar e ver falhar.** `cargo test -p lucerna-hello` → erro de compilação.
- [ ] **Passo 3: implementar** `lib.rs` e os arquivos; `main.rs` mínimo (`fn main() {}`) só para o bin compilar, a Tarefa 12 o completa.
- [ ] **Passo 4: verificar.** `cargo test -p lucerna-hello` → 10 passed.
- [ ] **Passo 5: commit.** `git commit -m "Hello: serviço de exemplo, unidades do systemd e manifesto"`

---

### Tarefa 12: `lucerna-hello` — o binário e os testes de processo

**Files:**
- Modify: `apps/lucerna-hello/src/main.rs`
- Test: `apps/lucerna-hello/tests/process.rs`

**Interfaces:**
- Consumes: `lucerna_hello::{hello, cli}`, `lucerna_cli::{Invocation, usage_exit}`, `lucerna_paths::Env`.
- Produces: o binário `lucerna-hello`. `main` (`#[tokio::main]`, devolve `ExitCode`): `Tui` → imprime a ajuda (`cli().clap().print_help()`) e `0`; `Serve` → `hello().service.run()`, erro → `erro: {e}` no stderr e `1`; `Cli(run)` → `cli().run(run, &Env::from_process(), stdout, stderr)`; erro de uso → `usage_exit`.

Ajudante dos testes: `fn hello(dir: &Path) -> Command` = `Command::new(env!("CARGO_BIN_EXE_lucerna-hello"))` com `env_clear()`, `HOME=<dir>`, `XDG_RUNTIME_DIR=<dir>/run` e `PATH` do processo; `fn serve(dir, idle_ms) -> Child` acrescenta `LUCERNA_IDLE_TIMEOUT_MS` e espera o socket `<dir>/run/lucerna/hello.sock` existir (até 5 s).

- [ ] **Passo 1: testes que falham.**

```rust
#[test]
fn cli_json_and_exit_codes() {
    let t = tempfile::tempdir().unwrap();
    let out = hello(t.path()).args(["status"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));                   // ainda sem serviço
    assert!(String::from_utf8_lossy(&out.stderr).starts_with("Sem conexão com o serviço hello"));
    let mut srv = serve(t.path(), 60_000);
    let out = hello(t.path()).args(["add", "3"]).output().unwrap();
    assert_eq!((out.status.code(), String::from_utf8(out.stdout).unwrap()), (Some(0), "Contador: 3\n".into()));
    let out = hello(t.path()).args(["--json", "status"]).output().unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&out.stdout).unwrap(), json!({"counter":{"value":3}}));
    let big = i64::MAX.to_string();
    hello(t.path()).args(["add", &big]).output().unwrap();
    let out = hello(t.path()).args(["add", &big]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(String::from_utf8(out.stderr).unwrap(), "erro: O contador passou do limite.\n");
    assert_eq!(hello(t.path()).args(["add", "três"]).output().unwrap().status.code(), Some(64));
    assert_eq!(hello(t.path()).output().unwrap().status.code(), Some(0)); // sem TUI: ajuda
    srv.kill().unwrap();
}

#[test]
fn watch_prints_json_lines() {
    // serve; spawn `--json watch counter` com stdout piped; lê a 1ª linha (seq 0, value 0); roda `add 2`;
    // 2ª linha: {"topic":"counter","seq":1,"data":{"value":2}}; mata o watch e o serviço.
}

#[test]
fn idle_exit() {
    let t = tempfile::tempdir().unwrap();
    let mut srv = serve(t.path(), 300);
    let mut watch = hello(t.path()).args(["watch", "counter"]).stdout(Stdio::null()).spawn().unwrap();
    std::thread::sleep(Duration::from_millis(800));
    assert!(srv.try_wait().unwrap().is_none(), "com cliente conectado não sai");
    watch.kill().unwrap(); watch.wait().unwrap();
    assert!(wait_exit(&mut srv, Duration::from_secs(3)).unwrap().success(), "sai sozinho e com 0");
    assert!(!t.path().join("run/lucerna/hello.sock").exists(), "o socket manual é apagado");
}

#[test]
fn bar_item_holds_the_service_even_without_shell() {
    // serve com idle 300 ms; add 1 → espera 1 s → ainda vivo; reset → sai em até 3 s.
}

#[test]
fn second_serve_is_refused() {
    // com um serve rodando, outro serve → código 1, stderr contém "Já há um serviço atendendo em".
}
```

`wait_exit(child, limite) -> Option<ExitStatus>` é um laço de `try_wait` a cada 20 ms.

- [ ] **Passo 2: rodar e ver falhar.** `cargo test -p lucerna-hello --test process` → falham (o `main` ainda é vazio).
- [ ] **Passo 3: implementar** o `main.rs` como nas Interfaces.
- [ ] **Passo 4: verificar.** `cargo test --workspace` → tudo passa, 1 ignorado. Conferência à mão (opcional, na máquina do usuário): `XDG_RUNTIME_DIR=$(mktemp -d) cargo run -p lucerna-hello -- serve` num terminal; no outro, `echo '{"jsonrpc":"2.0","id":1,"method":"lucerna.hello","params":{"protocol":"1.0","client":"socat","app":"hello"}}' | socat - UNIX-CONNECT:$XDG_RUNTIME_DIR/lucerna/hello.sock` mostra a resposta do `hello` numa linha.
- [ ] **Passo 5: commit.** `git commit -m "Hello: binário único (serve, subcomandos) e testes de processo"`

---

### Tarefa 13: CI, Dependabot e checks obrigatórios

**Files:**
- Create: `.github/workflows/ci.yml`, `.github/dependabot.yml`
- Modify: `README.md` (seção "Desenvolvimento": os três comandos do CI e o `--ignored` do systemd)

**Interfaces:**
- Consumes: o workspace inteiro.
- Produces: os checks `fmt`, `clippy`, `test` em todo PR e no `main`; o ruleset `main` exigindo os três.

- [ ] **Passo 1: workflow.** `ci.yml` no formato do `lucerna-shell`: comentário de cabeçalho em português; `on: pull_request, push (main), schedule "0 6 * * 1", workflow_dispatch`; `permissions: contents: read`; o mesmo bloco `concurrency`; `defaults.run.shell: bash`; `env: CARGO_TERM_COLOR: always`. Um job `check` com `name: ${{ matrix.job }}`, `runs-on: ubuntu-latest`, `timeout-minutes: 15`, `strategy.fail-fast: false`, `matrix.job: [fmt, clippy, test]`. Passos: `actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1` com `persist-credentials: false`; `rustup show` (instala o que o `rust-toolchain.toml` pede); `actions/cache@55cc8345863c7cc4c66a329aec7e433d2d1c52a9 # v6.1.0` com `~/.cargo/registry`, `~/.cargo/git` e `target`, chave `cargo-${{ matrix.job }}-${{ hashFiles('Cargo.lock', 'rust-toolchain.toml') }}`, `restore-keys: cargo-${{ matrix.job }}-`; e o passo do job:
  - `fmt`: `cargo fmt --all --check`
  - `clippy`: `cargo clippy --workspace --all-targets --locked -- -D warnings`
  - `test`: `cargo test --workspace --locked`

- [ ] **Passo 2: dependabot.** `version: 2`, comentário de cabeçalho; `cargo` em `/` semanal com grupo `cargo` (`patterns: ["*"]`) e `github-actions` em `/` semanal com grupo `actions`, como no `lucerna-shell`.

- [ ] **Passo 3: verificar localmente.** `cargo fmt --all --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked` → sai 0. Se houver `actionlint` instalado, `actionlint .github/workflows/ci.yml` sem saída.

- [ ] **Passo 4: commit, push e PR.**

```bash
git add .github README.md && git commit -m "CI: fmt, clippy e testes, com Dependabot"
gh auth switch --user EduardoSA8006
git push -u origin feat/sdk   # a branch da implementação
gh pr create --repo lucerna-desktop/lucerna-sdk --title "lucerna-sdk: protocolo, crates e lucerna-hello" --body-file <(…modelo .github/pull_request_template.md preenchido…)
```

- [ ] **Passo 5: esperar o CI do PR.** `gh pr checks <n> --repo lucerna-desktop/lucerna-sdk --watch` → `fmt`, `clippy` e `test` em `pass`.

- [ ] **Passo 6: checks obrigatórios no ruleset `main`** (id `25353655`; mantém as regras de hoje e acrescenta `required_status_checks`, no formato do ruleset do `lucerna-shell`):

```bash
tmp=$(mktemp)
gh api repos/lucerna-desktop/lucerna-sdk/rulesets/25353655 \
  | jq '{name, target, enforcement, conditions, bypass_actors,
         rules: ([.rules[] | select(.type != "required_status_checks")] + [{
           type: "required_status_checks",
           parameters: {
             strict_required_status_checks_policy: true,
             do_not_enforce_on_create: false,
             required_status_checks: [
               {context: "fmt", integration_id: 15368},
               {context: "clippy", integration_id: 15368},
               {context: "test", integration_id: 15368}]}}])}' > "$tmp"
gh api -X PUT repos/lucerna-desktop/lucerna-sdk/rulesets/25353655 --input "$tmp"
```

Verificação: `gh api repos/lucerna-desktop/lucerna-sdk/rulesets/25353655 --jq '[.rules[].type], [.rules[] | select(.type=="required_status_checks") | .parameters.required_status_checks[].context]'` → `["deletion","non_fast_forward","pull_request","required_status_checks"]` e `["fmt","clippy","test"]`.

- [ ] **Passo 7: avisar o usuário.** O merge é dele (merge commit).

---

## Autorrevisão

- **Cobertura do spec:** transporte, enquadramento e limite (T4, T5); JSON-RPC, nomes e reservados (T1); aperto de mão, `-32010`, `-32011` (T1, T5, T11); pedidos (T5, T6, T11); assinaturas, retrato, `seq`, coalescência, contínuos só com assinante, `unsubscribe`, fechar desfaz (T5, T11); erros e mensagens em português (T1); `shell.sock`, seis métodos, posse pela conexão, sem shell, reconexão com espera até 5 s, item na barra como trabalho pendente (T8, T11); notificações pelo D-Bus — fora do SDK, nada a fazer; manifesto, `schema`, `api`, exemplos compartilhados (T3); três versões (T1, T3; a doc do crate diz a versão do protocolo); ativação pelo socket e socket manual (T7, T12); ociosidade 60 s configurável (T7, T12); timer agendado que sobrevive ao reinício (T9); binário único com `serve`, CLI com `--json`, `status`, `watch` e códigos (T10, T12); pastas (T2); `lucerna-hello` sem painel com unidades e manifesto (T11, T12); testes de integração listados no spec (T11, T12); CI e ruleset (T13). Publicação no crates.io: fora deste plano (os `Cargo.toml` já têm os metadados; publicar é decisão do usuário).
- **Tipos consistentes:** `State`, `BarItem`, `HelloResult`, `Topic`, `ServerBuilder::method`, `Client::connect(path, client, app)`, `WorkTracker`, `Shell::spawn`, `FakeShell`, `CliApp::run`, `EXIT_*` conferidos entre as Interfaces e os testes das tarefas seguintes.
- **Review Focus:** os cinco itens têm teste na tarefa dona (T5 ×2, T7, T2/T10, T8).
- **Proporção:** o plano tem cerca de 6× o tamanho do spec, quase tudo em testes (pedidos como código) e assinaturas; os corpos ficam com o implementador, e os testes vizinhos de um mesmo molde aparecem com a entrada e as asserções em comentário em vez de repetidos por extenso.
