//! Cursor desenhado pelo próprio compositor, quando não há um desktop anfitrião para
//! mostrar o dele (backend DRM).
//!
//! As imagens vêm do tema de cursores do sistema (`XCURSOR_THEME` e `XCURSOR_SIZE`,
//! padrão `default` e 24 px). Sem tema, o mide desenha uma seta própria. Quando o
//! app pede uma superfície como cursor, ela é desenhada no lugar.

use std::collections::HashMap;

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            element::{
                memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
                surface::render_elements_from_surface_tree,
                Kind,
            },
            gles::GlesRenderer,
        },
    },
    input::pointer::{CursorImageAttributes, CursorImageStatus},
    utils::{Logical, Physical, Point, Rectangle, Transform},
    wayland::compositor::with_states,
};

use crate::{input::CursorShape, render::MideRenderElement, state::State};

/// Imagem de um cursor pronta para desenhar, com o ponto de clique (hotspot) e os
/// pixels (RGBA pré-multiplicado, para o plano de cursor da placa).
struct Image {
    buffer: MemoryRenderBuffer,
    hotspot: Point<i32, Logical>,
    pixels: Vec<u8>,
    size: (u32, u32),
    /// Tamanho nominal no tema (24, 32, 48...).
    nominal: u32,
}

/// O cursor a mostrar agora (para o plano de cursor da placa, no DRM).
pub enum Plan<'a> {
    Hidden,
    /// Imagem do tema (ou a seta do mide): o nome identifica a imagem.
    Image { name: String, pixels: &'a [u8], size: (u32, u32), hotspot: (i32, i32) },
    /// O app desenha o próprio cursor (uma superfície): só a cena sabe desenhar.
    Surface,
}

/// Cursores carregados do tema, por nome (`default`, `ew-resize`, `text`...).
pub struct Cursors {
    theme: xcursor::CursorTheme,
    size: u32,
    images: HashMap<String, Option<Image>>,
    /// Cursor crescido (ponteiro agitado), por nome: a imagem de alta resolução do tema
    /// (o maior tamanho que ele tem) e a metade dela, feita dos mesmos pixels. O desenho
    /// nunca troca durante a animação: a placa só escala uma das duas.
    hidpi: HashMap<String, Option<(Image, Image)>>,
    fallback: Image,
}

impl Cursors {
    pub fn new() -> Self {
        let name = std::env::var("XCURSOR_THEME").unwrap_or_else(|_| "default".into());
        let size = std::env::var("XCURSOR_SIZE").ok().and_then(|s| s.parse().ok()).unwrap_or(24);
        let mut cursors = Self { theme: xcursor::CursorTheme::load(&name), size, images: HashMap::new(), hidpi: HashMap::new(), fallback: arrow() };
        // A seta grande já pronta: a primeira agitada não espera a leitura do tema.
        let _ = cursors.hidpi("default");
        cursors
    }

    /// Elementos do cursor na posição do ponteiro (o primeiro da lista fica na frente).
    /// Cursor na tela que começa em `origin` (posição dela entre as telas).
    pub fn elements(&mut self, renderer: &mut GlesRenderer, state: &State, origin: smithay::utils::Point<i32, smithay::utils::Logical>) -> Vec<MideRenderElement> {
        let pointer = state.pointer_location.to_i32_round::<i32>() - origin;
        // Bordas de redimensionar e outras formas do mide valem sobre o que o app pediu.
        let name = match (state.cursor, &state.cursor_status) {
            (CursorShape::Default, CursorImageStatus::Hidden) => return Vec::new(),
            (CursorShape::Default, CursorImageStatus::Surface(surface)) => {
                let hotspot = with_states(surface, |s| {
                    s.data_map
                        .get::<std::sync::Mutex<CursorImageAttributes>>()
                        .and_then(|a| a.lock().ok().map(|a| a.hotspot))
                })
                .unwrap_or_default();
                let location: Point<i32, Physical> = crate::clip::phys(pointer - hotspot);
                return render_elements_from_surface_tree(renderer, surface, location, crate::clip::scale(), 1.0, Kind::Cursor);
            }
            (CursorShape::Default, CursorImageStatus::Named(icon)) => icon.name().to_string(),
            (shape, _) => shape_name(shape).to_string(),
        };
        // Ponteiro agitado: o cursor grande. Uma imagem de alta resolução do tema (e a
        // metade dela, dos mesmos pixels, para reduzir sem serrilhado) escalada pela GPU
        // a cada quadro; tamanho e ponto quente vêm sempre da imagem grande, então trocar
        // a metade pela inteira não mexe na geometria, só na amostragem.
        let grow = state.shake.scale(std::time::Instant::now());
        let base = self.size;
        if grow > 1.001 {
            let target = base as f32 * grow;
            let (half, full) = self.hidpi(&name);
            let image = if target <= half.nominal as f32 { half } else { full };
            // Proporção em relação à imagem inteira: o ponto quente fica exato (a metade
            // tem o ponto quente arredondado).
            let k = target as f64 / full.nominal.max(1) as f64;
            let hotspot = Point::<f64, Logical>::from((full.hotspot.x as f64 * k, full.hotspot.y as f64 * k));
            let size = smithay::utils::Size::<i32, Logical>::from(((full.size.0 as f64 * k).round() as i32, (full.size.1 as f64 * k).round() as i32));
            let location = (pointer.to_f64() - hotspot).to_physical(crate::clip::scale());
            // `src` é a imagem inteira: sem ele, o Smithay usa o `size` como recorte da
            // imagem (o cursor aparecia cortado e "trocando de imagem" em vez de escalar).
            let src = Rectangle::<f64, Logical>::from_size((image.size.0 as f64, image.size.1 as f64).into());
            // Este caminho roda em todo quadro da animação. TRACE evita escrita de
            // disco por quadro quando o usuário habilita DEBUG para diagnosticar.
            tracing::trace!("cursor agitado: escala {grow:.3} imagem {} tamanho {}x{} canto {:.2},{:.2}", image.nominal, size.w, size.h, location.x, location.y);
            return match MemoryRenderBufferRenderElement::from_buffer(renderer, location, &image.buffer, None, Some(src), Some(size), Kind::Cursor) {
                Ok(element) => vec![MideRenderElement::Title(element)],
                Err(err) => {
                    tracing::warn!("cursor não desenhado: {err}");
                    Vec::new()
                }
            };
        }
        let image = self.image(&name);
        let location = (pointer.to_f64() - image.hotspot.to_f64()).to_physical(crate::clip::scale());
        match MemoryRenderBufferRenderElement::from_buffer(renderer, location, &image.buffer, None, None, None, Kind::Cursor) {
            Ok(element) => vec![MideRenderElement::Title(element)],
            Err(err) => {
                tracing::warn!("cursor não desenhado: {err}");
                Vec::new()
            }
        }
    }

