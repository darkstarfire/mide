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
    Home,
    End,
    PageUp,
    PageDown,
}

/// Uma tecla apertada como chegou do teclado (para apps que tratam o teclado inteiro,
/// como o terminal): o símbolo da tecla (keysym do xkb), o texto que ela produz e os
/// modificadores de quando foi apertada.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyPress {
    pub keysym: u32,
    pub text: Option<String>,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    /// A tecla Super (⌘ no estilo Mac).
    pub logo: bool,
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
    /// Ctrl, Shift, Alt e Super (⌘) segurados.
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub logo: bool,
    /// Texto colado da área de transferência (Ctrl+V num campo), chegou neste quadro.
    pub paste: Option<String>,
    /// Rolagem vertical acumulada (positivo = para baixo), em pixels.
    pub scroll: f32,
    /// Texto digitado e teclas especiais.
    pub text: String,
    pub keys: Vec<Key>,
    /// Ctrl e Shift no momento de cada tecla de `keys` (o estado de `ctrl` e `shift`
    /// é o do fim do quadro: uma combinação rápida já pode ter soltado o Ctrl).
    pub key_mods: Vec<(bool, bool)>,
    /// Shift no momento de cada atalho de `shortcuts` (Ctrl+Shift+Z refaz).
    pub shortcut_shift: Vec<bool>,
    /// Atalhos com Ctrl neste quadro (a letra, em minúscula: "c", "v", "n"...).
    pub shortcuts: Vec<String>,
    /// Todas as teclas apertadas neste quadro, cruas e na ordem (ver [`KeyPress`]).
    pub presses: Vec<KeyPress>,
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
        self.key_mods.clear();
        self.shortcuts.clear();
        self.shortcut_shift.clear();
        self.presses.clear();
        self.paste = None;
    }

    /// Teclas deste quadro com o Ctrl e o Shift de quando cada uma foi apertada.
    pub fn key_events(&self) -> Vec<(Key, bool, bool)> {
        self.keys
            .iter()
            .enumerate()
            .map(|(i, k)| {
                let (ctrl, shift) = self.key_mods.get(i).copied().unwrap_or((self.ctrl, self.shift));
                (*k, ctrl, shift)
            })
            .collect()
    }

    /// A tecla foi apertada neste quadro?
    pub fn key(&self, key: Key) -> bool {
        self.keys.contains(&key)
    }

    /// Ctrl+`letra` neste quadro?
    pub fn shortcut(&self, letter: &str) -> bool {
        self.shortcuts.iter().any(|s| s == letter)
    }

    /// Ctrl+`letra` neste quadro, e se o Shift estava junto (`Some(true)` em Ctrl+Shift).
    pub fn shortcut_shift(&self, letter: &str) -> Option<bool> {
        let i = self.shortcuts.iter().position(|s| s == letter)?;
        Some(self.shortcut_shift.get(i).copied().unwrap_or(self.shift))
    }

    /// O ponteiro está dentro do retângulo (x, y, largura, altura)?
    pub fn over(&self, rect: (f32, f32, f32, f32)) -> bool {
        self.pointer.is_some_and(|(px, py)| px >= rect.0 && px <= rect.0 + rect.2 && py >= rect.1 && py <= rect.1 + rect.3)
    }
}
