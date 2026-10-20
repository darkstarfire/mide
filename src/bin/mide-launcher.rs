//! Launcher do mide em tela cheia, no estilo do Launchpad do macOS.
//!
//! Camada sobreposta (`wlr-layer-shell`, Overlay) sobre o papel de parede, com um
//! véu leve. Mostra os apps instalados numa grade paginada de ícones grandes (bons
//! para o dedo também), a busca no topo e, no fim, as pastas Utilitários, Sistema e
//! Outros, com miniaturas dos apps de dentro, como no macOS.
//!
//! - Digitar: busca em todos os apps (sem diferenciar acentos). Enter abre o
//!   primeiro (ou o escolhido com as setas).
//! - Esc: fecha a pasta aberta; depois limpa a busca; com a busca vazia, fecha.
//! - Clique num app: abre e fecha o launcher. Clique numa pasta: abre o painel da
//!   pasta. Clique no vazio: fecha a pasta, ou o launcher.
//! - Rolar para baixo vai para a página da direita (carrossel: os apps deslizam
//!   para a esquerda); para cima, para a da esquerda.
//!   Clique do meio vai para a próxima página e, da última, volta para a primeira.
//!   Page Up/Down, setas nas bordas e os pontos também trocam a página.

use std::{collections::HashMap, error::Error, num::NonZeroU32, time::Duration};

use mide::{
    apps::{self, App},
    launcher_layout::{self, Layout, Slot},
    icons::load_icon,
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
        calloop::{
            channel,
            timer::{TimeoutAction, Timer},
            EventLoop,
        },
        calloop_wayland_source::WaylandSource,
        client::{
            globals::registry_queue_init,
            protocol::{wl_keyboard, wl_output, wl_pointer, wl_seat, wl_shm, wl_surface},
            Connection, QueueHandle,
        },
    },
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers, RawModifiers},
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
use tiny_skia::{FillRule, FilterQuality, Paint, PathBuilder, Pixmap, PixmapPaint, Stroke, Transform};

/// Tamanho dos ícones carregados; na tela, são reduzidos para o tamanho da grade.
const ICON_LOAD: u32 = 96;
/// Altura reservada no topo: a barra (transparente sobre o launcher) e a busca.
const TOP: u32 = 104;
/// Altura reservada embaixo: a dock e os pontos das páginas logo acima dela.
const BOTTOM: u32 = 104;
/// Distância dos pontos das páginas até a base da tela (acima da dock).
const DOTS_FROM_BOTTOM: f32 = 88.0;
const SIDE: u32 = 48;
const MAX_COLS: u32 = 8;
const MAX_ROWS: u32 = 5;
const SEARCH_W: f32 = 300.0;
const SEARCH_H: f32 = 32.0;
const SEARCH_Y: f32 = 46.0;
const BTN_LEFT: u32 = 0x110;
const BTN_MIDDLE: u32 = 0x112;
/// Rolagem acumulada (em pixels) para trocar de página.
const SCROLL_STEP: f64 = 40.0;
/// Colunas e linhas máximas dentro do painel de uma pasta.
const FOLDER_COLS: u32 = 4;
const FOLDER_ROWS: u32 = 3;

/// Medidas da grade para um tamanho de tela: ícones maiores em telas maiores.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Grid {
    icon: u32,
    cell_w: u32,
    cell_h: u32,
    cols: u32,
    rows: u32,
    x0: u32,
    y0: u32,
}

impl Grid {
    fn for_size(w: u32, h: u32) -> Self {
        let icon = ((w as f32 / 12.0).min(h as f32 / 8.5).round() as u32).clamp(56, 96);
        let cell_w = icon * 16 / 10;
        let cell_h = icon * 155 / 100 + 10;
        let cols = (w.saturating_sub(2 * SIDE) / cell_w).clamp(1, MAX_COLS);
        let rows = (h.saturating_sub(TOP + BOTTOM) / cell_h).clamp(1, MAX_ROWS);
        Self::place(icon, cell_w, cell_h, cols, rows, (0, TOP, w, h.saturating_sub(TOP + BOTTOM)))
    }

    /// Grade de `cols` x `rows` centralizada na área (x, y, largura, altura).
    fn place(icon: u32, cell_w: u32, cell_h: u32, cols: u32, rows: u32, area: (u32, u32, u32, u32)) -> Self {
        let (ax, ay, aw, ah) = area;
        let x0 = ax + aw.saturating_sub(cols * cell_w) / 2;
        let y0 = ay + ah.saturating_sub(rows * cell_h) / 2;
        Self { icon, cell_w, cell_h, cols, rows, x0, y0 }
    }

    fn per_page(&self) -> usize {
        (self.cols * self.rows) as usize
    }

    /// Canto superior esquerdo da célula `slot` (posição dentro da página).
    fn cell(&self, slot: usize) -> (u32, u32) {
        let slot = slot as u32;
        (self.x0 + (slot % self.cols) * self.cell_w, self.y0 + (slot / self.cols) * self.cell_h)
    }

    /// Posição dentro da página sob o ponto, se cair no ícone ou no nome de uma célula.
    fn slot_at(&self, x: f64, y: f64) -> Option<usize> {
        if x < self.x0 as f64 || y < self.y0 as f64 {
            return None;
        }
        let col = ((x - self.x0 as f64) / self.cell_w as f64) as u32;
        let row = ((y - self.y0 as f64) / self.cell_h as f64) as u32;
        if col >= self.cols || row >= self.rows {
            return None;
        }
        // Só a área do ícone e do nome conta, não os vãos entre células.
        let (cx, cy) = self.cell((row * self.cols + col) as usize);
        let margin = (self.cell_w - self.icon) as f64 / 2.0 - 6.0;
        let inside_x = x >= cx as f64 + margin && x <= (cx + self.cell_w) as f64 - margin;
        let inside_y = y <= (cy + 8 + self.icon + 26) as f64;
        (inside_x && inside_y).then(|| (row * self.cols + col) as usize)
    }
}

/// Número de páginas para `count` itens.
fn page_count(count: usize, per_page: usize) -> usize {
    count.div_ceil(per_page.max(1)).max(1)
}

/// Pasta da grade: nome, apps (índices em `apps`) e lugar dela na organização.
struct FolderView {
    name: String,
    apps: Vec<usize>,
    slot: usize,
}

/// Arrasto de um item em andamento.
#[derive(Debug, Clone, Copy)]
struct Drag {
    /// Índice do item arrastado na lista mostrada (grade ou pasta aberta).
    entry: usize,
    start: (f64, f64),
    pos: (f64, f64),
    /// Passou do limiar: é arrasto, não clique.
    active: bool,
}

/// Durações das animações.
const CLOSE_MS: f32 = 1000.0;
const PAGE_MS: f32 = 320.0;
/// Pasta abrindo ou fechando (cresce a partir do ícone, como o launcher abrindo).
const FOLDER_MS: f32 = 240.0;

/// Transição da pasta (`[effects] transitions`): a grade sem a pasta e a tela com a
/// pasta aberta são desenhadas uma vez; cada quadro copia a grade e desenha o painel
/// em escala, saindo do ícone da pasta.
struct FolderAnim {
    start: std::time::Instant,
    opening: bool,
    folder: usize,
    /// Ícone da pasta na grade (de onde o painel sai e para onde volta).
    origin: (f32, f32, f32, f32),
    frames: Option<(Pixmap, Pixmap, (f32, f32, f32, f32))>,
}
const POP_MS: f32 = 340.0;

/// Desaceleração suave (cúbica).
fn ease_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// Cresce passando um pouco do tamanho final e volta (efeito de mola).
fn ease_out_back(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let c = 1.70158;
    1.0 + (c + 1.0) * (t - 1.0).powi(3) + c * (t - 1.0).powi(2)
}

/// Troca de página em andamento: o carrossel desliza da página `from` para `to`.
struct PageAnim {
    from: usize,
    to: usize,
    /// Troca de página dentro da pasta aberta (desliza só dentro do painel dela).
    folder: bool,
    start: std::time::Instant,
    /// Os dois quadros (página antiga e nova) já no formato da tela e a faixa que
    /// desliza; nos quadros seguintes só as linhas são copiadas.
    frames: Option<(Vec<u8>, Vec<u8>, (usize, usize, usize, usize))>,
}

/// Distância mínima para um clique virar arrasto.
const DRAG_THRESHOLD: f64 = 8.0;
/// Faixa nas bordas da tela que troca de página durante o arrasto.
const EDGE: f64 = 28.0;

/// Um item da grade: app (índice em `apps`) ou pasta (índice em `folders`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Entry {
    App(usize),
    Folder(usize),
}

