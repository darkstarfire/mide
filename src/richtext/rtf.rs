//! RTF (o formato nativo do TextEdit e do WordPad): ler e gravar, escrito aqui.
//!
//! Grava o que o documento tem (fontes, cores, negrito, itálico, sublinhado, tachado,
//! realce, sobrescrito, alinhamento, recuos, entrelinha, espaços, quebras de página,
//! imagens PNG/JPEG, papel e margens, cabeçalho e rodapé). Listas, nome do estilo e
//! numeração das páginas vão em destinos próprios (`{\*\mide...}`), que os outros
//! programas ignoram. Lê o RTF do TextEdit, do WordPad, do Word e do LibreOffice (o que
//! não entende, pula).

use std::rc::Rc;

use super::{Align, Baseline, CharStyle, Doc, Image, ListKind, NumberPosition, PageSetup, ParaStyle, Paragraph, LINE_BREAK, OBJECT};

fn twips(pt: f32) -> i32 {
    (pt * 20.0).round() as i32
}

fn escape(out: &mut String, text: &str) {
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '{' => out.push_str("\\{"),
            '}' => out.push_str("\\}"),
            '\t' => out.push_str("\\tab "),
            LINE_BREAK => out.push_str("\\line "),
            OBJECT => {}
            c if (c as u32) < 128 => out.push(c),
            c => {
                let mut buf = [0u16; 2];
                for unit in c.encode_utf16(&mut buf) {
                    out.push_str(&format!("\\u{}?", *unit as i16));
                }
            }
        }
    }
}

