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

/// Inércia da rolagem com o dedo: velocidade mínima (pontos por ms) para continuar
/// depois de soltar, passo do relógio e quanto a velocidade cai a cada passo.
const FLING_MIN: f64 = 0.3;
const FLING_STEP: Duration = Duration::from_millis(16);
const FLING_DECAY: f64 = 0.95;
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
/// Movimento do dedo (pontos) abaixo do qual ele conta como parado (o tremor na tela).
const STILL: f64 = 8.0;
/// Faixa das bordas da esquerda e da direita (pontos) e o caminho para dentro que volta.
const SIDE_EDGE: f64 = 14.0;
const BACK_TRAVEL: f64 = 70.0;
/// Gesto de baixo mais curto que isto (pontos): só mostra a dock, por `DOCK_REVEAL`.
const DOCK_SWIPE: f64 = 90.0;
/// Caminho ao longo da borda de baixo (pontos) para trocar de app.
const SWITCH_TRAVEL: f64 = 60.0;
/// Pinça com quatro ou cinco dedos: os dedos chegam a esta fração da abertura inicial.
const PINCH_HOME: f64 = 0.6;
/// Até quanto tempo depois do terceiro dedo o quarto ainda conta para o mesmo gesto.
const LATE_FINGER: Duration = Duration::from_millis(250);
const DOCK_REVEAL: Duration = Duration::from_secs(4);
/// Tempo que a faixa com os botões da janela (deslizar do meio de cima) fica à mostra.
pub const TABLET_STRIP_SHOWN: Duration = Duration::from_secs(4);

/// Borda onde um gesto do modo tablet começou.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Bottom,
    TopLeft,
    TopRight,
    /// Meio da borda de cima (o nome do app e os menus na barra): deslizar para baixo
    /// mostra os botões da janela sem barra; tocar e soltar é um toque comum na barra.
    TopCenter,
    /// Bordas da esquerda e da direita: deslizar para dentro volta, como no Android.
    Left,
    Right,
}

/// Um dedo na tela.
#[derive(Debug, Clone)]
enum Contact {
    /// Entregue ao app (`wl_touch`): a superfície e a origem dela.
    App { surface: WlSurface, origin: Point<f64, Logical> },
    /// Vira ponteiro: onde começou, se o botão já foi apertado (arrasto), se o
    /// botão direito já saiu (segurar) e o número do toque (para o relógio do segurar).
    /// Num app do mide-ui, arrastar na vertical rola (`scrolls`, `scrolling`), como no
    /// iPad, com a última posição e a velocidade (pontos por ms) para a inércia.
    Pointer { start: Point<f64, Logical>, pressed: bool, long: bool, seq: u64, scrolls: bool, scrolling: bool, last: Point<f64, Logical>, vel: f64, at: Instant },
    /// Gesto de borda: ainda decidindo (`active` falso: pode virar um toque comum),
    /// acompanhando (o launcher subindo) ou já feito (`done`: o resto do dedo não vale).
    Edge { edge: Edge, start: Point<f64, Logical>, last: Point<f64, Logical>, active: bool, done: bool, moved_at: Instant, seq: u64 },
}

/// Estado do toque, do modo tablet e da rotação.
#[derive(Debug, Default)]
pub struct Touch {
    contacts: HashMap<i32, Contact>,
    seq: u64,
    /// Gesto de três ou quatro dedos na tela em andamento (como no touchpad).
    multi: Option<Multi>,
    /// Inércia de uma rolagem com o dedo em andamento (o número do toque).
    fling: Option<u64>,
    /// Modo tablet ligado (tela dobrada para trás, ou escolhido em `[tablet] mode`).
    pub tablet: bool,
    /// A chave de modo tablet da tela, como o kernel a informa (vale no modo `auto`).
    pub switch_on: bool,
    /// O aparelho tem a chave de modo tablet (sem ela, a tela gira sempre, como no GNOME).
    pub tablet_switch: bool,
    /// Há sensor de orientação (`iio-sensor-proxy`).
    pub sensor: bool,
    /// Última orientação lida do sensor (`normal`, `left-up`, `right-up`, `bottom-up`).
    pub orientation: Option<String>,
}

