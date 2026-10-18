//! Entrada de teclado e ponteiro vinda do backend, repassada aos clientes.
//!
//! O foco do teclado segue o clique: a janela clicada vira a janela ativa e
//! recebe as teclas. O ponteiro envia movimento, botões e rolagem à superfície
//! que estiver sob o cursor.

use smithay::{
    backend::{
        input::{
            AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, InputEvent, KeyState,
            KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent,
        },
        winit::WinitInput,
    },
    desktop::{layer_map_for_output, utils::under_from_surface_tree, PopupManager, Window, WindowSurfaceType},
    input::{
        keyboard::FilterResult,
        pointer::{AxisFrame, ButtonEvent, MotionEvent},
    },
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Point, Size, SERIAL_COUNTER},
    wayland::shell::xdg::ToplevelSurface,
};

use std::time::{Duration, Instant};

use smithay::{
    reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State as XdgState,
    utils::Rectangle,
};

use smithay::wayland::shell::xdg::XdgShellHandler;

use crate::{
    config::{Action, Modifiers},
    decorations::{self, Button},
    state::{MoveGrab, ResizeGrab, State},
};

/// Intervalo máximo entre dois cliques para contar como duplo clique.
const DOUBLE_CLICK: Duration = Duration::from_millis(400);

/// Botão esquerdo do mouse, no formato evdev.
pub const BTN_LEFT: u32 = 0x110;
/// Botão direito do mouse, no formato evdev.
pub const BTN_RIGHT: u32 = 0x111;

/// Trata um evento de entrada vindo do backend winit.
/// `size` é o tamanho atual da janela do mide, usado para converter coordenadas.
pub fn handle_event(state: &mut State, event: InputEvent<WinitInput>, size: Size<i32, Logical>) {
    match event {
        InputEvent::Keyboard { event } => keyboard_key(state, event),
        InputEvent::PointerMotionAbsolute { event } => {
            let pos = event.position_transformed(size);
            pointer_motion(state, pos, event.time_msec());
        }
        InputEvent::PointerButton { event } => pointer_button(state, event),
        InputEvent::PointerAxis { event } => pointer_axis(state, event),
        _ => {}
    }
}

fn keyboard_key(state: &mut State, event: impl KeyboardKeyEvent<WinitInput>) {
    key_input(state, event.key_code(), event.state(), event.time_msec());
}

/// Tecla pressionada ou solta, pelo código evdev (usado pelo autoteste).
pub fn evdev_key(state: &mut State, evdev: u32, pressed: bool, time: u32) {
    let key_state = if pressed { KeyState::Pressed } else { KeyState::Released };
    key_input(state, (evdev + 8).into(), key_state, time);
}

/// Código evdev de uma tecla pelo nome (`enter`, `esc`, `ctrl`, letras a-z...).
pub fn evdev_code(name: &str) -> Option<u32> {
    const LETTERS: [u32; 26] = [
        30, 48, 46, 32, 18, 33, 34, 35, 23, 36, 37, 38, 50, 49, 24, 25, 16, 19, 31, 20, 22, 47, 17, 45, 21, 44,
    ];
    let code = match name {
        "enter" => 28,
        "esc" => 1,
        "backspace" => 14,
        "delete" => 111,
        "tab" => 15,
        "space" | "_" => 57,
        "left" => 105,
        "right" => 106,
        "up" => 103,
        "down" => 108,
        "pageup" => 104,
        "pagedown" => 109,
        "ctrl" => 29,
        "shift" => 42,
        "alt" => 56,
        "super" => 125,
        _ => {
            let mut chars = name.chars();
            let c = chars.next()?;
            if chars.next().is_some() || !c.is_ascii_lowercase() {
                return None;
            }
            LETTERS[(c as u8 - b'a') as usize]
        }
    };
    Some(code)
}

/// Envia um atalho (`ctrl+shift+z`) à janela em foco: pressiona na ordem e solta
/// na ordem inversa. Passa pelos atalhos do compositor, como uma tecla de verdade.
pub fn send_combo(state: &mut State, combo: &str) -> bool {
    let Some(codes) = combo.split('+').map(evdev_code).collect::<Option<Vec<u32>>>() else {
        return false;
    };
    let time = state.start_time.elapsed().as_millis() as u32;
    for code in &codes {
        evdev_key(state, *code, true, time);
    }
    for code in codes.iter().rev() {
        evdev_key(state, *code, false, time + 1);
    }
    true
}

