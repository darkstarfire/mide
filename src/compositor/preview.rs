//! Prévias das janelas de um app, mostradas acima da dock quando o ponteiro fica
//! sobre o ícone dele, como no GNOME.
//!
//! A dock pede (`preview <x> <ids>` no canal de controle) e o compositor desenha: só
//! ele tem o conteúdo das janelas. O painel fica enquanto o ponteiro estiver sobre a
//! dock ou sobre o painel. Clique numa prévia traz a janela para frente; o × da
//! prévia sob o ponteiro fecha a janela.

use smithay::{
    backend::renderer::{
        element::{
            surface::WaylandSurfaceRenderElement, utils::RescaleRenderElement, AsRenderElements,
        },
        gles::GlesRenderer,
    },
    desktop::{layer_map_for_output, Window},
    output::Output,
    utils::{Logical, Point, Rectangle, Scale},
};

use crate::{
    render::MideRenderElement,
    decorations::{self, Shaders},
    state::{window_id, State},
    title::TitleRenderer,
};

/// Caixa máxima de cada miniatura.
const THUMB_W: i32 = 200;
const THUMB_H: i32 = 128;
const GAP: i32 = 14;
const PAD: i32 = 12;
const TITLE_H: i32 = 24;
const CLOSE: i32 = 24;

/// Prévias pedidas pela dock.
#[derive(Debug, Clone, PartialEq)]
pub struct Preview {
    /// Números das janelas, na ordem da dock.
    pub ids: Vec<u64>,
    /// Centro x do ícone, em coordenadas da dock.
    pub dock_x: i32,
    /// Prévia sob o ponteiro e se o ponteiro está no × dela.
    pub hovered: Option<usize>,
    pub on_close: bool,
    /// Alternador de janelas (Alt+Tab): painel no meio da tela, a escolhida é a
    /// `hovered`, sem o × e sem sumir com o ponteiro.
    pub switcher: bool,
    /// Que alternador: todas as janelas, os apps (⌘Tab, com os ícones) ou as janelas
    /// do app em uso (⌘`).
    pub kind: SwitcherKind,
    /// Quando apareceu (entra esmaecendo, com "Transições suaves"); `None` sem transição.
    pub since: Option<std::time::Instant>,
    /// Desde quando o ponteiro está na prévia `hovered` (para espiar a janela).
    pub hover_since: Option<std::time::Instant>,
}

/// Os alternadores do teclado.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SwitcherKind {
    /// Todas as janelas (e as prévias da dock).
    #[default]
    Windows,
    /// Um app por vez, como o ⌘Tab do Mac: `ids` tem a janela de cima de cada app.
    Apps,
    /// Só as janelas do app em uso, como o ⌘` do Mac.
    AppWindows,
}

/// Lado do ícone no alternador de apps e o espaço em volta dele.
const APP_ICON: i32 = 96;
const APP_CELL: i32 = APP_ICON + 24;

/// Duração da entrada das prévias e do alternador.
pub const APPEAR: std::time::Duration = mide::motion::PREVIEW_APPEAR;

impl Preview {
    /// Quanto já apareceu (0 a 1, suavizado).
    pub fn appear(&self) -> f32 {
        let Some(since) = self.since else {
            return 1.0;
        };
        let t = (since.elapsed().as_secs_f32() / APPEAR.as_secs_f32()).clamp(0.0, 1.0);
        1.0 - (1.0 - t).powi(3)
    }

    pub fn appearing(&self) -> bool {
        self.since.is_some_and(|t| t.elapsed() < APPEAR)
    }

    /// Janela que se espia: com o ponteiro parado numa prévia da dock por `PEEK`,
    /// ela é desenhada por cima das outras (sem mudar o foco nem a ordem), para
    /// mostrar qual é. Ao sair da prévia, tudo volta como estava.
    pub fn peeking(&self) -> Option<u64> {
        if self.switcher || self.on_close || self.hover_since.is_none_or(|t| t.elapsed() < PEEK) {
            return None;
        }
        self.hovered.and_then(|i| self.ids.get(i).copied())
    }

    /// Esperando dar o tempo de espiar (pede quadros até lá).
    pub fn peek_pending(&self) -> bool {
        !self.switcher && self.hovered.is_some() && self.hover_since.is_some_and(|t| t.elapsed() < PEEK)
    }
}