/// Gesto de três ou quatro dedos na tela: a posição de cada dedo e a média delas (o
/// centro do grupo), para o deslizar valer como no touchpad.
#[derive(Debug)]
pub struct Multi {
    pos: HashMap<i32, Point<f64, Logical>>,
    last: Option<Point<f64, Logical>>,
    /// Abertura dos dedos (distância média ao centro) quando todos já estavam na tela,
    /// para a pinça; `pinched` quando ela já levou ao launcher.
    spread: Option<f64>,
    pinched: bool,
    /// Quando o gesto começou e com quantos dedos (o quarto que chega logo depois do
    /// terceiro passa o gesto a quatro dedos).
    started: Instant,
    fingers: u32,
    /// Onde cada dedo foi visto pela primeira vez (o quarto dedo herda o caminho já feito).
    firsts: HashMap<i32, Point<f64, Logical>>,
}

/// Distância média dos dedos ao centro deles.
fn spread(pos: &HashMap<i32, Point<f64, Logical>>) -> Option<f64> {
    let c = centroid(pos)?;
    Some(pos.values().map(|p| (p.x - c.x).hypot(p.y - c.y)).sum::<f64>() / pos.len() as f64)
}

/// Caminho médio dos dedos desde onde cada um foi visto pela primeira vez.
fn travelled(multi: &Multi) -> Option<Point<f64, Logical>> {
    let moves: Vec<Point<f64, Logical>> = multi.pos.iter().filter_map(|(s, p)| multi.firsts.get(s).map(|f| *p - *f)).collect();
    if moves.is_empty() {
        return None;
    }
    let n = moves.len() as f64;
    let (x, y) = moves.iter().fold((0.0, 0.0), |(x, y), p| (x + p.x, y + p.y));
    Some((x / n, y / n).into())
}

fn centroid(pos: &HashMap<i32, Point<f64, Logical>>) -> Option<Point<f64, Logical>> {
    if pos.is_empty() {
        return None;
    }
    let n = pos.len() as f64;
    let (x, y) = pos.values().fold((0.0, 0.0), |(x, y), p| (x + p.x, y + p.y));
    Some((x / n, y / n).into())
}

/// Três ou quatro dedos ao mesmo tempo viram um gesto: os toques que já estavam na
/// tela são cancelados (apps e ponteiro), e o deslizar segue o centro dos dedos.
/// Devolve `true` quando o dedo foi tomado pelo gesto.
fn multi_down(state: &mut State, slot: i32, pos: Point<f64, Logical>) -> bool {
    if let Some(multi) = state.touch.multi.as_mut() {
        // Caminho já feito pelos dedos que estavam na tela (antes do dedo novo mudar o centro).
        let travelled = travelled(multi);
        multi.pos.insert(slot, pos);
        multi.firsts.entry(slot).or_insert(pos);
        // O centro muda com o dedo novo: sem salto no deslizar.
        multi.last = centroid(&multi.pos);
        // Os dedos nunca encostam juntos: o gesto começa no terceiro, e o quarto que chega
        // logo depois faz dele um gesto de quatro dedos (a visão das janelas), em vez de
        // ficar no de três (o launcher).
        let count = multi.pos.len() as u32;
        if multi.fingers == 3 && count == 4 && multi.started.elapsed() < LATE_FINGER {
            multi.fingers = 4;
            tracing::info!("toque: quarto dedo: gesto de 4 dedos");
            crate::gesture::end(state, true);
            crate::gesture::begin(state, 4);
            // O caminho dos três primeiros vale para o de quatro: num deslizar rápido, quase
            // todo o movimento já tinha acontecido e o mosaico abria e fechava em 0%.
            if let Some(d) = travelled {
                crate::gesture::update(state, d.x, d.y);
            }
        }
        return true;
    }
    let fingers = state.touch.contacts.len() as u32 + 1;
    if !(3..=5).contains(&fingers) {
        return false;
    }
    // Posição conhecida dos dedos que já tocavam (o app não diz onde estão os dele).
    let mut known: HashMap<i32, Point<f64, Logical>> = state
        .touch
        .contacts
        .iter()
        .filter_map(|(s, c)| match c {
            Contact::Pointer { start, .. } => Some((*s, *start)),
            Contact::Edge { last, .. } => Some((*s, *last)),
            Contact::App { .. } => None,
        })
        .collect();
    cancel(state);
    known.insert(slot, pos);
    let last = centroid(&known);
    let firsts = known.clone();
    state.touch.multi = Some(Multi { pos: known, last, spread: None, pinched: false, started: Instant::now(), fingers: fingers.min(4), firsts });
    tracing::info!("toque: gesto de {fingers} dedos");
    crate::gesture::begin(state, fingers);
    true
}

