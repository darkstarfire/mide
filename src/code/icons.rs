//! Ícones do Code: dos arquivos e pastas (do Material Icon Theme, MIT, embutidos em
//! `file_icons_gen.rs`) e dos tipos de sugestão do completar (desenho próprio: selo
//! colorido com um símbolo branco, um por tipo do LSP).
//!
//! Os ícones vão para `~/.cache/mide/code-icons/` na primeira vez que aparecem e a
//! função devolve o caminho, que o mide-ui aceita no lugar do nome do tema.

use std::path::{Path, PathBuf};

#[path = "file_icons_gen.rs"]
mod generated;

/// Pasta dos ícones escritos (muda com a versão, para ícones novos não ficarem velhos).
fn cache_dir() -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).unwrap_or_else(|| {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp"));
        home.join(".cache")
    });
    base.join("mide").join(format!("code-icons-{}", env!("CARGO_PKG_VERSION")))
}

/// Escreve o SVG no cache (se ainda não está) e devolve o caminho.
fn cached(name: &str, svg: &[u8]) -> Option<String> {
    let dir = cache_dir();
    let path = dir.join(format!("{name}.svg"));
    if !path.exists() {
        std::fs::create_dir_all(&dir).ok()?;
        // Escrita por um temporário: dois apps abrindo juntos não leem meio arquivo.
        let tmp = dir.join(format!(".{name}.{}", std::process::id()));
        std::fs::write(&tmp, svg).ok()?;
        std::fs::rename(&tmp, &path).ok()?;
    }
    Some(path.to_string_lossy().into_owned())
}

fn svg_of(name: &str) -> Option<&'static [u8]> {
    generated::SVGS.binary_search_by(|(n, _)| n.cmp(&name)).ok().map(|i| generated::SVGS[i].1)
}

fn lookup(table: &'static [(&'static str, &'static str)], key: &str) -> Option<&'static str> {
    table.binary_search_by(|(k, _)| k.cmp(&key)).ok().map(|i| table[i].1)
}

/// Nome do ícone do tema para um arquivo: pelo nome inteiro, depois pela extensão
/// mais longa (`blade.php` antes de `php`, `d.ts` antes de `ts`).
pub fn file_icon_name(file_name: &str) -> &'static str {
    let lower = file_name.to_lowercase();
    if let Some(n) = lookup(generated::FILE_NAMES, &lower) {
        return n;
    }
    let mut rest = lower.as_str();
    while let Some(i) = rest.find('.') {
        rest = &rest[i + 1..];
        if let Some(n) = lookup(generated::EXTENSIONS, rest) {
            return n;
        }
    }
    "file"
}

/// Nome do ícone do tema para uma pasta (aberta ou fechada).
pub fn folder_icon_name(folder_name: &str, open: bool) -> String {
    let base = lookup(generated::FOLDER_NAMES, &folder_name.to_lowercase()).unwrap_or("folder");
    if open && svg_of(&format!("{base}-open")).is_some() { format!("{base}-open") } else { base.to_string() }
}

/// Tema de ícones de uma extensão do Zed escolhido nas Preferências (nenhum: o
/// Material Icon Theme embutido). Um para o tema claro e outro para o escuro.
static THEME: std::sync::RwLock<Option<(Option<crate::code::themes::IconTheme>, Option<crate::code::themes::IconTheme>)>> = std::sync::RwLock::new(None);

/// Escolhe o tema de ícones pelo nome ("" ou desconhecido: o embutido). Temas com
/// versões clara e escura (Catppuccin Latte/Mocha) usam a do mesmo grupo de nomes.
pub fn set_theme(name: &str) {
    let chosen = if name.is_empty() {
        None
    } else {
        let all = crate::code::themes::icon_themes();
        all.iter().find(|t| t.name == name).cloned().map(|picked| {
            // Par claro/escuro: o tema escolhido e um do mesmo arquivo com a outra
            // aparência (o primeiro que houver).
            let other = all.iter().find(|t| t.base == picked.base && t.dark != picked.dark).cloned();
            if picked.dark { (other, Some(picked)) } else { (Some(picked), other) }
        })
    };
    if let Ok(mut t) = THEME.write() {
        *t = chosen;
    }
}

