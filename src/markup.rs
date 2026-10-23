//! Marcações sobre imagens (a Captura e o Editor de Capturas): as cores, as setas,
//! círculos, quadrados e o desenho livre, e a área ajustável com bolinhas (recorte,
//! área da captura). Tudo em coordenadas livres: quem chama dá a transformação para o
//! canvas.

use tiny_skia::{Color, FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Rect, Stroke, Transform};

/// Cores para desenhar: nome e RGB.
pub const COLORS: [(&str, [u8; 3]); 6] = [
    ("vermelho", [235, 52, 52]),
    ("branco", [255, 255, 255]),
    ("preto", [20, 20, 22]),
    ("verde", [52, 199, 89]),
    ("amarelo", [255, 204, 0]),
    ("azul", [10, 132, 255]),
];

/// Retângulo (x, y, largura, altura).
pub type RectF = (f32, f32, f32, f32);

/// Bolinha da área escolhida: as 4 pontas e o meio dos 4 lados.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handle {
    TopLeft,
    TopRight,
    BottomRight,
    BottomLeft,
    Top,
    Right,
    Bottom,
    Left,
}

impl Handle {
    /// As pontas antes dos meios dos lados: numa área pequena, onde as duas ficam a
    /// menos de um raio, a ponta vence (muda largura e altura juntas).
    pub const ALL: [Handle; 8] = [Handle::TopLeft, Handle::TopRight, Handle::BottomRight, Handle::BottomLeft, Handle::Top, Handle::Right, Handle::Bottom, Handle::Left];

    /// Que lados a bolinha mexe: x (−1 esquerdo, 1 direito, 0 nenhum) e y (−1 de cima,
    /// 1 de baixo, 0 nenhum).
    pub fn sides(self) -> (i8, i8) {
        match self {
            Handle::TopLeft => (-1, -1),
            Handle::TopRight => (1, -1),
            Handle::BottomRight => (1, 1),
            Handle::BottomLeft => (-1, 1),
            Handle::Top => (0, -1),
            Handle::Right => (1, 0),
            Handle::Bottom => (0, 1),
            Handle::Left => (-1, 0),
        }
    }
}

/// Raio da bolinha desenhada e da área onde o clique pega nela (pontos).
pub const HANDLE_RADIUS: f32 = 5.0;
pub const HANDLE_HIT: f32 = 10.0;

/// Centro de cada bolinha da área `r`.
pub fn handles(r: RectF) -> [(Handle, (f32, f32)); 8] {
    Handle::ALL.map(|h| {
        let (sx, sy) = h.sides();
        let x = r.0 + r.2 * (sx as f32 + 1.0) / 2.0;
        let y = r.1 + r.3 * (sy as f32 + 1.0) / 2.0;
        (h, (x, y))
    })
}

/// A bolinha sob o ponto `p` (até `HANDLE_HIT` do centro), se houver.
pub fn handle_at(r: RectF, p: (f32, f32)) -> Option<Handle> {
    handles(r).into_iter().find(|(_, c)| (p.0 - c.0).hypot(p.1 - c.1) <= HANDLE_HIT).map(|(h, _)| h)
}

/// A área `orig` com a bolinha `h` arrastada de `delta`: o lado oposto fica parado, o
/// lado mexido não sai de `limits` (todas as telas), passar do outro lado vira a área
/// (normaliza) e o tamanho mínimo é 2 pontos.
pub fn resize_area(orig: RectF, h: Handle, delta: (f32, f32), limits: RectF) -> RectF {
    let (lx0, ly0, lx1, ly1) = (limits.0, limits.1, limits.0 + limits.2, limits.1 + limits.3);
    let (mut x0, mut y0, mut x1, mut y1) = (orig.0, orig.1, orig.0 + orig.2, orig.1 + orig.3);
    let (sx, sy) = h.sides();
    match sx {
        -1 => x0 = (x0 + delta.0).clamp(lx0, lx1),
        1 => x1 = (x1 + delta.0).clamp(lx0, lx1),
        _ => {}
    }
    match sy {
        -1 => y0 = (y0 + delta.1).clamp(ly0, ly1),
        1 => y1 = (y1 + delta.1).clamp(ly0, ly1),
        _ => {}
    }
    let fit = |a: f32, b: f32, lo: f32, hi: f32| {
        let (start, len) = (a.min(b), (a - b).abs());
        if len >= 2.0 { (start, len) } else { (start.min(hi - 2.0).max(lo), 2.0) }
    };
    let (x, w) = fit(x0, x1, lx0, lx1);
    let (y, h) = fit(y0, y1, ly0, ly1);
    (x, y, w, h)
}

