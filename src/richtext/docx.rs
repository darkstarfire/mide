//! DOCX (Word, Office Open XML): ler e gravar, escrito aqui.
//!
//! Grava parágrafos com estilo (Título, Título 1–3, Citação, Legenda, Referência viram
//! os estilos do Word: Title, Heading1..3, Quote, Caption, Bibliography), trechos com
//! fonte, tamanho, negrito, itálico, sublinhado, tachado, cor, realce, sobrescrito e
//! subscrito, alinhamento, recuos, entrelinha, espaços, quebra de página, listas
//! (numbering.xml), imagens, papel, margens, cabeçalho, rodapé e número de página.
//! Lê o mesmo dos arquivos do Word, do LibreOffice e do Google Docs; tabelas viram
//! parágrafos (até o editor ter tabelas).

use std::{
    collections::HashMap,
    io::{Cursor, Read, Write},
};

use super::{
    xml::{self, escape, Element, Node},
    Align, Baseline, CharStyle, Doc, Image, ListKind, NumberPosition, PageSetup, ParaStyle, Paragraph, Span, LINE_BREAK, NOTE, OBJECT,
};

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

fn twips(pt: f32) -> i64 {
    (pt * 20.0).round() as i64
}

fn emu(pt: f32) -> i64 {
    (pt * 12700.0).round() as i64
}

fn hex(c: [u8; 3]) -> String {
    format!("{:02X}{:02X}{:02X}", c[0], c[1], c[2])
}

/// Nome do estilo do mide → estilo do Word.
fn word_style(name: &str) -> Option<&'static str> {
    Some(match name {
        "Título" => "Title",
        "Título 1" => "Heading1",
        "Título 2" => "Heading2",
        "Título 3" => "Heading3",
        "Citação" => "Quote",
        "Legenda" => "Caption",
        "Referência" => "Bibliography",
        _ => return None,
    })
}

fn mide_style(id: &str) -> Option<&'static str> {
    Some(match id.to_lowercase().replace(' ', "").as_str() {
        "title" | "titulo" => "Título",
        "heading1" | "título1" | "titulo1" => "Título 1",
        "heading2" | "título2" | "titulo2" => "Título 2",
        "heading3" | "título3" | "titulo3" => "Título 3",
        "quote" | "intensequote" | "citação" | "citacao" => "Citação",
        "caption" | "legenda" => "Legenda",
        "bibliography" | "referência" | "referencia" => "Referência",
        _ => return None,
    })
}

fn run_props(s: &CharStyle) -> String {
    let mut p = String::from("<w:rPr>");
    let f = escape(&s.family);
    p.push_str(&format!("<w:rFonts w:ascii=\"{f}\" w:hAnsi=\"{f}\" w:cs=\"{f}\" w:eastAsia=\"{f}\"/>"));
    if s.bold {
        p.push_str("<w:b/><w:bCs/>");
    }
    if s.italic {
        p.push_str("<w:i/><w:iCs/>");
    }
    if s.strike {
        p.push_str("<w:strike/>");
    }
    if s.color != [0, 0, 0] {
        p.push_str(&format!("<w:color w:val=\"{}\"/>", hex(s.color)));
    }
    let half = (s.size * 2.0).round() as i64;
    p.push_str(&format!("<w:sz w:val=\"{half}\"/><w:szCs w:val=\"{half}\"/>"));
    if let Some(h) = s.highlight {
        p.push_str(&format!("<w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"{}\"/>", hex(h)));
    }
    if s.underline {
        p.push_str("<w:u w:val=\"single\"/>");
    }
    match s.baseline {
        Baseline::Super => p.push_str("<w:vertAlign w:val=\"superscript\"/>"),
        Baseline::Sub => p.push_str("<w:vertAlign w:val=\"subscript\"/>"),
        Baseline::Normal => {}
    }
    p.push_str("</w:rPr>");
    p
}

fn run_text(text: &str) -> String {
    let mut out = String::new();
    let mut buf = String::new();
    let flush = |buf: &mut String, out: &mut String| {
        if !buf.is_empty() {
            out.push_str(&format!("<w:t xml:space=\"preserve\">{}</w:t>", escape(buf)));
            buf.clear();
        }
    };
    for c in text.chars() {
        match c {
            '\t' => {
                flush(&mut buf, &mut out);
                out.push_str("<w:tab/>");
            }
            LINE_BREAK => {
                flush(&mut buf, &mut out);
                out.push_str("<w:br/>");
            }
            OBJECT | NOTE => {}
            c => buf.push(c),
        }
    }
    flush(&mut buf, &mut out);
    out
}

fn list_num_id(kind: ListKind) -> u32 {
    match kind {
        ListKind::Bullet => 1,
        ListKind::Dash => 2,
        ListKind::Number => 3,
        ListKind::Letter => 4,
    }
}

fn numbering_xml() -> String {
    let mut s = format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:numbering xmlns:w=\"{W}\">");
    for (id, kind) in [(1, ListKind::Bullet), (2, ListKind::Dash), (3, ListKind::Number), (4, ListKind::Letter)] {
        s.push_str(&format!("<w:abstractNum w:abstractNumId=\"{id}\"><w:multiLevelType w:val=\"hybridMultilevel\"/>"));
        for lvl in 0..9 {
            let (fmt, text) = match kind {
                ListKind::Bullet => ("bullet", ["•", "◦", "▪"][lvl % 3].to_string()),
                ListKind::Dash => ("bullet", "–".to_string()),
                ListKind::Number => ("decimal", format!("%{}.", lvl + 1)),
                ListKind::Letter => ("lowerLetter", format!("%{})", lvl + 1)),
            };
            let left = 360 * (lvl + 1) * 2 / 2 * 2;
            s.push_str(&format!(
                "<w:lvl w:ilvl=\"{lvl}\"><w:start w:val=\"1\"/><w:numFmt w:val=\"{fmt}\"/><w:lvlText w:val=\"{}\"/><w:lvlJc w:val=\"left\"/><w:pPr><w:ind w:left=\"{left}\" w:hanging=\"360\"/></w:pPr></w:lvl>",
                escape(&text)
            ));
        }
        s.push_str("</w:abstractNum>");
    }
    for id in 1..=4 {
        s.push_str(&format!("<w:num w:numId=\"{id}\"><w:abstractNumId w:val=\"{id}\"/></w:num>"));
    }
    s.push_str("</w:numbering>");
    s
}

