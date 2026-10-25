//! Aviso flutuante das teclas de volume, mudo, microfone e brilho (OSD, A23).
//!
//! Desenhado pelo próprio compositor, como o alternador de janelas (Alt+Tab): é um
//! elemento da cena, sem cliente, sem superfície e sem foco. Por isso não pega
//! teclado nem clique e aparece também por cima da tela de bloqueio (o compositor
//! decide o que desenha ali), sem abrir nenhum programa a mais.
//!
//! A tecla vai para uma thread só do aviso, com fila (`wpctl`/`brightnessctl` esperam o
//! PipeWire e o kernel), que aplica, lê o nível que ficou e manda para o laço do
//! compositor por um canal; o
//! aviso mostra esse nível. Apertos seguidos atualizam o mesmo aviso (sem entrar de
//! novo), que some `OSD_HOLD` depois da última tecla.

use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            element::{
                memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
                Kind as ElementKind,
            },
            gles::GlesRenderer,
        },
    },
    output::Output,
    reexports::calloop::{
        channel,
        timer::{TimeoutAction, Timer},
        LoopHandle,
    },
    utils::{Logical, Point, Rectangle, Transform},
};

use crate::{
    config::Media,
    decorations::{self, Shaders},
    render::MideRenderElement,
    state::State,
    title::TitleRenderer,
};

pub const APPEAR: Duration = mide::motion::OSD_APPEAR;
pub const HOLD: Duration = mide::motion::OSD_HOLD;
pub const FADE: Duration = mide::motion::OSD_FADE;

/// Tamanho do painel (pontos), raio dos cantos e tamanho do ícone.
const W: i32 = 260;
const H: i32 = 48;
const RADIUS: i32 = 14;
const ICON: i32 = 24;

/// O que o aviso mostra.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Volume,
    Mic,
    Brightness,
}

/// Nível lido depois da tecla.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reading {
    pub kind: Kind,
    /// De 0 a 1 (o microfone não tem nível: só ligado ou mudo).
    pub level: f32,
    pub muted: bool,
}

impl Reading {
    /// Texto do log e do valor no painel.
    fn describe(&self) -> String {
        let percent = (self.level * 100.0).round() as i32;
        match (self.kind, self.muted) {
            (Kind::Volume, false) => format!("volume {percent}%"),
            (Kind::Volume, true) => format!("volume {percent}% (mudo)"),
            (Kind::Brightness, _) => format!("brilho {percent}%"),
            (Kind::Mic, true) => "microfone desligado".into(),
            (Kind::Mic, false) => "microfone ligado".into(),
        }
    }

    /// Quantas ondas o alto-falante mostra (0 a 3).
    fn waves(&self) -> u8 {
        match self.level {
            l if l <= 0.0 => 0,
            l if l < 0.34 => 1,
            l if l < 0.67 => 2,
            _ => 3,
        }
    }
}

/// Resposta da thread: número do aperto, tela onde mostrar e o nível lido.
pub struct Update {
    seq: u64,
    output: Option<String>,
    reading: Option<Reading>,
    kind: Kind,
}

/// Aviso na tela.
pub struct Osd {
    pub reading: Reading,
    /// Tela do aviso (a do ponteiro quando a tecla foi apertada).
    pub output: Option<String>,
    /// Quando apareceu (entrada) e a última tecla (saída `HOLD` depois).
    pub since: Instant,
    pub last: Instant,
}

impl Osd {
    /// Opacidade e quanto ainda falta subir (pontos), com ou sem transições.
    fn look(&self, now: Instant, transitions: bool) -> (f32, i32) {
        if !transitions {
            return (1.0, 0);
        }
        let t = (now.duration_since(self.since).as_secs_f32() / APPEAR.as_secs_f32()).clamp(0.0, 1.0);
        let e = mide::motion::ease_out(t);
        let out = now.duration_since(self.last).saturating_sub(HOLD);
        let fade = 1.0 - mide::motion::smooth(out.as_secs_f32() / FADE.as_secs_f32());
        (e * fade, (8.0 * (1.0 - e)).round() as i32)
    }

