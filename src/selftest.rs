//! Autoteste de entrada: executa um roteiro de ações de ponteiro em frames definidos.
//!
//! Serve para testar cliques, arrastos e hit-testing sem um mouse de verdade. O
//! roteiro vem de `MIDE_SELFTEST`, com passos separados por `;`, no formato
//! `frame ação [x y]`:
//!
//! - `move x y`: move o ponteiro
//! - `press x y` / `release x y`: botão esquerdo pressionado ou solto em (x, y)
//! - `click x y`: move, pressiona e solta
//! - `rclick x y`: clique com o botão direito
//! - `list`: registra no log as janelas e camadas, com posição e moldura
//! - `restore`: restaura a janela minimizada mais recente
//! - `resize L A`: simula a saída mudando para L x A (janela do host redimensionada)
//! - `ctrl on` / `ctrl off`: finge o Ctrl pressionado (para o Ctrl+clique na dock)
//! - `key nome`: pressiona e solta uma tecla (`enter`, `esc`, `backspace`, setas,
//!   `space` ou uma letra)
//! - `type texto`: digita letras minúsculas (a-z) e espaços (use `_` para espaço)
//! - `combo ctrl+shift+space`: atalho, passando pelos atalhos do compositor
//! - `wheel down` / `wheel up` (ou um número de passos): roda do mouse
//! - `mclick x y`: clique do botão do meio
//!
//! Exemplo: `MIDE_SELFTEST="300 list; 320 click 540 160"`.

use smithay::{
    backend::input::ButtonState,
    desktop::layer_map_for_output,
    utils::{Logical, Point},
};

use crate::{
    input::{self, BTN_LEFT, BTN_RIGHT},
    state::{bar_height_for, State},
};

#[derive(Debug, Clone, Copy)]
enum Step {
    Move(f64, f64),
    Press(f64, f64),
    Release(f64, f64),
    Click(f64, f64),
    RightClick(f64, f64),
    List,
    Restore,
    Ctrl(bool),
    Resize(i32, i32),
    /// Código evdev da tecla e se foi pressionada.
    Key(u32, bool),
    /// Roda do mouse: passos (positivo = para baixo).
    Wheel(i32),
    MiddleClick(f64, f64),
}

/// Roteiro de autoteste em andamento.
pub struct SelfTest {
    steps: Vec<(u32, Step)>,
    frame: u32,
}

impl SelfTest {
    /// Lê o roteiro de `MIDE_SELFTEST`. Sem a variável, não há autoteste.
    pub fn from_env() -> Option<Self> {
        // Roteiro vazio é o mesmo que sem autoteste (sessão de teste comum, por
        // exemplo para medir o consumo parado).
        let script = std::env::var("MIDE_SELFTEST").ok().filter(|s| !s.trim().is_empty())?;
        let steps = parse(&script);
        tracing::info!("autoteste com {} passo(s)", steps.len());
        Some(Self { steps, frame: 0 })
    }

    /// Executa os passos deste frame. Chamado uma vez por frame desenhado.
    pub fn tick(&mut self, state: &mut State) {
        let frame = self.frame;
        self.frame += 1;
        let due: Vec<Step> = self
            .steps
            .iter()
            .filter(|(f, _)| *f == frame)
            .map(|(_, s)| *s)
            .collect();
        for step in due {
            run(state, step, frame);
        }
    }
}