/// O que está sob o ponteiro.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    /// Índice na lista de itens mostrada (grade principal ou pasta aberta).
    Entry(usize),
    Dot(usize),
    Search,
    /// Dentro do painel da pasta, fora dos apps.
    FolderPanel,
    Empty,
}

fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "mide_launcher=info".into()),
        )
        .init();

    let conn = Connection::connect_to_env()?;
    let (globals, event_queue) = registry_queue_init(&conn)?;
    let qh = event_queue.handle();

    let compositor = CompositorState::bind(&globals, &qh)?;
    let layer_shell = LayerShell::bind(&globals, &qh)?;
    let shm = Shm::bind(&globals, &qh)?;

    let surface = compositor.create_surface(&qh);
    let layer = layer_shell.create_layer_surface(&qh, surface, Layer::Overlay, Some("mide-launcher"), None);
    layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
    layer.set_exclusive_zone(-1);
    layer.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
    layer.commit();

    let apps = apps::load();
    // Organização salva pelo usuário (ajustada aos apps instalados) ou a padrão.
    let layout = match Layout::load() {
        Some(saved) => saved.reconcile(&apps),
        None => Layout::default_for(&apps),
    };

    let started = std::time::Instant::now();
    let mut launcher = Launcher {
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),
        pool: SlotPool::new(800 * 600 * 4, &shm)?,
        shm,
        layer,
        width: 0,
        height: 0,
        exit: false,
        apps,
        layout,
        folders: Vec::new(),
        top_entries: Vec::new(),
        drag: None,
        last_flip: std::time::Instant::now(),
        editing: None,
        title_rect: None,
        open_folder: None,
        folder_page: 0,
        query: String::new(),
        page: 0,
        selected: None,
        hover: None,
        press: None,
        pointer: None,
        keyboard: None,
        scroll: 0.0,
        ctrl: false,
        text: Text::new(),
        icons: HashMap::new(),
        labels: HashMap::new(),
        dots: Vec::new(),
        folder_rect: None,
        started: Some(started),
        closing: None,
        page_anim: None,
        folder_anim: None,
        panel_only: false,
        last_frame: None,
        scaled: HashMap::new(),
        warmed: false,
        pop: None,
        opened: started,
        dirty: false,
    };

    launcher.rebuild();
    tracing::info!(
        "launcher com {} app(s) em {} lugar(es) e {} pasta(s)",
        launcher.apps.len(),
        launcher.top_entries.len(),
        launcher.folders.len()
    );

    let mut event_loop: EventLoop<Launcher> = EventLoop::try_new()?;
    WaylandSource::new(conn.clone(), event_queue)
        .insert(event_loop.handle())
        .map_err(|e| e.to_string())?;

    // Ícones carregados numa thread, cada um chega pelo canal. Ordem: os da primeira
    // página, as miniaturas das pastas e depois o resto.
    let (sender, receiver) = channel::channel::<(String, Option<Pixmap>)>();
    let loose: Vec<usize> = launcher
        .top_entries
        .iter()
        .filter_map(|e| match e {
            Entry::App(i) => Some(*i),
            Entry::Folder(_) => None,
        })
        .collect();
    let first_page = loose.len().min(MAX_COLS as usize * MAX_ROWS as usize);
    let mut order: Vec<usize> = loose[..first_page].to_vec();
    for folder in &launcher.folders {
        order.extend(folder.apps.iter().take(9));
    }
    order.extend(&loose[first_page..]);
    for folder in &launcher.folders {
        order.extend(folder.apps.iter().skip(9));
    }
    let wanted: Vec<(String, Option<String>)> = order
        .iter()
        .map(|i| (launcher.apps[*i].id.clone(), launcher.apps[*i].icon.clone()))
        .collect();
    // Quatro threads pegam os ícones da fila em ordem, então a prioridade se mantém.
    let queue = std::sync::Arc::new(std::sync::Mutex::new(wanted.into_iter()));
    for _ in 0..4 {
        let queue = queue.clone();
        let sender = sender.clone();
        std::thread::spawn(move || loop {
            let next = queue.lock().ok().and_then(|mut q| q.next());
            let Some((id, name)) = next else {
                break;
            };
            let icon = name.as_deref().and_then(|n| load_icon(n, ICON_LOAD));
            if sender.send((id, icon)).is_err() {
                break;
            }
        });
    }
    drop(sender);
    event_loop
        .handle()
        .insert_source(receiver, |event, _, launcher: &mut Launcher| {
            if let channel::Event::Msg((id, icon)) = event {
                // Já deixa o ícone reduzido no tamanho da grade: trocar de página não
                // precisa reduzir nada na hora.
                if let (Some(pix), true) = (icon.as_ref(), launcher.width > 0) {
                    let size = launcher.grid().icon as f32;
                    let key = (id.clone(), (size * 4.0).round() as u32);
                    let px = size.ceil().max(1.0) as u32;
                    let mut out = Pixmap::new(px, px).unwrap_or_else(|| Pixmap::new(1, 1).unwrap());
                    draw_scaled(&mut out, pix, 0.0, 0.0, size);
                    launcher.scaled.insert(key, out);
                }
                launcher.icons.insert(id, icon);
                if launcher.icons.len() == launcher.apps.len() {
                    tracing::info!("ícones carregados em {} ms", launcher.opened.elapsed().as_millis());
                }
                // Redesenha em lotes: o quadro sai depois de tratar o que já chegou.
                launcher.redraw();
            }
        })
        .map_err(|e| e.to_string())?;

    // Enquanto algo anima, um quadro a cada ~16 ms; parado, nada é redesenhado.
    event_loop
        .handle()
        .insert_source(Timer::from_duration(Duration::from_millis(16)), |_, _, launcher: &mut Launcher| {
            // Segurança: se o compositor não fechar a camada, sai depois de 1 s.
            if launcher.closing.is_some_and(|t| t.elapsed().as_secs_f32() * 1000.0 >= CLOSE_MS) {
                launcher.exit = true;
            } else if launcher.animating() {
                launcher.redraw();
            } else {
                launcher.warm_pages();
            }
            TimeoutAction::ToDuration(Duration::from_millis(16))
        })
        .map_err(|e| e.to_string())?;

    while !launcher.exit {
        event_loop.dispatch(None, &mut launcher)?;
        if launcher.dirty && !launcher.exit {
            launcher.render();
        }
    }
    Ok(())
}

struct Launcher {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    shm: Shm,
    pool: SlotPool,
    layer: LayerSurface,
    width: u32,
    height: u32,
    exit: bool,
    apps: Vec<App>,
    /// Ordem dos apps e pastas (salva em `launcher.toml`).
    layout: Layout,
    folders: Vec<FolderView>,
    /// Grade principal sem busca, na ordem da organização.
    top_entries: Vec<Entry>,
    drag: Option<Drag>,
    /// Última troca de página pela borda, durante um arrasto.
    last_flip: std::time::Instant,
    /// Nome da pasta aberta sendo editado (clique no título).
    editing: Option<String>,
    /// Retângulo do título da pasta aberta, para o clique que renomeia.
    title_rect: Option<(f32, f32, f32, f32)>,
    open_folder: Option<usize>,
    folder_page: usize,
    query: String,
    page: usize,
    /// Item escolhido pelas setas (índice na lista mostrada).
    selected: Option<usize>,
    hover: Option<Target>,
    press: Option<Target>,
    pointer: Option<wl_pointer::WlPointer>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    scroll: f64,
    ctrl: bool,
    text: Text,
    icons: HashMap<String, Option<Pixmap>>,
    /// Nomes já desenhados, por (texto, largura, negrito).
    labels: HashMap<(String, u32, bool), Pixmap>,
    /// Centros x dos pontos das páginas desenhados por último.
    dots: Vec<f32>,
    /// Painel da pasta aberta (x, y, largura, altura), do último desenho.
    folder_rect: Option<(f32, f32, f32, f32)>,
    /// Início, para registrar quanto tempo levou o primeiro quadro.
    started: Option<std::time::Instant>,
    /// Animações: fechar (pedido ao compositor), trocar de página e a pasta surgindo.
    closing: Option<std::time::Instant>,
    page_anim: Option<PageAnim>,
    folder_anim: Option<FolderAnim>,
    /// Desenhando só o painel da pasta, sobre fundo transparente (para a transição).
    panel_only: bool,
    /// Último quadro mostrado, no formato da tela (a página antiga do carrossel).
    last_frame: Option<Vec<u8>>,
    /// Ícones já reduzidos, por (app, tamanho×4).
    scaled: HashMap<(String, u32), Pixmap>,
    /// As outras páginas já foram desenhadas uma vez (caches prontos).
    warmed: bool,
    pop: Option<(usize, std::time::Instant)>,
    opened: std::time::Instant,
    dirty: bool,
}

