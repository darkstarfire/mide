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
    /// O Alt de quando cada atalho foi apertado (Ctrl+Alt+letra).
    pub shortcut_alt: Vec<bool>,
    /// Atalhos com Ctrl neste quadro (a letra, em minúscula: "c", "v", "n"...).
    pub shortcuts: Vec<String>,
    /// Todas as teclas apertadas neste quadro, cruas e na ordem (ver [`KeyPress`]).
    pub presses: Vec<KeyPress>,
    /// Itens escolhidos nos menus do app na barra superior (a ação de cada um).
    pub menu: Vec<String>,
    /// A janela está com o foco do teclado.
    pub focused: bool,
    /// O compositor marcou a janela como ativa (`activated` do xdg-shell); `None` em
    /// camadas. Vale mais que o teclado para a aparência: um menu ou painel que pega o
    /// teclado não deixa a janela com cara de inativa.
    pub active: Option<bool>,
    /// Alguma tecla deste quadro veio da repetição de uma tecla segurada.
    pub repeat: bool,
    /// Um arrasto com arquivos (de outro app ou desta janela) está sobre a janela; o
    /// ponteiro dele fica em `pointer`.
    pub drop_hover: bool,
    /// Arquivos soltos na janela neste quadro (caminhos), onde o ponteiro está.
    pub dropped: Option<Vec<std::path::PathBuf>>,
    /// O arrasto que esta janela começou terminou neste quadro (solto em algum lugar
    /// ou cancelado).
    pub drag_ended: bool,
}

impl Input {
    /// A janela tem cara de ativa (luzes coloridas, seleção na cor de destaque).
    pub fn window_active(&self) -> bool {
        self.active.unwrap_or(self.focused)
    }

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
        self.shortcut_alt.clear();
        self.presses.clear();
        self.menu.clear();
        self.paste = None;
        self.repeat = false;
        self.dropped = None;
        self.drag_ended = false;
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
    /// Ctrl+Alt+letra apertado neste quadro (com o Alt de quando a tecla foi apertada).
    pub fn shortcut_alt(&self, letter: &str) -> bool {
        self.shortcuts.iter().enumerate().any(|(i, s)| s == letter && self.shortcut_alt.get(i).copied().unwrap_or(self.alt))
    }

    /// Ctrl+letra sem Alt.
    pub fn shortcut_plain(&self, letter: &str) -> bool {
        self.shortcuts.iter().enumerate().any(|(i, s)| s == letter && !self.shortcut_alt.get(i).copied().unwrap_or(self.alt))
    }

    pub fn shortcut_shift(&self, letter: &str) -> Option<bool> {
        let i = self.shortcuts.iter().position(|s| s == letter)?;
        Some(self.shortcut_shift.get(i).copied().unwrap_or(self.shift))
    }

    /// O ponteiro está dentro do retângulo (x, y, largura, altura)?
    pub fn over(&self, rect: (f32, f32, f32, f32)) -> bool {
        self.pointer.is_some_and(|(px, py)| px >= rect.0 && px <= rect.0 + rect.2 && py >= rect.1 && py <= rect.1 + rect.3)
    }
}

/// Caminhos de uma lista `text/uri-list` (só `file://`; comentários e outros
/// endereços ficam de fora), com os `%XX` decodificados.
pub fn parse_uri_list(text: &str) -> Vec<std::path::PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.strip_prefix("file://"))
        .map(|rest| rest.find('/').map_or("", |i| &rest[i..]))
        .filter(|p| !p.is_empty())
        .map(|p| {
            let b = p.as_bytes();
            let mut out = Vec::with_capacity(b.len());
            let mut i = 0;
            while i < b.len() {
                // `%XX` (pelos bytes: o texto pode ter acentos logo depois do `%`).
                let hex = (b[i] == b'%' && i + 2 < b.len())
                    .then(|| std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()))
                    .flatten();
                if let Some(v) = hex {
                    out.push(v);
                    i += 3;
                    continue;
                }
                out.push(b[i]);
                i += 1;
            }
            std::path::PathBuf::from(std::ffi::OsString::from_vec(out))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn lista_de_enderecos() {
        let paths = super::parse_uri_list("# comentário\r\nfile:///home/a/Meu%20Arquivo.txt\r\nhttps://x.com/\r\nfile://host/tmp/b\n");
        assert_eq!(paths, vec![std::path::PathBuf::from("/home/a/Meu Arquivo.txt"), std::path::PathBuf::from("/tmp/b")]);
    }
}
