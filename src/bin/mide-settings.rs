//! Configurações do Sistema do mide, feitas com o mide-ui: barra lateral com busca e
//! as seções; à direita, cartões com as opções.
//!
//! Seções: Wi-Fi, Bluetooth, Som, Papel de parede, Dock, Efeitos, Bloqueio de tela,
//! Usuários e grupos, Teclado, Sobre e outras.
//! O que é gravado vai para o `config.toml` (mantendo os comentários) e vale na hora:
//! o compositor relê a configuração (`reload`), e a dock e a barra percebem a mudança.
//!
//! Uso: `mide-settings [wifi|bluetooth|sound|wallpaper|dock|effects|lock|users|keyboard|about|...]`.

use std::{
    collections::HashMap,
    error::Error,
    fs,
    path::PathBuf,
    sync::mpsc::{channel, Receiver, Sender},
    time::Duration,
};

use mide::{
    control_client::ask_compositor,
    displays::{self, Display},
    kit::{run, App, ArrangedScreen, Key, Tile, Ui, Window},
    settings_file::{self, Value},
    system::{self, Status},
    tr,
    i18n::{trc, trn},
};
use tiny_skia::Pixmap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Section {
    Wifi,
    Network,
    Bluetooth,
    Appearance,
    Sound,
    Displays,
    Wallpaper,
    Dock,
    Bar,
    Effects,
    Lock,
    Users,
    Mouse,
    Keyboard,
    Region,
    Energy,
    DefaultApps,
    LoginItems,
    About,
}

/// Itens da barra lateral, na ordem: seção (ou espaço entre grupos), nome, ícone do
/// tema e cor do quadradinho.
const SIDEBAR: &[(Option<Section>, &str, &str, [u8; 3])] = &[
    (Some(Section::Wifi), "Wi-Fi", "network-wireless-symbolic", [10, 132, 255]),
    (Some(Section::Network), "Network", "network-wired-symbolic", [10, 132, 255]),
    (Some(Section::Bluetooth), "Bluetooth", "bluetooth-symbolic", [10, 132, 255]),
    (None, "", "", [0, 0, 0]),
    (Some(Section::Appearance), "Appearance", "preferences-desktop-appearance-symbolic", [72, 72, 80]),
    (Some(Section::Sound), "Sound", "audio-volume-high-symbolic", [255, 55, 95]),
    (Some(Section::Displays), "Displays", "video-display-symbolic", [52, 120, 246]),
    (Some(Section::Wallpaper), "Wallpaper", "preferences-desktop-wallpaper-symbolic", [100, 210, 255]),
    (Some(Section::Dock), "Dock", "view-app-grid-symbolic", [52, 199, 89]),
    (Some(Section::Bar), "Menu Bar", "window-new-symbolic", [88, 86, 214]),
    (Some(Section::Effects), "Effects", "starred-symbolic", [191, 90, 242]),
    (Some(Section::Lock), "Lock Screen", "system-lock-screen-symbolic", [255, 59, 48]),
    (Some(Section::Users), "Users & Groups", "avatar-default-symbolic", [10, 132, 255]),
    (None, "", "", [0, 0, 0]),
    (Some(Section::Mouse), "Mouse & Trackpad", "input-mouse-symbolic", [142, 142, 147]),
    (Some(Section::Keyboard), "Keyboard", "input-keyboard-symbolic", [142, 142, 147]),
    (Some(Section::Region), "Language & Region", "preferences-desktop-locale-symbolic", [10, 132, 255]),
    (Some(Section::Energy), "Energy", "battery-full-symbolic", [52, 199, 89]),
    (Some(Section::DefaultApps), "Default Apps", "applications-system-symbolic", [142, 142, 147]),
    (Some(Section::LoginItems), "Login Items", "system-run-symbolic", [142, 142, 147]),
    (Some(Section::About), "About", "help-about-symbolic", [142, 142, 147]),
];

/// Efeitos do `[effects]`: chave, rótulo e explicação (em inglês; traduzidos na tela).
const EFFECTS: &[(&str, &str, &str)] = &[
    ("shadows", "Window shadows", "A soft shadow around each window"),
    ("rounded_corners", "Rounded corners", "Windows with rounded corners and a thin outline"),
    ("animations", "Animations", "Smooth transitions when maximizing, restoring and closing windows and opening the launcher"),
    ("lamp", "Genie effect", "Minimizing pours the window into its Dock icon"),
    ("dock_blur", "Dock blur", "The Dock blurs what is behind it, like frosted glass"),
    ("launcher_blur", "Launcher blur", "The launcher background is the blurred wallpaper"),
    ("liquid_glass", "Liquid glass", "Dock and launcher folders with edges that refract light and a reflection, over the blur"),
    ("panel_glass", "Glass in bars and menus", "Menu bar, menus, notifications and window previews with glass"),
    ("menu_animation", "Menu animation", "Menus and lists appear with a short fade"),
    ("transitions", "Smooth transitions", "Dock previews and Alt+Tab, launcher pages and folders, Files folders, switching between icons and list, and the screenshot thumbnail"),
    ("shake_cursor", "Shake mouse pointer to locate", "Moving the mouse quickly back and forth makes the pointer big for a moment"),
    ("game_mode", "Game mode in full screen", "A full screen window gets the display to itself: the menu bar, Dock and glass pause, and notifications wait in the notification history"),
];

/// O que a busca encontra: nome da opção, palavras-chave e a seção dela (em inglês;
/// a busca olha o texto em inglês e o traduzido).
const SEARCHABLE: &[(&str, &str, Section)] = &[
    ("Lock the screen after", "lock password inactivity idle time screen saver", Section::Lock),
    ("Lock when suspending", "lock suspend sleep password", Section::Lock),
    ("Turn off the display after", "turn off screen display monitor power dpms saver", Section::Lock),
    ("Users & Groups", "users accounts user account admin administrator picture photo name", Section::Users),
    ("Add a user", "user account new create add password", Section::Users),
    ("Delete a user", "user account delete remove", Section::Users),
    ("Login window", "login window list users name password sleep restart shut down buttons hints input menu", Section::Users),
    ("Automatically log in", "login automatic autologin without password", Section::Users),
    ("Message on the login window", "login lock message text", Section::Users),
    ("Accent color", "appearance color accent blue purple pink red orange yellow green graphite", Section::Appearance),
    ("Appearance", "theme light dark mode night colors", Section::Appearance),
    ("Wi-Fi", "network internet wireless connect password", Section::Wifi),
    ("Network", "network cable wired ethernet internet address ip connection", Section::Network),
    ("Bluetooth", "headphones mouse keyboard devices pair wireless", Section::Bluetooth),
    ("Output volume", "sound audio speaker volume", Section::Sound),
    ("Mute sound", "mute silence sound", Section::Sound),
    ("Sound effects", "sound effects alert beep notification system sounds", Section::Sound),
    ("Displays", "screen screens monitor resolution refresh hz scale arrange main second display", Section::Displays),
    ("Display resolution and refresh rate", "monitor resolution frequency hz refresh rate", Section::Displays),
    ("Display scale", "monitor scale size zoom hidpi", Section::Displays),
    ("Wallpaper", "background image wallpaper desktop", Section::Wallpaper),
    ("Add a background image", "wallpaper image photo", Section::Wallpaper),
    ("Dock icon size", "dock icons size", Section::Dock),
    ("Automatically hide and show the Dock", "dock hide autohide automatic", Section::Dock),
    ("Clicking the focused app minimizes it", "dock minimize click focus previews windows", Section::Dock),
    ("Menu bar items", "bar menu top icons sound wifi bluetooth battery control center gear", Section::Bar),
    ("Window shadows", "effects shadow", Section::Effects),
    ("Rounded corners", "effects corners edges", Section::Effects),
    ("Animations", "effects animation transition", Section::Effects),
    ("Genie effect", "effects minimize genie lamp", Section::Effects),
    ("Dock blur", "effects blur glass", Section::Effects),
    ("Launcher blur", "effects blur launcher", Section::Effects),
    ("Liquid glass", "effects glass liquid transparency", Section::Effects),
    ("Glass in bars and menus", "effects glass bar menus notifications transparency", Section::Effects),
    ("Menu animation", "effects animation menus context fade", Section::Effects),
    ("Smooth transitions", "effects animation transition smooth launcher folders files list icons", Section::Effects),
    ("Game mode in full screen", "effects game games performance full screen fullscreen video notifications", Section::Effects),
    ("Pointer speed", "mouse trackpad touchpad pointer speed cursor tracking", Section::Mouse),
    ("Natural scrolling", "mouse trackpad touchpad scrolling natural scroll inverted", Section::Mouse),
    ("Tap to click", "trackpad touchpad tap click", Section::Mouse),
    ("Mouse acceleration", "mouse acceleration pointer precision", Section::Mouse),
    ("Keyboard layout", "keyboard layout abnt2 abnt language us international accents input source", Section::Keyboard),
    ("Mac-style keys", "keyboard command option cmd super ctrl shortcuts", Section::Keyboard),
    ("Keyboard shortcuts", "keyboard shortcuts keys combinations", Section::Keyboard),
    ("Change or create a shortcut", "keyboard shortcut new record combination remove default", Section::Keyboard),
    ("Language", "language translation portuguese english locale", Section::Region),
    ("Date and number formats", "region format date time numbers currency locale", Section::Region),
    ("Put the computer to sleep after", "energy suspend sleep inactivity battery", Section::Energy),
    ("Power button", "energy button power shut down suspend", Section::Energy),
    ("Default apps", "apps default browser email open with pdf images videos music folders text", Section::DefaultApps),
    ("Clicking the wallpaper reveals the desktop", "desktop wallpaper click move windows aside show desktop", Section::Wallpaper),
    ("Login items", "login session startup autostart open at login apps commands scripts", Section::LoginItems),
    ("About this computer", "system version memory processor", Section::About),
];

/// Grupos da lista de atalhos, na ordem em que aparecem: a chave (gravada em
/// `[settings] shortcuts_closed`, por isso não muda) e o nome na tela.
const SHORTCUT_GROUPS: [(&str, &str); 4] = [("Janelas", "Windows"), ("Launcher e busca", "Launcher & Search"), ("Apps", "Apps"), ("Sistema", "System")];

const DOCK_MIN: f32 = 32.0;
const DOCK_MAX: f32 = 64.0;
const DOCK_DEFAULT: i64 = 40;
/// Miniaturas dos papéis de parede e do seletor de imagens.
const THUMB: (u32, u32) = (120, 75);

/// Miniatura de um papel de parede (`None` = papel de parede padrão do mide).
type Thumb = (Option<PathBuf>, Option<Pixmap>);

/// Seletor de imagens ("Adicionar imagem…"): pasta, itens e miniaturas.
struct Picker {
    dir: PathBuf,
    entries: Vec<(PathBuf, bool)>,
    thumbs: HashMap<PathBuf, Option<Pixmap>>,
    rx: Receiver<(PathBuf, Option<Pixmap>)>,
}

struct Settings {
    section: Section,
    query: String,
    status: Status,
    status_rx: Receiver<Status>,
    wallpapers: Vec<Option<PathBuf>>,
    thumbs: HashMap<Option<PathBuf>, Option<Pixmap>>,
    thumbs_tx: Sender<Thumb>,
    thumbs_rx: Receiver<Thumb>,
    picker: Option<Picker>,
    bindings: Vec<(String, String)>,
    /// Apps instalados (para criar atalhos que abrem um app e mostrar o nome deles).
    apps: Vec<mide::apps::App>,
    /// Diálogo de mudar ou criar um atalho.
    shortcut: Option<ShortcutEditor>,
    info: Vec<(&'static str, String)>,
    volume: f32,
    dock_size: f32,
    /// A busca acabou de ser limpa (um resultado foi escolhido).
    query_reset: bool,
    /// Telas (Monitores): como estão na tela agora e como o usuário está deixando.
    screens_applied: Vec<Display>,
    screens: Vec<Display>,
    screen_selected: usize,
    /// Mudanças aplicadas esperando confirmação: quando foram aplicadas e as anteriores.
    screens_revert: Option<(std::time::Instant, Vec<Display>)>,
    /// Busca na lista de atalhos.
    shortcut_query: String,
    /// Idiomas instalados (`locale -a`), lidos ao abrir Idioma e região.
    locales: Vec<String>,
    /// Grupos de atalhos recolhidos (Configurações → Teclado).
    closed_groups: Vec<String>,
    /// Volume da entrada padrão (microfone), de 0 a 100.
    mic_volume: f32,
    /// Itens de início (relidos ao abrir a seção e depois de cada mudança).
    login_items: Option<Vec<mide::login_items::LoginItem>>,
    /// Diálogo de adicionar um item de início.
    login_dialog: Option<LoginDialog>,
    /// Usuários e grupos.
    users: Users,
}

/// Diálogo de adicionar um item de início: um app instalado ou um comando.
#[derive(Default)]
struct LoginDialog {
    command_mode: bool,
    query: String,
    name: String,
    command: String,
    error: Option<String>,
    /// Escolhendo o programa ou script pelo seletor de arquivos.
    chooser: Option<mide::kit::FileChooser>,
}

impl Settings {
    fn new(section: Section) -> Self {
        // Som, Wi-Fi e Bluetooth a cada 2 s, numa thread (os comandos levam tempo).
        let (status_tx, status_rx) = channel();
        std::thread::spawn(move || loop {
            if status_tx.send(system::read_status()).is_err() {
                break;
            }
            std::thread::sleep(Duration::from_secs(2));
        });
        let (thumbs_tx, thumbs_rx) = channel();
        let wallpapers = wallpaper_list();
        load_thumbs(thumbs_tx.clone(), wallpapers.clone());
        let mut app = Self {
            section,
            query: String::new(),
            status: Status::default(),
            status_rx,
            wallpapers,
            thumbs: HashMap::new(),
            thumbs_tx,
            thumbs_rx,
            picker: None,
            bindings: Vec::new(),
            apps: Vec::new(),
            shortcut: None,
            info: mide::sysinfo::system_info(),
            query_reset: false,
            screens_applied: Vec::new(),
            screens: Vec::new(),
            screen_selected: 0,
            screens_revert: None,
            shortcut_query: String::new(),
            locales: Vec::new(),
            closed_groups: match settings_file::get("settings", "shortcuts_closed") {
                Some(Value::Text(t)) => t.split(',').filter(|g| !g.is_empty()).map(str::to_string).collect(),
                _ => Vec::new(),
            },
            login_items: None,
            login_dialog: None,
            users: Users::new(),
            mic_volume: 0.0,
            volume: 0.0,
            dock_size: match settings_file::get("dock", "icon_size") {
                Some(Value::Int(n)) => (n as f32).clamp(DOCK_MIN, DOCK_MAX),
                _ => DOCK_DEFAULT as f32,
            },
        };
        app.load_bindings();
        app
    }

    fn load_bindings(&mut self) {
        if self.apps.is_empty() {
            self.apps = mide::apps::load();
        }
        self.bindings = ask_compositor("bindings")
            .unwrap_or_default()
            .iter()
            .filter_map(|l| l.split_once('\t').map(|(c, a)| (c.to_string(), a.to_string())))
            .collect();
    }

    /// Recebe o que as threads mandaram (estado do sistema, miniaturas).
    fn poll(&mut self, ui: &mut Ui) {
        while let Ok(status) = self.status_rx.try_recv() {
            if status != self.status {
                if let Some((v, _)) = status.volume {
                    self.volume = v * 100.0;
                }
                if let Some(mic) = status.inputs.iter().find(|d| d.default) {
                    self.mic_volume = mic.volume * 100.0;
                }
                self.status = status;
            }
        }
        let mut loading = false;
        while let Ok((path, thumb)) = self.thumbs_rx.try_recv() {
            self.thumbs.insert(path, thumb);
        }
        if self.thumbs.len() < self.wallpapers.len() {
            loading = true;
        }
        if let Some(picker) = self.picker.as_mut() {
            while let Ok((path, thumb)) = picker.rx.try_recv() {
                picker.thumbs.insert(path, thumb);
            }
            let images = picker.entries.iter().filter(|(_, d)| !d).count();
            loading |= picker.thumbs.len() < images.min(300);
        }
        if loading {
            ui.request_tick();
        }
        // O estado do sistema chega a cada 2 s.
        ui.wake_after(Duration::from_secs(2));
    }

