//! Backend X11: roda o mide como servidor de exibicao X11.
//!
//! Nao existe suporte nativo no Smithay para atuar como servidor X11, entao este
//! backend cria a propria conexao X11 (via x11rb), abre uma janela raiz de
//! composicao, renderiza sobre X11 via XPutImage e integra com o event loop
//! calloop ja usado pelo mide.

use smithay::reexports::calloop::{
    generic::Generic, timer::Timer, Interest, LoopHandle, Mode, PostAction,
};
use std::{
    error::Error,
    os::fd::{AsFd, AsRawFd, BorrowedFd, RawFd},
    sync::{Arc, Mutex},
    time::Duration,
};
use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{self, ConnectionExt as _};
use x11rb::rust_connection::RustConnection;

use crate::state::State;
use crate::x11_render::X11Render;
use crate::x11_scene::X11Scene;

/// Wrapper sem assumir propriedade do fd, para uso com `calloop::Generic`.
struct FdWrapper(RawFd);

impl AsFd for FdWrapper {
    fn as_fd(&self) -> BorrowedFd<'_> {
        // SAFETY: o fd e valido enquanto a conexao X11 estiver viva.
        unsafe { BorrowedFd::borrow_raw(self.0) }
    }
}

/// Conexao X11 encapsulada, compartilhada entre o backend e outros modulos.
pub struct X11Backend {
    pub conn: Arc<RustConnection>,
    pub screen_num: i32,
    pub window: xproto::Window,
    pub atom_wm_delete_window: xproto::Atom,
    pub atom_net_wm_pid: xproto::Atom,
    pub atom_net_wm_name: xproto::Atom,
    pub render: Arc<Mutex<X11Render>>,
    pub scene: Arc<Mutex<X11Scene>>,
    pub width: u32,
    pub height: u32,
    pub frame_count: u64,
}

