#![warn(missing_docs)]
//! Esqueleto da linha de comando dos apps do Lucerna.
//!
//! O app declara os subcomandos ([`CliCommand`]) e cada um vira um método do
//! protocolo. O [`CliApp`] acrescenta os comuns (`status`, `watch <tópico>`), o
//! oculto `serve` e a opção global `--json`, com a ajuda em português.
//!
//! Códigos de saída: [`EXIT_OK`], [`EXIT_APP`], [`EXIT_NO_SERVICE`] e
//! [`EXIT_USAGE`]; `--help`/`--version` saem com `0`.

pub mod render;

use std::ffi::OsString;
use std::io::Write;
use std::path::Path;
use std::time::Duration;

use clap::error::ErrorKind;
use clap::{Arg, ArgAction, ArgMatches, Command};
use lucerna_ipc::{Client, ClientError};
use lucerna_paths::{AppPaths, Env};
use lucerna_protocol::{RpcError, State, codes};
use serde_json::{Map, Value};

use crate::render::render_default;

/// Sucesso.
pub const EXIT_OK: i32 = 0;
/// O serviço respondeu com erro (ou com uma resposta que não dá para ler).
pub const EXIT_APP: i32 = 1;
/// Não deu para falar com o serviço.
pub const EXIT_NO_SERVICE: i32 = 2;
/// Uso errado da linha de comando (`EX_USAGE`; o `2` do clap colidiria com
/// "sem serviço").
pub const EXIT_USAGE: i32 = 64;

/// Prazo para conectar e fazer o aperto de mão.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// Prazo de cada chamada.
const CALL_TIMEOUT: Duration = Duration::from_secs(30);

/// Nomes que o [`CliApp`] declara sozinho.
const RESERVED: [&str; 3] = ["status", "watch", "serve"];

const TEMPLATE: &str = "{about}\n\nUso: {usage}\n\n{all-args}";

/// O método a chamar e os parâmetros.
#[derive(Clone, Debug, PartialEq)]
pub struct Call {
    /// Nome do método.
    pub method: String,
    /// Parâmetros (`None` manda o pedido sem `params`).
    pub params: Option<Value>,
}

/// Um subcomando do app.
pub struct CliCommand {
    /// O subcomando do clap: nome, ajuda em português e argumentos.
    pub command: Command,
    /// Transforma os argumentos lidos na chamada ao serviço.
    pub to_call: fn(&ArgMatches) -> Call,
    /// Texto legível do resultado; `None` usa [`render_default`].
    pub render: Option<fn(&Value) -> String>,
}

/// O que o binário deve fazer com os argumentos.
#[derive(Debug)]
pub enum Invocation {
    /// Sem subcomando: abrir a TUI.
    Tui,
    /// O subcomando oculto `serve`: rodar o serviço.
    Serve,
    /// Um subcomando da linha de comando: passe a [`CliApp::run`].
    Cli(CliRun),
}

/// Um subcomando da linha de comando já lido, pronto para [`CliApp::run`].
#[derive(Debug)]
pub struct CliRun {
    kind: RunKind,
    json: bool,
}

#[derive(Debug)]
enum RunKind {
    Call {
        call: Call,
        render: Option<fn(&Value) -> String>,
    },
    Status,
    Watch(String),
}

/// A linha de comando de um app.
pub struct CliApp {
    app: String,
    bin: String,
    version: String,
    about: String,
    commands: Vec<CliCommand>,
}

impl CliApp {
    /// Começa a linha de comando do app `app`, no binário `bin`. O app passa
    /// `env!("CARGO_PKG_VERSION")` em `version`; `about` abre a ajuda.
    pub fn new(app: &str, bin: &str, version: &str, about: &str) -> Self {
        CliApp {
            app: app.to_string(),
            bin: bin.to_string(),
            version: version.to_string(),
            about: about.to_string(),
            commands: Vec::new(),
        }
    }

    /// Acrescenta um subcomando do app.
    ///
    /// Entra em pânico com nome `status`, `watch`, `serve` ou repetido.
    pub fn command(mut self, c: CliCommand) -> Self {
        let name = c.command.get_name();
        assert!(
            !RESERVED.contains(&name),
            "o subcomando {name:?} é declarado pelo lucerna-cli"
        );
        assert!(
            self.commands.iter().all(|o| o.command.get_name() != name),
            "subcomando repetido: {name:?}"
        );
        self.commands.push(c);
        self
    }