    /// Qual cursor mostrar agora (o mesmo que `elements` desenharia).
    pub fn plan(&mut self, state: &State) -> Plan<'_> {
        // Cursor crescido (ponteiro agitado): maior que o plano da placa; a cena desenha.
        if state.shake.scale(std::time::Instant::now()) > 1.001 {
            return Plan::Surface;
        }
        let name = match (state.cursor, &state.cursor_status) {
            (CursorShape::Default, CursorImageStatus::Hidden) => return Plan::Hidden,
            (CursorShape::Default, CursorImageStatus::Surface(_)) => return Plan::Surface,
            (CursorShape::Default, CursorImageStatus::Named(icon)) => icon.name().to_string(),
            (shape, _) => shape_name(shape).to_string(),
        };
        let image = self.image(&name);
        Plan::Image { name, pixels: &image.pixels, size: image.size, hotspot: (image.hotspot.x, image.hotspot.y) }
    }

    /// O cursor grande: o maior tamanho do tema e a metade dele (para escalas até 2x,
    /// com menos serrilhado ao reduzir). Sem tema, a seta do mide.
    fn hidpi(&mut self, name: &str) -> (&Image, &Image) {
        if !self.hidpi.contains_key(name) {
            let loaded = names_for(name).iter().find_map(|n| load(&self.theme, n, u32::MAX)).map(|full| (half(&full), full));
            self.hidpi.insert(name.to_string(), loaded);
        }
        match self.hidpi.get(name).and_then(Option::as_ref) {
            Some((half, full)) => (half, full),
            None => (&self.fallback, &self.fallback),
        }
    }

    /// Imagem do tema pelo nome, com nomes antigos de reserva; a seta própria se faltar.
    fn image(&mut self, name: &str) -> &Image {
        if !self.images.contains_key(name) {
            let loaded = names_for(name).iter().find_map(|n| load(&self.theme, n, self.size));
            if loaded.is_none() {
                tracing::debug!("cursor '{name}' fora do tema: usando a seta do mide");
            }
            self.images.insert(name.to_string(), loaded);
        }
        self.images.get(name).and_then(Option::as_ref).unwrap_or(&self.fallback)
    }
}

/// Nome do cursor do tema para uma forma pedida pelo mide.
fn shape_name(shape: CursorShape) -> &'static str {
    match shape {
        CursorShape::Default => "default",
        CursorShape::ResizeHorizontal => "ew-resize",
        CursorShape::ResizeVertical => "ns-resize",
        CursorShape::ResizeNwse => "nwse-resize",
        CursorShape::ResizeNesw => "nesw-resize",
    }
}