impl Launcher {
    /// Monta a grade a partir da organização.
    fn rebuild(&mut self) {
        let index_of = |id: &str| self.apps.iter().position(|a| a.id == id);
        let mut folders = Vec::new();
        let mut entries = Vec::new();
        for (slot, item) in self.layout.slots.iter().enumerate() {
            match item {
                Slot::App(id) => {
                    if let Some(i) = index_of(id) {
                        entries.push(Entry::App(i));
                    }
                }
                Slot::Folder { name, apps } => {
                    entries.push(Entry::Folder(folders.len()));
                    folders.push(FolderView { name: name.clone(), apps: apps.iter().filter_map(|id| index_of(id)).collect(), slot });
                }
            }
        }
        self.folders = folders;
        self.top_entries = entries;
    }

    /// Salva a organização e remonta a grade.
    fn layout_changed(&mut self) {
        self.layout.save();
        let open_slot = self.open_folder.and_then(|f| self.folders.get(f)).map(|f| f.slot);
        self.rebuild();
        // A pasta aberta pode ter mudado de índice (ou deixado de existir).
        self.open_folder = open_slot.and_then(|slot| self.folders.iter().position(|f| f.slot == slot));
        self.redraw();
    }

    /// Solta o item arrastado no ponto (x, y).
    fn drop_drag(&mut self, drag: Drag) {
        let (x, y) = drag.pos;
        if let Some(f) = self.open_folder {
            let folder_slot = self.folders[f].slot;
            let inside = self.folder_rect.is_some_and(|(px, py, pw, ph)| {
                x >= px as f64 && x <= (px + pw) as f64 && y >= py as f64 && y <= (py + ph) as f64
            });
            let Some(app) = self.folders[f].apps.get(drag.entry).map(|i| self.apps[*i].id.clone()) else {
                return;
            };
            if !inside {
                // Arrastou para fora do painel: o app sai da pasta.
                tracing::info!("{app} saiu da pasta {}", self.folders[f].name);
                self.layout.take_out(folder_slot, &app);
                self.open_folder = None;
                self.folder_rect = None;
                self.layout_changed();
                return;
            }
            let (grid, page) = self.active_grid_page();
            if let Some(slot) = grid.slot_at(x, y) {
                let to = (page * grid.per_page() + slot).min(self.folders[f].apps.len() - 1);
                self.layout.move_in_folder(folder_slot, drag.entry, to);
                self.layout_changed();
            }
            return;
        }
        let (grid, page) = self.active_grid_page();
        let count = self.top_entries.len();
        let Some(slot) = grid.slot_at(x, y) else {
            return;
        };
        let target = page * grid.per_page() + slot;
        if target == drag.entry {
            return;
        }
        if target >= count {
            // Depois do último: vai para o fim.
            self.layout.move_slot(drag.entry, count - 1);
            self.layout_changed();
            return;
        }
        if self.combine_zone(&grid, slot, x, y) {
            let name = match (self.top_entries[drag.entry], self.top_entries[target]) {
                (Entry::App(a), Entry::App(b)) => launcher_layout::folder_name_for(&self.apps[a], &self.apps[b]),
                _ => "Pasta".to_string(),
            };
            if let Some(folder) = self.layout.drop_on(drag.entry, target, &name) {
                self.pop = Some((folder, std::time::Instant::now()));
                tracing::info!("pasta {:?} no lugar {folder}", self.layout.slots.get(folder).map(|s| match s {
                    Slot::Folder { name, apps } => format!("{name} ({} apps)", apps.len()),
                    Slot::App(id) => id.clone(),
                }));
            }
        } else {
            tracing::info!("app movido do lugar {} para {target}", drag.entry);
            self.layout.move_slot(drag.entry, target);
        }
        self.layout_changed();
    }

    /// O ponto está no miolo do ícone da célula (soltar ali junta numa pasta)?
    fn combine_zone(&self, grid: &Grid, slot: usize, x: f64, y: f64) -> bool {
        let (cx, cy) = grid.cell(slot);
        let ix = (cx + (grid.cell_w - grid.icon) / 2) as f64;
        let iy = (cy + 8) as f64;
        let m = grid.icon as f64 * 0.18;
        x >= ix + m && x <= ix + grid.icon as f64 - m && y >= iy + m && y <= iy + grid.icon as f64 - m
    }

    fn commit_rename(&mut self) {
        let (Some(name), Some(f)) = (self.editing.take(), self.open_folder) else {
            return;
        };
        let slot = self.folders[f].slot;
        tracing::info!("pasta renomeada para {name}");
        self.layout.rename(slot, &name);
        self.layout_changed();
    }

    fn grid(&self) -> Grid {
        Grid::for_size(self.width.max(1), self.height.max(1))
    }

    /// Grade dentro do painel da pasta aberta.
    fn folder_grid(&self) -> Grid {
        let main = self.grid();
        let count = self.open_folder.map(|f| self.folders[f].apps.len()).unwrap_or(0) as u32;
        let cols = FOLDER_COLS.min(main.cols).min(count.max(1));
        let rows = FOLDER_ROWS.min(main.rows).min(count.div_ceil(cols).max(1));
        let (pw, ph) = (cols * main.cell_w + 48, rows * main.cell_h + 40);
        let px = self.width.saturating_sub(pw) / 2;
        let py = (self.height.saturating_sub(ph) / 2).max(TOP + 30);
        Grid::place(main.icon, main.cell_w, main.cell_h, cols, rows, (px, py + 20, pw, ph - 28))
    }

    /// Itens da lista mostrada: a pasta aberta, o resultado da busca ou a grade principal.
    /// Registra onde ficam as pastas da página atual (os testes clicam nelas sem
    /// depender de quais apps estão instalados).
    fn log_folders(&self) {
        if self.open_folder.is_some() || !self.query.trim().is_empty() {
            return;
        }
        let grid = self.grid();
        let per_page = grid.per_page();
        for (index, entry) in self.entries().iter().enumerate().skip(self.page * per_page).take(per_page) {
            if let Entry::Folder(f) = entry {
                let (cx, cy) = grid.cell(index % per_page);
                let (x, y) = (cx + grid.cell_w / 2, cy + 8 + grid.icon / 2);
                tracing::info!("pasta {} na página {} em ({x}, {y})", self.folders[*f].name, self.page);
            }
        }
    }

    fn entries(&self) -> Vec<Entry> {
        if let Some(f) = self.open_folder {
            return self.folders[f].apps.iter().map(|i| Entry::App(*i)).collect();
        }
        if !self.query.trim().is_empty() {
            let found = apps::filter(&self.apps, &self.query, None);
            return found
                .iter()
                .filter_map(|a| self.apps.iter().position(|b| b.id == a.id))
                .map(Entry::App)
                .collect();
        }
        self.top_entries.clone()
    }

    /// Grade em uso e página atual (a da pasta aberta ou a principal).
    fn active_grid_page(&self) -> (Grid, usize) {
        if self.open_folder.is_some() {
            (self.folder_grid(), self.folder_page)
        } else {
            (self.grid(), self.page)
        }
    }

    fn pages(&self) -> usize {
        let (grid, _) = self.active_grid_page();
        page_count(self.entries().len(), grid.per_page())
    }

    fn target_at(&self, x: f64, y: f64) -> Target {
        let (sx, sy) = self.search_rect();
        if x >= sx as f64 && x <= (sx + SEARCH_W) as f64 && y >= sy as f64 && y <= (sy + SEARCH_H) as f64 {
            return Target::Search;
        }
        let (grid, page) = self.active_grid_page();
        if let Some(slot) = grid.slot_at(x, y) {
            let index = page * grid.per_page() + slot;
            if index < self.entries().len() {
                return Target::Entry(index);
            }
        }
        if let Some((px, py, pw, ph)) = self.folder_rect {
            let inside = x >= px as f64 && x <= (px + pw) as f64 && y >= py as f64 && y <= (py + ph) as f64;
            return if inside { Target::FolderPanel } else { Target::Empty };
        }
        let dot_y = self.height as f64 - DOTS_FROM_BOTTOM as f64;
        if (y - dot_y).abs() <= 10.0 {
            if let Some(i) = self.dots.iter().position(|cx| (x - *cx as f64).abs() <= 8.0) {
                return Target::Dot(i);
            }
        }
        Target::Empty
    }

