//! Como o serviço recebe o socket: do systemd (`LISTEN_FDS`) ou criado à mão.

use std::fs;
use std::io;
use std::os::fd::{FromRawFd, OwnedFd};
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

use lucerna_paths::{AppPaths, create_private_dir};

use crate::ServiceError;

/// O primeiro fd que o systemd passa (`SD_LISTEN_FDS_START`).
const LISTEN_FDS_START: i32 = 3;

/// Quantos fds o systemd passou a este processo.
///
/// Devolve `0` se falta alguma variável, se ela não é um número ou se
/// `LISTEN_PID` não é `own_pid` (as variáveis eram de outro processo).
pub fn listen_fds(listen_pid: Option<&str>, listen_fds: Option<&str>, own_pid: u32) -> usize {
    let (Some(pid), Some(fds)) = (listen_pid, listen_fds) else {
        return 0;
    };
    match (pid.parse::<u32>(), fds.parse::<usize>()) {
        (Ok(pid), Ok(fds)) if pid == own_pid => fds,
        _ => 0,
    }
}

/// Transforma o fd recebido num [`UnixListener`] não bloqueante.
///
/// Reabre o fd com `try_clone()` para ter `CLOEXEC` (o fd 3 do systemd vem sem
/// ele e vazaria para os processos filhos) e fecha o original.
///
/// Limitação: sem `libc` não conferimos `SO_TYPE` nem `SO_ACCEPTCONN`. O
/// `local_addr()` garante que é um socket Unix, e a unidade `.socket`
/// (`ListenStream=`) garante um socket de fluxo em escuta; um fd errado falha
/// no primeiro `accept` com o erro do sistema.
pub fn listener_from_fd(fd: OwnedFd) -> io::Result<UnixListener> {
    let fd = fd.try_clone()?; // o original fecha ao sair daqui
    let listener = UnixListener::from(fd);
    listener.local_addr()?;
    listener.set_nonblocking(true)?;
    Ok(listener)
}

/// De onde veio o socket do serviço.
#[derive(Debug)]
pub enum Activation {
    /// Passado pelo systemd; o arquivo é dele.
    Systemd,
    /// Criado à mão; o arquivo some quando o guarda cai.
    Manual(SocketGuard),
}

/// Apaga o arquivo do socket criado à mão quando é largado.
///
/// Só apaga se o arquivo ainda é o que criamos (mesmo dispositivo e inode): se
/// outro serviço tomou o caminho depois, o socket dele fica.
#[derive(Debug)]
pub struct SocketGuard {
    path: PathBuf,
    id: (u64, u64),
}

impl SocketGuard {
    /// O caminho do socket.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for SocketGuard {
    fn drop(&mut self) {
        if let Ok(md) = fs::symlink_metadata(&self.path)
            && (md.dev(), md.ino()) == self.id
        {
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// Cria o socket à mão em `socket`, com a pasta em modo `0700`.
///
/// Se o arquivo já existe e alguém atende nele, devolve
/// [`ServiceError::AlreadyRunning`] sem tocar nele. Se é um socket velho (o
/// `connect` é recusado), apaga e cria de novo. Um arquivo que não é socket
/// nunca é apagado.
pub fn bind_manual(socket: &Path) -> Result<(UnixListener, SocketGuard), ServiceError> {
    if let Some(dir) = socket.parent().filter(|p| !p.as_os_str().is_empty()) {
        create_private_dir(dir)?;
    }
    match fs::symlink_metadata(socket) {
        Ok(md) if md.file_type().is_socket() => match UnixStream::connect(socket) {
            Ok(_) => return Err(ServiceError::AlreadyRunning(socket.to_path_buf())),
            Err(e) if e.kind() == io::ErrorKind::ConnectionRefused => fs::remove_file(socket)?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        },
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("{} existe e não é um socket.", socket.display()),
            )
            .into());
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    let listener = UnixListener::bind(socket)?;
    listener.set_nonblocking(true)?;
    let md = fs::symlink_metadata(socket)?;
    let guard = SocketGuard {
        path: socket.to_path_buf(),
        id: (md.dev(), md.ino()),
    };
    Ok((listener, guard))
}

/// O socket do serviço: o do systemd, se `LISTEN_PID`/`LISTEN_FDS` são deste
/// processo, senão um criado à mão em `paths.socket()`.
///
/// Chame uma vez só por processo: o fd 3 passa a ser do listener devolvido.
pub fn listener_from_env(
    paths: &AppPaths,
) -> Result<(tokio::net::UnixListener, Activation), ServiceError> {
    let pid = std::env::var("LISTEN_PID").ok();
    let fds = std::env::var("LISTEN_FDS").ok();
    match listen_fds(pid.as_deref(), fds.as_deref(), std::process::id()) {
        0 => {
            let (listener, guard) = bind_manual(&paths.socket()?)?;
            let listener = tokio::net::UnixListener::from_std(listener)?;
            Ok((listener, Activation::Manual(guard)))
        }
        1 => {
            // SAFETY: com `LISTEN_PID` igual ao nosso pid e `LISTEN_FDS=1`, o
            // systemd nos passou o fd 3 aberto e ninguém mais neste processo é
            // dono dele (esta função é chamada uma vez só). O `OwnedFd` passa a
            // ser o único dono e o fecha depois de `listener_from_fd` reabri-lo.
            #[allow(unsafe_code)]
            let fd = unsafe { OwnedFd::from_raw_fd(LISTEN_FDS_START) };
            let listener = listener_from_fd(fd).map_err(|e| {
                ServiceError::Activation(format!("O socket passado pelo systemd não serve: {e}"))
            })?;
            let listener = tokio::net::UnixListener::from_std(listener)?;
            Ok((listener, Activation::Systemd))
        }
        n => Err(ServiceError::Activation(format!(
            "O systemd passou {n} sockets; o serviço espera um só."
        ))),
    }
}
