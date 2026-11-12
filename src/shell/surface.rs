//! Superfície dos programas da shell (barra, dock, teclado na tela): uma camada
//! Wayland (`wlr-layer-shell`) ou uma janela de camada do X11, com a mesma interface
//! para quadro, ponteiro e tamanho.
//!
//! O programa cria a superfície do jeito do backend dele (a criação no Wayland precisa
//! dos globais e da fila do programa) e guarda o resultado num `Surface`. Daí em
//! diante, o desenho e o ponteiro passam por aqui.
//!
//! No X11, a superfície é uma janela do tamanho do que ela desenha (uma barra, um
//! menu, uma dica), no lugar da tela inteira: sem canal alfa, uma tela inteira
//! transparente taparia o desktop. O quadro do programa é sempre em coordenadas de
//! tela; `present` recorta a parte da janela, e `to_local` converte o ponteiro de
//! volta para essas coordenadas, então a lógica do programa não muda entre os dois.

use mide_ui::kit::x11_layer::{Layer, PointerKind, Rect};
use smithay_client_toolkit::{
    reexports::client::protocol::{wl_shm, wl_surface},
    shell::{wlr_layer::LayerSurface, WaylandSurface},
    shm::slot::SlotPool,
};
use tiny_skia::{Pixmap, PixmapPaint, Transform};

use crate::shell::scale::SurfaceScale;
use crate::ui::draw::to_argb8888;

pub enum Surface {
    Wayland {
        layer: LayerSurface,
        pool: SlotPool,
        scale: SurfaceScale,
    },
    X11 {
        layer: Layer,
        /// Onde a janela fica na tela (coordenadas de tela, em pixels).
        rect: Rect,
        /// O canvas está em coordenadas de tela (menus, a barra) e `present` recorta a
        /// parte da janela; falso, o canvas já é da janela (dicas).
        screen_canvas: bool,
    },
}

impl Surface {
    /// A superfície Wayland é esta? (para casar com `PointerEvent::surface`).
    pub fn is_wayland(&self, surface: &wl_surface::WlSurface) -> bool {
        match self {
            Surface::Wayland { layer, .. } => layer.wl_surface() == surface,
            Surface::X11 { .. } => false,
        }
    }

    /// Id da janela X11 da superfície (`None` no Wayland).
    pub fn x11_window(&self) -> Option<u32> {
        match self {
            Surface::X11 { layer, .. } => Some(layer.window()),
            Surface::Wayland { .. } => None,
        }
    }

    /// A janela X11 é esta? (para casar com o `window` do evento).
    pub fn is_x11(&self, window: u32) -> bool {
        match self {
            Surface::X11 { layer, .. } => layer.window() == window,
            Surface::Wayland { .. } => false,
        }
    }

    /// Fator de escala da superfície (1 no X11: lá o ponto e o pixel são o mesmo).
    pub fn scale_factor(&self) -> f32 {
        match self {
            Surface::Wayland { scale, .. } => scale.scale(),
            Surface::X11 { .. } => 1.0,
        }
    }

    /// Tamanho em pixels de uma área de `w` × `h` pontos.
    pub fn pixels(&self, w: u32, h: u32) -> (u32, u32) {
        match self {
            Surface::Wayland { scale, .. } => scale.pixels(w, h),
            Surface::X11 { .. } => (w, h),
        }
    }

    /// O `LayerSurface` (só no Wayland), para os ajustes de âncora e margem.
    pub fn layer_surface(&self) -> Option<&LayerSurface> {
        match self {
            Surface::Wayland { layer, .. } => Some(layer),
            Surface::X11 { .. } => None,
        }
    }

    /// Confirma as mudanças pendentes da camada (no X11 não há o que confirmar).
    pub fn commit(&self) {
        if let Surface::Wayland { layer, .. } = self {
            layer.commit();
        }
    }

    /// Zona exclusiva da camada (reserva da borda). No X11 ela vem do `strut` da criação.
    pub fn set_exclusive_zone(&self, zone: i32) {
        if let Surface::Wayland { layer, .. } = self {
            layer.set_exclusive_zone(zone);
        }
    }

    /// Tamanho da janela X11 (em pixels); `None` no Wayland.
    pub fn x11_size(&self) -> Option<(u32, u32)> {
        match self {
            Surface::X11 { rect, .. } => Some((rect.w, rect.h)),
            Surface::Wayland { .. } => None,
        }
    }

    /// Canto de cima à esquerda da janela na tela (X11); (0, 0) no Wayland.
    pub fn origin(&self) -> (f64, f64) {
        match self {
            Surface::X11 { rect, .. } => (rect.x as f64, rect.y as f64),
            Surface::Wayland { .. } => (0.0, 0.0),
        }
    }

    /// Envia um quadro transparente de `size` (a faixa que só recebe ponteiro no Wayland).
    pub fn present_empty(&mut self, size: (u32, u32)) -> Result<(), String> {
        let blank = Pixmap::new(size.0.max(1), size.1.max(1)).ok_or("quadro de tamanho zero")?;
        self.present(&blank, size, [0, 0, 0])
    }

