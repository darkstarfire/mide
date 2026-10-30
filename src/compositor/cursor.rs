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
    /// Cursor crescido já reduzido no processador, por nome e tamanho em pixels: vai ao
    /// plano de cursor da placa como qualquer outra imagem, sem a cena desenhar quadros.
    grown: HashMap<(String, u32), Grown>,
    fallback: Image,
}

/// Uma etapa do cursor crescido (RGBA pré-multiplicado).
struct Grown {
    pixels: Vec<u8>,
    size: (u32, u32),
    hotspot: (i32, i32),
}

/// Tamanho do ponteiro da configuração (pontos), para o `XCURSOR_SIZE` dos apps que
/// o mide abre; 0 antes de a configuração ser lida.
static SESSION_SIZE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Guarda o tamanho do ponteiro da configuração (a cada leitura dela).
pub fn remember_size(size: u32) {
    SESSION_SIZE.store(size, std::sync::atomic::Ordering::Relaxed);
}

/// Tamanho do ponteiro para os apps (`XCURSOR_SIZE`), se a configuração já foi lida.
pub fn session_size() -> Option<u32> {
    Some(SESSION_SIZE.load(std::sync::atomic::Ordering::Relaxed)).filter(|s| *s > 0)
}

impl Cursors {
    /// Tamanho do ponteiro escolhido nas Configurações: mudou, as imagens são lidas de
    /// novo no tamanho novo (o crescido do cursor agitado parte dele também).
    fn sync_size(&mut self, state: &State) {
        let size = state.config.input.cursor_size();
        if size != self.size {
            tracing::info!("cursor: tamanho {size}");
            self.size = size;
            self.images.clear();
            self.grown.clear();
        }
    }

    pub fn new() -> Self {
        let name = std::env::var("XCURSOR_THEME").unwrap_or_else(|_| "default".into());
        let size = std::env::var("XCURSOR_SIZE").ok().and_then(|s| s.parse().ok()).unwrap_or(24);
        let mut cursors = Self { theme: xcursor::CursorTheme::load(&name), size, images: HashMap::new(), hidpi: HashMap::new(), grown: HashMap::new(), fallback: arrow() };
        // A seta grande já pronta: a primeira agitada não espera a leitura do tema.
        let _ = cursors.hidpi("default");
        cursors
    }

