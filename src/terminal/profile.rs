//! Perfis do terminal (ideia do iTerm2): cada perfil junta tema de cores, fonte,
//! cursor, histórico e o que abrir. Ficam em `~/.config/mide/terminal.toml`, mas o
//! usuário nunca edita o arquivo: tudo muda na janela de Preferências do terminal.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Cursor {
    #[default]
    Block,
    Bar,
    Underline,
}

/// Pasta onde abas e painéis novos começam.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum StartDir {
    /// A mesma do painel em foco (como "Reuse previous session's directory").
    #[default]
    Same,
    Home,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Profile {
    pub name: String,
    /// Nome do tema de cores (`palette::THEMES`).
    pub theme: String,
    pub font_size: f32,
    pub cursor: Cursor,
    pub cursor_blink: bool,
    /// Linhas guardadas no histórico.
    pub scrollback: usize,
    /// Texto em negrito também em cor clara (como os terminais antigos).
    pub bold_bright: bool,
    pub start_dir: StartDir,
    /// Programa no lugar do shell (vazio: o shell do usuário).
    pub command: String,
    /// Avisar com um piscar quando o programa toca o sino.
    pub visual_bell: bool,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            name: "Padrão".into(),
            theme: "Claro".into(),
            font_size: 13.0,
            cursor: Cursor::Block,
            cursor_blink: false,
            scrollback: super::screen::HISTORY,
            bold_bright: true,
            start_dir: StartDir::Same,
            command: String::new(),
            visual_bell: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Profiles {
    /// Perfil das janelas e abas novas.
    pub default: String,
    pub profiles: Vec<Profile>,
}

impl Default for Profiles {
    fn default() -> Self {
        Self { default: "Padrão".into(), profiles: vec![Profile::default()] }
    }
}

impl Profiles {
    /// Lê o arquivo (ou os perfis padrão, se não existir ou estiver com erro).
    pub fn load() -> Self {
        path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|text| toml::from_str::<Profiles>(&text).ok())
            .map(Profiles::fixed)
            .unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        let path = path().ok_or_else(|| std::io::Error::other("sem pasta de configuração"))?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = toml::to_string_pretty(self).map_err(std::io::Error::other)?;
        // Grava num temporário e troca: um erro no meio não estraga o arquivo.
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(tmp, path)
    }

    /// Sempre há pelo menos um perfil, os nomes não se repetem e o padrão existe.
    fn fixed(mut self) -> Self {
        if self.profiles.is_empty() {
            self.profiles.push(Profile::default());
        }
        let mut seen = std::collections::HashSet::new();
        for p in &mut self.profiles {
            if p.name.trim().is_empty() {
                p.name = "Perfil".into();
            }
            let base = p.name.clone();
            let mut n = 2;
            while !seen.insert(p.name.clone()) {
                p.name = format!("{base} {n}");
                n += 1;
            }
            p.font_size = p.font_size.clamp(8.0, 32.0);
            p.scrollback = p.scrollback.clamp(100, 200_000);
        }
        if !self.profiles.iter().any(|p| p.name == self.default) {
            self.default = self.profiles[0].name.clone();
        }
        self
    }

    pub fn get(&self, name: &str) -> &Profile {
        self.profiles.iter().find(|p| p.name == name).unwrap_or(&self.profiles[0])
    }

    pub fn default_profile(&self) -> &Profile {
        self.get(&self.default)
    }

    /// Nome livre a partir de `base` ("Perfil", "Perfil 2"...).
    pub fn free_name(&self, base: &str) -> String {
        let mut name = base.to_string();
        let mut n = 2;
        while self.profiles.iter().any(|p| p.name == name) {
            name = format!("{base} {n}");
            n += 1;
        }
        name
    }
}

/// `~/.config/mide/terminal.toml`, respeitando `XDG_CONFIG_HOME`.
pub fn path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("mide").join("terminal.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_com_campos_faltando_e_corrige() {
        let text = "default = \"Sumiu\"\n[[profiles]]\nname = \"A\"\nfont_size = 99\n[[profiles]]\nname = \"A\"\ntheme = \"Escuro\"\n";
        let p = toml::from_str::<Profiles>(text).unwrap().fixed();
        assert_eq!(p.profiles.len(), 2);
        assert_eq!(p.profiles[0].font_size, 32.0);
        assert_eq!(p.profiles[0].cursor, Cursor::Block);
        assert_eq!(p.profiles[1].name, "A 2");
        assert_eq!(p.default, "A");
        assert_eq!(p.get("A 2").theme, "Escuro");
        assert_eq!(p.free_name("A"), "A 3");
    }

    #[test]
    fn grava_e_le_de_volta() {
        let p = Profiles::default();
        let text = toml::to_string_pretty(&p).unwrap();
        assert_eq!(toml::from_str::<Profiles>(&text).unwrap(), p);
    }
}
