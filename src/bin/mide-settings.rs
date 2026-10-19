//! Configurações do Sistema do mide, feitas com o mide-ui: barra lateral com busca e
//! as seções; à direita, cartões com as opções.
//!
//! Seções: Wi-Fi, Bluetooth, Som, Papel de parede, Dock, Efeitos, Teclado e Sobre.
//! O que é gravado vai para o `config.toml` (mantendo os comentários) e vale na hora:
//! o compositor relê a configuração (`reload`), e a dock e a barra percebem a mudança.
//!
//! Uso: `mide-settings [wifi|bluetooth|sound|wallpaper|dock|effects|keyboard|about]`.

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
    kit::{run, App, Key, Tile, Ui, Window},
    settings_file::{self, Value},
    system::{self, Status},
};
use tiny_skia::Pixmap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Section {
    Wifi,
    Bluetooth,
    Sound,
    Wallpaper,
    Dock,
    Bar,
    Effects,
    Keyboard,
    About,
}

/// Itens da barra lateral, na ordem: seção (ou espaço entre grupos), nome, ícone do
/// tema e cor do quadradinho.
const SIDEBAR: &[(Option<Section>, &str, &str, [u8; 3])] = &[
    (Some(Section::Wifi), "Wi-Fi", "network-wireless", [10, 132, 255]),
    (Some(Section::Bluetooth), "Bluetooth", "bluetooth", [10, 132, 255]),
    (None, "", "", [0, 0, 0]),
    (Some(Section::Sound), "Som", "audio-volume-high", [255, 55, 95]),
    (Some(Section::Wallpaper), "Papel de parede", "preferences-desktop-wallpaper", [100, 210, 255]),
    (Some(Section::Dock), "Dock", "user-desktop", [52, 199, 89]),
    (Some(Section::Bar), "Barra superior", "gnome-panel", [88, 86, 214]),
    (Some(Section::Effects), "Efeitos", "preferences-desktop-theme", [191, 90, 242]),
    (None, "", "", [0, 0, 0]),
    (Some(Section::Keyboard), "Teclado", "preferences-desktop-keyboard", [142, 142, 147]),
    (Some(Section::About), "Sobre", "help-about", [142, 142, 147]),
];

/// Efeitos do `[effects]`: chave, rótulo e explicação.
const EFFECTS: &[(&str, &str, &str)] = &[
    ("shadows", "Sombras das janelas", "Sombra suave em volta de cada janela"),
    ("rounded_corners", "Cantos arredondados", "Janelas com cantos arredondados e contorno fino"),
    ("animations", "Animações", "Maximizar, restaurar e minimizar com transição suave"),
    ("lamp", "Efeito lâmpada", "Minimizar escorre a janela até o ícone da dock"),
    ("dock_blur", "Desfoque na dock", "A dock desfoca o que está atrás dela, como vidro fosco"),
    ("launcher_blur", "Desfoque no launcher", "O fundo do launcher é o papel de parede desfocado"),
    ("liquid_glass", "Vidro líquido", "Dock com bordas que refratam a luz e reflexo, sobre o desfoque"),
    ("panel_glass", "Vidro em barras e menus", "Barra superior, menus, notificações e prévias das janelas com vidro"),
];

