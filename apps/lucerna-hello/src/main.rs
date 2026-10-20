//! O binário `lucerna-hello`: `serve` roda o serviço; os outros subcomandos
//! falam com ele. Sem argumentos, mostra a ajuda (o hello não tem TUI).

use std::process::ExitCode;

use lucerna_cli::{Invocation, usage_exit};
use lucerna_hello::{cli, hello};
use lucerna_paths::Env;

#[tokio::main]
async fn main() -> ExitCode {
    let cli = cli();
    let code = match cli.parse_from(std::env::args_os()) {
        Ok(Invocation::Tui) => match cli.clap().print_help() {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("erro: {e}");
                1
            }
        },
        Ok(Invocation::Serve) => {
            // O produtor do `ticks` vive até o serviço parar.
            let h = hello();
            let _producer = h.producer;
            match h.service.run().await {
                Ok(()) => 0,
                Err(e) => {
                    eprintln!("erro: {e}");
                    1
                }
            }
        }
        Ok(Invocation::Cli(run)) => {
            let mut out = std::io::stdout();
            let mut err = std::io::stderr();
            cli.run(run, &Env::from_process(), &mut out, &mut err).await
        }
        Err(e) => usage_exit(&e),
    };
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}
