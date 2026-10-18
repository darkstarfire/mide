//! Configurações do Sistema do mide, no estilo do macOS: barra lateral com as
//! seções e, à direita, cartões brancos com as opções.
//!
//! Seções: Wi-Fi, Bluetooth, Som, Papel de parede, Dock, Efeitos, Atalhos de
//! teclado e Sobre. Papel de parede e efeitos são gravados no `config.toml` (mantendo os
//! comentários) e valem na hora: o compositor relê a configuração (`reload`).
//!
//! Uso: `mide-settings [wifi|bluetooth|sound|wallpaper|dock|effects|keyboard|about]`.

use std::{collections::HashMap, error::Error, fs, path::PathBuf, time::Duration};

use mide::{
    control_client::ask_compositor,
    icons::load_themed_icon,
    settings_file::{self, Value},
    system::{self, Status},
    ui::{
        draw::{fill_rounded, rgba, rounded_rect, to_argb8888},
        text::{FontWeight, Text},
    },
};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_registry,
    output::{OutputHandler, OutputState},
    reexports::{
        calloop::{channel, EventLoop},
        calloop_wayland_source::WaylandSource,
        client::{
            globals::registry_queue_init,
            protocol::{wl_output, wl_pointer, wl_seat, wl_shm, wl_surface},
            Connection, QueueHandle,
        },
    },
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        pointer::{PointerEvent, PointerEventKind, PointerHandler},
        Capability, SeatHandler, SeatState,
    },
    shell::{
        xdg::{
            window::{Window, WindowConfigure, WindowDecorations, WindowHandler},
            XdgShell,
        },
        WaylandSurface,
    },
    shm::{slot::SlotPool, Shm, ShmHandler},
};
use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, PixmapPaint, Stroke, Transform};

const WIDTH: u32 = 820;
const HEIGHT: u32 = 560;
const MIN_W: u32 = 640;
const MIN_H: u32 = 440;
const SIDEBAR_W: f32 = 220.0;
const ROW_H: f32 = 40.0;
const THUMB_W: u32 = 168;
const THUMB_H: u32 = 105;
const BTN_LEFT: u32 = 0x110;
const FONT: f32 = 13.5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Section {
    Wifi,
    Bluetooth,
    Sound,
    Wallpaper,
    Dock,
    Effects,
    Keyboard,
    About,
}

/// Seções da barra lateral: grupo (separado por espaço), nome, ícones do tema e cor
/// de reserva do quadradinho.
const SECTIONS: &[(Section, u8, &str, &[&str], (u8, u8, u8))] = &[
    (Section::Wifi, 0, "Wi-Fi", &["network-wireless", "network-wireless-symbolic"], (10, 132, 255)),
    (Section::Bluetooth, 0, "Bluetooth", &["bluetooth", "bluetooth-active", "preferences-system-bluetooth"], (10, 132, 255)),
    (Section::Sound, 1, "Som", &["audio-volume-high", "multimedia-volume-control", "preferences-desktop-sound"], (255, 55, 95)),
    (Section::Wallpaper, 1, "Papel de parede", &["preferences-desktop-wallpaper", "preferences-desktop-background"], (100, 210, 255)),
    (Section::Dock, 1, "Dock", &["preferences-desktop", "user-desktop", "desktop"], (52, 199, 89)),
    (Section::Effects, 1, "Efeitos", &["preferences-desktop-theme", "preferences-desktop-effects", "applications-graphics"], (191, 90, 242)),
    (Section::Keyboard, 2, "Atalhos de teclado", &["preferences-desktop-keyboard", "input-keyboard"], (142, 142, 147)),
    (Section::About, 2, "Sobre", &["help-about", "computer", "dialog-information"], (142, 142, 147)),
];

/// Efeitos do `[effects]`: chave e rótulo.
const EFFECTS: &[(&str, &str, &str)] = &[
    ("shadows", "Sombras das janelas", "Sombra suave em volta de cada janela"),
    ("rounded_corners", "Cantos arredondados", "Janelas com cantos arredondados e contorno fino"),
    ("animations", "Animações", "Maximizar, restaurar e minimizar com transição suave"),
    ("lamp", "Efeito lâmpada", "Minimizar escorre a janela até o ícone da dock"),
    ("dock_blur", "Desfoque na dock", "A dock desfoca o que está atrás dela, como vidro fosco"),
    ("launcher_blur", "Desfoque no launcher", "O fundo do launcher é o papel de parede desfocado"),
    ("liquid_glass", "Vidro líquido", "Dock com bordas que refratam a luz e reflexo, sobre o desfoque"),
    ("panel_glass", "Vidro em barras e menus", "Barra superior, menus e notificações sobre vidro desfocado"),
];

#[derive(Debug, Clone, PartialEq)]
enum Action {
    Select(Section),
    ToggleWifi,
    Connect(String),
    ToggleBluetooth,
    Device(usize),
    VolumeTrack,
    ToggleMute,
    Wallpaper(Option<PathBuf>),
    Effect(usize),
    /// Trilho do tamanho dos ícones da dock.
    DockSize,
    /// Barra de rolagem do conteúdo (trilho inteiro).
    ScrollBar,
    /// Papéis de parede: abrir o seletor, tirar uma imagem adicionada.
    AddWallpaper,
    RemoveWallpaper(PathBuf),
    /// Seletor: ir a uma pasta, subir, escolher uma imagem, cancelar.
    PickerDir(PathBuf),
    PickerUp,
    PickerChoose(PathBuf),
    PickerCancel,
    /// Área do seletor sem ação (o clique não passa para trás).
    PickerSheet,
}

/// Tamanhos dos ícones da dock (px).
const DOCK_MIN: u32 = 32;
const DOCK_MAX: u32 = 64;
const DOCK_DEFAULT: u32 = 40;

/// Mensagens das threads: estado do sistema e miniaturas dos papéis de parede.
enum Msg {
    Status(Status),
    /// Miniatura de um papel de parede (`None` = gradiente do mide).
    Thumb(Option<PathBuf>, Option<Pixmap>),
    /// Miniatura de uma imagem no seletor de arquivos.
    PickerThumb(PathBuf, Option<Pixmap>),
}

/// Miniaturas carregadas numa thread, uma por mensagem.
fn load_thumbs(tx: channel::Sender<Msg>, paths: Vec<Option<PathBuf>>) {
    std::thread::spawn(move || {
        for path in paths {
            let thumb = match &path {
                Some(path) => image::open(path).ok().map(|img| to_pixmap(&mide::backdrop::cover(&img.to_rgba8(), THUMB_W, THUMB_H))),
                None => Some(to_pixmap(&mide::backdrop::default_gradient(THUMB_W, THUMB_H))),
            };
            if tx.send(Msg::Thumb(path, thumb)).is_err() {
                break;
            }
        }
    });
}

/// Miniaturas das imagens de uma pasta do seletor.
fn load_picker_thumbs(tx: channel::Sender<Msg>, paths: Vec<PathBuf>) {
    std::thread::spawn(move || {
        for path in paths {
            let thumb = image::open(&path).ok().map(|img| to_pixmap(&mide::backdrop::cover(&img.to_rgba8(), PICK_W, PICK_H)));
            if tx.send(Msg::PickerThumb(path, thumb)).is_err() {
                break;
            }
        }
    });
}

/// Seletor de imagens (folha por cima do conteúdo, como no macOS).
struct Picker {
    dir: PathBuf,
    entries: Vec<(PathBuf, bool)>,
    thumbs: HashMap<PathBuf, Option<Pixmap>>,
    scroll: f32,
}

const PICK_W: u32 = 120;
const PICK_H: u32 = 80;

fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "mide_settings=info".into()),
        )
        .init();
    let section = match std::env::args().nth(1).as_deref() {
        Some("bluetooth") => Section::Bluetooth,
        Some("sound") => Section::Sound,
        Some("wallpaper") => Section::Wallpaper,
        Some("dock") => Section::Dock,
        Some("effects") => Section::Effects,
        Some("keyboard") => Section::Keyboard,
        Some("about") => Section::About,
        _ => Section::Wifi,
    };

    let conn = Connection::connect_to_env()?;
    let (globals, event_queue) = registry_queue_init(&conn)?;
    let qh = event_queue.handle();
    let compositor = CompositorState::bind(&globals, &qh)?;
    let xdg_shell = XdgShell::bind(&globals, &qh)?;
    let shm = Shm::bind(&globals, &qh)?;

    let surface = compositor.create_surface(&qh);
    let window = xdg_shell.create_window(surface, WindowDecorations::RequestServer, &qh);
    window.set_title("Configurações do Sistema");
    window.set_app_id("mide-settings");
    window.set_min_size(Some((MIN_W, MIN_H)));
    window.commit();

    let wallpapers = wallpaper_list();
    let mut app = Settings {
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),
        pool: SlotPool::new((WIDTH * HEIGHT * 4) as usize, &shm)?,
        shm,
        window,
        pointer: None,
        width: WIDTH,
        height: HEIGHT,
        configured: false,
        dirty: false,
        exit: false,
        section,
        status: Status::default(),
        text: Text::new(),
        icons: HashMap::new(),
        regions: Vec::new(),
        hover: None,
        dragging_volume: false,
        scroll: 0.0,
        thumbs: HashMap::new(),
        wallpapers,
        picker: None,
        tx: None,
        bindings: Vec::new(),
        info: mide::sysinfo::system_info(),
        dock_size: match settings_file::get("dock", "icon_size") {
            Some(Value::Int(n)) => (n as u32).clamp(DOCK_MIN, DOCK_MAX),
            _ => DOCK_DEFAULT,
        },
        dragging_dock: false,
        content_h: 0.0,
        dragging_scroll: None,
    };
    app.load_bindings();
    tracing::info!("configurações abertas: {:?}", app.section);

    let mut event_loop: EventLoop<Settings> = EventLoop::try_new()?;
    WaylandSource::new(conn.clone(), event_queue)
        .insert(event_loop.handle())
        .map_err(|e| e.to_string())?;
    let (tx, rx) = channel::channel::<Msg>();
    // Som, Wi-Fi e Bluetooth a cada 2 s.
    let status_tx = tx.clone();
    std::thread::spawn(move || loop {
        if status_tx.send(Msg::Status(system::read_status())).is_err() {
            break;
        }
        std::thread::sleep(Duration::from_secs(2));
    });
    // Miniaturas dos papéis de parede.
    load_thumbs(tx.clone(), app.wallpapers.clone());
    app.tx = Some(tx);
    event_loop
        .handle()
        .insert_source(rx, |event, _, app: &mut Settings| {
            if let channel::Event::Msg(msg) = event {
                match msg {
                    Msg::Status(status) => {
                        if status != app.status && !app.dragging_volume {
                            app.status = status;
                            app.dirty = true;
                        }
                    }
                    Msg::Thumb(path, pixmap) => {
                        app.thumbs.insert(path, pixmap);
                        app.dirty |= app.section == Section::Wallpaper;
                    }
                    Msg::PickerThumb(path, pixmap) => {
                        if let Some(picker) = app.picker.as_mut() {
                            picker.thumbs.insert(path, pixmap);
                            app.dirty = true;
                        }
                    }
                }
            }
        })
        .map_err(|e| e.to_string())?;

    while !app.exit {
        event_loop.dispatch(None, &mut app)?;
        if app.dirty && app.configured {
            app.dirty = false;
            app.draw();
        }
    }
    Ok(())
}

/// Papéis de parede para escolher: o gradiente do mide (`None`) e as imagens do sistema.
fn wallpaper_list() -> Vec<Option<PathBuf>> {
    let mut list = vec![None];
    let dirs = vec![PathBuf::from("/usr/share/backgrounds"), mide::places::user_backgrounds_dir()];
    let mut files: Vec<PathBuf> = dirs
        .iter()
        .filter_map(|d| fs::read_dir(d).ok())
        .flat_map(|d| d.flatten().map(|e| e.path()))
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| matches!(e.to_lowercase().as_str(), "jpg" | "jpeg" | "png"))
        })
        .collect();
    files.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()));
    list.extend(files.into_iter().take(60).map(Some));
    list
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

