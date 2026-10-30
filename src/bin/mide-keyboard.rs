//! Teclado na tela do mide (`mide-keyboard`), para tablets e notebooks conversíveis.
//!
//! É o método de entrada da sessão (`zwp_input_method_v2`): quando um app põe o foco
//! num campo de texto (`zwp_text_input_v3`: GTK, Qt, Chrome, Firefox, apps do mide), o
//! compositor avisa e o teclado sobe na borda de baixo; some quando o campo perde o
//! foco. Aparece sozinho no modo tablet (tela dobrada para trás; o compositor responde
//! `tablet_mode`) ou sempre, com Configurações → Teclado → "Mostrar o teclado na tela
//! ao digitar" (`[keyboard] on_screen`).
//!
//! O texto vai pronto ao app (`commit_string`), com acentos sem depender do layout:
//! segurar uma letra mostra as variações (á à â ã ä), como no iPad. Enter, setas, Tab
//! e Esc entram como teclas de verdade, pelo canal de controle (`osk key NOME`).

use std::{
    cell::{Cell, RefCell},
    error::Error,
    time::{Duration, Instant},
};

use mide::{
    shell::scale::{ScaleHandler, SurfaceScale},
    ui::{
        draw::{fill_rounded, rgba, to_argb8888},
        text::{FontWeight, Text},
    },
};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState, Region},
    delegate_registry,
    output::{OutputHandler, OutputState},
    reexports::{
        calloop::{
            timer::{TimeoutAction, Timer},
            EventLoop, LoopHandle, RegistrationToken,
        },
        calloop_wayland_source::WaylandSource,
        client::{
            globals::{registry_queue_init, GlobalList},
            protocol::{wl_output, wl_pointer, wl_seat, wl_shm, wl_surface, wl_touch},
            Connection, QueueHandle,
        },
        protocols_misc::zwp_input_method_v2::client::zwp_input_method_v2::ZwpInputMethodV2,
    },
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        input_method::{Active, InputMethod, InputMethodEventState, InputMethodHandler, InputMethodManager},
        pointer::{PointerEvent, PointerEventKind, PointerHandler},
        touch::TouchHandler,
        Capability, SeatHandler, SeatState,
    },
    shell::{
        wlr_layer::{Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface, LayerSurfaceConfigure},
        WaylandSurface,
    },
    shm::{slot::SlotPool, Shm, ShmHandler},
};
use tiny_skia::Pixmap;

/// Altura do teclado (pontos) e a faixa transparente acima dele, onde aparecem as
/// variações de uma letra segurada.
const PANEL_H: u32 = 236;
const POPUP_H: u32 = 64;
/// Largura máxima das teclas (telas largas: o teclado fica centrado).
const MAX_W: f32 = 1100.0;
const PAD: f32 = 8.0;
const GAP: f32 = 6.0;
/// Segurar uma letra até as variações aparecerem; segurar o apagar até repetir.
const LONG_PRESS: Duration = Duration::from_millis(450);
const REPEAT: Duration = Duration::from_millis(60);
const BTN_LEFT: u32 = 0x110;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    Letters,
    Numbers,
    Symbols,
}

#[derive(Debug, Clone, PartialEq)]
enum Action {
    Text(String),
    Shift,
    Backspace,
    Enter,
    Space,
    Page(Page),
    Hide,
}

#[derive(Debug, Clone)]
struct Key {
    label: String,
    action: Action,
    /// Largura em unidades (uma letra = 1).
    units: f32,
}

impl Key {
    fn text(s: &str) -> Self {
        Self { label: s.to_string(), action: Action::Text(s.to_string()), units: 1.0 }
    }
    fn special(label: &str, action: Action, units: f32) -> Self {
        Self { label: label.to_string(), action, units }
    }
}

/// Variações de uma letra (segurar), em minúsculas.
fn variants(letter: &str) -> &'static [&'static str] {
    match letter {
        "a" => &["á", "à", "â", "ã", "ä"],
        "e" => &["é", "è", "ê", "ë"],
        "i" => &["í", "ì", "î", "ï"],
        "o" => &["ó", "ò", "ô", "õ", "ö"],
        "u" => &["ú", "ù", "û", "ü"],
        "c" => &["ç"],
        "n" => &["ñ"],
        _ => &[],
    }
}