/// Tecla pelo código xkb: confere os atalhos e entrega ao foco do teclado.
fn key_input(state: &mut State, keycode: smithay::input::keyboard::Keycode, key_state: KeyState, time: u32) {
    let Some(keyboard) = state.seat.get_keyboard() else {
        return;
    };
    if key_state == KeyState::Pressed {
        state.mark_input("tecla", keyboard.current_focus());
    }
    let serial = SERIAL_COUNTER.next_serial();

    // Um atalho casado é interceptado: a tecla não chega ao app com foco.
    let action = keyboard.input::<Action, _>(
        state,
        keycode,
        key_state,
        serial,
        time,
        |state, mods, keysym| {
            if key_state != KeyState::Pressed {
                return FilterResult::Forward;
            }
            tracing::debug!(
                "tecla: {} (logo={}, ctrl={}, alt={}, shift={})",
                xkbcommon::xkb::keysym_get_name(keysym.modified_sym()),
                mods.logo,
                mods.ctrl,
                mods.alt,
                mods.shift
            );
            let modifiers = Modifiers {
                logo: mods.logo,
                ctrl: mods.ctrl,
                alt: mods.alt,
                shift: mods.shift,
            };
            // Usa a tecla sem o efeito do Shift, para que `ctrl+shift+e` case com `e`.
            let key = keysym.raw_latin_sym_or_raw_current_sym().unwrap_or_else(|| keysym.modified_sym());
            match state.config.match_binding(modifiers, key) {
                Some(action) => FilterResult::Intercept(action),
                None => FilterResult::Forward,
            }
        },
    );
    if let Some(action) = action {
        run_action(state, action);
    }
}

/// Executa a ação de um atalho.
fn run_action(state: &mut State, action: Action) {
    match action {
        Action::CloseWindow => {
            let Some(keyboard) = state.seat.get_keyboard() else {
                return;
            };
            let Some(focus) = keyboard.current_focus() else {
                return;
            };
            if let Some(window) = state.window_for(&focus) {
                if let Some(toplevel) = window.toplevel() {
                    toplevel.send_close();
                    tracing::info!("fechar janela pedido pelo atalho");
                }
            }
        }
        Action::Launch(command) => spawn(&command),
        Action::RestoreMinimized => state.restore_minimized(),
        Action::ToggleLauncher => state.toggle_launcher(),
        Action::Quit => {
            tracing::info!("encerrando por atalho");
            state.quit_requested = true;
        }
    }
}

/// Inicia um comando pelo shell, com o mide como servidor Wayland.
fn spawn(command: &str) {
    let mut process = std::process::Command::new("sh");
    process.arg("-c").arg(command);
    crate::child_env(&mut process);
    let child = process.spawn();
    match child {
        Ok(mut child) => {
            // Espera o processo em outra thread, para não virar zumbi.
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            tracing::info!("comando iniciado: {command}");
        }
        Err(err) => tracing::warn!("falha ao iniciar '{command}': {err}"),
    }
}

