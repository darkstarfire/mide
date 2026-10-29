//! Notificações de mudanças numa pasta, para atualizar o Arquivos sem polling.

use std::{
    ffi::CString,
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, Sender},
};

/// Observa a pasta atual e avisa a interface quando entradas mudam.
pub struct DirectoryWatch {
    path_tx: Sender<PathBuf>,
    changed_rx: Receiver<()>,
}

impl DirectoryWatch {
    /// Inicia o observador. Se o kernel não aceitar inotify, `None` deixa disponível o F5.
    pub fn new(path: PathBuf) -> Option<Self> {
        // SAFETY: devolve um descritor inotify novo, fechado pela thread ao terminar.
        let fd = unsafe { libc::inotify_init1(libc::IN_CLOEXEC | libc::IN_NONBLOCK) };
        if fd < 0 {
            return None;
        }
        let (path_tx, path_rx) = mpsc::channel();
        let (changed_tx, changed_rx) = mpsc::channel();
        if std::thread::Builder::new()
            .name("mide-files-watch".into())
            .spawn(move || watch_loop(fd, path_rx, changed_tx))
            .is_err()
        {
            // SAFETY: a thread não iniciou; este código ainda é dono do descritor.
            unsafe { libc::close(fd) };
            return None;
        }
        let _ = path_tx.send(path);
        Some(Self { path_tx, changed_rx })
    }

    /// Troca a pasta observada quando o usuário navega para outro lugar.
    pub fn set_path(&self, path: PathBuf) {
        let _ = self.path_tx.send(path);
    }

    /// Drena eventos repetidos e informa se houve ao menos uma mudança.
    pub fn take_change(&self) -> bool {
        self.changed_rx.try_iter().next().is_some()
    }
}

fn watch_path(path: &Path) -> PathBuf {
    if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent().unwrap_or(path).to_path_buf()
    }
}

fn add_watch(fd: libc::c_int, path: &Path) -> Option<libc::c_int> {
    let path = CString::new(path.as_os_str().as_bytes()).ok()?;
    let mask = libc::IN_ATTRIB
        | libc::IN_CLOSE_WRITE
        | libc::IN_CREATE
        | libc::IN_DELETE
        | libc::IN_DELETE_SELF
        | libc::IN_MOVED_FROM
        | libc::IN_MOVED_TO
        | libc::IN_MOVE_SELF;
    // SAFETY: `path` is NUL-terminated and remains alive for this call; fd is inotify.
    let wd = unsafe { libc::inotify_add_watch(fd, path.as_ptr(), mask) };
    (wd >= 0).then_some(wd)
}

fn watch_loop(fd: libc::c_int, paths: Receiver<PathBuf>, changed: Sender<()>) {
    let mut desired: Option<PathBuf> = None;
    let mut watched: Option<(PathBuf, libc::c_int)> = None;
    let mut buffer = [0u8; 8192];

    loop {
        let mut disconnected = false;
        loop {
            match paths.try_recv() {
                Ok(path) => desired = Some(path),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }
        if disconnected {
            break;
        }

        let resolved = desired.as_deref().map(watch_path);
        if resolved.as_ref() != watched.as_ref().map(|(path, _)| path) {
            if let Some((_, wd)) = watched.take() {
                // SAFETY: wd pertence ao descritor inotify aberto por esta thread.
                unsafe { libc::inotify_rm_watch(fd, wd) };
            }
            if let Some(path) = resolved {
                watched = add_watch(fd, &path).map(|wd| (path, wd));
            }
        }

        let mut pollfd = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
        // SAFETY: pollfd aponta para uma única estrutura válida durante a chamada.
        let ready = unsafe { libc::poll(&mut pollfd, 1, 250) };
        if ready < 0 {
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            break;
        }
        if ready == 0 || pollfd.revents & libc::POLLIN == 0 {
            continue;
        }

        let mut received = false;
        loop {
            // SAFETY: buffer é gravável e fd é inotify não bloqueante.
            let bytes = unsafe { libc::read(fd, buffer.as_mut_ptr().cast(), buffer.len()) };
            if bytes > 0 {
                received = true;
                continue;
            }
            if bytes < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            break;
        }
        if received && changed.send(()).is_err() {
            break;
        }
    }

    // SAFETY: esta thread é a única dona do descritor inotify.
    unsafe { libc::close(fd) };
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, io::Write, time::Duration};

    #[test]
    fn avisa_quando_arquivo_e_finalizado_na_pasta_observada() {
        let dir = std::env::temp_dir().join(format!(
            "mide-watch-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let Some(watch) = DirectoryWatch::new(dir.clone()) else {
            let _ = fs::remove_dir_all(dir);
            return;
        };
        std::thread::sleep(Duration::from_millis(50));
        let path = dir.join("video.mp4");
        let mut file = fs::File::create(&path).unwrap();
        file.write_all(b"video").unwrap();
        drop(file);
        assert!(watch.changed_rx.recv_timeout(Duration::from_secs(2)).is_ok());
        drop(watch);
        let _ = fs::remove_dir_all(dir);
    }
}
