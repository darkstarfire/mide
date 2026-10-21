//! Notificações do mide: servidor `org.freedesktop.Notifications` com banners no
//! estilo do macOS.
//!
//! Os apps (e o `notify-send`) mandam notificações pelo D-Bus. Cada uma vira um
//! banner no canto superior direito, abaixo da barra: claro, translúcido e
//! arredondado, com o ícone do app, o título em negrito e até duas linhas de texto.
//! Os banners se empilham (até 4), somem sozinhos depois de 5 s (os críticos ficam)
//! e, com o mouse em cima, mostram o × para fechar. Clique no banner aciona a ação
//! padrão do app; outras ações aparecem como botões.
//!
//! Se outro programa já for o servidor de notificações (por exemplo, o GNOME do
//! desktop onde o mide roda em janela), este sai sem fazer nada.

use std::{
    collections::HashMap,
    error::Error,
    sync::{
        atomic::{AtomicU32, Ordering},
        Mutex,
    },
    time::{Duration, Instant},
};

use mide::{
    icons::{load_icon, load_icon_file, load_themed_icon},
    notify::{self, reason},
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
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
        WaylandSurface,
    },
    shm::{slot::SlotPool, Shm, ShmHandler},
};
use tiny_skia::{Paint, PathBuilder, Pixmap, PixmapPaint, Stroke, Transform};
use zbus::zvariant::OwnedValue;

const BUS_NAME: &str = "org.freedesktop.Notifications";
const OBJECT_PATH: &str = "/org/freedesktop/Notifications";

const BANNER_W: f32 = 344.0;
const BANNER_H: f32 = 74.0;
const BUTTONS_H: f32 = 34.0;
const GAP: f32 = 8.0;
/// Borda transparente em volta dos banners, para a sombra.
const MARGIN: f32 = 10.0;
const RADIUS: f32 = 16.0;
const ICON: u32 = 38;
const MAX_VISIBLE: usize = 4;
const BTN_LEFT: u32 = 0x110;

/// Uma notificação recebida, pronta para mostrar.
struct Note {
    id: u32,
    app_name: String,
    icon: Option<String>,
    summary: String,
    body: String,
    has_default: bool,
    buttons: Vec<(String, String)>,
    timeout: Option<Duration>,
    shown_at: Instant,
}

/// Mensagens do servidor D-Bus (outra thread) para a interface.
enum Msg {
    Notify(Note),
    Close(u32),
}

/// Objeto D-Bus `/org/freedesktop/Notifications`.
struct Server {
    sender: Mutex<channel::Sender<Msg>>,
    next_id: AtomicU32,
}

fn hint_str(hints: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    hints.get(key).and_then(|v| String::try_from(v.try_clone().ok()?).ok()).filter(|s| !s.is_empty())
}

