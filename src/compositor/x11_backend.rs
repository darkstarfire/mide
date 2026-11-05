//! Backend X11: roda o mide como servidor de exibicao X11.
//!
//! Nao existe suporte nativo no Smithay para atuar como servidor X11, entao este
//! backend cria a propria conexao X11 (via x11rb), abre uma janela raiz de
//! composicao, renderiza com OpenGL sobre X11 (via glow) e integra com o event
//! loop calloop ja usado pelo mide.

use smithay::reexports::calloop::{generic::Generic, Interest, LoopHandle, Mode, PostAction};
use std::{
    error::Error,
    os::fd::{AsFd, AsRawFd, BorrowedFd, RawFd},
    sync::Arc,
};
use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{self, ConnectionExt as _};
use x11rb::rust_connection::RustConnection;

use crate::state::State;
use crate::x11_render::X11Render;

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
    pub render: Option<X11Render>,
    pub width: u32,
    pub height: u32,
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

        let atom_wm_delete_window = conn.intern_atom(false, b"WM_DELETE_WINDOW")?.reply()?.atom;
        let atom_net_wm_pid = conn.intern_atom(false, b"_NET_WM_PID")?.reply()?.atom;
        let atom_net_wm_name = conn.intern_atom(false, b"_NET_WM_NAME")?.reply()?.atom;

        conn.flush()?;

        let render = X11Render::new(conn.clone(), window, width, height)?;

        let backend = X11Backend {
            conn,
            screen_num: screen_num.try_into().unwrap_or(0),
            window,
            atom_wm_delete_window,
            atom_net_wm_pid,
            atom_net_wm_name,
            render: Some(render),
            width,
            height,
        };

        let raw_fd = backend.conn.stream().as_raw_fd();
        let backend_clone = backend.conn.clone();
        loop_handle.insert_source(
            Generic::new(FdWrapper(raw_fd), Interest::READ, Mode::Level),
            move |_, _, state: &mut State| {
                Self::dispatch_x11_events(&backend_clone, state);
                Ok(PostAction::Continue)
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
    fn dispatch_x11_events(conn: &Arc<RustConnection>, state: &mut State) {
        let _ = state;
        loop {
            match conn.poll_for_event() {
                Ok(Some(event)) => {
                    match event {
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
                        Event::DestroyNotify(ev) => {
                            tracing::trace!("X11 janela destruida: {}", ev.window);
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
}