    fn open_picker(&mut self, dir: PathBuf) {
        let entries = mide::places::folder_images(&dir);
        tracing::info!("seletor de imagens: {} ({} item(ns))", dir.display(), entries.len());
        let images: Vec<PathBuf> = entries.iter().filter(|(_, d)| !d).map(|(p, _)| p.clone()).take(300).collect();
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            for path in images {
                let thumb = image::open(&path).ok().map(|img| to_pixmap(&mide::backdrop::cover(&img.to_rgba8(), THUMB.0, THUMB.1)));
                if tx.send((path, thumb)).is_err() {
                    break;
                }
            }
        });
        self.picker = Some(Picker { dir, entries, thumbs: HashMap::new(), rx });
    }

    fn refresh_wallpapers(&mut self) {
        self.wallpapers = wallpaper_list();
        let missing: Vec<Option<PathBuf>> = self.wallpapers.iter().filter(|p| !self.thumbs.contains_key(*p)).cloned().collect();
        load_thumbs(self.thumbs_tx.clone(), missing);
    }

    fn set_wallpaper(&self, path: Option<PathBuf>) {
        let value = path.as_ref().map(|p| Value::Text(p.to_string_lossy().into_owned()));
        match settings_file::set("desktop", "wallpaper", value) {
            Ok(()) => {
                tracing::info!("papel de parede escolhido: {}", path.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "padrão do mide".into()));
                ask_compositor("reload");
            }
            Err(err) => tracing::warn!("papel de parede não gravado: {err}"),
        }
    }

    // ----- Seções -----

    fn wifi(&mut self, ui: &mut Ui) {
        ui.title("Wi-Fi");
        let (on, networks) = self.status.wifi.clone().unwrap_or((false, Vec::new()));
        let mut value = on;
        ui.card(|ui| {
            if ui.switch("Wi-Fi", &mut value).changed() {
                system::set_wifi(value);
            }
        });
        if value != on {
            if let Some(wifi) = self.status.wifi.as_mut() {
                wifi.0 = value;
            }
        }
        if !on {
            return;
        }
        ui.heading(&tr!("Networks"));
        ui.card(|ui| {
            if networks.is_empty() {
                ui.note(&tr!("Looking for networks…"));
            }
            for net in networks.iter().take(10) {
                let detail = if net.active {
                    tr!("Connected").into_owned()
                } else if net.secure {
                    tr!("{signal}% · secured", signal = net.signal)
                } else {
                    format!("{}%", net.signal)
                };
                if ui.list_item(&net.ssid, &detail).clicked() && !net.active {
                    system::connect_network(&net.ssid);
                }
            }
        });
    }

    /// Rede: as placas cabeadas (estado, conexão e endereço) e o resumo do Wi-Fi, com
    /// a conexão principal (por onde a internet sai) marcada, como em Ajustes → Rede.
    fn network(&mut self, ui: &mut Ui) {
        ui.title(&tr!("Network"));
        let wired = self.status.wired.clone();
        let primary = self.status.primary.clone();
        let wifi_net = self.status.active_network().map(|n| n.ssid.clone());
        let wifi_on = self.status.wifi.as_ref().is_some_and(|(on, _)| *on);
        let mut open_wifi = false;
        ui.card(|ui| {
            for w in &wired {
                let state = match w.state {
                    system::WiredState::Connected => match &w.address {
                        Some(ip) => tr!("Connected · {ip}", ip = ip),
                        None => tr!("Connected").into_owned(),
                    },
                    system::WiredState::Connecting => tr!("Connecting…").into_owned(),
                    system::WiredState::Disconnected => tr!("Cable connected, no connection").into_owned(),
                    system::WiredState::Unplugged => tr!("Cable unplugged").into_owned(),
                };
                let main = primary.as_deref() == Some(w.device.as_str()) && w.state == system::WiredState::Connected;
                let label = if wired.len() > 1 { format!("Ethernet ({})", w.device) } else { "Ethernet".to_string() };
                ui.list_item(&label, &if main { tr!("{state} · primary", state = state) } else { state });
            }
            let wifi_detail = match (&wifi_net, wifi_on) {
                (_, false) => tr!("Off").into_owned(),
                (None, true) => tr!("No network").into_owned(),
                (Some(ssid), true) => {
                    let main = primary.as_ref().is_some_and(|p| !wired.iter().any(|w| w.device == *p));
                    if main { tr!("{ssid} · primary", ssid = ssid) } else { ssid.clone() }
                }
            };
            if ui.list_item("Wi-Fi", &wifi_detail).clicked() {
                open_wifi = true;
            }
        });
        if wired.is_empty() {
            ui.note(&tr!("No wired network adapter found."));
        }
        if let Some(w) = wired.iter().find(|w| w.state == system::WiredState::Connected && !w.connection.is_empty()) {
            ui.heading(&tr!("Wired Connection"));
            ui.card(|ui| {
                ui.list_item(&tr!("Connection name"), &w.connection);
                ui.list_item(&tr!("Interface"), &w.device);
                if let Some(ip) = &w.address {
                    ui.list_item(&tr!("IP address"), ip);
                }
            });
        }
        if open_wifi {
            self.section = Section::Wifi;
        }
    }

    fn bluetooth(&mut self, ui: &mut Ui) {
        ui.title("Bluetooth");
        let (on, devices) = self.status.bluetooth.clone().unwrap_or((false, Vec::new()));
        let mut value = on;
        ui.card(|ui| {
            let detail = if on { tr!("Visible to paired devices") } else { tr!("Off") };
            if ui.switch_detail("Bluetooth", &detail, &mut value).changed() {
                system::set_bluetooth(value);
            }
        });
        if value != on {
            if let Some(bt) = self.status.bluetooth.as_mut() {
                bt.0 = value;
            }
        }
        if !on {
            return;
        }
        ui.heading(&tr!("My Devices"));
        ui.card(|ui| {
            if devices.is_empty() {
                ui.note(&tr!("No paired devices"));
            }
            for d in &devices {
                let detail = if d.connected { tr!("Connected · click to disconnect") } else { tr!("Not connected · click to connect") };
                if ui.list_item(&d.name, &detail).clicked() {
                    system::toggle_device(d);
                }
            }
        });
    }

    /// Monitores: arranjo das telas (arrastar), tela principal, ligar e desligar,
    /// resolução, frequência e escala. "Aplicar" grava e o compositor muda na hora;
    /// sem confirmar em 15 s, volta ao que era (se a tela ficar preta, basta esperar).
    fn displays(&mut self, ui: &mut Ui) {
        ui.title(&tr!("Displays"));
        if self.screens_applied.is_empty() {
            self.screens_applied = displays::load();
            self.screens = self.screens_applied.clone();
            self.screen_selected = self.screens.iter().position(|d| d.primary).unwrap_or(0);
        }
        if self.screens.is_empty() {
            ui.note(&tr!("mide did not report the displays (running in a window, the display is the window)."));
            return;
        }
        // Arranjo: só as telas ligadas.
        let on: Vec<usize> = (0..self.screens.len()).filter(|&i| self.screens[i].enabled).collect();
        let mut arranged: Vec<ArrangedScreen> = on
            .iter()
            .map(|&i| {
                let d = &self.screens[i];
                ArrangedScreen { name: d.name.clone(), pos: d.position, size: d.logical_size(), primary: d.primary, mm: d.physical_mm() }
            })
            .collect();
        let mut pick = on.iter().position(|&i| i == self.screen_selected).unwrap_or(0);
        let r = ui.arrangement(&mut arranged, &mut pick);
        if r.clicked || r.changed {
            if let Some(&i) = on.get(pick) {
                self.screen_selected = i;
            }
        }
        if r.changed {
            for (a, &i) in arranged.iter().zip(&on) {
                self.screens[i].position = a.pos;
            }
            tracing::info!("monitores: {} movida para ({}, {})", arranged[pick].name, arranged[pick].pos.0, arranged[pick].pos.1);
        }
        ui.note(&tr!("Drag the displays to arrange them. The white strip marks the main display, with the menu bar and the Dock."));
        // Aplicar logo abaixo do arranjo (à vista, sem rolar).
        let changed = self.screens != self.screens_applied;
        if changed && self.screens_revert.is_none() {
            ui.row(|ui| {
                if ui.button(&tr!("Undo")).clicked() {
                    self.screens = self.screens_applied.clone();
                }
                if ui.primary_button(&tr!("Apply")).clicked() {
                    match displays::save(&self.screens) {
                        Ok(()) => {
                            ask_compositor("outputs apply");
                            tracing::info!("monitores: aplicado");
                            self.screens_revert = Some((std::time::Instant::now(), self.screens_applied.clone()));
                            self.screens_applied = self.screens.clone();
                        }
                        Err(err) => tracing::warn!("monitores não gravados: {err}"),
                    }
                }
            });
        }

        let names: Vec<String> = self
            .screens
            .iter()
            .map(|d| if d.description.is_empty() { d.name.clone() } else { format!("{} ({})", d.name, d.description) })
            .collect();
        let others_on = self.screens.iter().enumerate().filter(|(i, d)| *i != self.screen_selected && d.enabled).count();
        ui.card(|ui| {
            if names.len() > 1 {
                ui.dropdown(&tr!("Display"), &names, &mut self.screen_selected);
            }
            let Some(d) = self.screens.get_mut(self.screen_selected) else {
                return;
            };
            let mut on = d.enabled;
            if ui.switch_detail(&trc("display", "On"), &tr!("When off, the display stays dark and mide does not use it"), &mut on).changed() {
                // A última tela ligada não desliga.
                d.enabled = on || others_on == 0;
            }
            if !d.enabled {
                return;
            }
            if d.primary {
                ui.list_item(&tr!("Main display"), &tr!("menu bar, Dock and launcher"));
            } else if ui.button(&tr!("Use as main display")).clicked() {
                let name = d.name.clone();
                for other in self.screens.iter_mut() {
                    other.primary = other.name == name;
                }
                return;
            }
            let Some(d) = self.screens.get_mut(self.screen_selected) else {
                return;
            };
            let sizes = d.sizes();
            let mut size_i = sizes.iter().position(|s| *s == (d.current.0, d.current.1)).unwrap_or(0);
            let labels: Vec<String> = sizes.iter().map(|s| displays::size_label(*s)).collect();
            if ui.dropdown(&tr!("Resolution"), &labels, &mut size_i).changed() {
                let size = sizes[size_i];
                let hz = d.rates(size).first().copied().unwrap_or(60.0);
                d.current = (size.0, size.1, hz);
            }
            let rates = d.rates((d.current.0, d.current.1));
            let mut rate_i = rates.iter().position(|r| (r - d.current.2).abs() < 0.5).unwrap_or(0);
            let labels: Vec<String> = rates.iter().map(|r| displays::rate_label(*r)).collect();
            if ui.dropdown(&tr!("Refresh rate"), &labels, &mut rate_i).changed() {
                d.current.2 = rates[rate_i];
            }
            // Tamanho físico (o arranjo desenha as telas do tamanho de verdade): o que o
            // monitor informa, ou a diagonal escolhida para quem não informa direito.
            let auto = match d.reported_mm() {
                Some((w, h)) => tr!("Automatic ({size}\")", size = format!("{:.0}", ((w * w + h * h).sqrt() / 25.4).round())),
                None => tr!("Automatic (estimated)").into_owned(),
            };
            let mut sizes_label = vec![auto];
            sizes_label.extend(displays::DIAGONALS.iter().map(|d| format!("{}\"", d.to_string().replace('.', ","))));
            let mut diag_i = d.diagonal.and_then(|v| displays::DIAGONALS.iter().position(|x| (x - v).abs() < 0.05)).map(|i| i + 1).unwrap_or(0);
            if ui.dropdown(&tr!("Screen size"), &sizes_label, &mut diag_i).changed() {
                d.diagonal = (diag_i > 0).then(|| displays::DIAGONALS[diag_i - 1]);
            }
            let mut scale_i = displays::SCALES.iter().position(|s| (s - d.scale).abs() < 0.01).unwrap_or(0);
            let labels: Vec<String> = displays::SCALES.iter().map(|s| displays::scale_label(*s)).collect();
            if ui.dropdown(&tr!("Scale"), &labels, &mut scale_i).changed() {
                d.scale = displays::SCALES[scale_i];
            }
        });
        // Espelhar: todas as telas mostram a principal (vale na hora).
        if self.screens.len() >= 2 {
            let mut mirror = matches!(settings_file::get("monitors", "mirror"), Some(Value::Bool(true)));
            ui.card(|ui| {
                if ui.switch_detail(&tr!("Mirror displays"), &tr!("All displays show the main one, with its menu bar and Dock"), &mut mirror).changed() {
                    match settings_file::set("monitors", "mirror", mirror.then_some(Value::Bool(true))) {
                        Ok(()) => {
                            tracing::info!("monitores: espelhar {}", if mirror { "ligado" } else { "desligado" });
                            ask_compositor("outputs apply");
                            // O arranjo novo vem do compositor no próximo quadro.
                            self.screens_applied.clear();
                        }
                        Err(err) => tracing::warn!("espelhar não gravado: {err}"),
                    }
                }
            });
        }
        // Trava de rotação: só em aparelhos com sensor de orientação (conversíveis).
        if orientation_sensor() {
            let mut locked = matches!(settings_file::get("display", "rotation_lock"), Some(Value::Bool(true)));
            ui.card(|ui| {
                if ui.switch_detail(&tr!("Rotation Lock"), &tr!("The built-in display does not turn with the device"), &mut locked).changed() {
                    match settings_file::set("display", "rotation_lock", locked.then_some(Value::Bool(true))) {
                        Ok(()) => {
                            tracing::info!("trava de rotação: {}", if locked { "ligada" } else { "desligada" });
                            ask_compositor("reload");
                        }
                        Err(err) => tracing::warn!("trava de rotação não gravada: {err}"),
                    }
                }
            });
        }
    }

    /// Depois de aplicar as telas: janelinha (modal) "Manter estas configurações?", por
    /// cima da janela inteira. Sem confirmar em 15 s, volta ao que era (se a tela
    /// ficar preta, basta esperar).
    fn displays_confirm(&mut self, ui: &mut Ui) {
        const KEEP_SECS: u64 = 15;
        let Some((at, previous)) = self.screens_revert.clone() else {
            return;
        };
        let left = KEEP_SECS.saturating_sub(at.elapsed().as_secs());
        let mut keep = false;
        let mut revert = left == 0;
        ui.modal(360.0, |ui| {
            ui.heading(&tr!("Keep these display settings?"));
            ui.note(&tr!("The displays go back to how they were in {seconds} s.", seconds = left));
            ui.row(|ui| {
                revert |= ui.button(&tr!("Revert")).clicked();
                keep = ui.primary_button(&tr!("Keep")).clicked();
            });
        });
        if keep {
            tracing::info!("monitores: mantido");
            self.screens_revert = None;
            self.screens_applied = displays::load();
            self.screens = self.screens_applied.clone();
        } else if revert {
            tracing::info!("monitores: revertido");
            if displays::save(&previous).is_ok() {
                ask_compositor("outputs apply");
            }
            self.screens_revert = None;
            self.screens_applied = Vec::new();
        } else {
            ui.wake_after(Duration::from_millis(500));
        }
    }

    fn sound(&mut self, ui: &mut Ui) {
        ui.title(&tr!("Sound"));
        ui.heading(&tr!("Output"));
        let (_, muted) = self.status.volume.unwrap_or((0.0, false));
        let mut mute = muted;
        let mut volume = self.volume;
        ui.card(|ui| {
            let r = ui.slider(&tr!("Output volume"), &mut volume, 0.0, 100.0);
            // Grava só ao soltar o trilho (cada mudança chama um comando do sistema).
            if r.submitted() {
                system::set_volume(volume / 100.0);
                if muted {
                    system::set_muted(false);
                }
            }
            if ui.switch(&tr!("Mute sound"), &mut mute).changed() {
                system::set_muted(mute);
            }
        });
        self.volume = volume;
        if mute != muted {
            if let Some(v) = self.status.volume.as_mut() {
                v.1 = mute;
            }
        }
        // Por onde o som sai (como "Saída" nos Ajustes do Mac): os apps passam a tocar
        // no aparelho escolhido.
        // Uma linha por saída de verdade (alto-falantes, fones, cada monitor), com o
        // conector ou a placa como detalhe; escolher uma troca o perfil da placa se
        // for preciso.
        let outputs = self.status.outputs.clone();
        if !outputs.is_empty() {
            let mut chosen = None;
            ui.card(|ui| {
                for (i, d) in outputs.iter().enumerate() {
                    if ui.choice_item(&d.name, &d.detail, d.default, true).clicked() && !d.default {
                        chosen = Some(i);
                    }
                }
            });
            if let Some(d) = chosen.and_then(|i| outputs.get(i)) {
                tracing::info!("som: saída {}", d.name);
                for o in &mut self.status.outputs {
                    o.default = o.key == d.key;
                }
                self.volume = d.volume * 100.0;
                system::select_audio(d, true);
            }
        }
        // Entrada: o microfone usado pelos apps, o volume dele e o mudo.
        let inputs = self.status.inputs.clone();
        if !inputs.is_empty() {
            ui.heading(&tr!("Input"));
            let mut chosen = None;
            let mut mic = self.mic_volume;
            let current = inputs.iter().find(|d| d.default).cloned();
            ui.card(|ui| {
                for (i, d) in inputs.iter().enumerate() {
                    if ui.choice_item(&d.name, &d.detail, d.default, true).clicked() && !d.default {
                        chosen = Some(i);
                    }
                }
            });
            ui.card(|ui| {
                if ui.slider(&tr!("Input volume"), &mut mic, 0.0, 100.0).submitted()
                    && let Some(d) = &current
                {
                    tracing::info!("som: volume de entrada {:.0}", mic);
                    system::set_device_volume(d, false, mic / 100.0);
                }
            });
            self.mic_volume = mic;
            if let Some(d) = chosen.and_then(|i| inputs.get(i)) {
                tracing::info!("som: entrada {}", d.name);
                for o in &mut self.status.inputs {
                    o.default = o.key == d.key;
                }
                self.mic_volume = d.volume * 100.0;
                system::select_audio(d, false);
            }
        }
        // Efeitos sonoros do mide (`[sounds]`): ligados, volume e o som de alerta.
        ui.heading(&tr!("Sound effects"));
        let mut on = mide::sounds::enabled();
        let mut effects = mide::sounds::volume() * 100.0;
        let names: Vec<String> = mide::sounds::ALERTS.iter().map(|(key, n)| alert_name(key, n)).collect();
        let current = mide::sounds::alert_key();
        let mut alert_i = mide::sounds::ALERTS.iter().position(|(k, _)| *k == current).unwrap_or(0);
        ui.card(|ui| {
            if ui.switch_detail(&tr!("Play sound effects"), &tr!("Notifications, screenshots, Trash, volume and alerts"), &mut on).changed() {
                match settings_file::set("sounds", "enabled", (!on).then_some(Value::Bool(false))) {
                    Ok(()) => {
                        tracing::info!("efeitos sonoros: {}", if on { "ligados" } else { "desligados" });
                        // Os apps GTK seguem (sons de evento no dconf da sessão).
                        mide::appearance::apply(false);
                    }
                    Err(err) => tracing::warn!("efeitos sonoros não gravados: {err}"),
                }
            }
            if ui.slider(&tr!("Effects volume"), &mut effects, 0.0, 100.0).submitted() {
                let value = effects.round() as i64;
                tracing::info!("efeitos sonoros: volume {value}");
                if settings_file::set("sounds", "volume", (value != 70).then_some(Value::Int(value))).is_ok() {
                    mide::sounds::play("audio-volume-change");
                }
            }
            if ui.dropdown(&tr!("Alert sound"), &names, &mut alert_i).changed() {
                let (key, name) = mide::sounds::ALERTS[alert_i];
                tracing::info!("efeitos sonoros: alerta {name}");
                if settings_file::set("sounds", "alert", (key != "vidro").then(|| Value::Text(key.into()))).is_ok() {
                    mide::sounds::preview_alert(key);
                }
            }
        });
        ui.note(&tr!("Do Not Disturb silences notification sounds. GTK apps opened afterwards also use these sounds."));
    }

    fn wallpaper(&mut self, ui: &mut Ui) {
        if self.picker.is_some() {
            self.picker_view(ui);
            return;
        }
        ui.title(&tr!("Wallpaper"));
        ui.card(|ui| {
            let mut on = settings_file::flag("desktop", "click_reveal", true);
            if ui.switch_detail(&tr!("Clicking the wallpaper reveals the desktop"), &tr!("Windows move aside to the edges; another click, Esc or a click on a window brings them back"), &mut on).changed() {
                // Ligado é o padrão: grava só quando desliga.
                match settings_file::set("desktop", "click_reveal", (!on).then_some(Value::Bool(false))) {
                    Ok(()) => {
                        tracing::info!("mesa: clique no papel de parede {}", if on { "ligado" } else { "desligado" });
                        ask_compositor("reload");
                    }
                    Err(err) => tracing::warn!("opção da mesa não gravada: {err}"),
                }
            }
        });
        let current = current_wallpaper();
        let mut chosen = None;
        let mut remove = None;
        let mut add = false;
        let wallpapers = self.wallpapers.clone();
        ui.wrap(|ui| {
            for path in &wallpapers {
                let name = match path {
                    None => tr!("mide default").into_owned(),
                    Some(p) => p.file_stem().map(|s| s.to_string_lossy().replace(['-', '_'], " ")).unwrap_or_default(),
                };
                let thumb = self.thumbs.get(path).cloned().flatten();
                let image = match &thumb {
                    Some(pix) => Tile::Image(pix),
                    None => Tile::Icon("image-x-generic"),
                };
                let r = ui.tile_wide(image, &name, *path == current, THUMB.0, THUMB.1);
                if r.clicked() {
                    chosen = Some(path.clone());
                }
                // Imagens adicionadas pelo usuário: menu para tirar da lista.
                if let Some(p) = path.as_ref().filter(|p| p.starts_with(mide::places::user_backgrounds_dir())) {
                    ui.context_menu(&r, |m| {
                        if m.danger(&tr!("Remove from List")) {
                            remove = Some(p.clone());
                        }
                    });
                }
            }
            add = ui.tile_wide(Tile::Icon("list-add"), &tr!("Add Image…"), false, THUMB.0, THUMB.1).clicked();
        });
        ui.note(&tr!("Right-click an added image to remove it from the list."));
        if let Some(path) = chosen {
            self.set_wallpaper(path);
        }
        if let Some(path) = remove {
            if current.as_ref() == Some(&path) {
                self.set_wallpaper(None);
            }
            match fs::remove_file(&path) {
                Ok(()) => tracing::info!("papel de parede removido da lista: {}", path.display()),
                Err(err) => tracing::warn!("não deu para remover {}: {err}", path.display()),
            }
            self.refresh_wallpapers();
        }
        if add {
            self.open_picker(mide::places::pictures_dir());
        }
    }

    /// Seletor de imagens: lugares, a pasta atual (pastas e imagens) e Cancelar.
    fn picker_view(&mut self, ui: &mut Ui) {
        let Some(picker) = self.picker.as_ref() else {
            return;
        };
        let dir = picker.dir.clone();
        let entries = picker.entries.clone();
        let thumbs = picker.thumbs.clone();
        ui.title(&tr!("Choose an Image"));
        let mut go = None;
        let mut choose = None;
        let mut cancel = false;
        ui.row(|ui| {
            for (name, place) in [(tr!("Pictures"), mide::places::pictures_dir()), (tr!("Downloads"), mide::places::downloads_dir()), (tr!("Home Folder"), mide::places::home_dir())] {
                let r = if place == dir { ui.primary_button(&name) } else { ui.button(&name) };
                if r.clicked() {
                    go = Some(place);
                }
            }
            if ui.button(&tr!("‹ Back")).clicked() {
                go = dir.parent().map(|p| p.to_path_buf());
            }
            cancel = ui.button(&tr!("Cancel")).clicked();
        });
        ui.note(&dir.display().to_string());
        if entries.is_empty() {
            ui.note(&tr!("No images in this folder."));
        }
        ui.wrap(|ui| {
            for (path, is_dir) in &entries {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let thumb = thumbs.get(path).cloned().flatten();
                let image = match (&thumb, is_dir) {
                    (_, true) => Tile::Icon("folder"),
                    (Some(pix), false) => Tile::Image(pix),
                    (None, false) => Tile::Icon("image-x-generic"),
                };
                if ui.tile_wide(image, &name, false, THUMB.0, THUMB.1).clicked() {
                    if *is_dir {
                        go = Some(path.clone());
                    } else {
                        choose = Some(path.clone());
                    }
                }
            }
        });
        if cancel {
            self.picker = None;
        } else if let Some(dir) = go {
            self.open_picker(dir);
        } else if let Some(image) = choose {
            self.picker = None;
            match mide::places::add_background(&image) {
                Ok(copy) => {
                    tracing::info!("papel de parede adicionado: {}", copy.display());
                    self.refresh_wallpapers();
                    self.set_wallpaper(Some(copy));
                }
                Err(err) => tracing::warn!("não deu para adicionar {}: {err}", image.display()),
            }
        }
    }

    fn dock(&mut self, ui: &mut Ui) {
        ui.title("Dock");
        let mut size = self.dock_size;
        let mut autohide = matches!(settings_file::get("dock", "autohide"), Some(Value::Bool(true)));
        ui.card(|ui| {
            if ui.slider(&tr!("Icon size"), &mut size, DOCK_MIN, DOCK_MAX).submitted() {
                let n = size.round() as i64;
                let value = (n != DOCK_DEFAULT).then_some(Value::Int(n));
                match settings_file::set("dock", "icon_size", value) {
                    Ok(()) => tracing::info!("tamanho dos ícones da dock: {n} px"),
                    Err(err) => tracing::warn!("tamanho da dock não gravado: {err}"),
                }
            }
            let detail = tr!("The Dock hides and appears when you move the pointer to the bottom edge");
            if ui.switch_detail(&tr!("Automatically hide and show the Dock"), &detail, &mut autohide).changed() {
                match settings_file::set("dock", "autohide", autohide.then_some(Value::Bool(true))) {
                    Ok(()) => tracing::info!("dock: ocultar automaticamente {}", if autohide { "ligado" } else { "desligado" }),
                    Err(err) => tracing::warn!("opção da dock não gravada: {err}"),
                }
            }
            // Ampliação (como no Mac): os ícones crescem sob o ponteiro.
            let mut magnify = settings_file::flag("dock", "magnification", false);
            if ui.switch_detail(&tr!("Magnification"), &tr!("Icons grow when the pointer moves over them"), &mut magnify).changed() {
                match settings_file::set("dock", "magnification", magnify.then_some(Value::Bool(true))) {
                    Ok(()) => tracing::info!("dock: ampliação {}", if magnify { "ligada" } else { "desligada" }),
                    Err(err) => tracing::warn!("opção da dock não gravada: {err}"),
                }
            }
            if magnify {
                let mut zoom = match settings_file::get("dock", "magnify_size") {
                    Some(Value::Int(n)) => n as f32,
                    _ => 70.0,
                };
                if ui.slider(&tr!("Magnification size"), &mut zoom, 56.0, 128.0).submitted() {
                    let n = zoom.round() as i64;
                    match settings_file::set("dock", "magnify_size", (n != 70).then_some(Value::Int(n))) {
                        Ok(()) => tracing::info!("dock: ampliação até {n} px"),
                        Err(err) => tracing::warn!("opção da dock não gravada: {err}"),
                    }
                }
            }
        });
        let mut click_minimize = settings_file::flag("dock", "click_minimize", true);
        ui.card(|ui| {
            let detail = tr!("With one window, minimizes it; with several, shows previews to choose from");
            if ui.switch_detail(&tr!("Clicking the focused app minimizes it"), &detail, &mut click_minimize).changed() {
                match settings_file::set("dock", "click_minimize", (!click_minimize).then_some(Value::Bool(false))) {
                    Ok(()) => tracing::info!("dock: clicar no app em foco minimiza {}", if click_minimize { "ligado" } else { "desligado" }),
                    Err(err) => tracing::warn!("opção da dock não gravada: {err}"),
                }
            }
        });
        self.dock_size = size;
        ui.note(&tr!("Without hiding, maximized windows stay above the Dock. To pin an app, right-click its icon and choose “Keep in Dock”."));
    }

    /// Bloqueio de tela (`[lock]`): depois de quanto tempo sem uso e ao suspender.
    fn lock(&mut self, ui: &mut Ui) {
        ui.title(&tr!("Lock Screen"));
        const MINUTES: [i64; 7] = [0, 1, 2, 5, 10, 15, 30];
        let current = match settings_file::get("lock", "idle_minutes") {
            Some(Value::Int(n)) => n,
            _ => 5,
        };
        let options: Vec<String> = MINUTES.iter().map(|m| duration_label(*m)).collect();
        let mut index = MINUTES.iter().position(|m| *m == current).unwrap_or(3);
        const BLANK: [i64; 8] = [0, 1, 2, 5, 10, 15, 30, 60];
        let blank_current = match settings_file::get("lock", "blank_minutes") {
            Some(Value::Int(n)) => n,
            _ => 10,
        };
        let blank_options: Vec<String> = BLANK.iter().map(|m| duration_label(*m)).collect();
        let mut blank_index = BLANK.iter().position(|m| *m == blank_current).unwrap_or(4);
        let mut on_suspend = settings_file::flag("lock", "on_suspend", true);
        ui.card(|ui| {
            if ui.dropdown(&tr!("Lock the screen when idle for"), &options, &mut index).changed() {
                let n = MINUTES[index];
                match settings_file::set("lock", "idle_minutes", (n != 5).then_some(Value::Int(n))) {
                    Ok(()) => {
                        tracing::info!("bloqueio: após {n} min sem uso");
                        // O compositor é quem conta o tempo sem uso: relê na hora (sem isto,
                        // a opção só valia no próximo login).
                        ask_compositor("reload");
                    }
                    Err(err) => tracing::warn!("opção do bloqueio não gravada: {err}"),
                }
            }
            if ui.dropdown(&tr!("Turn off the display when idle for"), &blank_options, &mut blank_index).changed() {
                let n = BLANK[blank_index];
                match settings_file::set("lock", "blank_minutes", (n != 10).then_some(Value::Int(n))) {
                    Ok(()) => {
                        tracing::info!("apagar a tela após {n} min sem uso");
                        // O compositor é quem conta o tempo sem uso: relê na hora (sem isto,
                        // a opção só valia no próximo login).
                        ask_compositor("reload");
                    }
                    Err(err) => tracing::warn!("opção do bloqueio não gravada: {err}"),
                }
            }
            if ui.switch_detail(&tr!("Lock when suspending"), &tr!("The password is required when waking from sleep"), &mut on_suspend).changed() {
                match settings_file::set("lock", "on_suspend", (!on_suspend).then_some(Value::Bool(false))) {
                    Ok(()) => {
                        tracing::info!("bloqueio ao suspender {}", if on_suspend { "ligado" } else { "desligado" });
                        // O compositor é quem conta o tempo sem uso: relê na hora (sem isto,
                        // a opção só valia no próximo login).
                        ask_compositor("reload");
                    }
                    Err(err) => tracing::warn!("opção do bloqueio não gravada: {err}"),
                }
            }
        });
        if ui.primary_button(&tr!("Lock Now")).clicked() {
            if std::env::var_os("MIDE_SYSTEM_DRY_RUN").is_some() {
                tracing::info!("sistema (teste): mide-lock");
            } else if let Err(err) = std::process::Command::new("mide-lock").spawn() {
                tracing::warn!("não foi possível abrir a tela de bloqueio: {err}");
            }
        }
        ui.note(&tr!("Super+L also locks. With the screen locked, the display turns off after 1 minute. Playing videos (in the browser, in mpv) prevent locking and turning off. In mide running in a window (development), the desktop underneath locks when idle."));
    }

    /// Itens da direita da barra superior (`[bar]`): todos à mostra por padrão.
    fn bar(&mut self, ui: &mut Ui) {
        ui.title(&tr!("Menu Bar"));
        ui.card(|ui| {
            // Tahoe: sem fundo por padrão (o texto fica sobre o papel de parede).
            let mut bg = settings_file::flag("bar", "background", false);
            if ui.switch_detail(&tr!("Show menu bar background"), &tr!("A translucent strip behind the menu bar; when off, the text sits right on the wallpaper, as in macOS Tahoe"), &mut bg).changed() {
                match settings_file::set("bar", "background", bg.then_some(Value::Bool(true))) {
                    Ok(()) => tracing::info!("barra superior: fundo {}", if bg { "ligado" } else { "desligado" }),
                    Err(err) => tracing::warn!("fundo da barra não gravado: {err}"),
                }
            }
            // Menu global: os menus do app em foco na barra superior, como no Mac. Saída de
            // segurança para apps que perdem o menu: desligado, os apps mantêm a barra de
            // menus deles dentro da janela.
            let mut menus = settings_file::flag("bar", "app_menus", true);
            if ui.switch_detail(&tr!("Show app menus in the menu bar"), &tr!("The menus of the app in focus appear in the menu bar, as on the Mac. Turn off if an app loses its menus: apps keep the menu bar in their own window. Apps already open may need to be reopened."), &mut menus).changed() {
                match settings_file::set("bar", "app_menus", (!menus).then_some(Value::Bool(false))) {
                    Ok(()) => {
                        tracing::info!("barra superior: menus dos apps {}", if menus { "ligados" } else { "desligados" });
                        ask_compositor("reload");
                    }
                    Err(err) => tracing::warn!("menus dos apps não gravados: {err}"),
                }
            }
        });
        ui.card(|ui| {
            for (key, label, detail) in settings_file::BAR_ITEMS {
                let mut on = settings_file::bar_item(key);
                let (name, about) = bar_item_text(key, label, detail);
                if ui.switch_detail(&name, &about, &mut on).changed() {
                    // À mostra é o padrão: grava só quando esconde.
                    let value = if on { None } else { Some(Value::Bool(false)) };
                    match settings_file::set("bar", key, value) {
                        Ok(()) => tracing::info!("barra superior: {label} {}", if on { "à mostra" } else { "escondido" }),
                        Err(err) => tracing::warn!("item da barra não gravado: {err}"),
                    }
                }
            }
        });
        ui.note(&tr!("The clock is always in the menu bar. Hidden items are still in Settings and in Control Center."));
    }

    /// Claro ou escuro (`[appearance] theme`): os apps do mide mudam na hora; os apps
    /// GTK e Qt abertos depois também (`mide::appearance`).
    fn appearance(&mut self, ui: &mut Ui) {
        ui.title(&tr!("Appearance"));
        let options = [tr!("Light").into_owned(), tr!("Dark").into_owned()];
        let mut chosen = usize::from(mide::kit::system_dark());
        ui.card(|ui| {
            if ui.dropdown(&tr!("Appearance"), &options, &mut chosen).changed() {
                let value = (chosen == 1).then(|| Value::Text("dark".into()));
                match settings_file::set("appearance", "theme", value) {
                    Ok(()) => {
                        tracing::info!("aparência: {}", options[chosen]);
                        ask_compositor("reload");
                        mide::appearance::apply(true);
                    }
                    Err(err) => tracing::warn!("aparência não gravada: {err}"),
                }
            }
            // Cor de destaque (botões, seleção, interruptores, deslizantes), como no Mac.
            let dark = mide::kit::system_dark();
            let names: Vec<String> = mide::kit::ACCENTS.iter().map(|(key, name, ..)| accent_name(key, name)).collect();
            let colors: Vec<(mide::kit::Rgba, &str)> = mide::kit::ACCENTS
                .iter()
                .zip(&names)
                .map(|((_, _, light, dim), name)| {
                    let c = if dark { dim } else { light };
                    ([c[0], c[1], c[2], 255], name.as_str())
                })
                .collect();
            let current = mide::kit::accent_key();
            let mut accent = mide::kit::ACCENTS.iter().position(|(k, ..)| *k == current).unwrap_or(0);
            if ui.color_choice(&tr!("Accent color"), &colors, &mut accent).changed() {
                let key = mide::kit::ACCENTS[accent].0;
                // Azul é o padrão: grava só as outras.
                match settings_file::set("appearance", "accent", (key != "blue").then(|| Value::Text(key.into()))) {
                    Ok(()) => tracing::info!("aparência: cor de destaque {key}"),
                    Err(err) => tracing::warn!("cor de destaque não gravada: {err}"),
                }
            }
        });
        ui.note(&tr!("mide apps, the menu bar and the Dock change immediately. GTK and Qt apps follow light or dark when they are opened."));
    }

    fn effects(&mut self, ui: &mut Ui) {
        ui.title(&tr!("Effects"));
        ui.card(|ui| {
            for (key, label, detail) in EFFECTS {
                let mut on = effect_on(key);
                if ui.switch_detail(&mide::i18n::tr(label), &mide::i18n::tr(detail), &mut on).changed() {
                    // Ligado é o padrão: grava só quando desliga, e apaga a chave ao religar.
                    let value = if on { None } else { Some(Value::Bool(false)) };
                    match settings_file::set("effects", key, value) {
                        Ok(()) => {
                            tracing::info!("efeito {key}: {}", if on { "ligado" } else { "desligado" });
                            ask_compositor("reload");
                        }
                        Err(err) => tracing::warn!("efeito não gravado: {err}"),
                    }
                }
            }
        });
        // Transparência das janelas sem foco (`[opacity] inactive`): 0% é opaca.
        let mut clear = match settings_file::get("opacity", "inactive") {
            Some(Value::Float(x)) => ((1.0 - x) * 100.0) as f32,
            _ => 0.0,
        };
        ui.heading(&tr!("Transparency"));
        ui.card(|ui| {
            if ui.slider(&tr!("Inactive windows"), &mut clear, 0.0, 60.0).submitted() {
                let opacity = ((100.0 - clear.round()) / 100.0) as f64;
                tracing::info!("janelas sem foco: opacidade {opacity}");
                let value = (opacity < 0.995).then_some(Value::Float(opacity));
                match settings_file::set("opacity", "inactive", value) {
                    Ok(()) => {
                        ask_compositor("reload");
                    }
                    Err(err) => tracing::warn!("transparência não gravada: {err}"),
                }
            }
        });
        ui.note(&tr!("Changes take effect immediately. Per-app transparency: [opacity] in config.toml (\"app_id\" = 0.9)."));
    }

    /// Mouse e trackpad (`[input]`): velocidade, tamanho do ponteiro, aceleração,
    /// rolagem natural e toque para clicar. Valem na hora para os aparelhos conectados.
    fn mouse(&mut self, ui: &mut Ui) {
        ui.title(&tr!("Mouse & Trackpad"));
        // Grava só o que difere do padrão (o resto sai do arquivo).
        let save = |key: &str, value: Option<Value>| match settings_file::set("input", key, value) {
            Ok(()) => {
                ask_compositor("reload");
            }
            Err(err) => tracing::warn!("opção de mouse não gravada: {err}"),
        };
        let flag = |key: &str, default: bool| match settings_file::get("input", key) {
            Some(Value::Bool(b)) => b,
            _ => default,
        };
        let mut speed = match settings_file::get("input", "pointer_speed") {
            Some(Value::Float(x)) => x as f32,
            Some(Value::Int(n)) => n as f32,
            _ => 0.0,
        } * 50.0
            + 50.0;
        // Tamanho do ponteiro em pontos (24 é o padrão), como o do Mac em Acessibilidade.
        let mut cursor_size = match settings_file::get("input", "cursor_size") {
            Some(Value::Int(n)) => n as f32,
            Some(Value::Float(x)) => x as f32,
            _ => 24.0,
        }
        .clamp(16.0, 96.0);
        let mut accel = flag("mouse_acceleration", true);
        let mut mouse_natural = flag("mouse_natural_scroll", false);
        let mut pad_natural = flag("touchpad_natural_scroll", true);
        let mut tap = flag("tap_to_click", true);
        ui.card(|ui| {
            if ui.slider(&tr!("Pointer speed"), &mut speed, 0.0, 100.0).submitted() {
                let value = ((speed - 50.0) / 50.0).clamp(-1.0, 1.0) as f64;
                let value = (value * 100.0).round() / 100.0;
                tracing::info!("mouse: velocidade {value}");
                save("pointer_speed", (value.abs() > 0.005).then_some(Value::Float(value)));
            }
            if ui.slider(&tr!("Pointer size"), &mut cursor_size, 16.0, 96.0).submitted() {
                let size = cursor_size.round() as i64;
                tracing::info!("mouse: tamanho do ponteiro {size}");
                save("cursor_size", (size != 24).then_some(Value::Int(size)));
            }
            if ui.switch_detail(&tr!("Mouse acceleration"), &tr!("Fast movements take the pointer farther"), &mut accel).changed() {
                tracing::info!("mouse: aceleração {}", if accel { "ligada" } else { "desligada" });
                save("mouse_acceleration", (!accel).then_some(Value::Bool(false)));
            }
            if ui.switch_detail(&tr!("Natural scrolling on the mouse"), &tr!("Content follows the movement of the wheel"), &mut mouse_natural).changed() {
                tracing::info!("mouse: rolagem natural {}", if mouse_natural { "ligada" } else { "desligada" });
                save("mouse_natural_scroll", mouse_natural.then_some(Value::Bool(true)));
            }
        });
        ui.heading("Trackpad");
        ui.card(|ui| {
            if ui.switch_detail(&tr!("Natural scrolling"), &tr!("Content follows your fingers, as on the Mac"), &mut pad_natural).changed() {
                tracing::info!("trackpad: rolagem natural {}", if pad_natural { "ligada" } else { "desligada" });
                save("touchpad_natural_scroll", (!pad_natural).then_some(Value::Bool(false)));
            }
            if ui.switch_detail(&tr!("Tap to click"), &tr!("A tap on the trackpad clicks, without pressing"), &mut tap).changed() {
                tracing::info!("trackpad: toque para clicar {}", if tap { "ligado" } else { "desligado" });
                save("tap_to_click", (!tap).then_some(Value::Bool(false)));
            }
        });
        ui.note(&tr!("Changes take effect immediately for connected mice and trackpads (in the mide session)."));
    }

    fn keyboard(&mut self, ui: &mut Ui) {
        ui.title(&tr!("Keyboard"));
        let mut mac = matches!(settings_file::get("keyboard", "mac_style"), Some(Value::Bool(true)));
        // Layouts: a lista escolhida (vazia: o do sistema). Com mais de um, o atalho
        // Ctrl+Super+Espaço e o seletor da barra trocam entre eles.
        let mut list = mide::keyboard_layouts::configured();
        let shown = if list.is_empty() { vec![mide::keyboard_layouts::system()] } else { list.clone() };
        let mut add_options = vec![tr!("Choose…").into_owned()];
        let addable: Vec<&(&str, &str, &str)> = mide::keyboard_layouts::LAYOUTS.iter().filter(|(_, l, v)| !shown.iter().any(|c| c.layout == *l && c.variant == *v)).collect();
        add_options.extend(addable.iter().map(|(n, ..)| mide::i18n::tr(n).into_owned()));
        let mut add_i = 0;
        let mut show_bar = settings_file::flag("keyboard", "show_layout", true);
        let mut changed = false;
        ui.card(|ui| {
            for (i, choice) in shown.iter().enumerate() {
                let detail = if shown.len() > 1 { tr!("Remove") } else { "".into() };
                let label = if list.is_empty() { tr!("{layout} (system)", layout = choice.name()) } else { choice.name() };
                if ui.list_item(&label, &detail).clicked() && shown.len() > 1 {
                    list.remove(i);
                    changed = true;
                }
            }
            if ui.dropdown(&tr!("Add layout"), &add_options, &mut add_i).changed() && add_i > 0 {
                if list.is_empty() {
                    list.push(mide::keyboard_layouts::system());
                }
                let (_, l, v) = addable[add_i - 1];
                list.push(mide::keyboard_layouts::Choice { layout: l.to_string(), variant: v.to_string() });
                changed = true;
            }
            if shown.len() > 1 && ui.switch_detail(&tr!("Show the layout in the menu bar"), &tr!("Abbreviation of the current layout in the menu bar, with the list to switch (Ctrl+Super+Space also switches)"), &mut show_bar).changed() {
                if let Err(err) = settings_file::set("keyboard", "show_layout", (!show_bar).then_some(Value::Bool(false))) {
                    tracing::warn!("opção do teclado não gravada: {err}");
                }
            }
            let detail = tr!("Command (Alt key) acts as Ctrl in apps and Option (Super/Windows key) is Alt; Ctrl stays Ctrl. Lock: Ctrl+⌘+Q");
            if ui.switch_detail(&tr!("Mac-style keys"), &detail, &mut mac).changed() {
                match settings_file::set("keyboard", "mac_style", mac.then_some(Value::Bool(true))) {
                    Ok(()) => {
                        tracing::info!("teclas no estilo Mac: {}", if mac { "ligado" } else { "desligado" });
                        ask_compositor("reload");
                    }
                    Err(err) => tracing::warn!("opção do teclado não gravada: {err}"),
                }
            }
        });
        // Repetição de tecla segurada: espera antes de repetir e velocidade. Vale na hora
        // (o compositor manda o novo valor a todos os apps).
        let int = |key: &str, default: i64| match settings_file::get("keyboard", key) {
            Some(Value::Int(n)) if n > 0 => n,
            _ => default,
        };
        let mut delay = int("repeat_delay", 600) as f32;
        let mut rate = int("repeat_rate", 25) as f32;
        ui.card(|ui| {
            if ui.slider(&tr!("Repeat delay (ms)"), &mut delay, 150.0, 2000.0).submitted() {
                let value = (delay / 50.0).round() as i64 * 50;
                tracing::info!("teclado: repetição depois de {value} ms");
                match settings_file::set("keyboard", "repeat_delay", (value != 600).then_some(Value::Int(value))) {
                    Ok(()) => {
                        ask_compositor("reload");
                    }
                    Err(err) => tracing::warn!("repetição não gravada: {err}"),
                }
            }
            if ui.slider(&tr!("Repeat rate (per second)"), &mut rate, 2.0, 60.0).submitted() {
                let value = rate.round() as i64;
                tracing::info!("teclado: {value} repetições por segundo");
                match settings_file::set("keyboard", "repeat_rate", (value != 25).then_some(Value::Int(value))) {
                    Ok(()) => {
                        ask_compositor("reload");
                    }
                    Err(err) => tracing::warn!("repetição não gravada: {err}"),
                }
            }
        });
        // Teclado na tela (`mide-keyboard`): sozinho no modo tablet; com esta opção,
        // sempre que um campo de texto pede (lida a cada campo, vale na hora).
        let mut on_screen = settings_file::flag("keyboard", "on_screen", false);
        ui.card(|ui| {
            if ui
                .switch_detail(&tr!("Show the on-screen keyboard when typing"), &tr!("Without this, it appears only in tablet mode (screen folded back)"), &mut on_screen)
                .changed()
            {
                match settings_file::set("keyboard", "on_screen", on_screen.then_some(Value::Bool(true))) {
                    Ok(()) => tracing::info!("teclado na tela: sempre {}", if on_screen { "ligado" } else { "desligado" }),
                    Err(err) => tracing::warn!("opção do teclado na tela não gravada: {err}"),
                }
            }
        });
        if changed {
            // Um layout só volta a ser o do sistema quando é o do sistema.
            let value = (list.len() > 1 || list.first().is_some_and(|c| *c != mide::keyboard_layouts::system())).then(|| Value::Text(mide::keyboard_layouts::join(&list)));
            let saved = settings_file::set("keyboard", "layouts", value)
                .and_then(|_| settings_file::set("keyboard", "layout", None))
                .and_then(|_| settings_file::set("keyboard", "variant", None));
            match saved {
                Ok(()) => {
                    tracing::info!("teclado: layouts {}", mide::keyboard_layouts::join(&list));
                    ask_compositor("reload");
                }
                Err(err) => tracing::warn!("layouts não gravados: {err}"),
            }
            ui.request_repaint();
        }
        // Busca pelo nome do atalho (ou pela combinação), sem diferenciar acentos.
        let mut query = std::mem::take(&mut self.shortcut_query);
        ui.text_input(&mut query, &tr!("Search shortcuts"));
        self.shortcut_query = query;
        let wanted = mide::apps::fold(self.shortcut_query.trim());
        let bindings = self.bindings.clone();
        let mut chosen = None;
        let mut shown = 0;
        // Por grupo: Janelas, Launcher e busca, Apps, Sistema.
        for (group, group_name) in SHORTCUT_GROUPS {
            let items: Vec<(String, String, String)> = bindings
                .iter()
                .filter(|(_, action)| shortcut_group(action) == group)
                .map(|(combo, action)| (describe_action(action, &self.apps), combo.clone(), action.clone()))
                .filter(|(title, combo, _)| {
                    wanted.is_empty() || mide::apps::fold(title).contains(&wanted) || mide::apps::fold(&pretty_combo(combo, mac)).contains(&wanted)
                })
                .collect();
            if items.is_empty() {
                continue;
            }
            shown += items.len();
            // Grupos recolhem (lembrado em `[settings] shortcuts_closed`); buscando,
            // os grupos com resultado ficam abertos.
            let mut open = !wanted.is_empty() || !self.closed_groups.iter().any(|g| g == group);
            let count = trn("{n} shortcut", "{n} shortcuts", items.len() as u64);
            if ui.disclosure(&mide::i18n::tr(group_name), &count, &mut open).changed() && wanted.is_empty() {
                if open {
                    self.closed_groups.retain(|g| g != group);
                } else {
                    self.closed_groups.push(group.to_string());
                }
                let value = (!self.closed_groups.is_empty()).then(|| Value::Text(self.closed_groups.join(",")));
                if let Err(err) = settings_file::set("settings", "shortcuts_closed", value) {
                    tracing::warn!("grupos de atalhos não gravados: {err}");
                }
            }
            if !open {
                continue;
            }
            ui.card(|ui| {
                for (title, combo, action) in &items {
                    if ui.list_item(title, &pretty_combo(combo, mac)).clicked() {
                        chosen = Some((combo.clone(), action.clone()));
                    }
                }
            });
        }
        if shown == 0 {
            ui.note(&if bindings.is_empty() { tr!("No shortcuts.") } else { tr!("No shortcuts with that name.") });
        }
        let (mut add, mut reset) = (false, false);
        ui.row(|ui| {
            ui.sized(170.0, |ui| add = ui.button(&tr!("New Shortcut…")).clicked());
            ui.sized(200.0, |ui| reset = ui.button(&tr!("Restore Defaults")).clicked());
        });
        ui.note(&tr!("Click a shortcut to change its key combination or remove it. Changes take effect immediately."));
        if let Some((combo, action)) = chosen {
            self.shortcut = Some(ShortcutEditor::edit(combo, action));
            ui.request_repaint();
        }
        if add {
            self.shortcut = Some(ShortcutEditor::new());
            ui.request_repaint();
        }
        if reset {
            match settings_file::clear_section("keybindings") {
                Ok(()) => self.apply_bindings(),
                Err(err) => tracing::warn!("atalhos não restaurados: {err}"),
            }
            ui.request_repaint();
        }
    }

    /// Idioma e formatos (`[locale] lang`/`formats`): o `mide-session` exporta no
    /// próximo login (`LANG` e `LC_TIME`, `LC_NUMERIC`, `LC_MONETARY`...).
    fn region(&mut self, ui: &mut Ui) {
        ui.title(&tr!("Language & Region"));
        if self.locales.is_empty() {
            self.locales = installed_locales();
        }
        let current = |key: &str| match settings_file::get("locale", key) {
            Some(Value::Text(t)) => t,
            _ => String::new(),
        };
        let mut options = vec![tr!("System default").into_owned()];
        options.extend(self.locales.iter().map(|l| locale_name(l)));
        let index_of = |value: &str| self.locales.iter().position(|l| *l == value).map_or(0, |i| i + 1);
        let (mut lang_i, mut formats_i) = (index_of(&current("lang")), index_of(&current("formats")));
        let locales = self.locales.clone();
        let save = |key: &str, i: usize| {
            let value = i.checked_sub(1).and_then(|i| locales.get(i)).map(|l| Value::Text(l.clone()));
            match settings_file::set("locale", key, value) {
                Ok(()) => tracing::info!("idioma e região: {key} = {}", options_name(&locales, i)),
                Err(err) => tracing::warn!("idioma não gravado: {err}"),
            }
        };
        ui.card(|ui| {
            if ui.dropdown(&tr!("Language"), &options, &mut lang_i).changed() {
                save("lang", lang_i);
            }
            if ui.dropdown(&tr!("Formats (date, time, numbers and currency)"), &options, &mut formats_i).changed() {
                save("formats", formats_i);
            }
        });
        ui.note(&tr!("Takes effect at the next login. The languages in the list are the ones installed on the system (locale -a)."));
    }

    /// Suspender por tempo sem uso e o botão de ligar (`[power]`).
    fn energy(&mut self, ui: &mut Ui) {
        ui.title(&tr!("Energy"));
        const SUSPEND: &[i64] = &[0, 15, 30, 60, 120, 180];
        const BUTTON: &[&str] = &["ask", "suspend", "poweroff", "nothing"];
        let minutes = match settings_file::get("power", "suspend_minutes") {
            Some(Value::Int(n)) => n,
            _ => 0,
        };
        let button = match settings_file::get("power", "button") {
            Some(Value::Text(t)) => t,
            _ => "ask".into(),
        };
        let suspend_names: Vec<String> = SUSPEND.iter().map(|m| duration_label(*m)).collect();
        let button_names: Vec<String> = BUTTON.iter().map(|b| power_button_label(b)).collect();
        let mut suspend_i = SUSPEND.iter().position(|m| *m == minutes).unwrap_or(0);
        let mut button_i = BUTTON.iter().position(|b| *b == button).unwrap_or(0);
        let save = |key: &str, value: Option<Value>| match settings_file::set("power", key, value) {
            Ok(()) => {
                ask_compositor("reload");
            }
            Err(err) => tracing::warn!("energia não gravada: {err}"),
        };
        ui.card(|ui| {
            if ui.dropdown(&tr!("Put the computer to sleep after"), &suspend_names, &mut suspend_i).changed() {
                let m = SUSPEND[suspend_i];
                tracing::info!("energia: suspender após {m} min");
                save("suspend_minutes", (m > 0).then_some(Value::Int(m)));
            }
            if ui.dropdown(&tr!("Power button"), &button_names, &mut button_i).changed() {
                let b = BUTTON[button_i];
                tracing::info!("energia: botão de ligar = {b}");
                save("button", (b != "ask").then(|| Value::Text(b.into())));
            }
        });
        ui.note(&tr!("When idle: the computer does not sleep while an app asks to keep the display on (video). Turning off and locking the display are in Lock Screen."));
    }

    /// App que abre cada tipo de arquivo (`~/.config/mide-mimeapps.list`, só na sessão do mide).
    fn default_apps(&mut self, ui: &mut Ui) {
        ui.title(&tr!("Default Apps"));
        // Por categoria: os apps que abrem o tipo (ids sem ".desktop"), os nomes e o
        // escolhido agora.
        let mut rows: Vec<(&mide::default_apps::Category, Vec<String>, Vec<String>, usize)> = Vec::new();
        for category in mide::default_apps::CATEGORIES {
            let candidates = mide::default_apps::candidates(&self.apps, category);
            let mut ids: Vec<String> = candidates.iter().map(|a| a.id.clone()).collect();
            let mut names: Vec<String> = candidates.iter().map(|a| a.name.clone()).collect();
            let current = mide::default_apps::current(category.mimes[0]).map(|c| desktop_id(&c));
            // O padrão atual pode não estar na lista (app sem o tipo no .desktop).
            if let Some(id) = current.as_ref().filter(|id| !ids.contains(id)) {
                ids.insert(0, id.clone());
                names.insert(0, id.clone());
            }
            if ids.is_empty() {
                continue;
            }
            let chosen = match current.and_then(|c| ids.iter().position(|i| *i == c)) {
                Some(i) => i,
                None => {
                    ids.insert(0, String::new());
                    names.insert(0, tr!("None").into_owned());
                    0
                }
            };
            rows.push((category, ids, names, chosen));
        }
        let mut changed = None;
        ui.card(|ui| {
            for (i, (category, _, names, chosen)) in rows.iter_mut().enumerate() {
                if ui.dropdown(&category_name(category.key, category.label), names, chosen).changed() {
                    changed = Some(i);
                }
            }
        });
        if let Some((category, ids, _, chosen)) = changed.and_then(|i| rows.get(i)) {
            if !ids[*chosen].is_empty() {
                let id = format!("{}.desktop", ids[*chosen]);
                match mide::default_apps::set(category, &id) {
                    Ok(()) => tracing::info!("apps padrão: {} = {id}", category.label),
                    Err(err) => tracing::warn!("app padrão não gravado: {err}"),
                }
            }
        }
        if rows.is_empty() {
            ui.note(&tr!("No installed app opens these file types."));
        }
        ui.note(&tr!("Applies only to the mide session: other installed desktops (Cinnamon, GNOME) keep their own choices."));
    }

    /// Apps e comandos que abrem ao entrar na sessão (autostart do freedesktop,
    /// `mide::login_items`), como os Itens de início de sessão do Mac.
    fn login_items(&mut self, ui: &mut Ui) {
        ui.title(&tr!("Login Items"));
        let items = self.login_items.get_or_insert_with(mide::login_items::list).clone();
        let mut changed = false;
        ui.note(&tr!("These open automatically when you log in."));
        ui.card(|ui| {
            if items.is_empty() {
                ui.note(&tr!("No items. Use Add App or Add Command."));
            }
            for item in &items {
                let mut on = item.enabled;
                let detail = if item.system { tr!("{command} · system", command = item.exec) } else { item.exec.clone() };
                let mut remove = false;
                ui.row(|ui| {
                    // Os do sistema não são apagados, só desligados: sem o botão de remover.
                    let w = ui.remaining_width() - 40.0;
                    ui.sized(w, |ui| {
                        if ui.switch_detail(&item.name, &detail, &mut on).changed() {
                            match mide::login_items::set_enabled(&item.file, on) {
                                Ok(()) => tracing::info!("itens de início: {} {}", item.name, if on { "ligado" } else { "desligado" }),
                                Err(err) => tracing::warn!("item de início não gravado: {err}"),
                            }
                            changed = true;
                        }
                    });
                    if !item.system {
                        let r = ui.icon_button("list-remove-symbolic", "−", true);
                        ui.tooltip(&r, &tr!("Remove"));
                        remove = r.clicked();
                    }
                });
                if remove {
                    match mide::login_items::remove(&item.file) {
                        Ok(()) => tracing::info!("itens de início: {} removido", item.name),
                        Err(err) => tracing::warn!("item de início não removido: {err}"),
                    }
                    changed = true;
                }
            }
        });
        let (mut add_app, mut add_cmd) = (false, false);
        ui.row(|ui| {
            ui.push_right(2.0 * 170.0 + 8.0);
            ui.sized(170.0, |ui| add_app = ui.button(&tr!("Add App…")).clicked());
            ui.sized(170.0, |ui| add_cmd = ui.button(&tr!("Add Command…")).clicked());
        });
        ui.note(&tr!("System items can only be turned off. The list also applies to other desktops, which read the same autostart folder."));
        if add_app || add_cmd {
            self.login_dialog = Some(LoginDialog { command_mode: add_cmd, ..Default::default() });
        }
        if changed {
            self.login_items = None;
            ui.request_repaint();
        }
    }

    /// Diálogo de adicionar um item de início.
    fn login_dialog(&mut self, ui: &mut Ui) {
        let Some(mut dialog) = self.login_dialog.take() else {
            return;
        };
        // Escolhendo o arquivo: o seletor ocupa o lugar do diálogo.
        if let Some(chooser) = dialog.chooser.as_mut() {
            match chooser.show(ui) {
                Some(mide::kit::Choice::Chosen(path)) => {
                    let text = path.to_string_lossy().into_owned();
                    dialog.command = if text.contains(' ') { format!("\"{text}\"") } else { text };
                    if dialog.name.trim().is_empty() {
                        dialog.name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                    }
                    dialog.chooser = None;
                }
                Some(mide::kit::Choice::Cancelled) => dialog.chooser = None,
                None => {}
            }
            ui.request_repaint();
            self.login_dialog = Some(dialog);
            return;
        }
        let mut close = ui.input().key(Key::Escape);
        let mut added = false;
        let mut pick: Option<mide::apps::App> = None;
        let mut save = false;
        let mut browse = false;
        let apps = &self.apps;
        ui.modal(440.0, |ui| {
            if !dialog.command_mode {
                ui.strong(&tr!("Add App"));
                ui.text_input(&mut dialog.query, &tr!("Search apps"));
                ui.card(|ui| {
                    let found = mide::apps::filter(apps, &dialog.query, None);
                    if found.is_empty() {
                        ui.note(&tr!("No apps found."));
                    }
                    for app in found.into_iter().take(6) {
                        if ui.list_item(&app.name, "").clicked() {
                            pick = Some(app.clone());
                        }
                    }
                });
                ui.row(|ui| {
                    ui.push_right(110.0);
                    ui.sized(110.0, |ui| close |= ui.button(&tr!("Cancel")).clicked());
                });
                return;
            }
            ui.strong(&tr!("Add Command"));
            ui.note(&tr!("A name, and the program or script that opens when you log in."));
            ui.text_input(&mut dialog.name, &tr!("Name"));
            ui.row(|ui| {
                let w = ui.remaining_width() - 118.0;
                ui.sized(w, |ui| {
                    ui.text_input(&mut dialog.command, &tr!("Command or script"));
                });
                ui.sized(110.0, |ui| browse = ui.button(&tr!("Choose…")).clicked());
            });
            if let Some(err) = &dialog.error {
                ui.error_note(err);
            }
            let ok = !dialog.name.trim().is_empty() && !dialog.command.trim().is_empty();
            ui.row(|ui| {
                ui.push_right(2.0 * 120.0 + 8.0);
                ui.sized(120.0, |ui| close |= ui.button(&tr!("Cancel")).clicked());
                ui.sized(120.0, |ui| save = ui.primary_button(&tr!("Add")).clicked() && ok);
            });
            save |= ok && ui.input().key(Key::Enter);
        });
        if browse {
            dialog.chooser = Some(mide::kit::FileChooser::open(&tr!("Choose a Program or Script"), None));
        }
        if let Some(app) = pick {
            match mide::login_items::add_app(&app) {
                Ok(file) => tracing::info!("itens de início: {} adicionado ({file})", app.name),
                Err(err) => tracing::warn!("item de início não adicionado: {err}"),
            }
            added = true;
        }
        if save {
            match mide::login_items::add_command(dialog.name.trim(), dialog.command.trim()) {
                Ok(file) => {
                    tracing::info!("itens de início: comando {} adicionado ({file})", dialog.name.trim());
                    added = true;
                }
                Err(err) => dialog.error = Some(tr!("Could not save: {error}", error = err)),
            }
        }
        if added {
            self.login_items = None;
            ui.request_repaint();
        } else if !close {
            self.login_dialog = Some(dialog);
        }
    }

    /// Avisa o compositor e relê os atalhos em uso.
    fn apply_bindings(&mut self) {
        ask_compositor("reload");
        self.load_bindings();
    }

    /// Grava a combinação nova da ação do diálogo. A combinação antiga é desligada
    /// (`none`, que também desliga um atalho padrão); a nova substitui o que houver nela.
    fn save_shortcut(&mut self, new: &str) {
        let Some(editor) = self.shortcut.as_ref() else {
            return;
        };
        let Some(action) = editor.action.clone() else {
            return;
        };
        let old = editor.old.clone();
        let result = old
            .filter(|old| old != new)
            .map_or(Ok(()), |old| settings_file::set("keybindings", &old, Some(Value::Text("none".into()))))
            .and_then(|()| settings_file::set("keybindings", new, Some(Value::Text(action.clone()))));
        match result {
            Ok(()) => {
                tracing::info!("atalho gravado: {new} = {action}");
                self.shortcut = None;
                self.apply_bindings();
            }
            Err(err) => {
                tracing::warn!("atalho não gravado: {err}");
                if let Some(editor) = self.shortcut.as_mut() {
                    editor.error = Some(tr!("Could not save: {error}", error = err));
                }
            }
        }
    }

    /// Diálogo do atalho: escolher a ação (atalho novo), gravar a combinação apertando
    /// as teclas, avisar se ela já é usada por outra ação, ou remover o atalho.
    fn shortcut_dialog(&mut self, ui: &mut Ui, mac: bool) {
        let Some(mut editor) = self.shortcut.take() else {
            return;
        };
        // Gravando: pergunta ao compositor se a combinação já foi apertada.
        if editor.capturing {
            match ask_compositor("shortcuts captured").and_then(|l| l.into_iter().next()).as_deref() {
                Some("waiting") => ui.wake_after(Duration::from_millis(80)),
                Some(combo) if combo != "cancel" && combo != "none" => {
                    editor.capturing = false;
                    editor.error = None;
                    match combo_problem(combo) {
                        Some(problem) => editor.error = Some(problem.into_owned()),
                        None => {
                            let other = self
                                .bindings
                                .iter()
                                .find(|(c, a)| c == combo && Some(c) != editor.old.as_ref() && Some(a) != editor.action.as_ref());
                            match other {
                                Some((_, a)) => {
                                    editor.conflict = Some((combo.to_string(), describe_action(a, &self.apps)));
                                }
                                None => {
                                    self.shortcut = Some(editor);
                                    self.save_shortcut(combo);
                                    ui.request_repaint();
                                    return;
                                }
                            }
                        }
                    }
                    ui.request_repaint();
                }
                _ => {
                    editor.capturing = false;
                    ui.request_repaint();
                }
            }
        }

        let title = match editor.action.as_deref() {
            Some(action) => describe_action(action, &self.apps),
            None => tr!("New Shortcut").into_owned(),
        };
        let mut close = !editor.capturing && ui.input().key(Key::Escape);
        let (mut record, mut remove, mut replace) = (false, false, false);
        let mut pick: Option<String> = None;
        let mut query = editor.query.clone();
        ui.modal(420.0, |ui| {
            ui.strong(&title);
            if editor.action.is_none() {
                // Sem busca, as ações do mide; buscando, os apps encontrados no lugar delas
                // (o diálogo cabe na janela).
                ui.note(&tr!("What does the shortcut do? A mide action or opening an app:"));
                ui.text_input(&mut query, &tr!("Search apps"));
                ui.card(|ui| {
                    if query.trim().is_empty() {
                        for action in FIXED_ACTIONS {
                            if ui.list_item(&describe_action(action, &[]), "").clicked() {
                                pick = Some(action.to_string());
                            }
                        }
                        return;
                    }
                    let found = mide::apps::filter(&self.apps, &query, None);
                    if found.is_empty() {
                        ui.note(&tr!("No apps found."));
                    }
                    for app in found.into_iter().take(5) {
                        if ui.list_item(&app.name, &tr!("App")).clicked() {
                            pick = Some(format!("launch:{}", app_command(app)));
                        }
                    }
                });
                ui.row(|ui| {
                    ui.push_right(110.0);
                    ui.sized(110.0, |ui| close |= ui.button(&tr!("Cancel")).clicked());
                });
                return;
            }
            if let Some((combo, used_by)) = &editor.conflict {
                ui.label(&tr!("{keys} is already used by “{action}”.", keys = pretty_combo(combo, mac), action = used_by));
                ui.note(&tr!("Replace moves this key combination to this shortcut."));
                ui.row(|ui| {
                    ui.push_right(2.0 * 150.0 + 8.0);
                    ui.sized(150.0, |ui| record = ui.button(&tr!("Record Another")).clicked());
                    ui.sized(150.0, |ui| replace = ui.primary_button(&tr!("Replace")).clicked());
                });
                return;
            }
            if editor.capturing {
                ui.label(&tr!("Press the new key combination…"));
                ui.note(&tr!("Use Ctrl, Alt or Super together with the key. Esc cancels."));
                return;
            }
            let current = editor.old.as_deref().map(|c| pretty_combo(c, mac)).unwrap_or_else(|| trc("key combination", "None").into_owned());
            ui.list_item(&tr!("Key combination"), &current);
            if let Some(err) = &editor.error {
                ui.note(err);
            }
            ui.row(|ui| {
                let buttons = if editor.old.is_some() { 3.0 } else { 2.0 };
                ui.push_right(buttons * 120.0 + (buttons - 1.0) * 8.0);
                if editor.old.is_some() {
                    ui.sized(120.0, |ui| remove = ui.button(&tr!("Remove")).clicked());
                }
                ui.sized(120.0, |ui| close |= ui.button(&tr!("Cancel")).clicked());
                ui.sized(120.0, |ui| record = ui.primary_button(&tr!("Record…")).clicked());
            });
        });
        editor.query = query;
        if let Some(action) = pick {
            editor.action = Some(action);
            ui.request_repaint();
        }
        if record {
            editor.conflict = None;
            editor.error = None;
            if ask_compositor("shortcuts capture").is_some() {
                editor.capturing = true;
                ui.wake_after(Duration::from_millis(80));
            } else {
                editor.error = Some(tr!("The compositor did not respond.").into_owned());
            }
            ui.request_repaint();
        }
        if close {
            if editor.capturing {
                ask_compositor("shortcuts stop");
            }
            ui.request_repaint();
            return;
        }
        if remove {
            if let Some(old) = editor.old.clone() {
                match settings_file::set("keybindings", &old, Some(Value::Text("none".into()))) {
                    Ok(()) => {
                        tracing::info!("atalho removido: {old}");
                        self.apply_bindings();
                    }
                    Err(err) => tracing::warn!("atalho não removido: {err}"),
                }
            }
            ui.request_repaint();
            return;
        }
        if replace {
            if let Some((combo, _)) = editor.conflict.take() {
                self.shortcut = Some(editor);
                self.save_shortcut(&combo);
                ui.request_repaint();
                return;
            }
        }
        self.shortcut = Some(editor);
    }

    fn about(&mut self, ui: &mut Ui) {
        ui.title(&tr!("About"));
        let info = self.info.clone();
        ui.card(|ui| {
            for (label, value) in &info {
                ui.list_item(label, value);
            }
        });
        ui.note(&format!("mide {}", env!("CARGO_PKG_VERSION")));
    }

    /// Resultados da busca: opções pelo nome ou por palavras-chave.
    fn results(&mut self, ui: &mut Ui) {
        ui.title(&tr!("Results"));
        let q = mide::apps::fold(self.query.trim());
        let found: Vec<&(&str, &str, Section)> = SEARCHABLE.iter().filter(|(name, keys, _)| search_matches(&q, name, keys)).collect();
        if found.is_empty() {
            ui.note(&tr!("No options found."));
            return;
        }
        let mut open = None;
        ui.card(|ui| {
            for (name, _, section) in &found {
                if ui.list_item(&mide::i18n::tr(name), &section_name(*section)).clicked() {
                    open = Some(*section);
                }
            }
        });
        if let Some(section) = open {
            self.section = section;
            self.query.clear();
            self.query_reset = true;
            if section == Section::Keyboard {
                self.load_bindings();
            }
            ui.request_repaint();
        }
    }
}

