//! Texto com as fontes do sistema (cosmic-text), desenhado num pixmap do tiny-skia.

use std::collections::HashMap;

use cosmic_text::{Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, Style, SwashCache, Weight};
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

/// Família e estilo da fonte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Face {
    Sans,
    Mono,
    MonoItalic,
}

/// Composição e desenho de texto. Carregar as fontes é caro, então crie um só.
pub struct Text {
    fonts: FontSystem,
    swash: SwashCache,
    /// Larguras já medidas: medir é caro, e as interfaces medem os mesmos textos a
    /// cada quadro (por exemplo, ao redimensionar uma janela).
    widths: HashMap<(String, u32, FontWeight, bool), f32>,
    /// Textos já desenhados (imagem, deslocamento e largura): as interfaces desenham os
    /// mesmos rótulos a cada quadro, e colar a imagem pronta é muito mais rápido que
    /// compor as letras de novo.
    images: HashMap<(String, u32, FontWeight, [u8; 4], Face), (Pixmap, i32, i32, f32)>,
    /// Família da fonte de largura fixa (`None`: a monoespaçada padrão do sistema).
    mono_family: Option<String>,
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
            mono_family: None,
        }
    }

    fn buffer(&mut self, text: &str, size: f32, weight: FontWeight, face: Face) -> Buffer {
        let mut buffer = Buffer::new(&mut self.fonts, Metrics::new(size, size * 1.25));
        buffer.set_size(Some(100_000.0), Some(size * 1.5));
        let family = match (face, &self.mono_family) {
            (Face::Sans, _) => Family::SansSerif,
            (_, Some(name)) => Family::Name(name),
            (_, None) => Family::Monospace,
        };
        let style = if face == Face::MonoItalic { Style::Italic } else { Style::Normal };
        let attrs = Attrs::new().family(family).weight(weight.to_cosmic()).style(style);
        buffer.set_text(text, &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.fonts, false);
        buffer
    }

    /// Troca a família da fonte de largura fixa (terminal, editor). Uma família que
    /// não existe volta à padrão do sistema.
    pub fn set_mono_family(&mut self, name: Option<&str>) {
        let name = name.map(str::trim).filter(|n| !n.is_empty()).filter(|n| self.monospace_families().iter().any(|f| f == n)).map(str::to_string);
        if name != self.mono_family {
            self.mono_family = name;
            self.widths.clear();
            self.images.clear();
        }
    }

    /// Famílias de fonte de largura fixa instaladas, em ordem alfabética.
    pub fn monospace_families(&mut self) -> Vec<String> {
        let mut names: Vec<String> = self
            .fonts
            .db()
            .faces()
            .filter(|f| f.monospaced)
            .filter_map(|f| f.families.first().map(|(name, _)| name.clone()))
            .collect();
        names.sort_by_key(|n| n.to_lowercase());
        names.dedup();
        names
    }

    /// Largura do texto em pixels.
    pub fn measure(&mut self, text: &str, size: f32, weight: FontWeight) -> f32 {
        self.measure_with(text, size, weight, false)
    }

    /// Largura do texto em fonte de largura fixa (editor de texto).
    pub fn measure_mono(&mut self, text: &str, size: f32) -> f32 {
        self.measure_with(text, size, FontWeight::Regular, true)
    }

    fn measure_with(&mut self, text: &str, size: f32, weight: FontWeight, mono: bool) -> f32 {
        // Em telas com escala, mede no tamanho em pixels e devolve em pontos: a largura
        // bate com o texto desenhado (`draw_with`).
        let k = super::draw::scale();
        if k != 1.0 {
            super::draw::set_scale(1.0);
            let width = self.measure_with(text, size * k, weight, mono) / k;
            super::draw::set_scale(k);
            return width;
        }
        let key = (text.to_string(), size.to_bits(), weight, mono);
        if let Some(width) = self.widths.get(&key) {
            return *width;
        }
        let buffer = self.buffer(text, size, weight, if mono { Face::Mono } else { Face::Sans });
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
        self.draw_with(pixmap, text, (x, y), size, weight, rgba, Face::Sans)
    }

    /// Desenha em fonte de largura fixa (editor de texto).
    pub fn draw_mono(&mut self, pixmap: &mut Pixmap, text: &str, x: f32, y: f32, size: f32, rgba: [u8; 4]) -> f32 {
        self.draw_with(pixmap, text, (x, y), size, FontWeight::Regular, rgba, Face::Mono)
    }

    /// Fonte de largura fixa em negrito e/ou itálico (terminal).
    #[allow(clippy::too_many_arguments)]
    pub fn draw_mono_styled(&mut self, pixmap: &mut Pixmap, text: &str, x: f32, y: f32, size: f32, rgba: [u8; 4], bold: bool, italic: bool) -> f32 {
        let weight = if bold { FontWeight::Bold } else { FontWeight::Regular };
        let face = if italic { Face::MonoItalic } else { Face::Mono };
        self.draw_with(pixmap, text, (x, y), size, weight, rgba, face)
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_with(&mut self, pixmap: &mut Pixmap, text: &str, (x, y): (f32, f32), size: f32, weight: FontWeight, rgba: [u8; 4], face: Face) -> f32 {
        // Pontos para pixels (ver `draw::scale`): o texto é rasterizado no tamanho real.
        let k = super::draw::scale();
        if k != 1.0 {
            super::draw::set_scale(1.0);
            let width = self.draw_with(pixmap, text, (x * k, y * k), size * k, weight, rgba, face) / k;
            super::draw::set_scale(k);
            return width;
        }
        let key = (text.to_string(), size.to_bits(), weight, rgba, face);
        if !self.images.contains_key(&key) {
            let image = self.render(text, size, weight, rgba, face);
            if self.images.len() > 4096 {
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
    fn render(&mut self, text: &str, size: f32, weight: FontWeight, rgba: [u8; 4], face: Face) -> (Pixmap, i32, i32, f32) {
        let mut buffer = self.buffer(text, size, weight, face);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Bancada: `cargo test --lib bancada_texto -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn bancada_texto() {
        let mut text = Text::new();
        let mut pix = Pixmap::new(400, 40).unwrap();
        text.draw(&mut pix, "aquecer", 0.0, 0.0, 14.0, FontWeight::Regular, [0, 0, 0, 255]);
        let t = std::time::Instant::now();
        for i in 0..40 {
            let s = format!("Resultado número {i} — Configurações");
            text.measure(&s, 14.0, FontWeight::Regular);
            text.draw(&mut pix, &s, 0.0, 0.0, 14.0, FontWeight::Regular, [0, 0, 0, 255]);
        }
        println!("40 textos novos (medir + desenhar): {:.2} ms", t.elapsed().as_secs_f32() * 1e3);
    }
}
