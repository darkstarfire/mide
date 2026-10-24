//! Modo formatado do Editor de Texto, como o TextEdit do Mac: páginas na tela, barra de
//! formatação (estilos, fonte, tamanho, cores, negrito/itálico/sublinhado,
//! alinhamento, espaçamento, listas, imagem), configurar página e as normas ABNT.

use std::time::{Duration, Instant};

use mide::kit::{ButtonStyle, Key, MenuGroup, MenuItem, Ui};
use mide::richtext::{
    cm,
    layout::{Layout, Layouter},
    render, to_cm, Align, Baseline, CharStyle, Doc, Image, ListKind, NumberPosition, PageSetup, ParaStyle, Paragraph, Pos, LINE_BREAK,
};

/// Espaço entre as páginas e em volta delas (pontos da tela).
const GAP: f32 = 24.0;
/// Pontos da tela por ponto tipográfico no zoom de 100% (96 por polegada, como no Word).
const SCREEN: f32 = 96.0 / 72.0;
/// Barra de formatação, abaixo da barra de ferramentas.
pub const FORMAT_H: f32 = 44.0;
/// Rodapé (página, palavras, zoom).
pub const STATUS_H: f32 = 26.0;

/// Tamanhos do menu de tamanho.
const SIZES: [f32; 16] = [8.0, 9.0, 10.0, 11.0, 12.0, 13.0, 14.0, 16.0, 18.0, 20.0, 24.0, 28.0, 36.0, 48.0, 64.0, 72.0];

/// Cores dos poços de cor (texto e realce).
const COLORS: [[u8; 3]; 16] = [
    [0, 0, 0],
    [84, 84, 88],
    [142, 142, 147],
    [199, 199, 204],
    [255, 255, 255],
    [255, 59, 48],
    [255, 149, 0],
    [255, 204, 0],
    [52, 199, 89],
    [0, 199, 190],
    [0, 122, 255],
    [88, 86, 214],
    [175, 82, 222],
    [255, 45, 85],
    [162, 132, 94],
    [0, 64, 128],
];

/// Cores de realce (marca-texto).
const HIGHLIGHTS: [[u8; 3]; 6] = [[255, 241, 118], [178, 235, 160], [160, 214, 255], [255, 190, 220], [255, 205, 150], [215, 215, 220]];

