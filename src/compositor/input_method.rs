//! Teclado na tela: os protocolos de método de entrada.
//!
//! - `zwp_text_input_v3`: o app avisa quando um campo de texto ganha o foco (GTK, Qt,
//!   Chrome, Firefox e os apps do mide-ui).
//! - `zwp_input_method_v2`: o teclado na tela do mide (`mide-keyboard`) é o método de
//!   entrada; recebe "ativar"/"desativar" junto com o campo e manda o texto pronto ao
//!   app (`commit_string`, acentos inclusive, sem depender do layout do teclado).
//!
//! O Smithay liga o campo ao foco do teclado sozinho. As teclas que não são texto
//! (Enter, Backspace, setas, Tab, Esc) chegam pelo canal de controle (`osk key NOME`) e
//! entram como teclas de verdade, com o layout do sistema.

use smithay::{
    delegate_input_method_manager, delegate_text_input_manager,
    reexports::wayland_server::{protocol::wl_surface::WlSurface, DisplayHandle},
    utils::{Logical, Rectangle},
    wayland::{
        input_method::{InputMethodHandler, InputMethodManagerState, PopupSurface},
        text_input::TextInputManagerState,
    },
};

use crate::state::State;

/// Cria os globais do texto e do método de entrada.
pub fn init(display: &DisplayHandle) {
    TextInputManagerState::new::<State>(display);
    InputMethodManagerState::new::<State, _>(display, |_| true);
}

impl InputMethodHandler for State {
    // Os popups de candidatos (métodos de entrada de chinês e japonês) ainda não são
    // desenhados: o teclado do mide não usa.
    fn new_popup(&mut self, _surface: PopupSurface) {
        tracing::debug!("método de entrada: popup ignorado");
    }

    fn dismiss_popup(&mut self, _surface: PopupSurface) {}

    fn popup_repositioned(&mut self, _surface: PopupSurface) {}

    fn parent_geometry(&self, parent: &WlSurface) -> Rectangle<i32, Logical> {
        self.space
            .elements()
            .find(|w| w.toplevel().is_some_and(|t| t.wl_surface() == parent))
            .and_then(|w| self.space.element_geometry(w))
            .unwrap_or_default()
    }
}

delegate_text_input_manager!(State);
delegate_input_method_manager!(State);

/// Tecla especial pedida pelo teclado na tela, pelo nome (`enter`, `backspace`,
/// `left`...), apertada e solta como uma tecla de verdade.
pub fn press_key(state: &mut State, name: &str) -> bool {
    let Some(code) = crate::input::evdev_code(name) else {
        return false;
    };
    let time = state.start_time.elapsed().as_millis() as u32;
    crate::input::evdev_key(state, code, true, time);
    crate::input::evdev_key(state, code, false, time);
    true
}