/// O que a busca encontra: nome da opção, palavras-chave e a seção dela.
const SEARCHABLE: &[(&str, &str, Section)] = &[
    ("Wi-Fi", "rede internet wireless conectar senha", Section::Wifi),
    ("Bluetooth", "fone mouse teclado aparelhos parear sem fio", Section::Bluetooth),
    ("Volume de saída", "som audio alto-falante volume", Section::Sound),
    ("Sem som", "mudo silenciar som", Section::Sound),
    ("Papel de parede", "fundo imagem wallpaper area de trabalho", Section::Wallpaper),
    ("Adicionar imagem de fundo", "papel de parede imagem foto", Section::Wallpaper),
    ("Tamanho dos ícones da dock", "dock icones tamanho", Section::Dock),
    ("Ocultar e mostrar a Dock automaticamente", "dock esconder ocultar automatico", Section::Dock),
    ("Itens da barra superior", "barra menu topo icones som wifi bluetooth bateria central controle engrenagem", Section::Bar),
    ("Sombras das janelas", "efeitos sombra", Section::Effects),
    ("Cantos arredondados", "efeitos cantos bordas", Section::Effects),
    ("Animações", "efeitos animacao transicao", Section::Effects),
    ("Efeito lâmpada", "efeitos minimizar genie", Section::Effects),
    ("Desfoque na dock", "efeitos blur desfoque vidro", Section::Effects),
    ("Desfoque no launcher", "efeitos blur desfoque launcher", Section::Effects),
    ("Vidro líquido", "efeitos vidro liquid glass transparencia", Section::Effects),
    ("Vidro em barras e menus", "efeitos vidro barra menus notificacoes transparencia", Section::Effects),
    ("Teclas no estilo Mac", "teclado command option cmd super ctrl atalhos", Section::Keyboard),
    ("Atalhos de teclado", "teclado atalhos teclas combinacoes", Section::Keyboard),
    ("Mudar ou criar atalho", "teclado atalho novo gravar combinacao remover padrao", Section::Keyboard),
    ("Sobre este computador", "sistema versao memoria processador", Section::About),
];

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
        ui.heading("Redes");
        ui.card(|ui| {
            if networks.is_empty() {
                ui.note("Procurando redes…");
            }
            for net in networks.iter().take(10) {
                let detail = if net.active { "Conectado".to_string() } else { format!("{}%{}", net.signal, if net.secure { " · com senha" } else { "" }) };
                if ui.list_item(&net.ssid, &detail).clicked() && !net.active {
                    system::connect_network(&net.ssid);
                }
            }
        });
    }

    fn bluetooth(&mut self, ui: &mut Ui) {
        ui.title("Bluetooth");
        let (on, devices) = self.status.bluetooth.clone().unwrap_or((false, Vec::new()));
        let mut value = on;
        ui.card(|ui| {
            let detail = if on { "Visível para os aparelhos pareados" } else { "Desligado" };
            if ui.switch_detail("Bluetooth", detail, &mut value).changed() {
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
        ui.heading("Meus aparelhos");
        ui.card(|ui| {
            if devices.is_empty() {
                ui.note("Nenhum aparelho pareado");
            }
            for d in &devices {
                let detail = if d.connected { "Conectado · clique para desconectar" } else { "Não conectado · clique para conectar" };
                if ui.list_item(&d.name, detail).clicked() {
                    system::toggle_device(d);
                }
            }
        });
    }

    fn sound(&mut self, ui: &mut Ui) {
        ui.title("Som");
        ui.heading("Saída");
        let (_, muted) = self.status.volume.unwrap_or((0.0, false));
        let mut mute = muted;
        let mut volume = self.volume;
        ui.card(|ui| {
            let r = ui.slider("Volume de saída", &mut volume, 0.0, 100.0);
            // Grava só ao soltar o trilho (cada mudança chama um comando do sistema).
            if r.submitted() {
                system::set_volume(volume / 100.0);
                if muted {
                    system::set_muted(false);
                }
            }
            if ui.switch("Sem som", &mut mute).changed() {
                system::set_muted(mute);
            }
        });
        self.volume = volume;
        if mute != muted {
            if let Some(v) = self.status.volume.as_mut() {
                v.1 = mute;
            }
        }
    }

    fn wallpaper(&mut self, ui: &mut Ui) {
        if self.picker.is_some() {
            self.picker_view(ui);
            return;
        }
        ui.title("Papel de parede");
        let current = current_wallpaper();
        let mut chosen = None;
        let mut remove = None;
        let mut add = false;
        let wallpapers = self.wallpapers.clone();
        ui.wrap(|ui| {
            for path in &wallpapers {
                let name = match path {
                    None => "Padrão do mide".to_string(),
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
                        if m.danger("Remover da lista") {
                            remove = Some(p.clone());
                        }
                    });
                }
            }
            add = ui.tile_wide(Tile::Icon("list-add"), "Adicionar imagem…", false, THUMB.0, THUMB.1).clicked();
        });
        ui.note("Clique com o botão direito numa imagem adicionada para tirá-la da lista.");
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
        ui.title("Escolher uma imagem");
        let mut go = None;
        let mut choose = None;
        let mut cancel = false;
        ui.row(|ui| {
            for (name, place) in [("Imagens", mide::places::pictures_dir()), ("Downloads", mide::places::downloads_dir()), ("Pasta pessoal", mide::places::home_dir())] {
                let r = if place == dir { ui.primary_button(name) } else { ui.button(name) };
                if r.clicked() {
                    go = Some(place);
                }
            }
            if ui.button("‹ Voltar").clicked() {
                go = dir.parent().map(|p| p.to_path_buf());
            }
            cancel = ui.button("Cancelar").clicked();
        });
        ui.note(&dir.display().to_string());
        if entries.is_empty() {
            ui.note("Nenhuma imagem nesta pasta.");
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
            if ui.slider("Tamanho dos ícones", &mut size, DOCK_MIN, DOCK_MAX).submitted() {
                let n = size.round() as i64;
                let value = (n != DOCK_DEFAULT).then_some(Value::Int(n));
                match settings_file::set("dock", "icon_size", value) {
                    Ok(()) => tracing::info!("tamanho dos ícones da dock: {n} px"),
                    Err(err) => tracing::warn!("tamanho da dock não gravado: {err}"),
                }
            }
            let detail = "A dock some e aparece ao levar o ponteiro até a borda de baixo";
            if ui.switch_detail("Ocultar e mostrar a Dock automaticamente", detail, &mut autohide).changed() {
                match settings_file::set("dock", "autohide", autohide.then_some(Value::Bool(true))) {
                    Ok(()) => tracing::info!("dock: ocultar automaticamente {}", if autohide { "ligado" } else { "desligado" }),
                    Err(err) => tracing::warn!("opção da dock não gravada: {err}"),
                }
            }
        });
        self.dock_size = size;
        ui.note("Sem ocultar, as janelas maximizadas ficam acima da dock. Para fixar um app, clique com o botão direito no ícone dele e escolha \"Manter na Dock\".");
    }

    /// Itens da direita da barra superior (`[bar]`): todos à mostra por padrão.
    fn bar(&mut self, ui: &mut Ui) {
        ui.title("Barra superior");
        ui.card(|ui| {
            for (key, label, detail) in settings_file::BAR_ITEMS {
                let mut on = settings_file::bar_item(key);
                if ui.switch_detail(label, detail, &mut on).changed() {
                    // À mostra é o padrão: grava só quando esconde.
                    let value = if on { None } else { Some(Value::Bool(false)) };
                    match settings_file::set("bar", key, value) {
                        Ok(()) => tracing::info!("barra superior: {label} {}", if on { "à mostra" } else { "escondido" }),
                        Err(err) => tracing::warn!("item da barra não gravado: {err}"),
                    }
                }
            }
        });
        ui.note("O relógio fica sempre na barra. Itens escondidos continuam nas Configurações e na central de controle.");
    }

    fn effects(&mut self, ui: &mut Ui) {
        ui.title("Efeitos");
        ui.card(|ui| {
            for (key, label, detail) in EFFECTS {
                let mut on = effect_on(key);
                if ui.switch_detail(label, detail, &mut on).changed() {
                    // Ligado é o padrão: grava só quando desliga, e apaga a chave ao religar.
                    let value = if on { None } else { Some(Value::Bool(false)) };
                    match settings_file::set("effects", key, value) {
                        Ok(()) => {
                            tracing::info!("efeito {label}: {}", if on { "ligado" } else { "desligado" });
                            ask_compositor("reload");
                        }
                        Err(err) => tracing::warn!("efeito não gravado: {err}"),
                    }
                }
            }
        });
        ui.note("As mudanças valem na hora.");
    }

    fn keyboard(&mut self, ui: &mut Ui) {
        ui.title("Teclado");
        let mut mac = matches!(settings_file::get("keyboard", "mac_style"), Some(Value::Bool(true)));
        ui.card(|ui| {
            let detail = "Command (tecla Super/Windows) faz o papel do Ctrl e Option é o Alt, como no Mac";
            if ui.switch_detail("Teclas no estilo Mac", detail, &mut mac).changed() {
                match settings_file::set("keyboard", "mac_style", mac.then_some(Value::Bool(true))) {
                    Ok(()) => {
                        tracing::info!("teclas no estilo Mac: {}", if mac { "ligado" } else { "desligado" });
                        ask_compositor("reload");
                    }
                    Err(err) => tracing::warn!("opção do teclado não gravada: {err}"),
                }
            }
        });
        ui.heading("Atalhos");
        let bindings = self.bindings.clone();
        let mut chosen = None;
        ui.card(|ui| {
            if bindings.is_empty() {
                ui.note("Nenhum atalho.");
            }
            for (combo, action) in &bindings {
                if ui.list_item(&describe_action(action, &self.apps), &pretty_combo(combo, mac)).clicked() {
                    chosen = Some((combo.clone(), action.clone()));
                }
            }
        });
        let (mut add, mut reset) = (false, false);
        ui.row(|ui| {
            ui.sized(170.0, |ui| add = ui.button("Novo atalho…").clicked());
            ui.sized(200.0, |ui| reset = ui.button("Restaurar os padrões").clicked());
        });
        ui.note("Clique num atalho para mudar a combinação ou removê-lo. As mudanças valem na hora.");
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
                    editor.error = Some(format!("Não foi possível gravar: {err}"));
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
                        Some(problem) => editor.error = Some(problem.into()),
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
            None => "Novo atalho".to_string(),
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
                ui.note("O que o atalho faz? Uma ação do mide ou abrir um app:");
                ui.text_input(&mut query, "Buscar app");
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
                        ui.note("Nenhum app encontrado.");
                    }
                    for app in found.into_iter().take(5) {
                        if ui.list_item(&app.name, "App").clicked() {
                            pick = Some(format!("launch:{}", app_command(app)));
                        }
                    }
                });
                ui.row(|ui| {
                    ui.push_right(110.0);
                    ui.sized(110.0, |ui| close |= ui.button("Cancelar").clicked());
                });
                return;
            }
            if let Some((combo, used_by)) = &editor.conflict {
                ui.label(&format!("{} já é usado por \"{used_by}\".", pretty_combo(combo, mac)));
                ui.note("Substituir faz essa combinação passar para este atalho.");
                ui.row(|ui| {
                    ui.push_right(2.0 * 150.0 + 8.0);
                    ui.sized(150.0, |ui| record = ui.button("Gravar outra").clicked());
                    ui.sized(150.0, |ui| replace = ui.primary_button("Substituir").clicked());
                });
                return;
            }
            if editor.capturing {
                ui.label("Aperte a nova combinação de teclas…");
                ui.note("Use Ctrl, Alt ou Super junto com a tecla. Esc cancela.");
                return;
            }
            let current = editor.old.as_deref().map(|c| pretty_combo(c, mac)).unwrap_or_else(|| "Nenhuma".into());
            ui.list_item("Combinação", &current);
            if let Some(err) = &editor.error {
                ui.note(err);
            }
            ui.row(|ui| {
                let buttons = if editor.old.is_some() { 3.0 } else { 2.0 };
                ui.push_right(buttons * 120.0 + (buttons - 1.0) * 8.0);
                if editor.old.is_some() {
                    ui.sized(120.0, |ui| remove = ui.button("Remover").clicked());
                }
                ui.sized(120.0, |ui| close |= ui.button("Cancelar").clicked());
                ui.sized(120.0, |ui| record = ui.primary_button("Gravar…").clicked());
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
                editor.error = Some("O compositor não respondeu.".into());
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
        ui.title("Sobre");
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
        ui.title("Resultados");
        let q = mide::apps::fold(self.query.trim());
        let found: Vec<&(&str, &str, Section)> = SEARCHABLE
            .iter()
            .filter(|(name, keys, _)| {
                let name = mide::apps::fold(name);
                name.contains(&q) || mide::apps::fold(keys).split(' ').any(|k| k.starts_with(&q))
            })
            .collect();
        if found.is_empty() {
            ui.note("Nenhuma opção encontrada.");
            return;
        }
        let mut open = None;
        ui.card(|ui| {
            for (name, _, section) in &found {
                if ui.list_item(name, section_name(*section)).clicked() {
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
        let items: Vec<(&str, &str, Option<[u8; 3]>)> = SIDEBAR.iter().map(|(s, name, icon, tint)| (*name, if s.is_some() { *icon } else { "" }, s.map(|_| *tint))).collect();
        let selected = SIDEBAR.iter().position(|(s, ..)| *s == Some(self.section)).unwrap_or(0);
        // O campo de busca fica na barra lateral; o conteúdo usa a busca deste quadro
        // (o que for digitado agora pede outro quadro, que já mostra os resultados).
        let mut query = self.query.clone();
        let searching = !query.trim().is_empty();
        let chosen = ui.sidebar_search(&items, if searching { usize::MAX } else { selected }, &mut query, "Buscar", |ui, _| {
            if !self.query.trim().is_empty() {
                self.results(ui);
                return;
            }
            match self.section {
                Section::Wifi => self.wifi(ui),
                Section::Bluetooth => self.bluetooth(ui),
                Section::Sound => self.sound(ui),
                Section::Wallpaper => self.wallpaper(ui),
                Section::Dock => self.dock(ui),
                Section::Bar => self.bar(ui),
                Section::Effects => self.effects(ui),
                Section::Keyboard => self.keyboard(ui),
                Section::About => self.about(ui),
            }
        });
        // Diálogo de atalho por cima da janela inteira (fora da área de conteúdo).
        if self.shortcut.is_some() {
            let mac = matches!(settings_file::get("keyboard", "mac_style"), Some(Value::Bool(true)));
            self.shortcut_dialog(ui, mac);
        }
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

fn section_name(section: Section) -> &'static str {
    SIDEBAR.iter().find(|(s, ..)| *s == Some(section)).map(|(_, n, ..)| *n).unwrap_or("")
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
/// as adicionadas pelo usuário.
fn wallpaper_list() -> Vec<Option<PathBuf>> {
    let mut list = vec![None];
    let dirs = [PathBuf::from("/usr/share/backgrounds"), mide::places::user_backgrounds_dir()];
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
fn pretty_combo(combo: &str, mac: bool) -> String {
    combo
        .split('+')
        .map(|k| match k {
            "ctrl" if mac => "⌘".to_string(),
            "ctrl" => "Ctrl".to_string(),
            "shift" => "⇧".to_string(),
            "alt" if mac => "⌥".to_string(),
            "alt" => "Alt".to_string(),
            "super" if mac => "Ctrl".to_string(),
            "super" => "Super".to_string(),
            "space" => "Espaço".to_string(),
            "escape" => "Esc".to_string(),
            "return" => "Enter".to_string(),
            other => other.to_uppercase(),
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
const FIXED_ACTIONS: &[&str] = &["toggle_search", "toggle_launcher", "close_window", "restore_minimized", "quit"];

/// Combinação que não serve para atalho global: sem Ctrl, Alt ou Super ela tomaria
/// uma tecla usada para digitar. Teclas de função (F1 a F24) podem ficar sozinhas.
fn combo_problem(combo: &str) -> Option<&'static str> {
    let parts: Vec<&str> = combo.split('+').collect();
    let key = parts.last().copied().unwrap_or("");
    let function_key = key.strip_prefix('f').is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()));
    let held = parts.iter().any(|p| matches!(*p, "ctrl" | "alt" | "super"));
    (!held && !function_key).then_some("Use Ctrl, Alt ou Super junto com a tecla, para não tomar uma tecla de digitar.")
}

/// Comando que abre o app (apps de terminal abrem no terminal).
fn app_command(app: &mide::apps::App) -> String {
    if app.terminal {
        format!("foot -e {0} || x-terminal-emulator -e {0}", app.command)
    } else {
        app.command.clone()
    }
}

/// Ação de um atalho em português. Atalhos que abrem um app mostram o nome dele.
fn describe_action(action: &str, apps: &[mide::apps::App]) -> String {
    match action {
        "close_window" => "Fechar a janela".into(),
        "quit" => "Encerrar a sessão do mide".into(),
        "restore_minimized" => "Restaurar a última janela minimizada".into(),
        "toggle_launcher" => "Abrir ou fechar o launcher".into(),
        "toggle_search" => "Abrir ou fechar o buscador".into(),
        other => match other.strip_prefix("launch:") {
            Some(cmd) => match apps.iter().find(|a| a.command == cmd || app_command(a) == cmd) {
                Some(app) => format!("Abrir {}", app.name),
                None => format!("Abrir: {cmd}"),
            },
            None => other.into(),
        },
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "mide_settings=info".into()))
        .init();
    let section = match std::env::args().nth(1).as_deref() {
        Some("bluetooth") => Section::Bluetooth,
        Some("sound") => Section::Sound,
        Some("wallpaper") => Section::Wallpaper,
        Some("dock") => Section::Dock,
        Some("bar") => Section::Bar,
        Some("effects") => Section::Effects,
        Some("keyboard") => Section::Keyboard,
        Some("about") => Section::About,
        _ => Section::Wifi,
    };
    tracing::info!("configurações abertas: {section:?}");
    let window = Window::new("Configurações do Sistema").app_id("mide-settings").size(820, 560).min_size(640, 440);
    run(window, Settings::new(section))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atalhos_em_portugues() {
        assert_eq!(pretty_combo("ctrl+shift+space", false), "Ctrl ⇧ Espaço");
        assert_eq!(pretty_combo("ctrl+shift+t", false), "Ctrl ⇧ T");
        assert_eq!(pretty_combo("ctrl+space", true), "⌘ Espaço");
        assert_eq!(describe_action("toggle_launcher", &[]), "Abrir ou fechar o launcher");
        assert_eq!(describe_action("toggle_search", &[]), "Abrir ou fechar o buscador");
        assert_eq!(describe_action("launch:foot", &[]), "Abrir: foot");
    }

    #[test]
    fn atalho_de_app_mostra_o_nome() {
        let app = mide::apps::App {
            id: "org.codeberg.dnkl.foot".into(),
            name: "Foot".into(),
            command: "foot".into(),
            icon: None,
            group: "Utilitários",
            terminal: false,
            keywords: String::new(),
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
        let q = mide::apps::fold("volume");
        let hits: Vec<&str> = SEARCHABLE.iter().filter(|(n, k, _)| mide::apps::fold(n).contains(&q) || mide::apps::fold(k).split(' ').any(|w| w.starts_with(&q))).map(|(n, ..)| *n).collect();
        assert_eq!(hits, vec!["Volume de saída"]);
    }
}
