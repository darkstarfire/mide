//! Cores do código (realce de sintaxe) do `mide-code`: um leitor próprio por família
//! de linguagem (`lang::Family`), que marca comentários, textos entre aspas, números,
//! palavras-chave, tipos, funções, chaves de dados e marcações. Rápido o bastante para
//! refazer o arquivo inteiro a cada mudança (arquivos de código comuns).

use std::ops::Range;

use super::lang::{Family, Language};

/// Tipo de trecho (cada um com a sua cor no tema).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Comment,
    String,
    Number,
    Keyword,
    Type,
    Function,
    /// Chave de um dado (JSON, TOML, YAML, INI) ou atributo de uma marcação.
    Key,
    /// Nome de uma marcação (`<div>`), seção (`[pacote]`) ou título do Markdown.
    Tag,
    /// Ênfase do Markdown e macros (`println!`, `@decorador`, `#include`).
    Special,
}

/// Cor de um tipo de trecho, no tema claro ou escuro (RGBA).
pub fn color(kind: Kind, dark: bool) -> [u8; 4] {
    let (light, night) = match kind {
        Kind::Comment => ([110, 120, 130], [125, 135, 145]),
        Kind::String => ([196, 26, 22], [252, 106, 93]),
        Kind::Number => ([28, 0, 207], [208, 191, 105]),
        Kind::Keyword => ([155, 35, 147], [252, 95, 163]),
        Kind::Type => ([11, 79, 121], [93, 216, 255]),
        Kind::Function => ([50, 109, 116], [103, 183, 164]),
        Kind::Key => ([100, 56, 32], [161, 103, 230]),
        Kind::Tag => ([15, 104, 160], [65, 161, 192]),
        Kind::Special => ([100, 56, 32], [253, 143, 63]),
    };
    let c = if dark { night } else { light };
    [c[0], c[1], c[2], 255]
}

/// Trechos coloridos do texto inteiro, em ordem e sem sobreposição.
pub fn highlight(text: &str, lang: &Language) -> Vec<(Range<usize>, Kind)> {
    match lang.family {
        Family::Plain => Vec::new(),
        Family::CLike => code(text, lang, &["//"], Some(("/*", "*/"))),
        Family::Hash => code(text, lang, &["#"], None),
        Family::Sql => code(text, lang, &["--"], Some(("/*", "*/"))),
        Family::Lua => code(text, lang, &["--"], Some(("--[[", "]]"))),
        Family::Data => data(text, lang),
        Family::Markup => markup(text),
        Family::Markdown => markdown(text),
    }
}

fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_' || c == '$'
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

/// Linguagens de programação: comentários, textos, números e palavras.
fn code(text: &str, lang: &Language, line_comments: &[&str], block: Option<(&str, &str)>) -> Vec<(Range<usize>, Kind)> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    let rust = lang.id == "rust";
    let css = lang.id == "css";
    // `#` em C/C++ é diretiva, não comentário; em CSS não há comentário de linha.
    let line_comments: Vec<&str> = if css { Vec::new() } else { line_comments.to_vec() };
    while i < text.len() {
        let rest = &text[i..];
        // Comentário de bloco.
        if let Some((open, close)) = block {
            if rest.starts_with(open) {
                let end = rest[open.len()..].find(close).map_or(text.len(), |e| i + open.len() + e + close.len());
                out.push((i..end, Kind::Comment));
                i = end;
                continue;
            }
        }
        // Comentário de linha (num shell, `#` só começa comentário no começo de uma
        // palavra: `$#` e `a#b` não são).
        if let Some(c) = line_comments.iter().find(|c| rest.starts_with(**c)) {
            let starts_word = *c != "#" || i == 0 || bytes[i - 1].is_ascii_whitespace() || matches!(bytes[i - 1], b';' | b'(');
            if starts_word && !(lang.family == Family::CLike && *c == "#") {
                let end = rest.find('\n').map_or(text.len(), |e| i + e);
                out.push((i..end, Kind::Comment));
                i = end;
                continue;
            }
        }
        let c = rest.chars().next().unwrap_or(' ');
        // Diretivas do C (`#include`) e decoradores (`@app.route`).
        if (c == '#' && lang.family == Family::CLike && !css) || (c == '@' && lang.id != "css") {
            let end = rest[1..].find(|ch: char| !is_ident(ch) && ch != '.').map_or(text.len(), |e| i + 1 + e);
            if end > i + 1 {
                out.push((i..end, Kind::Special));
                i = end;
                continue;
            }
        }
        // Textos entre aspas (com `\` escapando), e as três aspas do Python.
        if c == '"' || c == '`' || (c == '\'' && !rust) || (c == '\'' && rust && is_char_literal(rest)) {
            let triple: String = std::iter::repeat_n(c, 3).collect();
            let end = if lang.family == Family::Hash && rest.starts_with(&triple) {
                rest[3..].find(&triple).map_or(text.len(), |e| i + 3 + e + 3)
            } else {
                string_end(text, i, c)
            };
            out.push((i..end, Kind::String));
            i = end;
            continue;
        }
        // Textos crus do Rust: r"..." e r#"..."#.
        if rust && (rest.starts_with("r\"") || rest.starts_with("r#\"")) {
            let hashes = rest[1..].chars().take_while(|ch| *ch == '#').count();
            let close: String = std::iter::once('"').chain(std::iter::repeat_n('#', hashes)).collect();
            let start = 1 + hashes + 1;
            let end = rest[start..].find(&close).map_or(text.len(), |e| i + start + e + close.len());
            out.push((i..end, Kind::String));
            i = end;
            continue;
        }
        if c.is_ascii_digit() {
            let end = rest.find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '.' || ch == '_')).map_or(text.len(), |e| i + e);
            out.push((i..end, Kind::Number));
            i = end;
            continue;
        }
        if is_ident_start(c) || (css && c == '-') {
            let end = rest.find(|ch: char| !(is_ident(ch) || (css && ch == '-'))).map_or(text.len(), |e| i + e);
            let word = &text[i..end];
            let after = text[end..].trim_start_matches([' ', '\t']);
            let kind = if lang.keywords.contains(&word) || (lang.family == Family::Sql && lang.keywords.contains(&word.to_lowercase().as_str())) {
                Some(Kind::Keyword)
            } else if rust && after.starts_with('!') && !after.starts_with("!=") {
                Some(Kind::Special)
            } else if lang.types.contains(&word) || (lang.family == Family::CLike && word.chars().next().is_some_and(char::is_uppercase) && word.len() > 1) {
                Some(Kind::Type)
            } else if after.starts_with('(') && lang.family != Family::Sql {
                Some(Kind::Function)
            } else if css && after.starts_with(':') {
                Some(Kind::Key)
            } else {
                None
            };
            if let Some(kind) = kind {
                out.push((i..end, kind));
            }
            i = end;
            continue;
        }
        i += c.len_utf8();
    }
    out
}