/// Estilos de parágrafo do menu ¶ (nome e como aplica).
const STYLES: [&str; 8] = ["Corpo", "Título", "Título 1", "Título 2", "Título 3", "Citação", "Legenda", "Referência"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pop {
    Styles,
    Fonts,
    Size,
    Color,
    Highlight,
    Spacing,
    List,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Typing,
    Delete,
}

struct Snapshot {
    doc: Doc,
    cursor: Pos,
    anchor: Pos,
}

/// Altura da barra de busca.
const FIND_H: f32 = 44.0;

/// Buscar e substituir.
#[derive(Default)]
pub struct FindBar {
    query: String,
    replace: String,
    /// Diferenciar maiúsculas e minúsculas.
    case: bool,
    focus: bool,
}

/// Configurar página (diálogo), com os valores em centímetros.
struct PageDialog {
    paper: usize,
    landscape: bool,
    margins: [String; 4],
    header: String,
    footer: String,
    numbers: bool,
    position: usize,
    from: String,
}

/// Imprimir: destino (0 = Salvar como PDF; os outros, as impressoras), cópias, páginas
/// (0 todas, 1 a atual, 2 o intervalo), frente e verso e a página da prévia.
struct PrintDialog {
    printers: Vec<String>,
    dest: usize,
    copies: i64,
    pages: usize,
    from: String,
    to: String,
    duplex: bool,
    preview: usize,
}

/// Impressoras do sistema (CUPS), com a padrão primeiro.
fn printers() -> Vec<String> {
    let run = |args: &[&str]| std::process::Command::new("lpstat").args(args).env("LC_ALL", "C").output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    let mut list: Vec<String> = run(&["-e"]).lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
    let default = run(&["-d"]).split(':').nth(1).map(|d| d.trim().to_string()).filter(|d| !d.is_empty());
    if let Some(d) = default {
        list.retain(|p| *p != d);
        list.insert(0, d);
    }
    list
}

/// Diálogo das normas ABNT.
struct AbntDialog {
    font: usize,
    numbers_from: String,
}

pub struct Rich {
    pub doc: Doc,
    layouter: Layouter,
    layout: Layout,
    stale: bool,
    pub cursor: Pos,
    pub anchor: Pos,
    goal_x: Option<f32>,
    /// Estilo para o próximo texto digitado (negrito ligado sem seleção, por exemplo).
    typing: Option<CharStyle>,
    scroll: f32,
    pub zoom: f32,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    last_kind: Option<(Kind, Instant)>,
    version: u64,
    saved_version: u64,
    selecting: bool,
    clicks: (Instant, u32, Pos),
    clip: Option<(String, Vec<Paragraph>)>,
    pop: Option<Pop>,
    pop_anchor: (f32, f32, f32, f32),
    font_query: String,
    families: Vec<String>,
    page_dialog: Option<PageDialog>,
    print_dialog: Option<PrintDialog>,
    /// Pedido ao app: escolher onde salvar o PDF (com as páginas).
    pub wants_pdf_path: Option<Option<(usize, usize)>>,
    /// PDF a gravar (caminho e páginas), feito no próximo quadro (precisa das fontes).
    pub pdf_request: Option<(std::path::PathBuf, Option<(usize, usize)>)>,
    /// Impressão a mandar: impressora, cópias, páginas e frente e verso.
    print_job: Option<(String, i64, Option<(usize, usize)>, bool)>,
    abnt_dialog: Option<AbntDialog>,
    reveal: bool,
    blink: Instant,
    paste_pending: bool,
    /// Pedido ao app (abrir o seletor de imagem).
    pub wants_image: bool,
    pub find: Option<FindBar>,
    /// Passadas que faltam para acertar os números do sumário (cada uma depois de quebrar
    /// o documento de novo: o sumário pode mudar as páginas).
    toc_passes: u8,
    pub message: Option<String>,
}

fn same_style_name(a: &str, b: &str) -> bool {
    a == b || (a.is_empty() && b == "Corpo")
}

impl Rich {
    pub fn new(doc: Doc) -> Self {
        Self {
            doc,
            layouter: Layouter::default(),
            layout: Layout::default(),
            stale: true,
            cursor: Pos::default(),
            anchor: Pos::default(),
            goal_x: None,
            typing: None,
            scroll: 0.0,
            zoom: 1.0,
            undo: Vec::new(),
            redo: Vec::new(),
            last_kind: None,
            version: 0,
            saved_version: 0,
            selecting: false,
            clicks: (Instant::now() - Duration::from_secs(5), 0, Pos::default()),
            clip: None,
            pop: None,
            pop_anchor: (0.0, 0.0, 0.0, 0.0),
            font_query: String::new(),
            families: Vec::new(),
            page_dialog: None,
            print_dialog: None,
            wants_pdf_path: None,
            pdf_request: None,
            print_job: None,
            abnt_dialog: None,
            reveal: false,
            blink: Instant::now(),
            paste_pending: false,
            wants_image: false,
            find: None,
            toc_passes: 0,
            message: None,
        }
    }

    pub fn modified(&self) -> bool {
        self.version != self.saved_version
    }

    /// Marca o documento como alterado (convertido de texto simples, ainda não salvo).
    pub fn undo_point(&mut self) {
        self.version += 1;
    }

    pub fn mark_saved(&mut self) {
        self.saved_version = self.version;
    }

    fn sel(&self) -> Option<(Pos, Pos)> {
        (self.cursor != self.anchor).then(|| if self.cursor < self.anchor { (self.cursor, self.anchor) } else { (self.anchor, self.cursor) })
    }

    /// Intervalo para formatar: a seleção, ou o cursor.
    fn range(&self) -> (Pos, Pos) {
        self.sel().unwrap_or((self.cursor, self.cursor))
    }

    /// Antes de mudar o documento: ponto de desfazer (digitação seguida fica num só).
    fn begin(&mut self, kind: Option<Kind>) {
        let now = Instant::now();
        let join = kind.is_some() && self.last_kind.is_some_and(|(k, t)| Some(k) == kind && now.duration_since(t) < Duration::from_millis(1500));
        if !join {
            self.undo.push(Snapshot { doc: self.doc.clone(), cursor: self.cursor, anchor: self.anchor });
            if self.undo.len() > 300 {
                self.undo.remove(0);
            }
        }
        self.last_kind = kind.map(|k| (k, now));
        self.redo.clear();
        self.stale = true;
        self.version += 1;
        self.reveal = true;
        self.blink = now;
        self.goal_x = None;
    }

    pub fn undo(&mut self, redo: bool) {
        let (from, to) = if redo { (&mut self.redo, &mut self.undo) } else { (&mut self.undo, &mut self.redo) };
        if let Some(snap) = from.pop() {
            to.push(Snapshot { doc: std::mem::replace(&mut self.doc, snap.doc), cursor: self.cursor, anchor: self.anchor });
            self.cursor = self.doc.clamp(snap.cursor);
            self.anchor = self.doc.clamp(snap.anchor);
            self.stale = true;
            self.version += 1;
            self.reveal = true;
            self.last_kind = None;
        }
    }

    fn type_text(&mut self, s: &str) {
        self.begin(Some(Kind::Typing));
        if let Some((a, b)) = self.sel() {
            self.cursor = self.doc.delete(a, b);
        }
        let style = self.typing.take().unwrap_or_else(|| self.doc.style_at(self.cursor));
        self.cursor = self.doc.insert_text(self.cursor, s, &style);
        self.anchor = self.cursor;
    }

    fn enter(&mut self) {
        self.begin(None);
        if let Some((a, b)) = self.sel() {
            self.cursor = self.doc.delete(a, b);
        }
        let para = &self.doc.paras[self.cursor.para];
        // Enter numa linha de lista vazia termina a lista.
        if para.style.list.is_some() && para.text.is_empty() {
            self.doc.paras[self.cursor.para].style.list = None;
            self.anchor = self.cursor;
            return;
        }
        let typing = self.typing.take();
        self.cursor = self.doc.split(self.cursor);
        if let Some(t) = typing {
            self.typing = Some(t);
        }
        self.anchor = self.cursor;
    }

    fn backspace(&mut self, word: bool) {
        if let Some((a, b)) = self.sel() {
            self.begin(Some(Kind::Delete));
            self.cursor = self.doc.delete(a, b);
            self.anchor = self.cursor;
            return;
        }
        // No começo de um item de lista, o primeiro Backspace tira o marcador.
        let para = &self.doc.paras[self.cursor.para];
        if self.cursor.off == 0 && para.style.list.is_some() {
            self.begin(None);
            self.doc.paras[self.cursor.para].style.list = None;
            return;
        }
        let from = if word { self.doc.prev_word(self.cursor) } else { self.doc.prev(self.cursor) };
        if from == self.cursor {
            return;
        }
        self.begin(Some(Kind::Delete));
        self.cursor = self.doc.delete(from, self.cursor);
        self.anchor = self.cursor;
    }

    fn delete_forward(&mut self, word: bool) {
        if let Some((a, b)) = self.sel() {
            self.begin(Some(Kind::Delete));
            self.cursor = self.doc.delete(a, b);
            self.anchor = self.cursor;
            return;
        }
        let to = if word { self.doc.next_word(self.cursor) } else { self.doc.next(self.cursor) };
        if to == self.cursor {
            return;
        }
        self.begin(Some(Kind::Delete));
        self.cursor = self.doc.delete(self.cursor, to);
        self.anchor = self.cursor;
    }

    fn move_to(&mut self, p: Pos, extend: bool) {
        self.cursor = self.doc.clamp(p);
        if !extend {
            self.anchor = self.cursor;
        }
        self.reveal = true;
        self.blink = Instant::now();
        self.typing = None;
        self.last_kind = None;
    }

    /// Estilo de texto que vale agora (para os botões mostrarem o estado).
    pub fn current_style(&self) -> CharStyle {
        if let Some(t) = &self.typing {
            return t.clone();
        }
        match self.sel() {
            Some((a, _)) => {
                let next = self.doc.next(a);
                self.doc.style_at(if next.para == a.para { next } else { a })
            }
            None => self.doc.style_at(self.cursor),
        }
    }

    fn current_para(&self) -> ParaStyle {
        self.doc.paras[self.cursor.para].style.clone()
    }

    /// Muda o estilo do texto: na seleção, ou no que for digitado em seguida.
    fn char_style(&mut self, f: impl Fn(&mut CharStyle)) {
        match self.sel() {
            Some((a, b)) => {
                self.begin(None);
                self.doc.apply_char(a, b, f);
            }
            None => {
                let para = &self.doc.paras[self.cursor.para];
                if para.text.is_empty() {
                    // Parágrafo vazio: o estilo dele muda (o cursor mostra o tamanho novo).
                    self.begin(None);
                    self.doc.apply_char(self.cursor, self.cursor, &f);
                } else {
                    let mut s = self.current_style();
                    f(&mut s);
                    self.typing = Some(s);
                }
            }
        }
    }

    fn para_style(&mut self, f: impl Fn(&mut ParaStyle)) {
        let (a, b) = self.range();
        self.begin(None);
        self.doc.apply_para(a, b, f);
    }

    /// Aplica um estilo de parágrafo do menu ¶ (também a fonte e o tamanho do texto).
    fn apply_named(&mut self, name: &str) {
        let (a, b) = self.range();
        let a = Pos::new(a.para, 0);
        let b = Pos::new(b.para, self.doc.paras[b.para].text.len());
        let family = self.doc.paras[a.para].spans[0].style.family.clone();
        let body = self.body_style();
        self.begin(None);
        let (para, size, bold, italic): (ParaStyle, f32, bool, bool) = match name {
            "Título" => (ParaStyle { align: Align::Center, space_after: 12.0, ..body.clone() }, 20.0, true, false),
            "Título 1" => (ParaStyle { first_indent: 0.0, align: Align::Left, space_before: 12.0, space_after: 12.0, line_spacing: body.line_spacing, ..ParaStyle::default() }, body_size(&self.doc).max(12.0) + 4.0, true, false),
            "Título 2" => (ParaStyle { first_indent: 0.0, align: Align::Left, space_before: 12.0, space_after: 6.0, line_spacing: body.line_spacing, ..ParaStyle::default() }, body_size(&self.doc) + 2.0, true, false),
            "Título 3" => (ParaStyle { first_indent: 0.0, align: Align::Left, space_before: 6.0, space_after: 6.0, line_spacing: body.line_spacing, ..ParaStyle::default() }, body_size(&self.doc), true, true),
            // ABNT: citação longa com recuo de 4 cm, letra menor e entrelinha simples.
            "Citação" => (ParaStyle { left_indent: cm(4.0), first_indent: 0.0, align: Align::Justify, line_spacing: 1.0, space_before: 12.0, space_after: 12.0, ..ParaStyle::default() }, (body_size(&self.doc) - 2.0).max(8.0), false, false),
            "Legenda" => (ParaStyle { align: Align::Center, line_spacing: 1.0, space_after: 12.0, ..ParaStyle::default() }, (body_size(&self.doc) - 2.0).max(8.0), false, false),
            "Referência" => (ParaStyle { align: Align::Left, line_spacing: 1.0, space_after: 12.0, ..ParaStyle::default() }, body_size(&self.doc), false, false),
            _ => (body.clone(), body_size(&self.doc), false, false),
        };
        let mut para = para;
        para.name = if name == "Corpo" { String::new() } else { name.to_string() };
        self.doc.apply_para(a, b, |p| {
            let list = p.list;
            let level = p.level;
            *p = para.clone();
            if para.name.is_empty() {
                p.list = list;
                p.level = level;
            }
        });
        self.doc.apply_char(a, b, |s| {
            s.family = family.clone();
            s.size = size;
            s.bold = bold;
            s.italic = italic;
        });
    }

    /// O estilo de "Corpo" deste documento: o do primeiro parágrafo sem nome de estilo.
    fn body_style(&self) -> ParaStyle {
        self.doc.paras.iter().find(|p| p.style.name.is_empty() && p.style.list.is_none() && p.image.is_none() && !p.text.is_empty()).map(|p| ParaStyle { list: None, page_break_before: false, ..p.style.clone() }).unwrap_or_default()
    }

    pub fn select_all(&mut self) {
        self.anchor = Pos::default();
        self.cursor = self.doc.end();
    }

    pub fn copy(&mut self, ui: &mut Ui, cut: bool) {
        let Some((a, b)) = self.sel() else {
            return;
        };
        let plain = self.doc.plain(a, b);
        self.clip = Some((plain.clone(), self.doc.fragment(a, b)));
        ui.copy(&plain);
        if cut {
            self.begin(None);
            self.cursor = self.doc.delete(a, b);
            self.anchor = self.cursor;
        }
    }

    pub fn paste(&mut self, ui: &mut Ui) {
        self.paste_pending = true;
        ui.request_paste();
    }

    fn paste_text(&mut self, text: &str) {
        // O que este editor copiou volta com a formatação.
        let rich = self.clip.as_ref().filter(|(plain, _)| plain.replace("\r\n", "\n") == text.replace("\r\n", "\n")).map(|(_, f)| f.clone());
        self.begin(None);
        if let Some((a, b)) = self.sel() {
            self.cursor = self.doc.delete(a, b);
        }
        self.cursor = match rich {
            Some(frag) => self.doc.insert_fragment(self.cursor, &frag),
            None => {
                let style = self.doc.style_at(self.cursor);
                self.doc.insert_text(self.cursor, text, &style)
            }
        };
        self.anchor = self.cursor;
    }

    pub fn insert_image(&mut self, path: &std::path::Path) {
        let Ok(data) = std::fs::read(path) else {
            self.message = Some("Não foi possível ler a imagem.".into());
            return;
        };
        match Image::from_bytes(data, self.doc.page.content_width()) {
            Some(img) => {
                self.begin(None);
                if let Some((a, b)) = self.sel() {
                    self.cursor = self.doc.delete(a, b);
                }
                self.cursor = self.doc.insert_image(self.cursor, img);
                self.anchor = self.cursor;
                tracing::info!("editor: imagem inserida ({})", path.display());
            }
            None => self.message = Some("Formato de imagem não suportado (use PNG ou JPEG).".into()),
        }
    }

    pub fn page_break(&mut self) {
        self.begin(None);
        if let Some((a, b)) = self.sel() {
            self.cursor = self.doc.delete(a, b);
        }
        self.cursor = self.doc.split(self.cursor);
        self.doc.paras[self.cursor.para].style.page_break_before = true;
        self.anchor = self.cursor;
    }

    /// Pontos da tela por ponto do documento.
    fn z(&self) -> f32 {
        self.zoom * SCREEN
    }

    /// Os títulos do documento (nível, texto, parágrafo), para o sumário.
    fn headings(&self) -> Vec<(u8, String, usize)> {
        self.doc
            .paras
            .iter()
            .enumerate()
            .filter_map(|(i, p)| {
                let level = match p.style.name.as_str() {
                    "Título 1" => 1,
                    "Título 2" => 2,
                    "Título 3" => 3,
                    _ => return None,
                };
                let t = p.text.replace(LINE_BREAK, " ").trim().to_string();
                (!t.is_empty()).then_some((level, t, i))
            })
            .collect()
    }

    /// Formatar → Inserir sumário: o título "SUMÁRIO" e as linhas dos títulos, com o
    /// número da página à direita (acertado depois de quebrar o documento).
    fn insert_toc(&mut self) {
        if self.headings().is_empty() {
            self.message = Some("Não há títulos no documento: use os estilos Título 1, 2 e 3 (menu ¶) nos títulos das seções.".into());
            return;
        }
        self.begin(None);
        let mut at = self.cursor;
        if at.off > 0 || !self.doc.paras[at.para].text.is_empty() {
            at = self.doc.split(Pos::new(at.para, if at.off > 0 { self.doc.paras[at.para].text.len() } else { 0 }));
            if self.cursor.off == 0 {
                at = Pos::new(at.para - 1, 0);
            }
        }
        let body = self.doc.paras[at.para].spans[0].style.clone();
        let title_style = CharStyle { bold: true, size: body.size.max(12.0), ..body.clone() };
        let title = Paragraph::new("SUMÁRIO", title_style, ParaStyle { align: Align::Center, space_after: 18.0, name: "Sumário título".into(), ..ParaStyle::default() });
        self.doc.paras[at.para] = title;
        self.doc.paras.insert(at.para + 1, Paragraph::new("", body, ParaStyle { name: "Sumário".into(), leader: true, ..ParaStyle::default() }));
        // O texto depois do sumário começa numa página nova.
        if let Some(next) = self.doc.paras.get_mut(at.para + 2) {
            next.style.page_break_before = true;
        }
        self.fill_toc();
        self.toc_passes = 2;
        self.cursor = Pos::new(at.para, 0);
        self.anchor = self.cursor;
    }

    /// Refaz as linhas do sumário com os títulos e as páginas de agora.
    fn fill_toc(&mut self) {
        let Some(start) = self.doc.paras.iter().position(|p| p.style.name == "Sumário") else {
            return;
        };
        let end = self.doc.paras[start..].iter().position(|p| p.style.name != "Sumário").map_or(self.doc.paras.len(), |n| start + n);
        let base = self.doc.paras[start].spans[0].style.clone();
        let removed = end - start;
        let entries: Vec<(u8, String, usize)> = self.headings().into_iter().map(|(l, t, i)| (l, t, if i >= end { i - removed } else { i })).collect();
        let mut lines = Vec::new();
        for (level, text, para) in &entries {
            // A página do título no layout atual (as linhas do sumário ainda contam).
            let index = if *para >= start { para + removed } else { *para };
            let page = self.layout.first.get(index).and_then(|&f| self.layout.placed.get(f)).map_or(0, |p| p.page) + 1;
            let bold = *level == 1;
            let style = CharStyle { bold, ..base.clone() };
            lines.push(Paragraph::new(&format!("{text}\t{page}"), style, ParaStyle { name: "Sumário".into(), leader: true, left_indent: 14.0 * (*level as f32 - 1.0), space_after: 4.0, ..ParaStyle::default() }));
        }
        if lines.is_empty() {
            lines.push(Paragraph::new("", base, ParaStyle { name: "Sumário".into(), leader: true, ..ParaStyle::default() }));
        }
        let cursor_in = self.cursor.para >= start && self.cursor.para < end;
        self.doc.paras.splice(start..end, lines);
        if cursor_in {
            self.cursor = Pos::new(start, 0);
            self.anchor = self.cursor;
        }
        self.cursor = self.doc.clamp(self.cursor);
        self.anchor = self.doc.clamp(self.anchor);
        self.stale = true;
        self.version += 1;
    }

    pub fn toggle_find(&mut self) {
        self.find = match self.find.take() {
            Some(_) => None,
            None => {
                let query = self.sel().map(|(a, b)| self.doc.plain(a, b)).filter(|q| !q.contains('\n')).unwrap_or_default();
                Some(FindBar { query, focus: true, ..Default::default() })
            }
        };
    }

    /// Onde a busca acha `query` no parágrafo `i` (bytes), a partir de `from`.
    fn matches_in(&self, i: usize, query: &str, case: bool) -> Vec<usize> {
        let text = &self.doc.paras[i].text;
        if case {
            return text.match_indices(query).map(|(o, _)| o).collect();
        }
        let lower = text.to_lowercase();
        let q = query.to_lowercase();
        // Só quando minúsculas não mudam o tamanho dos bytes (quase sempre).
        if lower.len() != text.len() {
            return text.match_indices(query).map(|(o, _)| o).collect();
        }
        lower.match_indices(&q).map(|(o, _)| o).filter(|o| text.is_char_boundary(*o) && text.is_char_boundary(o + q.len())).collect()
    }

    fn find_next(&mut self, back: bool) {
        let Some((query, case)) = self.find.as_ref().map(|f| (f.query.clone(), f.case)).filter(|(q, _)| !q.is_empty()) else {
            return;
        };
        let (a, b) = self.sel().unwrap_or((self.cursor, self.cursor));
        let n = self.doc.paras.len();
        let mut found = None;
        if back {
            'outer: for k in 0..=n {
                let i = (a.para + n - k % n) % n;
                let mut ms = self.matches_in(i, &query, case);
                ms.reverse();
                for o in ms {
                    if k == 0 && o >= a.off {
                        continue;
                    }
                    found = Some(Pos::new(i, o));
                    break 'outer;
                }
            }
        } else {
            'outer2: for k in 0..=n {
                let i = (b.para + k) % n;
                for o in self.matches_in(i, &query, case) {
                    if k == 0 && o < b.off {
                        continue;
                    }
                    found = Some(Pos::new(i, o));
                    break 'outer2;
                }
            }
        }
        match found {
            Some(p) => {
                self.anchor = p;
                self.cursor = Pos::new(p.para, p.off + query.len());
                self.reveal = true;
                self.message = None;
            }
            None => self.message = Some(format!("\"{query}\" não encontrado.")),
        }
    }

    fn replace_one(&mut self) {
        let Some(f) = &self.find else {
            return;
        };
        let (query, with, case) = (f.query.clone(), f.replace.clone(), f.case);
        if let Some((a, b)) = self.sel() {
            let selected = self.doc.plain(a, b);
            let same = if case { selected == query } else { selected.to_lowercase() == query.to_lowercase() };
            if same && a.para == b.para {
                self.begin(None);
                let style = self.doc.style_at(self.doc.next(a));
                self.doc.delete(a, b);
                self.cursor = self.doc.insert_text(a, &with, &style);
                self.anchor = self.cursor;
            }
        }
        self.find_next(false);
    }

    fn replace_all(&mut self) {
        let Some(f) = &self.find else {
            return;
        };
        let (query, with, case) = (f.query.clone(), f.replace.clone(), f.case);
        if query.is_empty() {
            return;
        }
        let mut count = 0;
        self.begin(None);
        for i in 0..self.doc.paras.len() {
            let mut ms = self.matches_in(i, &query, case);
            ms.reverse();
            for o in ms {
                let a = Pos::new(i, o);
                let style = self.doc.style_at(self.doc.next(a));
                self.doc.delete(a, Pos::new(i, o + query.len()));
                self.doc.insert_text(a, &with, &style);
                count += 1;
            }
        }
        self.cursor = self.doc.clamp(self.cursor);
        self.anchor = self.cursor;
        self.message = Some(format!("{count} {} substituída(s).", if count == 1 { "ocorrência" } else { "ocorrências" }));
    }

    /// A barra de busca, abaixo da barra de formatação.
    fn find_bar(&mut self, ui: &mut Ui, y: f32, w: f32) {
        let Some(f) = self.find.as_mut() else {
            return;
        };
        let theme = ui.theme().clone();
        ui.paint(|pixmap, _| {
            let c = theme.background;
            mide::ui::draw::paint_rounded_solid(pixmap, 0.0, y, w, FIND_H, 0.0, [c[0], c[1], c[2], 255]);
        });
        let focus = std::mem::take(&mut f.focus);
        let (mut next, mut prev, mut one, mut all, mut close) = (false, false, false, false, false);
        let mut case = f.case;
        ui.at(12.0, y + 4.0, w - 24.0, |ui| {
            ui.row(|ui| {
                if focus {
                    ui.focus_next_input();
                }
                ui.sized(240.0, |ui| next = ui.search_field(&mut f.query, "Buscar").submitted());
                match ui.toolbar_buttons(&[("go-up-symbolic", "‹", false, true), ("go-down-symbolic", "›", false, true), ("", "Aa", case, true)]) {
                    Some(0) => prev = true,
                    Some(1) => next = true,
                    Some(2) => case = !case,
                    _ => {}
                }
                ui.sized(200.0, |ui| one = ui.search_field(&mut f.replace, "Substituir por").submitted());
                ui.sized(100.0, |ui| one |= ui.button_styled("Substituir", ButtonStyle::Bordered).clicked());
                ui.sized(70.0, |ui| all = ui.button_styled("Tudo", ButtonStyle::Bordered).clicked());
                ui.sized(60.0, |ui| close = ui.button_styled("OK", ButtonStyle::Prominent).clicked());
            });
        });
        f.case = case;
        if close || (ui.typing() && ui.input().key(Key::Escape)) {
            self.find = None;
            ui.blur();
            return;
        }
        if next || prev {
            self.find_next(prev);
        }
        if one {
            self.replace_one();
        }
        if all {
            self.replace_all();
        }
    }

    fn zoom_by(&mut self, factor: f32) {
        self.zoom = (self.zoom * factor).clamp(0.5, 3.0);
        if (self.zoom - 1.0).abs() < 0.04 {
            self.zoom = 1.0;
        }
    }

    /// Menus do modo formatado na barra superior.
    pub fn menus(&self) -> Vec<MenuGroup> {
        let st = self.current_style();
        let ps = self.current_para();
        vec![
            MenuGroup {
                title: "Formatar".into(),
                items: vec![
                    MenuItem::Submenu {
                        label: "Fonte".into(),
                        items: vec![
                            MenuItem::action("negrito", "Negrito").shortcut("Ctrl+B").checked(st.bold),
                            MenuItem::action("italico", "Itálico").shortcut("Ctrl+I").checked(st.italic),
                            MenuItem::action("sublinhado", "Sublinhado").shortcut("Ctrl+U").checked(st.underline),
                            MenuItem::action("tachado", "Tachado").checked(st.strike),
                            MenuItem::Separator,
                            MenuItem::action("sobrescrito", "Sobrescrito").checked(st.baseline == Baseline::Super),
                            MenuItem::action("subscrito", "Subscrito").checked(st.baseline == Baseline::Sub),
                            MenuItem::Separator,
                            MenuItem::action("maior", "Maior").shortcut("Ctrl+Shift+."),
                            MenuItem::action("menor", "Menor").shortcut("Ctrl+Shift+,"),
                        ],
                    },
                    MenuItem::Submenu {
                        label: "Texto".into(),
                        items: vec![
                            MenuItem::action("alinhar-esquerda", "Alinhar à esquerda").shortcut("Ctrl+L").checked(ps.align == Align::Left),
                            MenuItem::action("centralizar", "Centralizar").shortcut("Ctrl+E").checked(ps.align == Align::Center),
                            MenuItem::action("justificar", "Justificar").shortcut("Ctrl+J").checked(ps.align == Align::Justify),
                            MenuItem::action("alinhar-direita", "Alinhar à direita").shortcut("Ctrl+R").checked(ps.align == Align::Right),
                        ],
                    },
                    MenuItem::Submenu {
                        label: "Lista".into(),
                        items: vec![
                            MenuItem::action("lista-nenhuma", "Nenhuma").checked(ps.list.is_none()),
                            MenuItem::action("lista-marcadores", "• Marcadores").checked(ps.list == Some(ListKind::Bullet)),
                            MenuItem::action("lista-travessao", "– Travessões").checked(ps.list == Some(ListKind::Dash)),
                            MenuItem::action("lista-numeros", "1. Números").checked(ps.list == Some(ListKind::Number)),
                            MenuItem::action("lista-letras", "a) Letras").checked(ps.list == Some(ListKind::Letter)),
                        ],
                    },
                    MenuItem::Separator,
                    MenuItem::action("inserir-imagem", "Inserir imagem…"),
                    MenuItem::action("quebra-pagina", "Inserir quebra de página").shortcut("Ctrl+Enter"),
                    MenuItem::action("sumario", "Inserir sumário"),
                    MenuItem::action("atualizar-sumario", "Atualizar sumário"),
                    MenuItem::Separator,
                    MenuItem::action("configurar-pagina", "Configurar página…").shortcut("Ctrl+Shift+P"),
                    MenuItem::action("abnt", "Aplicar normas ABNT…"),
                    MenuItem::Separator,
                    MenuItem::action("texto-simples", "Converter em texto simples").shortcut("Ctrl+Shift+T"),
                ],
            },
            MenuGroup {
                title: "Visualizar".into(),
                items: vec![
                    MenuItem::action("zoom-mais", "Aumentar zoom").shortcut("Ctrl+="),
                    MenuItem::action("zoom-menos", "Diminuir zoom").shortcut("Ctrl+-"),
                    MenuItem::action("zoom-real", "Tamanho real").shortcut("Ctrl+0"),
                ],
            },
        ]
    }

    /// Ações dos menus e dos atalhos. Devolve `true` se tratou.
    pub fn command(&mut self, action: &str, ui: &mut Ui) -> bool {
        match action {
            "negrito" => {
                let on = !self.current_style().bold;
                self.char_style(|s| s.bold = on);
            }
            "italico" => {
                let on = !self.current_style().italic;
                self.char_style(|s| s.italic = on);
            }
            "sublinhado" => {
                let on = !self.current_style().underline;
                self.char_style(|s| s.underline = on);
            }
            "tachado" => {
                let on = !self.current_style().strike;
                self.char_style(|s| s.strike = on);
            }
            "sobrescrito" | "subscrito" => {
                let want = if action == "sobrescrito" { Baseline::Super } else { Baseline::Sub };
                let b = if self.current_style().baseline == want { Baseline::Normal } else { want };
                self.char_style(|s| s.baseline = b);
            }
            "maior" | "menor" => {
                let cur = self.current_style().size;
                let next = if action == "maior" { SIZES.iter().copied().find(|s| *s > cur).unwrap_or(cur + 4.0) } else { SIZES.iter().rev().copied().find(|s| *s < cur).unwrap_or((cur - 1.0).max(4.0)) };
                self.char_style(|s| s.size = next);
            }
            "alinhar-esquerda" => self.para_style(|p| p.align = Align::Left),
            "centralizar" => self.para_style(|p| p.align = Align::Center),
            "justificar" => self.para_style(|p| p.align = Align::Justify),
            "alinhar-direita" => self.para_style(|p| p.align = Align::Right),
            "lista-nenhuma" => self.para_style(|p| p.list = None),
            "lista-marcadores" => self.para_style(|p| p.list = Some(ListKind::Bullet)),
            "lista-travessao" => self.para_style(|p| p.list = Some(ListKind::Dash)),
            "lista-numeros" => self.para_style(|p| p.list = Some(ListKind::Number)),
            "lista-letras" => self.para_style(|p| p.list = Some(ListKind::Letter)),
            "inserir-imagem" => self.wants_image = true,
            "quebra-pagina" => self.page_break(),
            "configurar-pagina" => self.open_page_dialog(),
            "imprimir" => self.open_print_dialog(),
            "exportar-pdf" => self.wants_pdf_path = Some(None),
            "abnt" => self.abnt_dialog = Some(AbntDialog { font: 0, numbers_from: "2".into() }),
            "zoom-mais" => self.zoom_by(1.1),
            "zoom-menos" => self.zoom_by(1.0 / 1.1),
            "zoom-real" => self.zoom = 1.0,
            "desfazer" => self.undo(false),
            "refazer" => self.undo(true),
            "copiar" => self.copy(ui, false),
            "recortar" => self.copy(ui, true),
            "colar" => self.paste(ui),
            "selecionar-tudo" => self.select_all(),
            "sumario" => self.insert_toc(),
            "atualizar-sumario" => {
                if self.doc.paras.iter().any(|p| p.style.name == "Sumário") {
                    self.begin(None);
                    self.fill_toc();
                    self.toc_passes = 2;
                } else {
                    self.insert_toc();
                }
            }
            "buscar" => self.toggle_find(),
            _ => return false,
        }
        ui.request_repaint();
        true
    }

    fn open_print_dialog(&mut self) {
        let pages = self.layout.pages.max(1);
        let current = self.layout.line_of(self.cursor).map_or(0, |p| p.page);
        self.print_dialog = Some(PrintDialog { printers: printers(), dest: 0, copies: 1, pages: 0, from: "1".into(), to: pages.to_string(), duplex: false, preview: current });
        self.pop = None;
    }

    /// Grava o PDF (páginas de 0) no caminho.
    fn write_pdf(&mut self, fonts: &mut cosmic_text::FontSystem, path: &std::path::Path, range: Option<(usize, usize)>) -> Result<(), String> {
        let data = mide::richtext::pdf::write(&self.doc, &self.layout, &mut self.layouter, fonts, range);
        mide::files::write_atomic(path, &data).map_err(|e| e.to_string())
    }

    /// Manda para a impressora pelo CUPS (`lp`), com um PDF temporário.
    fn print(&mut self, fonts: &mut cosmic_text::FontSystem, printer: &str, copies: i64, range: Option<(usize, usize)>, duplex: bool) {
        let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(std::path::PathBuf::from).unwrap_or_else(std::env::temp_dir);
        let tmp = runtime.join(format!("mide-imprimir-{}.pdf", std::process::id()));
        if let Err(err) = self.write_pdf(fonts, &tmp, range) {
            self.message = Some(format!("Não foi possível preparar a impressão: {err}"));
            return;
        }
        let mut cmd = std::process::Command::new("lp");
        cmd.arg("-d").arg(printer).arg("-n").arg(copies.max(1).to_string());
        if duplex {
            cmd.args(["-o", "sides=two-sided-long-edge"]);
        }
        cmd.arg(&tmp).env("LC_ALL", "C");
        match cmd.output() {
            Ok(out) if out.status.success() => {
                tracing::info!("editor: impressão enviada para {printer}: {}", String::from_utf8_lossy(&out.stdout).trim());
                self.message = Some(format!("Enviado para a impressora {printer}."));
            }
            Ok(out) => self.message = Some(format!("A impressora recusou: {}", String::from_utf8_lossy(&out.stderr).trim())),
            Err(err) => self.message = Some(format!("Não foi possível imprimir (o CUPS está instalado?): {err}")),
        }
        let _ = std::fs::remove_file(&tmp);
    }

    fn open_page_dialog(&mut self) {
        let p = &self.doc.page;
        let a4 = PageSetup::a4();
        let paper = if (p.width.min(p.height) - a4.width).abs() < 2.0 { 0 } else { 1 };
        let f = |v: f32| format!("{:.2}", to_cm(v)).replace('.', ",");
        self.page_dialog = Some(PageDialog {
            paper,
            landscape: p.landscape(),
            margins: [f(p.margins[0]), f(p.margins[1]), f(p.margins[2]), f(p.margins[3])],
            header: p.header.clone(),
            footer: p.footer.clone(),
            numbers: p.numbers.show,
            position: match p.numbers.position {
                NumberPosition::TopRight => 0,
                NumberPosition::BottomCenter => 1,
                NumberPosition::BottomRight => 2,
            },
            from: p.numbers.show_from.to_string(),
        });
    }

    /// Aplica as normas ABNT (NBR 14724) ao documento todo.
    fn apply_abnt(&mut self, font: usize, from: usize) {
        self.begin(None);
        let family = if font == 0 { "Times New Roman" } else { "Arial" };
        let page = &mut self.doc.page;
        let (w, h) = (cm(21.0), cm(29.7));
        page.width = w;
        page.height = h;
        page.margins = [cm(3.0), cm(2.0), cm(2.0), cm(3.0)];
        page.numbers.show = true;
        page.numbers.position = NumberPosition::TopRight;
        page.numbers.show_from = from.max(1);
        for p in &mut self.doc.paras {
            for s in &mut p.spans {
                s.style.family = family.to_string();
            }
            match p.style.name.as_str() {
                "" if p.image.is_none() && p.style.list.is_none() => {
                    p.style.align = Align::Justify;
                    p.style.first_indent = cm(1.25);
                    p.style.line_spacing = 1.5;
                    p.style.left_indent = 0.0;
                    p.style.right_indent = 0.0;
                    p.style.space_before = 0.0;
                    p.style.space_after = 0.0;
                    for s in &mut p.spans {
                        s.style.size = 12.0;
                    }
                }
                "Citação" => {
                    p.style.left_indent = cm(4.0);
                    p.style.first_indent = 0.0;
                    p.style.line_spacing = 1.0;
                    p.style.align = Align::Justify;
                    for s in &mut p.spans {
                        s.style.size = 10.0;
                    }
                }
                "Legenda" | "Referência" => {
                    p.style.line_spacing = 1.0;
                    for s in &mut p.spans {
                        s.style.size = if p.style.name == "Legenda" { 10.0 } else { 12.0 };
                    }
                }
                n if n.starts_with("Título") => {
                    p.style.line_spacing = 1.5;
                    p.style.first_indent = 0.0;
                    for s in &mut p.spans {
                        s.style.size = 12.0;
                        s.style.bold = true;
                    }
                }
                _ => {}
            }
        }
        tracing::info!("editor: normas ABNT aplicadas ({family})");
        self.message = Some(format!("Normas ABNT aplicadas: A4, margens 3/2/2/3 cm, {family} 12, entrelinha 1,5, recuo de 1,25 cm, números no canto de cima a partir da página {from}."));
    }

    /// Teclas (sem outro campo com o foco).
    fn keys(&mut self, ui: &mut Ui) {
        let input = ui.input().clone();
        // Atalhos com Ctrl.
        let shortcuts = [("b", "negrito"), ("i", "italico"), ("u", "sublinhado"), ("e", "centralizar"), ("l", "alinhar-esquerda"), ("r", "alinhar-direita"), ("j", "justificar"), ("a", "selecionar-tudo"), ("c", "copiar"), ("x", "recortar"), ("v", "colar"), ("y", "refazer"), ("=", "zoom-mais"), ("+", "zoom-mais"), ("-", "zoom-menos"), ("0", "zoom-real")];
        for (letter, action) in shortcuts {
            if input.shortcut(letter) && !(input.alt && letter == "0") {
                self.command(action, ui);
            }
        }
        match input.shortcut_shift("z") {
            Some(true) => self.undo(true),
            Some(false) => self.undo(false),
            None => {}
        }
        if input.shortcut("f") {
            self.toggle_find();
        }
        // Títulos como no Word: Ctrl+Alt+1, 2, 3; Ctrl+Alt+0 volta ao corpo.
        if input.alt {
            for (key, name) in [("1", "Título 1"), ("2", "Título 2"), ("3", "Título 3"), ("0", "Corpo")] {
                if input.shortcut(key) {
                    self.apply_named(name);
                }
            }
        }
        match input.shortcut_shift("g") {
            Some(back) => self.find_next(back),
            None => {}
        }
        match input.shortcut_shift("p") {
            Some(true) => self.open_page_dialog(),
            Some(false) => self.open_print_dialog(),
            None => {}
        }
        if input.shortcut_shift(".") == Some(true) || input.shortcut_shift(">") == Some(true) {
            self.command("maior", ui);
        }
        if input.shortcut_shift(",") == Some(true) || input.shortcut_shift("<") == Some(true) {
            self.command("menor", ui);
        }
        if let Some(text) = input.paste.clone().filter(|_| self.paste_pending) {
            self.paste_pending = false;
            self.paste_text(&text);
        }
        for (key, ctrl, shift) in input.key_events() {
            let p = self.cursor;
            match key {
                Key::Left => {
                    let to = match (self.sel(), shift, ctrl) {
                        (Some((a, _)), false, false) => a,
                        (_, _, true) => self.doc.prev_word(p),
                        _ => self.doc.prev(p),
                    };
                    self.move_to(to, shift);
                }
                Key::Right => {
                    let to = match (self.sel(), shift, ctrl) {
                        (Some((_, b)), false, false) => b,
                        (_, _, true) => self.doc.next_word(p),
                        _ => self.doc.next(p),
                    };
                    self.move_to(to, shift);
                }
                Key::Up | Key::Down => {
                    let x = *self.goal_x.get_or_insert_with(|| self.layout.caret_x(p));
                    let to = self.layout.vertical(p, x, if key == Key::Up { -1 } else { 1 });
                    self.move_to(to, shift);
                    self.goal_x = Some(x);
                }
                Key::Home => {
                    let to = if ctrl { Pos::default() } else { self.layout.line_bounds(p).0 };
                    self.move_to(to, shift);
                }
                Key::End => {
                    let to = if ctrl { self.doc.end() } else { self.layout.line_bounds(p).1 };
                    self.move_to(to, shift);
                }
                Key::PageUp | Key::PageDown => {
                    let x = self.layout.caret_x(p);
                    let mut to = p;
                    for _ in 0..30 {
                        to = self.layout.vertical(to, x, if key == Key::PageUp { -1 } else { 1 });
                    }
                    self.move_to(to, shift);
                }
                Key::Backspace => self.backspace(ctrl),
                Key::Delete => self.delete_forward(ctrl),
                Key::Enter if ctrl => self.page_break(),
                Key::Enter if shift => self.type_text(&LINE_BREAK.to_string()),
                Key::Enter => self.enter(),
                Key::Tab => {
                    // Numa lista, Tab desce um nível (Shift+Tab sobe).
                    if self.doc.paras[p.para].style.list.is_some() && (p.off == 0 || self.sel().is_some()) {
                        self.para_style(|s| s.level = if shift { s.level.saturating_sub(1) } else { (s.level + 1).min(8) });
                    } else if !shift {
                        self.type_text("\t");
                    }
                }
                Key::Escape => {
                    self.anchor = self.cursor;
                }
            }
        }
        if !input.text.is_empty() && !input.ctrl && !input.logo {
            let text: String = input.text.chars().filter(|c| !c.is_control()).collect();
            if !text.is_empty() {
                self.type_text(&text);
            }
        }
        if input.keys.is_empty() && input.text.is_empty() {
            return;
        }
        ui.request_repaint();
    }

    /// Geometria das páginas: x da página, altura de cada uma e o topo da página `i`.
    fn page_x(&self, area: (f32, f32, f32, f32)) -> f32 {
        let pw = self.doc.page.width * self.z();
        area.0 + ((area.2 - pw) / 2.0).max(GAP)
    }

    fn page_top(&self, area: (f32, f32, f32, f32), i: usize) -> f32 {
        area.1 + GAP + i as f32 * (self.doc.page.height * self.z() + GAP) - self.scroll
    }

    fn content_height(&self) -> f32 {
        GAP + self.layout.pages.max(1) as f32 * (self.doc.page.height * self.z() + GAP)
    }

    /// A posição sob o ponto da janela.
    fn hit(&self, area: (f32, f32, f32, f32), (px, py): (f32, f32)) -> Pos {
        let ph = self.doc.page.height * self.z();
        let rel = py - area.1 + self.scroll - GAP;
        let page = ((rel / (ph + GAP)).floor().max(0.0) as usize).min(self.layout.pages.saturating_sub(1));
        let top = self.page_top(area, page);
        let x = (px - self.page_x(area)) / self.z() - self.doc.page.margins[3];
        let y = (py - top) / self.z();
        self.doc.clamp(self.layout.hit(page, x, y))
    }

    fn pointer(&mut self, ui: &mut Ui, area: (f32, f32, f32, f32)) {
        let input = ui.input().clone();
        let Some(p) = input.pointer else {
            return;
        };
        let inside = p.0 >= area.0 && p.0 <= area.0 + area.2 && p.1 >= area.1 && p.1 <= area.1 + area.3;
        if inside && input.scroll != 0.0 {
            let s = ui.take_scroll();
            if input.ctrl {
                self.zoom_by(if s < 0.0 { 1.1 } else { 1.0 / 1.1 });
            } else {
                self.scroll += s;
            }
            ui.request_repaint();
        }
        if input.pressed && inside && p.0 < area.0 + area.2 - 14.0 {
            let pos = self.hit(area, p);
            let now = Instant::now();
            let (at, count, last) = self.clicks;
            let count = if now.duration_since(at) < Duration::from_millis(450) && last == pos { count + 1 } else { 1 };
            self.clicks = (now, count, pos);
            self.typing = None;
            self.goal_x = None;
            self.last_kind = None;
            self.blink = now;
            match count {
                2 => {
                    let (a, b) = self.doc.word_at(pos);
                    self.anchor = a;
                    self.cursor = b;
                }
                n if n >= 3 => {
                    self.anchor = Pos::new(pos.para, 0);
                    self.cursor = Pos::new(pos.para, self.doc.paras[pos.para].text.len());
                }
                _ => {
                    self.cursor = pos;
                    if !input.shift {
                        self.anchor = pos;
                    }
                    self.selecting = true;
                }
            }
            ui.blur();
            self.pop = None;
            ui.request_repaint();
        }
        if self.selecting && input.down {
            let pos = self.hit(area, p);
            if pos != self.cursor {
                self.cursor = pos;
                ui.request_repaint();
            }
            // Arrastar para fora da área rola.
            if p.1 < area.1 {
                self.scroll -= 12.0;
            } else if p.1 > area.1 + area.3 {
                self.scroll += 12.0;
            }
        }
        if input.released || !input.down {
            self.selecting = false;
        }
    }

    /// A tela inteira do modo formatado, abaixo da barra de ferramentas do app.
    pub fn view(&mut self, ui: &mut Ui, top: f32) {
        let (w, h) = ui.window_size();
        let find_h = if self.find.is_some() { FIND_H } else { 0.0 };
        let area = (0.0, top + FORMAT_H + find_h, w, (h - top - FORMAT_H - find_h - STATUS_H).max(10.0));
        let blocked = ui.menu_open().is_some() || self.page_dialog.is_some() || self.abnt_dialog.is_some() || self.print_dialog.is_some();
        if !blocked && self.pop.is_none() && !ui.typing() {
            self.keys(ui);
        }
        if !blocked {
            self.pointer(ui, area);
        }
        // Quebra de novo, se mudou, e mantém o cursor à vista.
        let theme = ui.theme().clone();
        let mut relaid = false;
        if self.stale || self.layout.paras.len() != self.doc.paras.len() {
            ui.paint_text(|_, text, _| {
                self.layout = self.layouter.layout(text.font_system(), &self.doc);
            });
            self.stale = false;
            relaid = true;
            self.cursor = self.doc.clamp(self.cursor);
            self.anchor = self.doc.clamp(self.anchor);
        }
        let _ = relaid;
        // Sumário: números das páginas acertados com o layout novo (até duas vezes).
        if self.toc_passes > 0 && !self.stale {
            self.toc_passes -= 1;
            let before: Vec<String> = self.doc.paras.iter().filter(|p| p.style.name == "Sumário").map(|p| p.text.clone()).collect();
            self.fill_toc();
            let after: Vec<String> = self.doc.paras.iter().filter(|p| p.style.name == "Sumário").map(|p| p.text.clone()).collect();
            if before == after {
                self.toc_passes = 0;
                self.stale = false;
            } else {
                ui.paint_text(|_, text, _| {
                    self.layout = self.layouter.layout(text.font_system(), &self.doc);
                });
                self.stale = false;
            }
        }
        // PDF e impressão pedidos (precisam das fontes do sistema).
        if self.pdf_request.is_some() || self.print_job.is_some() {
            let pdf = self.pdf_request.take();
            let job = self.print_job.take();
            let mut result = None;
            ui.paint_text(|_, text, _| {
                if let Some((path, range)) = &pdf {
                    result = Some((path.clone(), self.write_pdf(text.font_system(), path, *range)));
                }
                if let Some((printer, copies, range, duplex)) = &job {
                    self.print(text.font_system(), printer, *copies, *range, *duplex);
                }
            });
            if let Some((path, r)) = result {
                self.message = Some(match r {
                    Ok(()) => {
                        tracing::info!("editor: PDF salvo em {}", path.display());
                        format!("PDF salvo: {}", path.display())
                    }
                    Err(err) => format!("Não foi possível salvar o PDF: {err}"),
                });
            }
        }
        if std::mem::take(&mut self.reveal) {
            if let Some(placed) = self.layout.line_of(self.cursor).copied() {
                let line = &self.layout.paras[self.cursor.para].lines[placed.line];
                let y = GAP + placed.page as f32 * (self.doc.page.height * self.z() + GAP) + placed.y * self.z();
                let lh = line.height * self.z();
                if y < self.scroll + 8.0 {
                    self.scroll = y - 8.0;
                } else if y + lh > self.scroll + area.3 - 8.0 {
                    self.scroll = y + lh - area.3 + 8.0;
                }
            }
        }
        let max_scroll = (self.content_height() - area.3).max(0.0);
        self.scroll = self.scroll.clamp(0.0, max_scroll);
        // Páginas.
        let k = mide::ui::draw::scale();
        let (pw, ph) = (self.doc.page.width * self.z(), self.doc.page.height * self.z());
        let px = self.page_x(area);
        let blink_on = (Instant::now().duration_since(self.blink).as_millis() / 530) % 2 == 0;
        let focused = ui.input().focused && !ui.typing();
        let caret = (focused && blink_on && !blocked).then_some(self.cursor);
        let selection = self.sel();
        let sel_color = if focused { [179, 215, 255, 255] } else { [220, 220, 224, 255] };
        let bg = if theme.dark { [44, 44, 48, 255] } else { [236, 236, 240, 255] };
        let pages = self.layout.pages.max(1);
        let tops: Vec<(usize, f32)> = (0..pages).map(|i| (i, self.page_top(area, i))).filter(|(_, t)| *t + ph >= area.1 && *t <= area.1 + area.3).collect();
        let z = self.z();
        let scroll = self.scroll;
        let doc = &self.doc;
        let layout = &self.layout;
        let layouter = &mut self.layouter;
        let accent = theme.accent;
        ui.paint_text(|pixmap, text, _| {
            mide::ui::draw::paint_rounded_solid(pixmap, area.0, area.1, area.2, area.3, 0.0, bg);
            for (i, t) in &tops {
                mide::ui::draw::soft_shadow(pixmap, px, *t, pw, ph, 0.0, 10.0, 2.0, 40);
                mide::ui::draw::paint_rounded_solid(pixmap, px, *t, pw, ph, 0.0, [255, 255, 255, 255]);
                let view = render::View {
                    page: *i,
                    origin: (px * k, t * k),
                    scale: z * k,
                    selection,
                    caret,
                    selection_color: sel_color,
                    caret_color: [0, 0, 0, 255],
                    visible: (area.1 * k, (area.1 + area.3) * k),
                };
                render::draw_page(text, layouter, pixmap, doc, layout, &view);
            }
            // Barra de rolagem (fina, à direita, sem entrar no canto da janela).
            if max_scroll > 0.0 {
                let track_top = area.1 + 4.0;
                let track_h = area.3 - 8.0;
                let thumb = (track_h * area.3 / (area.3 + max_scroll)).max(32.0);
                let ty = track_top + (track_h - thumb) * (scroll / max_scroll);
                mide::ui::draw::paint_rounded_solid(pixmap, area.0 + area.2 - 10.0, ty, 6.0, thumb, 3.0, [128, 128, 132, 150]);
            }
            let _ = accent;
        });
        // Cursor piscando.
        if focused && !blocked {
            ui.wake_after(Duration::from_millis(530));
        }
        // Barra de formatação (em cima das páginas) e a de busca.
        self.format_bar(ui, top, w);
        self.find_bar(ui, top + FORMAT_H, w);
        // Rodapé.
        let page_now = self.layout.line_of(self.cursor).map_or(1, |p| p.page + 1);
        let (words, chars, _) = self.doc.counts();
        let status = format!("Página {page_now} de {} · {words} {} · {chars} caracteres · {:.0}%", self.layout.pages.max(1), if words == 1 { "palavra" } else { "palavras" }, self.zoom * 100.0);
        ui.paint(|pixmap, _| {
            let c = theme.background;
            mide::ui::draw::paint_rounded_solid(pixmap, 0.0, h - STATUS_H, w, STATUS_H, 0.0, [c[0], c[1], c[2], 255]);
        });
        ui.at(0.0, h - STATUS_H, w, |ui| {
            ui.status_bar(&status);
        });
        self.popovers(ui);
        self.dialogs(ui);
    }

    /// A barra de formatação do TextEdit (Tahoe): grupos em cápsula.
    fn format_bar(&mut self, ui: &mut Ui, top: f32, w: f32) {
        let st = self.current_style();
        let ps = self.current_para();
        let theme = ui.theme().clone();
        ui.paint(|pixmap, _| {
            let c = theme.background;
            mide::ui::draw::paint_rounded_solid(pixmap, 0.0, top, w, FORMAT_H, 0.0, [c[0], c[1], c[2], 255]);
        });
        let mut pop: Option<(Pop, (f32, f32, f32, f32))> = None;
        let mut action: Option<&str> = None;
        let open = self.pop;
        ui.at(12.0, top + 4.0, w - 24.0, |ui| {
            ui.row(|ui| {
                // Estilo de parágrafo (¶) e fonte.
                let style_name = if ps.name.is_empty() { "Corpo".to_string() } else { ps.name.clone() };
                ui.sized(118.0, |ui| {
                    if ui.button_styled(&format!("¶ {style_name}"), ButtonStyle::Bordered).clicked() {
                        pop = Some((Pop::Styles, ui.last_rect()));
                    }
                });
                ui.sized(170.0, |ui| {
                    if ui.button_styled(&format!("{}  ⌄", st.family), ButtonStyle::Bordered).clicked() {
                        pop = Some((Pop::Fonts, ui.last_rect()));
                    }
                });
                ui.sized(64.0, |ui| {
                    let size = if st.size.fract() == 0.0 { format!("{:.0}", st.size) } else { format!("{:.1}", st.size) };
                    if ui.button_styled(&format!("{size} ⌄"), ButtonStyle::Bordered).clicked() {
                        pop = Some((Pop::Size, ui.last_rect()));
                    }
                });
                // Cor do texto e realce.
                match ui.toolbar_buttons(&[("", "A", open == Some(Pop::Color), true), ("", "▌", open == Some(Pop::Highlight), true)]) {
                    Some(0) => pop = Some((Pop::Color, seg(ui.last_rect(), 0))),
                    Some(1) => pop = Some((Pop::Highlight, seg(ui.last_rect(), 1))),
                    _ => {}
                }
                let colors = ui.last_rect();
                let (tc, hc) = (st.color, st.highlight);
                ui.paint(|pixmap, _| {
                    let a = seg(colors, 0);
                    mide::ui::draw::paint_rounded_solid(pixmap, a.0 + 11.0, a.1 + a.3 - 11.0, 18.0, 4.0, 2.0, [tc[0], tc[1], tc[2], 255]);
                    let b = seg(colors, 1);
                    let c = hc.unwrap_or([200, 200, 204]);
                    mide::ui::draw::paint_rounded_solid(pixmap, b.0 + 11.0, b.1 + 9.0, 18.0, b.3 - 18.0, 4.0, [c[0], c[1], c[2], if hc.is_some() { 255 } else { 90 }]);
                });
                match ui.toolbar_buttons(&[
                    ("", "B", st.bold, true),
                    ("", "I", st.italic, true),
                    ("", "U", st.underline, true),
                    ("", "S", st.strike, true),
                ]) {
                    Some(0) => action = Some("negrito"),
                    Some(1) => action = Some("italico"),
                    Some(2) => action = Some("sublinhado"),
                    Some(3) => action = Some("tachado"),
                    _ => {}
                }
                match ui.toolbar_buttons(&[
                    ("format-justify-left-symbolic", "⫷", ps.align == Align::Left, true),
                    ("format-justify-center-symbolic", "≡", ps.align == Align::Center, true),
                    ("format-justify-fill-symbolic", "☰", ps.align == Align::Justify, true),
                    ("format-justify-right-symbolic", "⫸", ps.align == Align::Right, true),
                ]) {
                    Some(0) => action = Some("alinhar-esquerda"),
                    Some(1) => action = Some("centralizar"),
                    Some(2) => action = Some("justificar"),
                    Some(3) => action = Some("alinhar-direita"),
                    _ => {}
                }
                match ui.toolbar_buttons(&[
                    ("format-line-spacing-symbolic", "↕", open == Some(Pop::Spacing), true),
                    ("view-list-bullet-symbolic", "•≡", open == Some(Pop::List) || ps.list.is_some(), true),
                    ("insert-image-symbolic", "▣", false, true),
                ]) {
                    Some(0) => pop = Some((Pop::Spacing, seg(ui.last_rect(), 0))),
                    Some(1) => pop = Some((Pop::List, seg(ui.last_rect(), 1))),
                    Some(2) => action = Some("inserir-imagem"),
                    _ => {}
                }
            });
        });
        if let Some((p, anchor)) = pop {
            self.pop = if self.pop == Some(p) { None } else { Some(p) };
            self.pop_anchor = anchor;
            if p == Pop::Fonts && self.families.is_empty() {
                ui.paint_text(|_, text, _| self.families = text.families());
            }
        }
        if let Some(a) = action {
            self.command(a, ui);
        }
    }

    fn popovers(&mut self, ui: &mut Ui) {
        let Some(pop) = self.pop else {
            return;
        };
        let mut open = true;
        let mut chosen_style: Option<&str> = None;
        let mut chosen_font: Option<String> = None;
        let mut chosen_size: Option<f32> = None;
        let mut chosen_color: Option<[u8; 3]> = None;
        let mut chosen_highlight: Option<Option<[u8; 3]>> = None;
        let mut spacing: Option<f32> = None;
        let mut space_after: Option<f32> = None;
        let mut list: Option<Option<ListKind>> = None;
        let cur = self.current_style();
        let ps = self.current_para();
        let width = match pop {
            Pop::Fonts => 280.0,
            Pop::Color | Pop::Highlight => 236.0,
            Pop::Size => 110.0,
            _ => 220.0,
        };
        let families = &self.families;
        let query = &mut self.font_query;
        ui.popover(&mut open, self.pop_anchor, width, |ui| match pop {
            Pop::Styles => {
                for name in STYLES {
                    let label = if same_style_name(&ps.name, name) { format!("✓  {name}") } else { format!("    {name}") };
                    if ui.button_styled(&label, ButtonStyle::Plain).clicked() {
                        chosen_style = Some(name);
                    }
                }
            }
            Pop::Fonts => {
                ui.search_field(query, "Buscar fonte");
                let q = query.to_lowercase();
                let list: Vec<&String> = families.iter().filter(|f| q.is_empty() || f.to_lowercase().contains(&q)).take(400).collect();
                ui.scroll_area(300.0, |ui| {
                    for f in list {
                        let label = if *f == cur.family { format!("✓  {f}") } else { format!("    {f}") };
                        if ui.button_styled(&label, ButtonStyle::Plain).clicked() {
                            chosen_font = Some(f.clone());
                        }
                    }
                });
            }
            Pop::Size => {
                ui.scroll_area(300.0, |ui| {
                    for s in SIZES {
                        let label = if s == cur.size { format!("✓ {s:.0}") } else { format!("   {s:.0}") };
                        if ui.button_styled(&label, ButtonStyle::Plain).clicked() {
                            chosen_size = Some(s);
                        }
                    }
                });
            }
            Pop::Color | Pop::Highlight => {
                ui.heading(if pop == Pop::Color { "Cor do texto" } else { "Realce" });
                let palette: Vec<[u8; 3]> = if pop == Pop::Color { COLORS.to_vec() } else { HIGHLIGHTS.to_vec() };
                for row in palette.chunks(8) {
                    ui.row(|ui| {
                        for c in row {
                            let on = if pop == Pop::Color { cur.color == *c } else { cur.highlight == Some(*c) };
                            if ui.tag_toggle([c[0], c[1], c[2], 255], on).clicked() {
                                if pop == Pop::Color {
                                    chosen_color = Some(*c);
                                } else {
                                    chosen_highlight = Some(Some(*c));
                                }
                            }
                        }
                    });
                }
                if pop == Pop::Highlight && ui.button_styled("Sem realce", ButtonStyle::Plain).clicked() {
                    chosen_highlight = Some(None);
                }
            }
            Pop::Spacing => {
                ui.heading("Entrelinha");
                for (v, label) in [(1.0, "Simples (1,0)"), (1.15, "1,15"), (1.5, "1,5 (ABNT)"), (2.0, "Dupla (2,0)")] {
                    let mark = if (ps.line_spacing - v).abs() < 0.01 { "✓ " } else { "   " };
                    if ui.button_styled(&format!("{mark}{label}"), ButtonStyle::Plain).clicked() {
                        spacing = Some(v);
                    }
                }
                ui.heading("Espaço depois do parágrafo");
                for (v, label) in [(0.0, "Nenhum"), (6.0, "6 pt"), (12.0, "12 pt")] {
                    let mark = if (ps.space_after - v).abs() < 0.01 { "✓ " } else { "   " };
                    if ui.button_styled(&format!("{mark}{label}"), ButtonStyle::Plain).clicked() {
                        space_after = Some(v);
                    }
                }
            }
            Pop::List => {
                for (kind, label) in [(None, "Nenhuma"), (Some(ListKind::Bullet), "•  Marcadores"), (Some(ListKind::Dash), "–  Travessões"), (Some(ListKind::Number), "1.  Números"), (Some(ListKind::Letter), "a)  Letras")] {
                    let mark = if ps.list == kind { "✓ " } else { "   " };
                    if ui.button_styled(&format!("{mark}{label}"), ButtonStyle::Plain).clicked() {
                        list = Some(kind);
                    }
                }
            }
        });
        if !open {
            self.pop = None;
        }
        let mut done = true;
        if let Some(name) = chosen_style {
            self.apply_named(name);
        } else if let Some(f) = chosen_font {
            self.char_style(|s| s.family = f.clone());
        } else if let Some(sz) = chosen_size {
            self.char_style(|s| s.size = sz);
        } else if let Some(c) = chosen_color {
            self.char_style(|s| s.color = c);
        } else if let Some(hl) = chosen_highlight {
            self.char_style(|s| s.highlight = hl);
        } else if let Some(v) = spacing {
            self.para_style(|p| p.line_spacing = v);
        } else if let Some(v) = space_after {
            self.para_style(|p| p.space_after = v);
        } else if let Some(kind) = list {
            self.para_style(|p| p.list = kind);
        } else {
            done = false;
        }
        if done {
            self.pop = None;
            ui.request_repaint();
        }
    }

    fn dialogs(&mut self, ui: &mut Ui) {
        if let Some(d) = self.page_dialog.as_mut() {
            let (mut ok, mut cancel) = (false, false);
            ui.modal(480.0, |ui| {
                ui.title("Configurar página");
                ui.form_row("Papel", 96.0, |ui| {
                    ui.sized(150.0, |ui| {
                        ui.segmented(&["A4", "Carta"], &mut d.paper, None);
                    });
                    let mut orient = usize::from(d.landscape);
                    ui.sized(190.0, |ui| {
                        if ui.segmented(&["Retrato", "Paisagem"], &mut orient, None).changed() {
                            d.landscape = orient == 1;
                        }
                    });
                });
                ui.heading("Margens (cm)");
                for (a, b) in [(0usize, 2usize), (3, 1)] {
                    let names = ["Superior", "Direita", "Inferior", "Esquerda"];
                    ui.form_row(names[a], 96.0, |ui| {
                        ui.sized(90.0, |ui| {
                            ui.text_input(&mut d.margins[a], "2,5");
                        });
                        ui.space_x(24.0);
                        ui.fixed_label(names[b], 80.0);
                        ui.sized(90.0, |ui| {
                            ui.text_input(&mut d.margins[b], "2,5");
                        });
                    });
                }
                ui.heading("Cabeçalho e rodapé");
                ui.form_row("Cabeçalho", 96.0, |ui| {
                    ui.text_input(&mut d.header, "Nenhum");
                });
                ui.form_row("Rodapé", 96.0, |ui| {
                    ui.text_input(&mut d.footer, "Nenhum");
                });
                ui.switch("Numerar as páginas", &mut d.numbers);
                if d.numbers {
                    ui.form_row("Posição", 96.0, |ui| {
                        ui.segmented(&["Em cima, à direita", "Embaixo", "Embaixo, à direita"], &mut d.position, None);
                    });
                    ui.form_row("A partir da", 96.0, |ui| {
                        ui.sized(70.0, |ui| {
                            ui.text_input(&mut d.from, "1");
                        });
                        ui.fixed_label("ª página (as anteriores contam)", 230.0);
                    });
                }
                ui.space(4.0);
                ui.row(|ui| {
                    ui.push_right(2.0 * 110.0 + 8.0);
                    ui.sized(110.0, |ui| cancel = ui.button("Cancelar").clicked());
                    ui.sized(110.0, |ui| ok = ui.primary_button("OK").clicked());
                });
            });
            if cancel || ui.input().key(Key::Escape) {
                self.page_dialog = None;
            } else if ok {
                let d = self.page_dialog.take().expect("diálogo");
                let parse = |s: &str, old: f32| s.trim().replace(',', ".").parse::<f32>().ok().filter(|v| (0.0..15.0).contains(v)).map(cm).unwrap_or(old);
                self.begin(None);
                let page = &mut self.doc.page;
                let base = if d.paper == 0 { PageSetup::a4() } else { PageSetup::letter() };
                let (pw, ph) = (base.width, base.height);
                (page.width, page.height) = if d.landscape { (ph, pw) } else { (pw, ph) };
                for i in 0..4 {
                    page.margins[i] = parse(&d.margins[i], page.margins[i]);
                }
                page.header = d.header.trim().to_string();
                page.footer = d.footer.trim().to_string();
                page.numbers.show = d.numbers;
                page.numbers.position = [NumberPosition::TopRight, NumberPosition::BottomCenter, NumberPosition::BottomRight][d.position.min(2)];
                page.numbers.show_from = d.from.trim().parse().unwrap_or(1).max(1);
                tracing::info!("editor: página {:.1}x{:.1} cm", to_cm(page.width), to_cm(page.height));
            }
        }
        if self.print_dialog.is_some() {
            self.print_dialog_view(ui);
        }
        if let Some(d) = self.abnt_dialog.as_mut() {
            let (mut ok, mut cancel) = (false, false);
            ui.modal(460.0, |ui| {
                ui.title("Normas ABNT");
                ui.note("Trabalho acadêmico (NBR 14724): papel A4, margens de 3 cm em cima e à esquerda e 2 cm embaixo e à direita, letra 12, entrelinha 1,5, recuo de 1,25 cm na primeira linha e texto justificado. Citações longas (estilo Citação) com recuo de 4 cm, letra 10 e entrelinha simples. Números das páginas no canto de cima, à direita.");
                ui.form_row("Fonte", 150.0, |ui| {
                    ui.segmented(&["Times New Roman", "Arial"], &mut d.font, None);
                });
                ui.form_row("Número a partir da", 150.0, |ui| {
                    ui.sized(70.0, |ui| {
                        ui.text_input(&mut d.numbers_from, "2");
                    });
                    ui.fixed_label("ª página", 80.0);
                });
                ui.note("A capa e a folha de rosto contam, mas não mostram o número.");
                ui.row(|ui| {
                    ui.push_right(2.0 * 110.0 + 8.0);
                    ui.sized(110.0, |ui| cancel = ui.button("Cancelar").clicked());
                    ui.sized(110.0, |ui| ok = ui.primary_button("Aplicar").clicked());
                });
            });
            if cancel || ui.input().key(Key::Escape) {
                self.abnt_dialog = None;
            } else if ok {
                let d = self.abnt_dialog.take().expect("diálogo");
                self.apply_abnt(d.font, d.numbers_from.trim().parse().unwrap_or(2));
            }
        }
    }
}