    /// Entrando ou saindo: o desenho continua pedindo quadros.
    pub fn moving(&self, transitions: bool) -> bool {
        let now = Instant::now();
        transitions && (now.duration_since(self.since) < APPEAR || now.duration_since(self.last) >= HOLD)
    }

    fn gone(&self, now: Instant, transitions: bool) -> bool {
        now.duration_since(self.last) >= HOLD + if transitions { FADE } else { Duration::ZERO }
    }
}

/// Aperto para a thread de trabalho: número, tela e a tecla.
pub struct Job {
    seq: u64,
    output: Option<String>,
    media: Media,
    kind: Kind,
}

/// Uma thread só para as teclas, com fila: os apertos são aplicados e lidos em
/// ordem, então a última leitura é sempre a do último aperto (com uma thread por
/// tecla, a leitura de um aperto podia sair antes de o comando do anterior valer).
pub fn init(loop_handle: &LoopHandle<'static, State>) -> Option<std::sync::mpsc::Sender<Job>> {
    let (sender, updates) = channel::channel::<Update>();
    let inserted = loop_handle.insert_source(updates, |event, _, state: &mut State| {
        if let channel::Event::Msg(update) = event {
            show(state, update);
        }
    });
    if let Err(err) = inserted {
        tracing::warn!("aviso de volume indisponível: {err}");
        return None;
    }
    let (jobs, queue) = std::sync::mpsc::channel::<Job>();
    let spawned = std::thread::Builder::new().name("mide-osd".into()).spawn(move || {
        for job in queue {
            let reading = apply(job.media).map(|(level, muted)| Reading { kind: job.kind, level: level.clamp(0.0, 1.0), muted });
            if sender.send(Update { seq: job.seq, output: job.output, reading, kind: job.kind }).is_err() {
                break;
            }
        }
    });
    match spawned {
        Ok(_) => Some(jobs),
        Err(err) => {
            tracing::warn!("aviso de volume indisponível: {err}");
            None
        }
    }
}

/// Aplica a tecla (espera o comando) e lê o nível que ficou.
fn apply(media: Media) -> Option<(f32, bool)> {
    use mide::system;
    match media {
        Media::VolumeUp | Media::VolumeDown => {
            system::volume_step(media == Media::VolumeUp);
            system::read_volume()
        }
        Media::Mute => {
            system::toggle_mute();
            system::read_volume()
        }
        Media::MicMute => {
            system::toggle_mic_mute();
            system::read_mic_muted().map(|muted| (if muted { 0.0 } else { 1.0 }, muted))
        }
        _ => {
            system::brightness_step(media == Media::BrightnessUp);
            system::read_brightness().map(|level| (level, false))
        }
    }
}

/// Tecla de volume, mudo, microfone ou brilho: aplica (na thread do aviso), lê o
/// nível e mostra o aviso. As outras teclas multimídia não têm aviso.
pub fn media_key(state: &mut State, media: Media) {
    let kind = match media {
        Media::VolumeUp | Media::VolumeDown | Media::Mute => Kind::Volume,
        Media::MicMute => Kind::Mic,
        Media::BrightnessUp | Media::BrightnessDown => Kind::Brightness,
        _ => return,
    };
    state.osd_seq += 1;
    let seq = state.osd_seq;
    let output = state.output_at(state.pointer_location).map(|o| o.name());
    let job = Job { seq, output, media, kind };
    // Sem a thread do aviso, a tecla ainda vale (sem aviso).
    let job = match &state.osd_sender {
        Some(jobs) => match jobs.send(job) {
            Ok(()) => return,
            Err(std::sync::mpsc::SendError(job)) => job,
        },
        None => job,
    };
    std::thread::spawn(move || {
        let _ = apply(job.media);
    });
}

