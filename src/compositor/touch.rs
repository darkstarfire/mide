//! Tela de toque e modo tablet (notebooks conversíveis, como o Chromebook).
//!
//! - Toque num app: vai como `wl_touch` (o app trata gestos, rolagem, seleção).
//! - Toque nas partes do mide (barra de título, luzes, bordas, barra, dock, mesa,
//!   launcher, tela de bloqueio, apps do mide-ui): vira ponteiro. Tocar e soltar é um
//!   clique (dois toques rápidos, o duplo clique: maximizar pela barra), arrastar
//!   move (a janela pela barra), segurar parado ~600 ms é o botão direito.
//! - Modo tablet: a chave do kernel (`SW_TABLET_MODE`, a tela dobrada para trás) liga e
//!   desliga; o backend desliga o teclado e o touchpad internos enquanto dura (e o
//!   teclado na tela pergunta `tablet_mode` pelo canal de controle).
//! - Gestos das bordas no modo tablet, como no iPad (`[tablet] edge_gestures`): da
//!   borda de baixo para cima, o launcher sobe acompanhando o dedo (a tela de início);
//!   subindo e parando, a visão de todas as janelas; do canto de cima à direita para
//!   baixo, a central de controle; do canto de cima à esquerda, as notificações. Um
//!   toque parado na borda continua sendo um toque comum.
//! - O launcher e o teclado na tela recebem o toque como `wl_touch` (passar páginas com
//!   o dedo, segurar para editar, deslizar no espaço para mover o cursor).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use smithay::{
    backend::input::ButtonState,
    input::touch::{DownEvent, MotionEvent, UpEvent},
    output::Output,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Point, Transform, SERIAL_COUNTER},
};

use crate::{
    input::{self, BTN_LEFT, BTN_RIGHT},
    state::State,
};

/// Tempo parado com o dedo para o botão direito.
const LONG_PRESS: Duration = Duration::from_millis(600);
/// Caminho do dedo (pontos) a partir do qual o toque vira arrasto.
const DRAG_START: f64 = 8.0;
/// Faixa das bordas (pontos) onde começam os gestos do modo tablet, e a largura dos
/// cantos de cima (fração da largura da tela).
const EDGE: f64 = 22.0;
const CORNER: f64 = 0.3;
/// Gesto de baixo: subindo e parando este tempo (depois de subir `HOLD_TRAVEL`), a
/// visão das janelas no lugar do launcher.
const HOLD: Duration = Duration::from_millis(280);
const HOLD_TRAVEL: f64 = 110.0;

/// Borda onde um gesto do modo tablet começou.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Bottom,
    TopLeft,
    TopRight,
}

/// Um dedo na tela.
#[derive(Debug, Clone)]
enum Contact {
    /// Entregue ao app (`wl_touch`): a superfície e a origem dela.
    App { surface: WlSurface, origin: Point<f64, Logical> },
    /// Vira ponteiro: onde começou, se o botão já foi apertado (arrasto), se o
    /// botão direito já saiu (segurar) e o número do toque (para o relógio do segurar).
    Pointer { start: Point<f64, Logical>, pressed: bool, long: bool, seq: u64 },
    /// Gesto de borda: ainda decidindo (`active` falso: pode virar um toque comum),
    /// acompanhando (o launcher subindo) ou já feito (`done`: o resto do dedo não vale).
    Edge { edge: Edge, start: Point<f64, Logical>, last: Point<f64, Logical>, active: bool, done: bool, moved_at: Instant, seq: u64 },
}

/// Estado do toque, do modo tablet e da rotação.
#[derive(Debug, Default)]
pub struct Touch {
    contacts: HashMap<i32, Contact>,
    seq: u64,
    /// Modo tablet ligado (tela dobrada para trás).
    pub tablet: bool,
    /// O aparelho tem a chave de modo tablet (sem ela, a tela gira sempre, como no GNOME).
    pub tablet_switch: bool,
    /// Há sensor de orientação (`iio-sensor-proxy`).
    pub sensor: bool,
    /// Última orientação lida do sensor (`normal`, `left-up`, `right-up`, `bottom-up`).
    pub orientation: Option<String>,
}

