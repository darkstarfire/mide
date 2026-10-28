//! Tradução da interface do mide (item I1): textos em inglês no código, catálogos `.po`
//! em `locales/<idioma>/<área>.po` (um por área, para cada parte do mide ter o seu).
//!
//! Use a macro [`tr!`](crate::tr) para todo texto que aparece na interface:
//! `tr!("New Folder")`, `tr!("Move “{name}” to the Trash?", name = file)`; plural com
//! [`trn`] e contexto com [`trc`]. Os logs e comentários continuam em português.
//! O motor (idioma da sessão, leitor de `.po`, plural) é o do mide-ui.

use std::sync::Once;

pub use mide_ui::kit::i18n::{fill, language, session_language, set_language};

/// Catálogos de `locales/` (gerados pelo `build.rs`).
const CATALOGS: &[(&str, &str)] = include!(concat!(env!("OUT_DIR"), "/catalogs.rs"));

/// Registra os catálogos do mide no motor do mide-ui (uma vez; as macros chamam sozinhas,
/// e os programas chamam no começo para os textos que passam direto pelo kit).
pub fn init() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        for (language, po) in CATALOGS {
            mide_ui::kit::i18n::add_catalog(language, po);
        }
    });
}

/// Texto traduzido (ver [`tr!`](crate::tr)).
pub fn tr(text: &str) -> std::borrow::Cow<'_, str> {
    init();
    mide_ui::kit::tr(text)
}

/// Plural traduzido, com `{n}` trocado pelo número: `trn("{n} item", "{n} items", n)`.
pub fn trn(singular: &str, plural: &str, n: u64) -> String {
    init();
    mide_ui::kit::trn(singular, plural, n)
}

/// Texto com contexto (`msgctxt`), para o mesmo texto em inglês com sentidos diferentes.
pub fn trc<'a>(context: &str, text: &'a str) -> std::borrow::Cow<'a, str> {
    init();
    mide_ui::kit::trc(context, text)
}

/// Texto da interface traduzido, com `{nome}` preenchido:
/// `tr!("Delete")` (um `Cow<str>`) ou `tr!("Delete “{name}”?", name = file)` (um `String`).
#[macro_export]
macro_rules! tr {
    ($text:expr $(,)?) => {
        $crate::i18n::tr($text)
    };
    ($text:expr, $($name:ident = $value:expr),+ $(,)?) => {
        $crate::i18n::fill(&$crate::i18n::tr($text), &[$((stringify!($name), &$value as &dyn ::std::fmt::Display)),+])
    };
}

#[cfg(test)]
mod tests {
    use std::{collections::HashSet, path::Path};

    /// Textos em inglês do código (`tr!("…")`, `trn("…", "…"`, `trc("…", "…")`):
    /// (arquivo, contexto, texto).
    fn msgids(dir: &Path, out: &mut Vec<(String, Option<String>, String)>) {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                msgids(&path, out);
                continue;
            }
            if path.extension().is_none_or(|e| e != "rs") || path.ends_with("i18n.rs") {
                continue;
            }
            let Ok(src) = std::fs::read_to_string(&path) else { continue };
            for marker in ["tr!(", "trn(", "trc("] {
                let mut rest = src.as_str();
                while let Some(i) = rest.find(marker) {
                    // `include_str!(` não é `tr!(`: a marca começa uma palavra.
                    let whole = !rest[..i].ends_with(|c: char| c.is_alphanumeric() || c == '_');
                    rest = &rest[i + marker.len()..];
                    if !whole {
                        continue;
                    }
                    let Some((first, after)) = string_literal(rest) else { continue };
                    let entry = if marker == "trc(" {
                        string_literal(after).map(|(text, _)| (Some(first), text))
                    } else {
                        Some((None, first))
                    };
                    if let Some((context, text)) = entry {
                        out.push((path.display().to_string(), context, text));
                    }
                }
            }
        }
    }

    /// Lê um literal de string no começo de `s` (depois de espaços e vírgula); devolve o
    /// texto (com os escapes desfeitos) e o resto.
    fn string_literal(s: &str) -> Option<(String, &str)> {
        let s = s.trim_start().trim_start_matches(',').trim_start();
        let body = s.strip_prefix('"')?;
        let mut text = String::new();
        let mut chars = body.char_indices();
        while let Some((i, c)) = chars.next() {
            match c {
                '"' => return Some((text, &body[i + 1..])),
                '\\' => match chars.next() {
                    Some((_, 'n')) => text.push('\n'),
                    Some((_, 't')) => text.push('\t'),
                    Some((_, '\n')) => {
                        // Continuação de linha do Rust: pula os espaços seguintes.
                        while chars.clone().next().is_some_and(|(_, c)| c.is_whitespace()) {
                            chars.next();
                        }
                    }
                    Some((_, other)) => text.push(other),
                    None => return None,
                },
                _ => text.push(c),
            }
        }
        None
    }

    /// Todo texto que passa por `tr!`/`trn`/`trc` no mide tem tradução para o pt-BR
    /// (nos catálogos do mide ou do mide-ui).
    #[test]
    fn todo_texto_tem_traducao_pt_br() {
        super::init();
        mide_ui::kit::i18n::set_language("pt_BR");
        let mut found = Vec::new();
        msgids(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut found);
        let mut missing: Vec<String> = Vec::new();
        let mut seen = HashSet::new();
        for (file, context, text) in found {
            // Textos sem letras ("…", "+") não precisam de tradução.
            if !seen.insert((context.clone(), text.clone())) || !text.chars().any(char::is_alphabetic) {
                continue;
            }
            if !mide_ui::kit::i18n::is_translated(context.as_deref(), &text) {
                missing.push(format!("{file}: {text:?}"));
            }
        }
        assert!(missing.is_empty(), "sem tradução em locales/pt_BR/*.po:\n{}", missing.join("\n"));
    }
}