/// Grava o documento em RTF.
pub fn write(doc: &Doc) -> String {
    let mut fonts: Vec<String> = Vec::new();
    let mut colors: Vec<[u8; 3]> = Vec::new();
    let font = |name: &str, fonts: &mut Vec<String>| match fonts.iter().position(|f| f == name) {
        Some(i) => i,
        None => {
            fonts.push(name.to_string());
            fonts.len() - 1
        }
    };
    let color = |c: [u8; 3], colors: &mut Vec<[u8; 3]>| {
        // Cor 0 é a "automática"; as nossas começam em 1.
        1 + match colors.iter().position(|x| *x == c) {
            Some(i) => i,
            None => {
                colors.push(c);
                colors.len() - 1
            }
        }
    };
    let mut body = String::new();
    let page = &doc.page;
    for p in &doc.paras {
        let st = &p.style;
        body.push_str("\\pard");
        body.push_str(match st.align {
            Align::Left => "\\ql",
            Align::Center => "\\qc",
            Align::Right => "\\qr",
            Align::Justify => "\\qj",
        });
        for (word, v) in [("fi", st.first_indent), ("li", st.left_indent), ("ri", st.right_indent), ("sb", st.space_before), ("sa", st.space_after)] {
            if v != 0.0 {
                body.push_str(&format!("\\{word}{}", twips(v)));
            }
        }
        if (st.line_spacing - 1.0).abs() > 0.001 {
            body.push_str(&format!("\\sl{}\\slmult1", (240.0 * st.line_spacing).round() as i32));
        }
        if st.page_break_before {
            body.push_str("\\pagebb");
        }
        if let Some(kind) = st.list {
            let k = match kind {
                ListKind::Bullet => "bullet",
                ListKind::Dash => "dash",
                ListKind::Number => "number",
                ListKind::Letter => "letter",
            };
            body.push_str(&format!("{{\\*\\midelist {k} {}}}", st.level));
        }
        if !st.name.is_empty() {
            body.push_str("{\\*\\midestyle ");
            escape(&mut body, &st.name);
            body.push('}');
        }
        if let Some(img) = &p.image {
            body.push_str(&format!("{{\\pict\\{}blip\\picwgoal{}\\pichgoal{} ", if img.format == "png" { "png" } else { "jpeg" }, twips(img.width), twips(img.height)));
            for (i, b) in img.data.iter().enumerate() {
                if i % 64 == 0 {
                    body.push('\n');
                }
                body.push_str(&format!("{b:02x}"));
            }
            body.push('}');
        } else {
            for (a, b, s) in p.runs() {
                if a == b && !p.text.is_empty() {
                    continue;
                }
                let f = font(&s.family, &mut fonts);
                body.push_str(&format!("{{\\f{f}\\fs{}", (s.size * 2.0).round() as i32));
                if s.bold {
                    body.push_str("\\b");
                }
                if s.italic {
                    body.push_str("\\i");
                }
                if s.underline {
                    body.push_str("\\ul");
                }
                if s.strike {
                    body.push_str("\\strike");
                }
                match s.baseline {
                    Baseline::Super => body.push_str("\\super"),
                    Baseline::Sub => body.push_str("\\sub"),
                    Baseline::Normal => {}
                }
                if s.color != [0, 0, 0] {
                    body.push_str(&format!("\\cf{}", color(s.color, &mut colors)));
                }
                if let Some(h) = s.highlight {
                    let c = color(h, &mut colors);
                    body.push_str(&format!("\\cb{c}\\highlight{c}"));
                }
                body.push(' ');
                escape(&mut body, &p.text[a..b]);
                body.push('}');
            }
        }
        body.push_str("\\par\n");
    }
    let mut out = String::from("{\\rtf1\\ansi\\ansicpg1252\\deff0\n{\\fonttbl");
    if fonts.is_empty() {
        fonts.push("Helvetica".into());
    }
    for (i, f) in fonts.iter().enumerate() {
        out.push_str(&format!("{{\\f{i}\\fnil "));
        escape(&mut out, f);
        out.push_str(";}");
    }
    out.push_str("}\n{\\colortbl;");
    for c in &colors {
        out.push_str(&format!("\\red{}\\green{}\\blue{};", c[0], c[1], c[2]));
    }
    out.push_str("}\n");
    out.push_str(&format!(
        "\\paperw{}\\paperh{}\\margl{}\\margr{}\\margt{}\\margb{}{}\n",
        twips(page.width),
        twips(page.height),
        twips(page.margins[3]),
        twips(page.margins[1]),
        twips(page.margins[0]),
        twips(page.margins[2]),
        if page.landscape() { "\\landscape" } else { "" }
    ));
    if page.numbers.show {
        let pos = match page.numbers.position {
            NumberPosition::TopRight => "top-right",
            NumberPosition::BottomCenter => "bottom-center",
            NumberPosition::BottomRight => "bottom-right",
        };
        out.push_str(&format!("{{\\*\\midepagenum {pos} {}}}\n", page.numbers.show_from));
    }
    for (word, text) in [("header", &page.header), ("footer", &page.footer)] {
        if !text.is_empty() {
            out.push_str(&format!("{{\\{word}\\pard\\qc "));
            escape(&mut out, text);
            out.push_str("\\par}\n");
        }
    }
    out.push_str(&body);
    out.push('}');
    out
}

/// Windows-1252 nos bytes 0x80–0x9F (o resto é Latin-1).
fn cp1252(b: u8) -> char {
    const HIGH: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8d}', 'Ž', '\u{8f}', '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9d}', 'ž', 'Ÿ',
    ];
    if (0x80..0xa0).contains(&b) { HIGH[(b - 0x80) as usize] } else { b as char }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Dest {
    Text,
    FontTable,
    ColorTable,
    Pict,
    Header,
    Footer,
    MideList,
    MideStyle,
    MidePageNum,
    Skip,
}

#[derive(Clone)]
struct State {
    dest: Dest,
    ch: CharStyle,
    font: i32,
    uc: usize,
}

