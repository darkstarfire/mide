//! Player de Vídeo do mide (`mide-player`), como o QuickTime Player do Mac no visual do
//! Tahoe: o vídeo na janela inteira, sobre preto, e os controles numa cápsula escura
//! que aparece com o mouse e some sozinha enquanto o vídeo toca.
//!
//! O núcleo (quadros, som e relógio) é `mide::media::video`; o `ffmpeg` decodifica
//! como programa à parte (decisão do usuário em 2026-10-25).
//!
//! Teclado: Espaço toca/pausa, ←/→ voltam/avançam 5 s, ↑/↓ mudam o volume, M tira o
//! som, Ctrl+O abre outro vídeo.

use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use mide::kit::{run, window_menu, App, Choice, FileChooser, Key, MenuGroup, MenuItem, Theme, Ui, Window};
use mide::media::video::{self, Player};
use mide::ui::{
    draw::{copy_rgba, fill_rounded, scale},
    text::FontWeight,
};
use tiny_skia::{Color, FillRule, Paint, PathBuilder, Pixmap};

/// Quanto tempo os controles ficam à mostra depois do último movimento do mouse.
const HIDE_AFTER: Duration = Duration::from_millis(2500);
/// Pulos dos botões e das setas.
const SKIP_BUTTON: f64 = 15.0;
const SKIP_KEY: f64 = 5.0;

struct VideoApp {
    player: Option<Player>,
    chooser: Option<FileChooser>,
    error: Option<String>,
    /// Controles à mostra até este instante (com o vídeo parado, sempre).
    shown_until: Instant,
    last_pointer: Option<(f32, f32)>,
    /// Arrastando a barra de tempo (posição escolhida, de 0 a 1) ou a do volume.
    scrubbing: Option<f64>,
    dragging_volume: bool,
    /// Último pulo pedido durante o arrasto da barra (para não recomeçar a cada pixel).
    scrub_sent: Option<Instant>,
    /// Tamanho do vídeo pedido ao decodificador, e quando a janela mudou de tamanho.
    resize_at: Option<(Instant, (u32, u32))>,
}

impl VideoApp {
    fn new(path: Option<PathBuf>) -> Self {
        let mut app = Self {
            player: None,
            chooser: None,
            error: None,
            shown_until: Instant::now() + HIDE_AFTER,
            last_pointer: None,
            scrubbing: None,
            dragging_volume: false,
            scrub_sent: None,
            resize_at: None,
        };
        match path {
            Some(path) => app.open(path),
            None => app.chooser = Some(open_dialog()),
        }
        app
    }

    fn open(&mut self, path: PathBuf) {
        self.player = None;
        // O tamanho de verdade vem no primeiro quadro (a janela já está aberta).
        match Player::open(&path, (960, 540), true) {
            Some(mut player) => {
                tracing::info!("player: {} ({}x{}, {:.1} s)", path.display(), player.info.width, player.info.height, player.info.duration);
                player.play();
                self.player = Some(player);
                self.error = None;
            }
            None => {
                tracing::warn!("player: não abriu {}", path.display());
                self.error = Some(format!("Não foi possível abrir \"{}\".", path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()));
            }
        }
    }

    /// Ações dos menus e do teclado.
    fn act(&mut self, action: &str) {
        let Some(player) = self.player.as_mut() else {
            if action == "open" {
                self.chooser = Some(open_dialog());
            }
            return;
        };
        match action {
            "open" => self.chooser = Some(open_dialog()),
            "toggle" => player.toggle(),
            "back" => player.seek(player.position() - SKIP_BUTTON),
            "forward" => player.seek(player.position() + SKIP_BUTTON),
            "back-key" => player.seek(player.position() - SKIP_KEY),
            "forward-key" => player.seek(player.position() + SKIP_KEY),
            "loop" => player.looped = !player.looped,
            "mute" => {
                let (v, m) = (player.volume, !player.muted);
                player.set_volume(v, m);
            }
            "volume-up" => {
                let v = player.volume + 0.1;
                player.set_volume(v, false);
            }
            "volume-down" => {
                let (v, m) = (player.volume - 0.1, player.muted);
                player.set_volume(v, m);
            }
            _ => {}
        }
        self.shown_until = Instant::now() + HIDE_AFTER;
    }
}

