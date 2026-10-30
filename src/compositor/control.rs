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
//! - `game_mode`: `on` se a tela principal está no modo de jogo em tela cheia (A18).
//! - `glass <camada> raio[@opacidade]:x,y,w,h ...`: a camada pede vidro (desfoque e vidro
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
            crate::watchdog::phase("canal de controle");
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
    // Quem pediu e não lê a resposta (programa travado ou congelado) não pode prender o
    // laço do compositor numa escrita sem fim.
    stream.set_write_timeout(Some(Duration::from_millis(500)))?;
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
        Some("list" | "icons" | "mods" | "windows" | "bindings" | "dock_covered" | "workarea" | "features" | "window-outputs" | "game_mode")
    ) || matches!(command.trim(), "launcher state" | "shortcuts captured");
    if !query {
        state.wake();
    }
    let mut words = command.split_whitespace();
    match words.next() {
        Some("list") => list(state),
        // Recursos que este compositor tem, uma palavra cada (o mide-ui pergunta uma vez):
        // `child_glass` = vidro por janela filha, pelo nome `app:<app_id>><título>`.
        Some("features") => "child_glass\n".into(),
        // Modo de jogo em tela cheia (A18) na tela principal (a da barra, da dock e dos
        // banners): `on` ou `off`. A dock pausa as tarefas periódicas e as notificações
        // vão só para a central enquanto for `on`.
        Some("game_mode") => if state.game_on_primary() { "on\n".into() } else { "off\n".into() },
        // Onde está o menu da janela em foco (apps GTK e Qt no Wayland): `gtk<TAB>bus
        // <TAB>barra<TAB>app<TAB>janela`, `dbusmenu<TAB>serviço<TAB>objeto` ou vazio.
        Some("appmenu") => {
            let focus = state.seat.get_keyboard().and_then(|k| k.current_focus());
            match focus.and_then(|s| crate::app_menus::menu_of(&s)) {
                Some(crate::app_menus::AppMenu::Gtk { bus, menubar, app, window }) => format!("gtk\t{bus}\t{menubar}\t{app}\t{window}\n"),
                Some(crate::app_menus::AppMenu::DbusMenu { service, path }) => format!("dbusmenu\t{service}\t{path}\n"),
                None => "\n".into(),
            }
        }
        // Layout de teclado ativo (`layout`: o número, a partir de 0) e trocar
        // (`layout 1`), para o seletor da barra.
        Some("layout") => match state.seat.get_keyboard() {
            Some(keyboard) => {
                if let Some(n) = words.next().and_then(|w| w.parse::<u32>().ok()) {
                    keyboard.with_xkb_state(state, |mut ctx| {
                        let count = ctx.xkb().lock().unwrap().layouts().count() as u32;
                        if n < count {
                            ctx.set_layout(smithay::input::keyboard::Layout(n));
                        }
                    });
                    tracing::info!("teclado: layout {n} pela barra");
                }
                let active = keyboard.with_xkb_state(state, |ctx| ctx.xkb().lock().unwrap().active_layout().0);
                format!("{active}\n")
            }
            None => "\n".into(),
        },
        // Gravar a tela: `record start [full|x y w h] [sistema] [microfone]`,
        // `record stop` e `record status` (`off`, ou `on <segundos> <arquivo>`).
        Some("record") => match words.next() {
            Some("start") => {
                let rest: Vec<&str> = words.collect();
                match crate::record::start(state, crate::record::parse_request(&rest)) {
                    Ok(path) => format!("ok {}\n", path.display()),
                    Err(err) => {
                        tracing::warn!("gravação não começou: {err}");
                        format!("erro: {err}\n")
                    }
                }
            }
            Some("stop") => match crate::record::stop(state) {
                Some(path) => format!("ok {}\n", path.display()),
                None => "off\n".into(),
            },
            _ => match &state.recording {
                Some(r) => format!("on {} {}\n", r.started.elapsed().as_secs(), r.path.display()),
                None => "off\n".into(),
            },
        },
        // A barra pergunta se há algo para ela (a confirmação do botão de ligar).
        Some("bar_request") => format!("{}\n", state.bar_request.take().unwrap_or("")),
        Some("icons") => {
            state.dock_icons = parse_icons(command.trim_start_matches("icons").trim());
            "ok\n".into()
        }
        Some("mods") => mods(state),
        // Atualizar sem sair da sessão (depois de recompilar): `restart shell` reinicia
        // barra, dock, área de trabalho, notificações e launcher com os programas novos,
        // sem fechar as janelas; `restart all` reinicia também o compositor (as janelas
        // dos apps fecham, a sessão de login continua).
        Some("restart") => match words.next() {
            // `restart shell mide-bar mide-dock`: só esses (o `mide-update` passa os que
            // mudaram ao recompilar); sem nomes, todos.
            Some("shell") => {
                let names: Vec<&str> = words.collect();
                if names.is_empty() {
                    crate::restart_shell();
                } else {
                    crate::restart_shell_only(&names);
                }
                "ok\n".into()
            }
            Some("all") => {
                tracing::info!("reiniciando o compositor com o programa novo");
                state.reexec_requested = true;
                state.quit_requested = true;
                "ok\n".into()
            }
            _ => "uso: restart shell [programas...]|all\n".into(),
        },
        // `copy <texto>`: o compositor guarda o texto na área de transferência.
        Some("copy") => {
            let text = command.trim_start().strip_prefix("copy").map(|t| t.strip_prefix(' ').unwrap_or(t)).unwrap_or("");
            state.copy_text(text);
            "ok\n".into()
        }
        Some("files-copy") => {
            match serde_json::from_str::<mide::storage::transfer::Clipboard>(command.trim_start().strip_prefix("files-copy ").unwrap_or("")) {
                Ok(clipboard) => {
                    state.copy_files(clipboard);
                    "ok\n".into()
                }
                Err(err) => format!("erro: {err}\n"),
            }
        }
        Some("files-paste") => format!("{}\n", serde_json::to_string(&state.file_clipboard).unwrap_or_default()),
        Some("files-pasted") => {
            if let Ok(done) = serde_json::from_str::<mide::storage::transfer::Clipboard>(command.trim_start().strip_prefix("files-pasted ").unwrap_or("")) {
                if let Some(clipboard) = state.file_clipboard.as_mut() {
                    clipboard.complete(done.id, &done.paths);
                    let clipboard = clipboard.clone();
                    state.offer_files(&clipboard);
                }
            }
            "ok\n".into()
        }
        // Telas: `outputs` lista (uma por linha, para as Configurações); `outputs apply`
        // relê a configuração e refaz as telas (resolução, posição, principal...).
        Some("outputs") => match words.next() {
            Some("apply") => {
                if let Ok(config) = crate::config::Config::load() {
                    state.config = config;
                }
                match &state.outputs_waker {
                    Some(waker) => {
                        waker.ping();
                        "ok\n".into()
                    }
                    None => "este backend não muda as telas\n".into(),
                }
            }
            _ => state.outputs_info.iter().map(|o| o.line() + "\n").collect(),
        },
        // `copy-file <tipo> <arquivo>`: o compositor guarda o conteúdo do arquivo (a
        // imagem da captura de tela) na área de transferência.
        Some("copy-file") => {
            let mime = words.next().unwrap_or("application/octet-stream").to_string();
            let path: String = words.collect::<Vec<_>>().join(" ");
            match std::fs::read(&path) {
                Ok(bytes) => {
                    state.copy_bytes(&mime, bytes);
                    "ok\n".into()
                }
                Err(err) => format!("erro: {err}\n"),
            }
        }
        // Captura de tela: o próximo quadro, sem o cursor, salvo em PNG nos pixels da
        // tela. O arquivo aparece pronto (gravado com outro nome e renomeado).
        // - `screenshot <arquivo.png>`: a tela principal;
        // - `screenshot tela NOME <arquivo.png>`: uma tela;
        // - `screenshot all <prefixo>`: todas, em `<prefixo>-NOME.png`, com uma linha
        //   por tela na resposta (`screen_line`). Espelhando, só a principal.
        Some("screenshot") => {
            let args: Vec<&str> = words.collect();
            let primary = state.primary_output();
            match parse_screenshot(&args) {
                Err(usage) => usage,
                Ok(ScreenshotCmd::Primary(path)) => match primary {
                    Some(output) => {
                        state.screenshot_requests.push((output.name(), path));
                        "ok\n".into()
                    }
                    None => "erro: nenhuma tela\n".into(),
                },
                Ok(ScreenshotCmd::One(name, path)) => {
                    if state.space.outputs().any(|o| o.name() == name) {
                        state.screenshot_requests.push((name, path));
                        "ok\n".into()
                    } else {
                        "erro: tela não encontrada\n".into()
                    }
                }
                Ok(ScreenshotCmd::All(prefix)) => {
                    let mirror = state.config.monitors.mirror;
                    let outputs: Vec<_> = state.space.outputs().filter(|o| !mirror || primary.as_ref() == Some(*o)).cloned().collect();
                    let mut out = String::new();
                    for output in outputs {
                        let Some(geo) = state.space.output_geometry(&output) else {
                            continue;
                        };
                        let name = output.name();
                        let path = PathBuf::from(format!("{}-{name}.png", prefix.display()));
                        let scale = output.current_scale().fractional_scale();
                        out.push_str(&screen_line(&name, (geo.loc.x, geo.loc.y, geo.size.w, geo.size.h), scale, primary.as_ref() == Some(&output), &path));
                        state.screenshot_requests.push((name, path));
                    }
                    if out.is_empty() { "erro: nenhuma tela\n".into() } else { out }
                }
            }
        }
        // Janelas à mostra com a moldura (incluindo a barra), a de cima primeiro, em
        // pontos no espaço do desktop (todas as telas):
        // `id<TAB>x<TAB>y<TAB>largura<TAB>altura<TAB>título` (captura de tela → Janela).
        Some("window_rects") => {
            let mut out = String::new();
            for window in state.space.elements().rev() {
                let Some(loc) = state.space.element_location(window) else {
                    continue;
                };
                let size = window.geometry().size;
                let bar = crate::state::bar_height_for(window);
                out.push_str(&format!(
                    "{}\t{}\t{}\t{}\t{}\t{}\n",
                    crate::state::window_id(window),
                    loc.x,
                    loc.y - bar,
                    size.w,
                    size.h + bar,
                    crate::render::window_title(window)
                ));
            }
            out
        }
        // Mesa: `reveal` (clique no papel de parede: afasta ou traz as janelas de volta,
        // se a opção estiver ligada), `reveal on|off`, `reveal state`.
        Some("reveal") => match words.next() {
            Some("state") => if state.desktop_revealed() { "on\n".into() } else { "off\n".into() },
            Some("on") => {
                state.reveal_desktop();
                "ok\n".into()
            }
            Some("off") => {
                state.restore_revealed_and_focus();
                "ok\n".into()
            }
            _ if state.config.desktop.click_reveals() || state.desktop_revealed() => {
                state.toggle_reveal_desktop();
                "ok\n".into()
            }
            _ => "none\n".into(),
        },
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
        // A janela em foco está em tela cheia? (o menu Janela da barra: "Entrar/Sair da
        // tela cheia").
        Some("fullscreen") => {
            let focus = state.seat.get_keyboard().and_then(|k| k.current_focus());
            let on = focus.and_then(|f| state.window_for(&f)).is_some_and(|w| state.fullscreen_entry(&w).is_some());
            if on { "on\n".into() } else { "off\n".into() }
        }
        // Janela em foco: `window close|minimize|zoom|fullscreen`.
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
                    state.preview = (!ids.is_empty()).then_some(crate::preview::Preview {
                        ids,
                        dock_x,
                        hovered: None,
                        on_close: false,
                        switcher: false,
                        since: state.config.effects.transitions.then(std::time::Instant::now),
                        hover_since: None,
                    });
                }
                None => state.preview = None,
            }
            "ok\n".into()
        }
        // Janelas, uma por linha: `id<TAB>app_id<TAB>título<TAB>focused|normal|minimized`.
        Some("windows") => windows(state),
        // Telas, uma por linha: `nome<TAB>descrição<TAB>here` (a da janela em foco) ou
        // `other`, para o menu Janela → Mover para.
        Some("window-outputs") => outputs_for_menu(state),
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
                // Layout do teclado trocado nas Configurações: vale na hora.
                let kb = config.keyboard.effective();
                if kb != state.config.keyboard.effective() {
                    let xkb = smithay::input::keyboard::XkbConfig { rules: "", model: &kb.model, layout: &kb.layout, variant: &kb.variant, options: kb.options.clone() };
                    if let Some(keyboard) = state.seat.get_keyboard() {
                        match keyboard.set_xkb_config(state, xkb) {
                            Ok(()) => tracing::info!("teclado: layout {} {}", kb.layout, kb.variant),
                            Err(err) => tracing::warn!("teclado: layout {} não carregou: {err:?}", kb.layout),
                        }
                    }
                }
                // Repetição de tecla segurada: os apps recebem a nova espera e taxa.
                let repeat = config.keyboard.repeat();
                if repeat != state.config.keyboard.repeat()
                    && let Some(keyboard) = state.seat.get_keyboard()
                {
                    keyboard.change_repeat_info(repeat.1, repeat.0);
                    tracing::info!("teclado: repetição depois de {} ms, {} por segundo", repeat.0, repeat.1);
                }
                state.config = config;
                // Menus dos apps na barra superior ligados ou desligados: os apps GTK abertos
                // ficam sabendo (mostram ou escondem a barra de menus deles).
                crate::app_menus::refresh_capabilities();
                // Mouse e trackpad (Configurações → Mouse e trackpad) valem na hora.
                if let Some(waker) = &state.input_waker {
                    waker.ping();
                }
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
        // Aparência das janelas de um app: `window_style app:<app_id> radius=N` (cantos).
        Some("window_style") => {
            let Some(app) = words.next().and_then(|w| w.strip_prefix("app:")) else {
                return "erro: falta app:<app_id>\n".into();
            };
            for word in words {
                if let Some(r) = word.strip_prefix("radius=").and_then(|r| r.parse::<i32>().ok()) {
                    state.window_radius.insert(app.to_string(), r.clamp(0, 40));
                }
            }
            state.redraw = true;
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
        // Área útil da tela principal, relativa a ela (a área de trabalho usa para pôr os
        // ícones dentro da própria camada).
        Some("workarea") => {
            let screen = state.primary_output().and_then(|o| state.space.output_geometry(&o)).map(|g| g.loc).unwrap_or_default();
            match state.usable_area() {
                Some(r) => format!("{} {} {} {}\n", r.loc.x - screen.x, r.loc.y - screen.y, r.size.w, r.size.h),
                None => "none\n".into(),
            }
        }
        // Abre ou fecha o buscador.
        Some("search") => {
            state.toggle_search();
            "ok\n".into()
        }
        Some("launcher") => match words.next() {
            // Segunda linha: o tom atrás da barra superior (`tone dark|light`), na tela
            // principal (com telas espelhadas, a barra aparece em todas). Quem lê só a
            // primeira linha (a captura) não muda; a barra pergunta isto a cada 50 ms.
            Some("state") => {
                let open = if state.launcher_open() { "open" } else { "closed" };
                let primary = state.primary_output().map(|o| o.name());
                let tone = primary.and_then(|name| state.bar_tone.get(&name).copied()).or_else(|| state.bar_tone.values().next().copied());
                let mut reply = match tone {
                    Some(dark) => format!("{open}\ntone {}\n", if dark { "dark" } else { "light" }),
                    None => format!("{open}\n"),
                };
                // Modo de jogo (A18) com a barra escondida: ela pausa relógio, bateria,
                // rede e mídia e pergunta menos vezes, até a tela cheia acabar ou o
                // ponteiro revelar a barra.
                if state.bar_paused() {
                    reply.push_str("game paused\n");
                }
                // Volume e brilho das teclas, para a barra acompanhar na hora.
                reply.push_str(&crate::osd::levels_for_bar());
                reply
            }
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
fn parse_glass(word: &str) -> Option<(smithay::utils::Rectangle<i32, smithay::utils::Logical>, f32, f32)> {
    let (look, rect) = word.split_once(':')?;
    // `raio@opacidade` (a opacidade é opcional: 1).
    let (radius, alpha) = look.split_once('@').unwrap_or((look, "1"));
    let alpha = alpha.parse::<f32>().unwrap_or(1.0).clamp(0.0, 1.0);
    let n: Vec<i32> = rect.split(',').map(|v| v.parse::<f32>().map(|f| f.round() as i32)).collect::<Result<_, _>>().ok()?;
    let [x, y, w, h] = n[..] else {
        return None;
    };
    (w > 0 && h > 0 && alpha > 0.0).then(|| (smithay::utils::Rectangle::new((x, y).into(), (w, h).into()), radius.parse().unwrap_or(0.0), alpha))
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
        // O processo da janela (a barra acha os menus registrados por ele).
        let pid = {
            use smithay::reexports::wayland_server::Resource;
            window.toplevel().and_then(|t| t.wl_surface().client()).and_then(|c| c.get_credentials(&state.display_handle).ok()).map_or(0, |c| c.pid)
        };
        out.push_str(&format!("{app_id}\t{title}\t{status}\t{pid}\n"));
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

fn outputs_for_menu(state: &State) -> String {
    let focus = state.seat.get_keyboard().and_then(|k| k.current_focus());
    let here = focus.as_ref().and_then(|f| state.window_for(f)).and_then(|w| {
        let loc = state.space.element_location(&w)?;
        let c = loc + smithay::utils::Point::from((w.geometry().size.w / 2, w.geometry().size.h / 2));
        state.space.output_under(c.to_f64()).next().map(|o| o.name())
    });
    let mut out = String::new();
    for o in state.space.outputs() {
        // Como no Mac ("Mover para Tela integrada"): a tela do notebook pelo tipo, as
        // outras pelo modelo (o nome do conector se não houver).
        let name = o.name();
        let model = o.physical_properties().model;
        let desc = if ["eDP", "LVDS", "DSI"].iter().any(|p| name.starts_with(p)) {
            mide::tr!("Built-in Display").into_owned()
        } else if !model.trim().is_empty() && !model.eq_ignore_ascii_case("unknown") {
            model.trim().to_string()
        } else {
            name.clone()
        };
        let mark = if Some(o.name()) == here { "here" } else { "other" };
        out.push_str(&format!("{}\t{}\t{mark}\n", o.name(), desc));
    }
    out
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

pub(crate) fn focus_window_id(state: &mut State, id: u64) -> bool {
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
    // A janela trazida para a frente fica sob o ponteiro parado: ele passa a ela.
    crate::input::refresh_pointer_focus(state);
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
        // Tela cheia (menu Janela → Entrar/Sair da tela cheia).
        "fullscreen" => state.toggle_fullscreen(&toplevel),
        // Menu Janela (como o do Mac): preencher a tela, centralizar, metades e quartos,
        // voltar ao tamanho anterior, outra tela, e todas as janelas do app.
        "fill" => {
            if let Some(area) = state.usable_area_for_surface(toplevel.wl_surface()) {
                state.maximize_in(&toplevel, area);
            }
        }
        "center" => state.center_window(&toplevel),
        "restore" => state.unmaximize(&toplevel),
        "left" => state.place_part(&toplevel, (0.0, 0.0, 0.5, 1.0)),
        "right" => state.place_part(&toplevel, (0.5, 0.0, 0.5, 1.0)),
        "top" => state.place_part(&toplevel, (0.0, 0.0, 1.0, 0.5)),
        "bottom" => state.place_part(&toplevel, (0.0, 0.5, 1.0, 0.5)),
        "top-left" => state.place_part(&toplevel, (0.0, 0.0, 0.5, 0.5)),
        "top-right" => state.place_part(&toplevel, (0.5, 0.0, 0.5, 0.5)),
        "bottom-left" => state.place_part(&toplevel, (0.0, 0.5, 0.5, 0.5)),
        "bottom-right" => state.place_part(&toplevel, (0.5, 0.5, 0.5, 0.5)),
        "minimize-all" | "front" => {
            let app = identity(&window).0;
            let same: Vec<smithay::desktop::Window> = state.space.elements().filter(|w| identity(w).0 == app).cloned().collect();
            for w in same {
                if let Some(t) = w.toplevel().cloned() {
                    if action == "minimize-all" {
                        state.minimize(t.wl_surface());
                    } else if w != window {
                        state.space.raise_element(&w, false);
                    }
                }
            }
            if action == "front" {
                // A janela em foco fica na frente das outras do app.
                state.space.raise_element(&window, true);
            }
        }
        other if other.starts_with("output=") => {
            if !state.move_to_output(&toplevel, &other["output=".len()..]) {
                return false;
            }
        }
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

/// Pedido de captura de tela (`screenshot ...`).
#[derive(Debug, PartialEq)]
pub enum ScreenshotCmd {
    /// `screenshot <arquivo>`: a tela principal.
    Primary(PathBuf),
    /// `screenshot tela NOME <arquivo>`: uma tela, pelo nome.
    One(String, PathBuf),
    /// `screenshot all <prefixo>`: todas as telas, cada uma em `<prefixo>-NOME.png`.
    All(PathBuf),
}

/// Lê os argumentos do `screenshot` (sem a palavra do comando). Caminhos podem ter
/// espaços: o resto da linha é o caminho. O erro é a resposta de uso.
pub fn parse_screenshot(args: &[&str]) -> Result<ScreenshotCmd, String> {
    const USAGE: &str = "uso: screenshot <arquivo.png> | screenshot tela NOME <arquivo.png> | screenshot all <prefixo>\n";
    let rest = |from: usize| args.get(from..).map(|a| a.join(" ")).filter(|p| !p.is_empty());
    match args.first().copied() {
        None => Err(USAGE.into()),
        Some("tela") => match (args.get(1), rest(2)) {
            (Some(name), Some(path)) => Ok(ScreenshotCmd::One(name.to_string(), path.into())),
            _ => Err(USAGE.into()),
        },
        Some("all") => rest(1).map(|p| ScreenshotCmd::All(p.into())).ok_or_else(|| USAGE.into()),
        Some(_) => Ok(ScreenshotCmd::Primary(args.join(" ").into())),
    }
}

/// Linha da resposta do `screenshot all`, uma por tela:
/// `NOME<TAB>X<TAB>Y<TAB>LARGURA<TAB>ALTURA<TAB>ESCALA<TAB>principal|-<TAB>CAMINHO`, com a
/// posição e o tamanho em pontos, no espaço do desktop (a imagem sai em pixels: o
/// tamanho vezes a escala).
pub fn screen_line(name: &str, (x, y, w, h): (i32, i32, i32, i32), scale: f64, primary: bool, path: &std::path::Path) -> String {
    format!("{name}\t{x}\t{y}\t{w}\t{h}\t{scale}\t{}\t{}\n", if primary { "principal" } else { "-" }, path.display())
}

#[cfg(test)]
mod tests {
    use super::{parse_glass, parse_icons, parse_screenshot, screen_line, ScreenshotCmd};

    #[test]
    fn regioes_de_vidro_sao_interpretadas() {
        let (rect, radius, alpha) = parse_glass("10:4,30.6,200,120").unwrap();
        assert_eq!((rect.loc.x, rect.loc.y, rect.size.w, rect.size.h), (4, 31, 200, 120));
        assert_eq!((radius, alpha), (10.0, 1.0));
        // Com opacidade (vidro esmaecendo junto com o desenho).
        let (_, radius, alpha) = parse_glass("28@0.5:0,0,10,10").unwrap();
        assert_eq!((radius, alpha), (28.0, 0.5));
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

    #[test]
    fn pedidos_de_captura_sao_interpretados() {
        assert_eq!(parse_screenshot(&["a.png"]), Ok(ScreenshotCmd::Primary("a.png".into())));
        // Caminho com espaços (o resto da linha).
        assert_eq!(parse_screenshot(&["/tmp/minha", "tela.png"]), Ok(ScreenshotCmd::Primary("/tmp/minha tela.png".into())));
        assert_eq!(parse_screenshot(&["tela", "HEADLESS-2", "a.png"]), Ok(ScreenshotCmd::One("HEADLESS-2".into(), "a.png".into())));
        assert_eq!(parse_screenshot(&["all", "/tmp/x"]), Ok(ScreenshotCmd::All("/tmp/x".into())));
        assert!(parse_screenshot(&[]).unwrap_err().starts_with("uso:"));
        assert!(parse_screenshot(&["tela", "HEADLESS-2"]).is_err(), "sem arquivo");
        assert!(parse_screenshot(&["all"]).is_err(), "sem prefixo");
    }

    #[test]
    fn linha_de_cada_tela_na_captura() {
        let line = screen_line("HEADLESS-2", (1366, 0, 1536, 864), 1.25, false, std::path::Path::new("/tmp/t-HEADLESS-2.png"));
        assert_eq!(line, "HEADLESS-2\t1366\t0\t1536\t864\t1.25\t-\t/tmp/t-HEADLESS-2.png\n");
        let line = screen_line("HEADLESS-1", (0, 0, 1366, 768), 1.0, true, std::path::Path::new("/tmp/t-HEADLESS-1.png"));
        assert_eq!(line, "HEADLESS-1\t0\t0\t1366\t768\t1\tprincipal\t/tmp/t-HEADLESS-1.png\n");
    }
}