/// Tempo com o ponteiro parado numa prévia até a janela dela aparecer por cima.
pub const PEEK: std::time::Duration = std::time::Duration::from_millis(400);

/// Uma miniatura posicionada.
pub struct Thumb {
    pub id: u64,
    pub window: Window,
    /// Caixa da miniatura (a janela fica centralizada nela).
    pub boxed: Rectangle<i32, Logical>,
    pub close: Rectangle<i32, Logical>,
}

/// Janela pelo número, mapeada ou minimizada.
fn window_by_id(state: &State, id: u64) -> Option<Window> {
    state
        .space
        .elements()
        .find(|w| window_id(w) == id)
        .cloned()
        .or_else(|| state.minimized.iter().find(|m| window_id(&m.window) == id).map(|m| m.window.clone()))
}

/// Retângulo da camada da dock, em coordenadas globais (de todas as telas).
fn dock_rect(state: &State, output: &Output) -> Option<Rectangle<i32, Logical>> {
    let map = layer_map_for_output(output);
    let dock = map.layers().find(|l| l.namespace() == "mide-dock")?;
    let geo = map.layer_geometry(dock)?;
    let screen = state.space.output_geometry(output)?;
    Some(Rectangle::new(geo.loc + screen.loc, geo.size))
}

/// Painel e miniaturas, em coordenadas globais.
pub fn layout(state: &State, preview: &Preview) -> Option<(Rectangle<i32, Logical>, Vec<Thumb>)> {
    let output = state.primary_output()?;
    let out = state.space.output_geometry(&output)?;
    let dock = dock_rect(state, &output)?;
    let windows: Vec<(u64, Window)> = preview.ids.iter().filter_map(|id| Some((*id, window_by_id(state, *id)?))).collect();
    if windows.is_empty() {
        return None;
    }
    let apps = preview.switcher && preview.kind == SwitcherKind::Apps;
    // Alternador de apps: células quadradas com o ícone; o das janelas, as miniaturas.
    let (base_w, base_h, gap) = if apps { (APP_CELL, APP_CELL, 4) } else { (THUMB_W, THUMB_H, GAP) };
    let n = windows.len();
    // Alternador: todas as janelas, em linhas que quebram, com o painel em no máximo
    // 90% da área útil da tela (sem a barra e a dock); se nem assim couber, as
    // miniaturas encolhem. Prévias da dock: uma linha, no tamanho normal.
    let (cols, rows, k) = if preview.switcher {
        let area = state.usable_area_of(&output).unwrap_or(out);
        let max = ((area.size.w as f32 * 0.9) as i32 - 2 * PAD, (area.size.h as f32 * 0.9) as i32 - 2 * PAD);
        grid(n, (base_w, base_h), gap, max)
    } else {
        (n, 1, 1.0)
    };
    let cell_w = (base_w as f32 * k).round() as i32;
    let cell_h = (base_h as f32 * k).round() as i32;
    let row_h = cell_h + TITLE_H;
    let w = cols as i32 * cell_w + (cols as i32 - 1) * gap + 2 * PAD;
    let h = rows as i32 * row_h + (rows as i32 - 1) * gap + 2 * PAD;
    let (x, y) = if preview.switcher {
        let area = state.usable_area_of(&output).unwrap_or(out);
        (area.loc.x + (area.size.w - w).max(0) / 2, area.loc.y + (area.size.h - h).max(0) / 2)
    } else {
        let center = dock.loc.x + preview.dock_x;
        ((center - w / 2).clamp(out.loc.x + 8, out.loc.x + out.size.w - w - 8), dock.loc.y - 10 - h)
    };
    let panel = Rectangle::new((x, y).into(), (w, h).into());
    let thumbs = windows
        .into_iter()
        .enumerate()
        .map(|(i, (id, window))| {
            let (row, col) = (i / cols, i % cols);
            // A última linha (incompleta) fica centralizada.
            let in_row = (n - row * cols).min(cols) as i32;
            let shift = (cols as i32 - in_row) * (cell_w + gap) / 2;
            let bx = x + PAD + shift + col as i32 * (cell_w + gap);
            let by = y + PAD + row as i32 * (row_h + gap);
            let boxed = Rectangle::new((bx, by).into(), (cell_w, cell_h).into());
            let close = Rectangle::new((bx + cell_w - CLOSE + 6, by - 6).into(), (CLOSE, CLOSE).into());
            Thumb { id, window, boxed, close }
        })
        .collect();
    Some((panel, thumbs))
}