/// Dedo encostou em `pos` (coordenadas globais).
pub fn down(state: &mut State, slot: i32, pos: Point<f64, Logical>, time: u32) {
    state.wake();
    if state.touch.contacts.is_empty()
        && let Some(edge) = edge_at(state, pos)
    {
        state.touch.seq += 1;
        let seq = state.touch.seq;
        tracing::info!("toque: na borda ({edge:?})");
        state.touch.contacts.insert(slot, Contact::Edge { edge, start: pos, last: pos, active: false, done: false, moved_at: Instant::now(), seq });
        return;
    }
    start(state, slot, pos, time);
}

/// Começo de um toque comum (no app como `wl_touch`, ou como ponteiro).
fn start(state: &mut State, slot: i32, pos: Point<f64, Logical>, time: u32) {
    if let Some((surface, origin)) = input::touch_app_target(state, pos) {
        // Toque numa janela: ela vem para frente com o foco, como o clique.
        if let Some(window) = state.window_for(&surface) {
            input::focus_window(state, &window, SERIAL_COUNTER.next_serial());
        }
        if let Some(touch) = state.seat.get_touch() {
            let event = DownEvent { slot: slot_of(slot), location: pos, serial: SERIAL_COUNTER.next_serial(), time };
            touch.down(state, Some((surface.clone(), origin)), &event);
        }
        tracing::info!("toque: no app (dedo {slot})");
        state.touch.contacts.insert(slot, Contact::App { surface, origin });
        return;
    }
    // Só um dedo vira ponteiro por vez (o ponteiro é um só).
    if state.touch.contacts.values().any(|c| matches!(c, Contact::Pointer { .. })) {
        return;
    }
    state.touch.seq += 1;
    let seq = state.touch.seq;
    input::pointer_motion(state, pos, time);
    state.touch.contacts.insert(slot, Contact::Pointer { start: pos, pressed: false, long: false, seq });
    // Segurar parado: botão direito.
    let timer = smithay::reexports::calloop::timer::Timer::from_duration(LONG_PRESS);
    let _ = state.loop_handle.insert_source(timer, move |_, _, state| {
        long_press(state, slot, seq);
        smithay::reexports::calloop::timer::TimeoutAction::Drop
    });
}

fn long_press(state: &mut State, slot: i32, seq: u64) {
    let Some(Contact::Pointer { pressed: false, long, seq: current, .. }) = state.touch.contacts.get_mut(&slot) else {
        return;
    };
    if *current != seq || *long {
        return;
    }
    *long = true;
    tracing::info!("toque: segurar (botão direito)");
    let time = state.start_time.elapsed().as_millis() as u32;
    input::button(state, BTN_RIGHT, ButtonState::Pressed, time);
    input::button(state, BTN_RIGHT, ButtonState::Released, time);
    state.wake();
}

/// O dedo andou.
pub fn motion(state: &mut State, slot: i32, pos: Point<f64, Logical>, time: u32) {
    let Some(contact) = state.touch.contacts.get(&slot).cloned() else {
        return;
    };
    state.wake();
    match contact {
        Contact::App { surface, origin } => {
            if let Some(touch) = state.seat.get_touch() {
                touch.motion(state, Some((surface, origin)), &MotionEvent { slot: slot_of(slot), location: pos, time });
            }
        }
        Contact::Edge { edge, start: from, last, active, done, moved_at, seq } => {
            if done {
                return;
            }
            let d = pos - from;
            if !active {
                let along = match edge {
                    Edge::Bottom => -d.y,
                    Edge::TopLeft | Edge::TopRight => d.y,
                };
                if along >= DRAG_START * 1.5 && along > d.x.abs() {
                    edge_begin(state, slot, edge, from, pos, seq);
                } else if d.x.hypot(d.y) >= DRAG_START * 1.5 {
                    // Outra direção: era um toque comum (rolar, arrastar) perto da borda.
                    state.touch.contacts.remove(&slot);
                    start(state, slot, from, time);
                    motion(state, slot, pos, time);
                }
                return;
            }
            // Launcher acompanhando o dedo.
            let step = pos - last;
            crate::gesture::update(state, step.x, step.y);
            let moved_at = if step.x.hypot(step.y) > 2.0 { Instant::now() } else { moved_at };
            state.touch.contacts.insert(slot, Contact::Edge { edge, start: from, last: pos, active, done, moved_at, seq });
        }
        Contact::Pointer { long: true, .. } => {}
        Contact::Pointer { start, pressed, long, seq } => {
            if !pressed {
                let d = pos - start;
                if d.x.hypot(d.y) < DRAG_START {
                    return;
                }
                // Começou a arrastar: o botão desce onde o dedo encostou.
                input::pointer_motion(state, start, time);
                input::button(state, BTN_LEFT, ButtonState::Pressed, time);
                state.touch.contacts.insert(slot, Contact::Pointer { start, pressed: true, long, seq });
            }
            input::pointer_motion(state, pos, time);
        }
    }
}

