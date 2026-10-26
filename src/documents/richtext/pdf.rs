//! Exportar o documento formatado em PDF, escrito aqui (sem biblioteca de PDF).
//!
//! As letras saem nas posições do mesmo layout da tela (o que se vê é o que se imprime),
//! com as fontes embutidas (o PDF abre igual em qualquer computador e na gráfica) e um
//! mapa para o Unicode (copiar e buscar o texto no leitor de PDF funciona). Imagens PNG
//! (com transparência) e JPEG (sem recompactar), cores, realce, sublinhado, tachado,
//! cabeçalho, rodapé e números de página. O conteúdo é compactado (zlib).

use std::{collections::BTreeMap, io::Write};

use cosmic_text::{fontdb, CacheKeyFlags, FontSystem};

use super::{
    layout::{Glyph, Layout, Layouter},
    CharStyle, Doc, NumberPosition, LINE_BREAK, OBJECT,
};

fn deflate(data: &[u8]) -> Vec<u8> {
    let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    let _ = enc.write_all(data);
    enc.finish().unwrap_or_default()
}

struct Writer {
    buf: Vec<u8>,
    offsets: Vec<usize>,
}

impl Writer {
    fn reserve(&mut self) -> usize {
        self.offsets.push(0);
        self.offsets.len()
    }

    fn obj(&mut self, id: usize, body: &[u8]) {
        self.offsets[id - 1] = self.buf.len();
        self.buf.extend_from_slice(format!("{id} 0 obj\n").as_bytes());
        self.buf.extend_from_slice(body);
        self.buf.extend_from_slice(b"\nendobj\n");
    }

    fn stream(&mut self, id: usize, dict: &str, data: &[u8], compress: bool) {
        let packed;
        let (data, filter) = if compress {
            packed = deflate(data);
            (&packed[..], "/Filter /FlateDecode ")
        } else {
            (data, "")
        };
        let mut body = format!("<< {dict} {filter}/Length {} >>\nstream\n", data.len()).into_bytes();
        body.extend_from_slice(data);
        body.extend_from_slice(b"\nendstream");
        self.obj(id, &body);
    }
}

/// Uma fonte usada no documento.
struct FontUse {
    id: fontdb::ID,
    /// Largura de cada letra (milésimos do tamanho) e o texto que ela desenha.
    widths: BTreeMap<u16, f32>,
    unicode: BTreeMap<u16, String>,
}

fn be16(d: &[u8], at: usize) -> usize {
    d.get(at..at + 2).map_or(0, |b| u16::from_be_bytes([b[0], b[1]]) as usize)
}

fn be32(d: &[u8], at: usize) -> usize {
    d.get(at..at + 4).map_or(0, |b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize)
}

/// Uma fonte de dentro de uma coleção (.ttc) como arquivo próprio (o PDF não aceita
/// coleções): copia as tabelas dela com um índice novo.
fn single_font(data: &[u8], index: u32) -> Option<Vec<u8>> {
    if data.get(0..4) != Some(b"ttcf") {
        return Some(data.to_vec());
    }
    if index as usize >= be32(data, 8) {
        return None;
    }
    let off = be32(data, 12 + 4 * index as usize);
    let tables = be16(data, off + 4);
    let mut out = data.get(off..off + 12)?.to_vec();
    let mut cursor = 12 + 16 * tables;
    let mut bodies = Vec::new();
    for i in 0..tables {
        let r = off + 12 + 16 * i;
        let (toff, len) = (be32(data, r + 8), be32(data, r + 12));
        out.extend_from_slice(data.get(r..r + 8)?);
        out.extend_from_slice(&(cursor as u32).to_be_bytes());
        out.extend_from_slice(&(len as u32).to_be_bytes());
        bodies.push(data.get(toff..toff + len)?);
        cursor += (len + 3) & !3;
    }
    for b in bodies {
        out.extend_from_slice(b);
        while out.len() % 4 != 0 {
            out.push(0);
        }
    }
    Some(out)
}

/// Nome de fonte para o PDF (só letras, números e hífen).
fn pdf_name(s: &str) -> String {
    let n: String = s.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
    if n.is_empty() { "Fonte".into() } else { n }
}

