//! Renderizacao X11: desenha frames sobre a janela X11 do mide.
//!
//! Usa XPutImage para copiar pixels de um buffer em memoria para a janela X11.
//! Desenha o fundo e cada janela cliente conhecida pela cena X11 com decocoes
//! do servidor: barra de titulo, botoes fechar/maximizar, borda pelo estado
//! EWMH e conteudo capturado das janelas clientes via XGetImage.

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

    /// Desenha uma linha horizontal de 1 px.
    fn hline(&mut self, x: i32, y: i32, w: u32, color: Rgb) {
        self.fill_rect(x, y, w, 1, color);
    }

    /// Desenha uma linha vertical de 1 px.
    fn vline(&mut self, x: i32, y: i32, h: u32, color: Rgb) {
        self.fill_rect(x, y, 1, h, color);
    }

    /// Desenha a borda de um retangulo (espessura 2 px) no buffer.
    fn stroke_rect(&mut self, x: i32, y: i32, w: u32, h: u32, color: Rgb, thickness: u32) {
        let t = thickness.clamp(1, 8) as i32;
        self.fill_rect(x, y, w, t as u32, color);                                // topo
        self.fill_rect(x, y + h as i32 - t, w, t as u32, color);                // fundo
        self.fill_rect(x, y, t as u32, h, color);                               // esquerda
        self.fill_rect(x + w as i32 - t, y, t as u32, h, color);                // direita
    }

    /// Desenha um "X" (botao fechar) dentro de um quadrado.
    fn draw_close(&mut self, cx: i32, cy: i32, size: u32, color: Rgb) {
        let s = size as i32;
        // Diagonal principal.
        for i in 0..s {
            let px = cx + i;
            let py = cy + i;
            if px >= 0 && py >= 0 && px < self.width as i32 && py < self.height as i32 {
                let i = ((py as u32 * self.width + px as u32) * 3) as usize;
                self.buffer[i] = color.0;
                self.buffer[i + 1] = color.1;
                self.buffer[i + 2] = color.2;
            }
        }
        // Diagonal secundaria.
        for i in 0..s {
            let px = cx + s - 1 - i;
            let py = cy + i;
            if px >= 0 && py >= 0 && px < self.width as i32 && py < self.height as i32 {
                let i = ((py as u32 * self.width + px as u32) * 3) as usize;
                self.buffer[i] = color.0;
                self.buffer[i + 1] = color.1;
                self.buffer[i + 2] = color.2;
            }
        }
    }

    /// Desenha um quadro (boto maximizar) dentro de um quadrado.
    fn draw_maximize(&mut self, cx: i32, cy: i32, size: u32, color: Rgb) {
        let s = size as i32;
        self.hline(cx, cy, s as u32, color);
        self.hline(cx, cy + s - 1, s as u32, color);
        self.vline(cx, cy, s as u32, color);
        self.vline(cx + s - 1, cy, s as u32, color);
    }

    /// Desenha um pequeno retangulo preenchido (indicador de titulo/icone).
    fn draw_title_marks(&mut self, cx: i32, cy: i32, w: u32, h: u32, color: Rgb) {
        self.fill_rect(cx, cy, w, h, color);
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

    /// Cor da barra de titulo conforme o estado EWMH da janela.
    fn title_color(maximized_vert: bool, maximized_horz: bool, fullscreen: bool) -> Rgb {
        if fullscreen {
            (0x30, 0x08, 0x08)
        } else if maximized_vert && maximized_horz {
            (0x08, 0x30, 0x08)
        } else if maximized_vert || maximized_horz {
            (0x30, 0x30, 0x08)
        } else {
            (0x08, 0x18, 0x30)
        }
    }

    /// Captura o conteudo de uma janela X11 via XGetImage (RGB, sem alpha).
    /// Retorna None se a janela nao for visivel ou houver erro.
    pub fn capture_window(
        &self,
        conn: &RustConnection,
        window: xproto::Window,
        w: u32,
        h: u32,
    ) -> Option<Vec<u8>> {
        let geom = conn.get_geometry(window).ok()?.reply().ok()?;
        let w = w.max(geom.width as u32);
        let h = h.max(geom.height as u32);
        let image = conn
            .get_image(
                xproto::ImageFormat::Z_PIXMAP,
                window,
                0,
                0,
                w as u16,
                h as u16,
                0xFF, // plane mask: todos os planos
            )
            .ok()?
            .reply()
            .ok()?;
        let data: Vec<u8> = image.data;
        // Z_PIXMAP com profundidade 24: 4 bytes por pixel (BGRX em little-endian).
        // Convertemos para RGB (3 bytes por pixel).
        let mut rgb = Vec::with_capacity((w * h * 3) as usize);
        for i in (0..data.len()).step_by(4) {
            if i + 3 >= data.len() {
                break;
            }
            // BGRX -> RGB
            rgb.push(data[i + 2]); // R
            rgb.push(data[i + 1]); // G
            rgb.push(data[i]);     // B
        }
        Some(rgb)
    }

    /// Desenha um quadro completo: fundo e cada janela cliente da cena.
    /// `wins` contem: (id, x, y, w, h, maximized_vert, maximized_horz, fullscreen,
    /// title, content_rgb, content_w, content_h).
    pub fn draw_scene(
        &mut self,
        wins: &[(xproto::Window, i16, i16, u16, u16, bool, bool, bool, &str, Option<&[u8]>, u32, u32)],
        bg: Rgb,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Fundo: cor solida.
        self.fill_rect(0, 0, self.width, self.height, bg);

        for (_id, x, y, w, h, maximized_vert, maximized_horz, fullscreen, title, content, cw, ch) in wins {
            let x = *x as i32;
            let y = *y as i32;
            let w = *w as u32;
            let h = *h as u32;
            if w == 0 || h == 0 {
                continue;
            }

            let title_h = super::x11_scene::TITLE_HEIGHT as u32;
            let btn_size = super::x11_scene::BTN_SIZE as u32;
            let btn_margin = super::x11_scene::BTN_MARGIN as u32;

            // Conteudo da janela (abaixo da barra de titulo, se houver).
            let content_y = if *fullscreen { 0 } else { title_h as i32 };
            let content_h = h.saturating_sub(content_y as u32);

            if let Some(pixels) = content {
                // Desenha o conteúdo capturado do cliente na area de conteúdo.
                let src_w = *cw;
                let src_h = *ch;
                let dst_x = x;
                let dst_y = y + content_y;
                let dst_w = w;
                let dst_h = content_h;

                for dy in 0..dst_h {
                    let sy = (dy as u32 * src_h) / dst_h;
                    if sy >= src_h {
                        break;
                    }
                    for dx in 0..dst_w {
                        let sx = (dx as u32 * src_w) / dst_w;
                        if sx >= src_w {
                            break;
                        }
                        let si = ((sy * src_w + sx) * 3) as usize;
                        if si + 2 >= pixels.len() {
                            break;
                        }
                        let di = ((dst_y as usize + dy as usize) * self.width as usize + (dst_x as usize + dx as usize)) * 3;
                        if di + 2 >= self.buffer.len() {
                            break;
                        }
                        self.buffer[di] = pixels[si];     // R
                        self.buffer[di + 1] = pixels[si + 1]; // G
                        self.buffer[di + 2] = pixels[si + 2]; // B
                    }
                }
            } else {
                // Sem conteúdo capturado: preenche com cor sólida.
                self.fill_rect(
                    x,
                    y + content_y,
                    w,
                    content_h,
                    (0x18, 0x18, 0x20),
                );
            }

            if !*fullscreen {
                // Barra de título.
                let tcolor = Self::title_color(*maximized_vert, *maximized_horz, *fullscreen);
                self.fill_rect(x, y, w, title_h, tcolor);

                // Indicador de título (retangulo branco simples no canto esquerdo).
                self.draw_title_marks(x + 8, y + 8, 16, 16, (0xc0, 0xc0, 0xc0));

                // Botoes no canto direito.
                let close_x = (x + w as i32) - btn_margin as i32 - btn_size as i32;
                let max_x = close_x.saturating_sub(btn_size as i32 + 2);
                let btn_y = y + (title_h as i32 - btn_size as i32) / 2;

                // Botao fechar (X vermelho).
                self.draw_close(close_x, btn_y, btn_size, (0xd0, 0x40, 0x40));
                self.stroke_rect(close_x, btn_y, btn_size, btn_size, (0x60, 0x20, 0x20), 1);

                // Botao maximizar.
                self.draw_maximize(max_x, btn_y, btn_size, (0x40, 0x80, 0xd0));
                self.stroke_rect(max_x, btn_y, btn_size, btn_size, (0x20, 0x40, 0x70), 1);

                // Linha separadora abaixo da barra de título.
                self.hline(x, y + title_h as i32, w, (0x40, 0x40, 0x48));
            }

            // Borda externa pelo estado.
            let bcolor = Self::border_color(*maximized_vert, *maximized_horz, *fullscreen);
            self.stroke_rect(x, y, w, h, bcolor, 3);
        }

        // Copia os pixels para a janela X11 via XPutImage.
        self._conn.put_image(
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
        self._conn.flush()?;
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