/// Nome do cursor e os nomes antigos (X11) que os temas mais velhos usam.
fn names_for(name: &str) -> Vec<&str> {
    let legacy: &[&str] = match name {
        "default" => &["left_ptr", "arrow"],
        "ew-resize" => &["sb_h_double_arrow", "h_double_arrow"],
        "ns-resize" => &["sb_v_double_arrow", "v_double_arrow"],
        "nwse-resize" => &["bd_double_arrow", "size_fdiag"],
        "nesw-resize" => &["fd_double_arrow", "size_bdiag"],
        "pointer" => &["hand2", "hand1"],
        "text" => &["xterm"],
        // Arrasto que vai copiar/importar: seta com o "+" (nomes de temas antigos).
        "copy" => &["dnd-copy", "1081e37283d90000800003c07f3ef6bf", "6407b0e94181790501fd1e167b474872"],
        _ => &[],
    };
    std::iter::once(name).chain(legacy.iter().copied()).collect()
}

/// Lê o cursor do tema no tamanho mais próximo de `size` (primeiro quadro, se animado).
fn load(theme: &xcursor::CursorTheme, name: &str, size: u32) -> Option<Image> {
    let path = theme.load_icon(name)?;
    let bytes = std::fs::read(path).ok()?;
    let images = xcursor::parser::parse_xcursor(&bytes)?;
    let nearest = images.iter().map(|i| i.size).min_by_key(|s| s.abs_diff(size))?;
    let image = images.iter().find(|i| i.size == nearest)?;
    // Os pixels do Xcursor vêm em RGBA na memória, já pré-multiplicados.
    let buffer = MemoryRenderBuffer::from_slice(
        &image.pixels_rgba,
        Fourcc::Abgr8888,
        (image.width as i32, image.height as i32),
        1,
        Transform::Normal,
        None,
    );
    Some(Image { buffer, hotspot: (image.xhot as i32, image.yhot as i32).into(), pixels: image.pixels_rgba.clone(), size: (image.width, image.height), nominal: image.size })
}

/// Metade de uma imagem (média de cada 2x2 pixels, já pré-multiplicados): a mesma
/// imagem, menor, para a animação não trocar de desenho.
fn half(full: &Image) -> Image {
    let (w, h) = full.size;
    let (hw, hh) = ((w / 2).max(1), (h / 2).max(1));
    let mut pixels = vec![0u8; (hw * hh * 4) as usize];
    for y in 0..hh {
        for x in 0..hw {
            for c in 0..4 {
                let at = |xx: u32, yy: u32| full.pixels[((yy.min(h - 1) * w + xx.min(w - 1)) * 4 + c) as usize] as u32;
                let sum = at(2 * x, 2 * y) + at(2 * x + 1, 2 * y) + at(2 * x, 2 * y + 1) + at(2 * x + 1, 2 * y + 1);
                pixels[((y * hw + x) * 4 + c) as usize] = ((sum + 2) / 4) as u8;
            }
        }
    }
    let buffer = MemoryRenderBuffer::from_slice(&pixels, Fourcc::Abgr8888, (hw as i32, hh as i32), 1, Transform::Normal, None);
    Image { buffer, hotspot: (full.hotspot.x / 2, full.hotspot.y / 2).into(), pixels, size: (hw, hh), nominal: (full.nominal / 2).max(1) }
}

/// Seta do próprio mide (preta com contorno branco), para quando não há tema.
fn arrow() -> Image {
    const W: u32 = 16;
    const H: u32 = 24;
    let mut pixmap = tiny_skia::Pixmap::new(W, H).expect("tamanho da seta");
    let mut pb = tiny_skia::PathBuilder::new();
    pb.move_to(1.5, 1.5);
    pb.line_to(1.5, 19.0);
    pb.line_to(6.0, 15.0);
    pb.line_to(9.0, 22.0);
    pb.line_to(12.0, 20.5);
    pb.line_to(9.0, 14.0);
    pb.line_to(14.5, 14.0);
    pb.close();
    if let Some(path) = pb.finish() {
        let mut paint = tiny_skia::Paint { anti_alias: true, ..Default::default() };
        paint.set_color_rgba8(20, 20, 20, 255);
        pixmap.fill_path(&path, &paint, tiny_skia::FillRule::Winding, tiny_skia::Transform::identity(), None);
        paint.set_color_rgba8(255, 255, 255, 255);
        let stroke = tiny_skia::Stroke { width: 1.4, ..Default::default() };
        pixmap.stroke_path(&path, &paint, &stroke, tiny_skia::Transform::identity(), None);
    }
    let buffer = MemoryRenderBuffer::from_slice(pixmap.data(), Fourcc::Abgr8888, (W as i32, H as i32), 1, Transform::Normal, None);
    Image { buffer, hotspot: (1, 1).into(), pixels: pixmap.data().to_vec(), size: (W, H), nominal: H }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formas_do_mide_tem_nomes_do_tema_e_reservas() {
        assert_eq!(shape_name(CursorShape::ResizeHorizontal), "ew-resize");
        assert_eq!(names_for("default"), vec!["default", "left_ptr", "arrow"]);
        assert_eq!(names_for("wait"), vec!["wait"]);
    }
}