fn open_dialog() -> FileChooser {
    let dir = mide::places::videos_dir();
    FileChooser::open("Abrir vídeo", Some(&dir)).extensions(&["mp4", "m4v", "mov", "mkv", "webm", "avi", "wmv", "flv", "ogv", "mpg", "mpeg", "3gp", "ts"])
}

/// Retângulos dos controles (em pontos), a partir do tamanho da janela.
struct Hud {
    panel: (f32, f32, f32, f32),
    back: (f32, f32, f32, f32),
    play: (f32, f32, f32, f32),
    forward: (f32, f32, f32, f32),
    speaker: (f32, f32, f32, f32),
    volume: (f32, f32, f32, f32),
    track: (f32, f32, f32, f32),
}

fn hud(w: f32, h: f32) -> Hud {
    let pw = (w - 32.0).min(560.0);
    let ph = 74.0;
    let (px, py) = ((w - pw) / 2.0, h - ph - 16.0);
    let cx = px + pw / 2.0;
    let row = py + 8.0;
    Hud {
        panel: (px, py, pw, ph),
        back: (cx - 72.0, row, 36.0, 34.0),
        play: (cx - 20.0, row - 2.0, 40.0, 38.0),
        forward: (cx + 36.0, row, 36.0, 34.0),
        speaker: (px + 14.0, row + 2.0, 30.0, 30.0),
        volume: (px + 46.0, row + 13.0, 80.0, 8.0),
        track: (px + 64.0, py + 52.0, pw - 128.0, 6.0),
    }
}

/// Corta o texto com reticências para caber em `max` pontos.
fn fit(text: &mut mide::ui::text::Text, s: &str, size: f32, max: f32) -> String {
    if text.measure(s, size, FontWeight::Semibold) <= max {
        return s.to_string();
    }
    let mut chars: Vec<char> = s.chars().collect();
    while chars.len() > 1 {
        chars.pop();
        let c = format!("{}…", chars.iter().collect::<String>().trim_end());
        if text.measure(&c, size, FontWeight::Semibold) <= max {
            return c;
        }
    }
    "…".into()
}

fn inside(r: (f32, f32, f32, f32), p: (f32, f32)) -> bool {
    p.0 >= r.0 && p.0 < r.0 + r.2 && p.1 >= r.1 && p.1 < r.1 + r.3
}

fn rgba(r: u8, g: u8, b: u8, a: u8) -> Color {
    Color::from_rgba8(r, g, b, a)
}

/// Triângulo de tocar, apontando para a direita (ou para a esquerda).
fn triangle(p: &mut Pixmap, x: f32, y: f32, size: f32, left: bool, c: Color) {
    let mut pb = PathBuilder::new();
    if left {
        pb.move_to(x + size, y);
        pb.line_to(x, y + size / 2.0);
        pb.line_to(x + size, y + size);
    } else {
        pb.move_to(x, y);
        pb.line_to(x + size * 0.9, y + size / 2.0);
        pb.line_to(x, y + size);
    }
    pb.close();
    if let Some(path) = pb.finish() {
        let mut paint = Paint::default();
        paint.set_color(c);
        paint.anti_alias = true;
        p.fill_path(&path, &paint, FillRule::Winding, mide::ui::draw::tf(), None);
    }
}