impl X11Backend {
    /// Conecta ao display X11, cria a janela de composicao e registra no event loop.
    pub fn init(
        loop_handle: &LoopHandle<'static, State>,
        _state: &mut State,
    ) -> Result<Self, Box<dyn Error>> {
        let (conn, screen_num) = RustConnection::connect(None)?;
        let conn = Arc::new(conn);

        let setup = conn.setup();
        let screen = &setup.roots[screen_num as usize];
        let width = screen.width_in_pixels as u32;
        let height = screen.height_in_pixels as u32;

        let window = conn.generate_id()?;
        let aux = xproto::CreateWindowAux {
            background_pixmap: None,
            background_pixel: None,
            border_pixmap: None,
            border_pixel: None,
            bit_gravity: None,
            win_gravity: None,
            backing_store: None,
            backing_planes: None,
            backing_pixel: None,
            override_redirect: None,
            save_under: None,
            event_mask: Some(
                xproto::EventMask::EXPOSURE
                    | xproto::EventMask::KEY_PRESS
                    | xproto::EventMask::KEY_RELEASE
                    | xproto::EventMask::BUTTON_PRESS
                    | xproto::EventMask::BUTTON_RELEASE
                    | xproto::EventMask::POINTER_MOTION
                    | xproto::EventMask::ENTER_WINDOW
                    | xproto::EventMask::LEAVE_WINDOW
                    | xproto::EventMask::STRUCTURE_NOTIFY,
            ),
            do_not_propogate_mask: None,
            colormap: None,
            cursor: None,
        };
        conn.create_window(
            screen.root_depth,
            window,
            screen.root,
            0,
            0,
            screen.width_in_pixels,
            screen.height_in_pixels,
            0,
            xproto::WindowClass::INPUT_OUTPUT,
            screen.root_visual,
            &aux,
        )?;

        conn.map_window(window)?;

        // Assume o papel de window manager: intercepta requisicoes de mapeamento
        // e configuracao de janelas clientes na janela raiz.
        let root_attr = xproto::ChangeWindowAttributesAux::new().event_mask(
            xproto::EventMask::SUBSTRUCTURE_REDIRECT | xproto::EventMask::SUBSTRUCTURE_NOTIFY,
        );
        if let Err(err) = conn.change_window_attributes(screen.root, &root_attr) {
            tracing::warn!("nao foi possivel assumir SubstructureRedirect na raiz: {err}");
        }

        let atom_wm_delete_window = conn.intern_atom(false, b"WM_DELETE_WINDOW")?.reply()?.atom;
        let atom_net_wm_pid = conn.intern_atom(false, b"_NET_WM_PID")?.reply()?.atom;
        let atom_net_wm_name = conn.intern_atom(false, b"_NET_WM_NAME")?.reply()?.atom;

        conn.flush()?;

        let render = X11Render::new(conn.clone(), window, width, height)?;
        let render = Arc::new(Mutex::new(render));
        let scene = Arc::new(Mutex::new(X11Scene::new()));

        let backend = X11Backend {
            conn,
            screen_num: screen_num.try_into().unwrap_or(0),
            window,
            atom_wm_delete_window,
            atom_net_wm_pid,
            atom_net_wm_name,
            render: render.clone(),
            scene: scene.clone(),
            width,
            height,
            frame_count: 0,
        };

        let raw_fd = backend.conn.stream().as_raw_fd();
        let backend_clone = backend.conn.clone();
        let scene_dispatch = backend.scene.clone();
        let comp_window = backend.window;
        loop_handle.insert_source(
            Generic::new(FdWrapper(raw_fd), Interest::READ, Mode::Level),
            move |_, _, state: &mut State| {
                Self::dispatch_x11_events(&backend_clone, &scene_dispatch, comp_window, state);
                Ok(PostAction::Continue)
            },
        )?;

        // Timer de renderizacao: dispara a cada 16 ms (~60 FPS) para manter a
        // tela atualizada.
        let render_conn = backend.conn.clone();
        let render_clone = render.clone();
        let scene_clone = backend.scene.clone();
        let mut frame_count: u64 = 0;
        loop_handle.insert_source(
            Timer::from_duration(Duration::from_millis(16)),
            move |_, _, state: &mut State| {
                let _ = state;
                // Desenha o quadro completo usando a cena X11.
                if let Ok(mut render) = render_clone.lock() {
                    if let Err(err) = scene_clone.lock().unwrap().draw_frame(&render_conn, &mut render) {
                        tracing::warn!("erro ao desenhar cena X11: {err}");
                    }
                }
                frame_count = frame_count.wrapping_add(1);
                smithay::reexports::calloop::timer::TimeoutAction::ToDuration(Duration::from_millis(16))
            },
        )?;

        tracing::info!(
            "backend X11 inicializado (tela {}, janela {}, {}x{})",
            screen_num,
            window,
            width,
            height
        );
        Ok(backend)
    }

