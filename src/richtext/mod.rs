//! Documento de texto formatado do Editor de Texto (o TextEdit do mide): parágrafos com
//! trechos de estilo (fonte, tamanho, negrito, cor...), estilo de parágrafo
//! (alinhamento, recuos, entrelinha, listas), imagens e a configuração da página.
//!
//! Tudo é escrito aqui: o modelo, a edição ([`Doc`]), a quebra de linhas e a paginação
//! ([`layout`]), o desenho ([`render`]) e os formatos de arquivo. O cosmic-text, que o
//! mide-ui já usa, só dá forma às letras (escolhe o desenho de cada uma e a largura).
//!
//! Medidas em pontos tipográficos (1/72 de polegada), como no Word e no TextEdit.

pub mod layout;
pub mod render;
pub mod docx;
pub mod odt;
pub mod pdf;
pub mod rtf;
pub mod xml;

use std::rc::Rc;

use tiny_skia::Pixmap;

/// Centímetros em pontos.
pub fn cm(v: f32) -> f32 {
    v * 72.0 / 2.54
}

/// Pontos em centímetros.
pub fn to_cm(pt: f32) -> f32 {
    pt * 2.54 / 72.0
}

/// Posição vertical do texto (sobrescrito e subscrito).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum Baseline {
    #[default]
    Normal,
    Super,
    Sub,
}

/// Estilo de um trecho de texto.
#[derive(Debug, Clone, PartialEq)]
pub struct CharStyle {
    pub family: String,
    /// Tamanho em pontos.
    pub size: f32,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub color: [u8; 3],
    /// Cor de fundo (realce), se houver.
    pub highlight: Option<[u8; 3]>,
    pub baseline: Baseline,
    pub link: Option<String>,
    /// Este trecho é a referência de uma nota de rodapé (o id dela em `Doc::notes`).
    pub note: Option<u32>,
}

impl Default for CharStyle {
    fn default() -> Self {
        Self {
            family: "Helvetica".into(),
            size: 12.0,
            bold: false,
            italic: false,
            underline: false,
            strike: false,
            color: [0, 0, 0],
            highlight: None,
            baseline: Baseline::Normal,
            link: None,
            note: None,
        }
    }
}

/// Alinhamento do parágrafo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
    Justify,
}

/// Lista do parágrafo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ListKind {
    /// • (e ◦, ▪ nos níveis de baixo).
    Bullet,
    /// – travessão.
    Dash,
    /// 1. 2. 3.
    Number,
    /// a) b) c)
    Letter,
}

/// Estilo de um parágrafo.
#[derive(Debug, Clone, PartialEq)]
pub struct ParaStyle {
    pub align: Align,
    /// Recuo da primeira linha (pode ser negativo: deslocamento), em pontos.
    pub first_indent: f32,
    pub left_indent: f32,
    pub right_indent: f32,
    /// Entrelinha em múltiplos da simples (1,0; 1,15; 1,5; 2,0).
    pub line_spacing: f32,
    /// Espaço antes e depois do parágrafo, em pontos.
    pub space_before: f32,
    pub space_after: f32,
    pub list: Option<ListKind>,
    /// Nível na lista (0 = primeiro).
    pub level: u8,
    /// Nome do estilo de parágrafo ("Corpo", "Título 1"...).
    pub name: String,
    /// Começa numa página nova.
    pub page_break_before: bool,
    /// A última tabulação vai até a margem direita com pontos (sumário: "Título ..... 5").
    pub leader: bool,
}

impl Default for ParaStyle {
    fn default() -> Self {
        Self {
            align: Align::Left,
            first_indent: 0.0,
            left_indent: 0.0,
            right_indent: 0.0,
            line_spacing: 1.0,
            space_before: 0.0,
            space_after: 0.0,
            list: None,
            level: 0,
            name: String::new(),
            page_break_before: false,
            leader: false,
        }
    }
}

/// Imagem num parágrafo próprio (alinhada pelo parágrafo).
#[derive(Debug, Clone)]
pub struct Image {
    /// O arquivo como veio (PNG ou JPEG), para gravar sem perder qualidade.
    pub data: Rc<Vec<u8>>,
    /// "png" ou "jpeg".
    pub format: &'static str,
    pub pixmap: Rc<Pixmap>,
    /// Tamanho mostrado, em pontos.
    pub width: f32,
    pub height: f32,
}

impl PartialEq for Image {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.data, &other.data) && self.width == other.width && self.height == other.height
    }
}

