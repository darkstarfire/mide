//! mide: Mac Inspired Desktop Environment.
//!
//! Ponto de entrada do compositor. Esta etapa abre o socket Wayland `mide-0`
//! e aceita clientes. A renderização ainda não existe.

mod activation;
mod app_menus;
mod backend_headless;
mod bar_tone;
mod backend_udev;
mod backend_winit;
mod blur;
mod clip;
mod config;
mod control;
mod cursor;
mod decorations;
mod dmabuf;
mod genie;
mod input;
mod lock;
mod osd;
mod outputs;
mod record;
mod screencopy;
mod selftest;
mod session;
mod shake;
mod snap;
mod preview;
mod presentation;
mod render;
mod state;
mod title;
mod wallpaper;
mod wm;
mod xwayland;

use std::{error::Error, sync::Arc, time::Duration};

use smithay::{
    reexports::{
        calloop::{generic::Generic, EventLoop, Interest, Mode, PostAction},
        wayland_server::Display,
    },
    wayland::socket::ListeningSocketSource,
};
use state::{ClientState, State};

/// Nome do socket Wayland: `mide-0`, ou o valor de `MIDE_SOCKET` (os testes usam
/// outro nome para não interferir numa sessão do mide aberta).
pub fn socket_name() -> &'static str {
    static NAME: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    NAME.get_or_init(|| {
        std::env::var("MIDE_SOCKET")
            .ok()
            .filter(|n| !n.is_empty() && !n.contains('/'))
            .unwrap_or_else(|| "mide-0".into())
    })
}

