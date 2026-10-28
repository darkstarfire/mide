//! Perfis do terminal (ideia do iTerm2): cada perfil junta tema de cores, fonte,
//! cursor, histórico e o que abrir. Ficam em `~/.config/mide/terminal.toml`, mas o
//! usuário nunca edita o arquivo: tudo muda na janela de Preferências do terminal.

use std::{borrow::Cow, collections::BTreeMap, path::PathBuf};

use serde::{Deserialize, Serialize};

use super::{
    palette::{self, Theme},
    screen::Rgb,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Cursor {
    #[default]
    Block,
    Bar,
    Underline,
}

/// O que um gatilho faz quando o texto aparece.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum TriggerAction {
    /// Pinta o fundo do texto encontrado.
    #[default]
    Highlight,
    /// Notificação do mide com a linha.
    Notify,
    /// O painel pisca (como o sino).
    Bell,
}

/// Gatilho (ideia do iTerm2): quando `text` aparece na saída, faz `action`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Trigger {
    pub text: String,
    pub action: TriggerAction,
    /// Cor do destaque (`#rrggbb`).
    pub color: String,
}

impl Default for Trigger {
    fn default() -> Self {
        Self { text: String::new(), action: TriggerAction::Highlight, color: "#ffd84d".into() }
    }
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
    /// Família da fonte (vazio: a de largura fixa padrão do sistema).
    pub font: String,
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
    /// Fundo translúcido (1 = opaco).
    pub opacity: f32,
    /// Desfocar o que está atrás do fundo translúcido (vidro).
    pub blur: bool,
    /// Cores trocadas pelo usuário por cima do tema (`COLOR_SLOTS` → `#rrggbb`).
    pub colors: BTreeMap<String, String>,
    /// Integração do shell (marcas dos prompts, aviso de comando terminado, `imgcat`).
    pub shell_integration: bool,
    /// Avisar quando um comando que levou pelo menos estes segundos termina com a
    /// janela sem foco (0: nunca).
    pub notify_after: u32,
    /// Gatilhos: texto na saída → destacar, avisar ou piscar.
    pub triggers: Vec<Trigger>,
}

/// Cores que o usuário pode trocar (chaves gravadas no perfil; o nome na tela vem de
/// [`color_slot_label`]).
pub const COLOR_SLOTS: [&str; 20] = [
    "foreground", "background", "cursor", "selection", "ansi0", "ansi1", "ansi2", "ansi3", "ansi4", "ansi5", "ansi6", "ansi7", "ansi8", "ansi9", "ansi10", "ansi11",
    "ansi12", "ansi13", "ansi14", "ansi15",
];

/// Nome na tela (traduzido) de uma cor de `COLOR_SLOTS`.
pub fn color_slot_label(key: &str) -> Cow<'static, str> {
    match key {
        "foreground" => crate::tr!("Text"),
        "background" => crate::tr!("Background"),
        "cursor" => crate::tr!("Cursor"),
        "selection" => crate::tr!("Selection"),
        "ansi0" => crate::tr!("Black"),
        "ansi1" => crate::tr!("Red"),
        "ansi2" => crate::tr!("Green"),
        "ansi3" => crate::tr!("Yellow"),
        "ansi4" => crate::tr!("Blue"),
        "ansi5" => crate::tr!("Magenta"),
        "ansi6" => crate::tr!("Cyan"),
        "ansi7" => crate::tr!("White"),
        "ansi8" => crate::tr!("Bright Black"),
        "ansi9" => crate::tr!("Bright Red"),
        "ansi10" => crate::tr!("Bright Green"),
        "ansi11" => crate::tr!("Bright Yellow"),
        "ansi12" => crate::tr!("Bright Blue"),
        "ansi13" => crate::tr!("Bright Magenta"),
        "ansi14" => crate::tr!("Bright Cyan"),
        "ansi15" => crate::tr!("Bright White"),
        other => Cow::Owned(other.to_string()),
    }
}

/// Nome do perfil criado pelo mide. Fica gravado assim no `terminal.toml` (é a chave
/// do perfil, e arquivos antigos já o usam); na tela aparece traduzido ([`profile_label`]).
pub const DEFAULT_NAME: &str = "Padrão";

/// Nome de um perfil na tela: o perfil padrão do mide aparece no idioma da sessão; os
/// outros, como o usuário os chamou.
pub fn profile_label(name: &str) -> Cow<'_, str> {
    if name == DEFAULT_NAME { crate::tr!("Default") } else { Cow::Borrowed(name) }
}