fn utf16_hex(s: &str) -> String {
    s.encode_utf16().map(|u| format!("{u:04X}")).collect()
}

struct Page {
    ops: String,
    fonts: Vec<usize>,
}

/// Gera o PDF das páginas `from..=to` (contando de 0; `None`: todas).
pub fn write(doc: &Doc, layout: &Layout, layouter: &mut Layouter, fonts: &mut FontSystem, range: Option<(usize, usize)>) -> Vec<u8> {
    let page = &doc.page;
    let (pw, ph) = (page.width, page.height);
    let ml = page.margins[3];
    let total = layout.pages.max(1);
    let (from, to) = range.map_or((0, total - 1), |(a, b)| (a.min(total - 1), b.min(total - 1).max(a.min(total - 1))));
    let mut w = Writer { buf: b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec(), offsets: Vec::new() };
    let catalog = w.reserve();
    let pages_id = w.reserve();
    let resources = w.reserve();
    let mut font_uses: Vec<FontUse> = Vec::new();
    let mut images: Vec<(usize, String)> = Vec::new();
    let mut image_of_para: BTreeMap<usize, usize> = BTreeMap::new();
    let mut pages: Vec<Page> = (from..=to).map(|_| Page { ops: String::new(), fonts: Vec::new() }).collect();

    let font_index = |id: fontdb::ID, font_uses: &mut Vec<FontUse>| match font_uses.iter().position(|f| f.id == id) {
        Some(i) => i,
        None => {
            font_uses.push(FontUse { id, widths: BTreeMap::new(), unicode: BTreeMap::new() });
            font_uses.len() - 1
        }
    };
    // Letras de uma linha: texto do parágrafo `text` (para o Unicode), na linha de base
    // `base_y` (do topo da página) e a partir do x `x0`.
    let glyph_ops = |ops: &mut String, glyphs: &[Glyph], text: &str, x0: f32, base_y: f32, color_of: &dyn Fn(&Glyph) -> [u8; 3], font_uses: &mut Vec<FontUse>| {
        if glyphs.is_empty() {
            return;
        }
        ops.push_str("BT\n");
        let mut cur_font = usize::MAX;
        let mut cur_size = -1.0f32;
        let mut cur_color = [1u8, 2, 3];
        for g in glyphs {
            let f = font_index(g.font_id, font_uses);
            let use_ = &mut font_uses[f];
            let width = g.width / g.size.max(0.01) * 1000.0;
            let entry = use_.widths.entry(g.glyph_id).or_insert(width);
            *entry = entry.min(width);
            let s = text.get(g.start..g.end).unwrap_or("").replace([OBJECT, LINE_BREAK, super::NOTE], "");
            if !s.is_empty() {
                use_.unicode.entry(g.glyph_id).or_insert(s);
            }
            if f != cur_font || g.size != cur_size {
                ops.push_str(&format!("/F{f} {:.2} Tf\n", g.size));
                cur_font = f;
                cur_size = g.size;
            }
            let c = color_of(g);
            if c != cur_color {
                ops.push_str(&format!("{:.3} {:.3} {:.3} rg\n", c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0));
                cur_color = c;
            }
            let skew = if g.flags.contains(CacheKeyFlags::FAKE_ITALIC) { 0.21 } else { 0.0 };
            ops.push_str(&format!("1 0 {skew} 1 {:.2} {:.2} Tm <{:04X}> Tj\n", x0 + g.x, ph - (base_y + g.dy), g.glyph_id));
        }
        ops.push_str("ET\n");
    };
    let rect = |ops: &mut String, x: f32, top: f32, w: f32, h: f32, c: [u8; 3]| {
        ops.push_str(&format!("{:.3} {:.3} {:.3} rg {:.2} {:.2} {:.2} {:.2} re f\n", c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0, x, ph - top - h, w, h));
    };

    for placed in &layout.placed {
        if placed.page < from || placed.page > to {
            continue;
        }
        let pg = &mut pages[placed.page - from];
        let para = &doc.paras[placed.para];
        let line = &layout.paras[placed.para].lines[placed.line];
        let styles = &layout.styles[placed.para];
        let base = placed.y + line.baseline;
        // Tabela: fundo, texto e bordas das células desta linha.
        if let (Some(table), Some(tb)) = (&para.table, &layout.paras[placed.para].table) {
            let r = placed.line;
            let rh = line.height;
            for (c, cell_box) in tb.rows[r].iter().enumerate() {
                let (cx, cw) = (ml + tb.col_x[c], tb.col_w[c]);
                let cell = &table.rows[r][c];
                if let Some(shade) = cell.shade {
                    rect(&mut pg.ops, cx, placed.y, cw, rh, shade);
                }
                let (ox, oy) = (cx + super::layout::CELL_PAD_X, placed.y + super::layout::CELL_PAD_Y);
                for cpl in &cell_box.layout.placed {
                    let cpara = &cell.doc.paras[cpl.para];
                    let cline = &cell_box.layout.paras[cpl.para].lines[cpl.line];
                    let cstyles = &cell_box.layout.styles[cpl.para];
                    let cstyle_of = |g: &Glyph| cstyles.get(g.span).cloned().unwrap_or_default();
                    for g in &cline.glyphs {
                        if let Some(hl) = cstyle_of(g).highlight {
                            rect(&mut pg.ops, ox + g.x, oy + cpl.y, g.width, cline.height, hl);
                        }
                    }
                    let base = oy + cpl.y + cline.baseline;
                    if cpl.line == 0
                        && let Some((glyphs, mx)) = &cell_box.layout.paras[cpl.para].marker {
                            let col = cstyles.first().map_or([0, 0, 0], |s| s.color);
                            glyph_ops(&mut pg.ops, glyphs, "", ox + mx, base, &|_| col, &mut font_uses);
                        }
                    glyph_ops(&mut pg.ops, &cline.glyphs, &cpara.text, ox, base, &|g| cstyle_of(g).color, &mut font_uses);
                    for g in &cline.glyphs {
                        let st = cstyle_of(g);
                        let thick = st.size * 0.06;
                        if st.underline || st.link.is_some() {
                            rect(&mut pg.ops, ox + g.x, base + st.size * 0.12, g.width, thick, st.color);
                        }
                        if st.strike {
                            rect(&mut pg.ops, ox + g.x, base - st.size * 0.28, g.width, thick, st.color);
                        }
                    }
                }
                if table.borders {
                    let b = [128, 128, 132];
                    rect(&mut pg.ops, cx, placed.y, cw, 0.5, b);
                    rect(&mut pg.ops, cx, placed.y + rh - 0.5, cw, 0.5, b);
                    rect(&mut pg.ops, cx, placed.y, 0.5, rh, b);
                    rect(&mut pg.ops, cx + cw - 0.5, placed.y, 0.5, rh, b);
                }
            }
            continue;
        }
        if let Some(img) = &para.image {
            let index = *image_of_para.entry(placed.para).or_insert_with(|| {
                let id = w.reserve();
                images.push((id, String::new()));
                images.len() - 1
            });
            if images[index].1.is_empty() {
                images[index].1 = write_image(&mut w, images[index].0, img);
            }
            let (iw, ih) = (line.x1 - line.x0, line.baseline);
            pg.ops.push_str(&format!("q {iw:.2} 0 0 {ih:.2} {:.2} {:.2} cm /Im{index} Do Q\n", ml + line.x0, ph - placed.y - ih));
            continue;
        }
        let style_of = |g: &Glyph| styles.get(g.span).cloned().unwrap_or_default();
        for g in &line.glyphs {
            if let Some(hl) = style_of(g).highlight {
                rect(&mut pg.ops, ml + g.x, placed.y, g.width, line.height, hl);
            }
        }
        if placed.line == 0
            && let Some((glyphs, mx)) = &layout.paras[placed.para].marker {
                let c = styles.first().map_or([0, 0, 0], |s| s.color);
                let marker_text: String = String::new();
                glyph_ops(&mut pg.ops, glyphs, &marker_text, ml + mx, base, &|_| c, &mut font_uses);
            }
        glyph_ops(&mut pg.ops, &line.glyphs, &para.text, ml, base, &|g| style_of(g).color, &mut font_uses);
        if let Some((a, b)) = line.leader {
            let c = styles.first().map_or([0, 0, 0], |s| s.color);
            let mut x = a + 4.0;
            while x < b - 4.0 {
                rect(&mut pg.ops, ml + x, base - 1.2, 1.2, 1.2, c);
                x += 4.0;
            }
        }
        for g in &line.glyphs {
            let st = style_of(g);
            let thick = st.size * 0.06;
            if st.underline || st.link.is_some() {
                rect(&mut pg.ops, ml + g.x, base + st.size * 0.12, g.width, thick, st.color);
            }
            if st.strike {
                rect(&mut pg.ops, ml + g.x, base - st.size * 0.28, g.width, thick, st.color);
            }
        }
    }
    // Notas de rodapé no pé de cada página.
    for (i, pg) in pages.iter_mut().enumerate() {
        let page_i = from + i;
        let notes_h = layout.notes_height(page_i);
        if notes_h == 0.0 {
            continue;
        }
        let mut y = page.height - page.margins[2] - notes_h;
        rect(&mut pg.ops, ml, y + 4.0, page.content_width() / 3.0, 0.5, [0, 0, 0]);
        y += super::layout::NOTE_SEPARATOR;
        for id in &layout.page_notes[page_i] {
            let Some((n, boxed, styles)) = layout.note_boxes.get(id) else {
                continue;
            };
            let full = format!("{n} {}", doc.note_text(*id).unwrap_or(""));
            for line in &boxed.lines {
                glyph_ops(&mut pg.ops, &line.glyphs, &full, ml, y + line.baseline, &|g| styles.get(g.span).map_or([0, 0, 0], |st| st.color), &mut font_uses);
                y += line.height;
            }
        }
    }
    // Cabeçalho, rodapé e números.
    // Cabeçalho, rodapé e números: só a fonte do documento (sem o negrito de um título).
    let family = doc.paras.iter().find(|p| p.style.name.is_empty() && !p.text.is_empty()).or(doc.paras.first()).map(|p| p.spans[0].style.family.clone()).unwrap_or_default();
    let small = CharStyle { size: 10.0, color: [80, 80, 80], family, ..CharStyle::default() };
    let content_w = page.content_width();
    let header_y = (page.margins[0] * 0.5).max(18.0);
    let footer_y = page.height - (page.margins[2] * 0.5).max(18.0) + 8.0;
    for (i, pg) in pages.iter_mut().enumerate() {
        let n = from + i + 1;
        let mut items: Vec<(String, f32, f32, CharStyle)> = vec![(page.header.clone(), header_y, 0.5, small.clone()), (page.footer.clone(), footer_y, 0.5, small.clone())];
        if page.numbers.show && n >= page.numbers.show_from {
            let st = CharStyle { color: [0, 0, 0], ..small.clone() };
            items.push(match page.numbers.position {
                NumberPosition::TopRight => (n.to_string(), super::cm(2.0), 1.0, st),
                NumberPosition::BottomCenter => (n.to_string(), footer_y, 0.5, st),
                NumberPosition::BottomRight => (n.to_string(), footer_y, 1.0, st),
            });
        }
        for (text, y, align, style) in items {
            if text.is_empty() {
                continue;
            }
            let (glyphs, gw) = layouter.shape_simple(fonts, &text, &style);
            let x = ml + (content_w - gw) * align;
            let c = style.color;
            glyph_ops(&mut pg.ops, &glyphs, &text, x, y, &|_| c, &mut font_uses);
        }
        pg.fonts = (0..font_uses.len()).collect();
    }
    // Páginas.
    let mut page_ids = Vec::new();
    for pg in &pages {
        let content = w.reserve();
        w.stream(content, "", pg.ops.as_bytes(), true);
        let id = w.reserve();
        w.obj(id, format!("<< /Type /Page /Parent {pages_id} 0 R /MediaBox [0 0 {pw:.2} {ph:.2}] /Resources {resources} 0 R /Contents {content} 0 R >>").as_bytes());
        page_ids.push(id);
    }
    // Fontes.
    let mut font_refs = String::new();
    for (i, f) in font_uses.iter().enumerate() {
        if let Some(id) = write_font(&mut w, fonts, f) {
            font_refs.push_str(&format!("/F{i} {id} 0 R "));
        }
    }
    let image_refs: String = images.iter().enumerate().map(|(i, (id, _))| format!("/Im{i} {id} 0 R ")).collect();
    w.obj(resources, format!("<< /Font << {font_refs}>> /XObject << {image_refs}>> >>").as_bytes());
    let kids: String = page_ids.iter().map(|id| format!("{id} 0 R ")).collect();
    w.obj(pages_id, format!("<< /Type /Pages /Kids [{kids}] /Count {} >>", page_ids.len()).as_bytes());
    w.obj(catalog, format!("<< /Type /Catalog /Pages {pages_id} 0 R >>").as_bytes());
    let info = w.reserve();
    w.obj(info, b"<< /Producer (mide Writer) /Creator (mide-writer) >>");
    let xref = w.buf.len();
    let mut tail = format!("xref\n0 {}\n0000000000 65535 f \n", w.offsets.len() + 1);
    for off in &w.offsets {
        tail.push_str(&format!("{off:010} 00000 n \n"));
    }
    tail.push_str(&format!("trailer\n<< /Size {} /Root {catalog} 0 R /Info {info} 0 R >>\nstartxref\n{xref}\n%%EOF\n", w.offsets.len() + 1));
    w.buf.extend_from_slice(tail.as_bytes());
    w.buf
}

