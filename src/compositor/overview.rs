//! Visão de todas as janelas (como o Mission Control do macOS): as janelas da tela
//! saem do lugar e vão para um mosaico, sem se sobrepor, com o título embaixo de cada
//! uma e o fundo escurecido. Abre com quatro dedos para cima no touchpad (acompanhando
//! os dedos) ou pelo atalho (`overview`, Super+W); clicar numa janela traz ela para a
//! frente e fecha a visão; Esc ou clique no vazio fecham sem mudar o foco.
//!
//! As janelas são desenhadas pelo caminho de sempre (`render::window_elements`), com
//! a posição e o zoom interpolados entre o lugar delas e o do mosaico (`place`), como
//! a animação de maximizar: nada é fotografado a cada quadro.

use std::time::Instant;

use smithay::{
    backend::renderer::gles::GlesRenderer,
    desktop::{layer_map_for_output, Window},
    output::Output,
    utils::{Logical, Point, Rectangle},
};

use crate::{
    decorations,
    render::MideRenderElement,
    state::{bar_height_for, window_id, State, ANIMATION_DURATION},
    title::TitleRenderer,
};

/// Margem do mosaico até a área livre da tela e espaço entre as janelas.
const MARGIN: i32 = 40;
const GAP: i32 = 32;
/// Faixa do título embaixo de cada janela.
const TITLE_H: i32 = 30;
const PILL_H: i32 = 22;

/// Uma janela no mosaico.
#[derive(Debug, Clone)]
pub struct Slot {
    pub id: u64,
    /// Canto do conteúdo (abaixo da barra do mide) no mosaico, em coordenadas globais.
    pub content: Point<i32, Logical>,
    /// Zoom do conteúdo no mosaico.
    pub scale: f64,
    /// Moldura inteira (barra + conteúdo) no mosaico, em coordenadas globais.
    pub frame: Rectangle<i32, Logical>,
}

/// Visão aberta (ou abrindo/fechando) numa tela.
#[derive(Debug, Clone)]
pub struct Overview {
    pub output: String,
    pub slots: Vec<Slot>,
    /// Acompanhando os dedos: quanto já abriu (0 a 1).
    pub drag: Option<f32>,
    /// Animação: início, de e para.
    pub anim: Option<(Instant, f32, f32)>,
    /// Onde ficou sem gesto nem animação.
    pub value: f32,
    pub hovered: Option<u64>,
}

impl Overview {
    /// Quanto da visão já apareceu (0: as janelas no lugar; 1: o mosaico).
    pub fn progress(&self) -> f32 {
        if let Some(p) = self.drag {
            return p;
        }
        match self.anim {
            Some((start, from, to)) => {
                let t = (start.elapsed().as_secs_f32() / ANIMATION_DURATION.as_secs_f32()).clamp(0.0, 1.0);
                from + (to - from) * (1.0 - (1.0 - t).powi(3))
            }
            None => self.value,
        }
    }

    /// Aberta de vez (não abrindo nem fechando): clique e ponteiro valem.
    pub fn settled_open(&self) -> bool {
        self.drag.is_none() && self.anim.is_none() && self.value >= 1.0
    }
}

/// Janelas que entram no mosaico: as comuns da tela (sem pai, fora da tela cheia),
/// da de cima para a de baixo.
fn windows_on(state: &State, output: &Output) -> Vec<Window> {
    let Some(geo) = state.space.output_geometry(output) else {
        return Vec::new();
    };
    state
        .space
        .elements()
        .rev()
        .filter(|w| w.toplevel().is_some_and(|t| t.parent().is_none()))
        .filter(|w| state.fullscreen_entry(w).is_none())
        .filter(|w| {
            state.space.element_location(w).is_some_and(|loc| {
                let size = w.geometry().size;
                geo.contains(loc + Point::from((size.w / 2, size.h.max(1) / 2)))
            })
        })
        .cloned()
        .collect()
}