fn parse(script: &str) -> Vec<(u32, Step)> {
    script
        .split(';')
        .flat_map(|part| -> Vec<(u32, Step)> {
            let words: Vec<&str> = part.split_whitespace().collect();
            let Some(frame) = words.first().and_then(|w| w.parse::<u32>().ok()) else {
                return Vec::new();
            };
            // Teclas viram vários passos no mesmo frame.
            let keys: Option<Vec<Step>> = match words.get(1).copied() {
                Some("key") => words.get(2).and_then(|n| input::evdev_code(n)).map(|c| vec![Step::Key(c, true), Step::Key(c, false)]),
                // Segurar e soltar uma tecla (ex.: Ctrl durante um clique no app).
                Some("keydown") => words.get(2).and_then(|n| input::evdev_code(n)).map(|c| vec![Step::Key(c, true)]),
                Some("keyup") => words.get(2).and_then(|n| input::evdev_code(n)).map(|c| vec![Step::Key(c, false)]),
                Some("type") => words.get(2).map(|text| {
                    text.chars()
                        .filter_map(|ch| input::evdev_code(&ch.to_string()))
                        .flat_map(|c| [Step::Key(c, true), Step::Key(c, false)])
                        .collect()
                }),
                Some("combo") => words.get(2).and_then(|combo| {
                    let codes: Option<Vec<u32>> = combo.split('+').map(input::evdev_code).collect();
                    codes.map(|codes| {
                        let mut steps: Vec<Step> = codes.iter().map(|c| Step::Key(*c, true)).collect();
                        steps.extend(codes.iter().rev().map(|c| Step::Key(*c, false)));
                        steps
                    })
                }),
                _ => None,
            };
            if let Some(keys) = keys {
                return keys.into_iter().map(|k| (frame, k)).collect();
            }
            parse_one(part).into_iter().collect()
        })
        .collect()
}

fn parse_one(part: &str) -> Option<(u32, Step)> {
    Some(part).and_then(|part| {
            let words: Vec<&str> = part.split_whitespace().collect();
            let frame: u32 = words.first()?.parse().ok()?;
            let coord = |i: usize| words.get(i).and_then(|w| w.parse::<f64>().ok());
            let step = match *words.get(1)? {
                "move" => Step::Move(coord(2)?, coord(3)?),
                "press" => Step::Press(coord(2)?, coord(3)?),
                "release" => Step::Release(coord(2)?, coord(3)?),
                "click" => Step::Click(coord(2)?, coord(3)?),
                "rclick" => Step::RightClick(coord(2)?, coord(3)?),
                "list" => Step::List,
                "restore" => Step::Restore,
                "resize" => Step::Resize(coord(2)? as i32, coord(3)? as i32),
                "ctrl" => Step::Ctrl(words.get(2).is_some_and(|w| *w == "on")),
                "wheel" => Step::Wheel(match words.get(2).copied() {
                    Some("up") => -1,
                    Some("down") => 1,
                    other => other.and_then(|w| w.parse().ok())?,
                }),
                "mclick" => Step::MiddleClick(coord(2)?, coord(3)?),
                other => {
                    tracing::warn!("autoteste: ação desconhecida '{other}'");
                    return None;
                }
            };
            Some((frame, step))
        })
}

fn run(state: &mut State, step: Step, frame: u32) {
    let time = frame * 16;
    let at = |x: f64, y: f64| -> Point<f64, Logical> { (x, y).into() };
    match step {
        Step::Move(x, y) => input::pointer_motion(state, at(x, y), time),
        Step::Press(x, y) => {
            input::pointer_motion(state, at(x, y), time);
            input::button(state, BTN_LEFT, ButtonState::Pressed, time);
        }
        Step::Release(x, y) => {
            input::pointer_motion(state, at(x, y), time);
            input::button(state, BTN_LEFT, ButtonState::Released, time);
        }
        Step::Click(x, y) => {
            input::pointer_motion(state, at(x, y), time);
            input::button(state, BTN_LEFT, ButtonState::Pressed, time);
            input::button(state, BTN_LEFT, ButtonState::Released, time + 40);
        }
        Step::RightClick(x, y) => {
            input::pointer_motion(state, at(x, y), time);
            input::button(state, BTN_RIGHT, ButtonState::Pressed, time);
            input::button(state, BTN_RIGHT, ButtonState::Released, time + 40);
        }
        Step::List => list(state),
        Step::Restore => state.restore_minimized(),
        Step::Ctrl(on) => state.test_ctrl = on,
        Step::Key(code, pressed) => input::evdev_key(state, code, pressed, time),
        Step::Wheel(steps) => input::wheel(state, steps, time),
        Step::MiddleClick(x, y) => {
            input::pointer_motion(state, at(x, y), time);
            input::button(state, 0x112, ButtonState::Pressed, time);
            input::button(state, 0x112, ButtonState::Released, time + 40);
        }
        Step::Resize(w, h) => {
            // Simula a janela do host mudando de tamanho: novo modo da saída e o mesmo
            // tratamento do evento real.
            let output = state.primary_output();
            if let Some(output) = output {
                let mode = smithay::output::Mode {
                    size: (w, h).into(),
                    refresh: 60_000,
                };
                output.change_current_state(Some(mode), None, None, None);
                output.set_preferred(mode);
                state.output_resized();
            }
        }
    }
    tracing::info!("autoteste: frame {frame}: {step:?}");
}

