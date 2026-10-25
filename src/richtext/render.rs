//! Desenho das páginas do documento formatado num pixmap (tela) a qualquer escala.

use cosmic_text::CacheKey;
use tiny_skia::{Color, Paint, Pixmap, PixmapPaint, Rect, Transform};

use super::layout::{Glyph, Layout, Layouter};
use super::{CharStyle, Doc, NumberPosition, Pos};

/// Como desenhar: a página `page`, com o canto em `origin` (pixels) e `scale` pixels por
/// ponto; seleção e cursor, se houver.
pub struct View<'a> {
    pub page: usize,
    pub origin: (f32, f32),
    pub scale: f32,
    pub selection: Option<(Pos, Pos)>,
    pub caret: Option<Pos>,
    pub selection_color: [u8; 4],
    pub caret_color: [u8; 4],
    /// Faixa vertical (pixels) que está à vista: as linhas fora dela não são desenhadas.
    pub visible: (f32, f32),
    /// Célula sendo editada: tabela (parágrafo), linha, coluna, a seleção e o cursor
    /// dentro dela.
    pub cell: Option<CellEdit>,
    /// Palavras com erro de ortografia de cada parágrafo (bytes): ondulado vermelho.
    pub misspelled: Option<&'a std::collections::HashMap<usize, Vec<(usize, usize)>>>,
}

/// Cursor e seleção dentro de uma célula de tabela.
#[derive(Debug, Clone, Copy)]
pub struct CellEdit {
    pub table: usize,
    pub row: usize,
    pub col: usize,
    pub selection: Option<(Pos, Pos)>,
    pub caret: Option<Pos>,
}

fn fill(pixmap: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, rgba: [u8; 4]) {
    if let Some(r) = Rect::from_xywh(x, y, w.max(0.5), h.max(0.5)) {
        let mut paint = Paint::default();
        paint.set_color(Color::from_rgba8(rgba[0], rgba[1], rgba[2], rgba[3]));
        paint.anti_alias = false;
        pixmap.fill_rect(r, &paint, Transform::identity(), None);
    }
}

fn draw_glyphs(text: &mut crate::ui::text::Text, pixmap: &mut Pixmap, glyphs: &[Glyph], (ox, baseline): (f32, f32), scale: f32, color: impl Fn(&Glyph) -> [u8; 4]) {
    for g in glyphs {
        let (key, x, y) = CacheKey::new(g.font_id, g.glyph_id, g.size * scale, (ox + g.x * scale, (baseline + g.dy * scale).trunc()), g.weight, g.flags);
        text.draw_glyph(pixmap, key, (x, y), color(g));
    }
}