impl App for Settings {
    fn view(&mut self, ui: &mut Ui) {
        self.poll(ui);
        let names: Vec<std::borrow::Cow<str>> = SIDEBAR.iter().map(|(_, name, ..)| mide::i18n::tr(name)).collect();
        let items: Vec<(&str, &str, Option<[u8; 3]>)> = SIDEBAR.iter().zip(&names).map(|((s, _, icon, tint), name)| (&**name, if s.is_some() { *icon } else { "" }, s.map(|_| *tint))).collect();
        let selected = SIDEBAR.iter().position(|(s, ..)| *s == Some(self.section)).unwrap_or(0);
        // O campo de busca fica na barra lateral; o conteúdo usa a busca deste quadro
        // (o que for digitado agora pede outro quadro, que já mostra os resultados).
        let mut query = self.query.clone();
        let searching = !query.trim().is_empty();
        // Tahoe: barra lateral flutuante de vidro, como nos Ajustes do Sistema.
        ui.glass_sidebar();
        let chosen = ui.sidebar_search(&items, if searching { usize::MAX } else { selected }, &mut query, &tr!("Search"), |ui, _| {
            if !self.query.trim().is_empty() {
                self.results(ui);
                return;
            }
            match self.section {
                Section::Wifi => self.wifi(ui),
                Section::Network => self.network(ui),
                Section::Bluetooth => self.bluetooth(ui),
                Section::Appearance => self.appearance(ui),
                Section::Sound => self.sound(ui),
                Section::Displays => self.displays(ui),
                Section::Wallpaper => self.wallpaper(ui),
                Section::Dock => self.dock(ui),
                Section::Lock => self.lock(ui),
                Section::Users => self.users(ui),
                Section::Bar => self.bar(ui),
                Section::Effects => self.effects(ui),
                Section::Mouse => self.mouse(ui),
                Section::Keyboard => self.keyboard(ui),
                Section::Region => self.region(ui),
                Section::Energy => self.energy(ui),
                Section::DefaultApps => self.default_apps(ui),
                Section::LoginItems => self.login_items(ui),
                Section::About => self.about(ui),
            }
        });
        // Confirmação das telas por cima da janela inteira.
        if self.screens_revert.is_some() {
            self.displays_confirm(ui);
        }
        // Diálogo de atalho por cima da janela inteira (fora da área de conteúdo).
        if self.shortcut.is_some() {
            let mac = matches!(settings_file::get("keyboard", "mac_style"), Some(Value::Bool(true)));
            self.shortcut_dialog(ui, mac);
        }
        // Diálogo de adicionar item de início, também por cima da janela inteira.
        self.login_dialog(ui);
        // Folhas de Usuários e grupos.
        self.user_sheet(ui);
        self.delete_user_dialog(ui);
        self.add_user_dialog(ui);
        // Escolher um resultado limpa a busca: aí o que estava no campo não volta.
        if std::mem::take(&mut self.query_reset) {
            ui.request_repaint();
        } else if query != self.query {
            self.query = query;
            ui.request_repaint();
        }
        if chosen != selected && chosen != usize::MAX {
            if let Some((Some(section), ..)) = SIDEBAR.get(chosen) {
                self.section = *section;
                self.query.clear();
                self.picker = None;
                tracing::info!("configurações: {section:?}");
                if *section == Section::Keyboard {
                    self.load_bindings();
                }
                ui.request_repaint();
            }
        }
    }
}

