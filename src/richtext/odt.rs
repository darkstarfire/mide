//! ODT (LibreOffice, OpenDocument): ler e gravar, escrito aqui.
//!
//! Mesmo conteúdo do DOCX: estilos de parágrafo (Título, Título 1–3, Citação, Legenda,
//! Referência viram os estilos do LibreOffice), fonte, tamanho, negrito, itálico,
//! sublinhado, tachado, cor, realce, sobrescrito, subscrito, alinhamento, recuos,
//! entrelinha, espaços, quebra de página, listas, imagens, papel, margens, cabeçalho,
//! rodapé e número de página.

use std::collections::HashMap;

use super::{
    docx::{unzip, zip_files},
    xml::{self, escape, Element, Node},
    Align, Baseline, CharStyle, Doc, Image, ListKind, NumberPosition, PageSetup, ParaStyle, Paragraph, Span, LINE_BREAK, NOTE, OBJECT,
};

const NS: &str = "xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" xmlns:style=\"urn:oasis:names:tc:opendocument:xmlns:style:1.0\" xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\" xmlns:table=\"urn:oasis:names:tc:opendocument:xmlns:table:1.0\" xmlns:draw=\"urn:oasis:names:tc:opendocument:xmlns:drawing:1.0\" xmlns:fo=\"urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" xmlns:svg=\"urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0\" xmlns:meta=\"urn:oasis:names:tc:opendocument:xmlns:meta:1.0\" office:version=\"1.3\"";

/// Altura do cabeçalho/rodapé no ODF (linha mínima e o espaço até o texto).
const HF_MIN: f32 = 12.0;
const HF_GAP: f32 = 6.0;
const HF_SPACE: f32 = HF_MIN + HF_GAP;

fn odf_style(name: &str) -> Option<(&'static str, &'static str)> {
    Some(match name {
        "Título" => ("Title", "Title"),
        "Título 1" => ("Heading_20_1", "Heading 1"),
        "Título 2" => ("Heading_20_2", "Heading 2"),
        "Título 3" => ("Heading_20_3", "Heading 3"),
        "Citação" => ("Quotations", "Quotations"),
        "Legenda" => ("Caption", "Caption"),
        "Referência" => ("Bibliography_20_1", "Bibliography 1"),
        _ => return None,
    })
}

fn mide_style(name: &str) -> Option<&'static str> {
    Some(match name.replace("_20_", " ").to_lowercase().as_str() {
        "title" => "Título",
        "heading 1" => "Título 1",
        "heading 2" => "Título 2",
        "heading 3" => "Título 3",
        "quotations" | "quote" => "Citação",
        "caption" | "figure" | "illustration" => "Legenda",
        "bibliography 1" | "bibliography" => "Referência",
        _ => return None,
    })
}