/// A área `orig` arrastada de `delta`, sem sair de `limits` (o tamanho não muda).
pub fn moved(orig: RectF, delta: (f32, f32), limits: RectF) -> RectF {
    let x = (orig.0 + delta.0).min(limits.0 + limits.2 - orig.2).max(limits.0);
    let y = (orig.1 + delta.1).min(limits.1 + limits.3 - orig.3).max(limits.1);
    (x, y, orig.2, orig.3)
}

/// O ponto `p` preso dentro de `limits`.
pub fn clamp_point(p: (f32, f32), limits: RectF) -> (f32, f32) {
    (p.0.clamp(limits.0, limits.0 + limits.2), p.1.clamp(limits.1, limits.1 + limits.3))
}

/// Retângulo (x, y, largura, altura) com largura e altura positivas.
pub fn normalized(a: (f32, f32), b: (f32, f32)) -> (f32, f32, f32, f32) {
    (a.0.min(b.0), a.1.min(b.1), (a.0 - b.0).abs(), (a.1 - b.1).abs())
}

pub fn inside(r: (f32, f32, f32, f32), p: (f32, f32)) -> bool {
    p.0 >= r.0 && p.0 <= r.0 + r.2 && p.1 >= r.1 && p.1 <= r.1 + r.3
}

/// Contorno branco da área escolhida.
pub fn outline(canvas: &mut Pixmap, r: RectF) {
    outline_with(canvas, r, Transform::identity());
}

/// Como `outline`, com `ts` levando de `r` ao canvas (o traço engrossa junto).
pub fn outline_with(canvas: &mut Pixmap, (x, y, w, h): RectF, ts: Transform) {
    if let Some(rect) = Rect::from_xywh(x, y, w, h) {
        let path = PathBuilder::from_rect(rect);
        let mut paint = Paint::default();
        paint.set_color(Color::from_rgba8(255, 255, 255, 230));
        canvas.stroke_path(&path, &paint, &Stroke { width: 1.5, ..Default::default() }, ts, None);
    }
}

/// Bolinhas da área `r` (no canvas): círculo na cor de destaque com um anel branco.
pub fn draw_handles(canvas: &mut Pixmap, r: RectF, accent: [u8; 4]) {
    draw_handles_with(canvas, r, accent, Transform::identity());
}

/// Como `draw_handles`, com `ts` levando de `r` ao canvas (as bolinhas crescem junto).
pub fn draw_handles_with(canvas: &mut Pixmap, r: RectF, accent: [u8; 4], ts: Transform) {
    let solid = |c: Color| Paint { shader: tiny_skia::Shader::SolidColor(c), anti_alias: true, ..Default::default() };
    let fill = solid(Color::from_rgba8(accent[0], accent[1], accent[2], 255));
    let ring = solid(Color::WHITE);
    for (_, (x, y)) in handles(r) {
        if let Some(path) = PathBuilder::from_circle(x, y, HANDLE_RADIUS) {
            canvas.fill_path(&path, &fill, FillRule::Winding, ts, None);
            canvas.stroke_path(&path, &ring, &Stroke { width: 1.5, ..Default::default() }, ts, None);
        }
    }
}

/// Tipo de marcação.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkKind {
    Arrow,
    Circle,
    Square,
    /// Desenho livre: os pontos por onde o ponteiro passou.
    Pen,
}

