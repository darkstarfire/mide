//! Apps X11 dentro do mide (A14), pelo `xwayland-satellite`.
//!
//! O satellite é um cliente Wayland comum: ele roda o Xwayland e faz o papel de
//! gerenciador de janelas X11, entregando cada janela X11 ao mide como uma janela
//! normal (xdg-shell). Assim o compositor não tem código X11.
//!
//! O mide inicia o satellite num display X livre (`:1`, `:2`...) e passa esse
//! `DISPLAY` aos apps que abre. `MIDE_XWAYLAND=0` desliga (os testes desligam).

use std::{
    path::{Path, PathBuf},
    process::{Child, Command},
    sync::OnceLock,
};

/// Display X dos apps abertos pelo mide (`:N`), quando o XWayland está ativo.
static DISPLAY: OnceLock<String> = OnceLock::new();

/// Display X que os apps do mide devem usar, se o XWayland estiver ativo.
pub fn display() -> Option<&'static str> {
    DISPLAY.get().map(String::as_str)
}

/// Satellite em execução; encerrado quando o mide termina.
pub struct XWayland {
    child: Child,
    marker: Option<PathBuf>,
}

impl Drop for XWayland {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(marker) = &self.marker {
            let _ = std::fs::remove_file(marker);
        }
    }
}

/// Inicia o satellite conectado ao socket Wayland do mide. Sem o programa ou sem o
/// Xwayland, avisa no log e segue sem apps X11.
pub fn start(socket: &str) -> Option<XWayland> {
    // Um mide anterior encerrado à força pode ter deixado o arquivo do display.
    let marker = std::env::var_os("XDG_RUNTIME_DIR").map(|d| PathBuf::from(d).join(format!("{socket}.xdisplay")));
    if let Some(path) = &marker {
        let _ = std::fs::remove_file(path);
    }
    if std::env::var("MIDE_XWAYLAND").is_ok_and(|v| v == "0") {
        tracing::info!("XWayland desligado (MIDE_XWAYLAND=0)");
        return None;
    }
    let Some(satellite) = find_program("xwayland-satellite") else {
        tracing::info!("XWayland indisponível: instale o xwayland-satellite para abrir apps X11");
        return None;
    };
    if find_program("Xwayland").is_none() {
        tracing::info!("XWayland indisponível: instale o Xwayland (pacote xwayland)");
        return None;
    }
    let Some(n) = free_display(Path::new("/tmp")) else {
        tracing::warn!("XWayland: nenhum display X livre");
        return None;
    };
    let x_display = format!(":{n}");
    let mut command = Command::new(&satellite);
    command.arg(&x_display).env("WAYLAND_DISPLAY", socket);
    match command.spawn() {
        Ok(child) => {
            tracing::info!("XWayland: apps X11 no display {x_display} ({})", satellite.display());
            let _ = DISPLAY.set(x_display.clone());
            // O display também fica num arquivo ao lado do socket, para ferramentas e
            // testes abrirem apps X11 dentro deste mide.
            if let Some(path) = &marker {
                let _ = std::fs::write(path, &x_display);
            }
            Some(XWayland { child, marker })
        }
        Err(err) => {
            tracing::warn!("XWayland não iniciado ({}): {err}", satellite.display());
            None
        }
    }
}

/// Programa pelo nome: ao lado do executável do mide, no PATH ou em `~/.cargo/bin`
/// (onde o `cargo install` põe o satellite).
fn find_program(name: &str) -> Option<PathBuf> {
    let beside = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join(name)));
    let path = std::env::var_os("PATH").into_iter().flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>()).map(|d| d.join(name));
    let cargo = std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cargo/bin").join(name));
    beside.into_iter().chain(path).chain(cargo).find(|p| p.is_file())
}

/// Primeiro display X sem socket nem trava em `tmp` (a partir do `:1`; o `:0`
/// costuma ser de outra sessão).
fn free_display(tmp: &Path) -> Option<u32> {
    (1..64).find(|n| !tmp.join(format!(".X11-unix/X{n}")).exists() && !tmp.join(format!(".X{n}-lock")).exists())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_livre_pula_os_ocupados() {
        let dir = std::env::temp_dir().join(format!("mide-xw-teste-{}", std::process::id()));
        std::fs::create_dir_all(dir.join(".X11-unix")).unwrap();
        assert_eq!(free_display(&dir), Some(1));
        std::fs::write(dir.join(".X11-unix/X1"), "").unwrap();
        std::fs::write(dir.join(".X2-lock"), "").unwrap();
        assert_eq!(free_display(&dir), Some(3));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
