//! # mide-ui
//!
//! O framework de apps nativos do mide (Mac Inspired Desktop Environment): janelas,
//! componentes no visual do macOS Tahoe, tema claro/escuro e cor de destaque da sessão,
//! menus na barra superior, área de transferência, arrastar e soltar, seletor de
//! arquivos, vidro e telas, sobre Wayland.
//!
//! Comece por [`kit`]: um app implementa [`kit::App`] e chama [`kit::run`] com uma
//! [`kit::Window`]. O guia completo está em `docs/mide-ui.md` no repositório do mide.
//!
//! ```no_run
//! use mide_ui::kit::{run, App, Ui, Window};
//!
//! struct Hello { clicks: u32 }
//!
//! impl App for Hello {
//!     fn view(&mut self, ui: &mut Ui) {
//!         ui.title("Olá");
//!         if ui.button("Clique").clicked() {
//!             self.clicks += 1;
//!         }
//!         ui.label(&format!("{} clique(s)", self.clicks));
//!     }
//! }
//!
//! fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     run(Window::new("Olá").app_id("hello").size(420, 300).full_size_content(), Hello { clicks: 0 })
//! }
//! ```
//!
//! Organização: `kit` (componentes, janela, eventos, tema), `ui` (desenho, texto e
//! ícones de baixo nível) e os auxiliares do desktop que o kit usa, também úteis aos
//! apps: `platform` (configuração, canal do compositor, sons), `shell` (menus na barra,
//! durações das animações), `applications` (apps instalados, ícones, apps padrão) e
//! `storage` (arquivos, lugares, miniaturas). Todos também ficam na raiz
//! (`mide_ui::places`).

pub mod applications;
pub mod kit;
pub mod platform;
pub mod shell;
pub mod storage;
pub mod ui;

pub use applications::{apps, default_apps, icons};
pub use platform::{control_client, settings_file, sounds};
pub use shell::{appmenu, motion};
pub use storage::{files, places, thumbnails};
