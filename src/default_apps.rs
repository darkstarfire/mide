//! Apps padrão (Configurações → Apps padrão): qual app abre páginas da web, e-mails,
//! pastas, textos, imagens, vídeos, músicas e PDFs.
//!
//! A escolha fica no `~/.config/mimeapps.list` (seção `[Default Applications]`), o
//! arquivo padrão do freedesktop que o `gio open`, o `xdg-open`, os navegadores e o
//! GNOME/Cinnamon também leem e gravam: um app escolhido aqui vale em todo lugar.

use std::{
    fs,
    path::{Path, PathBuf},
};

/// Um tipo de app padrão: chave, nome na tela e os tipos de arquivo (MIME) que ele
/// abre. O primeiro tipo é o que decide quais apps aparecem na lista.
#[derive(Debug, Clone, Copy)]
pub struct Category {
    pub key: &'static str,
    pub label: &'static str,
    pub mimes: &'static [&'static str],
}

pub const CATEGORIES: &[Category] = &[
    Category { key: "web", label: "Navegador", mimes: &["x-scheme-handler/http", "x-scheme-handler/https", "text/html", "application/xhtml+xml"] },
    Category { key: "mail", label: "E-mail", mimes: &["x-scheme-handler/mailto"] },
    Category { key: "files", label: "Pastas", mimes: &["inode/directory"] },
    Category { key: "text", label: "Textos", mimes: &["text/plain", "text/markdown", "text/x-log"] },
    Category { key: "image", label: "Imagens", mimes: &["image/png", "image/jpeg", "image/gif", "image/webp", "image/bmp", "image/svg+xml"] },
    Category { key: "video", label: "Vídeos", mimes: &["video/mp4", "video/x-matroska", "video/webm", "video/quicktime", "video/x-msvideo"] },
    Category { key: "music", label: "Músicas", mimes: &["audio/mpeg", "audio/flac", "audio/ogg", "audio/x-wav", "audio/mp4", "audio/x-opus+ogg"] },
    Category { key: "pdf", label: "PDF", mimes: &["application/pdf"] },
];

/// `~/.config/mimeapps.list` (respeitando `XDG_CONFIG_HOME`).
pub fn user_file() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("mimeapps.list"))
}

/// Arquivos com padrões, do mais importante para o menos: o do usuário, os do sistema
/// (`XDG_CONFIG_DIRS`) e os das pastas de apps (`XDG_DATA_DIRS`, inclusive o antigo
/// `defaults.list`).
fn lookup_files() -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = user_file().into_iter().collect();
    let config_dirs = std::env::var("XDG_CONFIG_DIRS").ok().filter(|v| !v.is_empty()).unwrap_or_else(|| "/etc/xdg".into());
    files.extend(config_dirs.split(':').filter(|d| !d.is_empty()).map(|d| Path::new(d).join("mimeapps.list")));
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")));
    let data_dirs = std::env::var("XDG_DATA_DIRS").ok().filter(|v| !v.is_empty()).unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    for dir in data_home.into_iter().chain(data_dirs.split(':').filter(|d| !d.is_empty()).map(PathBuf::from)) {
        files.push(dir.join("applications/mimeapps.list"));
        files.push(dir.join("applications/defaults.list"));
    }
    files
}

/// O app padrão de `mime` num texto no formato do `mimeapps.list` (o primeiro da lista).
pub fn default_in_text(text: &str, mime: &str) -> Option<String> {
    let mut in_defaults = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_defaults = line == "[Default Applications]";
            continue;
        }
        if !in_defaults {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            if key.trim() == mime {
                return value.split(';').map(str::trim).find(|v| !v.is_empty()).map(str::to_string);
            }
        }
    }
    None
}