fn styles_xml(base: &CharStyle) -> String {
    let f = escape(&base.family);
    let half = (base.size * 2.0).round() as i64;
    let mut s = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:styles xmlns:w=\"{W}\"><w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii=\"{f}\" w:hAnsi=\"{f}\" w:cs=\"{f}\" w:eastAsia=\"{f}\"/><w:sz w:val=\"{half}\"/><w:szCs w:val=\"{half}\"/><w:lang w:val=\"pt-BR\"/></w:rPr></w:rPrDefault><w:pPrDefault><w:pPr><w:spacing w:after=\"0\" w:line=\"240\" w:lineRule=\"auto\"/></w:pPr></w:pPrDefault></w:docDefaults>"
    );
    s.push_str("<w:style w:type=\"paragraph\" w:default=\"1\" w:styleId=\"Normal\"><w:name w:val=\"Normal\"/><w:qFormat/></w:style>");
    for (id, name, level) in [("Title", "Title", None), ("Heading1", "heading 1", Some(0)), ("Heading2", "heading 2", Some(1)), ("Heading3", "heading 3", Some(2)), ("Quote", "Quote", None), ("Caption", "caption", None), ("Bibliography", "Bibliography", None)] {
        s.push_str(&format!("<w:style w:type=\"paragraph\" w:styleId=\"{id}\"><w:name w:val=\"{name}\"/><w:basedOn w:val=\"Normal\"/><w:next w:val=\"Normal\"/><w:qFormat/>"));
        if let Some(l) = level {
            s.push_str(&format!("<w:pPr><w:keepNext/><w:outlineLvl w:val=\"{l}\"/></w:pPr>"));
        }
        s.push_str("</w:style>");
    }
    s.push_str("</w:styles>");
    s
}

/// O que os parágrafos juntam ao serem gravados: imagens, relações e notas.
struct WriteCtx {
    media: Vec<(String, Vec<u8>)>,
    rels: Vec<(String, String, String)>,
    notes: Vec<u32>,
}

/// Grava parágrafos (do documento ou de uma célula) em WordprocessingML.
fn write_paras(paras: &[Paragraph], content_width: f32, ctx: &mut WriteCtx) -> String {
    let mut body = String::new();
    let (media, rels, notes) = (&mut ctx.media, &mut ctx.rels, &mut ctx.notes);
    for p in paras {
        let st = &p.style;
        if let Some(table) = &p.table {
            let mut sub = WriteCtx { media: std::mem::take(media), rels: std::mem::take(rels), notes: std::mem::take(notes) };
            body.push_str(&write_table(table, &mut sub));
            (*media, *rels, *notes) = (sub.media, sub.rels, sub.notes);
            continue;
        }
        body.push_str("<w:p><w:pPr>");
        if let Some(id) = word_style(&st.name) {
            body.push_str(&format!("<w:pStyle w:val=\"{id}\"/>"));
        }
        if st.page_break_before {
            body.push_str("<w:pageBreakBefore/>");
        }
        if st.leader {
            body.push_str(&format!("<w:tabs><w:tab w:val=\"right\" w:leader=\"dot\" w:pos=\"{}\"/></w:tabs>", twips(content_width - st.right_indent)));
        }
        if let Some(kind) = st.list {
            body.push_str(&format!("<w:numPr><w:ilvl w:val=\"{}\"/><w:numId w:val=\"{}\"/></w:numPr>", st.level, list_num_id(kind)));
        }
        body.push_str(&format!(
            "<w:spacing w:before=\"{}\" w:after=\"{}\" w:line=\"{}\" w:lineRule=\"auto\"/>",
            twips(st.space_before),
            twips(st.space_after),
            (240.0 * st.line_spacing).round() as i64
        ));
        let list_left = if st.list.is_some() { 18.0 * (st.level as f32 + 1.0) } else { 0.0 };
        let first = if st.first_indent >= 0.0 { format!("w:firstLine=\"{}\"", twips(st.first_indent)) } else { format!("w:hanging=\"{}\"", twips(-st.first_indent)) };
        body.push_str(&format!("<w:ind w:left=\"{}\" w:right=\"{}\" {first}/>", twips(st.left_indent + list_left), twips(st.right_indent)));
        body.push_str(&format!(
            "<w:jc w:val=\"{}\"/>",
            match st.align {
                Align::Left => "left",
                Align::Center => "center",
                Align::Right => "right",
                Align::Justify => "both",
            }
        ));
        body.push_str("</w:pPr>");
        if let Some(img) = &p.image {
            let n = media.len() + 1;
            let ext = if img.format == "png" { "png" } else { "jpeg" };
            let file = format!("image{n}.{ext}");
            media.push((file.clone(), img.data.as_ref().clone()));
            let rid = format!("rIdImg{n}");
            rels.push((rid.clone(), "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image".into(), format!("media/{file}")));
            let (cx, cy) = (emu(img.width), emu(img.height));
            body.push_str(&format!(
                "<w:r><w:drawing><wp:inline distT=\"0\" distB=\"0\" distL=\"0\" distR=\"0\"><wp:extent cx=\"{cx}\" cy=\"{cy}\"/><wp:docPr id=\"{n}\" name=\"Imagem {n}\"/><a:graphic><a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/picture\"><pic:pic><pic:nvPicPr><pic:cNvPr id=\"{n}\" name=\"{file}\"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed=\"{rid}\"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r>"
            ));
        } else {
            for (a, b, s) in p.runs() {
                if a == b {
                    continue;
                }
                if let Some(id) = s.note {
                    body.push_str(&format!("<w:r><w:rPr><w:rStyle w:val=\"FootnoteReference\"/><w:vertAlign w:val=\"superscript\"/></w:rPr><w:footnoteReference w:id=\"{id}\"/></w:r>"));
                    notes.push(id);
                    continue;
                }
                let run = format!("<w:r>{}{}</w:r>", run_props(s), run_text(&p.text[a..b]));
                match &s.link {
                    Some(url) => {
                        let rid = format!("rIdLink{}", rels.len() + 1);
                        rels.push((rid.clone(), "http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink".into(), escape(url)));
                        body.push_str(&format!("<w:hyperlink r:id=\"{rid}\">{run}</w:hyperlink>"));
                    }
                    None => body.push_str(&run),
                }
            }
        }
        body.push_str("</w:p>");
    }
    body
}

