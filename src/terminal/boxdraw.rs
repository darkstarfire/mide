//! Caracteres desenhados pelo próprio terminal, sem a fonte: linhas de quadro
//! (U+2500–U+257F), blocos (U+2580–U+259F) e as setas e meios-círculos do powerline
//! (U+E0B0–U+E0BF).
//!
//! Na fonte, esses caracteres têm o tamanho do desenho dela, não o da célula: a linha
//! "│" de uma fonte de 1,3 em de altura não chega às bordas de uma célula mais alta,
//! e um "─" com avanço de 7,8 pixels deixa uma fresta numa célula de 8. Desenhados
//! aqui, cada traço vai de borda a borda da célula, sempre na mesma posição (as
//! linhas da célula de cima e da de baixo se encontram), e blocos e setas cobrem a
//! célula inteira. É o que os bons terminais fazem; o código é do mide.
//!
//! O resultado é uma máscara de cobertura (0 a 255 por pixel) do tamanho da célula;
//! quem desenha pinta a cor do texto com ela.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use tiny_skia::{FillRule, LineCap, Paint, PathBuilder, Pixmap, Rect, Stroke, Transform};

/// Cobertura de cada pixel da célula, linha a linha (`w * h` valores).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mask {
    pub w: u32,
    pub h: u32,
    pub data: Vec<u8>,
}

impl Mask {
    /// Cobertura do pixel `(x, y)` (fora da célula: 0).
    pub fn at(&self, x: u32, y: u32) -> u8 {
        if x >= self.w || y >= self.h {
            return 0;
        }
        self.data[(y * self.w + x) as usize]
    }
}

/// O caractere é desenhado aqui (e não pela fonte)?
pub fn is_drawn(c: char) -> bool {
    matches!(c as u32, 0x2500..=0x259F | 0xE0B0..=0xE0BF)
}

/// Máscara do caractere numa célula de `w` x `h` pixels (guardada: as mesmas
/// combinações se repetem em toda a tela).
pub fn mask(c: char, w: u32, h: u32) -> Option<Rc<Mask>> {
    type Cache = HashMap<(char, u32, u32), Option<Rc<Mask>>>;
    thread_local! {
        static CACHE: RefCell<Cache> = RefCell::new(HashMap::new());
    }
    if !is_drawn(c) || w == 0 || h == 0 {
        return None;
    }
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.len() > 2048 {
            cache.clear();
        }
        cache.entry((c, w, h)).or_insert_with(|| render(c, w, h).map(Rc::new)).clone()
    })
}

/// Desenha a máscara (sem guardar).
pub fn render(c: char, w: u32, h: u32) -> Option<Mask> {
    let shapes = shapes(c, w, h)?;
    let mut pixmap = Pixmap::new(w, h)?;
    for shape in &shapes {
        paint(&mut pixmap, shape);
    }
    let data = pixmap.pixels().iter().map(|p| p.alpha()).collect();
    Some(Mask { w, h, data })
}

/// Peças do desenho, em pixels da célula.
#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    /// Retângulo (bordas inteiras: sem borrão), com a opacidade (sombreados).
    Rect { x: f32, y: f32, w: f32, h: f32, alpha: f32 },
    /// Polígono preenchido, com antisserrilhado (setas, meios-círculos, cantos).
    Fill(Vec<(f32, f32)>),
    /// Linha com espessura, pontas retas, com antisserrilhado (arcos, diagonais).
    Line { points: Vec<(f32, f32)>, width: f32 },
}

fn paint(pixmap: &mut Pixmap, shape: &Shape) {
    let mut p = Paint::default();
    p.set_color_rgba8(255, 255, 255, 255);
    match shape {
        Shape::Rect { x, y, w, h, alpha } => {
            p.set_color_rgba8(255, 255, 255, (alpha.clamp(0.0, 1.0) * 255.0).round() as u8);
            p.anti_alias = false;
            if let Some(r) = Rect::from_xywh(*x, *y, *w, *h) {
                pixmap.fill_rect(r, &p, Transform::identity(), None);
            }
        }
        Shape::Fill(points) => {
            p.anti_alias = true;
            if let Some(path) = polyline(points, true) {
                pixmap.fill_path(&path, &p, FillRule::Winding, Transform::identity(), None);
            }
        }
        Shape::Line { points, width } => {
            p.anti_alias = true;
            if let Some(path) = polyline(points, false) {
                let stroke = Stroke { width: *width, line_cap: LineCap::Butt, ..Stroke::default() };
                pixmap.stroke_path(&path, &p, &stroke, Transform::identity(), None);
            }
        }
    }
}

