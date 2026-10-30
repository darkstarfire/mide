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

use std::collections::HashMap;
use std::time::Duration;

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

/// Um dedo na tela.
#[derive(Debug, Clone)]
enum Contact {
    /// Entregue ao app (`wl_touch`): a superfície e a origem dela.
    App { surface: WlSurface, origin: Point<f64, Logical> },
    /// Vira ponteiro: onde começou, se o botão já foi apertado (arrasto), se o
    /// botão direito já saiu (segurar) e o número do toque (para o relógio do segurar).
    Pointer { start: Point<f64, Logical>, pressed: bool, long: bool, seq: u64 },
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
    if contacts.iter().any(|c| matches!(c, Contact::Pointer { pressed: true, .. })) {
        let time = state.start_time.elapsed().as_millis() as u32;
        input::button(state, BTN_LEFT, ButtonState::Released, time);
    }
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
    // Em modo tablet a tela passa a girar com o sensor (e volta ao normal ao sair).
    crate::rotation::apply(state);
    state.wake();
}

#[cfg(test)]
mod tests {
    use super::*;

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