/// Grava a imagem (JPEG como veio; PNG em RGB com a transparência à parte).
fn write_image(w: &mut Writer, id: usize, img: &super::Image) -> String {
    let (iw, ih) = (img.pixmap.width(), img.pixmap.height());
    if img.format == "jpeg" {
        let gray = image::load_from_memory(&img.data).map(|d| d.color().channel_count() == 1).unwrap_or(false);
        let cs = if gray { "/DeviceGray" } else { "/DeviceRGB" };
        w.stream(id, &format!("/Type /XObject /Subtype /Image /Width {iw} /Height {ih} /ColorSpace {cs} /BitsPerComponent 8 /Filter /DCTDecode"), &img.data, false);
        return "jpeg".into();
    }
    let mut rgb = Vec::with_capacity((iw * ih * 3) as usize);
    let mut alpha = Vec::with_capacity((iw * ih) as usize);
    for p in img.pixmap.pixels() {
        let c = p.demultiply();
        rgb.extend_from_slice(&[c.red(), c.green(), c.blue()]);
        alpha.push(c.alpha());
    }
    let smask = if alpha.iter().any(|a| *a < 255) {
        let mask = w.reserve();
        w.stream(mask, &format!("/Type /XObject /Subtype /Image /Width {iw} /Height {ih} /ColorSpace /DeviceGray /BitsPerComponent 8"), &alpha, true);
        format!("/SMask {mask} 0 R ")
    } else {
        String::new()
    };
    w.stream(id, &format!("/Type /XObject /Subtype /Image /Width {iw} /Height {ih} /ColorSpace /DeviceRGB /BitsPerComponent 8 {smask}"), &rgb, true);
    "png".into()
}

