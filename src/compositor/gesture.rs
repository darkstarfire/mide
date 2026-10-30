//! Gestos de três dedos no touchpad, como no macOS:
//!
//! - para cima: o launcher entra acompanhando os dedos (aparece e encolhe até o lugar
//!   conforme eles sobem); ao soltar, termina de abrir ou volta, conforme o caminho
//!   andado;
//! - para baixo, com o launcher aberto: ele sai acompanhando os dedos;
//! - para a esquerda/direita: passa entre a mesa (as janelas comuns) e as janelas em
//!   tela cheia da tela sob o ponteiro, na ordem em que entraram em tela cheia, com a
//!   janela chegando de lado. Jogos ficam de fora: não entram na lista e, com um jogo
//!   em tela cheia em foco, os gestos de três dedos não fazem nada.
//!
//! Com quatro dedos (`overview`): para cima, a visão de todas as janelas entra
//! acompanhando os dedos; para baixo, sai; para os lados, a próxima/anterior janela
//! comum da tela vem para a frente.

use std::path::PathBuf;
use std::time::Instant;

use smithay::{
    desktop::{layer_map_for_output, Window},
    utils::Point,
};

use crate::state::{window_id, State, WindowAnimation, ANIMATION_DURATION};

/// Caminho dos dedos (em unidades do touchpad, ~pontos) para o launcher abrir inteiro.
const LAUNCHER_TRAVEL: f64 = 300.0;
/// Caminho até o gesto escolher a direção.
const DECIDE: f64 = 12.0;
/// Caminho para o lado que troca de janela em tela cheia.
const SWITCH: f64 = 80.0;
/// Ao soltar: abre se passou desta fração (e fecha se ficou abaixo de `1 - CLOSE_AT`).
const OPEN_AT: f32 = 0.3;
const CLOSE_AT: f32 = 0.3;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Axis {
    /// Ainda decidindo a direção.
    Undecided,
    /// Launcher acompanhando os dedos; `from` é onde começou (0 fechado, 1 aberto).
    Launcher { from: f32 },
    Sideways,
    /// Quatro dedos: a visão das janelas acompanhando (0 fechada, 1 aberta).
    Overview { from: f32 },
    /// Quatro dedos para os lados: troca a janela em foco.
    Cycle,
    /// Gesto que não faz nada (outra direção, jogo em foco, tela bloqueada).
    Ignored,
}

/// Gesto de três dedos em andamento.
#[derive(Debug, Clone, Copy)]
pub struct Swipe {
    fingers: u32,
    axis: Axis,
    dx: f64,
    dy: f64,
}

/// Começo de um deslizar com `fingers` dedos.
pub fn begin(state: &mut State, fingers: u32) {
    state.swipe = None;
    if fingers != 3 && fingers != 4 {
        return;
    }
    let axis = if state.lock.locked || focused_game(state) { Axis::Ignored } else { Axis::Undecided };
    if axis == Axis::Ignored {
        tracing::info!("gesto: {fingers} dedos ignorados (jogo em tela cheia ou tela bloqueada)");
    }
    state.swipe = Some(Swipe { fingers, axis, dx: 0.0, dy: 0.0 });
}

