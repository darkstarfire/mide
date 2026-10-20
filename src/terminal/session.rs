//! Sessão do terminal guardada ao fechar a janela (como o "restaurar janelas" do
//! iTerm2): as abas, a divisão dos painéis e, de cada painel, a pasta e o perfil.
//! Os programas não voltam (um shell novo abre em cada pasta).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::layout::{Layout, PaneId, Split};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum SavedLayout {
    Pane {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        dir: Option<PathBuf>,
        #[serde(default)]
        profile: String,
    },
    Split {
        split: Split,
        ratio: f32,
        a: Box<SavedLayout>,
        b: Box<SavedLayout>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedTab {
    pub layout: SavedLayout,
    /// Painel em foco (posição na ordem de leitura).
    #[serde(default)]
    pub focus: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Saved {
    pub tabs: Vec<SavedTab>,
    #[serde(default)]
    pub active: usize,
    #[serde(default)]
    pub font_size: Option<f32>,
}

impl SavedLayout {
    /// A árvore de painéis, com a pasta e o perfil de cada um (`info`).
    pub fn from_layout(layout: &Layout, info: &impl Fn(PaneId) -> (Option<PathBuf>, String)) -> Self {
        match layout {
            Layout::Pane(id) => {
                let (dir, profile) = info(*id);
                SavedLayout::Pane { dir, profile }
            }
            Layout::Split { dir, ratio, a, b } => SavedLayout::Split {
                split: *dir,
                ratio: *ratio,
                a: Box::new(Self::from_layout(a, info)),
                b: Box::new(Self::from_layout(b, info)),
            },
        }
    }

    /// Monta a árvore de volta, abrindo cada painel com `open` (pasta, perfil); um
    /// painel que não abre some e o outro lado ocupa o lugar.
    pub fn to_layout(&self, open: &mut impl FnMut(Option<&PathBuf>, &str) -> Option<PaneId>) -> Option<Layout> {
        match self {
            SavedLayout::Pane { dir, profile } => open(dir.as_ref(), profile).map(Layout::Pane),
            SavedLayout::Split { split, ratio, a, b } => match (a.to_layout(open), b.to_layout(open)) {
                (Some(a), Some(b)) => Some(Layout::Split { dir: *split, ratio: ratio.clamp(0.1, 0.9), a: Box::new(a), b: Box::new(b) }),
                (one, None) | (None, one) => one,
            },
        }
    }
}

impl Saved {
    pub fn load() -> Option<Self> {
        let text = std::fs::read_to_string(path()?).ok()?;
        toml::from_str::<Saved>(&text).ok().filter(|s| !s.tabs.is_empty())
    }

    pub fn save(&self) -> std::io::Result<()> {
        let path = path().ok_or_else(|| std::io::Error::other("sem pasta de estado"))?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = toml::to_string_pretty(self).map_err(std::io::Error::other)?;
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(tmp, path)
    }
}

/// `~/.local/state/mide/terminal-session.toml`, respeitando `XDG_STATE_HOME`.
pub fn path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("state")))?;
    Some(base.join("mide").join("terminal-session.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guarda_e_remonta_os_paineis() {
        let mut layout = Layout::Pane(1);
        layout.split(1, 2, Split::Side);
        layout.split(2, 3, Split::Stack);
        let info = |id: PaneId| (Some(PathBuf::from(format!("/p{id}"))), if id == 3 { "Escuro".into() } else { "Padrão".into() });
        let saved = Saved { tabs: vec![SavedTab { layout: SavedLayout::from_layout(&layout, &info), focus: 2 }], active: 0, font_size: Some(14.0) };
        let text = toml::to_string_pretty(&saved).unwrap();
        let back: Saved = toml::from_str(&text).unwrap();
        assert_eq!(back, saved);

        let mut opened = Vec::new();
        let mut next = 10;
        let rebuilt = back.tabs[0]
            .layout
            .to_layout(&mut |dir, profile| {
                opened.push((dir.cloned(), profile.to_string()));
                next += 1;
                Some(next)
            })
            .unwrap();
        assert_eq!(rebuilt.panes(), vec![11, 12, 13]);
        assert_eq!(opened[2], (Some(PathBuf::from("/p3")), "Escuro".to_string()));
    }

    #[test]
    fn painel_que_nao_abre_some() {
        let saved = SavedLayout::Split {
            split: Split::Side,
            ratio: 0.5,
            a: Box::new(SavedLayout::Pane { dir: None, profile: String::new() }),
            b: Box::new(SavedLayout::Pane { dir: None, profile: "falha".into() }),
        };
        let layout = saved.to_layout(&mut |_, profile| (profile != "falha").then_some(7)).unwrap();
        assert_eq!(layout, Layout::Pane(7));
    }
}
