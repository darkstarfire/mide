//! Tema do mide-ui: cores, cantos e tamanhos num lugar só. Os componentes leem tudo
//! daqui, então mudar o tema muda todos os apps feitos com o mide-ui.

use tiny_skia::Color;

/// Cor em RGBA (0 a 255).
pub type Rgba = [u8; 4];

/// Aparência dos componentes.
#[derive(Debug, Clone)]
pub struct Theme {
    /// Fundo da janela.
    pub background: Rgba,
    /// Fundo da barra lateral.
    pub sidebar: Rgba,
    /// Fundo dos cartões.
    pub card: Rgba,
    /// Linhas finas (bordas e separadores).
    pub line: Rgba,
    /// Texto principal e secundário.
    pub text: Rgba,
    pub text_secondary: Rgba,
    /// Nome embaixo dos ladrilhos (`tile`). Claro (sobre o papel de parede), ganha uma
    /// sombra para ser lido em qualquer fundo.
    pub tile_text: Rgba,
    /// Cor de destaque (botão principal, interruptor ligado, seleção).
    pub accent: Rgba,
    /// Texto sobre a cor de destaque.
    pub on_accent: Rgba,
    /// Fundo de botões comuns e de itens sob o ponteiro.
    pub control: Rgba,
    pub hover: Rgba,
    /// Cantos dos cartões e dos controles.
    pub radius: f32,
    pub control_radius: f32,
    /// Tamanhos de fonte.
    pub font: f32,
    pub font_small: f32,
    pub font_title: f32,
    /// Altura de uma linha de controle (botão, campo, item de lista).
    pub row: f32,
    /// Espaço entre itens empilhados e margem das bordas.
    pub spacing: f32,
    pub padding: f32,
}

impl Default for Theme {
    /// Tema claro do mide, o mesmo das Configurações do Sistema.
    fn default() -> Self {
        Self {
            background: [246, 246, 248, 255],
            sidebar: [232, 232, 236, 255],
            card: [255, 255, 255, 255],
            line: [0, 0, 0, 24],
            text: [24, 24, 28, 255],
            text_secondary: [130, 130, 138, 255],
            tile_text: [24, 24, 28, 255],
            accent: [10, 110, 235, 255],
            on_accent: [255, 255, 255, 255],
            control: [0, 0, 0, 16],
            hover: [0, 0, 0, 28],
            radius: 10.0,
            control_radius: 7.0,
            font: 13.5,
            font_small: 12.0,
            font_title: 20.0,
            row: 32.0,
            spacing: 10.0,
            padding: 24.0,
        }
    }
}

/// Converte para a cor do tiny-skia.
pub fn color(c: Rgba) -> Color {
    Color::from_rgba8(c[0], c[1], c[2], c[3])
}
