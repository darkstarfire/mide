//! Teclado na tela do mide (`mide-keyboard`), para tablets e notebooks conversíveis.
//!
//! É o método de entrada da sessão (`zwp_input_method_v2`): quando um app põe o foco
//! num campo de texto (`zwp_text_input_v3`: GTK, Qt, Firefox, Chrome com
//! `--enable-wayland-ime --wayland-text-input-version=3`, apps do mide-ui), o
//! compositor avisa e o teclado sobe na borda de baixo; some quando o campo perde o
//! foco. Aparece sozinho no modo tablet (tela dobrada para trás; o compositor responde
//! `tablet_mode`) ou sempre, com Configurações → Teclado → "Mostrar o teclado na tela
//! ao digitar" (`[keyboard] on_screen`).
//!
//! Como o teclado do iPad:
//! - O texto vai pronto ao app (`commit_string`), com acentos sem depender do layout:
//!   segurar uma letra mostra as variações (á à â ã ä).
//! - Puxar uma letra da fileira de cima para baixo digita o número dela (o número
//!   aparece pequeno em cima da letra).
//! - Shift duas vezes: maiúsculas fixas. Dois espaços: ponto final e espaço
//!   (`[keyboard] osk_period`).
//! - Segurar o espaço (ou arrastar o dedo sobre ele): as teclas somem e o teclado vira
//!   um trackpad que anda com o cursor do texto.
//! - Barra de atalhos em cima: desfazer, refazer, recortar, copiar, colar; à direita, o
//!   teclado flutuante (menor, arrastável pela alça embaixo; soltar perto da borda de
//!   baixo, ou abrir os dedos, volta a acoplar; juntar os dedos sobre o teclado o deixa
//!   flutuante).
//! - Som das teclas (`[keyboard] osk_sound`) e o realce da tecla tocada.
//! - Sobe deslizando ao aparecer (`mide::motion::KEYBOARD_SLIDE`).
//!
//! Enter, apagar, setas e os atalhos entram como teclas de verdade, pelo canal de
//! controle (`osk key NOME`, `keys COMBINAÇÃO`).

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
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