    fn search_rect(&self) -> (f32, f32) {
        ((self.width as f32 - SEARCH_W) / 2.0, SEARCH_Y)
    }

    fn set_page(&mut self, page: usize) {
        let page = page.min(self.pages() - 1);
        if self.open_folder.is_some() {
            if page != self.folder_page {
                tracing::info!("página da pasta {} → {}", self.folder_page, page);
                self.page_anim = Some(PageAnim { from: self.folder_page, to: page, folder: true, start: std::time::Instant::now(), frames: None });
                self.folder_page = page;
                self.redraw();
            }
            return;
        }
        if page != self.page {
            // Carrossel: a página atual sai para um lado e a nova entra pelo outro.
            tracing::info!("página {} → {}", self.page, page);
            self.page_anim = Some(PageAnim { from: self.page, to: page, folder: false, start: std::time::Instant::now(), frames: None });
            self.page = page;
            self.log_folders();
            self.redraw();
        }
    }

    fn elapsed_ms(t: std::time::Instant) -> f32 {
        t.elapsed().as_secs_f32() * 1000.0
    }

    /// Alguma animação em andamento (pede quadros seguidos). Abrir e fechar são
    /// animados pelo compositor, na GPU: aqui só o carrossel e a pasta surgindo.
    fn animating(&self) -> bool {
        self.page_anim.is_some() || self.folder_anim.is_some() || self.pop.is_some_and(|(_, t)| Self::elapsed_ms(t) < POP_MS)
    }

    /// Ícone da pasta `f` na grade principal (página atual), para a transição.
    fn folder_icon_rect(&self, f: usize) -> Option<(f32, f32, f32, f32)> {
        let grid = self.grid();
        let index = self.entries().iter().position(|e| matches!(e, Entry::Folder(x) if *x == f))?;
        let slot = index % grid.per_page();
        let (cx, cy) = grid.cell(slot);
        Some(((cx + (grid.cell_w - grid.icon) / 2) as f32, (cy + 8) as f32, grid.icon as f32, grid.icon as f32))
    }

    /// Começa a transição da pasta, se ligada em Configurações → Efeitos.
    fn start_folder_anim(&mut self, folder: usize, opening: bool) {
        if !mide::settings_file::effect("transitions") {
            return;
        }
        // O ícone é procurado na grade principal (sem a pasta aberta).
        let open = self.open_folder.take();
        let origin = self.folder_icon_rect(folder);
        self.open_folder = open;
        if let Some(origin) = origin {
            self.folder_anim = Some(FolderAnim { start: std::time::Instant::now(), opening, folder, origin, frames: None });
        }
    }

    fn current_page(&self) -> usize {
        self.active_grid_page().1
    }

    fn query_changed(&mut self) {
        self.open_folder = None;
        self.folder_page = 0;
        self.page = 0;
        self.selected = if self.query.is_empty() { None } else { Some(0) };
        self.redraw();
    }

    /// Abre o item: app (e fecha o launcher) ou pasta (mostra o painel).
    fn activate(&mut self, index: usize) {
        match self.entries().get(index).copied() {
            Some(Entry::App(i)) => {
                let app = self.apps[i].clone();
                match apps::launch(&app) {
                    Ok(()) => tracing::info!("abrindo {} ({})", app.name, app.id),
                    Err(err) => tracing::warn!("falha ao abrir {}: {err}", app.name),
                }
                self.close();
            }
            Some(Entry::Folder(f)) => {
                tracing::info!("pasta {} aberta", self.folders[f].name);
                self.start_folder_anim(f, true);
                self.editing = None;
                self.open_folder = Some(f);
                self.folder_page = 0;
                self.selected = None;
                self.hover = None;
                self.redraw();
            }
            None => {}
        }
    }

    fn close_folder(&mut self) {
        self.commit_rename();
        self.page_anim = None;
        if let Some(f) = self.open_folder {
            self.start_folder_anim(f, false);
        }
        if self.open_folder.take().is_some() {
            tracing::info!("pasta fechada");
            self.folder_rect = None;
            self.selected = None;
            self.hover = None;
            self.redraw();
        }
    }

    /// Pede ao compositor para fechar: ele esmaece o launcher e depois fecha a
    /// camada (e o programa sai em `closed`). Sem o compositor do mide, sai na hora.
    fn close(&mut self) {
        if self.closing.is_none() {
            tracing::info!("launcher fechado");
            self.closing = Some(std::time::Instant::now());
            if mide::control_client::ask_compositor("launcher close").is_none() {
                self.exit = true;
            }
        }
    }

    /// Move a seleção das setas e acompanha a página.
    fn move_selection(&mut self, delta: isize) {
        let count = self.entries().len();
        if count == 0 {
            return;
        }
        let (grid, page) = self.active_grid_page();
        let current = self.selected.unwrap_or(page * grid.per_page()) as isize;
        let next = (current + delta).clamp(0, count as isize - 1) as usize;
        self.selected = Some(next);
        let page = next / grid.per_page();
        if self.open_folder.is_some() {
            self.folder_page = page;
        } else {
            self.page = page;
        }
        self.redraw();
    }

    /// Pede um quadro novo. O desenho acontece uma vez depois de tratar os eventos
    /// pendentes, então várias teclas de uma vez geram um só quadro.
    fn redraw(&mut self) {
        self.dirty = true;
    }

    fn render(&mut self) {
        let frame_start = std::time::Instant::now();
        self.dirty = false;
        if self.width == 0 || self.height == 0 {
            return;
        }
        // Pasta abrindo ou fechando.
        if let Some(t) = self.folder_anim.as_ref().map(|a| Self::elapsed_ms(a.start) / FOLDER_MS) {
            if t < 1.0 {
                self.present_folder_anim(t);
                tracing::debug!("quadro do launcher (pasta): {:.1} ms", frame_start.elapsed().as_secs_f32() * 1000.0);
                return;
            }
            self.folder_anim = None;
        }
        // Carrossel: as duas páginas são desenhadas uma vez, já no formato final, e
        // cada quadro só copia as linhas deslocadas (sem desenhar nada).
        let sliding = self.page_anim.as_ref().map(|a| (a.folder, a.from, a.to, Self::elapsed_ms(a.start) / PAGE_MS));
        if let Some((folder, from, to, t)) = sliding {
            if t < 1.0 && folder == self.open_folder.is_some() {
                if self.page_anim.as_ref().is_some_and(|a| a.frames.is_none()) {
                    // A página antiga é o que já está na tela: só a nova é desenhada.
                    let bytes = |p: &Pixmap| {
                        let mut out = vec![0u8; p.data().len()];
                        to_argb8888(p, &mut out);
                        out
                    };
                    let old = match self.last_frame.take() {
                        Some(frame) if frame.len() == (self.width * self.height * 4) as usize => frame,
                        _ => bytes(&self.paint_scene(Some(from), false)),
                    };
                    let new = self.paint_scene(Some(to), false);
                    let band = self.slide_band(folder);
                    if let Some(anim) = self.page_anim.as_mut() {
                        anim.frames = Some((old, bytes(&new), band));
                    }
                }
                let dir = if to > from { 1 } else { -1 };
                self.present_slide(t, dir);
                tracing::debug!("quadro do launcher (carrossel): {:.1} ms", frame_start.elapsed().as_secs_f32() * 1000.0);
                return;
            }
            self.page_anim = None;
        }
        let pixmap = self.paint_scene(None, true);
        self.present(&pixmap);
        if let Some(started) = self.started.take() {
            tracing::info!("launcher desenhado em {} ms", started.elapsed().as_millis());
        }
        tracing::debug!("quadro do launcher: {:.1} ms", frame_start.elapsed().as_secs_f32() * 1000.0);
    }