/// Registra janelas e camadas com a posição no Space, a geometria e a moldura.
fn list(state: &State) {
    if state.blanked {
        tracing::info!("autoteste: tela apagada");
    }
    if state.lock.locked {
        let alive = state.lock.surfaces.iter().filter(|(_, s)| s.alive()).count();
        tracing::info!("autoteste: tela bloqueada ({alive} tela(s) de bloqueio)");
    } else if state.lock.unlocks > 0 {
        tracing::info!("autoteste: tela livre, desbloqueios: {}", state.lock.unlocks);
    }
    for window in state.space.elements() {
        let Some(loc) = state.space.element_location(window) else {
            continue;
        };
        let geo = window.geometry();
        let bar_h = bar_height_for(window);
        let title = window
            .toplevel()
            .map(|t| {
                smithay::wayland::compositor::with_states(t.wl_surface(), |s| {
                    s.data_map
                        .get::<smithay::wayland::shell::xdg::XdgToplevelSurfaceData>()
                        .and_then(|d| d.lock().ok().and_then(|a| a.title.clone()))
                })
                .unwrap_or_default()
            })
            .unwrap_or_default();
        let popups = window
            .toplevel()
            .map(|t| smithay::desktop::PopupManager::popups_for_surface(t.wl_surface()).count())
            .unwrap_or(0);
        tracing::info!(
            "autoteste: janela {title:?} popups={popups} posição=({}, {}) visível={}x{} deslocamento=({}, {}) barra={bar_h} moldura=({}, {}, {}, {})",
            loc.x,
            loc.y,
            geo.size.w,
            geo.size.h,
            geo.loc.x,
            geo.loc.y,
            loc.x,
            loc.y - bar_h,
            geo.size.w,
            geo.size.h + bar_h,
        );
    }
    // Telas e camadas de cada uma, em coordenadas globais.
    for output in state.space.outputs() {
        let Some(screen) = state.space.output_geometry(output) else {
            continue;
        };
        tracing::info!(
            "autoteste: tela {} em ({}, {}) {}x{}",
            output.name(),
            screen.loc.x,
            screen.loc.y,
            screen.size.w,
            screen.size.h
        );
        let map = layer_map_for_output(output);
        for layer in map.layers() {
            if let Some(g) = map.layer_geometry(layer) {
                tracing::info!(
                    "autoteste: camada {:?} em ({}, {}) {}x{}",
                    layer.namespace(),
                    screen.loc.x + g.loc.x,
                    screen.loc.y + g.loc.y,
                    g.size.w,
                    g.size.h
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roteiro_e_interpretado() {
        let steps = parse("300 list; 320 click 540 160 ; 400 press 1 2;bad; 500 xyz 1 2");
        assert_eq!(steps.len(), 3);
        assert_eq!(steps[0].0, 300);
        assert!(matches!(steps[1].1, Step::Click(x, y) if x == 540.0 && y == 160.0));
        assert!(matches!(steps[2].1, Step::Press(..)));
    }

    #[test]
    fn teclas_viram_passos_de_pressionar_e_soltar() {
        let steps = parse("10 type ab; 20 key enter; 30 combo ctrl+shift+space; 40 key ??");
        let keys: Vec<(u32, u32, bool)> = steps
            .iter()
            .filter_map(|(f, s)| match s {
                Step::Key(c, p) => Some((*f, *c, *p)),
                _ => None,
            })
            .collect();
        assert_eq!(&keys[..4], &[(10, 30, true), (10, 30, false), (10, 48, true), (10, 48, false)]);
        assert_eq!(&keys[4..6], &[(20, 28, true), (20, 28, false)]);
        // Combo: modificadores primeiro, soltos na ordem inversa.
        assert_eq!(&keys[6..], &[(30, 29, true), (30, 42, true), (30, 57, true), (30, 57, false), (30, 42, false), (30, 29, false)]);
    }
}
