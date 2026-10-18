//! mide-ui: framework para criar apps com a interface nativa do mide.
//!
//! Um app implementa [`App`] descrevendo a tela no `view`, e roda com [`run`]:
//!
//! ```ignore
//! use mide::kit::{run, App, Ui, Window};
//!
//! #[derive(Default)]
//! struct Contador {
//!     cliques: u32,
//! }
//!
//! impl App for Contador {
//!     fn view(&mut self, ui: &mut Ui) {
//!         ui.page(|ui| {
//!             ui.title("Contador");
//!             ui.label(&format!("Você clicou {} vezes.", self.cliques));
//!             if ui.primary_button("Clicar").clicked() {
//!                 self.cliques += 1;
//!             }
//!         });
//!     }
//! }
//!
//! fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     run(Window::new("Contador").size(360, 240), Contador::default())
//! }
//! ```
//!
//! O guia completo, com todos os componentes, fica em `docs/mide-ui.md`.

mod app;
mod input;
mod theme;
mod ui;

pub use app::{run, App, Window};
pub use input::{Input, Key};
pub use theme::{Rgba, Theme};
pub use ui::{Menu, Response, Tile, Ui};
