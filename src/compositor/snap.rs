//! Encaixe de janelas pelo topo da tela (como no Zorin, no ChromeOS e no Windows 11).
//!
//! Arrastando uma janela, quando o ponteiro encosta na barra superior:
//! - no terço esquerdo da tela, a janela vai para a metade esquerda;
//! - no terço direito, para a metade direita;
//! - no terço do meio, para a tela inteira, e abre o painel de layouts (metades,
//!   2/3 + 1/3, terços, metade + quartos, quartos): soltar sobre uma célula põe a
//!   janela ali.
//!
//! Enquanto arrasta, uma prévia translúcida na cor de destaque mostra onde a janela vai
//! ficar. O desenho é todo do compositor (como as prévias da dock): sem cliente e sem
//! foco. Esc ou sair da zona cancela.

use std::time::Instant;

use smithay::{
    desktop::Window,
    utils::{Logical, Point, Rectangle},
};

use crate::{
    decorations::{self, Shaders},
    render::MideRenderElement,
    state::State,
};

pub type Rect = Rectangle<i32, Logical>;

/// Layouts do painel: cada célula em frações (x, y, largura, altura) da área útil.
pub const LAYOUTS: &[&[(f32, f32, f32, f32)]] = &[
    &[(0.0, 0.0, 0.5, 1.0), (0.5, 0.0, 0.5, 1.0)],
    &[(0.0, 0.0, 2.0 / 3.0, 1.0), (2.0 / 3.0, 0.0, 1.0 / 3.0, 1.0)],
    &[(0.0, 0.0, 1.0 / 3.0, 1.0), (1.0 / 3.0, 0.0, 1.0 / 3.0, 1.0), (2.0 / 3.0, 0.0, 1.0 / 3.0, 1.0)],
    &[(0.0, 0.0, 0.5, 1.0), (0.5, 0.0, 0.5, 0.5), (0.5, 0.5, 0.5, 0.5)],
    &[(0.0, 0.0, 0.5, 0.5), (0.5, 0.0, 0.5, 0.5), (0.0, 0.5, 0.5, 0.5), (0.5, 0.5, 0.5, 0.5)],
];

/// Miniatura de cada layout no painel, espaço entre elas e margem do painel.
const MINI_W: i32 = 76;
const MINI_H: i32 = 48;
const GAP: i32 = 10;
const PAD: i32 = 12;
/// Distância do painel até o topo da área útil.
const PANEL_TOP: i32 = 10;
/// O ponteiro "encosta" na barra: até estes pontos abaixo do topo da área útil.
const EDGE: f64 = 2.0;

/// Para onde a janela vai.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zone {
    Left,
    Max,
    Right,
    /// Layout e célula do painel.
    Cell(usize, usize),
}

/// Encaixe em andamento (durante um arrasto).
#[derive(Debug, Clone)]
pub struct Snap {
    /// Área útil da tela onde o encaixe vale.
    pub area: Rect,
    pub zone: Option<Zone>,
    /// Painel de layouts aberto (o ponteiro passou pelo meio do topo).
    pub panel: bool,
    /// Quando o painel abriu (entra esmaecendo).
    pub since: Instant,
}

/// Painel de layouts: centrado no topo da área útil.
pub fn panel_rect(area: Rect) -> Rect {
    let n = LAYOUTS.len() as i32;
    let w = n * MINI_W + (n - 1) * GAP + 2 * PAD;
    let h = MINI_H + 2 * PAD;
    Rectangle::new((area.loc.x + (area.size.w - w) / 2, area.loc.y + PANEL_TOP).into(), (w, h).into())
}

/// Miniatura do layout `i` dentro do painel.
fn mini_rect(panel: Rect, i: usize) -> Rect {
    Rectangle::new((panel.loc.x + PAD + i as i32 * (MINI_W + GAP), panel.loc.y + PAD).into(), (MINI_W, MINI_H).into())
}