/// O dedo saiu da tela.
pub fn up(state: &mut State, slot: i32, time: u32) {
    let Some(contact) = state.touch.contacts.remove(&slot) else {
        return;
    };
    state.wake();
    match contact {
        Contact::Edge { done: true, .. } => {}
        Contact::Edge { active: true, .. } => crate::gesture::end(state, false),
        // Parado na borda: um toque comum ali.
        Contact::Edge { start: from, .. } => {
            start(state, slot, from, time);
            up(state, slot, time);
        }
        Contact::App { .. } => {
            if let Some(touch) = state.seat.get_touch() {
                touch.up(state, &UpEvent { slot: slot_of(slot), serial: SERIAL_COUNTER.next_serial(), time });
            }
        }
        Contact::Pointer { long: true, .. } => {}
        Contact::Pointer { pressed: true, .. } => input::button(state, BTN_LEFT, ButtonState::Released, time),
        // Tocar e soltar: um clique onde o dedo encostou.
        Contact::Pointer { .. } => {
            tracing::info!("toque: clique");
            input::button(state, BTN_LEFT, ButtonState::Pressed, time);
            input::button(state, BTN_LEFT, ButtonState::Released, time);
        }
    }
}

/// Fim de um grupo de eventos de toque (o app desenha depois dele).
pub fn frame(state: &mut State) {
    if state.touch.contacts.values().any(|c| matches!(c, Contact::App { .. }))
        && let Some(touch) = state.seat.get_touch()
    {
        touch.frame(state);
    }
}

/// O sistema cancelou o toque (outro gesto tomou conta): solta tudo.
pub fn cancel(state: &mut State) {
    let contacts: Vec<Contact> = state.touch.contacts.drain().map(|(_, c)| c).collect();
    if contacts.iter().any(|c| matches!(c, Contact::App { .. }))
        && let Some(touch) = state.seat.get_touch()
    {
        touch.cancel(state);
    }
    if contacts.iter().any(|c| matches!(c, Contact::Edge { active: true, done: false, .. })) {
        crate::gesture::end(state, true);
    }
    if contacts.iter().any(|c| matches!(c, Contact::Pointer { pressed: true, .. })) {
        let time = state.start_time.elapsed().as_millis() as u32;
        input::button(state, BTN_LEFT, ButtonState::Released, time);
    }
}

/// Borda de gesto sob `pos` (só no modo tablet, com a opção, sem a tela bloqueada).
fn edge_at(state: &State, pos: Point<f64, Logical>) -> Option<Edge> {
    if !state.touch.tablet || !state.config.tablet.edge_gestures || state.lock.locked {
        return None;
    }
    let output = state.output_at(pos)?;
    let geo = state.space.output_geometry(&output)?.to_f64();
    let (x, y) = (pos.x - geo.loc.x, pos.y - geo.loc.y);
    edge_in((geo.size.w, geo.size.h), (x, y))
}

