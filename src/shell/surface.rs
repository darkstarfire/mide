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