fn polyline(points: &[(f32, f32)], close: bool) -> Option<tiny_skia::Path> {
    let (first, rest) = points.split_first()?;
    let mut pb = PathBuilder::new();
    pb.move_to(first.0, first.1);
    for (x, y) in rest {
        pb.line_to(*x, *y);
    }
    if close {
        pb.close();
    }
    pb.finish()
}

/// Peso de um braço de linha de quadro.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Weight {
    None,
    Light,
    Heavy,
    Double,
}

use Weight::{Double as D, Heavy as H, Light as L, None as N};

/// Braços (cima, direita, baixo, esquerda) de U+2500–U+257F; `None` nos tracejados,
/// arcos e diagonais, que têm desenho próprio.
fn arms(c: char) -> Option<[Weight; 4]> {
    let i = (c as u32).checked_sub(0x2500)? as usize;
    const T: [[Weight; 4]; 128] = [
        [N, L, N, L], [N, H, N, H], [L, N, L, N], [H, N, H, N], // ─ ━ │ ┃
        [N, N, N, N], [N, N, N, N], [N, N, N, N], [N, N, N, N], // ┄ ┅ ┆ ┇
        [N, N, N, N], [N, N, N, N], [N, N, N, N], [N, N, N, N], // ┈ ┉ ┊ ┋
        [N, L, L, N], [N, H, L, N], [N, L, H, N], [N, H, H, N], // ┌ ┍ ┎ ┏
        [N, N, L, L], [N, N, L, H], [N, N, H, L], [N, N, H, H], // ┐ ┑ ┒ ┓
        [L, L, N, N], [L, H, N, N], [H, L, N, N], [H, H, N, N], // └ ┕ ┖ ┗
        [L, N, N, L], [L, N, N, H], [H, N, N, L], [H, N, N, H], // ┘ ┙ ┚ ┛
        [L, L, L, N], [L, H, L, N], [H, L, L, N], [L, L, H, N], // ├ ┝ ┞ ┟
        [H, L, H, N], [H, H, L, N], [L, H, H, N], [H, H, H, N], // ┠ ┡ ┢ ┣
        [L, N, L, L], [L, N, L, H], [H, N, L, L], [L, N, H, L], // ┤ ┥ ┦ ┧
        [H, N, H, L], [H, N, L, H], [L, N, H, H], [H, N, H, H], // ┨ ┩ ┪ ┫
        [N, L, L, L], [N, L, L, H], [N, H, L, L], [N, H, L, H], // ┬ ┭ ┮ ┯
        [N, L, H, L], [N, L, H, H], [N, H, H, L], [N, H, H, H], // ┰ ┱ ┲ ┳
        [L, L, N, L], [L, L, N, H], [L, H, N, L], [L, H, N, H], // ┴ ┵ ┶ ┷
        [H, L, N, L], [H, L, N, H], [H, H, N, L], [H, H, N, H], // ┸ ┹ ┺ ┻
        [L, L, L, L], [L, L, L, H], [L, H, L, L], [L, H, L, H], // ┼ ┽ ┾ ┿
        [H, L, L, L], [L, L, H, L], [H, L, H, L], [H, L, L, H], // ╀ ╁ ╂ ╃
        [H, H, L, L], [L, L, H, H], [L, H, H, L], [H, H, L, H], // ╄ ╅ ╆ ╇
        [L, H, H, H], [H, L, H, H], [H, H, H, L], [H, H, H, H], // ╈ ╉ ╊ ╋
        [N, N, N, N], [N, N, N, N], [N, N, N, N], [N, N, N, N], // ╌ ╍ ╎ ╏
        [N, D, N, D], [D, N, D, N], [N, D, L, N], [N, L, D, N], // ═ ║ ╒ ╓
        [N, D, D, N], [N, N, L, D], [N, N, D, L], [N, N, D, D], // ╔ ╕ ╖ ╗
        [L, D, N, N], [D, L, N, N], [D, D, N, N], [L, N, N, D], // ╘ ╙ ╚ ╛
        [D, N, N, L], [D, N, N, D], [L, D, L, N], [D, L, D, N], // ╜ ╝ ╞ ╟
        [D, D, D, N], [L, N, L, D], [D, N, D, L], [D, N, D, D], // ╠ ╡ ╢ ╣
        [N, D, L, D], [N, L, D, L], [N, D, D, D], [L, D, N, D], // ╤ ╥ ╦ ╧
        [D, L, N, L], [D, D, N, D], [L, D, L, D], [D, L, D, L], // ╨ ╩ ╪ ╫
        [D, D, D, D], [N, N, N, N], [N, N, N, N], [N, N, N, N], // ╬ ╭ ╮ ╯
        [N, N, N, N], [N, N, N, N], [N, N, N, N], [N, N, N, N], // ╰ ╱ ╲ ╳
        [N, N, N, L], [L, N, N, N], [N, L, N, N], [N, N, L, N], // ╴ ╵ ╶ ╷
        [N, N, N, H], [H, N, N, N], [N, H, N, N], [N, N, H, N], // ╸ ╹ ╺ ╻
        [N, H, N, L], [L, N, H, N], [N, L, N, H], [H, N, L, N], // ╼ ╽ ╾ ╿
    ];
    let a = *T.get(i)?;
    (a != [N; 4]).then_some(a)
}