/// `'a'`, `'\n'`, `'\u{1F600}'` são caracteres; `'a` (tempo de vida do Rust) não.
fn is_char_literal(rest: &str) -> bool {
    let mut chars = rest.chars().skip(1);
    match chars.next() {
        Some('\\') => rest[2..].find('\'').is_some_and(|e| e <= 10),
        Some(_) => chars.next() == Some('\''),
        None => false,
    }
}

/// Fim de um texto entre aspas que começa em `start` (depois da aspa de fechamento, ou
/// no fim da linha se não fechar, menos com crase, que pode ter várias linhas).
fn string_end(text: &str, start: usize, quote: char) -> usize {
    let mut escaped = false;
    for (off, ch) in text[start + quote.len_utf8()..].char_indices() {
        let at = start + quote.len_utf8() + off;
        if escaped {
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == quote {
            return at + ch.len_utf8();
        } else if ch == '\n' && quote != '`' && quote != '"' {
            return at;
        }
    }
    text.len()
}

/// Dados: JSON, TOML, YAML e INI (chaves, seções, textos, números e comentários).
fn data(text: &str, lang: &Language) -> Vec<(Range<usize>, Kind)> {
    let mut out = Vec::new();
    let json = lang.id == "json";
    let mut line_start = 0;
    for line in text.split_inclusive('\n') {
        let base = line_start;
        line_start += line.len();
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        // Seção: [pacote] (TOML, INI).
        if !json && trimmed.starts_with('[') {
            let end = trimmed.find(']').map_or(trimmed.trim_end().len(), |e| e + 1);
            out.push((base + indent..base + indent + end, Kind::Tag));
            continue;
        }
        if !lang.comment.is_empty() && (trimmed.starts_with(lang.comment) || (lang.id == "ini" && trimmed.starts_with(';'))) {
            out.push((base + indent..base + line.trim_end().len(), Kind::Comment));
            continue;
        }
        // Chave antes de `=` ou `:` (sem aspas no TOML, YAML e INI).
        if !json {
            let sep = if lang.id == "yaml" { trimmed.find(": ").or_else(|| trimmed.trim_end().strip_suffix(':').map(|s| s.len())) } else { trimmed.find('=') };
            if let Some(k) = sep {
                let key = trimmed[..k].trim_end().trim_start_matches("- ");
                if !key.is_empty() && !key.starts_with('"') && !key.starts_with('\'') {
                    let ks = base + indent + (trimmed.len() - trimmed.trim_start_matches("- ").len());
                    out.push((ks..ks + key.len(), Kind::Key));
                }
            }
        }
        // Textos, números e as palavras true/false/null no resto da linha.
        let mut i = base + indent;
        let end = base + line.len();
        while i < end {
            let rest = &text[i..end];
            let c = rest.chars().next().unwrap_or(' ');
            if c == '"' || c == '\'' {
                let e = string_end(text, i, c).min(end);
                // No JSON, um texto seguido de `:` é uma chave.
                let after = text[e..end].trim_start();
                let kind = if json && after.starts_with(':') { Kind::Key } else { Kind::String };
                if !out.iter().any(|(r, _): &(Range<usize>, Kind)| r.start <= i && i < r.end) {
                    out.push((i..e, kind));
                }
                i = e.max(i + 1);
                continue;
            }
            if (c.is_ascii_digit() || (c == '-' && rest[1..].starts_with(|d: char| d.is_ascii_digit()))) && (i == base || !is_ident(text[..i].chars().last().unwrap_or(' '))) {
                let e = rest[1..].find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '.' || ch == '_' || ch == '-' || ch == ':')).map_or(end, |x| i + 1 + x);
                if !out.iter().any(|(r, _)| r.start <= i && i < r.end) {
                    out.push((i..e, Kind::Number));
                }
                i = e;
                continue;
            }
            if is_ident_start(c) {
                let e = rest.find(|ch: char| !is_ident(ch)).map_or(end, |x| i + x);
                if lang.keywords.contains(&&text[i..e]) && !out.iter().any(|(r, _)| r.start <= i && i < r.end) {
                    out.push((i..e, Kind::Keyword));
                }
                i = e;
                continue;
            }
            if c == '#' && !json && lang.comment == "#" {
                // Comentário no fim da linha.
                out.push((i..base + line.trim_end().len(), Kind::Comment));
                break;
            }
            i += c.len_utf8();
        }
    }
    out.sort_by_key(|(r, _)| r.start);
    out
}

