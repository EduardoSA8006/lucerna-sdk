//! Conexão crua, linha por linha (como no `socat`), para testes e depuração.

use std::io;
use std::path::Path;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};

use crate::framing::{FrameError, LineReader, MAX_MESSAGE};

/// Quanto o `recv` espera por uma mensagem.
const RECV_TIMEOUT: Duration = Duration::from_secs(5);

/// Conexão crua a um socket do protocolo.
pub struct RawConn {
    reader: LineReader<OwnedReadHalf>,
    writer: OwnedWriteHalf,
}

impl RawConn {
    /// Conecta ao socket em `path`.
    pub async fn connect(path: &Path) -> io::Result<RawConn> {
        Ok(RawConn::from_stream(UnixStream::connect(path).await?))
    }

    /// Usa um socket já aberto (ex.: o lado aceito de um servidor falso).
    pub fn from_stream(stream: UnixStream) -> RawConn {
        let (rd, wr) = stream.into_split();
        RawConn {
            reader: LineReader::new(rd, MAX_MESSAGE),
            writer: wr,
        }
    }

    /// Manda `v` como uma linha.
    pub async fn send(&mut self, v: &Value) -> io::Result<()> {
        let mut bytes = v.to_string().into_bytes();
        bytes.push(b'\n');
        self.send_raw(&bytes).await
    }

    /// Manda os bytes como estão (sem acrescentar `\n`).
    pub async fn send_raw(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.writer.write_all(bytes).await?;
        self.writer.flush().await
    }

    /// Próxima mensagem, ou `None` quando o serviço fecha a conexão.
    ///
    /// Espera até 5 s; passou disso, `ErrorKind::TimedOut`.
    pub async fn recv(&mut self) -> io::Result<Option<Value>> {
        let line = match tokio::time::timeout(RECV_TIMEOUT, self.reader.next_line()).await {
            Err(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "nenhuma mensagem em 5 s",
                ));
            }
            Ok(Ok(Some(line))) => line,
            Ok(Ok(None)) => return Ok(None),
            // Fechar com dados não lidos dá `ECONNRESET` em vez de fim do fluxo.
            Ok(Err(FrameError::Io(e))) if e.kind() == io::ErrorKind::ConnectionReset => {
                return Ok(None);
            }
            Ok(Err(FrameError::Io(e))) => return Err(e),
            Ok(Err(e)) => return Err(io::Error::new(io::ErrorKind::InvalidData, e)),
        };
        serde_json::from_str(&line)
            .map(Some)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    /// Faz o aperto de mão com o app `app` e devolve a resposta inteira.
    pub async fn hello(&mut self, app: &str) -> io::Result<Value> {
        self.send(&json!({
            "jsonrpc": "2.0",
            "id": 0,
            "method": "lucerna.hello",
            "params": {"protocol": "1.0", "client": "test", "app": app},
        }))
        .await?;
        self.recv().await?.ok_or_else(|| {
            io::Error::new(io::ErrorKind::UnexpectedEof, "o serviço fechou a conexão")
        })
    }
}
