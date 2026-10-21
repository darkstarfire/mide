//! Captura de tela do mide (tecla Print): a tela congela e aparece por cima de tudo.
//!
//! - **Seleção** (padrão): arraste para escolher a área.
//! - **Janela**: clique numa janela.
//! - **Tela inteira**: a tela toda.
//!
//! Depois de escolher a área, arrastar dentro dela desenha com a ferramenta escolhida
//! (seta, círculo ou quadrado) na cor escolhida (vermelho, branco, preto, verde,
//! amarelo ou azul). **Copiar** (Enter ou Ctrl+C) põe a imagem na área de
//! transferência; **Salvar** (Ctrl+S) grava em Imagens/Capturas de tela; Esc cancela.
//!
//! A imagem vem do compositor (`screenshot`), sem o cursor; a cópia fica com o
//! compositor (`copy-file`), então continua lá depois que a captura fecha.

use std::{error::Error, path::PathBuf, time::Duration};

use mide::{
    control_client::ask_compositor,
    kit::{run, App, Key, Ui, Window},
};
use tiny_skia::{Color, FillRule, LineCap, Paint, PathBuilder, Pixmap, PixmapPaint, Rect, Stroke, Transform};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Selection,
    Window,
    Full,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tool {
    Arrow,
    Circle,
    Square,
}

/// Cores para desenhar: nome e RGB.
const COLORS: [(&str, [u8; 3]); 6] = [
    ("vermelho", [235, 52, 52]),
    ("branco", [255, 255, 255]),
    ("preto", [20, 20, 22]),
    ("verde", [52, 199, 89]),
    ("amarelo", [255, 204, 0]),
    ("azul", [10, 132, 255]),
];

/// Uma marcação desenhada sobre a captura (coordenadas da tela).
#[derive(Debug, Clone, Copy)]
struct Shape {
    tool: Tool,
    color: [u8; 3],
    from: (f32, f32),
    to: (f32, f32),
}

/// Retângulo (x, y, largura, altura) com largura e altura positivas.
fn normalized(a: (f32, f32), b: (f32, f32)) -> (f32, f32, f32, f32) {
    (a.0.min(b.0), a.1.min(b.1), (a.0 - b.0).abs(), (a.1 - b.1).abs())
}

fn inside(r: (f32, f32, f32, f32), p: (f32, f32)) -> bool {
    p.0 >= r.0 && p.0 <= r.0 + r.2 && p.1 >= r.1 && p.1 <= r.1 + r.3
}

struct Capture {
    shot: Pixmap,
    /// A captura já escurecida (fora da área escolhida), feita uma vez: cada quadro só
    /// copia bytes, sem misturar a tela inteira de novo.
    dimmed: Pixmap,
    mode: Mode,
    tool: Tool,
    color: usize,
    /// Área escolhida (na tela).
    selection: Option<(f32, f32, f32, f32)>,
    /// Arrasto em andamento: começo e se é uma área nova (senão, uma marcação).
    drag: Option<((f32, f32), bool)>,
    shapes: Vec<Shape>,
    /// Janelas à mostra (a de cima primeiro), para o modo Janela.
    windows: Vec<(f32, f32, f32, f32)>,
    /// Retângulo das cores no painel (para o clique nelas).
    swatches: Vec<(f32, f32, f32, f32)>,
    /// Modo rápido (⌘⇧4, `--area`): sem painel; ao soltar o mouse, a área é salva e
    /// copiada, como no Mac.
    quick: bool,
}

impl Capture {
    fn new(shot: Pixmap, windows: Vec<(f32, f32, f32, f32)>) -> Self {
        let mut dimmed = shot.clone();
        for px in dimmed.data_mut().chunks_mut(4) {
            for c in &mut px[..3] {
                *c = (*c as u16 * 145 / 255) as u8;
            }
        }
        Self { shot, dimmed, mode: Mode::Selection, tool: Tool::Arrow, color: 0, selection: None, drag: None, shapes: Vec::new(), windows, swatches: Vec::new(), quick: false }
    }