/// Altura das teclas acoplado (pontos), a barra de atalhos em cima delas e a faixa
/// transparente acima de tudo, onde aparecem as variações de uma letra segurada.
const PANEL_H: u32 = 236;
const BAR_H: u32 = 40;
const POPUP_H: u32 = 64;
/// Teclado flutuante: largura, altura das teclas e a alça de arrastar embaixo.
const FLOAT_W: u32 = 400;
const FLOAT_KEYS_H: u32 = 196;
const HANDLE_H: u32 = 22;
/// Largura máxima das teclas (telas largas: o teclado fica centrado).
const MAX_W: f32 = 1100.0;
const PAD: f32 = 8.0;
const GAP: f32 = 6.0;
/// Segurar uma letra até as variações aparecerem (e o espaço até virar trackpad);
/// segurar o apagar até repetir.
const LONG_PRESS: Duration = Duration::from_millis(450);
const REPEAT: Duration = Duration::from_millis(60);
/// Dois espaços seguidos neste tempo viram ". ".
const DOUBLE_SPACE: Duration = Duration::from_millis(450);
/// Puxar a letra para baixo este tanto (pontos) digita o número.
const FLICK: f64 = 18.0;
/// Trackpad do espaço: pontos de dedo por passo do cursor (lado e cima/baixo).
const TRACK_X: f64 = 11.0;
const TRACK_Y: f64 = 20.0;
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
    /// Atalho de teclado (`ctrl+z`...), pela barra de atalhos.
    Combo(&'static str),
    /// Flutuante ↔ acoplado.
    Float,
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

/// Símbolo desenhado no lugar do nome da tecla (os do mide-ui, nítidos em qualquer
/// escala), ou `None` para escrever o nome.
fn key_symbol(action: &Action, shift_on: bool, caps: bool) -> Option<&'static str> {
    match action {
        Action::Shift if caps => Some("caps-lock-symbolic"),
        Action::Shift if shift_on => Some("keyboard-shift-on-symbolic"),
        Action::Shift => Some("keyboard-shift-symbolic"),
        Action::Backspace => Some("keyboard-backspace-symbolic"),
        Action::Enter => Some("keyboard-return-symbolic"),
        Action::Hide => Some("keyboard-hide-symbolic"),
        _ => None,
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

/// Número da letra da fileira de cima (puxar para baixo), pela coluna.
fn flick_digit(page: Page, row: usize, col: usize) -> Option<&'static str> {
    (page == Page::Letters && row == 0).then(|| ["1", "2", "3", "4", "5", "6", "7", "8", "9", "0"].get(col).copied()).flatten()
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

/// Botões da barra de atalhos: símbolo, dica (para o log) e ação. No flutuante, só o
/// de acoplar.
fn bar_buttons(floating: bool) -> Vec<(&'static str, Action)> {
    if floating {
        return vec![("keyboard-dock-symbolic", Action::Float)];
    }
    vec![
        ("edit-undo-symbolic", Action::Combo("ctrl+z")),
        ("edit-redo-symbolic", Action::Combo("ctrl+shift+z")),
        ("edit-cut-symbolic", Action::Combo("ctrl+x")),
        ("edit-copy-symbolic", Action::Combo("ctrl+c")),
        ("edit-paste-symbolic", Action::Combo("ctrl+v")),
        ("keyboard-float-symbolic", Action::Float),
    ]
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

/// Posições das teclas numa superfície de `width` pontos, com as teclas a partir de
/// `top`, em `keys_h` de altura.
fn layout(rows: &[Vec<Key>], width: f32, top: f32, keys_h: f32) -> Vec<Placed> {
    let pad = if width < 500.0 { 5.0 } else { PAD };
    let gap = if width < 500.0 { 5.0 } else { GAP };
    let inner = (width - 2.0 * pad).min(MAX_W);
    let left0 = (width - inner) / 2.0;
    let key_h = (keys_h - 2.0 * pad - 3.0 * gap) / 4.0;
    // A unidade sai da fileira de cima (10 teclas), igual para todas.
    let unit = (inner - 9.0 * gap) / 10.0;
    let mut out = Vec::new();
    for (r, row) in rows.iter().enumerate() {
        let key_w = |k: &Key| k.units * unit + (k.units - 1.0) * gap;
        let row_w = row.iter().map(key_w).sum::<f32>() + (row.len() as f32 - 1.0) * gap;
        // A fileira de baixo e a terceira ocupam a largura toda; as outras, centradas.
        let (mut x, stretch) = if row_w > inner || r >= 2 { (left0, inner / row_w) } else { (left0 + (inner - row_w) / 2.0, 1.0) };
        let y = top + pad + r as f32 * (key_h + gap);
        for (c, key) in row.iter().enumerate() {
            let w = key_w(key) * stretch;
            out.push(Placed { x, y, w, h: key_h, row: r, col: c });
            x += w + gap * stretch;
        }
    }
    out
}

/// Retângulos dos botões da barra de atalhos: os de editar à esquerda, o de flutuar
/// (ou acoplar) à direita.
fn bar_layout(count: usize, width: f32, top: f32, floating: bool) -> Vec<(f32, f32, f32, f32)> {
    let inner = (width - 2.0 * PAD).min(MAX_W);
    let left0 = (width - inner) / 2.0;
    let (bw, bh) = (44.0, BAR_H as f32 - 8.0);
    let y = top + 6.0;
    (0..count)
        .map(|i| {
            let last = i + 1 == count;
            let x = if last || floating { left0 + inner - bw } else { left0 + i as f32 * (bw + 4.0) };
            (x, y, bw, bh)
        })
        .collect()
}

/// Letra segurada com as variações abertas.
struct Variants {
    /// Tecla de origem (fileira, coluna) e as variações.
    origin: (usize, usize),
    items: Vec<String>,
    /// Variação sob o dedo/ponteiro.
    chosen: Option<usize>,
}

/// O espaço segurado vira trackpad: último ponto e o caminho que ainda não deu um passo.
#[derive(Debug, Clone, Copy)]
struct Trackpad {
    last: (f64, f64),
    rest: (f64, f64),
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
    /// Último espaço digitado (dois seguidos viram ". ").
    last_space: Option<Instant>,
    pressed: Option<(usize, usize)>,
    /// Onde o dedo encostou (puxar a letra para baixo, trackpad).
    press_at: (f64, f64),
    /// Botão da barra de atalhos apertado.
    bar_pressed: Option<usize>,
    variants: Option<Variants>,
    trackpad: Option<Trackpad>,
    timer: Option<RegistrationToken>,
    pointer: Option<wl_pointer::WlPointer>,
    touch: Option<wl_touch::WlTouch>,
    /// Dedos na superfície (o primeiro digita; dois juntando ou abrindo mudam o modo).
    fingers: HashMap<i32, (f64, f64)>,
    touch_id: Option<i32>,
    /// Último ponto do dedo (o `up` do toque não traz a posição).
    last_touch: Option<(f64, f64)>,
    /// Distância dos dois dedos quando o segundo encostou.
    pinch: Option<f64>,
    text: Text,
    glass: mide::control_client::Glass,
    abnt: bool,
    placed: Vec<Placed>,
    /// Flutuante (menor, arrastável) e o lugar dele (margens esquerda e de baixo).
    floating: bool,
    float_at: (i32, i32),
    /// Arrastando o flutuante pela alça: o ponto pego (na superfície).
    grab: Option<(f64, f64)>,
    /// Quando o teclado apareceu (sobe deslizando).
    opened_at: Instant,
    /// Símbolos já desenhados, por (nome, pixels, cor).
    symbols: HashMap<(String, u32, [u8; 4]), Pixmap>,
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
    let int = |key: &str, default: i64| match mide::settings_file::get("keyboard", key) {
        Some(mide::settings_file::Value::Int(n)) => n,
        _ => default,
    };
    let float_at = (int("osk_float_x", -1) as i32, int("osk_float_y", 24) as i32);
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
        last_space: None,
        pressed: None,
        press_at: (0.0, 0.0),
        bar_pressed: None,
        variants: None,
        trackpad: None,
        timer: None,
        pointer: None,
        touch: None,
        fingers: HashMap::new(),
        touch_id: None,
        last_touch: None,
        pinch: None,
        text: Text::new(),
        glass: Default::default(),
        abnt: abnt(),
        placed: Vec::new(),
        floating: mide::settings_file::flag("keyboard", "osk_floating", false),
        float_at,
        grab: None,
        opened_at: Instant::now(),
        symbols: HashMap::new(),
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
    // Subindo ao aparecer: um quadro a cada 16 ms enquanto desliza.
    event_loop
        .handle()
        .insert_source(Timer::from_duration(Duration::from_millis(16)), |_, _, kbd: &mut Kbd| {
            if kbd.layer.is_some() && kbd.sliding() {
                kbd.draw();
            }
            TimeoutAction::ToDuration(Duration::from_millis(16))
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

    /// Altura da superfície (pontos): faixa das variações, barra, teclas (e alça).
    fn surface_height(&self) -> u32 {
        if self.floating { POPUP_H + BAR_H + FLOAT_KEYS_H + HANDLE_H } else { POPUP_H + BAR_H + PANEL_H }
    }

    fn open(&mut self) {
        tracing::info!("teclado na tela: aberto{}", if self.floating { " (flutuante)" } else { "" });
        self.abnt = abnt();
        let surface = self.compositor.create_surface(&self.qh);
        self.scale = Some(SurfaceScale::new(&self.globals, &self.qh, &surface));
        let layer = self.layer_shell.create_layer_surface(&self.qh, surface, Layer::Top, Some("mide-keyboard"), None);
        if self.floating {
            // Flutuante: não empurra as janelas; fica onde foi deixado.
            layer.set_anchor(Anchor::BOTTOM | Anchor::LEFT);
            layer.set_exclusive_zone(0);
            layer.set_size(FLOAT_W, self.surface_height());
            self.apply_float_margin(&layer);
        } else {
            layer.set_anchor(Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
            // Reserva a altura do painel: as janelas maximizadas sobem, o campo fica à vista.
            layer.set_exclusive_zone((PANEL_H + BAR_H) as i32);
            layer.set_size(0, self.surface_height());
        }
        // Nunca o teclado de verdade: o foco fica no app (o campo continua ativo).
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.commit();
        self.layer = Some(layer);
        self.configured = false;
        self.opened_at = Instant::now();
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
        self.trackpad = None;
        self.grab = None;
        self.bar_pressed = None;
    }

    /// Flutuante ↔ acoplado (refaz a camada no outro modo).
    fn set_floating(&mut self, floating: bool) {
        if self.floating == floating {
            return;
        }
        self.floating = floating;
        tracing::info!("teclado na tela: {}", if floating { "flutuante" } else { "acoplado" });
        let _ = mide::settings_file::set("keyboard", "osk_floating", Some(mide::settings_file::Value::Bool(floating)));
        if self.layer.is_some() {
            self.close();
            self.open();
        }
    }

    /// Margens do flutuante (esquerda e baixo); sem lugar guardado, centrado.
    fn apply_float_margin(&self, layer: &LayerSurface) {
        let screen_w = self.screen_width();
        let x = if self.float_at.0 < 0 { (screen_w - FLOAT_W as i32) / 2 } else { self.float_at.0 }.clamp(0, (screen_w - FLOAT_W as i32).max(0));
        layer.set_margin(0, 0, self.float_at.1.max(0), x.max(0));
    }

    /// Largura da tela (pontos) onde o teclado aparece.
    fn screen_width(&self) -> i32 {
        self.output_state
            .outputs()
            .filter_map(|o| self.output_state.info(&o))
            .find_map(|i| i.logical_size.map(|s| s.0))
            .unwrap_or(1366)
    }

    fn sliding(&self) -> bool {
        self.opened_at.elapsed() < mide::motion::KEYBOARD_SLIDE
    }

    /// Quanto o teclado ainda está abaixo do lugar dele (pontos), subindo ao aparecer.
    fn slide_offset(&self) -> f32 {
        if !mide::settings_file::effect("transitions") {
            return 0.0;
        }
        let t = mide::motion::progress(self.opened_at, mide::motion::KEYBOARD_SLIDE);
        (1.0 - mide::motion::ease_out(t)) * (self.surface_height() - POPUP_H) as f32
    }

    fn current_rows(&self) -> Vec<Vec<Key>> {
        rows(self.page, self.shift || self.caps, self.abnt)
    }

    fn keys_top(&self) -> f32 {
        (POPUP_H + BAR_H) as f32
    }

    fn keys_height(&self) -> f32 {
        if self.floating { FLOAT_KEYS_H as f32 } else { PANEL_H as f32 }
    }

    fn bar_rects(&self) -> Vec<(f32, f32, f32, f32)> {
        bar_layout(bar_buttons(self.floating).len(), self.size.0 as f32, POPUP_H as f32, self.floating)
    }

    fn bar_at(&self, x: f64, y: f64) -> Option<usize> {
        let (x, y) = (x as f32, y as f32);
        self.bar_rects().iter().position(|(bx, by, bw, bh)| x >= bx - 4.0 && x < bx + bw + 4.0 && y >= by - 4.0 && y < by + bh + 4.0)
    }

    /// Alça do flutuante (a faixa de baixo).
    fn on_handle(&self, y: f64) -> bool {
        self.floating && y >= (self.surface_height() - HANDLE_H) as f64 - 2.0
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
        let x0 = (origin.x + origin.w / 2.0 - total / 2.0).clamp(PAD, (self.size.0 as f32 - PAD - total).max(PAD));
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
        self.press_at = (x, y);
        if self.on_handle(y) {
            self.grab = Some((x, y));
            return;
        }
        if let Some(i) = self.bar_at(x, y) {
            self.bar_pressed = Some(i);
            self.click();
            self.draw();
            return;
        }
        let Some(at) = self.key_at(x, y) else {
            return;
        };
        self.pressed = Some(at);
        let Some(key) = self.key(at) else {
            return;
        };
        self.click();
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
            // Espaço segurado: trackpad.
            Action::Space => {
                let token = self.handle.insert_source(Timer::from_duration(LONG_PRESS), move |_, _, kbd: &mut Kbd| {
                    kbd.timer = None;
                    if kbd.pressed == Some(at) {
                        kbd.start_trackpad(kbd.press_at);
                    }
                    TimeoutAction::Drop
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

    fn start_trackpad(&mut self, at: (f64, f64)) {
        tracing::info!("teclado na tela: trackpad (espaço segurado)");
        self.cancel_timer();
        self.trackpad = Some(Trackpad { last: at, rest: (0.0, 0.0) });
        self.draw();
    }

    fn motion(&mut self, x: f64, y: f64) {
        // Flutuante pela alça: a camada anda (a posição na superfície fica a mesma).
        if let Some((gx, gy)) = self.grab {
            let (dx, dy) = ((x - gx).round() as i32, (y - gy).round() as i32);
            if dx != 0 || dy != 0 {
                let screen_w = self.screen_width();
                let cur_x = if self.float_at.0 < 0 { (screen_w - FLOAT_W as i32) / 2 } else { self.float_at.0 };
                self.float_at = ((cur_x + dx).clamp(0, (screen_w - FLOAT_W as i32).max(0)), (self.float_at.1 - dy).max(0));
                if let Some(layer) = self.layer.clone() {
                    self.apply_float_margin(&layer);
                    layer.commit();
                }
            }
            return;
        }
        if let Some(mut pad) = self.trackpad {
            pad.rest.0 += x - pad.last.0;
            pad.rest.1 += y - pad.last.1;
            pad.last = (x, y);
            while pad.rest.0 >= TRACK_X {
                pad.rest.0 -= TRACK_X;
                self.send_key("right");
            }
            while pad.rest.0 <= -TRACK_X {
                pad.rest.0 += TRACK_X;
                self.send_key("left");
            }
            while pad.rest.1 >= TRACK_Y {
                pad.rest.1 -= TRACK_Y;
                self.send_key("down");
            }
            while pad.rest.1 <= -TRACK_Y {
                pad.rest.1 += TRACK_Y;
                self.send_key("up");
            }
            self.trackpad = Some(pad);
            return;
        }
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
        if let Some(pressed) = self.pressed {
            let (dx, dy) = (x - self.press_at.0, y - self.press_at.1);
            // Arrastar o dedo pelo espaço também vira trackpad (sem esperar).
            if self.key(pressed).is_some_and(|k| k.action == Action::Space) && dx.abs() >= 16.0 {
                self.start_trackpad((x, y));
                return;
            }
            // Letra de cima puxada para baixo: o número dela (fica apertada).
            if flick_digit(self.page, pressed.0, pressed.1).is_some() && dy > 0.0 && dx.abs() < 26.0 {
                if dy >= FLICK {
                    self.cancel_timer();
                }
                return;
            }
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
        if self.grab.take().is_some() {
            // Solto perto da borda de baixo: acopla de novo.
            if self.float_at.1 < 12 {
                self.float_at.1 = 24;
                self.set_floating(false);
            } else {
                let _ = mide::settings_file::set("keyboard", "osk_float_x", Some(mide::settings_file::Value::Int(self.float_at.0 as i64)));
                let _ = mide::settings_file::set("keyboard", "osk_float_y", Some(mide::settings_file::Value::Int(self.float_at.1 as i64)));
            }
            return;
        }
        if let Some(i) = self.bar_pressed.take() {
            if self.bar_at(x, y) == Some(i)
                && let Some((name, action)) = bar_buttons(self.floating).into_iter().nth(i)
            {
                tracing::info!("teclado na tela: barra de atalhos ({name})");
                self.activate(action);
            }
            if self.layer.is_some() {
                self.draw();
            }
            return;
        }
        if self.trackpad.take().is_some() {
            self.pressed = None;
            self.draw();
            return;
        }
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
        let dy = y - self.press_at.1;
        if let Some(digit) = flick_digit(self.page, at.0, at.1).filter(|_| dy >= FLICK && (x - self.press_at.0).abs() < 26.0) {
            tracing::info!("teclado na tela: letra puxada para baixo ({digit})");
            self.type_text(digit);
        } else if self.key_at(x, y) == Some(at)
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
            Action::Space => {
                // Dois espaços: ". " (como no iPad), se o anterior foi um espaço há pouco.
                let period = mide::settings_file::flag("keyboard", "osk_period", true);
                if period && self.last_space.is_some_and(|t| t.elapsed() < DOUBLE_SPACE) {
                    self.last_space = None;
                    self.backspace();
                    self.type_text(". ");
                    tracing::info!("teclado na tela: dois espaços viraram ponto");
                } else {
                    self.type_text(" ");
                    self.last_space = Some(Instant::now());
                }
                return;
            }
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
                    tracing::info!("teclado na tela: maiúsculas fixas");
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
            Action::Combo(combo) => {
                let _ = self.conn.flush();
                let _ = mide::control_client::ask_compositor(&format!("keys {combo}"));
            }
            Action::Float => {
                let floating = !self.floating;
                self.set_floating(floating);
            }
        }
        self.last_space = None;
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

    /// Som da tecla (`[keyboard] osk_sound`).
    fn click(&self) {
        if mide::settings_file::flag("keyboard", "osk_sound", true) {
            mide::sounds::play("keyboard-click");
        }
    }

    /// Dois dedos sobre o teclado: juntando, ele fica flutuante; abrindo, acopla.
    fn pinch_moved(&mut self) {
        let Some(start) = self.pinch else {
            return;
        };
        let mut points = self.fingers.values();
        let (Some(a), Some(b)) = (points.next(), points.next()) else {
            return;
        };
        let d = ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt();
        if d < start * 0.65 && !self.floating {
            self.pinch = None;
            self.set_floating(true);
        } else if d > start * 1.5 && self.floating {
            self.pinch = None;
            self.set_floating(false);
        }
    }

    // --- desenho ---

    /// Símbolo do mide-ui tingido de `ink`, em `size` pixels.
    fn symbol(&mut self, name: &str, size: u32, ink: [u8; 4]) -> Option<Pixmap> {
        let key = (name.to_string(), size, ink);
        if let Some(p) = self.symbols.get(&key) {
            return Some(p.clone());
        }
        let mut icon = mide::icons::symbol(name, size)?;
        for px in icon.pixels_mut() {
            let a = px.alpha() as u32 * ink[3] as u32 / 255;
            let c = |v: u8| (v as u32 * a / 255) as u8;
            if let Some(color) = tiny_skia::PremultipliedColorU8::from_rgba(c(ink[0]), c(ink[1]), c(ink[2]), a as u8) {
                *px = color;
            }
        }
        self.symbols.insert(key, icon.clone());
        Some(icon)
    }

    /// Desenha o símbolo centrado no retângulo (pontos).
    fn draw_symbol(&mut self, pixmap: &mut Pixmap, name: &str, rect: (f32, f32, f32, f32), points: f32, ink: [u8; 4]) {
        let k = mide::ui::draw::scale();
        let px = (points * k).round() as u32;
        if let Some(icon) = self.symbol(name, px, ink) {
            let x = ((rect.0 + (rect.2 - points) / 2.0) * k).round() as i32;
            let y = ((rect.1 + (rect.3 - points) / 2.0) * k).round() as i32;
            pixmap.draw_pixmap(x, y, icon.as_ref(), &tiny_skia::PixmapPaint::default(), tiny_skia::Transform::identity(), None);
        }
    }

    fn draw(&mut self) {
        let Some(layer) = self.layer.clone() else {
            return;
        };
        if !self.configured || self.size.0 == 0 {
            return;
        }
        let (pw, ph) = self.size;
        let rows = self.current_rows();
        let slide = self.slide_offset();
        let top = self.keys_top();
        let placed: Vec<Placed> = layout(&rows, pw as f32, top, self.keys_height()).into_iter().map(|p| Placed { y: p.y + slide, ..p }).collect();
        if slide == 0.0 && (placed.len() != self.placed.len() || self.placed.first().map(|p| (p.x, p.w, p.y)) != placed.first().map(|p| (p.x, p.w, p.y))) {
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
        // Painel: vidro (o compositor desfoca por trás); acoplado, os cantos de cima
        // redondos; flutuante, todos, com sombra.
        let panel_top = POPUP_H as f32 + slide;
        let [r, g, b, _] = theme.menu;
        let fill = rgba(r, g, b, if glass { 196 } else { 246 });
        let panel = if self.floating {
            let (x, y, pw2, ph2) = (2.0, panel_top, pw as f32 - 4.0, ph as f32 - POPUP_H as f32 - 4.0);
            mide::ui::draw::soft_shadow(&mut pixmap, x, y, pw2, ph2, 20.0, 10.0, 3.0, 70);
            fill_rounded(&mut pixmap, x, y, pw2, ph2, 20.0, fill);
            (x, y, pw2, ph2, 20.0)
        } else {
            fill_rounded(&mut pixmap, 0.0, panel_top, pw as f32, ph as f32 - POPUP_H as f32 + 20.0, 18.0, fill);
            (0.0, panel_top, pw as f32, ph as f32 - POPUP_H as f32 + 20.0, 18.0)
        };
        let shown_glass = [panel];
        self.glass.set("mide-keyboard", if glass && slide == 0.0 { &shown_glass } else { &[] });
        let key_fill = if theme.dark { rgba(255, 255, 255, 38) } else { rgba(255, 255, 255, 235) };
        let special_fill = if theme.dark { rgba(255, 255, 255, 18) } else { rgba(0, 0, 0, 26) };
        let [ar, ag, ab, _] = theme.accent;
        let trackpad = self.trackpad.is_some();

        // Barra de atalhos.
        let buttons = bar_buttons(self.floating);
        let rects: Vec<(f32, f32, f32, f32)> = self.bar_rects().into_iter().map(|(x, y, bw, bh)| (x, y + slide, bw, bh)).collect();
        for (i, ((name, _), rect)) in buttons.iter().zip(&rects).enumerate() {
            if self.bar_pressed == Some(i) {
                fill_rounded(&mut pixmap, rect.0, rect.1, rect.2, rect.3, 9.0, special_fill);
            }
            let ink = if trackpad { [theme.text[0], theme.text[1], theme.text[2], 60] } else { theme.text };
            self.draw_symbol(&mut pixmap, name, *rect, 19.0, ink);
        }

        for (p, key) in placed.iter().zip(rows.iter().flatten()) {
            let pressed = self.pressed == Some((p.row, p.col)) && self.variants.is_none() && !trackpad;
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
            // Trackpad: as teclas ficam lisas (sem as letras), como no iPad.
            if trackpad {
                continue;
            }
            let ink = if pressed || lit { theme.on_accent } else { theme.text };
            if let Some(name) = key_symbol(&key.action, self.shift, self.caps) {
                self.draw_symbol(&mut pixmap, name, (p.x, p.y, p.w, p.h), 20.0, ink);
                continue;
            }
            // Número da letra de cima (puxar para baixo), pequeno e cinza.
            if let Some(digit) = flick_digit(self.page, p.row, p.col) {
                let dw = self.text.measure(digit, 11.0, FontWeight::Regular);
                self.text.draw(&mut pixmap, digit, p.x + (p.w - dw) / 2.0, p.y + 3.0, 11.0, FontWeight::Regular, theme.text_secondary);
            }
            // Letras grandes; símbolos médios; "123"/"ABC" menores.
            let size = match (&key.action, key.label.chars().count()) {
                (Action::Text(_), 1) => 21.0,
                (_, 1) => 19.0,
                _ => 15.0,
            };
            let tw = self.text.measure(&key.label, size, FontWeight::Regular);
            let ty = p.y + (p.h - size * 1.25) / 2.0 + if flick_digit(self.page, p.row, p.col).is_some() { 4.0 } else { 0.0 };
            self.text.draw(&mut pixmap, &key.label, p.x + (p.w - tw) / 2.0, ty, size, FontWeight::Regular, ink);
        }
        // Alça do flutuante.
        if self.floating {
            let hy = ph as f32 - HANDLE_H as f32 / 2.0 - 4.0 + slide;
            fill_rounded(&mut pixmap, pw as f32 / 2.0 - 22.0, hy - 2.5, 44.0, 5.0, 2.5, rgba(theme.text[0], theme.text[1], theme.text[2], 90));
        }
        // Variações da letra segurada, numa pílula acima dela.
        let vrects = self.variant_rects();
        if let Some(v) = &self.variants {
            if let (Some(first), Some(last)) = (vrects.first(), vrects.last()) {
                let (x, y, total_w, rh) = (first.0 - 4.0, first.1 - 4.0, last.0 + last.2 - first.0 + 8.0, first.3 + 8.0);
                fill_rounded(&mut pixmap, x, y + 2.0, total_w, rh, 14.0, rgba(0, 0, 0, 50));
                fill_rounded(&mut pixmap, x, y, total_w, rh, 14.0, {
                    let [r, g, b, _] = theme.menu;
                    rgba(r, g, b, 255)
                });
            }
            for (i, (rx, ry, rw, rh)) in vrects.iter().enumerate() {
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
            region.add(0, POPUP_H as i32, pw as i32, (ph - POPUP_H) as i32);
            if let (Some(first), Some(last)) = (vrects.first(), vrects.last()) {
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
        self.symbols.clear();
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
    // Tela girada: o flutuante continua dentro dela.
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {
        if self.floating
            && let Some(layer) = self.layer.clone()
        {
            self.apply_float_margin(&layer);
            layer.commit();
        }
    }
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
                PointerEventKind::Leave { .. } if self.grab.is_none() && (self.pressed.take().is_some() || self.variants.take().is_some() || self.trackpad.take().is_some()) => {
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
        if self.layer.as_ref().map(|l| l.wl_surface()) != Some(&surface) {
            return;
        }
        self.fingers.insert(id, position);
        if self.fingers.len() == 2 {
            // Segundo dedo: começa a pinça (o primeiro não digita mais).
            let mut points = self.fingers.values();
            if let (Some(a), Some(b)) = (points.next(), points.next()) {
                self.pinch = Some(((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt().max(1.0));
            }
            self.cancel_timer();
            self.pressed = None;
            self.variants = None;
            self.trackpad = None;
            self.bar_pressed = None;
            self.draw();
            return;
        }
        if self.touch_id.is_some() {
            return;
        }
        self.touch_id = Some(id);
        self.last_touch = Some(position);
        self.press(position.0, position.1);
    }

    fn up(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_touch::WlTouch, _: u32, _: u32, id: i32) {
        self.fingers.remove(&id);
        if self.fingers.len() < 2 {
            self.pinch = None;
        }
        if self.touch_id == Some(id) {
            self.touch_id = None;
            let last = self.last_touch.take().unwrap_or((-1.0, -1.0));
            self.release(last.0, last.1);
        }
    }

    fn motion(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_touch::WlTouch, _: u32, id: i32, position: (f64, f64)) {
        if let Some(p) = self.fingers.get_mut(&id) {
            *p = position;
        }
        if self.pinch.is_some() {
            self.pinch_moved();
            return;
        }
        if self.touch_id == Some(id) {
            self.last_touch = Some(position);
            Kbd::motion(self, position.0, position.1);
        }
    }

    fn shape(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_touch::WlTouch, _: i32, _: f64, _: f64) {}
    fn orientation(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_touch::WlTouch, _: i32, _: f64) {}

    fn cancel(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_touch::WlTouch) {
        self.touch_id = None;
        self.fingers.clear();
        self.pinch = None;
        self.pressed = None;
        self.variants = None;
        self.trackpad = None;
        self.grab = None;
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
        // Nenhuma tecla sai da superfície nem se sobrepõe na fileira (acoplado e
        // flutuante).
        for page in [Page::Letters, Page::Numbers, Page::Symbols] {
            let rows = rows(page, false, true);
            for (width, keys_h) in [(400.0, FLOAT_KEYS_H as f32), (600.0, PANEL_H as f32), (1366.0, PANEL_H as f32), (2560.0, PANEL_H as f32)] {
                let placed = layout(&rows, width, 104.0, keys_h);
                assert_eq!(placed.len(), rows.iter().map(Vec::len).sum::<usize>());
                for pair in placed.windows(2).filter(|p| p[0].row == p[1].row) {
                    assert!(pair[0].x + pair[0].w <= pair[1].x + 0.01, "{page:?} {width}");
                }
                assert!(placed.iter().all(|p| p.x >= 0.0 && p.x + p.w <= width + 0.01));
                assert!(placed.iter().all(|p| p.y >= 104.0 && p.y + p.h <= 104.0 + keys_h + 0.01));
            }
        }
        assert_eq!(variants("a").len(), 5);
        assert!(variants("q").is_empty());
    }

    #[test]
    fn numeros_puxando_a_fileira_de_cima() {
        assert_eq!(flick_digit(Page::Letters, 0, 0), Some("1"));
        assert_eq!(flick_digit(Page::Letters, 0, 9), Some("0"));
        assert_eq!(flick_digit(Page::Letters, 1, 0), None);
        assert_eq!(flick_digit(Page::Numbers, 0, 0), None);
    }

    #[test]
    fn barra_de_atalhos() {
        let docked = bar_layout(bar_buttons(false).len(), 1366.0, 64.0, false);
        assert_eq!(docked.len(), 6);
        // O de flutuar fica à direita, os outros à esquerda, sem se sobrepor.
        assert!(docked[5].0 > 1000.0 && docked[0].0 < 200.0);
        assert!(docked[0].0 + docked[0].2 <= docked[1].0);
        let floating = bar_layout(bar_buttons(true).len(), 400.0, 64.0, true);
        assert_eq!(floating.len(), 1);
        assert!(floating[0].0 + floating[0].2 <= 400.0);
    }
}
