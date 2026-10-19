//! Dock do mide: uma faixa flutuante na parte de baixo da tela, como no macOS.
//!
//! É um cliente Wayland comum, com janela de camada (`wlr-layer-shell`). Mostra os
//! apps fixados (de `~/.config/mide/dock.toml`, pelo id do `.desktop`) e, depois de
//! um separador, os apps abertos que não estão fixados. Conversa com o compositor
//! pelo canal de controle (`<socket>.ctl`) para saber quais apps estão abertos e
//! para focar ou restaurar janelas.
//!
//! - Clique esquerdo: foca ou restaura o app; se não estiver aberto, abre.
//! - Arrastar um ícone fixado: reordena (salva ao soltar).
//! - Clique direito: fixa ou desafixa o app (salva na hora).
//! - Ctrl+clique: abre uma instância nova, mesmo com o app já aberto.
//!
//! No fim da dock, depois de outro separador, ficam Downloads e Lixeira:
//! - Downloads: clique mostra os itens mais recentes; clique num item abre.
//! - Lixeira: clique abre a Lixeira no gerenciador de arquivos; clique direito tem
//!   "Esvaziar Lixeira…", que pede confirmação antes de apagar.

use std::{
    collections::{HashMap, HashSet},
    error::Error,
    fs,
    num::NonZeroU32,
    path::PathBuf,
    process::Command,
    time::Duration,
};

use mide::{
    control_client::ask_compositor,
    icons::{load_icon, load_themed_icon},
    places,
    ui::text::{FontWeight, Text},
};
use serde::Deserialize;
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    reexports::{
        calloop::{
            timer::{TimeoutAction, Timer},
            EventLoop, LoopHandle,
        },
        calloop_wayland_source::WaylandSource,
    },
    delegate_registry,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        pointer::{PointerEvent, PointerEventKind, PointerHandler},
        Capability, SeatHandler, SeatState,
    },
    shell::{
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
        WaylandSurface,
    },
    shm::{slot::SlotPool, Shm, ShmHandler},
};
use tiny_skia::{Color, FillRule, GradientStop, LinearGradient, Paint, PathBuilder, Pixmap, Point as SkPoint, SpreadMode, Stroke, Transform};
use smithay_client_toolkit::reexports::client::{
    globals::registry_queue_init,
    protocol::{wl_output, wl_pointer, wl_seat, wl_shm, wl_surface},
    Connection, QueueHandle,
};

/// Tamanho padrão dos ícones; o usuário muda em Configurações → Dock
/// (`[dock] icon_size` no `config.toml`, de 32 a 64).
const DEFAULT_ICON_SIZE: u32 = 40;
static ICON_SIZE_NOW: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(DEFAULT_ICON_SIZE);

/// Tamanho atual dos ícones da dock.
fn icon_size() -> u32 {
    ICON_SIZE_NOW.load(std::sync::atomic::Ordering::Relaxed)
}

/// Tamanho dos ícones gravado nas Configurações (ou o padrão).
fn configured_icon_size() -> u32 {
    match mide::settings_file::get("dock", "icon_size") {
        Some(mide::settings_file::Value::Int(n)) => (n.clamp(32, 64)) as u32,
        _ => DEFAULT_ICON_SIZE,
    }
}

/// O compositor está desenhando o vidro líquido atrás da dock? Precisa do desfoque e
/// do vidro líquido ligados em Configurações → Efeitos (os dois vêm ligados).
fn liquid_glass() -> bool {
    let on = |key| !matches!(mide::settings_file::get("effects", key), Some(mide::settings_file::Value::Bool(false)));
    on("dock_blur") && on("liquid_glass")
}

/// Ocultar a dock automaticamente? (`[dock] autohide`, desligado como no macOS.)
fn configured_autohide() -> bool {
    matches!(mide::settings_file::get("dock", "autohide"), Some(mide::settings_file::Value::Bool(true)))
}

/// Faixa da dock escondida que fica na tela (invisível) para o ponteiro achar.
const PEEK: i32 = 2;
/// Duração do deslizar ao mostrar e esconder, e a espera antes de esconder.
const SLIDE: Duration = Duration::from_millis(200);
const HIDE_DELAY: Duration = Duration::from_millis(450);

/// Margem de baixo com a dock toda escondida (negativa: só `PEEK` pixels na tela).
fn hidden_margin() -> i32 {
    PEEK - dock_height() as i32
}

/// Espaço reservado para a dock (sem ocultar): a altura dela e um respiro de 2 px
/// entre ela e as janelas maximizadas.
fn reserved_zone() -> i32 {
    dock_height() as i32 + 2
}

/// Altura da dock para o tamanho atual dos ícones.
fn dock_height() -> u32 {
    icon_size() + 2 * PADDING + BOTTOM_EXTRA
}
// Medidas próximas da dock do macOS: ícones quase encostados e borda justa.
const ICON_GAP: u32 = 3;
const PADDING: u32 = 7;
/// Espaço extra de cada lado do separador entre fixados e abertos.
const SEPARATOR_SPACE: u32 = 8;
/// Respiro extra embaixo dos ícones, onde fica o ponto de app aberto.
const BOTTOM_EXTRA: u32 = 3;
/// Raio dos cantos da faixa, como na dock do macOS (não é uma pílula).
const DOCK_RADIUS: f32 = 15.0;
const BOTTOM_MARGIN: i32 = 8;
const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
/// Distância mínima em pixels para um clique virar arrasto.
const DRAG_THRESHOLD: f64 = 6.0;

/// Apps fixados quando o usuário não definiu nenhum.
/// Como no macOS, as Configurações do Sistema vêm fixadas.
const DEFAULT_PINNED: &[&str] = &["org.codeberg.dnkl.foot", "org.gnome.Calculator", "mide-settings"];

#[derive(Deserialize, Default)]
#[serde(default)]
struct DockConfig {
    pinned: Vec<String>,
}

/// Um app na dock, resolvido a partir do `.desktop`.
#[derive(Clone)]
struct DockApp {
    id: String,
    name: String,
    exec: String,
    icon: Option<Pixmap>,
    /// Ids que podem aparecer como app_id da janela: o id do `.desktop`, a última
    /// parte dele e o `StartupWMClass`. Em minúsculas.
    match_ids: Vec<String>,
    /// Fixado na dock (fica mesmo fechado) ou só aberto agora.
    pinned: bool,
}

/// Catálogo dos `.desktop` instalados, com os ícones carregados sob demanda.
struct Catalog {
    locales: Vec<String>,
    entries: Vec<freedesktop_desktop_entry::DesktopEntry>,
    icons: HashMap<String, Option<Pixmap>>,
}

impl Catalog {
    fn load() -> Self {
        let locales = freedesktop_desktop_entry::get_languages_from_env();
        let entries = freedesktop_desktop_entry::desktop_entries(&locales);
        Self {
            locales,
            entries,
            icons: HashMap::new(),
        }
    }

    /// App do `.desktop` com o id dado, ou cujo app_id da janela casa com `app_id`.
    fn find(&mut self, wanted: &str, pinned: bool) -> Option<DockApp> {
        let wanted = wanted.to_lowercase();
        let entry = self
            .entries
            .iter()
            .find(|e| e.id().to_lowercase() == wanted)
            .or_else(|| self.entries.iter().find(|e| match_ids(e).contains(&wanted)))?
            .clone();
        let id = entry.id().to_string();
        let icon = self
            .icons
            .entry(id.clone())
            .or_insert_with(|| entry.icon().and_then(|name| load_icon(name, icon_size())))
            .clone();
        Some(DockApp {
            name: entry
                .name(&self.locales)
                .map(|n| n.into_owned())
                .unwrap_or_else(|| id.clone()),
            exec: entry.exec().unwrap_or("").to_string(),
            icon,
            match_ids: match_ids(&entry),
            pinned,
            id,
        })
    }

    /// App sem `.desktop` (como a janela "Sobre" do mide): aparece na dock com o
    /// título da janela e um ícone genérico do tema, para poder focar e restaurar.
    fn fallback(&mut self, app_id: &str, title: &str) -> DockApp {
        let icon = self
            .icons
            .entry(format!("fallback:{app_id}"))
            .or_insert_with(|| {
                let names: &[&str] = if app_id == "mide-settings" {
                    &["preferences-system", "preferences-desktop", "application-x-executable"]
                } else if app_id.starts_with("mide-") {
                    &["computer", "help-about", "application-x-executable"]
                } else {
                    &["application-x-executable", "applications-other"]
                };
                load_themed_icon(names, icon_size())
            })
            .clone();
        DockApp {
            id: app_id.to_string(),
            name: if title.is_empty() { app_id.to_string() } else { title.to_string() },
            exec: String::new(),
            icon,
            match_ids: vec![app_id.to_lowercase()],
            pinned: false,
        }
    }
}

/// Ids que identificam o app de uma janela, em minúsculas.
fn match_ids(entry: &freedesktop_desktop_entry::DesktopEntry) -> Vec<String> {
    let id = entry.id();
    let mut ids = vec![id.to_lowercase()];
    if let Some(last) = id.rsplit('.').next() {
        ids.push(last.to_lowercase());
    }
    if let Some(class) = entry.startup_wm_class() {
        ids.push(class.to_lowercase());
    }
    ids.dedup();
    ids
}

/// Clique esquerdo em andamento sobre um ícone.
struct Press {
    index: usize,
    start_x: f64,
    dragged: bool,
}

fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "mide_dock=info".into()),
        )
        .init();

    let conn = Connection::connect_to_env()?;
    let (globals, event_queue) = registry_queue_init(&conn)?;
    let qh = event_queue.handle();

    let compositor = CompositorState::bind(&globals, &qh)?;
    let layer_shell = LayerShell::bind(&globals, &qh)?;
    let shm = Shm::bind(&globals, &qh)?;

    let surface = compositor.create_surface(&qh);
    let layer = layer_shell.create_layer_surface(&qh, surface, Layer::Top, Some("mide-dock"), None);
    let trash_items = places::trash_count(&places::trash_dir());
    layer.set_anchor(Anchor::BOTTOM);
    layer.set_keyboard_interactivity(KeyboardInteractivity::None);
    ICON_SIZE_NOW.store(configured_icon_size(), std::sync::atomic::Ordering::Relaxed);
    // Sem ocultar, a dock reserva o espaço dela: janelas maximizadas param acima.
    let autohide = configured_autohide();
    layer.set_margin(0, 0, if autohide { hidden_margin() } else { BOTTOM_MARGIN }, 0);
    layer.set_exclusive_zone(if autohide { 0 } else { reserved_zone() });
    let mut catalog = Catalog::load();
    let items: Vec<DockApp> = pinned_ids()
        .iter()
        .filter_map(|id| catalog.find(id, true))
        .collect();
    let width = dock_width(&items);
    layer.set_size(width, dock_height());
    layer.commit();

    let pool = SlotPool::new((width * dock_height() * 4) as usize, &shm)?;
    let mut dock = Dock {
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),
        shm,
        pool,
        layer,
        width,
        height: dock_height(),
        first_configure: true,
        exit: false,
        items,
        catalog,
        press: None,
        pointer: None,
        pointer_x: 0.0,
        running: HashSet::new(),
        qh: qh.clone(),
        last_report: String::new(),
        compositor,
        layer_shell,
        downloads_icon: load_themed_icon(&["folder-download", "folder-downloads", "folder"], icon_size()),
        trash_empty_icon: load_themed_icon(&["user-trash", "user-trash-empty"], icon_size()),
        trash_full_icon: load_themed_icon(&["user-trash-full", "user-trash"], icon_size()),
        trash_items,
        menu: None,
        text: None,
        text_ready: Some(warm_text()),
        file_icons: HashMap::new(),
        screen: (0, 0),
        loop_handle: None,
        hover_index: None,
        hover_gen: 0,
        previews_shown: false,
        menu_dirty: false,
        config_mtime: mide::settings_file::config_path().and_then(|p| fs::metadata(p).ok()?.modified().ok()),
        liquid: liquid_glass(),
        autohide,
        shown: 1.0,
        show_target: 1.0,
        hide_at: None,
        sliding: false,
        pointer_inside: false,
        covered: false,
    };

    tracing::info!("dock pronta com {} app(s) fixado(s)", dock.items.len());

    // Event loop com Wayland e um timer que consulta os apps abertos no compositor.
    let mut event_loop: EventLoop<'static, Dock> = EventLoop::try_new()?;
    dock.loop_handle = Some(event_loop.handle());
    WaylandSource::new(conn.clone(), event_queue)
        .insert(event_loop.handle())
        .map_err(|e| e.to_string())?;
    event_loop
        .handle()
        .insert_source(Timer::from_duration(Duration::from_millis(100)), |_, _, dock| {
            dock.refresh_running();
            dock.refresh_trash();
            dock.watch_config();
            dock.poll_text();
            // Menu fechou com o ponteiro longe: a dock escondida volta a esconder.
            if dock.autohide && dock.covered && dock.show_target > 0.0 && dock.menu.is_none() && !dock.pointer_inside && dock.hide_at.is_none() {
                dock.schedule_hide();
            }
            TimeoutAction::ToDuration(Duration::from_millis(800))
        })
        .map_err(|e| e.to_string())?;

    // Ocultar inteligente: a cada 250 ms pergunta ao compositor se há janela sobre a
    // área da dock. Sem janela por cima, a dock fica à mostra; com janela, ela some
    // (e volta com o ponteiro na borda de baixo).
    event_loop
        .handle()
        .insert_source(Timer::from_duration(Duration::from_millis(250)), |_, _, dock| {
            if dock.autohide {
                dock.check_covered();
            }
            TimeoutAction::ToDuration(Duration::from_millis(250))
        })
        .map_err(|e| e.to_string())?;

    while !dock.exit {
        event_loop.dispatch(None, &mut dock)?;
        if dock.menu_dirty {
            dock.paint_menu();
        }
    }
    Ok(())
}

/// Downloads e Lixeira, sempre no fim da dock.
const STACKS: usize = 2;
const STACK_DOWNLOADS: usize = 0;
const STACK_TRASH: usize = 1;

/// Largura da dock para os itens, mais Downloads e Lixeira e os separadores.
fn dock_width(items: &[DockApp]) -> u32 {
    icon_x(items, items.len() + STACKS - 1) + icon_size() + PADDING
}

/// Há separador quando existem fixados e também apps abertos não fixados.
fn has_separator(items: &[DockApp]) -> bool {
    let pinned = items.iter().filter(|a| a.pinned).count();
    pinned > 0 && pinned < items.len()
}

/// Posição x da posição `index` (apps, depois Downloads e Lixeira); quem vem depois
/// de um separador anda o espaço dele.
fn icon_x(items: &[DockApp], index: usize) -> u32 {
    let pinned = items.iter().filter(|a| a.pinned).count();
    let mut extra = if has_separator(items) && index >= pinned { 2 * SEPARATOR_SPACE } else { 0 };
    if !items.is_empty() && index >= items.len() {
        extra += 2 * SEPARATOR_SPACE;
    }
    // A primeira posição é do Launchpad; os apps começam na segunda.
    PADDING + (index as u32 + 1) * (icon_size() + ICON_GAP) + extra
}

/// Posição do Launchpad (sempre a primeira da dock) no lugar de um índice de app.
const LAUNCHER: usize = usize::MAX;

/// Ids dos apps fixados em `~/.config/mide/dock.toml`, ou os padrões.
fn pinned_ids() -> Vec<String> {
    let config = config_path()
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|text| toml::from_str::<DockConfig>(&text).ok())
        .unwrap_or_default();
    if config.pinned.is_empty() {
        DEFAULT_PINNED.iter().map(|s| s.to_string()).collect()
    } else {
        config.pinned
    }
}

/// Grava a lista de fixados em `~/.config/mide/dock.toml`.
fn save_pinned(ids: &[String]) {
    let Some(path) = config_path() else {
        return;
    };
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let list = ids
        .iter()
        .map(|id| format!("\"{id}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let text = format!("# Apps fixados na dock, na ordem em que aparecem.\npinned = [{list}]\n");
    match fs::write(&path, text) {
        Ok(()) => tracing::info!("dock salva: {}", ids.join(", ")),
        Err(err) => tracing::warn!("falha ao salvar a dock: {err}"),
    }
}

fn config_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("mide").join("dock.toml"))
}

/// Ícone do Launchpad: quadrado claro arredondado com uma grade 3x3 de quadradinhos
/// coloridos, como no macOS.
fn draw_launchpad_icon(pixmap: &mut Pixmap, x: f32, y: f32, size: f32) {
    let inset = size * 0.06;
    let (bx, by, bs) = (x + inset, y + inset, size - 2.0 * inset);
    if let Some(path) = rounded_rect(bx, by, bs, bs, bs * 0.23) {
        let mut paint = Paint::default();
        paint.anti_alias = true;
        paint.shader = LinearGradient::new(
            SkPoint::from_xy(0.0, by),
            SkPoint::from_xy(0.0, by + bs),
            vec![
                GradientStop::new(0.0, Color::from_rgba8(252, 252, 254, 255)),
                GradientStop::new(1.0, Color::from_rgba8(226, 226, 234, 255)),
            ],
            SpreadMode::Pad,
            Transform::identity(),
        )
        .unwrap_or(tiny_skia::Shader::SolidColor(Color::WHITE));
        pixmap.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
        let mut border = Paint::default();
        border.anti_alias = true;
        border.set_color(Color::from_rgba8(0, 0, 0, 40));
        pixmap.stroke_path(&path, &border, &Stroke { width: 0.8, ..Default::default() }, Transform::identity(), None);
    }
    const COLORS: [(u8, u8, u8); 9] = [
        (255, 69, 58),
        (255, 159, 10),
        (255, 214, 10),
        (48, 209, 88),
        (100, 210, 255),
        (10, 132, 255),
        (94, 92, 230),
        (191, 90, 242),
        (255, 55, 95),
    ];
    let cell = bs * 0.2;
    let gap = bs * 0.09;
    let start = bx + (bs - 3.0 * cell - 2.0 * gap) / 2.0;
    let top = by + (bs - 3.0 * cell - 2.0 * gap) / 2.0;
    for (i, (r, g, b)) in COLORS.iter().enumerate() {
        let cx = start + (i % 3) as f32 * (cell + gap);
        let cy = top + (i / 3) as f32 * (cell + gap);
        if let Some(path) = rounded_rect(cx, cy, cell, cell, cell * 0.3) {
            let mut paint = Paint::default();
            paint.anti_alias = true;
            paint.set_color(Color::from_rgba8(*r, *g, *b, 255));
            pixmap.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
        }
    }
}