/// Parte de `r` dada pelas frações, com `gap` pontos de folga entre as partes.
fn part(r: Rect, (fx, fy, fw, fh): (f32, f32, f32, f32), gap: i32) -> Rect {
    let x0 = r.loc.x + (r.size.w as f32 * fx).round() as i32;
    let y0 = r.loc.y + (r.size.h as f32 * fy).round() as i32;
    let x1 = r.loc.x + (r.size.w as f32 * (fx + fw)).round() as i32;
    let y1 = r.loc.y + (r.size.h as f32 * (fy + fh)).round() as i32;
    let g = |edge: bool| if edge { 0 } else { gap / 2 };
    let (l, t) = (g(fx <= 0.0), g(fy <= 0.0));
    let (rr, b) = (g(fx + fw >= 0.999), g(fy + fh >= 0.999));
    Rectangle::new((x0 + l, y0 + t).into(), ((x1 - x0 - l - rr).max(1), (y1 - y0 - t - b).max(1)).into())
}

/// Célula do painel sob `pos`.
fn cell_at(panel: Rect, pos: Point<f64, Logical>) -> Option<(usize, usize)> {
    for (i, cells) in LAYOUTS.iter().enumerate() {
        let mini = mini_rect(panel, i);
        for (j, frac) in cells.iter().enumerate() {
            if part(mini, *frac, 4).to_f64().contains(pos) {
                return Some((i, j));
            }
        }
    }
    None
}

/// Área da tela onde a janela fica em cada zona.
pub fn target(area: Rect, zone: Zone) -> Rect {
    match zone {
        Zone::Left => part(area, (0.0, 0.0, 0.5, 1.0), 0),
        Zone::Right => part(area, (0.5, 0.0, 0.5, 1.0), 0),
        Zone::Max => area,
        Zone::Cell(i, j) => LAYOUTS.get(i).and_then(|cells| cells.get(j)).map(|f| part(area, *f, 0)).unwrap_or(area),
    }
}

/// Ponteiro andando durante o arrasto de uma janela: liga, troca ou desliga o encaixe.
pub fn update(state: &mut State, pos: Point<f64, Logical>) {
    let before = state.snap.as_ref().map(|s| (s.zone, s.panel));
    let Some(area) = state.output_at(pos).and_then(|o| state.usable_area_of(&o)) else {
        state.snap = None;
        return;
    };
    let mut panel = state.snap.as_ref().is_some_and(|s| s.panel && s.area == area);
    let top = area.loc.y as f64;
    let zone = if panel && cell_at(panel_rect(area), pos).is_some() {
        cell_at(panel_rect(area), pos).map(|(i, j)| Zone::Cell(i, j))
    } else if pos.y <= top + EDGE {
        let third = area.size.w as f64 / 3.0;
        let x = pos.x - area.loc.x as f64;
        if x < third {
            Some(Zone::Left)
        } else if x > 2.0 * third {
            Some(Zone::Right)
        } else {
            panel = true;
            Some(Zone::Max)
        }
    } else if panel && {
        // Descendo do topo até o painel: ele continua aberto, sem zona escolhida.
        let p = panel_rect(area);
        let reach = Rectangle::<i32, Logical>::new((p.loc.x - 24, area.loc.y).into(), (p.size.w + 48, p.loc.y - area.loc.y + p.size.h + 24).into());
        reach.to_f64().contains(pos)
    } {
        None
    } else {
        panel = false;
        None
    };
    if zone.is_none() && !panel {
        state.snap = None;
    } else {
        let since = state.snap.as_ref().filter(|s| s.panel).map(|s| s.since).unwrap_or_else(Instant::now);
        state.snap = Some(Snap { area, zone, panel, since });
    }
    if before != state.snap.as_ref().map(|s| (s.zone, s.panel)) {
        if let Some(zone) = state.snap.as_ref().and_then(|s| s.zone) {
            tracing::debug!("encaixe: {zone:?}");
        }
        state.wake();
    }
}

/// Soltou a janela: encaixa onde a prévia mostrava. Devolve se encaixou.
pub fn finish(state: &mut State, window: &Window) -> bool {
    let Some(snap) = state.snap.take() else {
        return false;
    };
    state.wake();
    let (Some(zone), Some(toplevel)) = (snap.zone, window.toplevel().cloned()) else {
        return false;
    };
    let rect = target(snap.area, zone);
    tracing::info!("encaixe: janela em {zone:?} ({}x{} em {:?})", rect.size.w, rect.size.h, rect.loc);
    match zone {
        Zone::Max => state.maximize_in(&toplevel, snap.area),
        _ => state.snap_to(&toplevel, rect, rect.loc.x == snap.area.loc.x),
    }
    true
}

