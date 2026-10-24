//! Configuração do mide, lida de `~/.config/mide/config.toml`.
//!
//! Seções: `[keybindings]`, `[effects]` e `[desktop]`. Cada atalho liga uma combinação, como
//! `super+shift+q`, a uma ação. As ações são `close_window` ou
//! `launch:<comando>`, que executa um binário ou script; `none` desliga a combinação
//! (inclusive um atalho padrão). Quem grava os atalhos é a tela Teclado das
//! Configurações: o usuário não edita o arquivo.

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
    /// Abre o buscador (estilo Spotlight), ou fecha se já estiver aberto.
    ToggleSearch,
    /// Passa para o próximo layout de teclado (com mais de um nas Configurações).
    NextLayout,
    /// Janela em foco: maximizar, restaurar (sair de maximizada ou da metade da tela),
    /// alternar entre os dois, minimizar e ocupar a metade esquerda ou direita.
    Maximize,
    Unmaximize,
    ToggleMaximize,
    Minimize,
    TileLeft,
    TileRight,
    /// Alternar janelas (Alt+Tab): para a próxima, ou para a anterior.
    SwitchWindows,
    SwitchWindowsBack,
    /// Minimiza todas as janelas (mostra a área de trabalho).
    ShowDesktop,
    /// Bloqueia a tela.
    LockScreen,
    /// Teclas no estilo Mac num terminal: ⌘+letra vira Ctrl+Shift+letra (⌘C copia e ⌘V
    /// cola, como no Terminal do Mac). Interno, não vem do arquivo.
    TerminalKey(char),
    /// Troca para o console de texto `n` (Ctrl+Alt+F`n` no backend DRM). Fixo, não
    /// vem do arquivo: é o jeito de sair do mide para outro console.
    SwitchVt(i32),
    /// Botão de ligar do computador: pergunta antes de desligar (a confirmação da
    /// barra), em vez do logind desligar direto. Fixo, não vem do arquivo.
    PowerButton,
    /// Teclas multimídia do teclado (volume, brilho, mídia). Fixas, não vêm do arquivo.
    Media(Media),
}

/// Uma tecla multimídia.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Media {
    VolumeUp,
    VolumeDown,
    Mute,
    MicMute,
    BrightnessUp,
    BrightnessDown,
    PlayPause,
    Pause,
    Stop,
    Next,
    Previous,
}

impl Media {
    /// A tecla (keysym do xkb) é multimídia?
    pub fn from_keysym(raw: u32) -> Option<Media> {
        use xkbcommon::xkb::keysyms as k;
        Some(match raw {
            k::KEY_XF86AudioRaiseVolume => Media::VolumeUp,
            k::KEY_XF86AudioLowerVolume => Media::VolumeDown,
            k::KEY_XF86AudioMute => Media::Mute,
            k::KEY_XF86AudioMicMute => Media::MicMute,
            k::KEY_XF86MonBrightnessUp => Media::BrightnessUp,
            k::KEY_XF86MonBrightnessDown => Media::BrightnessDown,
            k::KEY_XF86AudioPlay => Media::PlayPause,
            k::KEY_XF86AudioPause => Media::Pause,
            k::KEY_XF86AudioStop => Media::Stop,
            k::KEY_XF86AudioNext => Media::Next,
            k::KEY_XF86AudioPrev => Media::Previous,
            _ => return None,
        })
    }

    /// Vale com a tela bloqueada (volume e brilho; a mídia, não).
    pub fn when_locked(self) -> bool {
        matches!(self, Media::VolumeUp | Media::VolumeDown | Media::Mute | Media::MicMute | Media::BrightnessUp | Media::BrightnessDown)
    }
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
    keyboard: Keyboard,
    monitors: Monitors,
    input: Input,
    lock: Lock,
    opacity: Opacity,
    appearance: Appearance,
    power: Power,
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
    /// Vidro (desfoque e vidro líquido) atrás da barra, dos menus e das notificações.
    pub panel_glass: bool,
    /// Menus (da barra, da dock, dos apps e de contexto) aparecem com um esmaecer curto.
    pub menu_animation: bool,
    /// Transições suaves: prévias da dock e alternador entram esmaecendo, pastas do
    /// launcher e do Arquivos com transição.
    pub transitions: bool,
    /// Agitar o ponteiro faz o cursor crescer por um instante (para achar onde está).
    pub shake_cursor: bool,
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
            panel_glass: true,
            menu_animation: true,
            transitions: true,
            shake_cursor: true,
        }
    }
}