/// Grava uma tabela (larguras, bordas, fundo das células e o conteúdo delas).
fn write_table(table: &super::Table, ctx: &mut WriteCtx) -> String {
    let total: f32 = table.cols.iter().sum();
    let mut s = format!("<w:tbl><w:tblPr><w:tblW w:w=\"{}\" w:type=\"dxa\"/>", twips(total));
    if table.borders {
        s.push_str("<w:tblBorders>");
        for side in ["top", "left", "bottom", "right", "insideH", "insideV"] {
            s.push_str(&format!("<w:{side} w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"808084\"/>"));
        }
        s.push_str("</w:tblBorders>");
    } else {
        s.push_str("<w:tblBorders><w:top w:val=\"nil\"/><w:left w:val=\"nil\"/><w:bottom w:val=\"nil\"/><w:right w:val=\"nil\"/><w:insideH w:val=\"nil\"/><w:insideV w:val=\"nil\"/></w:tblBorders>");
    }
    s.push_str("<w:tblLayout w:type=\"fixed\"/><w:tblCellMar><w:left w:w=\"100\" w:type=\"dxa\"/><w:right w:w=\"100\" w:type=\"dxa\"/></w:tblCellMar></w:tblPr><w:tblGrid>");
    for c in &table.cols {
        s.push_str(&format!("<w:gridCol w:w=\"{}\"/>", twips(*c)));
    }
    s.push_str("</w:tblGrid>");
    for row in &table.rows {
        s.push_str("<w:tr>");
        for (c, cell) in row.iter().enumerate() {
            let w = table.cols.get(c).copied().unwrap_or(72.0);
            s.push_str(&format!("<w:tc><w:tcPr><w:tcW w:w=\"{}\" w:type=\"dxa\"/>", twips(w)));
            if let Some(sh) = cell.shade {
                s.push_str(&format!("<w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"{}\"/>", hex(sh)));
            }
            s.push_str("</w:tcPr>");
            s.push_str(&write_paras(&cell.doc.paras, w - 10.0, ctx));
            s.push_str("</w:tc>");
        }
        s.push_str("</w:tr>");
    }
    s.push_str("</w:tbl>");
    s
}

/// Grava o documento em DOCX.
pub fn write(doc: &Doc) -> Vec<u8> {
    let rels: Vec<(String, String, String)> = vec![
        ("rIdStyles".into(), "http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles".into(), "styles.xml".into()),
        ("rIdNumbering".into(), "http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering".into(), "numbering.xml".into()),
        ("rIdSettings".into(), "http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings".into(), "settings.xml".into()),
    ];
    let mut ctx = WriteCtx { media: Vec::new(), rels, notes: Vec::new() };
    let body = write_paras(&doc.paras, doc.page.content_width(), &mut ctx);
    let WriteCtx { media, mut rels, notes } = ctx;
    // Cabeçalho e rodapé (com o número da página).
    let page = &doc.page;
    // Cabeçalho, rodapé e notas: só a fonte do documento, letra 10.
    let family = doc.paras.iter().find(|p| p.style.name.is_empty() && !p.text.is_empty()).or(doc.paras.first()).map(|p| p.spans[0].style.family.clone()).unwrap_or_default();
    let small = CharStyle { size: 10.0, family, ..CharStyle::default() };
    let part = |text: &str, number: Option<&str>| -> String {
        let mut s = format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:{{}} xmlns:w=\"{W}\" xmlns:r=\"{R}\">");
        if !text.is_empty() {
            s.push_str(&format!("<w:p><w:pPr><w:jc w:val=\"center\"/></w:pPr><w:r>{}{}</w:r></w:p>", run_props(&small), run_text(text)));
        }
        if let Some(jc) = number {
            s.push_str(&format!("<w:p><w:pPr><w:jc w:val=\"{jc}\"/></w:pPr><w:fldSimple w:instr=\" PAGE \"><w:r>{}<w:t>1</w:t></w:r></w:fldSimple></w:p>", run_props(&small)));
        }
        if text.is_empty() && number.is_none() {
            s.push_str("<w:p/>");
        }
        s
    };
    let num_top = (page.numbers.show && page.numbers.position == NumberPosition::TopRight).then_some("right");
    let num_bottom = match (page.numbers.show, page.numbers.position) {
        (true, NumberPosition::BottomCenter) => Some("center"),
        (true, NumberPosition::BottomRight) => Some("right"),
        _ => None,
    };
    let mut parts: Vec<(String, String)> = Vec::new();
    let mut sect = String::from("<w:sectPr>");
    if !page.header.is_empty() || num_top.is_some() {
        let s = part(&page.header, num_top).replace("<w:{}", "<w:hdr") + "</w:hdr>";
        parts.push(("header1.xml".into(), s));
        rels.push(("rIdHeader".into(), "http://schemas.openxmlformats.org/officeDocument/2006/relationships/header".into(), "header1.xml".into()));
        sect.push_str("<w:headerReference w:type=\"default\" r:id=\"rIdHeader\"/>");
    }
    if !page.footer.is_empty() || num_bottom.is_some() {
        let s = part(&page.footer, num_bottom).replace("<w:{}", "<w:ftr") + "</w:ftr>";
        parts.push(("footer1.xml".into(), s));
        rels.push(("rIdFooter".into(), "http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer".into(), "footer1.xml".into()));
        sect.push_str("<w:footerReference w:type=\"default\" r:id=\"rIdFooter\"/>");
    }
    // Sem número na primeira página (a capa): página de título diferente, sem cabeçalho.
    let first_empty = page.numbers.show && page.numbers.show_from > 1;
    if first_empty {
        // A primeira página com o cabeçalho e o rodapé, só sem o número.
        let empty_h = part(&page.header, None).replace("<w:{}", "<w:hdr") + "</w:hdr>";
        let empty_f = part(&page.footer, None).replace("<w:{}", "<w:ftr") + "</w:ftr>";
        parts.push(("header2.xml".into(), empty_h));
        parts.push(("footer2.xml".into(), empty_f));
        rels.push(("rIdHeaderFirst".into(), "http://schemas.openxmlformats.org/officeDocument/2006/relationships/header".into(), "header2.xml".into()));
        rels.push(("rIdFooterFirst".into(), "http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer".into(), "footer2.xml".into()));
        sect.push_str("<w:headerReference w:type=\"first\" r:id=\"rIdHeaderFirst\"/><w:footerReference w:type=\"first\" r:id=\"rIdFooterFirst\"/>");
    }
    sect.push_str(&format!(
        "<w:pgSz w:w=\"{}\" w:h=\"{}\"{}/><w:pgMar w:top=\"{}\" w:right=\"{}\" w:bottom=\"{}\" w:left=\"{}\" w:header=\"708\" w:footer=\"708\" w:gutter=\"0\"/>",
        twips(page.width),
        twips(page.height),
        if page.landscape() { " w:orient=\"landscape\"" } else { "" },
        twips(page.margins[0]),
        twips(page.margins[1]),
        twips(page.margins[2]),
        twips(page.margins[3])
    ));
    if first_empty {
        sect.push_str("<w:titlePg/>");
    }
    sect.push_str("</w:sectPr>");
    let document = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:document xmlns:w=\"{W}\" xmlns:r=\"{R}\" xmlns:wp=\"http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing\" xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" xmlns:pic=\"http://schemas.openxmlformats.org/drawingml/2006/picture\"><w:body>{body}{sect}</w:body></w:document>"
    );
    // A página onde o número começa a aparecer (o Word só sabe "não na primeira").
    let settings = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:settings xmlns:w=\"{W}\"><w:defaultTabStop w:val=\"720\"/><w:docVars><w:docVar w:name=\"mideShowFrom\" w:val=\"{}\"/></w:docVars></w:settings>",
        page.numbers.show_from
    );
    // Notas de rodapé (as duas primeiras são os separadores que o Word espera).
    let mut footnotes = format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:footnotes xmlns:w=\"{W}\" xmlns:r=\"{R}\"><w:footnote w:type=\"separator\" w:id=\"-1\"><w:p><w:r><w:separator/></w:r></w:p></w:footnote><w:footnote w:type=\"continuationSeparator\" w:id=\"0\"><w:p><w:r><w:continuationSeparator/></w:r></w:p></w:footnote>");
    let note_style = CharStyle { size: 10.0, ..small.clone() };
    for id in &notes {
        footnotes.push_str(&format!(
            "<w:footnote w:id=\"{id}\"><w:p><w:pPr><w:jc w:val=\"both\"/></w:pPr><w:r><w:rPr><w:vertAlign w:val=\"superscript\"/></w:rPr><w:footnoteRef/></w:r><w:r>{}{}</w:r></w:p></w:footnote>",
            run_props(&note_style),
            run_text(&format!(" {}", doc.note_text(*id).unwrap_or("")))
        ));
    }
    footnotes.push_str("</w:footnotes>");
    rels.push(("rIdFootnotes".into(), "http://schemas.openxmlformats.org/officeDocument/2006/relationships/footnotes".into(), "footnotes.xml".into()));
    parts.push(("footnotes.xml".into(), footnotes));
    let doc_rels: String = rels
        .iter()
        .map(|(id, t, target)| {
            let external = if t.ends_with("/hyperlink") { " TargetMode=\"External\"" } else { "" };
            format!("<Relationship Id=\"{id}\" Type=\"{t}\" Target=\"{target}\"{external}/>")
        })
        .collect();
    let content_types = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Default Extension=\"png\" ContentType=\"image/png\"/><Default Extension=\"jpeg\" ContentType=\"image/jpeg\"/><Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/><Override PartName=\"/word/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml\"/><Override PartName=\"/word/numbering.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml\"/><Override PartName=\"/word/settings.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml\"/>{}</Types>",
        parts
            .iter()
            .map(|(name, _)| format!(
                "<Override PartName=\"/word/{name}\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.{}+xml\"/>",
                if name.starts_with("header") {
                    "header"
                } else if name.starts_with("footnotes") {
                    "footnotes"
                } else {
                    "footer"
                }
            ))
            .collect::<String>()
    );
    let base = doc.paras.iter().find(|p| p.style.name.is_empty() && !p.text.is_empty()).map(|p| p.spans[0].style.clone()).unwrap_or_default();
    let mut files: Vec<(String, Vec<u8>)> = vec![
        ("[Content_Types].xml".into(), content_types.into_bytes()),
        ("_rels/.rels".into(), "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/></Relationships>".to_string().into_bytes()),
        ("word/document.xml".into(), document.into_bytes()),
        ("word/_rels/document.xml.rels".into(), format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">{doc_rels}</Relationships>").into_bytes()),
        ("word/styles.xml".into(), styles_xml(&base).into_bytes()),
        ("word/numbering.xml".into(), numbering_xml().into_bytes()),
        ("word/settings.xml".into(), settings.into_bytes()),
    ];
    for (name, data) in parts {
        files.push((format!("word/{name}"), data.into_bytes()));
    }
    for (name, data) in media {
        files.push((format!("word/media/{name}"), data));
    }
    zip_files(&files, None)
}

