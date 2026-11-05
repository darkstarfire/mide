//! Cena de renderizacao X11: gerencia janelas X11 clientes e faz composicao.
//!
//! Armazena janelas X11 conhecidas, estado EWMH, titulos e oferece
//! hit-testing das decorações do servidor (barra de título, botoes de fechar
//! e maximizar).

use std::collections::HashMap;
use x11rb::protocol::xproto;

/// Regiao da janela onde um clique caiu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecorationHit {
    /// Fora da janela / cliente comum (clique passa para o cliente).
    Client,
    /// Barra de titulo: arrasta a janela.
    Title,
    /// Botao fechar (X).
    Close,
    /// Botao maximizar/restaurar.
    Maximize,
}

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
    /// Titulo da janela (_NET_WM_NAME / WM_NAME).
    pub title: Option<String>,
}

/// Altura da barra de desenho do servidor, em pontos (escala ja aplicada).
pub const TITLE_HEIGHT: u16 = 30;
/// Largura de cada boto da barra de titulo, em pontos.
pub const BTN_SIZE: u16 = 28;
/// Margem entre botoes e borda, em pontos.
pub const BTN_MARGIN: u16 = 4;

impl X11Window {
    /// Hit-test de um ponto (x, y) relativo ao canto superior-esquerdo da
    /// janela (já em pixels de saida, escala aplicada).
    /// Retorna a regiao de decoraçao ou Client.
    pub fn hit_test(&self, x: i16, y: i16) -> DecorationHit {
        // Janelas em tela cheia nao tem decoraçoes.
        if self.fullscreen {
            return DecorationHit::Client;
        }
        // Fora dos limites da janela.
        if x < 0 || y < 0 || x >= self.width as i16 || y >= self.height as i16 {
            return DecorationHit::Client;
        }
        // Barra de titulo.
        if y < TITLE_HEIGHT as i16 {
            // Botoes ficam no canto direito.
            let btn_x_start = self.width as i16
                - BTN_MARGIN as i16
                - BTN_SIZE as i16;
            if x >= btn_x_start && x <= self.width as i16 {
                // Botao fechar (mais a direita) ou maximizar (ao lado).
                let close_x = btn_x_start;
                let max_x = btn_x_start.saturating_sub(BTN_SIZE as i16).saturating_sub(2);
                if x >= close_x {
                    DecorationHit::Close
                } else if x >= max_x {
                    DecorationHit::Maximize
                } else {
                    DecorationHit::Title
                }
            } else {
                DecorationHit::Title
            }
        } else {
            DecorationHit::Client
        }
    }
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
    /// Fator de escala da saida (padrão 1,0; no X11 nao existe
    /// wp_fractional_scale_v1, entao vale para toda a sessao).
    pub output_scale: f32,
}

impl X11Scene {
    /// Cria uma cena vazia.
    pub fn new() -> Self {
        Self {
            windows: HashMap::new(),
            next_id: 1,
            output_width: 0,
            output_height: 0,
            output_scale: 1.0,
        }
    }

    /// Atualiza as dimensoes da saida.
    pub fn set_output_size(&mut self, width: u16, height: u16) {
        self.output_width = width;
        self.output_height = height;
    }

    /// Define o fator de escala da saida.
    pub fn set_output_scale(&mut self, scale: f32) {
        self.output_scale = scale;
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
                title: None,
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

    /// Define o titulo de uma janela.
    pub fn set_title(&mut self, id: xproto::Window, title: Option<String>) {
        if let Some(win) = self.windows.get_mut(&id) {
            win.title = title;
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

    /// Hit-test de um ponto (x, y) em pixels de saida contra uma janela.
    /// Retorna None se a janela nao existe ou nao esta mapeada.
    pub fn hit_test_window(
        &self,
        id: xproto::Window,
        x: i16,
        y: i16,
    ) -> Option<DecorationHit> {
        let win = self.windows.get(&id)?;
        if !win.mapped {
            return None;
        }
        let k = self.output_scale;
        // Converte coordenadas de saida para coordenadas da janela (escala
        // ja aplicada na cena, entao dividimos de volta).
        let wx = (x as f32 / k) as i16;
        let wy = (y as f32 / k) as i16;
        Some(win.hit_test(wx, wy))
    }

    /// Desenha o quadro completo: fundo e todas as janelas clientes.
    /// Cada janela e desenhada com barra de título, botoes e borda pelo estado EWMH.
    pub fn draw_frame(
        &self,
        conn: &x11rb::rust_connection::RustConnection,
        render: &mut crate::x11_render::X11Render,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let k = self.output_scale;
        let wins: Vec<_> = self
            .windows
            .values()
            .filter(|w| w.mapped)
            .map(|w| {
                (
                    w.id,
                    (w.x as f32 * k) as i16,
                    (w.y as f32 * k) as i16,
                    (w.width as f32 * k) as u16,
                    (w.height as f32 * k) as u16,
                    w.maximized_vert,
                    w.maximized_horz,
                    w.fullscreen,
                    w.title.as_deref().unwrap_or(""),
                )
            })
            .collect();
        render.draw_scene(conn, &wins, (0x1a, 0x1a, 0x20))?;

        for (id, win) in &self.windows {
            if win.mapped {
                tracing::trace!(
                    "janela X11 {} em ({}, {}) {}x{} (vert={} horz={} full={}) titulo={:?}",
                    id,
                    win.x,
                    win.y,
                    win.width,
                    win.height,
                    win.maximized_vert,
                    win.maximized_horz,
                    win.fullscreen,
                    win.title
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
