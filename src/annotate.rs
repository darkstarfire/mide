//! Formas editáveis do Editor de Capturas (como a Marcação do Mac): cada forma fica
//! por cima da imagem, pode ser escolhida, movida, redimensionada e ter as cores e o
//! traço trocados; só entra nos pixels ao salvar ou copiar.
//!
//! Também o reconhecimento do "Esboço": um traço feito à mão vira linha, seta,
//! retângulo, triângulo ou oval quando parece um deles.
//!
//! Coordenadas livres (o editor usa pixels da imagem); quem desenha dá a transformação.

use tiny_skia::{Color, FillRule, LineCap, LineJoin, Paint, Path, PathBuilder, Pixmap, Rect, Stroke, StrokeDash, Transform};

use crate::markup::{Handle, RectF};

/// Tipo de forma.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShapeKind {
    /// Desenho livre (todos os pontos).
    Pen,
    /// Linha e seta: do primeiro ao segundo ponto.
    Line,
    Arrow,
    /// Formas na caixa entre os dois pontos.
    Rect,
    RoundRect,
    Oval,
    Triangle,
    Star,
    /// Balão de fala (com o bico embaixo, à esquerda).
    Bubble,
    /// Caixa de texto (o texto começa no primeiro ponto).
    Text,
}

impl ShapeKind {
    /// Formas que vêm de uma caixa (dois cantos) e ganham as oito alças.
    pub fn boxed(self) -> bool {
        matches!(self, ShapeKind::Rect | ShapeKind::RoundRect | ShapeKind::Oval | ShapeKind::Triangle | ShapeKind::Star | ShapeKind::Bubble)
    }
}

/// Uma forma: tipo, pontos, cor da borda, preenchimento, espessura e estilo do traço.
#[derive(Debug, Clone, PartialEq)]
pub struct Shape {
    pub kind: ShapeKind,
    pub points: Vec<(f32, f32)>,
    pub stroke: [u8; 3],
    pub fill: Option<[u8; 3]>,
    pub width: f32,
    pub dashed: bool,
    pub text: String,
    pub font_size: f32,
}

impl Shape {
    pub fn new(kind: ShapeKind, points: Vec<(f32, f32)>, stroke: [u8; 3], width: f32) -> Self {
        Self { kind, points, stroke, fill: None, width, dashed: false, text: String::new(), font_size: 24.0 }
    }

    /// Caixa que contém a forma (sem a espessura do traço).
    pub fn bounds(&self, text: &mut crate::ui::text::Text) -> RectF {
        if self.kind == ShapeKind::Text {
            let (x, y) = self.points.first().copied().unwrap_or_default();
            let lines: Vec<&str> = if self.text.is_empty() { vec![" "] } else { self.text.split('\n').collect() };
            let w = lines.iter().map(|l| text.measure(l, self.font_size, crate::ui::text::FontWeight::Regular)).fold(10.0f32, f32::max);
            return (x, y, w, self.font_size * 1.3 * lines.len() as f32);
        }
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for (x, y) in &self.points {
            x0 = x0.min(*x);
            y0 = y0.min(*y);
            x1 = x1.max(*x);
            y1 = y1.max(*y);
        }
        if x0 > x1 {
            return (0.0, 0.0, 0.0, 0.0);
        }
        (x0, y0, x1 - x0, y1 - y0)
    }

    /// O ponto `p` está na forma (no traço, até `tol`; dentro, se preenchida, ou dentro
    /// da caixa das formas fechadas e do texto)?
    pub fn hit(&self, p: (f32, f32), tol: f32, text: &mut crate::ui::text::Text) -> bool {
        let tol = tol + self.width / 2.0;
        match self.kind {
            ShapeKind::Pen => self.points.windows(2).any(|w| segment_distance(p, w[0], w[1]) <= tol) || self.points.first().is_some_and(|a| (p.0 - a.0).hypot(p.1 - a.1) <= tol),
            ShapeKind::Line | ShapeKind::Arrow => match (self.points.first(), self.points.last()) {
                (Some(a), Some(b)) => segment_distance(p, *a, *b) <= tol,
                _ => false,
            },
            _ => {
                let b = self.bounds(text);
                p.0 >= b.0 - tol && p.0 <= b.0 + b.2 + tol && p.1 >= b.1 - tol && p.1 <= b.1 + b.3 + tol
            }
        }
    }