    /// Um quadro da transição da pasta: a grade, o escurecido e o painel (com o nome)
    /// crescendo do ícone até o lugar dele (ou voltando, ao fechar).
    fn present_folder_anim(&mut self, t: f32) {
        if self.folder_anim.as_ref().is_some_and(|a| a.frames.is_none()) {
            let Some(folder) = self.folder_anim.as_ref().map(|a| a.folder) else {
                return;
            };
            let current = self.open_folder;
            self.open_folder = None;
            let mut grid = self.paint_scene(None, false);
            // A grade já escurecida (como com a pasta aberta), feita uma vez só.
            let mut dim = Paint::default();
            dim.set_color(rgba(0, 0, 0, 60));
            if let Some(rect) = tiny_skia::Rect::from_xywh(0.0, 0.0, grid.width() as f32, grid.height() as f32) {
                grid.fill_rect(rect, &dim, Transform::identity(), None);
            }
            self.open_folder = Some(folder);
            self.panel_only = true;
            let open = self.paint_scene(None, false);
            self.panel_only = false;
            // Painel e o nome em cima dele.
            let panel = self.folder_rect.map(|(x, y, w, h)| (x, y - 56.0, w, h + 56.0)).unwrap_or((0.0, 0.0, 1.0, 1.0));
            self.open_folder = current;
            if let Some(anim) = self.folder_anim.as_mut() {
                anim.frames = Some((grid, open, panel));
            }
        }
        let Some(anim) = self.folder_anim.as_ref() else {
            return;
        };
        let Some((grid, open, panel)) = anim.frames.as_ref() else {
            return;
        };
        let eased = 1.0 - (1.0 - t).powi(3);
        let e = if anim.opening { eased } else { 1.0 - eased };
        let mut out = grid.clone();
        // O painel sai do ícone: tamanho e posição entre o ícone e o lugar final.
        let (px, py, pw, ph) = *panel;
        let (ox, oy, ow, oh) = anim.origin;
        let s = 0.25 + 0.75 * e;
        let (cx0, cy0) = (ox + ow / 2.0, oy + oh / 2.0);
        let (cx1, cy1) = (px + pw / 2.0, py + ph / 2.0);
        let (cx, cy) = (cx0 + (cx1 - cx0) * e, cy0 + (cy1 - cy0) * e);
        if let Some(crop) = tiny_skia::IntRect::from_xywh(px.max(0.0) as i32, py.max(0.0) as i32, pw as u32, ph as u32).and_then(|r| open.clone_rect(r)) {
            // Quase no tamanho final, sem filtro (mais barato e já nítido).
            let quality = if s > 0.97 { tiny_skia::FilterQuality::Nearest } else { tiny_skia::FilterQuality::Bilinear };
            let paint = tiny_skia::PixmapPaint { opacity: e.clamp(0.0, 1.0), quality, ..Default::default() };
            let transform = Transform::from_row(s, 0.0, 0.0, s, cx - pw * s / 2.0, cy - ph * s / 2.0);
            out.draw_pixmap(0, 0, crop.as_ref(), &paint, transform, None);
        }
        self.present(&out);
    }

    /// Parado e com os ícones carregados: desenha uma vez, fora da tela, as outras
    /// páginas, para os nomes e as miniaturas das pastas já estarem prontos quando o
    /// carrossel começar (a primeira troca de página não engasga).
    fn warm_pages(&mut self) {
        if self.warmed || self.width == 0 || self.icons.len() < self.apps.len() || self.open_folder.is_some() {
            return;
        }
        self.warmed = true;
        let pages = self.pages();
        for page in 0..pages {
            if page != self.page {
                let _ = self.paint_scene(Some(page), false);
            }
        }
        tracing::debug!("páginas preparadas: {pages}");
    }

    /// Faixa que desliza no carrossel (x0, y0, x1, y1): a grade inteira de lado a
    /// lado, ou só o miolo do painel da pasta.
    fn slide_band(&self, folder: bool) -> (usize, usize, usize, usize) {
        let (w, h) = (self.width as usize, self.height as usize);
        if folder {
            let fg = self.folder_grid();
            if let Some((px, _, pw, _)) = self.folder_rect {
                let y0 = fg.y0.saturating_sub(10) as usize;
                let y1 = (fg.y0 + fg.rows * fg.cell_h + 4) as usize;
                return ((px + 14.0) as usize, y0.min(h), (px + pw - 14.0) as usize, y1.min(h));
            }
        }
        let g = self.grid();
        let y0 = g.y0.saturating_sub(20) as usize;
        let y1 = (g.y0 + g.rows * g.cell_h + 10) as usize;
        (0, y0.min(h), w, y1.min(h))
    }

    /// Um quadro do carrossel: copia as linhas da página antiga deslocadas e a parte
    /// da página nova que já entrou, só dentro da faixa; fora dela, a página nova.
    fn present_slide(&mut self, t: f32, dir: i32) {
        let (w, h) = (self.width as usize, self.height as usize);
        let stride = w * 4;
        let Some((old, new, (bx0, by0, bx1, by1))) = self.page_anim.as_ref().and_then(|a| a.frames.clone()) else {
            return;
        };
        let bw = bx1.saturating_sub(bx0);
        let shift = ((ease_out(t) * bw as f32) as usize).min(bw);
        let Ok((buffer, canvas)) = self.pool.create_buffer(w as i32, h as i32, stride as i32, wl_shm::Format::Argb8888) else {
            return;
        };
        canvas.copy_from_slice(&new[..canvas.len().min(new.len())]);
        for y in by0..by1 {
            let row = y * stride;
            let at = |x: usize| row + x * 4;
            if dir > 0 {
                // A página antiga sai pela esquerda; a nova entra pela direita.
                canvas[at(bx0)..at(bx1 - shift)].copy_from_slice(&old[at(bx0 + shift)..at(bx1)]);
                canvas[at(bx1 - shift)..at(bx1)].copy_from_slice(&new[at(bx0)..at(bx0 + shift)]);
            } else {
                canvas[at(bx0 + shift)..at(bx1)].copy_from_slice(&old[at(bx0)..at(bx1 - shift)]);
                canvas[at(bx0)..at(bx0 + shift)].copy_from_slice(&new[at(bx1 - shift)..at(bx1)]);
            }
        }
        let surface = self.layer.wl_surface();
        surface.damage_buffer(0, 0, w as i32, h as i32);
        if buffer.attach_to(surface).is_ok() {
            self.layer.commit();
        }
    }

    fn present(&mut self, pixmap: &Pixmap) {
        let (w, h) = (self.width, self.height);
        let stride = w as i32 * 4;
        let (buffer, canvas) = match self.pool.create_buffer(w as i32, h as i32, stride, wl_shm::Format::Argb8888) {
            Ok(b) => b,
            Err(err) => {
                tracing::warn!("launcher: sem buffer para desenhar: {err}");
                return;
            }
        };
        to_argb8888(pixmap, canvas);
        // Guarda o quadro mostrado: vira a "página antiga" se o carrossel começar.
        let mut kept = self.last_frame.take().unwrap_or_default();
        kept.clear();
        kept.extend_from_slice(canvas);
        self.last_frame = Some(kept);
        let surface = self.layer.wl_surface();
        surface.damage_buffer(0, 0, w as i32, h as i32);
        if buffer.attach_to(surface).is_ok() {
            self.layer.commit();
        }
    }