/// Nível lido: mostra (ou atualiza) o aviso e marca a hora de sumir.
fn show(state: &mut State, update: Update) {
    // Resposta de um aperto mais antigo que o último mostrado: o nível já é velho.
    if update.seq < state.osd_shown_seq {
        return;
    }
    state.osd_shown_seq = update.seq;
    let Some(reading) = update.reading else {
        tracing::info!("osd: sem nível para mostrar ({:?})", update.kind);
        return;
    };
    let now = Instant::now();
    let transitions = state.config.effects.transitions;
    match state.osd.as_mut() {
        // Já na tela: só troca o valor (sem entrar de novo, sem piscar).
        Some(osd) if !osd.gone(now, transitions) => {
            osd.reading = reading;
            osd.last = now;
            osd.output = update.output;
        }
        _ => {
            tracing::info!("osd: aparece na tela {}", update.output.as_deref().unwrap_or("principal"));
            state.osd = Some(Osd { reading, output: update.output, since: now, last: now });
        }
    }
    tracing::info!("osd: {}", reading.describe());
    // Parado, o compositor não desenha: o relógio acorda o laço para o aviso sair.
    let timer = Timer::from_duration(HOLD + Duration::from_millis(5));
    if let Err(err) = state.loop_handle.insert_source(timer, |_, _, state| {
        state.wake();
        TimeoutAction::Drop
    }) {
        tracing::warn!("osd: relógio indisponível: {err}");
    }
    state.wake();
}

/// Tira o aviso que já terminou de sair (chamado a cada quadro).
pub fn finish(state: &mut State, now: Instant) {
    if state.osd.as_ref().is_some_and(|o| o.gone(now, state.config.effects.transitions)) {
        state.osd = None;
        tracing::info!("osd: escondido");
    }
}

/// Ícones desenhados pelo mide (alto-falante, microfone, sol), em cache.
#[derive(Default)]
pub struct Icons {
    images: HashMap<(Kind, u8, bool, bool, i32), MemoryRenderBuffer>,
}

impl Icons {
    fn get(&mut self, reading: &Reading, dark: bool, scale: i32) -> &MemoryRenderBuffer {
        let waves = if reading.kind == Kind::Volume && !reading.muted { reading.waves() } else { 0 };
        let key = (reading.kind, waves, reading.muted, dark, scale);
        self.images.entry(key).or_insert_with(|| {
            let color = if dark { [236, 236, 240, 255] } else { [24, 24, 28, 255] };
            let size = (ICON * scale) as u32;
            let pixels = draw_icon(reading.kind, waves, reading.muted, color, size);
            MemoryRenderBuffer::from_slice(&pixels, Fourcc::Abgr8888, (size as i32, size as i32), scale, Transform::Normal, None)
        })
    }
}

