//! Texto de várias linhas para editar (o documento do `Ui::text_area`): o texto, o
//! cursor e a seleção, desfazer e refazer, e as contas de linha e coluna.
//!
//! Lógica pura, testável sem janela. O desenho e as teclas ficam no `Ui::text_area`.
//! O histórico guarda cópias do texto (simples e suficiente para um editor mínimo);
//! um editor de arquivos grandes trocaria isso por uma estrutura própria.

use std::time::{Duration, Instant};

use super::textedit::Edit;

/// Tipo de mudança, para juntar no histórico o que foi digitado em sequência (desfazer
/// volta uma palavra digitada de uma vez, e não letra por letra).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Typing,
    Deleting,
    Other,
}

/// Documento em edição.
#[derive(Debug, Clone, Default)]
pub struct TextDoc {
    pub text: String,
    pub edit: Edit,
    /// Rolagem vertical e horizontal, em pixels.
    pub scroll: f32,
    pub scroll_x: f32,
    /// Coluna desejada (em pixels) ao subir e descer linhas por textos curtos.
    pub goal_x: Option<f32>,
    undo: Vec<(String, Edit)>,
    redo: Vec<(String, Edit)>,
    last_change: Option<(Change, Instant)>,
    /// O texto mudou desde que foi aberto ou salvo.
    pub modified: bool,
    /// Aumenta a cada mudança do texto (para saber se mudou num quadro).
    pub version: u64,
}

impl TextDoc {
    pub fn new(text: String) -> Self {
        Self { text, ..Default::default() }
    }

    /// Guarda o estado antes de uma mudança. Mudanças do mesmo tipo em sequência
    /// (digitar várias letras) ficam num passo só do desfazer.
    pub fn before_change(&mut self, change: Change) {
        let now = Instant::now();
        let joins = change != Change::Other
            && self.last_change.is_some_and(|(kind, at)| kind == change && now.duration_since(at) < Duration::from_secs(1));
        if !joins {
            self.undo.push((self.text.clone(), self.edit));
            if self.undo.len() > 500 {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
        self.last_change = Some((change, now));
        self.modified = true;
        self.version += 1;
    }

    /// Desfaz a última mudança; devolve se havia o que desfazer.
    pub fn undo(&mut self) -> bool {
        let Some((text, edit)) = self.undo.pop() else {
            return false;
        };
        self.redo.push((std::mem::replace(&mut self.text, text), self.edit));
        self.edit = edit;
        self.last_change = None;
        self.modified = true;
        self.version += 1;
        true
    }

    /// Refaz o que foi desfeito; devolve se havia o que refazer.
    pub fn redo(&mut self) -> bool {
        let Some((text, edit)) = self.redo.pop() else {
            return false;
        };
        self.undo.push((std::mem::replace(&mut self.text, text), self.edit));
        self.edit = edit;
        self.last_change = None;
        self.modified = true;
        self.version += 1;
        true
    }

    /// Começo de cada linha (índices de byte).
    pub fn line_starts(&self) -> Vec<usize> {
        line_starts(&self.text)
    }

    /// Escreve no cursor (no lugar da seleção), registrando no histórico.
    pub fn type_text(&mut self, s: &str) {
        if s.is_empty() {
            return;
        }
        self.before_change(if s.chars().any(char::is_whitespace) { Change::Other } else { Change::Typing });
        self.edit.insert(&mut self.text, s);
        self.goal_x = None;
    }

    /// Nova linha com o mesmo recuo da linha atual.
    pub fn newline(&mut self) {
        let starts = self.line_starts();
        let line = line_of(&starts, self.edit.selection().map_or(self.edit.cursor, |(a, _)| a));
        let indent: String = self.text[starts[line]..].chars().take_while(|c| *c == ' ' || *c == '\t').collect();
        self.before_change(Change::Other);
        self.edit.insert(&mut self.text, &format!("\n{indent}"));
        self.goal_x = None;
    }
}

/// Começo de cada linha (índices de byte); sempre há pelo menos uma linha.
pub fn line_starts(text: &str) -> Vec<usize> {
    std::iter::once(0).chain(text.match_indices('\n').map(|(i, _)| i + 1)).collect()
}

/// Linha (a partir de 0) que contém a posição `at`.
pub fn line_of(starts: &[usize], at: usize) -> usize {
    starts.partition_point(|s| *s <= at).saturating_sub(1)
}

/// Fim da linha `line` (antes do `\n`).
pub fn line_end(text: &str, starts: &[usize], line: usize) -> usize {
    starts.get(line + 1).map_or(text.len(), |next| next - 1)
}

/// Posição na linha `line` mais perto de `x` pixels, medindo com `width` (largura do
/// começo da linha até um ponto).
pub fn index_in_line(text: &str, starts: &[usize], line: usize, x: f32, mut width: impl FnMut(&str) -> f32) -> usize {
    let (start, end) = (starts[line], line_end(text, starts, line));
    let content = &text[start..end];
    let mut left = 0.0;
    for (i, c) in content.char_indices() {
        let right = width(&content[..i + c.len_utf8()]);
        if x < (left + right) / 2.0 {
            return start + i;
        }
        left = right;
    }
    end
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linhas_e_posicoes() {
        let text = "um\ndois\n\ntrês";
        let starts = line_starts(text);
        assert_eq!(starts, vec![0, 3, 8, 9]);
        assert_eq!(line_of(&starts, 0), 0);
        assert_eq!(line_of(&starts, 4), 1);
        assert_eq!(line_of(&starts, 8), 2);
        assert_eq!(line_of(&starts, text.len()), 3);
        assert_eq!(&text[starts[1]..line_end(text, &starts, 1)], "dois");
        assert_eq!(line_end(text, &starts, 3), text.len());
    }

    #[test]
    fn coluna_por_largura() {
        let text = "abc\nxyz";
        let starts = line_starts(text);
        // Cada letra com 10 px: x=14 fica entre a 1ª e a 2ª letra, mais perto da 2ª.
        let at = index_in_line(text, &starts, 1, 14.0, |s| s.chars().count() as f32 * 10.0);
        assert_eq!(at, 5);
        assert_eq!(index_in_line(text, &starts, 0, 999.0, |s| s.len() as f32 * 10.0), 3);
    }

    #[test]
    fn desfazer_junta_a_digitacao_e_refazer_volta() {
        let mut doc = TextDoc::new(String::new());
        doc.type_text("o");
        doc.type_text("l");
        doc.type_text("á");
        doc.type_text(" ");
        doc.type_text("mundo");
        assert_eq!(doc.text, "olá mundo");
        assert!(doc.undo());
        assert_eq!(doc.text, "olá ");
        assert!(doc.undo());
        assert_eq!(doc.text, "olá");
        assert!(doc.undo());
        assert_eq!(doc.text, "");
        assert!(!doc.undo());
        assert!(doc.redo());
        assert_eq!(doc.text, "olá");
        // Mudar o texto depois de desfazer apaga o que dava para refazer.
        doc.type_text("!");
        assert!(!doc.redo());
    }

    #[test]
    fn nova_linha_mantem_o_recuo() {
        let mut doc = TextDoc::new("fn main() {\n    let x = 1;".into());
        doc.edit = Edit::at_end(&doc.text);
        doc.newline();
        doc.type_text("y");
        assert_eq!(doc.text, "fn main() {\n    let x = 1;\n    y");
        assert!(doc.modified);
    }
}