/// Grava `desktop_id` como padrão de cada `mime`, mantendo o resto do arquivo (outras
/// seções, outros tipos, comentários).
pub fn set_in_text<'a>(text: &str, mimes: &[&'a str], desktop_id: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut in_defaults = false;
    let mut seen_section = false;
    let mut written: Vec<&'a str> = Vec::new();
    let flush = |out: &mut Vec<String>, written: &mut Vec<&'a str>| {
        for mime in mimes {
            if !written.contains(mime) {
                out.push(format!("{mime}={desktop_id}"));
                written.push(mime);
            }
        }
    };
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            if in_defaults {
                // Fim da seção: os tipos que faltavam entram antes da próxima.
                while out.last().is_some_and(|l| l.trim().is_empty()) {
                    out.pop();
                }
                flush(&mut out, &mut written);
                out.push(String::new());
            }
            in_defaults = trimmed == "[Default Applications]";
            seen_section |= in_defaults;
            out.push(line.to_string());
            continue;
        }
        if in_defaults {
            if let Some((key, _)) = trimmed.split_once('=') {
                let key = key.trim();
                if let Some(mime) = mimes.iter().find(|m| **m == key) {
                    if !written.contains(mime) {
                        out.push(format!("{mime}={desktop_id}"));
                        written.push(mime);
                    }
                    continue;
                }
            }
        }
        out.push(line.to_string());
    }
    if in_defaults {
        flush(&mut out, &mut written);
    } else if !seen_section {
        if out.last().is_some_and(|l| !l.trim().is_empty()) {
            out.push(String::new());
        }
        out.push("[Default Applications]".into());
        flush(&mut out, &mut written);
    }
    let mut text = out.join("\n");
    text.push('\n');
    text
}

/// App padrão de um tipo de arquivo (id do `.desktop`), pelo que o usuário e o sistema
/// escolheram.
pub fn current(mime: &str) -> Option<String> {
    lookup_files().into_iter().find_map(|f| fs::read_to_string(f).ok().and_then(|t| default_in_text(&t, mime)))
}

/// App padrão escolhido pelo próprio usuário (só o arquivo dele).
pub fn user_choice(mime: &str) -> Option<String> {
    fs::read_to_string(user_file()?).ok().and_then(|t| default_in_text(&t, mime))
}

/// Grava o app padrão de uma categoria no arquivo do usuário.
pub fn set(category: &Category, desktop_id: &str) -> std::io::Result<()> {
    let path = user_file().ok_or_else(|| std::io::Error::other("sem pasta de configuração"))?;
    let text = fs::read_to_string(&path).unwrap_or_default();
    let updated = set_in_text(&text, category.mimes, desktop_id);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("list.mide");
    fs::write(&tmp, updated)?;
    fs::rename(&tmp, &path)
}

/// Apps que abrem o primeiro tipo da categoria (para a lista das Configurações).
pub fn candidates<'a>(apps: &'a [crate::apps::App], category: &Category) -> Vec<&'a crate::apps::App> {
    let main = category.mimes[0];
    apps.iter().filter(|a| a.mime.iter().any(|m| m == main)).collect()
}

/// Tipo (MIME) de um arquivo pelo nome: pelas regras do shared-mime-info
/// (`/usr/share/mime/globs2`, a de maior peso e o padrão mais longo); pastas são
/// `inode/directory`.
pub fn mime_for(path: &Path) -> Option<String> {
    if path.is_dir() {
        return Some("inode/directory".into());
    }
    let name = path.file_name()?.to_string_lossy().to_lowercase();
    globs().iter().filter(|(_, pattern, _)| glob_matches(pattern, &name)).max_by_key(|(weight, pattern, _)| (*weight, pattern.len())).map(|(_, _, mime)| mime.clone())
}