#[zbus::interface(name = "org.freedesktop.Notifications")]
impl Server {
    fn get_capabilities(&self) -> Vec<String> {
        ["actions", "body", "body-markup", "icon-static", "persistence"].iter().map(|s| s.to_string()).collect()
    }

    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app_name: String,
        replaces_id: u32,
        app_icon: String,
        summary: String,
        body: String,
        actions: Vec<String>,
        hints: HashMap<String, OwnedValue>,
        expire_timeout: i32,
    ) -> u32 {
        let id = if replaces_id != 0 { replaces_id } else { self.next_id.fetch_add(1, Ordering::Relaxed) };
        let urgency = hints.get("urgency").and_then(|v| u8::try_from(v).ok()).unwrap_or(1);
        // Toda notificação vai para o histórico (central de notificações, no relógio),
        // inclusive as silenciadas pelo Não perturbe.
        let time = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        mide::notify::add_to_history(&mide::notify::Entry {
            time,
            app: app_name.clone(),
            summary: notify::plain_text(&summary),
            body: notify::plain_text(&body),
        });
        // Não perturbe (central de controle): só as urgentes aparecem.
        let dnd = matches!(mide::settings_file::get("notifications", "dnd"), Some(mide::settings_file::Value::Bool(true)));
        if dnd && urgency < 2 {
            tracing::info!("notificação {id} silenciada (não perturbe): {summary}");
            return id;
        }
        let icon = Some(app_icon)
            .filter(|s| !s.is_empty())
            .or_else(|| hint_str(&hints, "image-path"))
            .or_else(|| hint_str(&hints, "desktop-entry"));
        // Som de notificação (fora do Não perturbe), a não ser que o app peça silêncio
        // ou traga o próprio som.
        let quiet = hints.get("suppress-sound").and_then(|v| bool::try_from(v).ok()).unwrap_or(false) || hints.contains_key("sound-file") || hints.contains_key("sound-name");
        if !quiet {
            mide::sounds::play("message-new-instant");
        }
        let (has_default, buttons) = notify::actions(&actions);
        let note = Note {
            id,
            app_name,
            icon,
            summary: notify::plain_text(&summary),
            body: notify::plain_text(&body),
            has_default,
            buttons,
            timeout: notify::timeout(expire_timeout, urgency),
            shown_at: Instant::now(),
        };
        if let Ok(sender) = self.sender.lock() {
            let _ = sender.send(Msg::Notify(note));
        }
        id
    }

    fn close_notification(&self, id: u32) {
        if let Ok(sender) = self.sender.lock() {
            let _ = sender.send(Msg::Close(id));
        }
    }

    fn get_server_information(&self) -> (String, String, String, String) {
        ("mide".into(), "mide".into(), env!("CARGO_PKG_VERSION").into(), "1.2".into())
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "mide_notifications=info".into()),
        )
        .init();

    let (sender, receiver) = channel::channel::<Msg>();
    let server = Server {
        sender: Mutex::new(sender),
        next_id: AtomicU32::new(1),
    };
    let bus = match zbus::blocking::connection::Builder::session()
        .and_then(|b| b.name(BUS_NAME))
        .and_then(|b| b.serve_at(OBJECT_PATH, server))
        .and_then(|b| b.build())
    {
        Ok(bus) => bus,
        Err(err) => {
            // Outro servidor (como o do GNOME) já atende: nada a fazer.
            tracing::info!("notificações: servidor não iniciado ({err})");
            return Ok(());
        }
    };
    tracing::info!("servidor de notificações pronto");

    let conn = Connection::connect_to_env()?;
    let (globals, event_queue) = registry_queue_init(&conn)?;
    let qh = event_queue.handle();
    let mut ui = Banners {
        glass: Default::default(),
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),
        compositor: CompositorState::bind(&globals, &qh)?,
        layer_shell: LayerShell::bind(&globals, &qh)?,
        pool: None,
        shm: Shm::bind(&globals, &qh)?,
        qh: qh.clone(),
        layer: None,
        configured: false,
        size: (0, 0),
        pointer: None,
        notes: Vec::new(),
        hover: None,
        press: None,
        text: Text::new(),
        icons: HashMap::new(),
        bus,
        dirty: false,
    };

    let mut event_loop: EventLoop<Banners> = EventLoop::try_new()?;
    WaylandSource::new(conn.clone(), event_queue)
        .insert(event_loop.handle())
        .map_err(|e| e.to_string())?;
    event_loop
        .handle()
        .insert_source(receiver, |event, _, ui: &mut Banners| {
            if let channel::Event::Msg(msg) = event {
                match msg {
                    Msg::Notify(note) => ui.add(note),
                    Msg::Close(id) => ui.remove(id, reason::CLOSED_BY_CALL),
                }
            }
        })
        .map_err(|e| e.to_string())?;
    // Confere os banners que expiraram (com o mouse em cima, o banner espera).
    event_loop
        .handle()
        .insert_source(Timer::from_duration(Duration::from_millis(200)), |_, _, ui| {
            ui.expire();
            TimeoutAction::ToDuration(Duration::from_millis(200))
        })
        .map_err(|e| e.to_string())?;

    loop {
        event_loop.dispatch(None, &mut ui)?;
        if ui.dirty {
            ui.sync_layer();
        }
    }
}

/// Parte de um banner sob o ponteiro.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Spot {
    Body(usize),
    Close(usize),
    Button(usize, usize),
}

struct Banners {
    /// Vidro atrás dos banners (desenhado pelo compositor).
    glass: mide::control_client::Glass,
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    compositor: CompositorState,
    layer_shell: LayerShell,
    shm: Shm,
    pool: Option<SlotPool>,
    qh: QueueHandle<Banners>,
    /// Camada dos banners; só existe enquanto houver notificação na tela.
    layer: Option<LayerSurface>,
    configured: bool,
    size: (u32, u32),
    pointer: Option<wl_pointer::WlPointer>,
    /// Da mais nova (em cima) para a mais antiga.
    notes: Vec<Note>,
    /// Banner sob o ponteiro (índice em `notes`).
    hover: Option<usize>,
    press: Option<Spot>,
    text: Text,
    icons: HashMap<String, Option<Pixmap>>,
    bus: zbus::blocking::Connection,
    dirty: bool,
}