/// Grade do alternador para `n` células de `cell` (sem o título embaixo) separadas
/// por `gap`, dentro de `max`: (colunas, linhas, tamanho relativo das células). Quebra
/// em linhas quando não cabe na largura e encolhe as células quando as linhas não
/// cabem na altura (até um quarto do tamanho). As linhas ficam equilibradas (7 em
/// duas linhas: 4 e 3, não 6 e 1).
fn grid(n: usize, cell: (i32, i32), gap: i32, max: (i32, i32)) -> (usize, usize, f32) {
    let n = n.max(1);
    let mut k = 1.0_f32;
    loop {
        let (cw, ch) = ((cell.0 as f32 * k).round() as i32, (cell.1 as f32 * k).round() as i32 + TITLE_H);
        let fit = ((max.0 + gap) / (cw + gap)).max(1) as usize;
        let rows = n.div_ceil(fit.min(n));
        let cols = n.div_ceil(rows);
        let height = rows as i32 * ch + (rows as i32 - 1) * gap;
        if height <= max.1 || k <= 0.25 {
            return (cols, rows, k);
        }
        k = (k - 0.05).max(0.25);
    }
}

/// O ponto está sobre o painel ou sobre a dock? (Fora dos dois, as prévias somem.)
pub fn keeps_open(state: &State, preview: &Preview, pos: Point<f64, Logical>) -> bool {
    if preview.switcher {
        return true;
    }
    let Some(output) = state.primary_output() else {
        return false;
    };
    let over_panel = layout(state, preview).is_some_and(|(panel, _)| {
        // Inclui o vão entre o painel e a dock, para o ponteiro passar de um ao outro.
        let bridge = Rectangle::new(panel.loc, (panel.size.w, panel.size.h + 12).into());
        bridge.to_f64().contains(pos)
    });
    over_panel || dock_rect(state, &output).is_some_and(|d| d.to_f64().contains(pos))
}

/// Miniatura e × sob o ponto.
pub fn hit(state: &State, preview: &Preview, pos: Point<f64, Logical>) -> (Option<usize>, bool) {
    let Some((_, thumbs)) = layout(state, preview) else {
        return (None, false);
    };
    for (i, thumb) in thumbs.iter().enumerate() {
        if preview.hovered == Some(i) && thumb.close.to_f64().contains(pos) {
            return (Some(i), true);
        }
        let area = Rectangle::new(thumb.boxed.loc, (thumb.boxed.size.w, thumb.boxed.size.h + TITLE_H).into());
        if area.to_f64().contains(pos) {
            return (Some(i), false);
        }
    }
    (None, false)
}