impl Image {
    /// Lê uma imagem PNG ou JPEG; o tamanho começa no da imagem a 96 pontos por
    /// polegada, limitado a `max_width`.
    pub fn from_bytes(data: Vec<u8>, max_width: f32) -> Option<Self> {
        let format = match image::guess_format(&data).ok()? {
            image::ImageFormat::Png => "png",
            image::ImageFormat::Jpeg => "jpeg",
            _ => return None,
        };
        let decoded = image::load_from_memory(&data).ok()?.to_rgba8();
        let (w, h) = decoded.dimensions();
        let mut pixmap = Pixmap::new(w, h)?;
        for (dst, src) in pixmap.pixels_mut().iter_mut().zip(decoded.pixels()) {
            let [r, g, b, a] = src.0;
            let pm = |c: u8| ((c as u16 * a as u16 + 127) / 255) as u8;
            *dst = tiny_skia::PremultipliedColorU8::from_rgba(pm(r), pm(g), pm(b), a).unwrap_or(tiny_skia::PremultipliedColorU8::TRANSPARENT);
        }
        let (mut width, mut height) = (w as f32 * 0.75, h as f32 * 0.75);
        if width > max_width {
            height *= max_width / width;
            width = max_width;
        }
        Some(Self { data: Rc::new(data), format, pixmap: Rc::new(pixmap), width, height })
    }
}

/// Uma célula de tabela: um documento pequeno (parágrafos com formatação) e a cor de
/// fundo.
#[derive(Debug, Clone, PartialEq)]
pub struct Cell {
    pub doc: Doc,
    pub shade: Option<[u8; 3]>,
}

impl Cell {
    pub fn empty(style: &CharStyle) -> Self {
        Self::from_paras(vec![Paragraph::new("", style.clone(), ParaStyle::default())])
    }

    /// Célula com os parágrafos (a "página" dela não tem margens nem números).
    pub fn from_paras(mut paras: Vec<Paragraph>) -> Self {
        if paras.is_empty() {
            paras.push(Paragraph::new("", CharStyle::default(), ParaStyle::default()));
        }
        let page = PageSetup { margins: [0.0; 4], header: String::new(), footer: String::new(), numbers: PageNumbers::default(), ..PageSetup::default() };
        Self { doc: Doc { paras, page, notes: Vec::new() }, shade: None }
    }
}

/// Tabela num parágrafo dela (como a imagem): larguras das colunas (pontos), linhas de
/// células e as bordas.
#[derive(Debug, Clone, PartialEq)]
pub struct Table {
    pub cols: Vec<f32>,
    pub rows: Vec<Vec<Cell>>,
    pub borders: bool,
}

impl Table {
    /// Tabela vazia de `rows` × `cols` ocupando a largura `width`.
    pub fn new(rows: usize, cols: usize, width: f32, style: &CharStyle) -> Self {
        let cols = cols.max(1);
        Self { cols: vec![width / cols as f32; cols], rows: (0..rows.max(1)).map(|_| (0..cols).map(|_| Cell::empty(style)).collect()).collect(), borders: true }
    }

    pub fn width(&self) -> f32 {
        self.cols.iter().sum()
    }

    /// Ajusta as colunas à largura `width`, mantendo as proporções.
    pub fn fit(&mut self, width: f32) {
        let total = self.width().max(1.0);
        if (total - width).abs() > 0.5 {
            for c in &mut self.cols {
                *c *= width / total;
            }
        }
    }

    pub fn insert_row(&mut self, at: usize, style: &CharStyle) {
        let n = self.cols.len();
        self.rows.insert(at.min(self.rows.len()), (0..n).map(|_| Cell::empty(style)).collect());
    }

    pub fn insert_col(&mut self, at: usize, style: &CharStyle) {
        let at = at.min(self.cols.len());
        let w = self.width() / (self.cols.len() + 1) as f32;
        let total = self.width();
        self.cols.insert(at, w);
        self.fit(total);
        for row in &mut self.rows {
            row.insert(at, Cell::empty(style));
        }
    }

    pub fn remove_row(&mut self, at: usize) {
        if self.rows.len() > 1 && at < self.rows.len() {
            self.rows.remove(at);
        }
    }

    pub fn remove_col(&mut self, at: usize) {
        if self.cols.len() > 1 && at < self.cols.len() {
            let total = self.width();
            self.cols.remove(at);
            self.fit(total);
            for row in &mut self.rows {
                row.remove(at);
            }
        }
    }
}

/// Um trecho de `len` bytes com o mesmo estilo.
#[derive(Debug, Clone, PartialEq)]
pub struct Span {
    pub len: usize,
    pub style: CharStyle,
}

/// O caractere que marca uma imagem no texto do parágrafo dela.
pub const OBJECT: char = '\u{FFFC}';
/// Quebra de linha dentro do parágrafo (Shift+Enter).
pub const LINE_BREAK: char = '\u{2028}';
/// Referência de uma nota de rodapé no texto (o número é desenhado no lugar dele).
pub const NOTE: char = '\u{E001}';