impl Banners {
    fn add(&mut self, note: Note) {
        tracing::info!("notificação {}: {}", note.id, note.summary);
        if let Some(old) = self.notes.iter_mut().find(|n| n.id == note.id) {
            *old = note;
        } else {
            self.notes.insert(0, note);
        }
        // Mais de 4 na tela: as mais antigas saem.
        while self.notes.len() > MAX_VISIBLE {
            if let Some(old) = self.notes.pop() {
                self.signal_closed(old.id, reason::EXPIRED);
            }
        }
        self.dirty = true;
    }

    fn remove(&mut self, id: u32, why: u32) {
        let before = self.notes.len();
        self.notes.retain(|n| n.id != id);
        if self.notes.len() != before {
            let what = match why {
                reason::EXPIRED => "expirou",
                reason::DISMISSED => "fechada",
                _ => "fechada pelo app",
            };
            tracing::info!("notificação {id} {what}");
            self.signal_closed(id, why);
            self.hover = None;
            self.dirty = true;
        }
    }

    fn expire(&mut self) {
        let now = Instant::now();
        let hovered = self.hover.and_then(|i| self.notes.get(i)).map(|n| n.id);
        let expired: Vec<u32> = self
            .notes
            .iter()
            .filter(|n| Some(n.id) != hovered)
            .filter(|n| n.timeout.is_some_and(|t| now.duration_since(n.shown_at) >= t))
            .map(|n| n.id)
            .collect();
        for id in expired {
            self.remove(id, reason::EXPIRED);
        }
    }

    fn signal_closed(&self, id: u32, why: u32) {
        let _ = self.bus.emit_signal(None::<&str>, OBJECT_PATH, BUS_NAME, "NotificationClosed", &(id, why));
    }

    fn invoke(&mut self, index: usize, key: &str) {
        let Some(id) = self.notes.get(index).map(|n| n.id) else {
            return;
        };
        tracing::info!("ação {key} da notificação {id}");
        let _ = self.bus.emit_signal(None::<&str>, OBJECT_PATH, BUS_NAME, "ActionInvoked", &(id, key));
        self.remove(id, reason::DISMISSED);
    }

    fn banner_height(note: &Note) -> f32 {
        BANNER_H + if note.buttons.is_empty() { 0.0 } else { BUTTONS_H }
    }

    /// Topo de cada banner dentro da camada.
    fn banner_tops(&self) -> Vec<f32> {
        let mut y = MARGIN;
        self.notes
            .iter()
            .map(|n| {
                let top = y;
                y += Self::banner_height(n) + GAP;
                top
            })
            .collect()
    }

    fn surface_size(&self) -> (u32, u32) {
        let h: f32 = self.notes.iter().map(|n| Self::banner_height(n) + GAP).sum::<f32>() - GAP + 2.0 * MARGIN;
        ((BANNER_W + 2.0 * MARGIN) as u32, h.max(1.0) as u32)
    }

    /// Cria, ajusta ou remove a camada conforme as notificações e redesenha.
    fn sync_layer(&mut self) {
        self.dirty = false;
        if self.notes.is_empty() {
            if self.layer.take().is_some() {
                self.configured = false;
                self.pool = None;
            }
            return;
        }
        let size = self.surface_size();
        match &self.layer {
            None => {
                let surface = self.compositor.create_surface(&self.qh);
                let layer = self.layer_shell.create_layer_surface(&self.qh, surface, Layer::Top, Some("mide-notifications"), None);
                layer.set_anchor(Anchor::TOP | Anchor::RIGHT);
                layer.set_margin(0, 0, 0, 0);
                layer.set_exclusive_zone(0);
                layer.set_keyboard_interactivity(KeyboardInteractivity::None);
                layer.set_size(size.0, size.1);
                layer.commit();
                self.layer = Some(layer);
                self.size = size;
            }
            Some(layer) if self.size != size => {
                layer.set_size(size.0, size.1);
                layer.commit();
                self.size = size;
                // Redesenha quando o compositor confirmar o tamanho novo.
            }
            Some(_) => self.draw(),
        }
    }

