//! Escala da tela para os clientes de camada que desenham por conta própria (barra,
//! dock, launcher, notificações), pelo `wp_fractional_scale` e o `wp_viewporter`.
//!
//! Sem isto, eles desenham em escala 1 e o compositor estica a imagem em telas com
//! 125%/150%/200%: letras e ícones borrados (e o texto parece espaçado). Com isto, o
//! cliente desenha a imagem na resolução real (tamanho em pontos × escala, com
//! `mide::ui::draw::set_scale` para as funções de desenho e de texto) e o viewport diz
//! ao compositor o tamanho dela em pontos.
//!
//! Uso: `SurfaceScale::new(&globals, &qh, &surface)` ao criar a superfície; o estado
//! do app implementa `ScaleHandler` (redesenhar quando a escala muda) e chama
//! `delegate_surface_scale!(App)`. A cada quadro: `scale()` para o tamanho da imagem e
//! `set_destination(w, h)` em pontos antes do commit.

use std::sync::{
    atomic::{AtomicU32, Ordering},
    Arc,
};

use smithay_client_toolkit::reexports::{
    client::{
        globals::GlobalList,
        protocol::wl_surface::WlSurface,
        Connection, Dispatch, Proxy, QueueHandle,
    },
    protocols::wp::{
        fractional_scale::v1::client::{
            wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1,
            wp_fractional_scale_v1::{self, WpFractionalScaleV1},
        },
        viewporter::client::{wp_viewport::WpViewport, wp_viewporter::WpViewporter},
    },
};

/// Quem recebe o aviso de que a escala de uma superfície mudou (para redesenhar).
pub trait ScaleHandler {
    fn surface_scale_changed(&mut self, qh: &QueueHandle<Self>, surface: &WlSurface)
    where
        Self: Sized;
}

/// Dados do objeto de escala: a superfície e a escala atual em 120 avos.
#[derive(Debug, Clone)]
pub struct ScaleData {
    surface: WlSurface,
    value: Arc<AtomicU32>,
}

/// Escala e viewport de uma superfície.
#[derive(Debug)]
pub struct SurfaceScale {
    viewport: Option<WpViewport>,
    fractional: Option<WpFractionalScaleV1>,
    value: Arc<AtomicU32>,
}

impl SurfaceScale {
    /// Pede a escala da superfície ao compositor. Sem os protocolos, fica em 1.
    pub fn new<D>(globals: &GlobalList, qh: &QueueHandle<D>, surface: &WlSurface) -> Self
    where
        D: Dispatch<WpViewporter, ()>
            + Dispatch<WpViewport, ()>
            + Dispatch<WpFractionalScaleManagerV1, ()>
            + Dispatch<WpFractionalScaleV1, ScaleData>
            + 'static,
    {
        let value = Arc::new(AtomicU32::new(120));
        let viewporter = globals.bind::<WpViewporter, _, _>(qh, 1..=1, ()).ok();
        let manager = globals.bind::<WpFractionalScaleManagerV1, _, _>(qh, 1..=1, ()).ok();
        let (viewport, fractional) = match (viewporter, manager) {
            (Some(viewporter), Some(manager)) => {
                let data = ScaleData { surface: surface.clone(), value: value.clone() };
                (Some(viewporter.get_viewport(surface, qh, ())), Some(manager.get_fractional_scale(surface, qh, data)))
            }
            _ => (None, None),
        };
        Self { viewport, fractional, value }
    }

    /// Pixels por ponto (1, 1,25, 1,5, 2...). Sem viewport, sempre 1.
    pub fn scale(&self) -> f32 {
        if self.viewport.is_none() {
            return 1.0;
        }
        self.value.load(Ordering::Relaxed) as f32 / 120.0
    }

    /// Tamanho da imagem em pixels para `w` × `h` pontos.
    pub fn pixels(&self, w: u32, h: u32) -> (u32, u32) {
        let s = self.scale();
        (((w as f32) * s).round().max(1.0) as u32, ((h as f32) * s).round().max(1.0) as u32)
    }

    /// Tamanho da superfície em pontos (a imagem pode ter mais pixels). Chamar antes
    /// do commit de cada imagem; sem viewport não faz nada (a imagem é em pontos).
    pub fn set_destination(&self, w: u32, h: u32) {
        if let Some(viewport) = &self.viewport {
            viewport.set_destination(w.max(1) as i32, h.max(1) as i32);
        }
    }
}

impl Drop for SurfaceScale {
    fn drop(&mut self) {
        if let Some(fractional) = self.fractional.take() {
            fractional.destroy();
        }
        if let Some(viewport) = self.viewport.take() {
            viewport.destroy();
        }
    }
}

/// Despacho dos eventos de escala, para `delegate_surface_scale!`.
pub struct ScaleDispatch;

impl<D> Dispatch<WpFractionalScaleV1, ScaleData, D> for ScaleDispatch
where
    D: Dispatch<WpFractionalScaleV1, ScaleData> + ScaleHandler,
{
    fn event(
        state: &mut D,
        _: &WpFractionalScaleV1,
        event: wp_fractional_scale_v1::Event,
        data: &ScaleData,
        _: &Connection,
        qh: &QueueHandle<D>,
    ) {
        if let wp_fractional_scale_v1::Event::PreferredScale { scale } = event
            && data.value.swap(scale, Ordering::Relaxed) != scale
            && data.surface.is_alive()
        {
            state.surface_scale_changed(qh, &data.surface);
        }
    }
}

impl<D> Dispatch<WpViewporter, (), D> for ScaleDispatch
where
    D: Dispatch<WpViewporter, ()>,
{
    fn event(_: &mut D, _: &WpViewporter, _: <WpViewporter as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<D>) {}
}

impl<D> Dispatch<WpViewport, (), D> for ScaleDispatch
where
    D: Dispatch<WpViewport, ()>,
{
    fn event(_: &mut D, _: &WpViewport, _: <WpViewport as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<D>) {}
}

impl<D> Dispatch<WpFractionalScaleManagerV1, (), D> for ScaleDispatch
where
    D: Dispatch<WpFractionalScaleManagerV1, ()>,
{
    fn event(
        _: &mut D,
        _: &WpFractionalScaleManagerV1,
        _: <WpFractionalScaleManagerV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<D>,
    ) {
    }
}

/// Liga os eventos de escala ao estado do app (que implementa `ScaleHandler`).
#[macro_export]
macro_rules! delegate_surface_scale {
    ($ty:ty) => {
        smithay_client_toolkit::reexports::client::delegate_dispatch!($ty: [
            smithay_client_toolkit::reexports::protocols::wp::viewporter::client::wp_viewporter::WpViewporter: ()
        ] => $crate::shell::scale::ScaleDispatch);
        smithay_client_toolkit::reexports::client::delegate_dispatch!($ty: [
            smithay_client_toolkit::reexports::protocols::wp::viewporter::client::wp_viewport::WpViewport: ()
        ] => $crate::shell::scale::ScaleDispatch);
        smithay_client_toolkit::reexports::client::delegate_dispatch!($ty: [
            smithay_client_toolkit::reexports::protocols::wp::fractional_scale::v1::client::wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1: ()
        ] => $crate::shell::scale::ScaleDispatch);
        smithay_client_toolkit::reexports::client::delegate_dispatch!($ty: [
            smithay_client_toolkit::reexports::protocols::wp::fractional_scale::v1::client::wp_fractional_scale_v1::WpFractionalScaleV1: $crate::shell::scale::ScaleData
        ] => $crate::shell::scale::ScaleDispatch);
    };
}