/// Um parágrafo: o texto, os trechos de estilo (cobrem o texto inteiro, sempre ao
/// menos um, mesmo vazio) e o estilo do parágrafo.
#[derive(Debug, Clone, PartialEq)]
pub struct Paragraph {
    pub text: String,
    pub spans: Vec<Span>,
    pub style: ParaStyle,
    pub image: Option<Image>,
    pub table: Option<Box<Table>>,
}

impl Paragraph {
    pub fn new(text: &str, style: CharStyle, para: ParaStyle) -> Self {
        Self { text: text.to_string(), spans: vec![Span { len: text.len(), style }], style: para, image: None, table: None }
    }

    pub fn image(image: Image, char_style: CharStyle, mut para: ParaStyle) -> Self {
        if para.align == Align::Justify {
            para.align = Align::Center;
        }
        let text = OBJECT.to_string();
        Self { spans: vec![Span { len: text.len(), style: char_style }], text, style: para, image: Some(image), table: None }
    }

    /// Parágrafo de uma tabela (o caractere de objeto no texto, a tabela à parte).
    pub fn table(table: Table, char_style: CharStyle, mut para: ParaStyle) -> Self {
        para.list = None;
        para.first_indent = 0.0;
        let text = OBJECT.to_string();
        Self { spans: vec![Span { len: text.len(), style: char_style }], text, style: para, image: None, table: Some(Box::new(table)) }
    }

    /// É um objeto (imagem ou tabela) que ocupa o parágrafo todo.
    pub fn is_object(&self) -> bool {
        self.image.is_some() || self.table.is_some()
    }

    /// Estilo de quem digita na posição `off`: o do caractere antes dela (no começo,
    /// o primeiro).
    pub fn style_at(&self, off: usize) -> &CharStyle {
        let mut pos = 0;
        for span in &self.spans {
            if off <= pos + span.len && (off > pos || pos == 0) && span.len > 0 {
                return &span.style;
            }
            pos += span.len;
        }
        &self.spans.last().expect("parágrafo sem trecho").style
    }

    /// Junta trechos vizinhos iguais e tira os vazios (fica um, se o texto é vazio).
    fn normalize(&mut self) {
        let mut out: Vec<Span> = Vec::with_capacity(self.spans.len());
        let fallback = self.spans.first().map(|s| s.style.clone()).unwrap_or_default();
        for span in self.spans.drain(..) {
            if span.len == 0 {
                continue;
            }
            match out.last_mut() {
                Some(last) if last.style == span.style => last.len += span.len,
                _ => out.push(span),
            }
        }
        if out.is_empty() {
            out.push(Span { len: 0, style: fallback });
        }
        self.spans = out;
    }

    /// Divide o trecho que atravessa `off`, para `off` ser o começo de um trecho.
    fn split_span_at(&mut self, off: usize) {
        let mut pos = 0;
        for i in 0..self.spans.len() {
            let len = self.spans[i].len;
            if off > pos && off < pos + len {
                let style = self.spans[i].style.clone();
                self.spans[i].len = off - pos;
                self.spans.insert(i + 1, Span { len: pos + len - off, style });
                return;
            }
            pos += len;
        }
    }

    /// Insere `s` em `off` com o estilo `style`.
    fn insert(&mut self, off: usize, s: &str, style: &CharStyle) {
        if s.is_empty() {
            return;
        }
        self.text.insert_str(off, s);
        if self.spans.len() == 1 && self.spans[0].len == 0 {
            self.spans[0] = Span { len: s.len(), style: style.clone() };
            return;
        }
        self.split_span_at(off);
        let mut pos = 0;
        let mut index = self.spans.len();
        for (i, span) in self.spans.iter().enumerate() {
            if pos == off {
                index = i;
                break;
            }
            pos += span.len;
        }
        self.spans.insert(index, Span { len: s.len(), style: style.clone() });
        self.normalize();
    }

    /// Tira os bytes `a..b`.
    fn remove(&mut self, a: usize, b: usize) {
        if a >= b {
            return;
        }
        self.split_span_at(a);
        self.split_span_at(b);
        let mut pos = 0;
        for span in &mut self.spans {
            if pos >= a && pos + span.len <= b {
                let len = span.len;
                span.len = 0;
                pos += len;
            } else {
                pos += span.len;
            }
        }
        self.text.replace_range(a..b, "");
        if !self.text.contains(OBJECT) {
            self.image = None;
            self.table = None;
        }
        self.normalize();
    }