/// O layout do sistema tem o ç na fileira do meio (ABNT2)?
fn abnt() -> bool {
    let layouts = mide::settings_file::get("keyboard", "layouts").map(|v| format!("{v:?}")).unwrap_or_default();
    let first = mide::settings_file::get("keyboard", "layout").map(|v| format!("{v:?}")).unwrap_or_default();
    let all = format!("{layouts} {first}");
    // Sem nada escolhido, o do sistema: o idioma da sessão decide.
    if all.trim().is_empty() {
        return std::env::var("LANG").unwrap_or_default().starts_with("pt_BR");
    }
    all.contains("br")
}

/// As fileiras da página.
fn rows(page: Page, upper: bool, abnt: bool) -> Vec<Vec<Key>> {
    let letters = |s: &str| -> Vec<Key> {
        s.split(' ')
            .map(|c| {
                let c = if upper { c.to_uppercase() } else { c.to_string() };
                Key::text(&c)
            })
            .collect()
    };
    let bottom = |left: (&str, Page)| -> Vec<Key> {
        vec![
            Key::special(left.0, Action::Page(left.1), 1.5),
            Key::text(","),
            Key::special(" ", Action::Space, 5.0),
            Key::text("."),
            Key::special("⏎", Action::Enter, 1.5),
            Key::special("⌄", Action::Hide, 1.0),
        ]
    };
    match page {
        Page::Letters => {
            let mut middle = letters("a s d f g h j k l");
            if abnt {
                middle.push(Key::text(if upper { "Ç" } else { "ç" }));
            }
            let mut third = vec![Key::special("⇧", Action::Shift, 1.5)];
            third.extend(letters("z x c v b n m"));
            third.push(Key::special("⌫", Action::Backspace, 1.5));
            vec![letters("q w e r t y u i o p"), middle, third, bottom(("123", Page::Numbers))]
        }
        Page::Numbers | Page::Symbols => {
            let (first, second) = if page == Page::Numbers {
                ("1 2 3 4 5 6 7 8 9 0", "- / : ; ( ) $ & @ \"")
            } else {
                ("[ ] { } # % ^ * + =", "_ \\ | ~ < > € £ ¥ •")
            };
            let plain = |s: &str| -> Vec<Key> { s.split(' ').map(Key::text).collect() };
            let mut third = vec![if page == Page::Numbers {
                Key::special("#+=", Action::Page(Page::Symbols), 1.5)
            } else {
                Key::special("123", Action::Page(Page::Numbers), 1.5)
            }];
            third.extend(plain(". , ? ! '"));
            third.push(Key::special("⌫", Action::Backspace, 1.5));
            vec![plain(first), plain(second), third, bottom(("ABC", Page::Letters))]
        }
    }
}

/// Uma tecla desenhada: retângulo (pontos, na superfície) e o índice (fileira, coluna).
#[derive(Debug, Clone, Copy)]
struct Placed {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    row: usize,
    col: usize,
}

/// Posições das teclas numa superfície de `width` pontos (o painel começa em
/// `POPUP_H`).
fn layout(rows: &[Vec<Key>], width: f32) -> Vec<Placed> {
    let inner = (width - 2.0 * PAD).min(MAX_W);
    let left0 = (width - inner) / 2.0;
    let key_h = (PANEL_H as f32 - 2.0 * PAD - 3.0 * GAP) / 4.0;
    // A unidade sai da fileira de cima (10 teclas), igual para todas.
    let unit = (inner - 9.0 * GAP) / 10.0;
    let mut out = Vec::new();
    for (r, row) in rows.iter().enumerate() {
        let key_w = |k: &Key| k.units * unit + (k.units - 1.0) * GAP;
        let row_w = row.iter().map(key_w).sum::<f32>() + (row.len() as f32 - 1.0) * GAP;
        // A fileira de baixo e a terceira ocupam a largura toda; as outras, centradas.
        let (mut x, stretch) = if row_w > inner || r >= 2 { (left0, inner / row_w) } else { (left0 + (inner - row_w) / 2.0, 1.0) };
        let y = POPUP_H as f32 + PAD + r as f32 * (key_h + GAP);
        for (c, key) in row.iter().enumerate() {
            let w = key_w(key) * stretch;
            out.push(Placed { x, y, w, h: key_h, row: r, col: c });
            x += w + GAP * stretch;
        }
    }
    out
}

/// Letra segurada com as variações abertas.
struct Variants {
    /// Tecla de origem (fileira, coluna) e as variações.
    origin: (usize, usize),
    items: Vec<String>,
    /// Variação sob o dedo/ponteiro.
    chosen: Option<usize>,
}