/// Elementos das prévias, da frente para trás.
pub fn elements(
    renderer: &mut GlesRenderer,
    state: &State,
    origin: Point<i32, Logical>,
    shaders: &Shaders,
    titles: &mut TitleRenderer,
) -> Vec<MideRenderElement> {
    let Some(preview) = state.preview.as_ref() else {
        return Vec::new();
    };
    let Some((panel, mut thumbs)) = layout(state, preview) else {
        return Vec::new();
    };
    // Entrada: sobe 8 px e esmaece (com "Transições suaves").
    let e = preview.appear();
    let lift = Point::<i32, Logical>::from((0, (8.0 * (1.0 - e)).round() as i32));
    let fade = |c: [f32; 4]| [c[0], c[1], c[2], c[3] * e];
    // Segue o tema (claro/escuro), como os menus do compositor: o título já vem
    // em letra clara no escuro e sumia no painel claro.
    let dark = decorations::dark();
    // Desenho relativo à tela da dock.
    let panel = Rectangle::new(panel.loc - origin + lift, panel.size);
    for thumb in &mut thumbs {
        thumb.boxed.loc = thumb.boxed.loc - origin + lift;
        thumb.close.loc = thumb.close.loc - origin + lift;
    }
    let mut front: Vec<MideRenderElement> = Vec::new();
    let mut middle: Vec<MideRenderElement> = Vec::new();
    if preview.switcher && preview.kind == SwitcherKind::Apps {
        app_cells(renderer, preview, &thumbs, panel, dark, e, shaders, titles, &mut middle);
    }
    for (i, thumb) in thumbs.iter().enumerate() {
        if preview.switcher && preview.kind == SwitcherKind::Apps {
            break;
        }
        let hovered = preview.hovered == Some(i);
        // × no canto da prévia sob o ponteiro.
        // Botão de fechar no canto da prévia sob o ponteiro, como no GNOME: círculo
        // escuro com × branco (vermelho com o ponteiro em cima).
        if hovered && !preview.switcher {
            let glyph = Rectangle::new(thumb.close.loc + Point::from((7, 7)), (CLOSE - 14, CLOSE - 14).into());
            front.push(MideRenderElement::from(decorations::close_glyph_colored(shaders, glyph, [1.0, 1.0, 1.0, 1.0])));
            let fill = if preview.on_close { [0.92, 0.26, 0.24, 1.0] } else { [0.16, 0.16, 0.18, 0.88] };
            front.push(MideRenderElement::from(decorations::rounded_rect_element(shaders, thumb.close, fill, CLOSE / 2)));
            // Contorno claro fino, para o botão se destacar sobre janelas escuras.
            let ring = Rectangle::new(thumb.close.loc - Point::from((1, 1)), (CLOSE + 2, CLOSE + 2).into());
            front.push(MideRenderElement::from(decorations::rounded_rect_element(shaders, ring, [1.0, 1.0, 1.0, 0.7], CLOSE / 2 + 1)));
        }
        // Título embaixo da miniatura.
        let title = crate::render::window_title(&thumb.window);
        let (tw, th) = (thumb.boxed.size.w, thumb.boxed.size.h);
        let title_bar = Rectangle::new((thumb.boxed.loc.x - 40, thumb.boxed.loc.y + th).into(), (tw + 80, TITLE_H).into());
        if let Some(el) = crate::title::element(renderer, titles, &title, true, title_bar, 1.0) {
            middle.push(MideRenderElement::from(el));
        }
        // A janela reduzida, centralizada na caixa. Janelas com a barra do mide
        // ganham uma barra em miniatura em cima, para parecer a janela de verdade.
        let geo = thumb.window.geometry();
        let bar_h = crate::state::bar_height_for(&thumb.window);
        if geo.size.w > 0 && geo.size.h > 0 {
            let full_h = geo.size.h + bar_h;
            let scale = (tw as f64 / geo.size.w as f64).min(th as f64 / full_h as f64).min(1.0);
            let shown = (geo.size.w as f64 * scale, full_h as f64 * scale);
            let frame_x = thumb.boxed.loc.x as f64 + (tw as f64 - shown.0) / 2.0;
            let frame_y = thumb.boxed.loc.y as f64 + (th as f64 - shown.1) / 2.0;
            let bar_shown = bar_h as f64 * scale;
            let target = (frame_x, frame_y + bar_shown);
            // A superfície começa antes da área visível (sombras do app): desloca.
            // Em pixels da tela (vezes a escala dela).
            let s = crate::clip::scale();
            let origin: Point<i32, smithay::utils::Physical> = (
                ((target.0 - geo.loc.x as f64 * scale) * s).round() as i32,
                ((target.1 - geo.loc.y as f64 * scale) * s).round() as i32,
            )
                .into();
            let content: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
                thumb.window.render_elements(renderer, origin, Scale::from(s), e);
            for element in content {
                middle.push(MideRenderElement::Scaled(RescaleRenderElement::from_element(element, origin, scale)));
            }
            if bar_h > 0 {
                let bar = Rectangle::new(
                    (frame_x.round() as i32, frame_y.round() as i32).into(),
                    (shown.0.round() as i32, bar_shown.round().max(1.0) as i32).into(),
                );
                middle.push(MideRenderElement::from(decorations::rounded_rect_element(shaders, bar, fade(if dark { [0.24, 0.24, 0.26, 1.0] } else { [0.90, 0.90, 0.91, 1.0] }), 3)));
            }
        }
        // Prévia sob o ponteiro: vidro claro, como a seleção do launcher (com o vidro
        // desligado nas Configurações, só um véu branco). Um fundo escuro aqui fazia as
        // bordas claras das letras do título aparecerem como pontinhos brancos.
        if hovered {
            let glow = Rectangle::new(thumb.boxed.loc - Point::from((6, 6)), (tw + 12, th + TITLE_H + 8).into());
            // Alternador (Alt+Tab): a escolhida na cor de destaque do sistema (Aparência),
            // com o fundo translúcido e o contorno na cor cheia: o título segue legível.
            if preview.switcher {
                let [r, g, b, _] = mide::kit::system_accent(dark);
                let accent = |a: f32| [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, a];
                middle.push(MideRenderElement::from(decorations::stroke_element(shaders, glow, 10, accent(1.0))));
                middle.push(MideRenderElement::from(decorations::rounded_rect_element(shaders, glow, accent(if dark { 0.45 } else { 0.30 }), 10)));
                continue;
            }
            // O vidro é branco (62%): no escuro, a letra clara do título sumiria nele.
            let element = if state.config.effects.panel_glass && !dark {
                decorations::glass_element(shaders, glow, 10)
            } else {
                decorations::rounded_rect_element(shaders, glow, [1.0, 1.0, 1.0, if dark { 0.12 } else { 0.6 }], 10)
            };
            middle.push(MideRenderElement::from(element));
        }
    }
    let mut out = front;
    out.extend(middle);
    // Painel translúcido, como os menus do mide, com um contorno fino.
    let fill = if dark { [44.0 / 255.0, 44.0 / 255.0, 48.0 / 255.0, 0.95] } else { [0.965, 0.965, 0.972, 0.95] };
    out.push(MideRenderElement::from(decorations::rounded_rect_element(shaders, panel, fade(decorations::tinted(fill)), 14)));
    let border = Rectangle::new(panel.loc - Point::from((1, 1)), (panel.size.w + 2, panel.size.h + 2).into());
    out.push(MideRenderElement::from(decorations::rounded_rect_element(shaders, border, fade(if dark { [1.0, 1.0, 1.0, 0.14] } else { [0.0, 0.0, 0.0, 0.16] }), 15)));
    out
}