/// Desenho vetorial próprio de cada ícone, numa grade de 24x24 pontos, em RGBA
/// premultiplicado.
fn draw_icon(kind: Kind, waves: u8, muted: bool, color: [u8; 4], size: u32) -> Vec<u8> {
    use tiny_skia::{BlendMode, Color, FillRule, LineCap, Paint, PathBuilder, Pixmap, Stroke, Transform as Ts};
    let Some(mut pixmap) = Pixmap::new(size, size) else {
        return vec![0; (size * size * 4) as usize];
    };
    let ts = Ts::from_scale(size as f32 / 24.0, size as f32 / 24.0);
    let mut paint = Paint::default();
    paint.set_color(Color::from_rgba8(color[0], color[1], color[2], color[3]));
    paint.anti_alias = true;
    let stroke = |width: f32| Stroke { width, line_cap: LineCap::Round, ..Stroke::default() };
    // Arco em volta de (cx, cy), de `a0` a `a1` graus (0 = direita, sentido horário).
    let arc = |pb: &mut PathBuilder, cx: f32, cy: f32, r: f32, a0: f32, a1: f32| {
        let steps = 24;
        for i in 0..=steps {
            let a = (a0 + (a1 - a0) * i as f32 / steps as f32).to_radians();
            let (x, y) = (cx + r * a.cos(), cy + r * a.sin());
            if i == 0 { pb.move_to(x, y) } else { pb.line_to(x, y) }
        }
    };
    match kind {
        Kind::Volume => {
            // Caixa e cone do alto-falante.
            let mut pb = PathBuilder::new();
            pb.move_to(3.0, 9.0);
            pb.line_to(7.0, 9.0);
            pb.line_to(12.0, 4.5);
            pb.line_to(12.0, 19.5);
            pb.line_to(7.0, 15.0);
            pb.line_to(3.0, 15.0);
            pb.close();
            if let Some(path) = pb.finish() {
                pixmap.fill_path(&path, &paint, FillRule::Winding, ts, None);
            }
            // Ondas conforme o nível.
            for (i, r) in [4.0f32, 7.0, 10.0].into_iter().enumerate() {
                if (i as u8) < waves {
                    let mut pb = PathBuilder::new();
                    arc(&mut pb, 12.0, 12.0, r, -42.0, 42.0);
                    if let Some(path) = pb.finish() {
                        pixmap.stroke_path(&path, &paint, &stroke(1.8), ts, None);
                    }
                }
            }
        }
        Kind::Mic => {
            // Cápsula, suporte em U, haste e base.
            let mut pb = PathBuilder::new();
            arc(&mut pb, 12.0, 6.0, 3.0, 180.0, 360.0);
            pb.line_to(15.0, 11.0);
            arc(&mut pb, 12.0, 11.0, 3.0, 0.0, 180.0);
            pb.close();
            if let Some(path) = pb.finish() {
                pixmap.fill_path(&path, &paint, FillRule::Winding, ts, None);
            }
            let mut pb = PathBuilder::new();
            arc(&mut pb, 12.0, 11.0, 6.0, 0.0, 180.0);
            pb.move_to(12.0, 17.0);
            pb.line_to(12.0, 20.5);
            pb.move_to(8.5, 20.5);
            pb.line_to(15.5, 20.5);
            if let Some(path) = pb.finish() {
                pixmap.stroke_path(&path, &paint, &stroke(1.8), ts, None);
            }
        }
        Kind::Brightness => {
            // Sol: disco e oito raios.
            if let Some(path) = PathBuilder::from_circle(12.0, 12.0, 4.2) {
                pixmap.fill_path(&path, &paint, FillRule::Winding, ts, None);
            }
            let mut pb = PathBuilder::new();
            for i in 0..8 {
                let a = (i as f32 * 45.0).to_radians();
                pb.move_to(12.0 + 6.8 * a.cos(), 12.0 + 6.8 * a.sin());
                pb.line_to(12.0 + 9.6 * a.cos(), 12.0 + 9.6 * a.sin());
            }
            if let Some(path) = pb.finish() {
                pixmap.stroke_path(&path, &paint, &stroke(1.8), ts, None);
            }
        }
    }
    // Mudo: um traço cortando o ícone, com uma folga transparente dos dois lados.
    if muted {
        let mut pb = PathBuilder::new();
        pb.move_to(3.5, 3.5);
        pb.line_to(20.5, 20.5);
        if let Some(path) = pb.finish() {
            let clear = Paint { blend_mode: BlendMode::Clear, anti_alias: true, ..Paint::default() };
            pixmap.stroke_path(&path, &clear, &stroke(4.6), ts, None);
            pixmap.stroke_path(&path, &paint, &stroke(1.8), ts, None);
        }
    }
    pixmap.take()
}

/// Painel do aviso nesta tela, nas coordenadas dela (`None`: não aparece aqui).
fn panel(state: &State, osd: &Osd, output: &Output, lift: i32) -> Option<Rectangle<i32, Logical>> {
    let shown_here = state.config.monitors.mirror
        || match osd.output.as_deref() {
            Some(name) if state.space.outputs().any(|o| o.name() == name) => output.name() == name,
            _ => state.primary_output().is_some_and(|p| p == *output),
        };
    if !shown_here {
        return None;
    }
    let size = state.space.output_geometry(output)?.size;
    // No meio, embaixo (acima da dock), como os avisos passageiros.
    let x = (size.w - W) / 2;
    let y = (size.h as f32 * 0.80) as i32 - H / 2 + lift;
    Some(Rectangle::new((x, y).into(), (W, H).into()))
}