/// HTML e XML: comentários, nomes das marcações, atributos e valores.
fn markup(text: &str) -> Vec<(Range<usize>, Kind)> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(off) = text[i..].find('<') {
        let at = i + off;
        if text[at..].starts_with("<!--") {
            let end = text[at..].find("-->").map_or(text.len(), |e| at + e + 3);
            out.push((at..end, Kind::Comment));
            i = end;
            continue;
        }
        let name_start = at + 1 + text[at + 1..].chars().take_while(|c| *c == '/' || *c == '!' || *c == '?').count();
        let name_end = text[name_start..].find(|c: char| c.is_whitespace() || c == '>' || c == '/').map_or(text.len(), |e| name_start + e);
        if name_end > name_start {
            out.push((name_start..name_end, Kind::Tag));
        }
        let close = text[name_end..].find('>').map_or(text.len(), |e| name_end + e);
        // Atributos: nome="valor".
        let mut j = name_end;
        while j < close {
            let rest = &text[j..close];
            let c = rest.chars().next().unwrap_or(' ');
            if c == '"' || c == '\'' {
                let e = string_end(text, j, c).min(close);
                out.push((j..e, Kind::String));
                j = e.max(j + 1);
            } else if is_ident_start(c) {
                let e = rest.find(|ch: char| !(is_ident(ch) || ch == '-' || ch == ':')).map_or(close, |x| j + x);
                out.push((j..e, Kind::Key));
                j = e;
            } else {
                j += c.len_utf8();
            }
        }
        i = (close + 1).min(text.len());
        if i >= text.len() {
            break;
        }
    }
    out
}