fn main() -> Result<(), Box<dyn Error>> {
    session::capture();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "mide=info".into()),
        )
        .init();

    tracing::info!(
        "mide {} iniciando no socket {}",
        env!("CARGO_PKG_VERSION"),
        socket_name()
    );

    let display: Display<State> = Display::new()?;
    let display_handle = display.handle();

    let mut event_loop: EventLoop<State> = EventLoop::try_new()?;
    let loop_handle = event_loop.handle();

    // Novos clientes entram pelo socket e recebem o estado `ClientState`.
    let socket = ListeningSocketSource::with_name(socket_name())?;
    loop_handle.insert_source(socket, |client_stream, _, state| {
        let result = state
            .display_handle
            .insert_client(client_stream, Arc::new(ClientState::default()));
        if let Err(err) = result {
            tracing::warn!("falha ao aceitar cliente: {err}");
        }
    })?;

    // Mensagens dos clientes já conectados são despachadas quando chegam.
    loop_handle.insert_source(
        Generic::new(display, Interest::READ, Mode::Level),
        |_, display, state| {
            // SAFETY: o display não é descartado enquanto a fonte existir.
            // Um cliente que fecha a conexão de forma abrupta não pode derrubar o
            // compositor: o erro é registrado e o loop continua.
            if let Err(err) = unsafe { display.get_mut().dispatch_clients(state) } {
                tracing::warn!("erro ao despachar clientes: {err}");
            }
            // Pedido de um cliente (commit, popup, janela nova ou fechada): um quadro.
            state.wake();
            Ok(PostAction::Continue)
        },
    )?;

    let config = config::Config::load()?;
    tracing::info!("{} atalho(s) configurado(s)", config.bindings.len());
    let mut state = State::new(display_handle, config, loop_handle.clone())?;
    // Aviso de quando cada quadro apareceu na tela (o Chrome alinha a rolagem por ele).
    presentation::init(&state.display_handle);

    // DRM: o mide direto na tela (iniciado de um console de texto). Winit: numa janela
    // dentro do desktop atual (desenvolvimento). `MIDE_BACKEND=drm|winit|headless` escolhe;
    // sem ele, DRM quando não há desktop por baixo (nem WAYLAND_DISPLAY nem DISPLAY).
    // `headless` (testes): desenha na GPU sem tela nem janela. A variável sai do
    // ambiente: os apps abertos dentro da sessão não a herdam (um mide aberto de um
    // terminal da sessão não tenta tomar a tela).
    let backend = std::env::var("MIDE_BACKEND").unwrap_or_default();
    // SAFETY: ainda não há outras threads (o laço de eventos e os apps vêm depois).
    unsafe { std::env::remove_var("MIDE_BACKEND") };
    let drm = match backend.as_str() {
        "drm" => true,
        "winit" | "headless" => false,
        _ => std::env::var_os("WAYLAND_DISPLAY").is_none() && std::env::var_os("DISPLAY").is_none(),
    };
    if backend == "headless" {
        tracing::info!("backend: sem tela (testes)");
        backend_headless::init(&loop_handle, &mut state)?;
    } else if drm {
        tracing::info!("backend: DRM (direto na tela)");
        backend_udev::init(&event_loop, &loop_handle, &mut state)?;
    } else {
        tracing::info!("backend: winit (janela no desktop atual)");
        backend_winit::init(&event_loop, &loop_handle, &mut state)?;
    }

    // Bloqueio: pelo tempo sem uso e pelos pedidos do logind, só na sessão de verdade
    // (num mide em janela, quem bloqueia é o desktop de baixo).
    state.idle_lock = drm || std::env::var_os("MIDE_LOCK_IDLE").is_some();
    if drm {
        let (sender, requests) = smithay::reexports::calloop::channel::channel::<&'static str>();
        loop_handle.insert_source(requests, |event, _, state: &mut State| {
            if let smithay::reexports::calloop::channel::Event::Msg(reason) = event {
                if reason == "acordou" {
                    tracing::info!("o computador voltou da suspensão");
                    state.resumed = true;
                    state.activity();
                    state.redraw = true;
                    if let Some(waker) = &state.waker {
                        waker.ping();
                    }
                    return;
                }
                if reason == "suspender" && !state.config.lock.on_suspend {
                    return;
                }
                lock::request(state, reason);
            }
        })?;
        lock::watch_logind(sender);
    }

    // Apps X11 dentro do mide (xwayland-satellite); encerrado junto com o mide.
    let _xwayland = xwayland::start(socket_name());

    if let Err(err) = control::init(&loop_handle, socket_name()) {
        tracing::warn!("canal de controle indisponível: {err}");
    }

    // Sessão de login: ambiente para o D-Bus/systemd e apps de inicialização automática.
    session::start(socket_name());

    // Área de trabalho (ícones) primeiro: fica atrás de tudo.
    spawn_client("mide-desktop");
    spawn_client("mide-dock");
    spawn_client("mide-bar");
    spawn_client("mide-notifications");
    if wants_portal() {
        spawn_client("mide-portal");
    }
    if wants_polkit_agent() {
        spawn_client("mide-polkit-agent");
    }

    tracing::info!("pronto. Teste com: WAYLAND_DISPLAY={} <app-wayland>", socket_name());

    let stop = event_loop.get_signal();
    event_loop.run(Duration::from_millis(16), &mut state, |state| {
        if let Err(err) = state.display_handle.flush_clients() {
            tracing::warn!("falha ao enviar eventos: {err}");
        }
        lock::tick(state);
        if state.quit_requested {
            stop.stop();
        }
    })?;

    let reexec = state.reexec_requested;
    // Encerra na ordem: programas do DE, clientes (o display some com o estado), a
    // tela e a sessão (o laço de eventos guarda o backend), e o XWayland.
    stop_shell();
    drop(state);
    drop(event_loop);
    drop(_xwayland);
    if reexec {
        reexec_self();
    }
    session::stop();
    Ok(())
}

/// Programas do DE abertos pelo mide (barra, dock...), com o pid, para reiniciar.
static SHELL: std::sync::Mutex<Vec<(String, u32)>> = std::sync::Mutex::new(Vec::new());

/// Programas do DE que ficam sempre abertos (o launcher e o buscador abrem sob demanda).
const SHELL_CLIENTS: [&str; 4] = ["mide-desktop", "mide-dock", "mide-bar", "mide-notifications"];

