//! Canal de controle do mide: um socket Unix ao lado do socket Wayland
//! (`$XDG_RUNTIME_DIR/mide-0.ctl`), usado pela dock e por ferramentas.
//!
//! Cada conexão envia uma linha e recebe a resposta, terminada por uma linha `end`:
//!
//! - `list`: uma linha por janela, `app_id<TAB>título<TAB>estado`, onde o estado é
//!   `focused`, `normal` ou `minimized`.
//! - `activate id1 id2 ...`: restaura ou foca a janela cujo app_id casa com algum
//!   dos ids (sem diferenciar maiúsculas). Responde `ok` ou `none`.
//! - `icons ids:x:y:w:h;...`: a dock informa onde está cada ícone (ids separados
//!   por vírgula, retângulo relativo à dock). Usado pelo efeito lâmpada.
//! - `mods`: teclas modificadoras pressionadas agora (`ctrl`, `shift`, `alt`, `logo`).
//! - `quit`: encerra o compositor (menu "Encerrar sessão" da barra).
//! - `search`: abre ou fecha o buscador.
//! - `workarea`: área útil da tela (`x y largura altura`), sem a barra e a dock.
//! - `dock_covered`: `yes` se alguma janela está onde a dock aparece.
//! - `force_quit`: termina o processo do app em foco (menu do mide).
//! - `glass <camada> raio:x,y,w,h ...`: a camada pede vidro (desfoque e vidro
//!   líquido) atrás desses retângulos, relativos a ela; sem regiões, limpa.

use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::{UnixListener, UnixStream},
    path::PathBuf,
    time::Duration,
};

use smithay::{
    desktop::Window,
    reexports::calloop::{generic::Generic, Interest, LoopHandle, Mode, PostAction},
    wayland::{compositor::with_states, shell::xdg::XdgToplevelSurfaceData},
};

use crate::state::State;

/// Caminho do socket de controle para o socket Wayland `socket_name`.
pub fn socket_path(socket_name: &str) -> Option<PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")?;
    Some(PathBuf::from(runtime).join(format!("{socket_name}.ctl")))
}

/// Abre o socket de controle e registra no event loop.
pub fn init(loop_handle: &LoopHandle<'static, State>, socket_name: &str) -> std::io::Result<()> {
    let Some(path) = socket_path(socket_name) else {
        return Ok(());
    };
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path)?;
    listener.set_nonblocking(true)?;
    loop_handle
        .insert_source(Generic::new(listener, Interest::READ, Mode::Level), |_, listener, state| {
            while let Ok((stream, _)) = listener.accept() {
                if let Err(err) = serve(stream, state) {
                    tracing::warn!("controle: falha ao responder: {err}");
                }
            }
            Ok(PostAction::Continue)
        })
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    tracing::info!("canal de controle em {}", path.display());
    Ok(())
}

fn serve(stream: UnixStream, state: &mut State) -> std::io::Result<()> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_millis(200)))?;
    let mut line = String::new();
    BufReader::new(&stream).read_line(&mut line)?;
    let reply = handle(state, line.trim());
    let mut stream = stream;
    stream.write_all(reply.as_bytes())?;
    stream.write_all(b"end\n")?;
    Ok(())
}