/// Move o ponteiro para `pos`, independente do backend (winit, DRM ou teste).
pub fn pointer_motion(state: &mut State, pos: Point<f64, Logical>, time: u32) {
    let Some(pointer) = state.seat.get_pointer() else {
        return;
    };
    state.pointer_location = pos;

    // Prévias da dock: acompanham o ponteiro e somem quando ele sai da dock e do painel.
    if let Some(preview) = state.preview.clone() {
        if crate::preview::keeps_open(state, &preview, pos) {
            let (hovered, on_close) = crate::preview::hit(state, &preview, pos);
            if let Some(p) = state.preview.as_mut() {
                p.hovered = hovered;
                p.on_close = on_close;
            }
        } else {
            state.preview = None;
        }
    }

    // Forma do ponteiro: arraste de borda mantém a forma da borda; fora delas, padrão.
    state.cursor_icon = match (&state.resize_grab, resize_target(state, pos)) {
        (Some(grab), _) => cursor_for(grab.edges),
        (None, Some((_, edges, _, _))) if state.move_grab.is_none() => cursor_for(edges),
        _ => Default::default(),
    };

    // Durante um redimensionamento, a janela acompanha a borda puxada.
    if let Some(grab) = state.resize_grab.clone() {
        resize_to(state, &grab, pos);
        return;
    }

    // Durante um arrasto, a janela segue o cursor e o cliente não recebe o movimento.
    if let Some(grab) = &state.move_grab {
        let window = grab.window.clone();
        let mut loc: Point<i32, Logical> = (pos - grab.offset).to_i32_round();
        // Como no macOS, a barra de título não passa para baixo da barra superior.
        if let Some(area) = state.usable_area() {
            loc.y = loc.y.max(area.loc.y + crate::state::bar_height_for(&window));
        }
        state.space.map_element(window, loc, false);
        return;
    }

    let under = surface_under(state, pos);
    let serial = SERIAL_COUNTER.next_serial();
    pointer.motion(
        state,
        under,
        &MotionEvent {
            location: pos,
            serial,
            time,
        },
    );
    pointer.frame(state);
}

fn pointer_button(state: &mut State, event: impl PointerButtonEvent<WinitInput>) {
    button(state, event.button_code(), event.state(), event.time_msec());
}

/// Trata um botão do ponteiro, independente do backend (winit, DRM ou teste).
pub fn button(state: &mut State, code: u32, button_state: ButtonState, time: u32) {
    let Some(pointer) = state.seat.get_pointer() else {
        return;
    };
    let kind = if button_state == ButtonState::Pressed { "apertar" } else { "soltar" };
    state.mark_input(kind, pointer.current_focus());
    let serial = SERIAL_COUNTER.next_serial();

    if button_state == ButtonState::Pressed && code == BTN_LEFT {
        let pos = pointer.current_location();
        state.press_location = Some(pos);
        tracing::debug!("clique esquerdo em {:?}: superfície sob o ponteiro = {}", pos, surface_under(state, pos).is_some());
        // Clique nas prévias da dock: foca a janela (ou fecha, no ×). Fora delas, somem.
        if let Some(preview) = state.preview.clone() {
            let (hit, on_close) = crate::preview::hit(state, &preview, pos);
            let thumbs = crate::preview::layout(state, &preview).map(|(panel, t)| (panel, t.into_iter().map(|t| (t.id, t.window)).collect::<Vec<_>>()));
            state.preview = None;
            if let Some((panel, thumbs)) = thumbs {
                if let Some((id, window)) = hit.and_then(|i| thumbs.get(i).cloned()) {
                    if on_close {
                        if let Some(toplevel) = window.toplevel() {
                            tracing::info!("prévia: fechar janela {id}");
                            toplevel.send_close();
                        }
                    } else {
                        tracing::info!("prévia: janela {id} escolhida");
                        crate::control::handle(state, &format!("focus {id}"));
                    }
                    return;
                }
                if panel.to_f64().contains(pos) {
                    return;
                }
            }
        }
        // Barra, dock e launcher ficam por cima: o clique é só deles, sem
        // redimensionar, arrastar nem focar a janela que estiver atrás.
        if let Some((surface, _)) = top_layer_under(state, pos) {
            state.focus_layer_if_allowed(&surface);
            pointer.button(
                state,
                &ButtonEvent {
                    button: code,
                    state: button_state.into(),
                    serial,
                    time,
                },
            );
            pointer.frame(state);
            return;
        }
        // Bordas e cantos redimensionam; a barra de título é do mide. Nenhum dos
        // dois chega ao app.
        if resize_press(state, pos, serial) {
            tracing::debug!("clique tratado como redimensionamento");
            return;
        }
        if title_bar_press(state, pos, serial) {
            tracing::debug!("clique tratado pela barra de título");
            return;
        }
        match surface_under(state, pos) {
            None => dismiss_popups(state),
            Some((surface, _)) => state.focus_layer_if_allowed(&surface),
        }
        if let Some(window) = window_under(state, pos) {
            state.space.raise_element(&window, true);
            focus_window(state, &window, serial);
        }
    }
    if button_state == ButtonState::Released && code == BTN_LEFT {
        state.move_grab = None;
        state.resize_grab = None;
        if std::mem::take(&mut state.title_press) {
            return;
        }
    }

    pointer.button(
        state,
        &ButtonEvent {
            button: code,
            state: button_state.into(),
            serial,
            time,
        },
    );
    pointer.frame(state);
}