/// O `mide-portal` (Arquivos como `FileManager1` e o seletor de arquivos do portal, B25)
/// só roda na sessão do mide: num mide em janela (desenvolvimento, dentro do Cinnamon)
/// ele tiraria o "Mostrar na pasta" do gerenciador de arquivos de lá. Os testes ligam
/// com `MIDE_PORTAL_FORCE=1`, num barramento D-Bus próprio (`tests/scenario-dbus.sh`).
fn wants_portal() -> bool {
    session::active() || std::env::var("MIDE_PORTAL_FORCE").is_ok_and(|v| v == "1")
}

/// O agente do polkit (C7, senha de administrador dos apps) só roda na sessão do mide:
/// num mide em janela dentro de outro desktop, o agente de lá continua (e o registro
/// falharia). Os testes ligam com `MIDE_POLKIT_AGENT_FORCE=1`, sempre com um barramento
/// de teste em `MIDE_POLKIT_BUS` (nunca o polkit de verdade), e só em compilação de
/// desenvolvimento: na de distribuição o agente ignora o `MIDE_POLKIT_BUS` e se
/// registraria no polkit de verdade, por baixo do agente do outro desktop.
fn wants_polkit_agent() -> bool {
    session::active()
        || (cfg!(debug_assertions)
            && std::env::var("MIDE_POLKIT_AGENT_FORCE").is_ok_and(|v| v == "1")
            && std::env::var("MIDE_POLKIT_BUS").is_ok_and(|v| !v.is_empty()))
}

/// Encerra os programas do DE abertos pelo mide e espera eles saírem (até 2 s), para
/// os novos não brigarem com os antigos (o nome das notificações no D-Bus, por exemplo).
fn stop_shell() {
    stop_shell_only(None);
}

/// Encerra os programas do DE (só os de `only`, se dado) e espera eles saírem.
fn stop_shell_only(only: Option<&[&str]>) {
    let running: Vec<(String, u32)> = SHELL
        .lock()
        .map(|mut s| {
            let (stop, keep): (Vec<_>, Vec<_>) = std::mem::take(&mut *s).into_iter().partition(|(name, _)| only.is_none_or(|o| o.contains(&name.as_str())));
            *s = keep;
            stop
        })
        .unwrap_or_default();
    for (_, pid) in &running {
        let _ = std::process::Command::new("kill").args(["-TERM", &pid.to_string()]).status();
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while running.iter().any(|(_, pid)| std::path::Path::new(&format!("/proc/{pid}")).exists())
        && std::time::Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Reinicia barra, dock, área de trabalho e notificações com os programas atuais do
/// disco (depois de recompilar), sem mexer nas janelas abertas.
pub fn restart_shell() {
    tracing::info!("reiniciando os programas do DE");
    stop_shell();
    for name in SHELL_CLIENTS {
        spawn_client(name);
    }
    if wants_portal() {
        spawn_client("mide-portal");
    }
    if wants_polkit_agent() {
        spawn_client("mide-polkit-agent");
    }
}

/// Reinicia só os programas do DE pedidos (os que mudaram ao recompilar, pelo
/// `mide-update`), sem mexer nos outros nem nas janelas. Nomes que não são do DE
/// (apps comuns) ficam de fora: valem na próxima vez que o app abrir.
pub fn restart_shell_only(names: &[&str]) {
    let ours: Vec<&str> = names
        .iter()
        .copied()
        .filter(|n| SHELL_CLIENTS.contains(n) || (*n == "mide-portal" && wants_portal()) || (*n == "mide-polkit-agent" && wants_polkit_agent()))
        .collect();
    if ours.is_empty() {
        return;
    }
    tracing::info!("reiniciando só: {}", ours.join(", "));
    stop_shell_only(Some(&ours));
    for name in ours {
        spawn_client(name);
    }
}

/// O executável do mide no disco. Depois de recompilar, `/proc/self/exe` aponta para o
/// arquivo antigo, apagado ("... (deleted)"): o caminho certo é o mesmo, sem o sufixo.
fn exe_path() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let text = exe.to_string_lossy();
    Some(std::path::PathBuf::from(text.strip_suffix(" (deleted)").unwrap_or(&text).to_string()))
}

/// Troca este processo pelo mide novo (mesmo pid, mesmos argumentos): a sessão de login
/// continua. A sessão passa adiante sem repetir os apps automáticos.
fn reexec_self() {
    use std::os::unix::process::CommandExt;
    let Some(exe) = exe_path() else {
        tracing::warn!("reiniciar: executável do mide não encontrado");
        return;
    };
    tracing::info!("reiniciando: {}", exe.display());
    let mut command = std::process::Command::new(&exe);
    command.args(std::env::args_os().skip(1));
    if session::active() {
        command.env("MIDE_SESSION", "1").env("MIDE_AUTOSTART", session::started_list());
    }
    let err = command.exec();
    tracing::warn!("reiniciar: falhou ({err})");
}

/// Inicia um cliente do próprio DE (dock, barra) conectado a este mide. Procura o
/// programa ao lado do executável do mide, e depois no PATH.
/// Ambiente dos programas abertos pelo mide: o socket do mide, a pasta dos
/// programas do mide no PATH (para `mide-settings` e afins funcionarem pelo nome) e
/// a pasta de dados do mide em `XDG_DATA_DIRS` (os `.desktop` dele aparecem no
/// launcher e na dock). Em desenvolvimento, os dados ficam em `data/` no projeto.
pub fn child_env(command: &mut std::process::Command) {
    command.env("WAYLAND_DISPLAY", socket_name());
    // Apps X11 abrem dentro do mide (XWayland), e não no desktop por baixo.
    if let Some(display) = xwayland::display() {
        command.env("DISPLAY", display);
    }
    let Some(bin_dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.to_path_buf())) else {
        return;
    };
    let path = std::env::var("PATH").unwrap_or_default();
    command.env("PATH", format!("{}:{path}", bin_dir.display()));
    if let Some(dirs) = data_dirs() {
        command.env("XDG_DATA_DIRS", dirs);
    }
}