/// Borda de gesto num ponto `p` de uma tela `size` (pontos).
fn edge_in(size: (f64, f64), p: (f64, f64)) -> Option<Edge> {
    if p.1 >= size.1 - EDGE {
        return Some(Edge::Bottom);
    }
    if p.1 <= EDGE {
        if p.0 <= size.0 * CORNER {
            return Some(Edge::TopLeft);
        }
        if p.0 >= size.0 * (1.0 - CORNER) {
            return Some(Edge::TopRight);
        }
    }
    None
}

/// O dedo saiu da borda na direção do gesto: começa.
fn edge_begin(state: &mut State, slot: i32, edge: Edge, from: Point<f64, Logical>, pos: Point<f64, Logical>, seq: u64) {
    match edge {
        Edge::Bottom => {
            tracing::info!("toque: gesto da borda de baixo (tela de início)");
            crate::gesture::begin(state, 3);
            let d = pos - from;
            crate::gesture::update(state, d.x, d.y);
            state.touch.contacts.insert(slot, Contact::Edge { edge, start: from, last: pos, active: true, done: false, moved_at: Instant::now(), seq });
            // Subindo e parando: a visão das janelas.
            let timer = smithay::reexports::calloop::timer::Timer::from_duration(Duration::from_millis(60));
            let _ = state.loop_handle.insert_source(timer, move |_, _, state| hold_check(state, slot, seq));
        }
        Edge::TopLeft | Edge::TopRight => {
            let what = if edge == Edge::TopRight { "control-center" } else { "notifications" };
            tracing::info!("toque: gesto do canto de cima ({what})");
            state.bar_request = Some(what);
            state.touch.contacts.insert(slot, Contact::Edge { edge, start: from, last: pos, active: true, done: true, moved_at: Instant::now(), seq });
        }
    }
    state.wake();
}

/// Gesto de baixo parado depois de subir: troca o launcher pela visão das janelas.
fn hold_check(state: &mut State, slot: i32, seq: u64) -> smithay::reexports::calloop::timer::TimeoutAction {
    use smithay::reexports::calloop::timer::TimeoutAction;
    let Some(Contact::Edge { start: from, last, active: true, done: false, moved_at, seq: current, .. }) = state.touch.contacts.get(&slot).cloned() else {
        return TimeoutAction::Drop;
    };
    if current != seq {
        return TimeoutAction::Drop;
    }
    if from.y - last.y < HOLD_TRAVEL || moved_at.elapsed() < HOLD {
        return TimeoutAction::ToDuration(Duration::from_millis(50));
    }
    tracing::info!("toque: subir e parar (visão das janelas)");
    if let Some(Contact::Edge { done, .. }) = state.touch.contacts.get_mut(&slot) {
        *done = true;
    }
    // O launcher que vinha subindo volta; a visão das janelas entra.
    crate::gesture::end(state, true);
    if let Some(output) = state.output_at(last) {
        crate::overview::open(state, &output, None);
    }
    state.wake();
    TimeoutAction::Drop
}

/// No modo tablet, as janelas ocupam a tela (`[tablet] maximize_windows`).
pub fn maximize_windows(state: &State) -> bool {
    state.touch.tablet && state.config.tablet.maximize_windows
}

/// Barra e dock maiores (o modo tablet com `[tablet] large_targets`).
pub fn large_targets(state: &State) -> bool {
    state.touch.tablet && state.config.tablet.large_targets
}

fn slot_of(slot: i32) -> smithay::backend::input::TouchSlot {
    Some(slot.max(0) as u32).into()
}

/// Tela interna (a do painel com toque): `eDP`/`LVDS`/`DSI`, senão a principal.
pub fn internal_output(state: &State) -> Option<Output> {
    state
        .space
        .outputs()
        .find(|o| ["eDP", "LVDS", "DSI"].iter().any(|p| o.name().starts_with(p)))
        .cloned()
        .or_else(|| state.primary_output())
}

