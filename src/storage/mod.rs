//! Arquivos e pastas: operações, informações, visualização lembrada, lugares (Lixeira, pastas do usuário) e miniaturas.

pub mod files_info;
pub mod files_view;

// No framework mide-ui (github.com/fabioaacarneiro/mide-ui), repassados aqui.
pub use mide_ui::storage::{files, places, thumbnails};
