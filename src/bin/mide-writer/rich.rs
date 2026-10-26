//! Documento formatado do Writer (`mide-writer`), como o TextEdit do Mac: páginas na tela, barra de
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
    FontFace,
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
/// Altura da régua.
const RULER_H: f32 = 34.0;

/// Parte do seletor de cor sendo arrastada.
#[derive(Debug, Clone, Copy, PartialEq)]
enum WheelDrag {
    Wheel,
    Value,
}

/// Diâmetro da roda de cores e altura da barra de brilho (pontos).
const WHEEL: f32 = 148.0;
const VALUE_H: f32 = 18.0;

/// Desenha a roda (matiz pelo ângulo, saturação pelo raio, no brilho `v`) em `rect`.
fn paint_wheel(pixmap: &mut tiny_skia::Pixmap, rect: (f32, f32, f32, f32), hsv: [f32; 3]) {
    let k = mide::ui::draw::scale();
    let (cx, cy, r) = (rect.0 + rect.2 / 2.0, rect.1 + rect.3 / 2.0, rect.2.min(rect.3) / 2.0);
    let (w, h) = (pixmap.width() as i32, pixmap.height() as i32);
    let (x0, x1) = (((cx - r) * k).floor().max(0.0) as i32, (((cx + r) * k).ceil() as i32).min(w));
    let (y0, y1) = (((cy - r) * k).floor().max(0.0) as i32, (((cy + r) * k).ceil() as i32).min(h));
    let data = pixmap.data_mut();
    for y in y0..y1 {
        for x in x0..x1 {
            let (px, py) = ((x as f32 + 0.5) / k, (y as f32 + 0.5) / k);
            let (hue, sat) = wheel_hit(cx, cy, r, px, py);
            let dist = (px - cx).hypot(py - cy);
            // Borda suave de 1 pixel.
            let cover = ((r - dist) * k + 0.5).clamp(0.0, 1.0);
            if cover <= 0.0 {
                continue;
            }
            let [cr, cg, cb] = hsv_to_rgb([hue, sat, hsv[2]]);
            let i = ((y * w + x) * 4) as usize;
            let a = (cover * 255.0).round() as u32;
            data[i] = (cr as u32 * a / 255) as u8;
            data[i + 1] = (cg as u32 * a / 255) as u8;
            data[i + 2] = (cb as u32 * a / 255) as u8;
            data[i + 3] = a as u8;
        }
    }
    // Contorno fino e a alça na cor escolhida.
    if let Some(path) = mide::ui::draw::rounded_rect(cx - r + 0.5, cy - r + 0.5, 2.0 * r - 1.0, 2.0 * r - 1.0, r) {
        let mut paint = tiny_skia::Paint { anti_alias: true, ..Default::default() };
        paint.set_color(tiny_skia::Color::from_rgba8(0, 0, 0, 40));
        pixmap.stroke_path(&path, &paint, &tiny_skia::Stroke { width: 1.0, ..Default::default() }, mide::ui::draw::tf(), None);
    }
    let (hx, hy) = wheel_point(cx, cy, r, hsv[0], hsv[1]);
    let [cr, cg, cb] = hsv_to_rgb(hsv);
    mide::ui::draw::paint_rounded_solid(pixmap, hx - 8.0, hy - 8.0, 16.0, 16.0, 8.0, [0, 0, 0, 70]);
    mide::ui::draw::paint_rounded_solid(pixmap, hx - 7.0, hy - 7.0, 14.0, 14.0, 7.0, [255, 255, 255, 255]);
    mide::ui::draw::paint_rounded_solid(pixmap, hx - 5.0, hy - 5.0, 10.0, 10.0, 5.0, [cr, cg, cb, 255]);
}

/// Desenha a barra de brilho (do preto à cor cheia) com a alça.
fn paint_value_bar(pixmap: &mut tiny_skia::Pixmap, rect: (f32, f32, f32, f32), hsv: [f32; 3]) {
    let k = mide::ui::draw::scale();
    let full = hsv_to_rgb([hsv[0], hsv[1], 1.0]);
    let (w, h) = (pixmap.width() as i32, pixmap.height() as i32);
    let (x0, x1) = ((rect.0 * k).round() as i32, (((rect.0 + rect.2) * k).round() as i32).min(w));
    let (y0, y1) = ((rect.1 * k).round() as i32, (((rect.1 + rect.3) * k).round() as i32).min(h));
    let radius = rect.3 / 2.0;
    let data = pixmap.data_mut();
    for y in y0.max(0)..y1 {
        for x in x0.max(0)..x1 {
            let t = ((x - x0) as f32 / (x1 - x0).max(1) as f32).clamp(0.0, 1.0);
            // As pontas redondas: some o que passa do semicírculo.
            let (px, py) = ((x as f32 + 0.5) / k - rect.0, (y as f32 + 0.5) / k - rect.1);
            let inside = if px < radius {
                (px - radius).hypot(py - radius) <= radius
            } else if px > rect.2 - radius {
                (px - (rect.2 - radius)).hypot(py - radius) <= radius
            } else {
                true
            };
            if !inside {
                continue;
            }
            let i = ((y * w + x) * 4) as usize;
            for (n, c) in full.iter().enumerate() {
                data[i + n] = (*c as f32 * t).round() as u8;
            }
            data[i + 3] = 255;
        }
    }
    let kx = rect.0 + radius + hsv[2] * (rect.2 - 2.0 * radius);
    mide::ui::draw::paint_rounded_solid(pixmap, kx - 9.0, rect.1 - 1.0, 18.0, rect.3 + 2.0, 9.0, [0, 0, 0, 60]);
    mide::ui::draw::paint_rounded_solid(pixmap, kx - 8.0, rect.1, 16.0, rect.3, 8.0, [255, 255, 255, 255]);
    let [cr, cg, cb] = hsv_to_rgb(hsv);
    mide::ui::draw::paint_rounded_solid(pixmap, kx - 5.0, rect.1 + 3.0, 10.0, rect.3 - 6.0, 5.0, [cr, cg, cb, 255]);
}

/// Marcador da régua sendo arrastado.
#[derive(Debug, Clone, Copy, PartialEq)]
enum RulerDrag {
    First,
    Left,
    Right,
    Tab(f32),
}

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

