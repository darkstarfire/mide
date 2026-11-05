//! Renderizacao X11: desenha frames sobre a janela X11 do mide.
//!
//! Usa XPutImage para copiar pixels de um buffer em memoria para a janela X11.
//! Desenha o fundo e cada janela cliente conhecida pela cena X11 como um
//! retangulo com borda (placeholders visuais, sem fontes nem texturas).

use std::sync::Arc;

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{self, ConnectionExt as _};
use x11rb::rust_connection::RustConnection;

/// Cor RGB.
type Rgb = (u8, u8, u8);

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

    /// Preenche um retangulo no buffer (clipe ao tamanho do buffer).
    fn fill_rect(&mut self, x: i32, y: i32, w: u32, h: u32, color: Rgb) {
        let x0 = x.clamp(0, self.width as i32) as u32;
        let y0 = y.clamp(0, self.height as i32) as u32;
        let x1 = (x + w as i32).clamp(0, self.width as i32) as u32;
        let y1 = (y + h as i32).clamp(0, self.height as i32) as u32;
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        let (r, g, b) = color;
        for yy in y0..y1 {
            let row = yy * self.width;
            for xx in x0..x1 {
                let i = ((row + xx) * 3) as usize;
                self.buffer[i] = r;
                self.buffer[i + 1] = g;
                self.buffer[i + 2] = b;
            }
        }
    }

    /// Desenha a borda de um retangulo (espessura 2 px) no buffer.
    fn stroke_rect(&mut self, x: i32, y: i32, w: u32, h: u32, color: Rgb, thickness: u32) {
        let t = thickness.clamp(1, 8) as i32;
        self.fill_rect(x, y, w, t as u32, color);                                // topo
        self.fill_rect(x, y + h as i32 - t, w, t as u32, color);                // fundo
        self.fill_rect(x, y, t as u32, h, color);                               // esquerda
        self.fill_rect(x + w as i32 - t, y, t as u32, h, color);                // direita
    }

    /// Cor da borda conforme o estado EWMH da janela.
    fn border_color(maximized_vert: bool, maximized_horz: bool, fullscreen: bool) -> Rgb {
        if fullscreen {
            (0x50, 0x10, 0x10) // vermelho escuro
        } else if maximized_vert && maximized_horz {
            (0x10, 0x50, 0x10) // verde escuro
        } else if maximized_vert || maximized_horz {
            (0x50, 0x50, 0x10) // amarelo escuro
        } else {
            (0x10, 0x30, 0x60) // azul
        }
    }

    /// Desenha um quadro completo: fundo e cada janela cliente da cena.
    pub fn draw_scene(
        &mut self,
        conn: &RustConnection,
        wins: &[(xproto::Window, i16, i16, u16, u16, bool, bool, bool)],
        bg: Rgb,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Fundo: cor solida com uma suave variacao ao longo do tempo para
        // confirmar que o quadro esta sendo redesenhado.
        self.fill_rect(0, 0, self.width, self.height, bg);

        for (_id, x, y, w, h, maximized_vert, maximized_horz, fullscreen) in wins {
            let x = *x as i32;
            let y = *y as i32;
            let w = *w as u32;
            let h = *h as u32;
            if w == 0 || h == 0 {
                continue;
            }
            // Faixa de titulo (mais escura) + conteudo.
            let title_h = 24u32.min(h);
            self.fill_rect(x, y, w, title_h, (0x20, 0x20, 0x28));
            self.fill_rect(x, y + title_h as i32, w, h.saturating_sub(title_h), (0x18, 0x18, 0x20));
            // Borda pelo estado.
            let color = Self::border_color(*maximized_vert, *maximized_horz, *fullscreen);
            self.stroke_rect(x, y, w, h, color, 3);
        }

        // Copia os pixels para a janela X11 via XPutImage.
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
        _conn: &RustConnection,
        width: u32,
        height: u32,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.width = width;
        self.height = height;
        self.buffer = vec![0u8; (width * height * 3) as usize];
        Ok(())
    }
}
