//! Quebra de linhas e paginação do documento formatado.
//!
//! Cada parágrafo é "formatado" pelo cosmic-text (desenho e largura de cada letra, com
//! as fontes de reserva para o que a fonte não tem); a quebra em linhas (palavra a
//! palavra, com recuo da primeira linha, tabulações e justificado) e a divisão em
//! páginas são daqui. Os parágrafos já quebrados ficam guardados pela "impressão
//! digital" deles: digitar num livro inteiro só refaz o parágrafo mexido.

use std::{
    collections::{HashMap, HashSet},
    hash::{Hash, Hasher},
};

use cosmic_text::{Attrs, AttrsList, CacheKeyFlags, Family, FontSystem, Metrics, ShapeLine, Shaping, Style, Weight};

use super::{Align, Baseline, CharStyle, Doc, ListKind, PageSetup, Paragraph, Pos, LINE_BREAK, OBJECT};

/// Distância entre as tabulações padrão (meia polegada).
pub const TAB_STOP: f32 = 36.0;
/// Recuo de cada nível de lista.
pub const LIST_INDENT: f32 = 18.0;

/// Uma letra posicionada (x em relação à esquerda do texto da página).
#[derive(Debug, Clone)]
pub struct Glyph {
    pub x: f32,
    /// Deslocamento vertical da linha de base (sobrescrito, subscrito, acentos).
    pub dy: f32,
    pub font_id: cosmic_text::fontdb::ID,
    pub glyph_id: u16,
    pub size: f32,
    pub weight: Weight,
    pub flags: CacheKeyFlags,
    pub width: f32,
    /// Índice do trecho de estilo (para cor, sublinhado...).
    pub span: usize,
    /// O texto que a letra desenha (bytes no parágrafo), para o PDF saber o caractere.
    pub start: usize,
    pub end: usize,
}

/// Uma linha do parágrafo.
#[derive(Debug, Clone)]
pub struct Line {
    pub start: usize,
    pub end: usize,
    /// Altura da linha (com a entrelinha) e a linha de base a partir do topo dela.
    pub height: f32,
    pub baseline: f32,
    /// Altura das letras (para o cursor e a seleção).
    pub ascent: f32,
    pub descent: f32,
    pub glyphs: Vec<Glyph>,
    /// Onde fica cada posição de cursor da linha: (byte, x).
    pub carets: Vec<(usize, f32)>,
    /// Começo e fim do texto da linha (x), para a seleção.
    pub x0: f32,
    pub x1: f32,
}

/// Um parágrafo quebrado em linhas (largura fixa, sem a posição na página).
#[derive(Debug, Clone)]
pub struct ParaBox {
    pub lines: Vec<Line>,
    /// Marcador da lista ("•", "1.") e o x dele.
    pub marker: Option<(Vec<Glyph>, f32)>,
}

/// Uma linha no lugar: parágrafo, linha dele, página e o topo (pontos, desde o topo da
/// página).
#[derive(Debug, Clone, Copy)]
pub struct Placed {
    pub para: usize,
    pub line: usize,
    pub page: usize,
    pub y: f32,
}

/// O documento todo quebrado em linhas e páginas.
#[derive(Debug, Clone, Default)]
pub struct Layout {
    pub paras: Vec<ParaBox>,
    pub placed: Vec<Placed>,
    /// Índice em `placed` da primeira linha de cada parágrafo.
    pub first: Vec<usize>,
    pub pages: usize,
    /// Estilos de cada parágrafo (cópia, para o desenho achar cor e sublinhado).
    pub styles: Vec<Vec<CharStyle>>,
}

impl Layout {
    /// A linha colocada que contém a posição (no fim de uma linha quebrada, a próxima).
    pub fn line_of(&self, p: Pos) -> Option<&Placed> {
        let first = *self.first.get(p.para)?;
        let count = self.paras[p.para].lines.len();
        let mut best = first;
        for i in first..first + count {
            let line = &self.paras[p.para].lines[self.placed[i].line];
            if p.off >= line.start {
                best = i;
            }
        }
        self.placed.get(best)
    }

    /// x do cursor na posição (dentro da linha dela).
    pub fn caret_x(&self, p: Pos) -> f32 {
        let Some(placed) = self.line_of(p) else {
            return 0.0;
        };
        let line = &self.paras[p.para].lines[placed.line];
        caret_in(line, p.off)
    }