/// Mouse e trackpad, na seção `[input]` (Configurações → Mouse e trackpad).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Input {
    /// Velocidade do ponteiro, de -1 (devagar) a 1 (rápido); 0 é o padrão do libinput.
    pub pointer_speed: f64,
    /// Aceleração do mouse (quanto mais rápido o movimento, mais o ponteiro anda).
    pub mouse_acceleration: bool,
    /// Rolagem natural no mouse e no trackpad (o conteúdo segue o dedo, como no Mac).
    pub mouse_natural_scroll: bool,
    pub touchpad_natural_scroll: bool,
    /// Toque no trackpad clica.
    pub tap_to_click: bool,
}

impl Default for Input {
    fn default() -> Self {
        Self { pointer_speed: 0.0, mouse_acceleration: true, mouse_natural_scroll: false, touchpad_natural_scroll: true, tap_to_click: true }
    }
}

/// Bloqueio de tela, na seção `[lock]`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Lock {
    /// Bloquear depois de tantos minutos sem uso (0: nunca). Só na sessão de verdade
    /// (DRM); num mide em janela, o desktop de baixo é que bloqueia.
    pub idle_minutes: u32,
    /// Bloquear antes de suspender.
    pub on_suspend: bool,
    /// Apagar a tela depois de tantos minutos sem uso (0: nunca). Com a tela já
    /// bloqueada, ela apaga depois de 1 minuto.
    pub blank_minutes: u32,
}

impl Default for Lock {
    fn default() -> Self {
        Self { idle_minutes: 5, on_suspend: true, blank_minutes: 10 }
    }
}

/// Energia, na seção `[power]` (Configurações → Energia).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Power {
    /// Suspender depois de tantos minutos sem uso (0: nunca). Só na sessão de verdade,
    /// e não enquanto um app pede para a tela ficar acesa (vídeo).
    pub suspend_minutes: u32,
    /// Botão de ligar: `"ask"` (pergunta antes de desligar), `"suspend"`, `"poweroff"`
    /// ou `"nothing"`.
    pub button: String,
}

impl Default for Power {
    fn default() -> Self {
        Self { suspend_minutes: 0, button: "ask".into() }
    }
}

/// Aparência, na seção `[appearance]`: `theme = "dark"` deixa o mide escuro (barras de
/// título, apps do mide-ui, apps GTK e Qt da sessão).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct Appearance {
    theme: String,
}

/// Teclado, na seção `[keyboard]` do arquivo.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Keyboard {
    /// Teclas no estilo Mac: Command (a tecla Super/Windows) faz o papel do Ctrl e o
    /// Ctrl o do Super, então os atalhos ficam no mesmo lugar do Mac (⌘C, ⌘V, ⌘Espaço).
    pub mac_style: bool,
    /// Layout escolhido nas Configurações (`"us"`, `"br"`...); vazio usa o do sistema
    /// (`/etc/default/keyboard`). `variant` é a variante dele (`"intl"`).
    pub layout: String,
    pub variant: String,
    /// Vários layouts, em ordem (`"br,us:intl"`): o mapa tem um grupo por layout e o
    /// atalho `next_layout` troca entre eles. Vale no lugar de `layout`/`variant`.
    pub layouts: String,
}

impl Keyboard {
    /// Teclado em uso: o do sistema, com o(s) layout(s) das Configurações por cima.
    pub fn effective(&self) -> SystemKeyboard {
        let mut kb = system_keyboard();
        let list = mide::keyboard_layouts::parse(&self.layouts);
        if !list.is_empty() {
            kb.layout = list.iter().map(|c| c.layout.as_str()).collect::<Vec<_>>().join(",");
            kb.variant = list.iter().map(|c| c.variant.as_str()).collect::<Vec<_>>().join(",");
        } else if !self.layout.trim().is_empty() {
            kb.layout = self.layout.trim().to_string();
            kb.variant = self.variant.trim().to_string();
        }
        kb
    }
}

