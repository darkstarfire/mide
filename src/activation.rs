//! Protocolos que os apps usam para conversar com o compositor sobre foco e escala:
//!
//! - `xdg-activation`: um app pede para trazer uma janela para a frente (o navegador
//!   abrindo um link vindo de outro app, uma notificação clicada, o xwayland-satellite
//!   passando o foco de uma janela X11). A janela sobe, volta se estava minimizada e
//!   ganha o teclado, se o pedido for recente (até 10 s depois do token).
//! - `wp_fractional_scale`: cada janela e camada sabe a escala exata da tela onde está
//!   (1,25, 1,5...) e pode desenhar nítida nessa escala, em vez de ser esticada.

use smithay::{
    delegate_fractional_scale, delegate_xdg_activation,
    desktop::layer_map_for_output,
    output::Output,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    wayland::{
        compositor::with_states,
        fractional_scale::{with_fractional_scale, FractionalScaleHandler},
        xdg_activation::{XdgActivationHandler, XdgActivationState, XdgActivationToken, XdgActivationTokenData},
    },
};

use crate::state::State;

impl XdgActivationHandler for State {
    fn activation_state(&mut self) -> &mut XdgActivationState {
        &mut self.xdg_activation_state
    }

    fn request_activation(&mut self, _token: XdgActivationToken, token_data: XdgActivationTokenData, surface: WlSurface) {
        if token_data.timestamp.elapsed().as_secs() >= 10 {
            tracing::debug!("ativação recusada: pedido antigo");
            return;
        }
        // A janela da superfície, aberta ou minimizada.
        let id = self
            .space
            .elements()
            .chain(self.minimized.iter().map(|m| &m.window))
            .find(|w| w.toplevel().is_some_and(|t| t.wl_surface() == &surface))
            .map(crate::state::window_id);
        if let Some(id) = id {
            tracing::info!("ativação: a janela {id} veio para a frente");
            crate::control::focus_window_id(self, id);
        }
    }
}
delegate_xdg_activation!(State);

impl FractionalScaleHandler for State {
    fn new_fractional_scale(&mut self, surface: WlSurface) {
        // Até saber em que tela a superfície está, a escala da tela principal.
        let scale = self.primary_output().map(|o| o.current_scale().fractional_scale()).unwrap_or(1.0);
        with_states(&surface, |states| with_fractional_scale(states, |fs| fs.set_preferred_scale(scale)));
    }
}
delegate_fractional_scale!(State);

/// Escala desta tela para as janelas e camadas que estão nela (só manda quando muda).
pub fn update_scales(state: &State, output: &Output) {
    let scale = output.current_scale().fractional_scale();
    // `with_surfaces` já entrega o estado de cada superfície (pedir de novo com
    // `with_states` travaria: a trava do estado já está tomada).
    let set = |_: &WlSurface, states: &smithay::wayland::compositor::SurfaceData| {
        with_fractional_scale(states, |fs| fs.set_preferred_scale(scale));
    };
    for window in state.space.elements() {
        if state.space.outputs_for_element(window).first() == Some(output) {
            window.with_surfaces(set);
        }
    }
    for layer in layer_map_for_output(output).layers() {
        layer.with_surfaces(set);
    }
    if let Some(lock) = state.lock.surface_for(output) {
        with_states(lock.wl_surface(), |states| with_fractional_scale(states, |fs| fs.set_preferred_scale(scale)));
    }
}