/// Id do app sem o ".desktop" (como em `mide::apps::App::id`).
fn desktop_id(id: &str) -> String {
    id.trim_end_matches(".desktop").to_string()
}

/// Idiomas instalados com UTF-8 (`locale -a`) que a lista sabe nomear, no formato do
/// `LANG` (`pt_BR.UTF-8`).
fn installed_locales() -> Vec<String> {
    let out = std::process::Command::new("locale").arg("-a").output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    let mut list: Vec<String> = out
        .lines()
        .map(str::trim)
        .filter(|l| l.to_lowercase().ends_with(".utf8") || l.to_lowercase().ends_with(".utf-8"))
        .map(|l| format!("{}.UTF-8", l.split('.').next().unwrap_or(l)))
        .filter(|l| LOCALE_NAMES.iter().any(|(code, _)| l.starts_with(&format!("{code}."))))
        .collect();
    list.sort_by_key(|l| locale_name(l));
    list.dedup();
    list
}

/// Idiomas com nome na lista (os outros instalados não aparecem: são muitos, e a
/// lista precisa caber na janela).
const LOCALE_NAMES: &[(&str, &str)] = &[
    ("pt_BR", "Português (Brasil)"),
    ("pt_PT", "Português (Portugal)"),
    ("en_US", "English (United States)"),
    ("en_GB", "English (United Kingdom)"),
    ("es_ES", "Español (España)"),
    ("es_MX", "Español (México)"),
    ("es_AR", "Español (Argentina)"),
    ("fr_FR", "Français (France)"),
    ("de_DE", "Deutsch (Deutschland)"),
    ("it_IT", "Italiano (Italia)"),
    ("nl_NL", "Nederlands"),
    ("ru_RU", "Русский"),
    ("ja_JP", "日本語"),
    ("zh_CN", "中文 (简体)"),
];