fn color(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

fn text_props(s: &CharStyle) -> String {
    let f = escape(&s.family);
    let mut p = format!("<style:text-properties style:font-name=\"{f}\" fo:font-family=\"{f}\" style:font-family-asian=\"{f}\" style:font-family-complex=\"{f}\" fo:font-size=\"{}pt\" fo:color=\"{}\"", s.size, color(s.color));
    if s.bold {
        p.push_str(" fo:font-weight=\"bold\" style:font-weight-asian=\"bold\" style:font-weight-complex=\"bold\"");
    }
    if s.italic {
        p.push_str(" fo:font-style=\"italic\" style:font-style-asian=\"italic\" style:font-style-complex=\"italic\"");
    }
    if s.underline {
        p.push_str(" style:text-underline-style=\"solid\" style:text-underline-width=\"auto\" style:text-underline-color=\"font-color\"");
    }
    if s.strike {
        p.push_str(" style:text-line-through-style=\"solid\"");
    }
    if let Some(h) = s.highlight {
        p.push_str(&format!(" fo:background-color=\"{}\"", color(h)));
    }
    match s.baseline {
        Baseline::Super => p.push_str(" style:text-position=\"super 58%\""),
        Baseline::Sub => p.push_str(" style:text-position=\"sub 58%\""),
        Baseline::Normal => {}
    }
    p.push_str("/>");
    p
}

fn para_props(st: &ParaStyle, content_width: f32) -> String {
    let list_left = if st.list.is_some() { 18.0 * (st.level as f32 + 1.0) } else { 0.0 };
    let mut tabs = String::new();
    if st.leader || !st.tab_stops.is_empty() {
        tabs.push_str("<style:tab-stops>");
        for stop in st.tab_stops.iter().copied().filter(|stop| stop.is_finite() && *stop > 0.0) {
            tabs.push_str(&format!("<style:tab-stop style:position=\"{stop:.2}pt\" style:type=\"left\"/>"));
        }
        if st.leader {
            tabs.push_str(&format!("<style:tab-stop style:position=\"{:.2}pt\" style:type=\"right\" style:leader-style=\"dotted\" style:leader-text=\".\"/>", content_width - st.left_indent - st.right_indent));
        }
        tabs.push_str("</style:tab-stops>");
    }
    format!(
        "<style:paragraph-properties fo:text-align=\"{}\" fo:margin-left=\"{:.2}pt\" fo:margin-right=\"{:.2}pt\" fo:text-indent=\"{:.2}pt\" fo:margin-top=\"{:.2}pt\" fo:margin-bottom=\"{:.2}pt\" fo:line-height=\"{:.0}%\"{}>{tabs}</style:paragraph-properties>",
        match st.align {
            Align::Left => "start",
            Align::Center => "center",
            Align::Right => "end",
            Align::Justify => "justify",
        },
        st.left_indent + list_left,
        st.right_indent,
        st.first_indent,
        st.space_before,
        st.space_after,
        st.line_spacing * 100.0,
        if st.page_break_before { " fo:break-before=\"page\"" } else { "" },
    )
}

/// Texto com os espaços seguidos, tabulações e quebras do ODF: espaços no começo e os
/// que seguem o primeiro de uma sequência vão como `<text:s/>` (o ODF junta os outros).
fn odf_text(text: &str) -> String {
    let mut out = String::new();
    let mut spaces = 0;
    let flush = |spaces: &mut usize, out: &mut String| {
        if *spaces > 0 {
            if out.is_empty() || out.ends_with("/>") {
                out.push_str(&format!("<text:s text:c=\"{}\"/>", *spaces));
            } else {
                out.push(' ');
                if *spaces > 1 {
                    out.push_str(&format!("<text:s text:c=\"{}\"/>", *spaces - 1));
                }
            }
            *spaces = 0;
        }
    };
    for c in text.chars() {
        match c {
            ' ' => spaces += 1,
            '\t' => {
                flush(&mut spaces, &mut out);
                out.push_str("<text:tab/>");
            }
            LINE_BREAK => {
                flush(&mut spaces, &mut out);
                out.push_str("<text:line-break/>");
            }
            OBJECT | NOTE => {}
            c => {
                flush(&mut spaces, &mut out);
                out.push_str(&escape(&c.to_string()));
            }
        }
    }
    // Espaços no fim do trecho: todos como <text:s/> (o leitor junta os do fim).
    if spaces > 0 {
        out.push_str(&format!("<text:s text:c=\"{spaces}\"/>"));
    }
    out
}

fn list_style(name: &str, kind: ListKind) -> String {
    let mut s = format!("<text:list-style style:name=\"{name}\">");
    for lvl in 1..=9 {
        let pos = format!(
            "<style:list-level-properties text:list-level-position-and-space-mode=\"label-alignment\"><style:list-level-label-alignment text:label-followed-by=\"listtab\" text:list-tab-stop-position=\"{:.2}pt\" fo:text-indent=\"-18pt\" fo:margin-left=\"{:.2}pt\"/></style:list-level-properties>",
            18.0 * lvl as f32,
            18.0 * lvl as f32
        );
        match kind {
            ListKind::Bullet | ListKind::Dash => {
                let ch = if kind == ListKind::Dash { "–" } else { ["•", "◦", "▪"][(lvl - 1) % 3] };
                s.push_str(&format!("<text:list-level-style-bullet text:level=\"{lvl}\" text:bullet-char=\"{ch}\">{pos}</text:list-level-style-bullet>"));
            }
            ListKind::Number | ListKind::Letter => {
                let (fmt, suffix) = if kind == ListKind::Number { ("1", ".") } else { ("a", ")") };
                s.push_str(&format!("<text:list-level-style-number text:level=\"{lvl}\" style:num-format=\"{fmt}\" style:num-suffix=\"{suffix}\">{pos}</text:list-level-style-number>"));
            }
        }
    }
    s.push_str("</text:list-style>");
    s
}

/// O que os parágrafos juntam ao serem gravados: estilos automáticos, imagens e notas.
#[derive(Default)]
struct WriteCtx {
    auto: String,
    pstyles: Vec<(ParaStyle, String)>,
    tstyles: Vec<(CharStyle, String)>,
    pictures: Vec<(String, Vec<u8>, &'static str)>,
    last_list: Option<ListKind>,
    note_n: usize,
    tables: usize,
    cell_styles: Vec<(Option<[u8; 3]>, bool, String)>,
}

/// Grava parágrafos (do documento ou de uma célula) em ODF.
fn write_paras(paras: &[Paragraph], content_width: f32, doc: &Doc, ctx: &mut WriteCtx) -> String {
    let mut body = String::new();
    for p in paras {
        let st = &p.style;
        if let Some(table) = &p.table {
            body.push_str(&write_table(table, doc, ctx));
            ctx.last_list = None;
            continue;
        }
        let key = ParaStyle { list: st.list, level: st.level, ..st.clone() };
        let pname = match ctx.pstyles.iter().find(|(s, _)| *s == key) {
            Some((_, n)) => n.clone(),
            None => {
                let n = format!("P{}", ctx.pstyles.len() + 1);
                let parent = odf_style(&st.name).map_or(String::new(), |(id, _)| format!(" style:parent-style-name=\"{id}\""));
                ctx.auto.push_str(&format!("<style:style style:name=\"{n}\" style:family=\"paragraph\"{parent}>{}</style:style>", para_props(st, content_width)));
                ctx.pstyles.push((key, n.clone()));
                n
            }
        };
        let mut inner = String::new();
        if let Some(img) = &p.image {
            let ext = if img.format == "png" { "png" } else { "jpg" };
            let file = format!("Pictures/imagem{}.{ext}", ctx.pictures.len() + 1);
            ctx.pictures.push((file.clone(), img.data.as_ref().clone(), if img.format == "png" { "image/png" } else { "image/jpeg" }));
            inner.push_str(&format!(
                "<draw:frame draw:name=\"Imagem{}\" text:anchor-type=\"as-char\" svg:width=\"{:.2}pt\" svg:height=\"{:.2}pt\"><draw:image xlink:href=\"{file}\" xlink:type=\"simple\" xlink:show=\"embed\" xlink:actuate=\"onLoad\"/></draw:frame>",
                ctx.pictures.len(),
                img.width,
                img.height
            ));
        } else {
            for (a, b, s) in p.runs() {
                if a == b {
                    continue;
                }
                if let Some(id) = s.note {
                    ctx.note_n += 1;
                    inner.push_str(&format!(
                        "<text:note text:id=\"ftn{id}\" text:note-class=\"footnote\"><text:note-citation>{}</text:note-citation><text:note-body><text:p>{}</text:p></text:note-body></text:note>",
                        ctx.note_n,
                        odf_text(doc.note_text(id).unwrap_or(""))
                    ));
                    continue;
                }
                let tname = match ctx.tstyles.iter().find(|(t, _)| t == s) {
                    Some((_, n)) => n.clone(),
                    None => {
                        let n = format!("T{}", ctx.tstyles.len() + 1);
                        ctx.auto.push_str(&format!("<style:style style:name=\"{n}\" style:family=\"text\">{}</style:style>", text_props(s)));
                        ctx.tstyles.push((s.clone(), n.clone()));
                        n
                    }
                };
                let span = format!("<text:span text:style-name=\"{tname}\">{}</text:span>", odf_text(&p.text[a..b]));
                match &s.link {
                    Some(url) => inner.push_str(&format!("<text:a xlink:type=\"simple\" xlink:href=\"{}\">{span}</text:a>", escape(url))),
                    None => inner.push_str(&span),
                }
            }
        }
        let para = format!("<text:p text:style-name=\"{pname}\">{inner}</text:p>");
        match st.list {
            Some(kind) => {
                let lname = match kind {
                    ListKind::Bullet => "LBullet",
                    ListKind::Dash => "LDash",
                    ListKind::Number => "LNumber",
                    ListKind::Letter => "LLetter",
                };
                // Itens seguidos continuam a numeração; os níveis são listas dentro de itens.
                let cont = if ctx.last_list == Some(kind) { " text:continue-numbering=\"true\"" } else { "" };
                let mut item = para;
                for _ in 0..st.level {
                    item = format!("<text:list><text:list-item>{item}</text:list-item></text:list>");
                }
                body.push_str(&format!("<text:list text:style-name=\"{lname}\"{cont}><text:list-item>{item}</text:list-item></text:list>"));
                ctx.last_list = Some(kind);
            }
            None => {
                body.push_str(&para);
                ctx.last_list = None;
            }
        }
    }
    body
}

/// Grava uma tabela: colunas com largura, células com borda e fundo, e o conteúdo.
fn write_table(table: &super::Table, doc: &Doc, ctx: &mut WriteCtx) -> String {
    ctx.tables += 1;
    let tname = format!("Tabela{}", ctx.tables);
    let total: f32 = table.cols.iter().sum();
    ctx.auto.push_str(&format!("<style:style style:name=\"{tname}\" style:family=\"table\"><style:table-properties style:width=\"{total:.2}pt\" table:align=\"left\"/></style:style>"));
    let mut s = format!("<table:table table:name=\"{tname}\" table:style-name=\"{tname}\">");
    for (i, w) in table.cols.iter().enumerate() {
        let cname = format!("{tname}.C{}", i + 1);
        ctx.auto.push_str(&format!("<style:style style:name=\"{cname}\" style:family=\"table-column\"><style:table-column-properties style:column-width=\"{w:.2}pt\"/></style:style>"));
        s.push_str(&format!("<table:table-column table:style-name=\"{cname}\"/>"));
    }
    for row in &table.rows {
        s.push_str("<table:table-row>");
        for (c, cell) in row.iter().enumerate() {
            let key = (cell.shade, table.borders);
            let cs = match ctx.cell_styles.iter().find(|(sh, b, _)| (*sh, *b) == key) {
                Some((_, _, n)) => n.clone(),
                None => {
                    let n = format!("Celula{}", ctx.cell_styles.len() + 1);
                    let border = if table.borders { "0.5pt solid #808084" } else { "none" };
                    let bg = cell.shade.map_or(String::new(), |c| format!(" fo:background-color=\"{}\"", color(c)));
                    ctx.auto.push_str(&format!("<style:style style:name=\"{n}\" style:family=\"table-cell\"><style:table-cell-properties fo:padding=\"4pt\" fo:border=\"{border}\"{bg}/></style:style>"));
                    ctx.cell_styles.push((cell.shade, table.borders, n.clone()));
                    n
                }
            };
            let w = table.cols.get(c).copied().unwrap_or(72.0);
            s.push_str(&format!("<table:table-cell table:style-name=\"{cs}\" office:value-type=\"string\">"));
            let saved = ctx.last_list.take();
            s.push_str(&write_paras(&cell.doc.paras, w - 8.0, doc, ctx));
            ctx.last_list = saved;
            s.push_str("</table:table-cell>");
        }
        s.push_str("</table:table-row>");
    }
    s.push_str("</table:table>");
    s
}

/// Grava o documento em ODT.
pub fn write(doc: &Doc) -> Vec<u8> {
    let mut ctx = WriteCtx::default();
    for (kind, name) in [(ListKind::Bullet, "LBullet"), (ListKind::Dash, "LDash"), (ListKind::Number, "LNumber"), (ListKind::Letter, "LLetter")] {
        ctx.auto.push_str(&list_style(name, kind));
    }
    let body = write_paras(&doc.paras, doc.page.content_width(), doc, &mut ctx);
    let WriteCtx { auto, pictures, .. } = ctx;
    let content = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<office:document-content {NS}><office:automatic-styles>{auto}</office:automatic-styles><office:body><office:text>{body}</office:text></office:body></office:document-content>");
    // Estilos: os nomeados, a página e o cabeçalho/rodapé.
    let page = &doc.page;
    // O estilo padrão só com a fonte e o tamanho do corpo (cor, sublinhado e o resto
    // ficam nos trechos).
    let body_style = doc.paras.iter().find(|p| p.style.name.is_empty() && !p.text.is_empty()).map(|p| p.spans[0].style.clone()).unwrap_or_default();
    let base = CharStyle { family: body_style.family.clone(), size: body_style.size, ..CharStyle::default() };
    let mut named = format!("<style:default-style style:family=\"paragraph\">{}</style:default-style><style:style style:name=\"Standard\" style:family=\"paragraph\" style:class=\"text\"/>", text_props(&base));
    for (name, id, display) in [("Título", "Title", "Title"), ("Título 1", "Heading_20_1", "Heading 1"), ("Título 2", "Heading_20_2", "Heading 2"), ("Título 3", "Heading_20_3", "Heading 3"), ("Citação", "Quotations", "Quotations"), ("Legenda", "Caption", "Caption"), ("Referência", "Bibliography_20_1", "Bibliography 1")] {
        let level = name.strip_prefix("Título ").and_then(|n| n.parse::<u8>().ok());
        let outline = level.map_or(String::new(), |l| format!(" style:default-outline-level=\"{l}\""));
        named.push_str(&format!("<style:style style:name=\"{id}\" style:display-name=\"{display}\" style:family=\"paragraph\" style:parent-style-name=\"Standard\"{outline}/>"));
    }
    let small = CharStyle { size: 10.0, ..base.clone() };
    let hf = |text: &str, number: Option<&str>| -> String {
        let mut s = String::new();
        if !text.is_empty() {
            s.push_str(&format!("<text:p text:style-name=\"HFCenter\"><text:span text:style-name=\"HFText\">{}</text:span></text:p>", odf_text(text)));
        }
        if let Some(align) = number {
            s.push_str(&format!("<text:p text:style-name=\"HF{align}\"><text:span text:style-name=\"HFText\"><text:page-number text:select-page=\"current\">1</text:page-number></text:span></text:p>"));
        }
        s
    };
    let num_top = (page.numbers.show && page.numbers.position == NumberPosition::TopRight).then_some("Right");
    let num_bottom = match (page.numbers.show, page.numbers.position) {
        (true, NumberPosition::BottomCenter) => Some("Center"),
        (true, NumberPosition::BottomRight) => Some("Right"),
        _ => None,
    };
    let header = hf(&page.header, num_top);
    let footer = hf(&page.footer, num_bottom);
    // No ODF, a margem de cima vai até o cabeçalho (e a de baixo até o rodapé): o texto
    // começa depois da altura dele. Tira essa altura para o texto ficar no mesmo lugar.
    let (top, bottom) = (
        if header.is_empty() { page.margins[0] } else { (page.margins[0] - HF_SPACE).max(12.0) },
        if footer.is_empty() { page.margins[2] } else { (page.margins[2] - HF_SPACE).max(12.0) },
    );
    let styles = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<office:document-styles {NS}><office:styles>{named}</office:styles><office:automatic-styles><style:style style:name=\"HFCenter\" style:family=\"paragraph\"><style:paragraph-properties fo:text-align=\"center\"/></style:style><style:style style:name=\"HFRight\" style:family=\"paragraph\"><style:paragraph-properties fo:text-align=\"end\"/></style:style><style:style style:name=\"HFText\" style:family=\"text\">{}</style:style><style:page-layout style:name=\"pm1\"><style:page-layout-properties fo:page-width=\"{:.2}pt\" fo:page-height=\"{:.2}pt\" style:print-orientation=\"{}\" fo:margin-top=\"{:.2}pt\" fo:margin-right=\"{:.2}pt\" fo:margin-bottom=\"{:.2}pt\" fo:margin-left=\"{:.2}pt\"/><style:header-style><style:header-footer-properties fo:min-height=\"{HF_MIN}pt\" fo:margin-bottom=\"{HF_GAP}pt\"/></style:header-style><style:footer-style><style:header-footer-properties fo:min-height=\"{HF_MIN}pt\" fo:margin-top=\"{HF_GAP}pt\"/></style:footer-style></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name=\"Standard\" style:page-layout-name=\"pm1\">{}{}</style:master-page></office:master-styles></office:document-styles>",
        text_props(&small),
        page.width,
        page.height,
        if page.landscape() { "landscape" } else { "portrait" },
        top,
        page.margins[1],
        bottom,
        page.margins[3],
        if header.is_empty() { String::new() } else { format!("<style:header>{header}</style:header>") },
        if footer.is_empty() { String::new() } else { format!("<style:footer>{footer}</style:footer>") },
    );
    let meta = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<office:document-meta {NS}><office:meta><meta:generator>mide Editor de Texto</meta:generator><meta:user-defined meta:name=\"mideShowFrom\">{}</meta:user-defined></office:meta></office:document-meta>",
        page.numbers.show_from
    );
    let mut manifest = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\" manifest:version=\"1.3\"><manifest:file-entry manifest:full-path=\"/\" manifest:version=\"1.3\" manifest:media-type=\"application/vnd.oasis.opendocument.text\"/><manifest:file-entry manifest:full-path=\"content.xml\" manifest:media-type=\"text/xml\"/><manifest:file-entry manifest:full-path=\"styles.xml\" manifest:media-type=\"text/xml\"/><manifest:file-entry manifest:full-path=\"meta.xml\" manifest:media-type=\"text/xml\"/>");
    for (file, _, mime) in &pictures {
        manifest.push_str(&format!("<manifest:file-entry manifest:full-path=\"{file}\" manifest:media-type=\"{mime}\"/>"));
    }
    manifest.push_str("</manifest:manifest>");
    let mut files: Vec<(String, Vec<u8>)> = vec![
        ("mimetype".into(), b"application/vnd.oasis.opendocument.text".to_vec()),
        ("META-INF/manifest.xml".into(), manifest.into_bytes()),
        ("content.xml".into(), content.into_bytes()),
        ("styles.xml".into(), styles.into_bytes()),
        ("meta.xml".into(), meta.into_bytes()),
    ];
    for (file, data, _) in pictures {
        files.push((file, data));
    }
    zip_files(&files, Some("mimetype"))
}