/// Telas, na seção `[monitors]`: a principal e as opções de cada uma pelo nome do
/// conector (`DP-1`, `HDMI-A-1`, `eDP-1`...).
///
/// ```toml
/// [monitors]
/// primary = "DP-1"
/// [monitors.outputs."DP-1"]
/// mode = "1920x1080@60"
/// scale = 1.25
/// x = 0
/// y = 0
/// [monitors.outputs."HDMI-A-1"]
/// enabled = false
/// ```
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Monitors {
    /// Tela da barra, da dock e do launcher. Sem ela, a primeira.
    pub primary: Option<String>,
    /// Espelhar: todas as telas mostram a principal (na mesma posição, com a escala
    /// que deixa a largura igual à dela, e com a barra e a dock dela).
    pub mirror: bool,
    pub outputs: BTreeMap<String, Monitor>,
}

/// Opções de uma tela.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Monitor {
    /// Desligada: o mide não usa a tela.
    pub enabled: bool,
    /// Resolução e frequência (`1920x1080@60`); sem ela, o modo preferido da tela.
    pub mode: Option<String>,
    /// Escala (1.0 = 100%, 1.25, 1.5, 2.0...).
    pub scale: f64,
    /// Posição no arranjo das telas (canto superior esquerdo). Sem ela, lado a lado.
    pub x: Option<i32>,
    pub y: Option<i32>,
}

impl Default for Monitor {
    fn default() -> Self {
        Self { enabled: true, mode: None, scale: 1.0, x: None, y: None }
    }
}

impl Monitor {
    /// Resolução e frequência de `mode` (`1920x1080@60`, ou `1920x1080` sem frequência).
    pub fn parsed_mode(&self) -> Option<(u32, u32, Option<f64>)> {
        parse_mode(self.mode.as_deref()?)
    }
}

/// `LARGURAxALTURA[@HZ]` → (largura, altura, frequência).
pub fn parse_mode(text: &str) -> Option<(u32, u32, Option<f64>)> {
    let (size, hz) = match text.trim().split_once('@') {
        Some((size, hz)) => (size, Some(hz.trim().parse::<f64>().ok()?)),
        None => (text.trim(), None),
    };
    let (w, h) = size.split_once('x')?;
    Some((w.trim().parse().ok()?, h.trim().parse().ok()?, hz))
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

/// Transparência das janelas inteiras (`[opacity]`): conteúdo, barra de título e
/// contorno juntos. `inactive` vale para as janelas sem foco; os outros nomes são
/// `app_id` com a opacidade do app (multiplicam a de sem foco).
///
/// ```toml
/// [opacity]
/// inactive = 0.9
/// "mide-terminal" = 0.95
/// ```
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(default)]
pub struct Opacity {
    pub inactive: f32,
    #[serde(flatten)]
    pub apps: BTreeMap<String, f32>,
}

impl Default for Opacity {
    fn default() -> Self {
        Self { inactive: 1.0, apps: BTreeMap::new() }
    }
}

impl Opacity {
    /// Opacidade de uma janela (entre 0,2 e 1: nunca some de vez).
    pub fn for_window(&self, app_id: &str, focused: bool) -> f32 {
        let app = self.apps.iter().find(|(k, _)| k.eq_ignore_ascii_case(app_id)).map(|(_, v)| *v).unwrap_or(1.0);
        let base = if focused { 1.0 } else { self.inactive };
        (app * base).clamp(0.2, 1.0)
    }
}