/// Nome do idioma para a lista ("Português (Brasil) — pt_BR").
fn locale_name(code: &str) -> String {
    let base = code.split('.').next().unwrap_or(code);
    match LOCALE_NAMES.iter().find(|(c, _)| *c == base) {
        Some((_, name)) => format!("{name} — {base}"),
        None => base.to_string(),
    }
}

fn options_name(locales: &[String], i: usize) -> String {
    i.checked_sub(1).and_then(|i| locales.get(i)).cloned().unwrap_or_else(|| "o do sistema".into())
}

/// Nome da seção na tela (traduzido).
fn section_name(section: Section) -> std::borrow::Cow<'static, str> {
    mide::i18n::tr(SIDEBAR.iter().find(|(s, ..)| *s == Some(section)).map(|(_, n, ..)| *n).unwrap_or(""))
}

/// A busca acha a opção pelo nome ou pelas palavras-chave, em inglês ou traduzidos,
/// sem diferenciar acentos (`q` já vem dobrado por `mide::apps::fold`).
fn search_matches(q: &str, name: &str, keys: &str) -> bool {
    let names = [mide::apps::fold(name), mide::apps::fold(&mide::i18n::tr(name))];
    let keys = format!("{keys} {}", mide::i18n::tr(keys));
    names.iter().any(|n| n.contains(q)) || mide::apps::fold(&keys).split(' ').any(|k| k.starts_with(q))
}