    /// O parágrafo a partir de `off` (o resto fica neste).
    fn split_off(&mut self, off: usize) -> Paragraph {
        self.split_span_at(off);
        let mut pos = 0;
        let mut tail_spans = Vec::new();
        let mut keep = Vec::new();
        for span in self.spans.drain(..) {
            if pos >= off {
                tail_spans.push(span.clone());
            } else {
                keep.push(span.clone());
            }
            pos += span.len;
        }
        let last_style = keep.last().or(tail_spans.first()).map(|s| s.style.clone()).unwrap_or_default();
        let tail_text = self.text.split_off(off);
        let mut tail = Paragraph { text: tail_text, spans: tail_spans, style: self.style.clone(), image: None, table: None };
        if tail.spans.is_empty() {
            tail.spans.push(Span { len: 0, style: last_style.clone() });
        }
        self.spans = keep;
        if self.spans.is_empty() {
            self.spans.push(Span { len: 0, style: last_style });
        }
        if self.is_object() {
            if tail.text.contains(OBJECT) {
                tail.image = self.image.take();
                tail.table = self.table.take();
            } else if !self.text.contains(OBJECT) {
                self.image = None;
                self.table = None;
            }
        }
        self.normalize();
        tail.normalize();
        tail
    }

    /// Junta `other` no fim deste.
    fn append(&mut self, other: Paragraph) {
        if self.text.is_empty() {
            self.spans.clear();
        }
        self.text.push_str(&other.text);
        if !other.text.is_empty() {
            self.spans.extend(other.spans);
        }
        if self.image.is_none() {
            self.image = other.image;
        }
        if self.table.is_none() {
            self.table = other.table;
        }
        self.normalize();
    }

    /// Trechos `(início, fim, estilo)` em bytes.
    pub fn runs(&self) -> impl Iterator<Item = (usize, usize, &CharStyle)> {
        let mut pos = 0;
        self.spans.iter().map(move |s| {
            let r = (pos, pos + s.len, &s.style);
            pos += s.len;
            r
        })
    }
}

/// Posição no documento: parágrafo e byte dentro do texto dele.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Hash)]
pub struct Pos {
    pub para: usize,
    pub off: usize,
}

impl Pos {
    pub fn new(para: usize, off: usize) -> Self {
        Self { para, off }
    }
}

/// Numeração das páginas.
#[derive(Debug, Clone, PartialEq)]
pub struct PageNumbers {
    pub show: bool,
    /// Canto: "top-right", "bottom-center" ou "bottom-right".
    pub position: NumberPosition,
    /// Primeira página (contando de 1) que mostra o número; as anteriores contam mas não
    /// mostram (a ABNT conta desde a folha de rosto e mostra a partir da introdução).
    pub show_from: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumberPosition {
    TopRight,
    BottomCenter,
    BottomRight,
}

impl Default for PageNumbers {
    fn default() -> Self {
        Self { show: false, position: NumberPosition::BottomCenter, show_from: 1 }
    }
}

/// Página: tamanho e margens (pontos), cabeçalho, rodapé e numeração.
#[derive(Debug, Clone, PartialEq)]
pub struct PageSetup {
    pub width: f32,
    pub height: f32,
    /// Margens: em cima, à direita, embaixo, à esquerda.
    pub margins: [f32; 4],
    pub header: String,
    pub footer: String,
    pub numbers: PageNumbers,
}

impl PageSetup {
    pub fn a4() -> Self {
        Self { width: cm(21.0), height: cm(29.7), margins: [cm(2.5), cm(2.5), cm(2.5), cm(2.5)], header: String::new(), footer: String::new(), numbers: PageNumbers::default() }
    }

    pub fn letter() -> Self {
        Self { width: 612.0, height: 792.0, margins: [72.0; 4], ..Self::a4() }
    }

    /// Largura do texto (sem as margens).
    pub fn content_width(&self) -> f32 {
        (self.width - self.margins[1] - self.margins[3]).max(36.0)
    }

    /// A página está deitada.
    pub fn landscape(&self) -> bool {
        self.width > self.height
    }
}

impl Default for PageSetup {
    fn default() -> Self {
        Self::a4()
    }
}

/// O documento.
#[derive(Debug, Clone, PartialEq)]
pub struct Doc {
    pub paras: Vec<Paragraph>,
    pub page: PageSetup,
    /// Notas de rodapé: id e texto (as referências ficam no texto, com `CharStyle::note`).
    pub notes: Vec<(u32, String)>,
}

impl Default for Doc {
    fn default() -> Self {
        Self::new()
    }
}

impl Doc {
    pub fn new() -> Self {
        Self { paras: vec![Paragraph::new("", CharStyle::default(), ParaStyle::default())], page: PageSetup::default(), notes: Vec::new() }
    }

    /// Documento com o texto puro (um parágrafo por linha).
    pub fn from_plain(text: &str, style: CharStyle) -> Self {
        let paras = text.split('\n').map(|l| Paragraph::new(l.trim_end_matches('\r'), style.clone(), ParaStyle::default())).collect();
        Self { paras, page: PageSetup::default(), notes: Vec::new() }
    }