/// Retângulo arredondado como caminho, com cantos em curvas cúbicas.
fn rounded_rect(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<tiny_skia::Path> {
    const K: f32 = 0.5523;
    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.cubic_to(x + w - r + r * K, y, x + w, y + r - r * K, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.cubic_to(x + w, y + h - r + r * K, x + w - r + r * K, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.cubic_to(x + r - r * K, y + h, x, y + h - r + r * K, x, y + h - r);
    pb.line_to(x, y + r);
    pb.cubic_to(x, y + r - r * K, x + r - r * K, y, x + r, y);
    pb.close();
    pb.finish()
}

struct Dock {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    shm: Shm,
    pool: SlotPool,
    layer: LayerSurface,
    width: u32,
    height: u32,
    first_configure: bool,
    exit: bool,
    /// Fixados primeiro (na ordem salva), depois os abertos que não estão fixados.
    items: Vec<DockApp>,
    catalog: Catalog,
    press: Option<Press>,
    pointer: Option<wl_pointer::WlPointer>,
    pointer_x: f64,
    /// app_ids (em minúsculas) com janela aberta ou minimizada.
    running: HashSet<String>,
    qh: QueueHandle<Dock>,
    /// Última posição dos ícones informada ao compositor.
    last_report: String,
    compositor: CompositorState,
    layer_shell: LayerShell,
    downloads_icon: Option<Pixmap>,
    trash_empty_icon: Option<Pixmap>,
    trash_full_icon: Option<Pixmap>,
    trash_items: usize,
    /// Painel aberto acima da dock (Downloads, menu da Lixeira, confirmação).
    menu: Option<Menu>,
    /// Texto dos painéis; as fontes só são carregadas no primeiro painel.
    text: Option<Text>,
    /// Fontes carregando em segundo plano (carregar leva ~60 ms: no primeiro menu,
    /// isso atrasava a abertura).
    text_ready: Option<std::sync::mpsc::Receiver<Text>>,
    /// Ícones de tipo de arquivo já carregados, pelo nome no tema.
    file_icons: HashMap<&'static str, Option<Pixmap>>,
    screen: (u32, u32),
    /// Para agendar a espera antes de mostrar as prévias.
    loop_handle: Option<LoopHandle<'static, Dock>>,
    /// Ícone sob o ponteiro e um contador que invalida esperas antigas.
    hover_index: Option<usize>,
    hover_gen: u64,
    previews_shown: bool,
    menu_dirty: bool,
    /// Última modificação vista do `config.toml` (o tamanho dos ícones vem dele).
    config_mtime: Option<std::time::SystemTime>,
    /// O compositor desenha o vidro líquido atrás da dock (borda e reflexo vêm dele).
    liquid: bool,
    /// Ocultar automaticamente (Configurações → Dock): a dock desce e fica só uma faixa
    /// invisível na borda; o ponteiro nela faz a dock subir.
    autohide: bool,
    /// Quanto a dock aparece (0: escondida, 1: inteira), o alvo e quando esconder.
    shown: f32,
    show_target: f32,
    hide_at: Option<std::time::Instant>,
    sliding: bool,
    /// O ponteiro está sobre a dock.
    pointer_inside: bool,
    /// Há janela onde a dock aparece (só então a dock que se esconde sozinha some).
    covered: bool,
}

impl Dock {
    /// Desenha a faixa e os ícones num buffer de memória e apresenta.
    fn draw(&mut self, qh: &QueueHandle<Self>) {
        let start = std::time::Instant::now();
        let (w, h) = (self.width, self.height);
        let Some(mut pixmap) = Pixmap::new(w, h) else {
            return;
        };
        // Escondida: nada desenhado (só a faixa invisível que recebe o ponteiro).
        if self.autohide && self.shown <= 0.0 {
            self.present(&pixmap, qh);
            return;
        }

        // Fundo como a dock do macOS: vidro claro e translúcido (o mesmo da seleção
        // do launcher, em `mide::ui::draw::glass`). Com o vidro líquido, a borda, o
        // reflexo e o tom vêm do compositor (na GPU); aqui fica só um véu leve.
        if self.liquid {
            mide::ui::draw::glass_veil(&mut pixmap, 0.0, 0.0, w as f32, h as f32, DOCK_RADIUS);
        } else {
            mide::ui::draw::glass(&mut pixmap, 0.0, 0.0, w as f32, h as f32, DOCK_RADIUS);
        }

        // Separador fino entre os fixados e os apps abertos não fixados.
        let pinned_count = self.items.iter().filter(|a| a.pinned).count();
        if has_separator(&self.items) {
            let x = icon_x(&self.items, pinned_count) as f32 - (ICON_GAP + 2 * SEPARATOR_SPACE) as f32 / 2.0;
            if let Some(line) = rounded_rect(x - 0.5, PADDING as f32 + 6.0, 1.0, icon_size() as f32 - 12.0, 0.5) {
                let mut paint = Paint::default();
                paint.set_color(Color::from_rgba8(60, 60, 66, 120));
                pixmap.fill_path(&line, &paint, FillRule::Winding, Transform::identity(), None);
            }
        }

        draw_launchpad_icon(&mut pixmap, PADDING as f32, PADDING as f32, icon_size() as f32);

        // Separador antes de Downloads e Lixeira.
        if !self.items.is_empty() {
            let x = icon_x(&self.items, self.items.len()) as f32 - (ICON_GAP + 2 * SEPARATOR_SPACE) as f32 / 2.0;
            if let Some(line) = rounded_rect(x - 0.5, PADDING as f32 + 6.0, 1.0, icon_size() as f32 - 12.0, 0.5) {
                let mut paint = Paint::default();
                paint.set_color(Color::from_rgba8(60, 60, 66, 120));
                pixmap.fill_path(&line, &paint, FillRule::Winding, Transform::identity(), None);
            }
        }
        for stack in 0..STACKS {
            let x = icon_x(&self.items, self.items.len() + stack) as i32;
            let icon = match stack {
                STACK_DOWNLOADS => self.downloads_icon.as_ref(),
                _ if self.trash_items > 0 => self.trash_full_icon.as_ref(),
                _ => self.trash_empty_icon.as_ref(),
            };
            if let Some(icon) = icon {
                pixmap.draw_pixmap(x, PADDING as i32, icon.as_ref(), &Default::default(), Transform::identity(), None);
            } else if let Some(path) = rounded_rect(x as f32, PADDING as f32, icon_size() as f32, icon_size() as f32, 10.0) {
                let mut paint = Paint::default();
                paint.set_color(Color::from_rgba8(120, 120, 128, 255));
                paint.anti_alias = true;
                pixmap.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
            }
        }

        // Ícones, centralizados em cada posição.
        for (i, app) in self.items.iter().enumerate() {
            let x = icon_x(&self.items, i) as f32;
            let y = PADDING as f32;
            if let Some(icon) = &app.icon {
                pixmap.draw_pixmap(
                    x as i32,
                    y as i32,
                    icon.as_ref(),
                    &Default::default(),
                    Transform::identity(),
                    None,
                );
            } else if let Some(path) = rounded_rect(x, y, icon_size() as f32, icon_size() as f32, 10.0) {
                // Sem ícone: um bloco neutro no lugar.
                let mut paint = Paint::default();
                paint.set_color(Color::from_rgba8(120, 120, 128, 255));
                paint.anti_alias = true;
                pixmap.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
            }
            // Ponto embaixo do ícone quando o app tem janela aberta, como no macOS.
            if app.match_ids.iter().any(|id| self.running.contains(id)) {
                let cx = x + icon_size() as f32 / 2.0;
                let cy = y + icon_size() as f32 + (PADDING + BOTTOM_EXTRA) as f32 / 2.0;
                if let Some(dot) = PathBuilder::from_circle(cx, cy, 1.8) {
                    let mut paint = Paint::default();
                    paint.set_color(Color::from_rgba8(40, 40, 46, 230));
                    paint.anti_alias = true;
                    pixmap.fill_path(&dot, &paint, FillRule::Winding, Transform::identity(), None);
                }
            }
        }

        self.present(&pixmap, qh);
        tracing::debug!("quadro da dock: {:.2} ms", start.elapsed().as_secs_f32() * 1000.0);
        self.report_icons();
    }

    /// Manda a imagem da dock para o compositor (sem pedir quadros contínuos: a dock
    /// só redesenha quando algo muda).
    fn present(&mut self, pixmap: &Pixmap, _qh: &QueueHandle<Self>) {
        let (w, h) = (pixmap.width(), pixmap.height());
        let Ok((buffer, canvas)) = self.pool.create_buffer(w as i32, h as i32, w as i32 * 4, wl_shm::Format::Argb8888) else {
            return;
        };
        mide::ui::draw::to_argb8888(pixmap, canvas);
        let surface = self.layer.wl_surface();
        surface.damage_buffer(0, 0, w as i32, h as i32);
        if buffer.attach_to(surface).is_ok() {
            self.layer.commit();
        }
    }

    /// Confere se há janela sobre a área da dock e mostra ou esconde a dock.
    fn check_covered(&mut self) {
        let covered = ask_compositor("dock_covered").is_some_and(|l| l.first().is_some_and(|w| w == "yes"));
        if covered == self.covered {
            return;
        }
        self.covered = covered;
        tracing::debug!("dock: janela por cima {}", if covered { "sim" } else { "não" });
        if covered {
            if !self.pointer_inside && self.menu.is_none() {
                self.schedule_hide();
            }
        } else {
            self.hide_at = None;
            self.show_target = 1.0;
            self.slide();
        }
    }

    /// Esconde a dock daqui a pouco (com ocultar automaticamente), se o ponteiro não
    /// voltar e não houver menu aberto.
    fn schedule_hide(&mut self) {
        self.hide_at = Some(std::time::Instant::now() + HIDE_DELAY);
        self.slide();
    }

    /// Começa a deslizar a dock para `show_target` (mostrar ou esconder), quadro a
    /// quadro, mudando a margem de baixo da camada.
    fn slide(&mut self) {
        if self.sliding {
            return;
        }
        let Some(handle) = self.loop_handle.clone() else {
            return;
        };
        self.sliding = true;
        let step = Duration::from_millis(16);
        let _ = handle.insert_source(Timer::from_duration(Duration::ZERO), move |_, _, dock: &mut Dock| {
            // Hora de esconder (o ponteiro saiu e não há menu aberto)?
            if dock.hide_at.is_some_and(|at| std::time::Instant::now() >= at) {
                dock.hide_at = None;
                if dock.autohide && dock.covered && !dock.pointer_inside && dock.menu.is_none() && dock.press.is_none() {
                    dock.show_target = 0.0;
                }
            }
            let delta = step.as_secs_f32() / SLIDE.as_secs_f32();
            let before = dock.shown;
            dock.shown = if dock.show_target > dock.shown { (dock.shown + delta).min(dock.show_target) } else { (dock.shown - delta).max(dock.show_target) };
            if dock.shown != before {
                // Sai e entra com aceleração suave.
                let k = dock.shown * dock.shown * (3.0 - 2.0 * dock.shown);
                let hidden = hidden_margin() as f32;
                let margin = (hidden + (BOTTOM_MARGIN as f32 - hidden) * k).round() as i32;
                dock.layer.set_margin(0, 0, margin, 0);
                // Ao começar a aparecer ou terminar de sumir, a imagem muda.
                if before <= 0.0 || dock.shown <= 0.0 {
                    let qh = dock.qh.clone();
                    dock.draw(&qh);
                } else {
                    dock.layer.commit();
                }
            }
            if dock.shown == dock.show_target && dock.hide_at.is_none() {
                dock.sliding = false;
                if dock.shown >= 1.0 || dock.shown <= 0.0 {
                    tracing::info!("dock {}", if dock.shown >= 1.0 { "à mostra" } else { "escondida" });
                }
                return TimeoutAction::Drop;
            }
            TimeoutAction::ToDuration(step)
        });
    }

    /// Informa ao compositor onde está cada ícone (para o efeito lâmpada ir até ele).
    fn report_icons(&mut self) {
        let report = self
            .items
            .iter()
            .enumerate()
            .map(|(i, app)| {
                let x = icon_x(&self.items, i);
                format!("{}:{}:{}:{}:{}", app.match_ids.join(","), x, PADDING, icon_size(), icon_size())
            })
            .collect::<Vec<_>>()
            .join(";");
        if report != self.last_report {
            if ask_compositor(&format!("icons {report}")).is_some() {
                self.last_report = report;
            }
        }
    }

    /// Consulta o compositor e redesenha se o conjunto de apps abertos mudou.
    fn refresh_running(&mut self) {
        let Some(lines) = ask_compositor("list") else {
            return;
        };
        let running: HashSet<String> = lines
            .iter()
            .filter_map(|l| l.split('\t').next())
            .filter(|id| !id.is_empty())
            .map(|id| id.to_lowercase())
            .collect();
        // Título de uma janela de cada app, para os apps sem `.desktop`.
        let titles: HashMap<String, String> = lines
            .iter()
            .filter_map(|l| {
                let mut parts = l.split('\t');
                Some((parts.next()?.to_lowercase(), parts.next()?.to_string()))
            })
            .collect();
        if running == self.running {
            return;
        }
        self.running = running;

        // Apps abertos que não estão na dock entram depois dos fixados.
        let mut changed = false;
        let open_ids: Vec<String> = self.running.iter().cloned().collect();
        for app_id in open_ids {
            let known = self.items.iter().any(|a| a.match_ids.contains(&app_id));
            if !known {
                let title = titles.get(&app_id).cloned().unwrap_or_default();
                let app = self.catalog.find(&app_id, false).unwrap_or_else(|| self.catalog.fallback(&app_id, &title));
                self.items.push(app);
                changed = true;
            }
        }
        // Não fixados que fecharam saem da dock.
        let before = self.items.len();
        let running = &self.running;
        self.items
            .retain(|a| a.pinned || a.match_ids.iter().any(|id| running.contains(id)));
        changed |= self.items.len() != before;

        if changed {
            self.resize();
        } else if !self.first_configure {
            let qh = self.qh.clone();
            self.draw(&qh);
        }
    }

    /// Ajusta a largura da dock ao número de itens; o compositor reconfigura e ela redesenha.
    fn resize(&mut self) {
        let width = dock_width(&self.items);
        self.layer.set_size(width, dock_height());
        // O tamanho dos ícones muda a altura: o espaço reservado acompanha.
        if !self.autohide {
            self.layer.set_exclusive_zone(reserved_zone());
        }
        self.layer.commit();
    }

    /// Ids dos fixados, na ordem atual.
    fn pinned_order(&self) -> Vec<String> {
        self.items.iter().filter(|a| a.pinned).map(|a| a.id.clone()).collect()
    }

    /// Clique direito: fixa um app aberto ou desafixa um fixado.
    fn toggle_pin(&mut self, index: usize) {
        let mut app = self.items.remove(index);
        app.pinned = !app.pinned;
        let running = app.match_ids.iter().any(|id| self.running.contains(id));
        let pinned_count = self.items.iter().filter(|a| a.pinned).count();
        if app.pinned {
            tracing::info!("fixado na dock: {}", app.name);
            self.items.insert(pinned_count, app);
        } else {
            tracing::info!("desafixado da dock: {}", app.name);
            if running {
                self.items.insert(pinned_count, app);
            }
        }
        save_pinned(&self.pinned_order());
        self.resize();
    }

    /// Arrasto de um ícone fixado: troca de lugar com o ícone sob o ponteiro.
    fn drag_to(&mut self, x: f64) {
        let Some(press) = self.press.as_mut() else {
            return;
        };
        if !press.dragged && (x - press.start_x).abs() < DRAG_THRESHOLD {
            return;
        }
        press.dragged = true;
        let from = press.index;
        let pinned_count = self.items.iter().filter(|a| a.pinned).count();
        if from >= pinned_count {
            return;
        }
        let slot = (icon_size() + ICON_GAP) as f64;
        let target = (((x - PADDING as f64 - slot) / slot).floor().max(0.0) as usize).min(pinned_count - 1);
        if target != from {
            let app = self.items.remove(from);
            self.items.insert(target, app);
            if let Some(press) = self.press.as_mut() {
                press.index = target;
            }
            let qh = self.qh.clone();
            self.draw(&qh);
        }
    }

    /// Índice do ícone sob a coordenada `x`, se houver.
    fn icon_at(&self, x: f64) -> Option<usize> {
        // Os vãos entre ícones contam para o ícone mais próximo, para não haver
        // pontos mortos na dock.
        let half_gap = (ICON_GAP / 2) as f64;
        if x >= PADDING as f64 - half_gap && x < (PADDING + icon_size()) as f64 + half_gap {
            return Some(LAUNCHER);
        }
        (0..self.items.len() + STACKS).find(|&i| {
            let left = icon_x(&self.items, i) as f64;
            x >= left - half_gap && x < left + icon_size() as f64 + half_gap
        })
    }

    /// Foca ou restaura o app se ele já tiver janela; senão, abre.
    fn activate_or_launch(&self, index: usize) {
        let app = &self.items[index];
        // Ctrl+clique: abre uma instância nova mesmo com o app já aberto.
        let ctrl = ask_compositor("mods")
            .and_then(|lines| lines.first().cloned())
            .is_some_and(|mods| mods.split_whitespace().any(|m| m == "ctrl"));
        if ctrl {
            tracing::info!("ctrl+clique: nova instância de {}", app.name);
            self.launch(index);
            return;
        }
        let command = format!("activate {}", app.match_ids.join(" "));
        if let Some(reply) = ask_compositor(&command) {
            if reply.first().map(String::as_str) == Some("ok") {
                tracing::info!("{} já estava aberto: focado", app.name);
                return;
            }
        }
        self.launch(index);
    }

    fn launch(&self, index: usize) {
        let app = &self.items[index];
        // Avisa o compositor: a janela nova sai do ícone com o efeito lâmpada.
        ask_compositor(&format!("launching {}", app.match_ids.join(" ")));
        // Remove os códigos de campo do Exec (%u, %F, ...), que não usamos.
        let command: String = app
            .exec
            .split_whitespace()
            .filter(|t| !t.starts_with('%'))
            .collect::<Vec<_>>()
            .join(" ");
        if command.is_empty() {
            tracing::warn!("{} sem comando Exec", app.name);
            return;
        }
        match Command::new("sh").arg("-c").arg(&command).spawn() {
            Ok(mut child) => {
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
                tracing::info!("abrindo {} ({})", app.name, app.id);
            }
            Err(err) => tracing::warn!("falha ao abrir {}: {err}", app.name),
        }
    }
}


/// O que um painel acima da dock mostra.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MenuKind {
    /// Lista dos itens mais recentes de Downloads (clique em Downloads).
    Downloads,
    /// Menu do ícone de Downloads (clique direito).
    DownloadsContext,
    /// Menu de um arquivo da lista de Downloads (clique direito nele).
    DownloadItem,
    /// Lista dos itens apagados (clique na Lixeira).
    Trash,
    /// Menu do ícone da Lixeira (clique direito).
    TrashContext,
    /// Menu de um item da lista da Lixeira (clique direito nele).
    TrashItem,
    ConfirmEmpty,
    /// Menu de contexto de um app da dock (índice em `items`).
    App(usize),
    /// Menu do Launchpad (clique direito): ajustes da dock.
    Launchpad,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RowAction {
    None,
    Open(String),
    AskEmptyTrash,
    EmptyTrash,
    Close,
    /// Ações de app, pelo id do `.desktop` (os índices mudam se a dock mudar).
    NewWindow(String),
    TogglePin(String),
    Hide(Vec<String>),
    Quit(Vec<String>),
    /// Foca uma janela específica, pelo id dela no compositor.
    FocusWindow(u64),
    /// Ações de arquivo, pelo caminho.
    CopyPath(String),
    Trash(String),
    /// Ações de item da Lixeira, pelo nome em `files/`.
    Restore(String),
    DeleteForever(String),
    /// Abre uma seção das Configurações do Sistema.
    Settings(&'static str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RowKind {
    Header,
    Item,
    /// Ação que apaga coisas: texto em vermelho.
    Danger,
    /// Texto informativo, sem clique.
    Note,
    Separator,
}

#[derive(Clone)]
struct Row {
    kind: RowKind,
    label: String,
    detail: String,
    icon: Option<Pixmap>,
    action: RowAction,
    /// Alvo do menu de contexto da linha (caminho do arquivo ou nome na Lixeira).
    target: Option<String>,
}

impl Row {
    fn new(kind: RowKind, label: impl Into<String>, action: RowAction) -> Self {
        Self { kind, label: label.into(), detail: String::new(), icon: None, action, target: None }
    }

    fn separator() -> Self {
        Self::new(RowKind::Separator, "", RowAction::None)
    }

    fn height(&self) -> f32 {
        match self.kind {
            RowKind::Separator => 9.0,
            RowKind::Header => 28.0,
            _ if self.icon.is_some() => 34.0,
            _ => 26.0,
        }
    }

    fn clickable(&self) -> bool {
        matches!(self.kind, RowKind::Item | RowKind::Danger) && self.action != RowAction::None
    }
}

/// Um painel desenhado: linhas, retângulo e linha sob o ponteiro.
struct Panel {
    kind: MenuKind,
    rows: Vec<Row>,
    /// (x, y, largura, altura) na tela.
    rect: (f32, f32, f32, f32),
    spans: Vec<(f32, f32)>,
    hover: Option<usize>,
    pressed: Option<usize>,
}

impl Panel {
    fn new(kind: MenuKind, rows: Vec<Row>, rect: (f32, f32, f32, f32)) -> Self {
        Self { kind, rows, rect, spans: Vec::new(), hover: None, pressed: None }
    }

    fn height_for(rows: &[Row]) -> f32 {
        rows.iter().map(Row::height).sum::<f32>() + 2.0 * MENU_PAD
    }

    fn contains(&self, x: f32, y: f32) -> bool {
        let (px, py, pw, ph) = self.rect;
        x >= px && x <= px + pw && y >= py && y <= py + ph
    }

    fn row_at(&self, y: f32) -> Option<usize> {
        self.spans.iter().position(|(a, b)| y >= *a && y < *b)
    }
}

/// Painéis abertos acima da dock, numa camada de tela cheia: um clique fora fecha.
/// O painel principal fica acima do ícone; o de contexto (clique direito num item
/// da lista) abre no ponto do clique, por cima da lista, sem fechá-la.
struct Menu {
    layer: LayerSurface,
    pool: SlotPool,
    main: Panel,
    context: Option<Panel>,
    /// Centro x do ícone da dock, em coordenadas da dock.
    anchor_x: f32,
    width: f32,
    /// Caixa desenhada no último quadro (x, y, largura, altura), para avisar o
    /// compositor só do que mudou.
    last_box: Option<(i32, i32, i32, i32)>,
    /// Vidro atrás dos painéis (desenhado pelo compositor).
    glass: mide::control_client::Glass,
}

const MENU_RADIUS: f32 = 10.0;
const MENU_PAD: f32 = 6.0;
const MAX_RECENT: usize = 8;

impl Dock {
    /// Lixeira cheia ou vazia: troca o ícone quando muda.
    fn refresh_trash(&mut self) {
        let count = places::trash_count(&places::trash_dir());
        if count != self.trash_items {
            self.trash_items = count;
            if !self.first_configure {
                let qh = self.qh.clone();
                self.draw(&qh);
            }
        }
    }

    /// Clique em Downloads ou na Lixeira (`context` = botão direito).
    fn stack_click(&mut self, stack: usize, context: bool) {
        let slot = self.items.len() + stack;
        let kind = match (stack, context) {
            (STACK_DOWNLOADS, false) => MenuKind::Downloads,
            (STACK_DOWNLOADS, true) => MenuKind::DownloadsContext,
            (_, false) => MenuKind::Trash,
            (_, true) => MenuKind::TrashContext,
        };
        self.toggle_menu(kind, slot);
    }

    fn file_icon(&mut self, name: &'static str) -> Option<Pixmap> {
        self.file_icons
            .entry(name)
            .or_insert_with(|| load_themed_icon(&[name, "text-x-generic"], 24))
            .clone()
    }

    fn menu_rows(&mut self, kind: MenuKind, target: &str) -> Vec<Row> {
        let downloads = places::downloads_dir();
        let now = std::time::SystemTime::now();
        match kind {
            MenuKind::Downloads => {
                let items = places::recent_items(&downloads, MAX_RECENT);
                tracing::info!("pilha Downloads aberta: {} item(ns) em {}", items.len(), downloads.display());
                let mut rows = vec![Row::new(RowKind::Header, "Downloads", RowAction::None)];
                if items.is_empty() {
                    rows.push(Row::new(RowKind::Note, "Nenhum item", RowAction::None));
                }
                for item in items {
                    let path = item.path.to_string_lossy().into_owned();
                    rows.push(Row {
                        kind: RowKind::Item,
                        label: item.name.clone(),
                        detail: places::age_text(item.modified, now),
                        icon: self.file_icon(places::icon_name_for(&item)),
                        action: RowAction::Open(path.clone()),
                        target: Some(path),
                    });
                }
                rows.push(Row::separator());
                rows.push(Row::new(
                    RowKind::Item,
                    "Abrir no Gerenciador de Arquivos",
                    RowAction::Open(downloads.to_string_lossy().into_owned()),
                ));
                rows
            }
            MenuKind::Trash => {
                let trash = places::trash_dir();
                let items = places::trash_items(&trash, MAX_RECENT);
                tracing::info!("lista da Lixeira aberta: {} item(ns)", self.trash_items);
                let mut rows = vec![Row::new(RowKind::Header, "Lixeira", RowAction::None)];
                if items.is_empty() {
                    rows.push(Row::new(RowKind::Note, "A Lixeira está vazia", RowAction::None));
                }
                for item in items {
                    let as_item = places::Item {
                        path: item.original.clone(),
                        name: item.name.clone(),
                        is_dir: item.is_dir,
                        modified: item.deleted,
                    };
                    rows.push(Row {
                        kind: RowKind::Item,
                        label: item.name.clone(),
                        detail: places::age_text(item.deleted, now),
                        icon: self.file_icon(places::icon_name_for(&as_item)),
                        // Clique esquerdo num item apagado abre a Lixeira; as ações
                        // ficam no clique direito.
                        action: RowAction::Open("trash:///".into()),
                        target: Some(item.name),
                    });
                }
                rows.push(Row::separator());
                rows.push(Row::new(RowKind::Item, "Abrir no Gerenciador de Arquivos", RowAction::Open("trash:///".into())));
                let empty = if self.trash_items > 0 { RowAction::AskEmptyTrash } else { RowAction::None };
                rows.push(Row::new(RowKind::Item, "Esvaziar Lixeira…", empty));
                rows
            }
            MenuKind::App(index) => {
                let Some(app) = self.items.get(index).cloned() else {
                    return Vec::new();
                };
                let running = app.match_ids.iter().any(|id| self.running.contains(id));
                tracing::info!("menu de {} aberto", app.name);
                let mut rows = vec![Row::new(RowKind::Header, app.name.clone(), RowAction::None)];
                // Com mais de uma janela, lista as janelas para escolher qual focar.
                let windows = app_windows(&app);
                if windows.len() > 1 {
                    for (id, title) in &windows {
                        let label = if title.is_empty() { app.name.clone() } else { title.clone() };
                        rows.push(Row::new(RowKind::Item, label, RowAction::FocusWindow(*id)));
                    }
                    rows.push(Row::separator());
                }
                // Sem `.desktop` não dá para abrir de novo nem manter na dock.
                if !app.exec.is_empty() {
                    rows.push(Row::new(RowKind::Item, if running { "Nova janela" } else { "Abrir" }, RowAction::NewWindow(app.id.clone())));
                    rows.push(Row::separator());
                    rows.push(Row::new(
                        RowKind::Item,
                        if app.pinned { "Remover da Dock" } else { "Manter na Dock" },
                        RowAction::TogglePin(app.id.clone()),
                    ));
                }
                if running {
                    if rows.last().is_some_and(|r| r.kind != RowKind::Separator) && rows.len() > 1 {
                        rows.push(Row::separator());
                    }
                    rows.push(Row::new(RowKind::Item, "Ocultar", RowAction::Hide(app.match_ids.clone())));
                    rows.push(Row::new(RowKind::Item, "Encerrar", RowAction::Quit(app.match_ids.clone())));
                }
                rows
            }
            MenuKind::DownloadItem => {
                let path = target.to_string();
                let name = std::path::Path::new(&path)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let folder = std::path::Path::new(&path)
                    .parent()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default();
                tracing::info!("menu do arquivo {name} aberto");
                vec![
                    Row::new(RowKind::Header, name, RowAction::None),
                    Row::new(RowKind::Item, "Abrir", RowAction::Open(path.clone())),
                    Row::new(RowKind::Item, "Mostrar na pasta", RowAction::Open(folder)),
                    Row::new(RowKind::Item, "Copiar caminho", RowAction::CopyPath(path.clone())),
                    Row::separator(),
                    Row::new(RowKind::Danger, "Mover para a Lixeira", RowAction::Trash(path)),
                ]
            }
            MenuKind::TrashItem => {
                tracing::info!("menu do item apagado {target} aberto");
                vec![
                    Row::new(RowKind::Header, target, RowAction::None),
                    Row::new(RowKind::Item, "Restaurar", RowAction::Restore(target.to_string())),
                    Row::separator(),
                    Row::new(RowKind::Danger, "Apagar permanentemente", RowAction::DeleteForever(target.to_string())),
                ]
            }
            MenuKind::Launchpad => vec![
                Row::new(RowKind::Item, "Abrir o Launchpad", RowAction::Open("launcher:".into())),
                Row::separator(),
                Row::new(RowKind::Item, "Ajustes da Dock…", RowAction::Settings("dock")),
            ],
            MenuKind::DownloadsContext => vec![Row::new(
                RowKind::Item,
                "Abrir pasta Downloads",
                RowAction::Open(downloads.to_string_lossy().into_owned()),
            )],
            MenuKind::TrashContext => {
                tracing::info!("menu da Lixeira aberto ({} item(ns))", self.trash_items);
                let empty_action = if self.trash_items > 0 { RowAction::AskEmptyTrash } else { RowAction::None };
                vec![
                    Row::new(RowKind::Item, "Abrir", RowAction::Open("trash:///".into())),
                    Row::separator(),
                    Row::new(RowKind::Item, "Esvaziar Lixeira…", empty_action),
                ]
            }
            MenuKind::ConfirmEmpty => {
                let n = self.trash_items;
                let what = if n == 1 { "1 item será apagado".to_string() } else { format!("{n} itens serão apagados") };
                vec![
                    Row::new(RowKind::Header, "Esvaziar a Lixeira?", RowAction::None),
                    Row::new(RowKind::Note, format!("{what} para sempre."), RowAction::None),
                    Row::new(RowKind::Note, "Não dá para desfazer.", RowAction::None),
                    Row::separator(),
                    Row::new(RowKind::Danger, "Esvaziar Lixeira", RowAction::EmptyTrash),
                    Row::new(RowKind::Item, "Cancelar", RowAction::Close),
                ]
            }
        }
    }

    /// Clique num ícone: abre o painel, ou fecha se ele já estiver aberto (como no macOS).
    /// `slot` é a posição do ícone (apps, depois Downloads e Lixeira).
    fn toggle_menu(&mut self, kind: MenuKind, slot: usize) {
        if self.menu.as_ref().is_some_and(|m| m.main.kind == kind) {
            self.menu = None;
        } else {
            self.open_menu(kind, slot);
        }
    }

    fn ensure_text(&mut self) {
        if self.text.is_none() {
            // Normalmente já chegou da thread de aquecimento; senão, espera por ela.
            self.text = Some(self.text_ready.take().and_then(|rx| rx.recv().ok()).unwrap_or_else(Text::new));
        }
    }

    /// Fontes carregadas em segundo plano chegaram (sem travar a dock).
    fn poll_text(&mut self) {
        if self.text.is_none() {
            if let Some(text) = self.text_ready.as_ref().and_then(|rx| rx.try_recv().ok()) {
                self.text = Some(text);
                self.text_ready = None;
            }
        }
    }

    /// Abre o painel acima do ícone da posição `slot`, ou troca o que já está aberto.
    fn open_menu(&mut self, kind: MenuKind, slot: usize) {
        self.ensure_text();
        let rows = self.menu_rows(kind, "");
        let left = if slot == LAUNCHER { PADDING } else { icon_x(&self.items, slot) };
        let anchor_x = left as f32 + icon_size() as f32 / 2.0;
        let width = match kind {
            MenuKind::Downloads | MenuKind::Trash => 320.0,
            MenuKind::ConfirmEmpty => 280.0,
            _ => 240.0,
        };
        let main = Panel::new(kind, rows, (0.0, 0.0, width, 0.0));
        if let Some(menu) = self.menu.as_mut() {
            menu.main = main;
            menu.context = None;
            menu.anchor_x = anchor_x;
            menu.width = width;
            self.draw_menu();
            return;
        }
        let surface = self.compositor.create_surface(&self.qh);
        let layer = self.layer_shell.create_layer_surface(&self.qh, surface, Layer::Overlay, Some("mide-dock-menu"), None);
        layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
        layer.set_exclusive_zone(-1);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.commit();
        let Ok(pool) = SlotPool::new(256 * 256 * 4, &self.shm) else {
            return;
        };
        self.menu = Some(Menu { layer, pool, main, context: None, anchor_x, width, last_box: None, glass: Default::default() });
    }

    /// Abre o menu de contexto de um item da lista no ponto (x, y), por cima dela.
    fn open_context(&mut self, kind: MenuKind, target: &str, x: f32, y: f32) {
        let rows = self.menu_rows(kind, target);
        let (sw, sh) = (self.screen.0 as f32, self.screen.1 as f32);
        let (w, h) = (230.0, Panel::height_for(&rows));
        // Como no macOS: o canto do menu fica no ponteiro, sem sair da tela.
        let px = (x + 2.0).min(sw - w - 6.0).max(6.0);
        let py = y.min(sh - h - 6.0).max(6.0);
        if let Some(menu) = self.menu.as_mut() {
            menu.context = Some(Panel::new(kind, rows, (px, py, w, h)));
        }
        self.draw_menu();
    }

    /// Pede um quadro novo do painel; o desenho sai uma vez depois dos eventos pendentes.
    fn draw_menu(&mut self) {
        self.menu_dirty = true;
    }

    fn paint_menu(&mut self) {
        self.menu_dirty = false;
        let (sw, sh) = self.screen;
        let dock_w = self.width;
        let Some(menu) = self.menu.as_mut() else {
            return;
        };
        let Some(text) = self.text.as_mut() else {
            return;
        };
        if sw == 0 || sh == 0 {
            return;
        }
        let start = std::time::Instant::now();

        // Painel principal acima da dock, centrado no ícone e dentro da tela.
        let ph = Panel::height_for(&menu.main.rows);
        let pw = menu.width;
        // Mesmo arredondamento do compositor ao centralizar a dock.
        let dock_x = ((sw as i64 - dock_w as i64 + 1) / 2) as f32;
        let px = (dock_x + menu.anchor_x - pw / 2.0).clamp(8.0, sw as f32 - pw - 8.0);
        let py = sh as f32 - BOTTOM_MARGIN as f32 - dock_height() as f32 - 10.0 - ph;
        menu.main.rect = (px, py, pw, ph);

        // A camada cobre a tela (para o clique fora fechar), mas só a caixa dos painéis
        // (com a sombra) é desenhada e convertida; o resto do buffer fica transparente.
        const SHADOW: f32 = 12.0;
        let mut bounds = (menu.main.rect.0, menu.main.rect.1, menu.main.rect.0 + pw, menu.main.rect.1 + ph);
        if let Some(c) = menu.context.as_ref() {
            bounds = (bounds.0.min(c.rect.0), bounds.1.min(c.rect.1), bounds.2.max(c.rect.0 + c.rect.2), bounds.3.max(c.rect.1 + c.rect.3));
        }
        let bx = ((bounds.0 - SHADOW).floor() as i32).clamp(0, sw as i32 - 1);
        let by = ((bounds.1 - SHADOW).floor() as i32).clamp(0, sh as i32 - 1);
        let bw = (((bounds.2 + SHADOW).ceil() as i32).min(sw as i32) - bx).max(1);
        // A ponta do menu desce até perto do ícone (cabe no espaço acima da dock).
        let bh = (((bounds.3 + SHADOW + ARROW_H).ceil() as i32).min(sh as i32) - by).max(1);
        let Some(mut pixmap) = Pixmap::new(bw as u32, bh as u32) else {
            return;
        };
        // Desenha com os painéis deslocados para a caixa e devolve as posições de tela
        // (os toques usam `rect` e `spans` em coordenadas da tela).
        let shift = |panel: &mut Panel, dx: f32, dy: f32| {
            panel.rect.0 += dx;
            panel.rect.1 += dy;
            for span in &mut panel.spans {
                span.0 += dy;
                span.1 += dy;
            }
        };
        // Vidro atrás dos painéis, em coordenadas da tela (a camada cobre a tela).
        let glass = mide::control_client::panel_glass();
        let mut rects = vec![(menu.main.rect.0, menu.main.rect.1, menu.main.rect.2, menu.main.rect.3, MENU_RADIUS)];
        if let Some(c) = menu.context.as_ref() {
            rects.push((c.rect.0, c.rect.1, c.rect.2, c.rect.3, MENU_RADIUS));
        }
        menu.glass.set("mide-dock-menu", if glass { &rects } else { &[] });
        let (fx, fy) = (bx as f32, by as f32);
        shift(&mut menu.main, -fx, -fy);
        draw_panel(&mut pixmap, text, &mut menu.main, glass);
        // Ponta embaixo do painel principal, apontando para o ícone da dock.
        let icon_x = dock_x + menu.anchor_x - fx;
        let (mx, my, mw, mh) = menu.main.rect;
        draw_arrow(&mut pixmap, icon_x.clamp(mx + 18.0, mx + mw - 18.0), my + mh, glass);
        shift(&mut menu.main, fx, fy);
        if let Some(context) = menu.context.as_mut() {
            shift(context, -fx, -fy);
            draw_panel(&mut pixmap, text, context, glass);
            shift(context, fx, fy);
        }

        let stride = sw as i32 * 4;
        let Ok((buffer, canvas)) = menu.pool.create_buffer(sw as i32, sh as i32, stride, wl_shm::Format::Argb8888) else {
            return;
        };
        canvas.fill(0);
        let row = bw as usize * 4;
        for (r, src) in pixmap.data().chunks_exact(row).enumerate() {
            let at = ((by as usize + r) * sw as usize + bx as usize) * 4;
            mide::ui::draw::argb_row(src, &mut canvas[at..at + row]);
        }
        let surface = menu.layer.wl_surface();
        // Só mudou a caixa nova e a do quadro anterior (que agora fica vazia).
        for (x, y, w, h) in [Some((bx, by, bw, bh)), menu.last_box].into_iter().flatten() {
            surface.damage_buffer(x, y, w, h);
        }
        menu.last_box = Some((bx, by, bw, bh));
        if buffer.attach_to(surface).is_ok() {
            menu.layer.commit();
        }
        tracing::debug!("menu da dock desenhado: {:.2} ms", start.elapsed().as_secs_f32() * 1000.0);
    }

    fn menu_pointer(&mut self, event: &PointerEvent) {
        let (x, y) = (event.position.0 as f32, event.position.1 as f32);
        let Some(menu) = self.menu.as_mut() else {
            return;
        };
        // O menu de contexto fica por cima: ele recebe primeiro.
        let in_context = menu.context.as_ref().is_some_and(|c| c.contains(x, y));
        let in_main = menu.main.contains(x, y);
        match event.kind {
            PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => {
                let mut changed = false;
                if let Some(context) = menu.context.as_mut() {
                    let hover = if in_context { context.row_at(y) } else { None };
                    changed |= context.hover != hover;
                    context.hover = hover;
                }
                let hover = if in_main && !in_context { menu.main.row_at(y) } else { None };
                changed |= menu.main.hover != hover;
                menu.main.hover = hover;
                if changed {
                    self.draw_menu();
                }
            }
            PointerEventKind::Press { button, .. } => {
                if in_context {
                    if let Some(context) = menu.context.as_mut() {
                        context.pressed = context.row_at(y);
                    }
                    return;
                }
                if menu.context.take().is_some() {
                    // Clique fora do menu de contexto: fecha só ele (a lista continua).
                    if !in_main {
                        self.menu = None;
                    } else {
                        self.draw_menu();
                    }
                    return;
                }
                if !in_main {
                    tracing::info!("painel da dock fechado");
                    self.menu = None;
                    return;
                }
                let row = menu.main.row_at(y);
                if button == BTN_RIGHT {
                    // Clique direito num item da lista: menu dele no ponto do clique.
                    let context = row.and_then(|r| menu.main.rows.get(r)).and_then(|r| r.target.clone()).and_then(|target| {
                        match menu.main.kind {
                            MenuKind::Downloads => Some((MenuKind::DownloadItem, target)),
                            MenuKind::Trash => Some((MenuKind::TrashItem, target)),
                            _ => None,
                        }
                    });
                    if let Some((kind, target)) = context {
                        self.open_context(kind, &target, x, y);
                    }
                    return;
                }
                menu.main.pressed = row;
            }
            PointerEventKind::Release { .. } => {
                let panel = if in_context { menu.context.as_mut() } else if in_main { Some(&mut menu.main) } else { None };
                let Some(panel) = panel else {
                    return;
                };
                let pressed = panel.pressed.take();
                let Some(index) = panel.row_at(y).filter(|r| Some(*r) == pressed) else {
                    return;
                };
                let Some(row) = panel.rows.get(index) else {
                    return;
                };
                if !row.clickable() {
                    return;
                }
                let action = row.action.clone();
                self.menu_action(action);
            }
            _ => {}
        }
    }

    fn menu_action(&mut self, action: RowAction) {
        match action {
            RowAction::None => {}
            RowAction::Close => self.menu = None,
            RowAction::Open(target) if target == "launcher:" => {
                self.menu = None;
                ask_compositor("launcher");
            }
            RowAction::Open(target) => {
                self.menu = None;
                places::open(&target);
            }
            RowAction::Settings(section) => {
                self.menu = None;
                mide::system::open_settings(section);
            }
            // Troca o menu pela confirmação, no mesmo lugar.
            RowAction::AskEmptyTrash => self.open_menu(MenuKind::ConfirmEmpty, self.items.len() + STACK_TRASH),
            RowAction::NewWindow(id) => {
                self.menu = None;
                if let Some(index) = self.items.iter().position(|a| a.id == id) {
                    tracing::info!("nova janela de {id}");
                    self.launch(index);
                }
            }
            RowAction::TogglePin(id) => {
                self.menu = None;
                if let Some(index) = self.items.iter().position(|a| a.id == id) {
                    self.toggle_pin(index);
                }
            }
            RowAction::Hide(ids) => {
                self.menu = None;
                ask_compositor(&format!("hide {}", ids.join(" ")));
            }
            RowAction::Quit(ids) => {
                self.menu = None;
                tracing::info!("encerrar {}", ids.first().cloned().unwrap_or_default());
                ask_compositor(&format!("close {}", ids.join(" ")));
            }
            RowAction::FocusWindow(id) => {
                self.menu = None;
                tracing::info!("focar janela {id}");
                ask_compositor(&format!("focus {id}"));
            }
            RowAction::CopyPath(path) => {
                self.menu = None;
                places::copy_text(&path);
            }
            RowAction::Trash(path) => {
                self.menu = None;
                match places::move_to_trash(std::path::Path::new(&path), &places::trash_dir()) {
                    Ok(name) => tracing::info!("movido para a lixeira: {name}"),
                    Err(err) => tracing::warn!("falha ao mover {path} para a lixeira: {err}"),
                }
                self.refresh_trash();
            }
            RowAction::Restore(name) => {
                self.menu = None;
                match places::restore_from_trash(&places::trash_dir(), &name) {
                    Ok(path) => tracing::info!("restaurado da lixeira: {}", path.display()),
                    Err(err) => tracing::warn!("falha ao restaurar {name}: {err}"),
                }
                self.refresh_trash();
            }
            RowAction::DeleteForever(name) => {
                self.menu = None;
                match places::delete_from_trash(&places::trash_dir(), &name) {
                    Ok(()) => tracing::info!("apagado permanentemente: {name}"),
                    Err(err) => tracing::warn!("falha ao apagar {name}: {err}"),
                }
                self.refresh_trash();
            }
            RowAction::EmptyTrash => {
                self.menu = None;
                match places::empty_trash(&places::trash_dir()) {
                    Ok(n) => tracing::info!("lixeira esvaziada: {n} item(ns)"),
                    Err(err) => tracing::warn!("falha ao esvaziar a lixeira: {err}"),
                }
                self.refresh_trash();
            }
        }
    }
}

impl Dock {
    /// Com o `config.toml` alterado (Configurações → Dock), aplica o tamanho novo dos
    /// ícones: recarrega os ícones nesse tamanho e ajusta a dock.
    fn watch_config(&mut self) {
        let mtime = mide::settings_file::config_path().and_then(|p| fs::metadata(p).ok()?.modified().ok());
        if mtime == self.config_mtime {
            return;
        }
        self.config_mtime = mtime;
        let autohide = configured_autohide();
        if autohide != self.autohide {
            tracing::info!("dock: ocultar automaticamente {}", if autohide { "ligado" } else { "desligado" });
            self.autohide = autohide;
            self.show_target = 1.0;
            self.hide_at = None;
            self.covered = false;
            self.layer.set_exclusive_zone(if autohide { 0 } else { reserved_zone() });
            // A zona nova só vale com o commit (o deslizar pode nem começar).
            self.layer.commit();
            self.slide();
        }
        let liquid = liquid_glass();
        if liquid != self.liquid {
            tracing::info!("vidro líquido na dock: {}", if liquid { "ligado" } else { "desligado" });
            self.liquid = liquid;
            let qh = self.qh.clone();
            self.draw(&qh);
        }
        let size = configured_icon_size();
        if size == icon_size() {
            return;
        }
        tracing::info!("tamanho dos ícones da dock: {size} px");
        ICON_SIZE_NOW.store(size, std::sync::atomic::Ordering::Relaxed);
        self.catalog.icons.clear();
        let items = std::mem::take(&mut self.items);
        self.items = items
            .into_iter()
            .map(|item| match self.catalog.find(&item.id, item.pinned) {
                Some(found) => DockApp { icon: found.icon, ..item },
                None => {
                    let fresh = self.catalog.fallback(&item.id, &item.name);
                    DockApp { icon: fresh.icon, ..item }
                }
            })
            .collect();
        self.downloads_icon = load_themed_icon(&["folder-download", "folder-downloads", "folder"], size);
        self.trash_empty_icon = load_themed_icon(&["user-trash", "user-trash-empty"], size);
        self.trash_full_icon = load_themed_icon(&["user-trash-full", "user-trash"], size);
        self.menu = None;
        self.last_report.clear();
        self.resize();
    }

    /// O ponteiro passou para outro ícone: depois de uma pausa sobre um app aberto,
    /// mostra as prévias das janelas dele.
    fn hover_changed(&mut self, index: Option<usize>) {
        if index == self.hover_index {
            return;
        }
        self.hover_index = index;
        self.hover_gen += 1;
        let Some(index) = index.filter(|i| *i < self.items.len()) else {
            return;
        };
        let running = self.items[index].match_ids.iter().any(|id| self.running.contains(id));
        if !running {
            return;
        }
        let generation = self.hover_gen;
        let Some(handle) = self.loop_handle.clone() else {
            return;
        };
        let wait = if self.previews_shown { 120 } else { 450 };
        let _ = handle.insert_source(Timer::from_duration(Duration::from_millis(wait)), move |_, _, dock: &mut Dock| {
            if dock.hover_gen == generation && dock.menu.is_none() && dock.press.is_none() {
                dock.show_previews(index);
            }
            TimeoutAction::Drop
        });
    }

    fn show_previews(&mut self, index: usize) {
        let Some(app) = self.items.get(index).cloned() else {
            return;
        };
        let windows = app_windows(&app);
        if windows.is_empty() {
            return;
        }
        let dock_x = icon_x(&self.items, index) + icon_size() / 2;
        let ids: Vec<String> = windows.iter().map(|(id, _)| id.to_string()).collect();
        tracing::info!("prévias de {}: {} janela(s)", app.name, ids.len());
        ask_compositor(&format!("preview {dock_x} {}", ids.join(" ")));
        self.previews_shown = true;
    }

    fn hide_previews(&mut self) {
        self.hover_gen += 1;
        if std::mem::take(&mut self.previews_shown) {
            ask_compositor("preview hide");
        }
    }
}

/// Janelas abertas de um app: id no compositor e título.
fn app_windows(app: &DockApp) -> Vec<(u64, String)> {
    ask_compositor("windows")
        .unwrap_or_default()
        .iter()
        .filter_map(|line| {
            let mut parts = line.split('\t');
            let id: u64 = parts.next()?.parse().ok()?;
            let app_id = parts.next()?.to_lowercase();
            let title = parts.next().unwrap_or("").to_string();
            app.match_ids.contains(&app_id).then_some((id, title))
        })
        .collect()
}

/// Desenha um painel no estilo dos menus do macOS: claro, translúcido, cantos
/// arredondados, sombra suave e destaque azul na linha sob o ponteiro.
/// Carrega as fontes numa thread e já compõe os textos mais comuns dos menus, para o
/// primeiro menu abrir na hora.
fn warm_text() -> std::sync::mpsc::Receiver<Text> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let start = std::time::Instant::now();
        let mut text = Text::new();
        let mut scratch = Pixmap::new(200, 40).unwrap();
        for (label, size, weight) in [
            ("Abrir Esvaziar Lixeira… Remover da Dock Encerrar", 13.0, FontWeight::Regular),
            ("Manter na Dock Ocultar Nova janela Mostrar na pasta", 13.0, FontWeight::Regular),
            ("Downloads Lixeira", 13.0, FontWeight::Semibold),
            ("há 2 min 0123456789", 11.5, FontWeight::Regular),
        ] {
            text.draw(&mut scratch, label, 0.0, 0.0, size, weight, [0, 0, 0, 255]);
        }
        tracing::debug!("fontes da dock prontas em {:.0} ms", start.elapsed().as_secs_f32() * 1000.0);
        let _ = tx.send(text);
    });
    rx
}

/// Altura da ponta dos menus da dock.
const ARROW_H: f32 = mide::ui::draw::ARROW_H;

/// Ponta do menu com o bico em (`cx`, `top + ARROW_H`), da cor do painel.
fn draw_arrow(pixmap: &mut Pixmap, cx: f32, top: f32, glass: bool) {
    mide::ui::draw::menu_arrow(pixmap, cx, top, true, [246, 246, 248, if glass { 180 } else { 242 }]);
}

fn draw_panel(pixmap: &mut Pixmap, text: &mut Text, panel: &mut Panel, glass: bool) {
    let (px, py, pw, ph) = panel.rect;
    for (grow, alpha) in [(6.0, 14u8), (3.0, 22), (1.0, 34)] {
        mide::ui::draw::fill_rounded(pixmap, px - grow, py - grow + 4.0, pw + 2.0 * grow, ph + 2.0 * grow, MENU_RADIUS + grow, Color::from_rgba8(0, 0, 0, alpha));
    }
    // Com vidro (desenhado pelo compositor atrás), o painel é translúcido: a sombra
    // não pode ficar por baixo dele.
    if glass {
        mide::ui::draw::clear_rounded(pixmap, px, py, pw, ph, MENU_RADIUS);
    }
    mide::ui::draw::fill_rounded(pixmap, px, py, pw, ph, MENU_RADIUS, Color::from_rgba8(246, 246, 248, if glass { 180 } else { 242 }));
    if let Some(path) = rounded_rect(px + 0.5, py + 0.5, pw - 1.0, ph - 1.0, MENU_RADIUS) {
        let mut paint = Paint::default();
        paint.set_color(Color::from_rgba8(0, 0, 0, 46));
        paint.anti_alias = true;
        pixmap.stroke_path(&path, &paint, &Stroke { width: 1.0, ..Default::default() }, Transform::identity(), None);
    }

    panel.spans.clear();
    let mut y = py + MENU_PAD;
    for (i, row) in panel.rows.iter().enumerate() {
        let h = row.height();
        panel.spans.push((y, y + h));
        let hovered = panel.hover == Some(i) && row.clickable();
        if hovered {
            mide::ui::draw::fill_rounded(pixmap, px + 5.0, y, pw - 10.0, h, 6.0, Color::from_rgba8(10, 100, 230, 255));
        }
        let (color, weight, size) = match row.kind {
            RowKind::Header => ([30, 30, 34, 255], FontWeight::Bold, 13.0),
            RowKind::Note => ([110, 110, 118, 255], FontWeight::Regular, 12.5),
            RowKind::Danger if !hovered => ([210, 40, 40, 255], FontWeight::Regular, 13.0),
            _ if hovered => ([255, 255, 255, 255], FontWeight::Regular, 13.0),
            _ if !row.clickable() => ([150, 150, 156, 255], FontWeight::Regular, 13.0),
            _ => ([30, 30, 34, 255], FontWeight::Regular, 13.0),
        };
        match row.kind {
            RowKind::Separator => {
                mide::ui::draw::fill_rounded(pixmap, px + 12.0, y + 4.0, pw - 24.0, 1.0, 0.0, Color::from_rgba8(0, 0, 0, 30));
            }
            _ => {
                let mut tx = px + 14.0;
                if let Some(icon) = &row.icon {
                    pixmap.draw_pixmap((px + 12.0) as i32, (y + (h - 24.0) / 2.0) as i32, icon.as_ref(), &Default::default(), Transform::identity(), None);
                    tx = px + 44.0;
                }
                let detail_w = if row.detail.is_empty() { 0.0 } else { text.measure(&row.detail, 11.5, FontWeight::Regular) + 10.0 };
                let max = px + pw - 14.0 - detail_w - tx;
                let label = fit_text(text, &row.label, size, weight, max);
                text.draw(pixmap, &label, tx, y + (h - size * 1.25) / 2.0, size, weight, color);
                if !row.detail.is_empty() {
                    let dc = if hovered { [230, 236, 255, 255] } else { [130, 130, 138, 255] };
                    let dw = detail_w - 10.0;
                    text.draw(pixmap, &row.detail, px + pw - 14.0 - dw, y + (h - 11.5 * 1.25) / 2.0, 11.5, FontWeight::Regular, dc);
                }
            }
        }
        y += h;
    }
}

/// Corta o texto com reticências para caber em `max` pixels.
fn fit_text(text: &mut Text, label: &str, size: f32, weight: FontWeight, max: f32) -> String {
    if text.measure(label, size, weight) <= max {
        return label.to_string();
    }
    let mut chars: Vec<char> = label.chars().collect();
    while !chars.is_empty() {
        chars.pop();
        let candidate = format!("{}…", chars.iter().collect::<String>().trim_end());
        if text.measure(&candidate, size, weight) <= max {
            return candidate;
        }
    }
    "…".into()
}

impl CompositorHandler for Dock {
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: i32) {}

    fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: wl_output::Transform) {}

    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {}

    fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}

    fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
}