/// Caminho do ícone (no cache) de um arquivo ou pasta da árvore. `light`: tema claro
/// (alguns ícones têm uma versão própria para fundo claro).
pub fn path_icon(path: &Path, is_dir: bool, open: bool, light: bool) -> Option<String> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if let Ok(guard) = THEME.read()
        && let Some((l, d)) = guard.as_ref()
        && let Some(theme) = if light { l.as_ref().or(d.as_ref()) } else { d.as_ref().or(l.as_ref()) }
        && let Some(icon) = theme.icon_for(&name, is_dir, open)
    {
        return Some(icon);
    }
    let icon = if is_dir { folder_icon_name(&name, open) } else { file_icon_name(&name).to_string() };
    let light_name = format!("{icon}_light");
    let chosen = if light && svg_of(&light_name).is_some() { light_name } else { icon };
    let svg = svg_of(&chosen).or_else(|| svg_of(if is_dir { "folder" } else { "file" }))?;
    cached(&chosen, svg)
}

/// Cor e símbolo (traços brancos num quadro de 16) de cada tipo de sugestão.
fn kind_badge(kind: u8) -> (&'static str, &'static str) {
    // Traço branco; `F` no começo = preenchido em vez de contorno.
    const FUNCTION: &str = "M10.5 4.6c-1.4-.5-2.4.1-2.7 1.5L6.6 11.4c-.3 1.4-1.3 1.9-2.7 1.4M5.4 7.6h4.4";
    const METHOD: &str = "M8 3.8l3.6 2v4.4L8 12.2l-3.6-2V5.8zM4.4 5.8L8 7.8l3.6-2M8 7.8v4.4";
    const VARIABLE: &str = "M5.4 4.5H4v7h1.4M10.6 4.5H12v7h-1.4M6.6 6.4l2.8 3.2M9.4 6.4L6.6 9.6";
    const FIELD: &str = "M3.8 8a2 2 0 1 0 4 0 2 2 0 1 0-4 0M7.8 8h4.4M11 8v2";
    const CLASS: &str = "M3.8 4.2h4v3h-4zM8.2 8.8h4v3h-4zM5.8 7.2v3.1h2.4";
    const STRUCT: &str = "M4 4h3.4v3.4H4zM8.6 4H12v3.4H8.6zM4 8.6h3.4V12H4zM8.6 8.6H12V12H8.6z";
    const INTERFACE: &str = "M3.4 8h4.2M7.6 8a2.4 2.4 0 1 0 4.8 0 2.4 2.4 0 1 0-4.8 0";
    const MODULE: &str = "M6.2 3.8c-1.4 0-1.4 1-1.4 2s-.4 2.2-1.4 2.2c1 0 1.4 1.2 1.4 2.2s0 2 1.4 2M9.8 3.8c1.4 0 1.4 1 1.4 2s.4 2.2 1.4 2.2c-1 0-1.4 1.2-1.4 2.2s0 2-1.4 2";
    const ENUM: &str = "M4 5h8M4 8h5M4 11h7";
    const CONSTANT: &str = "M4.4 6.4h7.2M4.4 9.6h7.2";
    const KEYWORD: &str = "M4 5h3M9 5h3M4 8h8M4 11h5";
    const SNIPPET: &str = "M4 6V4h2M10 4h2v2M12 10v2h-2M6 12H4v-2M7 8h2";
    const TEXT: &str = "M4 5.4h8M4 8h8M4 10.6h5";
    const FILE: &str = "M5 3.6h3.8l2.4 2.4v6.4H5zM8.6 3.6V6.2h2.6";
    const FOLDER: &str = "M3.6 5h2.8l1 1h5v5.4H3.6z";
    const EVENT: &str = "FM9.2 3L5 9h3l-1.2 4 4.4-6H8.2z";
    const OPERATOR: &str = "M5 8h6M8 5v6";
    const TYPE: &str = "M4.4 4.6h7.2M8 4.6v7";
    const COLOR: &str = "M8 4a4 4 0 1 0 0 8 4 4 0 1 0 0-8M8 4v8";
    match kind {
        2..=4 => ("#9b5de5", if kind == 2 { METHOD } else { FUNCTION }),
        5 | 10 => ("#2f7de1", FIELD),
        6 => ("#2f7de1", VARIABLE),
        7 => ("#e8912d", CLASS),
        22 => ("#e8912d", STRUCT),
        8 => ("#1aa39a", INTERFACE),
        25 => ("#1aa39a", TYPE),
        9 => ("#7a8291", MODULE),
        13 | 20 => ("#d17c0f", ENUM),
        11 | 12 | 21 => ("#0e9fd8", CONSTANT),
        14 => ("#6c7686", KEYWORD),
        15 => ("#16a36b", SNIPPET),
        16 => ("#d6458f", COLOR),
        17 | 18 => ("#7a8291", FILE),
        19 => ("#7a8291", FOLDER),
        23 => ("#d4a20b", EVENT),
        24 => ("#6c7686", OPERATOR),
        _ => ("#8a94a6", TEXT),
    }
}

