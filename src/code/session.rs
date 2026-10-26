//! Estado do `mide-code` guardado entre uma vez e outra: o projeto aberto, os arquivos
//! das abas e a aba escolhida. Aberto sem argumentos, o Code volta como estava
//! (reabrir o projeto da última vez, E11).
//!
//! Fica em `~/.local/state/mide/code-session.toml` (respeitando `XDG_STATE_HOME`).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Session {
    pub root: Option<PathBuf>,
    pub files: Vec<PathBuf>,
    pub current: usize,
    /// Pastas abertas na gaveta.
    pub expanded: Vec<PathBuf>,
}

/// `~/.local/state/mide/code-session.toml`.
pub fn path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("state")))?;
    Some(base.join("mide").join("code-session.toml"))
}

impl Session {
    pub fn load() -> Option<Self> {
        Self::load_from(&path()?)
    }

    /// Lê a sessão, sem os arquivos e pastas que não existem mais.
    pub fn load_from(file: &Path) -> Option<Self> {
        let mut s: Session = toml::from_str(&std::fs::read_to_string(file).ok()?).ok()?;
        let cur = s.files.get(s.current).cloned();
        s.files.retain(|f| f.is_file());
        s.current = cur.and_then(|c| s.files.iter().position(|f| *f == c)).unwrap_or(0);
        s.root = s.root.filter(|r| r.is_dir());
        s.expanded.retain(|d| d.is_dir());
        Some(s)
    }

    pub fn save(&self) -> std::io::Result<()> {
        self.save_to(&path().ok_or_else(|| std::io::Error::other("sem pasta de estado"))?)
    }

    pub fn save_to(&self, file: &Path) -> std::io::Result<()> {
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = toml::to_string(self).map_err(std::io::Error::other)?;
        crate::files::write_atomic(file, text.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guarda_e_volta_sem_o_que_sumiu() {
        let dir = std::env::temp_dir().join(format!("mide-code-sessao-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("proj/src")).unwrap();
        std::fs::write(dir.join("proj/a.rs"), "").unwrap();
        std::fs::write(dir.join("proj/b.rs"), "").unwrap();
        let s = Session {
            root: Some(dir.join("proj")),
            files: vec![dir.join("proj/a.rs"), dir.join("proj/sumiu.rs"), dir.join("proj/b.rs")],
            current: 2,
            expanded: vec![dir.join("proj/src"), dir.join("proj/nada")],
        };
        let file = dir.join("estado/code-session.toml");
        s.save_to(&file).unwrap();
        let back = Session::load_from(&file).unwrap();
        assert_eq!(back.files, vec![dir.join("proj/a.rs"), dir.join("proj/b.rs")]);
        // A aba escolhida continua sendo o b.rs.
        assert_eq!(back.current, 1);
        assert_eq!(back.expanded, vec![dir.join("proj/src")]);
        assert_eq!(back.root, Some(dir.join("proj")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