/// Desenha uma página (o fundo branco fica com quem chama).
pub fn draw_page(text: &mut crate::ui::text::Text, layouter: &mut Layouter, pixmap: &mut Pixmap, doc: &Doc, layout: &Layout, view: &View<'_>) {
    let s = view.scale;
    let left = view.origin.0 + doc.page.margins[3] * s;
    let sel = view.selection.map(|(a, b)| if a <= b { (a, b) } else { (b, a) }).filter(|(a, b)| a != b);
    for placed in layout.placed.iter().filter(|p| p.page == view.page) {
        let para = &doc.paras[placed.para];
        let line = &layout.paras[placed.para].lines[placed.line];
        let top = view.origin.1 + placed.y * s;
        if top > view.visible.1 || top + line.height * s < view.visible.0 {
            continue;
        }
        let baseline = top + line.baseline * s;
        let styles = &layout.styles[placed.para];
        // Seleção.
        if let Some((a, b)) = sel {
            let here = Pos::new(placed.para, line.start);
            let line_end = Pos::new(placed.para, line.end);
            if a <= line_end && b >= here {
                let x_at = |off: usize| {
                    let mut x = line.x0;
                    for &(o, cx) in &line.carets {
                        if o <= off {
                            x = cx;
                        }
                    }
                    x
                };
                let xa = if a.para == placed.para && a.off >= line.start { x_at(a.off) } else { line.x0 };
                let mut xb = if b.para == placed.para && b.off <= line.end { x_at(b.off) } else { line.x1 };
                // Seleção que passa do fim do parágrafo: um pouco além do texto.
                if b.para > placed.para && placed.line + 1 == layout.paras[placed.para].lines.len() {
                    xb += 5.0;
                }
                fill(pixmap, left + xa * s, top, (xb - xa) * s, line.height * s, view.selection_color);
            }
        }
        // Tabela: fundo, conteúdo e bordas das células desta linha.
        if let (Some(table), Some(tb)) = (&para.table, &layout.paras[placed.para].table) {
            let r = placed.line;
            let rh = line.height;
            for (c, cell_box) in tb.rows[r].iter().enumerate() {
                let (cx, cw) = (tb.col_x[c], tb.col_w[c]);
                let cell = &table.rows[r][c];
                if let Some(shade) = cell.shade {
                    fill(pixmap, left + cx * s, top, cw * s, rh * s, [shade[0], shade[1], shade[2], 255]);
                }
                let editing = view.cell.filter(|e| e.table == placed.para && e.row == r && e.col == c);
                let sub = View {
                    page: 0,
                    origin: (left + (cx + super::layout::CELL_PAD_X) * s, top + super::layout::CELL_PAD_Y * s),
                    scale: s,
                    selection: editing.and_then(|e| e.selection),
                    caret: editing.and_then(|e| e.caret),
                    selection_color: view.selection_color,
                    caret_color: view.caret_color,
                    visible: view.visible,
                    cell: None,
                    misspelled: None,
                };
                draw_page(text, layouter, pixmap, &cell.doc, &cell_box.layout, &sub);
                if table.borders {
                    let b = [128, 128, 132, 255];
                    let t = (0.75 * s).max(1.0);
                    fill(pixmap, left + cx * s, top, cw * s, t, b);
                    fill(pixmap, left + cx * s, top + rh * s - t, cw * s, t, b);
                    fill(pixmap, left + cx * s, top, t, rh * s, b);
                    fill(pixmap, left + (cx + cw) * s - t, top, t, rh * s, b);
                }
            }
            // A tabela inteira selecionada (de fora): um véu por cima.
            if let Some((a, b)) = sel
                && a <= Pos::new(placed.para, 0) && b >= Pos::new(placed.para, line.end) {
                    let mut c = view.selection_color;
                    c[3] = 90;
                    fill(pixmap, left + line.x0 * s, top, (line.x1 - line.x0) * s, rh * s, c);
                }
            continue;
        }
        // Imagem.
        if let Some(img) = &para.image {
            let w = (line.x1 - line.x0) * s;
            let h = line.baseline * s;
            let sx = w / img.pixmap.width() as f32;
            let sy = h / img.pixmap.height() as f32;
            let paint = PixmapPaint { quality: tiny_skia::FilterQuality::Bilinear, ..Default::default() };
            pixmap.draw_pixmap(0, 0, img.pixmap.as_ref().as_ref(), &paint, Transform::from_row(sx, 0.0, 0.0, sy, left + line.x0 * s, top), None);
            if let Some((a, b)) = sel
                && a <= Pos::new(placed.para, 0) && b >= Pos::new(placed.para, line.end) {
                    fill(pixmap, left + line.x0 * s, top, w, h, view.selection_color);
                }
            continue;
        }
        // Realce (cor de fundo do texto).
        for g in &line.glyphs {
            if let Some(hl) = styles.get(g.span).and_then(|st| st.highlight) {
                fill(pixmap, left + g.x * s, top, g.width * s + 0.5, line.height * s, [hl[0], hl[1], hl[2], 255]);
            }
        }
        // Marcador da lista.
        if placed.line == 0
            && let Some((glyphs, mx)) = &layout.paras[placed.para].marker {
                let c = styles.first().map_or([0, 0, 0], |st| st.color);
                draw_glyphs(text, pixmap, glyphs, (left + mx * s, baseline), s, |_| [c[0], c[1], c[2], 255]);
            }
        let style_of = |g: &Glyph| styles.get(g.span).cloned().unwrap_or_default();
        draw_glyphs(text, pixmap, &line.glyphs, (left, baseline), s, |g| {
            let c = style_of(g).color;
            [c[0], c[1], c[2], 255]
        });
        // Pontilhado do sumário.
        if let Some((a, b)) = line.leader {
            let c = styles.first().map_or([0, 0, 0], |st| st.color);
            let dot = (1.2 * s).max(1.0);
            let mut x = a + 4.0;
            while x < b - 4.0 {
                fill(pixmap, left + x * s, baseline - dot, dot, dot, [c[0], c[1], c[2], 255]);
                x += 4.0;
            }
        }
        // Ortografia: ondulado vermelho embaixo das palavras erradas desta linha.
        if let Some(words) = view.misspelled.and_then(|m| m.get(&placed.para)) {
            let x_at = |off: usize| {
                let mut x = line.x0;
                for &(o, cx) in &line.carets {
                    if o <= off {
                        x = cx;
                    }
                }
                x
            };
            let dot = (1.3 * s).max(1.5);
            for &(a, b) in words.iter().filter(|(a, b)| *b > line.start && *a < line.end) {
                let (xa, xb) = (x_at(a.max(line.start)), x_at(b.min(line.end)));
                let y = baseline + (line.descent * 0.55) * s;
                let mut x = left + xa * s;
                let mut up = false;
                while x < left + xb * s {
                    fill(pixmap, x, y + if up { 0.0 } else { dot }, dot, dot, [230, 40, 40, 255]);
                    x += dot;
                    up = !up;
                }
            }
        }
        // Sublinhado e tachado.
        for g in &line.glyphs {
            let st = style_of(g);
            let c = [st.color[0], st.color[1], st.color[2], 255];
            let thick = (st.size * 0.06 * s).max(1.0);
            if st.underline || st.link.is_some() {
                fill(pixmap, left + g.x * s, baseline + st.size * 0.12 * s, g.width * s + 0.5, thick, c);
            }
            if st.strike {
                fill(pixmap, left + g.x * s, baseline - st.size * 0.28 * s, g.width * s + 0.5, thick, c);
            }
        }
    }
    // Notas de rodapé no pé da página: o traço e as notas.
    let notes_h = layout.notes_height(view.page);
    if notes_h > 0.0 {
        let mut y = doc.page.height - doc.page.margins[2] - notes_h;
        fill(pixmap, left, view.origin.1 + (y + 4.0) * s, doc.page.content_width() / 3.0 * s, (0.5 * s).max(1.0), [0, 0, 0, 255]);
        y += super::layout::NOTE_SEPARATOR;
        for id in &layout.page_notes[view.page] {
            let Some((_, boxed, styles)) = layout.note_boxes.get(id) else {
                continue;
            };
            for line in &boxed.lines {
                let base = view.origin.1 + (y + line.baseline) * s;
                draw_glyphs(text, pixmap, &line.glyphs, (left, base), s, |g| {
                    let c = styles.get(g.span).map_or([0, 0, 0], |st| st.color);
                    [c[0], c[1], c[2], 255]
                });
                y += line.height;
            }
        }
    }
    // Cursor.
    if let (Some(p), None) = (view.caret, sel)
        && let Some(placed) = layout.line_of(p).filter(|pl| pl.page == view.page) {
            let line = &layout.paras[p.para].lines[placed.line];
            let x = left + layout.caret_x(p) * s;
            let top = view.origin.1 + placed.y * s;
            let h = (line.ascent + line.descent) * s;
            fill(pixmap, x.round(), top + (line.baseline - line.ascent) * s, (1.5 * s / 1.333).max(1.0), h, view.caret_color);
        }
    // Cabeçalho, rodapé e número da página.
    let page = &doc.page;
    // Cabeçalho, rodapé e números: só a fonte do documento (sem o negrito de um título).
    let family = doc.paras.iter().find(|p| p.style.name.is_empty() && !p.text.is_empty()).or(doc.paras.first()).map(|p| p.spans[0].style.family.clone()).unwrap_or_default();
    let small = CharStyle { size: 10.0, color: [80, 80, 80], family, ..CharStyle::default() };
    let content_w = page.content_width();
    let mut put = |text_s: &str, y_pt: f32, align: f32, style: &CharStyle| {
        if text_s.is_empty() {
            return;
        }
        let (glyphs, w) = layouter.shape_simple(text.font_system(), text_s, style);
        let x = page.margins[3] + (content_w - w) * align;
        draw_glyphs(text, pixmap, &glyphs, (view.origin.0 + x * s, view.origin.1 + y_pt * s), s, |_| [style.color[0], style.color[1], style.color[2], 255]);
    };
    let header_y = (page.margins[0] * 0.5).max(18.0);
    let footer_y = page.height - (page.margins[2] * 0.5).max(18.0) + 8.0;
    put(&page.header, header_y, 0.5, &small);
    put(&page.footer, footer_y, 0.5, &small);
    let n = view.page + 1;
    if page.numbers.show && n >= page.numbers.show_from {
        let style = CharStyle { size: 10.0, color: [0, 0, 0], ..small.clone() };
        // ABNT: no canto de cima à direita, a 2 cm da borda.
        match page.numbers.position {
            NumberPosition::TopRight => put(&n.to_string(), super::cm(2.0), 1.0, &style),
            NumberPosition::BottomCenter => put(&n.to_string(), footer_y, 0.5, &style),
            NumberPosition::BottomRight => put(&n.to_string(), footer_y, 1.0, &style),
        }
    }
}
