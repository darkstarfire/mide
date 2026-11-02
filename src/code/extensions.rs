//! Extensões do Code no formato das do Zed (`extension.toml` e as pastas
//! `languages/`, `themes/`, `icon_themes/`, `snippets/`), lidas de
//! `~/.local/share/mide/code/extensions/<id>/` e, só para leitura, das instaladas no
//! próprio Zed (`~/.local/share/zed/extensions/installed/<id>/`). Só os formatos do
//! Zed servem de referência: nada do código dele é usado.

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
    /// Arquivos de temas de cores (`themes/x.json`), de ícones e pastas de linguagens.
    pub themes: Vec<String>,
    pub icon_themes: Vec<String>,
    pub languages: Vec<String>,
    /// Arquivos de trechos (`snippets = "snippets.json"` ou uma lista).
    #[serde(deserialize_with = "one_or_many")]
    pub snippets: Vec<String>,
}

fn one_or_many<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(String),
        Many(Vec<String>),
    }
    Ok(match OneOrMany::deserialize(d)? {
        OneOrMany::One(s) => vec![s],
        OneOrMany::Many(v) => v,
    })
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

/// Uma extensão instalada: a pasta, o manifesto e se é a do Zed (só leitura).
#[derive(Debug, Clone)]
pub struct Extension {
    pub dir: PathBuf,
    pub manifest: Manifest,
    pub from_zed: bool,
}

impl Extension {
    /// Arquivos de temas (os do manifesto; sem ele, os `.json` de `themes/`).
    pub fn theme_files(&self, icons: bool) -> Vec<PathBuf> {
        let (listed, folder) = if icons { (&self.manifest.icon_themes, "icon_themes") } else { (&self.manifest.themes, "themes") };
        if !listed.is_empty() {
            return listed.iter().map(|f| self.dir.join(f)).collect();
        }
        json_files(&self.dir.join(folder))
    }

    /// Pastas de linguagens (as do manifesto; sem ele, as de `languages/`).
    pub fn language_dirs(&self) -> Vec<PathBuf> {
        if !self.manifest.languages.is_empty() {
            return self.manifest.languages.iter().map(|f| self.dir.join(f)).collect();
        }
        std::fs::read_dir(self.dir.join("languages")).map(|r| r.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect()).unwrap_or_default()
    }

    /// Arquivos de trechos (os do manifesto e os `.json` de `snippets/`).
    pub fn snippet_files(&self) -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = self.manifest.snippets.iter().map(|f| self.dir.join(f)).collect();
        out.extend(json_files(&self.dir.join("snippets")));
        out.sort();
        out.dedup();
        out
    }
}

fn json_files(dir: &std::path::Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir).map(|r| r.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "json")).collect()).unwrap_or_default();
    v.sort();
    v
}

/// Pasta das extensões instaladas no Zed (só leitura).
pub fn zed_root() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()).map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))?;
    Some(base.join("zed").join("extensions").join("installed"))
}

/// Pasta das extensões do Code.
pub fn root() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()).map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))?;
    Some(base.join("mide").join("code").join("extensions"))
}

/// As extensões instaladas (pastas com um `extension.toml` que dá para ler): as do
/// mide e as do Zed (uma do mide com o mesmo id vale no lugar da do Zed), por id.
pub fn installed() -> Vec<Extension> {
    let mut out: Vec<Extension> = Vec::new();
    for (root, from_zed) in [(root(), false), (zed_root(), true)] {
        let Some(root) = root else {
            continue;
        };
        let Ok(entries) = std::fs::read_dir(&root) else {
            continue;
        };
        for e in entries.flatten() {
            let dir = e.path();
            let Ok(text) = std::fs::read_to_string(dir.join("extension.toml")) else {
                continue;
            };
            let Ok(mut manifest) = toml::from_str::<Manifest>(&text) else {
                continue;
            };
            if manifest.id.is_empty() {
                manifest.id = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            }
            if out.iter().any(|x| x.manifest.id == manifest.id) {
                continue;
            }
            out.push(Extension { dir, manifest, from_zed });
        }
    }
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