    pub fn end(&self) -> Pos {
        let last = self.paras.len() - 1;
        Pos::new(last, self.paras[last].text.len())
    }

    /// A posição, presa ao documento e a uma fronteira de caractere.
    pub fn clamp(&self, p: Pos) -> Pos {
        let para = p.para.min(self.paras.len() - 1);
        let text = &self.paras[para].text;
        let mut off = p.off.min(text.len());
        while !text.is_char_boundary(off) {
            off -= 1;
        }
        Pos::new(para, off)
    }

    /// Caractere seguinte (passa para o próximo parágrafo no fim).
    pub fn next(&self, p: Pos) -> Pos {
        let text = &self.paras[p.para].text;
        match text[p.off..].chars().next() {
            Some(c) => Pos::new(p.para, p.off + c.len_utf8()),
            None if p.para + 1 < self.paras.len() => Pos::new(p.para + 1, 0),
            None => p,
        }
    }

    /// Caractere anterior.
    pub fn prev(&self, p: Pos) -> Pos {
        let text = &self.paras[p.para].text;
        match text[..p.off].chars().next_back() {
            Some(c) => Pos::new(p.para, p.off - c.len_utf8()),
            None if p.para > 0 => Pos::new(p.para - 1, self.paras[p.para - 1].text.len()),
            None => p,
        }
    }

    /// Fim da palavra seguinte (Ctrl+→).
    pub fn next_word(&self, p: Pos) -> Pos {
        let text = &self.paras[p.para].text;
        if p.off >= text.len() {
            return self.next(p);
        }
        let rest = &text[p.off..];
        let mut seen = false;
        for (i, c) in rest.char_indices() {
            let word = c.is_alphanumeric() || c == '_';
            if word {
                seen = true;
            } else if seen {
                return Pos::new(p.para, p.off + i);
            }
        }
        Pos::new(p.para, text.len())
    }

    /// Começo da palavra anterior (Ctrl+←).
    pub fn prev_word(&self, p: Pos) -> Pos {
        if p.off == 0 {
            return self.prev(p);
        }
        let text = &self.paras[p.para].text[..p.off];
        let mut seen = false;
        for (i, c) in text.char_indices().rev() {
            let word = c.is_alphanumeric() || c == '_';
            if word {
                seen = true;
            } else if seen {
                return Pos::new(p.para, i + c.len_utf8());
            }
        }
        Pos::new(p.para, 0)
    }

    /// A palavra em volta de `p` (duplo clique).
    pub fn word_at(&self, p: Pos) -> (Pos, Pos) {
        let text = &self.paras[p.para].text;
        let is_word = |c: char| c.is_alphanumeric() || c == '_';
        let mut a = p.off;
        while let Some(c) = text[..a].chars().next_back() {
            if !is_word(c) {
                break;
            }
            a -= c.len_utf8();
        }
        let mut b = p.off;
        while let Some(c) = text[b..].chars().next() {
            if !is_word(c) {
                break;
            }
            b += c.len_utf8();
        }
        if a == b {
            b = self.next(p).off.max(b);
            if self.next(p).para != p.para {
                b = p.off;
            }
        }
        (Pos::new(p.para, a), Pos::new(p.para, b))
    }

    /// Estilo para o que for digitado em `p` (depois de uma nota, o texto não é nota).
    pub fn style_at(&self, p: Pos) -> CharStyle {
        let mut s = self.paras[p.para].style_at(p.off).clone();
        s.note = None;
        s
    }

    /// Texto de uma nota de rodapé.
    pub fn note_text(&self, id: u32) -> Option<&str> {
        self.notes.iter().find(|(i, _)| *i == id).map(|(_, t)| t.as_str())
    }

    /// Guarda uma nota nova e devolve o id dela.
    pub fn add_note(&mut self, text: &str) -> u32 {
        let id = self.notes.iter().map(|(i, _)| *i).max().unwrap_or(0) + 1;
        self.notes.push((id, text.to_string()));
        id
    }

    /// Insere a referência de uma nota (com o texto dela) em `p`; devolve a posição depois.
    pub fn insert_note(&mut self, p: Pos, text: &str) -> Pos {
        let id = self.add_note(text);
        let mut style = self.style_at(p);
        style.note = Some(id);
        style.baseline = Baseline::Normal;
        self.insert_text(p, &NOTE.to_string(), &style)
    }

    /// A nota cuja referência está logo antes ou logo depois de `p`.
    pub fn note_near(&self, p: Pos) -> Option<u32> {
        let para = &self.paras[p.para];
        let mut best = None;
        for (a, b, s) in para.runs() {
            if let Some(id) = s.note {
                if p.off >= a && p.off <= b {
                    best = Some(id);
                }
            }
        }
        best
    }