    /// Desenha a tela inteira: véu, busca, grade (ou a pasta aberta) e o item
    /// arrastado. `page` troca a página mostrada (da grade, ou da pasta aberta).
    fn paint_scene(&mut self, page: Option<usize>, interactive: bool) -> Pixmap {
        let (w, h) = (self.width, self.height);
        let mut pixmap = Pixmap::new(w, h).unwrap_or_else(|| Pixmap::new(1, 1).unwrap());
        // Véu leve: o papel de parede (desfocado pelo compositor) continua aparecendo.
        if !self.panel_only {
            mide::ui::draw::fill_vertical_fast(&mut pixmap, [8, 10, 30, 95], [6, 6, 20, 130]);
            self.draw_search(&mut pixmap);
        }

        // Grade principal. Com uma pasta aberta, ela some (no macOS fica desfocada
        // atrás do painel) e só o painel da pasta aparece.
        let grid = self.grid();
        let main_entries = if self.open_folder.is_some() { Vec::new() } else { self.entries() };
        let pages = page_count(main_entries.len(), grid.per_page());
        self.page = self.page.min(pages - 1);
        if main_entries.is_empty() && self.open_folder.is_none() {
            let msg = "Nenhum app encontrado";
            let mw = self.text.measure(msg, 15.0, FontWeight::Regular);
            self.text.draw(&mut pixmap, msg, (w as f32 - mw) / 2.0, h as f32 / 2.0 - 10.0, 15.0, FontWeight::Regular, [255, 255, 255, 190]);
        }
        if self.open_folder.is_none() {
            let shown = page.unwrap_or(self.page).min(pages - 1);
            self.draw_entries(&mut pixmap, &grid, &main_entries, shown, interactive);
            self.draw_dots(&mut pixmap, pages, self.page, h as f32 - DOTS_FROM_BOTTOM, true);
        }

        // Pasta aberta: escurece o resto e mostra o painel com o nome em cima.
        self.folder_rect = None;
        self.title_rect = None;
        if let Some(f) = self.open_folder {
            if !self.panel_only {
                let mut dim = Paint::default();
                dim.set_color(rgba(0, 0, 0, 60));
                if let Some(rect) = tiny_skia::Rect::from_xywh(0.0, 0.0, w as f32, h as f32) {
                    pixmap.fill_rect(rect, &dim, Transform::identity(), None);
                }
            }
            let fg = self.folder_grid();
            let pw = (fg.cols * fg.cell_w + 48) as f32;
            let ph = (fg.rows * fg.cell_h + 40) as f32;
            let px = (w as f32 - pw) / 2.0;
            let py = fg.y0 as f32 - 20.0 - ((fg.rows * fg.cell_h + 40 - 28).saturating_sub(fg.rows * fg.cell_h) / 2) as f32;
            fill_rounded(&mut pixmap, px, py, pw, ph, 28.0, rgba(255, 255, 255, 46));
            if let Some(path) = rounded_rect(px + 0.5, py + 0.5, pw - 1.0, ph - 1.0, 27.5) {
                let mut border = Paint::default();
                border.set_color(rgba(255, 255, 255, 80));
                border.anti_alias = true;
                pixmap.stroke_path(&path, &border, &Stroke { width: 1.0, ..Default::default() }, Transform::identity(), None);
            }
            self.folder_rect = Some((px, py, pw, ph));
            // Nome da pasta; clicando nele, vira um campo para renomear.
            let name = self.editing.clone().unwrap_or_else(|| self.folders[f].name.clone());
            let tw = self.text.measure(&name, 24.0, FontWeight::Semibold);
            let (tx, ty) = ((w as f32 - tw) / 2.0, py - 46.0);
            if self.editing.is_some() {
                let fw = (tw + 40.0).max(180.0);
                fill_rounded(&mut pixmap, (w as f32 - fw) / 2.0, ty - 4.0, fw, 38.0, 10.0, rgba(255, 255, 255, 52));
                fill_rounded(&mut pixmap, tx + tw + 2.0, ty + 2.0, 2.0, 26.0, 1.0, rgba(255, 255, 255, 230));
            }
            self.text.draw(&mut pixmap, &name, tx, ty, 24.0, FontWeight::Semibold, [255, 255, 255, 245]);
            self.title_rect = Some((tx - 12.0, ty - 4.0, tw + 24.0, 38.0));
            let entries = self.entries();
            let fpages = page_count(entries.len(), fg.per_page());
            self.folder_page = self.folder_page.min(fpages - 1);
            let shown = page.unwrap_or(self.folder_page).min(fpages - 1);
            self.draw_entries(&mut pixmap, &fg, &entries, shown, interactive);
            self.draw_dots(&mut pixmap, fpages, self.folder_page, py + ph - 16.0, true);
        }

        // Item arrastado, por cima de tudo, centrado no ponteiro.
        if let Some(d) = self.drag.filter(|d| d.active) {
            let size = if self.open_folder.is_some() { self.folder_grid().icon } else { grid.icon } as f32;
            let (x, y) = (d.pos.0 as f32 - size / 2.0, d.pos.1 as f32 - size / 2.0);
            match self.entries().get(d.entry).copied() {
                Some(Entry::App(i)) => {
                    let app = self.apps[i].clone();
                    self.draw_app_icon(&mut pixmap, &app, x, y, size * 1.08);
                }
                Some(Entry::Folder(f)) => self.draw_folder_icon(&mut pixmap, f, x, y, size * 1.08),
                None => {}
            }
        }
        pixmap
    }