fn pointer_axis(state: &mut State, event: impl PointerAxisEvent<WinitInput>) {
    let Some(pointer) = state.seat.get_pointer() else {
        return;
    };

    let mut frame = AxisFrame::new(event.time_msec()).source(event.source());
    for axis in [Axis::Horizontal, Axis::Vertical] {
        let steps = event.amount_v120(axis);
        // A roda do mouse vem só em passos (v120); o protocolo também pede o valor
        // contínuo, que muitos clientes leem. 15 px por passo, como nos outros desktops.
        let amount = event.amount(axis).or_else(|| steps.map(|s| s / 120.0 * 15.0));
        if let Some(amount) = amount {
            frame = frame.value(axis, amount);
            // Fim do gesto no touchpad: valor zero com "stop".
            if event.source() == AxisSource::Finger && amount == 0.0 {
                frame = frame.stop(axis);
            }
        }
        if let Some(steps) = steps {
            frame = frame.v120(axis, steps as i32);
        }
    }

    pointer.axis(state, frame);
    pointer.frame(state);
}

/// Roda do mouse (autoteste): `steps` passos, positivo para baixo. Mesmo quadro que
/// a roda de verdade gera (valor contínuo e v120).
pub fn wheel(state: &mut State, steps: i32, time: u32) {
    let Some(pointer) = state.seat.get_pointer() else {
        return;
    };
    state.mark_input("roda", pointer.current_focus());
    let frame = AxisFrame::new(time)
        .source(AxisSource::Wheel)
        .value(Axis::Vertical, steps as f64 * 15.0)
        .v120(Axis::Vertical, steps * 120);
    pointer.axis(state, frame);
    pointer.frame(state);
}

/// A janela recebe input? Janelas sendo minimizadas (ainda na animação) não.
fn interactive(state: &State, window: &Window) -> bool {
    !state.minimized.iter().any(|m| &m.window == window)
}

/// Janela que recebe input sob `pos` (a de cima primeiro) e a origem da superfície dela.
fn window_with_origin_under(state: &State, pos: Point<f64, Logical>) -> Option<(Window, Point<i32, Logical>)> {
    use smithay::desktop::space::SpaceElement;
    state.space.elements().rev().filter(|w| interactive(state, w)).find_map(|window| {
        let origin = state.space.element_location(window)? - window.geometry().loc;
        window
            .is_in_input_region(&(pos - origin.to_f64()))
            .then(|| (window.clone(), origin))
    })
}

/// Janela mapeada sob `pos`, a de cima primeiro.
fn window_under(state: &State, pos: Point<f64, Logical>) -> Option<Window> {
    window_with_origin_under(state, pos).map(|(window, _)| window)
}

/// Camada de topo ou sobreposta (barra, dock, launcher) sob `pos`. Elas ficam por
/// cima das janelas, então um clique nelas não chega a nenhuma janela.
fn top_layer_under(state: &State, pos: Point<f64, Logical>) -> Option<(WlSurface, Point<f64, Logical>)> {
    let output = state.space.outputs().next()?;
    let map = layer_map_for_output(output);
    for layer in crate::state::front_layers(&map).iter() {
        let Some(geo) = map.layer_geometry(layer) else {
            continue;
        };
        if let Some((surface, p)) = layer.surface_under(pos - geo.loc.to_f64(), WindowSurfaceType::ALL) {
            return Some((surface, (p + geo.loc).to_f64()));
        }
    }
    None
}