/// Tempo para as listas de Bloqueio de tela e Energia ("Nunca", "5 minutos", "1 hora").
fn duration_label(minutes: i64) -> String {
    match minutes {
        0 => tr!("Never").into_owned(),
        m if m % 60 == 0 => trn("{n} hour", "{n} hours", (m / 60) as u64),
        m => trn("{n} minute", "{n} minutes", m as u64),
    }
}

/// O que o botão de ligar faz, pela chave gravada em `[power] button`.
fn power_button_label(key: &str) -> String {
    match key {
        "ask" => tr!("Ask What to Do"),
        "suspend" => trc("power button", "Sleep"),
        "poweroff" => trc("power button", "Shut Down"),
        _ => tr!("Do Nothing"),
    }
    .into_owned()
}

/// Nome de um item da barra superior (`settings_file::BAR_ITEMS`, do mide-ui) e a
/// explicação, pela chave; um item desconhecido fica com o texto do mide-ui.
fn bar_item_text(key: &str, label: &str, detail: &str) -> (String, String) {
    let (name, about) = match key {
        "media" => (tr!("Media controls"), tr!("Current player, track and playback controls")),
        "gear" => (tr!("Settings"), tr!("Gear that opens System Settings")),
        "sound" => (tr!("Sound"), tr!("Volume and sound output")),
        "wifi" => ("Wi-Fi".into(), tr!("Wireless networks")),
        "bluetooth" => ("Bluetooth".into(), tr!("Wireless devices")),
        "battery" => (tr!("Battery"), tr!("Charge and time remaining (laptops only)")),
        "control" => (tr!("Control Center"), tr!("Wi-Fi, Bluetooth, Do Not Disturb, brightness and sound in a single panel")),
        _ => (label.into(), detail.into()),
    };
    (name.into_owned(), about.into_owned())
}

/// Nome de um som de alerta (`mide::sounds::ALERTS`, do mide-ui), pela chave.
fn alert_name(key: &str, name: &str) -> String {
    match key {
        "vidro" => trc("alert sound", "Glass"),
        "gota" => trc("alert sound", "Drop"),
        "sino" => trc("alert sound", "Bell"),
        "pop" => trc("alert sound", "Pop"),
        _ => name.into(),
    }
    .into_owned()
}

/// Nome de uma cor de destaque (`mide::kit::ACCENTS`, do mide-ui), pela chave.
fn accent_name(key: &str, name: &str) -> String {
    match key {
        "blue" => tr!("Blue"),
        "purple" => tr!("Purple"),
        "pink" => tr!("Pink"),
        "red" => tr!("Red"),
        "orange" => tr!("Orange"),
        "yellow" => tr!("Yellow"),
        "green" => tr!("Green"),
        "graphite" => tr!("Graphite"),
        _ => name.into(),
    }
    .into_owned()
}

/// Nome de uma categoria de Apps padrão (`mide::default_apps::CATEGORIES`, do
/// mide-ui), pela chave.
fn category_name(key: &str, label: &str) -> String {
    match key {
        "web" => trc("default apps", "Web browser"),
        "mail" => trc("default apps", "Email"),
        "files" => trc("default apps", "Folders"),
        "text" => trc("default apps", "Text"),
        "documents" => trc("default apps", "Documents"),
        "image" => trc("default apps", "Images"),
        "video" => trc("default apps", "Videos"),
        "music" => trc("default apps", "Music"),
        "pdf" => "PDF".into(),
        _ => label.into(),
    }
    .into_owned()
}

fn effect_on(key: &str) -> bool {
    !matches!(settings_file::get("effects", key), Some(Value::Bool(false)))
}

fn current_wallpaper() -> Option<PathBuf> {
    match settings_file::get("desktop", "wallpaper") {
        Some(Value::Text(t)) if !t.trim().is_empty() => {
            let t = t.trim();
            Some(match t.strip_prefix("~/") {
                Some(rest) => PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(rest),
                None => PathBuf::from(t),
            })
        }
        _ => None,
    }
}

/// Papéis de parede para escolher: o padrão do mide (`None`) e as imagens do sistema e
/// as adicionadas pelo usuário. `MIDE_BACKGROUNDS_DIR` troca a pasta do sistema (os
/// testes usam uma com imagens conhecidas).
fn wallpaper_list() -> Vec<Option<PathBuf>> {
    let mut list = vec![None];
    let system = std::env::var_os("MIDE_BACKGROUNDS_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/usr/share/backgrounds"));
    let dirs = [system, mide::places::user_backgrounds_dir()];
    let mut files: Vec<PathBuf> = dirs
        .iter()
        .filter_map(|d| fs::read_dir(d).ok())
        .flat_map(|d| d.flatten().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|e| e.to_str()).is_some_and(|e| matches!(e.to_lowercase().as_str(), "jpg" | "jpeg" | "png")))
        .collect();
    files.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()));
    list.extend(files.into_iter().take(60).map(Some));
    list
}

/// Miniaturas carregadas numa thread, uma por mensagem.
fn load_thumbs(tx: Sender<Thumb>, paths: Vec<Option<PathBuf>>) {
    std::thread::spawn(move || {
        for path in paths {
            let thumb = match &path {
                Some(path) => image::open(path).ok().map(|img| to_pixmap(&mide::backdrop::cover(&img.to_rgba8(), THUMB.0, THUMB.1))),
                None => Some(to_pixmap(&mide::backdrop::default_gradient(THUMB.0, THUMB.1))),
            };
            if tx.send((path, thumb)).is_err() {
                break;
            }
        }
    });
}

/// Imagem RGBA para pixmap (premultiplicado).
fn to_pixmap(img: &image::RgbaImage) -> Pixmap {
    let mut pixmap = Pixmap::new(img.width().max(1), img.height().max(1)).unwrap_or_else(|| Pixmap::new(1, 1).unwrap());
    for (dst, src) in pixmap.data_mut().chunks_exact_mut(4).zip(img.as_raw().chunks_exact(4)) {
        let a = src[3] as u32;
        dst[0] = (src[0] as u32 * a / 255) as u8;
        dst[1] = (src[1] as u32 * a / 255) as u8;
        dst[2] = (src[2] as u32 * a / 255) as u8;
        dst[3] = src[3];
    }
    pixmap
}

/// "ctrl+shift+space" vira "Ctrl ⇧ Espaço" (ou "⌘ ⇧ Espaço" com as teclas no estilo Mac).
/// No estilo Mac, os atalhos do mide veem o ⌘ como Super (`super` vira ⌘), o Ctrl
/// junto do ⌘ é o ⌃ Control e um Ctrl sozinho também vale com o ⌘ (aparece como ⌘); os
/// modificadores vêm na ordem do Mac: ⌃ ⌥ ⇧ ⌘ e a tecla (⇧⌘4).
fn pretty_combo(combo: &str, mac: bool) -> String {
    let parts: Vec<&str> = combo.split('+').collect();
    let key_name = |k: &str| match k {
        "space" => tr!("Space").into_owned(),
        "escape" => "Esc".to_string(),
        "return" => "Enter".to_string(),
        "tab" => "Tab".to_string(),
        "up" => "↑".to_string(),
        "down" => "↓".to_string(),
        "left" => "←".to_string(),
        "right" => "→".to_string(),
        "print" => "Print Screen".to_string(),
        "delete" => "Delete".to_string(),
        "backspace" => "⌫".to_string(),
        other => other.to_uppercase(),
    };
    if mac {
        let has = |m: &str| parts.contains(&m);
        let command = has("super") || (has("ctrl") && !has("super"));
        let mut out = Vec::new();
        if has("ctrl") && has("super") {
            out.push("⌃".to_string());
        }
        if has("alt") {
            out.push("⌥".to_string());
        }
        if has("shift") {
            out.push("⇧".to_string());
        }
        if command {
            out.push("⌘".to_string());
        }
        out.extend(parts.iter().filter(|k| !matches!(**k, "ctrl" | "super" | "alt" | "shift")).map(|k| key_name(k)));
        return out.join(" ");
    }
    parts
        .iter()
        .map(|k| match *k {
            "ctrl" => "Ctrl".to_string(),
            "shift" => "⇧".to_string(),
            "alt" => "Alt".to_string(),
            "super" => "Super".to_string(),
            other => key_name(other),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Diálogo de mudar ou criar um atalho.
#[derive(Default)]
struct ShortcutEditor {
    /// Combinação atual (`None`: atalho novo).
    old: Option<String>,
    /// Ação do atalho (`None`: atalho novo, ainda escolhendo).
    action: Option<String>,
    /// Busca de apps, ao escolher a ação.
    query: String,
    /// Esperando o usuário apertar a combinação (o compositor grava).
    capturing: bool,
    /// Combinação apertada que já é de outra ação: a combinação e o nome da outra.
    conflict: Option<(String, String)>,
    error: Option<String>,
}

impl ShortcutEditor {
    fn new() -> Self {
        Self::default()
    }

    fn edit(combo: String, action: String) -> Self {
        Self { old: Some(combo), action: Some(action), ..Self::default() }
    }
}

/// Ações do mide que um atalho novo pode fazer (além de abrir um app).
const FIXED_ACTIONS: &[&str] = &[
    "switch_windows",
    "switch_windows_back",
    "maximize",
    "unmaximize",
    "toggle_maximize",
    "minimize",
    "tile_left",
    "tile_right",
    "toggle_fullscreen",
    "close_window",
    "restore_minimized",
    "show_desktop",
    "overview",
    "toggle_launcher",
    "toggle_search",
    "next_layout",
    "lock_screen",
    "quit",
];

/// Grupo de um atalho (pela ação): a chave do grupo em `SHORTCUT_GROUPS`.
fn shortcut_group(action: &str) -> &'static str {
    match action {
        "toggle_launcher" | "toggle_search" => "Launcher e busca",
        "lock_screen" | "quit" | "launch:mide-screenshot" | "next_layout" => "Sistema",
        a if a.starts_with("launch:") => "Apps",
        _ => "Janelas",
    }
}

/// Combinação que não serve para atalho global: sem Ctrl, Alt ou Super ela tomaria
/// uma tecla usada para digitar. Teclas de função (F1 a F24) podem ficar sozinhas.
fn combo_problem(combo: &str) -> Option<std::borrow::Cow<'static, str>> {
    let parts: Vec<&str> = combo.split('+').collect();
    let key = parts.last().copied().unwrap_or("");
    let function_key = key.strip_prefix('f').is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()));
    let held = parts.iter().any(|p| matches!(*p, "ctrl" | "alt" | "super"));
    (!held && !function_key).then(|| tr!("Use Ctrl, Alt or Super together with the key, so it does not take over a typing key."))
}

/// Comando que abre o app (apps de terminal abrem no terminal).
fn app_command(app: &mide::apps::App) -> String {
    if app.terminal {
        format!("mide-terminal -e {}", app.command)
    } else {
        app.command.clone()
    }
}

/// Ação de um atalho, no idioma da sessão. Atalhos que abrem um app mostram o nome dele.
fn describe_action(action: &str, apps: &[mide::apps::App]) -> String {
    let text = match action {
        "close_window" => tr!("Close window"),
        "quit" => tr!("Log out of mide"),
        "restore_minimized" => tr!("Restore the last minimized window"),
        "toggle_launcher" => tr!("Show or hide the launcher"),
        "toggle_search" => tr!("Show or hide search"),
        "next_layout" => tr!("Next keyboard layout"),
        "switch_windows" => tr!("Switch windows"),
        "switch_windows_back" => tr!("Switch windows (backwards)"),
        "maximize" => tr!("Maximize window"),
        "unmaximize" => tr!("Restore window size"),
        "toggle_maximize" => tr!("Maximize or restore window"),
        "minimize" => tr!("Minimize window"),
        "tile_left" => tr!("Window on the left half"),
        "tile_right" => tr!("Window on the right half"),
        "toggle_fullscreen" => tr!("Enter or exit full screen"),
        "show_desktop" => tr!("Show desktop"),
        "overview" => tr!("Show all windows"),
        "lock_screen" => tr!("Lock screen"),
        "launch:mide-screenshot" => tr!("Screenshot"),
        "launch:mide-terminal --hotkey" => tr!("Show or hide the hotkey terminal"),
        other => {
            return match other.strip_prefix("launch:") {
                Some(cmd) => match apps.iter().find(|a| a.command == cmd || app_command(a) == cmd) {
                    Some(app) => tr!("Open {app}", app = app.name),
                    None => tr!("Open: {command}", command = cmd),
                },
                None => other.into(),
            };
        }
    };
    text.into_owned()
}

fn main() -> Result<(), Box<dyn Error>> {
    let _logging = mide::logging::init("mide-settings", "mide_settings=info");
    let section = match std::env::args().nth(1).as_deref() {
        Some("bluetooth") => Section::Bluetooth,
        Some("appearance") => Section::Appearance,
        Some("sound") => Section::Sound,
        Some("displays") => Section::Displays,
        Some("wallpaper") => Section::Wallpaper,
        Some("dock") => Section::Dock,
        Some("bar") => Section::Bar,
        Some("effects") => Section::Effects,
        Some("lock") => Section::Lock,
        Some("users") => Section::Users,
        Some("mouse") => Section::Mouse,
        Some("keyboard") => Section::Keyboard,
        Some("region") => Section::Region,
        Some("energy") => Section::Energy,
        Some("default-apps") => Section::DefaultApps,
        Some("login-items") => Section::LoginItems,
        Some("about") => Section::About,
        Some("network") => Section::Network,
        _ => Section::Wifi,
    };
    tracing::info!("configurações abertas: {section:?}");
    mide::i18n::init();
    let window = Window::new(&tr!("System Settings")).app_id("mide-settings").size(820, 560).min_size(640, 440).full_size_content();
    run(window, Settings::new(section))
}

/// Há sensor de orientação (o compositor diz)? Perguntado no máximo a cada 5 s: a tela
/// das Configurações é redesenhada a cada quadro.
fn orientation_sensor() -> bool {
    static CACHE: std::sync::Mutex<Option<(std::time::Instant, bool)>> = std::sync::Mutex::new(None);
    let Ok(mut cache) = CACHE.lock() else {
        return false;
    };
    if cache.is_none_or(|(at, _)| at.elapsed() > std::time::Duration::from_secs(5)) {
        let sensor = ask_compositor("orientation_sensor").is_some_and(|r| r.first().is_some_and(|l| l.trim() == "yes"));
        *cache = Some((std::time::Instant::now(), sensor));
    }
    cache.is_some_and(|(_, s)| s)
}

