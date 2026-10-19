//! Edição de texto de uma linha: cursor, seleção, apagar, pular palavras, colar.
//!
//! Lógica pura, sem desenho nem entrada: o campo de texto do mide-ui (e, depois, o
//! editor de texto) traduz teclas e cliques em chamadas daqui. Posições são índices
//! de byte no texto, sempre em limites de caractere.

/// Cursor e seleção de um campo. A seleção vai de `anchor` até `cursor` (em qualquer
/// ordem); sem `anchor`, não há seleção.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Edit {
    pub cursor: usize,
    pub anchor: Option<usize>,
}

impl Edit {
    /// Cursor no fim do texto, sem seleção.
    pub fn at_end(text: &str) -> Self {
        Self { cursor: text.len(), anchor: None }
    }

    /// Texto inteiro selecionado (digitar substitui).
    pub fn all(text: &str) -> Self {
        Self { cursor: text.len(), anchor: Some(0) }
    }

    /// Trecho selecionado (início, fim), se não estiver vazio.
    pub fn selection(&self) -> Option<(usize, usize)> {
        let anchor = self.anchor?;
        (anchor != self.cursor).then(|| (anchor.min(self.cursor), anchor.max(self.cursor)))
    }

    /// Texto selecionado.
    pub fn selected<'a>(&self, text: &'a str) -> Option<&'a str> {
        self.selection().map(|(a, b)| &text[a..b])
    }

    /// Mantém o cursor dentro do texto e em limite de caractere (o texto pode ter
    /// mudado por fora, pelo app).
    pub fn clamp(&mut self, text: &str) {
        self.cursor = floor_char(text, self.cursor.min(text.len()));
        self.anchor = self.anchor.map(|a| floor_char(text, a.min(text.len())));
    }

    /// Escreve `s` no cursor, no lugar da seleção, se houver.
    pub fn insert(&mut self, text: &mut String, s: &str) {
        self.delete_selection(text);
        text.insert_str(self.cursor, s);
        self.cursor += s.len();
    }

    /// Apaga a seleção; devolve se havia algo selecionado.
    pub fn delete_selection(&mut self, text: &mut String) -> bool {
        let Some((a, b)) = self.selection() else {
            self.anchor = None;
            return false;
        };
        text.replace_range(a..b, "");
        self.cursor = a;
        self.anchor = None;
        true
    }

    /// Backspace: a seleção, ou o caractere (com `word`, a palavra) antes do cursor.
    pub fn backspace(&mut self, text: &mut String, word: bool) -> bool {
        if self.delete_selection(text) {
            return true;
        }
        let start = if word { word_left(text, self.cursor) } else { prev_char(text, self.cursor) };
        if start == self.cursor {
            return false;
        }
        text.replace_range(start..self.cursor, "");
        self.cursor = start;
        true
    }

    /// Delete: a seleção, ou o caractere (com `word`, a palavra) depois do cursor.
    pub fn delete(&mut self, text: &mut String, word: bool) -> bool {
        if self.delete_selection(text) {
            return true;
        }
        let end = if word { word_right(text, self.cursor) } else { next_char(text, self.cursor) };
        if end == self.cursor {
            return false;
        }
        text.replace_range(self.cursor..end, "");
        true
    }

    /// Move o cursor para `to`. Com `select`, estende a seleção; sem, desfaz.
    pub fn move_to(&mut self, to: usize, select: bool) {
        if select {
            self.anchor.get_or_insert(self.cursor);
        } else {
            self.anchor = None;
        }
        self.cursor = to;
    }

    /// Seta para a esquerda: sem Shift e com seleção, vai para o começo dela.
    pub fn left(&mut self, text: &str, word: bool, select: bool) {
        if let (false, Some((a, _))) = (select, self.selection()) {
            return self.move_to(a, false);
        }
        let to = if word { word_left(text, self.cursor) } else { prev_char(text, self.cursor) };
        self.move_to(to, select);
    }

    /// Seta para a direita: sem Shift e com seleção, vai para o fim dela.
    pub fn right(&mut self, text: &str, word: bool, select: bool) {
        if let (false, Some((_, b))) = (select, self.selection()) {
            return self.move_to(b, false);
        }
        let to = if word { word_right(text, self.cursor) } else { next_char(text, self.cursor) };
        self.move_to(to, select);
    }

    /// Seleciona a palavra em volta de `at` (duplo clique).
    pub fn select_word(&mut self, text: &str, at: usize) {
        let at = floor_char(text, at.min(text.len()));
        let start = text[..at].char_indices().rev().take_while(|(_, c)| is_word(*c)).last().map_or(at, |(i, _)| i);
        let end = text[at..].char_indices().find(|(_, c)| !is_word(*c)).map_or(text.len(), |(i, _)| at + i);
        self.anchor = Some(start);
        self.cursor = end;
    }
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Maior limite de caractere em `i` ou antes.
fn floor_char(text: &str, mut i: usize) -> usize {
    while i > 0 && !text.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn prev_char(text: &str, i: usize) -> usize {
    text[..i].char_indices().next_back().map_or(0, |(p, _)| p)
}

fn next_char(text: &str, i: usize) -> usize {
    text[i..].chars().next().map_or(i, |c| i + c.len_utf8())
}

/// Começo da palavra à esquerda (pula espaços e pontuação antes).
fn word_left(text: &str, i: usize) -> usize {
    let mut pos = i;
    let mut in_word = false;
    for (p, c) in text[..i].char_indices().rev() {
        if is_word(c) {
            in_word = true;
        } else if in_word {
            break;
        }
        pos = p;
    }
    pos
}

/// Fim da palavra à direita (pula espaços e pontuação antes).
fn word_right(text: &str, i: usize) -> usize {
    let rest = &text[i..];
    let skip = rest.char_indices().find(|(_, c)| is_word(*c)).map_or(rest.len(), |(p, _)| p);
    let end = rest[skip..].char_indices().find(|(_, c)| !is_word(*c)).map_or(rest.len(), |(p, _)| skip + p);
    i + end
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit(text: &str, cursor: usize) -> (String, Edit) {
        (text.to_string(), Edit { cursor, anchor: None })
    }

    #[test]
    fn digita_no_meio() {
        let (mut t, mut e) = edit("casa", 2);
        e.insert(&mut t, "XY");
        assert_eq!(t, "caXYsa");
        assert_eq!(e.cursor, 4);
    }

    #[test]
    fn acentos_contam_como_um_caractere() {
        let (mut t, mut e) = edit("ação", "ação".len());
        e.left(&t, false, false);
        e.left(&t, false, false);
        assert_eq!(&t[..e.cursor], "aç");
        e.backspace(&mut t, false);
        assert_eq!(t, "aão");
        e.delete(&mut t, false);
        assert_eq!(t, "ao");
    }

    #[test]
    fn selecao_com_shift_e_substituir() {
        let (mut t, mut e) = edit("olá mundo", 0);
        for _ in 0..3 {
            e.right(&t, false, true);
        }
        assert_eq!(e.selected(&t), Some("olá"));
        e.insert(&mut t, "oi");
        assert_eq!(t, "oi mundo");
        assert_eq!(e.selection(), None);
    }

    #[test]
    fn seta_sem_shift_vai_para_a_ponta_da_selecao() {
        let (t, mut e) = edit("abcdef", 1);
        e.right(&t, false, true);
        e.right(&t, false, true);
        e.left(&t, false, false);
        assert_eq!((e.cursor, e.selection()), (1, None));
    }

    #[test]
    fn pula_e_apaga_palavras() {
        let (mut t, mut e) = edit("abrir o arquivo.txt", "abrir o arquivo.txt".len());
        e.left(&t, true, false);
        assert_eq!(&t[e.cursor..], "txt");
        e.left(&t, true, false);
        assert_eq!(&t[e.cursor..], "arquivo.txt");
        e.right(&t, true, false);
        assert_eq!(&t[..e.cursor], "abrir o arquivo");
        let mut e = Edit::at_end(&t);
        e.backspace(&mut t, true);
        assert_eq!(t, "abrir o arquivo.");
        e.move_to(0, false);
        e.delete(&mut t, true);
        assert_eq!(t, " o arquivo.");
    }

    #[test]
    fn tudo_selecionado_e_apagar() {
        let mut t = "texto".to_string();
        let mut e = Edit::all(&t);
        assert_eq!(e.selected(&t), Some("texto"));
        assert!(e.backspace(&mut t, false));
        assert_eq!((t.as_str(), e.cursor), ("", 0));
        assert!(!e.backspace(&mut t, false));
    }

    #[test]
    fn duplo_clique_seleciona_a_palavra() {
        let (t, mut e) = edit("nova_pasta de fotos", 0);
        e.select_word(&t, 3);
        assert_eq!(e.selected(&t), Some("nova_pasta"));
        e.select_word(&t, 15);
        assert_eq!(e.selected(&t), Some("fotos"));
    }

    #[test]
    fn cursor_fora_do_texto_e_corrigido() {
        let t = "ão".to_string();
        let mut e = Edit { cursor: 1, anchor: Some(99) };
        e.clamp(&t);
        assert_eq!((e.cursor, e.anchor), (0, Some(t.len())));
    }
}