/// Posição global de um toque dado em fração do painel (0 a 1, sem a rotação): o
/// painel não gira, a imagem sim; volta para o espaço da tela girada.
pub fn panel_to_global(state: &State, u: f64, v: f64) -> Option<Point<f64, Logical>> {
    let output = internal_output(state)?;
    let geo = state.space.output_geometry(&output)?;
    let (x, y) = unrotate(output.current_transform(), u.clamp(0.0, 1.0), v.clamp(0.0, 1.0));
    Some((geo.loc.x as f64 + x * geo.size.w as f64, geo.loc.y as f64 + y * geo.size.h as f64).into())
}

/// Fração do painel (sem rotação) para fração da tela girada.
fn unrotate(transform: Transform, u: f64, v: f64) -> (f64, f64) {
    // A saída girada `_90` mostra o painel girado: o que está em (x, y) da tela girada
    // fica no painel em `transform` aplicado. Aqui, o caminho de volta.
    let size = smithay::utils::Size::<f64, smithay::utils::Physical>::from((1.0, 1.0));
    let p = transform.invert().transform_point_in(Point::<f64, smithay::utils::Physical>::from((u, v)), &size);
    (p.x, p.y)
}

/// Liga ou desliga o modo tablet (a chave do kernel ou o roteiro de teste).
pub fn set_tablet_mode(state: &mut State, on: bool) {
    state.touch.tablet_switch = true;
    if state.touch.tablet == on {
        return;
    }
    state.touch.tablet = on;
    tracing::info!("modo tablet: {}", if on { "ligado" } else { "desligado" });
    // O backend desliga (ou religa) o teclado e o touchpad internos.
    if let Some(waker) = &state.input_waker {
        waker.ping();
    }
    // Em modo tablet a tela passa a girar com o sensor (e volta ao normal ao sair). O
    // aparelho pode já estar em pé: a orientação é lida de novo na hora.
    crate::rotation::apply(state);
    if on {
        crate::rotation::read_now(state);
        // Apps ocupando a tela, como no iPad.
        if maximize_windows(state) {
            maximize_all(state);
        }
    }
    state.wake();
}

/// Põe as janelas abertas ocupando a área útil da tela delas (ao entrar no modo tablet).
fn maximize_all(state: &mut State) {
    use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State as XdgState;
    let windows: Vec<_> = state.space.elements().cloned().collect();
    for window in windows {
        let Some(toplevel) = window.toplevel().cloned() else {
            continue;
        };
        if crate::state::fixed_size(&window) || state.fullscreen_entry(&window).is_some() || toplevel.current_state().states.contains(XdgState::Maximized) {
            continue;
        }
        if let Some(area) = state.usable_area_for_surface(toplevel.wl_surface()) {
            state.maximize_in(&toplevel, area);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bordas_dos_gestos() {
        let screen = (800.0, 1280.0);
        assert_eq!(edge_in(screen, (400.0, 1275.0)), Some(Edge::Bottom));
        assert_eq!(edge_in(screen, (700.0, 5.0)), Some(Edge::TopRight));
        assert_eq!(edge_in(screen, (40.0, 5.0)), Some(Edge::TopLeft));
        // No meio da borda de cima (o nome do app na barra): toque comum.
        assert_eq!(edge_in(screen, (400.0, 5.0)), None);
        assert_eq!(edge_in(screen, (400.0, 600.0)), None);
    }

    #[test]
    fn painel_para_tela_girada() {
        // Sem rotação, igual.
        assert_eq!(unrotate(Transform::Normal, 0.25, 0.75), (0.25, 0.75));
        // Meia volta: o canto oposto.
        let (x, y) = unrotate(Transform::_180, 0.25, 0.75);
        assert!((x - 0.75).abs() < 1e-9 && (y - 0.25).abs() < 1e-9);
        // Um quarto de volta e de volta: ida e volta é a identidade.
        for t in [Transform::_90, Transform::_270] {
            let (x, y) = unrotate(t, 0.2, 0.6);
            let size = smithay::utils::Size::<f64, smithay::utils::Physical>::from((1.0, 1.0));
            let back = t.transform_point_in(Point::<f64, smithay::utils::Physical>::from((x, y)), &size);
            assert!((back.x - 0.2).abs() < 1e-9 && (back.y - 0.6).abs() < 1e-9, "{t:?}");
        }
    }
}