/// Desenha a marcação `kind` na cor `rgb` pelos pontos `points` (seta, círculo e
/// quadrado usam o primeiro e o último; o desenho livre, todos), com o traço de
/// `width`. `ts` leva das coordenadas dos pontos ao canvas; numa ampliação, o traço
/// engrossa junto.
pub fn draw_mark(canvas: &mut Pixmap, kind: MarkKind, rgb: [u8; 3], points: &[(f32, f32)], width: f32, ts: Transform) {
    let (Some(&a), Some(&b)) = (points.first(), points.last()) else {
        return;
    };
    let [r, g, bl] = rgb;
    let mut paint = Paint { anti_alias: true, ..Default::default() };
    paint.set_color(Color::from_rgba8(r, g, bl, 255));
    let stroke = Stroke { width, line_cap: LineCap::Round, line_join: LineJoin::Round, ..Default::default() };
    match kind {
        MarkKind::Arrow => {
            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
            let len = dx.hypot(dy).max(1.0);
            let (ux, uy) = (dx / len, dy / len);
            let head = (4.5 * width).min(len * 0.6);
            // Haste até a base da ponta.
            let base = (b.0 - ux * head * 0.8, b.1 - uy * head * 0.8);
            let mut pb = PathBuilder::new();
            pb.move_to(a.0, a.1);
            pb.line_to(base.0, base.1);
            if let Some(path) = pb.finish() {
                canvas.stroke_path(&path, &paint, &stroke, ts, None);
            }
            // Ponta: triângulo cheio.
            let (px, py) = (-uy, ux);
            let mut pb = PathBuilder::new();
            pb.move_to(b.0, b.1);
            pb.line_to(b.0 - ux * head + px * head * 0.55, b.1 - uy * head + py * head * 0.55);
            pb.line_to(b.0 - ux * head - px * head * 0.55, b.1 - uy * head - py * head * 0.55);
            pb.close();
            if let Some(path) = pb.finish() {
                canvas.fill_path(&path, &paint, FillRule::Winding, ts, None);
            }
        }
        MarkKind::Circle => {
            let (x, y, w, h) = normalized(a, b);
            if let Some(path) = Rect::from_xywh(x, y, w.max(1.0), h.max(1.0)).and_then(PathBuilder::from_oval) {
                canvas.stroke_path(&path, &paint, &stroke, ts, None);
            }
        }
        MarkKind::Square => {
            let (x, y, w, h) = normalized(a, b);
            if let Some(rect) = Rect::from_xywh(x, y, w.max(1.0), h.max(1.0)) {
                canvas.stroke_path(&PathBuilder::from_rect(rect), &paint, &stroke, ts, None);
            }
        }
        MarkKind::Pen => {
            let mut pb = PathBuilder::new();
            pb.move_to(a.0, a.1);
            if points.len() == 1 {
                // Um toque só: um ponto (traço de comprimento quase zero, com ponta redonda).
                pb.line_to(a.0 + 0.01, a.1);
            }
            for p in &points[1..] {
                pb.line_to(p.0, p.1);
            }
            if let Some(path) = pb.finish() {
                canvas.stroke_path(&path, &paint, &stroke, ts, None);
            }
        }
    }
}

/// Arquivo com o PID da miniatura da captura à mostra (B23), um por sessão do mide (o
/// socket entra no nome: os testes em paralelo não se misturam). Quem grava outro PID
/// nele manda a miniatura sair na hora (uma captura nova começou).
pub fn thumbnail_pid_file() -> std::path::PathBuf {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(std::path::PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let socket = std::env::var("MIDE_SOCKET").or_else(|_| std::env::var("WAYLAND_DISPLAY")).unwrap_or_else(|_| "mide-0".into());
    let socket: String = socket.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect();
    runtime.join(format!("mide-miniatura-{socket}.pid"))
}

/// O processo `pid` é uma miniatura da captura (`mide-screenshot-editor --miniatura`)?
/// Confere a linha de comando, não o nome: a Captura e o editor têm o mesmo começo de
/// nome, e um PID velho pode ter sido reaproveitado por eles.
pub fn is_thumbnail_process(pid: u32) -> bool {
    std::fs::read(format!("/proc/{pid}/cmdline")).is_ok_and(|c| is_thumbnail_cmdline(&c))
}

/// A linha de comando (argumentos separados por NUL, como em `/proc/PID/cmdline`) é a
/// de uma miniatura?
pub fn is_thumbnail_cmdline(cmdline: &[u8]) -> bool {
    let mut args = cmdline.split(|b| *b == 0);
    let program = args.next().unwrap_or_default();
    program.ends_with(b"mide-screenshot-editor") && args.next() == Some(b"--miniatura".as_slice())
}

/// Um programa do mide: o da mesma pasta que este (o build de onde se roda), senão o
/// do `PATH`.
pub fn sibling_program(name: &str) -> std::path::PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|d| d.join(name)))
        .filter(|p| p.exists())
        .unwrap_or_else(|| std::path::PathBuf::from(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconhece_a_linha_de_comando_da_miniatura() {
        assert!(is_thumbnail_cmdline(b"/usr/bin/mide-screenshot-editor\0--miniatura\0/tmp/a.png\0"));
        assert!(is_thumbnail_cmdline(b"mide-screenshot-editor\0--miniatura\0x.png"));
        // O editor aberto com um arquivo, a Captura e outro programa não são.
        assert!(!is_thumbnail_cmdline(b"/usr/bin/mide-screenshot-editor\0/tmp/--miniatura.png\0"));
        assert!(!is_thumbnail_cmdline(b"/usr/bin/mide-screenshot\0--area\0"));
        assert!(!is_thumbnail_cmdline(b"/usr/bin/mide-screenshot\0--miniatura\0"));
        assert!(!is_thumbnail_cmdline(b""));
        // O próprio processo dos testes não é uma miniatura.
        assert!(!is_thumbnail_process(std::process::id()));
    }
}