/// Markdown: títulos, blocos de código, código no meio do texto, ênfase e links.
fn markdown(text: &str) -> Vec<(Range<usize>, Kind)> {
    let mut out = Vec::new();
    let mut in_fence: Option<usize> = None;
    let mut pos = 0;
    for line in text.split_inclusive('\n') {
        let base = pos;
        pos += line.len();
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            match in_fence.take() {
                Some(start) => out.push((start..base + line.trim_end().len(), Kind::String)),
                None => in_fence = Some(base),
            }
            continue;
        }
        if in_fence.is_some() {
            continue;
        }
        if trimmed.starts_with('#') {
            out.push((base..base + line.trim_end().len(), Kind::Tag));
            continue;
        }
        if trimmed.starts_with("> ") {
            out.push((base..base + line.trim_end().len(), Kind::Comment));
            continue;
        }
        let bytes = line.as_bytes();
        let mut i = 0;
        while i < line.len() {
            let rest = &line[i..];
            if rest.starts_with('`') {
                let e = rest[1..].find('`').map_or(line.trim_end().len(), |x| i + 1 + x + 1);
                out.push((base + i..base + e, Kind::String));
                i = e;
            } else if rest.starts_with("**") || rest.starts_with("__") {
                let mark = &rest[..2];
                match rest[2..].find(mark) {
                    Some(x) => {
                        out.push((base + i..base + i + 2 + x + 2, Kind::Special));
                        i += 2 + x + 2;
                    }
                    None => i += 2,
                }
            } else if rest.starts_with('[') {
                match (rest.find("]("), rest.find(')')) {
                    (Some(a), Some(b)) if a < b => {
                        out.push((base + i..base + i + a + 1, Kind::Key));
                        out.push((base + i + a + 1..base + i + b + 1, Kind::Type));
                        i += b + 1;
                    }
                    _ => i += 1,
                }
            } else if (i == 0 || bytes[i - 1] == b' ') && (rest.starts_with("- ") || rest.starts_with("* ") || rest.starts_with("+ ")) && line[..i].trim().is_empty() {
                out.push((base + i..base + i + 1, Kind::Keyword));
                i += 1;
            } else {
                i += rest.chars().next().map_or(1, char::len_utf8);
            }
        }
    }
    if let Some(start) = in_fence {
        out.push((start..text.len(), Kind::String));
    }
    out.sort_by_key(|(r, _)| r.start);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code::lang::by_id;

    fn kinds(text: &str, lang: &str) -> Vec<(String, Kind)> {
        highlight(text, by_id(lang).unwrap()).into_iter().map(|(r, k)| (text[r].to_string(), k)).collect()
    }

    #[test]
    fn rust() {
        let k = kinds("fn main() { let s = \"oi\"; // nada\n    println!(\"{}\", 42); let c = 'x'; let r: &'a str; }", "rust");
        assert!(k.contains(&("fn".into(), Kind::Keyword)));
        assert!(k.contains(&("main".into(), Kind::Function)));
        assert!(k.contains(&("\"oi\"".into(), Kind::String)));
        assert!(k.contains(&("// nada".into(), Kind::Comment)));
        assert!(k.contains(&("println".into(), Kind::Special)));
        assert!(k.contains(&("42".into(), Kind::Number)));
        assert!(k.contains(&("'x'".into(), Kind::String)));
        assert!(k.contains(&("str".into(), Kind::Type)));
        // O tempo de vida 'a não vira texto.
        assert!(!k.iter().any(|(s, _)| s.starts_with("'a")));
    }

    #[test]
    fn python_e_shell() {
        let k = kinds("def f(x):\n    \"\"\"doc\n    longa\"\"\"\n    return x # fim\n", "python");
        assert!(k.contains(&("def".into(), Kind::Keyword)));
        assert!(k.iter().any(|(s, k)| s.starts_with("\"\"\"doc") && *k == Kind::String));
        assert!(k.contains(&("# fim".into(), Kind::Comment)));
        let k = kinds("echo $# \"a#b\" # comentário\n", "shellscript");
        assert!(k.contains(&("# comentário".into(), Kind::Comment)));
        assert_eq!(k.iter().filter(|(_, k)| *k == Kind::Comment).count(), 1);
    }

    #[test]
    fn dados_e_marcacao() {
        let k = kinds("[package]\nname = \"mide\" # nome\nversion = 1\n", "toml");
        assert!(k.contains(&("[package]".into(), Kind::Tag)));
        assert!(k.contains(&("name".into(), Kind::Key)));
        assert!(k.contains(&("\"mide\"".into(), Kind::String)));
        assert!(k.contains(&("# nome".into(), Kind::Comment)));
        let k = kinds("{\"a\": [1, true, \"b\"]}", "json");
        assert!(k.contains(&("\"a\"".into(), Kind::Key)));
        assert!(k.contains(&("\"b\"".into(), Kind::String)));
        assert!(k.contains(&("true".into(), Kind::Keyword)));
        let k = kinds("<div class=\"x\"><!-- c --></div>", "html");
        assert!(k.contains(&("div".into(), Kind::Tag)));
        assert!(k.contains(&("class".into(), Kind::Key)));
        assert!(k.contains(&("<!-- c -->".into(), Kind::Comment)));
        let k = kinds("# Título\ntexto `code` e **forte**\n```\nbloco\n```\n", "markdown");
        assert!(k.contains(&("# Título".into(), Kind::Tag)));
        assert!(k.contains(&("`code`".into(), Kind::String)));
        assert!(k.contains(&("**forte**".into(), Kind::Special)));
        assert!(k.iter().any(|(s, k)| s.contains("bloco") && *k == Kind::String));
    }

    #[test]
    fn trechos_em_ordem_e_dentro_do_texto() {
        for lang in crate::code::lang::LANGUAGES {
            let text = "x = \"não fecha\n# comentário çã\n<a b='c'>\n{\"k\": 1}\n";
            let mut last = 0;
            for (r, _) in highlight(text, lang) {
                assert!(r.start >= last && r.end <= text.len() && text.is_char_boundary(r.start) && text.is_char_boundary(r.end), "{} {r:?}", lang.id);
                last = r.end;
            }
        }
    }
}
