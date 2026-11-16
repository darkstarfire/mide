//! Apps instalados e como abrir coisas: catálogo de `.desktop`, ícones do tema, apps padrão, abrir arquivos e programas, AppImage, catálogo da Loja (AppStream).

pub mod programs;
pub mod appimage;
pub mod xpm;
pub mod appstream;

// No framework mide-ui (github.com/fabioaacarneiro/mide-ui), repassados aqui.
pub use mide_ui::applications::{apps, default_apps, icons};
