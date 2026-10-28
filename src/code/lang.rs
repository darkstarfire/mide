//! Linguagens do `mide-code`: como reconhecer pelo nome do arquivo, como comentar e as
//! palavras que ganham cor. Inspirado nas gramáticas do TextMate (só a ideia: o
//! realce é feito por um leitor próprio, em `highlight.rs`).

use std::path::Path;

/// Família do leitor de cores.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    /// Texto sem cores.
    Plain,
    /// Linguagens com cara de C (chaves, `//`, `/* */`, aspas): Rust, C, C++, Go, Java,
    /// JavaScript, TypeScript, C#, Kotlin, Swift, CSS, PHP...
    CLike,
    /// Comentário com `#`, aspas simples e duplas: Python, shell, Ruby, Perl, R...
    Hash,
    /// Marcação: HTML, XML, SVG.
    Markup,
    /// Markdown.
    Markdown,
    /// Dados com chave e valor: JSON, TOML, YAML, INI.
    Data,
    /// SQL (comentário `--`).
    Sql,
    /// Lua (comentário `--`).
    Lua,
}

/// Uma linguagem: id (o `languageId` do LSP), nome mostrado, extensões, nomes de arquivo
/// fixos, família, comentário de linha e palavras-chave.
#[derive(Debug)]
pub struct Language {
    pub id: &'static str,
    pub name: &'static str,
    pub exts: &'static [&'static str],
    pub files: &'static [&'static str],
    pub family: Family,
    pub comment: &'static str,
    pub keywords: &'static [&'static str],
    /// Tipos e constantes que ganham a cor de tipo (além dos nomes com maiúscula).
    pub types: &'static [&'static str],
}

impl Language {
    /// Nome mostrado na tela, no idioma da sessão (só "Plain Text" muda; os nomes das
    /// linguagens são os mesmos em todo idioma).
    pub fn label(&self) -> std::borrow::Cow<'static, str> {
        if self.id == "plaintext" { crate::tr!("Plain Text") } else { std::borrow::Cow::Borrowed(self.name) }
    }
}

const RUST_KW: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub",
    "ref", "return", "self", "Self", "static", "struct", "super", "trait", "type", "unsafe", "use", "where", "while", "true", "false",
];
const RUST_TYPES: &[&str] = &["i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64", "u128", "usize", "f32", "f64", "bool", "char", "str", "String", "Vec", "Option", "Result", "Some", "None", "Ok", "Err", "Box"];
const C_KW: &[&str] = &[
    "auto", "break", "case", "const", "continue", "default", "do", "else", "enum", "extern", "for", "goto", "if", "inline", "register", "return", "sizeof", "static", "struct", "switch",
    "typedef", "union", "volatile", "while", "#include", "#define", "#ifdef", "#ifndef", "#endif", "#if", "#else", "#pragma", "NULL", "true", "false", "class", "namespace", "public",
    "private", "protected", "template", "typename", "virtual", "override", "new", "delete", "this", "using", "nullptr", "try", "catch", "throw", "operator", "constexpr",
];
const C_TYPES: &[&str] = &["int", "char", "float", "double", "void", "long", "short", "unsigned", "signed", "bool", "size_t", "uint8_t", "uint32_t", "int32_t", "uint64_t", "int64_t", "std", "string", "vector", "auto"];
const GO_KW: &[&str] = &[
    "break", "case", "chan", "const", "continue", "default", "defer", "else", "fallthrough", "for", "func", "go", "goto", "if", "import", "interface", "map", "package", "range", "return",
    "select", "struct", "switch", "type", "var", "true", "false", "nil", "iota",
];
const GO_TYPES: &[&str] = &["int", "int8", "int16", "int32", "int64", "uint", "uint8", "uint16", "uint32", "uint64", "float32", "float64", "string", "bool", "byte", "rune", "error", "any"];
const JS_KW: &[&str] = &[
    "async", "await", "break", "case", "catch", "class", "const", "continue", "debugger", "default", "delete", "do", "else", "export", "extends", "finally", "for", "from", "function", "if",
    "import", "in", "instanceof", "let", "new", "of", "return", "static", "super", "switch", "this", "throw", "try", "typeof", "var", "void", "while", "yield", "true", "false", "null",
    "undefined", "interface", "type", "enum", "implements", "private", "public", "protected", "readonly", "as",
];
const JS_TYPES: &[&str] = &["string", "number", "boolean", "any", "unknown", "never", "void", "object", "Array", "Promise", "Map", "Set"];
const JAVA_KW: &[&str] = &[
    "abstract", "break", "case", "catch", "class", "continue", "default", "do", "else", "enum", "extends", "final", "finally", "for", "if", "implements", "import", "instanceof",
    "interface", "new", "package", "private", "protected", "public", "return", "static", "super", "switch", "synchronized", "this", "throw", "throws", "try", "void", "while", "true",
    "false", "null", "var", "val", "fun", "when", "object", "override", "namespace", "using", "func", "let", "guard", "struct", "protocol", "extension", "init", "self",
];
const JAVA_TYPES: &[&str] = &["int", "long", "short", "byte", "char", "float", "double", "boolean", "String", "Int", "Long", "Double", "Bool", "Unit", "Any"];
const PY_KW: &[&str] = &[
    "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif", "else", "except", "finally", "for", "from", "global", "if", "import", "in", "is", "lambda",
    "nonlocal", "not", "or", "pass", "raise", "return", "try", "while", "with", "yield", "True", "False", "None", "self", "match", "case",
];
const PY_TYPES: &[&str] = &["int", "float", "str", "bool", "list", "dict", "set", "tuple", "bytes", "object"];
const SH_KW: &[&str] = &[
    "if", "then", "else", "elif", "fi", "case", "esac", "for", "while", "until", "do", "done", "in", "function", "return", "local", "export", "readonly", "set", "unset", "shift", "exit",
    "echo", "source", "cd", "true", "false",
];
const RUBY_KW: &[&str] = &["def", "end", "class", "module", "if", "elsif", "else", "unless", "while", "until", "do", "return", "yield", "require", "nil", "true", "false", "self", "begin", "rescue"];
const CSS_KW: &[&str] = &["@media", "@import", "@keyframes", "@font-face", "!important", "inherit", "initial", "none", "auto"];
const SQL_KW: &[&str] = &[
    "select", "from", "where", "insert", "into", "values", "update", "set", "delete", "create", "table", "drop", "alter", "index", "join", "left", "right", "inner", "outer", "on", "and",
    "or", "not", "null", "group", "by", "order", "having", "limit", "as", "distinct", "primary", "key", "foreign", "references", "SELECT", "FROM", "WHERE", "INSERT", "INTO", "VALUES",
    "UPDATE", "SET", "DELETE", "CREATE", "TABLE", "DROP", "ALTER", "JOIN", "LEFT", "ON", "AND", "OR", "NOT", "NULL", "GROUP", "BY", "ORDER", "LIMIT", "AS", "PRIMARY", "KEY",
];
const LUA_KW: &[&str] = &["and", "break", "do", "else", "elseif", "end", "false", "for", "function", "if", "in", "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while"];
const DATA_KW: &[&str] = &["true", "false", "null", "yes", "no", "on", "off"];
const PHP_KW: &[&str] = &["function", "class", "public", "private", "protected", "static", "return", "if", "else", "elseif", "foreach", "as", "while", "for", "new", "echo", "use", "namespace", "true", "false", "null", "array"];