/// Mosaico das janelas na área livre da tela (sem a barra e a dock): o número de
/// colunas que deixa as janelas maiores, linhas centradas e a ordem pela posição real
/// (de cima para baixo, da esquerda para a direita), como no Mac.
fn layout(state: &State, output: &Output, windows: &[Window]) -> Vec<Slot> {
    let Some(screen) = state.space.output_geometry(output) else {
        return Vec::new();
    };
    let zone = layer_map_for_output(output).non_exclusive_zone();
    let area = Rectangle::<i32, Logical>::new(
        (screen.loc.x + zone.loc.x + MARGIN, screen.loc.y + zone.loc.y + MARGIN).into(),
        ((zone.size.w - 2 * MARGIN).max(1), (zone.size.h - 2 * MARGIN).max(1)).into(),
    );
    let n = windows.len();
    if n == 0 {
        return Vec::new();
    }
    let sizes: Vec<(f64, f64, f64)> = windows
        .iter()
        .map(|w| {
            let g = w.geometry().size;
            (g.w.max(1) as f64, g.h.max(1) as f64, bar_height_for(w) as f64)
        })
        .collect();
    // Escala de cada janela numa célula de `cw` x `ch` (o título fica embaixo).
    let fit = |(w, h, bar): (f64, f64, f64), cw: f64, ch: f64| (cw / w).min((ch - TITLE_H as f64 - bar).max(1.0) / h).min(1.0);
    let mut best: Option<(f64, usize)> = None;
    for cols in 1..=n {
        let rows = n.div_ceil(cols);
        let cw = (area.size.w - GAP * (cols as i32 - 1)) as f64 / cols as f64;
        let ch = (area.size.h - GAP * (rows as i32 - 1)) as f64 / rows as f64;
        let covered: f64 = sizes.iter().map(|s| {
            let k = fit(*s, cw, ch);
            k * k * s.0 * s.1
        }).sum();
        if best.is_none_or(|(b, _)| covered > b) {
            best = Some((covered, cols));
        }
    }
    let cols = best.map_or(1, |(_, c)| c);
    let rows = n.div_ceil(cols);
    let cw = (area.size.w - GAP * (cols as i32 - 1)) as f64 / cols as f64;
    let ch = (area.size.h - GAP * (rows as i32 - 1)) as f64 / rows as f64;
    // Ordem pela posição real: por linha (centro y) e, dentro dela, pelo centro x.
    let mut order: Vec<usize> = (0..n).collect();
    let center = |i: usize| {
        let loc = state.space.element_location(&windows[i]).unwrap_or_default();
        (loc.x as f64 + sizes[i].0 / 2.0, loc.y as f64 + sizes[i].1 / 2.0)
    };
    order.sort_by(|a, b| center(*a).1.total_cmp(&center(*b).1));
    let mut slots = Vec::with_capacity(n);
    for (row, chunk) in order.chunks(cols).enumerate() {
        let mut chunk = chunk.to_vec();
        chunk.sort_by(|a, b| center(*a).0.total_cmp(&center(*b).0));
        // Linha incompleta: centrada.
        let used = chunk.len() as f64 * cw + GAP as f64 * (chunk.len() as f64 - 1.0);
        let left = area.loc.x as f64 + (area.size.w as f64 - used) / 2.0;
        for (col, &i) in chunk.iter().enumerate() {
            let k = fit(sizes[i], cw, ch);
            let (w, h, bar) = sizes[i];
            let fw = w * k;
            let fh = bar + h * k;
            let cell_x = left + col as f64 * (cw + GAP as f64);
            let cell_y = area.loc.y as f64 + row as f64 * (ch + GAP as f64);
            let x = (cell_x + (cw - fw) / 2.0).round() as i32;
            let y = (cell_y + (ch - TITLE_H as f64 - fh) / 2.0).round() as i32;
            slots.push(Slot {
                id: window_id(&windows[i]),
                content: (x, y + bar as i32).into(),
                scale: k,
                frame: Rectangle::new((x, y).into(), (fw.round() as i32, fh.round() as i32).into()),
            });
        }
    }
    slots
}