/// Elementos do aviso (da frente para trás), o retângulo do painel (para o vidro) e
/// a opacidade.
pub fn elements(
    renderer: &mut GlesRenderer,
    state: &State,
    output: &Output,
    shaders: &Shaders,
    titles: &mut TitleRenderer,
    icons: &mut Icons,
) -> Option<(Vec<MideRenderElement>, Rectangle<i32, Logical>, f32)> {
    // Contagem própria nos elementos guardados: o aviso aparecendo ou sumindo não
    // muda a identidade das molduras das janelas e da dock (sem dano na tela toda).
    decorations::set_group(decorations::GROUP_OSD);
    let out = build(renderer, state, output, shaders, titles, icons);
    decorations::set_group(0);
    out
}

fn build(
    renderer: &mut GlesRenderer,
    state: &State,
    output: &Output,
    shaders: &Shaders,
    titles: &mut TitleRenderer,
    icons: &mut Icons,
) -> Option<(Vec<MideRenderElement>, Rectangle<i32, Logical>, f32)> {
    let osd = state.osd.as_ref()?;
    let (alpha, lift) = osd.look(Instant::now(), state.config.effects.transitions);
    if alpha <= 0.0 {
        return None;
    }
    let panel = panel(state, osd, output, lift)?;
    let dark = state.config.dark;
    let fade = |c: [f32; 4]| [c[0], c[1], c[2], c[3] * alpha];
    let reading = osd.reading;
    let at = |dx: i32, dy: i32| -> Point<i32, Logical> { panel.loc + Point::from((dx, dy)) };
    let mut out: Vec<MideRenderElement> = Vec::new();

    // Valor (à direita) ou o estado do microfone (no lugar da barra). O texto é
    // centrado no retângulo dado, descontadas as margens do título das janelas.
    let margin = crate::title::TITLE_SIDE_MARGIN;
    let (text, center, width) = match reading.kind {
        Kind::Mic => (if reading.muted { "Microfone desligado".to_string() } else { "Microfone ligado".to_string() }, W / 2 + 18, W - 70),
        _ if reading.muted => ("Mudo".to_string(), W - 34, 50),
        _ => (format!("{}%", (reading.level * 100.0).round() as i32), W - 34, 50),
    };
    let text_box = Rectangle::new(at(center - width / 2 - margin, 0), (width + 2 * margin, H).into());
    if let Some(el) = crate::title::element(renderer, titles, &text, true, text_box, alpha) {
        out.push(MideRenderElement::Title(el));
    }
    // Ícone.
    let scale = crate::clip::scale().ceil().max(1.0) as i32;
    let icon = icons.get(&reading, dark, scale);
    let location = crate::clip::phys(at(14, (H - ICON) / 2)).to_f64();
    if let Ok(el) = MemoryRenderBufferRenderElement::from_buffer(renderer, location, icon, Some(alpha), None, None, ElementKind::Unspecified) {
        out.push(MideRenderElement::Title(el));
    }
    // Barra de nível: trilho e parte cheia (cinza no mudo), contínua.
    if reading.kind != Kind::Mic {
        let (x0, x1, h) = (50, W - 62, 6);
        let track = Rectangle::new(at(x0, (H - h) / 2), (x1 - x0, h).into());
        let filled = ((x1 - x0) as f32 * reading.level).round() as i32;
        if filled > 0 {
            let color = match (reading.muted, dark) {
                (true, false) => [130.0 / 255.0, 130.0 / 255.0, 138.0 / 255.0, 1.0],
                (true, true) => [152.0 / 255.0, 152.0 / 255.0, 160.0 / 255.0, 1.0],
                (false, false) => [10.0 / 255.0, 110.0 / 255.0, 235.0 / 255.0, 1.0],
                (false, true) => [30.0 / 255.0, 125.0 / 255.0, 245.0 / 255.0, 1.0],
            };
            let fill = Rectangle::new(track.loc, (filled.max(h), h).into());
            out.push(MideRenderElement::from(decorations::rounded_rect_element(shaders, fill, fade(color), h / 2)));
        }
        let track_color = if dark { [1.0, 1.0, 1.0, 0.16] } else { [0.0, 0.0, 0.0, 0.10] };
        out.push(MideRenderElement::from(decorations::rounded_rect_element(shaders, track, fade(track_color), h / 2)));
    }
    // Contorno fino e véu do vidro de painel (opaco com o vidro desligado).
    let stroke = if dark { [1.0, 1.0, 1.0, 0.14] } else { [0.0, 0.0, 0.0, 0.12] };
    out.push(MideRenderElement::from(decorations::stroke_element(shaders, panel, RADIUS, fade(stroke))));
    // Com a tela bloqueada não há vidro atrás do aviso (o caminho do bloqueio não
    // desfoca regiões): véu opaco, para o texto continuar legível.
    let veil_alpha = if state.config.effects.panel_glass && !state.lock.locked { 0.70 } else { 0.96 };
    let veil = if dark { [36.0 / 255.0, 36.0 / 255.0, 40.0 / 255.0, veil_alpha] } else { [248.0 / 255.0, 248.0 / 255.0, 250.0 / 255.0, veil_alpha] };
    out.push(MideRenderElement::from(decorations::rounded_rect_element(shaders, panel, fade(veil), RADIUS)));
    Some((out, panel, alpha))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ondas_pelo_nivel() {
        let r = |level| Reading { kind: Kind::Volume, level, muted: false };
        assert_eq!(r(0.0).waves(), 0);
        assert_eq!(r(0.2).waves(), 1);
        assert_eq!(r(0.5).waves(), 2);
        assert_eq!(r(1.0).waves(), 3);
    }

    #[test]
    fn texto_do_log() {
        assert_eq!(Reading { kind: Kind::Volume, level: 0.55, muted: false }.describe(), "volume 55%");
        assert_eq!(Reading { kind: Kind::Volume, level: 0.5, muted: true }.describe(), "volume 50% (mudo)");
        assert_eq!(Reading { kind: Kind::Mic, level: 0.0, muted: true }.describe(), "microfone desligado");
        assert_eq!(Reading { kind: Kind::Brightness, level: 0.75, muted: false }.describe(), "brilho 75%");
    }

    #[test]
    fn some_depois_da_ultima_tecla() {
        let now = Instant::now();
        let osd = Osd { reading: Reading { kind: Kind::Volume, level: 0.5, muted: false }, output: None, since: now, last: now };
        assert!(!osd.gone(now + HOLD - Duration::from_millis(1), true));
        assert!(!osd.gone(now + HOLD, true));
        assert!(osd.gone(now + HOLD + FADE, true));
        assert!(osd.gone(now + HOLD, false));
        // Parado no meio: inteiro; saindo: mais transparente.
        assert_eq!(osd.look(now + APPEAR, true).0, 1.0);
        assert!(osd.look(now + HOLD + FADE / 2, true).0 < 1.0);
        assert_eq!(osd.look(now, false), (1.0, 0));
    }

    #[test]
    fn icones_desenham_algo() {
        for kind in [Kind::Volume, Kind::Mic, Kind::Brightness] {
            let pixels = draw_icon(kind, 3, false, [0, 0, 0, 255], 24);
            assert!(pixels.chunks(4).any(|p| p[3] > 200), "{kind:?} vazio");
        }
        // Mudo: o traço corta o ícone (há pixels no canto de cima à esquerda).
        let muted = draw_icon(Kind::Volume, 0, true, [0, 0, 0, 255], 24);
        assert!(muted[(4 * 24 + 4) * 4 + 3] > 100);
    }
}
