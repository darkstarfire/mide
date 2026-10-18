//! Configuração do mide, lida de `~/.config/mide/config.toml`.
//!
//! Seções: `[keybindings]`, `[effects]` e `[desktop]`. Cada atalho liga uma combinação, como
//! `super+shift+q`, a uma ação. As ações são `close_window` ou
//! `launch:<comando>`, que executa um binário ou script.

use std::{collections::BTreeMap, error::Error, fmt, fs, path::PathBuf};

use serde::Deserialize;
use xkbcommon::xkb::{self, Keysym};

/// Ação disparada por um atalho.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Pede para fechar a janela que tem o foco.
    CloseWindow,
    /// Executa um comando pelo shell do usuário.
    Launch(String),
    /// Encerra o compositor.
    Quit,
    /// Traz de volta a janela minimizada mais recente.
    RestoreMinimized,
    /// Abre o launcher em tela cheia, ou fecha se já estiver aberto.
    ToggleLauncher,
}

/// Combinação de teclas já convertida, pronta para comparar com o input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    pub logo: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub key: Keysym,
    pub action: Action,
}

/// Modificadores pressionados no momento de uma tecla.
#[derive(Debug, Clone, Copy, Default)]
pub struct Modifiers {
    pub logo: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

/// Formato do arquivo. Atalhos podem ficar na seção `[keybindings]` ou soltos
/// no topo, como `"super+e" = "launch:..."`. Os dois formas são aceitos.
#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct FileConfig {
    keybindings: BTreeMap<String, String>,
    effects: Effects,
    desktop: Desktop,
    /// Qualquer outra chave de texto é tratada como atalho solto no topo.
    #[serde(flatten)]
    top_level: BTreeMap<String, toml::Value>,
}

impl FileConfig {
    /// Todos os atalhos do arquivo, nas duas formas.
    fn all_bindings(self) -> BTreeMap<String, String> {
        // Só textos soltos no topo são atalhos; seções de outros programas do mide
        // (como `[dock]`, das Configurações) são ignoradas aqui.
        let mut all: BTreeMap<String, String> = self
            .top_level
            .into_iter()
            .filter_map(|(k, v)| v.as_str().map(|s| (k, s.to_string())))
            .collect();
        all.extend(self.keybindings);
        all
    }
}

/// Efeitos visuais, cada um pode ser desligado na seção `[effects]` do arquivo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Effects {
    /// Sombra em volta das janelas.
    pub shadows: bool,
    /// Cantos arredondados e contorno fino das janelas.
    pub rounded_corners: bool,
    /// Animações de maximizar, restaurar e minimizar.
    pub animations: bool,
    /// Efeito lâmpada ao minimizar e restaurar (com animações ligadas).
    pub lamp: bool,
    /// Desfoque do que fica atrás da dock.
    pub dock_blur: bool,
    /// Fundo do launcher com o papel de parede desfocado.
    pub launcher_blur: bool,
    /// Vidro líquido na dock (refração nas bordas, reflexo e tom), sobre o desfoque.
    pub liquid_glass: bool,
}

impl Default for Effects {
    fn default() -> Self {
        Self {
            shadows: true,
            rounded_corners: true,
            animations: true,
            lamp: true,
            dock_blur: true,
            launcher_blur: true,
            liquid_glass: true,
        }
    }
}

/// Área de trabalho, na seção `[desktop]` do arquivo.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Desktop {
    /// Imagem do papel de parede (PNG ou JPEG). `~/` é o diretório do usuário.
    /// Sem imagem, o fundo é um gradiente próprio do mide.
    pub wallpaper: Option<String>,
}

impl Desktop {
    /// Caminho do papel de parede com `~/` expandido.
    pub fn wallpaper_path(&self) -> Option<PathBuf> {
        let raw = self.wallpaper.as_deref()?.trim();
        if raw.is_empty() {
            return None;
        }
        match raw.strip_prefix("~/") {
            Some(rest) => std::env::var_os("HOME").map(|h| PathBuf::from(h).join(rest)),
            None => Some(PathBuf::from(raw)),
        }
    }
}

/// Atalhos padrão. Um atalho do arquivo com a mesma combinação substitui o padrão.
const DEFAULT_KEYBINDINGS: &[(&str, &str)] = &[
    // Super é capturada pelo desktop hospedeiro durante o desenvolvimento.
    ("ctrl+shift+q", "close_window"),
    ("ctrl+shift+escape", "quit"),
    ("ctrl+shift+m", "restore_minimized"),
    // foot é nativo de Wayland e não depende de servidor compartilhado. Sem ele,
    // usa o terminal padrão do sistema.
    ("ctrl+shift+t", "launch:foot || x-terminal-emulator"),
    // Buscador de programas externo, até o launcher próprio do mide (etapa B6).
    ("ctrl+shift+space", "toggle_launcher"),
];

/// Configuração carregada, com os atalhos já convertidos.
#[derive(Debug, Clone)]
pub struct Config {
    pub bindings: Vec<Binding>,
    pub effects: Effects,
    pub desktop: Desktop,
}