    /// O `clap::Command` completo: os subcomandos do app, `status`, `watch`, o
    /// `serve` oculto e `--json`, com a ajuda em português.
    pub fn clap(&self) -> Command {
        let mut root = Command::new(self.bin.clone())
            .version(self.version.clone())
            .about(self.about.clone())
            .override_usage(format!("{} [OPÇÕES] [COMANDO]", self.bin));
        for c in &self.commands {
            root = root.subcommand(c.command.clone());
        }
        let root = root
            .subcommand(Command::new("status").about("Mostra o estado de todos os tópicos."))
            .subcommand(
                Command::new("watch")
                    .about("Acompanha as mudanças de um tópico.")
                    .arg(
                        Arg::new("topic")
                            .value_name("tópico")
                            .required(true)
                            .help("O tópico a acompanhar."),
                    ),
            )
            .subcommand(Command::new("serve").about("Roda o serviço.").hide(true));
        localize(root)
            .arg(
                Arg::new("version")
                    .short('V')
                    .long("version")
                    .action(ArgAction::Version)
                    .help("Mostra a versão."),
            )
            .arg(
                Arg::new("json")
                    .long("json")
                    .global(true)
                    .action(ArgAction::SetTrue)
                    .help_heading("Opções")
                    .help("Imprime o resultado em JSON, para scripts."),
            )
    }

    /// Lê os argumentos (o primeiro é o nome do binário).
    ///
    /// O erro inclui `--help` e `--version`: passe a [`usage_exit`].
    pub fn parse_from<I, T>(&self, args: I) -> Result<Invocation, clap::Error>
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        let m = self.clap().try_get_matches_from(args)?;
        let Some((name, sub)) = m.subcommand() else {
            return Ok(Invocation::Tui);
        };
        let kind = match name {
            "serve" => return Ok(Invocation::Serve),
            "status" => RunKind::Status,
            "watch" => RunKind::Watch(sub.get_one::<String>("topic").cloned().unwrap_or_default()),
            _ => {
                // O clap só aceita subcomandos declarados: este existe.
                let c = self
                    .commands
                    .iter()
                    .find(|c| c.command.get_name() == name)
                    .expect("subcomando declarado");
                RunKind::Call {
                    call: (c.to_call)(sub),
                    render: c.render,
                }
            }
        };
        let json = m.get_flag("json") || sub.get_flag("json");
        Ok(Invocation::Cli(CliRun { kind, json }))
    }

    /// Conecta ao serviço (`client` = `cli`), roda o subcomando e devolve o
    /// código de saída. O resultado vai para `out`, as mensagens de erro para `err`.
    pub async fn run(
        &self,
        run: CliRun,
        env: &Env,
        out: &mut (dyn Write + Send),
        err: &mut (dyn Write + Send),
    ) -> i32 {
        let path = match AppPaths::new(&self.app, env).and_then(|p| p.socket()) {
            Ok(p) => p,
            Err(e) => return self.no_service(err, &e.to_string()),
        };
        let client = match Client::connect_timeout(&path, "cli", &self.app, CONNECT_TIMEOUT).await {
            Ok(c) => c,
            Err(e) => return self.fail(e, Stage::Connect(&path), run.json, err),
        };
        match run.kind {
            RunKind::Call { call, render } => {
                match client
                    .call_timeout(&call.method, call.params, CALL_TIMEOUT)
                    .await
                {
                    Ok(v) => {
                        let text = if run.json {
                            v.to_string()
                        } else {
                            render.unwrap_or(render_default)(&v)
                        };
                        if !text.is_empty() {
                            let _ = writeln!(out, "{text}");
                        }
                        let _ = out.flush();
                        EXIT_OK
                    }
                    Err(e) => self.fail(e, Stage::Call, run.json, err),
                }
            }
            RunKind::Status => self.status(&client, run.json, out, err).await,
            RunKind::Watch(topic) => self.watch(client, &topic, run.json, out, err).await,
        }
    }

    async fn status(
        &self,
        client: &Client,
        json: bool,
        out: &mut (dyn Write + Send),
        err: &mut (dyn Write + Send),
    ) -> i32 {
        let topics = client.hello().topics.clone();
        let states = if topics.is_empty() {
            Vec::new()
        } else {
            let refs: Vec<&str> = topics.iter().map(String::as_str).collect();
            match subscribe(client, &refs).await {
                Ok(s) => s,
                Err(e) => return self.fail(e, Stage::Call, json, err),
            }
        };
        if json {
            let map: Map<String, Value> = states.into_iter().map(|s| (s.topic, s.data)).collect();
            let _ = writeln!(out, "{}", Value::Object(map));
        } else {
            for s in &states {
                let _ = write!(out, "{}:\n{}", s.topic, indented(&s.data));
            }
        }
        let _ = out.flush();
        EXIT_OK
    }

    async fn watch(
        &self,
        mut client: Client,
        topic: &str,
        json: bool,
        out: &mut (dyn Write + Send),
        err: &mut (dyn Write + Send),
    ) -> i32 {
        let first = match subscribe(&client, &[topic]).await {
            Ok(s) => s,
            Err(e) => return self.fail(e, Stage::Call, json, err),
        };
        let print = |out: &mut (dyn Write + Send), s: &State| {
            if json {
                let line = serde_json::to_string(s).unwrap_or_default();
                writeln!(out, "{line}")?;
            } else {
                write!(out, "{} #{}\n{}", s.topic, s.seq, indented(&s.data))?;
            }
            out.flush()
        };
        for s in &first {
            if print(out, s).is_err() {
                // Quem lia a saída foi embora (ex.: `| head`): nada a fazer.
                return EXIT_OK;
            }
        }
        while let Some(s) = client.next_state().await {
            if print(out, &s).is_err() {
                return EXIT_OK;
            }
        }
        self.no_service(err, "o serviço fechou a conexão.")
    }

    /// Mensagem de "sem serviço" e o código `2`.
    fn no_service(&self, err: &mut (dyn Write + Send), detail: &str) -> i32 {
        let _ = writeln!(err, "Sem conexão com o serviço {}: {detail}", self.app);
        let _ = err.flush();
        EXIT_NO_SERVICE
    }

    /// Erro do app (`erro: …` ou `{"error":…}` com `--json`) e o código `1`.
    fn app_error(&self, err: &mut (dyn Write + Send), e: &RpcError, json: bool) -> i32 {
        if json {
            let v = serde_json::json!({ "error": e });
            let _ = writeln!(err, "{v}");
        } else {
            let _ = writeln!(err, "erro: {}", e.message);
        }
        let _ = err.flush();
        EXIT_APP
    }

    fn fail(
        &self,
        e: ClientError,
        stage: Stage<'_>,
        json: bool,
        err: &mut (dyn Write + Send),
    ) -> i32 {
        match e {
            ClientError::Connect(io) => {
                let detail = match (io.kind(), stage) {
                    (std::io::ErrorKind::NotFound, Stage::Connect(p)) => {
                        format!("o socket {} não existe.", p.display())
                    }
                    (std::io::ErrorKind::ConnectionRefused, Stage::Connect(p)) => {
                        format!("ninguém atende em {}.", p.display())
                    }
                    _ => format!("{io}."),
                };
                self.no_service(err, &detail)
            }
            ClientError::Io(io) => self.no_service(err, &format!("erro de comunicação: {io}.")),
            ClientError::Closed => self.no_service(err, "o serviço fechou a conexão."),
            ClientError::Timeout => self.no_service(err, "tempo esgotado."),
            // No aperto de mão (versão incompatível, socket de outro app): não
            // dá para falar com este serviço.
            ClientError::Rpc(e) if matches!(stage, Stage::Connect(_)) => {
                self.no_service(err, &e.message)
            }
            ClientError::Rpc(e) => self.app_error(err, &e, json),
            // O serviço existe e respondeu, mas fora da forma do protocolo:
            // não é "sem serviço".
            ClientError::Decode(d) => self.app_error(
                err,
                &RpcError {
                    code: codes::INTERNAL_ERROR,
                    message: format!("Resposta inválida do serviço: {d}."),
                    data: None,
                },
                json,
            ),
        }
    }
}

