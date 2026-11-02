//! Realce por tree-sitter com as gramáticas das extensões do Zed (`grammars/*.wasm`,
//! já compiladas para WebAssembly) e as consultas delas (`languages/<x>/highlights.scm`).
//! Linguagem sem gramática nas extensões fica com o realce próprio (`highlight`).

use std::{collections::HashMap, ops::Range, path::PathBuf};

use tree_sitter::{wasmtime::Engine, Language, Parser, Query, QueryCursor, StreamingIterator, WasmStore};

use crate::code::highlight::Kind;
use crate::code::lang::Language as Lang;

/// Uma gramática carregada: a linguagem do tree-sitter, a consulta de realce e o tipo
/// de realce de cada captura.
struct Grammar {
    language: Language,
    query: Query,
    kinds: Vec<Option<Kind>>,
}

/// Onde estão a gramática e a consulta de uma linguagem nas extensões.
pub fn find_grammar(lang: &Lang) -> Option<(String, PathBuf, PathBuf)> {
    for ext in crate::code::extensions::installed() {
        for dir in ext.language_dirs() {
            let Some(cfg) = std::fs::read_to_string(dir.join("config.toml")).ok().and_then(|t| toml::from_str::<toml::Value>(&t).ok()) else {
                continue;
            };
            let name = cfg.get("name").and_then(|v| v.as_str()).unwrap_or_default();
            let same = name.eq_ignore_ascii_case(lang.name) || name.to_lowercase().replace(' ', "") == lang.id;
            let Some(grammar) = cfg.get("grammar").and_then(|v| v.as_str()).filter(|_| same) else {
                continue;
            };
            let wasm = ext.dir.join("grammars").join(format!("{grammar}.wasm"));
            let query = dir.join("highlights.scm");
            if wasm.exists() && query.exists() {
                return Some((grammar.to_string(), wasm, query));
            }
        }
    }
    None
}

/// O tipo de realce do mide para um nome de captura do tree-sitter (`keyword`,
/// `function.method`, `string.special`...).
pub fn kind_of(capture: &str) -> Option<Kind> {
    let first = capture.split('.').next().unwrap_or(capture);
    Some(match first {
        "comment" => Kind::Comment,
        "string" | "character" | "escape" => Kind::String,
        "number" | "float" | "boolean" | "constant" => Kind::Number,
        "keyword" | "conditional" | "repeat" | "include" | "exception" | "storageclass" => Kind::Keyword,
        "type" | "constructor" | "namespace" | "module" => Kind::Type,
        "function" | "method" => Kind::Function,
        "property" | "attribute" | "field" => Kind::Key,
        "tag" | "title" | "label" => Kind::Tag,
        "preproc" | "embedded" | "emphasis" | "link_uri" | "link_text" => Kind::Special,
        "variable" if capture.contains("special") || capture.contains("builtin") => Kind::Special,
        "punctuation" if capture.contains("special") => Kind::Special,
        _ => return None,
    })
}

/// O realce por tree-sitter de uma janela (o motor de WebAssembly e as gramáticas
/// carregadas, uma vez por linguagem).
pub struct Syntax {
    parser: Option<Parser>,
    engine: Option<Engine>,
    grammars: HashMap<&'static str, Option<Grammar>>,
}

impl Default for Syntax {
    fn default() -> Self {
        Self::new()
    }
}

impl Syntax {
    pub fn new() -> Self {
        Self { parser: None, engine: None, grammars: HashMap::new() }
    }

    /// O motor e o analisador com o armazém de WebAssembly (na primeira vez).
    fn parser(&mut self) -> Option<&mut Parser> {
        if self.parser.is_none() {
            let engine = self.engine.get_or_insert_with(Engine::default).clone();
            let store = WasmStore::new(&engine).ok()?;
            let mut parser = Parser::new();
            parser.set_wasm_store(store).ok()?;
            self.parser = Some(parser);
        }
        self.parser.as_mut()
    }

    /// Carrega a gramática da linguagem (se as extensões têm). `false`: não há.
    fn load(&mut self, lang: &'static Lang) -> bool {
        if let Some(g) = self.grammars.get(lang.id) {
            return g.is_some();
        }
        let loaded = find_grammar(lang).and_then(|(name, wasm, query_path)| {
            let bytes = std::fs::read(&wasm).ok()?;
            let query_text = std::fs::read_to_string(&query_path).ok()?;
            let g = self.load_grammar(&name, &bytes, &query_text);
            if g.is_some() {
                tracing::info!("tree-sitter: gramática {name} ({})", wasm.display());
            }
            g
        });
        let ok = loaded.is_some();
        self.grammars.insert(lang.id, loaded);
        ok
    }

    /// Carrega uma gramática em WebAssembly com a consulta de realce dela.
    fn load_grammar(&mut self, name: &str, bytes: &[u8], query_text: &str) -> Option<Grammar> {
        {
            let parser = self.parser()?;
            let mut store = parser.take_wasm_store()?;
            let language = store.load_language(name, bytes);
            parser.set_wasm_store(store).ok()?;
            let language = match language {
                Ok(l) => l,
                Err(err) => {
                    tracing::warn!("tree-sitter: gramática {name} não carregou: {err:?}");
                    return None;
                }
            };
            let query = match Query::new(&language, query_text) {
                Ok(q) => q,
                Err(err) => {
                    tracing::warn!("tree-sitter: consulta de {name} com erro: {err}");
                    return None;
                }
            };
            let kinds = query.capture_names().iter().map(|c| kind_of(c)).collect();
            Some(Grammar { language, query, kinds })
        }
    }