    /// Desenha uma página de itens na grade. `interactive` liga o destaque de seleção
    /// e de passagem do mouse.
    fn draw_entries(&mut self, pixmap: &mut Pixmap, grid: &Grid, entries: &[Entry], page: usize, interactive: bool) {
        let per_page = grid.per_page();
        let start = page * per_page;
        let drag = self.drag.filter(|d| d.active && interactive);
        for (slot, entry) in entries.iter().enumerate().skip(start).take(per_page).map(|(i, e)| (i - start, *e)) {
            let index = start + slot;
            let (cx, cy) = grid.cell(slot);
            let ix = cx + (grid.cell_w - grid.icon) / 2;
            let iy = cy + 8;
            if let Some(d) = drag {
                // O item arrastado segue o ponteiro; o lugar dele fica vazio.
                if d.entry == index {
                    continue;
                }
                // Alvo de "juntar numa pasta": um quadro claro em volta do ícone.
                if self.open_folder.is_none() && grid.slot_at(d.pos.0, d.pos.1) == Some(slot) && self.combine_zone(grid, slot, d.pos.0, d.pos.1) {
                    let pad = grid.icon as f32 * 0.12;
                    fill_rounded(pixmap, ix as f32 - pad, iy as f32 - pad, grid.icon as f32 + 2.0 * pad, grid.icon as f32 + 2.0 * pad, grid.icon as f32 * 0.3, rgba(255, 255, 255, 90));
                }
            }
            let highlighted = interactive && (self.selected == Some(index) || self.hover == Some(Target::Entry(index)));
            if highlighted {
                // O destaque cobre o ícone e o nome embaixo dele, com o vidro da dock.
                let pad = grid.icon as f32 * 0.12;
                let (hx, hw) = (cx as f32 + 4.0, grid.cell_w as f32 - 8.0);
                let hh = grid.icon as f32 + 2.0 * pad + 7.0 + 20.0;
                mide::ui::draw::glass(pixmap, hx, iy as f32 - pad, hw, hh, 15.0);
            }
            // Pasta recém-criada: surge crescendo com um pequeno efeito de mola.
            let grow = match self.pop {
                Some((p, t)) if p == index && interactive && self.open_folder.is_none() => {
                    let k = Self::elapsed_ms(t) / POP_MS;
                    if k >= 1.0 { 1.0 } else { 0.55 + 0.45 * ease_out_back(k) }
                }
                _ => 1.0,
            };
            let size = grid.icon as f32 * grow;
            let (gx, gy) = (ix as f32 + (grid.icon as f32 - size) / 2.0, iy as f32 + (grid.icon as f32 - size) / 2.0);
            let (label, bold) = match entry {
                Entry::App(i) => {
                    let app = self.apps[i].clone();
                    self.draw_app_icon(pixmap, &app, gx, gy, size);
                    (app.name.clone(), false)
                }
                Entry::Folder(f) => {
                    self.draw_folder_icon(pixmap, f, gx, gy, size);
                    (self.folders[f].name.clone(), true)
                }
            };
            let label = self.label(&label, grid.cell_w, bold);
            pixmap.draw_pixmap(cx as i32, (iy + grid.icon + 7) as i32, label.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
        }
    }

    /// Ícone de um app reduzido para `size`, ou um espaço discreto enquanto carrega.
    fn draw_app_icon(&mut self, pixmap: &mut Pixmap, app: &App, x: f32, y: f32, size: f32) {
        match self.icons.get(&app.id).cloned() {
            None => fill_rounded(pixmap, x + size * 0.1, y + size * 0.1, size * 0.8, size * 0.8, size * 0.22, rgba(255, 255, 255, 20)),
            Some(Some(icon)) => {
                // Ícone já reduzido para este tamanho (reduzir a cada quadro é caro).
                let key = (app.id.clone(), (size * 4.0).round() as u32);
                let scaled = self.scaled.entry(key).or_insert_with(|| {
                    let px = size.ceil().max(1.0) as u32;
                    let mut out = Pixmap::new(px, px).unwrap_or_else(|| Pixmap::new(1, 1).unwrap());
                    draw_scaled(&mut out, &icon, 0.0, 0.0, size);
                    out
                });
                pixmap.draw_pixmap(x.round() as i32, y.round() as i32, scaled.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
            }
            Some(None) => {
                // Sem ícone no tema: um bloco com a inicial do app.
                fill_rounded(pixmap, x + size * 0.06, y + size * 0.06, size * 0.88, size * 0.88, size * 0.22, rgba(110, 116, 136, 255));
                let initial: String = app.name.chars().take(1).collect::<String>().to_uppercase();
                let fs = size * 0.42;
                let iw = self.text.measure(&initial, fs, FontWeight::Semibold);
                self.text.draw(pixmap, &initial, x + (size - iw) / 2.0, y + (size - fs * 1.25) / 2.0, fs, FontWeight::Semibold, [255, 255, 255, 235]);
            }
        }
    }

    /// Pasta como no macOS: quadrado claro translúcido com miniaturas 3x3 dos apps.
    fn draw_folder_icon(&mut self, pixmap: &mut Pixmap, folder: usize, x: f32, y: f32, size: f32) {
        let inset = size * 0.04;
        let (bx, by, bs) = (x + inset, y + inset, size - 2.0 * inset);
        fill_rounded(pixmap, bx, by, bs, bs, bs * 0.24, rgba(255, 255, 255, 62));
        if let Some(path) = rounded_rect(bx + 0.5, by + 0.5, bs - 1.0, bs - 1.0, bs * 0.24) {
            let mut border = Paint::default();
            border.set_color(rgba(255, 255, 255, 105));
            border.anti_alias = true;
            pixmap.stroke_path(&path, &border, &Stroke { width: 1.0, ..Default::default() }, Transform::identity(), None);
        }
        let mini = bs * 0.22;
        let gap = bs * 0.07;
        let start_x = bx + (bs - 3.0 * mini - 2.0 * gap) / 2.0;
        let start_y = by + (bs - 3.0 * mini - 2.0 * gap) / 2.0;
        let apps: Vec<usize> = self.folders[folder].apps.iter().take(9).copied().collect();
        for (k, i) in apps.into_iter().enumerate() {
            let app = self.apps[i].clone();
            let mx = start_x + (k % 3) as f32 * (mini + gap);
            let my = start_y + (k / 3) as f32 * (mini + gap);
            self.draw_app_icon(pixmap, &app, mx, my, mini);
        }
    }

    fn draw_search(&mut self, pixmap: &mut Pixmap) {
        let (x, y) = self.search_rect();
        let r = SEARCH_H / 2.0;
        fill_rounded(pixmap, x, y, SEARCH_W, SEARCH_H, r, rgba(255, 255, 255, if self.query.is_empty() { 34 } else { 52 }));
        if let Some(path) = rounded_rect(x + 0.5, y + 0.5, SEARCH_W - 1.0, SEARCH_H - 1.0, r - 0.5) {
            let mut paint = Paint::default();
            paint.set_color(rgba(255, 255, 255, 70));
            paint.anti_alias = true;
            pixmap.stroke_path(&path, &paint, &Stroke { width: 1.0, ..Default::default() }, Transform::identity(), None);
        }
        // Lupa: círculo e cabo.
        let (lx, ly) = (x + 19.0, y + SEARCH_H / 2.0 - 1.5);
        let mut paint = Paint::default();
        paint.set_color(rgba(255, 255, 255, 200));
        paint.anti_alias = true;
        let stroke = Stroke { width: 1.7, ..Default::default() };
        if let Some(circle) = PathBuilder::from_circle(lx, ly, 5.5) {
            pixmap.stroke_path(&circle, &paint, &stroke, Transform::identity(), None);
        }
        let mut pb = PathBuilder::new();
        pb.move_to(lx + 4.0, ly + 4.0);
        pb.line_to(lx + 8.0, ly + 8.0);
        if let Some(handle) = pb.finish() {
            pixmap.stroke_path(&handle, &paint, &stroke, Transform::identity(), None);
        }

        let tx = x + 34.0;
        let ty = y + (SEARCH_H - 14.0 * 1.25) / 2.0;
        if self.query.is_empty() {
            self.text.draw(pixmap, "Buscar Aplicativos", tx, ty, 14.0, FontWeight::Regular, [255, 255, 255, 150]);
        } else {
            let query = self.query.clone();
            let tw = self.text.draw(pixmap, &query, tx, ty, 14.0, FontWeight::Regular, [255, 255, 255, 250]);
            fill_rounded(pixmap, tx + tw + 1.0, y + 8.0, 1.5, SEARCH_H - 16.0, 0.5, rgba(255, 255, 255, 220));
        }
    }

    fn draw_dots(&mut self, pixmap: &mut Pixmap, pages: usize, current: usize, y: f32, record: bool) {
        if record {
            self.dots.clear();
        }
        if pages < 2 {
            return;
        }
        let spacing = 18.0;
        let start = (self.width as f32 - (pages - 1) as f32 * spacing) / 2.0;
        for i in 0..pages {
            let x = start + i as f32 * spacing;
            let alpha = if i == current { 240 } else { 95 };
            if let Some(dot) = PathBuilder::from_circle(x, y, 4.0) {
                let mut paint = Paint::default();
                paint.set_color(rgba(255, 255, 255, alpha));
                paint.anti_alias = true;
                pixmap.fill_path(&dot, &paint, FillRule::Winding, Transform::identity(), None);
            }
            if record {
                self.dots.push(x);
            }
        }
    }

    /// Nome desenhado (e guardado) numa faixa da largura da célula, centralizado e
    /// cortado com reticências se não couber.
    fn label(&mut self, name: &str, width: u32, bold: bool) -> Pixmap {
        let key = (name.to_string(), width, bold);
        if let Some(p) = self.labels.get(&key) {
            return p.clone();
        }
        let size = 13.0;
        let weight = if bold { FontWeight::Semibold } else { FontWeight::Regular };
        let max = (width - 8) as f32;
        let mut text = name.to_string();
        if self.text.measure(&text, size, weight) > max {
            let mut chars: Vec<char> = name.chars().collect();
            while !chars.is_empty() {
                chars.pop();
                let candidate = format!("{}…", chars.iter().collect::<String>().trim_end());
                if self.text.measure(&candidate, size, weight) <= max {
                    text = candidate;
                    break;
                }
            }
        }
        let mut pixmap = Pixmap::new(width, 20).unwrap_or_else(|| Pixmap::new(1, 1).unwrap());
        let tw = self.text.measure(&text, size, weight);
        let x = (width as f32 - tw) / 2.0;
        // Sombra leve para ler sobre qualquer papel de parede.
        self.text.draw(&mut pixmap, &text, x, 1.0, size, weight, [0, 0, 0, 140]);
        self.text.draw(&mut pixmap, &text, x, 0.0, size, weight, [255, 255, 255, 245]);
        self.labels.insert(key, pixmap.clone());
        pixmap
    }
}

/// Desenha `icon` reduzido (ou ampliado) para `size` em (x, y), com filtro suave.
/// Bilinear, não bicúbico: o bicúbico "passa do ponto" nas bordas transparentes e gera
/// pixels com cor maior que a opacidade, que viram pontinhos brancos quando o
/// compositor esmaece o launcher ao fechar.
fn draw_scaled(pixmap: &mut Pixmap, icon: &Pixmap, x: f32, y: f32, size: f32) {
    let scale = size / icon.width().max(1) as f32;
    let paint = PixmapPaint { quality: FilterQuality::Bilinear, ..Default::default() };
    let transform = Transform::from_translate(x, y).pre_scale(scale, scale);
    pixmap.draw_pixmap(0, 0, icon.as_ref(), &paint, transform, None);
    mide::ui::draw::fix_premultiplied(pixmap);
}

impl CompositorHandler for Launcher {
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: i32) {}
    fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: wl_output::Transform) {}
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {}
    fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
    fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
}

impl OutputHandler for Launcher {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl LayerShellHandler for Launcher {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        self.exit = true;
    }

    fn configure(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface, configure: LayerSurfaceConfigure, _: u32) {
        self.width = NonZeroU32::new(configure.new_size.0).map_or(self.width.max(800), NonZeroU32::get);
        self.height = NonZeroU32::new(configure.new_size.1).map_or(self.height.max(600), NonZeroU32::get);
        self.labels.clear();
        self.redraw();
    }
}

impl SeatHandler for Launcher {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}

    fn new_capability(&mut self, _: &Connection, qh: &QueueHandle<Self>, seat: wl_seat::WlSeat, capability: Capability) {
        if capability == Capability::Pointer && self.pointer.is_none() {
            self.pointer = self.seat_state.get_pointer(qh, &seat).ok();
        }
        if capability == Capability::Keyboard && self.keyboard.is_none() {
            self.keyboard = self.seat_state.get_keyboard(qh, &seat, None).ok();
        }
    }

    fn remove_capability(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat, capability: Capability) {
        if capability == Capability::Pointer {
            if let Some(pointer) = self.pointer.take() {
                pointer.release();
            }
        }
        if capability == Capability::Keyboard {
            if let Some(keyboard) = self.keyboard.take() {
                keyboard.release();
            }
        }
    }

    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl KeyboardHandler for Launcher {
    fn enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: &wl_surface::WlSurface, _: u32, _: &[u32], _: &[Keysym]) {}

    fn leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: &wl_surface::WlSurface, _: u32) {}

    fn press_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, event: KeyEvent) {
        self.key(event);
    }

    fn repeat_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, event: KeyEvent) {
        self.key(event);
    }

    fn release_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, _: KeyEvent) {}

    fn update_modifiers(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, modifiers: Modifiers, _: RawModifiers, _: u32) {
        self.ctrl = modifiers.ctrl;
    }
}