struct ImageDialog {
    para: usize,
    width: String,
    height: String,
    ratio: f32,
    keep_ratio: bool,
    focus: bool,
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

/// Célula em edição: o documento de fora fica guardado aqui enquanto `Rich::doc` é o da
/// célula (assim digitar, formatar, colar e desfazer funcionam igual dentro dela).
struct Outer {
    doc: Doc,
    /// O documento de fora como estava ao entrar (vira um passo de desfazer ao sair, se a
    /// célula mudou).
    start: Doc,
    cursor: Pos,
    anchor: Pos,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    version: u64,
    table: usize,
    row: usize,
    col: usize,
}

/// Link: o endereço para o texto escolhido.
struct LinkDialog {
    url: String,
    focus: bool,
}

/// Inserir tabela: linhas e colunas.
struct TableDialog {
    rows: i64,
    cols: i64,
}

/// Nota de rodapé: nova (`id` vazio) ou a que está sendo editada.
struct NoteDialog {
    id: Option<u32>,
    text: String,
    focus: bool,
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
    /// Texto do campo "Cor personalizada" dos popovers de cor.
    custom_color: String,
    /// Cor escolhida na roda (matiz, saturação, brilho) e o que está sendo arrastado.
    hsv: [f32; 3],
    wheel_drag: Option<WheelDrag>,
    families: Vec<String>,
    page_dialog: Option<PageDialog>,
    image_dialog: Option<ImageDialog>,
    print_dialog: Option<PrintDialog>,
    note_dialog: Option<NoteDialog>,
    table_dialog: Option<TableDialog>,
    link_dialog: Option<LinkDialog>,
    /// Ortografia: o dicionário (carregado em segundo plano), ligado ou não, as palavras
    /// erradas de cada parágrafo e o que foi verificado (texto → erros).
    speller: Option<mide::spell::Speller>,
    speller_rx: Option<std::sync::mpsc::Receiver<Option<mide::spell::Speller>>>,
    pub spell_on: bool,
    /// Régua (Visualizar → Mostrar régua) e o marcador sendo arrastado.
    pub ruler: bool,
    ruler_drag: Option<RulerDrag>,
    misspelled: std::collections::HashMap<usize, Vec<(usize, usize)>>,
    spell_cache: std::collections::HashMap<String, Vec<(usize, usize)>>,
    spell_version: u64,
    /// Palavra do menu de contexto: o intervalo, a palavra e as sugestões.
    spell_target: Option<(Pos, Pos, String, Vec<String>)>,
    /// Célula a abrir depois da próxima quebra (a tabela mudou de forma).
    pending_cell: Option<(usize, usize, usize)>,
    outer: Option<Outer>,
    /// Com uma célula em edição: o documento todo (com a célula) e o layout dele, para
    /// desenhar as páginas (`layout` é o da célula).
    full: Option<(Doc, Layout)>,
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

fn font_face_label(style: &CharStyle) -> &'static str {
    match (style.bold, style.italic) {
        (false, false) => "Regular",
        (true, false) => "Bold",
        (false, true) => "Italic",
        (true, true) => "Bold Italic",
    }
}

fn set_tab_stop(stops: &mut Vec<f32>, old: Option<f32>, new: Option<f32>) {
    if let Some(old) = old
        && let Some(i) = stops.iter().position(|stop| (*stop - old).abs() < 1.0) {
            stops.remove(i);
        }
    if let Some(new) = new.filter(|stop| stop.is_finite() && *stop > 0.0) {
        stops.push(new);
    }
    stops.sort_by(f32::total_cmp);
    stops.dedup_by(|a, b| (*a - *b).abs() < 1.0);
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
            custom_color: String::new(),
            hsv: [0.0, 0.0, 1.0],
            wheel_drag: None,
            families: Vec::new(),
            page_dialog: None,
            image_dialog: None,
            print_dialog: None,
            note_dialog: None,
            table_dialog: None,
            link_dialog: None,
            pending_cell: None,
            speller: None,
            speller_rx: None,
            spell_on: true,
            ruler: true,
            ruler_drag: None,
            misspelled: std::collections::HashMap::new(),
            spell_cache: std::collections::HashMap::new(),
            spell_version: u64::MAX,
            spell_target: None,
            outer: None,
            full: None,
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
            None => self.message = Some("Formato de imagem não suportado (use PNG, JPEG, WebP, GIF ou BMP).".into()),
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

    /// O documento das páginas (o de fora, com uma célula em edição).
    fn page_doc(&self) -> &Doc {
        self.full.as_ref().map_or(&self.doc, |f| &f.0)
    }

    fn page_layout(&self) -> &Layout {
        self.full.as_ref().map_or(&self.layout, |f| &f.1)
    }

    pub fn in_cell(&self) -> bool {
        self.outer.is_some()
    }

    /// Quebra o documento de novo (com uma célula em edição, o documento todo e a célula).
    fn relayout(&mut self, fonts: &mut cosmic_text::FontSystem) {
        match &self.outer {
            None => {
                self.layout = self.layouter.layout(fonts, &self.doc);
                self.full = None;
            }
            Some(o) => {
                let mut doc = o.doc.clone();
                if let Some(t) = doc.paras.get_mut(o.table).and_then(|p| p.table.as_mut())
                    && let Some(cell) = t.rows.get_mut(o.row).and_then(|r| r.get_mut(o.col)) {
                        cell.doc = self.doc.clone();
                    }
                let full = self.layouter.layout(fonts, &doc);
                self.layout = full.paras.get(o.table).and_then(|b| b.table.as_ref()).and_then(|tb| tb.rows.get(o.row)).and_then(|r| r.get(o.col)).map(|c| c.layout.clone()).unwrap_or_default();
                self.full = Some((doc, full));
            }
        }
    }

    /// Entra na célula (a tabela é o parágrafo `t` do documento de fora).
    fn enter_cell(&mut self, t: usize, r: usize, c: usize, at_end: bool) {
        if self.outer.is_some() {
            self.leave_cell();
        }
        let Some(table) = self.doc.paras.get(t).and_then(|p| p.table.as_ref()) else {
            return;
        };
        if r >= table.rows.len() || c >= table.rows[r].len() {
            return;
        }
        let start = self.doc.clone();
        let page_layout = self.layout.clone();
        let cell_layout = page_layout.paras.get(t).and_then(|b| b.table.as_ref()).and_then(|tb| tb.rows.get(r)).and_then(|row| row.get(c)).map(|cb| cb.layout.clone()).unwrap_or_default();
        let cell_doc = std::mem::take(&mut self.doc.paras[t].table.as_mut().expect("tabela").rows[r][c].doc);
        let outer_doc = std::mem::replace(&mut self.doc, cell_doc);
        self.outer = Some(Outer { doc: outer_doc, start: start.clone(), cursor: self.cursor, anchor: self.anchor, undo: std::mem::take(&mut self.undo), redo: std::mem::take(&mut self.redo), version: self.version, table: t, row: r, col: c });
        self.full = Some((start, page_layout));
        self.layout = cell_layout;
        self.cursor = if at_end { self.doc.end() } else { Pos::default() };
        self.anchor = self.cursor;
        self.typing = None;
        self.last_kind = None;
        self.reveal = true;
        self.stale = true;
    }

    /// Sai da célula: ela volta para a tabela; o que mudou nela vira um passo de desfazer.
    pub fn leave_cell(&mut self) -> bool {
        let Some(o) = self.outer.take() else {
            return false;
        };
        let cell_doc = std::mem::replace(&mut self.doc, o.doc);
        if let Some(cell) = self.doc.paras.get_mut(o.table).and_then(|p| p.table.as_mut()).and_then(|t| t.rows.get_mut(o.row)).and_then(|r| r.get_mut(o.col)) {
            cell.doc = cell_doc;
        }
        self.undo = o.undo;
        self.redo = o.redo;
        if self.version != o.version {
            self.undo.push(Snapshot { doc: o.start, cursor: o.cursor, anchor: o.anchor });
            self.redo.clear();
        }
        let after = Pos::new((o.table + 1).min(self.doc.paras.len() - 1), 0);
        self.cursor = after;
        self.anchor = after;
        if let Some((_, full)) = self.full.take() {
            self.layout = full;
        }
        self.stale = true;
        self.reveal = true;
        self.last_kind = None;
        true
    }

    /// Célula sob o ponto (página, x desde a margem esquerda, y desde o topo da página):
    /// tabela, linha, coluna e o ponto dentro do texto da célula.
    fn cell_at(&self, page: usize, x: f32, y: f32) -> Option<(usize, usize, usize, f32, f32)> {
        let layout = self.page_layout();
        for pl in layout.placed.iter().filter(|p| p.page == page) {
            let pb = &layout.paras[pl.para];
            let Some(tb) = &pb.table else {
                continue;
            };
            let line = &pb.lines[pl.line];
            if y < pl.y || y > pl.y + line.height {
                continue;
            }
            for c in 0..tb.col_x.len() {
                if x >= tb.col_x[c] && x <= tb.col_x[c] + tb.col_w[c] {
                    return Some((pl.para, pl.line, c, x - tb.col_x[c] - mide::richtext::layout::CELL_PAD_X, y - pl.y - mide::richtext::layout::CELL_PAD_Y));
                }
            }
        }
        None
    }

    /// Onde a célula em edição está: página, x e y do canto do texto dela.
    fn cell_origin(&self) -> Option<(usize, f32, f32)> {
        let o = self.outer.as_ref()?;
        let layout = self.page_layout();
        let first = *layout.first.get(o.table)?;
        let pl = layout.placed.get(first + o.row)?;
        let tb = layout.paras.get(o.table)?.table.as_ref()?;
        Some((pl.page, tb.col_x[o.col] + mide::richtext::layout::CELL_PAD_X, pl.y + mide::richtext::layout::CELL_PAD_Y))
    }

    /// Próxima célula (Tab) ou a anterior (Shift+Tab); depois da última, uma linha nova.
    fn next_cell(&mut self, back: bool) {
        let Some((t, r, c)) = self.outer.as_ref().map(|o| (o.table, o.row, o.col)) else {
            return;
        };
        let (rows, cols) = {
            let doc = &self.outer.as_ref().expect("célula").doc;
            let tb = doc.paras[t].table.as_ref().expect("tabela");
            (tb.rows.len(), tb.cols.len())
        };
        let (mut nr, mut nc) = (r as i64, c as i64 + if back { -1 } else { 1 });
        if nc >= cols as i64 {
            nc = 0;
            nr += 1;
        } else if nc < 0 {
            nc = cols as i64 - 1;
            nr -= 1;
        }
        if nr < 0 {
            return;
        }
        if nr as usize >= rows {
            self.table_op("tabela-linha-abaixo");
            return;
        }
        self.leave_cell();
        self.enter_cell(t, nr as usize, nc as usize, false);
    }

    /// Operações de tabela (com a célula em edição como referência).
    fn table_op(&mut self, op: &str) {
        let Some((t, r, c)) = self.outer.as_ref().map(|o| (o.table, o.row, o.col)) else {
            self.message = Some("Clique numa célula da tabela primeiro.".into());
            return;
        };
        self.leave_cell();
        self.begin(None);
        let style = self.doc.style_at(Pos::new(t, 0));
        let Some(table) = self.doc.paras[t].table.as_mut() else {
            return;
        };
        let (mut nr, mut nc) = (r, c);
        let mut gone = false;
        match op {
            "tabela-linha-acima" => table.insert_row(r, &style),
            "tabela-linha-abaixo" => {
                table.insert_row(r + 1, &style);
                nr = r + 1;
            }
            "tabela-coluna-esquerda" => table.insert_col(c, &style),
            "tabela-coluna-direita" => {
                table.insert_col(c + 1, &style);
                nc = c + 1;
            }
            "tabela-excluir-linha" => {
                table.remove_row(r);
                nr = r.min(table.rows.len() - 1);
            }
            "tabela-excluir-coluna" => {
                table.remove_col(c);
                nc = c.min(table.cols.len() - 1);
            }
            "tabela-bordas" => table.borders = !table.borders,
            "tabela-excluir" => gone = true,
            _ => {}
        }
        if gone {
            self.doc.paras.remove(t);
            if self.doc.paras.is_empty() {
                self.doc.paras.push(Paragraph::new("", style, ParaStyle::default()));
            }
            self.cursor = self.doc.clamp(Pos::new(t, 0));
            self.anchor = self.cursor;
            return;
        }
        // A célula entra com o layout novo no próximo quadro.
        self.pending_cell = Some((t, nr, nc));
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

    /// Carrega o dicionário em segundo plano (uma vez) e marca as palavras erradas quando
    /// o texto muda.
    fn spelling(&mut self, ui: &mut Ui) {
        if self.speller.is_none() && self.speller_rx.is_none() {
            let (tx, rx) = std::sync::mpsc::channel();
            let waker = ui.waker();
            std::thread::spawn(move || {
                let s = mide::spell::Speller::load_default();
                if s.is_none() {
                    tracing::info!("editor: sem dicionário de ortografia (instale o hunspell-pt-br)");
                }
                let _ = tx.send(s);
                if let Some(w) = waker {
                    w.wake();
                }
            });
            self.speller_rx = Some(rx);
        }
        if let Some(rx) = &self.speller_rx
            && let Ok(s) = rx.try_recv() {
                if let Some(s) = &s {
                    tracing::info!("editor: dicionário de ortografia {}", s.language);
                }
                self.speller = s;
                self.speller_rx = None;
                self.spell_version = u64::MAX;
            }
        if self.speller.is_none() {
            self.misspelled.clear();
            return;
        }
        if !self.spell_on {
            self.misspelled.clear();
            return;
        }
        // Só o documento de fora (as células ficam para depois) e só quando muda.
        let key = self.version ^ ((self.cursor.para as u64) << 40) ^ ((self.cursor.off as u64) << 20);
        if key == self.spell_version {
            return;
        }
        self.spell_version = key;
        let typing = self.last_kind.is_some_and(|(k, _)| k == Kind::Typing);
        let mut cache = std::mem::take(&mut self.spell_cache);
        let (Some(speller), doc) = (&self.speller, self.full.as_ref().map_or(&self.doc, |f| &f.0)) else {
            return;
        };
        let mut out = std::collections::HashMap::new();
        let mut used = std::collections::HashMap::new();
        for (i, p) in doc.paras.iter().enumerate() {
            if p.is_object() || p.text.is_empty() {
                continue;
            }
            let bad = match cache.remove(&p.text) {
                Some(b) => b,
                None => mide::spell::words(&p.text).into_iter().filter(|&(a, b)| !speller.check(&p.text[a..b])).collect(),
            };
            // A palavra que está sendo digitada não aparece errada antes de terminar.
            let shown: Vec<(usize, usize)> = bad.iter().copied().filter(|&(a, b)| !(typing && self.outer.is_none() && self.cursor.para == i && self.cursor.off >= a && self.cursor.off <= b)).collect();
            if !shown.is_empty() {
                out.insert(i, shown);
            }
            used.insert(p.text.clone(), bad);
        }
        self.spell_cache = used;
        self.misspelled = out;
    }

    /// Clique direito numa palavra errada: guarda as sugestões para o menu.
    fn spell_menu_at(&mut self, pos: Pos) {
        self.spell_target = None;
        if self.outer.is_some() {
            return;
        }
        let Some(words) = self.misspelled.get(&pos.para) else {
            return;
        };
        let Some(&(a, b)) = words.iter().find(|(a, b)| pos.off >= *a && pos.off <= *b) else {
            return;
        };
        let word = self.doc.paras[pos.para].text[a..b].to_string();
        let sugg = self.speller.as_ref().map(|s| s.suggest(&word, 6)).unwrap_or_default();
        self.anchor = Pos::new(pos.para, a);
        self.cursor = Pos::new(pos.para, b);
        self.spell_target = Some((Pos::new(pos.para, a), Pos::new(pos.para, b), word, sugg));
    }

    /// Menu de contexto das páginas: sugestões de ortografia e as ações de edição.
    fn context_menu(&mut self, ui: &mut Ui, area: (f32, f32, f32, f32)) {
        let r = ui.background(area);
        let target = self.spell_target.clone();
        let mut chosen: Option<String> = None;
        let mut learn = false;
        let mut ignore = false;
        let mut action: Option<&str> = None;
        let has_sel = self.sel().is_some();
        ui.context_menu(&r, |m| {
            if let Some((_, _, _, sugg)) = &target {
                if sugg.is_empty() {
                    m.item("Nenhuma sugestão");
                }
                for s in sugg {
                    if m.item(s) {
                        chosen = Some(s.clone());
                    }
                }
                m.separator();
                if m.item("Aprender ortografia") {
                    learn = true;
                }
                if m.item("Ignorar") {
                    ignore = true;
                }
                m.separator();
            }
            if has_sel && m.item("Recortar") {
                action = Some("recortar");
            }
            if has_sel && m.item("Copiar") {
                action = Some("copiar");
            }
            if m.item("Colar") {
                action = Some("colar");
            }
        });
        if let Some((a, b, word, _)) = target.filter(|_| chosen.is_some() || learn || ignore) {
            if let Some(new) = chosen {
                self.begin(None);
                let style = self.doc.style_at(self.doc.next(a));
                self.doc.delete(a, b);
                self.cursor = self.doc.insert_text(a, &new, &style);
                self.anchor = self.cursor;
            }
            if let Some(s) = self.speller.as_mut() {
                if learn {
                    s.learn(&word);
                } else if ignore {
                    s.ignore(&word);
                }
            }
            self.spell_cache.clear();
            self.spell_version = u64::MAX;
            self.spell_target = None;
        }
        if let Some(a) = action {
            self.command(a, ui);
        }
    }

    /// A régua: centímetros, as margens e os recuos do parágrafo (arrastáveis).
    fn ruler_view(&mut self, ui: &mut Ui, y: f32, w: f32, area: (f32, f32, f32, f32), blocked: bool) {
        let theme = ui.theme().clone();
        let z = self.z();
        let page = self.page_doc().page.clone();
        // Numa célula, a régua mostra a coluna dela.
        let (origin, width) = match self.cell_origin() {
            Some((_, cx, _)) => {
                let o = self.outer.as_ref().expect("célula");
                let cw = self.page_layout().paras.get(o.table).and_then(|b| b.table.as_ref()).map_or(72.0, |tb| tb.col_w[o.col]) - 2.0 * mide::richtext::layout::CELL_PAD_X;
                (self.page_x(area) + (page.margins[3] + cx) * z, cw)
            }
            None => (self.page_x(area) + page.margins[3] * z, page.content_width()),
        };
        let ps = self.current_para();
        let first_x = origin + (ps.left_indent + ps.first_indent) * z;
        let left_x = origin + ps.left_indent * z;
        let right_x = origin + (width - ps.right_indent) * z;
        let tab_stops = ps.tab_stops.clone();
        // Arrastar os marcadores.
        let input = ui.input().clone();
        if !blocked
            && let Some(p) = input.pointer {
                let inside = p.1 >= y && p.1 <= y + RULER_H;
                if input.pressed && inside {
                    let near = |x: f32| (p.0 - x).abs() <= 6.0;
                    let upper = p.1 < y + RULER_H / 2.0;
                    let tab = tab_stops.iter().copied().find(|stop| near(origin + stop * z));
                    self.ruler_drag = if near(first_x) && upper {
                        Some(RulerDrag::First)
                    } else if near(right_x) && upper {
                        Some(RulerDrag::Right)
                    } else if near(left_x) {
                        Some(RulerDrag::Left)
                    } else if near(first_x) {
                        Some(RulerDrag::First)
                    } else if near(right_x) {
                        Some(RulerDrag::Right)
                    } else if let Some(tab) = tab {
                        Some(RulerDrag::Tab(tab))
                    } else if (origin..=origin + width * z).contains(&p.0) {
                        let step = mide::richtext::cm(0.25);
                        let at = (((p.0 - origin) / z) / step).round() * step;
                        if at > 0.0 && at < width {
                            self.begin(None);
                            let (a, b) = self.range();
                            self.doc.apply_para(a, b, |s| set_tab_stop(&mut s.tab_stops, None, Some(at)));
                            self.stale = true;
                            self.version += 1;
                            Some(RulerDrag::Tab(at))
                        } else {
                            None
                        }
                    } else {
                        None
                    };
                    if self.ruler_drag.is_some() && tab.is_some() {
                        self.begin(None);
                    } else if matches!(self.ruler_drag, Some(RulerDrag::First | RulerDrag::Left | RulerDrag::Right)) {
                        self.begin(None);
                    }
                }
                if let Some(d) = self.ruler_drag.filter(|_| input.down) {
                    // Em passos de 0,25 cm, como na régua do Word.
                    let step = mide::richtext::cm(0.25);
                    let at = (((p.0 - origin) / z) / step).round() * step;
                    let (a, b) = self.range();
                    let old = ps.clone();
                    let doc = &mut self.doc;
                    match d {
                        RulerDrag::First => doc.apply_para(a, b, |s| s.first_indent = (at - s.left_indent).clamp(-s.left_indent, width - s.left_indent - s.right_indent - 18.0)),
                        RulerDrag::Left => {
                            let first_abs = old.left_indent + old.first_indent;
                            doc.apply_para(a, b, |s| {
                                s.left_indent = at.clamp(0.0, width - s.right_indent - 36.0);
                                s.first_indent = first_abs - s.left_indent;
                            });
                        }
                        RulerDrag::Right => doc.apply_para(a, b, |s| s.right_indent = (width - at).clamp(0.0, width - s.left_indent - 36.0)),
                        RulerDrag::Tab(old) => {
                            let target = if p.1 < y || p.1 > y + RULER_H || at <= 0.0 || at >= width { None } else { Some(at) };
                            doc.apply_para(a, b, |s| set_tab_stop(&mut s.tab_stops, Some(old), target));
                            self.ruler_drag = target.map(RulerDrag::Tab);
                        }
                    }
                    self.stale = true;
                    self.version += 1;
                    ui.request_repaint();
                }
                if input.released || !input.down {
                    self.ruler_drag = None;
                }
            }
        let accent = theme.accent;
        let bg = theme.background;
        let ink = if theme.dark { [190, 190, 196, 255] } else { [70, 70, 76, 255] };
        let defaults = default_tab_stops(width, ps.right_indent, tab_stops.last().copied().unwrap_or(0.0));
        ui.paint_text(|pixmap, text, _| {
            mide::ui::draw::paint_rounded_solid(pixmap, 0.0, y, w, RULER_H, 0.0, [bg[0], bg[1], bg[2], 255]);
            // A faixa do texto em branco; as margens em cinza.
            let page_left = origin - page.margins[3] * z;
            let band_y = y + 9.0;
            mide::ui::draw::paint_rounded_solid(pixmap, page_left, band_y, page.width * z, 14.0, 2.0, [222, 222, 226, 255]);
            mide::ui::draw::paint_rounded_solid(pixmap, origin, band_y, width * z, 14.0, 0.0, [255, 255, 255, 255]);
            // Marcas a cada 0,5 cm e números a cada centímetro.
            let mut c = 0.0f32;
            while c * 72.0 / 2.54 <= width + 0.1 {
                let x = origin + mide::richtext::cm(c) * z;
                let whole = (c.fract()).abs() < 0.01;
                let th = if whole { 5.0 } else { 3.0 };
                mide::ui::draw::paint_rounded_solid(pixmap, x, band_y + 14.0 - th, 0.8, th, 0.0, [120, 120, 126, 255]);
                if whole && c > 0.0 {
                    text.draw(pixmap, &format!("{c:.0}"), x - 3.0, band_y + 0.5, 9.5, mide::ui::text::FontWeight::Regular, [90, 90, 98, 255]);
                }
                c += 0.5;
            }
            // Triangulo deitado (parada de tabulação), com a ponta para a direita.
            let tab_tri = |pixmap: &mut tiny_skia::Pixmap, x: f32, rgba: [u8; 4]| {
                let mut pb = tiny_skia::PathBuilder::new();
                pb.move_to(x - 3.0, y + RULER_H - 11.0);
                pb.line_to(x - 3.0, y + RULER_H - 3.0);
                pb.line_to(x + 4.0, y + RULER_H - 7.0);
                pb.close();
                if let Some(path) = pb.finish() {
                    let mut paint = tiny_skia::Paint { anti_alias: true, ..Default::default() };
                    paint.set_color(tiny_skia::Color::from_rgba8(rgba[0], rgba[1], rgba[2], rgba[3]));
                    pixmap.fill_path(&path, &paint, tiny_skia::FillRule::Winding, mide::ui::draw::tf(), None);
                }
            };
            for stop in &defaults {
                tab_tri(pixmap, origin + stop * z, ink);
            }
            // Marcadores: primeira linha (em cima), recuo esquerdo e direito (embaixo).
            let tri = |pixmap: &mut tiny_skia::Pixmap, x: f32, down: bool| {
                let mut pb = tiny_skia::PathBuilder::new();
                let (top, bottom) = if down { (y + 2.0, y + 9.0) } else { (y + RULER_H - 9.0, y + RULER_H - 2.0) };
                if down {
                    pb.move_to(x - 5.0, top);
                    pb.line_to(x + 5.0, top);
                    pb.line_to(x, bottom);
                } else {
                    pb.move_to(x - 5.0, bottom);
                    pb.line_to(x + 5.0, bottom);
                    pb.line_to(x, top);
                }
                pb.close();
                if let Some(path) = pb.finish() {
                    let mut paint = tiny_skia::Paint { anti_alias: true, ..Default::default() };
                    paint.set_color(tiny_skia::Color::from_rgba8(accent[0], accent[1], accent[2], 255));
                    pixmap.fill_path(&path, &paint, tiny_skia::FillRule::Winding, mide::ui::draw::tf(), None);
                }
            };
            tri(pixmap, first_x, true);
            tri(pixmap, left_x, false);
            tri(pixmap, right_x, true);
            for stop in &tab_stops {
                tab_tri(pixmap, origin + stop * z, [accent[0], accent[1], accent[2], 255]);
            }
        });
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
                    MenuItem::action("imagem-tamanho", "Tamanho da imagem…").enabled(self.doc.paras.get(self.cursor.para).is_some_and(|p| p.image.is_some())),
                    MenuItem::action("quebra-pagina", "Inserir quebra de página").shortcut("Ctrl+Enter"),
                    MenuItem::action("link", "Adicionar link…").shortcut("Ctrl+K"),
                    MenuItem::action("nota", "Inserir nota de rodapé…").shortcut("Ctrl+Alt+F"),
                    MenuItem::action("sumario", "Inserir sumário"),
                    MenuItem::action("atualizar-sumario", "Atualizar sumário"),
                    MenuItem::Separator,
                    MenuItem::action("configurar-pagina", "Configurar página…").shortcut("Ctrl+Shift+P"),
                    MenuItem::action("abnt", "Aplicar normas ABNT…"),
                ],
            },
            MenuGroup {
                title: "Tabela".into(),
                items: {
                    let on = self.in_cell();
                    vec![
                        MenuItem::action("tabela", "Inserir tabela…").shortcut("Ctrl+Alt+T"),
                        MenuItem::Separator,
                        MenuItem::action("tabela-linha-acima", "Inserir linha acima").enabled(on),
                        MenuItem::action("tabela-linha-abaixo", "Inserir linha abaixo").enabled(on),
                        MenuItem::action("tabela-coluna-esquerda", "Inserir coluna à esquerda").enabled(on),
                        MenuItem::action("tabela-coluna-direita", "Inserir coluna à direita").enabled(on),
                        MenuItem::Separator,
                        MenuItem::action("tabela-excluir-linha", "Excluir linha").enabled(on),
                        MenuItem::action("tabela-excluir-coluna", "Excluir coluna").enabled(on),
                        MenuItem::action("tabela-excluir", "Excluir tabela").enabled(on),
                        MenuItem::Separator,
                        MenuItem::action("tabela-bordas", "Mostrar bordas").enabled(on).checked(self.outer.as_ref().and_then(|o| o.doc.paras[o.table].table.as_ref()).is_some_and(|t| t.borders)),
                    ]
                },
            },
            MenuGroup {
                title: "Visualizar".into(),
                items: vec![
                    MenuItem::action("regua", "Mostrar régua").shortcut("Ctrl+Shift+R").checked(self.ruler),
                    MenuItem::Separator,
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
            "imagem-tamanho" => self.open_image_dialog(),
            "quebra-pagina" => self.page_break(),
            "configurar-pagina" => {
                self.leave_cell();
                self.open_page_dialog();
            }
            "imprimir" => {
                self.leave_cell();
                self.open_print_dialog();
            }
            "exportar-pdf" => {
                self.leave_cell();
                self.wants_pdf_path = Some(None);
            }
            "abnt" => {
                self.leave_cell();
                self.abnt_dialog = Some(AbntDialog { font: 0, numbers_from: "2".into() });
            }
            "regua" => self.ruler = !self.ruler,
            "zoom-mais" => self.zoom_by(1.1),
            "zoom-menos" => self.zoom_by(1.0 / 1.1),
            "zoom-real" => self.zoom = 1.0,
            "desfazer" => self.undo(false),
            "refazer" => self.undo(true),
            "copiar" => self.copy(ui, false),
            "recortar" => self.copy(ui, true),
            "colar" => self.paste(ui),
            "selecionar-tudo" => self.select_all(),
            "link" => {
                if self.sel().is_none() {
                    // Sem seleção: a palavra sob o cursor.
                    let (a, b) = self.doc.word_at(self.cursor);
                    self.anchor = a;
                    self.cursor = b;
                }
                let url = self.current_style().link.unwrap_or_default();
                self.link_dialog = Some(LinkDialog { url, focus: true });
            }
            "sumario" => {
                self.leave_cell();
                self.insert_toc();
            }
            "tabela" => {
                self.leave_cell();
                self.table_dialog = Some(TableDialog { rows: 3, cols: 3 });
            }
            op if op.starts_with("tabela-") => self.table_op(op),
            "nota" => self.note_dialog = Some(NoteDialog { id: None, text: String::new(), focus: true }),
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
            "ortografia" => {
                self.spell_on = !self.spell_on;
                self.spell_version = u64::MAX;
            }
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

    fn open_image_dialog(&mut self) {
        let Some(image) = self.doc.paras.get(self.cursor.para).and_then(|p| p.image.as_ref()) else {
            return;
        };
        self.image_dialog = Some(ImageDialog {
            para: self.cursor.para,
            width: format!("{:.2}", to_cm(image.width)).replace('.', ","),
            height: format!("{:.2}", to_cm(image.height)).replace('.', ","),
            ratio: image.height / image.width,
            keep_ratio: true,
            focus: true,
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
            if input.shortcut_plain(letter) && !(letter == "r" && input.shortcut_shift("r") == Some(true)) {
                self.command(action, ui);
            }
        }
        match input.shortcut_shift("z") {
            Some(true) => self.undo(true),
            Some(false) => self.undo(false),
            None => {}
        }
        if input.shortcut_plain("f") {
            self.toggle_find();
        }
        // Títulos como no Word: Ctrl+Alt+1, 2, 3; Ctrl+Alt+0 volta ao corpo.
        if input.shortcut_shift("r") == Some(true) {
            self.ruler = !self.ruler;
        }
        if input.shortcut_plain("k") {
            self.command("link", ui);
        }
        if input.shortcut_alt("t") {
            self.command("tabela", ui);
        }
        if input.shortcut_alt("f") {
            self.note_dialog = Some(NoteDialog { id: None, text: String::new(), focus: true });
        }
        for (key, name) in [("1", "Título 1"), ("2", "Título 2"), ("3", "Título 3"), ("0", "Corpo")] {
            if input.shortcut_alt(key) {
                self.apply_named(name);
            }
        }
        if let Some(back) = input.shortcut_shift("g") { self.find_next(back) }
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
                Key::Up | Key::Down if self.in_cell() && {
                    let lay = &self.layout;
                    let idx = lay.line_of(p).and_then(|pl| lay.placed.iter().position(|x| x.para == pl.para && x.line == pl.line));
                    match (key, idx) {
                        (Key::Up, Some(0)) => true,
                        (Key::Down, Some(i)) => i + 1 >= lay.placed.len(),
                        _ => false,
                    }
                } => {
                    // Na primeira/última linha da célula: a célula de cima/de baixo (ou sai).
                    let (t, r, c, rows) = {
                        let o = self.outer.as_ref().expect("célula");
                        (o.table, o.row, o.col, o.doc.paras[o.table].table.as_ref().map_or(1, |tb| tb.rows.len()))
                    };
                    self.leave_cell();
                    if key == Key::Up && r > 0 {
                        self.enter_cell(t, r - 1, c, true);
                    } else if key == Key::Down && r + 1 < rows {
                        self.enter_cell(t, r + 1, c, false);
                    } else if key == Key::Up {
                        let to = if t > 0 { Pos::new(t - 1, self.doc.paras[t - 1].text.len()) } else { Pos::new(t, 0) };
                        self.move_to(to, false);
                    }
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
                Key::Tab if self.in_cell() => self.next_cell(shift),
                Key::Escape if self.in_cell() => {
                    self.leave_cell();
                }
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
        let pw = self.page_doc().page.width * self.z();
        area.0 + ((area.2 - pw) / 2.0).max(GAP)
    }

    fn page_top(&self, area: (f32, f32, f32, f32), i: usize) -> f32 {
        area.1 + GAP + i as f32 * (self.page_doc().page.height * self.z() + GAP) - self.scroll
    }

    fn content_height(&self) -> f32 {
        GAP + self.page_layout().pages.max(1) as f32 * (self.page_doc().page.height * self.z() + GAP)
    }

    /// Ponto da janela → página e o ponto no texto da página (desde a margem esquerda).
    fn to_page(&self, area: (f32, f32, f32, f32), (px, py): (f32, f32)) -> (usize, f32, f32) {
        let ph = self.page_doc().page.height * self.z();
        let rel = py - area.1 + self.scroll - GAP;
        let page = ((rel / (ph + GAP)).floor().max(0.0) as usize).min(self.page_layout().pages.saturating_sub(1));
        let top = self.page_top(area, page);
        ((page), (px - self.page_x(area)) / self.z() - self.page_doc().page.margins[3], (py - top) / self.z())
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
        if inside && input.scroll != 0.0 && self.pop.is_none() {
            let s = ui.take_scroll();
            if input.ctrl {
                self.zoom_by(if s < 0.0 { 1.1 } else { 1.0 / 1.1 });
            } else {
                self.scroll += s;
            }
            ui.request_repaint();
        }
        if input.secondary && inside && self.outer.is_none() {
            let pos = self.hit(area, p);
            self.spell_menu_at(pos);
            if self.spell_target.is_none() && self.sel().is_none_or(|(a, b)| pos < a || pos > b) {
                self.cursor = pos;
                self.anchor = pos;
            }
        }
        if input.pressed && inside && p.0 < area.0 + area.2 - 14.0 {
            // Clique numa célula: entra nela; fora da célula em edição: sai dela.
            let (page, x, y) = self.to_page(area, p);
            let pos = match self.cell_at(page, x, y) {
                Some((t, r, c, lx, ly)) => {
                    let same = self.outer.as_ref().is_some_and(|o| (o.table, o.row, o.col) == (t, r, c));
                    if !same {
                        self.leave_cell();
                        self.enter_cell(t, r, c, false);
                    }
                    self.doc.clamp(self.layout.hit(0, lx, ly))
                }
                None => {
                    if self.leave_cell() {
                        self.layout = self.full.take().map_or(std::mem::take(&mut self.layout), |f| f.1);
                    }
                    self.hit(area, p)
                }
            };
            let now = Instant::now();
            let (at, count, last) = self.clicks;
            let count = if now.duration_since(at) < Duration::from_millis(450) && last == pos { count + 1 } else { 1 };
            self.clicks = (now, count, pos);
            self.typing = None;
            self.goal_x = None;
            self.last_kind = None;
            self.blink = now;
            // Ctrl+clique num link: abre no navegador.
            if input.ctrl && count == 1
                && let Some(url) = self.doc.paras.get(pos.para).and_then(|p| p.runs().find(|(a, b, st)| st.link.is_some() && pos.off >= *a && pos.off <= *b).and_then(|(_, _, st)| st.link.clone())) {
                    tracing::info!("editor: abrindo o link {url}");
                    let _ = std::process::Command::new("xdg-open").arg(&url).spawn();
                    return;
                }
            match count {
                // Duplo clique no número de uma nota: edita a nota.
                2 if self.doc.note_near(pos).is_some() => {
                    let id = self.doc.note_near(pos).expect("nota");
                    let text = self.doc.note_text(id).unwrap_or("").to_string();
                    self.note_dialog = Some(NoteDialog { id: Some(id), text, focus: true });
                }
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
            let pos = match self.cell_origin() {
                Some((_, cx, cy)) => {
                    let (_, x, y) = self.to_page(area, p);
                    self.doc.clamp(self.layout.hit(0, x - cx, y - cy))
                }
                None => self.hit(area, p),
            };
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
        let ruler_h = if self.ruler { RULER_H } else { 0.0 };
        let area = (0.0, top + FORMAT_H + find_h + ruler_h, w, (h - top - FORMAT_H - find_h - ruler_h - STATUS_H).max(10.0));
        let blocked = ui.menu_open().is_some() || self.pop.is_some() || self.page_dialog.is_some() || self.image_dialog.is_some() || self.abnt_dialog.is_some() || self.print_dialog.is_some() || self.note_dialog.is_some() || self.table_dialog.is_some() || self.link_dialog.is_some();
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
            ui.paint_text(|_, text, _| self.relayout(text.font_system()));
            self.stale = false;
            relaid = true;
            self.cursor = self.doc.clamp(self.cursor);
            self.anchor = self.doc.clamp(self.anchor);
        }
        let _ = relaid;
        if let Some((t, r, c)) = self.pending_cell.take() {
            self.enter_cell(t, r, c, false);
            ui.paint_text(|_, text, _| self.relayout(text.font_system()));
            self.stale = false;
        }
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
                ui.paint_text(|_, text, _| self.relayout(text.font_system()));
                self.stale = false;
            }
        }
        // PDF e impressão pedidos (precisam das fontes do sistema).
        if self.pdf_request.is_some() || self.print_job.is_some() {
            if self.leave_cell() {
                ui.paint_text(|_, text, _| self.relayout(text.font_system()));
                self.stale = false;
            }
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
            // Dentro de uma célula: a linha da célula somada ao lugar dela na página.
            let (cell_page, cell_dy) = self.cell_origin().map_or((None, 0.0), |(pg, _, cy)| (Some(pg), cy));
            if let Some(placed) = self.layout.line_of(self.cursor).copied() {
                let line = &self.layout.paras[self.cursor.para].lines[placed.line];
                let page = cell_page.unwrap_or(placed.page);
                let y = GAP + page as f32 * (self.page_doc().page.height * self.z() + GAP) + (placed.y + cell_dy) * self.z();
                let lh = line.height * self.z();
                if y < self.scroll + 8.0 {
                    self.scroll = y - 8.0;
                } else if y + lh > self.scroll + area.3 - 8.0 {
                    self.scroll = y + lh - area.3 + 8.0;
                }
            }
        }
        self.spelling(ui);
        let max_scroll = (self.content_height() - area.3).max(0.0);
        self.scroll = self.scroll.clamp(0.0, max_scroll);
        // Páginas.
        let k = mide::ui::draw::scale();
        let (pw, ph) = (self.page_doc().page.width * self.z(), self.page_doc().page.height * self.z());
        let px = self.page_x(area);
        let blink_on = (Instant::now().duration_since(self.blink).as_millis() / 530).is_multiple_of(2);
        let focused = ui.input().focused && !ui.typing();
        let caret_on = focused && blink_on && !blocked;
        // Numa célula, o cursor e a seleção são desenhados dentro dela.
        let cell_edit = self.outer.as_ref().map(|o| render::CellEdit { table: o.table, row: o.row, col: o.col, selection: self.sel(), caret: caret_on.then_some(self.cursor) });
        let caret = if cell_edit.is_some() { None } else { caret_on.then_some(self.cursor) };
        let selection = if cell_edit.is_some() { None } else { self.sel() };
        let sel_color = if focused { [179, 215, 255, 255] } else { [220, 220, 224, 255] };
        let bg = if theme.dark { [44, 44, 48, 255] } else { [236, 236, 240, 255] };
        let pages = self.page_layout().pages.max(1);
        let tops: Vec<(usize, f32)> = (0..pages).map(|i| (i, self.page_top(area, i))).filter(|(_, t)| *t + ph >= area.1 && *t <= area.1 + area.3).collect();
        let z = self.z();
        let scroll = self.scroll;
        let spelling = (!self.misspelled.is_empty() && self.outer.is_none()).then_some(&self.misspelled);
        let (doc, layout) = match &self.full {
            Some((d, l)) => (d, l),
            None => (&self.doc, &self.layout),
        };
        let layouter = &mut self.layouter;
        let accent = theme.accent;
        ui.paint_text(|pixmap, text, _| {
            // As páginas rolam por baixo das barras: o que sai da área do documento volta como estava.
            let stride = pixmap.width() as usize * 4;
            let rows = pixmap.height() as usize;
            let top_px = ((area.1 * k).round() as usize).min(rows);
            let bottom_px = (((area.1 + area.3) * k).round() as usize).clamp(top_px, rows);
            let saved_top = pixmap.data()[..top_px * stride].to_vec();
            let saved_bottom = pixmap.data()[bottom_px * stride..].to_vec();
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
                    cell: cell_edit,
                    misspelled: spelling,
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
            pixmap.data_mut()[..top_px * stride].copy_from_slice(&saved_top);
            pixmap.data_mut()[bottom_px * stride..].copy_from_slice(&saved_bottom);
        });
        // Cursor piscando.
        if focused && !blocked {
            ui.wake_after(Duration::from_millis(530));
        }
        // Barra de formatação (em cima das páginas) e a de busca.
        self.format_bar(ui, top, w);
        self.find_bar(ui, top + FORMAT_H, w);
        if self.ruler {
            self.ruler_view(ui, top + FORMAT_H + find_h, w, area, blocked);
        }
        // Rodapé.
        let page_now = match self.cell_origin() {
            Some((pg, _, _)) => pg + 1,
            None => self.layout.line_of(self.cursor).map_or(1, |p| p.page + 1),
        };
        let (words, chars, _) = self.page_doc().counts();
        let status = format!("Página {page_now} de {} · {words} {} · {chars} caracteres · {:.0}%", self.page_layout().pages.max(1), if words == 1 { "palavra" } else { "palavras" }, self.zoom * 100.0);
        ui.paint(|pixmap, _| {
            let c = theme.background;
            mide::ui::draw::paint_rounded_solid(pixmap, 0.0, h - STATUS_H, w, STATUS_H, 0.0, [c[0], c[1], c[2], 255]);
        });
        ui.at(0.0, h - STATUS_H, w, |ui| {
            ui.status_bar(&status);
        });
        // O menu de contexto é desenhado a cada quadro enquanto está aberto (só os
        // diálogos o impedem).
        let dialog = self.page_dialog.is_some() || self.image_dialog.is_some() || self.abnt_dialog.is_some() || self.print_dialog.is_some() || self.note_dialog.is_some() || self.table_dialog.is_some() || self.link_dialog.is_some();
        if !dialog {
            self.context_menu(ui, area);
        }
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
        let (show_face, style_w, family_w, size_w) = bar_widths(w);
        ui.at(12.0, top + 4.0, w - 24.0, |ui| {
            ui.row(|ui| {
                // Estilo de parágrafo (¶) e fonte.
                let style_name = if ps.name.is_empty() { "Corpo".to_string() } else { ps.name.clone() };
                ui.sized(style_w, |ui| {
                    if ui.button_styled(&format!("¶ {style_name}"), ButtonStyle::Bordered).clicked() {
                        pop = Some((Pop::Styles, ui.last_rect()));
                    }
                });
                ui.sized(family_w, |ui| {
                    if ui.button_styled(&format!("{}  ⌄", st.family), ButtonStyle::Bordered).clicked() {
                        pop = Some((Pop::Fonts, ui.last_rect()));
                    }
                });
                if show_face {
                    ui.sized(112.0, |ui| {
                        if ui.button_styled(&format!("{}  ⌄", font_face_label(&st)), ButtonStyle::Bordered).clicked() {
                            pop = Some((Pop::FontFace, ui.last_rect()));
                        }
                    });
                }
                ui.sized(size_w, |ui| {
                    let size = if st.size.fract() == 0.0 { format!("{:.0}", st.size) } else { format!("{:.1}", st.size) };
                    if ui.button_styled(&format!("{size} ⌄"), ButtonStyle::Bordered).clicked() {
                        pop = Some((Pop::Size, ui.last_rect()));
                    }
                });
                // Cor do texto e realce: os ícones são desenhados aqui, sem o texto de reserva.
                match ui.toolbar_buttons(&[("", " ", open == Some(Pop::Color), true), ("", " ", open == Some(Pop::Highlight), true)]) {
                    Some(0) => pop = Some((Pop::Color, seg(ui.last_rect(), 0))),
                    Some(1) => pop = Some((Pop::Highlight, seg(ui.last_rect(), 1))),
                    _ => {}
                }
                let colors = ui.last_rect();
                let (tc, hc) = (st.color, st.highlight);
                let ink = theme.text;
                ui.paint_text(|pixmap, text, _| {
                    let a = seg(colors, 0);
                    text.draw(pixmap, "A", a.0 + 14.0, a.1 + 5.0, 17.0, mide::ui::text::FontWeight::Semibold, [ink[0], ink[1], ink[2], 255]);
                    mide::ui::draw::paint_rounded_solid(pixmap, a.0 + 11.0, a.1 + a.3 - 10.0, 18.0, 4.0, 2.0, [tc[0], tc[1], tc[2], 255]);
                    let b = seg(colors, 1);
                    let (bx, by) = (b.0 + 11.0, b.1 + 8.0);
                    match hc {
                        Some(c) => mide::ui::draw::paint_rounded_solid(pixmap, bx, by, 18.0, 18.0, 5.0, [c[0], c[1], c[2], 255]),
                        None => {
                            mide::ui::draw::paint_rounded_solid(pixmap, bx, by, 18.0, 18.0, 5.0, [ink[0], ink[1], ink[2], 70]);
                            mide::ui::draw::paint_rounded_solid(pixmap, bx + 1.5, by + 1.5, 15.0, 15.0, 3.5, [255, 255, 255, 255]);
                        }
                    }
                    text.draw(pixmap, "a", bx + 5.0, by + 0.5, 14.0, mide::ui::text::FontWeight::Semibold, [ink[0], ink[1], ink[2], 255]);
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
            if matches!(p, Pop::Color | Pop::Highlight) {
                let now = if p == Pop::Color { Some(st.color) } else { st.highlight };
                self.custom_color = now.map(hex_color).unwrap_or_default();
                self.hsv = now.map_or([0.0, 0.0, 1.0], rgb_to_hsv);
                self.wheel_drag = None;
            }
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
        let mut chosen_face: Option<(bool, bool)> = None;
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
            Pop::FontFace => 150.0,
            Pop::Color | Pop::Highlight => 236.0,
            Pop::Size => 110.0,
            _ => 220.0,
        };
        let families = &self.families;
        let query = &mut self.font_query;
        let custom = &mut self.custom_color;
        let hsv = &mut self.hsv;
        let drag = &mut self.wheel_drag;
        let (win_w, win_h) = ui.window_size();
        let show_wheel = win_h - (self.pop_anchor.1 + self.pop_anchor.3) >= 470.0;
        let above_list = if pop == Pop::Fonts { 44.0 + if win_w < 1000.0 { 190.0 } else { 0.0 } } else { 0.0 };
        let list_h = popover_list_height(win_h, self.pop_anchor.1 + self.pop_anchor.3, above_list);
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
                if ui.window_size().0 < 1000.0 {
                    ui.heading("Face");
                    for (bold, italic, label) in [(false, false, "Regular"), (true, false, "Bold"), (false, true, "Italic"), (true, true, "Bold Italic")] {
                        let mark = if (cur.bold, cur.italic) == (bold, italic) { "✓ " } else { "   " };
                        if ui.button_styled(&format!("{mark}{label}"), ButtonStyle::Plain).clicked() {
                            chosen_face = Some((bold, italic));
                        }
                    }
                    ui.separator();
                }
                ui.search_field(query, "Buscar fonte");
                let q = query.to_lowercase();
                let list: Vec<&String> = families.iter().filter(|f| q.is_empty() || f.to_lowercase().contains(&q)).take(400).collect();
                ui.scroll_area(list_h, |ui| {
                    for f in list {
                        let label = if *f == cur.family { format!("✓  {f}") } else { format!("    {f}") };
                        if ui.button_styled(&label, ButtonStyle::Plain).clicked() {
                            chosen_font = Some(f.clone());
                        }
                    }
                });
            }
            Pop::FontFace => {
                for (bold, italic, label) in [(false, false, "Regular"), (true, false, "Bold"), (false, true, "Italic"), (true, true, "Bold Italic")] {
                    let mark = if (cur.bold, cur.italic) == (bold, italic) { "✓ " } else { "   " };
                    if ui.button_styled(&format!("{mark}{label}"), ButtonStyle::Plain).clicked() {
                        chosen_face = Some((bold, italic));
                    }
                }
            }
            Pop::Size => {
                ui.scroll_area(list_h, |ui| {
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
                ui.separator();
                ui.heading("Cor personalizada");
                // O texto digitado (válido e diferente do que a roda mostra) move a roda.
                let typed = parse_hex_color(custom);
                if drag.is_none() && typed.is_some_and(|c| c != hsv_to_rgb(*hsv)) {
                    *hsv = rgb_to_hsv(typed.unwrap_or_default());
                }
                if show_wheel {
                    let pointer = ui.pointer();
                    let held = ui.input().down;
                    let pressed = ui.input().pressed;
                    let mut wheel_rect = (0.0, 0.0, 0.0, 0.0);
                    ui.custom(WHEEL + 6.0, |_, rect| {
                        wheel_rect = ((rect.0 + (rect.2 - WHEEL) / 2.0).round(), rect.1 + 3.0, WHEEL, WHEEL);
                    });
                    ui.space(2.0);
                    let mut bar_rect = (0.0, 0.0, 0.0, 0.0);
                    ui.custom(VALUE_H + 2.0, |_, rect| {
                        bar_rect = (rect.0 + 4.0, rect.1 + 1.0, rect.2 - 8.0, VALUE_H);
                    });
                    let inside = |r: (f32, f32, f32, f32), p: (f32, f32)| p.0 >= r.0 - 6.0 && p.0 <= r.0 + r.2 + 6.0 && p.1 >= r.1 - 6.0 && p.1 <= r.1 + r.3 + 6.0;
                    if pressed && let Some(p) = pointer {
                        *drag = if inside(wheel_rect, p) { Some(WheelDrag::Wheel) } else if inside(bar_rect, p) { Some(WheelDrag::Value) } else { None };
                        // Preto puro esconde a roda: ao escolher um matiz o brilho sobe ao máximo.
                        if *drag == Some(WheelDrag::Wheel) && hsv[2] < 0.05 {
                            hsv[2] = 1.0;
                        }
                    }
                    if !held {
                        *drag = None;
                    }
                    if let (Some(d), Some(p)) = (*drag, pointer) {
                        match d {
                            WheelDrag::Wheel => {
                                let (h, s) = wheel_hit(wheel_rect.0 + WHEEL / 2.0, wheel_rect.1 + WHEEL / 2.0, WHEEL / 2.0, p.0, p.1);
                                (hsv[0], hsv[1]) = (h, s);
                            }
                            WheelDrag::Value => {
                                let r = VALUE_H / 2.0;
                                hsv[2] = ((p.0 - bar_rect.0 - r) / (bar_rect.2 - 2.0 * r)).clamp(0.0, 1.0);
                            }
                        }
                        *custom = hex_color(hsv_to_rgb(*hsv));
                    }
                    let state = *hsv;
                    ui.paint(|pixmap, _| {
                        paint_wheel(pixmap, wheel_rect, state);
                        paint_value_bar(pixmap, bar_rect, state);
                    });
                }
                let shown = parse_hex_color(custom);
                ui.row(|ui| {
                    ui.sized(96.0, |ui| {
                        ui.text_input(custom, "#RRGGBB");
                    });
                    let sw = shown.unwrap_or([200, 200, 204]);
                    ui.tag_toggle([sw[0], sw[1], sw[2], if shown.is_some() { 255 } else { 90 }], false);
                    ui.sized(76.0, |ui| {
                        if ui.button_styled("Aplicar", ButtonStyle::Prominent).clicked()
                            && let Some(c) = shown {
                                if pop == Pop::Color {
                                    chosen_color = Some(c);
                                } else {
                                    chosen_highlight = Some(Some(c));
                                }
                            }
                    });
                });
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
        } else if let Some((bold, italic)) = chosen_face {
            self.char_style(|s| {
                s.bold = bold;
                s.italic = italic;
            });
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
        let had = self.page_dialog.is_some() || self.image_dialog.is_some() || self.abnt_dialog.is_some() || self.print_dialog.is_some() || self.note_dialog.is_some() || self.table_dialog.is_some() || self.link_dialog.is_some();
        self.dialogs_inner(ui);
        // Diálogo fechado: o teclado volta para o texto (o campo dele sumiu).
        let has = self.page_dialog.is_some() || self.image_dialog.is_some() || self.abnt_dialog.is_some() || self.print_dialog.is_some() || self.note_dialog.is_some() || self.table_dialog.is_some() || self.link_dialog.is_some();
        if had && !has {
            ui.blur();
        }
    }

    fn dialogs_inner(&mut self, ui: &mut Ui) {
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
        if let Some(d) = self.image_dialog.as_mut() {
            let (mut ok, mut cancel) = (false, false);
            ui.modal(400.0, |ui| {
                ui.title("Tamanho da imagem");
                ui.switch("Manter proporção", &mut d.keep_ratio);
                if std::mem::take(&mut d.focus) {
                    ui.focus_next_input();
                }
                ui.form_row("Largura (cm)", 100.0, |ui| {
                    ui.text_input(&mut d.width, "10,0");
                });
                if d.keep_ratio {
                    let height = d.width.trim().replace(',', ".").parse::<f32>().ok().filter(|w| w.is_finite() && *w > 0.0).map(|w| format!("{:.2} cm", w * d.ratio)).unwrap_or_else(|| "—".into());
                    ui.form_row("Altura", 100.0, |ui| ui.fixed_label(&height, 100.0));
                } else {
                    ui.form_row("Altura (cm)", 100.0, |ui| {
                        ui.text_input(&mut d.height, "10,0");
                    });
                }
                ui.row(|ui| {
                    ui.push_right(2.0 * 110.0 + 8.0);
                    ui.sized(110.0, |ui| cancel = ui.button("Cancelar").clicked());
                    ui.sized(110.0, |ui| ok = ui.primary_button("Aplicar").clicked());
                });
            });
            if cancel || ui.input().key(Key::Escape) {
                self.image_dialog = None;
            } else if ok || ui.input().key(Key::Enter) {
                let d = self.image_dialog.take().expect("diálogo");
                let parse_cm = |s: &str| s.trim().replace(',', ".").parse::<f32>().ok().filter(|v| v.is_finite() && (0.1..=1000.0).contains(v)).map(cm);
                let width = parse_cm(&d.width);
                let height = if d.keep_ratio { width.map(|w| w * d.ratio) } else { parse_cm(&d.height) };
                match (width, height, self.doc.paras.get(d.para).and_then(|p| p.image.as_ref())) {
                    (Some(width), Some(height), Some(_)) if height.is_finite() && height > 0.0 => {
                        self.begin(None);
                        if let Some(image) = self.doc.paras.get_mut(d.para).and_then(|p| p.image.as_mut()) {
                            if d.keep_ratio {
                                image.resize_to_width(width);
                            } else {
                                image.width = width;
                                image.height = height;
                            }
                        }
                        tracing::info!("editor: imagem redimensionada para {:.2} x {:.2} cm", to_cm(width), to_cm(height));
                    }
                    _ => {
                        self.message = Some("Informe dimensões válidas entre 0,1 e 1000 cm.".into());
                        self.image_dialog = Some(d);
                    }
                }
            }
        }
        if self.print_dialog.is_some() {
            self.print_dialog_view(ui);
        }
        if let Some(d) = self.link_dialog.as_mut() {
            let (mut ok, mut cancel, mut remove) = (false, false, false);
            let had = !d.url.is_empty();
            ui.modal(460.0, |ui| {
                ui.title("Link");
                ui.note("Ctrl+clique no texto abre o endereço no navegador.");
                if std::mem::take(&mut d.focus) {
                    ui.focus_next_input();
                }
                ok = ui.text_input(&mut d.url, "https://exemplo.com.br").submitted();
                ui.row(|ui| {
                    if had {
                        ui.sized(130.0, |ui| remove = ui.button_styled("Remover link", ButtonStyle::Bordered).clicked());
                    }
                    ui.push_right(2.0 * 110.0 + 8.0);
                    ui.sized(110.0, |ui| cancel = ui.button("Cancelar").clicked());
                    ui.sized(110.0, |ui| ok |= ui.primary_button("OK").clicked());
                });
            });
            if cancel || ui.input().key(Key::Escape) {
                self.link_dialog = None;
            } else if ok || remove {
                let d = self.link_dialog.take().expect("diálogo");
                let url = d.url.trim().to_string();
                let url = if url.is_empty() || remove {
                    None
                } else if url.contains("://") || url.starts_with("mailto:") {
                    Some(url)
                } else if url.contains('@') && !url.contains('/') {
                    Some(format!("mailto:{url}"))
                } else {
                    Some(format!("https://{url}"))
                };
                if let Some((a, b)) = self.sel() {
                    self.begin(None);
                    let on = url.is_some();
                    self.doc.apply_char(a, b, |s| {
                        s.link = url.clone();
                        s.color = if on { [0, 102, 204] } else { [0, 0, 0] };
                    });
                }
            }
        }
        if let Some(d) = self.table_dialog.as_mut() {
            let (mut ok, mut cancel) = (false, false);
            ui.modal(360.0, |ui| {
                ui.title("Inserir tabela");
                ui.form_row("Linhas", 90.0, |ui| {
                    ui.stepper(&mut d.rows, 1, 200);
                });
                ui.form_row("Colunas", 90.0, |ui| {
                    ui.stepper(&mut d.cols, 1, 20);
                });
                ui.row(|ui| {
                    ui.push_right(2.0 * 110.0 + 8.0);
                    ui.sized(110.0, |ui| cancel = ui.button("Cancelar").clicked());
                    ui.sized(110.0, |ui| ok = ui.primary_button("Inserir").clicked());
                });
            });
            if cancel || ui.input().key(Key::Escape) {
                self.table_dialog = None;
            } else if ok || ui.input().key(Key::Enter) {
                let d = self.table_dialog.take().expect("diálogo");
                self.begin(None);
                if let Some((a, b)) = self.sel() {
                    self.cursor = self.doc.delete(a, b);
                }
                let after = self.doc.insert_table(self.cursor, d.rows as usize, d.cols as usize);
                let t = after.para - 1;
                self.cursor = after;
                self.anchor = after;
                self.pending_cell = Some((t, 0, 0));
                tracing::info!("editor: tabela {}x{} inserida", d.rows, d.cols);
            }
        }
        if let Some(d) = self.note_dialog.as_mut() {
            let (mut ok, mut cancel, mut remove) = (false, false, false);
            let editing = d.id.is_some();
            ui.modal(480.0, |ui| {
                ui.title(if editing { "Editar nota de rodapé" } else { "Nota de rodapé" });
                ui.note("O número aparece no texto, onde está o cursor, e a nota no pé da mesma página (letra 10, como na ABNT).");
                if std::mem::take(&mut d.focus) {
                    ui.focus_next_input();
                }
                ok = ui.text_input(&mut d.text, "Texto da nota").submitted();
                ui.row(|ui| {
                    if editing {
                        ui.sized(110.0, |ui| remove = ui.button_styled("Remover", ButtonStyle::Bordered).clicked());
                    }
                    ui.push_right(2.0 * 110.0 + 8.0);
                    ui.sized(110.0, |ui| cancel = ui.button("Cancelar").clicked());
                    ui.sized(110.0, |ui| ok |= ui.primary_button("OK").clicked());
                });
            });
            if cancel || ui.input().key(Key::Escape) {
                self.note_dialog = None;
            } else if remove || ok {
                let d = self.note_dialog.take().expect("diálogo");
                self.begin(None);
                match (d.id, remove) {
                    (Some(id), true) => {
                        // Tira a referência do texto (a nota some junto).
                        for i in 0..self.doc.paras.len() {
                            let hits: Vec<(usize, usize)> = self.doc.paras[i].runs().filter(|(_, _, st)| st.note == Some(id)).map(|(a, b, _)| (a, b)).collect();
                            for (a, b) in hits.into_iter().rev() {
                                self.doc.delete(Pos::new(i, a), Pos::new(i, b));
                            }
                        }
                        self.doc.notes.retain(|(n, _)| *n != id);
                        self.cursor = self.doc.clamp(self.cursor);
                        self.anchor = self.cursor;
                    }
                    (Some(id), false) => {
                        if let Some(n) = self.doc.notes.iter_mut().find(|(n, _)| *n == id) {
                            n.1 = d.text.trim().to_string();
                        }
                    }
                    (None, _) => {
                        if let Some((a, b)) = self.sel() {
                            self.cursor = self.doc.delete(a, b);
                        }
                        self.cursor = self.doc.insert_note(self.cursor, d.text.trim());
                        self.anchor = self.cursor;
                        tracing::info!("editor: nota de rodapé inserida");
                    }
                }
            }
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
                let view = render::View { page, origin: (x * k, y * k), scale: scale * k, selection: None, caret: None, selection_color: [0; 4], caret_color: [0; 4], visible: (y * k, (y + ph * scale) * k), cell: None, misspelled: None };
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

/// Altura da lista rolável de um popover: o que cabe abaixo do botão na janela, entre 80 e 300.
fn popover_list_height(window_h: f32, anchor_bottom: f32, fixed: f32) -> f32 {
    (window_h - anchor_bottom - 8.0 - 20.0 - 12.0 - fixed).clamp(80.0, 300.0)
}

/// Larguras da barra de formatação pela largura da janela: (face visível, estilo, família, tamanho).
fn bar_widths(w: f32) -> (bool, f32, f32, f32) {
    let room = w - 24.0;
    let fixed = 84.0 + 164.0 + 164.0 + 124.0;
    if room >= fixed + 118.0 + 170.0 + 112.0 + 64.0 + 80.0 {
        (true, 118.0, 170.0, 64.0)
    } else if room >= fixed + 118.0 + 170.0 + 64.0 + 70.0 {
        (false, 118.0, 170.0, 64.0)
    } else {
        (false, 96.0, 120.0, 56.0)
    }
}

/// Paradas padrão que a régua mostra: depois da última personalizada e do recuo, antes do recuo direito.
fn default_tab_stops(width: f32, right_indent: f32, after: f32) -> Vec<f32> {
    let step = mide::richtext::layout::TAB_STOP;
    (1..).map(|k| k as f32 * step).take_while(|p| *p < width - right_indent).filter(|p| *p > after).collect()
}

/// `#rrggbb`, `rrggbb` ou `#rgb`.
fn parse_hex_color(s: &str) -> Option<[u8; 3]> {
    let s = s.trim().trim_start_matches('#');
    if !s.is_ascii() {
        return None;
    }
    let full = match s.len() {
        3 => s.chars().flat_map(|c| [c, c]).collect::<String>(),
        6 => s.to_string(),
        _ => return None,
    };
    let v = u32::from_str_radix(&full, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

fn hex_color(c: [u8; 3]) -> String {
    format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2])
}

/// HSV (matiz em graus, saturação e brilho de 0 a 1) para RGB.
fn hsv_to_rgb([h, s, v]: [f32; 3]) -> [u8; 3] {
    let h = h.rem_euclid(360.0) / 60.0;
    let (s, v) = (s.clamp(0.0, 1.0), v.clamp(0.0, 1.0));
    let c = v * s;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    [r, g, b].map(|p| ((p + m) * 255.0).round().clamp(0.0, 255.0) as u8)
}

fn rgb_to_hsv(c: [u8; 3]) -> [f32; 3] {
    let [r, g, b] = c.map(|p| p as f32 / 255.0);
    let max = r.max(g).max(b);
    let d = max - r.min(g).min(b);
    let h = if d == 0.0 {
        0.0
    } else if max == r {
        60.0 * ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    [h, if max == 0.0 { 0.0 } else { d / max }, max]
}

/// Matiz e saturação do ponto `(px, py)` na roda de centro `(cx, cy)` e raio `r`: o ângulo
/// (0° à direita, crescendo no sentido horário) é o matiz e a distância, a saturação.
fn wheel_hit(cx: f32, cy: f32, r: f32, px: f32, py: f32) -> (f32, f32) {
    let (dx, dy) = (px - cx, py - cy);
    (dy.atan2(dx).to_degrees().rem_euclid(360.0), (dx.hypot(dy) / r).clamp(0.0, 1.0))
}

/// O ponto da roda para um matiz e uma saturação (o inverso de `wheel_hit`).
fn wheel_point(cx: f32, cy: f32, r: f32, hue: f32, sat: f32) -> (f32, f32) {
    let a = hue.to_radians();
    (cx + a.cos() * sat * r, cy + a.sin() * sat * r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::rc::Rc;

    fn relayout(r: &mut Rich, fs: &mut cosmic_text::FontSystem) {
        r.layout = r.layouter.layout(fs, &r.doc);
        r.stale = false;
    }

    #[test]
    fn roda_de_cores_converte_e_aponta() {
        assert_eq!(hsv_to_rgb([0.0, 1.0, 1.0]), [255, 0, 0]);
        assert_eq!(hsv_to_rgb([120.0, 1.0, 1.0]), [0, 255, 0]);
        assert_eq!(hsv_to_rgb([240.0, 1.0, 1.0]), [0, 0, 255]);
        assert_eq!(hsv_to_rgb([0.0, 0.0, 1.0]), [255, 255, 255]);
        assert_eq!(hsv_to_rgb([200.0, 0.7, 0.0]), [0, 0, 0]);
        // Ida e volta por cores variadas: sai a mesma cor (até 1 de diferença por arredondamento).
        for c in [[255, 136, 0], [12, 200, 90], [60, 60, 200], [128, 128, 128], [255, 255, 0], [3, 7, 11]] {
            let back = hsv_to_rgb(rgb_to_hsv(c));
            assert!(c.iter().zip(back).all(|(a, b)| (*a as i32 - b as i32).abs() <= 1), "{c:?} -> {back:?}");
        }
        // Direita = vermelho, embaixo = verde-azulado (90°, sentido horário), centro = branco.
        let (h, s) = wheel_hit(50.0, 50.0, 40.0, 90.0, 50.0);
        assert!(h.abs() < 0.01 && (s - 1.0).abs() < 0.01);
        let (h, _) = wheel_hit(50.0, 50.0, 40.0, 50.0, 90.0);
        assert!((h - 90.0).abs() < 0.01);
        assert_eq!(wheel_hit(50.0, 50.0, 40.0, 50.0, 50.0).1, 0.0);
        // Arrastar para fora da roda prende na borda.
        assert_eq!(wheel_hit(50.0, 50.0, 40.0, 500.0, 50.0).1, 1.0);
        // `wheel_point` desfaz `wheel_hit`.
        let (x, y) = wheel_point(50.0, 50.0, 40.0, 200.0, 0.5);
        let (h, s) = wheel_hit(50.0, 50.0, 40.0, x, y);
        assert!((h - 200.0).abs() < 0.01 && (s - 0.5).abs() < 0.01);
    }

    #[test]
    fn cor_hexadecimal_e_paradas_padrao_e_barra() {
        assert_eq!(parse_hex_color("#FF8800"), Some([255, 136, 0]));
        assert_eq!(parse_hex_color("ff8800"), Some([255, 136, 0]));
        assert_eq!(parse_hex_color(" #f80 "), Some([255, 136, 0]));
        assert_eq!(parse_hex_color("#12345"), None);
        assert_eq!(parse_hex_color("xyzxyz"), None);
        assert_eq!(hex_color([255, 136, 0]), "#FF8800");
        assert_eq!(default_tab_stops(100.0, 0.0, 0.0), vec![36.0, 72.0]);
        assert_eq!(default_tab_stops(100.0, 0.0, 40.0), vec![72.0]);
        assert_eq!(default_tab_stops(100.0, 40.0, 0.0), vec![36.0]);
        // Em qualquer largura a partir do mínimo da janela, a barra inteira cabe.
        for w in [920.0, 1000.0, 1040.0, 1104.0, 1400.0] {
            let (face, a, b, c) = bar_widths(w);
            let total = a + b + c + if face { 112.0 } else { 0.0 } + 84.0 + 164.0 + 164.0 + 124.0 + 70.0;
            assert!(total <= w - 24.0, "{w}: {total}");
        }
    }

    #[test]
    fn lista_do_popover_cabe_na_janela() {
        assert_eq!(popover_list_height(660.0, 100.0, 0.0), 300.0);
        assert_eq!(popover_list_height(300.0, 100.0, 0.0), 160.0);
        assert_eq!(popover_list_height(300.0, 100.0, 44.0), 116.0);
        assert_eq!(popover_list_height(200.0, 150.0, 0.0), 80.0);
    }

    #[test]
    fn face_da_fonte_reflete_negrito_e_italico() {
        let regular = CharStyle::default();
        let bold = CharStyle { bold: true, ..regular.clone() };
        let italic = CharStyle { italic: true, ..regular.clone() };
        let bold_italic = CharStyle { bold: true, italic: true, ..regular };

        assert_eq!(font_face_label(&CharStyle::default()), "Regular");
        assert_eq!(font_face_label(&bold), "Bold");
        assert_eq!(font_face_label(&italic), "Italic");
        assert_eq!(font_face_label(&bold_italic), "Bold Italic");
    }

    #[test]
    fn parada_de_tabulacao_pode_ser_criada_mover_e_removida() {
        let mut stops = vec![mide::richtext::cm(2.0), mide::richtext::cm(4.0)];
        set_tab_stop(&mut stops, Some(mide::richtext::cm(2.0)), Some(mide::richtext::cm(3.0)));
        assert_eq!(stops, vec![mide::richtext::cm(3.0), mide::richtext::cm(4.0)]);
        set_tab_stop(&mut stops, Some(mide::richtext::cm(3.0)), None);
        assert_eq!(stops, vec![mide::richtext::cm(4.0)]);
    }

    #[test]
    fn dialogo_de_imagem_carrega_tamanho_e_proporcao() {
        let image = Image {
            data: Rc::new(Vec::new()),
            format: "png",
            pixmap: Rc::new(tiny_skia::Pixmap::new(28, 14).unwrap()),
            width: mide::richtext::cm(10.0),
            height: mide::richtext::cm(5.0),
        };
        let mut doc = Doc::new();
        doc.paras[0] = Paragraph::image(image, CharStyle::default(), ParaStyle::default());
        let mut rich = Rich::new(doc);

        rich.open_image_dialog();

        let dialog = rich.image_dialog.expect("diálogo da imagem");
        assert_eq!(dialog.width, "10,00");
        assert_eq!(dialog.height, "5,00");
        assert!((dialog.ratio - 0.5).abs() < 0.001);
        assert!(dialog.keep_ratio);
        assert!(dialog.focus);
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