/// Comprimento do ODF em pontos ("2.5cm", "12pt", "0.5in", "10mm", "1pc").
fn length(s: &str) -> Option<f32> {
    let s = s.trim();
    let split = s.find(|c: char| c.is_ascii_alphabetic() || c == '%').unwrap_or(s.len());
    let v: f32 = s[..split].parse().ok()?;
    Some(match &s[split..] {
        "cm" => v * 72.0 / 2.54,
        "mm" => v * 72.0 / 25.4,
        "in" => v * 72.0,
        "pc" => v * 12.0,
        "px" => v * 0.75,
        _ => v,
    })
}

fn parse_color(s: &str) -> Option<[u8; 3]> {
    let h = s.strip_prefix('#')?;
    if h.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(h, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

/// Estilos lidos: propriedades de parágrafo e de texto e o estilo pai.
#[derive(Default, Clone)]
struct OdfStyle {
    para: Option<Element>,
    text: Option<Element>,
    parent: Option<String>,
    /// O estilo inteiro (colunas e células de tabela).
    whole: Option<Element>,
}

struct Reader<'a> {
    files: &'a HashMap<String, Vec<u8>>,
    notes: std::cell::RefCell<Vec<(u32, String)>>,
    styles: HashMap<String, OdfStyle>,
    /// Estilos de lista: nome → tipo por nível.
    lists: HashMap<String, Vec<ListKind>>,
    default_text: CharStyle,
    content_width: f32,
}

fn apply_text(e: &Element, s: &mut CharStyle) {
    if let Some(f) = e.attr("style:font-name").or(e.attr("fo:font-family")) {
        s.family = f.trim_matches('\'').to_string();
    }
    if let Some(sz) = e.attr("fo:font-size") {
        if let Some(p) = sz.strip_suffix('%') {
            if let Ok(v) = p.parse::<f32>() {
                s.size *= v / 100.0;
            }
        } else if let Some(v) = length(sz) {
            s.size = v;
        }
    }
    if let Some(w) = e.attr("fo:font-weight") {
        s.bold = w == "bold" || w.parse::<u32>().is_ok_and(|n| n >= 600);
    }
    if let Some(st) = e.attr("fo:font-style") {
        s.italic = st == "italic" || st == "oblique";
    }
    if let Some(u) = e.attr("style:text-underline-style") {
        s.underline = u != "none";
    }
    if let Some(t) = e.attr("style:text-line-through-style") {
        s.strike = t != "none";
    }
    if let Some(c) = e.attr("fo:color").and_then(parse_color) {
        s.color = c;
    }
    if let Some(bg) = e.attr("fo:background-color") {
        s.highlight = parse_color(bg);
    }
    if let Some(pos) = e.attr("style:text-position") {
        s.baseline = if pos.starts_with("super") || pos.starts_with('3') {
            Baseline::Super
        } else if pos.starts_with("sub") || pos.starts_with('-') {
            Baseline::Sub
        } else {
            Baseline::Normal
        };
    }
}

fn apply_para(e: &Element, p: &mut ParaStyle) {
    if let Some(a) = e.attr("fo:text-align") {
        p.align = match a {
            "center" => Align::Center,
            "end" | "right" => Align::Right,
            "justify" => Align::Justify,
            _ => Align::Left,
        };
    }
    for (attr, field) in [("fo:margin-left", 0), ("fo:margin-right", 1), ("fo:text-indent", 2), ("fo:margin-top", 3), ("fo:margin-bottom", 4)] {
        if let Some(v) = e.attr(attr).and_then(length) {
            match field {
                0 => p.left_indent = v,
                1 => p.right_indent = v,
                2 => p.first_indent = v,
                3 => p.space_before = v,
                _ => p.space_after = v,
            }
        }
    }
    if let Some(lh) = e.attr("fo:line-height")
        && let Some(pct) = lh.strip_suffix('%').and_then(|v| v.parse::<f32>().ok()) {
            p.line_spacing = (pct / 100.0).clamp(0.5, 4.0);
        }
    if e.attr("fo:break-before") == Some("page") {
        p.page_break_before = true;
    }
    if let Some(tabs) = e.child("style:tab-stops") {
        p.leader = tabs.elements().any(|t| t.attr("style:type") == Some("right") && t.attr("style:leader-text").is_some_and(|l| !l.trim().is_empty()));
        p.tab_stops = tabs.elements().filter(|t| t.attr("style:type").is_none_or(|v| v == "left")).filter_map(|t| t.attr("style:position").and_then(length)).filter(|v| v.is_finite() && *v > 0.0).collect();
        p.tab_stops.sort_by(f32::total_cmp);
        p.tab_stops.dedup_by(|a, b| (*a - *b).abs() < 0.01);
    }
}

impl Reader<'_> {
    /// Estilo de texto e de parágrafo de um nome (com os pais).
    fn resolve(&self, name: &str, ch: &mut CharStyle, para: &mut ParaStyle) {
        let mut chain = Vec::new();
        let mut cur = Some(name.to_string());
        while let Some(n) = cur {
            if chain.len() > 10 || chain.contains(&n) {
                break;
            }
            cur = self.styles.get(&n).and_then(|s| s.parent.clone());
            chain.push(n);
        }
        for n in chain.iter().rev() {
            if let Some(name) = mide_style(n) {
                para.name = name.to_string();
            }
            if let Some(s) = self.styles.get(n) {
                if let Some(t) = &s.text {
                    apply_text(t, ch);
                }
                if let Some(p) = &s.para {
                    apply_para(p, para);
                }
            }
        }
    }

    fn block(&self, e: &Element, out: &mut Vec<Paragraph>, list: Option<(&str, u8)>) {
        for c in e.elements() {
            match c.name.as_str() {
                "text:p" | "text:h" => self.paragraph(c, out, list),
                "text:list" => {
                    let style = c.attr("text:style-name").or(list.map(|l| l.0)).unwrap_or("");
                    let level = list.map_or(0, |l| l.1 + 1);
                    for item in c.elements().filter(|i| i.name == "text:list-item" || i.name == "text:list-header") {
                        self.block(item, out, Some((style, level)));
                    }
                }
                "text:section" | "text:list-item" | "text:table-of-content" | "text:index-body" => self.block(c, out, list),
                "table:table" => {
                    if let Some(p) = self.table(c) {
                        out.push(p);
                        out.push(Paragraph::new("", self.default_text.clone(), ParaStyle::default()));
                    }
                }
                _ => {}
            }
        }
    }

    fn paragraph(&self, e: &Element, out: &mut Vec<Paragraph>, list: Option<(&str, u8)>) {
        let mut base = self.default_text.clone();
        let mut pstyle = ParaStyle::default();
        if let Some(n) = e.attr("text:style-name") {
            self.resolve(n, &mut base, &mut pstyle);
        }
        if e.name == "text:h" && pstyle.name.is_empty() {
            let level = e.attr("text:outline-level").and_then(|l| l.parse::<u8>().ok()).unwrap_or(1).clamp(1, 3);
            pstyle.name = format!("Título {level}");
        }
        if let Some((style, level)) = list {
            let kinds = self.lists.get(style);
            pstyle.list = Some(kinds.and_then(|k| k.get(level as usize).copied()).unwrap_or(ListKind::Bullet));
            pstyle.level = level;
            pstyle.left_indent = (pstyle.left_indent - 18.0 * (level as f32 + 1.0)).max(0.0);
            pstyle.first_indent = pstyle.first_indent.max(0.0);
        }
        let mut para = Paragraph { text: String::new(), spans: Vec::new(), style: pstyle.clone(), image: None, table: None };
        self.inline(e, &base, &mut para, out, &pstyle);
        if para.spans.is_empty() {
            para.spans.push(Span { len: 0, style: base });
        }
        para.normalize();
        out.push(para);
    }

    fn inline(&self, e: &Element, style: &CharStyle, para: &mut Paragraph, out: &mut Vec<Paragraph>, pstyle: &ParaStyle) {
        for n in &e.children {
            match n {
                Node::Text(t) => {
                    // Espaços e quebras do XML viram um espaço (como no ODF).
                    let collapsed: String = t.split_whitespace().collect::<Vec<_>>().join(" ");
                    let mut s = collapsed;
                    if t.starts_with(char::is_whitespace) && !para.text.is_empty() && !para.text.ends_with(' ') {
                        s.insert(0, ' ');
                    }
                    if t.ends_with(char::is_whitespace) && !s.is_empty() {
                        s.push(' ');
                    }
                    push(para, &s, style);
                }
                Node::Element(c) => match c.name.as_str() {
                    "text:span" | "text:a" => {
                        let mut s = style.clone();
                        let mut dummy = ParaStyle::default();
                        if let Some(n) = c.attr("text:style-name") {
                            self.resolve(n, &mut s, &mut dummy);
                        }
                        if c.name == "text:a" {
                            s.link = c.attr("xlink:href").map(str::to_string);
                        }
                        self.inline(c, &s, para, out, pstyle);
                    }
                    "text:s" => {
                        let n = c.attr("text:c").and_then(|v| v.parse::<usize>().ok()).unwrap_or(1);
                        push(para, &" ".repeat(n), style);
                    }
                    "text:tab" => push(para, "\t", style),
                    "text:note" => {
                        let text = c.child("text:note-body").map(|b| b.text()).unwrap_or_default();
                        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
                        let mut notes = self.notes.borrow_mut();
                        let id = notes.len() as u32 + 1;
                        notes.push((id, text));
                        let s = CharStyle { note: Some(id), baseline: Baseline::Normal, ..style.clone() };
                        push(para, &NOTE.to_string(), &s);
                    }
                    "text:line-break" => push(para, &LINE_BREAK.to_string(), style),
                    "draw:frame" => {
                        if let Some(img) = self.image(c) {
                            if !para.text.is_empty() {
                                let done = std::mem::replace(para, Paragraph { text: String::new(), spans: Vec::new(), style: pstyle.clone(), image: None, table: None });
                                let mut done = done;
                                done.normalize();
                                out.push(done);
                            }
                            let mut ps = pstyle.clone();
                            if ps.align == Align::Left {
                                ps.align = Align::Center;
                            }
                            out.push(Paragraph::image(img, style.clone(), ps));
                        }
                    }
                    "text:soft-page-break" | "text:bookmark" | "text:bookmark-start" | "text:bookmark-end" | "office:annotation" => {}
                    _ => self.inline(c, style, para, out, pstyle),
                },
            }
        }
    }

    /// Propriedades de um estilo de coluna ou de célula.
    fn props(&self, style: Option<&str>, child: &str) -> Option<Element> {
        self.styles.get(style?)?.whole.as_ref()?.child(child).cloned()
    }

    /// Uma tabela do ODF: colunas (com as repetidas), linhas (também as de cabeçalho),
    /// células (as cobertas por uma mesclagem ficam vazias), fundo e bordas.
    fn table(&self, e: &Element) -> Option<Paragraph> {
        let mut cols = Vec::new();
        let mut col_elems = Vec::new();
        for c in e.elements() {
            match c.name.as_str() {
                "table:table-column" => col_elems.push(c),
                "table:table-columns" | "table:table-header-columns" => col_elems.extend(c.elements().filter(|x| x.name == "table:table-column")),
                _ => {}
            }
        }
        for c in col_elems {
            let n = c.attr("table:number-columns-repeated").and_then(|v| v.parse::<usize>().ok()).unwrap_or(1).min(64);
            let w = self.props(c.attr("table:style-name"), "style:table-column-properties").and_then(|p| p.attr("style:column-width").and_then(length)).unwrap_or(0.0);
            cols.extend(std::iter::repeat_n(w, n));
        }
        let mut rows_e = Vec::new();
        for c in e.elements() {
            match c.name.as_str() {
                "table:table-row" => rows_e.push(c),
                "table:table-header-rows" | "table:table-rows" => rows_e.extend(c.elements().filter(|x| x.name == "table:table-row")),
                _ => {}
            }
        }
        let mut borders = false;
        let mut rows = Vec::new();
        for r in rows_e {
            let mut row = Vec::new();
            for cell in r.elements() {
                match cell.name.as_str() {
                    "table:table-cell" => {
                        let mut paras = Vec::new();
                        self.block(cell, &mut paras, None);
                        let mut c = super::Cell::from_paras(paras);
                        if let Some(props) = self.props(cell.attr("table:style-name"), "style:table-cell-properties") {
                            c.shade = props.attr("fo:background-color").and_then(parse_color);
                            borders |= ["fo:border", "fo:border-top", "fo:border-left"].iter().any(|k| props.attr(k).is_some_and(|b| b != "none" && !b.is_empty()));
                        }
                        let n = cell.attr("table:number-columns-repeated").and_then(|v| v.parse::<usize>().ok()).unwrap_or(1).min(64);
                        for _ in 1..n {
                            row.push(c.clone());
                        }
                        row.push(c);
                    }
                    "table:covered-table-cell" => row.push(super::Cell::empty(&self.default_text)),
                    _ => {}
                }
            }
            if !row.is_empty() {
                rows.push(row);
            }
        }
        if rows.is_empty() {
            return None;
        }
        let n = rows.iter().map(Vec::len).max().unwrap_or(1);
        for row in &mut rows {
            while row.len() < n {
                row.push(super::Cell::empty(&self.default_text));
            }
        }
        if cols.len() != n || cols.iter().any(|w| *w <= 0.0) {
            cols = vec![self.content_width / n as f32; n];
        }
        Some(Paragraph::table(super::Table { cols, rows, borders }, self.default_text.clone(), ParaStyle::default()))
    }

    fn image(&self, frame: &Element) -> Option<Image> {
        let href = frame.child("draw:image")?.attr("xlink:href")?;
        let data = self.files.get(href.trim_start_matches("./"))?.clone();
        let mut img = Image::from_bytes(data, self.content_width)?;
        if let (Some(w), Some(h)) = (frame.attr("svg:width").and_then(length), frame.attr("svg:height").and_then(length)) {
            img.width = w;
            img.height = h;
        }
        Some(img)
    }
}