/// Alternador de apps (⌘Tab), como no Mac: o ícone grande de cada app, o escolhido
/// num quadrado arredondado translúcido e o nome dele embaixo.
#[allow(clippy::too_many_arguments)]
fn app_cells(
    renderer: &mut GlesRenderer,
    preview: &Preview,
    thumbs: &[Thumb],
    panel: Rectangle<i32, Logical>,
    dark: bool,
    alpha: f32,
    shaders: &Shaders,
    titles: &mut TitleRenderer,
    out: &mut Vec<MideRenderElement>,
) {
    use smithay::backend::renderer::element::{memory::MemoryRenderBufferRenderElement, Kind};
    let scale = crate::clip::scale().ceil().max(1.0) as i32;
    for (i, thumb) in thumbs.iter().enumerate() {
        let app = crate::render::app_id_of(&thumb.window);
        // Ícone na proporção da célula (ela encolhe com muitos apps).
        let side = (thumb.boxed.size.w * APP_ICON / APP_CELL).max(8);
        let pad = (thumb.boxed.size.w - side) / 2;
        let at = thumb.boxed.loc + Point::from((pad, pad));
        let location = crate::clip::phys(at).to_f64();
        APP_ICONS.with(|icons| {
            let mut icons = icons.borrow_mut();
            let buffer = icons.entry((app.to_lowercase(), scale * side)).or_insert_with(|| app_icon(&app, scale, side));
            if let Some(buffer) = buffer
                && let Ok(el) = MemoryRenderBufferRenderElement::from_buffer(renderer, location, buffer, Some(alpha), None, None, Kind::Unspecified)
            {
                out.push(MideRenderElement::Title(el));
            }
        });
        if preview.hovered == Some(i) {
            // Nome do app embaixo do escolhido (o da janela, sem `app_id`).
            let name = app_name(&app).unwrap_or_else(|| crate::render::window_title(&thumb.window));
            let bar = Rectangle::new((panel.loc.x, thumb.boxed.loc.y + thumb.boxed.size.h + 2).into(), (panel.size.w, TITLE_H).into());
            if let Some(el) = crate::title::element(renderer, titles, &name, true, bar, alpha) {
                out.push(MideRenderElement::from(el));
            }
            let fill = if dark { [1.0, 1.0, 1.0, 0.16] } else { [0.0, 0.0, 0.0, 0.12] };
            out.push(MideRenderElement::from(decorations::rounded_rect_element(shaders, thumb.boxed, [fill[0], fill[1], fill[2], fill[3] * alpha], 18)));
        }
    }
}

