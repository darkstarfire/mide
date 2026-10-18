//! Texto com as fontes do sistema (cosmic-text), desenhado num pixmap do tiny-skia.

use std::collections::HashMap;

use cosmic_text::{Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, SwashCache, Weight};
use tiny_skia::Pixmap;

/// Peso da fonte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FontWeight {
    Regular,
    Semibold,
    Bold,
}

impl FontWeight {
    fn to_cosmic(self) -> Weight {
        match self {
            FontWeight::Regular => Weight::NORMAL,
            FontWeight::Semibold => Weight::SEMIBOLD,
            FontWeight::Bold => Weight::BOLD,
        }
    }
}

/// Composição e desenho de texto. Carregar as fontes é caro, então crie um só.
pub struct Text {
    fonts: FontSystem,
    swash: SwashCache,
    /// Larguras já medidas: medir é caro, e as interfaces medem os mesmos textos a
    /// cada quadro (por exemplo, ao redimensionar uma janela).
    widths: HashMap<(String, u32, FontWeight), f32>,
    /// Textos já desenhados (imagem, deslocamento e largura): as interfaces desenham os
    /// mesmos rótulos a cada quadro, e colar a imagem pronta é muito mais rápido que
    /// compor as letras de novo.
    images: HashMap<(String, u32, FontWeight, [u8; 4]), (Pixmap, i32, i32, f32)>,
}

impl Default for Text {
    fn default() -> Self {
        Self::new()
    }
}

impl Text {
    pub fn new() -> Self {
        Self {
            fonts: FontSystem::new(),
            swash: SwashCache::new(),
            widths: HashMap::new(),
            images: HashMap::new(),
        }
    }

    fn buffer(&mut self, text: &str, size: f32, weight: FontWeight) -> Buffer {
        let mut buffer = Buffer::new(&mut self.fonts, Metrics::new(size, size * 1.25));
        buffer.set_size(Some(10_000.0), Some(size * 1.5));
        let attrs = Attrs::new().family(Family::SansSerif).weight(weight.to_cosmic());
        buffer.set_text(text, &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.fonts, false);
        buffer
    }

    /// Largura do texto em pixels.
    pub fn measure(&mut self, text: &str, size: f32, weight: FontWeight) -> f32 {
        let key = (text.to_string(), size.to_bits(), weight);
        if let Some(width) = self.widths.get(&key) {
            return *width;
        }
        let buffer = self.buffer(text, size, weight);
        let width = buffer.layout_runs().map(|r| r.line_w).fold(0.0, f32::max);
        if self.widths.len() > 8192 {
            self.widths.clear();
        }
        self.widths.insert(key, width);
        width
    }

    /// Desenha `text` com o topo da linha em `(x, y)`. Devolve a largura desenhada.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        pixmap: &mut Pixmap,
        text: &str,
        x: f32,
        y: f32,
        size: f32,
        weight: FontWeight,
        rgba: [u8; 4],
    ) -> f32 {
        let key = (text.to_string(), size.to_bits(), weight, rgba);
        if !self.images.contains_key(&key) {
            let image = self.render(text, size, weight, rgba);
            if self.images.len() > 1024 {
                self.images.clear();
            }
            self.images.insert(key.clone(), image);
        }
        let (image, dx, dy, width) = &self.images[&key];
        crate::ui::draw::blit(pixmap, image, x.round() as i32 + dx, y.round() as i32 + dy, 1.0);
        *width
    }

    /// Desenha o texto numa imagem transparente só dele: a imagem, onde ela fica em
    /// relação ao ponto do texto e a largura do texto.
    fn render(&mut self, text: &str, size: f32, weight: FontWeight, rgba: [u8; 4]) -> (Pixmap, i32, i32, f32) {
        let mut buffer = self.buffer(text, size, weight);
        let width = buffer.layout_runs().map(|r| r.line_w).fold(0.0, f32::max);
        // Margem para letras que passam um pouco do retângulo (itálicos, acentos).
        let margin = (size * 0.5).ceil() as i32;
        let (pw, ph) = (width.ceil() as i32 + 2 * margin, (size * 1.5).ceil() as i32 + 2 * margin);
        let mut pixmap = Pixmap::new(pw.max(1) as u32, ph.max(1) as u32).unwrap_or_else(|| Pixmap::new(1, 1).unwrap());
        let color = Color::rgba(rgba[0], rgba[1], rgba[2], rgba[3]);
        let data = pixmap.data_mut();
        buffer.draw(&mut self.fonts, &mut self.swash, color, |gx, gy, w, h, c| {
            let a = c.a() as u32;
            if a == 0 {
                return;
            }
            for py in 0..h as i32 {
                for px in 0..w as i32 {
                    let (tx, ty) = (margin + gx + px, margin + gy + py);
                    if tx < 0 || ty < 0 || tx >= pw || ty >= ph {
                        continue;
                    }
                    let i = ((ty * pw + tx) * 4) as usize;
                    // Sobreposição premultiplicada.
                    for (k, ch) in [c.r(), c.g(), c.b()].into_iter().enumerate() {
                        let src = ch as u32 * a / 255;
                        let dst = data[i + k] as u32;
                        data[i + k] = (src + dst * (255 - a) / 255).min(255) as u8;
                    }
                    let dst_a = data[i + 3] as u32;
                    data[i + 3] = (a + dst_a * (255 - a) / 255).min(255) as u8;
                }
            }
        });
        (pixmap, -margin, -margin, width)
    }
}
