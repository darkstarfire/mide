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
/// Texto das dicas: o `font_small` do tema do kit.
const TIP_FONT_SIZE: f32 = 12.0;
const TIP_LINE_HEIGHT: f32 = 15.0;
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
    images: HashMap<(String, bool, i32, bool, bool), Option<TitleImage>>,
}

/// Dica de um botão da barra de título (I2): qual botão de qual janela está sob o
/// ponteiro e desde quando. Aparece depois de [`TIP_DELAY`] parado; some ao sair do
/// botão ou ao clicar (`dismissed`).
pub struct Tip {
    pub window: smithay::desktop::Window,
    pub button: crate::decorations::Button,
    pub since: std::time::Instant,
    pub dismissed: bool,
}

/// Espera antes de a dica aparecer (a mesma do `ui.tooltip` do kit).
pub const TIP_DELAY: std::time::Duration = std::time::Duration::from_millis(800);

/// Texto da dica de cada botão, no idioma da sessão. O botão verde entra na tela
/// cheia (ou sai, na faixa revelada da tela cheia), como no Mac; com ⌥ (Alt)
/// apertado, maximiza ("Zoom"), e a dica diz isso.
pub fn tip_text(button: crate::decorations::Button, look: crate::decorations::BarLook) -> String {
    use crate::decorations::Button;
    match button {
        Button::Close => mide::i18n::trc("title bar", "Close").into_owned(),
        Button::Minimize => mide::i18n::trc("title bar", "Minimize").into_owned(),
        Button::Maximize if look.fullscreen => mide::i18n::trc("title bar", "Exit Full Screen").into_owned(),
        Button::Maximize if look.alt => mide::i18n::trc("title bar", "Zoom").into_owned(),
        Button::Maximize => mide::i18n::trc("title bar", "Enter Full Screen").into_owned(),
    }
}

/// Altura e raio da cápsula da dica (os do `ui.tooltip` do kit).
pub const TIP_HEIGHT: i32 = 22;

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
        self.image_styled(title, focused, max_width, false)
    }

    /// Imagem do texto de uma dica (fonte pequena e normal, cor do texto do tema).
    pub fn tip_image(&mut self, text: &str) -> Option<&TitleImage> {
        self.image_styled(text, true, 4000, true)
    }

    fn image_styled(&mut self, title: &str, focused: bool, max_width: i32, tip: bool) -> Option<&TitleImage> {
        let key = (title.to_string(), focused, max_width, crate::decorations::dark(), tip);
        if !self.images.contains_key(&key) {
            let image = self.render(title, focused, max_width, tip);
            self.images.insert(key.clone(), image);
        }
        self.images.get(&key).and_then(|i| i.as_ref())
    }

    fn attrs(tip: bool) -> Attrs<'static> {
        Attrs::new()
            .family(Family::SansSerif)
            .weight(if tip { Weight::NORMAL } else { Weight::SEMIBOLD })
    }

    fn metrics(tip: bool) -> Metrics {
        if tip {
            Metrics::new(TIP_FONT_SIZE, TIP_LINE_HEIGHT)
        } else {
            Metrics::new(FONT_SIZE, LINE_HEIGHT)
        }
    }

    /// Largura natural, em pixels, do texto com as fontes atuais.
    fn measure(&mut self, text: &str, tip: bool) -> i32 {
        let metrics = Self::metrics(tip);
        let mut buffer = Buffer::new(&mut self.fonts, metrics);
        buffer.set_size(Some(4000.0), Some(metrics.line_height));
        buffer.set_text(text, &Self::attrs(tip), Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.fonts, false);
        buffer
            .layout_runs()
            .map(|run| run.line_w)
            .fold(0.0f32, f32::max)
            .ceil() as i32
    }

    /// Versão do título que cabe em `max_width`, com reticências no fim se for cortada.
    fn fit(&mut self, title: &str, max_width: i32, tip: bool) -> String {
        if self.measure(title, tip) <= max_width {
            return title.to_string();
        }
        let mut chars: Vec<char> = title.chars().collect();
        while !chars.is_empty() {
            chars.pop();
            let candidate: String = chars.iter().collect::<String>().trim_end().to_string() + "…";
            if self.measure(&candidate, tip) <= max_width {
                return candidate;
            }
        }
        String::new()
    }

    fn render(&mut self, title: &str, focused: bool, max_width: i32, tip: bool) -> Option<TitleImage> {
        if title.trim().is_empty() {
            return None;
        }
        let fitted = self.fit(title.trim(), max_width.max(1), tip);
        if fitted.is_empty() {
            return None;
        }
        let metrics = Self::metrics(tip);
        let attrs = Self::attrs(tip);

        // Largura do texto já cortado; a segunda passada rasteriza nesse tamanho exato.
        let text_width = self.measure(&fitted, tip);
        if text_width <= 0 {
            return None;
        }
        let mut buffer = Buffer::new(&mut self.fonts, metrics);
        let width = text_width.max(1);
        let height = metrics.line_height.ceil() as i32;
        buffer.set_size(Some(width as f32), Some(height as f32));
        buffer.set_text(&fitted, &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.fonts, false);

        // Mac: cinza-escuro com foco, cinza-claro sem foco (no escuro, o contrário).
        let color = match (crate::decorations::dark(), focused) {
            // Dica: a cor do texto do tema do kit.
            (false, _) if tip => Color::rgba(24, 24, 28, 0xff),
            (true, _) if tip => Color::rgba(236, 236, 240, 0xff),
            (false, true) => Color::rgba(0x4d, 0x4d, 0x4d, 0xff),
            (false, false) => Color::rgba(0x9a, 0x9a, 0x9a, 0xff),
            (true, true) => Color::rgba(0xdc, 0xdc, 0xe0, 0xff),
            (true, false) => Color::rgba(0x80, 0x80, 0x86, 0xff),
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
