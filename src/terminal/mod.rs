//! Emulador de terminal do mide, escrito do zero, com o desenho em camadas que o
//! iTerm2 usa (referência de pesquisa; nenhum código dele é copiado):
//!
//! - [`parser`]: bytes do programa → texto, controles e sequências de escape;
//! - [`screen`]: o estado (grade, histórico com reflow, cursor, modos, cores);
//! - [`keys`]: teclas e texto colado → bytes para o programa;
//! - [`pty`]: o pseudoterminal e o processo;
//! - [`width`] e [`palette`]: largura dos caracteres e temas de cores;
//! - [`layout`] e [`profile`]: painéis de uma aba e perfis (tema, fonte, cursor);
//! - [`session`]: abas e painéis guardados para reabrir a janela como estava;
//! - [`integration`]: scripts do bash e do zsh que marcam prompts e comandos.
//!
//! A janela (`mide-terminal`) junta as partes e desenha as células com o mide-ui.

pub mod integration;
pub mod keys;
pub mod layout;
pub mod palette;
pub mod parser;
pub mod profile;
pub mod pty;
pub mod screen;
pub mod session;
pub mod width;

pub use screen::Terminal;
