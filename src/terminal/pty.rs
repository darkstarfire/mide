//! O programa do terminal num pseudoterminal (PTY): abre o par mestre/escravo,
//! inicia o shell como líder de uma sessão nova com o escravo como terminal de
//! controle, e lê a saída numa thread (cada pedaço vai por um canal e acorda a
//! janela).

use std::{
    ffi::OsString,
    fs::File,
    io::{Read, Write},
    os::fd::{AsFd, BorrowedFd, OwnedFd},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::mpsc,
};

use rustix::{
    fs::{Mode, OFlags},
    pty::{grantpt, openpt, ptsname, unlockpt, OpenptFlags},
    termios::{tcsetwinsize, Winsize},
};

/// O que rodar no terminal.
#[derive(Debug, Clone, Default)]
pub struct Spawn {
    /// Programa e argumentos (`None`: o shell do usuário).
    pub command: Option<(String, Vec<String>)>,
    pub directory: Option<PathBuf>,
    /// Variáveis de ambiente a mais.
    pub env: Vec<(String, String)>,
}

/// Tamanho em células e em pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Size {
    pub cols: u16,
    pub rows: u16,
    pub width: u16,
    pub height: u16,
}

pub struct Pty {
    master: File,
    child: Child,
    writes: mpsc::Sender<Vec<u8>>,
}

impl Pty {
    /// Abre o PTY e inicia o programa. `output` recebe cada pedaço da saída (e um
    /// vetor vazio quando o programa termina); `wake` é chamado depois de cada envio.
    pub fn spawn(spawn: &Spawn, size: Size, output: mpsc::Sender<Vec<u8>>, wake: impl Fn() + Send + 'static) -> std::io::Result<Pty> {
        let master = openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY | OpenptFlags::CLOEXEC)?;
        grantpt(&master)?;
        unlockpt(&master)?;
        let name = ptsname(&master, Vec::new())?;
        let slave: OwnedFd = rustix::fs::open(name.as_c_str(), OFlags::RDWR | OFlags::NOCTTY | OFlags::CLOEXEC, Mode::empty())?;
        set_size(master.as_fd(), size)?;

        let (program, args) = spawn.command.clone().unwrap_or_else(|| (user_shell(), Vec::new()));
        let mut command = Command::new(&program);
        command
            .args(&args)
            .env("TERM", "xterm-256color")
            .env("COLORTERM", "truecolor")
            .env("TERM_PROGRAM", "mide")
            .env_remove("COLUMNS")
            .env_remove("LINES")
            .envs(spawn.env.iter().map(|(k, v)| (k, v)))
            .stdin(Stdio::from(slave.try_clone()?))
            .stdout(Stdio::from(slave.try_clone()?))
            .stderr(Stdio::from(slave));
        let home = std::env::var_os("HOME").map(PathBuf::from);
        if let Some(dir) = spawn.directory.clone().filter(|d| d.is_dir()).or(home) {
            command.current_dir(dir);
        }
        // Shell de login como nos terminais do Mac? Não: como nos do Linux, o shell
        // interativo comum (o ambiente da sessão já vem pronto).
        // SAFETY: no processo filho, antes do exec, só chamadas de sistema simples.
        unsafe {
            use std::os::unix::process::CommandExt;
            command.pre_exec(|| {
                rustix::process::setsid()?;
                rustix::process::ioctl_tiocsctty(BorrowedFd::borrow_raw(0))?;
                Ok(())
            });
        }
        let child = command.spawn()?;

        let master = File::from(master);
        let mut reader = master.try_clone()?;
        std::thread::spawn(move || {
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                match reader.read(&mut buf) {
                    Ok(n) if n > 0 => {
                        if output.send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                        wake();
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                    // Fim (EIO quando todos os processos do terminal saíram).
                    _ => {
                        let _ = output.send(Vec::new());
                        wake();
                        break;
                    }
                }
            }
        });
        // Escrita numa thread: um programa que não lê a entrada (colar um texto
        // grande) não trava a janela.
        let (writes, pending) = mpsc::channel::<Vec<u8>>();
        let mut writer = master.try_clone()?;
        std::thread::spawn(move || {
            for bytes in pending {
                if writer.write_all(&bytes).is_err() {
                    break;
                }
            }
        });
        Ok(Pty { master, child, writes })
    }

    pub fn write(&self, bytes: Vec<u8>) {
        if !bytes.is_empty() {
            let _ = self.writes.send(bytes);
        }
    }

    /// Avisa o programa do tamanho novo (ele recebe SIGWINCH).
    pub fn resize(&self, size: Size) {
        let _ = set_size(self.master.as_fd(), size);
    }

    /// Pasta atual do programa (do shell, normalmente).
    pub fn cwd(&self) -> Option<PathBuf> {
        std::fs::read_link(format!("/proc/{}/cwd", self.child.id())).ok()
    }

    /// Nome do programa em primeiro plano no terminal (o shell, ou o que ele rodou:
    /// `vim`, `ssh`...) e se é outro que não o próprio shell.
    pub fn foreground(&self) -> Option<(String, bool)> {
        let pgrp = rustix::termios::tcgetpgrp(self.master.as_fd()).ok()?;
        let name = std::fs::read_to_string(format!("/proc/{}/comm", pgrp.as_raw_nonzero())).ok()?;
        Some((name.trim().to_string(), pgrp.as_raw_nonzero().get() as u32 != self.child.id()))
    }

    /// Recolhe o processo se ele já terminou.
    pub fn reap(&mut self) {
        let _ = self.child.try_wait();
    }
}

fn set_size(fd: BorrowedFd, size: Size) -> std::io::Result<()> {
    tcsetwinsize(fd, Winsize { ws_row: size.rows, ws_col: size.cols, ws_xpixel: size.width, ws_ypixel: size.height })?;
    Ok(())
}

/// O shell do usuário: `$SHELL`, ou o da conta (`/etc/passwd`), ou `/bin/sh`.
pub fn user_shell() -> String {
    if let Some(shell) = std::env::var_os("SHELL").filter(|s| !s.is_empty()) {
        return OsString::into_string(shell).unwrap_or_else(|_| "/bin/sh".into());
    }
    let uid = rustix::process::getuid().as_raw();
    std::fs::read_to_string("/etc/passwd")
        .ok()
        .and_then(|passwd| {
            passwd.lines().find_map(|line| {
                let fields: Vec<&str> = line.split(':').collect();
                (fields.len() >= 7 && fields[2].parse::<u32>().ok() == Some(uid)).then(|| fields[6].to_string())
            })
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "/bin/sh".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roda_um_programa_e_le_a_saida() {
        let (tx, rx) = mpsc::channel();
        let spawn = Spawn { command: Some(("sh".into(), vec!["-c".into(), "stty size; tty; echo fim".into()])), directory: None, env: Vec::new() };
        let mut pty = Pty::spawn(&spawn, Size { cols: 33, rows: 7, width: 0, height: 0 }, tx, || {}).unwrap();
        let mut out = Vec::new();
        while let Ok(chunk) = rx.recv_timeout(std::time::Duration::from_secs(5)) {
            if chunk.is_empty() {
                break;
            }
            out.extend(chunk);
        }
        pty.reap();
        let text = String::from_utf8_lossy(&out);
        assert!(text.contains("7 33"), "{text}");
        assert!(text.contains("/dev/pts/"), "{text}");
        assert!(text.contains("fim"), "{text}");
    }
}