/// Abre a visão na tela `output`, já no ponto `p` (0 quando os dedos começam).
/// Com `animate`, anima até aberta.
pub fn open(state: &mut State, output: &Output, p: Option<f32>) {
    let windows = windows_on(state, output);
    let slots = layout(state, output, &windows);
    tracing::info!("visão das janelas: abrindo em {} ({} janela(s))", output.name(), slots.len());
    for slot in &slots {
        tracing::debug!("visão das janelas: janela {} em {:?} (zoom {:.2})", slot.id, slot.frame, slot.scale);
    }
    // As prévias da dock e o alternador saem (a visão é a mesma ideia, maior).
    state.preview = None;
    let animate = state.config.effects.animations;
    let anim = (p.is_none() && animate).then(|| (Instant::now(), 0.0, 1.0));
    let value = if p.is_none() && !animate { 1.0 } else { 0.0 };
    state.overview = Some(Overview { output: output.name(), slots, drag: p, anim, value, hovered: None });
    state.wake();
}

/// Abre ou fecha (atalho).
pub fn toggle(state: &mut State) {
    match &state.overview {
        Some(o) if o.anim.is_none_or(|(_, _, to)| to > 0.5) => close(state, None),
        _ => {
            let Some(output) = state.output_at(state.pointer_location) else {
                return;
            };
            open(state, &output, None);
        }
    }
}

/// Fecha a visão, trazendo `focus` para a frente antes (a janela volta ao lugar dela
/// por cima das outras).
pub fn close(state: &mut State, focus: Option<u64>) {
    if let Some(id) = focus {
        tracing::info!("visão das janelas: janela {id} escolhida");
        crate::control::handle(state, &format!("focus {id}"));
    } else {
        tracing::info!("visão das janelas: fechando");
    }
    let animate = state.config.effects.animations;
    if let Some(o) = state.overview.as_mut() {
        let p = o.progress();
        o.drag = None;
        o.hovered = None;
        if animate && p > 0.0 {
            o.anim = Some((Instant::now(), p, 0.0));
        } else {
            state.overview = None;
            tracing::info!("visão das janelas: fechada");
        }
    }
    state.wake();
}

/// Dedos acompanhados: a visão no ponto `p`.
pub fn drag(state: &mut State, p: f32) {
    if let Some(o) = state.overview.as_mut() {
        o.drag = Some(p.clamp(0.0, 1.0));
        o.anim = None;
        state.wake();
    }
}

/// Dedos soltos em `p`: termina de abrir ou de fechar a partir dali.
pub fn settle(state: &mut State, open: bool) {
    let animate = state.config.effects.animations;
    let Some(o) = state.overview.as_mut() else {
        return;
    };
    let p = o.drag.take().unwrap_or(o.value);
    tracing::info!("gesto: visão das janelas {} (em {:.0}%)", if open { "aberta" } else { "fechada" }, p * 100.0);
    let to = if open { 1.0 } else { 0.0 };
    if animate && (p - to).abs() > 0.001 {
        o.anim = Some((Instant::now(), p, to));
    } else {
        o.value = to;
        if !open {
            state.overview = None;
        }
    }
    state.wake();
}

