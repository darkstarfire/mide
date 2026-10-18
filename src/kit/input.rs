//! Entrada de um quadro: ponteiro, botões, rolagem e teclas, já traduzidos para o
//! que os componentes precisam.

/// Tecla especial que os componentes tratam (o resto chega como texto).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Backspace,
    Enter,
    Escape,
    Tab,
    Left,
    Right,
    Up,
    Down,
    Delete,
}

/// Tudo o que aconteceu desde o último quadro.
#[derive(Debug, Clone, Default)]
pub struct Input {
    /// Posição do ponteiro na janela (`None` fora dela).
    pub pointer: Option<(f32, f32)>,
    /// Botão esquerdo apertado neste quadro, solto neste quadro e segurado agora.
    pub pressed: bool,
    pub released: bool,
    pub down: bool,
    /// Botão direito apertado neste quadro.
    pub secondary: bool,
    /// Ctrl segurado.
    pub ctrl: bool,
    /// Rolagem vertical acumulada (positivo = para baixo), em pixels.
    pub scroll: f32,
    /// Texto digitado e teclas especiais.
    pub text: String,
    pub keys: Vec<Key>,
    /// Atalhos com Ctrl neste quadro (a letra, em minúscula: "c", "v", "n"...).
    pub shortcuts: Vec<String>,
}

impl Input {
    /// Limpa o que vale só para um quadro (o estado do botão e a posição ficam).
    pub fn end_frame(&mut self) {
        self.pressed = false;
        self.released = false;
        self.secondary = false;
        self.scroll = 0.0;
        self.text.clear();
        self.keys.clear();
        self.shortcuts.clear();
    }

    /// A tecla foi apertada neste quadro?
    pub fn key(&self, key: Key) -> bool {
        self.keys.contains(&key)
    }

    /// Ctrl+`letra` neste quadro?
    pub fn shortcut(&self, letter: &str) -> bool {
        self.shortcuts.iter().any(|s| s == letter)
    }

    /// O ponteiro está dentro do retângulo (x, y, largura, altura)?
    pub fn over(&self, rect: (f32, f32, f32, f32)) -> bool {
        self.pointer.is_some_and(|(px, py)| px >= rect.0 && px <= rect.0 + rect.2 && py >= rect.1 && py <= rect.1 + rect.3)
    }
}