/// Os dedos andaram (`dx`, `dy`), em unidades do touchpad.
pub fn update(state: &mut State, dx: f64, dy: f64) {
    let Some(mut swipe) = state.swipe else {
        return;
    };
    swipe.dx += dx;
    swipe.dy += dy;
    if swipe.fingers == 4 && swipe.axis == Axis::Undecided && swipe.dx.hypot(swipe.dy) >= DECIDE {
        swipe.axis = four_fingers(state, swipe.dx, swipe.dy);
    }
    if swipe.axis == Axis::Undecided && swipe.dx.hypot(swipe.dy) >= DECIDE {
        swipe.axis = if swipe.dx.abs() > swipe.dy.abs() {
            Axis::Sideways
        } else {
            let open = launcher_shown(state);
            match (swipe.dy < 0.0, open) {
                (true, false) => {
                    tracing::info!("gesto: launcher entrando");
                    // Fechando ainda (esmaecendo)? O gesto pega dali; senão, abre.
                    if !state.launcher_open() {
                        crate::spawn_client("mide-launcher");
                    }
                    state.launcher_gone = false;
                    state.launcher_anim = None;
                    state.launcher_close_pending = false;
                    Axis::Launcher { from: 0.0 }
                }
                (false, true) => {
                    tracing::info!("gesto: launcher saindo");
                    state.launcher_anim = None;
                    Axis::Launcher { from: 1.0 }
                }
                _ => Axis::Ignored,
            }
        };
    }
    if let Axis::Overview { from } = swipe.axis {
        crate::overview::drag(state, (from as f64 - swipe.dy / LAUNCHER_TRAVEL) as f32);
    }
    if let Axis::Launcher { from } = swipe.axis {
        let p = (from as f64 - swipe.dy / LAUNCHER_TRAVEL).clamp(0.0, 1.0) as f32;
        state.launcher_drag = Some(p);
        state.wake();
    }
    state.swipe = Some(swipe);
}

/// Os dedos saíram do touchpad (`cancelled`: o libinput desistiu do gesto).
pub fn end(state: &mut State, cancelled: bool) {
    let Some(swipe) = state.swipe.take() else {
        return;
    };
    match swipe.axis {
        Axis::Launcher { from } => {
            let p = state.launcher_drag.take().unwrap_or(from);
            let open = if cancelled {
                from > 0.5
            } else if from < 0.5 {
                p >= OPEN_AT
            } else {
                p > 1.0 - CLOSE_AT
            };
            settle_launcher(state, p, open);
        }
        Axis::Overview { from } => {
            let p = state.overview.as_ref().map_or(from, |o| o.progress());
            let open = if cancelled {
                from > 0.5
            } else if from < 0.5 {
                p >= OPEN_AT
            } else {
                p > 1.0 - CLOSE_AT
            };
            crate::overview::settle(state, open);
        }
        Axis::Cycle if !cancelled && swipe.dx.abs() >= SWITCH => {
            // Dedos para a esquerda: a próxima (a de baixo da de cima), como os lados dos três.
            crate::overview::cycle(state, if swipe.dx < 0.0 { 1 } else { -1 });
        }
        Axis::Sideways if !cancelled && swipe.dx.abs() >= SWITCH => {
            // Dedos para a esquerda: a próxima (à direita), como os espaços do Mac.
            switch(state, if swipe.dx < 0.0 { 1 } else { -1 });
        }
        _ => {}
    }
    state.wake();
}

/// Direção dos quatro dedos: para os lados troca a janela; para cima abre a visão
/// (fechada) e para baixo fecha (aberta).
fn four_fingers(state: &mut State, dx: f64, dy: f64) -> Axis {
    if dx.abs() > dy.abs() {
        return Axis::Cycle;
    }
    let open = state.overview.as_ref().is_some_and(|o| o.anim.is_none_or(|(_, _, to)| to > 0.5));
    match (dy < 0.0, open) {
        (true, false) => {
            let Some(output) = state.output_at(state.pointer_location) else {
                return Axis::Ignored;
            };
            tracing::info!("gesto: visão das janelas entrando");
            crate::overview::open(state, &output, Some(0.0));
            Axis::Overview { from: 0.0 }
        }
        (false, true) => {
            tracing::info!("gesto: visão das janelas saindo");
            let p = state.overview.as_ref().map_or(1.0, |o| o.progress());
            crate::overview::drag(state, p);
            Axis::Overview { from: p }
        }
        _ => Axis::Ignored,
    }
}

/// Termina o launcher de onde os dedos o deixaram: abre ou fecha.
fn settle_launcher(state: &mut State, p: f32, open: bool) {
    tracing::info!("gesto: launcher {} (em {:.0}%)", if open { "aberto" } else { "fechado" }, p * 100.0);
    let animate = state.config.effects.animations;
    state.launcher_from = p;
    if open {
        state.launcher_anim = (animate && p < 1.0).then(|| (Instant::now(), true));
        return;
    }
    // Fechar: a camada ainda não chegou (o launcher acabou de ser iniciado)? Fecha
    // quando chegar.
    if !state.launcher_seen {
        state.launcher_close_pending = true;
        return;
    }
    if animate && p > 0.0 {
        state.launcher_anim = Some((Instant::now(), false));
    } else {
        state.launcher_anim = None;
        state.launcher_gone = true;
        close_launcher_layer(state);
    }
}