/// A cada quadro: termina a animação (e some, se fechou).
pub fn tick(state: &mut State) {
    // Janela aberta ou fechada com a visão aberta: o mosaico se refaz.
    if let Some(name) = state.overview.as_ref().map(|o| o.output.clone())
        && let Some(output) = state.space.outputs().find(|o| o.name() == name).cloned()
    {
        let windows = windows_on(state, &output);
        let same = state.overview.as_ref().is_some_and(|o| {
            o.slots.len() == windows.len() && windows.iter().all(|w| o.slots.iter().any(|s| s.id == window_id(w)))
        });
        if !same {
            let slots = layout(state, &output, &windows);
            tracing::info!("visão das janelas: mosaico refeito ({} janela(s))", slots.len());
            if let Some(o) = state.overview.as_mut() {
                o.slots = slots;
            }
        }
    }
    let Some(o) = state.overview.as_mut() else {
        return;
    };
    let Some((start, _, to)) = o.anim else {
        return;
    };
    if start.elapsed() < ANIMATION_DURATION {
        return;
    }
    o.anim = None;
    o.value = to;
    if to <= 0.0 {
        state.overview = None;
        tracing::info!("visão das janelas: fechada");
    } else {
        tracing::info!("visão das janelas: aberta");
    }
}

/// A visão está mexendo (pede quadros).
pub fn animating(state: &State) -> bool {
    state.overview.as_ref().is_some_and(|o| o.anim.is_some() || o.drag.is_some())
}

/// Posição, zoom e tamanho de uma janela da visão agora: entre o lugar dela
/// (`loc`, `size`) e o do mosaico. `None` para janelas fora da visão.
pub fn place(state: &State, window: &Window, loc: Point<i32, Logical>, size: smithay::utils::Size<i32, Logical>) -> Option<(Point<i32, Logical>, f64, i32, i32)> {
    let o = state.overview.as_ref()?;
    let id = window_id(window);
    let slot = o.slots.iter().find(|s| s.id == id)?;
    let p = o.progress() as f64;
    if p <= 0.0 {
        return None;
    }
    let lerp = |a: f64, b: f64| a + (b - a) * p;
    let x = lerp(loc.x as f64, slot.content.x as f64).round() as i32;
    let y = lerp(loc.y as f64, slot.content.y as f64).round() as i32;
    let zoom = lerp(1.0, slot.scale);
    Some(((x, y).into(), zoom, (size.w as f64 * zoom).round() as i32, (size.h as f64 * zoom).round() as i32))
}

/// Janela do mosaico sob o ponto (aberta de vez).
fn slot_at(state: &State, pos: Point<f64, Logical>) -> Option<u64> {
    let o = state.overview.as_ref()?;
    o.slots.iter().find(|s| {
        let area = Rectangle::new(s.frame.loc, (s.frame.size.w, s.frame.size.h + TITLE_H).into());
        area.to_f64().contains(pos)
    }).map(|s| s.id)
}

/// O ponteiro andou: destaca a janela sob ele.
pub fn hover(state: &mut State, pos: Point<f64, Logical>) {
    let hovered = slot_at(state, pos);
    if let Some(o) = state.overview.as_mut()
        && o.hovered != hovered
    {
        o.hovered = hovered;
        state.wake();
    }
}

/// Clique na visão: a janela sob o ponteiro vem para a frente; no vazio, só fecha.
pub fn click(state: &mut State, pos: Point<f64, Logical>) {
    let id = slot_at(state, pos);
    close(state, id);
}

/// Fundo escurecido da visão, atrás das janelas e na frente da mesa e do papel de parede.
pub fn dim(state: &State, output: &Output, origin: Point<i32, Logical>, shaders: &decorations::Shaders) -> Option<MideRenderElement> {
    let o = state.overview.as_ref().filter(|o| o.output == output.name())?;
    let p = o.progress();
    if p <= 0.0 {
        return None;
    }
    let screen = state.space.output_geometry(output)?;
    let rect = Rectangle::new(screen.loc - origin, screen.size);
    Some(MideRenderElement::from(decorations::rounded_rect_element(shaders, rect, [0.0, 0.0, 0.0, 0.45 * p], 0)))
}