    /// Insere texto (com `\n` separando parágrafos) e devolve a posição depois dele.
    pub fn insert_text(&mut self, p: Pos, s: &str, style: &CharStyle) -> Pos {
        let mut pos = p;
        for (i, piece) in s.split('\n').enumerate() {
            if i > 0 {
                pos = self.split(pos);
            }
            let piece = piece.trim_end_matches('\r');
            // Texto num parágrafo de imagem vai para um parágrafo novo ao lado.
            if self.paras[pos.para].is_object() && !piece.is_empty() {
                pos = self.split(pos);
                if self.paras[pos.para].is_object() {
                    // Estava antes da imagem: o novo parágrafo é o de cima.
                    pos = Pos::new(pos.para - 1, 0);
                }
            }
            self.paras[pos.para].insert(pos.off, piece, style);
            pos.off += piece.len();
        }
        pos
    }

    /// Quebra o parágrafo em `p` (Enter); devolve o começo do novo.
    pub fn split(&mut self, p: Pos) -> Pos {
        let tail = self.paras[p.para].split_off(p.off);
        self.paras.insert(p.para + 1, tail);
        // Um título seguido de Enter continua no corpo (como nos processadores de texto).
        let next = &mut self.paras[p.para + 1];
        if next.text.is_empty() && next.style.name.starts_with("Título") {
            next.style.name = String::new();
        }
        Pos::new(p.para + 1, 0)
    }

    /// Apaga o intervalo (em qualquer ordem) e devolve onde ficou o cursor.
    pub fn delete(&mut self, a: Pos, b: Pos) -> Pos {
        let (a, b) = if a <= b { (a, b) } else { (b, a) };
        if a == b {
            return a;
        }
        if a.para == b.para {
            self.paras[a.para].remove(a.off, b.off);
            return a;
        }
        let tail = self.paras[b.para].split_off(b.off);
        let len = self.paras[a.para].text.len();
        self.paras[a.para].remove(a.off, len);
        self.paras.drain(a.para + 1..=b.para);
        self.paras[a.para].append(tail);
        a
    }

    /// Texto puro do intervalo (parágrafos separados por `\n`).
    pub fn plain(&self, a: Pos, b: Pos) -> String {
        let (a, b) = if a <= b { (a, b) } else { (b, a) };
        let mut out = String::new();
        for i in a.para..=b.para {
            let text = &self.paras[i].text;
            let from = if i == a.para { a.off } else { 0 };
            let to = if i == b.para { b.off } else { text.len() };
            if let (Some(t), true) = (&self.paras[i].table, from == 0 && to > 0) {
                // Tabela: uma linha por linha, Tab entre as células.
                let rows: Vec<String> = t.rows.iter().map(|r| r.iter().map(|c| c.doc.all_plain().replace('\n', " ")).collect::<Vec<_>>().join("\t")).collect();
                out.push_str(&rows.join("\n"));
            } else {
                out.push_str(&text[from..to].replace([OBJECT, NOTE], "").replace(LINE_BREAK, "\n"));
            }
            if i != b.para {
                out.push('\n');
            }
        }
        out
    }

    /// Texto puro do documento todo.
    pub fn all_plain(&self) -> String {
        self.plain(Pos::default(), self.end())
    }

    /// Cópia do intervalo como parágrafos (para copiar e colar com formatação).
    pub fn fragment(&self, a: Pos, b: Pos) -> Vec<Paragraph> {
        let (a, b) = if a <= b { (a, b) } else { (b, a) };
        let mut out = Vec::new();
        for i in a.para..=b.para {
            let mut p = self.paras[i].clone();
            let to = if i == b.para { b.off } else { p.text.len() };
            let _ = p.split_off(to);
            let from = if i == a.para { a.off } else { 0 };
            let tail = p.split_off(from);
            out.push(tail);
        }
        out
    }

    /// Cola parágrafos em `p`: o primeiro se junta ao parágrafo atual, o último recebe o
    /// resto dele. Devolve a posição depois do que foi colado.
    pub fn insert_fragment(&mut self, p: Pos, frag: &[Paragraph]) -> Pos {
        if frag.is_empty() {
            return p;
        }
        let tail = self.paras[p.para].split_off(p.off);
        let mut index = p.para;
        for (i, piece) in frag.iter().enumerate() {
            if i == 0 && !piece.is_object() && !self.paras[index].is_object() {
                let mut first = piece.clone();
                if !self.paras[index].text.is_empty() {
                    first.style = self.paras[index].style.clone();
                }
                self.paras[index].append(first.clone());
                if self.paras[index].text == first.text {
                    self.paras[index].style = first.style;
                }
            } else {
                index += 1;
                self.paras.insert(index, piece.clone());
            }
        }
        let end = Pos::new(index, self.paras[index].text.len());
        if !tail.text.is_empty() || tail.image.is_some() {
            if self.paras[index].is_object() || tail.is_object() {
                self.paras.insert(index + 1, tail);
            } else {
                self.paras[index].append(tail);
            }
        }
        end
    }