/// Alto-falante com ondas pelo volume (sem ondas e com um traço no mudo).
fn speaker(p: &mut Pixmap, x: f32, y: f32, volume: f32, muted: bool, c: Color) {
    fill_rounded(p, x, y + 6.0, 5.0, 8.0, 1.0, c);
    let mut pb = PathBuilder::new();
    pb.move_to(x + 4.0, y + 6.0);
    pb.line_to(x + 10.0, y + 1.0);
    pb.line_to(x + 10.0, y + 19.0);
    pb.line_to(x + 4.0, y + 14.0);
    pb.close();
    let mut paint = Paint::default();
    paint.set_color(c);
    paint.anti_alias = true;
    if let Some(path) = pb.finish() {
        p.fill_path(&path, &paint, FillRule::Winding, mide::ui::draw::tf(), None);
    }
    let stroke = tiny_skia::Stroke { width: 1.6, line_cap: tiny_skia::LineCap::Round, ..Default::default() };
    if muted {
        let mut pb = PathBuilder::new();
        pb.move_to(x + 13.0, y + 6.0);
        pb.line_to(x + 19.0, y + 14.0);
        pb.move_to(x + 19.0, y + 6.0);
        pb.line_to(x + 13.0, y + 14.0);
        if let Some(path) = pb.finish() {
            p.stroke_path(&path, &paint, &stroke, mide::ui::draw::tf(), None);
        }
        return;
    }
    let waves = if volume <= 0.01 { 0 } else if volume < 0.4 { 1 } else if volume < 0.75 { 2 } else { 3 };
    for i in 0..waves {
        let r = 4.0 + i as f32 * 3.5;
        let mut pb = PathBuilder::new();
        let (cx, cy) = (x + 10.0, y + 10.0);
        pb.move_to(cx + r * 0.6, cy - r * 0.8);
        pb.quad_to(cx + r * 1.25, cy, cx + r * 0.6, cy + r * 0.8);
        if let Some(path) = pb.finish() {
            p.stroke_path(&path, &paint, &stroke, mide::ui::draw::tf(), None);
        }
    }
}