/// Um dos dedos do gesto andou: o centro do grupo anda junto (o deslizar).
fn multi_motion(state: &mut State, slot: i32, pos: Point<f64, Logical>) {
    let delta = {
        let Some(multi) = state.touch.multi.as_mut() else {
            return;
        };
        // Dedo que aparece pela primeira vez no gesto: só define a posição (sem salto).
        let known = multi.pos.insert(slot, pos).is_some();
        multi.firsts.entry(slot).or_insert(pos);
        // Pinça com quatro ou cinco dedos: vai ao launcher (a tela de início do iPad).
        if multi.pos.len() >= 4 && !multi.pinched {
            let now = spread(&multi.pos);
            match (multi.spread, now) {
                (None, Some(now)) => multi.spread = Some(now),
                (Some(start), Some(now)) if start > 0.0 && now / start <= PINCH_HOME => {
                    multi.pinched = true;
                    pinch_home(state);
                    return;
                }
                _ => {}
            }
        }
        let now = centroid(&multi.pos);
        let prev = std::mem::replace(&mut multi.last, now);
        match (known, now, prev) {
            (true, Some(now), Some(prev)) => Some(now - prev),
            _ => None,
        }
    };
    if let Some(d) = delta {
        crate::gesture::update(state, d.x, d.y);
    }
    state.wake();
}

/// Pinça de quatro ou cinco dedos: o deslizar em andamento para e o launcher abre.
fn pinch_home(state: &mut State) {
    tracing::info!("toque: pinça de vários dedos: launcher");
    crate::gesture::end(state, true);
    if !state.launcher_open() {
        state.toggle_launcher();
    }
    state.wake();
}

/// Um dedo do gesto saiu: quando o último sai, o gesto termina (abre ou fecha).
fn multi_up(state: &mut State, slot: i32) {
    let finished = {
        let Some(multi) = state.touch.multi.as_mut() else {
            return;
        };
        multi.pos.remove(&slot);
        multi.pos.is_empty()
    };
    if finished {
        state.touch.multi = None;
        crate::gesture::end(state, false);
    }
    state.wake();
}

/// Dedo encostou em `pos` (coordenadas globais).
pub fn down(state: &mut State, slot: i32, pos: Point<f64, Logical>, time: u32) {
    state.wake();
    if multi_down(state, slot, pos) {
        return;
    }
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
    // Tocar a tela esconde o ponteiro até o mouse ou o touchpad se mexer.
    if state.touch.contacts.is_empty() {
        let shown = std::mem::replace(&mut state.cursor_status, smithay::input::pointer::CursorImageStatus::Hidden);
        if state.touch_hid_cursor.is_none() {
            state.touch_hid_cursor = Some(shown);
        }
        state.wake();
    }
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
    // Um toque novo para a inércia de uma rolagem anterior.
    state.touch.fling = None;
    let scrolls = input::touch_scrolls(state, pos);
    state.touch.contacts.insert(slot, Contact::Pointer { start: pos, pressed: false, long: false, seq, scrolls, scrolling: false, last: pos, vel: 0.0, at: Instant::now() });
    // Segurar parado: botão direito.
    let timer = smithay::reexports::calloop::timer::Timer::from_duration(LONG_PRESS);
    let _ = state.loop_handle.insert_source(timer, move |_, _, state| {
        long_press(state, slot, seq);
        smithay::reexports::calloop::timer::TimeoutAction::Drop
    });
}