// ------------------------------------------------------------ Usuários e grupos

/// Pasta da tela de login do mide instalada pelo `data/mide-login-install`.
const LOGIN_LIB: &str = "/usr/local/lib/mide-login";
/// Foto pequena (lista) e grande (folha do usuário), em pontos.
const AVATAR_SMALL: f32 = 32.0;
const AVATAR_LARGE: f32 = 72.0;

/// O que as threads de Usuários e grupos mandam de volta.
enum UsersMsg {
    List(Result<Vec<mide::accounts::Account>, mide::accounts::Error>),
    /// Uma mudança terminou: `Ok` ou o texto do erro (já traduzido).
    Done(Result<(), String>),
    /// O `--sync` da tela de login terminou: `Ok` ou o erro.
    Synced(Result<(), String>),
}

/// Usuários e grupos (contas do AccountsService) e as opções da tela de início de sessão.
struct Users {
    tx: Sender<UsersMsg>,
    rx: Receiver<UsersMsg>,
    /// Pedido de leitura feito (a lista vem numa thread).
    requested: bool,
    accounts: Option<Result<Vec<mide::accounts::Account>, mide::accounts::Error>>,
    /// Uma mudança esperando o AccountsService (e talvez a senha de administrador).
    busy: bool,
    /// Resultado da última mudança: (erro?, texto).
    message: Option<(bool, String)>,
    avatars: HashMap<(PathBuf, u32), Option<Pixmap>>,
    sheet: Option<UserSheet>,
    add: Option<AddUser>,
    /// Confirmação de apagar: a conta e "apagar também a pasta pessoal".
    delete: Option<(mide::accounts::Account, bool)>,
    /// Mensagem da tela de início de sessão sendo escrita, e o interruptor dela.
    login_message: String,
    login_message_on: bool,
    syncing: bool,
    sync_result: Option<(bool, String)>,
}

/// Folha de um usuário (como o ⓘ do Mac): foto, nome, dica e tipo de conta.
struct UserSheet {
    account: mide::accounts::Account,
    name: String,
    hint: String,
    admin: bool,
    /// Foto nova escolhida (vale ao clicar em OK).
    icon: Option<PathBuf>,
    chooser: Option<mide::kit::FileChooser>,
}

/// Folha de adicionar um usuário.
#[derive(Default)]
struct AddUser {
    admin: bool,
    name: String,
    login: String,
    /// O nome da conta foi mudado à mão (deixa de seguir o nome completo).
    login_edited: bool,
    first_login: bool,
    password: String,
    verify: String,
    hint: String,
    /// Campo de senha com o teclado (0 = senha, 1 = confirmar); `None`, os de texto.
    secret_focus: Option<usize>,
    error: Option<String>,
    focused: bool,
}

impl Drop for AddUser {
    fn drop(&mut self) {
        mide::elevate::wipe(&mut self.password);
        mide::elevate::wipe(&mut self.verify);
    }
}

impl Users {
    fn new() -> Self {
        let (tx, rx) = channel();
        let message = match settings_file::get("login", "message") {
            Some(Value::Text(t)) => t,
            _ => String::new(),
        };
        Self {
            tx,
            rx,
            requested: false,
            accounts: None,
            busy: false,
            message: None,
            avatars: HashMap::new(),
            sheet: None,
            add: None,
            delete: None,
            login_message_on: !message.is_empty(),
            login_message: message,
            syncing: false,
            sync_result: None,
        }
    }

    /// Lê a lista de novo, numa thread.
    fn reload(&mut self) {
        self.requested = true;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(UsersMsg::List(mide::accounts::list()));
        });
    }

    /// Roda uma mudança numa thread e relê a lista no fim.
    fn run(&mut self, what: &'static str, job: impl FnOnce() -> Result<(), mide::accounts::Error> + Send + 'static) {
        self.busy = true;
        self.message = None;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let result = job();
            match &result {
                Ok(()) => tracing::info!("usuários: {what}"),
                Err(err) => tracing::warn!("usuários: {what} falhou: {err}"),
            }
            let _ = tx.send(UsersMsg::Done(result.map_err(|e| account_error_text(&e))));
            let _ = tx.send(UsersMsg::List(mide::accounts::list()));
        });
    }

    fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok(msg) = self.rx.try_recv() {
            changed = true;
            match msg {
                UsersMsg::List(list) => {
                    // Fotos podem ter mudado no mesmo arquivo.
                    self.avatars.clear();
                    self.accounts = Some(list);
                }
                UsersMsg::Done(result) => {
                    self.busy = false;
                    self.message = result.err().map(|e| (true, e));
                }
                UsersMsg::Synced(result) => {
                    self.syncing = false;
                    self.sync_result = Some(match result {
                        Ok(()) => (false, tr!("The login window now uses these settings.").into_owned()),
                        Err(e) => (true, e),
                    });
                }
            }
        }
        changed
    }

    /// Foto redonda de `path` em `px` pixels (guardada).
    fn avatar(&mut self, path: &std::path::Path, px: u32) -> Option<Pixmap> {
        self.avatars
            .entry((path.to_path_buf(), px))
            .or_insert_with(|| {
                let img = image::open(path).ok()?.to_rgba8();
                let (w, h) = img.dimensions();
                let side = w.min(h);
                let square = image::imageops::crop_imm(&img, (w - side) / 2, (h - side) / 2, side, side).to_image();
                let small = image::imageops::resize(&square, px, px, image::imageops::FilterType::Triangle);
                Some(to_pixmap(&small))
            })
            .clone()
    }
}

/// Erro de uma mudança nas contas, para a tela.
fn account_error_text(err: &mide::accounts::Error) -> String {
    match err {
        mide::accounts::Error::Denied => tr!("Not changed: the administrator password was not given.").into_owned(),
        mide::accounts::Error::Unavailable(_) => tr!("Not changed: the accounts service (AccountsService) is not running.").into_owned(),
        mide::accounts::Error::Failed(e) => tr!("Not changed: {error}", error = e),
    }
}

/// Iniciais do nome ("Ana Souza" → "AS"), para quem não tem foto.
fn initials(name: &str) -> String {
    let letters: Vec<char> = name.split_whitespace().filter_map(|w| w.chars().next()).filter(|c| c.is_alphanumeric()).collect();
    let picked: Vec<char> = match letters.len() {
        0 => vec!['?'],
        1 => vec![letters[0]],
        n => vec![letters[0], letters[n - 1]],
    };
    picked.into_iter().flat_map(char::to_uppercase).collect()
}

/// Desenha a foto (ou as iniciais num círculo cinza, como no Mac) de `size` pontos com o
/// canto em (x, y) no canvas.
fn paint_avatar(pixmap: &mut Pixmap, text: &mut mide::ui::text::Text, photo: Option<&Pixmap>, name: &str, x: f32, y: f32, size: f32) {
    let k = mide::ui::draw::scale();
    match photo {
        Some(photo) => {
            let shader = tiny_skia::Pattern::new(photo.as_ref(), tiny_skia::SpreadMode::Pad, tiny_skia::FilterQuality::Bilinear, 1.0, tiny_skia::Transform::from_translate((x * k).round(), (y * k).round()));
            let paint = tiny_skia::Paint { shader, anti_alias: true, ..Default::default() };
            if let Some(circle) = tiny_skia::PathBuilder::from_circle((x + size / 2.0) * k, (y + size / 2.0) * k, size * k / 2.0) {
                pixmap.fill_path(&circle, &paint, tiny_skia::FillRule::Winding, tiny_skia::Transform::identity(), None);
            }
        }
        None => {
            // Cinza em degradê suave, como as iniciais dos Contatos.
            let paint = tiny_skia::Paint {
                shader: mide::ui::draw::vertical_gradient(y, y + size, mide::ui::draw::rgba(166, 166, 172, 255), mide::ui::draw::rgba(122, 122, 128, 255)),
                anti_alias: true,
                ..Default::default()
            };
            if let Some(circle) = tiny_skia::PathBuilder::from_circle(x + size / 2.0, y + size / 2.0, size / 2.0) {
                pixmap.fill_path(&circle, &paint, tiny_skia::FillRule::Winding, mide::ui::draw::tf(), None);
            }
            let letters = initials(name);
            let font = (size * 0.4).round();
            let w = text.measure(&letters, font, mide::ui::text::FontWeight::Semibold);
            text.draw(pixmap, &letters, x + (size - w) / 2.0, y + (size - font * 1.25) / 2.0, font, mide::ui::text::FontWeight::Semibold, [255, 255, 255, 255]);
        }
    }
}

/// Campo de senha que só recebe o teclado quando `active` (o `password_field` do kit
/// pega todas as teclas): inativo, é desenhado igual, com as bolinhas, e o clique nele
/// responde `true` para o app passar o teclado a ele.
fn secret_field(ui: &mut Ui, value: &mut String, placeholder: &str, active: bool) -> bool {
    if active {
        ui.password_field(value, placeholder, false);
        return false;
    }
    let t = ui.theme().clone();
    let mut rect = (0.0, 0.0, 0.0, 0.0);
    ui.custom(t.row, |_, r| rect = r);
    let clicked = ui.over(rect) && ui.input().pressed;
    let dots = "•".repeat(value.chars().count().min(40));
    ui.paint_text(|pixmap, text, _| {
        let c = |v: [u8; 4]| tiny_skia::Color::from_rgba8(v[0], v[1], v[2], v[3]);
        mide::ui::draw::fill_rounded(pixmap, rect.0, rect.1, rect.2, rect.3, t.control_radius, c(t.card));
        if let Some(path) = mide::ui::draw::rounded_rect(rect.0 + 0.5, rect.1 + 0.5, rect.2 - 1.0, rect.3 - 1.0, t.control_radius) {
            let mut paint = tiny_skia::Paint { anti_alias: true, ..Default::default() };
            paint.set_color(c(t.line));
            pixmap.stroke_path(&path, &paint, &tiny_skia::Stroke { width: 1.0, ..Default::default() }, mide::ui::draw::tf(), None);
        }
        let ty = rect.1 + (rect.3 - t.font * 1.25) / 2.0;
        if dots.is_empty() {
            text.draw(pixmap, placeholder, rect.0 + 10.0, ty, t.font, mide::ui::text::FontWeight::Regular, t.text_secondary);
        } else {
            text.draw(pixmap, &dots, rect.0 + 10.0, ty - 1.0, t.font + 2.0, mide::ui::text::FontWeight::Regular, t.text);
        }
    });
    clicked
}

impl Settings {
    /// Usuários e grupos: a lista das contas e as opções da tela de início de sessão,
    /// como nos Ajustes do Sistema do Mac.
    fn users(&mut self, ui: &mut Ui) {
        ui.title(&tr!("Users & Groups"));
        if self.users.poll() {
            ui.request_repaint();
        }
        if !self.users.requested {
            self.users.reload();
        }
        if self.users.busy || self.users.syncing || self.users.accounts.is_none() {
            // A resposta chega por canal: confere de novo daqui a pouco.
            ui.wake_after(Duration::from_millis(150));
        }
        let accounts = match &self.users.accounts {
            None => {
                ui.note(&tr!("Reading the users…"));
                return;
            }
            Some(Err(mide::accounts::Error::Unavailable(_))) => {
                ui.card(|ui| {
                    ui.strong(&tr!("Users can't be managed here"));
                    ui.note(&tr!("The system service that manages accounts (AccountsService) is not running. Install the “accountsservice” package with your distribution's package manager and open Settings again."));
                });
                Vec::new()
            }
            Some(Err(err)) => {
                ui.error_note(&tr!("Could not read the users: {error}", error = err));
                Vec::new()
            }
            Some(Ok(list)) => list.clone(),
        };
        if let Some((error, text)) = self.users.message.clone() {
            if error {
                ui.error_note(&text);
            } else {
                ui.note(&text);
            }
        }
        let blocked = self.users.sheet.is_some() || self.users.add.is_some() || self.users.delete.is_some();
        let mut open = None;
        if !accounts.is_empty() {
            ui.card(|ui| {
                for (i, account) in accounts.iter().enumerate() {
                    if self.user_row(ui, account, i > 0, blocked) {
                        open = Some(account.clone());
                    }
                }
            });
        }
        let mut add = false;
        ui.row(|ui| {
            if self.users.busy {
                ui.note(&tr!("Waiting for the administrator password…"));
            }
            ui.push_right(200.0);
            ui.sized(200.0, |ui| add = ui.button(&tr!("Add User…")).clicked() && !self.users.busy);
        });
        if let Some(account) = open.filter(|_| !self.users.busy) {
            tracing::info!("usuários: folha de {}", account.login);
            self.users.sheet = Some(UserSheet { name: account.real_name.clone(), hint: account.hint.clone(), admin: account.admin, icon: None, chooser: None, account });
            ui.request_repaint();
        }
        if add {
            self.users.add = Some(AddUser::default());
            ui.request_repaint();
        }
        self.login_window(ui, &accounts);
    }

    /// Uma linha da lista: foto, nome e o tipo de conta embaixo. Devolve o clique.
    fn user_row(&mut self, ui: &mut Ui, account: &mide::accounts::Account, line: bool, blocked: bool) -> bool {
        let t = ui.theme().clone();
        let h = 44.0;
        let mut rect = (0.0, 0.0, 0.0, 0.0);
        ui.custom(h, |_, r| rect = r);
        let hovered = !blocked && ui.over(rect);
        let clicked = hovered && ui.input().released;
        let k = mide::ui::draw::scale();
        let photo = account.icon.clone().and_then(|p| self.users.avatar(&p, (AVATAR_SMALL * k).round() as u32));
        let name = account.display_name().to_string();
        let mut detail = if account.admin { tr!("Admin").into_owned() } else { tr!("Standard").into_owned() };
        if account.current {
            detail = tr!("{kind} · You", kind = detail);
        }
        let spacing = t.spacing;
        ui.paint_text(|pixmap, text, _| {
            let c = |v: [u8; 4]| tiny_skia::Color::from_rgba8(v[0], v[1], v[2], v[3]);
            if hovered {
                // A linha inteira, de borda a borda do cartão (o cartão recorta os cantos).
                mide::ui::draw::fill_rounded(pixmap, rect.0 - 16.0, rect.1 - spacing / 2.0, rect.2 + 32.0, rect.3 + spacing, 0.0, c(t.control));
            }
            if line {
                // Divisória recuada até o texto, como nas listas do Tahoe.
                let x = rect.0 + AVATAR_SMALL + 12.0;
                mide::ui::draw::fill_rounded(pixmap, x, rect.1 - spacing / 2.0 - 0.5, rect.0 + rect.2 - x, 1.0, 0.0, c(t.line));
            }
            paint_avatar(pixmap, text, photo.as_ref(), &name, rect.0, rect.1 + (h - AVATAR_SMALL) / 2.0, AVATAR_SMALL);
            let x = rect.0 + AVATAR_SMALL + 12.0;
            let block = t.font * 1.25 + t.font_small * 1.25;
            let y = rect.1 + (h - block) / 2.0;
            text.draw(pixmap, &name, x, y, t.font, mide::ui::text::FontWeight::Regular, t.text);
            text.draw(pixmap, &detail, x, y + t.font * 1.25, t.font_small, mide::ui::text::FontWeight::Regular, t.text_secondary);
            // Setinha de "abrir" à direita.
            text.draw(pixmap, "›", rect.0 + rect.2 - 10.0, rect.1 + (h - t.font * 1.6) / 2.0, t.font * 1.3, mide::ui::text::FontWeight::Regular, t.text_secondary);
        });
        clicked
    }

    /// Opções da tela de início de sessão (`[login]`, lidas pelo `mide-login`).
    fn login_window(&mut self, ui: &mut Ui, accounts: &[mide::accounts::Account]) {
        ui.space(8.0);
        ui.heading(&tr!("Login Window"));
        let save = |key: &str, value: Option<Value>| match settings_file::set("login", key, value) {
            Ok(()) => tracing::info!("tela de início de sessão: {key} mudado"),
            Err(err) => tracing::warn!("opção da tela de início de sessão não gravada: {err}"),
        };
        let show_name = matches!(settings_file::get("login", "show"), Some(Value::Text(t)) if t == "name");
        let mut show = usize::from(show_name);
        let mut power = settings_file::flag("login", "power_buttons", true);
        let mut hints = settings_file::flag("login", "password_hints", true);
        let mut input_menu = settings_file::flag("login", "input_menu", true);
        let auto = match settings_file::get("login", "autologin") {
            Some(Value::Text(t)) => t,
            _ => String::new(),
        };
        let mut auto_options = vec![tr!("Off").into_owned()];
        auto_options.extend(accounts.iter().map(|a| a.display_name().to_string()));
        // Usuário gravado que não está na lista (a lista não leu): aparece pelo nome da conta.
        let mut auto_index = accounts.iter().position(|a| a.login == auto).map(|i| i + 1).unwrap_or(0);
        if auto_index == 0 && !auto.is_empty() {
            auto_options.push(auto.clone());
            auto_index = auto_options.len() - 1;
        }
        let shows = [tr!("List of users").into_owned(), tr!("Name and password").into_owned()];
        let mut message_on = self.users.login_message_on;
        ui.card(|ui| {
            if ui.dropdown(&tr!("Automatically log in as"), &auto_options, &mut auto_index).changed() {
                let login = match auto_index {
                    0 => None,
                    i => accounts.get(i - 1).map(|a| a.login.clone()).or_else(|| Some(auto.clone())),
                };
                save("autologin", login.filter(|l| !l.is_empty()).map(Value::Text));
            }
            if ui.dropdown(&tr!("Display login window as"), &shows, &mut show).changed() {
                save("show", (show == 1).then(|| Value::Text("name".into())));
            }
            if ui.switch_detail(&tr!("Show the Sleep, Restart and Shut Down buttons"), "", &mut power).changed() {
                save("power_buttons", (!power).then_some(Value::Bool(false)));
            }
            if ui.switch_detail(&tr!("Show password hints"), &tr!("After three wrong passwords"), &mut hints).changed() {
                save("password_hints", (!hints).then_some(Value::Bool(false)));
            }
            if ui.switch_detail(&tr!("Show the input menu in the login window"), &tr!("To choose the keyboard layout before typing the password"), &mut input_menu).changed() {
                save("input_menu", (!input_menu).then_some(Value::Bool(false)));
            }
            if ui.switch_detail(&tr!("Show message when locked"), &tr!("On the login window and the lock screen"), &mut message_on).changed() && !message_on {
                self.users.login_message.clear();
                save("message", None);
            }
            if message_on && ui.text_input(&mut self.users.login_message, &tr!("Message")).changed() {
                let text = self.users.login_message.trim().to_string();
                save("message", (!text.is_empty()).then_some(Value::Text(text)));
            }
        });
        self.users.login_message_on = message_on;
        // Levar as opções à tela de login de verdade (que roda como outro usuário).
        let lib = login_lib();
        if lib.join("mide-login").exists() {
            let installer = lib.join("mide-login-install");
            if installer.exists() {
                ui.row(|ui| {
                    if self.users.syncing {
                        ui.note(&tr!("Applying…"));
                    }
                    ui.push_right(300.0);
                    ui.sized(300.0, |ui| {
                        if ui.button(&tr!("Apply to Login Window…")).clicked() && !self.users.syncing {
                            self.sync_login_window(installer.clone(), accounts);
                        }
                    });
                });
                match &self.users.sync_result {
                    Some((true, text)) => ui.error_note(text),
                    Some((false, text)) => ui.note(text),
                    None => {}
                }
                ui.note(&tr!("The login window runs before anyone logs in, so it keeps its own copy of these options, the wallpaper and the displays. Apply copies yours to it (asks for the administrator password)."));
            } else {
                ui.note(&tr!("To apply these options, update the mide login window: in the mide project folder, run “sudo data/mide-login-install” once."));
            }
        } else {
            ui.note(&tr!("These options are for the mide login window, which is not installed. To install it, run “sudo data/mide-login-install” in the mide project folder."));
        }
    }