struct Settings {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    shm: Shm,
    pool: SlotPool,
    window: Window,
    pointer: Option<wl_pointer::WlPointer>,
    width: u32,
    height: u32,
    configured: bool,
    dirty: bool,
    exit: bool,
    section: Section,
    status: Status,
    text: Text,
    icons: HashMap<Section, Option<Pixmap>>,
    regions: Vec<((f32, f32, f32, f32), Action)>,
    hover: Option<usize>,
    dragging_volume: bool,
    /// Rolagem do conteúdo (seções compridas, como a dos papéis de parede).
    scroll: f32,
    wallpapers: Vec<Option<PathBuf>>,
    thumbs: HashMap<Option<PathBuf>, Option<Pixmap>>,
    /// Seletor de imagens aberto (Adicionar imagem…).
    picker: Option<Picker>,
    /// Canal das threads de miniaturas.
    tx: Option<channel::Sender<Msg>>,
    bindings: Vec<(String, String)>,
    info: Vec<(&'static str, String)>,
    /// Tamanho dos ícones da dock e se o trilho está sendo arrastado.
    dock_size: u32,
    dragging_dock: bool,
    /// Altura do conteúdo do último desenho e o arrasto da barra de rolagem (ponto
    /// do clique dentro da alça).
    content_h: f32,
    dragging_scroll: Option<f32>,
}

impl Settings {
    fn load_bindings(&mut self) {
        self.bindings = ask_compositor("bindings")
            .unwrap_or_default()
            .iter()
            .filter_map(|l| l.split_once('\t').map(|(c, a)| (c.to_string(), a.to_string())))
            .collect();
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

    fn draw(&mut self) {
        let (w, h) = (self.width, self.height);
        let Some(mut pixmap) = Pixmap::new(w, h) else {
            return;
        };
        pixmap.fill(tiny_skia::Color::from_rgba8(246, 246, 248, 255));
        self.regions.clear();
        let hover = self.hover;

        // Conteúdo à direita (com rolagem), depois a barra lateral por cima.
        let cx = SIDEBAR_W + 28.0;
        let cw = w as f32 - cx - 28.0;
        let mut content = Pixmap::new(w, 2400).unwrap_or_else(|| Pixmap::new(1, 1).unwrap());
        let mut regions = Vec::new();
        let content_h = self.draw_section(&mut content, cx, cw, &mut regions, hover);
        let max_scroll = (content_h - h as f32 + 24.0).max(0.0);
        self.scroll = self.scroll.clamp(0.0, max_scroll);
        pixmap.draw_pixmap(0, -self.scroll as i32, content.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
        for ((rx, ry, rw, rh), action) in regions {
            self.regions.push(((rx, ry - self.scroll, rw, rh), action));
        }
        self.content_h = content_h + 24.0;

        // Barra de rolagem fina, como no macOS: só aparece quando o conteúdo não cabe;
        // fica mais grossa com o mouse em cima ou arrastando.
        if max_scroll > 0.0 {
            let (track_x, track_h) = (w as f32 - 14.0, h as f32 - 8.0);
            let (thumb_y, thumb_h) = self.thumb();
            let active = self.dragging_scroll.is_some() || hover == Some(self.regions.len());
            let tw = if active { 8.0 } else { 6.0 };
            if active {
                fill_rounded(&mut pixmap, track_x - 1.0, 4.0, 10.0, track_h, 5.0, rgba(0, 0, 0, 14));
            }
            fill_rounded(&mut pixmap, track_x + (8.0 - tw) / 2.0, thumb_y, tw, thumb_h, tw / 2.0, rgba(0, 0, 0, if active { 120 } else { 80 }));
            self.regions.push(((track_x - 4.0, 0.0, 18.0, h as f32), Action::ScrollBar));
        }

        // Barra lateral: cinza claro, com as seções em grupos.
        fill_rounded(&mut pixmap, 0.0, 0.0, SIDEBAR_W, h as f32, 0.0, rgba(232, 232, 236, 255));
        fill_rounded(&mut pixmap, SIDEBAR_W - 1.0, 0.0, 1.0, h as f32, 0.0, rgba(0, 0, 0, 22));
        let mut y = 16.0;
        let mut group = 0;
        for (section, g, name, icon_names, color) in SECTIONS {
            if *g != group {
                group = *g;
                y += 12.0;
            }
            let selected = *section == self.section;
            let hovered = !selected && hover == Some(self.regions.len());
            if selected {
                fill_rounded(&mut pixmap, 10.0, y, SIDEBAR_W - 20.0, 30.0, 7.0, rgba(10, 100, 225, 255));
            } else if hovered {
                fill_rounded(&mut pixmap, 10.0, y, SIDEBAR_W - 20.0, 30.0, 7.0, rgba(0, 0, 0, 16));
            }
            let icon = self.icons.entry(*section).or_insert_with(|| load_themed_icon(icon_names, 20)).clone();
            fill_rounded(&mut pixmap, 18.0, y + 3.0, 24.0, 24.0, 6.0, rgba(color.0, color.1, color.2, 255));
            if let Some(icon) = icon {
                pixmap.draw_pixmap(20, (y + 5.0) as i32, icon.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
            }
            let tc = if selected { [255, 255, 255, 255] } else { [30, 30, 34, 255] };
            self.text.draw(&mut pixmap, name, 52.0, y + (30.0 - FONT * 1.25) / 2.0, FONT, FontWeight::Regular, tc);
            self.regions.push(((10.0, y, SIDEBAR_W - 20.0, 30.0), Action::Select(*section)));
            y += 34.0;
        }

        if self.picker.is_some() {
            self.draw_picker(&mut pixmap, hover);
        }

        let stride = w as i32 * 4;
        let Ok((buffer, canvas)) = self.pool.create_buffer(w as i32, h as i32, stride, wl_shm::Format::Argb8888) else {
            return;
        };
        to_argb8888(&pixmap, canvas);
        let surface = self.window.wl_surface();
        surface.damage_buffer(0, 0, w as i32, h as i32);
        if buffer.attach_to(surface).is_ok() {
            self.window.commit();
        }
    }

    /// Cartão branco arredondado com borda fina; devolve o topo do cartão.
    fn card(p: &mut Pixmap, x: f32, y: f32, w: f32, h: f32) {
        fill_rounded(p, x, y, w, h, 10.0, rgba(255, 255, 255, 255));
        if let Some(path) = rounded_rect(x + 0.5, y + 0.5, w - 1.0, h - 1.0, 10.0) {
            let mut paint = Paint::default();
            paint.set_color(rgba(0, 0, 0, 26));
            paint.anti_alias = true;
            p.stroke_path(&path, &paint, &Stroke::default(), Transform::identity(), None);
        }
    }

    fn row_line(p: &mut Pixmap, x: f32, y: f32, w: f32) {
        fill_rounded(p, x + 14.0, y, w - 28.0, 1.0, 0.0, rgba(0, 0, 0, 20));
    }

    fn title(&mut self, p: &mut Pixmap, x: f32, title: &str) -> f32 {
        self.text.draw(p, title, x, 22.0, 20.0, FontWeight::Bold, [24, 24, 28, 255]);
        64.0
    }

    /// Linha de cartão com texto à esquerda e um interruptor à direita.
    #[allow(clippy::too_many_arguments)]
    fn switch_row(&mut self, p: &mut Pixmap, x: f32, y: f32, w: f32, label: &str, detail: &str, on: bool, regions: &mut Vec<((f32, f32, f32, f32), Action)>, action: Action) {
        let h = if detail.is_empty() { ROW_H } else { ROW_H + 12.0 };
        let ty = if detail.is_empty() { y + (h - FONT * 1.25) / 2.0 } else { y + 8.0 };
        self.text.draw(p, label, x + 16.0, ty, FONT, FontWeight::Regular, [24, 24, 28, 255]);
        if !detail.is_empty() {
            self.text.draw(p, detail, x + 16.0, ty + 19.0, 11.5, FontWeight::Regular, [130, 130, 138, 255]);
        }
        draw_switch(p, x + w - 56.0, y + (h - 22.0) / 2.0, on);
        regions.push(((x + w - 64.0, y, 64.0, h), action));
    }

    fn draw_section(&mut self, p: &mut Pixmap, x: f32, w: f32, regions: &mut Vec<((f32, f32, f32, f32), Action)>, hover: Option<usize>) -> f32 {
        // As áreas do conteúdo vêm antes das da barra lateral na lista final, com o
        // mesmo índice: o destaque usa `hover` só onde faz sentido.
        match self.section {
            Section::Wifi => {
                let mut y = self.title(p, x, "Wi-Fi");
                let (on, networks) = self.status.wifi.clone().unwrap_or((false, Vec::new()));
                Self::card(p, x, y, w, ROW_H);
                self.switch_row(p, x, y, w, "Wi-Fi", "", on, regions, Action::ToggleWifi);
                y += ROW_H + 24.0;
                if on {
                    self.text.draw(p, "Redes", x + 4.0, y, 12.5, FontWeight::Semibold, [110, 110, 118, 255]);
                    y += 22.0;
                    let rows = networks.len().clamp(1, 10);
                    Self::card(p, x, y, w, rows as f32 * ROW_H);
                    if networks.is_empty() {
                        self.text.draw(p, "Procurando redes…", x + 16.0, y + 12.0, FONT, FontWeight::Regular, [130, 130, 138, 255]);
                    }
                    for (i, net) in networks.iter().take(10).enumerate() {
                        let ry = y + i as f32 * ROW_H;
                        if i > 0 {
                            Self::row_line(p, x, ry, w);
                        }
                        let weight = if net.active { FontWeight::Semibold } else { FontWeight::Regular };
                        self.text.draw(p, &net.ssid, x + 16.0, ry + (ROW_H - FONT * 1.25) / 2.0, FONT, weight, [24, 24, 28, 255]);
                        let right = if net.active { "Conectado".to_string() } else { format!("{}%{}", net.signal, if net.secure { " · com senha" } else { "" }) };
                        let rw = self.text.measure(&right, 12.0, FontWeight::Regular);
                        self.text.draw(p, &right, x + w - 16.0 - rw, ry + 12.0, 12.0, FontWeight::Regular, [130, 130, 138, 255]);
                        if !net.active {
                            regions.push(((x, ry, w, ROW_H), Action::Connect(net.ssid.clone())));
                        }
                    }
                    y += rows as f32 * ROW_H;
                }
                y + 24.0
            }
            Section::Bluetooth => {
                let mut y = self.title(p, x, "Bluetooth");
                let (on, devices) = self.status.bluetooth.clone().unwrap_or((false, Vec::new()));
                Self::card(p, x, y, w, ROW_H + 12.0);
                let detail = if on { "Visível como este computador para aparelhos pareados" } else { "Desligado" };
                self.switch_row(p, x, y, w, "Bluetooth", detail, on, regions, Action::ToggleBluetooth);
                y += ROW_H + 12.0 + 24.0;
                if on {
                    self.text.draw(p, "Meus aparelhos", x + 4.0, y, 12.5, FontWeight::Semibold, [110, 110, 118, 255]);
                    y += 22.0;
                    let rows = devices.len().max(1);
                    Self::card(p, x, y, w, rows as f32 * ROW_H);
                    if devices.is_empty() {
                        self.text.draw(p, "Nenhum aparelho pareado", x + 16.0, y + 12.0, FONT, FontWeight::Regular, [130, 130, 138, 255]);
                    }
                    for (i, d) in devices.iter().enumerate() {
                        let ry = y + i as f32 * ROW_H;
                        if i > 0 {
                            Self::row_line(p, x, ry, w);
                        }
                        self.text.draw(p, &d.name, x + 16.0, ry + (ROW_H - FONT * 1.25) / 2.0, FONT, FontWeight::Regular, [24, 24, 28, 255]);
                        let right = if d.connected { "Conectado · clique para desconectar" } else { "Não conectado · clique para conectar" };
                        let rw = self.text.measure(right, 12.0, FontWeight::Regular);
                        self.text.draw(p, right, x + w - 16.0 - rw, ry + 12.0, 12.0, FontWeight::Regular, [130, 130, 138, 255]);
                        regions.push(((x, ry, w, ROW_H), Action::Device(i)));
                    }
                    y += rows as f32 * ROW_H;
                }
                y + 24.0
            }
            Section::Sound => {
                let mut y = self.title(p, x, "Som");
                let (volume, muted) = self.status.volume.unwrap_or((0.0, false));
                self.text.draw(p, "Saída", x + 4.0, y, 12.5, FontWeight::Semibold, [110, 110, 118, 255]);
                y += 22.0;
                Self::card(p, x, y, w, 2.0 * ROW_H + 12.0);
                self.text.draw(p, "Volume de saída", x + 16.0, y + 13.0, FONT, FontWeight::Regular, [24, 24, 28, 255]);
                let (tx, tw) = (x + 170.0, w - 170.0 - 60.0);
                let ty = y + ROW_H / 2.0 + 2.0;
                fill_rounded(p, tx, ty - 3.0, tw, 6.0, 3.0, rgba(0, 0, 0, 30));
                let level = if muted { 0.0 } else { volume.clamp(0.0, 1.0) };
                fill_rounded(p, tx, ty - 3.0, tw * level, 6.0, 3.0, rgba(10, 100, 225, 255));
                if let Some(knob) = PathBuilder::from_circle(tx + tw * level, ty, 9.0) {
                    let mut paint = Paint::default();
                    paint.anti_alias = true;
                    paint.set_color(rgba(255, 255, 255, 255));
                    p.fill_path(&knob, &paint, FillRule::Winding, Transform::identity(), None);
                    paint.set_color(rgba(0, 0, 0, 60));
                    p.stroke_path(&knob, &paint, &Stroke::default(), Transform::identity(), None);
                }
                let pct = format!("{}%", (level * 100.0).round() as i32);
                self.text.draw(p, &pct, tx + tw + 14.0, y + 13.0, 12.5, FontWeight::Regular, [110, 110, 118, 255]);
                regions.push(((tx - 8.0, y, tw + 16.0, ROW_H), Action::VolumeTrack));
                Self::row_line(p, x, y + ROW_H + 6.0, w);
                self.switch_row(p, x, y + ROW_H + 6.0, w, "Sem som", "", muted, regions, Action::ToggleMute);
                y + 2.0 * ROW_H + 12.0 + 24.0
            }
            Section::Wallpaper => {
                let mut y = self.title(p, x, "Papel de parede");
                let current = Self::current_wallpaper();
                let gap = 14.0;
                let cols = (((w + gap) / (THUMB_W as f32 + gap)) as usize).max(1);
                for (i, path) in self.wallpapers.clone().iter().enumerate() {
                    let col = (i % cols) as f32;
                    let row = (i / cols) as f32;
                    let tx = x + col * (THUMB_W as f32 + gap);
                    let ty = y + row * (THUMB_H as f32 + 40.0);
                    let selected = *path == current;
                    if selected {
                        fill_rounded(p, tx - 4.0, ty - 4.0, THUMB_W as f32 + 8.0, THUMB_H as f32 + 8.0, 12.0, rgba(10, 100, 225, 255));
                    }
                    match self.thumbs.get(path).cloned().flatten().as_ref() {
                        Some(thumb) => {
                            // Miniatura com cantos arredondados (máscara).
                            let mut masked = Pixmap::new(THUMB_W, THUMB_H).unwrap_or_else(|| Pixmap::new(1, 1).unwrap());
                            masked.draw_pixmap(0, 0, thumb.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
                            if let Some(mask_path) = rounded_rect(0.0, 0.0, THUMB_W as f32, THUMB_H as f32, 9.0) {
                                if let Some(mut mask) = tiny_skia::Mask::new(THUMB_W, THUMB_H) {
                                    mask.fill_path(&mask_path, FillRule::Winding, true, Transform::identity());
                                    masked.apply_mask(&mask);
                                }
                            }
                            p.draw_pixmap(tx as i32, ty as i32, masked.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
                        }
                        None => fill_rounded(p, tx, ty, THUMB_W as f32, THUMB_H as f32, 9.0, rgba(0, 0, 0, 20)),
                    }
                    let name = match path {
                        None => "Gradiente do mide".to_string(),
                        Some(path) => path
                            .file_stem()
                            .map(|s| s.to_string_lossy().replace(['-', '_'], " "))
                            .unwrap_or_default(),
                    };
                    let name = fit(&mut self.text, &name, 12.0, THUMB_W as f32);
                    let nw = self.text.measure(&name, 12.0, FontWeight::Regular);
                    self.text.draw(p, &name, tx + (THUMB_W as f32 - nw) / 2.0, ty + THUMB_H as f32 + 8.0, 12.0, FontWeight::Regular, [60, 60, 66, 255]);
                    regions.push(((tx, ty, THUMB_W as f32, THUMB_H as f32 + 26.0), Action::Wallpaper(path.clone())));
                    // Imagem adicionada pelo usuário: × para tirar da lista (com o mouse em cima).
                    let user_added = path.as_ref().is_some_and(|p| p.starts_with(mide::places::user_backgrounds_dir()));
                    let tile_hovered = hover == Some(regions.len() - 1) || hover == Some(regions.len());
                    if user_added && tile_hovered {
                        let (bx, by) = (tx + THUMB_W as f32 - 12.0, ty + 12.0);
                        if let Some(c) = PathBuilder::from_circle(bx, by, 10.0) {
                            let mut paint = Paint::default();
                            paint.anti_alias = true;
                            paint.set_color(rgba(30, 30, 34, 200));
                            p.fill_path(&c, &paint, FillRule::Winding, Transform::identity(), None);
                        }
                        let mut pb = PathBuilder::new();
                        pb.move_to(bx - 3.5, by - 3.5);
                        pb.line_to(bx + 3.5, by + 3.5);
                        pb.move_to(bx + 3.5, by - 3.5);
                        pb.line_to(bx - 3.5, by + 3.5);
                        if let Some(xp) = pb.finish() {
                            let mut paint = Paint::default();
                            paint.anti_alias = true;
                            paint.set_color(rgba(255, 255, 255, 255));
                            p.stroke_path(&xp, &paint, &Stroke { width: 1.8, ..Default::default() }, Transform::identity(), None);
                        }
                    }
                    if user_added {
                        if let Some(path) = path {
                            regions.push(((tx + THUMB_W as f32 - 24.0, ty, 24.0, 24.0), Action::RemoveWallpaper(path.clone())));
                        }
                    }
                }
                // Último quadro: adicionar uma imagem.
                let i = self.wallpapers.len();
                let (tx, ty) = (x + (i % cols) as f32 * (THUMB_W as f32 + gap), y + (i / cols) as f32 * (THUMB_H as f32 + 40.0));
                fill_rounded(p, tx, ty, THUMB_W as f32, THUMB_H as f32, 9.0, rgba(0, 0, 0, if hover == Some(regions.len()) { 26 } else { 14 }));
                let pw = self.text.measure("+", 34.0, FontWeight::Regular);
                self.text.draw(p, "+", tx + (THUMB_W as f32 - pw) / 2.0, ty + 22.0, 34.0, FontWeight::Regular, [110, 110, 118, 255]);
                let label = "Adicionar imagem…";
                let lw = self.text.measure(label, 12.0, FontWeight::Regular);
                self.text.draw(p, label, tx + (THUMB_W as f32 - lw) / 2.0, ty + THUMB_H as f32 + 8.0, 12.0, FontWeight::Regular, [10, 100, 225, 255]);
                regions.push(((tx, ty, THUMB_W as f32, THUMB_H as f32 + 26.0), Action::AddWallpaper));
                let rows = (self.wallpapers.len() + 1).div_ceil(cols);
                y += rows as f32 * (THUMB_H as f32 + 40.0);
                y + 12.0
            }
            Section::Dock => {
                let mut y = self.title(p, x, "Dock");
                let preview = self.dock_size as f32 * 0.6;
                let card_h = ROW_H + 30.0 + preview + 12.0 + 14.0;
                Self::card(p, x, y, w, card_h);
                self.text.draw(p, "Tamanho dos ícones", x + 16.0, y + 13.0, FONT, FontWeight::Regular, [24, 24, 28, 255]);
                let (tx, tw) = (x + 190.0, w - 190.0 - 70.0);
                let ty = y + ROW_H / 2.0 + 2.0;
                let level = (self.dock_size - DOCK_MIN) as f32 / (DOCK_MAX - DOCK_MIN) as f32;
                fill_rounded(p, tx, ty - 3.0, tw, 6.0, 3.0, rgba(0, 0, 0, 30));
                fill_rounded(p, tx, ty - 3.0, tw * level, 6.0, 3.0, rgba(10, 100, 225, 255));
                if let Some(knob) = PathBuilder::from_circle(tx + tw * level, ty, 9.0) {
                    let mut paint = Paint::default();
                    paint.anti_alias = true;
                    paint.set_color(rgba(255, 255, 255, 255));
                    p.fill_path(&knob, &paint, FillRule::Winding, Transform::identity(), None);
                    paint.set_color(rgba(0, 0, 0, 60));
                    p.stroke_path(&knob, &paint, &Stroke::default(), Transform::identity(), None);
                }
                let label = format!("{} px", self.dock_size);
                self.text.draw(p, &label, tx + tw + 14.0, y + 13.0, 12.5, FontWeight::Regular, [110, 110, 118, 255]);
                self.text.draw(p, "Pequeno", tx - 4.0, ty + 12.0, 11.0, FontWeight::Regular, [130, 130, 138, 255]);
                let gw = self.text.measure("Grande", 11.0, FontWeight::Regular);
                self.text.draw(p, "Grande", tx + tw - gw + 4.0, ty + 12.0, 11.0, FontWeight::Regular, [130, 130, 138, 255]);
                regions.push(((tx - 8.0, y, tw + 16.0, ROW_H), Action::DockSize));
                // Prévia: uma dock em miniatura com ícones no tamanho escolhido.
                let py = y + ROW_H + 30.0;
                Self::row_line(p, x, py - 8.0, w);
                let size = preview;
                let n = 5.0;
                let dw = n * size + (n - 1.0) * 3.0 + 12.0;
                let dx = x + (w - dw) / 2.0;
                fill_rounded(p, dx, py, dw, size + 12.0, 10.0, rgba(0, 0, 0, 18));
                let colors = [(255, 69, 58), (255, 159, 10), (48, 209, 88), (10, 132, 255), (191, 90, 242)];
                for (i, c) in colors.iter().enumerate() {
                    fill_rounded(p, dx + 6.0 + i as f32 * (size + 3.0), py + 6.0, size, size, size * 0.22, rgba(c.0, c.1, c.2, 255));
                }
                y += card_h + 14.0;
                self.text.draw(p, "Para fixar um app, clique com o botão direito no ícone dele na dock e escolha \"Manter na Dock\".", x + 4.0, y, 12.0, FontWeight::Regular, [130, 130, 138, 255]);
                y + 40.0
            }
            Section::Effects => {
                let mut y = self.title(p, x, "Efeitos");
                let h = EFFECTS.len() as f32 * (ROW_H + 12.0);
                Self::card(p, x, y, w, h);
                for (i, (key, label, detail)) in EFFECTS.iter().enumerate() {
                    let ry = y + i as f32 * (ROW_H + 12.0);
                    if i > 0 {
                        Self::row_line(p, x, ry, w);
                    }
                    self.switch_row(p, x, ry, w, label, detail, Self::effect_on(key), regions, Action::Effect(i));
                }
                y += h + 14.0;
                self.text.draw(p, "As mudanças valem na hora.", x + 4.0, y, 12.0, FontWeight::Regular, [130, 130, 138, 255]);
                y + 40.0
            }
            Section::Keyboard => {
                let mut y = self.title(p, x, "Atalhos de teclado");
                let bindings = self.bindings.clone();
                let rows = bindings.len().max(1);
                Self::card(p, x, y, w, rows as f32 * ROW_H);
                for (i, (combo, action)) in bindings.iter().enumerate() {
                    let ry = y + i as f32 * ROW_H;
                    if i > 0 {
                        Self::row_line(p, x, ry, w);
                    }
                    let label = describe_action(action);
                    let label = fit(&mut self.text, &label, FONT, w - 220.0);
                    self.text.draw(p, &label, x + 16.0, ry + (ROW_H - FONT * 1.25) / 2.0, FONT, FontWeight::Regular, [24, 24, 28, 255]);
                    let keys = pretty_combo(combo);
                    let kw = self.text.measure(&keys, 12.5, FontWeight::Semibold);
                    fill_rounded(p, x + w - 24.0 - kw, ry + 9.0, kw + 12.0, 22.0, 6.0, rgba(0, 0, 0, 14));
                    self.text.draw(p, &keys, x + w - 18.0 - kw, ry + 12.0, 12.5, FontWeight::Semibold, [60, 60, 66, 255]);
                }
                y += rows as f32 * ROW_H + 14.0;
                self.text.draw(p, "Para mudar, edite a seção [keybindings] de ~/.config/mide/config.toml.", x + 4.0, y, 12.0, FontWeight::Regular, [130, 130, 138, 255]);
                y + 40.0
            }
            Section::About => {
                let mut y = self.title(p, x, "Sobre");
                let info = self.info.clone();
                Self::card(p, x, y, w, info.len() as f32 * ROW_H);
                for (i, (label, value)) in info.iter().enumerate() {
                    let ry = y + i as f32 * ROW_H;
                    if i > 0 {
                        Self::row_line(p, x, ry, w);
                    }
                    self.text.draw(p, label, x + 16.0, ry + (ROW_H - FONT * 1.25) / 2.0, FONT, FontWeight::Regular, [24, 24, 28, 255]);
                    let value = fit(&mut self.text, value, 12.5, w - 180.0);
                    let vw = self.text.measure(&value, 12.5, FontWeight::Regular);
                    self.text.draw(p, &value, x + w - 16.0 - vw, ry + 12.0, 12.5, FontWeight::Regular, [110, 110, 118, 255]);
                }
                y += info.len() as f32 * ROW_H + 14.0;
                let version = format!("mide {}", env!("CARGO_PKG_VERSION"));
                self.text.draw(p, &version, x + 4.0, y, 12.0, FontWeight::Regular, [130, 130, 138, 255]);
                y + 40.0
            }
        }
    }

    /// Retângulo da folha do seletor, centrada na janela.
    fn picker_rect(&self) -> (f32, f32, f32, f32) {
        let (w, h) = (self.width as f32, self.height as f32);
        let pw = (w - 60.0).min(680.0);
        let ph = (h - 50.0).min(480.0);
        ((w - pw) / 2.0, (h - ph) / 2.0, pw, ph)
    }

    /// Seletor de imagens: lugares à esquerda, pastas e miniaturas à direita.
    fn draw_picker(&mut self, pixmap: &mut Pixmap, hover: Option<usize>) {
        let (w, h) = (self.width as f32, self.height as f32);
        fill_rounded(pixmap, 0.0, 0.0, w, h, 0.0, rgba(0, 0, 0, 70));
        self.regions.push(((0.0, 0.0, w, h), Action::PickerSheet));
        let (px, py, pw, ph) = self.picker_rect();
        for (grow, alpha) in [(10.0, 10u8), (5.0, 18), (2.0, 30)] {
            fill_rounded(pixmap, px - grow, py - grow + 6.0, pw + 2.0 * grow, ph + 2.0 * grow, 14.0 + grow, rgba(0, 0, 0, alpha));
        }
        fill_rounded(pixmap, px, py, pw, ph, 14.0, rgba(246, 246, 248, 255));
        // Lugares.
        let side = 160.0;
        fill_rounded(pixmap, px, py, side, ph, 14.0, rgba(232, 232, 236, 255));
        fill_rounded(pixmap, px + side - 14.0, py, 14.0, ph, 0.0, rgba(232, 232, 236, 255));
        self.text.draw(pixmap, "Lugares", px + 16.0, py + 16.0, 11.5, FontWeight::Semibold, [130, 130, 138, 255]);
        let current = self.picker.as_ref().map(|p| p.dir.clone()).unwrap_or_default();
        let places = [
            ("Imagens", mide::places::pictures_dir()),
            ("Downloads", mide::places::downloads_dir()),
            ("Pasta pessoal", mide::places::home_dir()),
        ];
        for (i, (name, dir)) in places.iter().enumerate() {
            let y = py + 38.0 + i as f32 * 30.0;
            let selected = *dir == current;
            let hovered = hover == Some(self.regions.len());
            if selected || hovered {
                fill_rounded(pixmap, px + 8.0, y, side - 16.0, 28.0, 7.0, if selected { rgba(10, 100, 225, 255) } else { rgba(0, 0, 0, 16) });
            }
            let color = if selected { [255, 255, 255, 255] } else { [30, 30, 34, 255] };
            self.text.draw(pixmap, name, px + 18.0, y + (28.0 - FONT * 1.25) / 2.0, FONT, FontWeight::Regular, color);
            self.regions.push(((px + 8.0, y, side - 16.0, 28.0), Action::PickerDir(dir.clone())));
        }
        // Topo: voltar para a pasta de cima e o nome da pasta.
        let cx = px + side + 16.0;
        let cw = pw - side - 32.0;
        let up_hover = hover == Some(self.regions.len());
        fill_rounded(pixmap, cx, py + 12.0, 30.0, 26.0, 7.0, rgba(0, 0, 0, if up_hover { 26 } else { 12 }));
        self.text.draw(pixmap, "‹", cx + 10.0, py + 12.0, 18.0, FontWeight::Semibold, [60, 60, 66, 255]);
        self.regions.push(((cx, py + 12.0, 30.0, 26.0), Action::PickerUp));
        let title = current.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| current.display().to_string());
        let title = fit(&mut self.text, &title, 15.0, cw - 50.0);
        self.text.draw(pixmap, &title, cx + 42.0, py + 15.0, 15.0, FontWeight::Bold, [24, 24, 28, 255]);
        // Grade de pastas e imagens, com rolagem.
        let (gx, gy, gh) = (cx, py + 52.0, ph - 52.0 - 52.0);
        let tile_w = PICK_W as f32 + 16.0;
        let tile_h = PICK_H as f32 + 34.0;
        let cols = ((cw / tile_w) as usize).max(1);
        let (entries, scroll) = match self.picker.as_ref() {
            Some(p) => (p.entries.clone(), p.scroll),
            None => return,
        };
        let rows = entries.len().div_ceil(cols);
        let max_scroll = (rows as f32 * tile_h - gh).max(0.0);
        let scroll = scroll.clamp(0.0, max_scroll);
        if let Some(p) = self.picker.as_mut() {
            p.scroll = scroll;
        }
        let mut grid = Pixmap::new(cw as u32, gh.max(1.0) as u32).unwrap_or_else(|| Pixmap::new(1, 1).unwrap());
        if entries.is_empty() {
            self.text.draw(&mut grid, "Nenhuma imagem nesta pasta", 8.0, 12.0, FONT, FontWeight::Regular, [130, 130, 138, 255]);
        }
        let folder_icon = load_themed_icon(&["folder"], 56);
        for (i, (path, is_dir)) in entries.iter().enumerate() {
            let tx = (i % cols) as f32 * tile_w;
            let ty = (i / cols) as f32 * tile_h - scroll;
            if ty + tile_h < 0.0 || ty > gh {
                continue;
            }
            let screen_rect = (gx + tx, gy + ty, tile_w, tile_h);
            let hovered = hover == Some(self.regions.len());
            if hovered {
                fill_rounded(&mut grid, tx + 2.0, ty + 2.0, tile_w - 4.0, tile_h - 4.0, 8.0, rgba(0, 0, 0, 16));
            }
            let (ix, iy) = (tx + 8.0, ty + 6.0);
            if *is_dir {
                if let Some(icon) = &folder_icon {
                    grid.draw_pixmap((ix + (PICK_W as f32 - 56.0) / 2.0) as i32, (iy + (PICK_H as f32 - 56.0) / 2.0) as i32, icon.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
                }
            } else {
                let thumb = self.picker.as_ref().and_then(|p| p.thumbs.get(path).cloned()).flatten();
                match thumb {
                    Some(t) => grid.draw_pixmap(ix as i32, iy as i32, t.as_ref(), &PixmapPaint::default(), Transform::identity(), None),
                    None => fill_rounded(&mut grid, ix, iy, PICK_W as f32, PICK_H as f32, 6.0, rgba(0, 0, 0, 18)),
                }
            }
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let name = fit(&mut self.text, &name, 11.5, tile_w - 8.0);
            let nw = self.text.measure(&name, 11.5, FontWeight::Regular);
            self.text.draw(&mut grid, &name, tx + (tile_w - nw) / 2.0, iy + PICK_H as f32 + 6.0, 11.5, FontWeight::Regular, [40, 40, 46, 255]);
            let action = if *is_dir { Action::PickerDir(path.clone()) } else { Action::PickerChoose(path.clone()) };
            // Só a parte visível da grade é clicável.
            let (rx, ry, rw, rh) = screen_rect;
            let top = ry.max(gy);
            let bottom = (ry + rh).min(gy + gh);
            if bottom > top {
                self.regions.push(((rx, top, rw, bottom - top), action));
            }
        }
        pixmap.draw_pixmap(gx as i32, gy as i32, grid.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
        // Rodapé: dica e Cancelar.
        fill_rounded(pixmap, px + side, py + ph - 52.0, pw - side, 1.0, 0.0, rgba(0, 0, 0, 22));
        self.text.draw(pixmap, "Escolha uma imagem (PNG ou JPEG) para usar como papel de parede.", cx, py + ph - 33.0, 11.5, FontWeight::Regular, [130, 130, 138, 255]);
        let cancel_hover = hover == Some(self.regions.len());
        let (bx, by) = (px + pw - 110.0, py + ph - 40.0);
        fill_rounded(pixmap, bx, by, 94.0, 28.0, 7.0, rgba(0, 0, 0, if cancel_hover { 30 } else { 16 }));
        let cw2 = self.text.measure("Cancelar", FONT, FontWeight::Regular);
        self.text.draw(pixmap, "Cancelar", bx + (94.0 - cw2) / 2.0, by + 5.0, FONT, FontWeight::Regular, [30, 30, 34, 255]);
        self.regions.push(((bx, by, 94.0, 28.0), Action::PickerCancel));
    }

    fn open_picker(&mut self, dir: PathBuf) {
        let entries = mide::places::folder_images(&dir);
        tracing::info!("seletor de imagens: {} ({} item(ns))", dir.display(), entries.len());
        let images: Vec<PathBuf> = entries.iter().filter(|(_, d)| !d).map(|(p, _)| p.clone()).take(300).collect();
        let thumbs = self.picker.take().map(|p| p.thumbs).unwrap_or_default();
        if let Some(tx) = self.tx.clone() {
            let missing: Vec<PathBuf> = images.into_iter().filter(|p| !thumbs.contains_key(p)).collect();
            load_picker_thumbs(tx, missing);
        }
        self.picker = Some(Picker { dir, entries, thumbs, scroll: 0.0 });
    }

    /// Relê a lista de papéis de parede e carrega as miniaturas novas.
    fn refresh_wallpapers(&mut self) {
        self.wallpapers = wallpaper_list();
        let missing: Vec<Option<PathBuf>> = self.wallpapers.iter().filter(|p| !self.thumbs.contains_key(*p)).cloned().collect();
        if let Some(tx) = self.tx.clone() {
            load_thumbs(tx, missing);
        }
    }

    /// Topo e altura da alça da barra de rolagem.
    fn thumb(&self) -> (f32, f32) {
        let h = self.height as f32;
        let track_h = h - 8.0;
        let max_scroll = (self.content_h - h).max(1.0);
        let thumb_h = (track_h * h / self.content_h.max(h)).max(32.0);
        let y = 4.0 + (self.scroll / max_scroll).clamp(0.0, 1.0) * (track_h - thumb_h);
        (y, thumb_h)
    }

    /// Rolagem para a alça com o topo em `thumb_top`.
    fn scroll_to_thumb(&mut self, thumb_top: f32) {
        let h = self.height as f32;
        let (_, thumb_h) = self.thumb();
        let track = (h - 8.0 - thumb_h).max(1.0);
        let max_scroll = (self.content_h - h).max(0.0);
        self.scroll = ((thumb_top - 4.0) / track).clamp(0.0, 1.0) * max_scroll;
        self.dirty = true;
    }

    fn region_at(&self, x: f64, y: f64) -> Option<usize> {
        let (x, y) = (x as f32, y as f32);
        // A barra lateral fica por cima do conteúdo: procura de trás para frente.
        self.regions.iter().rposition(|((rx, ry, rw, rh), _)| x >= *rx && x <= rx + rw && y >= *ry && y <= ry + rh)
    }

    fn volume_from_x(&self, x: f64) -> Option<f32> {
        let ((rx, _, rw, _), _) = self.regions.iter().find(|(_, a)| *a == Action::VolumeTrack)?;
        Some(((x as f32 - (rx + 8.0)) / (rw - 16.0)).clamp(0.0, 1.0))
    }

    fn dock_size_from_x(&self, x: f64) -> Option<u32> {
        let ((rx, _, rw, _), _) = self.regions.iter().find(|(_, a)| *a == Action::DockSize)?;
        let k = ((x as f32 - (rx + 8.0)) / (rw - 16.0)).clamp(0.0, 1.0);
        Some(DOCK_MIN + (k * (DOCK_MAX - DOCK_MIN) as f32).round() as u32)
    }

    /// Grava o tamanho dos ícones; a dock percebe a mudança no arquivo e se ajusta.
    fn save_dock_size(&self) {
        let value = (self.dock_size != DOCK_DEFAULT).then_some(Value::Int(self.dock_size as i64));
        match settings_file::set("dock", "icon_size", value) {
            Ok(()) => tracing::info!("tamanho dos ícones da dock: {} px", self.dock_size),
            Err(err) => tracing::warn!("tamanho da dock não gravado: {err}"),
        }
    }

    fn act(&mut self, action: Action, x: f64) {
        tracing::info!("configurações: {action:?}");
        match action {
            Action::Select(section) => {
                self.section = section;
                self.scroll = 0.0;
                if section == Section::Keyboard {
                    self.load_bindings();
                }
            }
            Action::ToggleWifi => {
                if let Some((on, list)) = self.status.wifi.clone() {
                    self.status.wifi = Some((!on, list));
                    system::set_wifi(!on);
                }
            }
            Action::Connect(ssid) => system::connect_network(&ssid),
            Action::ToggleBluetooth => {
                if let Some((on, list)) = self.status.bluetooth.clone() {
                    self.status.bluetooth = Some((!on, list));
                    system::set_bluetooth(!on);
                }
            }
            Action::Device(i) => {
                if let Some(device) = self.status.bluetooth.as_ref().and_then(|(_, d)| d.get(i)).cloned() {
                    system::toggle_device(&device);
                }
            }
            Action::VolumeTrack => {
                if let Some(value) = self.volume_from_x(x) {
                    let was_muted = self.status.volume.is_some_and(|(_, m)| m);
                    self.status.volume = Some((value, false));
                    system::set_volume(value);
                    if was_muted {
                        system::set_muted(false);
                    }
                    self.dragging_volume = true;
                }
            }
            Action::ToggleMute => {
                if let Some((v, m)) = self.status.volume {
                    self.status.volume = Some((v, !m));
                    system::set_muted(!m);
                }
            }
            Action::Wallpaper(path) => {
                let value = path.as_ref().map(|p| Value::Text(p.to_string_lossy().into_owned()));
                match settings_file::set("desktop", "wallpaper", value) {
                    Ok(()) => {
                        tracing::info!("papel de parede escolhido: {}", path.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "gradiente do mide".into()));
                        ask_compositor("reload");
                    }
                    Err(err) => tracing::warn!("papel de parede não gravado: {err}"),
                }
            }
            Action::ScrollBar | Action::PickerSheet => {}
            Action::AddWallpaper => self.open_picker(mide::places::pictures_dir()),
            Action::PickerDir(dir) => self.open_picker(dir),
            Action::PickerUp => {
                if let Some(parent) = self.picker.as_ref().and_then(|p| p.dir.parent().map(|d| d.to_path_buf())) {
                    self.open_picker(parent);
                }
            }
            Action::PickerCancel => self.picker = None,
            Action::PickerChoose(image) => {
                self.picker = None;
                match mide::places::add_background(&image) {
                    Ok(copy) => {
                        tracing::info!("papel de parede adicionado: {}", copy.display());
                        self.refresh_wallpapers();
                        self.act(Action::Wallpaper(Some(copy)), x);
                    }
                    Err(err) => tracing::warn!("não deu para adicionar {}: {err}", image.display()),
                }
            }
            Action::RemoveWallpaper(path) => {
                if Self::current_wallpaper().as_ref() == Some(&path) {
                    self.act(Action::Wallpaper(None), x);
                }
                match fs::remove_file(&path) {
                    Ok(()) => tracing::info!("papel de parede removido da lista: {}", path.display()),
                    Err(err) => tracing::warn!("não deu para remover {}: {err}", path.display()),
                }
                self.refresh_wallpapers();
            }
            Action::DockSize => {
                if let Some(size) = self.dock_size_from_x(x) {
                    self.dock_size = size;
                    self.dragging_dock = true;
                }
            }
            Action::Effect(i) => {
                let (key, label, _) = EFFECTS[i];
                let on = !Self::effect_on(key);
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
        self.dirty = true;
    }
}

/// Corta o texto com reticências.
fn fit(text: &mut Text, value: &str, size: f32, max: f32) -> String {
    if text.measure(value, size, FontWeight::Regular) <= max {
        return value.to_string();
    }
    let mut chars: Vec<char> = value.chars().collect();
    while !chars.is_empty() {
        chars.pop();
        let candidate = format!("{}…", chars.iter().collect::<String>().trim_end());
        if text.measure(&candidate, size, FontWeight::Regular) <= max {
            return candidate;
        }
    }
    "…".into()
}

/// "ctrl+shift+space" vira "Ctrl ⇧ Espaço".
fn pretty_combo(combo: &str) -> String {
    combo
        .split('+')
        .map(|k| match k {
            "ctrl" => "Ctrl".to_string(),
            "shift" => "⇧".to_string(),
            "alt" => "Alt".to_string(),
            "super" => "Super".to_string(),
            "space" => "Espaço".to_string(),
            "escape" => "Esc".to_string(),
            "return" => "Enter".to_string(),
            other => other.to_uppercase(),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Ação de um atalho em português.
fn describe_action(action: &str) -> String {
    match action {
        "close_window" => "Fechar a janela".into(),
        "quit" => "Encerrar a sessão do mide".into(),
        "restore_minimized" => "Restaurar a última janela minimizada".into(),
        "toggle_launcher" => "Abrir ou fechar o launcher".into(),
        other => match other.strip_prefix("launch:") {
            Some(cmd) => format!("Abrir: {cmd}"),
            None => other.into(),
        },
    }
}

/// Interruptor no estilo do macOS.
fn draw_switch(p: &mut Pixmap, x: f32, y: f32, on: bool) {
    let fill = if on { rgba(10, 120, 240, 255) } else { rgba(0, 0, 0, 40) };
    fill_rounded(p, x, y, 40.0, 22.0, 11.0, fill);
    let kx = if on { x + 29.0 } else { x + 11.0 };
    if let Some(knob) = PathBuilder::from_circle(kx, y + 11.0, 9.0) {
        let mut paint = Paint::default();
        paint.anti_alias = true;
        paint.set_color(rgba(255, 255, 255, 255));
        p.fill_path(&knob, &paint, FillRule::Winding, Transform::identity(), None);
    }
}

impl CompositorHandler for Settings {
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: i32) {}
    fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: wl_output::Transform) {}
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {}
    fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
    fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
}

impl OutputHandler for Settings {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl WindowHandler for Settings {
    fn request_close(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &Window) {
        self.exit = true;
    }

    fn configure(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &Window, configure: WindowConfigure, _: u32) {
        let (w, h) = configure.new_size;
        if let Some(w) = w {
            self.width = w.get().max(MIN_W);
        }
        if let Some(h) = h {
            self.height = h.get().max(MIN_H);
        }
        self.configured = true;
        self.dirty = true;
    }
}

impl SeatHandler for Settings {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
    fn new_capability(&mut self, _: &Connection, qh: &QueueHandle<Self>, seat: wl_seat::WlSeat, capability: Capability) {
        if capability == Capability::Pointer && self.pointer.is_none() {
            self.pointer = self.seat_state.get_pointer(qh, &seat).ok();
        }
    }
    fn remove_capability(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat, capability: Capability) {
        if capability == Capability::Pointer {
            if let Some(pointer) = self.pointer.take() {
                pointer.release();
            }
        }
    }
    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl PointerHandler for Settings {
    fn pointer_frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_pointer::WlPointer, events: &[PointerEvent]) {
        for event in events {
            if &event.surface != self.window.wl_surface() {
                continue;
            }
            let (x, y) = event.position;
            match event.kind {
                PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => {
                    if let Some(grab) = self.dragging_scroll {
                        self.scroll_to_thumb(y as f32 - grab);
                        continue;
                    }
                    if self.dragging_dock {
                        if let Some(size) = self.dock_size_from_x(x) {
                            if size != self.dock_size {
                                self.dock_size = size;
                                self.dirty = true;
                            }
                        }
                        continue;
                    }
                    if self.dragging_volume {
                        if let Some(value) = self.volume_from_x(x) {
                            self.status.volume = Some((value, false));
                            self.dirty = true;
                        }
                        continue;
                    }
                    let hover = self.region_at(x, y);
                    if hover != self.hover {
                        self.hover = hover;
                        self.dirty = true;
                    }
                }
                PointerEventKind::Press { button, .. } if button == BTN_LEFT => {
                    if let Some(action) = self.region_at(x, y).map(|i| self.regions[i].1.clone()) {
                        if action == Action::ScrollBar {
                            // Na alça: arrasta. No trilho: pula para o ponto e arrasta dali.
                            let (top, thumb_h) = self.thumb();
                            let grab = if (y as f32) >= top && (y as f32) <= top + thumb_h { y as f32 - top } else { thumb_h / 2.0 };
                            self.dragging_scroll = Some(grab);
                            self.scroll_to_thumb(y as f32 - grab);
                            continue;
                        }
                        self.act(action, x);
                    }
                }
                PointerEventKind::Release { button, .. } if button == BTN_LEFT => {
                    if self.dragging_scroll.take().is_some() {
                        self.dirty = true;
                    }
                    if std::mem::take(&mut self.dragging_dock) {
                        self.save_dock_size();
                    }
                    if std::mem::take(&mut self.dragging_volume) {
                        if let Some((value, _)) = self.status.volume {
                            system::set_volume(value);
                        }
                    }
                }
                PointerEventKind::Axis { vertical, .. } => {
                    let delta = if vertical.discrete != 0 { vertical.discrete as f64 * 40.0 } else { vertical.absolute };
                    if let Some(picker) = self.picker.as_mut() {
                        picker.scroll = (picker.scroll + delta as f32).max(0.0);
                        self.dirty = true;
                        continue;
                    }
                    if x as f32 > SIDEBAR_W && delta != 0.0 {
                        self.scroll = (self.scroll + delta as f32).max(0.0);
                        self.dirty = true;
                    }
                }
                _ => {}
            }
        }
    }
}

impl ShmHandler for Settings {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

delegate_registry!(Settings);
smithay_client_toolkit::delegate_dispatch2!(Settings);

impl ProvidesRegistryState for Settings {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }

    registry_handlers![OutputState, SeatState];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atalhos_em_portugues() {
        assert_eq!(pretty_combo("ctrl+shift+space"), "Ctrl ⇧ Espaço");
        assert_eq!(pretty_combo("ctrl+shift+t"), "Ctrl ⇧ T");
        assert_eq!(describe_action("toggle_launcher"), "Abrir ou fechar o launcher");
        assert_eq!(describe_action("launch:foot"), "Abrir: foot");
    }
}