/// Junta os arquivos num .zip (`stored`: o primeiro, sem compactar, como o ODT pede).
pub fn zip_files(files: &[(String, Vec<u8>)], stored: Option<&str>) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut out);
        for (name, data) in files {
            let method = if Some(name.as_str()) == stored { zip::CompressionMethod::Stored } else { zip::CompressionMethod::Deflated };
            let opts = zip::write::SimpleFileOptions::default().compression_method(method);
            if z.start_file(name.as_str(), opts).is_ok() {
                let _ = z.write_all(data);
            }
        }
        let _ = z.finish();
    }
    out.into_inner()
}

/// Lê os arquivos de um .zip.
pub fn unzip(data: &[u8]) -> Option<HashMap<String, Vec<u8>>> {
    let mut z = zip::ZipArchive::new(Cursor::new(data)).ok()?;
    let mut out = HashMap::new();
    for i in 0..z.len() {
        let mut f = z.by_index(i).ok()?;
        if f.is_dir() || f.size() > 200 * 1024 * 1024 {
            continue;
        }
        let mut buf = Vec::new();
        f.read_to_end(&mut buf).ok()?;
        out.insert(f.name().to_string(), buf);
    }
    Some(out)
}

fn val(e: &Element) -> Option<&str> {
    e.attr("w:val")
}

/// Liga/desliga do Word: presente sem valor é ligado; "0", "false", "none" desligam.
fn on(e: Option<&Element>) -> Option<bool> {
    e.map(|e| !matches!(val(e), Some("0" | "false" | "none" | "off")))
}

fn num(e: Option<&Element>, attr: &str) -> Option<f32> {
    e?.attr(attr)?.parse::<f32>().ok()
}

fn named_highlight(name: &str) -> Option<[u8; 3]> {
    Some(match name {
        "yellow" => [255, 255, 0],
        "green" => [0, 255, 0],
        "cyan" => [0, 255, 255],
        "magenta" => [255, 0, 255],
        "blue" => [0, 0, 255],
        "red" => [255, 0, 0],
        "darkBlue" => [0, 0, 139],
        "darkCyan" => [0, 139, 139],
        "darkGreen" => [0, 100, 0],
        "darkMagenta" => [139, 0, 139],
        "darkRed" => [139, 0, 0],
        "darkYellow" => [128, 128, 0],
        "darkGray" => [169, 169, 169],
        "lightGray" => [211, 211, 211],
        "black" => [0, 0, 0],
        _ => return None,
    })
}