struct Kbd {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    compositor: CompositorState,
    layer_shell: LayerShell,
    shm: Shm,
    pool: Option<SlotPool>,
    globals: GlobalList,
    qh: QueueHandle<Kbd>,
    handle: LoopHandle<'static, Kbd>,
    conn: Connection,
    im_manager: Option<InputMethodManager>,
    im: Option<InputMethod>,
    /// O que o compositor disse no último `done` (o tratador recebe `&self`).
    im_state: RefCell<Option<InputMethodEventState>>,
    im_changed: Cell<bool>,
    /// Outro método de entrada já ocupa a sessão.
    unavailable: Cell<bool>,
    active: bool,
    surrounding: bool,
    /// Escondido pela tecla de esconder (até o próximo campo).
    user_hidden: bool,
    tablet: bool,
    layer: Option<LayerSurface>,
    scale: Option<SurfaceScale>,
    configured: bool,
    size: (u32, u32),
    page: Page,
    shift: bool,
    caps: bool,
    last_shift: Option<Instant>,
    pressed: Option<(usize, usize)>,
    variants: Option<Variants>,
    timer: Option<RegistrationToken>,
    pointer: Option<wl_pointer::WlPointer>,
    touch: Option<wl_touch::WlTouch>,
    touch_id: Option<i32>,
    /// Último ponto do dedo (o `up` do toque não traz a posição).
    last_touch: Option<(f64, f64)>,
    text: Text,
    glass: mide::control_client::Glass,
    abnt: bool,
    placed: Vec<Placed>,
}

fn main() -> Result<(), Box<dyn Error>> {
    let _logging = mide::logging::init("mide-keyboard", "mide_keyboard=info");
    let conn = Connection::connect_to_env()?;
    let (globals, event_queue) = registry_queue_init(&conn)?;
    let qh = event_queue.handle();
    let mut event_loop: EventLoop<'static, Kbd> = EventLoop::try_new()?;
    let im_manager = InputMethodManager::bind(&globals, &qh).ok();
    if im_manager.is_none() {
        tracing::info!("teclado na tela: o compositor não oferece método de entrada");
        return Ok(());
    }
    let mut kbd = Kbd {
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),
        compositor: CompositorState::bind(&globals, &qh)?,
        layer_shell: LayerShell::bind(&globals, &qh)?,
        shm: Shm::bind(&globals, &qh)?,
        pool: None,
        globals,
        qh: qh.clone(),
        handle: event_loop.handle(),
        conn: conn.clone(),
        im_manager,
        im: None,
        im_state: RefCell::new(None),
        im_changed: Cell::new(false),
        unavailable: Cell::new(false),
        active: false,
        surrounding: false,
        user_hidden: false,
        tablet: false,
        layer: None,
        scale: None,
        configured: false,
        size: (0, 0),
        page: Page::Letters,
        shift: false,
        caps: false,
        last_shift: None,
        pressed: None,
        variants: None,
        timer: None,
        pointer: None,
        touch: None,
        touch_id: None,
        last_touch: None,
        text: Text::new(),
        glass: Default::default(),
        abnt: abnt(),
        placed: Vec::new(),
    };
    // O seat que já existe (o `new_seat` vale só para os que chegam depois).
    let mut event_queue = event_queue;
    event_queue.roundtrip(&mut kbd)?;
    if kbd.im.is_none()
        && let (Some(manager), Some(seat)) = (&kbd.im_manager, kbd.seat_state.seats().next())
    {
        kbd.im = Some(manager.get_input_method(&qh, &seat));
    }
    WaylandSource::new(conn.clone(), event_queue).insert(event_loop.handle()).map_err(|e| e.to_string())?;
    // Campo ativo com o teclado escondido: o modo tablet pode ter ligado depois.
    event_loop
        .handle()
        .insert_source(Timer::from_duration(Duration::from_secs(1)), |_, _, kbd: &mut Kbd| {
            if kbd.active && kbd.layer.is_none() && !kbd.user_hidden {
                kbd.sync_visibility(true);
            }
            TimeoutAction::ToDuration(Duration::from_secs(1))
        })
        .map_err(|e| e.to_string())?;
    tracing::info!("teclado na tela pronto");
    loop {
        event_loop.dispatch(None, &mut kbd)?;
        if kbd.unavailable.get() {
            tracing::info!("teclado na tela: outro método de entrada já está na sessão");
            return Ok(());
        }
        if kbd.im_changed.replace(false) {
            kbd.apply_im_state();
        }
    }
}

