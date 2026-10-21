//! Entrada de teclado e ponteiro vinda do backend, repassada aos clientes.
//!
//! O foco do teclado segue o clique: a janela clicada vira a janela ativa e
//! recebe as teclas. O ponteiro envia movimento, botões e rolagem à superfície
//! que estiver sob o cursor.

use smithay::{
    backend::input::{
        AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, InputBackend, InputEvent, KeyState,
        KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent, PointerMotionEvent,
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

/// Forma do ponteiro pedida pelo mide (bordas de redimensionar). Cada backend a
/// traduz: o winit muda o cursor da janela do anfitrião; no DRM, o compositor
/// desenha a imagem correspondente.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CursorShape {
    #[default]
    Default,
    ResizeHorizontal,
    ResizeVertical,
    ResizeNwse,
    ResizeNesw,
}

/// Trata um evento de entrada vindo do backend (winit hoje, libinput depois).
/// `size` é o tamanho atual da saída, usado para converter posições absolutas.
pub fn handle_event<B: InputBackend>(state: &mut State, event: InputEvent<B>, size: Size<i32, Logical>) {
    handle_event_in(state, event, smithay::utils::Rectangle::from_size(size));
}

/// Como `handle_event`, com várias telas: `bounds` é a caixa que contém todas (para
/// posições absolutas). O ponteiro anda de uma tela para a outra e não sai delas.
pub fn handle_event_in<B: InputBackend>(state: &mut State, event: InputEvent<B>, bounds: smithay::utils::Rectangle<i32, Logical>) {
    match event {
        InputEvent::Keyboard { event } => keyboard_key(state, event),
        InputEvent::PointerMotionAbsolute { event } => {
            let pos = event.position_transformed(bounds.size) + bounds.loc.to_f64();
            pointer_motion(state, pos, event.time_msec());
        }
        // Mouse e touchpad de verdade (libinput) andam por deslocamento; o ponteiro
        // passa entre as telas, mas não sai delas (nem cai num vão entre telas de
        // alturas diferentes).
        InputEvent::PointerMotion { event } => {
            let pos = state.pointer_location + event.delta();
            let pos = if state.space.output_under(pos).next().is_some() {
                pos
            } else {
                let screen = state
                    .space
                    .output_under(state.pointer_location)
                    .next()
                    .and_then(|o| state.space.output_geometry(o))
                    .unwrap_or(bounds);
                let mut pos = pos;
                pos.x = pos.x.clamp(screen.loc.x as f64, (screen.loc.x + screen.size.w - 1) as f64);
                pos.y = pos.y.clamp(screen.loc.y as f64, (screen.loc.y + screen.size.h - 1) as f64);
                pos
            };
            pointer_motion(state, pos, event.time_msec());
        }
        InputEvent::PointerButton { event } => pointer_button(state, event),
        InputEvent::PointerAxis { event } => pointer_axis(state, event),
        _ => {}
    }
}

fn keyboard_key<B: InputBackend>(state: &mut State, event: impl KeyboardKeyEvent<B>) {
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
        "." => 52,
        "," => 51,
        "/" => 53,
        "-" => 12,
        "=" => 13,
        "0" => 11,
        d @ ("1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9") => d.parse::<u32>().ok()? + 1,
        "tab" => 15,
        // Teclas mortas do ABNT2: acento agudo (´) e til (~).
        "leftbrace" => 26,
        "apostrophe" => 40,
        "space" | "_" => 57,
        "left" => 105,
        "right" => 106,
        "up" => 103,
        "down" => 108,
        "pageup" => 104,
        "pagedown" => 109,
        "home" => 102,
        "end" => 107,
        "ctrl" => 29,
        "shift" => 42,
        "alt" => 56,
        "super" => 125,
        // Print Screen (captura de tela).
        "print" => 99,
        "f1" => 59,
        "f2" => 60,
        "f3" => 61,
        "f4" => 62,
        "f5" => 63,
        "f6" => 64,
        "f7" => 65,
        "f8" => 66,
        "f9" => 67,
        "f10" => 68,
        "f11" => 87,
        "f12" => 88,
        // Botão de ligar do computador (KEY_POWER).
        "power" => 116,
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

/// Teclas no estilo Mac (`[keyboard] mac_style`), em códigos evdev: o Alt (esquerdo)
/// é o ⌘ Command e o Super é o ⌥ Option; o Ctrl continua Ctrl e o Alt da direita
/// continua AltGr (acentos e símbolos do ABNT2). Nos apps, o ⌘ chega como Ctrl (⌘C,
/// ⌘V, ⌘L); no terminal, como Super, para o Ctrl continuar interrompendo (Ctrl+C) e o
/// ⌘ copiar e colar. O ⌥ chega como Alt.
pub fn mac_map(evdev: u32, terminal: bool) -> u32 {
    match evdev {
        56 if terminal => 125, // ⌘ -> Super (o terminal trata o ⌘)
        56 => 29,              // ⌘ -> Ctrl
        125 | 126 => 56,       // ⌥ -> Alt
        other => other,
    }
}

/// No estilo Mac, combinações do ⌘ que ficam com o app, como no Mac (⌘A seleciona
/// tudo, ⌘L vai para a barra de endereço, ⌘ com as setas anda no texto), mesmo que um
/// atalho do mide use a tecla Super com elas.
fn mac_app_combo(mods: Modifiers, key: xkbcommon::xkb::Keysym) -> bool {
    use xkbcommon::xkb::Keysym;
    let plain_cmd = mods.logo && !mods.ctrl && !mods.alt;
    let lower = xkbcommon::xkb::keysym_to_utf32(key);
    plain_cmd && (matches!(key, Keysym::Left | Keysym::Right | Keysym::Up | Keysym::Down) || ['a', 'd', 'l'].iter().any(|c| *c as u32 == lower))
}

/// Solta todas as teclas que o mide acha que estão apertadas (sem passar pelos
/// atalhos): os apps recebem o soltar e os modificadores voltam ao normal.
pub fn release_all_keys(state: &mut State) {
    let Some(keyboard) = state.seat.get_keyboard() else {
        return;
    };
    let pressed = keyboard.pressed_keys();
    if pressed.is_empty() {
        return;
    }
    tracing::debug!("teclado: soltando {} tecla(s) presa(s)", pressed.len());
    let time = state.start_time.elapsed().as_millis() as u32;
    for code in pressed {
        keyboard.input::<(), _>(state, code, KeyState::Released, SERIAL_COUNTER.next_serial(), time, |_, _, _| FilterResult::Forward);
    }
}

fn key_input(state: &mut State, keycode: smithay::input::keyboard::Keycode, key_state: KeyState, time: u32) {
    let Some(keyboard) = state.seat.get_keyboard() else {
        return;
    };
    // Estilo Mac: o ⌘ e o ⌥ viram outras teclas (`mac_map`). O soltar usa a mesma troca
    // do apertar (o foco pode ter mudado de um app para o terminal no meio).
    let mac = state.config.keyboard.mac_style;
    let terminal = mac && focused_is_terminal(state);
    let keycode = if mac {
        let raw = keycode.raw() - 8;
        let pressed = key_state == KeyState::Pressed;
        match raw {
            29 | 97 => state.mac_keys.ctrl = pressed,
            56 => state.mac_keys.command = pressed,
            _ => {}
        }
        let mapped = if pressed {
            let mapped = mac_map(raw, terminal);
            state.mac_keys.pressed.insert(raw, mapped);
            mapped
        } else {
            state.mac_keys.pressed.remove(&raw).unwrap_or_else(|| mac_map(raw, terminal))
        };
        (mapped + 8).into()
    } else {
        keycode
    };
    if key_state == KeyState::Pressed {
        state.mark_input("tecla", keyboard.current_focus());
    }
    let serial = SERIAL_COUNTER.next_serial();
    // Tela bloqueada: as teclas só vão para a tela de bloqueio.
    let locked = state.lock.locked;
    if locked {
        let lock_focus = state
            .primary_output()
            .and_then(|o| state.lock.surface_for(&o).map(|s| s.wl_surface().clone()))
            .or_else(|| state.lock.surfaces.iter().find(|(_, s)| s.alive()).map(|(_, s)| s.wl_surface().clone()));
        if keyboard.current_focus() != lock_focus && lock_focus.is_some() {
            keyboard.set_focus(state, lock_focus, serial);
        }
    }

    // Um atalho casado é interceptado: a tecla não chega ao app com foco. Gravando um
    // atalho (Configurações), a combinação é guardada e também não chega ao app.
    let action = keyboard.input::<Option<Action>, _>(
        state,
        keycode,
        key_state,
        serial,
        time,
        |state, mods, keysym| {
            let switching = state.preview.as_ref().is_some_and(|p| p.switcher);
            if key_state != KeyState::Pressed {
                // Alt (ou Super) solto: o alternador foca a janela escolhida.
                if switching && !mods.alt && !mods.logo {
                    finish_switcher(state, true);
                }
                return FilterResult::Forward;
            }
            // Tela bloqueada: nenhum atalho do mide, só trocar de console (Ctrl+Alt+Fn);
            // o resto vai para a tela de bloqueio (sem ela, a nenhum lugar).
            // Botão de ligar: o mide pergunta antes de desligar (o logind não desliga
            // sozinho, porque o mide segura o botão; veja `session`). Com a tela
            // bloqueada, só acende a tela.
            if keysym.modified_sym() == xkbcommon::xkb::Keysym::XF86_PowerOff {
                return FilterResult::Intercept((!locked).then_some(Action::PowerButton));
            }
            if locked {
                let vt = keysym.modified_sym().raw();
                if (xkbcommon::xkb::keysyms::KEY_XF86Switch_VT_1..=xkbcommon::xkb::keysyms::KEY_XF86Switch_VT_12).contains(&vt) {
                    let n = (vt - xkbcommon::xkb::keysyms::KEY_XF86Switch_VT_1 + 1) as i32;
                    return FilterResult::Intercept(Some(Action::SwitchVt(n)));
                }
                return FilterResult::Forward;
            }
            if switching && keysym.modified_sym() == xkbcommon::xkb::Keysym::Escape {
                finish_switcher(state, false);
                return FilterResult::Intercept(None);
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
            // Ctrl+Alt+F1…F12 (o xkb entrega como XF86Switch_VT_n): trocar de console.
            let vt = keysym.modified_sym().raw();
            if (xkbcommon::xkb::keysyms::KEY_XF86Switch_VT_1..=xkbcommon::xkb::keysyms::KEY_XF86Switch_VT_12).contains(&vt) {
                let n = (vt - xkbcommon::xkb::keysyms::KEY_XF86Switch_VT_1 + 1) as i32;
                return FilterResult::Intercept(Some(Action::SwitchVt(n)));
            }
            // Usa a tecla sem o efeito do Shift, para que `ctrl+shift+e` case com `e`.
            let key = keysym.raw_latin_sym_or_raw_current_sym().unwrap_or_else(|| keysym.modified_sym());
            // Estilo Mac (fora do terminal): os atalhos do mide veem o ⌘ como Super e o
            // Ctrl como Ctrl (⌘Tab troca de janela, Ctrl+⌘+Q bloqueia). Os apps recebem
            // o ⌘ como Ctrl, então os atalhos com Ctrl também valem com o ⌘ (⌘Espaço
            // abre o buscador). Bloquear exige o Ctrl (⌘L é do app).
            let mac_mods = Modifiers { logo: state.mac_keys.command, ctrl: state.mac_keys.ctrl, ..modifiers };
            let modifiers = if mac && !terminal && state.shortcut_capture.is_none() {
                match state.config.match_binding(mac_mods, key) {
                    Some(Action::LockScreen) if !mac_mods.ctrl => {}
                    Some(_) if mac_app_combo(mac_mods, key) => {}
                    Some(action) => return FilterResult::Intercept(Some(action)),
                    None => {}
                }
                modifiers
            } else if mac && !terminal {
                // Gravando um atalho: grava como no Mac (⌘ = Super).
                mac_mods
            } else {
                modifiers
            };
            if let Some(capture) = state.shortcut_capture.as_mut().filter(|c| !c.expired() && c.combo.is_none()) {
                // Só modificador ainda (Ctrl, Shift...): espera a tecla da combinação.
                if keysym.modified_sym().is_modifier_key() {
                    return FilterResult::Forward;
                }
                let plain = !(mods.logo || mods.ctrl || mods.alt || mods.shift);
                let combo = if plain && keysym.modified_sym() == xkbcommon::xkb::Keysym::Escape {
                    "cancel".to_string()
                } else {
                    crate::config::combo_name(modifiers, key)
                };
                tracing::info!("atalhos: combinação gravada: {combo}");
                capture.combo = Some(combo);
                return FilterResult::Intercept(None);
            }
            // O Terminal do mide trata o ⌘ ele mesmo (⌘T, ⌘D, ⌘F...), e o atalho que
            // abre um terminal vira "nova aba" nele.
            // No terminal (estilo Mac), o ⌘ chega como Super: as mesmas regras dos apps
            // valem aqui (⌘L, ⌘D, ⌘A e ⌘ com setas são do app; bloquear exige o Ctrl).
            let real_ctrl = state.mac_keys.ctrl;
            let app_combo = mac && terminal && mac_app_combo(modifiers, key);
            let for_app = |action: &Action| app_combo || (mac && terminal && *action == Action::LockScreen && !real_ctrl);
            if focused_app_id(state).as_deref() == Some("mide-terminal") {
                return match state.config.match_binding(modifiers, key) {
                    Some(Action::Launch(command)) if command.trim() == "mide-terminal" => FilterResult::Forward,
                    Some(action) if for_app(&action) => FilterResult::Forward,
                    Some(action) => FilterResult::Intercept(Some(action)),
                    None => FilterResult::Forward,
                };
            }
            // Terminal no estilo Mac: ⌘C copia e ⌘V cola (Ctrl+Shift+C/V do terminal).
            if terminal && mods.logo && !mods.ctrl && !mods.alt && !mods.shift {
                let c = xkbcommon::xkb::keysym_to_utf32(key);
                if c == 'c' as u32 || c == 'v' as u32 {
                    return FilterResult::Intercept(Some(Action::TerminalKey(char::from_u32(c).unwrap_or('c'))));
                }
            }
            match state.config.match_binding(modifiers, key) {
                Some(action) if for_app(&action) => FilterResult::Forward,
                Some(action) => FilterResult::Intercept(Some(action)),
                None => FilterResult::Forward,
            }
        },
    );
    if let Some(Some(action)) = action {
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
                state.request_close(&window);
                tracing::info!("fechar janela pedido pelo atalho");
            }
        }
        Action::Launch(command) => spawn(&command),
        Action::RestoreMinimized => state.restore_minimized(),
        Action::ToggleLauncher => state.toggle_launcher(),
        Action::ToggleSearch => state.toggle_search(),
        // O que fazer vem de Configurações → Energia (`[power] button`).
        Action::PowerButton => match state.config.power.button.as_str() {
            "suspend" => {
                tracing::info!("botão de ligar: suspendendo");
                mide::system::power(mide::system::Power::Sleep);
            }
            "poweroff" => {
                tracing::info!("botão de ligar: desligando");
                mide::system::power(mide::system::Power::Shutdown);
            }
            "nothing" => tracing::info!("botão de ligar: nada a fazer (Configurações → Energia)"),
            _ => {
                tracing::info!("botão de ligar: pedindo confirmação para desligar");
                state.bar_request = Some("confirm-shutdown");
            }
        },
        Action::SwitchVt(n) => match state.session.as_mut() {
            Some(session) => {
                use smithay::backend::session::Session;
                tracing::info!("trocando para o console {n}");
                if let Err(err) = session.change_vt(n) {
                    tracing::warn!("não foi possível trocar para o console {n}: {err}");
                }
            }
            // Dentro de um anfitrião (winit), quem troca de console é o anfitrião.
            None => tracing::debug!("console {n}: sem sessão própria"),
        },
        Action::Quit => {
            tracing::info!("encerrando por atalho");
            state.quit_requested = true;
        }
        Action::TerminalKey(c) => {
            // Solta o ⌘ para o terminal, manda Ctrl+Shift+letra e aperta o ⌘ de novo (ele
            // continua segurado de verdade).
            let Some(letter) = evdev_code(&c.to_string()) else {
                return;
            };
            let time = state.start_time.elapsed().as_millis() as u32;
            for (code, pressed) in [(125, false), (29, true), (42, true), (letter, true), (letter, false), (42, false), (29, false), (125, true)] {
                evdev_key(state, code, pressed, time);
            }
            tracing::debug!("terminal no estilo Mac: ⌘{c} → Ctrl+Shift+{c}");
        }
        Action::SwitchWindows => switch_windows(state, true),
        Action::SwitchWindowsBack => switch_windows(state, false),
        Action::Maximize | Action::Unmaximize | Action::ToggleMaximize | Action::Minimize | Action::TileLeft | Action::TileRight => {
            use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State as XdgState;
            let Some(toplevel) = focused_toplevel(state) else {
                return;
            };
            let maximized = toplevel.current_state().states.contains(XdgState::Maximized);
            match action {
                Action::Maximize if !maximized => state.maximize(&toplevel),
                Action::Unmaximize => state.unmaximize(&toplevel),
                Action::ToggleMaximize => toggle_maximize(state, toplevel),
                Action::Minimize => state.minimize(toplevel.wl_surface()),
                Action::TileLeft => state.tile(&toplevel, true),
                Action::TileRight => state.tile(&toplevel, false),
                _ => {}
            }
        }
        Action::ShowDesktop => {
            // Minimiza todas as janelas à mostra (a área de trabalho aparece).
            let surfaces: Vec<_> = state.space.elements().filter_map(|w| w.toplevel().map(|t| t.wl_surface().clone())).collect();
            tracing::info!("mostrar a área de trabalho: {} janela(s) minimizada(s)", surfaces.len());
            for surface in surfaces {
                state.minimize(&surface);
            }
        }
        // A tela de bloqueio do mide (`mide-lock`), pelo protocolo de bloqueio.
        Action::LockScreen => crate::lock::request(state, "atalho"),
    }
}

/// Apps de terminal (o estilo Mac trata o teclado como no Terminal do Mac).
const TERMINALS: &[&str] = &[
    "mide-terminal", "foot", "footclient", "org.codeberg.dnkl.foot", "kitty", "alacritty", "org.alacritty.alacritty", "org.gnome.terminal",
    "gnome-terminal-server", "org.gnome.console", "kgx", "org.gnome.ptyxis", "ptyxis", "konsole", "org.kde.konsole",
    "xterm", "uxterm", "tilix", "com.gexperts.tilix", "terminator", "wezterm", "org.wezfurlong.wezterm",
    "com.mitchellh.ghostty", "ghostty", "xfce4-terminal",
];

/// O app com o foco do teclado é um terminal?
fn focused_is_terminal(state: &State) -> bool {
    focused_app_id(state).is_some_and(|id| TERMINALS.contains(&id.as_str()))
}

/// `app_id` (em minúsculas) da janela com o foco do teclado.
fn focused_app_id(state: &State) -> Option<String> {
    let toplevel = focused_toplevel(state)?;
    smithay::wayland::compositor::with_states(toplevel.wl_surface(), |s| {
        s.data_map
            .get::<smithay::wayland::shell::xdg::XdgToplevelSurfaceData>()
            .and_then(|d| d.lock().ok().and_then(|a| a.app_id.clone()))
    })
    .map(|id| id.to_lowercase())
}

/// Janela com o foco do teclado.
fn focused_toplevel(state: &State) -> Option<smithay::wayland::shell::xdg::ToplevelSurface> {
    let focus = state.seat.get_keyboard()?.current_focus()?;
    state.window_for(&focus)?.toplevel().cloned()
}

/// Alt+Tab: abre o alternador (painel com as janelas, a de cima primeiro, já na
/// próxima) ou avança/volta a escolha. Soltar o Alt foca a escolhida.
fn switch_windows(state: &mut State, forward: bool) {
    let mut ids: Vec<u64> = state.space.elements().rev().map(crate::state::window_id).collect();
    ids.extend(state.minimized.iter().rev().map(|m| crate::state::window_id(&m.window)));
    if ids.is_empty() {
        return;
    }
    let n = ids.len();
    let step = |i: usize| if forward { (i + 1) % n } else { (i + n - 1) % n };
    let current = state.preview.as_ref().filter(|p| p.switcher);
    let next = match current {
        Some(p) => step(p.hovered.unwrap_or(0)),
        None => step(0),
    };
    // A entrada (esmaecer) só quando o alternador abre, não a cada Tab.
    let since = match current {
        Some(p) => p.since,
        None => state.config.effects.transitions.then(std::time::Instant::now),
    };
    tracing::info!("alternador: janela {} de {n}", next + 1);
    state.preview = Some(crate::preview::Preview { ids, dock_x: 0, hovered: Some(next), on_close: false, switcher: true, since });
    state.wake();
}

/// Fecha o alternador: foca a janela escolhida (ou só fecha, com Esc).
fn finish_switcher(state: &mut State, choose: bool) {
    let Some(preview) = state.preview.take() else {
        return;
    };
    state.wake();
    let Some(id) = preview.hovered.and_then(|i| preview.ids.get(i).copied()).filter(|_| choose) else {
        tracing::info!("alternador fechado");
        return;
    };
    tracing::info!("alternador: janela {id} escolhida");
    crate::control::handle(state, &format!("focus {id}"));
}

/// Inicia um comando pelo shell, com o mide como servidor Wayland.
pub(crate) fn spawn(command: &str) {
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
    state.activity();
    // Tela bloqueada: o ponteiro só vê a tela de bloqueio da tela onde está.
    if state.lock.locked {
        let under = lock_surface_under(state, pos);
        pointer.motion(state, under, &MotionEvent { location: pos, serial: SERIAL_COUNTER.next_serial(), time });
        pointer.frame(state);
        return;
    }

    // Prévias da dock: acompanham o ponteiro e somem quando ele sai da dock e do painel.
    if let Some(preview) = state.preview.clone() {
        if crate::preview::keeps_open(state, &preview, pos) {
            let (hovered, on_close) = crate::preview::hit(state, &preview, pos);
            if let Some(p) = state.preview.as_mut() {
                // No alternador (Alt+Tab), o mouse só troca a escolha em cima de uma
                // miniatura (fora delas, fica a do teclado).
                if !p.switcher || hovered.is_some() {
                    p.hovered = hovered;
                    p.on_close = on_close && !p.switcher;
                }
            }
        } else {
            state.preview = None;
        }
    }

    // Forma do ponteiro: arraste de borda mantém a forma da borda; fora delas, padrão.
    state.cursor = match (&state.resize_grab, resize_target(state, pos)) {
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

    // Em cima do painel das prévias (ou do alternador), o ponteiro é do mide: a janela
    // de trás não recebe o movimento (nem troca o cursor para o de texto dela).
    let over_preview = state
        .preview
        .as_ref()
        .and_then(|p| crate::preview::layout(state, p))
        .is_some_and(|(panel, _)| panel.to_f64().contains(pos));
    let under = if over_preview {
        state.cursor_status = smithay::input::pointer::CursorImageStatus::default_named();
        None
    } else {
        surface_under(state, pos)
    };
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

fn pointer_button<B: InputBackend>(state: &mut State, event: impl PointerButtonEvent<B>) {
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

    // Tela bloqueada: o clique vai só para a tela de bloqueio (que também pega o teclado).
    if state.lock.locked {
        if let Some((surface, _)) = lock_surface_under(state, pointer.current_location()) {
            if let Some(keyboard) = state.seat.get_keyboard() {
                keyboard.set_focus(state, Some(surface), serial);
            }
        }
        pointer.button(state, &ButtonEvent { button: code, state: button_state.into(), serial, time });
        pointer.frame(state);
        return;
    }

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
                        tracing::info!("prévia: fechar janela {id}");
                        state.request_close(&window);
                    } else {
                        tracing::info!("prévia: janela {id} escolhida");
                        // Com o launcher aberto, ele fecha e a janela escolhida aparece.
                        if state.launcher_open() {
                            state.toggle_launcher();
                        }
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
            Some((surface, _)) => {
                // Clique na área de trabalho também fecha os popups abertos.
                if back_layer_under(state, pos).is_some_and(|(s, _)| s == surface) {
                    dismiss_popups(state);
                }
                state.focus_layer_if_allowed(&surface);
            }
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

fn pointer_axis<B: InputBackend>(state: &mut State, event: impl PointerAxisEvent<B>) {
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
    // As camadas ficam na tela sob o ponto; a posição delas é relativa a essa tela.
    let output = state.output_at(pos)?;
    let screen = state.space.output_geometry(&output)?.loc;
    let map = layer_map_for_output(&output);
    for layer in crate::state::front_layers(&map).iter() {
        let Some(geo) = map.layer_geometry(layer) else {
            continue;
        };
        let loc = geo.loc + screen;
        if let Some((surface, p)) = layer.surface_under(pos - loc.to_f64(), WindowSurfaceType::ALL) {
            return Some((surface, (p + loc).to_f64()));
        }
    }
    None
}

/// Camada de fundo (área de trabalho) sob `pos`, quando não há janela por cima.
fn back_layer_under(state: &State, pos: Point<f64, Logical>) -> Option<(WlSurface, Point<f64, Logical>)> {
    let output = state.output_at(pos)?;
    let screen = state.space.output_geometry(&output)?.loc;
    let map = layer_map_for_output(&output);
    for layer in crate::state::back_layers(&map).iter() {
        let Some(geo) = map.layer_geometry(layer) else {
            continue;
        };
        let loc = geo.loc + screen;
        if let Some((surface, p)) = layer.surface_under(pos - loc.to_f64(), WindowSurfaceType::ALL) {
            return Some((surface, (p + loc).to_f64()));
        }
    }
    None
}

/// Tela de bloqueio da tela sob `pos`, com a origem dela (o canto da tela).
fn lock_surface_under(state: &State, pos: Point<f64, Logical>) -> Option<(WlSurface, Point<f64, Logical>)> {
    let output = state.space.output_under(pos).next()?;
    let origin = state.space.output_geometry(output)?.loc.to_f64();
    let surface = state.lock.surface_for(output)?;
    Some((surface.wl_surface().clone(), origin))
}

/// Superfície sob `pos`, com a origem dela em coordenadas globais.
/// Popups são testados antes das janelas, porque ficam por cima delas.
fn surface_under(state: &State, pos: Point<f64, Logical>) -> Option<(WlSurface, Point<f64, Logical>)> {
    // Camadas de topo e overlay (barra, dock, launcher) ficam por cima das janelas.
    if let Some(found) = top_layer_under(state, pos) {
        return Some(found);
    }

    if let Some(found) = popup_under(state, pos) {
        return Some(found);
    }

    if let Some((window, loc)) = window_with_origin_under(state, pos) {
        return window
            .surface_under(pos - loc.to_f64(), WindowSurfaceType::ALL)
            .map(|(surface, p)| (surface, (p + loc).to_f64()));
    }
    // Sem janela: a área de trabalho (ícones), se houver.
    back_layer_under(state, pos)
}

/// Popup (menu de um app) sob `pos`. Eles ficam por cima de todas as janelas.
fn popup_under(state: &State, pos: Point<f64, Logical>) -> Option<(WlSurface, Point<f64, Logical>)> {
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
    None
}

/// Moldura de uma janela na tela: barra de título do mide (se tiver) e conteúdo.
fn window_frame(state: &State, window: &Window) -> Option<decorations::Rect> {
    let visible = state.visible_origin(window)?;
    let size = window.geometry().size;
    let bar_h = crate::state::bar_height_for(window);
    Some(Rectangle::new((visible.x, visible.y - bar_h).into(), (size.w, bar_h + size.h).into()))
}

/// A janela de cima no ponto `pos`: a primeira, de cima para baixo, cuja moldura ou
/// área de entrada (com as sombras e bordas do app) contém o ponto. Barra de título e
/// bordas de redimensionar só valem para ela: as de uma janela de trás, coberta por
/// outra (um navegador com barra própria, por exemplo), não recebem o clique.
/// Sobre um popup, nenhuma.
fn window_on_top_at(state: &State, pos: Point<f64, Logical>) -> Option<Window> {
    use smithay::desktop::space::SpaceElement;
    if popup_under(state, pos).is_some() {
        return None;
    }
    state.space.elements().rev().filter(|w| interactive(state, w)).find(|window| {
        let in_frame = window_frame(state, window).is_some_and(|f| f.to_f64().contains(pos));
        let in_input = state.space.element_location(window).is_some_and(|loc| {
            let origin = loc - window.geometry().loc;
            window.is_in_input_region(&(pos - origin.to_f64()))
        });
        in_frame || in_input
    }).cloned()
}

/// Fecha todos os popups abertos. Chamado quando o clique cai fora deles.
fn dismiss_popups(state: &mut State) {
    let mut roots: Vec<WlSurface> = state
        .space
        .elements()
        .filter_map(|w| w.toplevel().map(|t| t.wl_surface().clone()))
        .collect();
    // E os das camadas (menus de painéis de terceiros).
    for output in state.space.outputs() {
        roots.extend(layer_map_for_output(output).layers().map(|l| l.wl_surface().clone()));
    }
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
    let window = window_on_top_at(state, pos)?;
    let loc = state.visible_origin(&window)?;
    let width = window.geometry().size.w;
    if crate::state::bar_height_for(&window) == 0 {
        return None;
    }
    let bar = decorations::bar_rect(loc, width);
    bar.to_f64().contains(pos).then_some((window, bar))
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
            state.request_close(&window);
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
        state.unmaximize(&toplevel);
    } else {
        state.maximize(&toplevel);
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
    let window = window_on_top_at(state, pos)?;
    let toplevel = window.toplevel()?;
    let loc = state.space.element_location(&window)?;
    let flags = toplevel.current_state().states;
    if flags.contains(XdgState::Maximized) {
        return None;
    }
    let size = window.geometry().size;
    let edges = decorations::resize_edges_at(window_frame(state, &window)?, pos)?;
    (!edges.only_top()).then_some((window, edges, loc, size))
}

/// Forma do ponteiro para uma borda ou canto.
fn cursor_for(edges: decorations::Edges) -> CursorShape {
    let horizontal = edges.left || edges.right;
    let vertical = edges.top || edges.bottom;
    match (horizontal, vertical) {
        (true, false) => CursorShape::ResizeHorizontal,
        (false, true) => CursorShape::ResizeVertical,
        _ if (edges.left && edges.top) || (edges.right && edges.bottom) => CursorShape::ResizeNwse,
        _ => CursorShape::ResizeNesw,
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