impl App for VideoApp {
    fn view(&mut self, ui: &mut Ui) {
        let (w, h) = ui.window_size();
        let now = Instant::now();
        // Vídeo solto na janela (da área de trabalho, do Arquivos): toca nela.
        if let Some(path) = ui.input().dropped.clone().and_then(|paths| paths.into_iter().find(|p| mide::media::video::is_video(p))) {
            self.chooser = None;
            self.open(path);
            ui.request_repaint();
        }
        // Ações dos menus da barra.
        for action in ui.input().menu.clone() {
            match action.as_str() {
                "close" => ui.close(),
                other => self.act(other),
            }
        }
        // Seletor de arquivo (sem vídeo, ou Abrir…).
        if let Some(chooser) = self.chooser.as_mut() {
            match chooser.show(ui) {
                Some(Choice::Chosen(path)) => {
                    self.chooser = None;
                    self.open(path);
                }
                Some(Choice::Cancelled) => {
                    self.chooser = None;
                    if self.player.is_none() && self.error.is_none() {
                        ui.close();
                    }
                }
                None => {}
            }
            return;
        }
        let input = ui.input().clone();
        // Teclado.
        if input.text.contains(' ') {
            self.act("toggle");
        }
        if input.text.to_lowercase().contains('m') && !input.ctrl {
            self.act("mute");
        }
        if input.key(Key::Left) {
            self.act("back-key");
        }
        if input.key(Key::Right) {
            self.act("forward-key");
        }
        if input.key(Key::Up) {
            self.act("volume-up");
        }
        if input.key(Key::Down) {
            self.act("volume-down");
        }
        if input.shortcut("o") {
            self.act("open");
        }
        if input.shortcut("w") {
            ui.close();
        }
        // Mouse mexendo: controles à mostra.
        if input.pointer != self.last_pointer {
            self.last_pointer = input.pointer;
            self.shown_until = now + HIDE_AFTER;
        }

        // Fundo preto e o quadro, centrado (em pixels: cópia direta, sem escala).
        let k = scale();
        let (pw, ph) = ((w * k).round() as u32, (h * k).round() as u32);
        if let Some(player) = self.player.as_mut() {
            // Janela de outro tamanho: o decodificador recomeça no tamanho novo (depois
            // de um instante parada, para não recomeçar a cada pixel do arrasto).
            if video::fit(&player.info, (pw, ph)) != player.size() {
                match self.resize_at {
                    Some((at, size)) if size == (pw, ph) && now.duration_since(at) > Duration::from_millis(200) => {
                        player.resize((pw, ph));
                        self.resize_at = None;
                    }
                    Some((_, size)) if size == (pw, ph) => ui.wake_after(Duration::from_millis(220)),
                    _ => {
                        self.resize_at = Some((now, (pw, ph)));
                        ui.wake_after(Duration::from_millis(220));
                    }
                }
            }
            player.tick();
        }
        let frame = self.player.as_ref().and_then(|p| p.frame().cloned());
        ui.paint(|canvas, _| {
            canvas.fill(rgba(0, 0, 0, 255));
            if let Some(f) = &frame {
                let x = (canvas.width() as i32 - f.width as i32) / 2;
                let y = (canvas.height() as i32 - f.height as i32) / 2;
                copy_rgba(canvas, &f.pixels, f.width, f.height, x, y);
            }
        });

        let playing = self.player.as_ref().is_some_and(|p| p.playing());
        let shown = !playing || now < self.shown_until || self.scrubbing.is_some() || self.dragging_volume || input.pointer.is_some_and(|p| inside(hud(w, h).panel, p));
        if let Some(err) = &self.error {
            let err = err.clone();
            ui.paint_text(|canvas, text, _| {
                let tw = text.measure(&err, 15.0, FontWeight::Regular);
                text.draw(canvas, &err, (w - tw) / 2.0, h / 2.0 - 10.0, 15.0, FontWeight::Regular, [235, 235, 240, 255]);
            });
        }
        if shown {
            // Luzes e título no topo, sobre uma faixa escura suave.
            let title = self.player.as_ref().and_then(|p| p.path.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Player de Vídeo".into());
            ui.paint(|canvas, _| fill_rounded(canvas, 0.0, 0.0, w, 44.0, 0.0, rgba(0, 0, 0, 90)));
            ui.paint_text(|canvas, text, _| {
                let size = 13.0;
                let t = fit(text, &title, size, w - 200.0);
                let tw = text.measure(&t, size, FontWeight::Semibold);
                text.draw(canvas, &t, (w - tw) / 2.0, 13.0, size, FontWeight::Semibold, [235, 235, 240, 255]);
            });
            ui.window_controls(14.0, 16.0);
            if self.player.is_some() {
                self.draw_hud(ui, w, h, &input);
            }
        } else if playing {
            ui.wake_after(self.shown_until.saturating_duration_since(now) + Duration::from_millis(20));
        }
        // Clique no vídeo (fora dos controles e da faixa de cima): toca ou pausa.
        let over_hud = shown && self.player.is_some() && input.pointer.is_some_and(|p| inside(hud(w, h).panel, p));
        if input.pressed && !over_hud && input.pointer.is_some_and(|p| p.1 > 44.0) && self.player.is_some() {
            self.act("toggle");
        }
        // Faixa de cima: arrasta a janela (duplo clique maximiza).
        ui.window_drag_area((0.0, 0.0, w, 44.0));
        // Próximo quadro na hora certa; controles somem sozinhos.
        if let Some(due) = self.player.as_ref().and_then(|p| p.next_due()) {
            ui.wake_after(due);
        }
        if shown && playing {
            ui.wake_after(self.shown_until.saturating_duration_since(now) + Duration::from_millis(20));
        }
    }

    fn menu_bar(&mut self) -> Vec<MenuGroup> {
        let (playing, looped, muted, has) = match self.player.as_ref() {
            Some(p) => (p.playing(), p.looped, p.muted, true),
            None => (false, false, false, false),
        };
        vec![
            MenuGroup {
                title: "Arquivo".into(),
                items: vec![MenuItem::action("open", "Abrir…").shortcut("Ctrl+O"), MenuItem::Separator, MenuItem::action("close", "Fechar").shortcut("Ctrl+W")],
            },
            MenuGroup {
                title: "Reprodução".into(),
                items: vec![
                    MenuItem::action("toggle", if playing { "Pausar" } else { "Reproduzir" }).enabled(has),
                    MenuItem::Separator,
                    MenuItem::action("back", "Voltar 15 segundos").enabled(has),
                    MenuItem::action("forward", "Avançar 15 segundos").enabled(has),
                    MenuItem::Separator,
                    MenuItem::action("loop", "Repetir").checked(looped).enabled(has),
                    MenuItem::action("mute", "Sem som").checked(muted).enabled(has),
                ],
            },
            window_menu(Vec::new()),
        ]
    }
}

impl VideoApp {
    /// Cápsula de controles: voltar, tocar/pausar, avançar, volume e a barra de tempo.
    fn draw_hud(&mut self, ui: &mut Ui, w: f32, h: f32, input: &mide::kit::Input) {
        let hud = hud(w, h);
        let accent = ui.theme().accent;
        let Some(player) = self.player.as_mut() else {
            return;
        };
        let duration = player.info.duration.max(0.001);
        let pointer = input.pointer;
        // Arrasto da barra de tempo e do volume.
        if input.pressed {
            if let Some(p) = pointer {
                let grow = |r: (f32, f32, f32, f32)| (r.0 - 6.0, r.1 - 10.0, r.2 + 12.0, r.3 + 20.0);
                if inside(grow(hud.track), p) {
                    self.scrubbing = Some(0.0);
                } else if inside(grow(hud.volume), p) {
                    self.dragging_volume = true;
                } else if inside(hud.play, p) {
                    player.toggle();
                } else if inside(hud.back, p) {
                    player.seek(player.position() - SKIP_BUTTON);
                } else if inside(hud.forward, p) {
                    player.seek(player.position() + SKIP_BUTTON);
                } else if inside(hud.speaker, p) {
                    let (v, m) = (player.volume, !player.muted);
                    player.set_volume(v, m);
                }
            }
        }
        let frac_at = |r: (f32, f32, f32, f32), x: f32| ((x - r.0) / r.2).clamp(0.0, 1.0) as f64;
        if let (Some(_), Some(p)) = (self.scrubbing, pointer) {
            let f = frac_at(hud.track, p.0);
            self.scrubbing = Some(f);
            // Durante o arrasto, o quadro acompanha (no máximo um pulo a cada 120 ms).
            if self.scrub_sent.is_none_or(|t| t.elapsed() > Duration::from_millis(120)) || input.released {
                player.seek(f * duration);
                self.scrub_sent = Some(Instant::now());
            }
        }
        if self.dragging_volume {
            if let Some(p) = pointer {
                let v = frac_at(hud.volume, p.0) as f32;
                player.set_volume(v, false);
            }
        }
        if input.released || !input.down {
            self.scrubbing = None;
            self.dragging_volume = false;
        }
        let position = self.scrubbing.map(|f| f * duration).unwrap_or_else(|| player.position());
        let playing = player.playing();
        let (volume, muted) = (player.volume, player.muted);
        let elapsed = video::time_text(position);
        let remaining = format!("-{}", video::time_text(duration - position));
        let hover = |r: (f32, f32, f32, f32)| pointer.is_some_and(|p| inside(r, p));
        let (hb, hp, hf) = (hover(hud.back), hover(hud.play), hover(hud.forward));
        ui.paint(|canvas, _| {
            let (x, y, pw, ph) = hud.panel;
            fill_rounded(canvas, x, y, pw, ph, 20.0, rgba(28, 28, 30, 200));
            let white = rgba(240, 240, 245, 255);
            for (r, on) in [(hud.back, hb), (hud.play, hp), (hud.forward, hf)] {
                if on {
                    fill_rounded(canvas, r.0, r.1, r.2, r.3, r.3 / 2.0, rgba(255, 255, 255, 30));
                }
            }
            // Voltar e avançar: dois triângulos; tocar/pausar no meio, maior.
            let (bx, by) = (hud.back.0 + 8.0, hud.back.1 + 10.0);
            triangle(canvas, bx, by, 14.0, true, white);
            triangle(canvas, bx + 8.0, by, 14.0, true, white);
            let (fx, fy) = (hud.forward.0 + 8.0, hud.forward.1 + 10.0);
            triangle(canvas, fx, fy, 14.0, false, white);
            triangle(canvas, fx + 8.0, fy, 14.0, false, white);
            let (cx, cy) = (hud.play.0 + hud.play.2 / 2.0, hud.play.1 + hud.play.3 / 2.0);
            if playing {
                fill_rounded(canvas, cx - 8.0, cy - 10.0, 5.5, 20.0, 1.5, white);
                fill_rounded(canvas, cx + 2.5, cy - 10.0, 5.5, 20.0, 1.5, white);
            } else {
                triangle(canvas, cx - 7.0, cy - 11.0, 22.0, false, white);
            }
            // Volume.
            speaker(canvas, hud.speaker.0 + 4.0, hud.speaker.1 + 5.0, volume, muted, white);
            let v = if muted { 0.0 } else { volume };
            let (vx, vy, vw, vh) = hud.volume;
            fill_rounded(canvas, vx, vy + 1.0, vw, vh - 2.0, (vh - 2.0) / 2.0, rgba(255, 255, 255, 60));
            fill_rounded(canvas, vx, vy + 1.0, (vw * v).max(vh - 2.0), vh - 2.0, (vh - 2.0) / 2.0, white);
            // Barra de tempo na cor de destaque.
            let (tx, ty, tw, th) = hud.track;
            let f = (position / duration).clamp(0.0, 1.0) as f32;
            fill_rounded(canvas, tx, ty, tw, th, th / 2.0, rgba(255, 255, 255, 60));
            fill_rounded(canvas, tx, ty, (tw * f).max(th), th, th / 2.0, rgba(accent[0], accent[1], accent[2], 255));
            fill_rounded(canvas, tx + tw * f - 6.0, ty - 3.0, 12.0, 12.0, 6.0, white);
        });
        ui.paint_text(|canvas, text, _| {
            let c = [220, 220, 228, 255];
            let (x, y, pw, _) = hud.panel;
            text.draw(canvas, &elapsed, x + 14.0, y + 46.0, 11.5, FontWeight::Regular, c);
            let rw = text.measure(&remaining, 11.5, FontWeight::Regular);
            text.draw(canvas, &remaining, x + pw - 14.0 - rw, y + 46.0, 11.5, FontWeight::Regular, c);
        });
        if self.scrubbing.is_some() || self.dragging_volume {
            ui.request_tick();
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "mide_player=info,mide=info".into()))
        .init();
    let path = std::env::args().nth(1).map(|a| PathBuf::from(a.strip_prefix("file://").unwrap_or(&a)));
    // Janela no tamanho do vídeo (até 960x540), no escuro como o QuickTime.
    let size = path
        .as_deref()
        .and_then(video::probe)
        .map(|info| video::fit(&info, (960, 540)))
        .map(|(w, h)| (w.max(480), h.max(270)))
        .unwrap_or((800, 450));
    let mut window = Window::new("Player de Vídeo").app_id("mide-player").size(size.0, size.1).min_size(400, 225).full_size_content();
    let mut theme = Theme::dark();
    theme.accent = mide::kit::system_accent(true);
    window.theme = theme;
    window.theme_fixed = true;
    run(window, VideoApp::new(path))
}