/// Espessura da linha fina: 1 pixel até células de 13 de largura, depois cresce com a
/// célula (fontes grandes).
pub fn light_width(w: u32) -> u32 {
    ((w as f32 / 9.0).round() as u32).max(1)
}

/// Faixa de `thick` pixels centrada em `total` (começo, fim). A mesma conta em todas
/// as células: as linhas de células vizinhas caem nos mesmos pixels.
fn band(total: u32, thick: u32) -> (f32, f32) {
    let start = total.saturating_sub(thick) / 2;
    (start as f32, (start + thick) as f32)
}

/// Peças de um caractere desenhado aqui (`None`: fica com a fonte).
pub fn shapes(c: char, w: u32, h: u32) -> Option<Vec<Shape>> {
    let code = c as u32;
    match code {
        0x2500..=0x257F => lines(c, w, h),
        0x2580..=0x259F => Some(blocks(code, w as f32, h as f32)),
        0xE0B0..=0xE0BF => Some(powerline(code, w as f32, h as f32, light_width(w) as f32)),
        _ => None,
    }
}

fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> Shape {
    Shape::Rect { x: x0, y: y0, w: (x1 - x0).max(0.0), h: (y1 - y0).max(0.0), alpha: 1.0 }
}

fn lines(c: char, w: u32, h: u32) -> Option<Vec<Shape>> {
    let t = light_width(w);
    let code = c as u32;
    let (wf, hf) = (w as f32, h as f32);
    // Tracejados: (traços, grossa?, horizontal?).
    let dashed = match code {
        0x2504 => Some((3, false, true)),
        0x2505 => Some((3, true, true)),
        0x2506 => Some((3, false, false)),
        0x2507 => Some((3, true, false)),
        0x2508 => Some((4, false, true)),
        0x2509 => Some((4, true, true)),
        0x250A => Some((4, false, false)),
        0x250B => Some((4, true, false)),
        0x254C => Some((2, false, true)),
        0x254D => Some((2, true, true)),
        0x254E => Some((2, false, false)),
        0x254F => Some((2, true, false)),
        _ => None,
    };
    if let Some((n, heavy, horizontal)) = dashed {
        let thick = if heavy { 2 * t } else { t };
        let mut out = Vec::new();
        let length = if horizontal { wf } else { hf };
        let gap = (length / n as f32 / 3.0).round().max(1.0);
        for i in 0..n {
            // Cada traço começa no início do seu pedaço; a folga fica no fim (o último
            // traço deixa a folga antes do primeiro da célula seguinte).
            let a = (i as f32 * length / n as f32).round();
            let b = ((i + 1) as f32 * length / n as f32).round() - gap;
            if horizontal {
                let (y0, y1) = band(h, thick);
                out.push(rect(a, y0, b.max(a + 1.0), y1));
            } else {
                let (x0, x1) = band(w, thick);
                out.push(rect(x0, a, x1, b.max(a + 1.0)));
            }
        }
        return Some(out);
    }
    // Arcos: um quarto de círculo ligando os meios de duas bordas.
    if (0x256D..=0x2570).contains(&code) {
        let (vx0, vx1) = band(w, t);
        let (hy0, hy1) = band(h, t);
        let (cx, cy) = ((vx0 + vx1) / 2.0, (hy0 + hy1) / 2.0);
        // (direção horizontal, vertical) para onde o arco vai: ╭ direita/baixo etc.
        let (right, down) = match code {
            0x256D => (true, true),
            0x256E => (false, true),
            0x256F => (false, false),
            _ => (true, false),
        };
        // Raio: o espaço até a borda mais perto, menos um pixel de reta (a ponta reta
        // encosta inteira na linha da célula vizinha).
        let r = ((if right { wf - cx } else { cx }).min(if down { hf - cy } else { cy }) - 1.0).max(1.0);
        let (sx, sy) = (if right { 1.0 } else { -1.0 }, if down { 1.0 } else { -1.0 });
        // Centro do arco: a r do ponto central, nas duas direções.
        let (ox, oy) = (cx + sx * r, cy + sy * r);
        let mut points = vec![(cx, if down { hf } else { 0.0 }), (cx, oy)];
        let steps = 16;
        for i in 0..=steps {
            let a = i as f32 / steps as f32 * std::f32::consts::FRAC_PI_2;
            points.push((ox - sx * r * a.cos(), oy - sy * r * a.sin()));
        }
        points.push((if right { wf } else { 0.0 }, cy));
        return Some(vec![Shape::Line { points, width: t as f32 }]);
    }
    // Diagonais: de canto a canto, passando um pouco da célula (encostam nas vizinhas).
    if (0x2571..=0x2573).contains(&code) {
        let e = t as f32;
        let k = e / (wf * wf + hf * hf).sqrt();
        let (dx, dy) = (wf * k, hf * k);
        let slash = Shape::Line { points: vec![(wf + dx, -dy), (-dx, hf + dy)], width: t as f32 };
        let back = Shape::Line { points: vec![(-dx, -dy), (wf + dx, hf + dy)], width: t as f32 };
        return Some(match code {
            0x2571 => vec![slash],
            0x2572 => vec![back],
            _ => vec![slash, back],
        });
    }
    let [up, right, down, left] = arms(c)?;
    let thick = |a: Weight| match a {
        Weight::Heavy => 2 * t,
        _ => t,
    };
    // Faixas das linhas simples (fina ou grossa) que cruzam o centro.
    let vbands: Vec<(f32, f32)> = [up, down].iter().filter(|a| matches!(a, L | H)).map(|a| band(w, thick(*a))).collect();
    let hbands: Vec<(f32, f32)> = [left, right].iter().filter(|a| matches!(a, L | H)).map(|a| band(h, thick(*a))).collect();
    // Faixa dupla: duas linhas finas com uma fresta da mesma largura entre elas.
    let (dv0, _) = band(w, 3 * t);
    let (dh0, _) = band(h, 3 * t);
    let tf = t as f32;
    let (dv_a, dv_b) = ((dv0, dv0 + tf), (dv0 + 2.0 * tf, dv0 + 3.0 * tf));
    let (dh_a, dh_b) = ((dh0, dh0 + tf), (dh0 + 2.0 * tf, dh0 + 3.0 * tf));
    let vdouble = up == D || down == D;
    let hdouble = left == D || right == D;
    let vmax = vbands.iter().map(|b| b.1).fold(f32::MIN, f32::max);
    let vmin = vbands.iter().map(|b| b.0).fold(f32::MAX, f32::min);
    let hmax = hbands.iter().map(|b| b.1).fold(f32::MIN, f32::max);
    let hmin = hbands.iter().map(|b| b.0).fold(f32::MAX, f32::min);
    let mut out = Vec::new();
    // Braços horizontais. Um braço simples vai até o fim da estrutura do outro eixo,
    // para as linhas se encontrarem sem fresta; diante de uma linha dupla, até a
    // segunda linha dela (canto ou cruzamento) ou só até a primeira, quando a dupla
    // atravessa a célula e o braço não continua do outro lado (╢).
    for (arm, is_left) in [(left, true), (right, false)] {
        match arm {
            N => {}
            L | H => {
                let (y0, y1) = band(h, thick(arm));
                let opposite = if is_left { right != N } else { left != N };
                let stops_at_first = up == D && down == D && !opposite;
                let own = band(w, thick(arm));
                if is_left {
                    let end = if vdouble {
                        if stops_at_first { dv_a.1 } else { dv_b.1 }
                    } else {
                        own.1.max(vmax)
                    };
                    out.push(rect(0.0, y0, end, y1));
                } else {
                    let start = if vdouble {
                        if stops_at_first { dv_b.0 } else { dv_a.0 }
                    } else {
                        own.0.min(vmin)
                    };
                    out.push(rect(start, y0, wf, y1));
                }
            }
            D => {
                // Linha de cima (a) e de baixo (b) do braço duplo.
                for (stroke, is_top) in [(dh_a, true), (dh_b, false)] {
                    let toward = if is_top { up } else { down };
                    let away = if is_top { down } else { up };
                    if is_left {
                        let end = if toward == D {
                            dv_a.1
                        } else if away == D {
                            dv_b.1
                        } else if vmax > f32::MIN {
                            vmax
                        } else {
                            dv_b.1
                        };
                        out.push(rect(0.0, stroke.0, end, stroke.1));
                    } else {
                        let start = if toward == D {
                            dv_b.0
                        } else if away == D {
                            dv_a.0
                        } else if vmin < f32::MAX {
                            vmin
                        } else {
                            dv_a.0
                        };
                        out.push(rect(start, stroke.0, wf, stroke.1));
                    }
                }
            }
        }
    }
    // Braços verticais (as mesmas regras, trocando os eixos).
    for (arm, is_up) in [(up, true), (down, false)] {
        match arm {
            N => {}
            L | H => {
                let (x0, x1) = band(w, thick(arm));
                let opposite = if is_up { down != N } else { up != N };
                let stops_at_first = left == D && right == D && !opposite;
                let own = band(h, thick(arm));
                if is_up {
                    let end = if hdouble {
                        if stops_at_first { dh_a.1 } else { dh_b.1 }
                    } else {
                        own.1.max(hmax)
                    };
                    out.push(rect(x0, 0.0, x1, end));
                } else {
                    let start = if hdouble {
                        if stops_at_first { dh_b.0 } else { dh_a.0 }
                    } else {
                        own.0.min(hmin)
                    };
                    out.push(rect(x0, start, x1, hf));
                }
            }
            D => {
                for (stroke, is_leftside) in [(dv_a, true), (dv_b, false)] {
                    let toward = if is_leftside { left } else { right };
                    let away = if is_leftside { right } else { left };
                    if is_up {
                        let end = if toward == D {
                            dh_a.1
                        } else if away == D {
                            dh_b.1
                        } else if hmax > f32::MIN {
                            hmax
                        } else {
                            dh_b.1
                        };
                        out.push(rect(stroke.0, 0.0, stroke.1, end));
                    } else {
                        let start = if toward == D {
                            dh_b.0
                        } else if away == D {
                            dh_a.0
                        } else if hmin < f32::MAX {
                            hmin
                        } else {
                            dh_a.0
                        };
                        out.push(rect(stroke.0, start, stroke.1, hf));
                    }
                }
            }
        }
    }
    Some(out)
}

