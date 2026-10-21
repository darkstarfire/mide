//! Título da janela desenhado na barra de título.
//!
//! O texto é composto com `cosmic-text` (fontes do sistema), rasterizado para
//! pixels RGBA premultiplicados e enviado à GPU como um buffer de memória. Cada
//! combinação de título e foco fica em cache, para não rasterizar a cada frame.

use std::collections::HashMap;

use cosmic_text::{Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, SwashCache, Weight};
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            element::{memory::MemoryRenderBuffer, Kind},
            gles::GlesRenderer,
        },
    },
    utils::{Logical, Point, Rectangle, Transform},
};

const FONT_SIZE: f32 = 13.0;
const LINE_HEIGHT: f32 = 16.0;
/// Espaço reservado nas duas pontas da barra (botões à esquerda, folga à direita).
pub const TITLE_SIDE_MARGIN: i32 = 70;

/// Texto já rasterizado, pronto para ser desenhado.
pub struct TitleImage {
    pub buffer: MemoryRenderBuffer,
    pub width: i32,
    pub height: i32,
}

/// Compõe e guarda em cache os títulos das janelas.
pub struct TitleRenderer {
    fonts: FontSystem,
    swash: SwashCache,
    images: HashMap<(String, bool, i32), Option<TitleImage>>,
}

impl TitleRenderer {
    pub fn new() -> Self {
        Self {
            fonts: FontSystem::new(),
            swash: SwashCache::new(),
            images: HashMap::new(),
        }
    }

    /// Imagem do título, cortada com reticências para caber em `max_width` pixels.
    /// Devolve `None` se o título estiver vazio.
    pub fn image(&mut self, title: &str, focused: bool, max_width: i32) -> Option<&TitleImage> {
        let key = (title.to_string(), focused, max_width);
        if !self.images.contains_key(&key) {
            let image = self.render(title, focused, max_width);
            self.images.insert(key.clone(), image);
        }
        self.images.get(&key).and_then(|i| i.as_ref())
    }

    fn attrs() -> Attrs<'static> {
        Attrs::new()
            .family(Family::SansSerif)
            .weight(Weight::SEMIBOLD)
    }

    /// Largura natural, em pixels, do texto com as fontes atuais.
    fn measure(&mut self, text: &str) -> i32 {
        let mut buffer = Buffer::new(&mut self.fonts, Metrics::new(FONT_SIZE, LINE_HEIGHT));
        buffer.set_size(Some(4000.0), Some(LINE_HEIGHT));
        buffer.set_text(text, &Self::attrs(), Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.fonts, false);
        buffer
            .layout_runs()
            .map(|run| run.line_w)
            .fold(0.0f32, f32::max)
            .ceil() as i32
    }

    /// Versão do título que cabe em `max_width`, com reticências no fim se for cortada.
    fn fit(&mut self, title: &str, max_width: i32) -> String {
        if self.measure(title) <= max_width {
            return title.to_string();
        }
        let mut chars: Vec<char> = title.chars().collect();
        while !chars.is_empty() {
            chars.pop();
            let candidate: String = chars.iter().collect::<String>().trim_end().to_string() + "…";
            if self.measure(&candidate) <= max_width {
                return candidate;
            }
        }
        String::new()
    }

    fn render(&mut self, title: &str, focused: bool, max_width: i32) -> Option<TitleImage> {
        if title.trim().is_empty() {
            return None;
        }
        let fitted = self.fit(title.trim(), max_width.max(1));
        if fitted.is_empty() {
            return None;
        }
        let metrics = Metrics::new(FONT_SIZE, LINE_HEIGHT);
        let attrs = Self::attrs();

        // Largura do texto já cortado; a segunda passada rasteriza nesse tamanho exato.
        let text_width = self.measure(&fitted);
        if text_width <= 0 {
            return None;
        }
        let mut buffer = Buffer::new(&mut self.fonts, metrics);
        let width = text_width.max(1);
        let height = LINE_HEIGHT.ceil() as i32;
        buffer.set_size(Some(width as f32), Some(height as f32));
        buffer.set_text(&fitted, &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.fonts, false);

        // Mac: cinza-escuro com foco, cinza-claro sem foco.
        let color = if focused {
            Color::rgba(0x4d, 0x4d, 0x4d, 0xff)
        } else {
            Color::rgba(0x9a, 0x9a, 0x9a, 0xff)
        };
        let mut pixels = vec![0u8; (width * height * 4) as usize];
        buffer.draw(&mut self.fonts, &mut self.swash, color, |x, y, w, h, c| {
            let a = c.a() as u32;
            for py in 0..h as i32 {
                for px in 0..w as i32 {
                    let (gx, gy) = (x + px, y + py);
                    if gx < 0 || gy < 0 || gx >= width || gy >= height {
                        continue;
                    }
                    let i = ((gy * width + gx) * 4) as usize;
                    // Sobreposição premultiplicada: o texto por cima do que já existe.
                    let channels = [c.r(), c.g(), c.b()];
                    for (k, channel) in channels.into_iter().enumerate() {
                        let src_p = channel as u32 * a / 255;
                        let dst = pixels[i + k] as u32;
                        pixels[i + k] = (src_p + dst * (255 - a) / 255).min(255) as u8;
                    }
                    let dst_a = pixels[i + 3] as u32;
                    pixels[i + 3] = (a + dst_a * (255 - a) / 255).min(255) as u8;
                }
            }
        });

        // Memória RGBA (R, G, B, A) corresponde a ABGR8888 em little-endian.
        let buffer = MemoryRenderBuffer::from_slice(&pixels, Fourcc::Abgr8888, (width, height), 1, Transform::Normal, None);
        Some(TitleImage {
            buffer,
            width,
            height,
        })
    }
}

/// Elemento de desenho do título, centralizado em `bar` (retângulo da barra).
pub fn element(
    renderer: &mut GlesRenderer,
    titles: &mut TitleRenderer,
    title: &str,
    focused: bool,
    bar: Rectangle<i32, Logical>,
    alpha: f32,
) -> Option<smithay::backend::renderer::element::memory::MemoryRenderBufferRenderElement<GlesRenderer>> {
    use smithay::backend::renderer::element::memory::MemoryRenderBufferRenderElement;
    let max_width = bar.size.w - 2 * TITLE_SIDE_MARGIN;
    let image = titles.image(title, focused, max_width)?;
    let x = bar.loc.x + (bar.size.w - image.width) / 2;
    let y = bar.loc.y + (bar.size.h - image.height) / 2;
    let location: Point<f64, smithay::utils::Physical> = crate::clip::phys((x as i32, y as i32).into()).to_f64();
    MemoryRenderBufferRenderElement::from_buffer(
        renderer,
        location,
        &image.buffer,
        (alpha < 1.0).then_some(alpha),
        None,
        None,
        Kind::Unspecified,
    )
    .ok()
}