/// Todas as linguagens conhecidas (a primeira é texto simples).
pub const LANGUAGES: &[Language] = &[
    Language { id: "plaintext", name: "Plain Text", exts: &["txt", "log", "text"], files: &[], family: Family::Plain, comment: "", keywords: &[], types: &[] },
    Language { id: "rust", name: "Rust", exts: &["rs"], files: &[], family: Family::CLike, comment: "//", keywords: RUST_KW, types: RUST_TYPES },
    Language { id: "c", name: "C", exts: &["c", "h"], files: &[], family: Family::CLike, comment: "//", keywords: C_KW, types: C_TYPES },
    Language { id: "cpp", name: "C++", exts: &["cpp", "cc", "cxx", "hpp", "hh", "hxx"], files: &[], family: Family::CLike, comment: "//", keywords: C_KW, types: C_TYPES },
    Language { id: "go", name: "Go", exts: &["go"], files: &[], family: Family::CLike, comment: "//", keywords: GO_KW, types: GO_TYPES },
    Language { id: "javascript", name: "JavaScript", exts: &["js", "mjs", "cjs", "jsx"], files: &[], family: Family::CLike, comment: "//", keywords: JS_KW, types: JS_TYPES },
    Language { id: "typescript", name: "TypeScript", exts: &["ts", "tsx", "mts"], files: &[], family: Family::CLike, comment: "//", keywords: JS_KW, types: JS_TYPES },
    Language { id: "java", name: "Java", exts: &["java"], files: &[], family: Family::CLike, comment: "//", keywords: JAVA_KW, types: JAVA_TYPES },
    Language { id: "kotlin", name: "Kotlin", exts: &["kt", "kts"], files: &[], family: Family::CLike, comment: "//", keywords: JAVA_KW, types: JAVA_TYPES },
    Language { id: "swift", name: "Swift", exts: &["swift"], files: &[], family: Family::CLike, comment: "//", keywords: JAVA_KW, types: JAVA_TYPES },
    Language { id: "csharp", name: "C#", exts: &["cs"], files: &[], family: Family::CLike, comment: "//", keywords: JAVA_KW, types: JAVA_TYPES },
    Language { id: "php", name: "PHP", exts: &["php"], files: &[], family: Family::CLike, comment: "//", keywords: PHP_KW, types: &[] },
    Language { id: "css", name: "CSS", exts: &["css", "scss", "less"], files: &[], family: Family::CLike, comment: "", keywords: CSS_KW, types: &[] },
    Language { id: "python", name: "Python", exts: &["py", "pyw", "pyi"], files: &[], family: Family::Hash, comment: "#", keywords: PY_KW, types: PY_TYPES },
    Language {
        id: "shellscript",
        name: "Shell",
        exts: &["sh", "bash", "zsh", "fish"],
        files: &[".bashrc", ".zshrc", ".profile", ".bash_profile", "PKGBUILD"],
        family: Family::Hash,
        comment: "#",
        keywords: SH_KW,
        types: &[],
    },
    Language { id: "ruby", name: "Ruby", exts: &["rb"], files: &["Gemfile", "Rakefile"], family: Family::Hash, comment: "#", keywords: RUBY_KW, types: &[] },
    Language { id: "makefile", name: "Makefile", exts: &["mk"], files: &["Makefile", "makefile", "GNUmakefile"], family: Family::Hash, comment: "#", keywords: &[], types: &[] },
    Language { id: "dockerfile", name: "Dockerfile", exts: &[], files: &["Dockerfile", "Containerfile"], family: Family::Hash, comment: "#", keywords: &["FROM", "RUN", "CMD", "COPY", "ADD", "ENV", "WORKDIR", "EXPOSE", "ENTRYPOINT", "ARG", "USER", "VOLUME", "LABEL"], types: &[] },
    Language { id: "lua", name: "Lua", exts: &["lua"], files: &[], family: Family::Lua, comment: "--", keywords: LUA_KW, types: &[] },
    Language { id: "sql", name: "SQL", exts: &["sql"], files: &[], family: Family::Sql, comment: "--", keywords: SQL_KW, types: &[] },
    Language { id: "html", name: "HTML", exts: &["html", "htm", "xhtml"], files: &[], family: Family::Markup, comment: "", keywords: &[], types: &[] },
    Language { id: "xml", name: "XML", exts: &["xml", "svg", "xsd", "plist", "ui", "glade"], files: &[], family: Family::Markup, comment: "", keywords: &[], types: &[] },
    Language { id: "markdown", name: "Markdown", exts: &["md", "markdown", "mdown"], files: &["README"], family: Family::Markdown, comment: "", keywords: &[], types: &[] },
    Language { id: "json", name: "JSON", exts: &["json", "jsonc", "json5"], files: &[], family: Family::Data, comment: "//", keywords: DATA_KW, types: &[] },
    Language { id: "toml", name: "TOML", exts: &["toml"], files: &["Cargo.lock"], family: Family::Data, comment: "#", keywords: DATA_KW, types: &[] },
    Language { id: "yaml", name: "YAML", exts: &["yaml", "yml"], files: &[], family: Family::Data, comment: "#", keywords: DATA_KW, types: &[] },
    Language { id: "ini", name: "INI", exts: &["ini", "conf", "cfg", "desktop", "service", "properties"], files: &[".gitconfig", ".editorconfig"], family: Family::Data, comment: "#", keywords: DATA_KW, types: &[] },
];