/// Executa um comando e devolve a resposta (sem a linha `end`).
pub fn handle(state: &mut State, command: &str) -> String {
    // Consultas (a barra, a dock e o launcher perguntam o tempo todo) não mudam a
    // tela; os outros comandos pedem um quadro.
    let query = matches!(
        command.split_whitespace().next(),
        Some("list" | "icons" | "mods" | "windows" | "bindings" | "dock_covered" | "workarea")
    ) || matches!(command.trim(), "launcher state" | "shortcuts captured");
    if !query {
        state.wake();
    }
    let mut words = command.split_whitespace();
    match words.next() {
        Some("list") => list(state),
        Some("icons") => {
            state.dock_icons = parse_icons(command.trim_start_matches("icons").trim());
            "ok\n".into()
        }
        Some("mods") => mods(state),
        Some("quit") => {
            tracing::info!("encerrando por pedido do canal de controle");
            state.quit_requested = true;
            "ok\n".into()
        }
        Some("activate") => {
            let ids: Vec<String> = words.map(|w| w.to_lowercase()).collect();
            if activate(state, &ids) { "ok\n".into() } else { "none\n".into() }
        }
        // Todas as janelas de um app: `close <ids>` pede para fechar, `hide <ids>` minimiza.
        Some(cmd @ ("close" | "hide")) => {
            let ids: Vec<String> = words.map(|w| w.to_lowercase()).collect();
            let n = app_windows(state, &ids, cmd);
            format!("{n}\n")
        }
        // Janela em foco: `window close|minimize|zoom`.
        Some("window") => {
            let ok = focused_window_action(state, words.next().unwrap_or(""));
            if ok { "ok\n".into() } else { "none\n".into() }
        }
        // Atalho para a janela em foco, como se fosse digitado: `keys ctrl+c`.
        Some("keys") => match words.next().map(|combo| crate::input::send_combo(state, combo)) {
            Some(true) => "ok\n".into(),
            _ => "erro: atalho desconhecido\n".into(),
        },
        // `launcher` abre ou fecha; `launcher state` diz se está aberto (`open` ou
        // `closed`); `launcher close` fecha se estiver aberto.
        // Prévias das janelas acima da dock: `preview <x do ícone na dock> <ids...>`,
        // ou `preview hide`.
        Some("preview") => {
            let first = words.next();
            match first.and_then(|w| w.parse::<i32>().ok()) {
                Some(dock_x) => {
                    let ids: Vec<u64> = words.filter_map(|w| w.parse().ok()).collect();
                    tracing::info!("prévias: {} janela(s)", ids.len());
                    state.preview = (!ids.is_empty()).then_some(crate::preview::Preview { ids, dock_x, hovered: None, on_close: false });
                }
                None => state.preview = None,
            }
            "ok\n".into()
        }
        // Janelas, uma por linha: `id<TAB>app_id<TAB>título<TAB>focused|normal|minimized`.
        Some("windows") => windows(state),
        // Foca a janela pelo número, restaurando se estiver minimizada.
        Some("focus") => {
            let id = words.next().and_then(|w| w.parse::<u64>().ok()).unwrap_or(0);
            if focus_window_id(state, id) { "ok\n".into() } else { "none\n".into() }
        }
        // A dock vai abrir um app: a primeira janela dele sai do ícone (efeito lâmpada).
        Some("launching") => {
            let ids: Vec<String> = words.map(|w| w.to_lowercase()).collect();
            if !ids.is_empty() {
                state.pending_open.push((ids, std::time::Instant::now()));
            }
            "ok\n".into()
        }
        // Atalhos em uso, um por linha: `combinação<TAB>ação`.
        Some("bindings") => {
            let mut out = String::new();
            for b in &state.config.bindings {
                let mods = crate::config::Modifiers { logo: b.logo, ctrl: b.ctrl, alt: b.alt, shift: b.shift };
                out.push_str(&format!("{}\t{}\n", crate::config::combo_name(mods, b.key), b.action));
            }
            out
        }
        // Gravação de atalho pelas Configurações: `shortcuts capture` faz a próxima
        // combinação apertada ser guardada (sem disparar atalhos nem chegar ao app);
        // `shortcuts captured` responde `waiting`, a combinação, `cancel` (Esc) ou
        // `none` (sem gravação ou expirou); `shortcuts stop` desiste.
        Some("shortcuts") => match words.next() {
            Some("capture") => {
                state.shortcut_capture = Some(crate::state::ShortcutCapture { started: std::time::Instant::now(), combo: None });
                tracing::info!("atalhos: gravando a próxima combinação");
                "ok\n".into()
            }
            Some("captured") => match state.shortcut_capture.as_ref().filter(|c| !c.expired()) {
                None => {
                    state.shortcut_capture = None;
                    "none\n".into()
                }
                Some(crate::state::ShortcutCapture { combo: None, .. }) => "waiting\n".into(),
                Some(crate::state::ShortcutCapture { combo: Some(combo), .. }) => {
                    let reply = format!("{combo}\n");
                    state.shortcut_capture = None;
                    reply
                }
            },
            Some("stop") => {
                state.shortcut_capture = None;
                "ok\n".into()
            }
            _ => "erro: use shortcuts capture|captured|stop\n".into(),
        },
        // Lê de novo o `config.toml` (atalhos, efeitos, papel de parede), sem reiniciar.
        Some("reload") => match crate::config::Config::load() {
            Ok(config) => {
                state.config = config;
                tracing::info!("configuração recarregada");
                "ok\n".into()
            }
            Err(err) => {
                tracing::warn!("configuração não recarregada: {err}");
                format!("erro: {err}\n")
            }
        },
        // Regiões de vidro de uma camada: `glass <camada> raio:x,y,w,h ...` (relativas
        // à camada); `glass <camada>` sem regiões limpa.
        Some("glass") => {
            let Some(namespace) = words.next() else {
                return "erro: falta a camada\n".into();
            };
            let regions: Vec<_> = words.filter_map(parse_glass).collect();
            if regions.is_empty() {
                state.glass.remove(namespace);
            } else {
                state.glass.insert(namespace.to_string(), regions);
            }
            "ok\n".into()
        }
        // Forçar encerrar o app em foco (menu do mide).
        Some("force_quit") => {
            if focused_window_action(state, "kill") { "ok\n".into() } else { "none\n".into() }
        }
        // Há janela sobre a área da dock? (`yes`/`no`; a dock que se esconde sozinha
        // só some nesse caso.)
        Some("dock_covered") => if state.dock_covered() { "yes\n".into() } else { "no\n".into() },
        // Área útil (sem a barra e a dock reservada): `x y largura altura`.
        Some("workarea") => match state.usable_area() {
            Some(r) => format!("{} {} {} {}\n", r.loc.x, r.loc.y, r.size.w, r.size.h),
            None => "none\n".into(),
        },
        // Abre ou fecha o buscador.
        Some("search") => {
            state.toggle_search();
            "ok\n".into()
        }
        Some("launcher") => match words.next() {
            Some("state") => if state.launcher_open() { "open\n".into() } else { "closed\n".into() },
            Some("close") => {
                if state.launcher_open() {
                    state.toggle_launcher();
                }
                "ok\n".into()
            }
            _ => {
                state.toggle_launcher();
                "ok\n".into()
            }
        },
        _ => "erro: comando desconhecido\n".into(),
    }
}

