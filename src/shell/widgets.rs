//! Widgets da mesa (B27), como os do macOS Sonoma e Tahoe: cartões de vidro sobre o
//! papel de parede, no canto de cima à esquerda (os ícones ficam à direita). Relógio,
//! Calendário, Bateria, Disco e Nota; arrastar muda o lugar, o clique direito remove.
//!
//! Lugares e textos em `~/.local/state/mide/desktop-widgets.toml`. Lógica de lugar e de
//! arquivo pura (testada); o desenho usa o mide-ui.

use std::path::PathBuf;

use chrono::{Datelike, Local, Timelike};
use serde::{Deserialize, Serialize};

use crate::kit::Ui;
use crate::tr;

/// Lado de um widget pequeno (pontos), como o "pequeno" do Mac; o calendário é largo.
pub const SMALL: f32 = 164.0;
pub const GAP: f32 = 16.0;
const RADIUS: f32 = 22.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Clock,
    Calendar,
    Battery,
    Disk,
    Note,
}

impl Kind {
    pub const ALL: [Kind; 5] = [Kind::Clock, Kind::Calendar, Kind::Battery, Kind::Disk, Kind::Note];

    /// Tamanho em pontos.
    pub fn size(self) -> (f32, f32) {
        match self {
            Kind::Calendar => (2.0 * SMALL + GAP, SMALL),
            _ => (SMALL, SMALL),
        }
    }

    pub fn name(self) -> String {
        match self {
            Kind::Clock => tr!("Clock"),
            Kind::Calendar => tr!("Calendar"),
            Kind::Battery => tr!("Battery"),
            Kind::Disk => tr!("Disk Space"),
            Kind::Note => tr!("Note"),
        }
        .into_owned()
    }
}

/// Um widget na mesa: o tipo, o canto de cima à esquerda (pontos, relativo à área útil)
/// e o texto (da nota).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Widget {
    pub kind: Kind,
    pub x: f32,
    pub y: f32,
    #[serde(default)]
    pub text: String,
}

impl Widget {
    pub fn rect(&self, origin: (f32, f32)) -> (f32, f32, f32, f32) {
        let (w, h) = self.kind.size();
        (origin.0 + self.x, origin.1 + self.y, w, h)
    }
}

#[derive(Default, Serialize, Deserialize)]
struct Saved {
    #[serde(default)]
    widget: Vec<Widget>,
}

fn path() -> Option<PathBuf> {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))
        .map(|d| d.join("mide/desktop-widgets.toml"))
}

/// Os widgets guardados.
pub fn load() -> Vec<Widget> {
    path().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| toml::from_str::<Saved>(&t).ok()).map(|s| s.widget).unwrap_or_default()
}

/// Guarda os widgets.
pub fn save(widgets: &[Widget]) {
    let Some(file) = path() else {
        return;
    };
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    match toml::to_string(&Saved { widget: widgets.to_vec() }) {
        Ok(text) => {
            let _ = std::fs::write(file, text);
        }
        Err(err) => tracing::warn!("widgets não gravados: {err}"),
    }
}

fn overlaps(a: (f32, f32, f32, f32), b: (f32, f32, f32, f32)) -> bool {
    a.0 < b.0 + b.2 && b.0 < a.0 + a.2 && a.1 < b.1 + b.3 && b.1 < a.1 + a.3
}

/// Lugar livre para um widget novo: de cima para baixo e da esquerda para a direita,
/// na grade dos pequenos, dentro da metade esquerda da área (`w`×`h`), sem cobrir os
/// que já estão lá. Sem lugar: o canto de cima.
pub fn free_spot(existing: &[Widget], kind: Kind, (w, h): (f32, f32)) -> (f32, f32) {
    let (kw, kh) = kind.size();
    let step = SMALL + GAP;
    let max_x = (w * 0.6 - kw).max(GAP);
    let mut x = GAP;
    while x <= max_x {
        let mut y = GAP;
        while y + kh <= h - GAP {
            let me = (x, y, kw, kh);
            if !existing.iter().any(|o| overlaps(me, o.rect((0.0, 0.0)))) {
                return (x, y);
            }
            y += step;
        }
        x += step;
    }
    (GAP, GAP)
}