impl Kbd {
    /// O compositor mandou um estado novo do campo de texto.
    fn apply_im_state(&mut self) {
        let Some(state) = self.im_state.borrow().clone() else {
            return;
        };
        let (active, surrounding) = match state.active {
            Active::Active { surrounding_text, .. } => (true, surrounding_text),
            Active::NegotiatingCapabilities { surrounding_text, .. } => (true, surrounding_text),
            Active::Inactive => (false, false),
        };
        self.surrounding = surrounding;
        if active != self.active {
            self.active = active;
            tracing::info!("teclado na tela: campo de texto {}", if active { "ativo" } else { "inativo" });
            if active {
                // Campo novo: o teclado volta a valer, nas letras.
                self.user_hidden = false;
                self.page = Page::Letters;
                self.shift = false;
                self.caps = false;
            }
            self.sync_visibility(true);
        }
    }

    /// Mostra ou esconde conforme o campo, o modo tablet e a opção "sempre".
    fn sync_visibility(&mut self, ask_tablet: bool) {
        if ask_tablet {
            self.tablet = mide::control_client::ask_compositor("tablet_mode").is_some_and(|l| l.first().map(String::as_str) == Some("on"));
        }
        let always = mide::settings_file::flag("keyboard", "on_screen", false);
        let show = self.active && !self.user_hidden && (self.tablet || always);
        match (show, self.layer.is_some()) {
            (true, false) => self.open(),
            (false, true) => self.close(),
            _ => {}
        }
    }