    /// Aplica `f` ao estilo do texto no intervalo.
    pub fn apply_char(&mut self, a: Pos, b: Pos, f: impl Fn(&mut CharStyle)) {
        let (a, b) = if a <= b { (a, b) } else { (b, a) };
        for i in a.para..=b.para {
            let para = &mut self.paras[i];
            let from = if i == a.para { a.off } else { 0 };
            let to = if i == b.para { b.off } else { para.text.len() };
            if from == to && !(para.text.is_empty()) {
                continue;
            }
            para.split_span_at(from);
            para.split_span_at(to);
            let mut pos = 0;
            for span in &mut para.spans {
                if (pos >= from && pos + span.len <= to && span.len > 0) || para.text.is_empty() {
                    f(&mut span.style);
                }
                pos += span.len;
            }
            para.normalize();
        }
    }

    /// Aplica `f` ao estilo dos parágrafos do intervalo.
    pub fn apply_para(&mut self, a: Pos, b: Pos, f: impl Fn(&mut ParaStyle)) {
        let (a, b) = if a <= b { (a, b) } else { (b, a) };
        for i in a.para..=b.para {
            f(&mut self.paras[i].style);
        }
    }

    /// Insere uma imagem em `p` (num parágrafo dela) e devolve a posição depois.
    pub fn insert_image(&mut self, p: Pos, image: Image) -> Pos {
        let style = self.style_at(p);
        let mut para_style = self.paras[p.para].style.clone();
        para_style.align = Align::Center;
        para_style.first_indent = 0.0;
        para_style.list = None;
        let at = if p.off == 0 && self.paras[p.para].text.is_empty() {
            p.para
        } else if p.off == 0 {
            self.paras.insert(p.para, Paragraph::new("", style.clone(), self.paras[p.para].style.clone()));
            p.para
        } else {
            let next = self.split(p);
            self.paras.insert(next.para, Paragraph::new("", style.clone(), self.paras[p.para].style.clone()));
            next.para
        };
        self.paras[at] = Paragraph::image(image, style.clone(), para_style);
        if at + 1 >= self.paras.len() {
            let mut body = self.paras[at].style.clone();
            body.align = Align::Left;
            self.paras.push(Paragraph::new("", style, body));
        }
        Pos::new(at + 1, 0)
    }

    /// Insere uma tabela em `p` (num parágrafo dela) e devolve a posição depois.
    pub fn insert_table(&mut self, p: Pos, rows: usize, cols: usize) -> Pos {
        let style = self.style_at(p);
        let width = self.page.content_width();
        let table = Table::new(rows, cols, width, &style);
        let body = ParaStyle { list: None, first_indent: 0.0, ..self.paras[p.para].style.clone() };
        let at = if p.off == 0 && self.paras[p.para].text.is_empty() {
            p.para
        } else if p.off == 0 {
            self.paras.insert(p.para, Paragraph::new("", style.clone(), body.clone()));
            p.para
        } else {
            let next = self.split(p);
            self.paras.insert(next.para, Paragraph::new("", style.clone(), body.clone()));
            next.para
        };
        self.paras[at] = Paragraph::table(table, style.clone(), ParaStyle { align: Align::Left, space_after: 6.0, ..ParaStyle::default() });
        if at + 1 >= self.paras.len() || self.paras[at + 1].is_object() {
            self.paras.insert(at + 1, Paragraph::new("", style, body));
        } else if self.paras[at + 1].text.is_empty() {
            // O parágrafo vazio depois da tabela não continua uma lista.
            self.paras[at + 1].style = body;
        }
        Pos::new(at + 1, 0)
    }