/// Embute a fonte (TrueType ou OpenType/CFF inteira) e devolve o objeto Type0.
fn write_font(w: &mut Writer, fonts: &mut FontSystem, f: &FontUse) -> Option<usize> {
    let face = fonts.db().face(f.id)?;
    let name = pdf_name(&face.post_script_name);
    let (italic, bold) = (face.style != fontdb::Style::Normal, face.weight.0 >= 600);
    let data = fonts.db().with_face_data(f.id, single_font)??;
    let cff = data.get(0..4) == Some(b"OTTO");
    let file = w.reserve();
    if cff {
        w.stream(file, "/Subtype /OpenType", &data, true);
    } else {
        w.stream(file, &format!("/Length1 {}", data.len()), &data, true);
    }
    let flags = 32 | if italic { 64 } else { 0 } | if bold { 1 << 18 } else { 0 };
    let desc = w.reserve();
    w.obj(
        desc,
        format!(
            "<< /Type /FontDescriptor /FontName /{name} /Flags {flags} /FontBBox [-1000 -500 2000 1200] /ItalicAngle {} /Ascent 900 /Descent -250 /CapHeight 700 /StemV {} /{} {file} 0 R >>",
            if italic { -12 } else { 0 },
            if bold { 140 } else { 80 },
            if cff { "FontFile3" } else { "FontFile2" }
        )
        .as_bytes(),
    );
    let widths: String = f.widths.iter().map(|(gid, wd)| format!("{gid} [{:.0}] ", wd)).collect();
    let cid = w.reserve();
    w.obj(
        cid,
        format!(
            "<< /Type /Font /Subtype /{} /BaseFont /{name} /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /FontDescriptor {desc} 0 R {}/DW 1000 /W [{widths}] >>",
            if cff { "CIDFontType0" } else { "CIDFontType2" },
            if cff { "" } else { "/CIDToGIDMap /Identity " }
        )
        .as_bytes(),
    );
    // Do número da letra ao texto (copiar e buscar no leitor de PDF).
    let mut cmap = String::from("/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n/CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n");
    let entries: Vec<(&u16, &String)> = f.unicode.iter().collect();
    for chunk in entries.chunks(100) {
        cmap.push_str(&format!("{} beginbfchar\n", chunk.len()));
        for (gid, s) in chunk {
            cmap.push_str(&format!("<{:04X}> <{}>\n", gid, utf16_hex(s)));
        }
        cmap.push_str("endbfchar\n");
    }
    cmap.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
    let to_unicode = w.reserve();
    w.stream(to_unicode, "", cmap.as_bytes(), true);
    let id = w.reserve();
    w.obj(id, format!("<< /Type /Font /Subtype /Type0 /BaseFont /{name} /Encoding /Identity-H /DescendantFonts [{cid} 0 R] /ToUnicode {to_unicode} 0 R >>").as_bytes());
    Some(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::richtext::{Paragraph, ParaStyle, Pos};

    #[test]
    fn pdf_com_texto_copiavel() {
        let mut fonts = FontSystem::new();
        let mut doc = Doc::new();
        doc.insert_text(Pos::default(), "Olá, PDF do mide!\nSegunda página", &CharStyle::default());
        doc.paras[1].style.page_break_before = true;
        doc.apply_char(Pos::new(0, 0), Pos::new(0, 4), |s| s.color = [200, 0, 0]);
        doc.page.numbers.show = true;
        // Uma tabela 2x2 com texto.
        let after = doc.insert_table(Pos::new(0, doc.paras[0].text.len()), 2, 2);
        if let Some(t) = doc.paras[after.para - 1].table.as_mut() {
            t.rows[0][0].doc.insert_text(Pos::default(), "Célula A1", &CharStyle::default());
            t.rows[1][1].doc.insert_text(Pos::default(), "Célula B2", &CharStyle::default());
        }
        let end = Pos::new(0, doc.paras[0].text.len());
        doc.insert_note(end, "SILVA, J. Métodos do mide. São Paulo, 2026.");
        let mut l = Layouter::default();
        let lay = l.layout(&mut fonts, &doc);
        let pdf = write(&doc, &lay, &mut l, &mut fonts, None);
        assert!(pdf.starts_with(b"%PDF-1.7"));
        assert!(pdf.ends_with(b"%%EOF\n"));
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.contains("/Count 2"), "duas páginas");
        assert!(text.contains("/FontFile"), "fonte embutida");
        assert!(text.contains("/ToUnicode"));
        assert_eq!(lay.page_notes[0].len(), 1, "a nota fica na página 1");
        // Só a segunda página.
        let one = write(&doc, &lay, &mut l, &mut fonts, Some((1, 1)));
        assert!(String::from_utf8_lossy(&one).contains("/Count 1"));
        if let Ok(dir) = std::env::var("MIDE_PDF_OUT") {
            let _ = std::fs::write(format!("{dir}/teste.pdf"), &pdf);
        }
        let _ = (Paragraph::new("", CharStyle::default(), ParaStyle::default()),);
    }
}