    /// Anda a forma de `d`.
    pub fn translate(&mut self, d: (f32, f32)) {
        for p in &mut self.points {
            p.0 += d.0;
            p.1 += d.1;
        }
    }

    /// Leva a forma da caixa `from` para a caixa `to` (alças de redimensionar).
    pub fn fit_to(&mut self, from: RectF, to: RectF) {
        let sx = if from.2.abs() > 0.01 { to.2 / from.2 } else { 1.0 };
        let sy = if from.3.abs() > 0.01 { to.3 / from.3 } else { 1.0 };
        for p in &mut self.points {
            p.0 = to.0 + (p.0 - from.0) * sx;
            p.1 = to.1 + (p.1 - from.1) * sy;
        }
        if self.kind == ShapeKind::Text {
            self.font_size = (self.font_size * sy.abs()).clamp(8.0, 400.0);
        }
    }

    /// Gira a forma 90° (`clockwise`) numa imagem de largura `w` e altura `h`.
    pub fn rotate(&mut self, clockwise: bool, w: f32, h: f32) {
        for p in &mut self.points {
            *p = if clockwise { (h - p.1, p.0) } else { (p.1, w - p.0) };
        }
    }
}

/// Distância do ponto `p` ao segmento `a`–`b`.
pub fn segment_distance(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 { (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
    (p.0 - (a.0 + t * dx)).hypot(p.1 - (a.1 + t * dy))
}

/// Caixa com largura e altura positivas a partir de dois pontos.
fn boxed(points: &[(f32, f32)]) -> Option<RectF> {
    let (a, b) = (points.first()?, points.last()?);
    Some((a.0.min(b.0), a.1.min(b.1), (a.0 - b.0).abs().max(1.0), (a.1 - b.1).abs().max(1.0)))
}

/// Caminho da forma (sem o texto).
fn path_of(shape: &Shape) -> Option<Path> {
    let mut pb = PathBuilder::new();
    match shape.kind {
        ShapeKind::Pen => {
            let a = shape.points.first()?;
            pb.move_to(a.0, a.1);
            if shape.points.len() == 1 {
                pb.line_to(a.0 + 0.01, a.1);
            }
            for p in &shape.points[1..] {
                pb.line_to(p.0, p.1);
            }
        }
        ShapeKind::Line | ShapeKind::Arrow => {
            let (a, b) = (shape.points.first()?, shape.points.last()?);
            let b = if shape.kind == ShapeKind::Arrow {
                // A haste para antes da ponta (a ponta cobre o resto).
                let (dx, dy) = (b.0 - a.0, b.1 - a.1);
                let len = dx.hypot(dy).max(1.0);
                let head = (4.5 * shape.width).min(len * 0.6) * 0.8;
                (b.0 - dx / len * head, b.1 - dy / len * head)
            } else {
                *b
            };
            pb.move_to(a.0, a.1);
            pb.line_to(b.0, b.1);
        }
        ShapeKind::Rect => return Rect::from_xywh(boxed(&shape.points)?.0, boxed(&shape.points)?.1, boxed(&shape.points)?.2, boxed(&shape.points)?.3).map(PathBuilder::from_rect),
        ShapeKind::RoundRect => {
            let (x, y, w, h) = boxed(&shape.points)?;
            return crate::ui::draw::rounded_rect(x, y, w, h, (w.min(h) * 0.18).min(28.0));
        }
        ShapeKind::Oval => {
            let (x, y, w, h) = boxed(&shape.points)?;
            return Rect::from_xywh(x, y, w, h).and_then(PathBuilder::from_oval);
        }
        ShapeKind::Triangle => {
            let (x, y, w, h) = boxed(&shape.points)?;
            pb.move_to(x + w / 2.0, y);
            pb.line_to(x + w, y + h);
            pb.line_to(x, y + h);
            pb.close();
        }
        ShapeKind::Star => {
            let (x, y, w, h) = boxed(&shape.points)?;
            let (cx, cy) = (x + w / 2.0, y + h / 2.0);
            for i in 0..10 {
                let r = if i % 2 == 0 { 1.0 } else { 0.42 };
                let a = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI / 5.0;
                let (px, py) = (cx + a.cos() * w / 2.0 * r, cy + a.sin() * h / 2.0 * r);
                if i == 0 {
                    pb.move_to(px, py);
                } else {
                    pb.line_to(px, py);
                }
            }
            pb.close();
        }
        ShapeKind::Bubble => {
            let (x, y, w, h) = boxed(&shape.points)?;
            // Corpo arredondado em cima (80% da altura) e o bico embaixo à esquerda.
            let body = h * 0.8;
            let r = (w.min(body) * 0.25).min(30.0);
            pb.move_to(x + r, y);
            pb.line_to(x + w - r, y);
            pb.quad_to(x + w, y, x + w, y + r);
            pb.line_to(x + w, y + body - r);
            pb.quad_to(x + w, y + body, x + w - r, y + body);
            pb.line_to(x + w * 0.38, y + body);
            pb.line_to(x + w * 0.18, y + h);
            pb.line_to(x + w * 0.24, y + body);
            pb.line_to(x + r, y + body);
            pb.quad_to(x, y + body, x, y + body - r);
            pb.line_to(x, y + r);
            pb.quad_to(x, y, x + r, y);
            pb.close();
        }
        ShapeKind::Text => return None,
    }
    pb.finish()
}

/// Desenha a forma no canvas (`ts` leva das coordenadas dela ao canvas).
pub fn draw(canvas: &mut Pixmap, shape: &Shape, ts: Transform, text: &mut crate::ui::text::Text) {
    let color = |c: [u8; 3]| Color::from_rgba8(c[0], c[1], c[2], 255);
    if shape.kind == ShapeKind::Text {
        let (x, y) = shape.points.first().copied().unwrap_or_default();
        // O texto é desenhado pela escala da transformação (a imagem pode estar reduzida).
        let k = ts.sx;
        let c = shape.stroke;
        for (i, line) in shape.text.split('\n').enumerate() {
            let size = shape.font_size * k;
            text.draw_px(canvas, line, ts.tx + x * k, ts.ty + (y + i as f32 * shape.font_size * 1.3) * k, size, crate::ui::text::FontWeight::Regular, [c[0], c[1], c[2], 255]);
        }
        return;
    }
    let Some(path) = path_of(shape) else {
        return;
    };
    let mut paint = Paint { anti_alias: true, ..Default::default() };
    if let Some(fill) = shape.fill {
        paint.set_color(color(fill));
        canvas.fill_path(&path, &paint, FillRule::Winding, ts, None);
    }
    paint.set_color(color(shape.stroke));
    let mut stroke = Stroke { width: shape.width, line_cap: LineCap::Round, line_join: LineJoin::Round, ..Default::default() };
    if shape.dashed {
        stroke.line_cap = LineCap::Butt;
        stroke.dash = StrokeDash::new(vec![shape.width * 3.0, shape.width * 2.0], 0.0);
    }
    canvas.stroke_path(&path, &paint, &stroke, ts, None);
    if shape.kind == ShapeKind::Arrow {
        let (Some(a), Some(b)) = (shape.points.first(), shape.points.last()) else {
            return;
        };
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let len = dx.hypot(dy).max(1.0);
        let (ux, uy) = (dx / len, dy / len);
        let head = (4.5 * shape.width).min(len * 0.6);
        let (px, py) = (-uy, ux);
        let mut pb = PathBuilder::new();
        pb.move_to(b.0, b.1);
        pb.line_to(b.0 - ux * head + px * head * 0.55, b.1 - uy * head + py * head * 0.55);
        pb.line_to(b.0 - ux * head - px * head * 0.55, b.1 - uy * head - py * head * 0.55);
        pb.close();
        if let Some(head) = pb.finish() {
            canvas.fill_path(&head, &paint, FillRule::Winding, ts, None);
        }
    }
}

// -------------------------------------------------- reconhecimento do Esboço

/// Simplifica a linha (Ramer–Douglas–Peucker): os pontos que se afastam mais de `eps`
/// da reta entre os vizinhos.
fn simplify(points: &[(f32, f32)], eps: f32) -> Vec<(f32, f32)> {
    if points.len() < 3 {
        return points.to_vec();
    }
    let (a, b) = (points[0], points[points.len() - 1]);
    let (mut index, mut far) = (0, 0.0f32);
    for (i, p) in points.iter().enumerate().take(points.len() - 1).skip(1) {
        let d = segment_distance(*p, a, b);
        if d > far {
            far = d;
            index = i;
        }
    }
    if far <= eps {
        return vec![a, b];
    }
    let mut left = simplify(&points[..=index], eps);
    let right = simplify(&points[index..], eps);
    left.pop();
    left.extend(right);
    left
}


/// Maior distância de um ponto de `points` à reta de `a` a `b`.
fn deviation(points: &[(f32, f32)], a: (f32, f32), b: (f32, f32)) -> f32 {
    let len = (b.0 - a.0).hypot(b.1 - a.1).max(1e-3);
    points.iter().map(|p| ((b.0 - a.0) * (a.1 - p.1) - (a.0 - p.0) * (b.1 - a.1)).abs() / len).fold(0.0, f32::max)
}

/// Área do polígono fechado pelos pontos (fórmula do laço).
fn polygon_area(points: &[(f32, f32)]) -> f32 {
    let n = points.len();
    (0..n).map(|i| {
        let (p, q) = (points[i], points[(i + 1) % n]);
        p.0 * q.1 - q.0 * p.1
    }).sum::<f32>().abs() / 2.0
}

/// Quanto o traço gira em volta do centro (radianos, com sinal).
fn winding(points: &[(f32, f32)], c: (f32, f32)) -> f32 {
    let mut total = 0.0;
    for w in points.windows(2) {
        let a0 = (w[0].1 - c.1).atan2(w[0].0 - c.0);
        let a1 = (w[1].1 - c.1).atan2(w[1].0 - c.0);
        let mut d = a1 - a0;
        if d > std::f32::consts::PI {
            d -= std::f32::consts::TAU;
        } else if d < -std::f32::consts::PI {
            d += std::f32::consts::TAU;
        }
        total += d;
    }
    total
}

/// Reconhece um traço à mão (pontos da tela, na ordem do desenho), como o Esboço da
/// Marcação do Mac: linha, seta, retângulo, oval, triângulo ou estrela, com os pontos da
/// forma (as pontas, ou a caixa). `None`: fica desenho livre.
///
/// A mão treme: "reto" é medido pelo desvio máximo da reta (não pelo comprimento do
/// caminho, que cresce com o tremor), e as formas fechadas pela parte da caixa que o
/// traço cobre (retângulo ≈ 1, oval ≈ π/4, triângulo ≈ 1/2, estrela menos).
pub fn recognize(points: &[(f32, f32)]) -> Option<(ShapeKind, Vec<(f32, f32)>)> {
    if points.len() < 2 {
        return None;
    }
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for (x, y) in points {
        x0 = x0.min(*x);
        y0 = y0.min(*y);
        x1 = x1.max(*x);
        y1 = y1.max(*y);
    }
    let diag = (x1 - x0).hypot(y1 - y0);
    if diag < 12.0 {
        return None;
    }
    let (first, last) = (points[0], points[points.len() - 1]);
    let center = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    let turn = winding(points, center).abs();
    let gap = (first.0 - last.0).hypot(first.1 - last.1);
    // Fechado: o fim volta perto do começo, ou o traço dá a volta inteira (passando do
    // começo ou deixando uma fresta pequena).
    let closed = gap < diag * 0.25 || (turn > std::f32::consts::TAU * 0.85 && gap < diag * 0.45);
    if !closed {
        return open_shape(points, diag);
    }
    // Só uma volta: o que passa do começo não conta.
    let mut acc = 0.0;
    let mut end = points.len();
    for (i, w) in points.windows(2).enumerate() {
        let a0 = (w[0].1 - center.1).atan2(w[0].0 - center.0);
        let a1 = (w[1].1 - center.1).atan2(w[1].0 - center.0);
        let mut d = a1 - a0;
        if d > std::f32::consts::PI {
            d -= std::f32::consts::TAU;
        } else if d < -std::f32::consts::PI {
            d += std::f32::consts::TAU;
        }
        acc += d;
        if acc.abs() >= std::f32::consts::TAU {
            end = i + 2;
            break;
        }
    }
    let loop_pts = &points[..end.min(points.len())];
    let fill = polygon_area(loop_pts) / ((x1 - x0) * (y1 - y0)).max(1.0);
    let mut corners = simplify(loop_pts, diag * 0.07);
    if corners.len() > 2 {
        let (a, b) = (corners[0], corners[corners.len() - 1]);
        if (a.0 - b.0).hypot(a.1 - b.1) < diag * 0.25 {
            corners.pop();
        }
    }
    let boxed = vec![(x0, y0), (x1, y1)];
    // Estrela: muitas pontas, alternando longe e perto do centro, e pouco da caixa.
    if (8..=12).contains(&corners.len()) && fill < 0.6 {
        let r: Vec<f32> = corners.iter().map(|p| ((p.0 - center.0) / (x1 - x0).max(1.0)).hypot((p.1 - center.1) / (y1 - y0).max(1.0))).collect();
        let alternating = (0..r.len()).filter(|&i| {
            let (a, b, c) = (r[(i + r.len() - 1) % r.len()], r[i], r[(i + 1) % r.len()]);
            (b > a && b > c) || (b < a && b < c)
        }).count();
        if alternating as f32 >= r.len() as f32 * 0.7 {
            return Some((ShapeKind::Star, boxed));
        }
    }
    if corners.len() == 3 && fill < 0.68 {
        return Some((ShapeKind::Triangle, boxed));
    }
    if fill >= 0.86 && corners.len() <= 6 {
        return Some((ShapeKind::Rect, boxed));
    }
    if (0.6..0.9).contains(&fill) {
        return Some((ShapeKind::Oval, boxed));
    }
    if (0.4..0.68).contains(&fill) && corners.len() <= 4 {
        return Some((ShapeKind::Triangle, boxed));
    }
    None
}

/// Traço aberto: linha (quase reta) ou seta (reta até a ponta e a cabeça em volta dela).
fn open_shape(points: &[(f32, f32)], diag: f32) -> Option<(ShapeKind, Vec<(f32, f32)>)> {
    let (first, last) = (points[0], points[points.len() - 1]);
    let straight = (first.0 - last.0).hypot(first.1 - last.1);
    if straight > diag * 0.85 && deviation(points, first, last) < straight * 0.07 {
        return Some((ShapeKind::Line, vec![first, last]));
    }
    // A ponta: o ponto mais longe do começo (a primeira vez que chega lá).
    let mut tip_i = 0;
    let mut far = -1.0f32;
    for (i, p) in points.iter().enumerate() {
        let d = (p.0 - first.0).hypot(p.1 - first.1);
        if d > far {
            far = d;
            tip_i = i;
        }
    }
    let tip = points[tip_i];
    let shaft = far;
    if shaft < 16.0 || tip_i == 0 {
        return None;
    }
    // A haste é reta (desvio pequeno) e a cabeça fica perto da ponta, voltando para trás.
    if deviation(&points[..=tip_i], first, tip) > shaft * 0.12 {
        return None;
    }
    let head = &points[tip_i..];
    let near = head.iter().all(|p| (p.0 - tip.0).hypot(p.1 - tip.1) < shaft * 0.6 + 6.0);
    let size = head.iter().map(|p| (p.0 - tip.0).hypot(p.1 - tip.1)).fold(0.0, f32::max);
    // A cabeça sai para o lado (não é só a haste tremendo na ponta).
    let side = deviation(head, first, tip);
    if near && size > (shaft * 0.08).max(5.0) && side > (shaft * 0.04).max(3.0) {
        return Some((ShapeKind::Arrow, vec![first, tip]));
    }
    None
}

/// A cabeça de seta desenhada num segundo traço: o traço `points` (tela) é pequeno e
/// fica perto de uma das pontas da linha `a`–`b` (tela). Devolve a ponta (0 ou 1).
pub fn head_for_line(points: &[(f32, f32)], a: (f32, f32), b: (f32, f32)) -> Option<usize> {
    let len = (b.0 - a.0).hypot(b.1 - a.1);
    if points.len() < 2 || len < 16.0 {
        return None;
    }
    let size = |e: (f32, f32)| points.iter().map(|p| (p.0 - e.0).hypot(p.1 - e.1)).fold(0.0, f32::max);
    let touch = |e: (f32, f32)| points.iter().map(|p| (p.0 - e.0).hypot(p.1 - e.1)).fold(f32::MAX, f32::min);
    [a, b].iter().enumerate().filter(|(_, e)| touch(**e) < len * 0.2 + 10.0 && size(**e) < len * 0.6 + 10.0).min_by(|x, y| touch(*x.1).total_cmp(&touch(*y.1))).map(|(i, _)| i)
}

/// A caixa `orig` com a alça `h` arrastada de `delta` (sem limites; vira ao passar do
/// outro lado).
pub fn resize_box(orig: RectF, h: Handle, delta: (f32, f32)) -> RectF {
    let (mut x0, mut y0, mut x1, mut y1) = (orig.0, orig.1, orig.0 + orig.2, orig.1 + orig.3);
    let (sx, sy) = h.sides();
    match sx {
        -1 => x0 += delta.0,
        1 => x1 += delta.0,
        _ => {}
    }
    match sy {
        -1 => y0 += delta.1,
        1 => y1 += delta.1,
        _ => {}
    }
    (x0.min(x1), y0.min(y1), (x1 - x0).abs().max(2.0), (y1 - y0).abs().max(2.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(a: (f32, f32), b: (f32, f32), n: usize) -> Vec<(f32, f32)> {
        (0..=n).map(|i| {
            let t = i as f32 / n as f32;
            (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
        }).collect()
    }

    #[test]
    fn traco_reto_vira_linha() {
        let pts = line((10.0, 10.0), (200.0, 60.0), 30);
        assert_eq!(recognize(&pts).map(|r| r.0), Some(ShapeKind::Line));
    }

    #[test]
    fn traco_com_cabeca_vira_seta() {
        let mut pts = line((10.0, 100.0), (200.0, 100.0), 30);
        pts.extend(line((200.0, 100.0), (175.0, 80.0), 6));
        pts.extend(line((175.0, 80.0), (200.0, 100.0), 6));
        pts.extend(line((200.0, 100.0), (175.0, 120.0), 6));
        let r = recognize(&pts).expect("reconhecido");
        assert_eq!(r.0, ShapeKind::Arrow);
        assert!((r.1[1].0 - 200.0).abs() < 1.0, "ponta em {:?}", r.1[1]);
    }

    #[test]
    fn retangulo_triangulo_e_oval() {
        let mut rect = line((0.0, 0.0), (200.0, 0.0), 20);
        rect.extend(line((200.0, 0.0), (200.0, 120.0), 12));
        rect.extend(line((200.0, 120.0), (0.0, 120.0), 20));
        rect.extend(line((0.0, 120.0), (0.0, 3.0), 12));
        assert_eq!(recognize(&rect).map(|r| r.0), Some(ShapeKind::Rect));

        let mut tri = line((100.0, 0.0), (200.0, 150.0), 20);
        tri.extend(line((200.0, 150.0), (0.0, 150.0), 20));
        tri.extend(line((0.0, 150.0), (98.0, 4.0), 20));
        assert_eq!(recognize(&tri).map(|r| r.0), Some(ShapeKind::Triangle));

        let oval: Vec<(f32, f32)> = (0..=60).map(|i| {
            let a = i as f32 / 60.0 * std::f32::consts::TAU;
            (150.0 + a.cos() * 120.0, 100.0 + a.sin() * 70.0)
        }).collect();
        assert_eq!(recognize(&oval).map(|r| r.0), Some(ShapeKind::Oval));
    }

    /// Tremor de mão: ±2 pontos alternados em cada ponto.
    fn shaky(points: Vec<(f32, f32)>) -> Vec<(f32, f32)> {
        points.into_iter().enumerate().map(|(i, (x, y))| (x + if i % 2 == 0 { 2.0 } else { -2.0 }, y + if i % 3 == 0 { 2.0 } else { -1.5 })).collect()
    }

    #[test]
    fn seta_tremida_e_curta() {
        let mut pts: Vec<(f32, f32)> = (0..=60).map(|i| (10.0 + i as f32, 50.0)).collect();
        pts.extend(line((70.0, 50.0), (60.0, 42.0), 5));
        pts.extend(line((60.0, 42.0), (70.0, 50.0), 5));
        pts.extend(line((70.0, 50.0), (60.0, 58.0), 5));
        assert_eq!(recognize(&shaky(pts)).map(|r| r.0), Some(ShapeKind::Arrow));
        // Cabeça de uma asa só, voltando para trás.
        let mut pts = line((0.0, 0.0), (120.0, 40.0), 80);
        pts.extend(line((120.0, 40.0), (100.0, 50.0), 10));
        assert_eq!(recognize(&shaky(pts)).map(|r| r.0), Some(ShapeKind::Arrow));
    }

    #[test]
    fn circulo_que_passa_do_comeco_e_com_fresta() {
        let circle = |from: f32, to: f32| -> Vec<(f32, f32)> {
            (0..=80).map(|i| {
                let a = from + (to - from) * i as f32 / 80.0;
                (100.0 + a.cos() * 40.0, 100.0 + a.sin() * 50.0)
            }).collect()
        };
        assert_eq!(recognize(&shaky(circle(0.0, 7.2))).map(|r| r.0), Some(ShapeKind::Oval));
        assert_eq!(recognize(&shaky(circle(0.0, 5.7))).map(|r| r.0), Some(ShapeKind::Oval));
    }

    #[test]
    fn estrela() {
        let pts: Vec<(f32, f32)> = (0..=10).map(|i| {
            let a = i as f32 * std::f32::consts::PI / 5.0 - std::f32::consts::FRAC_PI_2;
            let r = if i % 2 == 0 { 100.0 } else { 40.0 };
            (150.0 + a.cos() * r, 150.0 + a.sin() * r)
        }).collect();
        let mut dense = Vec::new();
        for w in pts.windows(2) {
            dense.extend(line(w[0], w[1], 10));
        }
        assert_eq!(recognize(&dense).map(|r| r.0), Some(ShapeKind::Star));
    }

    #[test]
    fn cabeca_em_outro_traco() {
        let head = vec![(190.0, 90.0), (200.0, 100.0), (190.0, 110.0)];
        assert_eq!(head_for_line(&head, (10.0, 100.0), (200.0, 100.0)), Some(1));
        assert_eq!(head_for_line(&head, (10.0, 300.0), (200.0, 300.0)), None);
    }

    #[test]
    fn rabisco_fica_desenho_livre() {
        let zig: Vec<(f32, f32)> = (0..40).map(|i| (i as f32 * 8.0, if i % 2 == 0 { 0.0 } else { 40.0 })).collect();
        assert_eq!(recognize(&zig), None);
    }

    #[test]
    fn redimensionar_e_girar() {
        let mut s = Shape::new(ShapeKind::Rect, vec![(10.0, 10.0), (110.0, 60.0)], [255, 0, 0], 4.0);
        let r = resize_box((10.0, 10.0, 100.0, 50.0), Handle::BottomRight, (100.0, 50.0));
        assert_eq!(r, (10.0, 10.0, 200.0, 100.0));
        s.fit_to((10.0, 10.0, 100.0, 50.0), r);
        assert_eq!(s.points[1], (210.0, 110.0));
        s.rotate(true, 300.0, 200.0);
        assert_eq!(s.points[0], (190.0, 10.0));
    }
}