/// Lê um RTF. `None` se não for RTF.
pub fn read(data: &[u8]) -> Option<Doc> {
    let src = std::str::from_utf8(data).map(str::to_string).unwrap_or_else(|_| data.iter().map(|&b| b as char).collect());
    if !src.trim_start().starts_with("{\\rtf") {
        return None;
    }
    let bytes = src.as_bytes();
    let mut doc = Doc { paras: Vec::new(), page: PageSetup::default() };
    let mut fonts: Vec<(i32, String)> = Vec::new();
    let mut font_name = String::new();
    let mut font_id = 0;
    let mut colors: Vec<[u8; 3]> = Vec::new();
    let mut rgb = [0u8; 3];
    let mut st = State { dest: Dest::Text, ch: CharStyle::default(), font: 0, uc: 1 };
    let mut stack: Vec<State> = Vec::new();
    let mut para = Paragraph::new("", st.ch.clone(), ParaStyle::default());
    para.spans.clear();
    let mut pstyle = ParaStyle::default();
    let mut pict_hex = String::new();
    let mut pict_png = true;
    let mut pict_goal = (0.0f32, 0.0f32);
    let mut pict_px = (0.0f32, 0.0f32);
    let mut side_text = String::new();
    let mut skip_chars = 0usize;
    let mut next_page_break = false;
    let mut cbg: Option<usize> = None;
    let mut i = 0;
    let push_text = |para: &mut Paragraph, s: &str, ch: &CharStyle| {
        if s.is_empty() {
            return;
        }
        para.text.push_str(s);
        match para.spans.last_mut() {
            Some(last) if last.style == *ch => last.len += s.len(),
            _ => para.spans.push(super::Span { len: s.len(), style: ch.clone() }),
        }
    };
    let finish = |doc: &mut Doc, para: &mut Paragraph, pstyle: &ParaStyle, ch: &CharStyle, page_break: &mut bool| {
        let mut p = std::mem::replace(para, Paragraph { text: String::new(), spans: Vec::new(), style: ParaStyle::default(), image: None });
        if p.spans.is_empty() {
            p.spans.push(super::Span { len: 0, style: ch.clone() });
        }
        p.style = pstyle.clone();
        if *page_break {
            p.style.page_break_before = true;
            *page_break = false;
        }
        p.normalize();
        doc.paras.push(p);
    };
    while i < bytes.len() {
        let c = bytes[i];
        match c {
            b'{' => {
                stack.push(st.clone());
                i += 1;
            }
            b'}' => {
                // Fim de um destino: guarda o que foi juntado.
                match st.dest {
                    Dest::Pict => {
                        let data: Vec<u8> = (0..pict_hex.len() / 2).filter_map(|k| u8::from_str_radix(&pict_hex[2 * k..2 * k + 2], 16).ok()).collect();
                        if let Some(mut img) = Image::from_bytes(data, doc.page.content_width()) {
                            if pict_goal.0 > 0.0 && pict_goal.1 > 0.0 {
                                (img.width, img.height) = pict_goal;
                            } else if pict_px.0 > 0.0 {
                                (img.width, img.height) = (pict_px.0 * 0.75, pict_px.1 * 0.75);
                            }
                            let _ = pict_png;
                            // A imagem fica num parágrafo dela.
                            if !para.text.is_empty() {
                                finish(&mut doc, &mut para, &pstyle, &st.ch, &mut next_page_break);
                            }
                            let mut ps = pstyle.clone();
                            if ps.align == Align::Left {
                                ps.align = Align::Center;
                            }
                            para = Paragraph::image(img, st.ch.clone(), ps);
                            let ps2 = para.style.clone();
                            finish(&mut doc, &mut para, &ps2, &st.ch, &mut next_page_break);
                            para.spans.clear();
                        }
                        pict_hex.clear();
                        pict_goal = (0.0, 0.0);
                        pict_px = (0.0, 0.0);
                    }
                    Dest::Header if !side_text.trim().is_empty() => doc.page.header = side_text.trim().to_string(),
                    Dest::Footer if !side_text.trim().is_empty() => doc.page.footer = side_text.trim().to_string(),
                    Dest::MideList => {
                        let mut parts = side_text.split_whitespace();
                        pstyle.list = match parts.next() {
                            Some("bullet") => Some(ListKind::Bullet),
                            Some("dash") => Some(ListKind::Dash),
                            Some("number") => Some(ListKind::Number),
                            Some("letter") => Some(ListKind::Letter),
                            _ => None,
                        };
                        pstyle.level = parts.next().and_then(|l| l.parse().ok()).unwrap_or(0);
                    }
                    Dest::MideStyle => pstyle.name = side_text.trim().to_string(),
                    Dest::MidePageNum => {
                        let mut parts = side_text.split_whitespace();
                        doc.page.numbers.show = true;
                        doc.page.numbers.position = match parts.next() {
                            Some("top-right") => NumberPosition::TopRight,
                            Some("bottom-right") => NumberPosition::BottomRight,
                            _ => NumberPosition::BottomCenter,
                        };
                        doc.page.numbers.show_from = parts.next().and_then(|n| n.parse().ok()).unwrap_or(1);
                    }
                    Dest::FontTable if !font_name.is_empty() => {
                        fonts.push((font_id, font_name.trim().trim_end_matches(';').to_string()));
                        font_name.clear();
                    }
                    _ => {}
                }
                let leaving = st.dest;
                if let Some(prev) = stack.pop() {
                    st = prev;
                }
                if leaving != st.dest && matches!(leaving, Dest::Header | Dest::Footer | Dest::MideList | Dest::MideStyle | Dest::MidePageNum) {
                    side_text.clear();
                }
                i += 1;
            }
            b'\\' => {
                i += 1;
                let Some(&n) = bytes.get(i) else { break };
                if !n.is_ascii_alphabetic() {
                    // Símbolos de controle.
                    i += 1;
                    match n {
                        b'\'' => {
                            let hex = src.get(i..i + 2).unwrap_or("3f");
                            i += 2;
                            if skip_chars > 0 {
                                skip_chars -= 1;
                                continue;
                            }
                            let ch = cp1252(u8::from_str_radix(hex, 16).unwrap_or(b'?'));
                            match st.dest {
                                Dest::Text => push_text(&mut para, &ch.to_string(), &st.ch),
                                Dest::FontTable => font_name.push(ch),
                                Dest::Header | Dest::Footer | Dest::MideStyle => side_text.push(ch),
                                _ => {}
                            }
                        }
                        b'*' => {
                            // Destino opcional: os nossos ou pular.
                            let rest = &src[i..];
                            let word: String = rest.trim_start_matches('\\').chars().take_while(|c| c.is_ascii_alphabetic()).collect();
                            st.dest = match word.as_str() {
                                "midelist" => Dest::MideList,
                                "midestyle" => Dest::MideStyle,
                                "midepagenum" => Dest::MidePageNum,
                                _ => Dest::Skip,
                            };
                            if st.dest != Dest::Skip {
                                side_text.clear();
                                i += 1 + word.len();
                                if bytes.get(i) == Some(&b' ') {
                                    i += 1;
                                }
                            }
                        }
                        b'~' if st.dest == Dest::Text => push_text(&mut para, "\u{a0}", &st.ch),
                        b'_' if st.dest == Dest::Text => push_text(&mut para, "\u{2011}", &st.ch),
                        b'\n' | b'\r' if st.dest == Dest::Text => finish(&mut doc, &mut para, &pstyle, &st.ch, &mut next_page_break),
                        b'\\' | b'{' | b'}' => {
                            let s = (n as char).to_string();
                            match st.dest {
                                Dest::Text => push_text(&mut para, &s, &st.ch),
                                Dest::Header | Dest::Footer | Dest::MideStyle => side_text.push_str(&s),
                                _ => {}
                            }
                        }
                        _ => {}
                    }
                    continue;
                }
                let start = i;
                while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
                    i += 1;
                }
                let word = &src[start..i];
                let num_start = i;
                if i < bytes.len() && (bytes[i] == b'-' || bytes[i].is_ascii_digit()) {
                    i += 1;
                    while i < bytes.len() && bytes[i].is_ascii_digit() {
                        i += 1;
                    }
                }
                let param: Option<i32> = src[num_start..i].parse().ok();
                if bytes.get(i) == Some(&b' ') {
                    i += 1;
                }
                let p = param.unwrap_or(1);
                let pt = |v: i32| v as f32 / 20.0;
                match word {
                    "fonttbl" => st.dest = Dest::FontTable,
                    "colortbl" => {
                        st.dest = Dest::ColorTable;
                        rgb = [0, 0, 0];
                    }
                    "pict" => {
                        st.dest = Dest::Pict;
                        pict_hex.clear();
                    }
                    "pngblip" => pict_png = true,
                    "jpegblip" => pict_png = false,
                    "picwgoal" => pict_goal.0 = pt(p),
                    "pichgoal" => pict_goal.1 = pt(p),
                    "picw" => pict_px.0 = p as f32,
                    "pich" => pict_px.1 = p as f32,
                    "header" | "headerr" | "headerf" => {
                        st.dest = Dest::Header;
                        side_text.clear();
                    }
                    "footer" | "footerr" | "footerf" => {
                        st.dest = Dest::Footer;
                        side_text.clear();
                    }
                    "info" | "stylesheet" | "listtable" | "listoverridetable" | "generator" | "xmlnstbl" | "rsidtbl" | "themedata" | "colorschememapping" | "latentstyles" | "datastore" | "fldinst" | "bkmkstart" | "bkmkend" | "object" | "nonshppict" | "expandedcolortbl" | "listtext" | "pntext" | "pntxtb" | "pntxta" => st.dest = Dest::Skip,
                    "red" if st.dest == Dest::ColorTable => rgb[0] = p as u8,
                    "green" if st.dest == Dest::ColorTable => rgb[1] = p as u8,
                    "blue" if st.dest == Dest::ColorTable => rgb[2] = p as u8,
                    "f" if st.dest == Dest::FontTable => {
                        if !font_name.trim().is_empty() {
                            fonts.push((font_id, font_name.trim().trim_end_matches(';').to_string()));
                        }
                        font_name.clear();
                        font_id = p;
                    }
                    "f" => {
                        st.font = p;
                        if let Some((_, name)) = fonts.iter().find(|(id, _)| *id == p) {
                            st.ch.family = name.clone();
                        }
                    }
                    "fs" => st.ch.size = p as f32 / 2.0,
                    "b" => st.ch.bold = param != Some(0),
                    "i" => st.ch.italic = param != Some(0),
                    "ul" => st.ch.underline = param != Some(0),
                    "ulnone" => st.ch.underline = false,
                    "strike" => st.ch.strike = param != Some(0),
                    "super" => st.ch.baseline = Baseline::Super,
                    "sub" => st.ch.baseline = Baseline::Sub,
                    "nosupersub" => st.ch.baseline = Baseline::Normal,
                    "cf" => st.ch.color = if p == 0 { [0, 0, 0] } else { colors.get(p as usize - 1).copied().unwrap_or([0, 0, 0]) },
                    "cb" | "highlight" | "chcbpat" => {
                        cbg = Some(p as usize);
                        st.ch.highlight = if p == 0 { None } else { colors.get(p as usize - 1).copied() };
                    }
                    "plain" => {
                        let family = st.ch.family.clone();
                        st.ch = CharStyle { family, ..CharStyle::default() };
                    }
                    "pard" => {
                        let keep_list = std::mem::take(&mut pstyle.list);
                        let keep_level = pstyle.level;
                        pstyle = ParaStyle::default();
                        let _ = (keep_list, keep_level);
                    }
                    "ql" => pstyle.align = Align::Left,
                    "qc" => pstyle.align = Align::Center,
                    "qr" => pstyle.align = Align::Right,
                    "qj" => pstyle.align = Align::Justify,
                    "fi" => pstyle.first_indent = pt(p),
                    "li" => pstyle.left_indent = pt(p),
                    "ri" => pstyle.right_indent = pt(p),
                    "sb" => pstyle.space_before = pt(p),
                    "sa" => pstyle.space_after = pt(p),
                    "sl" => {
                        // Com \slmult1, múltiplo de 240; sem ele, altura exata (aprox.).
                        pstyle.line_spacing = if p > 0 { (p as f32 / 240.0).max(0.5) } else { 1.0 };
                    }
                    "pagebb" => pstyle.page_break_before = true,
                    "page" if st.dest == Dest::Text => {
                        if !para.text.is_empty() {
                            finish(&mut doc, &mut para, &pstyle, &st.ch, &mut next_page_break);
                        }
                        next_page_break = true;
                    }
                    "paperw" => doc.page.width = pt(p),
                    "paperh" => doc.page.height = pt(p),
                    "margl" => doc.page.margins[3] = pt(p),
                    "margr" => doc.page.margins[1] = pt(p),
                    "margt" => doc.page.margins[0] = pt(p),
                    "margb" => doc.page.margins[2] = pt(p),
                    "uc" => st.uc = p.max(0) as usize,
                    "u" => {
                        let code = if p < 0 { (p + 65536) as u32 } else { p as u32 };
                        let s: String = if (0xD800..0xDC00).contains(&code) {
                            // Primeira metade de um par: espera a segunda.
                            let mut j = i + st.uc;
                            if src[j.min(src.len())..].starts_with("\\u") {
                                j += 2;
                                let e = src[j..].find(|c: char| !(c.is_ascii_digit() || c == '-')).map_or(src.len(), |k| j + k);
                                let low: i32 = src[j..e].parse().unwrap_or(0);
                                let low = if low < 0 { (low + 65536) as u32 } else { low as u32 };
                                i = e;
                                char::decode_utf16([code as u16, low as u16]).filter_map(Result::ok).collect()
                            } else {
                                String::new()
                            }
                        } else {
                            char::from_u32(code).map(String::from).unwrap_or_default()
                        };
                        skip_chars = st.uc;
                        match st.dest {
                            Dest::Text => push_text(&mut para, &s, &st.ch),
                            Dest::FontTable => font_name.push_str(&s),
                            Dest::Header | Dest::Footer | Dest::MideStyle => side_text.push_str(&s),
                            _ => {}
                        }
                    }
                    "par" | "sect" if st.dest == Dest::Text => finish(&mut doc, &mut para, &pstyle, &st.ch, &mut next_page_break),
                    "par" if matches!(st.dest, Dest::Header | Dest::Footer) => side_text.push(' '),
                    "line" if st.dest == Dest::Text => push_text(&mut para, &LINE_BREAK.to_string(), &st.ch),
                    "tab" if st.dest == Dest::Text => push_text(&mut para, "\t", &st.ch),
                    "emdash" if st.dest == Dest::Text => push_text(&mut para, "—", &st.ch),
                    "endash" if st.dest == Dest::Text => push_text(&mut para, "–", &st.ch),
                    "bullet" if st.dest == Dest::Text => push_text(&mut para, "•", &st.ch),
                    "lquote" if st.dest == Dest::Text => push_text(&mut para, "‘", &st.ch),
                    "rquote" if st.dest == Dest::Text => push_text(&mut para, "’", &st.ch),
                    "ldblquote" if st.dest == Dest::Text => push_text(&mut para, "“", &st.ch),
                    "rdblquote" if st.dest == Dest::Text => push_text(&mut para, "”", &st.ch),
                    _ => {}
                }
            }
            b'\r' | b'\n' => i += 1,
            _ => {
                // Texto até o próximo controle.
                let start = i;
                while i < bytes.len() && !matches!(bytes[i], b'{' | b'}' | b'\\' | b'\r' | b'\n') {
                    i += 1;
                }
                let mut chunk = &src[start..i];
                while skip_chars > 0 && !chunk.is_empty() {
                    let c = chunk.chars().next().expect("não vazio");
                    chunk = &chunk[c.len_utf8()..];
                    skip_chars -= 1;
                }
                match st.dest {
                    Dest::Text => push_text(&mut para, chunk, &st.ch),
                    Dest::FontTable => font_name.push_str(chunk),
                    Dest::ColorTable => {
                        for _ in chunk.matches(';') {
                            colors.push(rgb);
                            rgb = [0, 0, 0];
                        }
                        // O primeiro ";" (cor automática) não entra.
                        if colors.len() == 1 && chunk.trim_start().starts_with(';') && rgb == [0, 0, 0] && start > 0 && src[..start].ends_with("colortbl") {
                            colors.clear();
                        }
                    }
                    Dest::Pict => pict_hex.extend(chunk.chars().filter(|c| c.is_ascii_hexdigit())),
                    Dest::Header | Dest::Footer | Dest::MideList | Dest::MideStyle | Dest::MidePageNum => side_text.push_str(chunk),
                    Dest::Skip => {}
                }
            }
        }
    }
    let _ = cbg;
    if !para.text.is_empty() {
        finish(&mut doc, &mut para, &pstyle, &st.ch, &mut next_page_break);
    }
    if doc.paras.is_empty() {
        doc.paras.push(Paragraph::new("", CharStyle::default(), ParaStyle::default()));
    }
    // Imagens sem dados (falharam) não deixam o objeto solto no texto.
    for p in &mut doc.paras {
        if p.image.is_none() && p.text.contains(OBJECT) {
            let t = p.text.replace(OBJECT, "");
            *p = Paragraph::new(&t, p.spans[0].style.clone(), p.style.clone());
        }
    }
    let _ = Rc::new(());
    Some(doc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::richtext::Pos;

    #[test]
    fn grava_e_le_de_volta() {
        let mut d = Doc::new();
        d.insert_text(Pos::default(), "Título\nOlá, “mundo” — ção {x}\\", &CharStyle { family: "Times New Roman".into(), ..Default::default() });
        d.apply_char(Pos::new(0, 0), Pos::new(0, "Título".len()), |s| {
            s.bold = true;
            s.size = 18.0;
            s.color = [200, 10, 20];
        });
        d.apply_para(Pos::new(0, 0), Pos::new(0, 0), |p| p.align = Align::Center);
        d.apply_para(Pos::new(1, 0), Pos::new(1, 0), |p| {
            p.first_indent = crate::richtext::cm(1.25);
            p.line_spacing = 1.5;
            p.list = Some(ListKind::Number);
        });
        d.page.numbers.show = true;
        d.page.numbers.position = NumberPosition::TopRight;
        d.page.numbers.show_from = 3;
        d.page.header = "Cabeçalho".into();
        let rtf = write(&d);
        let back = read(rtf.as_bytes()).expect("RTF");
        assert_eq!(back.paras.len(), 2, "{:?}", back.paras.iter().map(|p| &p.text).collect::<Vec<_>>());
        assert_eq!(back.paras[0].text, "Título");
        assert_eq!(back.paras[1].text, "Olá, “mundo” — ção {x}\\");
        assert!(back.paras[0].spans[0].style.bold);
        assert_eq!(back.paras[0].spans[0].style.size, 18.0);
        assert_eq!(back.paras[0].spans[0].style.color, [200, 10, 20]);
        assert_eq!(back.paras[0].spans[0].style.family, "Times New Roman");
        assert_eq!(back.paras[0].style.align, Align::Center);
        assert!((back.paras[1].style.line_spacing - 1.5).abs() < 0.01);
        assert!((back.paras[1].style.first_indent - crate::richtext::cm(1.25)).abs() < 0.1);
        assert_eq!(back.paras[1].style.list, Some(ListKind::Number));
        assert_eq!(back.page.numbers.show_from, 3);
        assert_eq!(back.page.header, "Cabeçalho");
        assert!((back.page.width - d.page.width).abs() < 0.1);
    }

    #[test]
    fn le_rtf_do_textedit() {
        let src = r"{\rtf1\ansi\ansicpg1252\cocoartf2822
{\fonttbl\f0\fswiss\fcharset0 Helvetica;\f1\froman\fcharset0 Times-Roman;}
{\colortbl;\red255\green255\blue255;\red251\green2\blue7;}
\paperw11900\paperh16840\margl1440\margr1440\vieww11520\viewh8400\viewkind0
\pard\tx566\tx1133\pardirnatural\qc\partightenfactor0

\f0\fs24 \cf0 Ol\'e1 \b mundo\b0 \
\pard\pardirnatural
\cf2 vermelho \uc0\u8364  euro}";
        let d = read(src.as_bytes()).expect("RTF");
        assert_eq!(d.paras[0].text, "Olá mundo");
        assert_eq!(d.paras[0].style.align, Align::Center);
        assert!(d.paras[0].spans.iter().any(|s| s.style.bold));
        assert_eq!(d.paras[1].text, "vermelho € euro");
        assert_eq!(d.paras[1].spans[0].style.color, [251, 2, 7]);
        assert!((d.page.width - 595.0).abs() < 1.0);
    }
}