fn push(para: &mut Paragraph, s: &str, style: &CharStyle) {
    if s.is_empty() {
        return;
    }
    para.text.push_str(s);
    match para.spans.last_mut() {
        Some(last) if last.style == *style => last.len += s.len(),
        _ => para.spans.push(Span { len: s.len(), style: style.clone() }),
    }
}

fn collect_styles(root: &Element, styles: &mut HashMap<String, OdfStyle>, lists: &mut HashMap<String, Vec<ListKind>>) {
    for group in root.elements().filter(|e| e.name == "office:styles" || e.name == "office:automatic-styles") {
        for s in group.elements() {
            match s.name.as_str() {
                "style:style" => {
                    let name = s.attr("style:name").unwrap_or("").to_string();
                    styles.insert(
                        name,
                        OdfStyle {
                            para: s.child("style:paragraph-properties").cloned(),
                            text: s.child("style:text-properties").cloned(),
                            parent: s.attr("style:parent-style-name").map(str::to_string),
                            whole: matches!(s.attr("style:family"), Some("table-column" | "table-cell")).then(|| s.clone()),
                        },
                    );
                }
                "text:list-style" => {
                    let name = s.attr("style:name").unwrap_or("").to_string();
                    let mut kinds = vec![ListKind::Bullet; 10];
                    for lvl in s.elements() {
                        let level: usize = lvl.attr("text:level").and_then(|l| l.parse().ok()).unwrap_or(1);
                        let kind = match lvl.name.as_str() {
                            "text:list-level-style-number" => match lvl.attr("style:num-format") {
                                Some("a" | "A") => ListKind::Letter,
                                Some("") | None => ListKind::Bullet,
                                _ => ListKind::Number,
                            },
                            _ if lvl.attr("text:bullet-char").is_some_and(|c| c.contains('–') || c.contains('-')) => ListKind::Dash,
                            _ => ListKind::Bullet,
                        };
                        if (1..=10).contains(&level) {
                            kinds[level - 1] = kind;
                        }
                    }
                    lists.insert(name, kinds);
                }
                _ => {}
            }
        }
    }
}