    /// A área escolhida com as marcações, recortada da captura.
    fn result(&self) -> Option<Pixmap> {
        let (x, y, w, h) = self.selection?;
        let (w, h) = (w.round().max(1.0) as u32, h.round().max(1.0) as u32);
        let mut out = Pixmap::new(w, h)?;
        out.draw_pixmap(-(x.round() as i32), -(y.round() as i32), self.shot.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
        for shape in &self.shapes {
            draw_shape(&mut out, shape, (-x.round(), -y.round()));
        }
        Some(out)
    }

    fn copy(&self) -> bool {
        let Some(image) = self.result() else {
            return false;
        };
        let path = runtime_dir().join(format!("mide-captura-{}.png", std::process::id()));
        if let Err(err) = image.save_png(&path) {
            tracing::warn!("captura: não foi possível preparar a cópia: {err}");
            mide::notify::send_and_wait("Captura de tela", "A captura não foi copiada", &err.to_string());
            return false;
        }
        let ok = ask_compositor(&format!("copy-file image/png {}", path.display())).is_some_and(|r| r.first().map(String::as_str) == Some("ok"));
        let _ = std::fs::remove_file(&path);
        if ok {
            tracing::info!("captura copiada ({}x{})", image.width(), image.height());
            mide::notify::send_and_wait("Captura de tela", "Captura copiada", "Cole com Ctrl+V (⌘V no estilo Mac).");
        } else {
            mide::notify::send_and_wait("Captura de tela", "A captura não foi copiada", "O mide não respondeu.");
        }
        ok
    }

    /// Grava em Imagens/Capturas de tela, com o nome "Captura de tela" mais a data e a
    /// hora (`Captura de tela 2026-10-20 21-13-38.png`). Avisa onde ficou (ou o erro).
    fn save(&self) -> Option<PathBuf> {
        let image = self.result()?;
        let dir = mide::places::pictures_dir().join("Capturas de tela");
        let path = dir.join(format!("Captura de tela {}.png", chrono_name()));
        let result = std::fs::create_dir_all(&dir).map_err(|e| e.to_string()).and_then(|()| image.save_png(&path).map_err(|e| e.to_string()));
        match result {
            Ok(()) => {
                tracing::info!("captura salva: {}", path.display());
                mide::notify::send_and_wait("Captura de tela", "Captura salva", &path.display().to_string());
                Some(path)
            }
            Err(err) => {
                tracing::warn!("captura não salva em {}: {err}", dir.display());
                mide::notify::send_and_wait("Captura de tela", "A captura não foi salva", &format!("{}: {err}", dir.display()));
                None
            }
        }
    }

    /// Clique e arrasto na tela (fora do painel).
    fn pointer(&mut self, ui: &Ui, pos: (f32, f32), over_panel: bool) {
        let input = ui.input();
        if over_panel && self.drag.is_none() {
            return;
        }
        match self.mode {
            Mode::Window => {
                if input.released {
                    if let Some(rect) = self.windows.iter().find(|r| inside(**r, pos)) {
                        self.selection = Some(clamp_rect(*rect, (self.shot.width() as f32, self.shot.height() as f32)));
                        self.mode = Mode::Selection;
                        self.shapes.clear();
                    }
                }
            }
            Mode::Full | Mode::Selection => {
                if input.pressed {
                    // Dentro da área escolhida: desenha. Fora: escolhe outra área.
                    let new_area = self.selection.is_none_or(|s| !inside(s, pos));
                    if new_area {
                        self.shapes.clear();
                        self.mode = Mode::Selection;
                    }
                    self.drag = Some((pos, new_area));
                }
                if let Some((start, new_area)) = self.drag {
                    if new_area {
                        let r = normalized(start, pos);
                        self.selection = (r.2 >= 2.0 && r.3 >= 2.0).then_some(r);
                    }
                    if input.released {
                        self.drag = None;
                        if !new_area && (pos.0 - start.0).hypot(pos.1 - start.1) >= 4.0 {
                            self.shapes.push(Shape { tool: self.tool, color: COLORS[self.color].1, from: start, to: pos });
                        }
                    }
                }
            }
        }
    }
}

fn clamp_rect(r: (f32, f32, f32, f32), screen: (f32, f32)) -> (f32, f32, f32, f32) {
    let x = r.0.max(0.0);
    let y = r.1.max(0.0);
    let right = (r.0 + r.2).min(screen.0);
    let bottom = (r.1 + r.3).min(screen.1);
    (x, y, (right - x).max(1.0), (bottom - y).max(1.0))
}

impl App for Capture {
    fn view(&mut self, ui: &mut Ui) {
        let (sw, sh) = (self.shot.width() as f32, self.shot.height() as f32);
        if self.mode == Mode::Full {
            self.selection = Some((0.0, 0.0, sw, sh));
        }
        // Teclado: Esc cancela, Enter e Ctrl+C copiam, Ctrl+S salva.
        let keys = ui.input().keys.clone();
        let shortcuts = ui.input().shortcuts.clone();
        if keys.contains(&Key::Escape) {
            tracing::info!("captura cancelada");
            ui.close();
            return;
        }
        if (keys.contains(&Key::Enter) || shortcuts.iter().any(|s| s == "c")) && self.copy() {
            ui.close();
            return;
        }
        if shortcuts.iter().any(|s| s == "s") && self.save().is_some() {
            ui.close();
            return;
        }

        // A captura congelada, escurecida fora da área escolhida.
        let selection = self.selection;
        let hovered_window = (self.mode == Mode::Window)
            .then(|| ui.input().pointer.and_then(|p| self.windows.iter().find(|r| inside(**r, p)).copied()))
            .flatten();
        let drawing = self.drag.filter(|(_, new_area)| !*new_area).and_then(|(start, _)| {
            ui.input().pointer.map(|p| Shape { tool: self.tool, color: COLORS[self.color].1, from: start, to: p })
        });
        let (shot, dimmed) = (&self.shot, &self.dimmed);
        let shapes = &self.shapes;
        ui.paint(|canvas, _| {
            // Tela escurecida e, por cima, a área escolhida com as cores de verdade:
            // só cópia de bytes (rápido mesmo numa tela grande).
            if canvas.data().len() == dimmed.data().len() {
                canvas.data_mut().copy_from_slice(dimmed.data());
            }
            if let Some((x, y, w, h)) = selection.or(hovered_window) {
                copy_rect(canvas, shot, (x, y, w, h));
                outline(canvas, (x, y, w, h));
            }
            for shape in shapes.iter().chain(drawing.iter()) {
                draw_shape(canvas, shape, (0.0, 0.0));
            }
        });

        // Tamanho da área escolhida, logo acima dela (ou dentro, se não couber).
        if let Some((x, y, w, h)) = self.selection.filter(|_| self.mode != Mode::Full) {
            let text = format!("{} × {}", w.round() as i32, h.round() as i32);
            let ly = if y >= 44.0 { y - 40.0 } else { y + 6.0 };
            ui.glass_panel(x, ly, 120.0, 8.0, |ui| ui.label(&text));
        }

        // Modo rápido: só a seleção; ao soltar, salva e copia (como o ⌘⇧4 do Mac).
        if self.quick {
            if let Some(pos) = ui.input().pointer {
                let releasing = ui.input().released && self.drag.is_some();
                self.pointer(ui, pos, false);
                if releasing && self.selection.is_some() {
                    tracing::info!("captura rápida de uma área");
                    let copied = self.copy();
                    if self.save().is_some() || copied {
                        ui.close();
                    }
                }
            }
            if self.drag.is_some() {
                ui.request_repaint();
            }
            return;
        }

        // Painel: modo, ferramenta, cor e ações, embaixo e no meio.
        let panel_w = 720.0;
        let (px, py) = ((sw - panel_w) / 2.0, sh - 128.0);
        let mut action = None;
        let rect = ui.glass_panel(px, py, panel_w, 14.0, |ui| {
            ui.row(|ui| {
                for (mode, label) in [(Mode::Selection, "Seleção"), (Mode::Window, "Janela"), (Mode::Full, "Tela inteira")] {
                    let r = if self.mode == mode { ui.primary_button(label) } else { ui.button(label) };
                    if r.clicked() {
                        self.mode = mode;
                        if mode != Mode::Full {
                            self.selection = None;
                        }
                        self.shapes.clear();
                    }
                }
                ui.push_right(268.0);
                if ui.button("Cancelar").clicked() {
                    action = Some("cancelar");
                }
                if ui.button("Salvar").clicked() {
                    action = Some("salvar");
                }
                if ui.primary_button("Copiar").clicked() {
                    action = Some("copiar");
                }
            });
            ui.row(|ui| {
                for (tool, label) in [(Tool::Arrow, "Seta"), (Tool::Circle, "Círculo"), (Tool::Square, "Quadrado")] {
                    let r = if self.tool == tool { ui.primary_button(label) } else { ui.button(label) };
                    if r.clicked() {
                        self.tool = tool;
                    }
                }
            });
        });
        // Cores: círculos à direita das ferramentas, na segunda linha do painel.
        let row_y = rect.1 + rect.3 - 30.0;
        self.swatches = (0..COLORS.len()).map(|i| (rect.0 + rect.2 - 14.0 - (COLORS.len() - i) as f32 * 30.0, row_y, 24.0, 24.0)).collect();
        let swatches = self.swatches.clone();
        let chosen = self.color;
        ui.paint(|canvas, _| {
            for (i, (x, y, w, _)) in swatches.iter().enumerate() {
                let [r, g, b] = COLORS[i].1;
                let center = (x + w / 2.0, y + w / 2.0);
                if let Some(path) = PathBuilder::from_circle(center.0, center.1, w / 2.0 - 2.0) {
                    let mut paint = Paint::default();
                    paint.anti_alias = true;
                    paint.set_color(Color::from_rgba8(r, g, b, 255));
                    canvas.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
                    paint.set_color(Color::from_rgba8(0, 0, 0, 70));
                    canvas.stroke_path(&path, &paint, &Stroke { width: 1.0, ..Default::default() }, Transform::identity(), None);
                }
                if i == chosen {
                    if let Some(ring) = PathBuilder::from_circle(center.0, center.1, w / 2.0 + 1.5) {
                        let mut paint = Paint::default();
                        paint.anti_alias = true;
                        paint.set_color(Color::from_rgba8(10, 132, 255, 255));
                        canvas.stroke_path(&ring, &paint, &Stroke { width: 2.0, ..Default::default() }, Transform::identity(), None);
                    }
                }
            }
        });
        let pointer = ui.input().pointer;
        if let Some(pos) = pointer {
            let over_panel = inside(rect, pos);
            if over_panel && ui.input().released {
                if let Some(i) = self.swatches.iter().position(|s| inside(*s, pos)) {
                    self.color = i;
                    tracing::info!("captura: cor {}", COLORS[i].0);
                }
            }
            self.pointer(ui, pos, over_panel);
        }
        match action {
            Some("cancelar") => ui.close(),
            Some("salvar") if self.save().is_some() => ui.close(),
            Some("copiar") if self.copy() => ui.close(),
            _ => {}
        }
        if self.drag.is_some() {
            ui.request_repaint();
        }
    }
}

/// Copia o retângulo `r` de `src` para o mesmo lugar em `dst` (mesmo tamanho), linha
/// a linha.
fn copy_rect(dst: &mut Pixmap, src: &Pixmap, r: (f32, f32, f32, f32)) {
    let (w, h) = (src.width() as i64, src.height() as i64);
    if dst.width() as i64 != w || dst.height() as i64 != h {
        return;
    }
    let x0 = (r.0.floor() as i64).clamp(0, w);
    let x1 = ((r.0 + r.2).ceil() as i64).clamp(0, w);
    let y0 = (r.1.floor() as i64).clamp(0, h);
    let y1 = ((r.1 + r.3).ceil() as i64).clamp(0, h);
    if x1 <= x0 {
        return;
    }
    let stride = w as usize * 4;
    let (a, b) = (x0 as usize * 4, x1 as usize * 4);
    let src = src.data();
    let dst = dst.data_mut();
    for y in y0 as usize..y1 as usize {
        let row = y * stride;
        dst[row + a..row + b].copy_from_slice(&src[row + a..row + b]);
    }
}

/// Contorno branco da área escolhida.
fn outline(canvas: &mut Pixmap, (x, y, w, h): (f32, f32, f32, f32)) {
    if let Some(rect) = Rect::from_xywh(x, y, w, h) {
        let path = PathBuilder::from_rect(rect);
        let mut paint = Paint::default();
        paint.set_color(Color::from_rgba8(255, 255, 255, 230));
        canvas.stroke_path(&path, &paint, &Stroke { width: 1.5, ..Default::default() }, Transform::identity(), None);
    }
}

/// Desenha a seta, o círculo ou o quadrado, deslocado por `offset`.
fn draw_shape(canvas: &mut Pixmap, shape: &Shape, offset: (f32, f32)) {
    let a = (shape.from.0 + offset.0, shape.from.1 + offset.1);
    let b = (shape.to.0 + offset.0, shape.to.1 + offset.1);
    let [r, g, bl] = shape.color;
    let mut paint = Paint::default();
    paint.anti_alias = true;
    paint.set_color(Color::from_rgba8(r, g, bl, 255));
    let stroke = Stroke { width: 4.0, line_cap: LineCap::Round, ..Default::default() };
    match shape.tool {
        Tool::Arrow => {
            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
            let len = dx.hypot(dy).max(1.0);
            let (ux, uy) = (dx / len, dy / len);
            let head = 18.0f32.min(len * 0.6);
            // Haste até a base da ponta.
            let base = (b.0 - ux * head * 0.8, b.1 - uy * head * 0.8);
            let mut pb = PathBuilder::new();
            pb.move_to(a.0, a.1);
            pb.line_to(base.0, base.1);
            if let Some(path) = pb.finish() {
                canvas.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
            }
            // Ponta: triângulo cheio.
            let (px, py) = (-uy, ux);
            let mut pb = PathBuilder::new();
            pb.move_to(b.0, b.1);
            pb.line_to(b.0 - ux * head + px * head * 0.55, b.1 - uy * head + py * head * 0.55);
            pb.line_to(b.0 - ux * head - px * head * 0.55, b.1 - uy * head - py * head * 0.55);
            pb.close();
            if let Some(path) = pb.finish() {
                canvas.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
            }
        }
        Tool::Circle => {
            let (x, y, w, h) = normalized(a, b);
            if let Some(rect) = Rect::from_xywh(x, y, w.max(1.0), h.max(1.0)) {
                if let Some(path) = PathBuilder::from_oval(rect) {
                    canvas.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
                }
            }
        }
        Tool::Square => {
            let (x, y, w, h) = normalized(a, b);
            if let Some(rect) = Rect::from_xywh(x, y, w.max(1.0), h.max(1.0)) {
                let path = PathBuilder::from_rect(rect);
                let stroke = Stroke { width: 4.0, line_join: tiny_skia::LineJoin::Round, ..Default::default() };
                canvas.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
            }
        }
    }
}

fn runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir)
}