    /// Despacha todos os eventos X11 pendentes.
    fn dispatch_x11_events(
        conn: &Arc<RustConnection>,
        scene: &Arc<Mutex<X11Scene>>,
        comp_window: xproto::Window,
        state: &mut State,
    ) {
        let _ = state;
        loop {
            match conn.poll_for_event() {
                Ok(Some(event)) => {
                    match event {
                        Event::MapRequest(ev) => {
                            // Janela cliente pediu para ser mapeada: reparenta sob a
                            // janela de composicao e mapeia.
                            if let Err(err) =
                                Self::adopt_window(conn, scene, comp_window, ev.window)
                            {
                                tracing::warn!(
                                    "falha ao adotar janela X11 {}: {err}",
                                    ev.window
                                );
                            }
                        }
                        Event::ConfigureRequest(ev) => {
                            // Repassa a requisicao de configuracao para a janela cliente.
                            let aux = xproto::ConfigureWindowAux::new()
                                .x(ev.x as i32)
                                .y(ev.y as i32)
                                .width(ev.width as u32)
                                .height(ev.height as u32);
                            let _ = conn.configure_window(ev.window, &aux);
                            let _ = conn.flush();
                        }
                        Event::UnmapNotify(ev) => {
                            if let Ok(mut scene) = scene.lock() {
                                scene.set_mapped(ev.window, false);
                            }
                        }
                        Event::DestroyNotify(ev) => {
                            if let Ok(mut scene) = scene.lock() {
                                scene.remove_window(ev.window);
                            }
                            tracing::debug!("janela X11 {} destruida", ev.window);
                        }
                        Event::KeyPress(ev) => {
                            tracing::trace!("X11 tecla pressionada: code={}", ev.detail);
                        }
                        Event::KeyRelease(ev) => {
                            tracing::trace!("X11 tecla solta: code={}", ev.detail);
                        }
                        Event::ButtonPress(ev) => {
                            tracing::trace!(
                                "X11 botao pressionado: button={} em ({}, {})",
                                ev.detail,
                                ev.event_x,
                                ev.event_y
                            );
                            // Foca a janela cliente sob o ponteiro, se houver.
                            if ev.child != 0 {
                                let _ = conn.set_input_focus(
                                    xproto::InputFocus::POINTER_ROOT,
                                    ev.child,
                                    xproto::Time::CURRENT_TIME,
                                );
                                let _ = conn.flush();
                            }
                        }
                        Event::EnterNotify(ev) => {
                            if ev.child != 0 {
                                let _ = conn.set_input_focus(
                                    xproto::InputFocus::POINTER_ROOT,
                                    ev.child,
                                    xproto::Time::CURRENT_TIME,
                                );
                                let _ = conn.flush();
                            }
                        }
                        Event::ButtonRelease(ev) => {
                            tracing::trace!(
                                "X11 botao solto: button={} em ({}, {})",
                                ev.detail,
                                ev.event_x,
                                ev.event_y
                            );
                        }
                        Event::MotionNotify(ev) => {
                            tracing::trace!(
                                "X11 movimento do ponteiro em ({}, {})",
                                ev.event_x,
                                ev.event_y
                            );
                        }
                        Event::Expose(ev) => {
                            tracing::trace!(
                                "X11 exposicao: janela {}, ({}, {}), {}x{}",
                                ev.window,
                                ev.x,
                                ev.y,
                                ev.width,
                                ev.height
                            );
                        }
                        Event::ConfigureNotify(ev) => {
                            tracing::trace!(
                                "X11 configuracao: janela {}, ({}, {}), {}x{}",
                                ev.window,
                                ev.x,
                                ev.y,
                                ev.width,
                                ev.height
                            );
                        }
                        Event::ClientMessage(ev) => {
                            tracing::trace!("X11 mensagem do cliente: janela {}", ev.window);
                        }
                        _ => {}
                    }
                }
                Ok(None) => break,
                Err(err) => {
                    tracing::warn!("erro ao ler evento X11: {err}");
                    break;
                }
            }
        }
    }

    /// Adota uma janela cliente: reparenta sob a janela de composicao, registra
    /// na cena e a mapeia.
    fn adopt_window(
        conn: &Arc<RustConnection>,
        scene: &Arc<Mutex<X11Scene>>,
        comp_window: xproto::Window,
        window: xproto::Window,
    ) -> Result<(), Box<dyn Error>> {
        conn.reparent_window(window, comp_window, 0, 0)?;

        let geom = conn.get_geometry(window)?.reply()?;
        let attrs = conn.get_window_attributes(window)?.reply()?;
        if let Ok(mut scene) = scene.lock() {
            scene.add_window(window, 0, 0, geom.width, geom.height);
            scene.set_mapped(window, attrs.map_state == xproto::MapState::VIEWABLE);
        }

        conn.map_window(window)?;
        let _ = conn.set_input_focus(
            xproto::InputFocus::POINTER_ROOT,
            window,
            xproto::Time::CURRENT_TIME,
        );
        conn.flush()?;
        tracing::info!(
            "janela X11 {} adotada ({}x{})",
            window,
            geom.width,
            geom.height
        );
        Ok(())
    }
}