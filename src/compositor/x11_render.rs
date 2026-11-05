//! Renderizacao X11: desenha frames sobre a janela X11 do mide.
//!
//! Por enquanto oferece apenas a estrutura basica: um contexto associado a
//! janela X11 e metodos para redimensionar e desenhar. A renderizacao real
//! (XPutImage ou OpenGL/GLX) sera implementada em etapas posteriores.

use std::sync::Arc;

use x11rb::protocol::xproto;
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
        conn: Arc<RustConnection>,
        window: xproto::Window,
        width: u32,
        height: u32,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self {
            _conn: conn,
            window,
            width,
            height,
        })
    }

    /// Desenha um quadro completo na janela X11.
    /// Por enquanto e um no-op; a renderizacao real sera implementada depois.
    pub fn draw_frame(&mut self, _conn: &RustConnection) -> Result<(), Box<dyn std::error::Error>> {
        Ok(())
    }

    /// Redimensiona o contexto quando a janela muda de tamanho.
    pub fn resize(&mut self, _conn: &RustConnection, width: u32, height: u32) -> Result<(), Box<dyn std::error::Error>> {
        self.width = width;
        self.height = height;
        Ok(())
    }
}