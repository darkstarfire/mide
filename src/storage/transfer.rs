//! Copiar e recortar arquivos entre janelas e a área de trabalho.

use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};
use crate::{control_client::ask_compositor, files, thumbnails};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Clipboard {
    pub paths: Vec<PathBuf>,
    pub cut: bool,
    /// Identifica a seleção: terminar um movimento não apaga uma cópia mais recente.
    pub id: u64,
}

impl Clipboard {
    pub fn uris(&self) -> String {
        self.paths.iter().map(|p| thumbnails::file_uri(p)).collect::<Vec<_>>().join("\r\n")
    }

    pub fn complete(&mut self, id: u64, paths: &[PathBuf]) {
        if self.cut && self.id == id {
            self.paths.retain(|p| !paths.contains(p));
        }
    }
}

pub fn publish(paths: &[PathBuf], cut: bool) -> bool {
    let clipboard = Clipboard { paths: paths.to_vec(), cut, id: 0 };
    let Ok(json) = serde_json::to_string(&clipboard) else { return false };
    ask_compositor(&format!("files-copy {json}")).is_some_and(|r| r.first().is_some_and(|s| s == "ok"))
}

pub fn current() -> Option<Clipboard> {
    serde_json::from_str(&ask_compositor("files-paste")?.join("\n")).ok()
}

/// O que está recortado agora (para desenhar esmaecido). `None` quando o compositor não
/// guarda arquivos (versão antiga): o app usa o que ele mesmo recortou.
pub fn cut_paths() -> Option<Vec<PathBuf>> {
    let reply = ask_compositor("files-paste")?.join("\n");
    if reply.trim() == "null" {
        return Some(Vec::new());
    }
    let clipboard: Clipboard = serde_json::from_str(&reply).ok()?;
    Some(if clipboard.cut { clipboard.paths } else { Vec::new() })
}

#[derive(Default)]
pub struct Pasted {
    pub done: Vec<(PathBuf, PathBuf)>,
    pub errors: Vec<String>,
}

/// Falhas ficam disponíveis para nova tentativa; só os movimentos concluídos saem
/// da área de transferência. O chamador guarda `done` para desfazer.
pub fn paste(clipboard: &Clipboard, dir: &Path) -> Pasted {
    let result = transfer(clipboard, dir);
    if clipboard.cut && !result.done.is_empty() {
        let completed = Clipboard { paths: result.done.iter().map(|(p, _)| p.clone()).collect(), ..clipboard.clone() };
        if let Ok(json) = serde_json::to_string(&completed) {
            ask_compositor(&format!("files-pasted {json}"));
        }
    }
    result
}

fn transfer(clipboard: &Clipboard, dir: &Path) -> Pasted {
    let mut result = Pasted::default();
    for path in &clipboard.paths {
        // Colar um recorte na pasta de origem não cria duplicata nem um desfazer falso.
        if clipboard.cut && path.parent() == Some(dir) {
            continue;
        }
        let operation = if clipboard.cut { files::move_into(path, dir) } else { files::copy_into(path, dir) };
        match operation {
            Ok(dest) => result.done.push((path.clone(), dest)),
            Err(err) => result.errors.push(format!("{}: {err}", path.display())),
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recortar_move_sem_sobrescrever_e_preserva_falhas() {
        let base = std::env::temp_dir().join(format!("mide-transfer-{}", std::process::id()));
        let src = base.join("src");
        let dst = base.join("dst");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&dst).unwrap();
        let file = src.join("app.desktop");
        std::fs::write(&file, "atalho").unwrap();
        std::fs::write(dst.join("app.desktop"), "existente").unwrap();
        let missing = src.join("ausente");
        let mut clipboard = Clipboard { paths: vec![file.clone(), missing.clone()], cut: true, id: 7 };
        assert!(transfer(&clipboard, &src).done.is_empty());
        let result = transfer(&clipboard, &dst);
        assert_eq!(result.done.len(), 1);
        assert_eq!(result.errors.len(), 1);
        assert!(!file.exists());
        assert_eq!(std::fs::read_to_string(dst.join("app.desktop")).unwrap(), "existente");
        assert_eq!(std::fs::read_to_string(&result.done[0].1).unwrap(), "atalho");
        clipboard.complete(6, &[file.clone()]);
        assert_eq!(clipboard.paths.len(), 2);
        clipboard.complete(7, &[file]);
        assert_eq!(clipboard.paths, vec![missing]);
        std::fs::remove_dir_all(base).unwrap();
    }
}