fn parse_hex(s: &str) -> Option<[u8; 3]> {
    if s.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(s, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

/// Aplica as propriedades de texto do Word (`w:rPr`) ao estilo.
fn apply_rpr(rpr: &Element, s: &mut CharStyle, theme_font: &str) {
    if let Some(f) = rpr.child("w:rFonts") {
        if let Some(name) = f.attr("w:ascii").or(f.attr("w:hAnsi")).or(f.attr("w:cs")) {
            s.family = name.to_string();
        } else if f.attr("w:asciiTheme").is_some() {
            s.family = theme_font.to_string();
        }
    }
    if let Some(b) = on(rpr.child("w:b")) {
        s.bold = b;
    }
    if let Some(i) = on(rpr.child("w:i")) {
        s.italic = i;
    }
    if let Some(u) = rpr.child("w:u") {
        s.underline = !matches!(val(u), Some("none" | "0" | "false"));
    }
    if let Some(st) = on(rpr.child("w:strike")).or(on(rpr.child("w:dstrike"))) {
        s.strike = st;
    }
    if let Some(c) = rpr.child("w:color").and_then(val) {
        s.color = parse_hex(c).unwrap_or([0, 0, 0]);
    }
    if let Some(sz) = num(rpr.child("w:sz"), "w:val") {
        s.size = sz / 2.0;
    }
    if let Some(h) = rpr.child("w:highlight").and_then(val) {
        s.highlight = named_highlight(h);
    }
    if let Some(fill) = rpr.child("w:shd").and_then(|e| e.attr("w:fill")) {
        if fill != "auto" {
            s.highlight = parse_hex(fill).or(s.highlight);
        }
    }
    if let Some(v) = rpr.child("w:vertAlign").and_then(val) {
        s.baseline = match v {
            "superscript" => Baseline::Super,
            "subscript" => Baseline::Sub,
            _ => Baseline::Normal,
        };
    }
}

/// Aplica as propriedades de parágrafo do Word (`w:pPr`).
fn apply_ppr(ppr: &Element, p: &mut ParaStyle) {
    if let Some(jc) = ppr.child("w:jc").and_then(val) {
        p.align = match jc {
            "center" => Align::Center,
            "right" | "end" => Align::Right,
            "both" | "distribute" | "lowKashida" | "highKashida" | "mediumKashida" | "thaiDistribute" => Align::Justify,
            _ => Align::Left,
        };
    }
    if let Some(ind) = ppr.child("w:ind") {
        if let Some(v) = ind.attr("w:left").or(ind.attr("w:start")).and_then(|v| v.parse::<f32>().ok()) {
            p.left_indent = v / 20.0;
        }
        if let Some(v) = ind.attr("w:right").or(ind.attr("w:end")).and_then(|v| v.parse::<f32>().ok()) {
            p.right_indent = v / 20.0;
        }
        if let Some(v) = num(Some(ind), "w:firstLine") {
            p.first_indent = v / 20.0;
        }
        if let Some(v) = num(Some(ind), "w:hanging") {
            p.first_indent = -v / 20.0;
        }
    }
    if let Some(sp) = ppr.child("w:spacing") {
        if let Some(v) = num(Some(sp), "w:before") {
            p.space_before = v / 20.0;
        }
        if let Some(v) = num(Some(sp), "w:after") {
            p.space_after = v / 20.0;
        }
        if let Some(v) = num(Some(sp), "w:line") {
            p.line_spacing = match sp.attr("w:lineRule") {
                Some("exact" | "atLeast") => (v / 20.0 / 14.0).clamp(0.8, 3.0),
                _ => (v / 240.0).clamp(0.5, 4.0),
            };
        }
    }
    if let Some(b) = on(ppr.child("w:pageBreakBefore")) {
        p.page_break_before = b;
    }
    if let Some(tabs) = ppr.child("w:tabs") {
        p.leader = tabs.elements().any(|t| t.attr("w:leader").is_some_and(|l| l == "dot") && t.attr("w:val") == Some("right"));
    }
    // O sumário do Word (TOC 1, TOC 2...) com pontilhado.
    if let Some(id) = ppr.child("w:pStyle").and_then(val) {
        if id.to_lowercase().starts_with("toc") || id.to_lowercase().starts_with("sumário") || id.to_lowercase().starts_with("sumario") {
            p.leader = true;
            p.name = "Sumário".into();
        }
    }
    if let Some(id) = ppr.child("w:pStyle").and_then(val) {
        if let Some(name) = mide_style(id) {
            p.name = name.to_string();
        }
    }
}

struct Reader<'a> {
    files: &'a HashMap<String, Vec<u8>>,
    /// Texto das notas de rodapé do arquivo (id do Word → texto).
    footnotes: HashMap<String, String>,
    notes: std::cell::RefCell<Vec<(u32, String)>>,
    rels: HashMap<String, String>,
    /// Estilos de parágrafo do Word: id → (pPr, rPr, basedOn).
    styles: HashMap<String, (Option<Element>, Option<Element>, Option<String>)>,
    default_run: CharStyle,
    default_para: ParaStyle,
    theme_font: String,
    /// numId → (abstrato) e abstrato → formato por nível.
    nums: HashMap<String, String>,
    formats: HashMap<(String, u8), ListKind>,
    content_width: f32,
}

impl Reader<'_> {
    fn style_chain(&self, id: &str, out: &mut Vec<String>) {
        if out.len() > 10 || out.iter().any(|x| x == id) {
            return;
        }
        if let Some((_, _, based)) = self.styles.get(id) {
            if let Some(b) = based {
                self.style_chain(b, out);
            }
            out.push(id.to_string());
        }
    }

    fn list_kind(&self, num_id: &str, level: u8) -> Option<ListKind> {
        if num_id == "0" {
            return None;
        }
        let abs = self.nums.get(num_id)?;
        Some(*self.formats.get(&(abs.clone(), level)).unwrap_or(&ListKind::Bullet))
    }

    fn paragraphs(&self, parent: &Element, out: &mut Vec<Paragraph>) {
        for e in parent.elements() {
            match e.name.as_str() {
                "w:p" => self.paragraph(e, out),
                "w:tbl" => {
                    if let Some(p) = self.table(e) {
                        out.push(p);
                        // Depois de uma tabela, sempre um parágrafo (para o cursor sair dela).
                        if !matches!(e_next_is_para(parent, e), Some(true)) {
                            out.push(Paragraph::new("", self.default_run.clone(), ParaStyle::default()));
                        }
                    }
                }
                "w:sdt" => {
                    if let Some(content) = e.child("w:sdtContent") {
                        self.paragraphs(content, out);
                    }
                }
                _ => {}
            }
        }
    }

    fn paragraph(&self, e: &Element, out: &mut Vec<Paragraph>) {
        let mut pstyle = self.default_para.clone();
        let mut base_run = self.default_run.clone();
        let ppr = e.child("w:pPr");
        if let Some(id) = ppr.and_then(|p| p.child("w:pStyle")).and_then(val) {
            let mut chain = Vec::new();
            self.style_chain(id, &mut chain);
            for sid in chain {
                if let Some((sp, sr, _)) = self.styles.get(&sid) {
                    if let Some(sp) = sp {
                        apply_ppr(sp, &mut pstyle);
                    }
                    if let Some(sr) = sr {
                        apply_rpr(sr, &mut base_run, &self.theme_font);
                    }
                }
            }
        }
        if let Some(ppr) = ppr {
            apply_ppr(ppr, &mut pstyle);
            if let Some(np) = ppr.child("w:numPr") {
                let level = np.child("w:ilvl").and_then(val).and_then(|v| v.parse().ok()).unwrap_or(0u8);
                if let Some(id) = np.child("w:numId").and_then(val) {
                    pstyle.list = self.list_kind(id, level);
                    pstyle.level = level;
                    if pstyle.list.is_some() {
                        // O recuo da lista é do editor (o do Word inclui o marcador).
                        pstyle.left_indent = (pstyle.left_indent - 18.0 * (level as f32 + 1.0)).max(0.0);
                        pstyle.first_indent = pstyle.first_indent.max(0.0);
                    }
                }
            }
        }
        let mut para = Paragraph { text: String::new(), spans: Vec::new(), style: pstyle.clone(), image: None, table: None };
        let mut pending_break = false;
        let mut runs = Vec::new();
        collect_runs(e, &mut runs, None);
        for (r, link) in runs {
            let mut style = base_run.clone();
            if let Some(rpr) = r.child("w:rPr") {
                apply_rpr(rpr, &mut style, &self.theme_font);
            }
            style.link = link.and_then(|id| self.rels.get(&id).cloned());
            for c in r.elements() {
                match c.name.as_str() {
                    "w:t" | "w:delText" if c.name == "w:t" => push(&mut para, &c.text(), &style),
                    "w:tab" | "w:ptab" => push(&mut para, "\t", &style),
                    "w:br" if c.attr("w:type") == Some("page") => {
                        finish(out, &mut para, &style);
                        pending_break = true;
                        para.style = pstyle.clone();
                        para.style.page_break_before = true;
                    }
                    "w:br" | "w:cr" => push(&mut para, &LINE_BREAK.to_string(), &style),
                    "w:noBreakHyphen" => push(&mut para, "\u{2011}", &style),
                    "w:footnoteReference" => {
                        let text = c.attr("w:id").and_then(|i| self.footnotes.get(i)).cloned().unwrap_or_default();
                        let mut notes = self.notes.borrow_mut();
                        let id = notes.len() as u32 + 1;
                        notes.push((id, text));
                        let s = CharStyle { note: Some(id), baseline: Baseline::Normal, ..style.clone() };
                        push(&mut para, &NOTE.to_string(), &s);
                    }
                    "w:sym" => {
                        if let Some(ch) = c.attr("w:char").and_then(|h| u32::from_str_radix(h, 16).ok()).and_then(char::from_u32) {
                            push(&mut para, &ch.to_string(), &style);
                        }
                    }
                    "w:drawing" | "w:pict" => {
                        if let Some(img) = self.image(c) {
                            finish(out, &mut para, &style);
                            let mut ps = pstyle.clone();
                            if ps.align == Align::Left {
                                ps.align = Align::Center;
                            }
                            out.push(Paragraph::image(img, style.clone(), ps));
                            para.style = pstyle.clone();
                        }
                    }
                    _ => {}
                }
            }
        }
        let _ = pending_break;
        if para.text.is_empty() && para.spans.is_empty() {
            para.spans.push(Span { len: 0, style: base_run });
        }
        // Parágrafo vazio que sobrou depois de uma imagem não vira linha a mais.
        if !(para.text.is_empty() && out.last().is_some_and(|p| p.image.is_some()) && e.elements().any(|r| r.child("w:drawing").is_some())) {
            para.normalize();
            out.push(para);
        }
    }

    /// Uma tabela do Word: colunas (`w:gridCol`), linhas, células (as mescladas na
    /// horizontal ocupam o espaço delas, com células vazias depois), fundo e bordas.
    fn table(&self, e: &Element) -> Option<Paragraph> {
        let cols: Vec<f32> = e.child("w:tblGrid").map(|g| g.elements().filter(|c| c.name == "w:gridCol").filter_map(|c| num(Some(c), "w:w")).map(|w| w / 20.0).collect()).unwrap_or_default();
        let mut rows: Vec<Vec<super::Cell>> = Vec::new();
        for tr in e.elements().filter(|r| r.name == "w:tr") {
            let mut row = Vec::new();
            for tc in tr.elements().filter(|c| c.name == "w:tc") {
                let mut paras = Vec::new();
                self.paragraphs(tc, &mut paras);
                let mut cell = super::Cell::from_paras(paras);
                cell.shade = tc.path("w:tcPr/w:shd").and_then(|s| s.attr("w:fill")).filter(|f| *f != "auto").and_then(parse_hex);
                let span = tc.path("w:tcPr/w:gridSpan").and_then(val).and_then(|v| v.parse::<usize>().ok()).unwrap_or(1);
                row.push(cell);
                for _ in 1..span {
                    row.push(super::Cell::empty(&self.default_run));
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
                row.push(super::Cell::empty(&self.default_run));
            }
        }
        let mut cols = cols;
        if cols.len() != n || cols.iter().any(|c| *c <= 0.0) {
            cols = vec![self.content_width / n as f32; n];
        }
        // Bordas: as da tabela, se não forem "nil"/"none" (sem nada, como a "Tabela com grade").
        let borders = e.path("w:tblPr/w:tblBorders").is_none_or(|b| b.elements().any(|x| !matches!(x.attr("w:val"), Some("nil" | "none"))));
        let table = super::Table { cols, rows, borders };
        Some(Paragraph::table(table, self.default_run.clone(), ParaStyle::default()))
    }

    fn image(&self, e: &Element) -> Option<Image> {
        let mut blips = Vec::new();
        e.find_all("a:blip", &mut blips);
        let rid = blips.first().and_then(|b| b.attr("r:embed"))?;
        let target = self.rels.get(rid)?;
        let path = if target.starts_with('/') { target.trim_start_matches('/').to_string() } else { format!("word/{target}") };
        let data = self.files.get(&path)?.clone();
        let mut img = Image::from_bytes(data, self.content_width)?;
        let mut extents = Vec::new();
        e.find_all("wp:extent", &mut extents);
        if let Some(ext) = extents.first() {
            let cx = ext.attr("cx").and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.0) / 12700.0;
            let cy = ext.attr("cy").and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.0) / 12700.0;
            if cx > 0.0 && cy > 0.0 {
                img.width = cx;
                img.height = cy;
            }
        }
        Some(img)
    }
}