    /// Roda o `--sync` do instalador da tela de login pelo `pkexec`, numa thread.
    fn sync_login_window(&mut self, installer: PathBuf, accounts: &[mide::accounts::Account]) {
        let me = accounts.iter().find(|a| a.current).map(|a| a.login.clone()).or_else(|| std::env::var("USER").ok()).unwrap_or_default();
        self.users.syncing = true;
        self.users.sync_result = None;
        let tx = self.users.tx.clone();
        tracing::info!("tela de início de sessão: aplicando ({} --sync {me})", installer.display());
        std::thread::spawn(move || {
            let result = if std::env::var_os("MIDE_SYSTEM_DRY_RUN").is_some() {
                tracing::info!("sistema (teste): pkexec {} --sync {me}", installer.display());
                Ok(())
            } else {
                match std::process::Command::new("pkexec").arg(&installer).arg("--sync").arg(&me).output() {
                    Ok(out) if out.status.success() => Ok(()),
                    // 126: o polkit não autorizou (senha cancelada ou errada).
                    Ok(out) if out.status.code() == Some(126) || out.status.code() == Some(127) => Err(tr!("Not applied: the administrator password was not given.").into_owned()),
                    Ok(out) => {
                        let err = String::from_utf8_lossy(&out.stderr);
                        let line = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim_start_matches("erro: ").to_string();
                        tracing::warn!("tela de início de sessão: --sync falhou: {err}");
                        Err(tr!("Not applied: {error}", error = line))
                    }
                    Err(err) => Err(tr!("Not applied: {error}", error = err)),
                }
            };
            let _ = tx.send(UsersMsg::Synced(result));
        });
    }

    /// Folha de um usuário, por cima da janela inteira.
    fn user_sheet(&mut self, ui: &mut Ui) {
        let Some(mut sheet) = self.users.sheet.take() else {
            return;
        };
        if let Some(chooser) = sheet.chooser.as_mut() {
            match chooser.show(ui) {
                Some(mide::kit::Choice::Chosen(path)) => {
                    sheet.icon = Some(path);
                    sheet.chooser = None;
                }
                Some(mide::kit::Choice::Cancelled) => sheet.chooser = None,
                None => {}
            }
            ui.request_repaint();
            self.users.sheet = Some(sheet);
            return;
        }
        let k = mide::ui::draw::scale();
        let px = (AVATAR_LARGE * k).round() as u32;
        let photo = sheet.icon.clone().or_else(|| sheet.account.icon.clone()).and_then(|p| self.users.avatar(&p, px));
        let mut close = ui.input().key(Key::Escape);
        let (mut ok, mut change_picture, mut delete) = (false, false, false);
        let display = if sheet.name.trim().is_empty() { sheet.account.login.clone() } else { sheet.name.clone() };
        let others = !sheet.account.current;
        ui.modal(460.0, |ui| {
            let mut rect = (0.0, 0.0, 0.0, 0.0);
            ui.custom(AVATAR_LARGE, |_, r| rect = r);
            ui.paint_text(|pixmap, text, _| paint_avatar(pixmap, text, photo.as_ref(), &display, rect.0 + (rect.2 - AVATAR_LARGE) / 2.0, rect.1, AVATAR_LARGE));
            ui.row(|ui| {
                let w = 170.0;
                ui.space_x(((ui.remaining_width() - w) / 2.0).max(0.0));
                ui.sized(w, |ui| change_picture = ui.button(&tr!("Change Picture…")).clicked());
            });
            const LABEL: f32 = 130.0;
            ui.form_row(&tr!("Full name"), LABEL, |ui| {
                ui.text_input(&mut sheet.name, &tr!("Full name"));
            });
            ui.form_row(&tr!("Account name"), LABEL, |ui| ui.label(&sheet.account.login));
            ui.form_row(&tr!("Password hint"), LABEL, |ui| {
                ui.text_input(&mut sheet.hint, &tr!("Optional"));
            });
            if others {
                let options = [tr!("Standard").into_owned(), tr!("Administrator").into_owned()];
                let mut index = usize::from(sheet.admin);
                if ui.dropdown(&tr!("Account type"), &options, &mut index).changed() {
                    sheet.admin = index == 1;
                }
            } else {
                ui.note(&tr!("To change your own password, run “passwd” in Terminal."));
            }
            ui.space(4.0);
            ui.row(|ui| {
                if others {
                    ui.sized(150.0, |ui| delete = ui.button_styled(&tr!("Delete User…"), mide::kit::ButtonStyle::Destructive).clicked());
                }
                let w = ui.remaining_width();
                ui.sized(w, |ui| match ui.dialog_buttons(&[&tr!("Cancel"), &tr!("OK")]) {
                    Some(0) => close = true,
                    Some(_) => ok = true,
                    None => {}
                });
            });
        });
        if change_picture {
            let images = mide::kit::Filter { name: tr!("Images").into_owned(), patterns: vec![mide::kit::Pattern::Mime("image/*".into())] };
            let pictures = std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Pictures"));
            sheet.chooser = Some(mide::kit::FileChooser::open(&tr!("Choose a Picture"), pictures.as_deref().filter(|p| p.is_dir())).filters(vec![images], 0));
            self.users.sheet = Some(sheet);
            return;
        }
        if delete {
            self.users.delete = Some((sheet.account.clone(), false));
            return;
        }
        if ok {
            let a = sheet.account.clone();
            let (name, hint, admin, icon) = (sheet.name.trim().to_string(), sheet.hint.trim().to_string(), sheet.admin, sheet.icon.clone());
            let changed = name != a.real_name || hint != a.hint || (others && admin != a.admin) || icon.is_some();
            if changed {
                self.users.run("conta mudada", move || {
                    if let Some(icon) = &icon {
                        mide::accounts::set_icon(&a, icon)?;
                    }
                    if name != a.real_name {
                        mide::accounts::set_real_name(&a, &name)?;
                    }
                    if hint != a.hint {
                        mide::accounts::set_password_hint(&a, &hint)?;
                    }
                    if others && admin != a.admin {
                        mide::accounts::set_admin(&a, admin)?;
                    }
                    Ok(())
                });
            }
            ui.blur();
            ui.request_repaint();
            return;
        }
        if close {
            ui.blur();
            ui.request_repaint();
            return;
        }
        self.users.sheet = Some(sheet);
    }

    /// Confirmação de apagar um usuário.
    fn delete_user_dialog(&mut self, ui: &mut Ui) {
        let Some((account, mut remove_home)) = self.users.delete.take() else {
            return;
        };
        let mut choice = None;
        ui.modal(440.0, |ui| {
            ui.strong(&tr!("Delete the user “{name}”?", name = account.display_name()));
            ui.note(&tr!("The account is removed from this computer. This can't be undone."));
            ui.checkbox(&tr!("Also delete the home folder ({folder})", folder = format!("/home/{}", account.login)), &mut remove_home);
            choice = ui.dialog_buttons(&[&tr!("Cancel"), &tr!("Delete User")]);
        });
        if choice.is_none() && ui.input().key(Key::Escape) {
            choice = Some(0);
        }
        match choice {
            Some(1) => {
                let a = account.clone();
                self.users.run("usuário apagado", move || mide::accounts::delete(&a, remove_home));
                ui.request_repaint();
            }
            Some(_) => ui.request_repaint(),
            None => self.users.delete = Some((account, remove_home)),
        }
    }

    /// Folha de adicionar um usuário.
    fn add_user_dialog(&mut self, ui: &mut Ui) {
        let Some(mut add) = self.users.add.take() else {
            return;
        };
        if !std::mem::replace(&mut add.focused, true) {
            ui.focus_next_input();
        }
        let mut choice = None;
        ui.modal(480.0, |ui| {
            ui.strong(&tr!("New User"));
            const LABEL: f32 = 150.0;
            ui.form_row(&tr!("Account type"), LABEL, |ui| {
                let options = [tr!("Standard").into_owned(), tr!("Administrator").into_owned()];
                let refs: Vec<&str> = options.iter().map(String::as_str).collect();
                let mut index = usize::from(add.admin);
                if ui.segmented(&refs, &mut index, Some(280.0)).changed() {
                    add.admin = index == 1;
                }
            });
            ui.form_row(&tr!("Full name"), LABEL, |ui| {
                let r = ui.text_input(&mut add.name, &tr!("Full name"));
                if r.clicked() {
                    add.secret_focus = None;
                }
                if r.changed() && !add.login_edited {
                    add.login = mide::accounts::suggest_login(&add.name);
                }
            });
            ui.form_row(&tr!("Account name"), LABEL, |ui| {
                let r = ui.text_input(&mut add.login, &tr!("Account name"));
                if r.clicked() {
                    add.secret_focus = None;
                }
                if r.changed() {
                    add.login_edited = true;
                }
            });
            ui.checkbox(&tr!("The user chooses a password at first login"), &mut add.first_login);
            if !add.first_login {
                ui.form_row(&tr!("Password"), LABEL, |ui| {
                    if secret_field(ui, &mut add.password, &tr!("Required"), add.secret_focus == Some(0)) {
                        add.secret_focus = Some(0);
                    }
                });
                ui.form_row(&tr!("Verify"), LABEL, |ui| {
                    if secret_field(ui, &mut add.verify, &tr!("Verify"), add.secret_focus == Some(1)) {
                        add.secret_focus = Some(1);
                    }
                });
            }
            ui.form_row(&tr!("Password hint"), LABEL, |ui| {
                let r = ui.text_input(&mut add.hint, &tr!("Optional"));
                if r.clicked() {
                    add.secret_focus = None;
                }
            });
            if let Some(err) = &add.error {
                ui.error_note(err);
            }
            choice = ui.dialog_buttons(&[&tr!("Cancel"), &tr!("Create User")]);
        });
        // O Esc de um campo de senha só apaga o que foi digitado nele.
        if choice.is_none() && add.secret_focus.is_none() && ui.input().key(Key::Escape) {
            choice = Some(0);
        }
        match choice {
            Some(1) => {
                let login = add.login.trim().to_string();
                add.error = if add.name.trim().is_empty() {
                    Some(tr!("Type the full name.").into_owned())
                } else if !mide::accounts::valid_login(&login) {
                    Some(tr!("The account name must start with a lowercase letter and have only lowercase letters, digits, “-” and “_”.").into_owned())
                } else if self.users.accounts.as_ref().and_then(|r| r.as_ref().ok()).is_some_and(|l| l.iter().any(|a| a.login == login)) {
                    Some(tr!("There is already a user named “{name}”.", name = login))
                } else if !add.first_login && add.password.is_empty() {
                    Some(tr!("Type a password, or let the user choose one at first login.").into_owned())
                } else if !add.first_login && add.password != add.verify {
                    Some(tr!("The passwords don't match.").into_owned())
                } else {
                    None
                };
                if add.error.is_some() {
                    self.users.add = Some(add);
                    ui.request_repaint();
                    return;
                }
                let (name, admin, hint, first_login) = (add.name.trim().to_string(), add.admin, add.hint.trim().to_string(), add.first_login);
                let mut password = std::mem::take(&mut add.password);
                self.users.run("usuário criado", move || {
                    let account = mide::accounts::create(&login, &name, admin)?;
                    let result = if first_login { mide::accounts::password_at_first_login(&account) } else { mide::accounts::set_password(&account, &mut password, &hint) };
                    mide::elevate::wipe(&mut password);
                    result?;
                    if first_login && !hint.is_empty() {
                        mide::accounts::set_password_hint(&account, &hint)?;
                    }
                    Ok(())
                });
                ui.blur();
                ui.request_repaint();
            }
            Some(_) => {
                ui.blur();
                ui.request_repaint();
            }
            None => self.users.add = Some(add),
        }
    }
}

/// Pasta da tela de login instalada (`MIDE_LOGIN_LIB` troca, nos testes de
/// desenvolvimento).
fn login_lib() -> PathBuf {
    if cfg!(debug_assertions)
        && let Some(dir) = std::env::var_os("MIDE_LOGIN_LIB")
    {
        return PathBuf::from(dir);
    }
    PathBuf::from(LOGIN_LIB)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Os testes conferem os textos em pt-BR (todos pedem o mesmo idioma, então rodar
    /// em paralelo não atrapalha).
    fn pt_br() {
        mide::i18n::init();
        mide::i18n::set_language("pt_BR");
    }

    #[test]
    fn atalhos_em_portugues() {
        pt_br();
        assert_eq!(pretty_combo("ctrl+shift+space", false), "Ctrl ⇧ Espaço");
        assert_eq!(pretty_combo("ctrl+shift+t", false), "Ctrl ⇧ T");
        assert_eq!(pretty_combo("ctrl+space", true), "⌘ Espaço");
        // Estilo Mac: as capturas são ⇧⌘3/4/5 (o ⌘ é o Super dos atalhos do mide).
        assert_eq!(pretty_combo("super+shift+4", true), "⇧ ⌘ 4");
        assert_eq!(pretty_combo("super+shift+4", false), "Super ⇧ 4");
        assert_eq!(pretty_combo("ctrl+super+q", true), "⌃ ⌘ Q");
        assert_eq!(pretty_combo("ctrl+shift+space", true), "⇧ ⌘ Espaço");
        assert_eq!(describe_action("toggle_launcher", &[]), "Abrir ou fechar o launcher");
        assert_eq!(describe_action("toggle_search", &[]), "Abrir ou fechar o buscador");
        assert_eq!(describe_action("launch:foot", &[]), "Abrir: foot");
    }

    #[test]
    fn atalho_de_app_mostra_o_nome() {
        pt_br();
        let app = mide::apps::App {
            id: "org.codeberg.dnkl.foot".into(),
            name: "Foot".into(),
            command: "foot".into(),
            icon: None,
            group: "Utilitários",
            terminal: false,
            keywords: String::new(),
            mime: Vec::new(),
        };
        assert_eq!(describe_action("launch:foot", &[app]), "Abrir Foot");
    }

    #[test]
    fn combinacao_precisa_de_modificador() {
        assert!(combo_problem("ctrl+shift+t").is_none());
        assert!(combo_problem("super+e").is_none());
        assert!(combo_problem("alt+f4").is_none());
        assert!(combo_problem("f5").is_none());
        assert!(combo_problem("a").is_some());
        assert!(combo_problem("shift+a").is_some());
        assert!(combo_problem("space").is_some());
    }

    #[test]
    fn busca_acha_opcoes_por_palavra_chave() {
        pt_br();
        let hits = |q: &str| -> Vec<&str> {
            let q = mide::apps::fold(q);
            SEARCHABLE.iter().filter(|(n, k, _)| search_matches(&q, n, k)).map(|(n, ..)| *n).collect()
        };
        assert_eq!(hits("volume"), vec!["Output volume"]);
        // Pelo nome traduzido e pelas palavras-chave das duas línguas.
        assert_eq!(hits("teclas")[0], "Mac-style keys");
        assert!(hits("acceleration").contains(&"Mouse acceleration"));
        assert!(hits("aceleracao").contains(&"Mouse acceleration"));
    }

    /// Os textos das tabelas (barra lateral, efeitos, busca, grupos, layouts) passam pelo
    /// `tr` na hora de aparecer, longe de um `tr!("…")` que o teste de `mide::i18n`
    /// enxergue: este confere que todos têm tradução.
    #[test]
    fn tabelas_tem_traducao() {
        pt_br();
        let mut texts: Vec<&str> = Vec::new();
        texts.extend(SIDEBAR.iter().filter(|(s, ..)| s.is_some()).map(|(_, n, ..)| *n));
        texts.extend(EFFECTS.iter().flat_map(|(_, l, d)| [*l, *d]));
        texts.extend(SEARCHABLE.iter().flat_map(|(n, k, _)| [*n, *k]));
        texts.extend(SHORTCUT_GROUPS.iter().map(|(_, n)| *n));
        texts.extend(mide::keyboard_layouts::LAYOUTS.iter().map(|(n, ..)| *n));
        let missing: Vec<&str> = texts.into_iter().filter(|t| !mide_ui::kit::i18n::is_translated(None, t)).collect();
        assert!(missing.is_empty(), "sem tradução em locales/pt_BR/settings.po: {missing:?}");
    }
}