    fn icon_for(&mut self, note_icon: Option<String>, app_name: &str) -> Option<Pixmap> {
        let key = note_icon.clone().unwrap_or_else(|| app_name.to_lowercase());
        if let Some(icon) = self.icons.get(&key) {
            return icon.clone();
        }
        let icon = match note_icon.as_deref() {
            Some(path) if path.starts_with('/') || path.starts_with("file://") => load_icon_file(path, ICON),
            Some(name) => load_icon(name, ICON).or_else(|| load_themed_icon(&[name], ICON)),
            None => load_icon(&app_name.to_lowercase(), ICON),
        }
        .or_else(|| load_themed_icon(&["dialog-information", "preferences-system-notifications"], ICON));
        self.icons.insert(key, icon.clone());
        icon
    }

    fn draw(&mut self) {
        let Some(layer) = self.layer.as_ref() else {
            return;
        };
        if !self.configured {
            return;
        }
        let (w, h) = self.size;
        let Some(mut pixmap) = Pixmap::new(w, h) else {
            return;
        };
        let tops = self.banner_tops();
        let layer = layer.clone();
        let glass = mide::control_client::panel_glass();
        let rects: Vec<_> = tops.iter().zip(&self.notes).map(|(top, n)| (MARGIN, *top, BANNER_W, Self::banner_height(n), RADIUS)).collect();
        self.glass.set("mide-notifications", if glass { &rects } else { &[] });
        for (i, top) in tops.into_iter().enumerate() {
            let (icon_name, app_name) = (self.notes[i].icon.clone(), self.notes[i].app_name.clone());
            let icon = self.icon_for(icon_name, &app_name);
            self.draw_banner(&mut pixmap, i, MARGIN, top, icon);
        }

        if self.pool.is_none() {
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
        if buffer.attach_to(surface).is_ok() {
            layer.commit();
        }
    }

    fn draw_banner(&mut self, pixmap: &mut Pixmap, index: usize, x: f32, y: f32, icon: Option<Pixmap>) {
        // Claro ou escuro, como as Configurações → Aparência.
        let theme = mide::kit::Theme::system();
        let ink = |a: u8| if theme.dark { rgba(255, 255, 255, a) } else { rgba(0, 0, 0, a) };
        let note = &self.notes[index];
        let bh = Self::banner_height(note);
        let hovered = self.hover == Some(index);
        let (app, summary, body, buttons) = (note.app_name.clone(), note.summary.clone(), note.body.clone(), note.buttons.clone());

        // Sombra suave, vidro claro e borda fina, como os banners do macOS.
        for (grow, alpha) in [(8.0, 10u8), (4.0, 16), (1.5, 26)] {
            fill_rounded(pixmap, x - grow, y - grow + 3.0, BANNER_W + 2.0 * grow, bh + 2.0 * grow, RADIUS + grow, rgba(0, 0, 0, alpha));
        }
        // Com vidro, o banner é translúcido e a sombra não fica por baixo dele.
        let glass = mide::control_client::panel_glass();
        if glass {
            mide::ui::draw::clear_rounded(pixmap, x, y, BANNER_W, bh, RADIUS);
        }
        fill_rounded(pixmap, x, y, BANNER_W, bh, RADIUS, {
            let [r, g, b, _] = theme.menu;
            rgba(r, g, b, if glass { 186 } else { 238 })
        });
        if let Some(path) = rounded_rect(x + 0.5, y + 0.5, BANNER_W - 1.0, bh - 1.0, RADIUS - 0.5) {
            let mut paint = Paint::default();
            paint.set_color(ink(34));
            paint.anti_alias = true;
            pixmap.stroke_path(&path, &paint, &Stroke { width: 1.0, ..Default::default() }, Transform::identity(), None);
        }

        // Ícone do app à esquerda.
        let ix = x + 14.0;
        let iy = y + (BANNER_H - ICON as f32) / 2.0;
        match icon {
            Some(icon) => pixmap.draw_pixmap(ix as i32, iy as i32, icon.as_ref(), &PixmapPaint::default(), Transform::identity(), None),
            None => fill_rounded(pixmap, ix, iy, ICON as f32, ICON as f32, 9.0, rgba(140, 140, 150, 255)),
        }

        // Título, "agora" à direita e o corpo em até duas linhas.
        let tx = ix + ICON as f32 + 12.0;
        let max_w = x + BANNER_W - 14.0 - tx;
        let now_w = self.text.measure("agora", 11.5, FontWeight::Regular);
        self.text.draw(pixmap, "agora", x + BANNER_W - 14.0 - now_w, y + 12.0, 11.5, FontWeight::Regular, theme.text_secondary);
        let title = if summary.is_empty() { app.clone() } else { summary };
        let title = fit(&mut self.text, &title, 13.5, FontWeight::Semibold, max_w - now_w - 8.0);
        self.text.draw(pixmap, &title, tx, y + 11.0, 13.5, FontWeight::Semibold, theme.text);
        let lines = wrap(&mut self.text, &body, 12.5, max_w, 2);
        for (k, line) in lines.iter().enumerate() {
            self.text.draw(pixmap, line, tx, y + 31.0 + k as f32 * 16.0, 12.5, FontWeight::Regular, theme.text_secondary);
        }

        // Botões das ações, alinhados à direita embaixo.
        if !buttons.is_empty() {
            let by = y + BANNER_H - 4.0;
            let mut bx = x + BANNER_W - 12.0;
            for (k, (_, label)) in buttons.iter().enumerate().rev() {
                let lw = self.text.measure(label, 12.5, FontWeight::Semibold) + 24.0;
                bx -= lw;
                let pressed = self.press == Some(Spot::Button(index, k));
                fill_rounded(pixmap, bx, by, lw, 26.0, 8.0, ink(if pressed { 40 } else { 18 }));
                self.text.draw(pixmap, label, bx + 12.0, by + 5.0, 12.5, FontWeight::Semibold, theme.text);
                bx -= 8.0;
            }
        }

        // × para fechar, no canto de cima à esquerda, só com o mouse em cima.
        if hovered {
            let (cx, cy) = (x + 4.0, y + 4.0);
            if let Some(circle) = PathBuilder::from_circle(cx, cy, 9.0) {
                let mut paint = Paint::default();
                paint.anti_alias = true;
                paint.set_color(rgba(250, 250, 252, 255));
                pixmap.fill_path(&circle, &paint, tiny_skia::FillRule::Winding, Transform::identity(), None);
                paint.set_color(rgba(0, 0, 0, 60));
                pixmap.stroke_path(&circle, &paint, &Stroke { width: 1.0, ..Default::default() }, Transform::identity(), None);
            }
            let mut pb = PathBuilder::new();
            pb.move_to(cx - 3.0, cy - 3.0);
            pb.line_to(cx + 3.0, cy + 3.0);
            pb.move_to(cx + 3.0, cy - 3.0);
            pb.line_to(cx - 3.0, cy + 3.0);
            if let Some(path) = pb.finish() {
                let mut paint = Paint::default();
                paint.anti_alias = true;
                paint.set_color(rgba(60, 60, 66, 255));
                pixmap.stroke_path(&path, &paint, &Stroke { width: 1.6, ..Default::default() }, Transform::identity(), None);
            }
        }
    }

    /// O que está sob o ponto: × (só no banner com o mouse em cima), botão ou banner.
    fn spot_at(&mut self, x: f64, y: f64) -> Option<Spot> {
        let (x, y) = (x as f32, y as f32);
        let tops = self.banner_tops();
        for (i, top) in tops.into_iter().enumerate() {
            let bh = Self::banner_height(&self.notes[i]);
            let close = ((x - (MARGIN + 4.0)).powi(2) + (y - (top + 4.0)).powi(2)).sqrt() <= 11.0;
            if close && self.hover == Some(i) {
                return Some(Spot::Close(i));
            }
            if x < MARGIN || x > MARGIN + BANNER_W || y < top || y > top + bh {
                continue;
            }
            let buttons = self.notes[i].buttons.clone();
            if !buttons.is_empty() && y >= top + BANNER_H - 4.0 {
                let mut bx = MARGIN + BANNER_W - 12.0;
                for (k, (_, label)) in buttons.iter().enumerate().rev() {
                    let lw = self.text.measure(label, 12.5, FontWeight::Semibold) + 24.0;
                    bx -= lw;
                    if x >= bx && x <= bx + lw {
                        return Some(Spot::Button(i, k));
                    }
                    bx -= 8.0;
                }
            }
            return Some(Spot::Body(i));
        }
        None
    }
}

/// Corta o texto com reticências para caber em `max` pixels.
fn fit(text: &mut Text, value: &str, size: f32, weight: FontWeight, max: f32) -> String {
    if text.measure(value, size, weight) <= max {
        return value.to_string();
    }
    let mut chars: Vec<char> = value.chars().collect();
    while !chars.is_empty() {
        chars.pop();
        let candidate = format!("{}…", chars.iter().collect::<String>().trim_end());
        if text.measure(&candidate, size, weight) <= max {
            return candidate;
        }
    }
    "…".into()
}

/// Quebra o texto em até `max_lines` linhas de no máximo `max` pixels; a última
/// ganha reticências se sobrar texto.
fn wrap(text: &mut Text, value: &str, size: f32, max: f32, max_lines: usize) -> Vec<String> {
    let words: Vec<&str> = value.split_whitespace().collect();
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut i = 0;
    while i < words.len() {
        let candidate = if current.is_empty() { words[i].to_string() } else { format!("{current} {}", words[i]) };
        if text.measure(&candidate, size, FontWeight::Regular) <= max || current.is_empty() {
            current = candidate;
            i += 1;
        } else {
            lines.push(std::mem::take(&mut current));
            if lines.len() == max_lines {
                break;
            }
        }
    }
    if !current.is_empty() && lines.len() < max_lines {
        lines.push(current);
    }
    if i < words.len() || lines.last().is_some_and(|l| text.measure(l, size, FontWeight::Regular) > max) {
        if let Some(last) = lines.last_mut() {
            let rest = words[i.min(words.len())..].join(" ");
            let joined = if rest.is_empty() { last.clone() } else { format!("{last} {rest}") };
            *last = fit(text, &joined, size, FontWeight::Regular, max);
        }
    }
    lines
}

impl CompositorHandler for Banners {
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: i32) {}
    fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: wl_output::Transform) {}
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {}
    fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
    fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
}