fn long_press(state: &mut State, slot: i32, seq: u64) {
    let Some(Contact::Pointer { pressed: false, scrolling: false, long, seq: current, .. }) = state.touch.contacts.get_mut(&slot) else {
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
    if state.touch.multi.is_some() {
        multi_motion(state, slot, pos);
        return;
    }
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
                    Edge::TopLeft | Edge::TopRight | Edge::TopCenter => d.y,
                    Edge::Left => d.x,
                    Edge::Right => -d.x,
                };
                if along >= DRAG_START * 1.5 && along > d.x.abs() {
                    edge_begin(state, slot, edge, from, pos, seq);
                } else if edge == Edge::Bottom && d.x.abs() > along.abs() {
                    // Ao longo da borda de baixo: troca de app, como no iPad (para a direita,
                    // o app usado antes; para a esquerda, volta).
                    if d.x.abs() >= SWITCH_TRAVEL {
                        let dir = if d.x > 0.0 { 1 } else { -1 };
                        tracing::info!("toque: borda de baixo para {}: troca de app", if dir > 0 { "a direita" } else { "a esquerda" });
                        crate::overview::cycle(state, dir);
                        state.touch.contacts.insert(slot, Contact::Edge { edge, start: from, last: pos, active: true, done: true, moved_at, seq });
                    }
                } else if d.x.hypot(d.y) >= DRAG_START * 1.5 {
                    // Outra direção: era um toque comum (rolar, arrastar) perto da borda.
                    state.touch.contacts.remove(&slot);
                    start(state, slot, from, time);
                    motion(state, slot, pos, time);
                }
                return;
            }
            // Launcher acompanhando o dedo. Um dedo parado na tela treme alguns pixels o
            // tempo todo: passos menores que `STILL` se juntam até passar dele, e só então
            // contam como movimento (antes cada tremor reiniciava o "parado" e o subir e
            // parar nunca abria a visão das janelas).
            let step = pos - last;
            if step.x.hypot(step.y) < STILL {
                return;
            }
            crate::gesture::update(state, step.x, step.y);
            state.touch.contacts.insert(slot, Contact::Edge { edge, start: from, last: pos, active, done, moved_at: Instant::now(), seq });
        }
        Contact::Pointer { long: true, .. } => {}
        Contact::Pointer { start, pressed, long, seq, scrolls, scrolling, last, vel, at } => {
            if scrolling {
                // Rolando: o conteúdo segue o dedo (dedo para cima, o conteúdo sobe).
                let dy = pos.y - last.y;
                let now = Instant::now();
                let dt = now.duration_since(at).as_secs_f64() * 1000.0;
                let vel = if dt > 0.5 { 0.6 * (-dy / dt) + 0.4 * vel } else { vel };
                input::finger_scroll(state, -dy, time, false);
                state.touch.contacts.insert(slot, Contact::Pointer { start, pressed, long, seq, scrolls, scrolling, last: pos, vel, at: now });
                return;
            }
            if !pressed {
                let d = pos - start;
                if d.x.hypot(d.y) < DRAG_START {
                    return;
                }
                if scrolls && d.y.abs() > d.x.abs() {
                    tracing::info!("toque: rolar com o dedo");
                    input::finger_scroll(state, -d.y, time, false);
                    state.touch.contacts.insert(slot, Contact::Pointer { start, pressed, long, seq, scrolls, scrolling: true, last: pos, vel: 0.0, at: Instant::now() });
                    return;
                }
                // Começou a arrastar: o botão desce onde o dedo encostou.
                input::pointer_motion(state, start, time);
                input::button(state, BTN_LEFT, ButtonState::Pressed, time);
                state.touch.contacts.insert(slot, Contact::Pointer { start, pressed: true, long, seq, scrolls, scrolling, last: pos, vel, at });
            }
            input::pointer_motion(state, pos, time);
        }
    }
}

/// O dedo saiu da tela.
pub fn up(state: &mut State, slot: i32, time: u32) {
    if state.touch.multi.is_some() {
        multi_up(state, slot);
        return;
    }
    let Some(contact) = state.touch.contacts.remove(&slot) else {
        return;
    };
    state.wake();
    match contact {
        Contact::Edge { done: true, .. } => {}
        // Borda de baixo, deslizar curto: como no iPad, só mostra a dock por cima do app
        // (o launcher que começou a subir volta). Mais longo: o launcher (tela de início).
        Contact::Edge { active: true, edge: Edge::Bottom, start, last, .. } if start.y - last.y < DOCK_SWIPE => {
            crate::gesture::end(state, true);
            tracing::info!("toque: deslizar curto da borda de baixo: dock à mostra");
            state.dock_reveal_until = Some(Instant::now() + DOCK_REVEAL);
        }
        // Bordas laterais: soltar depois de puxar para dentro volta (como no Android);
        // puxar pouco não faz nada.
        Contact::Edge { active: true, edge: edge @ (Edge::Left | Edge::Right), start, last, .. } => {
            let inward = if edge == Edge::Left { last.x - start.x } else { start.x - last.x };
            if inward >= BACK_TRAVEL {
                go_back(state);
            }
        }
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
        // Fim da rolagem: com o dedo ainda rápido, o conteúdo continua e desacelera.
        Contact::Pointer { scrolling: true, vel, at, seq, .. } => {
            let recent = at.elapsed() < Duration::from_millis(80);
            if recent && vel.abs() > FLING_MIN {
                fling(state, vel, seq);
            } else {
                input::finger_scroll(state, 0.0, time, true);
            }
        }
        Contact::Pointer { pressed: true, .. } => input::button(state, BTN_LEFT, ButtonState::Released, time),
        // Tocar e soltar: um clique onde o dedo encostou.
        Contact::Pointer { .. } => {
            tracing::info!("toque: clique");
            input::button(state, BTN_LEFT, ButtonState::Pressed, time);
            input::button(state, BTN_LEFT, ButtonState::Released, time);
        }
    }
}