    fn open(&mut self) {
        tracing::info!("teclado na tela: aberto");
        self.abnt = abnt();
        let surface = self.compositor.create_surface(&self.qh);
        self.scale = Some(SurfaceScale::new(&self.globals, &self.qh, &surface));
        let layer = self.layer_shell.create_layer_surface(&self.qh, surface, Layer::Top, Some("mide-keyboard"), None);
        layer.set_anchor(Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
        // Reserva a altura do painel: as janelas maximizadas sobem, o campo fica à vista.
        layer.set_exclusive_zone(PANEL_H as i32);
        // Nunca o teclado de verdade: o foco fica no app (o campo continua ativo).
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.set_size(0, PANEL_H + POPUP_H);
        layer.commit();
        self.layer = Some(layer);
        self.configured = false;
    }

    fn close(&mut self) {
        tracing::info!("teclado na tela: fechado");
        self.cancel_timer();
        self.glass.set("mide-keyboard", &[]);
        self.layer = None;
        self.scale = None;
        self.pool = None;
        self.configured = false;
        self.pressed = None;
        self.variants = None;
    }

    fn current_rows(&self) -> Vec<Vec<Key>> {
        rows(self.page, self.shift || self.caps, self.abnt)
    }

    fn key_at(&self, x: f64, y: f64) -> Option<(usize, usize)> {
        let (x, y) = (x as f32, y as f32);
        self.placed
            .iter()
            .find(|p| x >= p.x - GAP / 2.0 && x < p.x + p.w + GAP / 2.0 && y >= p.y - GAP / 2.0 && y < p.y + p.h + GAP / 2.0)
            .map(|p| (p.row, p.col))
    }

    fn key(&self, at: (usize, usize)) -> Option<Key> {
        self.current_rows().get(at.0).and_then(|r| r.get(at.1)).cloned()
    }

    /// Retângulos das variações abertas (pontos).
    fn variant_rects(&self) -> Vec<(f32, f32, f32, f32)> {
        let Some(v) = &self.variants else {
            return Vec::new();
        };
        let Some(origin) = self.placed.iter().find(|p| (p.row, p.col) == v.origin) else {
            return Vec::new();
        };
        let w = origin.w.max(44.0);
        let total = v.items.len() as f32 * w;
        let x0 = (origin.x + origin.w / 2.0 - total / 2.0).clamp(PAD, self.size.0 as f32 - PAD - total);
        let y = origin.y - POPUP_H as f32 + 4.0;
        (0..v.items.len()).map(|i| (x0 + i as f32 * w, y, w, POPUP_H as f32 - 12.0)).collect()
    }

    fn variant_at(&self, x: f64, y: f64) -> Option<usize> {
        let rects = self.variant_rects();
        let (x, y) = (x as f32, y as f32);
        // Na faixa das variações, a mais perto na horizontal (o dedo não precisa acertar).
        let (_, top, _, h) = *rects.first()?;
        if y > top + h + 40.0 {
            return None;
        }
        rects.iter().position(|(rx, _, rw, _)| x >= *rx && x < rx + rw).or_else(|| {
            if x < rects[0].0 {
                Some(0)
            } else {
                Some(rects.len() - 1)
            }
        })
    }

    fn cancel_timer(&mut self) {
        if let Some(token) = self.timer.take() {
            self.handle.remove(token);
        }
    }

    // --- entrada ---

    fn press(&mut self, x: f64, y: f64) {
        let Some(at) = self.key_at(x, y) else {
            return;
        };
        self.pressed = Some(at);
        let Some(key) = self.key(at) else {
            return;
        };
        self.cancel_timer();
        match &key.action {
            // Apagar: já na hora, e repetindo enquanto segura.
            Action::Backspace => {
                self.backspace();
                let token = self.handle.insert_source(Timer::from_duration(LONG_PRESS), |_, _, kbd: &mut Kbd| {
                    if kbd.pressed.and_then(|at| kbd.key(at)).is_some_and(|k| k.action == Action::Backspace) {
                        kbd.backspace();
                        TimeoutAction::ToDuration(REPEAT)
                    } else {
                        kbd.timer = None;
                        TimeoutAction::Drop
                    }
                });
                self.timer = token.ok();
            }
            Action::Text(t) if !variants(&t.to_lowercase()).is_empty() => {
                let token = self.handle.insert_source(Timer::from_duration(LONG_PRESS), move |_, _, kbd: &mut Kbd| {
                    kbd.timer = None;
                    if kbd.pressed == Some(at) {
                        kbd.open_variants(at);
                    }
                    TimeoutAction::Drop
                });
                self.timer = token.ok();
            }
            _ => {}
        }
        self.draw();
    }

    fn motion(&mut self, x: f64, y: f64) {
        if self.variants.is_some() {
            let chosen = self.variant_at(x, y);
            if let Some(v) = self.variants.as_mut()
                && v.chosen != chosen
            {
                v.chosen = chosen;
                self.draw();
            }
            return;
        }
        if self.pressed.is_some() {
            let at = self.key_at(x, y);
            if at != self.pressed {
                // Saiu da tecla: o toque não vale (como no celular).
                self.pressed = at;
                self.cancel_timer();
                self.draw();
            }
        }
    }

    fn release(&mut self, x: f64, y: f64) {
        self.cancel_timer();
        if let Some(v) = self.variants.take() {
            if let Some(item) = self.variant_at(x, y).or(v.chosen).and_then(|i| v.items.get(i).cloned()) {
                self.type_text(&item);
            }
            self.pressed = None;
            self.draw();
            return;
        }
        let Some(at) = self.pressed.take() else {
            return;
        };
        if self.key_at(x, y) == Some(at)
            && let Some(key) = self.key(at)
        {
            self.activate(key.action);
        }
        if self.layer.is_some() {
            self.draw();
        }
    }

    fn open_variants(&mut self, at: (usize, usize)) {
        let Some(Action::Text(t)) = self.key(at).map(|k| k.action) else {
            return;
        };
        let upper = t.chars().next().is_some_and(char::is_uppercase);
        let items: Vec<String> = variants(&t.to_lowercase()).iter().map(|v| if upper { v.to_uppercase() } else { v.to_string() }).collect();
        self.variants = Some(Variants { origin: at, items: items.clone(), chosen: Some(0) });
        let places: Vec<String> = items.iter().zip(self.variant_rects()).map(|(v, (x, y, w, h))| format!("{v}@{},{}", (x + w / 2.0) as i32, (y + h / 2.0) as i32)).collect();
        tracing::info!("teclado na tela: variações de {t}: {}", places.join(" "));
        self.draw();
    }

    fn activate(&mut self, action: Action) {
        match action {
            Action::Text(t) => self.type_text(&t),
            Action::Space => self.type_text(" "),
            Action::Backspace => {}
            Action::Enter => self.send_key("enter"),
            Action::Shift => {
                // Duas vezes seguidas: maiúsculas fixas.
                let double = self.last_shift.is_some_and(|t| t.elapsed() < Duration::from_millis(400));
                if self.caps {
                    self.caps = false;
                    self.shift = false;
                } else if double {
                    self.caps = true;
                    self.shift = false;
                } else {
                    self.shift = !self.shift;
                }
                self.last_shift = Some(Instant::now());
            }
            Action::Page(page) => self.page = page,
            Action::Hide => {
                self.user_hidden = true;
                self.close();
            }
        }
    }

    fn type_text(&mut self, text: &str) {
        let Some(im) = &self.im else {
            return;
        };
        im.commit_string(text.to_string());
        im.commit::<()>();
        let _ = self.conn.flush();
        tracing::debug!("teclado na tela: texto {text:?}");
        if self.shift && !self.caps {
            self.shift = false;
        }
    }

    fn backspace(&mut self) {
        match (&self.im, self.surrounding) {
            // O app diz o texto em volta: apaga o caractere antes do cursor pelo
            // protocolo (conta em bytes, então o tamanho do caractere).
            (Some(im), true) => {
                let state = self.im_state.borrow().clone();
                let before = state.map(|s| {
                    let cursor = (s.surrounding.cursor as usize).min(s.surrounding.text.len());
                    s.surrounding.text[..cursor].chars().next_back().map_or(0, char::len_utf8)
                });
                match before {
                    Some(n) if n > 0 => {
                        im.delete_surrounding_text(n as u32, 0);
                        im.commit::<()>();
                        let _ = self.conn.flush();
                    }
                    _ => self.send_key("backspace"),
                }
            }
            _ => self.send_key("backspace"),
        }
    }

    /// Tecla que não é texto, apertada pelo compositor no app com o foco.
    fn send_key(&self, name: &str) {
        // O texto mandado antes chega primeiro.
        let _ = self.conn.flush();
        let _ = mide::control_client::ask_compositor(&format!("osk key {name}"));
    }

    // --- desenho ---

    fn draw(&mut self) {
        let Some(layer) = self.layer.clone() else {
            return;
        };
        if !self.configured || self.size.0 == 0 {
            return;
        }
        let (pw, ph) = self.size;
        let rows = self.current_rows();
        let placed = layout(&rows, pw as f32);
        if placed.len() != self.placed.len() || self.placed.first().map(|p| (p.x, p.w)) != placed.first().map(|p| (p.x, p.w)) {
            // Para os testes: onde cada tecla está (centro, em pontos na camada).
            let keys: Vec<String> = placed
                .iter()
                .zip(rows.iter().flatten())
                .map(|(p, k)| format!("{}@{},{}", if k.label.trim().is_empty() { "espaço" } else { &k.label }, (p.x + p.w / 2.0) as i32, (p.y + p.h / 2.0) as i32))
                .collect();
            tracing::info!("teclado na tela: teclas {}", keys.join(" "));
        }
        self.placed = placed.clone();
        let scale = self.scale.as_ref().map_or(1.0, SurfaceScale::scale);
        let (w, h) = self.scale.as_ref().map_or((pw, ph), |s| s.pixels(pw, ph));
        mide::ui::draw::set_scale(scale);
        let Some(mut pixmap) = Pixmap::new(w, h) else {
            return;
        };
        let theme = mide::kit::Theme::system();
        let glass = mide::control_client::panel_glass();
        // Painel: vidro (o compositor desfoca por trás) com os cantos de cima redondos.
        let top = POPUP_H as f32;
        let [r, g, b, _] = theme.menu;
        fill_rounded(&mut pixmap, 0.0, top, pw as f32, PANEL_H as f32 + 20.0, 18.0, rgba(r, g, b, if glass { 196 } else { 246 }));
        let panel = [(0.0, top, pw as f32, PANEL_H as f32 + 20.0, 18.0)];
        self.glass.set("mide-keyboard", if glass { &panel } else { &[] });
        let key_fill = if theme.dark { rgba(255, 255, 255, 38) } else { rgba(255, 255, 255, 235) };
        let special_fill = if theme.dark { rgba(255, 255, 255, 18) } else { rgba(0, 0, 0, 26) };
        let [ar, ag, ab, _] = theme.accent;
        for (p, key) in placed.iter().zip(rows.iter().flatten()) {
            let pressed = self.pressed == Some((p.row, p.col)) && self.variants.is_none();
            let special = !matches!(key.action, Action::Text(_) | Action::Space);
            let lit = matches!(key.action, Action::Shift) && (self.shift || self.caps);
            let fill = if pressed || lit {
                rgba(ar, ag, ab, 255)
            } else if special {
                special_fill
            } else {
                key_fill
            };
            // Sombra fina embaixo, como as teclas do iPad.
            fill_rounded(&mut pixmap, p.x, p.y + 1.0, p.w, p.h, 9.0, rgba(0, 0, 0, if theme.dark { 60 } else { 40 }));
            fill_rounded(&mut pixmap, p.x, p.y, p.w, p.h, 9.0, fill);
            let label = if matches!(key.action, Action::Shift) && self.caps { "⇪".to_string() } else { key.label.clone() };
            // Letras grandes; símbolos das teclas especiais médios; "123"/"ABC" menores.
            let size = match (&key.action, label.chars().count()) {
                (Action::Text(_), 1) => 21.0,
                (_, 1) => 19.0,
                _ => 15.0,
            };
            let ink = if pressed || lit { theme.on_accent } else { theme.text };
            let tw = self.text.measure(&label, size, FontWeight::Regular);
            self.text.draw(&mut pixmap, &label, p.x + (p.w - tw) / 2.0, p.y + (p.h - size * 1.25) / 2.0, size, FontWeight::Regular, ink);
        }
        // Variações da letra segurada, numa pílula acima dela.
        let rects = self.variant_rects();
        if let Some(v) = &self.variants {
            if let (Some(first), Some(last)) = (rects.first(), rects.last()) {
                let (x, y, total_w, rh) = (first.0 - 4.0, first.1 - 4.0, last.0 + last.2 - first.0 + 8.0, first.3 + 8.0);
                fill_rounded(&mut pixmap, x, y + 2.0, total_w, rh, 14.0, rgba(0, 0, 0, 50));
                fill_rounded(&mut pixmap, x, y, total_w, rh, 14.0, {
                    let [r, g, b, _] = theme.menu;
                    rgba(r, g, b, 255)
                });
            }
            for (i, (rx, ry, rw, rh)) in rects.iter().enumerate() {
                let chosen = v.chosen == Some(i);
                if chosen {
                    fill_rounded(&mut pixmap, *rx + 2.0, *ry + 2.0, rw - 4.0, rh - 4.0, 10.0, rgba(ar, ag, ab, 255));
                }
                let item = &v.items[i];
                let tw = self.text.measure(item, 22.0, FontWeight::Regular);
                self.text.draw(&mut pixmap, item, rx + (rw - tw) / 2.0, ry + (rh - 27.5) / 2.0, 22.0, FontWeight::Regular, if chosen { theme.on_accent } else { theme.text });
            }
        }

        // Só o painel (e as variações abertas) recebem o toque; a faixa de cima é
        // transparente para quem está atrás.
        if let Ok(region) = Region::new(&self.compositor) {
            region.add(0, POPUP_H as i32, pw as i32, PANEL_H as i32);
            if let (Some(first), Some(last)) = (rects.first(), rects.last()) {
                region.add(first.0 as i32 - 4, first.1 as i32 - 4, (last.0 + last.2 - first.0) as i32 + 8, first.3 as i32 + 8);
            }
            layer.wl_surface().set_input_region(Some(region.wl_region()));
        }

        if self.pool.as_ref().is_none_or(|p| p.len() < (w * h * 4) as usize) {
            self.pool = SlotPool::new((w * h * 4) as usize, &self.shm).ok();
        }
        let Some(pool) = self.pool.as_mut() else {
            return;
        };
        let Ok((buffer, canvas)) = pool.create_buffer(w as i32, h as i32, w as i32 * 4, wl_shm::Format::Argb8888) else {
            return;
        };
        to_argb8888(&pixmap, canvas);
        let surface = layer.wl_surface();
        surface.damage_buffer(0, 0, w as i32, h as i32);
        if let Some(scale) = &self.scale {
            scale.set_destination(pw, ph);
        }
        if buffer.attach_to(surface).is_ok() {
            layer.commit();
        }
    }
}

impl InputMethodHandler for Kbd {
    fn handle_done(&self, _: &Connection, _: &QueueHandle<Self>, _: &ZwpInputMethodV2, state: &InputMethodEventState) {
        *self.im_state.borrow_mut() = Some(state.clone());
        self.im_changed.set(true);
    }

