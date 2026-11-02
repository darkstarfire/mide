//! Temas de cores e de ícones do Code no formato do Zed (`themes/*.json` e
//! `icon_themes/*.json` das extensões, e os temas do usuário em
//! `~/.config/zed/themes/`). Só o formato serve de referência.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use serde_json::Value;

use crate::code::highlight::Kind;

pub type Rgba = [u8; 4];

/// Um tema de cores: nome, se é escuro, as cores do editor e as da sintaxe.
#[derive(Debug, Clone, PartialEq)]
pub struct ColorTheme {
    pub name: String,
    pub dark: bool,
    pub background: Option<Rgba>,
    pub foreground: Option<Rgba>,
    pub gutter: Option<Rgba>,
    pub line_number: Option<Rgba>,
    pub active_line: Option<Rgba>,
    pub syntax: HashMap<Kind, Rgba>,
}

/// Cor `#rgb`, `#rrggbb` ou `#rrggbbaa`.
pub fn parse_color(s: &str) -> Option<Rgba> {
    let h = s.strip_prefix('#')?;
    let byte = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok();
    match h.len() {
        3 => {
            let d = |i: usize| u8::from_str_radix(&h[i..i + 1], 16).ok().map(|v| v * 17);
            Some([d(0)?, d(1)?, d(2)?, 255])
        }
        6 => Some([byte(0)?, byte(2)?, byte(4)?, 255]),
        8 => Some([byte(0)?, byte(2)?, byte(4)?, byte(6)?]),
        _ => None,
    }
}

/// Tipo do realce do mide para as chaves de sintaxe do Zed (a primeira que existir).
const SYNTAX: &[(Kind, &[&str])] = &[
    (Kind::Comment, &["comment", "comment.doc"]),
    (Kind::String, &["string", "string.special"]),
    (Kind::Number, &["number", "constant", "boolean"]),
    (Kind::Keyword, &["keyword"]),
    (Kind::Type, &["type", "type.builtin", "constructor"]),
    (Kind::Function, &["function", "function.method", "function.definition"]),
    (Kind::Key, &["property", "attribute", "property.json_key"]),
    (Kind::Tag, &["tag", "title"]),
    (Kind::Special, &["preproc", "function.special", "emphasis", "constant.builtin"]),
];

/// Os temas de um arquivo de tema do Zed.
pub fn parse_color_themes(text: &str) -> Vec<ColorTheme> {
    let Some(v) = crate::code::jsonc::parse(text) else {
        return Vec::new();
    };
    let color = |st: &Value, key: &str| st.get(key).and_then(Value::as_str).and_then(parse_color);
    v["themes"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|t| {
            let st = &t["style"];
            let mut syntax = HashMap::new();
            for (kind, keys) in SYNTAX {
                if let Some(c) = keys.iter().find_map(|k| st["syntax"][*k]["color"].as_str().and_then(parse_color)) {
                    syntax.insert(*kind, c);
                }
            }
            ColorTheme {
                name: t["name"].as_str().unwrap_or("?").to_string(),
                dark: t["appearance"].as_str() != Some("light"),
                background: color(st, "editor.background"),
                foreground: color(st, "editor.foreground").or_else(|| color(st, "text")),
                gutter: color(st, "editor.gutter.background"),
                line_number: color(st, "editor.line_number"),
                active_line: color(st, "editor.active_line.background"),
                syntax,
            }
        })
        .collect()
}

/// Todos os temas de cores das extensões e de `~/.config/zed/themes/`, por nome.
pub fn color_themes() -> Vec<ColorTheme> {
    let mut files: Vec<PathBuf> = crate::code::extensions::installed().iter().flat_map(|e| e.theme_files(false)).collect();
    if let Some(dir) = crate::code::zed::config_dir() {
        files.extend(std::fs::read_dir(dir.join("themes")).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "json")));
    }
    let mut out: Vec<ColorTheme> = files.iter().filter_map(|f| std::fs::read_to_string(f).ok()).flat_map(|t| parse_color_themes(&t)).collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out.dedup_by(|a, b| a.name == b.name);
    out
}

/// Um tema de ícones: pasta base (os caminhos do JSON são relativos à extensão) e os
/// mapas do Zed.
#[derive(Debug, Clone, Default)]
pub struct IconTheme {
    pub name: String,
    pub dark: bool,
    pub base: PathBuf,
    pub folder: (Option<String>, Option<String>),
    pub named_folders: HashMap<String, (Option<String>, Option<String>)>,
    pub stems: HashMap<String, String>,
    pub suffixes: HashMap<String, String>,
    pub icons: HashMap<String, String>,
}

fn pair(v: &Value) -> (Option<String>, Option<String>) {
    (v["collapsed"].as_str().map(str::to_string), v["expanded"].as_str().map(str::to_string))
}