    /// Número de palavras e de caracteres (sem os espaços e com eles).
    pub fn counts(&self) -> (usize, usize, usize) {
        let mut words = 0;
        let mut chars = 0;
        let mut no_spaces = 0;
        for p in &self.paras {
            if let Some(t) = &p.table {
                for cell in t.rows.iter().flatten() {
                    let (w, c, n) = cell.doc.counts();
                    words += w;
                    chars += c;
                    no_spaces += n;
                }
            }
            words += p.text.split(|c: char| c.is_whitespace() || c == LINE_BREAK).filter(|w| w.chars().any(char::is_alphanumeric)).count();
            for c in p.text.chars().filter(|c| *c != OBJECT) {
                chars += 1;
                if !c.is_whitespace() {
                    no_spaces += 1;
                }
            }
        }
        (words, chars, no_spaces)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bold() -> CharStyle {
        CharStyle { bold: true, ..Default::default() }
    }

    #[test]
    fn digitar_quebrar_e_apagar() {
        let mut d = Doc::new();
        let s = CharStyle::default();
        let p = d.insert_text(Pos::default(), "Olá mundo", &s);
        assert_eq!(p, Pos::new(0, "Olá mundo".len()));
        d.split(Pos::new(0, "Olá ".len()));
        assert_eq!(d.paras.len(), 2);
        assert_eq!(d.paras[0].text, "Olá ");
        assert_eq!(d.paras[1].text, "mundo");
        let at = d.delete(Pos::new(0, 2), Pos::new(1, 2));
        assert_eq!(at, Pos::new(0, 2));
        assert_eq!(d.paras.len(), 1);
        assert_eq!(d.paras[0].text, "Olndo");
        assert_eq!(d.paras[0].spans.iter().map(|s| s.len).sum::<usize>(), d.paras[0].text.len());
    }

    #[test]
    fn estilos_em_trechos() {
        let mut d = Doc::new();
        d.insert_text(Pos::default(), "abcdef", &CharStyle::default());
        d.apply_char(Pos::new(0, 2), Pos::new(0, 4), |s| s.bold = true);
        assert_eq!(d.paras[0].spans.len(), 3);
        assert!(d.paras[0].spans[1].style.bold);
        // Digitar no fim do trecho em negrito continua em negrito.
        let style = d.style_at(Pos::new(0, 4));
        assert!(style.bold);
        d.insert_text(Pos::new(0, 4), "X", &style);
        assert_eq!(d.paras[0].spans[1].len, 3);
        // Tirar o negrito junta tudo de novo.
        d.apply_char(Pos::new(0, 0), Pos::new(0, 7), |s| s.bold = false);
        assert_eq!(d.paras[0].spans.len(), 1);
        assert_eq!(d.style_at(Pos::new(0, 0)), CharStyle::default());
        let _ = bold();
    }

    #[test]
    fn copiar_e_colar_com_formatacao() {
        let mut d = Doc::new();
        d.insert_text(Pos::default(), "um\ndois\ntrês", &CharStyle::default());
        d.apply_char(Pos::new(1, 0), Pos::new(1, 4), |s| s.italic = true);
        let frag = d.fragment(Pos::new(0, 1), Pos::new(1, 2));
        assert_eq!(frag.len(), 2);
        assert_eq!(frag[0].text, "m");
        assert_eq!(frag[1].text, "do");
        assert!(frag[1].spans[0].style.italic);
        let end = d.insert_fragment(Pos::new(2, 2), &frag);
        assert_eq!(d.paras[2].text, "trm");
        assert_eq!(d.paras[3].text, "doês");
        assert_eq!(end, Pos::new(3, 2));
        assert_eq!(d.plain(Pos::new(0, 0), d.end()), "um\ndois\ntrm\ndoês");
    }

    #[test]
    fn palavras_e_contagem() {
        let mut d = Doc::new();
        d.insert_text(Pos::default(), "Era uma vez, no mide.", &CharStyle::default());
        assert_eq!(d.word_at(Pos::new(0, 5)), (Pos::new(0, 4), Pos::new(0, 7)));
        assert_eq!(d.next_word(Pos::new(0, 0)), Pos::new(0, 3));
        assert_eq!(d.prev_word(Pos::new(0, 7)), Pos::new(0, 4));
        assert_eq!(d.counts().0, 5);
    }
}

#[cfg(test)]
mod external_files {
    /// Lê arquivos feitos por outros programas (`MIDE_READ_IN=a.docx:b.odt`) e mostra
    /// o texto e os estilos, para conferir a compatibilidade à mão.
    #[test]
    #[ignore]
    fn ler_arquivos_externos() {
        let Ok(list) = std::env::var("MIDE_READ_IN") else {
            return;
        };
        for path in list.split(':') {
            let data = std::fs::read(path).expect("arquivo");
            let doc = if path.ends_with(".docx") { super::docx::read(&data) } else { super::odt::read(&data) }.expect("documento");
            println!("== {path}: margens {:?}, números {:?}, cabeçalho {:?}", doc.page.margins.map(super::to_cm), doc.page.numbers, doc.page.header);
            for p in &doc.paras {
                let s = &p.spans[0].style;
                println!("[{}|{:?}|{:?}|{:.2}] {} ({} {} b={} i={} u={} cor={:?})", p.style.name, p.style.align, p.style.list, p.style.line_spacing, p.text, s.family, s.size, s.bold, s.italic, s.underline, s.color);
            }
        }
    }
}
