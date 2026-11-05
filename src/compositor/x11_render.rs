//! Renderizacao X11: desenha frames sobre a janela X11 do mide.
//!
//! Por enquanto oferece apenas a estrutura basica: um contexto associado a
//! janela X11 e metodos para redimensionar e desenhar. A renderizacao real
//! (OpenGL/GLX ou copia de pixels via XPutImage) sera implementada em etapas
//! posteriores.

use std::sync::Arc;

use tiny_skia::Pixmap;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{self, ConnectionExt as _};
use x11rb::rust_connection::RustConnection;

/// Contexto de renderizacao sobre X11.
pub struct X11Render {
    pub _conn: Arc<RustConnection>,
    pub window: xproto::Window,
    pub width: u32,
    pub height: u32,
}

impl X11Render {
    /// Cria o contexto de renderizacao associado a janela X11 existente.
    pub fn new(
        _conn: Arc<RustConnection>,
        window: xproto::Window,
        width: u32,
        height: u32,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self {
            _conn,
            window,
            width,
            height,
        })
    }

    /// Desenha um quadro completo na janela X11.
    /// Por enquanto apenas armazena o pixmap para uso posterior.
    pub fn draw_frame(&mut self, pixmap: &Pixmap) -> Result<(), Box<dyn std::error::Error>> {
        if pixmap.width() != self.width || pixmap.height() != self.height {
            self.width = pixmap.width();
            self.height = pixmap.height();
        }
        // TODO: copiar pixels do tiny_skia para a janela X11 via XPutImage ou OpenGL.
        let _ = pixmap;
        Ok(())
    }

    /// Redimensiona o contexto quando a janela muda de tamanho.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
    }
}