fn hf_text(e: &Element) -> (String, Option<String>) {
    let mut text = String::new();
    let mut number = None;
    for p in e.elements() {
        let mut pages = Vec::new();
        p.find_all("text:page-number", &mut pages);
        if !pages.is_empty() {
            number = Some(p.attr("text:style-name").unwrap_or("").to_string());
            continue;
        }
        let t = p.text();
        if !t.trim().is_empty() {
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(t.trim());
        }
    }
    (text, number)
}

/// Lê um ODT. `None` se não for um.
pub fn read(data: &[u8]) -> Option<Doc> {
    let files = unzip(data)?;
    let content = xml::parse(&String::from_utf8_lossy(files.get("content.xml")?))?;
    let styles_root = files.get("styles.xml").and_then(|s| xml::parse(&String::from_utf8_lossy(s)));
    let mut styles = HashMap::new();
    let mut lists = HashMap::new();
    let mut default_text = CharStyle { family: "Liberation Serif".into(), ..Default::default() };
    let mut page = PageSetup::a4();
    if let Some(sr) = &styles_root {
        collect_styles(sr, &mut styles, &mut lists);
        if let Some(def) = sr.child("office:styles").and_then(|s| s.elements().find(|e| e.name == "style:default-style" && e.attr("style:family") == Some("paragraph")))
            && let Some(t) = def.child("style:text-properties") {
                apply_text(t, &mut default_text);
            }
        // Página do estilo de página "Standard" (ou o primeiro).
        let auto = sr.child("office:automatic-styles");
        let master = sr.path("office:master-styles").and_then(|m| m.elements().find(|e| e.name == "style:master-page"));
        let layout_name = master.and_then(|m| m.attr("style:page-layout-name"));
        if let Some(layout) = auto.and_then(|a| a.elements().find(|e| e.name == "style:page-layout" && (layout_name.is_none() || e.attr("style:name") == layout_name)))
            && let Some(p) = layout.child("style:page-layout-properties") {
                if let (Some(w), Some(h)) = (p.attr("fo:page-width").and_then(length), p.attr("fo:page-height").and_then(length)) {
                    page.width = w;
                    page.height = h;
                }
                for (i, k) in ["fo:margin-top", "fo:margin-right", "fo:margin-bottom", "fo:margin-left"].iter().enumerate() {
                    if let Some(v) = p.attr(k).and_then(length) {
                        page.margins[i] = v;
                    }
                }
            }
        // Com cabeçalho/rodapé, o texto começa depois deles: soma a altura à margem.
        if let Some(layout) = auto.and_then(|a| a.elements().find(|e| e.name == "style:page-layout" && (layout_name.is_none() || e.attr("style:name") == layout_name))) {
            for (part, style, index, gap_attr) in [("style:header", "style:header-style", 0, "fo:margin-bottom"), ("style:footer", "style:footer-style", 2, "fo:margin-top")] {
                if master.and_then(|m| m.child(part)).is_some()
                    && let Some(props) = layout.child(style).and_then(|s| s.child("style:header-footer-properties")) {
                        let h = props.attr("fo:min-height").or(props.attr("svg:height")).and_then(length).unwrap_or(0.0);
                        // Com "dynamic-spacing" (o modo do Word), a altura já inclui o espaço.
                        let gap = if props.attr("style:dynamic-spacing") == Some("true") { 0.0 } else { props.attr(gap_attr).and_then(length).unwrap_or(0.0) };
                        page.margins[index] += h.max(12.0) + gap;
                    }
            }
        }
        if let Some(m) = master {
            for (part, header) in [("style:header", true), ("style:footer", false)] {
                if let Some(e) = m.child(part) {
                    let (text, number) = hf_text(e);
                    if header {
                        page.header = text;
                    } else {
                        page.footer = text;
                    }
                    if let Some(style) = number {
                        page.numbers.show = true;
                        let right = styles.get(&style).and_then(|s| s.para.as_ref()).and_then(|p| p.attr("fo:text-align")).is_some_and(|a| a == "end" || a == "right");
                        page.numbers.position = match (header, right) {
                            (true, _) => NumberPosition::TopRight,
                            (false, true) => NumberPosition::BottomRight,
                            (false, false) => NumberPosition::BottomCenter,
                        };
                    }
                }
            }
        }
    }
    collect_styles(&content, &mut styles, &mut lists);
    if let Some(meta) = files.get("meta.xml").and_then(|m| xml::parse(&String::from_utf8_lossy(m))) {
        let mut defs = Vec::new();
        meta.find_all("meta:user-defined", &mut defs);
        if let Some(n) = defs.iter().find(|d| d.attr("meta:name") == Some("mideShowFrom")).and_then(|d| d.text().trim().parse().ok()) {
            page.numbers.show_from = n;
        }
    }
    let reader = Reader { files: &files, notes: std::cell::RefCell::new(Vec::new()), styles, lists, default_text: default_text.clone(), content_width: page.content_width() };
    let text = content.path("office:body/office:text")?;
    let mut paras = Vec::new();
    reader.block(text, &mut paras, None);
    if paras.is_empty() {
        paras.push(Paragraph::new("", default_text, ParaStyle::default()));
    }
    let notes = reader.notes.take();
    Some(Doc { paras, page, notes })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::richtext::{cm, Pos};

    #[test]
    fn grava_e_le_odt() {
        let mut d = Doc::new();
        d.insert_text(Pos::default(), "Capítulo\nTexto  com  espaços duplos.\nprimeiro\nsegundo", &CharStyle { family: "Arial".into(), ..Default::default() });
        d.paras[0].style.name = "Título 1".into();
        d.apply_char(Pos::new(0, 0), Pos::new(0, "Capítulo".len()), |s| s.bold = true);
        d.apply_char(Pos::new(1, 0), Pos::new(1, 5), |s| {
            s.underline = true;
            s.color = [10, 120, 30];
        });
        d.apply_para(Pos::new(1, 0), Pos::new(1, 0), |p| {
            p.align = Align::Center;
            p.line_spacing = 2.0;
            p.page_break_before = true;
            p.tab_stops = vec![cm(2.0), cm(4.0)];
        });
        d.apply_para(Pos::new(2, 0), Pos::new(3, 0), |p| p.list = Some(ListKind::Letter));
        d.page.margins = [cm(3.0), cm(2.0), cm(2.0), cm(3.0)];
        d.page.numbers.show = true;
        d.page.numbers.position = NumberPosition::BottomRight;
        d.page.header = "Universidade do mide".into();
        d.apply_char(Pos::new(3, 0), Pos::new(3, 7), |s| s.link = Some("https://libreoffice.org".into()));
        d.insert_note(Pos::new(0, d.paras[0].text.len()), "Nota do LibreOffice.");
        let after = d.insert_table(d.end(), 2, 2);
        if let Some(t) = d.paras[after.para - 1].table.as_mut() {
            t.rows[1][0].doc.insert_text(Pos::default(), "A2", &CharStyle { italic: true, ..CharStyle::default() });
            t.rows[0][1].shade = Some([255, 230, 200]);
        }
        let bytes = write(&d);
        assert_eq!(&bytes[30..38], b"mimetype");
        let back = read(&bytes).expect("odt");
        assert!(back.paras.len() >= 5, "{:?}", back.paras.iter().map(|p| &p.text).collect::<Vec<_>>());
        let table = back.paras.iter().find_map(|p| p.table.as_ref()).expect("tabela");
        assert_eq!((table.rows.len(), table.cols.len()), (2, 2));
        assert_eq!(table.rows[1][0].doc.paras[0].text, "A2");
        assert!(table.rows[1][0].doc.paras[0].spans[0].style.italic);
        assert_eq!(table.rows[0][1].shade, Some([255, 230, 200]));
        assert!(table.borders);
        assert_eq!(back.paras[0].style.name, "Título 1");
        assert!(back.paras[0].spans[0].style.bold);
        assert_eq!(back.paras[1].text, "Texto  com  espaços duplos.");
        assert!(back.paras[1].spans[0].style.underline);
        assert_eq!(back.paras[1].spans[0].style.color, [10, 120, 30]);
        assert_eq!(back.paras[1].spans[0].style.family, "Arial");
        assert_eq!(back.paras[1].style.align, Align::Center);
        assert_eq!(back.paras[1].style.tab_stops.len(), 2);
        assert!((back.paras[1].style.tab_stops[0] - cm(2.0)).abs() < 0.1);
        assert!((back.paras[1].style.tab_stops[1] - cm(4.0)).abs() < 0.1);
        assert!((back.paras[1].style.line_spacing - 2.0).abs() < 0.01);
        assert!(back.paras[1].style.page_break_before);
        assert_eq!(back.paras[2].style.list, Some(ListKind::Letter));
        assert!((back.page.margins[3] - cm(3.0)).abs() < 0.2);
        assert_eq!(back.page.header, "Universidade do mide");
        assert_eq!(back.page.numbers.position, NumberPosition::BottomRight);
        assert_eq!(back.notes, vec![(1, "Nota do LibreOffice.".to_string())]);
        assert!(back.paras.iter().any(|p| p.spans.iter().any(|s| s.style.link.as_deref() == Some("https://libreoffice.org"))), "link");
        assert!(back.paras[0].text.ends_with(crate::richtext::NOTE));
        if let Ok(dir) = std::env::var("MIDE_DOCX_OUT") {
            let _ = std::fs::write(format!("{dir}/teste.odt"), &bytes);
        }
    }
}
