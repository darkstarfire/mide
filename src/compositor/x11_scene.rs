//! Cena de renderizacao X11: gerencia janelas X11 clientes e faz composicao.
//!
//! Por enquanto e uma estrutura basica que armazena janelas X11 conhecidas e
//! oferece um placeholder para desenhar o quadro completo. A integracao com a
//! cena do Smithay (`crate::render::Scene`) sera feita em etapas posteriores.

use std::collections::HashMap;
use x11rb::protocol::xproto;

/// Janela X11 cliente conhecida pela cena.
#[derive(Debug, Clone)]
pub struct X11Window {
    pub id: xproto::Window,
    pub x: i16,
    pub y: i16,
    pub width: u16,
    pub height: u16,
    pub mapped: bool,
}

/// Cena de renderizacao X11.
pub struct X11Scene {
    /// Janelas X11 clientes indexadas pelo ID.
    pub windows: HashMap<xproto::Window, X11Window>,
    /// Proximo ID sequencial para janelas criadas pela cena.
    pub next_id: u32,
}

impl X11Scene {
    /// Cria uma cena vazia.
    pub fn new() -> Self {
        Self {
            windows: HashMap::new(),
            next_id: 1,
        }
    }

    /// Registra uma janela X11 cliente na cena.
    pub fn add_window(&mut self, id: xproto::Window, x: i16, y: i16, w: u16, h: u16) {
        self.windows.insert(
            id,
            X11Window {
                id,
                x,
                y,
                width: w,
                height: h,
                mapped: true,
            },
        );
    }

    /// Remove uma janela da cena.
    pub fn remove_window(&mut self, id: xproto::Window) {
        self.windows.remove(&id);
    }

    /// Atualiza a geometria de uma janela existente.
    pub fn update_window(
        &mut self,
        id: xproto::Window,
        x: i16,
        y: i16,
        w: u16,
        h: u16,
    ) {
        if let Some(win) = self.windows.get_mut(&id) {
            win.x = x;
            win.y = y;
            win.width = w;
            win.height = h;
        }
    }

    /// Desenha o quadro completo: fundo e todas as janelas clientes.
    /// Por enquanto preenche com uma cor solida e lista as janelas no log.
    pub fn draw_frame(
        &self,
        conn: &x11rb::rust_connection::RustConnection,
        render: &mut crate::x11_render::X11Render,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Fundo: cor escura
        render.draw_frame(conn, 0x1a, 0x1a, 0x20)?;

        // Desenha cada janela cliente como um retangulo de contorno (placeholder).
        for (id, win) in &self.windows {
            tracing::trace!(
                "janela X11 {} em ({}, {}) {}x{}",
                id,
                win.x,
                win.y,
                win.width,
                win.height
            );
        }

        Ok(())
    }
}

impl Default for X11Scene {
    fn default() -> Self {
        Self::new()
    }
}