/// Regras do shared-mime-info (peso, padrão em minúsculas, tipo), lidas uma vez.
fn globs() -> &'static Vec<(u32, String, String)> {
    static GLOBS: std::sync::OnceLock<Vec<(u32, String, String)>> = std::sync::OnceLock::new();
    GLOBS.get_or_init(|| {
        let data_dirs = std::env::var("XDG_DATA_DIRS").ok().filter(|v| !v.is_empty()).unwrap_or_else(|| "/usr/local/share:/usr/share".into());
        let mut out = Vec::new();
        for dir in data_dirs.split(':').filter(|d| !d.is_empty()).chain(["/usr/share"]) {
            let Ok(text) = fs::read_to_string(Path::new(dir).join("mime/globs2")) else {
                continue;
            };
            for line in text.lines().filter(|l| !l.starts_with('#')) {
                let mut parts = line.splitn(4, ':');
                if let (Some(weight), Some(mime), Some(pattern)) = (parts.next(), parts.next(), parts.next()) {
                    out.push((weight.parse().unwrap_or(50), pattern.to_lowercase(), mime.to_string()));
                }
            }
            if !out.is_empty() {
                break;
            }
        }
        out
    })
}

/// Padrões do shared-mime-info: `*.ext`, nomes exatos e `*` simples.
fn glob_matches(pattern: &str, name: &str) -> bool {
    match pattern.split_once('*') {
        None => pattern == name,
        Some((before, after)) if !after.contains('*') => name.len() >= before.len() + after.len() && name.starts_with(before) && name.ends_with(after),
        _ => false,
    }
}

/// Apps que abrem o tipo do arquivo, o padrão primeiro (para "Abrir com").
pub fn apps_for<'a>(apps: &'a [crate::apps::App], path: &Path) -> Vec<&'a crate::apps::App> {
    let Some(mime) = mime_for(path) else {
        return Vec::new();
    };
    let default = current(&mime).map(|d| d.trim_end_matches(".desktop").to_string());
    let mut list: Vec<&crate::apps::App> = apps.iter().filter(|a| a.mime.iter().any(|m| *m == mime)).collect();
    list.sort_by_key(|a| (Some(&a.id) != default.as_ref(), crate::apps::fold(&a.name)));
    list
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn padroes_de_nome() {
        assert!(glob_matches("*.txt", "notas.txt"));
        assert!(!glob_matches("*.txt", "notas.md"));
        assert!(glob_matches("makefile", "makefile"));
        assert!(glob_matches("readme*", "readme.md"));
    }

    #[test]
    fn le_e_grava_o_mimeapps() {
        let text = "# meu arquivo\n[Added Associations]\ntext/plain=gedit.desktop;\n\n[Default Applications]\ntext/plain=gedit.desktop\nimage/png=eog.desktop\n";
        assert_eq!(default_in_text(text, "text/plain").as_deref(), Some("gedit.desktop"));
        assert_eq!(default_in_text(text, "video/mp4"), None);
        let out = set_in_text(text, &["text/plain", "text/markdown"], "mide-text.desktop");
        assert!(out.contains("# meu arquivo"));
        // A associação extra (outra seção) fica como estava.
        assert!(out.contains("[Added Associations]\ntext/plain=gedit.desktop;"));
        assert_eq!(default_in_text(&out, "text/plain").as_deref(), Some("mide-text.desktop"));
        assert_eq!(default_in_text(&out, "text/markdown").as_deref(), Some("mide-text.desktop"));
        assert_eq!(default_in_text(&out, "image/png").as_deref(), Some("eog.desktop"));
        // Arquivo novo: cria a seção.
        let fresh = set_in_text("", &["inode/directory"], "mide-files.desktop");
        assert_eq!(fresh, "[Default Applications]\ninode/directory=mide-files.desktop\n");
        // Seção no meio do arquivo: os tipos novos entram nela, antes da próxima.
        let middle = set_in_text("[Default Applications]\na/b=x.desktop\n\n[Outra]\nk=v\n", &["c/d"], "y.desktop");
        assert_eq!(default_in_text(&middle, "c/d").as_deref(), Some("y.desktop"));
        assert!(middle.contains("[Outra]\nk=v"));
    }
}