    /// A posição mais perto do ponto (pontos dentro da página `page`; x desde a
    /// esquerda do texto).
    pub fn hit(&self, page: usize, x: f32, y: f32) -> Pos {
        let mut best: Option<&Placed> = None;
        for placed in self.placed.iter().filter(|pl| pl.page == page) {
            let line = &self.paras[placed.para].lines[placed.line];
            if best.is_none() || y >= placed.y {
                best = Some(placed);
            }
            if y < placed.y + line.height {
                break;
            }
        }
        let Some(placed) = best.or_else(|| self.placed.iter().rev().find(|pl| pl.page < page)).or(self.placed.first()) else {
            return Pos::default();
        };
        let line = &self.paras[placed.para].lines[placed.line];
        Pos::new(placed.para, nearest(line, x))
    }

    /// A posição na linha de cima (`dir` = -1) ou de baixo (+1), mantendo o x.
    pub fn vertical(&self, p: Pos, x: f32, dir: i32) -> Pos {
        let Some(placed) = self.line_of(p) else {
            return p;
        };
        let index = self.placed.iter().position(|pl| pl.para == placed.para && pl.line == placed.line).unwrap_or(0);
        let target = index as i64 + dir as i64;
        if target < 0 {
            return Pos::new(0, 0);
        }
        let Some(t) = self.placed.get(target as usize) else {
            let last = self.placed.last().expect("sem linhas");
            let line = &self.paras[last.para].lines[last.line];
            return Pos::new(last.para, line.end);
        };
        let line = &self.paras[t.para].lines[t.line];
        Pos::new(t.para, nearest(line, x))
    }

    /// Começo e fim da linha da posição (Home, End).
    pub fn line_bounds(&self, p: Pos) -> (Pos, Pos) {
        let Some(placed) = self.line_of(p) else {
            return (p, p);
        };
        let line = &self.paras[p.para].lines[placed.line];
        let last = placed.line + 1 == self.paras[p.para].lines.len();
        // No fim de uma linha quebrada, o cursor fica antes do espaço que quebrou.
        let mut end = line.end;
        if !last {
            if let Some(&(off, _)) = line.carets.iter().rev().find(|(o, _)| *o < line.end) {
                end = off.max(line.start);
            }
        }
        (Pos::new(p.para, line.start), Pos::new(p.para, end))
    }
}

fn caret_in(line: &Line, off: usize) -> f32 {
    let mut x = line.x0;
    for &(o, cx) in &line.carets {
        if o <= off {
            x = cx;
        } else {
            break;
        }
    }
    x
}

fn nearest(line: &Line, x: f32) -> usize {
    line.carets.iter().min_by(|a, b| (a.1 - x).abs().total_cmp(&(b.1 - x).abs())).map_or(line.start, |c| c.0)
}

/// Quem quebra o documento: guarda os parágrafos já quebrados e a escolha das fontes.
#[derive(Default)]
pub struct Layouter {
    cache: HashMap<u64, ParaBox>,
    families: HashMap<String, String>,
}

/// Fontes de troca para as famílias comuns que podem faltar no Linux (as de métricas
/// iguais primeiro: um trabalho em "Times New Roman" ocupa as mesmas páginas).
fn substitutes(name: &str) -> &'static [&'static str] {
    match name.to_lowercase().as_str() {
        "times new roman" | "times" | "serif" => &["Liberation Serif", "Tinos", "Nimbus Roman", "TeX Gyre Termes", "DejaVu Serif", "Noto Serif"],
        "arial" | "helvetica" | "helvetica neue" | "sans-serif" | "sans" => &["Liberation Sans", "Arimo", "Nimbus Sans", "TeX Gyre Heros", "Inter", "DejaVu Sans", "Noto Sans"],
        "courier new" | "courier" | "monospace" | "menlo" => &["Liberation Mono", "Cousine", "Nimbus Mono PS", "DejaVu Sans Mono", "Noto Sans Mono"],
        "calibri" => &["Carlito", "Liberation Sans", "DejaVu Sans"],
        "cambria" => &["Caladea", "Liberation Serif", "DejaVu Serif"],
        "georgia" => &["Gelasio", "DejaVu Serif"],
        _ => &["Liberation Sans", "DejaVu Sans", "Noto Sans"],
    }
}