    fn handle_unavailable(&self, _: &Connection, _: &QueueHandle<Self>, _: &ZwpInputMethodV2) {
        self.unavailable.set(true);
    }
}

impl ScaleHandler for Kbd {
    fn surface_scale_changed(&mut self, _: &QueueHandle<Self>, _: &wl_surface::WlSurface) {
        self.draw();
    }
}

mide::delegate_surface_scale!(Kbd);

impl CompositorHandler for Kbd {
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: i32) {}
    fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: wl_output::Transform) {}
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {}
    fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
    fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
}

impl OutputHandler for Kbd {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl LayerShellHandler for Kbd {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        self.close();
    }

    fn configure(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface, configure: LayerSurfaceConfigure, _: u32) {
        if configure.new_size.0 > 0 && configure.new_size.1 > 0 {
            self.size = configure.new_size;
        }
        self.configured = true;
        self.draw();
    }
}

impl SeatHandler for Kbd {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }
    fn new_seat(&mut self, _: &Connection, qh: &QueueHandle<Self>, seat: wl_seat::WlSeat) {
        if self.im.is_none()
            && let Some(manager) = &self.im_manager
        {
            self.im = Some(manager.get_input_method(qh, &seat));
        }
    }
    fn new_capability(&mut self, _: &Connection, qh: &QueueHandle<Self>, seat: wl_seat::WlSeat, capability: Capability) {
        if capability == Capability::Pointer && self.pointer.is_none() {
            self.pointer = self.seat_state.get_pointer(qh, &seat).ok();
        }
        if capability == Capability::Touch && self.touch.is_none() {
            self.touch = self.seat_state.get_touch(qh, &seat).ok();
        }
    }
    fn remove_capability(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat, capability: Capability) {
        if capability == Capability::Pointer
            && let Some(pointer) = self.pointer.take()
        {
            pointer.release();
        }
        if capability == Capability::Touch
            && let Some(touch) = self.touch.take()
        {
            touch.release();
        }
    }
    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl PointerHandler for Kbd {
    fn pointer_frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_pointer::WlPointer, events: &[PointerEvent]) {
        for event in events {
            if self.layer.as_ref().map(|l| l.wl_surface()) != Some(&event.surface) {
                continue;
            }
            let (x, y) = event.position;
            match event.kind {
                PointerEventKind::Press { button, .. } if button == BTN_LEFT => self.press(x, y),
                PointerEventKind::Release { button, .. } if button == BTN_LEFT => self.release(x, y),
                PointerEventKind::Motion { .. } => self.motion(x, y),
                PointerEventKind::Leave { .. } if self.pressed.take().is_some() || self.variants.take().is_some() => {
                    self.cancel_timer();
                    self.draw();
                }
                _ => {}
            }
        }
    }
}