impl Config {
    /// Carrega o arquivo do usuário, se existir, sobre os atalhos padrão.
    pub fn load() -> Result<Self, Box<dyn Error>> {
        let (user, effects, desktop) = match config_path() {
            Some(path) if path.exists() => {
                let text = fs::read_to_string(&path)?;
                let mut file = toml::from_str::<FileConfig>(&text)
                    .map_err(|e| format!("{}: {e}", path.display()))?;
                let effects = file.effects;
                let desktop = std::mem::take(&mut file.desktop);
                (file.all_bindings(), effects, desktop)
            }
            _ => (BTreeMap::new(), Effects::default(), Desktop::default()),
        };
        let mut config = Self::from_entries(&user)?;
        config.effects = effects;
        config.desktop = desktop;
        Ok(config)
    }

    /// Monta a configuração a partir dos atalhos do usuário, sobre os padrões.
    pub fn from_entries(user: &BTreeMap<String, String>) -> Result<Self, Box<dyn Error>> {
        let mut merged: BTreeMap<String, String> = DEFAULT_KEYBINDINGS
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        for (combo, action) in user {
            merged.insert(combo.clone(), action.clone());
        }

        let bindings = merged
            .iter()
            .map(|(combo, action)| {
                let mut binding = parse_combo(combo)?;
                binding.action = parse_action(action)?;
                Ok(binding)
            })
            .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
        Ok(Self {
            bindings,
            effects: Effects::default(),
            desktop: Desktop::default(),
        })
    }

    /// Ação do primeiro atalho que casa com a tecla e os modificadores.
    pub fn match_binding(&self, mods: Modifiers, key: Keysym) -> Option<Action> {
        self.bindings
            .iter()
            .find(|b| {
                b.key == key
                    && b.logo == mods.logo
                    && b.ctrl == mods.ctrl
                    && b.alt == mods.alt
                    && b.shift == mods.shift
            })
            .map(|b| b.action.clone())
    }
}

/// Caminho de `config.toml`, respeitando `XDG_CONFIG_HOME`.
fn config_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("mide").join("config.toml"))
}

/// Converte `super+shift+q` em uma combinação. A ação fica como `CloseWindow` e
/// é trocada depois por `parse_action`.
fn parse_combo(combo: &str) -> Result<Binding, Box<dyn Error>> {
    let mut binding = Binding {
        logo: false,
        ctrl: false,
        alt: false,
        shift: false,
        key: Keysym::NoSymbol,
        action: Action::CloseWindow,
    };
    let mut key_name = None;

    for part in combo.split('+').map(str::trim).filter(|p| !p.is_empty()) {
        match part.to_ascii_lowercase().as_str() {
            "super" | "logo" | "mod4" => binding.logo = true,
            "ctrl" | "control" => binding.ctrl = true,
            "alt" | "mod1" => binding.alt = true,
            "shift" => binding.shift = true,
            _ => key_name = Some(part),
        }
    }

    let name = key_name.ok_or_else(|| format!("atalho sem tecla: '{combo}'"))?;
    let key = xkb::keysym_from_name(name, xkb::KEYSYM_CASE_INSENSITIVE);
    if key == Keysym::NoSymbol {
        return Err(format!("tecla desconhecida em '{combo}': '{name}'").into());
    }
    binding.key = key;
    Ok(binding)
}