    /// O ponteiro do X11 chega relativo à janela: aqui vira coordenada de tela (o
    /// que o programa usa). No Wayland, já vem em coordenadas da superfície.
    pub fn to_local(&self, x: f64, y: f64) -> (f64, f64) {
        match self {
            Surface::X11 { rect, screen_canvas: true, .. } => (x + rect.x as f64, y + rect.y as f64),
            Surface::X11 { .. } => (x, y),
            Surface::Wayland { .. } => (x, y),
        }
    }

    /// Envia o quadro. `canvas` está em coordenadas de tela (no Wayland, da superfície);
    /// `points` é o tamanho lógico (em pontos) para a escala do Wayland.
    pub fn present(&mut self, canvas: &Pixmap, points: (u32, u32), background: [u8; 3]) -> Result<(), String> {
        match self {
            Surface::Wayland { layer, pool, scale } => {
                let (w, h) = (canvas.width() as i32, canvas.height() as i32);
                let (buffer, bytes) = pool.create_buffer(w, h, w * 4, wl_shm::Format::Argb8888).map_err(|e| e.to_string())?;
                to_argb8888(canvas, bytes);
                scale.set_destination(points.0, points.1);
                let surface = layer.wl_surface();
                surface.damage_buffer(0, 0, w, h);
                buffer.attach_to(surface).map_err(|e| e.to_string())?;
                layer.commit();
                Ok(())
            }
            Surface::X11 { layer, rect, screen_canvas } => {
                let (w, h) = (rect.w.max(1), rect.h.max(1));
                let mut out = Pixmap::new(w, h).ok_or("quadro de tamanho zero")?;
                // Com o canvas em coordenadas de tela, recorta a parte que a janela cobre;
                // com o canvas da própria janela, ele vai direto (a cor de fundo compõe as
                // partes transparentes, em `present` do Layer).
                let (dx, dy) = if *screen_canvas { (-rect.x, -rect.y) } else { (0, 0) };
                out.draw_pixmap(dx, dy, canvas.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
                layer.present(&out, background).map_err(|e| e.to_string())
            }
        }
    }

    /// Como `present`, e no X11 recorta a janela pelo alfa do quadro (cantos arredondados
    /// e sombra fora da forma saem; sem compositor a janela é retangular e o resto sairia
    /// preto). Só para superfícies de tamanho certo, como menus e dicas.
    pub fn present_shaped(&mut self, canvas: &Pixmap, points: (u32, u32), background: [u8; 3], min_alpha: u8) -> Result<(), String> {
        match self {
            Surface::X11 { layer, rect, screen_canvas } => {
                let (w, h) = (rect.w.max(1), rect.h.max(1));
                let mut out = Pixmap::new(w, h).ok_or("quadro de tamanho zero")?;
                let (dx, dy) = if *screen_canvas { (-rect.x, -rect.y) } else { (0, 0) };
                out.draw_pixmap(dx, dy, canvas.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
                layer.present(&out, background).map_err(|e| e.to_string())?;
                shape_from_alpha(layer.window(), &out, min_alpha);
                Ok(())
            }
            Surface::Wayland { .. } => self.present(canvas, points, background),
        }
    }

    /// Muda o tamanho da superfície (em pontos). No X11 também move a janela para `rect`.
    pub fn resize(&mut self, points: (u32, u32), rect: Option<Rect>) -> Result<(), String> {
        match self {
            Surface::Wayland { layer, .. } => {
                layer.set_size(points.0, points.1);
                layer.commit();
                Ok(())
            }
            Surface::X11 { layer, rect: current, .. } => {
                let target = rect.unwrap_or(Rect { x: current.x, y: current.y, w: points.0, h: points.1 });
                *current = target;
                layer.set_rect(target).map_err(|e| e.to_string())
            }
        }
    }

    /// Área que recebe o ponteiro (no X11; o resto deixa passar). Só o X11 tem isso: no
    /// Wayland a região é ajustada pelo programa com o `wl_region`.
    pub fn set_input_region(&mut self, region: Option<Rect>) -> Result<(), String> {
        match self {
            Surface::X11 { layer, .. } => layer.set_input_region(region).map_err(|e| e.to_string()),
            Surface::Wayland { .. } => Ok(()),
        }
    }

    /// Forma visível da janela (X11): só a parte `region` aparece. No Wayland, a camada
    /// já é transparente, então não faz nada.
    pub fn set_bounding_region(&mut self, region: Option<Rect>) -> Result<(), String> {
        match self {
            Surface::X11 { layer, .. } => layer.set_bounding_region(region).map_err(|e| e.to_string()),
            Surface::Wayland { .. } => Ok(()),
        }
    }

    /// Prende o ponteiro (menus no X11). No Wayland o menu já recebe tudo (é tela cheia).
    pub fn grab_pointer(&self) -> bool {
        match self {
            Surface::X11 { layer, .. } => layer.grab_pointer(),
            Surface::Wayland { .. } => true,
        }
    }

    pub fn ungrab_pointer(&self) {
        if let Surface::X11 { layer, .. } = self {
            layer.ungrab_pointer();
        }
    }
}

/// Tipo do evento de ponteiro, igual no Wayland e no X11: os programas da shell tratam
/// os dois do mesmo jeito.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PtrKind {
    Enter {},
    Leave {},
    Motion {},
    Press { button: u32 },
    Release { button: u32 },
    Other,
}

/// Botão do X11 para o código de botão do evento Linux (1 esquerdo, 2 meio, 3 direito).
/// Rolagem (4 a 7) e outros botões ficam de fora (`None`).
pub fn x11_button(button: u32) -> Option<u32> {
    match button {
        1 => Some(0x110),
        2 => Some(0x112),
        3 => Some(0x111),
        _ => None,
    }
}

/// Tipo do ponteiro do X11 já no formato dos programas da shell.
pub fn ptr_from_x11(kind: PointerKind) -> PtrKind {
    match kind {
        PointerKind::Motion => PtrKind::Motion {},
        PointerKind::Enter => PtrKind::Enter {},
        PointerKind::Leave => PtrKind::Leave {},
        PointerKind::Press(b) => x11_button(b as u32).map_or(PtrKind::Other, |button| PtrKind::Press { button }),
        PointerKind::Release(b) => x11_button(b as u32).map_or(PtrKind::Other, |button| PtrKind::Release { button }),
    }
}

/// Descritor do X para o laço de eventos (calloop) acordar quando o servidor manda algo.
pub struct XFd(pub std::os::unix::io::RawFd);

impl std::os::fd::AsFd for XFd {
    fn as_fd(&self) -> std::os::fd::BorrowedFd<'_> {
        // SAFETY: a conexão X (dona do descritor) vive enquanto o laço roda.
        unsafe { std::os::fd::BorrowedFd::borrow_raw(self.0) }
    }
}