/// Desenha um widget em `rect` (pontos da janela). A nota é desenhada por quem chama
/// (ela tem um campo de texto que precisa do documento dela).
pub fn draw(ui: &mut Ui, w: &Widget, rect: (f32, f32, f32, f32)) {
    let t = ui.theme().clone();
    ui.glass_fill(rect, RADIUS);
    let edge = if t.dark { [255, 255, 255, 34] } else { [255, 255, 255, 120] };
    ui.paint(move |canvas, _| {
        // Véu do cartão (o vidro do compositor fica atrás) e a borda de luz.
        let veil = if t.dark { [30, 30, 34, 120] } else { [255, 255, 255, 110] };
        crate::ui::draw::fill_rounded(canvas, rect.0, rect.1, rect.2, rect.3, RADIUS, color(veil));
        if let Some(path) = crate::ui::draw::rounded_rect(rect.0 + 0.5, rect.1 + 0.5, rect.2 - 1.0, rect.3 - 1.0, RADIUS - 0.5) {
            let mut paint = tiny_skia::Paint { anti_alias: true, ..Default::default() };
            paint.set_color(color(edge));
            canvas.stroke_path(&path, &paint, &tiny_skia::Stroke { width: 1.0, ..Default::default() }, crate::ui::draw::tf(), None);
        }
    });
    match w.kind {
        Kind::Clock => clock(ui, rect),
        Kind::Calendar => calendar(ui, rect),
        Kind::Battery => battery(ui, rect),
        Kind::Disk => disk(ui, rect),
        Kind::Note => {
            // Título na faixa de cima (por onde a nota se arrasta), no amarelo das notas.
            let title = tr!("Note").into_owned();
            ui.paint_text(move |canvas, text, _| {
                text.draw(canvas, &title, rect.0 + 14.0, rect.1 + 10.0, 12.0, crate::ui::text::FontWeight::Semibold, [230, 165, 0, 255]);
            });
        }
    }
}

fn color(c: [u8; 4]) -> tiny_skia::Color {
    tiny_skia::Color::from_rgba8(c[0], c[1], c[2], c[3])
}

/// Relógio de ponteiros com o dia da semana embaixo.
fn clock(ui: &mut Ui, (x, y, w, h): (f32, f32, f32, f32)) {
    let t = ui.theme().clone();
    let now = Local::now();
    let (cx, cy, r) = (x + w / 2.0, y + h / 2.0, w.min(h) / 2.0 - 16.0);
    let ink = t.text;
    let accent = t.accent;
    let (hh, mm, ss) = (now.hour() % 12, now.minute(), now.second());
    ui.paint(move |canvas, _| {
        let stroke = |canvas: &mut tiny_skia::Pixmap, a: f32, len: f32, width: f32, c: [u8; 4]| {
            let (sx, sy) = (cx + a.sin() * len, cy - a.cos() * len);
            let mut pb = tiny_skia::PathBuilder::new();
            pb.move_to(cx, cy);
            pb.line_to(sx, sy);
            if let Some(path) = pb.finish() {
                let mut paint = tiny_skia::Paint { anti_alias: true, ..Default::default() };
                paint.set_color(color(c));
                canvas.stroke_path(&path, &paint, &tiny_skia::Stroke { width, line_cap: tiny_skia::LineCap::Round, ..Default::default() }, crate::ui::draw::tf(), None);
            }
        };
        // Marcas das horas.
        for i in 0..12 {
            let a = i as f32 * std::f32::consts::TAU / 12.0;
            let (ax, ay) = (cx + a.sin() * (r - 2.0), cy - a.cos() * (r - 2.0));
            crate::ui::draw::fill_rounded(canvas, ax - 1.5, ay - 1.5, 3.0, 3.0, 1.5, color([ink[0], ink[1], ink[2], 150]));
        }
        let tau = std::f32::consts::TAU;
        stroke(canvas, (hh as f32 + mm as f32 / 60.0) / 12.0 * tau, r * 0.5, 4.0, ink);
        stroke(canvas, (mm as f32 + ss as f32 / 60.0) / 60.0 * tau, r * 0.78, 3.0, ink);
        stroke(canvas, ss as f32 / 60.0 * tau, r * 0.85, 1.4, accent);
        crate::ui::draw::fill_rounded(canvas, cx - 3.5, cy - 3.5, 7.0, 7.0, 3.5, color(accent));
    });
    ui.wake_after(std::time::Duration::from_millis(1000 - (now.timestamp_subsec_millis() as u64).min(999)));
}