impl Rich {
    /// Diálogo de imprimir, como o do Mac: prévia da página à esquerda e as opções.
    fn print_dialog_view(&mut self, ui: &mut Ui) {
        let pages = self.layout.pages.max(1);
        let Some(d) = self.print_dialog.as_mut() else {
            return;
        };
        d.preview = d.preview.min(pages - 1);
        let (mut ok, mut cancel, mut prev, mut next) = (false, false, false, false);
        let mut dests = vec!["Salvar como PDF".to_string()];
        dests.extend(d.printers.iter().cloned());
        let mut thumb = (0.0, 0.0, 0.0, 0.0);
        let (pw, ph) = (self.doc.page.width, self.doc.page.height);
        ui.modal(600.0, |ui| {
            ui.title("Imprimir");
            let tw = 180.0;
            let th = tw * ph / pw;
            ui.columns(tw + 16.0, 16.0, |ui| {
                // Prévia (a página é desenhada depois, no lugar reservado).
                ui.custom(th + 6.0, |_, _| {});
                thumb = ui.last_rect();
                ui.row(|ui| {
                    if ui.button_styled("‹", ButtonStyle::Plain).clicked() {
                        prev = true;
                    }
                    ui.fixed_label(&format!("{} de {pages}", d.preview + 1), 70.0);
                    if ui.button_styled("›", ButtonStyle::Plain).clicked() {
                        next = true;
                    }
                });
            }, |ui| {
                ui.form_row("Destino", 80.0, |ui| {
                    ui.dropdown("", &dests, &mut d.dest);
                });
                if d.printers.is_empty() {
                    ui.note("Nenhuma impressora configurada: salve em PDF. Para adicionar uma impressora, use as configurações de impressoras do sistema.");
                }
                if d.dest > 0 {
                    ui.form_row("Cópias", 80.0, |ui| {
                        ui.stepper(&mut d.copies, 1, 999);
                    });
                }
                ui.form_row("Páginas", 80.0, |ui| {
                    ui.segmented(&["Todas", "Atual", "Intervalo"], &mut d.pages, None);
                });
                if d.pages == 2 {
                    ui.form_row("De", 80.0, |ui| {
                        ui.sized(64.0, |ui| {
                            ui.text_input(&mut d.from, "1");
                        });
                        ui.fixed_label("até", 36.0);
                        ui.sized(64.0, |ui| {
                            ui.text_input(&mut d.to, &pages.to_string());
                        });
                    });
                }
                if d.dest > 0 {
                    ui.switch("Frente e verso", &mut d.duplex);
                }
            });
            ui.row(|ui| {
                ui.push_right(110.0 + 130.0 + 8.0);
                ui.sized(110.0, |ui| cancel = ui.button("Cancelar").clicked());
                ui.sized(130.0, |ui| ok = ui.primary_button(if d.dest == 0 { "Salvar PDF…" } else { "Imprimir" }).clicked());
            });
        });
        if prev {
            d.preview = d.preview.saturating_sub(1);
        }
        if next {
            d.preview = (d.preview + 1).min(pages - 1);
        }
        // A prévia: a página no tamanho pequeno.
        if thumb.2 > 0.0 {
            let k = mide::ui::draw::scale();
            let tw = 180.0;
            let scale = tw / pw;
            let (x, y) = (thumb.0 + 2.0, thumb.1 + 2.0);
            let page = d.preview;
            let doc = &self.doc;
            let layout = &self.layout;
            let layouter = &mut self.layouter;
            ui.paint_text(|pixmap, text, _| {
                mide::ui::draw::soft_shadow(pixmap, x, y, tw, ph * scale, 0.0, 6.0, 1.0, 50);
                mide::ui::draw::paint_rounded_solid(pixmap, x, y, tw, ph * scale, 0.0, [255, 255, 255, 255]);
                let view = render::View { page, origin: (x * k, y * k), scale: scale * k, selection: None, caret: None, selection_color: [0; 4], caret_color: [0; 4], visible: (y * k, (y + ph * scale) * k) };
                render::draw_page(text, layouter, pixmap, doc, layout, &view);
            });
        }
        if cancel || ui.input().key(Key::Escape) {
            self.print_dialog = None;
            return;
        }
        if ok {
            let d = self.print_dialog.take().expect("diálogo");
            let current = self.layout.line_of(self.cursor).map_or(0, |p| p.page);
            let range = match d.pages {
                1 => Some((current, current)),
                2 => {
                    let a = d.from.trim().parse::<usize>().unwrap_or(1).max(1) - 1;
                    let b = d.to.trim().parse::<usize>().unwrap_or(pages).max(1) - 1;
                    Some((a.min(b), a.max(b)))
                }
                _ => None,
            };
            if d.dest == 0 {
                self.wants_pdf_path = Some(range);
            } else {
                self.print_job = Some((d.printers[d.dest - 1].clone(), d.copies, range, d.duplex));
            }
        }
    }
}