/// Uma região de vidro no formato `raio:x,y,w,h`.
fn parse_glass(word: &str) -> Option<(smithay::utils::Rectangle<i32, smithay::utils::Logical>, f32)> {
    let (radius, rect) = word.split_once(':')?;
    let n: Vec<i32> = rect.split(',').map(|v| v.parse::<f32>().map(|f| f.round() as i32)).collect::<Result<_, _>>().ok()?;
    let [x, y, w, h] = n[..] else {
        return None;
    };
    (w > 0 && h > 0).then(|| (smithay::utils::Rectangle::new((x, y).into(), (w, h).into()), radius.parse().unwrap_or(0.0)))
}

/// app_id e título que o app definiu para a janela.
fn identity(window: &Window) -> (String, String) {
    window
        .toplevel()
        .map(|t| {
            with_states(t.wl_surface(), |s| {
                s.data_map
                    .get::<XdgToplevelSurfaceData>()
                    .and_then(|d| d.lock().ok().map(|a| (a.app_id.clone().unwrap_or_default(), a.title.clone().unwrap_or_default())))
                    .unwrap_or_default()
            })
        })
        .unwrap_or_default()
}

fn list(state: &State) -> String {
    let focus = state.seat.get_keyboard().and_then(|k| k.current_focus());
    let mut out = String::new();
    for window in state.space.elements() {
        let (app_id, title) = identity(window);
        let focused = window.toplevel().map(|t| t.wl_surface().clone()) == focus;
        let status = if focused { "focused" } else { "normal" };
        out.push_str(&format!("{app_id}\t{title}\t{status}\n"));
    }
    for minimized in &state.minimized {
        let (app_id, title) = identity(&minimized.window);
        out.push_str(&format!("{app_id}\t{title}\tminimized\n"));
    }
    out
}

/// Restaura a janela minimizada do app ou, se não houver, foca a janela aberta.
fn activate(state: &mut State, ids: &[String]) -> bool {
    let matches = |w: &Window| {
        let (app_id, _) = identity(w);
        !app_id.is_empty() && ids.iter().any(|id| id == &app_id.to_lowercase())
    };

    if let Some(index) = state.minimized.iter().rposition(|m| matches(&m.window)) {
        let entry = state.minimized.remove(index);
        state.minimized.push(entry);
        state.restore_minimized();
        return true;
    }

    let found = state.space.elements().rev().find(|w| matches(w)).cloned();
    if let Some(window) = found {
        state.space.raise_element(&window, true);
        let serial = smithay::utils::SERIAL_COUNTER.next_serial();
        crate::input::focus_window(state, &window, serial);
        return true;
    }
    false
}

fn windows(state: &State) -> String {
    let focus = state.seat.get_keyboard().and_then(|k| k.current_focus());
    let mut out = String::new();
    let mapped = state.space.elements().filter(|w| !state.minimized.iter().any(|m| &m.window == *w));
    for window in mapped {
        let (app_id, title) = identity(window);
        let focused = window.toplevel().map(|t| t.wl_surface().clone()) == focus;
        let status = if focused { "focused" } else { "normal" };
        out.push_str(&format!("{}\t{app_id}\t{title}\t{status}\n", crate::state::window_id(window)));
    }
    for m in &state.minimized {
        let (app_id, title) = identity(&m.window);
        out.push_str(&format!("{}\t{app_id}\t{title}\tminimized\n", crate::state::window_id(&m.window)));
    }
    out
}

fn focus_window_id(state: &mut State, id: u64) -> bool {
    if let Some(index) = state.minimized.iter().position(|m| crate::state::window_id(&m.window) == id) {
        let entry = state.minimized.remove(index);
        state.minimized.push(entry);
        state.restore_minimized();
        return true;
    }
    let found = state.space.elements().find(|w| crate::state::window_id(w) == id).cloned();
    let Some(window) = found else {
        return false;
    };
    state.space.raise_element(&window, true);
    let serial = smithay::utils::SERIAL_COUNTER.next_serial();
    crate::input::focus_window(state, &window, serial);
    tracing::info!("controle: janela {id} focada");
    true
}