/// Blocos e sombreados (U+2580–U+259F): frações da célula arredondadas para pixels
/// inteiros, de modo que ▀ e ▄ juntos cobrem a célula sem fresta nem sobra.
fn blocks(code: u32, w: f32, h: f32) -> Vec<Shape> {
    let ry = |k: f32| (h * k / 8.0).round();
    let rx = |k: f32| (w * k / 8.0).round();
    // Meio da célula: o mesmo corte do ▄ (de baixo) e do ▌ (da esquerda).
    let (mx, my) = (rx(4.0), h - ry(4.0));
    let quad = |ul: bool, ur: bool, ll: bool, lr: bool| {
        let mut v = Vec::new();
        if ul {
            v.push(rect(0.0, 0.0, mx, my));
        }
        if ur {
            v.push(rect(mx, 0.0, w, my));
        }
        if ll {
            v.push(rect(0.0, my, mx, h));
        }
        if lr {
            v.push(rect(mx, my, w, h));
        }
        v
    };
    let shade = |alpha: f32| vec![Shape::Rect { x: 0.0, y: 0.0, w, h, alpha }];
    match code {
        0x2580 => vec![rect(0.0, 0.0, w, my)],
        0x2581..=0x2587 => vec![rect(0.0, h - ry((code - 0x2580) as f32), w, h)],
        0x2588 => vec![rect(0.0, 0.0, w, h)],
        0x2589..=0x258F => vec![rect(0.0, 0.0, rx((0x2590 - code) as f32), h)],
        0x2590 => vec![rect(mx, 0.0, w, h)],
        0x2591 => shade(0.25),
        0x2592 => shade(0.5),
        0x2593 => shade(0.75),
        0x2594 => vec![rect(0.0, 0.0, w, ry(1.0))],
        0x2595 => vec![rect(w - rx(1.0), 0.0, w, h)],
        0x2596 => quad(false, false, true, false),
        0x2597 => quad(false, false, false, true),
        0x2598 => quad(true, false, false, false),
        0x2599 => quad(true, false, true, true),
        0x259A => quad(true, false, false, true),
        0x259B => quad(true, true, true, false),
        0x259C => quad(true, true, false, true),
        0x259D => quad(false, true, false, false),
        0x259E => quad(false, true, true, false),
        _ => quad(false, true, true, true),
    }
}