/// Tamanho da letra do corpo do documento.
fn body_size(doc: &Doc) -> f32 {
    doc.paras.iter().find(|p| p.style.name.is_empty() && !p.text.is_empty()).map_or(12.0, |p| p.spans[0].style.size)
}

/// Retângulo do botão `i` de um grupo de `toolbar_buttons`.
fn seg(group: (f32, f32, f32, f32), i: usize) -> (f32, f32, f32, f32) {
    (group.0 + 2.0 + i as f32 * 40.0, group.1, 40.0, group.3)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn relayout(r: &mut Rich, fs: &mut cosmic_text::FontSystem) {
        r.layout = r.layouter.layout(fs, &r.doc);
        r.stale = false;
    }

    #[test]
    fn sumario_com_paginas() {
        let mut fs = cosmic_text::FontSystem::new();
        let mut doc = Doc::new();
        doc.insert_text(Pos::default(), "1 INTRODUÇÃO\nTexto.\n2 DESENVOLVIMENTO\n2.1 Método\nMais texto.", &CharStyle::default());
        doc.paras[0].style.name = "Título 1".into();
        doc.paras[2].style.name = "Título 1".into();
        doc.paras[2].style.page_break_before = true;
        doc.paras[3].style.name = "Título 2".into();
        let mut r = Rich::new(doc);
        relayout(&mut r, &mut fs);
        r.cursor = Pos::default();
        r.anchor = r.cursor;
        r.insert_toc();
        for _ in 0..3 {
            relayout(&mut r, &mut fs);
            r.fill_toc();
        }
        let toc: Vec<String> = r.doc.paras.iter().filter(|p| p.style.name == "Sumário").map(|p| p.text.clone()).collect();
        assert_eq!(r.doc.paras[0].text, "SUMÁRIO");
        // O sumário fica na página 1; a introdução começa na 2 e o desenvolvimento na 3.
        assert_eq!(toc, ["1 INTRODUÇÃO\t2", "2 DESENVOLVIMENTO\t3", "2.1 Método\t3"], "{toc:?}");
        // O pontilhado vai até a margem.
        relayout(&mut r, &mut fs);
        let first = r.doc.paras.iter().position(|p| p.style.name == "Sumário").unwrap();
        let line = &r.layout.paras[first].lines[0];
        let (a, b) = line.leader.expect("pontilhado");
        assert!(b > a + 100.0);
        assert!((line.x1 - r.doc.page.content_width()).abs() < 2.0, "número na margem: {} de {}", line.x1, r.doc.page.content_width());
    }
}