impl OutputHandler for Dock {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}

    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}

    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl LayerShellHandler for Dock {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface) {
        if self.menu.as_ref().is_some_and(|m| &m.layer == layer) {
            self.menu = None;
            return;
        }
        self.exit = true;
    }

    fn configure(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _: u32,
    ) {
        if self.menu.as_ref().is_some_and(|m| &m.layer == layer) {
            self.screen = configure.new_size;
            self.draw_menu();
            return;
        }
        self.width = NonZeroU32::new(configure.new_size.0).map_or(self.width, NonZeroU32::get);
        self.height = NonZeroU32::new(configure.new_size.1).map_or(self.height, NonZeroU32::get);
        self.first_configure = false;
        self.draw(qh);
    }
}

impl SeatHandler for Dock {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}

    fn new_capability(&mut self, _: &Connection, qh: &QueueHandle<Self>, seat: wl_seat::WlSeat, capability: Capability) {
        if capability == Capability::Pointer && self.pointer.is_none() {
            if let Ok(pointer) = self.seat_state.get_pointer(qh, &seat) {
                self.pointer = Some(pointer);
            }
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

impl PointerHandler for Dock {
    fn pointer_frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_pointer::WlPointer, events: &[PointerEvent]) {
        for event in events {
            if self.menu.as_ref().is_some_and(|m| &event.surface == m.layer.wl_surface()) {
                self.menu_pointer(event);
                continue;
            }
            if &event.surface != self.layer.wl_surface() {
                continue;
            }
            match event.kind {
                PointerEventKind::Enter { .. } => {
                    self.pointer_x = event.position.0;
                    self.pointer_inside = true;
                    if self.autohide {
                        self.hide_at = None;
                        self.show_target = 1.0;
                        self.slide();
                    }
                }
                PointerEventKind::Motion { .. } => {
                    self.pointer_x = event.position.0;
                    self.drag_to(event.position.0);
                    self.hover_changed(self.icon_at(event.position.0));
                }
                PointerEventKind::Press { button, .. } if button == BTN_LEFT || button == BTN_RIGHT => {
                    self.hide_previews();
                    if button == BTN_RIGHT {
                        // Clique direito sempre abre o menu de contexto do ícone.
                        match self.icon_at(event.position.0) {
                            None => {}
                            Some(LAUNCHER) => self.toggle_menu(MenuKind::Launchpad, LAUNCHER),
                            Some(index) if index >= self.items.len() => self.stack_click(index - self.items.len(), true),
                            Some(index) => self.toggle_menu(MenuKind::App(index), index),
                        }
                        continue;
                    }
                    self.press = self.icon_at(event.position.0).map(|index| Press {
                        index,
                        start_x: event.position.0,
                        dragged: false,
                    });
                }
                PointerEventKind::Release { button, .. } if button == BTN_LEFT => {
                    if let Some(press) = self.press.take() {
                        // Clicar num app ou pasta da dock fecha o launcher, como no macOS.
                        if press.index != LAUNCHER && !press.dragged {
                            ask_compositor("launcher close");
                        }
                        if press.dragged {
                            save_pinned(&self.pinned_order());
                        } else if press.index == LAUNCHER {
                            self.menu = None;
                            tracing::info!("Launchpad");
                            ask_compositor("launcher");
                        } else if press.index >= self.items.len() {
                            self.stack_click(press.index - self.items.len(), false);
                        } else {
                            self.activate_or_launch(press.index);
                        }
                    }
                }
                PointerEventKind::Leave { .. } => {
                    self.press = None;
                    // As prévias somem pelo compositor quando o ponteiro sai da dock e do
                    // painel; aqui só cancela uma espera em andamento.
                    self.hover_index = None;
                    self.hover_gen += 1;
                    self.pointer_inside = false;
                    if self.autohide && self.covered {
                        self.schedule_hide();
                    }
                }
                _ => {}
            }
        }
    }
}

impl ShmHandler for Dock {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

delegate_registry!(Dock);
smithay_client_toolkit::delegate_dispatch2!(Dock);

impl ProvidesRegistryState for Dock {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }

    registry_handlers![OutputState, SeatState];
}