thread_local! {
    /// Ícones dos apps no alternador, por `app_id` e tamanho em pixels.
    static APP_ICONS: std::cell::RefCell<std::collections::HashMap<(String, i32), Option<smithay::backend::renderer::element::memory::MemoryRenderBuffer>>> = Default::default();
    /// Apps instalados (nome e ícone de cada `.desktop`), lidos uma vez.
    static INSTALLED: Vec<(String, String, Option<String>)> = mide::apps::load_handlers()
        .into_iter()
        .map(|a| (a.id.to_lowercase(), a.name, a.icon))
        .collect();
}

/// O `.desktop` de um `app_id`: pelo id inteiro ou pela última parte
/// (`org.gnome.Calculator` casa com `gnome-calculator`? não; com `calculator`, sim).
fn installed_for<T>(app_id: &str, pick: impl Fn(&(String, String, Option<String>)) -> Option<T>) -> Option<T> {
    let id = app_id.to_lowercase();
    if id.is_empty() {
        return None;
    }
    let last = |s: &str| s.rsplit('.').next().unwrap_or(s).to_string();
    INSTALLED.with(|apps| {
        apps.iter()
            .find(|a| a.0 == id)
            .or_else(|| apps.iter().find(|a| last(&a.0) == last(&id)))
            .and_then(&pick)
    })
}

/// Nome do app (do `.desktop`), para mostrar embaixo do ícone escolhido.
fn app_name(app_id: &str) -> Option<String> {
    installed_for(app_id, |a| Some(a.1.clone()))
}

/// Ícone do app no tamanho do alternador, nos pixels da tela.
fn app_icon(app_id: &str, scale: i32, side: i32) -> Option<smithay::backend::renderer::element::memory::MemoryRenderBuffer> {
    use smithay::backend::allocator::Fourcc;
    use smithay::utils::Transform;
    let px = (side * scale) as u32;
    let icon = installed_for(app_id, |a| a.2.clone());
    let lower = app_id.to_lowercase();
    let mut names: Vec<&str> = Vec::new();
    if let Some(icon) = icon.as_deref() {
        names.push(icon);
    }
    names.extend([lower.as_str(), "application-x-executable", "applications-other"]);
    let pixmap = mide::icons::load_themed_icon(&names, px)?;
    let (w, h) = (pixmap.width() as i32, pixmap.height() as i32);
    Some(smithay::backend::renderer::element::memory::MemoryRenderBuffer::from_slice(pixmap.data(), Fourcc::Abgr8888, (w, h), scale, Transform::Normal, None))
}

#[cfg(test)]
mod tests {
    use super::grid;

    #[test]
    fn grade_quebra_linhas_e_encolhe() {
        // Cabem 5 por linha: 12 viram 3 linhas de 4 (equilibradas).
        assert_eq!(grid(12, (200, 128), 14, (1100, 700)), (4, 3, 1.0));
        // 7 em duas linhas: 4 e 3.
        assert_eq!(grid(7, (200, 128), 14, (1100, 700)).0, 4);
        // Poucas: uma linha só.
        assert_eq!(grid(3, (200, 128), 14, (1100, 700)), (3, 1, 1.0));
        // Muitas para a altura: encolhem e continuam dentro dela.
        let (cols, rows, k) = grid(60, (200, 128), 14, (1100, 600));
        assert!(k < 1.0 && cols * rows >= 60);
        let row_h = (128.0 * k).round() as i32 + super::TITLE_H;
        assert!(rows as i32 * row_h + (rows as i32 - 1) * 14 <= 600);
    }
}