/// Pede ao launcher para fechar.
pub fn close_launcher_layer(state: &State) {
    let layer = state
        .primary_output()
        .and_then(|o| layer_map_for_output(&o).layers().find(|l| l.namespace() == "mide-launcher").cloned());
    if let Some(layer) = layer {
        layer.layer_surface().send_close();
    }
}

/// O launcher está à vista (aberto e não saindo)?
fn launcher_shown(state: &State) -> bool {
    state.launcher_open() && !state.launcher_gone && !state.launcher_anim.is_some_and(|(_, opening)| !opening)
}

/// A janela com o foco do teclado.
fn focused_window(state: &State) -> Option<Window> {
    let focus = state.seat.get_keyboard()?.current_focus()?;
    state.space.elements().find(|w| w.toplevel().is_some_and(|t| t.wl_surface() == &focus)).cloned()
}

/// Um jogo em tela cheia está em foco?
fn focused_game(state: &State) -> bool {
    focused_window(state).is_some_and(|w| state.fullscreen_entry(&w).is_some() && is_game(&app_id_of(&w)))
}

/// Passa para a tela cheia seguinte (`dir` 1) ou anterior (-1) na tela sob o ponteiro.
/// A posição 0 é a mesa: a janela comum de cima vem para a frente.
fn switch(state: &mut State, dir: i32) {
    let Some(output) = state.output_at(state.pointer_location) else {
        return;
    };
    let name = output.name();
    let spaces: Vec<Window> = state
        .fullscreen
        .iter()
        .filter(|e| e.output == name)
        .map(|e| e.window.clone())
        .filter(|w| !state.minimized.iter().any(|m| &m.window == w))
        .filter(|w| !is_game(&app_id_of(w)))
        .collect();
    if spaces.is_empty() {
        tracing::info!("gesto: nenhuma janela em tela cheia em {name}");
        return;
    }
    let current = state
        .fullscreen_on(&output)
        .and_then(|top| spaces.iter().position(|w| *w == top))
        .map_or(0, |i| i + 1) as i32;
    let target = current + dir;
    if target < 0 || target > spaces.len() as i32 {
        tracing::info!("gesto: já na {} (tela cheia {current} de {})", if dir < 0 { "primeira" } else { "última" }, spaces.len());
        return;
    }
    let window = if target == 0 {
        // A mesa: a janela comum (não em tela cheia) de cima nessa tela.
        let geo = state.space.output_geometry(&output);
        let normal = state
            .space
            .elements()
            .rev()
            .filter(|w| state.fullscreen_entry(w).is_none())
            .filter(|w| w.toplevel().is_none_or(|t| t.parent().is_none()))
            .find(|w| {
                state.space.element_location(w).zip(geo).is_some_and(|(loc, geo)| {
                    let size = w.geometry().size;
                    geo.contains(loc + Point::from((size.w / 2, size.h.max(1) / 2)))
                })
            })
            .cloned();
        match normal {
            Some(w) => w,
            None => {
                tracing::info!("gesto: nenhuma janela comum para mostrar em {name}");
                return;
            }
        }
    } else {
        spaces[target as usize - 1].clone()
    };
    let id = window_id(&window);
    tracing::info!("gesto: tela cheia {current} -> {target} em {name} (janela {id})");
    crate::control::handle(state, &format!("focus {id}"));
    // Chega de lado, como a troca de espaços do Mac.
    if !state.config.effects.animations {
        return;
    }
    if let (Some(loc), Some(toplevel), Some(geo)) =
        (state.space.element_location(&window), window.toplevel(), state.space.output_geometry(&output))
    {
        let size = window.geometry().size;
        let surface = toplevel.wl_surface().clone();
        state.animations.retain(|a| a.surface != surface);
        state.animations.push(WindowAnimation {
            window: window.clone(),
            surface,
            from_loc: loc + Point::from((dir * geo.size.w, 0)),
            from_size: size,
            to_loc: loc,
            to_size: size,
            zoom_from: 1.0,
            zoom_to: 1.0,
            alpha_from: 1.0,
            alpha_to: 1.0,
            hide_at_end: false,
            genie: false,
            follow_icon: false,
            duration: ANIMATION_DURATION,
            start: Instant::now(),
        });
    }
}