    /// Elementos do cursor na posição do ponteiro (o primeiro da lista fica na frente).
    /// Cursor na tela que começa em `origin` (posição dela entre as telas).
    pub fn elements(&mut self, renderer: &mut GlesRenderer, state: &State, origin: smithay::utils::Point<i32, smithay::utils::Logical>) -> Vec<MideRenderElement> {
        self.sync_size(state);
        let pointer = state.pointer_location.to_i32_round::<i32>() - origin;
        let grow = state.shake.scale(std::time::Instant::now());
        // Bordas de redimensionar e outras formas do mide valem sobre o que o app pediu.
        let name = match (state.cursor, &state.cursor_status) {
            (CursorShape::Default, CursorImageStatus::Hidden) => return Vec::new(),
            // Ponteiro agitado sobre um app com cursor próprio: cresce a seta do tema.
            (CursorShape::Default, CursorImageStatus::Surface(_)) if grow > 1.001 => "default".to_string(),
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
        // a metade pela inteira não mexe na geometria, só na amostragem. (No DRM este
        // caminho só vale quando o plano da placa é pequeno demais: ver `plan`.)
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
    ///
    /// Ponteiro agitado: o cursor crescido também vai ao plano da placa, numa imagem
    /// reduzida aqui para cada tamanho inteiro em pixels. Antes ele saía do plano e a
    /// cena inteira era composta em GLES a cada vblank da animação, o que travava a tela
    /// em placas Intel (Chromebook) até suspender e voltar.
    pub fn plan(&mut self, state: &State, scale: f64) -> Plan<'_> {
        self.sync_size(state);
        let grow = state.shake.scale(std::time::Instant::now());
        let name = match (state.cursor, &state.cursor_status) {
            (CursorShape::Default, CursorImageStatus::Hidden) => return Plan::Hidden,
            (CursorShape::Default, CursorImageStatus::Surface(_)) if grow > 1.001 => "default".to_string(),
            (CursorShape::Default, CursorImageStatus::Surface(_)) => return Plan::Surface,
            (CursorShape::Default, CursorImageStatus::Named(icon)) => icon.name().to_string(),
            (shape, _) => shape_name(shape).to_string(),
        };
        // Em pixels da tela: o tamanho escolhido (pontos) × a escala da tela × o
        // crescimento do agito. O nome mandado à placa leva o tamanho: com o mesmo nome,
        // ela continuava com a imagem antiga ao mudar o tamanho nas Configurações.
        let px = (self.size as f64 * scale.max(1.0) * grow as f64).round().max(1.0) as u32;
        // Agito acabado: as etapas dele não servem mais (a próxima agitada refaz as dela).
        if grow <= 1.001 {
            self.grown.retain(|(_, size), _| *size == px);
        }
        let natural = self.image(&name).nominal;
        if px == natural {
            let image = self.image(&name);
            return Plan::Image { name: format!("{name}@{px}"), pixels: &image.pixels, size: image.size, hotspot: (image.hotspot.x, image.hotspot.y) };
        }
        let key = (name, px);
        if !self.grown.contains_key(&key) {
            let (_, full) = self.hidpi(&key.0);
            let grown = scaled(full, px);
            self.grown.insert(key.clone(), grown);
        }
        let image = &self.grown[&key];
        Plan::Image { name: format!("{}@{}", key.0, key.1), pixels: &image.pixels, size: image.size, hotspot: image.hotspot }
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

/// A imagem grande do tema com `px` pixels de tamanho nominal (o tamanho do cursor
/// normal vezes a escala da agitada). Cada pixel de saída é a média de amostras em grade
/// interpoladas na origem: reduz sem serrilhado e amplia suave, como a GPU fazia. O ponto
/// quente segue a mesma proporção.
fn scaled(full: &Image, px: u32) -> Grown {
    let k = px as f64 / full.nominal.max(1) as f64;
    let (sw, sh) = full.size;
    let w = ((sw as f64 * k).round() as u32).max(1);
    let h = ((sh as f64 * k).round() as u32).max(1);
    // Amostras por eixo: o bastante para cobrir cada pixel de origem ao reduzir.
    let n = (1.0 / k).ceil().clamp(1.0, 8.0) as u32;
    let mut pixels = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let mut sum = [0u32; 4];
            for j in 0..n {
                for i in 0..n {
                    // Ponto da amostra na origem, interpolado entre os 4 vizinhos.
                    let fx = ((x as f64 + (i as f64 + 0.5) / n as f64) / k - 0.5).clamp(0.0, (sw - 1) as f64);
                    let fy = ((y as f64 + (j as f64 + 0.5) / n as f64) / k - 0.5).clamp(0.0, (sh - 1) as f64);
                    let (x0, y0) = (fx as u32, fy as u32);
                    let (x1, y1) = ((x0 + 1).min(sw - 1), (y0 + 1).min(sh - 1));
                    let (tx, ty) = (fx - x0 as f64, fy - y0 as f64);
                    let at = |xx: u32, yy: u32, c: usize| full.pixels[((yy * sw + xx) * 4) as usize + c] as f64;
                    for (c, total) in sum.iter_mut().enumerate() {
                        let top = at(x0, y0, c) * (1.0 - tx) + at(x1, y0, c) * tx;
                        let bottom = at(x0, y1, c) * (1.0 - tx) + at(x1, y1, c) * tx;
                        *total += (top * (1.0 - ty) + bottom * ty).round() as u32;
                    }
                }
            }
            let d = ((y * w + x) * 4) as usize;
            for (c, total) in sum.iter().enumerate() {
                pixels[d + c] = ((total + n * n / 2) / (n * n)) as u8;
            }
        }
    }
    let hotspot = ((full.hotspot.x as f64 * k).round() as i32, (full.hotspot.y as f64 * k).round() as i32);
    Grown { pixels, size: (w, h), hotspot }
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

    #[test]
    fn cursor_crescido_escala_tamanho_e_ponto_quente() {
        let seta = arrow();
        let grande = scaled(&seta, seta.nominal * 3);
        assert_eq!(grande.size, (seta.size.0 * 3, seta.size.1 * 3));
        assert_eq!(grande.hotspot, (3, 3));
        let pequeno = scaled(&seta, seta.nominal / 2);
        assert_eq!(pequeno.size, (seta.size.0 / 2, seta.size.1 / 2));
        // Pixel opaco do miolo continua opaco depois de ampliar.
        assert!(grande.pixels.chunks(4).any(|p| p[3] == 255));
    }
}
