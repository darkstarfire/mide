//! Texto de várias linhas para editar (o documento do `Ui::text_area`): o texto, o
//! cursor e a seleção, desfazer e refazer, as contas de linha e coluna e as
//! decorações (cores, sublinhados e texto no fim da linha, do `Ui::text_area_with`).
//!
//! Lógica pura, testável sem janela. O desenho e as teclas ficam no `Ui::text_area`.
//! O histórico guarda cópias do texto (simples e suficiente para um editor mínimo);
//! um editor de arquivos grandes trocaria isso por uma estrutura própria.

use std::ops::Range;
use std::time::{Duration, Instant};

use super::{textedit::Edit, theme::Rgba};

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

    /// Há o que desfazer (o menu Editar desativa "Desfazer" sem isso).
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// Há o que refazer.
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
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
/// começo da linha até um ponto). A largura do começo só cresce, então a busca é
/// binária: poucas medidas mesmo numa linha de milhares de caracteres (um JSON
/// minificado), em vez de uma por caractere.
pub fn index_in_line(text: &str, starts: &[usize], line: usize, x: f32, mut width: impl FnMut(&str) -> f32) -> usize {
    let (start, end) = (starts[line], line_end(text, starts, line));
    let content = &text[start..end];
    // Fim de cada caractere; o caractere k fica entre ends[k-1] (ou 0) e ends[k].
    let ends: Vec<usize> = content.char_indices().map(|(i, c)| i + c.len_utf8()).collect();
    let mut w = |k: usize| if k == 0 { 0.0 } else { width(&content[..ends[k - 1]]) };
    // O primeiro caractere cujo meio passa de x: o cursor fica antes dele.
    let (mut lo, mut hi) = (0, ends.len());
    while lo < hi {
        let mid = (lo + hi) / 2;
        if x < (w(mid) + w(mid + 1)) / 2.0 {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    if lo == ends.len() { end } else { start + if lo == 0 { 0 } else { ends[lo - 1] } }
}

/// Decorações de uma área de texto (`Ui::text_area_with`), em índices de byte do
/// texto: trechos coloridos, trechos sublinhados (pontilhado na cor de perigo: os
/// erros) e um texto desenhado depois do fim de uma linha (o resultado das notas da
/// Calculadora). Ficam fora do texto: cursor, seleção, setas e clique não mudam.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Decorations {
    pub colors: Vec<(Range<usize>, Rgba)>,
    pub underlines: Vec<Range<usize>>,
    /// (linha a partir de 0, texto, cor): o texto vem 4 pt depois do fim da linha;
    /// vários na mesma linha vêm na ordem da lista, com 4 pt entre eles. O que não cabe
    /// até a borda termina com "…".
    pub trailing: Vec<(usize, String, Rgba)>,
}

/// O trecho `r` cortado na linha `start..end`, se ele vale: dentro do texto, nos
/// limites de caractere e não vazio depois do corte.
fn clip_to_line(text: &str, start: usize, end: usize, r: &Range<usize>) -> Option<(usize, usize)> {
    let ok = |i: usize| i <= text.len() && text.is_char_boundary(i);
    if r.start >= r.end || !ok(r.start) || !ok(r.end) {
        return None;
    }
    let (a, b) = (r.start.max(start), r.end.min(end));
    (a < b).then_some((a, b))
}

/// Pedaços da linha `start..end` por cor, em ordem e cobrindo a linha inteira:
/// (início, fim, cor), com `None` na cor normal do texto. Trechos fora do texto ou fora
/// do limite de um caractere são ignorados; quando dois se cobrem, vale o primeiro da
/// lista.
pub fn color_runs(text: &str, start: usize, end: usize, colors: &[(Range<usize>, Rgba)]) -> Vec<(usize, usize, Option<Rgba>)> {
    let spans: Vec<(usize, usize, Rgba)> = colors.iter().filter_map(|(r, c)| clip_to_line(text, start, end, r).map(|(a, b)| (a, b, *c))).collect();
    let mut cuts = vec![start, end];
    for (a, b, _) in &spans {
        cuts.push(*a);
        cuts.push(*b);
    }
    cuts.sort_unstable();
    cuts.dedup();
    let mut runs: Vec<(usize, usize, Option<Rgba>)> = Vec::new();
    for pair in cuts.windows(2) {
        let (p, q) = (pair[0], pair[1]);
        let color = spans.iter().find(|(a, b, _)| *a <= p && q <= *b).map(|s| s.2);
        match runs.last_mut() {
            Some(last) if last.2 == color => last.1 = q,
            _ => runs.push((p, q, color)),
        }
    }
    runs
}

/// Os trechos do pedaço `a..b` (da linha que começa em `start`) que podem ser
/// desenhados sozinhos. O texto desenhado conta as paradas de tabulação do começo dele:
/// um pedaço do meio da linha com tabulação é cortado nela, e cada trecho vai para o x
/// medido do começo da linha (a tabulação em si não desenha nada). Um pedaço que começa
/// a linha fica inteiro (o Editor de Texto, sem cores, desenha a linha de uma vez).
pub fn tab_pieces(text: &str, start: usize, a: usize, b: usize) -> Vec<(usize, usize)> {
    if a == start || !text[a..b].contains('\t') {
        return vec![(a, b)];
    }
    let mut pieces = Vec::new();
    let mut from = a;
    for (i, _) in text[a..b].match_indices('\t') {
        if from < a + i {
            pieces.push((from, a + i));
        }
        from = a + i + 1;
    }
    if from < b {
        pieces.push((from, b));
    }
    pieces
}

/// Os sublinhados que caem na linha `start..end`, cortados nela (os inválidos saem).
pub fn underline_runs(text: &str, start: usize, end: usize, underlines: &[Range<usize>]) -> Vec<(usize, usize)> {
    underlines.iter().filter_map(|r| clip_to_line(text, start, end, r)).collect()
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
    fn coluna_por_largura_em_linha_longa() {
        // Mesmo resultado da busca caractere a caractere, com poucas medidas.
        let text = format!("{}é{}", "a".repeat(3000), "b".repeat(10));
        let starts = line_starts(&text);
        let width = |s: &str| s.chars().count() as f32 * 7.0;
        for x in [0.0, 3.4, 3.6, 7000.0, 21_003.0, 21_004.0, 21_010.0, 99_999.0] {
            let slow = {
                let mut left = 0.0;
                let mut at = text.len();
                for (i, c) in text.char_indices() {
                    let right = width(&text[..i + c.len_utf8()]);
                    if x < (left + right) / 2.0 {
                        at = i;
                        break;
                    }
                    left = right;
                }
                at
            };
            let mut calls = 0;
            assert_eq!(index_in_line(&text, &starts, 0, x, |s| { calls += 1; width(s) }), slow, "x = {x}");
            assert!(calls < 60, "{calls} medidas");
        }
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
    fn pedacos_de_cor() {
        let text = "x = 2 + y\nsegunda";
        let starts = line_starts(text);
        let (s, e) = (starts[0], line_end(text, &starts, 0));
        let blue = [0, 0, 255, 255];
        let red = [255, 0, 0, 255];
        // Sem cores: a linha inteira na cor normal; linha vazia, nada.
        assert_eq!(color_runs(text, s, e, &[]), [(0, 9, None)]);
        assert!(color_runs("", 0, 0, &[]).is_empty());
        // Em ordem, com os buracos na cor normal; o trecho que passa da linha é cortado.
        let runs = color_runs(text, s, e, &[(8..12, red), (0..1, blue)]);
        assert_eq!(runs, [(0, 1, Some(blue)), (1, 8, None), (8, 9, Some(red))]);
        // Fora do texto, no meio de um caractere, vazio ou invertido: ignorados.
        let acento = "pé = 1";
        #[allow(clippy::reversed_empty_ranges)]
        let bad = [(0..2, blue), (0..99, red), (3..3, red), (4..2, red)];
        assert_eq!(color_runs(acento, 0, acento.len(), &bad), [(0, acento.len(), None)]);
        // Dois que se cobrem: vale o primeiro da lista.
        assert_eq!(color_runs(text, s, e, &[(0..5, blue), (2..9, red)]), [(0, 5, Some(blue)), (5, 9, Some(red))]);
        // Sublinhados: cortados na linha, os inválidos saem.
        assert_eq!(underline_runs(text, starts[1], text.len(), &[8..13, 0..99]), [(10, 13)]);
    }

    #[test]
    fn tabulacao_corta_o_pedaco_do_meio() {
        let text = "aluguel\t* 2 =\tfim";
        // O pedaço que começa a linha fica inteiro, mesmo com tabulação.
        assert_eq!(tab_pieces(text, 0, 0, 7), [(0, 7)]);
        assert_eq!(tab_pieces(text, 0, 0, text.len()), [(0, text.len())]);
        // Do meio da linha: cortado em cada tabulação, sem ela.
        assert_eq!(tab_pieces(text, 0, 7, text.len()), [(8, 13), (14, 17)]);
        // Tabulações seguidas e no fim: nenhum trecho vazio.
        assert_eq!(tab_pieces("a\t\tb\t", 0, 1, 5), [(3, 4)]);
        // Sem tabulação: o pedaço como veio.
        assert_eq!(tab_pieces(text, 0, 8, 13), [(8, 13)]);
    }

    #[test]
    fn pode_desfazer_e_refazer() {
        let mut doc = TextDoc::new("a".into());
        assert!(!doc.can_undo() && !doc.can_redo());
        doc.type_text("b");
        assert!(doc.can_undo() && !doc.can_redo());
        doc.undo();
        assert!(!doc.can_undo() && doc.can_redo());
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