/// Títulos em cápsula embaixo de cada janela e o contorno da janela sob o ponteiro, na
/// cor de destaque do sistema (da frente para trás).
pub fn elements(renderer: &mut GlesRenderer, state: &State, output: &Output, origin: Point<i32, Logical>, shaders: &decorations::Shaders, titles: &mut TitleRenderer) -> Vec<MideRenderElement> {
    let Some(o) = state.overview.as_ref().filter(|o| o.output == output.name()) else {
        return Vec::new();
    };
    let p = o.progress();
    if p < 0.5 {
        return Vec::new();
    }
    // Títulos e destaque entram na segunda metade do caminho.
    let alpha = ((p - 0.5) * 2.0).clamp(0.0, 1.0);
    let dark = decorations::dark();
    let mut front = Vec::new();
    let mut back = Vec::new();
    for slot in &o.slots {
        let frame = Rectangle::new(slot.frame.loc - origin, slot.frame.size);
        if o.hovered == Some(slot.id) && o.settled_open() {
            let [r, g, b, _] = mide::kit::system_accent(dark);
            let ring = Rectangle::new(frame.loc - Point::from((4, 4)), (frame.size.w + 8, frame.size.h + 8).into());
            front.push(MideRenderElement::from(decorations::stroke_element(shaders, ring, 14, [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, alpha])));
        }
        let Some(window) = state.space.elements().find(|w| window_id(w) == slot.id) else {
            continue;
        };
        let title = crate::render::window_title(window);
        let max = (frame.size.w + GAP - 16).max(40);
        let Some(image) = titles.image(&title, true, max) else {
            continue;
        };
        let pill_w = image.width + 20;
        let pill = Rectangle::new((frame.loc.x + (frame.size.w - pill_w) / 2, frame.loc.y + frame.size.h + 8).into(), (pill_w, PILL_H).into());
        if let Some(text) = image.render_element(renderer, (pill.loc.x + 10, pill.loc.y + (PILL_H - image.height) / 2).into(), alpha) {
            front.push(MideRenderElement::Title(text));
        }
        let fill = if dark { [44.0 / 255.0, 44.0 / 255.0, 48.0 / 255.0, 0.92 * alpha] } else { [246.0 / 255.0, 246.0 / 255.0, 248.0 / 255.0, 0.92 * alpha] };
        back.push(MideRenderElement::from(decorations::rounded_rect_element(shaders, pill, fill, PILL_H / 2)));
    }
    front.extend(back);
    front
}

/// Quatro dedos para os lados: a próxima (`dir` 1) ou a anterior (-1) janela comum da
/// tela sob o ponteiro, na ordem de uso (a de cima é a atual), vem para a frente.
pub fn cycle(state: &mut State, dir: i32) {
    let Some(output) = state.output_at(state.pointer_location) else {
        return;
    };
    let windows = windows_on(state, &output);
    if windows.len() < 2 {
        tracing::info!("gesto: uma janela só em {}", output.name());
        return;
    }
    let window = if dir > 0 { windows[1].clone() } else { windows[windows.len() - 1].clone() };
    let id = window_id(&window);
    tracing::info!("gesto: foco na janela {id} ({})", if dir > 0 { "próxima" } else { "anterior" });
    crate::control::handle(state, &format!("focus {id}"));
    // Transição curta: a janela que vem cresce um pouco até o tamanho dela.
    if state.config.effects.animations
        && let (Some(loc), Some(toplevel)) = (state.space.element_location(&window), window.toplevel())
    {
        let size = window.geometry().size;
        let surface = toplevel.wl_surface().clone();
        state.animations.retain(|a| a.surface != surface);
        let inset = Point::from(((size.w as f64 * 0.02).round() as i32, (size.h as f64 * 0.02).round() as i32));
        state.animations.push(crate::state::WindowAnimation {
            window: window.clone(),
            surface,
            from_loc: loc + inset,
            from_size: (size.w - 2 * inset.x, size.h - 2 * inset.y).into(),
            to_loc: loc,
            to_size: size,
            zoom_from: 0.96,
            zoom_to: 1.0,
            alpha_from: 0.7,
            alpha_to: 1.0,
            hide_at_end: false,
            genie: false,
            follow_icon: false,
            duration: std::time::Duration::from_millis(160),
            start: Instant::now(),
        });
    }
}