/// Atalhos padrão. Um atalho do arquivo com a mesma combinação substitui o padrão.
const DEFAULT_KEYBINDINGS: &[(&str, &str)] = &[
    // Super é capturada pelo desktop hospedeiro durante o desenvolvimento.
    ("ctrl+shift+q", "close_window"),
    ("ctrl+shift+escape", "quit"),
    ("ctrl+shift+m", "restore_minimized"),
    // Terminal do mide.
    ("ctrl+shift+t", "launch:mide-terminal"),
    ("ctrl+shift+space", "toggle_launcher"),
    // Buscador do mide (estilo Spotlight).
    ("ctrl+space", "toggle_search"),
    // Próximo layout de teclado (como o Ctrl+⌥+Espaço do Mac; no estilo Mac o ⌥ é a
    // tecla Super). Não colide com o buscador nem com o launcher.
    ("ctrl+super+space", "next_layout"),
    // Captura de tela do mide (seleção, janela ou tela inteira, com marcações).
    ("print", "launch:mide-screenshot"),
    // Como no Mac: ⌘⇧3 salva a tela inteira e ⌘⇧4 uma área (a imagem também vai para
    // a área de transferência). No estilo Mac o ⌘ é o Alt; fora dele, a tecla Super.
    ("super+shift+3", "launch:mide-screenshot --full"),
    ("super+shift+4", "launch:mide-screenshot --area"),
    // ⌘⇧5: gravar a tela (inteira ou uma área, com som); de novo, para a gravação.
    ("super+shift+5", "launch:mide-screenshot --record"),
    // Terminal de atalho: desce do topo e some com a mesma tecla (como o Guake).
    ("f12", "launch:mide-terminal --hotkey"),
    // Janelas, como no GNOME.
    ("alt+tab", "switch_windows"),
    ("alt+shift+tab", "switch_windows_back"),
    ("super+tab", "switch_windows"),
    ("super+shift+tab", "switch_windows_back"),
    ("super+up", "maximize"),
    ("super+down", "unmaximize"),
    ("alt+f10", "toggle_maximize"),
    ("super+h", "minimize"),
    ("super+left", "tile_left"),
    ("super+right", "tile_right"),
    ("alt+f4", "close_window"),
    ("super+d", "show_desktop"),
    ("super+a", "toggle_launcher"),
    ("super+l", "lock_screen"),
    // Bloquear como no Mac (Ctrl+⌘+Q; no estilo Mac, o ⌘ é o Alt).
    ("ctrl+super+q", "lock_screen"),
];

/// Configuração carregada, com os atalhos já convertidos.
#[derive(Debug, Clone)]
pub struct Config {
    pub opacity: Opacity,
    pub bindings: Vec<Binding>,
    pub effects: Effects,
    pub desktop: Desktop,
    pub keyboard: Keyboard,
    pub monitors: Monitors,
    pub input: Input,
    pub lock: Lock,
    /// Tema escuro (`[appearance] theme = "dark"`).
    pub dark: bool,
    pub power: Power,
}

