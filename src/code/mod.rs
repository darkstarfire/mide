//! Núcleo do `mide-code` (o editor de texto e código do mide, inspirado no TextMate):
//! linguagens, cores, cliente LSP, preferências, a pasta do projeto, edições de linhas,
//! busca no projeto, divisão do editor e a sessão. Sem interface: testável sem janela;
//! o app fica em `src/bin/mide-code`.

pub mod config;
pub mod edit;
pub mod highlight;
pub mod lang;
pub mod lsp;
pub mod panes;
pub mod project;
pub mod search;
pub mod session;