fn fingerprint(p: &Paragraph, width: f32) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    width.to_bits().hash(&mut h);
    p.text.hash(&mut h);
    for s in &p.spans {
        s.len.hash(&mut h);
        let c = &s.style;
        (&c.family, c.size.to_bits(), c.bold, c.italic, c.underline, c.strike, c.color, c.highlight, c.baseline).hash(&mut h);
    }
    let st = &p.style;
    (st.align, st.first_indent.to_bits(), st.left_indent.to_bits(), st.right_indent.to_bits(), st.line_spacing.to_bits(), st.list, st.level).hash(&mut h);
    if let Some(img) = &p.image {
        (img.width.to_bits(), img.height.to_bits()).hash(&mut h);
    }
    h.finish()
}

/// Altura das letras (acima e abaixo da linha de base) em múltiplos do tamanho, quando
/// não há letra para medir (parágrafo vazio).
const EMPTY_ASCENT: f32 = 0.891;
const EMPTY_DESCENT: f32 = 0.216;

impl Layouter {
    /// A família instalada para o nome pedido (o próprio nome, ou uma de troca).
    pub fn resolve(&mut self, fonts: &FontSystem, name: &str) -> String {
        if let Some(found) = self.families.get(name) {
            return found.clone();
        }
        let has = |n: &str| fonts.db().faces().any(|f| f.families.iter().any(|(fam, _)| fam.eq_ignore_ascii_case(n)));
        let found = if has(name) {
            name.to_string()
        } else {
            substitutes(name).iter().find(|n| has(n)).map(|n| n.to_string()).unwrap_or_else(|| "sans-serif".into())
        };
        self.families.insert(name.to_string(), found.clone());
        found
    }

    /// Quebra o documento todo.
    pub fn layout(&mut self, fonts: &mut FontSystem, doc: &Doc) -> Layout {
        let page = &doc.page;
        let width = page.content_width();
        let mut used = HashSet::new();
        let mut out = Layout::default();
        let mut counters = [0usize; 9];
        for (index, para) in doc.paras.iter().enumerate() {
            // Numeração das listas: conta os parágrafos seguidos do mesmo nível.
            let number = match para.style.list {
                Some(ListKind::Number | ListKind::Letter) => {
                    let level = (para.style.level as usize).min(8);
                    counters[level] += 1;
                    for c in counters.iter_mut().skip(level + 1) {
                        *c = 0;
                    }
                    counters[level]
                }
                Some(_) => 0,
                None => {
                    counters = [0; 9];
                    0
                }
            };
            let key = fingerprint(para, width) ^ (number as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
            used.insert(key);
            let boxed = match self.cache.get(&key) {
                Some(b) => b.clone(),
                None => {
                    let b = self.para(fonts, para, width, number);
                    self.cache.insert(key, b.clone());
                    b
                }
            };
            out.paras.push(boxed);
            out.styles.push(para.spans.iter().map(|s| s.style.clone()).collect());
            let _ = index;
        }
        self.cache.retain(|k, _| used.contains(k));
        paginate(&mut out, doc, page);
        out
    }

    fn attrs<'a>(&mut self, fonts: &FontSystem, style: &CharStyle, family: &'a mut String, index: usize) -> (Attrs<'a>, f32) {
        *family = self.resolve(fonts, &style.family);
        let size = match style.baseline {
            Baseline::Normal => style.size,
            _ => style.size * 0.66,
        };
        let attrs = Attrs::new()
            .family(Family::Name(family))
            .weight(if style.bold { Weight::BOLD } else { Weight::NORMAL })
            .style(if style.italic { Style::Italic } else { Style::Normal })
            .metrics(Metrics::new(size, size * 1.2))
            .metadata(index);
        (attrs, size)
    }

    /// Dá forma a um texto de um estilo só (marcadores de lista, números de página).
    pub fn shape_simple(&mut self, fonts: &mut FontSystem, text: &str, style: &CharStyle) -> (Vec<Glyph>, f32) {
        let mut family = String::new();
        let (attrs, size) = self.attrs(fonts, style, &mut family, 0);
        let list = AttrsList::new(&attrs);
        let shaped = ShapeLine::new(fonts, text, &list, Shaping::Advanced, 4);
        let mut x = 0.0;
        let mut glyphs = Vec::new();
        for span in &shaped.spans {
            for word in &span.words {
                for g in &word.glyphs {
                    let w = g.width(size);
                    glyphs.push(Glyph { x: x + g.x_offset * size, dy: -g.y_offset * size, font_id: g.font_id, glyph_id: g.glyph_id, size, weight: g.font_weight, flags: g.cache_key_flags, width: w, span: 0, start: g.start, end: g.end });
                    x += w;
                }
            }
        }
        (glyphs, x)
    }