/// `app_id` da janela (vazio sem ele).
fn app_id_of(window: &Window) -> String {
    window
        .toplevel()
        .and_then(|t| {
            smithay::wayland::compositor::with_states(t.wl_surface(), |s| {
                s.data_map
                    .get::<smithay::wayland::shell::xdg::XdgToplevelSurfaceData>()
                    .and_then(|d| d.lock().ok().and_then(|a| a.app_id.clone()))
            })
        })
        .unwrap_or_default()
}

/// O app é um jogo? Pelo nome (Steam, gamescope, Wine) ou pela categoria `Game` do
/// `.desktop` dele (pelo nome do arquivo ou pelo `StartupWMClass`).
pub fn is_game(app_id: &str) -> bool {
    let id = app_id.to_lowercase();
    if id.is_empty() {
        return false;
    }
    if id.starts_with("steam_app_") || id.starts_with("gamescope") || id.ends_with(".exe") {
        return true;
    }
    desktop_dirs().iter().any(|dir| {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return false;
        };
        entries.flatten().any(|entry| {
            let path = entry.path();
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_lowercase();
            if path.extension().is_none_or(|e| e != "desktop") {
                return false;
            }
            let by_name = stem == id || stem.rsplit('.').next() == Some(id.as_str());
            let Ok(text) = std::fs::read_to_string(&path) else {
                return false;
            };
            entry_is_game(&text, &id, by_name)
        })
    })
}

/// O `.desktop` (texto) descreve o app `id` e é um jogo?
fn entry_is_game(text: &str, id: &str, by_name: bool) -> bool {
    let mut in_main = false;
    let mut matches = by_name;
    let mut game = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_main = line == "[Desktop Entry]";
            continue;
        }
        if !in_main {
            continue;
        }
        if let Some(class) = line.strip_prefix("StartupWMClass=") {
            matches |= class.trim().to_lowercase() == id;
        } else if let Some(categories) = line.strip_prefix("Categories=") {
            game = categories.split(';').any(|c| c.trim() == "Game");
        }
    }
    matches && game
}

/// Pastas `applications` do XDG (a do usuário primeiro).
fn desktop_dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let data_home = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".local/share"));
    let data_dirs = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    std::iter::once(data_home)
        .chain(data_dirs.split(':').filter(|d| !d.is_empty()).map(PathBuf::from))
        .chain([home.join(".local/share/flatpak/exports/share"), PathBuf::from("/var/lib/flatpak/exports/share")])
        .map(|d| d.join("applications"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jogos_pelo_nome_e_pela_categoria() {
        assert!(is_game("steam_app_570"));
        assert!(is_game("Game.exe"));
        assert!(!is_game(""));
        let entry = "[Desktop Entry]\nName=SuperTux\nCategories=Game;ArcadeGame;\n";
        assert!(entry_is_game(entry, "supertux", true));
        assert!(!entry_is_game(entry, "supertux", false));
        let by_class = "[Desktop Entry]\nStartupWMClass=Minecraft\nCategories=Game;\n";
        assert!(entry_is_game(by_class, "minecraft", false));
        let player = "[Desktop Entry]\nName=mpv\nCategories=AudioVideo;Player;\n";
        assert!(!entry_is_game(player, "mpv", true));
        // A categoria de uma ação ([Desktop Action ...]) não conta.
        let action = "[Desktop Entry]\nCategories=Utility;\n[Desktop Action x]\nCategories=Game;\n";
        assert!(!entry_is_game(action, "x", true));
    }
}
