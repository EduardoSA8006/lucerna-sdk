//! Enquadramento por linha: uma mensagem JSON por linha, UTF-8, terminada em `\n`.

use std::io;

use lucerna_protocol::Message;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

/// Limite padrão de uma mensagem, em bytes, sem contar o `\n` (1 MiB).
pub const MAX_MESSAGE: usize = 1_048_576;

/// Erro ao ler uma linha.
#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    /// A linha passou do limite; o leitor não deve mais ser usado (a conexão fecha).
    #[error("mensagem grande demais")]
    TooLarge,
    /// A linha não é UTF-8 válido; ela foi consumida inteira e o leitor continua.
    #[error("mensagem não é UTF-8 válido")]
    InvalidUtf8,
    /// Erro de E/S do fluxo.
    #[error("erro de E/S: {0}")]
    Io(#[from] io::Error),
}

/// Lê linhas de um fluxo, com limite de tamanho por linha.
pub struct LineReader<R> {
    inner: BufReader<R>,
    max: usize,
    line: Vec<u8>,
}

impl<R: AsyncRead + Unpin> LineReader<R> {
    /// Cria um leitor sobre `inner` que recusa linhas com mais de `max` bytes
    /// (sem contar o `\n` e um `\r` antes dele).
    pub fn new(inner: R, max: usize) -> Self {
        Self {
            inner: BufReader::new(inner),
            max,
            line: Vec::new(),
        }
    }

    /// Devolve a próxima linha não vazia, sem `\n` nem `\r` final.
    ///
    /// `Ok(None)` no fim do fluxo; um resto sem `\n` no fim é descartado.
    pub async fn next_line(&mut self) -> Result<Option<String>, FrameError> {
        loop {
            let chunk = self.inner.fill_buf().await?;
            if chunk.is_empty() {
                self.line.clear();
                return Ok(None);
            }
            match chunk.iter().position(|&b| b == b'\n') {
                Some(i) => {
                    self.line.extend_from_slice(&chunk[..i]);
                    self.inner.consume(i + 1);
                    let mut line = std::mem::take(&mut self.line);
                    if line.last() == Some(&b'\r') {
                        line.pop();
                    }
                    if line.len() > self.max {
                        return Err(FrameError::TooLarge);
                    }
                    if line.is_empty() {
                        continue;
                    }
                    return String::from_utf8(line)
                        .map(Some)
                        .map_err(|_| FrameError::InvalidUtf8);
                }
                None => {
                    // Cabe um `\r` além do limite; passou disso, a linha já é grande demais.
                    if self.line.len() + chunk.len() > self.max + 1 {
                        self.line.clear();
                        return Err(FrameError::TooLarge);
                    }
                    let n = chunk.len();
                    self.line.extend_from_slice(chunk);
                    self.inner.consume(n);
                }
            }
        }
    }
}

/// Escreve `msg` como uma linha (`to_json()` + `\n`) e dá `flush`.
pub async fn write_message<W: AsyncWrite + Unpin>(w: &mut W, msg: &Message) -> io::Result<()> {
    let mut bytes = msg.to_json().into_bytes();
    bytes.push(b'\n');
    w.write_all(&bytes).await?;
    w.flush().await
}