impl Config {
    /// Carrega o arquivo do usuário, se existir, sobre os atalhos padrão.
    pub fn load() -> Result<Self, Box<dyn Error>> {
        let mut dark = false;
        let mut power = Power::default();
        let (user, effects, desktop, keyboard, monitors, input, lock, opacity) = match config_path() {
            Some(path) if path.exists() => {
                let text = fs::read_to_string(&path)?;
                let mut file = toml::from_str::<FileConfig>(&text)
                    .map_err(|e| format!("{}: {e}", path.display()))?;
                let effects = file.effects;
                let desktop = std::mem::take(&mut file.desktop);
                let keyboard = std::mem::take(&mut file.keyboard);
                let monitors = std::mem::take(&mut file.monitors);
                let input = std::mem::take(&mut file.input);
                let lock = std::mem::take(&mut file.lock);
                let opacity = std::mem::take(&mut file.opacity);
                dark = file.appearance.theme == "dark";
                power = std::mem::take(&mut file.power);
                (file.all_bindings(), effects, desktop, keyboard, monitors, input, lock, opacity)
            }
            _ => (BTreeMap::new(), Effects::default(), Desktop::default(), Keyboard::default(), Monitors::default(), Input::default(), Lock::default(), Opacity::default()),
        };
        let mut config = Self::from_entries(&user)?;
        config.effects = effects;
        config.desktop = desktop;
        config.keyboard = keyboard;
        config.monitors = monitors;
        config.input = input;
        config.lock = lock;
        config.opacity = opacity;
        config.dark = dark;
        config.power = power;
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
        // `none` desliga a combinação (um atalho padrão removido nas Configurações).
        merged.retain(|_, action| action.trim() != "none");

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
            keyboard: Keyboard::default(),
            monitors: Monitors::default(),
            input: Input::default(),
            lock: Lock::default(),
            opacity: Opacity::default(),
            dark: false,
            power: Power::default(),
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

/// Combinação no formato do arquivo (`super+ctrl+alt+shift+tecla`), a partir dos
/// modificadores e da tecla sem o efeito do Shift (a mesma usada para casar atalhos).
pub fn combo_name(mods: Modifiers, key: Keysym) -> String {
    let mut parts: Vec<String> = Vec::new();
    for (on, name) in [(mods.logo, "super"), (mods.ctrl, "ctrl"), (mods.alt, "alt"), (mods.shift, "shift")] {
        if on {
            parts.push(name.into());
        }
    }
    parts.push(xkb::keysym_get_name(key).to_lowercase());
    parts.join("+")
}

/// Teclado do sistema (modelo, layout, variante e opções do xkb).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SystemKeyboard {
    pub model: String,
    pub layout: String,
    pub variant: String,
    pub options: Option<String>,
}

/// Teclado configurado no sistema (`/etc/default/keyboard`, padrão no Debian e no
/// Ubuntu), para o mide digitar com o mesmo layout (ABNT2, por exemplo) também fora
/// de um desktop anfitrião. Com `XKB_DEFAULT_LAYOUT` definido, vale o ambiente:
/// campos vazios fazem o xkb usar as variáveis `XKB_DEFAULT_*`.
pub fn system_keyboard() -> SystemKeyboard {
    if std::env::var_os("XKB_DEFAULT_LAYOUT").is_some() {
        return SystemKeyboard::default();
    }
    fs::read_to_string("/etc/default/keyboard").map(|t| parse_system_keyboard(&t)).unwrap_or_default()
}

/// Lê as linhas `XKBLAYOUT="br"` (e modelo, variante, opções) do arquivo de teclado.
fn parse_system_keyboard(text: &str) -> SystemKeyboard {
    let mut kb = SystemKeyboard::default();
    for line in text.lines() {
        let Some((key, value)) = line.trim().split_once('=') else {
            continue;
        };
        let value = value.trim().trim_matches('"').trim_matches('\'').to_string();
        match key.trim() {
            "XKBMODEL" => kb.model = value,
            "XKBLAYOUT" => kb.layout = value,
            "XKBVARIANT" => kb.variant = value,
            "XKBOPTIONS" if !value.is_empty() => kb.options = Some(value),
            _ => {}
        }
    }
    kb
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
    if action == "toggle_search" {
        return Ok(Action::ToggleSearch);
    }
    if action == "next_layout" {
        return Ok(Action::NextLayout);
    }
    let simple = [
        ("maximize", Action::Maximize),
        ("unmaximize", Action::Unmaximize),
        ("toggle_maximize", Action::ToggleMaximize),
        ("minimize", Action::Minimize),
        ("tile_left", Action::TileLeft),
        ("tile_right", Action::TileRight),
        ("switch_windows", Action::SwitchWindows),
        ("switch_windows_back", Action::SwitchWindowsBack),
        ("show_desktop", Action::ShowDesktop),
        ("lock_screen", Action::LockScreen),
    ];
    if let Some((_, a)) = simple.into_iter().find(|(name, _)| *name == action) {
        return Ok(a);
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
            Action::ToggleSearch => write!(f, "toggle_search"),
            Action::NextLayout => write!(f, "next_layout"),
            Action::Maximize => write!(f, "maximize"),
            Action::Unmaximize => write!(f, "unmaximize"),
            Action::ToggleMaximize => write!(f, "toggle_maximize"),
            Action::Minimize => write!(f, "minimize"),
            Action::TileLeft => write!(f, "tile_left"),
            Action::TileRight => write!(f, "tile_right"),
            Action::SwitchWindows => write!(f, "switch_windows"),
            Action::SwitchWindowsBack => write!(f, "switch_windows_back"),
            Action::ShowDesktop => write!(f, "show_desktop"),
            Action::LockScreen => write!(f, "lock_screen"),
            Action::SwitchVt(n) => write!(f, "switch_vt:{n}"),
            Action::PowerButton => write!(f, "power_button"),
            Action::Media(m) => write!(f, "media:{m:?}"),
            Action::TerminalKey(c) => write!(f, "terminal_key:{c}"),
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
    fn teclado_do_sistema() {
        let text = "# KEYBOARD\nXKBMODEL=\"pc105\"\nXKBLAYOUT=\"br\"\nXKBVARIANT=\"\"\nXKBOPTIONS=\"\"\nBACKSPACE=\"guess\"\n";
        let kb = parse_system_keyboard(text);
        assert_eq!(kb.model, "pc105");
        assert_eq!(kb.layout, "br");
        assert_eq!(kb.variant, "");
        assert_eq!(kb.options, None);
        assert_eq!(parse_system_keyboard("XKBOPTIONS=\"ctrl:nocaps\"").options.as_deref(), Some("ctrl:nocaps"));
    }

    #[test]
    fn none_desliga_um_atalho_padrao() {
        let c = cfg(&[("ctrl+shift+q", "none"), ("ctrl+alt+q", "close_window")]).unwrap();
        let ctrl_shift = Modifiers { ctrl: true, shift: true, ..Default::default() };
        let ctrl_alt = Modifiers { ctrl: true, alt: true, ..Default::default() };
        assert_eq!(c.match_binding(ctrl_shift, sym("q")), None);
        assert_eq!(c.match_binding(ctrl_alt, sym("q")), Some(Action::CloseWindow));
    }

    #[test]
    fn nome_da_combinacao_volta_a_ser_lido() {
        let mods = Modifiers { logo: true, shift: true, ..Default::default() };
        let name = combo_name(mods, sym("t"));
        assert_eq!(name, "super+shift+t");
        let c = cfg(&[(name.as_str(), "close_window")]).unwrap();
        assert_eq!(c.match_binding(mods, sym("t")), Some(Action::CloseWindow));
        assert_eq!(combo_name(Modifiers { ctrl: true, ..Default::default() }, sym("space")), "ctrl+space");
    }

    #[test]
    fn atalhos_principais_de_terminal_e_buscador() {
        let c = cfg(&[]).unwrap();
        let ctrl_shift = Modifiers { ctrl: true, shift: true, ..Default::default() };
        assert_eq!(
            c.match_binding(ctrl_shift, sym("t")),
            Some(Action::Launch("mide-terminal".into()))
        );
        assert_eq!(
            c.match_binding(ctrl_shift, sym("space")),
            Some(Action::ToggleLauncher)
        );
        assert_eq!(parse_action("toggle_launcher").unwrap(), Action::ToggleLauncher);
        assert_eq!(parse_action("toggle_search").unwrap(), Action::ToggleSearch);
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

    #[test]
    fn telas_no_config() {
        let text = "[monitors]\nprimary = \"DP-1\"\n[monitors.outputs.\"DP-1\"]\nmode = \"2560x1440@144\"\nscale = 1.25\nx = 1920\n[monitors.outputs.\"HDMI-A-1\"]\nenabled = false\n";
        let file: FileConfig = toml::from_str(text).unwrap();
        assert_eq!(file.monitors.primary.as_deref(), Some("DP-1"));
        let dp = &file.monitors.outputs["DP-1"];
        assert_eq!(dp.parsed_mode(), Some((2560, 1440, Some(144.0))));
        assert_eq!((dp.scale, dp.x, dp.y, dp.enabled), (1.25, Some(1920), None, true));
        assert!(!file.monitors.outputs["HDMI-A-1"].enabled);
        assert_eq!(parse_mode("1366x768"), Some((1366, 768, None)));
        assert_eq!(parse_mode("x"), None);
        // A seção das telas não vira atalho.
        assert!(file.all_bindings().is_empty());
    }

    #[test]
    fn mouse_e_trackpad_no_config() {
        let file: FileConfig = toml::from_str("[input]\npointer_speed = -0.4\ntap_to_click = false\n").unwrap();
        assert_eq!(file.input.pointer_speed, -0.4);
        assert!(!file.input.tap_to_click);
        // O que não está no arquivo fica no padrão (rolagem natural no trackpad, como no Mac).
        assert!(file.input.touchpad_natural_scroll && file.input.mouse_acceleration && !file.input.mouse_natural_scroll);
        assert!(file.all_bindings().is_empty());
    }

    #[test]
    fn opacidade_das_janelas() {
        let file: FileConfig = toml::from_str("[opacity]\ninactive = 0.5\n\"mide-terminal\" = 0.8\n").unwrap();
        let o = file.opacity;
        assert_eq!(o.inactive, 0.5);
        assert_eq!(o.for_window("mide-terminal", true), 0.8);
        assert_eq!(o.for_window("mide-terminal", false), 0.4);
        assert_eq!(o.for_window("firefox", true), 1.0);
        assert_eq!(o.for_window("firefox", false), 0.5);
    }
}
