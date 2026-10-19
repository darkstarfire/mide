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
    utils::{Logical, Physical, Point, Transform},
    wayland::compositor::with_states,
};

use crate::{input::CursorShape, render::MideRenderElement, state::State};

/// Imagem de um cursor pronta para desenhar, com o ponto de clique (hotspot).
struct Image {
    buffer: MemoryRenderBuffer,
    hotspot: Point<i32, Logical>,
}

/// Cursores carregados do tema, por nome (`default`, `ew-resize`, `text`...).
pub struct Cursors {
    theme: xcursor::CursorTheme,
    size: u32,
    images: HashMap<String, Option<Image>>,
    fallback: Image,
}

impl Cursors {
    pub fn new() -> Self {
        let name = std::env::var("XCURSOR_THEME").unwrap_or_else(|_| "default".into());
        let size = std::env::var("XCURSOR_SIZE").ok().and_then(|s| s.parse().ok()).unwrap_or(24);
        Self { theme: xcursor::CursorTheme::load(&name), size, images: HashMap::new(), fallback: arrow() }
    }

    /// Elementos do cursor na posição do ponteiro (o primeiro da lista fica na frente).
    pub fn elements(&mut self, renderer: &mut GlesRenderer, state: &State) -> Vec<MideRenderElement> {
        let pointer = state.pointer_location.to_i32_round::<i32>();
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
                let location: Point<i32, Physical> = (pointer - hotspot).to_physical(1);
                return render_elements_from_surface_tree(renderer, surface, location, 1.0, 1.0, Kind::Cursor);
            }
            (CursorShape::Default, CursorImageStatus::Named(icon)) => icon.name().to_string(),
            (shape, _) => shape_name(shape).to_string(),
        };
        let image = self.image(&name);
        let location = (pointer - image.hotspot).to_f64().to_physical(1.0);
        match MemoryRenderBufferRenderElement::from_buffer(renderer, location, &image.buffer, None, None, None, Kind::Cursor) {
            Ok(element) => vec![MideRenderElement::Title(element)],
            Err(err) => {
                tracing::warn!("cursor não desenhado: {err}");
                Vec::new()
            }
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
    Some(Image { buffer, hotspot: (image.xhot as i32, image.yhot as i32).into() })
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
    Image { buffer, hotspot: (1, 1).into() }
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