/// Em que ponto o erro aconteceu.
#[derive(Clone, Copy)]
enum Stage<'a> {
    /// Conectando ao socket em `path` ou no aperto de mão.
    Connect(&'a Path),
    /// Numa chamada, com a conexão aberta.
    Call,
}

async fn subscribe(client: &Client, topics: &[&str]) -> Result<Vec<State>, ClientError> {
    tokio::time::timeout(CALL_TIMEOUT, client.subscribe(topics))
        .await
        .map_err(|_| ClientError::Timeout)?
}

/// As linhas do [`render_default`] com dois espaços na frente, cada uma com `\n`.
fn indented(v: &Value) -> String {
    let text = render_default(v);
    text.lines().map(|l| format!("  {l}\n")).collect()
}

/// Ajuda em português num comando e nos subcomandos dele.
fn localize(c: Command) -> Command {
    c.help_template(TEMPLATE)
        .disable_help_flag(true)
        .disable_version_flag(true)
        .disable_help_subcommand(true)
        .allow_negative_numbers(true)
        .subcommand_help_heading("Comandos")
        .subcommand_value_name("COMANDO")
        .mut_args(|a| {
            if a.get_help_heading().is_some() {
                a
            } else if a.is_positional() {
                a.help_heading("Argumentos")
            } else {
                a.help_heading("Opções")
            }
        })
        .next_help_heading("Opções")
        .arg(
            Arg::new("help")
                .short('h')
                .long("help")
                .action(ArgAction::Help)
                .help("Mostra esta ajuda."),
        )
        .mut_subcommands(localize)
}

/// Imprime o erro do clap (ajuda, versão ou uso errado) e devolve o código de
/// saída: `0` para `--help`/`--version`, [`EXIT_USAGE`] para o resto.
pub fn usage_exit(e: &clap::Error) -> i32 {
    let _ = e.print();
    match e.kind() {
        ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => EXIT_OK,
        _ => EXIT_USAGE,
    }
}