/// Inércia da rolagem com o dedo: a cada 16 ms o conteúdo anda `vel` × 16 e a
/// velocidade cai, até parar (um toque novo também para).
fn fling(state: &mut State, vel: f64, seq: u64) {
    state.touch.fling = Some(seq);
    let mut vel = vel;
    let timer = smithay::reexports::calloop::timer::Timer::from_duration(FLING_STEP);
    let _ = state.loop_handle.insert_source(timer, move |_, _, state| {
        let time = state.start_time.elapsed().as_millis() as u32;
        if state.touch.fling != Some(seq) {
            return smithay::reexports::calloop::timer::TimeoutAction::Drop;
        }
        vel *= FLING_DECAY;
        if vel.abs() < FLING_MIN / 4.0 {
            state.touch.fling = None;
            input::finger_scroll(state, 0.0, time, true);
            return smithay::reexports::calloop::timer::TimeoutAction::Drop;
        }
        input::finger_scroll(state, vel * FLING_STEP.as_secs_f64() * 1000.0, time, false);
        state.wake();
        smithay::reexports::calloop::timer::TimeoutAction::ToDuration(FLING_STEP)
    });
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
    if state.touch.multi.take().is_some() {
        crate::gesture::end(state, true);
    }
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
        return Some(Edge::TopCenter);
    }
    if p.0 <= SIDE_EDGE {
        return Some(Edge::Left);
    }
    if p.0 >= size.0 - SIDE_EDGE {
        return Some(Edge::Right);
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
        Edge::Left | Edge::Right => {
            state.touch.contacts.insert(slot, Contact::Edge { edge, start: from, last: pos, active: true, done: false, moved_at: Instant::now(), seq });
        }
        Edge::TopCenter => {
            state.touch.contacts.insert(slot, Contact::Edge { edge, start: from, last: pos, active: true, done: true, moved_at: Instant::now(), seq });
            show_window_controls(state, from);
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

/// Voltar, como o gesto do Android: a tecla Voltar (XF86Back) vai ao app em foco
/// (navegadores, apps GTK e o Arquivos do mide entendem). Com o launcher ou a visão das
/// janelas abertos, fecha-os.
fn go_back(state: &mut State) {
    if state.overview.is_some() {
        tracing::info!("toque: voltar: fecha a visão das janelas");
        crate::overview::close(state, None);
    } else if state.launcher_open() {
        tracing::info!("toque: voltar: fecha o launcher");
        state.toggle_launcher();
    } else {
        tracing::info!("toque: voltar (tecla Voltar ao app)");
        let time = state.start_time.elapsed().as_millis() as u32;
        crate::input::evdev_key(state, KEY_BACK, true, time);
        crate::input::evdev_key(state, KEY_BACK, false, time);
    }
    state.wake();
}

/// Tecla Voltar do evdev (`KEY_BACK`, XF86Back no xkb).
const KEY_BACK: u32 = 158;

/// Deslizar do meio de cima no modo tablet sem barra: a faixa com os botões da janela em
/// foco desce por cima dela por alguns segundos (os controles da janela do iPadOS 26).
fn show_window_controls(state: &mut State, at: Point<f64, Logical>) {
    use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State as XdgState;
    if !crate::state::TABLET_BARLESS.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    let Some(output) = state.output_at(at) else {
        return;
    };
    let window = crate::gesture::focused_window(state).filter(|w| {
        w.toplevel().is_some_and(|t| t.current_state().states.contains(XdgState::Maximized))
            && state.fullscreen_entry(w).is_none()
            && state.space.element_location(w).is_some_and(|loc| state.space.output_geometry(&output).is_some_and(|g| g.contains(loc)))
    });
    let Some(window) = window else {
        return;
    };
    tracing::info!("toque: meio de cima: botões da janela à mostra");
    state.tablet_strip = Some((window, Instant::now()));
    // Some sozinha: um quadro novo quando o tempo acaba.
    let timer = smithay::reexports::calloop::timer::Timer::from_duration(TABLET_STRIP_SHOWN + Duration::from_millis(20));
    let _ = state.loop_handle.insert_source(timer, |_, _, state| {
        if state.tablet_strip.as_ref().is_some_and(|(_, since)| since.elapsed() >= TABLET_STRIP_SHOWN) {
            state.tablet_strip = None;
            state.wake();
        }
        smithay::reexports::calloop::timer::TimeoutAction::Drop
    });
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

/// Liga ou desliga o modo tablet (a chave do kernel ou o roteiro de teste). Vale só no
/// modo `auto` de `[tablet] mode`; nos outros, a escolha do usuário manda.
pub fn set_tablet_mode(state: &mut State, on: bool) {
    state.touch.tablet_switch = true;
    state.touch.switch_on = on;
    apply_tablet_mode(state);
}

/// Põe o modo tablet no que `[tablet] mode` pede (e na chave, no modo `auto`).
pub fn apply_tablet_mode(state: &mut State) {
    let on = match state.config.tablet.mode {
        crate::config::TabletMode::Auto => state.touch.switch_on,
        crate::config::TabletMode::Desktop => false,
        crate::config::TabletMode::Tablet => true,
    };
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
    // Apps ocupando a tela sem a barra de título, como no iPad; ao sair, a barra volta.
    let barless = on && maximize_windows(state);
    crate::state::TABLET_BARLESS.store(barless, std::sync::atomic::Ordering::Relaxed);
    if on {
        crate::rotation::read_now(state);
        if maximize_windows(state) {
            maximize_all(state);
        }
    }
    refill_maximized(state);
    state.wake();
}

/// As janelas já maximizadas ocupam de novo a área útil: com a barra de título saindo
/// (ou voltando) no modo tablet, o conteúdo cresce (ou encolhe) os 28 pontos dela.
fn refill_maximized(state: &mut State) {
    use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State as XdgState;
    let windows: Vec<_> = state.space.elements().cloned().collect();
    for window in windows {
        let Some(toplevel) = window.toplevel().cloned() else {
            continue;
        };
        if !toplevel.current_state().states.contains(XdgState::Maximized) || state.fullscreen_entry(&window).is_some() {
            continue;
        }
        if let Some(area) = state.usable_area_for_surface(toplevel.wl_surface()) {
            state.maximize_in(&toplevel, area);
        }
    }
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
    fn pinca_de_quatro_dedos() {
        let dedos = |r: f64| -> HashMap<i32, Point<f64, Logical>> {
            [(0, (100.0 - r, 100.0)), (1, (100.0 + r, 100.0)), (2, (100.0, 100.0 - r)), (3, (100.0, 100.0 + r))]
                .into_iter()
                .map(|(s, p)| (s, Point::from(p)))
                .collect()
        };
        let abertos = dedos(100.0);
        let c = centroid(&abertos).unwrap();
        assert!((c.x - 100.0).abs() < 1e-9 && (c.y - 100.0).abs() < 1e-9);
        assert!((spread(&abertos).unwrap() - 100.0).abs() < 1e-9);
        // Fechando até 60% da abertura: a pinça vale.
        assert!(spread(&dedos(60.0)).unwrap() / spread(&abertos).unwrap() <= PINCH_HOME);
        assert!(spread(&dedos(70.0)).unwrap() / spread(&abertos).unwrap() > PINCH_HOME);
        assert!(centroid(&HashMap::new()).is_none());
    }

    #[test]
    fn bordas_dos_gestos() {
        let screen = (800.0, 1280.0);
        assert_eq!(edge_in(screen, (400.0, 1275.0)), Some(Edge::Bottom));
        assert_eq!(edge_in(screen, (700.0, 5.0)), Some(Edge::TopRight));
        assert_eq!(edge_in(screen, (40.0, 5.0)), Some(Edge::TopLeft));
        // No meio da borda de cima (o nome do app na barra): deslizar para baixo mostra os
        // botões da janela; parado, vira toque comum na barra.
        assert_eq!(edge_in(screen, (400.0, 5.0)), Some(Edge::TopCenter));
        assert_eq!(edge_in(screen, (400.0, 600.0)), None);
        // Bordas laterais: deslizar para dentro volta (Android).
        assert_eq!(edge_in(screen, (5.0, 600.0)), Some(Edge::Left));
        assert_eq!(edge_in(screen, (795.0, 600.0)), Some(Edge::Right));
        assert_eq!(edge_in(screen, (30.0, 600.0)), None);
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
