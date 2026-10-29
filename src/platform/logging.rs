//! Registro unificado dos processos do mide.
//!
//! Cada evento segue para o journal (`stderr`) e para `mide.log` em
//! `$XDG_STATE_HOME/mide` (padrão `~/.local/state/mide`). O arquivo é compartilhado
//! entre processos, protegido por `flock` e rotacionado para manter o uso de disco
//! limitado. Nenhuma variável de ambiente completa ou conteúdo de documentos é
//! registrado automaticamente.

use std::{
    collections::VecDeque,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

use tracing_subscriber::{EnvFilter, fmt::writer::MakeWriter};

const MAX_BYTES: u64 = 8 * 1024 * 1024;
const BACKUPS: usize = 3;
const DEFAULT_TAIL: usize = 300;
static WRITE_WARNING: AtomicBool = AtomicBool::new(false);

/// Inicializa o registro persistente e retorna uma guarda que registra o fim normal
/// do processo. `filter` é usado quando `RUST_LOG` não foi definido.
pub fn init(program: &'static str, filter: &'static str) -> LoggingSession {
    let path = log_path();
    let setup_error = prepare(&path).err();
    let writer = LogWriterFactory { path: path.clone(), enabled: setup_error.is_none(), program, pid: std::process::id(), gate: Mutex::new(()) };
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new(format!("{filter},mide::platform::logging=info"))
    });
    let result = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .with_target(true)
        .with_thread_ids(true)
        .with_thread_names(true)
        .with_file(true)
        .with_line_number(true)
        .with_writer(writer)
        .try_init();

    if result.is_ok() {
        install_panic_hook();
        tracing::info!(program, pid = std::process::id(), version = env!("CARGO_PKG_VERSION"), log = %path.display(), "processo iniciado");
        if let Some(err) = setup_error {
            tracing::warn!(log = %path.display(), error = %err, "arquivo persistente indisponível; os eventos seguem para o journal");
        }
    } else {
        eprintln!("mide logging: não foi possível instalar tracing: {}", result.unwrap_err());
    }

    LoggingSession { program }
}

/// Caminho principal do log, respeitando `XDG_STATE_HOME`.
pub fn log_path() -> PathBuf {
    if let Some(path) = std::env::var_os("MIDE_LOG_FILE") {
        return PathBuf::from(path);
    }
    let state = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    state.join("mide/logs/mide.log")
}

/// Os arquivos de log em ordem cronológica, do mais antigo ao atual.
pub fn log_files() -> Vec<PathBuf> {
    log_files_for(&log_path())
}

/// Lê as últimas linhas do arquivo atual sem carregar o arquivo todo na memória.
pub fn tail(count: usize) -> io::Result<Vec<String>> {
    let path = log_path();
    if !path.exists() {
        return Ok(Vec::new());
    }
    let mut file = File::open(path)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let text = String::from_utf8_lossy(&bytes);
    let mut lines = VecDeque::with_capacity(count.min(DEFAULT_TAIL));
    for line in text.lines() {
        if lines.len() == count {
            lines.pop_front();
        }
        lines.push_back(line.to_string());
    }
    Ok(lines.into_iter().collect())
}

/// Guarda de vida do processo; o `Drop` registra somente término normal ou unwind.
pub struct LoggingSession {
    program: &'static str,
}

impl Drop for LoggingSession {
    fn drop(&mut self) {
        tracing::info!(program = self.program, pid = std::process::id(), "processo encerrado");
    }
}

struct LogWriterFactory {
    path: PathBuf,
    enabled: bool,
    program: &'static str,
    pid: u32,
    gate: Mutex<()>,
}

impl<'a> MakeWriter<'a> for LogWriterFactory {
    type Writer = RecordWriter<'a>;

    fn make_writer(&'a self) -> Self::Writer {
        let gate = self.gate.lock().unwrap_or_else(|e| e.into_inner());
        RecordWriter { path: &self.path, enabled: self.enabled, program: self.program, pid: self.pid, _gate: gate, bytes: Vec::with_capacity(256) }
    }
}

struct RecordWriter<'a> {
    path: &'a Path,
    enabled: bool,
    program: &'static str,
    pid: u32,
    // Serializa eventos de várias threads deste processo durante a formatação.
    _gate: std::sync::MutexGuard<'a, ()>,
    bytes: Vec<u8>,
}

impl Write for RecordWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.bytes.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for RecordWriter<'_> {
    fn drop(&mut self) {
        if self.bytes.is_empty() {
            return;
        }
        // Preserva o journal do systemd e o `RUST_LOG` usado por quem executa no terminal.
        let mut record = Vec::with_capacity(self.bytes.len() + 40);
        for line in self.bytes.split_inclusive(|b| *b == b'\n') {
            record.extend_from_slice(format!("[pid={} process={}] ", self.pid, self.program).as_bytes());
            record.extend_from_slice(line);
        }
        let mut stderr = io::stderr().lock();
        let _ = stderr.write_all(&record);
        let _ = stderr.flush();
        if self.enabled
            && let Err(err) = append_record(self.path, &record, MAX_BYTES)
            && !WRITE_WARNING.swap(true, Ordering::Relaxed)
        {
            eprintln!("mide logging: falha ao gravar {}: {err}", self.path.display());
        }
    }
}