fn parse_action(action: &str) -> Result<Action, Box<dyn Error>> {
    if action == "close_window" {
        return Ok(Action::CloseWindow);
    }
    if action == "quit" {
        return Ok(Action::Quit);
    }
    if action == "restore_minimized" {
        return Ok(Action::RestoreMinimized);
    }
    if action == "toggle_launcher" {
        return Ok(Action::ToggleLauncher);
    }
    if let Some(command) = action.strip_prefix("launch:") {
        if command.trim().is_empty() {
            return Err("ação launch: sem comando".into());
        }
        return Ok(Action::Launch(command.trim().to_string()));
    }
    Err(format!("ação desconhecida: '{action}'").into())
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Action::CloseWindow => write!(f, "close_window"),
            Action::Launch(cmd) => write!(f, "launch:{cmd}"),
            Action::Quit => write!(f, "quit"),
            Action::RestoreMinimized => write!(f, "restore_minimized"),
            Action::ToggleLauncher => write!(f, "toggle_launcher"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(entries: &[(&str, &str)]) -> Result<Config, Box<dyn Error>> {
        let map = entries
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        Config::from_entries(&map)
    }

    fn sym(name: &str) -> Keysym {
        xkb::keysym_from_name(name, xkb::KEYSYM_CASE_INSENSITIVE)
    }

    #[test]
    fn padroes_existem_sem_arquivo() {
        let c = cfg(&[]).unwrap();
        let ctrl_shift = Modifiers { ctrl: true, shift: true, ..Default::default() };
        assert_eq!(c.match_binding(ctrl_shift, sym("q")), Some(Action::CloseWindow));
    }

    #[test]
    fn atalho_de_sair_existe_por_padrao() {
        let c = cfg(&[]).unwrap();
        let mods = Modifiers { ctrl: true, shift: true, ..Default::default() };
        assert_eq!(c.match_binding(mods, sym("escape")), Some(Action::Quit));
    }

    #[test]
    fn atalhos_principais_de_terminal_e_buscador() {
        let c = cfg(&[]).unwrap();
        let ctrl_shift = Modifiers { ctrl: true, shift: true, ..Default::default() };
        assert_eq!(
            c.match_binding(ctrl_shift, sym("t")),
            Some(Action::Launch("foot || x-terminal-emulator".into()))
        );
        assert_eq!(
            c.match_binding(ctrl_shift, sym("space")),
            Some(Action::ToggleLauncher)
        );
        assert_eq!(parse_action("toggle_launcher").unwrap(), Action::ToggleLauncher);
    }

    #[test]
    fn combinacao_com_varios_modificadores() {
        let c = cfg(&[("super+shift+t", "launch:foot")]).unwrap();
        let mods = Modifiers { logo: true, shift: true, ..Default::default() };
        assert_eq!(
            c.match_binding(mods, sym("t")),
            Some(Action::Launch("foot".into()))
        );
    }

    #[test]
    fn modificador_extra_nao_casa() {
        let c = cfg(&[]).unwrap();
        let mods = Modifiers { logo: true, shift: true, ..Default::default() };
        assert_eq!(c.match_binding(mods, sym("q")), None);
    }

    #[test]
    fn letra_maiuscula_no_arquivo_funciona() {
        let c = cfg(&[("Ctrl+Alt+Q", "close_window")]).unwrap();
        let mods = Modifiers { ctrl: true, alt: true, ..Default::default() };
        assert_eq!(c.match_binding(mods, sym("q")), Some(Action::CloseWindow));
    }

    #[test]
    fn usuario_substitui_padrao_da_mesma_combinacao() {
        let c = cfg(&[("ctrl+shift+q", "launch:htop")]).unwrap();
        let mods = Modifiers { ctrl: true, shift: true, ..Default::default() };
        assert_eq!(
            c.match_binding(mods, sym("q")),
            Some(Action::Launch("htop".into()))
        );
    }

    #[test]
    fn tecla_desconhecida_e_erro() {
        assert!(cfg(&[("super+naoexiste", "close_window")]).is_err());
    }

    #[test]
    fn acao_desconhecida_e_erro() {
        assert!(cfg(&[("super+q", "explodir")]).is_err());
    }

    #[test]
    fn launch_sem_comando_e_erro() {
        assert!(cfg(&[("super+x", "launch:   ")]).is_err());
    }

    #[test]
    fn arquivo_toml_com_secao_e_lido() {
        let file: FileConfig = toml::from_str(
            r#"
            [keybindings]
            "super+e" = "launch:nautilus"
            "#,
        )
        .unwrap();
        let all = file.all_bindings();
        assert_eq!(all.get("super+e").map(String::as_str), Some("launch:nautilus"));
    }

    #[test]
    fn efeitos_ligados_por_padrao_e_desligaveis() {
        let file: FileConfig = toml::from_str("").unwrap();
        assert_eq!(file.effects, Effects::default());
        let file: FileConfig = toml::from_str(
            r#"
            [effects]
            shadows = false
            animations = false
            "#,
        )
        .unwrap();
        assert!(!file.effects.shadows);
        assert!(!file.effects.animations);
        assert!(file.effects.rounded_corners);
    }

    #[test]
    fn secoes_de_outros_programas_nao_quebram_a_leitura() {
        let file: FileConfig = toml::from_str(
            r#"
            "ctrl+e" = "launch:foot"
            [dock]
            [effects]
            shadows = false
            [outra]
            chave = 3
            "#,
        )
        .unwrap();
        assert!(!file.effects.shadows);
        assert_eq!(file.all_bindings().len(), 1);
    }

    #[test]
    fn arquivo_toml_com_atalho_no_topo_e_lido() {
        let file: FileConfig = toml::from_str(r#""super+e" = "launch:nautilus""#).unwrap();
        let all = file.all_bindings();
        assert_eq!(all.get("super+e").map(String::as_str), Some("launch:nautilus"));
    }

    #[test]
    fn papel_de_parede_lido_e_til_expandido() {
        let file: FileConfig = toml::from_str("").unwrap();
        assert_eq!(file.desktop.wallpaper_path(), None);
        let file: FileConfig = toml::from_str(
            r#"
            "ctrl+e" = "launch:foot"
            [desktop]
            wallpaper = "~/Imagens/fundo.jpg"
            "#,
        )
        .unwrap();
        let path = file.desktop.wallpaper_path().unwrap();
        assert!(path.ends_with("Imagens/fundo.jpg"));
        assert!(!path.starts_with("~"));
        assert_eq!(file.all_bindings().len(), 1);
    }
}