/// Desenho do encaixe numa tela: o painel (na frente de tudo) e a prévia (na frente
/// das janelas, atrás da barra e da dock). Coordenadas da tela desenhada.
pub fn elements(state: &State, output: &smithay::output::Output, shaders: &Shaders) -> (Vec<MideRenderElement>, Vec<MideRenderElement>) {
    let (mut front, mut back) = (Vec::new(), Vec::new());
    let Some(snap) = state.snap.as_ref() else {
        return (front, back);
    };
    let Some(geo) = state.space.output_geometry(output) else {
        return (front, back);
    };
    if !geo.overlaps(snap.area) {
        return (front, back);
    }
    let local = |r: Rect| Rectangle::new(r.loc - geo.loc, r.size);
    let dark = state.config.dark;
    let [ar, ag, ab, _] = mide::kit::system_accent(dark);
    let accent = |a: f32| [ar as f32 / 255.0, ag as f32 / 255.0, ab as f32 / 255.0, a];
    // Painel de layouts, entrando esmaecido.
    if snap.panel {
        let t = if state.config.effects.transitions { (snap.since.elapsed().as_secs_f32() / 0.15).min(1.0) } else { 1.0 };
        let fade = |c: [f32; 4]| [c[0], c[1], c[2], c[3] * t];
        let panel = panel_rect(snap.area);
        for (i, cells) in LAYOUTS.iter().enumerate() {
            let mini = mini_rect(panel, i);
            for (j, frac) in cells.iter().enumerate() {
                let cell = part(mini, *frac, 4);
                let on = snap.zone == Some(Zone::Cell(i, j));
                let color = if on {
                    accent(1.0)
                } else if dark {
                    [1.0, 1.0, 1.0, 0.22]
                } else {
                    [0.0, 0.0, 0.0, 0.14]
                };
                front.push(MideRenderElement::from(decorations::rounded_rect_element(shaders, local(cell), fade(color), 4)));
            }
        }
        let stroke = if dark { [1.0, 1.0, 1.0, 0.14] } else { [0.0, 0.0, 0.0, 0.12] };
        front.push(MideRenderElement::from(decorations::stroke_element(shaders, local(panel), 16, fade(stroke))));
        let veil = if dark { [36.0 / 255.0, 36.0 / 255.0, 40.0 / 255.0, 0.94] } else { [248.0 / 255.0, 248.0 / 255.0, 250.0 / 255.0, 0.94] };
        front.push(MideRenderElement::from(decorations::rounded_rect_element(shaders, local(panel), fade(veil), 16)));
    }
    // Prévia de onde a janela vai ficar, com folga das bordas.
    if let Some(zone) = snap.zone {
        let r = target(snap.area, zone);
        let inset = Rectangle::<i32, Logical>::new((r.loc.x + 6, r.loc.y + 6).into(), ((r.size.w - 12).max(1), (r.size.h - 12).max(1)).into());
        back.push(MideRenderElement::from(decorations::stroke_element(shaders, local(inset), 14, accent(0.85))));
        back.push(MideRenderElement::from(decorations::rounded_rect_element(shaders, local(inset), accent(0.22), 14)));
    }
    (front, back)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area() -> Rect {
        Rectangle::new((0, 28).into(), (1366, 673).into())
    }

    #[test]
    fn zonas_viram_areas() {
        assert_eq!(target(area(), Zone::Left), Rectangle::new((0, 28).into(), (683, 673).into()));
        assert_eq!(target(area(), Zone::Right), Rectangle::new((683, 28).into(), (683, 673).into()));
        assert_eq!(target(area(), Zone::Max), area());
        // Quartos: o de baixo à direita.
        assert_eq!(target(area(), Zone::Cell(4, 3)), Rectangle::new((683, 365).into(), (683, 336).into()));
        // Terços: o do meio.
        assert_eq!(target(area(), Zone::Cell(2, 1)), Rectangle::new((455, 28).into(), (456, 673).into()));
    }

    #[test]
    fn celula_sob_o_ponteiro() {
        let panel = panel_rect(area());
        // Centro da primeira célula do primeiro layout (metade esquerda).
        let mini = mini_rect(panel, 0);
        let p = Point::from(((mini.loc.x + 10) as f64, (mini.loc.y + 10) as f64));
        assert_eq!(cell_at(panel, p), Some((0, 0)));
        // Fora das miniaturas.
        assert_eq!(cell_at(panel, Point::from((panel.loc.x as f64 + 2.0, panel.loc.y as f64 + 2.0))), None);
    }
}