/// Calendário largo: o dia grande à esquerda e o mês à direita, com hoje destacado.
fn calendar(ui: &mut Ui, (x, y, w, h): (f32, f32, f32, f32)) {
    let t = ui.theme().clone();
    let today = Local::now().date_naive();
    let first = crate::kit::region::region().first_weekday;
    let weekday = crate::calendar::weekday_name(today.weekday().num_days_from_sunday());
    let month = crate::calendar::month_name(today.month());
    let initials = crate::calendar::weekday_initials_from(first);
    let grid = crate::calendar::month_grid_from(today.year(), today.month(), first);
    let (ink, sec, accent, on_accent) = (t.text, t.text_secondary, t.accent, t.on_accent);
    let half = w / 2.0;
    ui.paint_text(move |canvas, text, _| {
        use crate::ui::text::FontWeight;
        text.draw(canvas, &weekday.to_uppercase(), x + 18.0, y + 18.0, 12.0, FontWeight::Semibold, [accent[0], accent[1], accent[2], 255]);
        text.draw(canvas, &today.day().to_string(), x + 16.0, y + 34.0, 52.0, FontWeight::Regular, ink);
        text.draw(canvas, &month, x + 18.0, y + h - 34.0, 13.0, FontWeight::Semibold, sec);
        // O mês: iniciais e as seis semanas.
        let (gx, gy) = (x + half + 4.0, y + 18.0);
        let cw = (half - 22.0) / 7.0;
        let rh = (h - 36.0) / 7.0;
        for (i, ini) in initials.iter().enumerate() {
            let tw = text.measure(ini, 10.0, FontWeight::Semibold);
            text.draw(canvas, ini, gx + i as f32 * cw + (cw - tw) / 2.0, gy, 10.0, FontWeight::Semibold, sec);
        }
        for (i, day) in grid.iter().enumerate() {
            let (col, row) = (i % 7, i / 7 + 1);
            let label = day.date.day().to_string();
            let tw = text.measure(&label, 10.5, FontWeight::Regular);
            let (dx, dy) = (gx + col as f32 * cw + (cw - tw) / 2.0, gy + row as f32 * rh);
            let c = if day.date == today {
                crate::ui::draw::fill_rounded(canvas, gx + col as f32 * cw + cw / 2.0 - 9.0, dy - 3.0, 18.0, 18.0, 9.0, color(accent));
                on_accent
            } else if day.in_month {
                ink
            } else {
                [sec[0], sec[1], sec[2], 110]
            };
            text.draw(canvas, &label, dx, dy, 10.5, FontWeight::Regular, c);
        }
    });
}

