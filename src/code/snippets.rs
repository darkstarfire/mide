//! Trechos (snippets) do usuário e das extensões, no formato do VS Code e do Zed:
//! um JSON por linguagem (`rust.json`, `php.json`...) ou `snippets.json` para todas,
//! com `{ "nome": { "prefix": "fn", "body": ["..."], "description": "..." } }`.
//!
//! Pastas lidas, em ordem: `~/.config/mide/code-snippets/` e a pasta `snippets/` de
//! cada extensão instalada (`~/.local/share/mide/code/extensions/*/snippets/`).

use std::path::{Path, PathBuf};

use serde_json::Value;

/// Um trecho: o que se digita, o texto (com as marcas `${1:...}`) e a descrição.
#[derive(Debug, Clone, PartialEq)]
pub struct Snippet {
    pub name: String,
    pub prefix: String,
    pub body: String,
    pub description: String,
}

/// Lê um arquivo de trechos (aceita comentários `//` e vírgulas sobrando, como o
/// VS Code).
pub fn parse(text: &str) -> Vec<Snippet> {
    let Ok(v) = serde_json::from_str::<Value>(&crate::code::jsonc::strip(text)) else {
        return Vec::new();
    };
    let Some(map) = v.as_object() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (name, s) in map {
        let body = match &s["body"] {
            Value::String(b) => b.clone(),
            Value::Array(lines) => lines.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("\n"),
            _ => continue,
        };
        let prefixes: Vec<String> = match &s["prefix"] {
            Value::String(p) => vec![p.clone()],
            Value::Array(ps) => ps.iter().filter_map(Value::as_str).map(str::to_string).collect(),
            _ => vec![name.clone()],
        };
        let description = s["description"].as_str().unwrap_or_default().to_string();
        for prefix in prefixes {
            out.push(Snippet { name: name.clone(), prefix, body: body.clone(), description: description.clone() });
        }
    }
    out.sort_by(|a, b| a.prefix.cmp(&b.prefix));
    out
}

/// Pasta dos trechos do usuário.
pub fn user_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()).map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("mide").join("code-snippets"))
}

/// Os trechos de uma linguagem (pelo id do mide e o nome que o VS Code/Zed usam),
/// mais os de todas as linguagens: os do usuário e os das extensões.
pub fn for_language(lang: &str) -> Vec<Snippet> {
    let names = file_names(lang);
    let wanted = |path: &Path| path.file_stem().is_some_and(|s| {
        let s = s.to_string_lossy().to_lowercase();
        names.contains(&s) || s == "snippets" || s == "global"
    });
    let mut files: Vec<PathBuf> = Vec::new();
    if let Some(dir) = user_dir() {
        files.extend(std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()));
    }
    for ext in crate::code::extensions::installed() {
        files.extend(ext.snippet_files());
    }
    files.into_iter().filter(|p| wanted(p)).flat_map(|p| read(&p)).collect()
}

fn read(path: &Path) -> Vec<Snippet> {
    std::fs::read_to_string(path).map(|t| parse(&t)).unwrap_or_default()
}

/// Nomes de arquivo de trechos de uma linguagem (o id do mide e os apelidos).
fn file_names(lang: &str) -> Vec<String> {
    let mut v = vec![lang.to_string()];
    let extra: &[&str] = match lang {
        "shellscript" => &["shell", "bash", "sh"],
        "cpp" => &["c++"],
        "csharp" => &["c#", "cs"],
        "javascript" => &["js", "javascriptreact"],
        "typescript" => &["ts", "typescriptreact", "tsx"],
        "python" => &["py"],
        "markdown" => &["md"],
        _ => &[],
    };
    v.extend(extra.iter().map(|s| s.to_string()));
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_o_formato_do_vscode() {
        let text = r##"{
            // comentário
            "Função": { "prefix": ["fn", "func"], "body": ["fn ${1:nome}() {", "\t$0", "}"], "description": "Uma função" },
            "Teste": { "prefix": "test", "body": "#[test]\nfn $1() {}", },
        }"##;
        let s = parse(text);
        assert_eq!(s.len(), 3);
        assert_eq!(s[0].prefix, "fn");
        assert_eq!(s[0].body, "fn ${1:nome}() {\n\t$0\n}");
        assert_eq!(s[2].prefix, "test");
    }
}