/// Superfície sob `pos`, com a origem dela em coordenadas globais.
/// Popups são testados antes das janelas, porque ficam por cima delas.
fn surface_under(state: &State, pos: Point<f64, Logical>) -> Option<(WlSurface, Point<f64, Logical>)> {
    // Camadas de topo e overlay (barra, dock, launcher) ficam por cima das janelas.
    if let Some(found) = top_layer_under(state, pos) {
        return Some(found);
    }

    for window in state.space.elements().rev().filter(|w| interactive(state, w)) {
        let (Some(toplevel), Some(loc)) = (window.toplevel(), state.space.element_location(window))
        else {
            continue;
        };
        let root = toplevel.wl_surface();
        for (popup, popup_offset) in PopupManager::popups_for_surface(root) {
            let popup_loc = loc + popup_offset - popup.geometry().loc;
            if let Some((surface, p)) =
                under_from_surface_tree(popup.wl_surface(), pos, popup_loc, WindowSurfaceType::ALL)
            {
                return Some((surface, (p).to_f64()));
            }
        }
    }

    window_with_origin_under(state, pos).and_then(|(window, loc)| {
        window
            .surface_under(pos - loc.to_f64(), WindowSurfaceType::ALL)
            .map(|(surface, p)| (surface, (p + loc).to_f64()))
    })
}

/// Fecha todos os popups abertos. Chamado quando o clique cai fora deles.
fn dismiss_popups(state: &mut State) {
    let roots: Vec<WlSurface> = state
        .space
        .elements()
        .filter_map(|w| w.toplevel().map(|t| t.wl_surface().clone()))
        .collect();
    for root in roots {
        for (popup, _) in PopupManager::popups_for_surface(&root) {
            let _ = PopupManager::dismiss_popup(&root, &popup);
        }
    }
}

/// Dá o foco do teclado à janela e marca só ela como ativa.
pub fn focus_window(state: &mut State, window: &Window, serial: smithay::utils::Serial) {
    let Some(keyboard) = state.seat.get_keyboard() else {
        return;
    };
    let Some(target) = window.toplevel().map(|t| t.wl_surface().clone()) else {
        return;
    };

    tracing::debug!("foco do teclado: nova janela ativa");
    for other in state.space.elements() {
        if let Some(toplevel) = other.toplevel() {
            set_activated(toplevel, other == window);
        }
    }
    keyboard.set_focus(state, Some(target), serial);
}

fn set_activated(toplevel: &ToplevelSurface, activated: bool) {
    use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State as XdgState;

    let changed = toplevel.with_pending_state(|s| {
        let had = s.states.contains(XdgState::Activated);
        if activated {
            s.states.set(XdgState::Activated);
        } else {
            s.states.unset(XdgState::Activated);
        }
        had != activated
    });
    if changed {
        toplevel.send_configure();
    }
}

/// Janela e retângulo da barra de título sob `pos`, se houver.
fn title_bar_under(state: &State, pos: Point<f64, Logical>) -> Option<(Window, decorations::Rect)> {
    state.space.elements().rev().filter(|w| interactive(state, w)).find_map(|window| {
        let loc = state.visible_origin(window)?;
        let width = window.geometry().size.w;
        if crate::state::bar_height_for(window) == 0 {
            return None;
        }
        let bar = decorations::bar_rect(loc, width);
        bar.to_f64().contains(pos).then(|| (window.clone(), bar))
    })
}

/// Trata um clique na barra de título. Devolve `true` se o clique foi da barra.
fn title_bar_press(state: &mut State, pos: Point<f64, Logical>, serial: smithay::utils::Serial) -> bool {
    let Some((window, bar)) = title_bar_under(state, pos) else {
        return false;
    };
    let Some(toplevel) = window.toplevel().cloned() else {
        return false;
    };
    state.title_press = true;
    state.space.raise_element(&window, true);
    focus_window(state, &window, serial);

    match decorations::button_at(bar, pos) {
        Some(Button::Close) => {
            toplevel.send_close();
            tracing::info!("fechar janela pedido pela barra de título");
        }
        Some(Button::Minimize) => {
            let surface = toplevel.wl_surface().clone();
            state.minimize(&surface);
        }
        Some(Button::Maximize) => toggle_maximize(state, toplevel),
        None => {
            let now = Instant::now();
            let double = matches!(
                &state.last_title_click,
                Some((last, when)) if *last == window && now.duration_since(*when) < DOUBLE_CLICK
            );
            if double {
                state.last_title_click = None;
                toggle_maximize(state, toplevel);
            } else {
                state.last_title_click = Some((window.clone(), now));
                let loc = state.space.element_location(&window).unwrap_or_default();
                state.move_grab = Some(MoveGrab {
                    window,
                    offset: pos - loc.to_f64(),
                });
            }
        }
    }
    true
}