/// Fecha (`close`) ou minimiza (`hide`) todas as janelas do app. Devolve quantas.
fn app_windows(state: &mut State, ids: &[String], cmd: &str) -> usize {
    let windows: Vec<Window> = state
        .space
        .elements()
        .filter(|w| {
            let (app_id, _) = identity(w);
            !app_id.is_empty() && ids.iter().any(|id| id == &app_id.to_lowercase())
        })
        .cloned()
        .collect();
    for window in &windows {
        let Some(toplevel) = window.toplevel().cloned() else {
            continue;
        };
        if cmd == "close" {
            state.request_close(window);
        } else if !state.minimized.iter().any(|m| &m.window == window) {
            state.minimize(toplevel.wl_surface());
        }
    }
    tracing::info!("controle: {cmd} em {} janela(s)", windows.len());
    windows.len()
}

/// Ação na janela com o foco do teclado.
fn focused_window_action(state: &mut State, action: &str) -> bool {
    let Some(focus) = state.seat.get_keyboard().and_then(|k| k.current_focus()) else {
        return false;
    };
    let Some(window) = state.window_for(&focus) else {
        return false;
    };
    let Some(toplevel) = window.toplevel().cloned() else {
        return false;
    };
    match action {
        "close" => state.request_close(&window),
        "minimize" => state.minimize(toplevel.wl_surface()),
        "zoom" => crate::input::toggle_maximize(state, toplevel),
        // Forçar encerrar: termina o processo do app (quando ele não responde).
        "kill" => {
            use smithay::reexports::wayland_server::Resource;
            let pid = toplevel
                .wl_surface()
                .client()
                .and_then(|c| c.get_credentials(&state.display_handle).ok())
                .map(|c| c.pid);
            match pid {
                Some(pid) if pid > 1 && pid as u32 != std::process::id() => {
                    tracing::info!("controle: forçar encerrar o processo {pid}");
                    let _ = std::process::Command::new("kill").args(["-KILL", &pid.to_string()]).status();
                }
                _ => return false,
            }
        }
        _ => return false,
    }
    tracing::info!("controle: janela em foco: {action}");
    true
}

/// Interpreta `ids:x:y:w:h;ids:x:y:w:h`, com ids separados por vírgula.
pub fn parse_icons(text: &str) -> Vec<(Vec<String>, smithay::utils::Rectangle<i32, smithay::utils::Logical>)> {
    text.split(';')
        .filter_map(|part| {
            let fields: Vec<&str> = part.trim().split(':').collect();
            if fields.len() != 5 {
                return None;
            }
            let ids = fields[0]
                .split(',')
                .filter(|s| !s.is_empty())
                .map(|s| s.to_lowercase())
                .collect();
            let n = |i: usize| fields[i].parse::<i32>().ok();
            let rect = smithay::utils::Rectangle::new((n(1)?, n(2)?).into(), (n(3)?, n(4)?).into());
            Some((ids, rect))
        })
        .collect()
}

fn mods(state: &State) -> String {
    let Some(keyboard) = state.seat.get_keyboard() else {
        return "\n".into();
    };
    let m = keyboard.modifier_state();
    let mut held = Vec::new();
    if m.ctrl || state.test_ctrl {
        held.push("ctrl");
    }
    if m.shift {
        held.push("shift");
    }
    if m.alt {
        held.push("alt");
    }
    if m.logo {
        held.push("logo");
    }
    format!("{}\n", held.join(" "))
}

#[cfg(test)]
mod tests {
    use super::{parse_glass, parse_icons};

    #[test]
    fn regioes_de_vidro_sao_interpretadas() {
        let (rect, radius) = parse_glass("10:4,30.6,200,120").unwrap();
        assert_eq!((rect.loc.x, rect.loc.y, rect.size.w, rect.size.h), (4, 31, 200, 120));
        assert_eq!(radius, 10.0);
        assert!(parse_glass("10:4,30,0,120").is_none(), "sem área");
        assert!(parse_glass("lixo").is_none());
        assert!(parse_glass("3:1,2,3").is_none());
    }

    #[test]
    fn icones_da_dock_sao_interpretados() {
        let icons = parse_icons("org.gnome.Calculator,calculator:70:10:48:48;foot:10:10:48:48;ruim");
        assert_eq!(icons.len(), 2);
        assert_eq!(icons[0].0, vec!["org.gnome.calculator".to_string(), "calculator".to_string()]);
        assert_eq!(icons[0].1.loc.x, 70);
        assert_eq!(icons[1].1.size.w, 48);
    }
}