/// O SVG do selo de um tipo de sugestão.
pub fn kind_svg(kind: u8) -> String {
    let (fill, glyph) = kind_badge(kind);
    let glyph = match glyph.strip_prefix('F') {
        Some(d) => format!(r##"<path d="{d}" fill="#fff"/>"##),
        None => format!(r##"<path d="{glyph}" fill="none" stroke="#fff" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round"/>"##),
    };
    format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><rect x="0.5" y="0.5" width="15" height="15" rx="4" fill="{fill}"/>{glyph}</svg>"#)
}

/// Caminho (no cache) do selo de um tipo de sugestão.
pub fn kind_icon(kind: u8) -> Option<String> {
    cached(&format!("kind-{kind}"), kind_svg(kind).as_bytes())
}

/// Nome do tipo de sugestão, em inglês (traduzido na interface).
pub fn kind_name(kind: u8) -> &'static str {
    match kind {
        0 => "Word",
        1 => "Text",
        2 => "Method",
        3 => "Function",
        4 => "Constructor",
        5 => "Field",
        6 => "Variable",
        7 => "Class",
        8 => "Interface",
        9 => "Module",
        10 => "Property",
        11 => "Unit",
        12 => "Value",
        13 => "Enum",
        14 => "Keyword",
        15 => "Snippet",
        16 => "Color",
        17 => "File",
        18 => "Reference",
        19 => "Folder",
        20 => "Enum Member",
        21 => "Constant",
        22 => "Struct",
        23 => "Event",
        24 => "Operator",
        _ => "Type Parameter",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabelas_ordenadas() {
        for t in [generated::EXTENSIONS, generated::FILE_NAMES, generated::FOLDER_NAMES] {
            assert!(t.windows(2).all(|w| w[0].0 < w[1].0));
        }
        assert!(generated::SVGS.windows(2).all(|w| w[0].0 < w[1].0));
    }

    #[test]
    fn icones_de_arquivo_e_pasta() {
        assert_eq!(file_icon_name("HomeController.php"), "php");
        assert_eq!(file_icon_name("main.rs"), "rust");
        assert_eq!(file_icon_name("Cargo.toml"), file_icon_name("cargo.toml"));
        assert_eq!(file_icon_name("sem-extensao"), "file");
        assert_eq!(folder_icon_name("src", false), "folder-src");
        assert_eq!(folder_icon_name("src", true), "folder-src");
        assert_eq!(folder_icon_name("qualquer-coisa", true), "folder-open");
        assert_eq!(folder_icon_name("qualquer-coisa", false), "folder");
        for name in ["php", "rust", "file", "folder", "folder-open", "folder-src"] {
            assert!(svg_of(name).is_some(), "{name}");
        }
    }

    #[test]
    fn selos_validos() {
        for k in 0..=25 {
            let svg = kind_svg(k);
            assert!(resvg::usvg::Tree::from_data(svg.as_bytes(), &Default::default()).is_ok(), "{k}");
        }
    }
}
