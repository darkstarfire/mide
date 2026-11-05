//! Cena de renderizacao X11: gerencia janelas X11 clientes e faz composicao.
//!
//! Armazena janelas X11 conhecidas, estado EWMH, titulos e oferece
//! hit-testing das decorações do servidor (barra de título, botoes de fechar
//! e maximizar).

use std::collections::HashMap;
use x11rb::connection::Connection;
use x11rb::protocol::xproto;
use x11rb::protocol::xproto::ConnectionExt;
use crate::x11_render::X11Render;

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
    /// Geometria antes de maximizar/entrar em tela cheia.
    pub normal: Option<(i16, i16, u16, u16)>,
    /// Titulo da janela (_NET_WM_NAME / WM_NAME).
    pub title: Option<String>,
    /// Conteudo capturado da janela (RGB, sem alpha) via XGetImage.
    pub content_buffer: Option<Vec<u8>>,
    /// Dimensoes do conteudo capturado.
    pub content_size: Option<(u32, u32)>,
    /// O conteudo precisa ser recapturado inteiro?
    pub content_dirty: bool,
    /// Regioes sujas em coordenadas da janela (x, y, w, h) em pixels de saida.
    /// Quando vazio, a janela inteira esta limpa.
    pub dirty_rects: Vec<(i16, i16, u16, u16)>,
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
                let max_x = close_x.saturating_sub(BTN_SIZE as i32 as i16).saturating_sub(2);
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
                content_buffer: None,
                content_size: None,
                content_dirty: true,
                dirty_rects: Vec::new(),
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
            win.content_dirty = true;
            win.dirty_rects.push((0, 0, w, h));
        }
    }

    /// Define o titulo de uma janela.
    pub fn set_title(&mut self, id: xproto::Window, title: Option<String>) {
        if let Some(win) = self.windows.get_mut(&id) {
            win.title = title;
        }
    }

    /// Marca o conteudo de uma janela como precisando de recaptura (appos Expose/ConfigureNotify).
    pub fn mark_content_dirty(&mut self, id: xproto::Window) {
        if let Some(win) = self.windows.get_mut(&id) {
            win.content_dirty = true;
        }
    }

    /// Marca uma regiao da janela como suja, precisando de redesenho.
    /// Coordenadas em pixels de saida, relativas ao canto superior-esquerdo da janela.
    pub fn mark_dirty_rect(&mut self, id: xproto::Window, x: i16, y: i16, w: u16, h: u16) {
        if let Some(win) = self.windows.get_mut(&id) {
            if w == 0 || h == 0 {
                return;
            }
            win.dirty_rects.push((x, y, w, h));
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
    /// Apenas janelas sujas sao redesenhadas; janelas estaticas sao puladas.
    pub fn draw_frame(
        &mut self,
        conn: &x11rb::rust_connection::RustConnection,
        render: &mut crate::x11_render::X11Render,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let k = self.output_scale;

        // Fundo: cor solida (sempre desenha, pois janelas nao cobrem tudo).
        render.fill_rect(0, 0, self.output_width as u32, self.output_height as u32, (0x1a, 0x1a, 0x20));

        // Primeira passagem: coletar janelas sujas e suas regioes.
        let mut dirty_wins: Vec<_> = self
            .windows
            .values_mut()
            .filter(|w| w.mapped && (w.content_dirty || !w.dirty_rects.is_empty()))
            .map(|w| {
                // Se o conteudo inteiro precisa ser recapturado, faz isso primeiro.
                if w.content_dirty {
                    if let Some(buf) = render.capture_window(
                        conn,
                        w.id,
                        w.width as u32,
                        w.height as u32,
                    ) {
                        let (cw, ch) = (w.width as u32, w.height as u32);
                        w.content_buffer = Some(buf);
                        w.content_size = Some((cw, ch));
                        w.content_dirty = false;
                        w.dirty_rects.clear(); // conteudo inteiro ja esta atualizado
                    }
                }
                let content = w.content_buffer.as_deref();
                let (cw, ch) = w.content_size.unwrap_or((0, 0));
                let rects: Vec<(i16, i16, u16, u16)> =
                    std::mem::take(&mut w.dirty_rects);
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
                    content,
                    cw,
                    ch,
                    rects,
                )
            })
            .collect();

        // Desenha apenas janelas sujas.
        for (_id, x, y, w, h, maximized_vert, maximized_horz, fullscreen, title, content, cw, ch, rects) in dirty_wins {
            let x = x as i32;
            let y = y as i32;
            let w = w as u32;
            let h = h as u32;
            if w == 0 || h == 0 {
                continue;
            }

            let title_h = TITLE_HEIGHT as u32;
            let btn_size = BTN_SIZE as u32;
            let btn_margin = BTN_MARGIN as u32;

            let content_y = if fullscreen { 0 } else { title_h as i32 };
            let content_h = h.saturating_sub(content_y as u32);

            // Se nao tem regioes sujas específicas, desenha a janela inteira.
            if rects.is_empty() {
                if let Some(pixels) = content {
                    let src_w = cw;
                    let src_h = ch;
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
                            let sx = (dx as u32 * src_w) / src_w.max(1);
                            if sx >= src_w {
                                break;
                            }
                            let si = ((sy * src_w + sx) * 3) as usize;
                            if si + 2 >= pixels.len() {
                                break;
                            }
                            let di = ((dst_y as usize + dy as usize) * render.width as usize + (dst_x as usize + dx as usize)) * 3;
                            if di + 2 >= render.buffer.len() {
                                break;
                            }
                            render.buffer[di] = pixels[si];
                            render.buffer[di + 1] = pixels[si + 1];
                            render.buffer[di + 2] = pixels[si + 2];
                        }
                    }
                } else {
                    render.fill_rect(x, y + content_y, w, content_h, (0x18, 0x18, 0x20));
                }

                if !fullscreen {
                    let tcolor = X11Render::title_color(maximized_vert, maximized_horz, fullscreen);
                    render.fill_rect(x, y, w, title_h, tcolor);
                    render.draw_title_marks(x + 8, y + 8, 16, 16, (0xc0, 0xc0, 0xc0));

                    let close_x = (x + w as i32) - btn_margin as i32 - btn_size as i32;
                    let max_x = close_x.saturating_sub(btn_size as i32 + 2);
                    let btn_y = y + (title_h as i32 - btn_size as i32) / 2;

                    render.draw_close(close_x, btn_y, btn_size, (0xd0, 0x40, 0x40));
                    render.stroke_rect(close_x, btn_y, btn_size, btn_size, (0x60, 0x20, 0x20), 1);

                    render.draw_maximize(max_x, btn_y, btn_size, (0x40, 0x80, 0xd0));
                    render.stroke_rect(max_x, btn_y, btn_size, btn_size, (0x20, 0x40, 0x70), 1);

                    render.hline(x, y + title_h as i32, w, (0x40, 0x40, 0x48));
                }

                let bcolor = X11Render::border_color(maximized_vert, maximized_horz, fullscreen);
                render.stroke_rect(x, y, w, h, bcolor, 3);
            } else {
                // Desenha apenas regioes sujas.
                for (rx, ry, rw, rh) in &rects {
                    let rx = *rx as i32;
                    let ry = *ry as i32;
                    let rw = *rw as u32;
                    let rh = *rh as u32;
                    if rw == 0 || rh == 0 {
                        continue;
                    }

                    // rx, ry ja estao em pixels de saida relativos ao canto da janela.
                    let abs_x = x + rx as i32;
                    let abs_y = y + ry as i32;

                    // Desenha conteudo se houver.
                    if let Some(pixels) = content {
                        let src_w = cw;
                        let src_h = ch;
                        for dy in 0..rh {
                            let sy = (dy as u32 * src_h) / rw.max(1);
                            if sy >= src_h {
                                break;
                            }
                            for dx in 0..rw {
                                let sx = (dx as u32 * src_w) / rw.max(1);
                                if sx >= src_w {
                                    break;
                                }
                                let si = ((sy * src_w + sx) * 3) as usize;
                                if si + 2 >= pixels.len() {
                                    break;
                                }
                                let di = ((abs_y as usize + dy as usize) * render.width as usize + (abs_x as usize + dx as usize)) * 3;
                                if di + 2 >= render.buffer.len() {
                                    break;
                                }
                                render.buffer[di] = pixels[si];
                                render.buffer[di + 1] = pixels[si + 1];
                                render.buffer[di + 2] = pixels[si + 2];
                            }
                        }
                    }

                    // Desenha decoracoes apenas se a regiao suja intersectar a barra de titulo ou bordas.
                    if !fullscreen {
                        // Barra de titulo: de y a y + title_h
                        if ry < title_h as i32 && ry + rh as i32 > y {
                            // Interseca com barra de título.
                            let draw_y = y.max(ry);
                            let draw_h = (ry + rh as i32).min(title_h as i32) - draw_y;
                            if draw_h > 0 {
                                let tcolor = X11Render::title_color(maximized_vert, maximized_horz, fullscreen);
                                render.fill_rect(x, draw_y, w, draw_h as u32, tcolor);
                                render.draw_title_marks(x + 8, draw_y + 8, 16, 16, (0xc0, 0xc0, 0xc0));

                                let close_x = (x + w as i32) - btn_margin as i32 - btn_size as i32;
                                let max_x = close_x.saturating_sub(btn_size as i32 + 2);
                                let btn_y = y + (title_h as i32 - btn_size as i32) / 2;

                                // Botao fechar so se intersectar.
                                if ry + rh as i32 > btn_y && ry < btn_y + btn_size as i32 && rx + rw as i32 > close_x && rx < close_x + btn_size as i32 {
                                    render.draw_close(close_x, btn_y, btn_size, (0xd0, 0x40, 0x40));
                                    render.stroke_rect(close_x, btn_y, btn_size, btn_size, (0x60, 0x20, 0x20), 1);
                                }
                                // Botao maximizar.
                                if ry + rh as i32 > btn_y && ry < btn_y + btn_size as i32 && rx + rw as i32 > max_x && rx < max_x + btn_size as i32 {
                                    render.draw_maximize(max_x, btn_y, btn_size, (0x40, 0x80, 0xd0));
                                    render.stroke_rect(max_x, btn_y, btn_size, btn_size, (0x20, 0x40, 0x70), 1);
                                }

                                // Linha separadora.
                                if ry + rh as i32 > y + title_h as i32 && ry < y + title_h as i32 {
                                    render.hline(x, y + title_h as i32, w, (0x40, 0x40, 0x48));
                                }
                            }
                        }
                        // Borda esquerda.
                        if rx < x as i32 + 3 && rx + rw as i32 > x as i32 {
                            let bcolor = X11Render::border_color(maximized_vert, maximized_horz, fullscreen);
                            render.stroke_rect(x, y, w, h, bcolor, 3);
                        }
                        // Borda direita.
                        if rx + rw as i32 > x as i32 + w as i32 && rx < x as i32 + w as i32 {
                            let bcolor = X11Render::border_color(maximized_vert, maximized_horz, fullscreen);
                            render.stroke_rect(x, y, w, h, bcolor, 3);
                        }
                        // Borda superior e inferior.
                        if ry < title_h as i32 || ry + rh as i32 > y + h as i32 - 3 {
                            let bcolor = X11Render::border_color(maximized_vert, maximized_horz, fullscreen);
                            render.stroke_rect(x, y, w, h, bcolor, 3);
                        }
                    }
                }
            }
        }

        // Copia os pixels para a janela X11.
        render._conn.put_image(
            xproto::ImageFormat::Z_PIXMAP,
            render.window,
            render.gc,
            render.width as u16,
            render.height as u16,
            0,
            0,
            0,
            24,
            &render.buffer,
        )?;
        render._conn.flush()?;
        Ok(())
    }
}

impl Default for X11Scene {
    fn default() -> Self {
        Self::new()
    }
}