impl OutputHandler for Banners {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl LayerShellHandler for Banners {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        self.layer = None;
        self.configured = false;
    }

    fn configure(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface, configure: LayerSurfaceConfigure, _: u32) {
        if configure.new_size.0 > 0 && configure.new_size.1 > 0 {
            self.size = configure.new_size;
        }
        self.configured = true;
        self.draw();
    }
}

impl SeatHandler for Banners {
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

impl PointerHandler for Banners {
    fn pointer_frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_pointer::WlPointer, events: &[PointerEvent]) {
        for event in events {
            if self.layer.as_ref().map(|l| l.wl_surface()) != Some(&event.surface) {
                continue;
            }
            let (x, y) = event.position;
            match event.kind {
                PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => {
                    let hover = match self.spot_at(x, y) {
                        Some(Spot::Body(i) | Spot::Close(i) | Spot::Button(i, _)) => Some(i),
                        None => None,
                    };
                    if hover != self.hover {
                        self.hover = hover;
                        self.draw();
                    }
                }
                PointerEventKind::Leave { .. } => {
                    if self.hover.take().is_some() {
                        self.draw();
                    }
                }
                PointerEventKind::Press { button, .. } if button == BTN_LEFT => {
                    self.press = self.spot_at(x, y);
                    self.draw();
                }
                PointerEventKind::Release { button, .. } if button == BTN_LEFT => {
                    let spot = self.spot_at(x, y);
                    if self.press.take() != spot {
                        self.draw();
                        continue;
                    }
                    match spot {
                        Some(Spot::Close(i)) => {
                            if let Some(id) = self.notes.get(i).map(|n| n.id) {
                                self.remove(id, reason::DISMISSED);
                            }
                        }
                        Some(Spot::Button(i, k)) => {
                            if let Some(key) = self.notes.get(i).and_then(|n| n.buttons.get(k)).map(|(key, _)| key.clone()) {
                                self.invoke(i, &key);
                            }
                        }
                        Some(Spot::Body(i)) => {
                            // Clique no banner: a ação padrão do app, e o banner sai.
                            let (has_default, id) = match self.notes.get(i) {
                                Some(n) => (n.has_default, n.id),
                                None => continue,
                            };
                            if has_default {
                                self.invoke(i, "default");
                            } else {
                                self.remove(id, reason::DISMISSED);
                            }
                        }
                        None => {}
                    }
                }
                _ => {}
            }
        }
    }
}

impl ShmHandler for Banners {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

delegate_registry!(Banners);
smithay_client_toolkit::delegate_dispatch2!(Banners);

impl ProvidesRegistryState for Banners {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }

    registry_handlers![OutputState, SeatState];
}