impl TouchHandler for Kbd {
    fn down(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_touch::WlTouch, _: u32, _: u32, surface: wl_surface::WlSurface, id: i32, position: (f64, f64)) {
        if self.layer.as_ref().map(|l| l.wl_surface()) != Some(&surface) || self.touch_id.is_some() {
            return;
        }
        self.touch_id = Some(id);
        self.last_touch = Some(position);
        self.press(position.0, position.1);
    }

    fn up(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_touch::WlTouch, _: u32, _: u32, id: i32) {
        if self.touch_id == Some(id) {
            self.touch_id = None;
            let last = self.last_touch.take().unwrap_or((-1.0, -1.0));
            self.release(last.0, last.1);
        }
    }

    fn motion(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_touch::WlTouch, _: u32, id: i32, position: (f64, f64)) {
        if self.touch_id == Some(id) {
            self.last_touch = Some(position);
            Kbd::motion(self, position.0, position.1);
        }
    }

    fn shape(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_touch::WlTouch, _: i32, _: f64, _: f64) {}
    fn orientation(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_touch::WlTouch, _: i32, _: f64) {}

    fn cancel(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_touch::WlTouch) {
        self.touch_id = None;
        self.pressed = None;
        self.variants = None;
        self.cancel_timer();
        self.draw();
    }
}