fn str_map(v: &Value) -> HashMap<String, String> {
    v.as_object().map(|o| o.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.to_lowercase(), s.to_string()))).collect()).unwrap_or_default()
}

/// Os temas de ícones de um arquivo do Zed (`base`: a pasta da extensão).
pub fn parse_icon_themes(text: &str, base: &Path) -> Vec<IconTheme> {
    let Some(v) = crate::code::jsonc::parse(text) else {
        return Vec::new();
    };
    v["themes"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|t| IconTheme {
            name: t["name"].as_str().unwrap_or("?").to_string(),
            dark: t["appearance"].as_str() != Some("light"),
            base: base.to_path_buf(),
            folder: pair(&t["directory_icons"]),
            named_folders: t["named_directory_icons"].as_object().map(|o| o.iter().map(|(k, v)| (k.to_lowercase(), pair(v))).collect()).unwrap_or_default(),
            stems: str_map(&t["file_stems"]),
            suffixes: str_map(&t["file_suffixes"]),
            icons: t["file_icons"].as_object().map(|o| o.iter().filter_map(|(k, v)| v["path"].as_str().map(|p| (k.clone(), p.to_string()))).collect()).unwrap_or_default(),
        })
        .collect()
}

/// Todos os temas de ícones das extensões.
pub fn icon_themes() -> Vec<IconTheme> {
    let mut out: Vec<IconTheme> = crate::code::extensions::installed()
        .iter()
        .flat_map(|e| e.theme_files(true).into_iter().filter_map(|f| std::fs::read_to_string(&f).ok()).flat_map(|t| parse_icon_themes(&t, &e.dir)).collect::<Vec<_>>())
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

impl IconTheme {
    fn resolve(&self, rel: &str) -> Option<String> {
        let p = self.base.join(rel.trim_start_matches("./"));
        p.exists().then(|| p.to_string_lossy().into_owned())
    }

    /// Ícone de um arquivo ou pasta neste tema (caminho absoluto), se houver.
    pub fn icon_for(&self, name: &str, is_dir: bool, open: bool) -> Option<String> {
        let lower = name.to_lowercase();
        if is_dir {
            let (closed, expanded) = self.named_folders.get(&lower).cloned().unwrap_or_else(|| self.folder.clone());
            let rel = if open { expanded.or(closed) } else { closed }?;
            return self.resolve(&rel);
        }
        // Nome inteiro ("Makefile", "Cargo.toml"), depois a extensão mais longa.
        let mut key = self.stems.get(&lower).or_else(|| self.suffixes.get(&lower)).cloned();
        if key.is_none() {
            let mut rest = lower.as_str();
            while let Some(i) = rest.find('.') {
                rest = &rest[i + 1..];
                if let Some(k) = self.suffixes.get(rest) {
                    key = Some(k.clone());
                    break;
                }
            }
        }
        let key = key.unwrap_or_else(|| "default".into());
        self.icons.get(&key).or_else(|| self.icons.get("default")).and_then(|rel| self.resolve(rel))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tema_de_cores_do_zed() {
        let t = r##"{ "name": "x", "themes": [{ "name": "Noite", "appearance": "dark", "style": {
            "editor.background": "#1E293B", "editor.foreground": "#fff",
            "syntax": { "keyword": { "color": "#CBD5E1" }, "comment": { "color": "#64748b80" } } } }] }"##;
        let th = &parse_color_themes(t)[0];
        assert!(th.dark);
        assert_eq!(th.background, Some([0x1e, 0x29, 0x3b, 255]));
        assert_eq!(th.foreground, Some([255, 255, 255, 255]));
        assert_eq!(th.syntax[&Kind::Comment], [0x64, 0x74, 0x8b, 0x80]);
    }

    #[test]
    fn tema_de_icones_do_zed() {
        let dir = std::env::temp_dir().join(format!("mide-icon-theme-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("icons")).unwrap();
        for f in ["rust.svg", "folder.svg", "src.svg", "file.svg"] {
            std::fs::write(dir.join("icons").join(f), "<svg/>").unwrap();
        }
        let t = r#"{ "themes": [{ "name": "Pastel", "appearance": "dark",
            "directory_icons": { "collapsed": "./icons/folder.svg" },
            "named_directory_icons": { "src": { "collapsed": "./icons/src.svg" } },
            "file_suffixes": { "rs": "rust" }, "file_stems": {},
            "file_icons": { "rust": { "path": "./icons/rust.svg" }, "default": { "path": "./icons/file.svg" } } }] }"#;
        let th = &parse_icon_themes(t, &dir)[0];
        assert!(th.icon_for("main.rs", false, false).unwrap().ends_with("rust.svg"));
        assert!(th.icon_for("leia.txt", false, false).unwrap().ends_with("file.svg"));
        assert!(th.icon_for("src", true, false).unwrap().ends_with("src.svg"));
        assert!(th.icon_for("outra", true, true).unwrap().ends_with("folder.svg"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
