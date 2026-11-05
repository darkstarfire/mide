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
    /// Estado EWMH: maximizada na vertical/horizontal ou em tela cheia.
    pub maximized_vert: bool,
    pub maximized_horz: bool,
    pub fullscreen: bool,
    /// Geometria antes de maximizar/entrar em tela cheia, para restaurar.
    pub normal: Option<(i16, i16, u16, u16)>,
}

/// Cena de renderizacao X11.
pub struct X11Scene {
    /// Janelas X11 clientes indexadas pelo ID.
    pub windows: HashMap<xproto::Window, X11Window>,
    /// Proximo ID sequencial para janelas criadas pela cena.
    pub next_id: u32,
    /// Dimensoes atuais da saida (janela de composicao).
    pub output_width: u16,
    pub output_height: u16,
}

impl X11Scene {
    /// Cria uma cena vazia.
    pub fn new() -> Self {
        Self {
            windows: HashMap::new(),
            next_id: 1,
            output_width: 0,
            output_height: 0,
        }
    }

    /// Atualiza as dimensoes da saida.
    pub fn set_output_size(&mut self, width: u16, height: u16) {
        self.output_width = width;
        self.output_height = height;
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
                maximized_vert: false,
                maximized_horz: false,
                fullscreen: false,
                normal: None,
            },
        );
    }

    /// Remove uma janela da cena.
    pub fn remove_window(&mut self, id: xproto::Window) {
        self.windows.remove(&id);
    }

    /// Marca uma janela como mapeada ou nao.
    pub fn set_mapped(&mut self, id: xproto::Window, mapped: bool) {
        if let Some(win) = self.windows.get_mut(&id) {
            win.mapped = mapped;
        }
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

    /// Estado EWMH atual e a geometria de restauracao da janela.
    pub fn state_of(
        &self,
        id: xproto::Window,
    ) -> Option<(bool, bool, bool, Option<(i16, i16, u16, u16)>)> {
        self.windows
            .get(&id)
            .map(|w| (w.maximized_vert, w.maximized_horz, w.fullscreen, w.normal))
    }

    /// Grava o novo estado EWMH da janela.
    pub fn set_state(
        &mut self,
        id: xproto::Window,
        maximized_vert: bool,
        maximized_horz: bool,
        fullscreen: bool,
        normal: Option<(i16, i16, u16, u16)>,
    ) {
        if let Some(win) = self.windows.get_mut(&id) {
            win.maximized_vert = maximized_vert;
            win.maximized_horz = maximized_horz;
            win.fullscreen = fullscreen;
            win.normal = normal;
        }
    }

    /// Desenha o quadro completo: fundo e todas as janelas clientes.
    /// Cada janela e desenhada como um retangulo com borda cujo cor indica o
    /// estado EWMH (azul=normal, verde=maximizada, vermelho=tela cheia).
    pub fn draw_frame(
        &self,
        conn: &x11rb::rust_connection::RustConnection,
        render: &mut crate::x11_render::X11Render,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let wins: Vec<_> = self
            .windows
            .values()
            .filter(|w| w.mapped)
            .map(|w| {
                (
                    w.id,
                    w.x,
                    w.y,
                    w.width,
                    w.height,
                    w.maximized_vert,
                    w.maximized_horz,
                    w.fullscreen,
                )
            })
            .collect();
        render.draw_scene(conn, &wins, (0x1a, 0x1a, 0x20))?;

        for (id, win) in &self.windows {
            if win.mapped {
                tracing::trace!(
                    "janela X11 {} em ({}, {}) {}x{} (vert={} horz={} full={})",
                    id,
                    win.x,
                    win.y,
                    win.width,
                    win.height,
                    win.maximized_vert,
                    win.maximized_horz,
                    win.fullscreen
                );
            }
        }

        Ok(())
    }
}

impl Default for X11Scene {
    fn default() -> Self {
        Self::new()
    }
}