/// O elemento depois de `e` em `parent` é um parágrafo?
fn e_next_is_para(parent: &Element, e: &Element) -> Option<bool> {
    let mut it = parent.elements().skip_while(|x| !std::ptr::eq(*x, e));
    it.next()?;
    Some(it.next()?.name == "w:p")
}

/// Os trechos (`w:r`) do parágrafo em ordem, entrando em links e campos.
fn collect_runs<'a>(e: &'a Element, out: &mut Vec<(&'a Element, Option<String>)>, link: Option<String>) {
    for c in e.elements() {
        match c.name.as_str() {
            "w:r" => out.push((c, link.clone())),
            "w:hyperlink" => collect_runs(c, out, c.attr("r:id").map(str::to_string).or(link.clone())),
            "w:ins" | "w:smartTag" | "w:fldSimple" | "w:customXml" | "w:sdt" | "w:sdtContent" => collect_runs(c, out, link.clone()),
            _ => {}
        }
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

fn finish(out: &mut Vec<Paragraph>, para: &mut Paragraph, style: &CharStyle) {
    if para.text.is_empty() {
        return;
    }
    let mut p = std::mem::replace(para, Paragraph { text: String::new(), spans: Vec::new(), style: para.style.clone(), image: None, table: None });
    if p.spans.is_empty() {
        p.spans.push(Span { len: 0, style: style.clone() });
    }
    p.normalize();
    out.push(p);
}

fn xml_of(files: &HashMap<String, Vec<u8>>, name: &str) -> Option<Element> {
    xml::parse(&String::from_utf8_lossy(files.get(name)?))
}

fn rels_of(files: &HashMap<String, Vec<u8>>, name: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    if let Some(root) = xml_of(files, name) {
        for r in root.elements() {
            if let (Some(id), Some(t)) = (r.attr("Id"), r.attr("Target")) {
                out.insert(id.to_string(), t.to_string());
            }
        }
    }
    out
}

/// Texto de um cabeçalho/rodapé e se ele tem o campo do número da página (e o
/// alinhamento do parágrafo do número).
fn header_text(e: &Element) -> (String, Option<String>) {
    let mut text = String::new();
    let mut number = None;
    let mut paras = Vec::new();
    e.find_all("w:p", &mut paras);
    for p in paras {
        let s = p.to_owned();
        let mut instr = Vec::new();
        s.find_all("w:instrText", &mut instr);
        let mut simple = Vec::new();
        s.find_all("w:fldSimple", &mut simple);
        let has_page = instr.iter().any(|i| i.text().contains("PAGE")) || simple.iter().any(|f| f.attr("w:instr").is_some_and(|i| i.contains("PAGE")));
        let jc = s.path("w:pPr/w:jc").and_then(val).unwrap_or("left").to_string();
        if has_page {
            number = Some(jc);
            continue;
        }
        let mut ts = Vec::new();
        s.find_all("w:t", &mut ts);
        let t: String = ts.iter().map(|t| t.text()).collect();
        if !t.trim().is_empty() {
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(t.trim());
        }
    }
    (text, number)
}

/// Lê um DOCX. `None` se não for um.
pub fn read(data: &[u8]) -> Option<Doc> {
    let files = unzip(data)?;
    let document = xml_of(&files, "word/document.xml")?;
    let body = document.child("w:body")?;
    let rels = rels_of(&files, "word/_rels/document.xml.rels");
    // Fonte do tema (as que vêm como "asciiTheme").
    let theme_font = xml_of(&files, "word/theme/theme1.xml")
        .and_then(|t| {
            let mut v = Vec::new();
            t.find_all("a:minorFont", &mut v);
            v.first().and_then(|m| m.child("a:latin")).and_then(|l| l.attr("typeface")).map(str::to_string)
        })
        .unwrap_or_else(|| "Calibri".into());
    let mut default_run = CharStyle { family: theme_font.clone(), size: 11.0, ..Default::default() };
    let mut default_para = ParaStyle::default();
    let mut styles = HashMap::new();
    if let Some(st) = xml_of(&files, "word/styles.xml") {
        if let Some(rpr) = st.path("w:docDefaults/w:rPrDefault/w:rPr") {
            apply_rpr(rpr, &mut default_run, &theme_font);
        }
        if let Some(ppr) = st.path("w:docDefaults/w:pPrDefault/w:pPr") {
            apply_ppr(ppr, &mut default_para);
        }
        for s in st.elements().filter(|e| e.name == "w:style" && e.attr("w:type") == Some("paragraph")) {
            let id = s.attr("w:styleId").unwrap_or("").to_string();
            let based = s.child("w:basedOn").and_then(val).map(str::to_string);
            styles.insert(id.clone(), (s.child("w:pPr").cloned(), s.child("w:rPr").cloned(), based));
            // O estilo padrão ("Normal") vale para todos.
            if s.attr("w:default") == Some("1") {
                if let Some(ppr) = s.child("w:pPr") {
                    apply_ppr(ppr, &mut default_para);
                }
                if let Some(rpr) = s.child("w:rPr") {
                    apply_rpr(rpr, &mut default_run, &theme_font);
                }
            }
        }
    }
    default_para.name = String::new();
    let mut nums = HashMap::new();
    let mut formats = HashMap::new();
    if let Some(n) = xml_of(&files, "word/numbering.xml") {
        for e in n.elements() {
            match e.name.as_str() {
                "w:abstractNum" => {
                    let id = e.attr("w:abstractNumId").unwrap_or("").to_string();
                    for lvl in e.elements().filter(|l| l.name == "w:lvl") {
                        let level: u8 = lvl.attr("w:ilvl").and_then(|v| v.parse().ok()).unwrap_or(0);
                        let fmt = lvl.child("w:numFmt").and_then(val).unwrap_or("bullet");
                        let text = lvl.child("w:lvlText").and_then(val).unwrap_or("");
                        let kind = match fmt {
                            "bullet" if text.contains('–') || text.contains('-') => ListKind::Dash,
                            "bullet" | "none" => ListKind::Bullet,
                            "lowerLetter" | "upperLetter" => ListKind::Letter,
                            _ => ListKind::Number,
                        };
                        formats.insert((id.clone(), level), kind);
                    }
                }
                "w:num" => {
                    if let (Some(id), Some(abs)) = (e.attr("w:numId"), e.child("w:abstractNumId").and_then(val)) {
                        nums.insert(id.to_string(), abs.to_string());
                    }
                }
                _ => {}
            }
        }
    }
    // Página.
    let mut page = PageSetup::a4();
    if let Some(sect) = body.child("w:sectPr") {
        if let Some(sz) = sect.child("w:pgSz") {
            if let (Some(w), Some(h)) = (num(Some(sz), "w:w"), num(Some(sz), "w:h")) {
                page.width = w / 20.0;
                page.height = h / 20.0;
            }
        }
        if let Some(m) = sect.child("w:pgMar") {
            for (i, k) in ["w:top", "w:right", "w:bottom", "w:left"].iter().enumerate() {
                if let Some(v) = num(Some(m), k) {
                    page.margins[i] = v.abs() / 20.0;
                }
            }
        }
        let title_pg = on(sect.child("w:titlePg")).unwrap_or(false);
        for r in sect.elements().filter(|e| e.name == "w:headerReference" || e.name == "w:footerReference") {
            if r.attr("w:type").is_some_and(|t| t != "default") {
                continue;
            }
            let Some(target) = r.attr("r:id").and_then(|id| rels.get(id)) else {
                continue;
            };
            let Some(part) = xml_of(&files, &format!("word/{target}")) else {
                continue;
            };
            let (text, number) = header_text(&part);
            let header = r.name == "w:headerReference";
            if header {
                page.header = text;
            } else {
                page.footer = text;
            }
            if let Some(jc) = number {
                page.numbers.show = true;
                page.numbers.position = match (header, jc.as_str()) {
                    (true, _) => NumberPosition::TopRight,
                    (false, "center") => NumberPosition::BottomCenter,
                    _ => NumberPosition::BottomRight,
                };
                page.numbers.show_from = if title_pg { 2 } else { 1 };
            }
        }
    }
    // A página do número gravada pelo mide (o Word só sabe "não na primeira").
    if let Some(settings) = xml_of(&files, "word/settings.xml") {
        let mut vars = Vec::new();
        settings.find_all("w:docVar", &mut vars);
        if let Some(n) = vars.iter().find(|v| v.attr("w:name") == Some("mideShowFrom")).and_then(|v| v.attr("w:val")).and_then(|v| v.parse().ok()) {
            page.numbers.show_from = n;
        }
    }
    // Notas de rodapé: o texto de cada uma (sem a marca do número).
    let mut footnotes = HashMap::new();
    if let Some(f) = xml_of(&files, "word/footnotes.xml") {
        for n in f.elements().filter(|e| e.name == "w:footnote" && e.attr("w:type").is_none()) {
            let mut ts = Vec::new();
            n.find_all("w:t", &mut ts);
            let text: String = ts.iter().map(|t| t.text()).collect();
            footnotes.insert(n.attr("w:id").unwrap_or("").to_string(), text.trim().to_string());
        }
    }
    let reader = Reader { files: &files, footnotes, notes: std::cell::RefCell::new(Vec::new()), rels, styles, default_run: default_run.clone(), default_para, theme_font, nums, formats, content_width: page.content_width() };
    let mut paras = Vec::new();
    reader.paragraphs(body, &mut paras);
    // Parágrafo vazio que só tinha a quebra de página: a quebra passa para o seguinte.
    let mut i = 0;
    while i + 1 < paras.len() {
        if paras[i].text.is_empty() && paras[i].image.is_none() && paras[i].style.page_break_before {
            paras.remove(i);
            paras[i].style.page_break_before = true;
        } else {
            i += 1;
        }
    }
    if paras.is_empty() {
        paras.push(Paragraph::new("", default_run, ParaStyle::default()));
    }
    let _ = Node::Text(String::new());
    let notes = reader.notes.take();
    Some(Doc { paras, page, notes })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::richtext::{cm, Pos};

    #[test]
    fn grava_e_le_docx() {
        let mut d = Doc::new();
        d.insert_text(Pos::default(), "Introdução\nTexto do corpo com acentuação.\nitem um\nitem dois", &CharStyle { family: "Times New Roman".into(), ..Default::default() });
        d.paras[0].style.name = "Título 1".into();
        d.apply_char(Pos::new(0, 0), Pos::new(0, "Introdução".len()), |s| {
            s.bold = true;
            s.size = 16.0;
        });
        d.apply_char(Pos::new(1, 0), Pos::new(1, 5), |s| {
            s.color = [0, 0, 255];
            s.highlight = Some([255, 255, 0]);
            s.italic = true;
        });
        d.apply_para(Pos::new(1, 0), Pos::new(1, 0), |p| {
            p.align = Align::Justify;
            p.first_indent = cm(1.25);
            p.line_spacing = 1.5;
        });
        d.apply_para(Pos::new(2, 0), Pos::new(3, 0), |p| p.list = Some(ListKind::Number));
        d.page.margins = [cm(3.0), cm(2.0), cm(2.0), cm(3.0)];
        d.page.numbers.show = true;
        d.page.numbers.position = NumberPosition::TopRight;
        d.page.numbers.show_from = 3;
        d.page.footer = "Rodapé".into();
        d.apply_char(Pos::new(1, 9), Pos::new(1, 14), |s| s.link = Some("https://mide.dev".into()));
        let end = Pos::new(1, d.paras[1].text.len());
        d.insert_note(end, "Nota do Word.");
        // Tabela no fim.
        let last = d.end();
        let after = d.insert_table(last, 2, 3);
        if let Some(t) = d.paras[after.para - 1].table.as_mut() {
            t.rows[0][1].doc.insert_text(Pos::default(), "B1", &CharStyle { bold: true, ..CharStyle::default() });
            t.rows[1][2].shade = Some([200, 220, 255]);
        }
        let bytes = write(&d);
        let back = read(&bytes).expect("docx");
        assert!(back.paras.len() >= 5, "{:?}", back.paras.iter().map(|p| &p.text).collect::<Vec<_>>());
        assert_eq!(back.paras[0].text, "Introdução");
        assert_eq!(back.paras[0].style.name, "Título 1");
        assert!(back.paras[0].spans[0].style.bold);
        assert_eq!(back.paras[0].spans[0].style.size, 16.0);
        assert_eq!(back.paras[1].spans[0].style.color, [0, 0, 255]);
        assert_eq!(back.paras[1].spans[0].style.highlight, Some([255, 255, 0]));
        assert!(back.paras[1].spans[0].style.italic);
        assert_eq!(back.paras[1].spans[0].style.family, "Times New Roman");
        assert_eq!(back.paras[1].style.align, Align::Justify);
        assert!((back.paras[1].style.first_indent - cm(1.25)).abs() < 0.2);
        assert!((back.paras[1].style.line_spacing - 1.5).abs() < 0.01);
        assert_eq!(back.paras[2].style.list, Some(ListKind::Number));
        assert!(back.paras[2].style.left_indent.abs() < 0.5);
        assert!((back.page.margins[0] - cm(3.0)).abs() < 0.2);
        assert!(back.page.numbers.show);
        assert_eq!(back.page.numbers.position, NumberPosition::TopRight);
        assert_eq!(back.page.numbers.show_from, 3);
        assert_eq!(back.page.footer, "Rodapé");
        let table = back.paras.iter().find_map(|p| p.table.as_ref()).expect("tabela");
        assert_eq!((table.rows.len(), table.cols.len()), (2, 3));
        assert_eq!(table.rows[0][1].doc.paras[0].text, "B1");
        assert!(table.rows[0][1].doc.paras[0].spans[0].style.bold);
        assert_eq!(table.rows[1][2].shade, Some([200, 220, 255]));
        assert!(table.borders);
        assert!(back.paras[1].spans.iter().any(|s| s.style.link.as_deref() == Some("https://mide.dev")), "link");
        assert_eq!(back.notes.len(), 1);
        assert_eq!(back.notes[0].1, "Nota do Word.");
        assert!(back.paras[1].text.ends_with(crate::richtext::NOTE));
        if let Ok(dir) = std::env::var("MIDE_DOCX_OUT") {
            let _ = std::fs::write(format!("{dir}/teste.docx"), &bytes);
        }
    }
}
