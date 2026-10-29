//! Peças do shell (barra, dock, launcher, busca, notificações): menus dos apps, ícones de status, calendário, fundo por trás das camadas, durações das animações, organização do launcher e busca.

pub mod notify;
pub mod tray;
pub mod gmenu;
pub mod xmenu;
pub mod calendar;
pub mod backdrop;
pub mod launcher_layout;
pub mod search;
pub mod scale;

// No framework mide-ui (github.com/fabioaacarneiro/mide-ui), repassados aqui.
pub use mide_ui::shell::{appmenu, motion};
