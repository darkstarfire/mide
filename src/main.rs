//! mide: Mac Inspired Desktop Environment.
//!
//! Ponto de entrada do compositor. Esta etapa abre o socket Wayland `mide-0`
//! e aceita clientes. A renderização ainda não existe.

mod backend_udev;
mod backend_winit;
mod blur;
mod clip;
mod config;
mod control;
mod cursor;
mod decorations;
mod genie;
mod input;
mod selftest;
mod session;
mod preview;
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
    let mut state = State::new(display_handle, config)?;

    // DRM: o mide direto na tela (iniciado de um console de texto). Winit: numa janela
    // dentro do desktop atual (desenvolvimento). `MIDE_BACKEND=drm|winit` escolhe;
    // sem ele, DRM quando não há desktop por baixo (nem WAYLAND_DISPLAY nem DISPLAY).
    let drm = match std::env::var("MIDE_BACKEND").as_deref() {
        Ok("drm") => true,
        Ok("winit") => false,
        _ => std::env::var_os("WAYLAND_DISPLAY").is_none() && std::env::var_os("DISPLAY").is_none(),
    };
    if drm {
        tracing::info!("backend: DRM (direto na tela)");
        backend_udev::init(&event_loop, &loop_handle, &mut state)?;
    } else {
        tracing::info!("backend: winit (janela no desktop atual)");
        backend_winit::init(&event_loop, &loop_handle, &mut state)?;
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

    tracing::info!("pronto. Teste com: WAYLAND_DISPLAY={} <app-wayland>", socket_name());

    let stop = event_loop.get_signal();
    event_loop.run(Duration::from_millis(16), &mut state, |state| {
        if let Err(err) = state.display_handle.flush_clients() {
            tracing::warn!("falha ao enviar eventos: {err}");
        }
        if state.quit_requested {
            stop.stop();
        }
    })?;

    Ok(())
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
    let data_dir = std::env::var_os("MIDE_DATA_DIR")
        .map(std::path::PathBuf::from)
        .into_iter()
        .chain([bin_dir.join("../../data"), bin_dir.join("../share/mide")])
        .find(|d| d.join("applications").is_dir());
    if let Some(data_dir) = data_dir.and_then(|d| d.canonicalize().ok()) {
        let dirs = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
        command.env("XDG_DATA_DIRS", format!("{}:{dirs}", data_dir.display()));
    }
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
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            tracing::info!("{name} iniciado ({})", program.display());
        }
        Err(err) => tracing::warn!(
            "{name} não iniciado ({}): {err}. Rode `cargo build` para gerar os programas",
            program.display()
        ),
    }
}