    /// Quebra um parágrafo na largura `width` (a do texto da página).
    fn para(&mut self, fonts: &mut FontSystem, para: &Paragraph, width: f32, number: usize) -> ParaBox {
        let st = &para.style;
        let list_indent = if st.list.is_some() { LIST_INDENT * (st.level as f32 + 1.0) } else { 0.0 };
        let left = st.left_indent + list_indent;
        let right = st.right_indent;
        let first_style = &para.spans[0].style;
        // Imagem: uma linha com a altura dela.
        if let Some(img) = &para.image {
            let avail = (width - left - right).max(1.0);
            let w = img.width.min(avail);
            let h = img.height * w / img.width.max(1.0);
            let x0 = match st.align {
                Align::Center | Align::Justify => left + (avail - w) / 2.0,
                Align::Right => left + avail - w,
                Align::Left => left,
            };
            let end = para.text.len();
            let line = Line {
                start: 0,
                end,
                height: h + 4.0,
                baseline: h,
                ascent: h,
                descent: 0.0,
                glyphs: Vec::new(),
                carets: vec![(0, x0), (end, x0 + w)],
                x0,
                x1: x0 + w,
            };
            return ParaBox { lines: vec![line], marker: None };
        }
        // Atributos por trecho.
        let mut families: Vec<String> = vec![String::new(); para.spans.len()];
        let mut sizes = Vec::with_capacity(para.spans.len());
        let mut list = None::<AttrsList>;
        {
            let mut pos = 0;
            let mut fams = families.iter_mut();
            for (i, span) in para.spans.iter().enumerate() {
                let fam = fams.next().expect("família");
                let (attrs, size) = self.attrs(fonts, &span.style, fam, i);
                sizes.push(size);
                let l = list.get_or_insert_with(|| AttrsList::new(&attrs));
                if span.len > 0 {
                    l.add_span(pos..pos + span.len, &attrs);
                }
                pos += span.len;
            }
        }
        let attrs_list = list.expect("ao menos um trecho");
        let shaped = ShapeLine::new(fonts, &para.text, &attrs_list, Shaping::Advanced, 4);
        // As palavras em ordem (texto da esquerda para a direita).
        struct Word {
            glyphs: Vec<(cosmic_text::ShapeGlyph, f32)>,
            blank: bool,
            forced: bool,
            tab: bool,
        }
        let mut words = Vec::new();
        for span in &shaped.spans {
            for word in &span.words {
                let text = word.glyphs.first().map(|g| &para.text[g.start..word.glyphs.last().map_or(g.end, |l| l.end)]).unwrap_or("");
                let forced = text.contains(LINE_BREAK);
                let tab = text == "\t";
                let glyphs = word
                    .glyphs
                    .iter()
                    .map(|g| {
                        let size = sizes.get(g.metadata).copied().unwrap_or(12.0);
                        (g.clone(), size)
                    })
                    .collect();
                words.push(Word { glyphs, blank: word.blank, forced, tab });
            }
        }
        let width_of = |w: &Word| w.glyphs.iter().map(|(g, s)| g.width(*s)).sum::<f32>();
        // Quebra gulosa.
        let mut lines_words: Vec<(Vec<usize>, bool)> = Vec::new();
        let mut cur: Vec<usize> = Vec::new();
        let mut x = 0.0f32;
        let avail_for = |first: bool| (width - left - right - if first { st.first_indent } else { 0.0 }).max(12.0);
        let mut avail = avail_for(true);
        for (i, w) in words.iter().enumerate() {
            let ww = if w.tab { TAB_STOP - ((x + st.first_indent.max(0.0)) % TAB_STOP) } else { width_of(w) };
            let has_content = cur.iter().any(|&j| !words[j].blank);
            if !w.blank && x + ww > avail && has_content {
                lines_words.push((std::mem::take(&mut cur), false));
                x = 0.0;
                avail = avail_for(false);
            }
            cur.push(i);
            x += ww;
            if w.forced {
                lines_words.push((std::mem::take(&mut cur), true));
                x = 0.0;
                avail = avail_for(false);
            }
        }
        lines_words.push((cur, true));
        // Linhas: posições das letras, alinhamento e cursores.
        let mut lines = Vec::new();
        let n_lines = lines_words.len();
        for (li, (idx, forced)) in lines_words.into_iter().enumerate() {
            let first = li == 0;
            let line_left = left + if first { st.first_indent } else { 0.0 };
            let avail = avail_for(first);
            // Largura sem os espaços do fim.
            let mut content_end = idx.len();
            while content_end > 0 && words[idx[content_end - 1]].blank {
                content_end -= 1;
            }
            let mut natural = 0.0;
            let mut pen = 0.0;
            for (k, &wi) in idx.iter().enumerate() {
                let w = &words[wi];
                let ww = if w.tab { TAB_STOP - ((line_left + pen) % TAB_STOP).max(0.0) } else { width_of(w) };
                pen += ww;
                if k < content_end {
                    natural = pen;
                }
            }
            let gaps = idx[..content_end].iter().filter(|&&wi| words[wi].blank && !words[wi].tab).count();
            let justify = st.align == Align::Justify && !forced && gaps > 0;
            let extra = if justify { (avail - natural).max(0.0) / gaps as f32 } else { 0.0 };
            let shift = match st.align {
                Align::Center => (avail - natural).max(0.0) / 2.0,
                Align::Right => (avail - natural).max(0.0),
                _ => 0.0,
            };
            let mut glyphs = Vec::new();
            let mut carets: Vec<(usize, f32)> = Vec::new();
            let mut pen = line_left + shift;
            let (mut ascent, mut descent) = (0.0f32, 0.0f32);
            let mut start = usize::MAX;
            let mut end = 0;
            for (k, &wi) in idx.iter().enumerate() {
                let w = &words[wi];
                let tab_w = if w.tab { Some(TAB_STOP - ((pen) % TAB_STOP).max(0.0)) } else { None };
                let gap_extra = if justify && w.blank && !w.tab && k < content_end { extra } else { 0.0 };
                for (g, size) in &w.glyphs {
                    let gw = tab_w.unwrap_or(g.width(*size)) + if w.blank { gap_extra / w.glyphs.len().max(1) as f32 } else { 0.0 };
                    start = start.min(g.start);
                    end = end.max(g.end);
                    // Cursores: um por caractere do grupo de letras (ligaduras dividem).
                    let cluster = &para.text[g.start..g.end];
                    let n = cluster.chars().count().max(1);
                    for (ci, (bo, _)) in cluster.char_indices().enumerate() {
                        carets.push((g.start + bo, pen + gw * ci as f32 / n as f32));
                    }
                    let span_style = &para.spans.get(g.metadata).map(|s| &s.style).unwrap_or(first_style);
                    let dy = -g.y_offset * size
                        + match span_style.baseline {
                            Baseline::Normal => 0.0,
                            Baseline::Super => -span_style.size * 0.33,
                            Baseline::Sub => span_style.size * 0.14,
                        };
                    let is_break = cluster.contains(LINE_BREAK) || cluster == "\t" || cluster.contains(OBJECT);
                    if !is_break {
                        glyphs.push(Glyph { x: pen + g.x_offset * size, dy, font_id: g.font_id, glyph_id: g.glyph_id, size: *size, weight: g.font_weight, flags: g.cache_key_flags, width: gw, span: g.metadata, start: g.start, end: g.end });
                    }
                    ascent = ascent.max(g.ascent * size);
                    descent = descent.max(g.descent * size);
                    pen += gw;
                }
            }
            if start == usize::MAX {
                // Linha vazia (parágrafo vazio): o cursor no começo, com a altura da fonte.
                start = para.text.len();
                end = para.text.len();
                let size = first_style.size;
                ascent = size * EMPTY_ASCENT;
                descent = size * EMPTY_DESCENT;
            }
            // Fim da linha: depois da última letra (numa quebra forçada, antes dela).
            let last = li + 1 == n_lines;
            if last {
                end = para.text.len();
                carets.push((end, pen));
            } else if forced {
                // O cursor depois da quebra fica na linha seguinte.
                carets.retain(|c| c.0 < end);
                if let Some(&(o, x)) = carets.last() {
                    let _ = (o, x);
                }
            }
            let single = ascent + descent;
            let height = single * st.line_spacing.max(0.5);
            let x0 = carets.first().map_or(line_left + shift, |c| c.1);
            lines.push(Line { start, end, height, baseline: ascent, ascent, descent, glyphs, carets, x0, x1: pen });
        }
        // Marcador da lista, à esquerda do texto.
        let marker = st.list.map(|kind| {
            let text = match kind {
                ListKind::Bullet => ["•", "◦", "▪"][st.level as usize % 3].to_string(),
                ListKind::Dash => "–".to_string(),
                ListKind::Number => format!("{number}."),
                ListKind::Letter => format!("{})", (b'a' + ((number.max(1) - 1) % 26) as u8) as char),
            };
            let (glyphs, w) = self.shape_simple(fonts, &text, first_style);
            let x = (left + st.first_indent.min(0.0) - 6.0 - w).max(st.left_indent);
            (glyphs, x)
        });
        ParaBox { lines, marker }
    }
}