    /// Trechos de realce do texto pelo tree-sitter (`None`: sem gramática para a
    /// linguagem; vale o realce próprio).
    pub fn highlight(&mut self, text: &str, lang: &'static Lang) -> Option<Vec<(Range<usize>, Kind)>> {
        if !self.load(lang) {
            return None;
        }
        self.run(lang.id, text)
    }

    fn run(&mut self, id: &str, text: &str) -> Option<Vec<(Range<usize>, Kind)>> {
        let language = self.grammars.get(id)?.as_ref()?.language.clone();
        let parser = self.parser()?;
        parser.set_language(&language).ok()?;
        let tree = parser.parse(text, None)?;
        let g = self.grammars.get(id)?.as_ref()?;
        let mut cursor = QueryCursor::new();
        let mut captures = cursor.captures(&g.query, tree.root_node(), text.as_bytes());
        let mut spans: Vec<(Range<usize>, Kind)> = Vec::new();
        while let Some((m, i)) = captures.next() {
            let c = m.captures[*i];
            if let Some(kind) = g.kinds[c.index as usize] {
                let r = c.node.byte_range();
                // Mesmo trecho capturado de novo: vale o primeiro padrão (como no
                // tree-sitter-highlight).
                if spans.last().is_none_or(|(last, _)| *last != r) {
                    spans.push((r, kind));
                }
            }
        }
        Some(flatten(spans, text.len()))
    }
}

/// Trechos sem sobreposição: dentro de um trecho, o menor (o mais de dentro) vale.
fn flatten(mut spans: Vec<(Range<usize>, Kind)>, len: usize) -> Vec<(Range<usize>, Kind)> {
    spans.sort_by(|a, b| a.0.start.cmp(&b.0.start).then(b.0.end.cmp(&a.0.end)));
    let mut out: Vec<(Range<usize>, Kind)> = Vec::new();
    // Pilha dos trechos abertos (o de cima é o mais de dentro).
    let mut stack: Vec<(Range<usize>, Kind)> = Vec::new();
    let mut pos = 0;
    let emit = |out: &mut Vec<(Range<usize>, Kind)>, a: usize, b: usize, k: Kind| {
        if a < b {
            match out.last_mut() {
                Some((r, lk)) if r.end == a && *lk == k => r.end = b,
                _ => out.push((a..b, k)),
            }
        }
    };
    for (r, k) in spans {
        let r = r.start.min(len)..r.end.min(len);
        // Fecha os que terminam antes do próximo começar.
        while let Some((top, tk)) = stack.last().cloned() {
            if top.end <= r.start {
                emit(&mut out, pos.max(top.start), top.end, tk);
                pos = pos.max(top.end);
                stack.pop();
            } else {
                break;
            }
        }
        if let Some((top, tk)) = stack.last() {
            emit(&mut out, pos.max(top.start), r.start, *tk);
        }
        pos = pos.max(r.start);
        stack.push((r, k));
    }
    while let Some((top, tk)) = stack.pop() {
        emit(&mut out, pos.max(top.start), top.end, tk);
        pos = pos.max(top.end);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capturas_viram_tipos() {
        assert_eq!(kind_of("keyword.function"), Some(Kind::Keyword));
        assert_eq!(kind_of("function.method"), Some(Kind::Function));
        assert_eq!(kind_of("variable"), None);
        assert_eq!(kind_of("variable.special"), Some(Kind::Special));
    }

    /// Com a gramática PHP de uma extensão do Zed instalada nesta máquina (senão, pula).
    #[test]
    fn gramatica_do_zed_em_webassembly() {
        let Some(home) = std::env::var_os("HOME") else { return };
        let dir = PathBuf::from(home).join(".local/share/zed/extensions/installed/php");
        let (Ok(bytes), Ok(query)) = (std::fs::read(dir.join("grammars/php.wasm")), std::fs::read_to_string(dir.join("languages/php/highlights.scm"))) else {
            return;
        };
        let mut s = Syntax::new();
        let g = s.load_grammar("php", &bytes, &query).expect("gramática carregada");
        s.grammars.insert("php", Some(g));
        let text = "<?php\nfunction oi() { return \"a\"; }\n";
        let spans = s.run("php", text).unwrap();
        let at = |w: &str| spans.iter().find(|(r, _)| &text[r.clone()] == w).map(|(_, k)| *k);
        assert_eq!(at("function"), Some(Kind::Keyword));
        assert_eq!(at("oi"), Some(Kind::Function));
        assert!(spans.iter().any(|(r, k)| *k == Kind::String && text[r.clone()].contains('a')));
    }

    #[test]
    fn trechos_sem_sobreposicao() {
        // Uma string inteira com um escape dentro: o escape fica com a cor dele.
        let spans = vec![(0..10, Kind::String), (3..5, Kind::Special)];
        assert_eq!(flatten(spans, 10), vec![(0..3, Kind::String), (3..5, Kind::Special), (5..10, Kind::String)]);
    }
}