impl Profile {
    /// Tema do perfil com as cores trocadas pelo usuário.
    pub fn theme(&self) -> Theme {
        let mut theme = palette::theme(&self.theme);
        for (key, value) in &self.colors {
            if let Some(rgb) = parse_hex(value) {
                *slot_mut(&mut theme, key) = rgb;
            }
        }
        theme
    }

    /// Cor de um espaço (`COLOR_SLOTS`) como está agora, em `#rrggbb`.
    pub fn color_hex(&self, key: &str) -> String {
        let mut theme = self.theme();
        let rgb = *slot_mut(&mut theme, key);
        format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2])
    }
}

fn slot_mut<'a>(theme: &'a mut Theme, key: &str) -> &'a mut Rgb {
    match key {
        "foreground" => &mut theme.foreground,
        "background" => &mut theme.background,
        "cursor" => &mut theme.cursor,
        "selection" => &mut theme.selection,
        other => {
            let i = other.strip_prefix("ansi").and_then(|n| n.parse::<usize>().ok()).unwrap_or(0).min(15);
            &mut theme.ansi[i]
        }
    }
}

/// `#rrggbb` (ou `rrggbb`) em RGB.
pub fn parse_hex(text: &str) -> Option<Rgb> {
    let hex = text.trim().trim_start_matches('#');
    if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let v = u32::from_str_radix(hex, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            name: DEFAULT_NAME.into(),
            theme: "Claro".into(),
            font_size: 13.0,
            font: String::new(),
            cursor: Cursor::Block,
            cursor_blink: false,
            scrollback: super::screen::HISTORY,
            bold_bright: true,
            start_dir: StartDir::Same,
            command: String::new(),
            visual_bell: true,
            opacity: 1.0,
            blur: true,
            colors: BTreeMap::new(),
            shell_integration: true,
            notify_after: 10,
            triggers: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Profiles {
    /// Perfil das janelas e abas novas.
    pub default: String,
    /// Reabrir as abas e painéis da última janela fechada (com as pastas).
    pub restore: bool,
    /// Altura do terminal de atalho (fração da área útil da tela).
    pub hotkey_height: f32,
    pub profiles: Vec<Profile>,
}

impl Default for Profiles {
    fn default() -> Self {
        Self { default: DEFAULT_NAME.into(), restore: false, hotkey_height: 0.45, profiles: vec![Profile::default()] }
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
            p.opacity = p.opacity.clamp(0.3, 1.0);
            p.colors.retain(|k, v| COLOR_SLOTS.iter().any(|s| s == k) && parse_hex(v).is_some());
        }
        self.hotkey_height = self.hotkey_height.clamp(0.2, 0.95);
        if !self.profiles.iter().any(|p| p.name == self.default) {
            self.default = self.profiles[0].name.clone();
        }
        self
    }

    /// Perfil pelo nome; vazio é o padrão (o que estiver marcado como padrão agora).
    pub fn get(&self, name: &str) -> &Profile {
        let name = if name.is_empty() { self.default.as_str() } else { name };
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
    fn cores_trocadas_por_cima_do_tema() {
        let mut p = Profile::default();
        p.colors.insert("background".into(), "#102030".into());
        p.colors.insert("ansi9".into(), "ff0000".into());
        let t = p.theme();
        assert_eq!(t.background, [16, 32, 48]);
        assert_eq!(t.ansi[9], [255, 0, 0]);
        assert_eq!(t.foreground, palette::theme("Claro").foreground);
        assert_eq!(p.color_hex("background"), "#102030");
        assert_eq!(parse_hex("#12345"), None);
        // Chaves desconhecidas e cores inválidas somem ao ler.
        let text = "[[profiles]]\nname = \"A\"\nopacity = 0.1\n[profiles.colors]\nbackground = \"#000000\"\nnada = \"#ffffff\"\ncursor = \"azul\"\n";
        let ps = toml::from_str::<Profiles>(text).unwrap().fixed();
        assert_eq!(ps.profiles[0].opacity, 0.3);
        assert_eq!(ps.profiles[0].colors.len(), 1);
    }

    #[test]
    fn grava_e_le_de_volta() {
        let p = Profiles::default();
        let text = toml::to_string_pretty(&p).unwrap();
        assert_eq!(toml::from_str::<Profiles>(&text).unwrap(), p);
    }
}
