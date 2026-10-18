//! Lado cliente do canal de controle do compositor (`$XDG_RUNTIME_DIR/<socket>.ctl`).

use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixStream,
    path::PathBuf,
    time::Duration,
};

/// Caminho do canal de controle do compositor (`<socket>.ctl`).
pub fn control_path() -> Option<PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")?;
    let display = std::env::var("WAYLAND_DISPLAY").ok()?;
    Some(PathBuf::from(runtime).join(format!("{display}.ctl")))
}

/// Envia um comando ao compositor e devolve as linhas da resposta (sem `end`).
pub fn ask_compositor(command: &str) -> Option<Vec<String>> {
    let mut stream = UnixStream::connect(control_path()?).ok()?;
    stream.set_read_timeout(Some(Duration::from_millis(300))).ok()?;
    stream.write_all(format!("{command}\n").as_bytes()).ok()?;
    let mut lines = Vec::new();
    for line in BufReader::new(stream).lines() {
        let line = line.ok()?;
        if line == "end" {
            break;
        }
        lines.push(line);
    }
    Some(lines)
}
