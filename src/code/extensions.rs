//! Extensões do Code no formato das do Zed (`extension.toml` e as pastas
//! `languages/`, `themes/`, `icon_themes/`, `snippets/`), lidas de
//! `~/.local/share/mide/code/extensions/<id>/`. Só os formatos do Zed servem de
//! referência: nada do código dele é usado.

use std::path::PathBuf;

use serde::Deserialize;

/// O manifesto (`extension.toml`), com os campos que o mide usa.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub authors: Vec<String>,
    pub repository: String,
    /// Servidores de linguagem que a extensão traz (`[language_servers.<id>]`).
    pub language_servers: std::collections::BTreeMap<String, LanguageServerEntry>,
    /// Gramáticas tree-sitter (`[grammars.<nome>]`: repositório e commit).
    pub grammars: std::collections::BTreeMap<String, GrammarEntry>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct LanguageServerEntry {
    pub name: String,
    pub language: String,
    pub languages: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct GrammarEntry {
    pub repository: String,
    #[serde(alias = "commit")]
    pub rev: String,
    pub path: String,
}

/// Uma extensão instalada: a pasta e o manifesto.
#[derive(Debug, Clone)]
pub struct Extension {
    pub dir: PathBuf,
    pub manifest: Manifest,
}

/// Pasta das extensões do Code.
pub fn root() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()).map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))?;
    Some(base.join("mide").join("code").join("extensions"))
}

/// As extensões instaladas (pastas com um `extension.toml` que dá para ler), por id.
pub fn installed() -> Vec<Extension> {
    let Some(root) = root() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&root) else {
        return Vec::new();
    };
    let mut out: Vec<Extension> = entries
        .flatten()
        .filter_map(|e| {
            let dir = e.path();
            let text = std::fs::read_to_string(dir.join("extension.toml")).ok()?;
            let mut manifest: Manifest = toml::from_str(&text).ok()?;
            if manifest.id.is_empty() {
                manifest.id = dir.file_name()?.to_string_lossy().into_owned();
            }
            Some(Extension { dir, manifest })
        })
        .collect();
    out.sort_by(|a, b| a.manifest.id.cmp(&b.manifest.id));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_o_manifesto_do_zed() {
        let text = r#"
id = "php"
name = "PHP"
version = "0.2.0"
schema_version = 1
authors = ["Alguém <a@b.c>"]
description = "Suporte a PHP"
repository = "https://github.com/x/y"

[language_servers.intelephense]
name = "Intelephense"
language = "PHP"

[grammars.php]
repository = "https://github.com/tree-sitter/tree-sitter-php"
commit = "abc123"
path = "php"
"#;
        let m: Manifest = toml::from_str(text).unwrap();
        assert_eq!(m.id, "php");
        assert_eq!(m.language_servers["intelephense"].language, "PHP");
        assert_eq!(m.grammars["php"].rev, "abc123");
    }
}