/// Coloca as linhas nas páginas.
fn paginate(out: &mut Layout, doc: &Doc, page: &PageSetup) {
    let top = page.margins[0];
    let bottom = page.height - page.margins[2];
    let mut y = top;
    let mut page_no = 0;
    for (pi, para) in doc.paras.iter().enumerate() {
        let st = &para.style;
        if st.page_break_before && y > top {
            page_no += 1;
            y = top;
        }
        if y > top {
            y += st.space_before;
        }
        out.first.push(out.placed.len());
        let count = out.paras[pi].lines.len();
        for li in 0..count {
            let h = out.paras[pi].lines[li].height;
            if y + h > bottom && y > top {
                page_no += 1;
                y = top;
            }
            out.placed.push(Placed { para: pi, line: li, page: page_no, y });
            y += h;
        }
        y += st.space_after;
    }
    out.pages = page_no + 1;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::richtext::{cm, CharStyle, ParaStyle};

    fn fonts() -> FontSystem {
        FontSystem::new()
    }

    #[test]
    fn quebra_linhas_e_paginas() {
        let mut fs = fonts();
        let mut doc = Doc::new();
        let text = "palavra ".repeat(400);
        doc.paras[0] = Paragraph::new(&text, CharStyle::default(), ParaStyle::default());
        let mut l = Layouter::default();
        let lay = l.layout(&mut fs, &doc);
        assert!(lay.paras[0].lines.len() > 20, "linhas: {}", lay.paras[0].lines.len());
        let w = doc.page.content_width();
        for line in &lay.paras[0].lines {
            assert!(line.glyphs.last().map_or(0.0, |g| g.x + g.width) <= w + 30.0);
        }
        // As linhas cobrem o texto inteiro, em ordem.
        let lines = &lay.paras[0].lines;
        assert_eq!(lines[0].start, 0);
        for pair in lines.windows(2) {
            assert!(pair[1].start >= pair[0].start);
        }
        // Texto de 3 páginas.
        doc.paras = (0..150).map(|_| Paragraph::new("linha", CharStyle::default(), ParaStyle { line_spacing: 1.5, ..Default::default() })).collect();
        let lay = l.layout(&mut fs, &doc);
        assert!(lay.pages >= 2, "páginas: {}", lay.pages);
        assert!(lay.placed.iter().all(|p| p.y + 1.0 >= doc.page.margins[0] && p.y <= doc.page.height - doc.page.margins[2]));
    }

    #[test]
    fn centralizado_e_cursor() {
        let mut fs = fonts();
        let mut doc = Doc::new();
        doc.paras[0] = Paragraph::new("Título", CharStyle::default(), ParaStyle { align: Align::Center, ..Default::default() });
        let mut l = Layouter::default();
        let lay = l.layout(&mut fs, &doc);
        let line = &lay.paras[0].lines[0];
        let w = doc.page.content_width();
        let mid = (line.x0 + line.x1) / 2.0;
        assert!((mid - w / 2.0).abs() < 2.0, "meio {mid} de {w}");
        // Clique no meio da palavra cai dentro dela.
        let p = lay.hit(0, mid, doc.page.margins[0] + 5.0);
        assert!(p.off > 0 && p.off < "Título".len());
        assert!((lay.caret_x(Pos::new(0, 0)) - line.x0).abs() < 0.01);
        let _ = cm(1.0);
    }

    #[test]
    fn recuo_da_primeira_linha() {
        let mut fs = fonts();
        let mut doc = Doc::new();
        doc.paras[0] = Paragraph::new("Texto", CharStyle::default(), ParaStyle { first_indent: cm(1.25), ..Default::default() });
        let mut l = Layouter::default();
        let lay = l.layout(&mut fs, &doc);
        assert!((lay.paras[0].lines[0].x0 - cm(1.25)).abs() < 0.5);
    }
}