impl ShmHandler for Kbd {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

delegate_registry!(Kbd);
smithay_client_toolkit::delegate_dispatch2!(Kbd);

impl ProvidesRegistryState for Kbd {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fileiras_e_posicoes() {
        let br = rows(Page::Letters, false, true);
        assert_eq!(br[1].last().map(|k| k.label.as_str()), Some("ç"));
        assert!(rows(Page::Letters, false, false)[1].iter().all(|k| k.label != "ç"));
        assert_eq!(rows(Page::Letters, true, false)[0][0].label, "Q");
        // Nenhuma tecla sai da superfície nem se sobrepõe na fileira.
        for page in [Page::Letters, Page::Numbers, Page::Symbols] {
            let rows = rows(page, false, true);
            for width in [600.0, 1366.0, 2560.0] {
                let placed = layout(&rows, width);
                assert_eq!(placed.len(), rows.iter().map(Vec::len).sum::<usize>());
                for pair in placed.windows(2).filter(|p| p[0].row == p[1].row) {
                    assert!(pair[0].x + pair[0].w <= pair[1].x + 0.01, "{page:?} {width}");
                }
                assert!(placed.iter().all(|p| p.x >= 0.0 && p.x + p.w <= width + 0.01));
            }
        }
        assert_eq!(variants("a").len(), 5);
        assert!(variants("q").is_empty());
    }
}
