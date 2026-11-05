//! Renderizacao X11: desenha frames sobre a janela X11 do mide.
//!
//! Usa XPutImage para copiar pixels de um buffer em memoria para a janela X11.
//! Por enquanto desenha uma cor solida por frame; a renderizacao de conteudo de
//! janelas clientes sera adicionada em etapas posteriores.

use std::sync::Arc;

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{self, ConnectionExt as _};
use x11rb::rust_connection::RustConnection;

/// Contexto de renderizacao sobre X11.
pub struct X11Render {
    pub _conn: Arc<RustConnection>,
    pub window: xproto::Window,
    pub gc: xproto::Gcontext,
    pub width: u32,
    pub height: u32,
    /// Buffer de pixels RGB (8 bits por canal, sem alpha).
    pub buffer: Vec<u8>,
}

impl X11Render {
    /// Cria o contexto de renderizacao associado a janela X11 existente.
    pub fn new(
        conn: Arc<RustConnection>,
        window: xproto::Window,
        width: u32,
        height: u32,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let gc = conn.generate_id()?;
        let aux = xproto::CreateGCAux {
            plane_mask: Some(0xFFFFFFFF),
            ..Default::default()
        };
        conn.create_gc(gc, window, &aux)?;

        let buffer = vec![0u8; (width * height * 3) as usize];

        Ok(Self {
            _conn: conn,
            window,
            gc,
            width,
            height,
            buffer,
        })
    }

    /// Desenha um quadro completo na janela X11, preenchendo com a cor dada.
    pub fn draw_frame(
        &mut self,
        conn: &RustConnection,
        r: u8,
        g: u8,
        b: u8,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Preenche o buffer com a cor solida (RGB).
        for pixel in self.buffer.chunks_exact_mut(3) {
            pixel[0] = r;
            pixel[1] = g;
            pixel[2] = b;
        }

        // Copia os pixels para a janela X11 via XPutImage.
        // Usamos ZPixmap com profundidade 24 (RGB).
        conn.put_image(
            xproto::ImageFormat::Z_PIXMAP,
            self.window,
            self.gc,
            self.width as u16,
            self.height as u16,
            0, // dst_x
            0, // dst_y
            0, // left_pad
            24, // depth (24-bit RGB)
            &self.buffer,
        )?;

        conn.flush()?;
        Ok(())
    }

    /// Redimensiona o contexto quando a janela muda de tamanho.
    pub fn resize(
        &mut self,
        conn: &RustConnection,
        width: u32,
        height: u32,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.width = width;
        self.height = height;
        self.buffer = vec![0u8; (width * height * 3) as usize];
        let _ = conn;
        Ok(())
    }
}