/// Meia elipse de `(cx, cy)` com raios `rx`, `ry`, de `from` a `to` graus (0° à
/// direita, 90° embaixo).
fn arc(cx: f32, cy: f32, rx: f32, ry: f32, from: f32, to: f32) -> Vec<(f32, f32)> {
    let steps = 24;
    (0..=steps)
        .map(|i| {
            let a = (from + (to - from) * i as f32 / steps as f32).to_radians();
            (cx + rx * a.cos(), cy + ry * a.sin())
        })
        .collect()
}

/// Símbolos do powerline (U+E0B0–U+E0BF), da altura e da largura da célula: as setas
/// encostam no fundo da célula vizinha sem fresta.
fn powerline(code: u32, w: f32, h: f32, t: f32) -> Vec<Shape> {
    let mid = h / 2.0;
    // Linhas finas: metade da espessura para dentro, para não serem cortadas na borda.
    let i = t / 2.0;
    // As setas e meios-círculos cheios levam a coluna de pixels da borda reta inteira
    // (nos cantos, o antisserrilhado deixaria meio pixel da cor do fundo: uma fresta
    // entre a seta e a célula vizinha).
    let left_col = rect(0.0, 0.0, 1.0, h);
    let right_col = rect(w - 1.0, 0.0, w, h);
    match code {
        0xE0B0 => vec![Shape::Fill(vec![(0.0, 0.0), (w, mid), (0.0, h)]), left_col],
        0xE0B1 => vec![Shape::Line { points: vec![(0.0, 0.0), (w - i, mid), (0.0, h)], width: t }],
        0xE0B2 => vec![Shape::Fill(vec![(w, 0.0), (0.0, mid), (w, h)]), right_col],
        0xE0B3 => vec![Shape::Line { points: vec![(w, 0.0), (i, mid), (w, h)], width: t }],
        0xE0B4 => vec![Shape::Fill(arc(0.0, mid, w, mid, -90.0, 90.0)), left_col],
        0xE0B5 => vec![Shape::Line { points: arc(0.0, mid, w - i, mid - i, -90.0, 90.0), width: t }],
        0xE0B6 => vec![Shape::Fill(arc(w, mid, w, mid, 90.0, 270.0)), right_col],
        0xE0B7 => vec![Shape::Line { points: arc(w, mid, w - i, mid - i, 90.0, 270.0), width: t }],
        0xE0B8 => vec![Shape::Fill(vec![(0.0, 0.0), (w, h), (0.0, h)])],
        0xE0B9 => vec![Shape::Line { points: vec![(0.0, 0.0), (w, h)], width: t }],
        0xE0BA => vec![Shape::Fill(vec![(w, 0.0), (w, h), (0.0, h)])],
        0xE0BB => vec![Shape::Line { points: vec![(0.0, h), (w, 0.0)], width: t }],
        0xE0BC => vec![Shape::Fill(vec![(0.0, 0.0), (w, 0.0), (0.0, h)])],
        0xE0BD => vec![Shape::Line { points: vec![(0.0, h), (w, 0.0)], width: t }],
        0xE0BE => vec![Shape::Fill(vec![(0.0, 0.0), (w, 0.0), (w, h)])],
        _ => vec![Shape::Line { points: vec![(0.0, 0.0), (w, h)], width: t }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tamanhos de célula de fontes de 9 a 24 pontos (largura, altura).
    const CELLS: [(u32, u32); 6] = [(6, 12), (8, 17), (9, 19), (10, 22), (13, 26), (15, 31)];

    fn filled(m: &Mask, x: u32, y: u32) -> bool {
        m.at(x, y) >= 250
    }

    /// Colunas pintadas numa linha (para comparar com a célula vizinha).
    fn row(m: &Mask, y: u32) -> Vec<bool> {
        (0..m.w).map(|x| filled(m, x, y)).collect()
    }

    fn col(m: &Mask, x: u32) -> Vec<bool> {
        (0..m.h).map(|y| filled(m, x, y)).collect()
    }

    /// Linhas pintadas na borda esquerda (x = 0) e direita (x = w - 1).
    fn edge_rows(m: &Mask, right: bool) -> Vec<u32> {
        let x = if right { m.w - 1 } else { 0 };
        (0..m.h).filter(|y| filled(m, x, *y)).collect()
    }

    fn edge_cols(m: &Mask, bottom: bool) -> Vec<u32> {
        let y = if bottom { m.h - 1 } else { 0 };
        (0..m.w).filter(|x| filled(m, *x, y)).collect()
    }

    #[test]
    fn todo_o_bloco_desenhado_aqui() {
        for c in ['─', '│', '┼', '╭', '╳', '▀', '█', '░', '▟', '\u{E0B0}', '\u{E0BF}'] {
            assert!(is_drawn(c), "{c}");
        }
        for c in ['a', '■', '\u{E0C0}', '\u{E5FF}'] {
            assert!(!is_drawn(c), "{c}");
        }
        for (w, h) in CELLS {
            for code in (0x2500..=0x259F).chain(0xE0B0..=0xE0BF) {
                let c = char::from_u32(code).unwrap();
                let m = render(c, w, h).unwrap_or_else(|| panic!("{c} sem desenho"));
                assert!(m.data.iter().any(|a| *a > 0), "{c} vazio em {w}x{h}");
            }
        }
    }

    #[test]
    fn linhas_encontram_as_das_celulas_vizinhas() {
        // Toda linha que sai pela direita de uma célula entra pela esquerda da seguinte
        // nos mesmos pixels (e o mesmo de cima para baixo), para qualquer par de
        // caracteres com braço naquele lado e do mesmo peso.
        for (w, h) in CELLS {
            for weight_group in [
                &['─', '┌', '└', '├', '┬', '┴', '┼', '╶', '╭', '╰'][..],
                &['━', '┏', '┗', '┣', '┳', '┻', '╋', '╺'][..],
                &['═', '╔', '╚', '╠', '╦', '╩', '╬'][..],
            ] {
                let right: Vec<Vec<u32>> = weight_group.iter().map(|c| edge_rows(&render(*c, w, h).unwrap(), true)).collect();
                let left_side: Vec<char> = weight_group
                    .iter()
                    .map(|c| match c {
                        '┌' | '╶' | '╭' | '╰' | '└' | '├' => '─',
                        '┏' | '┗' | '┣' | '╺' => '━',
                        '╔' | '╚' | '╠' => '═',
                        other => *other,
                    })
                    .collect();
                for (i, r) in right.iter().enumerate() {
                    assert!(!r.is_empty(), "{} sem linha à direita em {w}x{h}", weight_group[i]);
                    for l in &left_side {
                        let left = edge_rows(&render(*l, w, h).unwrap(), false);
                        assert_eq!(r, &left, "{} → {l} em {w}x{h}", weight_group[i]);
                    }
                }
            }
            for group in [&['│', '┌', '┐', '├', '┤', '┬', '┼', '╷', '╭', '╮'][..], &['║', '╔', '╗', '╠', '╣', '╦', '╬'][..]] {
                let reference = edge_cols(&render(group[0], w, h).unwrap(), false);
                for c in group {
                    let bottom = edge_cols(&render(*c, w, h).unwrap(), true);
                    assert_eq!(bottom, reference, "{c} embaixo em {w}x{h}");
                }
            }
        }
    }

    #[test]
    fn linhas_atravessam_a_celula_sem_falha() {
        for (w, h) in CELLS {
            let m = render('─', w, h).unwrap();
            let y = edge_rows(&m, false)[0];
            assert!(row(&m, y).iter().all(|f| *f), "─ com falha em {w}x{h}");
            let m = render('│', w, h).unwrap();
            let x = edge_cols(&m, false)[0];
            assert!(col(&m, x).iter().all(|f| *f), "│ com falha em {w}x{h}");
            // O cruzamento junta os quatro braços no centro.
            let m = render('┼', w, h).unwrap();
            assert!(row(&m, y).iter().all(|f| *f) && col(&m, x).iter().all(|f| *f), "┼ em {w}x{h}");
            // ┌: a linha de baixo chega à linha horizontal (canto fechado).
            let m = render('┌', w, h).unwrap();
            assert!(filled(&m, x, y), "┌ sem canto em {w}x{h}");
            assert!(!filled(&m, 0, y + 3.min(h - y - 1)) || h < 4);
            // ╬: quatro cantos e o meio vazio.
            let m = render('╬', w, h).unwrap();
            let t = light_width(w);
            let (cx, cy) = (band(w, 3 * t).0 as u32 + t, band(h, 3 * t).0 as u32 + t);
            assert!(!filled(&m, cx, cy), "╬ com o meio cheio em {w}x{h}");
            assert!(filled(&m, cx - 1, 0) && filled(&m, 0, cy - 1), "╬ sem as linhas em {w}x{h}");
        }
    }

    #[test]
    fn duplas_tem_duas_linhas() {
        for (w, h) in CELLS {
            let m = render('═', w, h).unwrap();
            let rows = edge_rows(&m, false);
            let t = light_width(w);
            assert_eq!(rows.len() as u32, 2 * t, "═ em {w}x{h}");
            assert_eq!(rows[rows.len() - 1] - rows[0] + 1, 3 * t, "fresta do ═ em {w}x{h}");
            // ╔: a linha de fora vai mais longe que a de dentro.
            let m = render('╔', w, h).unwrap();
            let right = edge_rows(&m, true);
            assert_eq!(right, rows, "╔ à direita em {w}x{h}");
        }
    }

    #[test]
    fn arcos_ligam_as_bordas() {
        for (w, h) in CELLS {
            let line_row = edge_rows(&render('─', w, h).unwrap(), false);
            let line_col = edge_cols(&render('│', w, h).unwrap(), false);
            // ╭ sai pela direita na altura do ─ e por baixo na coluna do │.
            let m = render('╭', w, h).unwrap();
            let col_hit = (0..w).filter(|x| m.at(*x, h - 1) > 128).collect::<Vec<_>>();
            assert_eq!(col_hit, line_col, "╭ embaixo em {w}x{h}");
            let row_hit = (0..h).filter(|y| m.at(w - 1, *y) > 128).collect::<Vec<_>>();
            assert_eq!(row_hit, line_row, "╭ à direita em {w}x{h}");
            // ╯ sai por cima e pela esquerda.
            let m = render('╯', w, h).unwrap();
            assert_eq!((0..w).filter(|x| m.at(*x, 0) > 128).collect::<Vec<_>>(), line_col, "╯ em cima");
            assert_eq!((0..h).filter(|y| m.at(0, *y) > 128).collect::<Vec<_>>(), line_row, "╯ à esquerda");
            // Nada nos cantos de fora.
            assert_eq!(render('╭', w, h).unwrap().at(0, 0), 0);
        }
    }

    #[test]
    fn tracejados_tem_frestas() {
        for (w, h) in CELLS {
            for (c, n) in [('┄', 3), ('┈', 4), ('╌', 2)] {
                let m = render(c, w, h).unwrap();
                let y = edge_rows(&render('─', w, h).unwrap(), false)[0];
                let r = row(&m, y);
                let runs = r.windows(2).filter(|p| p[0] && !p[1]).count() + usize::from(*r.last().unwrap());
                if w >= 2 * n {
                    assert_eq!(runs, n as usize, "{c} em {w}x{h}: {r:?}");
                }
                assert!(r[0], "{c} começa na borda");
            }
        }
    }

    #[test]
    fn blocos_cobrem_a_celula() {
        for (w, h) in CELLS {
            let full = render('█', w, h).unwrap();
            assert!(full.data.iter().all(|a| *a == 255), "█ com fresta em {w}x{h}");
            // ▀ + ▄ = █, sem sobreposição; ▌ + ▐ também.
            for (a, b) in [('▀', '▄'), ('▌', '▐'), ('▘', '▟'), ('▝', '▙'), ('▖', '▜'), ('▗', '▛'), ('▚', '▞')] {
                let (ma, mb) = (render(a, w, h).unwrap(), render(b, w, h).unwrap());
                for i in 0..(w * h) as usize {
                    assert!((ma.data[i] == 255) ^ (mb.data[i] == 255), "{a}{b} no pixel {i} em {w}x{h}");
                }
            }
            // Oitavos: ▁ a ▇ crescem de baixo para cima e encostam na base.
            let mut last = 0;
            for code in 0x2581..=0x2587 {
                let m = render(char::from_u32(code).unwrap(), w, h).unwrap();
                let rows = (0..h).filter(|y| filled(&m, 0, *y)).count();
                assert!(rows >= last && filled(&m, w - 1, h - 1));
                last = rows;
            }
            // Sombreado: a célula inteira, translúcida.
            let m = render('▒', w, h).unwrap();
            assert!(m.data.iter().all(|a| (120..=135).contains(a)), "▒ em {w}x{h}");
        }
    }

    #[test]
    fn powerline_encosta_nas_bordas() {
        for (w, h) in CELLS {
            // : a coluna da esquerda inteira pintada (encosta no fundo da célula
            // anterior), a ponta no meio da borda direita e nada nos cantos da direita.
            let m = render('\u{E0B0}', w, h).unwrap();
            assert!((0..h).all(|y| m.at(0, y) == 255), " sem a borda esquerda em {w}x{h}");
            assert!(m.at(w - 1, h / 2) > 64, " sem a ponta em {w}x{h}");
            assert_eq!(m.at(w - 1, 0), 0);
            assert_eq!(m.at(w - 1, h - 1), 0);
            // : o espelho.
            let m = render('\u{E0B2}', w, h).unwrap();
            assert!((0..h).all(|y| m.at(w - 1, y) == 255), " em {w}x{h}");
            assert_eq!(m.at(0, 0), 0);
            //  e : meio-círculos da altura da célula.
            let m = render('\u{E0B4}', w, h).unwrap();
            assert!((2..h - 2).all(|y| m.at(0, y) > 128) && m.at(w - 1, h / 2) > 64);
            let m = render('\u{E0B6}', w, h).unwrap();
            assert!((2..h - 2).all(|y| m.at(w - 1, y) > 128) && m.at(0, h / 2) > 64);
            // Triângulos de canto: ◣ cheio embaixo à esquerda, vazio em cima à direita.
            let m = render('\u{E0B8}', w, h).unwrap();
            assert!(m.at(0, h - 1) > 200 && m.at(w - 1, 0) == 0);
        }
    }
}