/// Texto simples (a primeira da lista).
pub fn plain() -> &'static Language {
    &LANGUAGES[0]
}

/// Linguagem pelo id do LSP (`rust`, `python`...).
pub fn by_id(id: &str) -> Option<&'static Language> {
    LANGUAGES.iter().find(|l| l.id == id)
}

/// Linguagem de um arquivo: pelo nome fixo (Makefile, Dockerfile), pela extensão ou,
/// sem extensão, pela primeira linha (`#!/bin/bash`, `#!/usr/bin/env python3`).
pub fn detect(path: &Path, first_line: &str) -> &'static Language {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if let Some(l) = LANGUAGES.iter().find(|l| l.files.contains(&name.as_str())) {
        return l;
    }
    if let Some(ext) = path.extension().map(|e| e.to_string_lossy().to_lowercase()) {
        if let Some(l) = LANGUAGES.iter().find(|l| l.exts.contains(&ext.as_str())) {
            return l;
        }
    }
    if let Some(shebang) = first_line.strip_prefix("#!") {
        let prog = shebang.split_whitespace().map(|w| w.rsplit('/').next().unwrap_or(w)).find(|w| *w != "env").unwrap_or("");
        let id = match prog {
            p if p.starts_with("python") => "python",
            "bash" | "sh" | "zsh" | "dash" | "fish" => "shellscript",
            "ruby" => "ruby",
            "node" | "deno" => "javascript",
            "lua" => "lua",
            _ => "",
        };
        if let Some(l) = by_id(id) {
            return l;
        }
    }
    plain()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconhece_linguagens() {
        assert_eq!(detect(Path::new("src/main.rs"), "").id, "rust");
        assert_eq!(detect(Path::new("Makefile"), "").id, "makefile");
        assert_eq!(detect(Path::new("x/Dockerfile"), "").id, "dockerfile");
        assert_eq!(detect(Path::new("script"), "#!/usr/bin/env python3").id, "python");
        assert_eq!(detect(Path::new("run"), "#!/bin/bash").id, "shellscript");
        assert_eq!(detect(Path::new("notas.TXT"), "").id, "plaintext");
        assert_eq!(detect(Path::new("dados.JSON"), "").id, "json");
        assert_eq!(detect(Path::new("sem-nada"), "olá").id, "plaintext");
    }
}