/// Recorte SHAPE da janela X11 com os retângulos dados (coordenadas da janela). Vale
/// para qualquer janela X11 do programa; devolve se o X aceitou.
pub fn set_x11_shape(window: u32, rects: &[x11rb::protocol::xproto::Rectangle]) -> bool {
    use std::sync::OnceLock;
    use x11rb::connection::{Connection as _, RequestConnection as _};
    use x11rb::protocol::shape::{ConnectionExt as _, SK, SO};
    use x11rb::protocol::xproto::ClipOrdering;
    use x11rb::rust_connection::RustConnection;
    static CONN: OnceLock<Option<RustConnection>> = OnceLock::new();
    let Some(conn) = CONN.get_or_init(|| RustConnection::connect(None).ok().map(|(c, _)| c)).as_ref() else {
        return false;
    };
    let _ = conn.prefetch_extension_information("SHAPE");
    if !matches!(conn.extension_information("SHAPE"), Ok(Some(_))) {
        return false;
    }
    // Uma requisição tem tamanho máximo: o primeiro bloco troca o recorte, os seguintes
    // somam a ele (UNION).
    let mut ok = true;
    if rects.is_empty() {
        ok = conn.shape_rectangles(SO::SET, SK::BOUNDING, ClipOrdering::UNSORTED, window, 0, 0, &[]).is_ok();
    }
    for (i, chunk) in rects.chunks(8000).enumerate() {
        let op = if i == 0 { SO::SET } else { SO::UNION };
        ok &= conn.shape_rectangles(op, SK::BOUNDING, ClipOrdering::UNSORTED, window, 0, 0, chunk).is_ok();
    }
    let _ = conn.flush();
    ok
}

/// Recorte da janela pelo alfa do quadro: o que tem alfa de `min_alpha` para cima fica
/// visível, o resto sai. Sem compositor no X11 a janela é retangular, e as bordas
/// semitransparentes seriam misturadas com preto (halo escuro): por isso o limite alto
/// nas ampliações, onde as bordas são só dos ícones.
pub fn shape_from_alpha(window: u32, pixmap: &tiny_skia::Pixmap, min_alpha: u8) {
    let (w, h) = (pixmap.width() as usize, pixmap.height() as usize);
    let data = pixmap.data();
    let mut rects = Vec::new();
    for y in 0..h {
        let row = &data[y * w * 4..(y + 1) * w * 4];
        let mut x = 0;
        while x < w {
            if row[x * 4 + 3] < min_alpha {
                x += 1;
                continue;
            }
            let start = x;
            while x < w && row[x * 4 + 3] >= min_alpha {
                x += 1;
            }
            rects.push(x11rb::protocol::xproto::Rectangle { x: start as i16, y: y as i16, width: (x - start) as u16, height: 1 });
        }
    }
    set_x11_shape(window, &rects);
}

