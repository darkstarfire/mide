//! Núcleo do `mide-code` (o editor de texto e código do mide, inspirado no TextMate):
//! linguagens, cores, cliente LSP, preferências e a pasta do projeto. Sem interface:
//! testável sem janela; o app fica em `src/bin/mide-code`.

pub mod config;
pub mod highlight;
pub mod lang;
pub mod lsp;
pub mod project;