/// Anel de progresso com o valor no meio (bateria e disco).
fn ring(ui: &mut Ui, (x, y, w, h): (f32, f32, f32, f32), fraction: f32, big: String, small: String, fill: [u8; 4]) {
    let t = ui.theme().clone();
    let (cx, cy, r) = (x + w / 2.0, y + h / 2.0 - 8.0, w.min(h) / 2.0 - 30.0);
    let track = t.shade(if t.dark { 50 } else { 30 });
    let (ink, sec) = (t.text, t.text_secondary);
    ui.paint(move |canvas, _| {
        let arc = |canvas: &mut tiny_skia::Pixmap, from: f32, to: f32, c: [u8; 4]| {
            let mut pb = tiny_skia::PathBuilder::new();
            let steps = 64;
            for i in 0..=steps {
                let a = from + (to - from) * i as f32 / steps as f32 - std::f32::consts::FRAC_PI_2;
                let (px, py) = (cx + a.cos() * r, cy + a.sin() * r);
                if i == 0 { pb.move_to(px, py) } else { pb.line_to(px, py) }
            }
            if let Some(path) = pb.finish() {
                let mut paint = tiny_skia::Paint { anti_alias: true, ..Default::default() };
                paint.set_color(color(c));
                canvas.stroke_path(&path, &paint, &tiny_skia::Stroke { width: 9.0, line_cap: tiny_skia::LineCap::Round, ..Default::default() }, crate::ui::draw::tf(), None);
            }
        };
        let tau = std::f32::consts::TAU;
        arc(canvas, 0.0, tau, track);
        if fraction > 0.005 {
            arc(canvas, 0.0, tau * fraction.clamp(0.0, 1.0), fill);
        }
    });
    ui.paint_text(move |canvas, text, _| {
        use crate::ui::text::FontWeight;
        let bw = text.measure(&big, 22.0, FontWeight::Semibold);
        text.draw(canvas, &big, cx - bw / 2.0, cy - 14.0, 22.0, FontWeight::Semibold, ink);
        let sw = text.measure(&small, 11.5, FontWeight::Regular);
        text.draw(canvas, &small, cx - sw / 2.0, y + h - 26.0, 11.5, FontWeight::Regular, sec);
    });
}

fn battery(ui: &mut Ui, rect: (f32, f32, f32, f32)) {
    match crate::battery::read() {
        Some(b) => {
            let fill = if b.percent <= 20 && !b.charging { [255, 69, 58, 255] } else { [52, 199, 89, 255] };
            let note = if b.charging { tr!("Charging") } else { tr!("Battery") }.into_owned();
            ring(ui, rect, b.percent as f32 / 100.0, format!("{}%", b.percent), note, fill);
        }
        None => ring(ui, rect, 0.0, "—".into(), tr!("No battery").into_owned(), [52, 199, 89, 255]),
    }
}

fn disk(ui: &mut Ui, rect: (f32, f32, f32, f32)) {
    let home = crate::places::home_dir();
    let accent = ui.theme().accent;
    match rustix::fs::statvfs(&home) {
        Ok(v) => {
            let total = (v.f_blocks as f64) * v.f_frsize as f64;
            let free = (v.f_bavail as f64) * v.f_frsize as f64;
            let used = if total > 0.0 { 1.0 - free / total } else { 0.0 };
            let free_text = tr!("{size} free", size = crate::files::size_text(free as u64));
            ring(ui, rect, used as f32, format!("{:.0}%", used * 100.0), free_text, accent);
        }
        Err(_) => ring(ui, rect, 0.0, "—".into(), tr!("Disk Space").into_owned(), accent),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lugar_livre_sem_cobrir_os_outros() {
        let area = (1366.0, 700.0);
        let a = free_spot(&[], Kind::Clock, area);
        assert_eq!(a, (GAP, GAP));
        let w1 = Widget { kind: Kind::Clock, x: a.0, y: a.1, text: String::new() };
        let b = free_spot(std::slice::from_ref(&w1), Kind::Calendar, area);
        // Logo abaixo do relógio (a coluna da esquerda primeiro).
        assert_eq!(b, (GAP, GAP + SMALL + GAP));
        let w2 = Widget { kind: Kind::Calendar, x: b.0, y: b.1, text: String::new() };
        let c = free_spot(&[w1, w2], Kind::Note, area);
        assert!(!overlaps((c.0, c.1, SMALL, SMALL), (b.0, b.1, 2.0 * SMALL + GAP, SMALL)));
    }

    #[test]
    fn grava_e_le_o_arquivo() {
        let text = toml::to_string(&Saved { widget: vec![Widget { kind: Kind::Note, x: 16.0, y: 196.0, text: "comprar pão".into() }] }).unwrap();
        let back: Saved = toml::from_str(&text).unwrap();
        assert_eq!(back.widget[0].kind, Kind::Note);
        assert_eq!(back.widget[0].text, "comprar pão");
    }
}