fn prepare(path: &Path) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
    }
    OpenOptions::new().create(true).append(true).write(true).open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn rotated(path: &Path, index: usize) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".{index}"));
    path.with_file_name(name)
}

fn append_record(path: &Path, record: &[u8], max_bytes: u64) -> io::Result<()> {
    let lock_path = path.with_extension("log.lock");
    let mut lock_options = OpenOptions::new();
    lock_options.create(true).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        lock_options.mode(0o600);
    }
    let lock = lock_options.open(lock_path)?;
    lock_exclusive(&lock)?;
    let current_len = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    if current_len > 0 && current_len.saturating_add(record.len() as u64) > max_bytes {
        let oldest = rotated(path, BACKUPS);
        let _ = fs::remove_file(oldest);
        for index in (1..BACKUPS).rev() {
            let from = rotated(path, index);
            if from.exists() {
                fs::rename(&from, rotated(path, index + 1))?;
            }
        }
        if path.exists() {
            fs::rename(path, rotated(path, 1))?;
        }
    }
    let mut options = OpenOptions::new();
    options.create(true).append(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(record)?;
    // Erros e avisos importantes sobrevivem melhor a uma queda abrupta do sistema.
    if record.windows(6).any(|w| w == b"ERROR ") || record.windows(5).any(|w| w == b"WARN ") {
        file.sync_data()?;
    }
    drop(file);
    Ok(())
}

fn lock_exclusive(file: &File) -> io::Result<()> {
    loop {
        // SAFETY: o descritor pertence ao `File` emprestado e continua aberto durante o lock.
        use std::os::fd::AsRawFd;
        let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
        if result == 0 {
            return Ok(());
        }
        let err = io::Error::last_os_error();
        if err.kind() != io::ErrorKind::Interrupted {
            return Err(err);
        }
    }
}

fn install_panic_hook() {
    static INSTALLED: OnceLock<()> = OnceLock::new();
    INSTALLED.get_or_init(|| {
        let _ = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let location = info.location().map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column())).unwrap_or_else(|| "local desconhecido".into());
            tracing::error!(thread = ?std::thread::current().name(), location = %location, backtrace = %std::backtrace::Backtrace::force_capture(), "pânico; conteúdo da mensagem omitido por segurança");
        }));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_log_path() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mide-logging-test-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        fs::create_dir_all(&dir).unwrap();
        dir.join("mide.log")
    }

    #[test]
    fn log_rotaciona_e_preserva_eventos_recentes() {
        let path = test_log_path();
        append_record(&path, b"primeiro\n", 18).unwrap();
        append_record(&path, b"segundo\n", 18).unwrap();
        append_record(&path, b"terceiro\n", 18).unwrap();
        assert_eq!(fs::read_to_string(rotated(&path, 1)).unwrap(), "primeiro\nsegundo\n");
        assert_eq!(fs::read_to_string(path).unwrap(), "terceiro\n");
    }

    #[test]
    fn caminhos_de_log_incluem_rotacoes_do_antigo_ao_atual() {
        let path = test_log_path();
        fs::write(rotated(&path, 2), "antigo").unwrap();
        fs::write(rotated(&path, 1), "recente").unwrap();
        fs::write(&path, "atual").unwrap();
        let files = log_files_for(&path);
        assert_eq!(files.len(), 3);
        assert!(files[0].ends_with("mide.log.2"));
        assert!(files[2].ends_with("mide.log"));
    }

    #[test]
    fn varias_threads_nao_intercalam_eventos() {
        let path = test_log_path();
        let workers = (0..8)
            .map(|worker| {
                let path = path.clone();
                std::thread::spawn(move || {
                    for event in 0..25 {
                        append_record(&path, format!("{worker}:{event}\n").as_bytes(), u64::MAX).unwrap();
                    }
                })
            })
            .collect::<Vec<_>>();
        for worker in workers {
            worker.join().unwrap();
        }
        let text = fs::read_to_string(path).unwrap();
        let lines = text.lines().collect::<Vec<_>>();
        assert_eq!(lines.len(), 200);
        assert!(lines.iter().all(|line| {
            let Some((worker, event)) = line.split_once(':') else { return false };
            worker.parse::<u8>().is_ok_and(|n| n < 8)
                && event.parse::<u8>().is_ok_and(|n| n < 25)
        }));
    }
}

fn log_files_for(path: &Path) -> Vec<PathBuf> {
    let mut files = (1..=BACKUPS).rev().map(|n| rotated(path, n)).filter(|p| p.is_file()).collect::<Vec<_>>();
    if path.is_file() {
        files.push(path.to_path_buf());
    }
    files
}