/// `XDG_DATA_DIRS` com a pasta de dados do mide na frente (os `.desktop` dele e a
/// configuração dos portais). Em desenvolvimento, `data/` no projeto.
pub fn data_dirs() -> Option<String> {
    let data_dir = mide_data_dir()?;
    let dirs = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    Some(format!("{}:{dirs}", data_dir.display()))
}

/// Pasta de dados do mide (`.desktop`, portais): `MIDE_DATA_DIR`, `data/` do projeto
/// (desenvolvimento) ou `share/mide` da instalação.
pub fn mide_data_dir() -> Option<std::path::PathBuf> {
    let bin_dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    std::env::var_os("MIDE_DATA_DIR")
        .map(std::path::PathBuf::from)
        .into_iter()
        .chain([bin_dir.join("../../data"), bin_dir.join("../share/mide")])
        .find(|d| d.join("applications").is_dir())?
        .canonicalize()
        .ok()
}

pub fn spawn_client(name: &str) {
    let sibling = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(name)))
        .filter(|path| path.exists());
    let program = sibling.unwrap_or_else(|| name.into());
    let mut command = std::process::Command::new(&program);
    child_env(&mut command);
    match command.spawn() {
        Ok(mut child) => {
            let pid = child.id();
            if let Ok(mut shell) = SHELL.lock() {
                shell.push((name.to_string(), pid));
            }
            std::thread::spawn(move || {
                let _ = child.wait();
                if let Ok(mut shell) = SHELL.lock() {
                    shell.retain(|(_, p)| *p != pid);
                }
            });
            tracing::info!("{name} iniciado ({})", program.display());
        }
        Err(err) => tracing::warn!(
            "{name} não iniciado ({}): {err}. Rode `cargo build` para gerar os programas",
            program.display()
        ),
    }
}