/// Data e hora para o nome do arquivo (`2026-10-19 21-40-05`), pelo relógio local.
fn chrono_name() -> String {
    let out = std::process::Command::new("date").arg("+%Y-%m-%d %H-%M-%S").output().ok();
    out.and_then(|o| String::from_utf8(o.stdout).ok()).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| {
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        secs.to_string()
    })
}

/// Pede a captura ao compositor e espera o arquivo (até 3 s). `wait_launcher`: aberta
/// pelo launcher (o app Captura de Tela), espera ele fechar, senão a captura sai com
/// o launcher por cima; pelos atalhos (⌘⇧3, ⌘⇧4), a tela sai como está.
fn capture(wait_launcher: bool) -> Result<Pixmap, Box<dyn Error>> {
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    while wait_launcher && ask_compositor("launcher state").is_some_and(|r| r.first().map(String::as_str) == Some("open")) {
        if std::time::Instant::now() > deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    // O último quadro do esmaecer e os menus que fecham junto.
    std::thread::sleep(Duration::from_millis(60));
    let path = runtime_dir().join(format!("mide-tela-{}.png", std::process::id()));
    let _ = std::fs::remove_file(&path);
    ask_compositor(&format!("screenshot {}", path.display())).ok_or("o mide não respondeu")?;
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while !path.exists() {
        if std::time::Instant::now() > deadline {
            return Err("a captura não chegou".into());
        }
        std::thread::sleep(Duration::from_millis(15));
    }
    let image = image::open(&path)?.to_rgba8();
    let _ = std::fs::remove_file(&path);
    let (w, h) = image.dimensions();
    // Opaca: o RGBA já serve como pré-multiplicado.
    Pixmap::from_vec(image.into_raw(), tiny_skia::IntSize::from_wh(w, h).ok_or("tamanho inválido")?).ok_or_else(|| "imagem inválida".into())
}

/// Janelas à mostra, a de cima primeiro.
fn window_rects() -> Vec<(f32, f32, f32, f32)> {
    ask_compositor("window_rects")
        .unwrap_or_default()
        .iter()
        .filter_map(|line| {
            let f: Vec<&str> = line.split('\t').collect();
            Some((f.get(1)?.parse().ok()?, f.get(2)?.parse().ok()?, f.get(3)?.parse().ok()?, f.get(4)?.parse().ok()?))
        })
        .collect()
}

fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "mide_screenshot=info".into()))
        .init();
    // `--full` (⌘⇧3): a tela inteira, salva e copiada, sem janela. `--area` (⌘⇧4):
    // escolher uma área, salva e copiada ao soltar o mouse.
    let mode = std::env::args().nth(1).unwrap_or_default();
    let shot = capture(mode.is_empty())?;
    if mode == "--full" {
        let (w, h) = (shot.width() as f32, shot.height() as f32);
        let mut full = Capture::new(shot, Vec::new());
        full.selection = Some((0.0, 0.0, w, h));
        tracing::info!("captura rápida da tela inteira ({w}x{h})");
        let copied = full.copy();
        return if full.save().is_some() || copied { Ok(()) } else { Err("a captura não foi salva nem copiada".into()) };
    }
    tracing::info!("captura de tela aberta ({}x{})", shot.width(), shot.height());
    let windows = window_rects();
    let mut app = Capture::new(shot, windows);
    app.quick = mode == "--area";
    // A imagem capturada é pintada pixel a pixel (`ui.paint`): escala 1.
    run(Window::new("Captura de tela").app_id("mide-screenshot").overlay("mide-screenshot").fixed_scale(), app)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recorta_a_area_com_as_marcacoes() {
        let mut shot = Pixmap::new(200, 100).unwrap();
        shot.fill(Color::from_rgba8(255, 255, 255, 255));
        let mut capture = Capture::new(shot, vec![(10.0, 10.0, 50.0, 40.0)]);
        capture.selection = Some((20.0, 10.0, 100.0, 60.0));
        capture.shapes.push(Shape { tool: Tool::Square, color: [235, 52, 52], from: (30.0, 20.0), to: (60.0, 50.0) });
        let out = capture.result().unwrap();
        assert_eq!((out.width(), out.height()), (100, 60));
        // A borda do quadrado vermelho cai em (10, 10) no recorte.
        let px = out.pixel(10, 20).unwrap();
        assert!(px.red() > 200 && px.green() < 100, "quadrado vermelho no recorte");
        // Fora das marcações, o branco da captura.
        assert_eq!(out.pixel(80, 50).unwrap().green(), 255);
    }

    #[test]
    fn janela_fora_da_tela_e_recortada() {
        assert_eq!(clamp_rect((-10.0, 20.0, 100.0, 50.0), (80.0, 60.0)), (0.0, 20.0, 80.0, 40.0));
    }
}