/// Maximiza ou restaura a janela, conforme o estado atual.
pub fn toggle_maximize(state: &mut State, toplevel: smithay::wayland::shell::xdg::ToplevelSurface) {
    use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State as XdgState;
    if toplevel.current_state().states.contains(XdgState::Maximized) {
        state.unmaximize_request(toplevel);
    } else {
        state.maximize_request(toplevel);
    }
}

/// Janela, bordas e geometria sob `pos` quando o ponteiro está numa borda que
/// redimensiona. A borda superior sem canto fica para o arrasto da barra.
fn resize_target(
    state: &State,
    pos: Point<f64, Logical>,
) -> Option<(Window, decorations::Edges, Point<i32, Logical>, smithay::utils::Size<i32, Logical>)> {
    if top_layer_under(state, pos).is_some() {
        return None;
    }
    state.space.elements().rev().filter(|w| interactive(state, w)).find_map(|window| {
        let toplevel = window.toplevel()?;
        let loc = state.space.element_location(window)?;
        let flags = toplevel.current_state().states;
        if flags.contains(XdgState::Maximized) {
            return None;
        }
        let geo = window.geometry();
        let visible = state.visible_origin(window)?;
        let bar_h = crate::state::bar_height_for(window);
        let frame = Rectangle::new((visible.x, visible.y - bar_h).into(), (geo.size.w, bar_h + geo.size.h).into());
        let edges = decorations::resize_edges_at(frame, pos)?;
        (!edges.only_top()).then(|| (window.clone(), edges, loc, geo.size))
    })
}

/// Forma do ponteiro para uma borda ou canto.
fn cursor_for(edges: decorations::Edges) -> smithay::reexports::winit::window::CursorIcon {
    use smithay::reexports::winit::window::CursorIcon;
    let horizontal = edges.left || edges.right;
    let vertical = edges.top || edges.bottom;
    match (horizontal, vertical) {
        (true, false) => CursorIcon::EwResize,
        (false, true) => CursorIcon::NsResize,
        _ if (edges.left && edges.top) || (edges.right && edges.bottom) => CursorIcon::NwseResize,
        _ => CursorIcon::NeswResize,
    }
}

/// Inicia o redimensionamento se o clique for numa borda ou canto de uma janela.
fn resize_press(state: &mut State, pos: Point<f64, Logical>, serial: smithay::utils::Serial) -> bool {
    let Some((window, edges, loc, size)) = resize_target(state, pos) else {
        return false;
    };
    state.title_press = true;
    state.space.raise_element(&window, true);
    focus_window(state, &window, serial);
    state.resize_grab = Some(ResizeGrab {
        window,
        edges,
        start_pointer: pos,
        start_loc: loc,
        start_size: size,
    });
    true
}

/// Aplica o tamanho e a posição da janela para o ponteiro atual.
fn resize_to(state: &mut State, grab: &ResizeGrab, pos: Point<f64, Logical>) {
    const MIN_W: i32 = 200;
    const MIN_H: i32 = 120;
    let Some(toplevel) = grab.window.toplevel().cloned() else {
        return;
    };
    let dx = (pos.x - grab.start_pointer.x).round() as i32;
    let dy = (pos.y - grab.start_pointer.y).round() as i32;
    let e = grab.edges;

    let mut w = grab.start_size.w;
    let mut h = grab.start_size.h;
    if e.right {
        w += dx;
    }
    if e.left {
        w -= dx;
    }
    if e.bottom {
        h += dy;
    }
    if e.top {
        h -= dy;
    }
    let w = w.max(MIN_W);
    let h = h.max(MIN_H);

    // Pela esquerda e pelo topo, a origem acompanha a parte que sobrou ou foi cortada.
    let x = if e.left {
        grab.start_loc.x + (grab.start_size.w - w)
    } else {
        grab.start_loc.x
    };
    let y = if e.top {
        grab.start_loc.y + (grab.start_size.h - h)
    } else {
        grab.start_loc.y
    };

    toplevel.with_pending_state(|s| s.size = Some((w, h).into()));
    toplevel.send_configure();
    state.space.map_element(grab.window.clone(), (x, y), false);
}