impl Launcher {
    fn key(&mut self, event: KeyEvent) {
        // Renomeando a pasta: as teclas vão para o nome.
        if let Some(name) = self.editing.as_mut() {
            match event.keysym {
                Keysym::Escape => self.editing = None,
                Keysym::Return | Keysym::KP_Enter => self.commit_rename(),
                Keysym::BackSpace => {
                    name.pop();
                }
                _ => {
                    if let Some(typed) = event.utf8.filter(|t| !t.is_empty() && t.chars().all(|c| !c.is_control())) {
                        name.push_str(&typed);
                    }
                }
            }
            self.redraw();
            return;
        }
        let (grid, page) = self.active_grid_page();
        let cols = grid.cols as isize;
        match event.keysym {
            Keysym::Escape => {
                if self.open_folder.is_some() {
                    self.close_folder();
                } else if self.query.is_empty() {
                    self.close();
                } else {
                    self.query.clear();
                    self.query_changed();
                }
            }
            Keysym::Return | Keysym::KP_Enter => {
                let index = self.selected.unwrap_or(page * grid.per_page());
                if index < self.entries().len() {
                    self.activate(index);
                }
            }
            Keysym::BackSpace => {
                if self.ctrl {
                    self.query.clear();
                } else {
                    self.query.pop();
                }
                self.query_changed();
            }
            Keysym::Left => self.move_selection(-1),
            Keysym::Right => self.move_selection(1),
            Keysym::Up => self.move_selection(-cols),
            Keysym::Down => self.move_selection(cols),
            Keysym::Page_Up => {
                let page = self.current_page().saturating_sub(1);
                self.set_page(page);
            }
            Keysym::Page_Down => {
                let page = self.current_page() + 1;
                self.set_page(page);
            }
            _ => {
                if self.ctrl {
                    return;
                }
                let Some(typed) = event.utf8.filter(|t| !t.is_empty() && t.chars().all(|c| !c.is_control())) else {
                    return;
                };
                self.query.push_str(&typed);
                tracing::info!("busca: {}", self.query);
                self.query_changed();
            }
        }
    }
}

impl PointerHandler for Launcher {
    fn pointer_frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_pointer::WlPointer, events: &[PointerEvent]) {
        for event in events {
            if &event.surface != self.layer.wl_surface() {
                continue;
            }
            let (x, y) = event.position;
            match event.kind {
                PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => {
                    if let Some(drag) = self.drag.as_mut() {
                        drag.pos = (x, y);
                        let moved = ((x - drag.start.0).powi(2) + (y - drag.start.1).powi(2)).sqrt();
                        if !drag.active && moved >= DRAG_THRESHOLD {
                            drag.active = true;
                            tracing::info!("arrastando o item {}", drag.entry);
                        }
                        if drag.active {
                            // Perto da borda, troca de página (no máximo a cada 0,7 s).
                            if self.last_flip.elapsed() >= std::time::Duration::from_millis(700) && self.open_folder.is_none() {
                                let page = self.current_page();
                                if x < EDGE && page > 0 {
                                    self.set_page(page - 1);
                                    self.last_flip = std::time::Instant::now();
                                } else if x > self.width as f64 - EDGE {
                                    self.set_page(page + 1);
                                    self.last_flip = std::time::Instant::now();
                                }
                            }
                            self.redraw();
                            continue;
                        }
                    }
                    let target = self.target_at(x, y);
                    let hover = matches!(target, Target::Entry(_) | Target::Dot(_)).then_some(target);
                    if hover != self.hover {
                        self.hover = hover;
                        self.redraw();
                    }
                }
                PointerEventKind::Leave { .. } => {
                    if self.hover.take().is_some() {
                        self.redraw();
                    }
                }
                PointerEventKind::Press { button, .. } if button == BTN_LEFT => {
                    // Clique fora do título encerra a edição do nome (salvando).
                    let on_title = self.title_rect.is_some_and(|(tx, ty, tw, th)| {
                        x >= tx as f64 && x <= (tx + tw) as f64 && y >= ty as f64 && y <= (ty + th) as f64
                    });
                    if self.editing.is_some() && !on_title {
                        self.commit_rename();
                    }
                    if on_title && self.editing.is_none() {
                        if let Some(f) = self.open_folder {
                            tracing::info!("renomeando a pasta {}", self.folders[f].name);
                            self.editing = Some(self.folders[f].name.clone());
                            self.redraw();
                        }
                        continue;
                    }
                    let target = self.target_at(x, y);
                    self.press = Some(target);
                    // Arrastar só com a grade organizada (sem busca).
                    if let Target::Entry(entry) = target {
                        if self.query.trim().is_empty() {
                            self.drag = Some(Drag { entry, start: (x, y), pos: (x, y), active: false });
                        }
                    }
                }
                PointerEventKind::Press { button, .. } if button == BTN_MIDDLE => {
                    // Próxima página; da última, volta para a primeira.
                    let pages = self.pages();
                    let next = (self.current_page() + 1) % pages;
                    self.set_page(next);
                }
                PointerEventKind::Release { button, .. } if button == BTN_LEFT => {
                    if let Some(drag) = self.drag.take() {
                        if drag.active {
                            self.press = None;
                            self.drop_drag(Drag { pos: (x, y), ..drag });
                            self.redraw();
                            continue;
                        }
                    }
                    let target = self.target_at(x, y);
                    if self.press.take() != Some(target) {
                        continue;
                    }
                    match target {
                        Target::Entry(index) => self.activate(index),
                        Target::Dot(page) => self.set_page(page),
                        Target::Search | Target::FolderPanel => {}
                        Target::Empty if self.open_folder.is_some() => self.close_folder(),
                        Target::Empty => self.close(),
                    }
                }
                PointerEventKind::Axis { horizontal, vertical, .. } => {
                    // Roda do mouse: passos (v120 ou o "discrete" antigo); touchpad: valor contínuo.
                    let steps = (vertical.value120 + horizontal.value120) as f64 / 120.0;
                    let discrete = (vertical.discrete + horizontal.discrete) as f64;
                    let delta = if vertical.absolute.abs() > horizontal.absolute.abs() { vertical.absolute } else { horizontal.absolute };
                    let amount = if steps != 0.0 { steps * SCROLL_STEP } else if discrete != 0.0 { discrete * SCROLL_STEP } else { delta };
                    // Um passo por troca: com o carrossel andando, a roda espera.
                    if self.page_anim.is_some() {
                        self.scroll = 0.0;
                        continue;
                    }
                    self.scroll += amount;
                    if self.scroll.abs() >= SCROLL_STEP {
                        // Para baixo (valor positivo): página da direita (os apps vão
                        // para a esquerda); para cima: página da esquerda.
                        let current = self.current_page();
                        let page = if self.scroll > 0.0 { current + 1 } else { current.saturating_sub(1) };
                        self.scroll = 0.0;
                        self.set_page(page);
                    }
                }
                _ => {}
            }
        }
    }
}

impl ShmHandler for Launcher {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

delegate_registry!(Launcher);
smithay_client_toolkit::delegate_dispatch2!(Launcher);

impl ProvidesRegistryState for Launcher {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }

    registry_handlers![OutputState, SeatState];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grade_em_800x600() {
        let g = Grid::for_size(800, 600);
        assert_eq!(g.icon, 67);
        assert_eq!((g.cols, g.rows), (6, 3));
        assert_eq!(g.per_page(), 18);
        assert_eq!(g.cell(0), (g.x0, g.y0));
        assert_eq!(g.cell(7), (g.x0 + g.cell_w, g.y0 + g.cell_h));
    }

    #[test]
    fn icones_maiores_em_tela_grande_e_grade_limitada() {
        let g = Grid::for_size(3840, 2160);
        assert_eq!(g.icon, 96);
        assert_eq!((g.cols, g.rows), (MAX_COLS, MAX_ROWS));
    }

    #[test]
    fn celula_sob_o_ponto() {
        let g = Grid::for_size(800, 600);
        let center = |slot: usize| {
            let (x, y) = g.cell(slot);
            ((x + g.cell_w / 2) as f64, (y + 8 + g.icon / 2) as f64)
        };
        assert_eq!(g.slot_at(center(0).0, center(0).1), Some(0));
        assert_eq!(g.slot_at(center(7).0, center(7).1), Some(7));
        assert_eq!(g.slot_at(0.0, 0.0), None);
    }

    #[test]
    fn paginas() {
        assert_eq!(page_count(0, 18), 1);
        assert_eq!(page_count(18, 18), 1);
        assert_eq!(page_count(19, 18), 2);
    }
}
