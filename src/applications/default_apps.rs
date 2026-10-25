//! Apps padrão (Configurações → Apps padrão): qual app abre páginas da web, e-mails,
//! pastas, textos, imagens, vídeos, músicas e PDFs.
//!
//! A escolha fica no `~/.config/mide-mimeapps.list` (seção `[Default Applications]`):
//! pelo padrão do freedesktop, o `gio open`, o `xdg-open` e os navegadores leem
//! `<desktop>-mimeapps.list` antes do `mimeapps.list` em cada pasta, e só quando
//! `XDG_CURRENT_DESKTOP` traz esse desktop. Assim a escolha feita no mide vale só na
//! sessão do mide; o Cinnamon e o GNOME (que usam o `mimeapps.list` geral) não mudam.
//! Escolhas antigas do `mimeapps.list` geral continuam valendo como reserva.

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
    Category {
        key: "documents",
        label: "Documentos",
        mimes: &["application/vnd.openxmlformats-officedocument.wordprocessingml.document", "application/vnd.oasis.opendocument.text", "text/rtf", "application/rtf"],
    },
    Category { key: "image", label: "Imagens", mimes: &["image/png", "image/jpeg", "image/gif", "image/webp", "image/bmp", "image/svg+xml"] },
    Category { key: "video", label: "Vídeos", mimes: &["video/mp4", "video/x-matroska", "video/webm", "video/quicktime", "video/x-msvideo"] },
    Category { key: "music", label: "Músicas", mimes: &["audio/mpeg", "audio/flac", "audio/ogg", "audio/x-wav", "audio/mp4", "audio/x-opus+ogg"] },
    Category { key: "pdf", label: "PDF", mimes: &["application/pdf"] },
];

/// Nome do arquivo de padrões que só vale na sessão do mide (`XDG_CURRENT_DESKTOP=mide`).
pub const SESSION_FILE: &str = "mide-mimeapps.list";

/// Pasta de configuração do usuário (`XDG_CONFIG_HOME` ou `~/.config`).
fn config_home() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
}

/// `~/.config/mide-mimeapps.list` (respeitando `XDG_CONFIG_HOME`): onde o mide grava.
pub fn user_file() -> Option<PathBuf> {
    Some(config_home()?.join(SESSION_FILE))
}

/// Arquivos com padrões, do mais importante para o menos, na ordem do GLib: em cada
/// pasta, primeiro o do mide e depois o geral; as pastas são a do usuário, as do
/// sistema (`XDG_CONFIG_DIRS`) e as de apps (`XDG_DATA_HOME` e `XDG_DATA_DIRS`, com o
/// antigo `defaults.list` no fim de cada uma).
fn lookup_files() -> Vec<PathBuf> {
    let config_dirs = std::env::var("XDG_CONFIG_DIRS").ok().filter(|v| !v.is_empty()).unwrap_or_else(|| "/etc/xdg".into());
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")));
    let data_dirs = std::env::var("XDG_DATA_DIRS").ok().filter(|v| !v.is_empty()).unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    lookup_files_in(
        config_home(),
        config_dirs.split(':').filter(|d| !d.is_empty()).map(PathBuf::from).collect(),
        data_home.into_iter().chain(data_dirs.split(':').filter(|d| !d.is_empty()).map(PathBuf::from)).collect(),
    )
}

fn lookup_files_in(config_home: Option<PathBuf>, config_dirs: Vec<PathBuf>, data_dirs: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for dir in config_home.into_iter().chain(config_dirs) {
        files.push(dir.join(SESSION_FILE));
        files.push(dir.join("mimeapps.list"));
    }
    for dir in data_dirs {
        let apps = dir.join("applications");
        files.push(apps.join(SESSION_FILE));
        files.push(apps.join("mimeapps.list"));
        files.push(apps.join("defaults.list"));
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

/// App padrão escolhido para o mide (só o `mide-mimeapps.list` do usuário). O
/// `mimeapps.list` geral fica de fora de propósito: nele estão as escolhas de outros
/// desktops (Dolphin, TextEditor), que não devem tirar pastas e textos dos apps do mide.
pub fn user_choice(mime: &str) -> Option<String> {
    fs::read_to_string(user_file()?).ok().and_then(|t| default_in_text(&t, mime))
}

/// Grava o app padrão de um tipo de arquivo só (Obter informações → Abrir com →
/// "Alterar todos").
pub fn set_for_mime(mime: &str, desktop_id: &str) -> std::io::Result<()> {
    let path = user_file().ok_or_else(|| std::io::Error::other("sem pasta de configuração"))?;
    write_defaults(&path, &[mime], desktop_id)
}

/// Grava o app padrão de uma categoria no arquivo do mide.
pub fn set(category: &Category, desktop_id: &str) -> std::io::Result<()> {
    let path = user_file().ok_or_else(|| std::io::Error::other("sem pasta de configuração"))?;
    write_defaults(&path, category.mimes, desktop_id)
}

/// Grava `desktop_id` para `mimes` no arquivo `path` (troca atômica).
fn write_defaults(path: &Path, mimes: &[&str], desktop_id: &str) -> std::io::Result<()> {
    let text = fs::read_to_string(path).unwrap_or_default();
    let updated = set_in_text(&text, mimes, desktop_id);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("list.mide");
    fs::write(&tmp, updated)?;
    fs::rename(&tmp, path)
}

/// Apps do mide que são o padrão da sessão: pastas no Arquivos e textos no Editor de
/// Texto. Imagens, compactados e outros entram quando o mide tiver o app (B23, E4).
pub const SESSION_DEFAULTS: &[(&str, &str)] = &[("files", "mide-files.desktop"), ("text", "mide-text.desktop"), ("documents", "mide-text.desktop"), ("video", "mide-player.desktop")];

/// Na sessão do mide: grava no `mide-mimeapps.list` os apps do mide para os tipos que
/// ainda não têm escolha nele (uma escolha feita nas Configurações nunca é trocada).
/// Devolve os tipos gravados.
pub fn ensure_session_defaults() -> std::io::Result<Vec<String>> {
    let path = user_file().ok_or_else(|| std::io::Error::other("sem pasta de configuração"))?;
    ensure_defaults_in(&path)
}

fn ensure_defaults_in(path: &Path) -> std::io::Result<Vec<String>> {
    let text = fs::read_to_string(path).unwrap_or_default();
    let mut written = Vec::new();
    for (key, desktop_id) in SESSION_DEFAULTS {
        let Some(category) = CATEGORIES.iter().find(|c| c.key == *key) else {
            continue;
        };
        let missing: Vec<&str> = category.mimes.iter().copied().filter(|m| default_in_text(&text, m).is_none()).collect();
        if missing.is_empty() {
            continue;
        }
        write_defaults(path, &missing, desktop_id)?;
        written.extend(missing.iter().map(|m| m.to_string()));
        // Relido: o próximo tipo vê o que acabou de ser gravado.
        return ensure_defaults_in(path).map(|mut more| {
            written.append(&mut more);
            written
        });
    }
    Ok(written)
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

/// Padrões de nome no estilo do shell: `*` (qualquer trecho), `?` (um caractere) e
/// `[abc]`/`[a-z]`/`[!a]` (um caractere da lista). O shared-mime-info e os filtros
/// dos seletores de arquivos (portal) usam esse formato. Compara como recebe (quem
/// chama decide sobre maiúsculas).
pub fn glob_matches(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = name.chars().collect();
    glob_at(&p, &n)
}

fn glob_at(p: &[char], n: &[char]) -> bool {
    match p.first() {
        None => n.is_empty(),
        Some('*') => (0..=n.len()).any(|i| glob_at(&p[1..], &n[i..])),
        Some('?') => !n.is_empty() && glob_at(&p[1..], &n[1..]),
        Some('[') => {
            let Some(close) = p.iter().skip(2).position(|c| *c == ']').map(|i| i + 2) else {
                return n.first() == Some(&'[') && glob_at(&p[1..], &n[1..]);
            };
            let Some(c) = n.first() else {
                return false;
            };
            let set = &p[1..close];
            let (negate, set) = match set.first() {
                Some('!') | Some('^') => (true, &set[1..]),
                _ => (false, set),
            };
            let mut found = false;
            let mut i = 0;
            while i < set.len() {
                if i + 2 < set.len() && set[i + 1] == '-' {
                    found |= set[i] <= *c && *c <= set[i + 2];
                    i += 3;
                } else {
                    found |= set[i] == *c;
                    i += 1;
                }
            }
            found != negate && glob_at(&p[close + 1..], &n[1..])
        }
        Some(c) => n.first() == Some(c) && glob_at(&p[1..], &n[1..]),
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
        // Filtros dos seletores (portal): listas e caracteres soltos.
        assert!(glob_matches("*.[pP][nN][gG]", "foto.PNG"));
        assert!(!glob_matches("*.[pP][nN][gG]", "foto.jpg"));
        assert!(glob_matches("img_??.jpg", "img_01.jpg"));
        assert!(!glob_matches("img_??.jpg", "img_1.jpg"));
        assert!(glob_matches("[!.]*", "a.txt"));
        assert!(!glob_matches("[!.]*", ".oculto"));
        assert!(glob_matches("*.tar.*", "a.tar.gz"));
        assert!(glob_matches("*", ""));
        assert!(glob_matches("*.[0-9]", "log.7"));
    }

    #[test]
    fn ordem_de_leitura_como_o_glib() {
        let files = lookup_files_in(Some("/c".into()), vec!["/etc/xdg".into()], vec!["/d".into()]);
        let names: Vec<String> = files.iter().map(|f| f.display().to_string()).collect();
        assert_eq!(
            names,
            [
                "/c/mide-mimeapps.list",
                "/c/mimeapps.list",
                "/etc/xdg/mide-mimeapps.list",
                "/etc/xdg/mimeapps.list",
                "/d/applications/mide-mimeapps.list",
                "/d/applications/mimeapps.list",
                "/d/applications/defaults.list",
            ]
        );
    }

    #[test]
    fn padroes_da_sessao_sem_trocar_a_escolha() {
        let dir = std::env::temp_dir().join(format!("mide-apps-padrao-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(SESSION_FILE);
        // O usuário já escolheu o Dolphin para pastas no mide e um editor para Markdown.
        fs::write(&path, "[Default Applications]\ninode/directory=org.kde.dolphin.desktop\ntext/markdown=x.desktop\n").unwrap();
        let written = ensure_defaults_in(&path).unwrap();
        assert_eq!(&written[..2], ["text/plain", "text/x-log"]);
        // Documentos (DOCX, ODT, RTF) também vão para o Editor de Texto.
        assert!(written.iter().any(|m| m == "application/vnd.oasis.opendocument.text"));
        let text = fs::read_to_string(&path).unwrap();
        assert_eq!(default_in_text(&text, "inode/directory").as_deref(), Some("org.kde.dolphin.desktop"));
        assert_eq!(default_in_text(&text, "text/markdown").as_deref(), Some("x.desktop"));
        assert_eq!(default_in_text(&text, "text/plain").as_deref(), Some("mide-text.desktop"));
        // Segunda vez: nada muda.
        assert!(ensure_defaults_in(&path).unwrap().is_empty());
        // Arquivo novo: pastas e textos com os apps do mide.
        let fresh = dir.join("novo").join(SESSION_FILE);
        let written = ensure_defaults_in(&fresh).unwrap();
        assert!(written.contains(&"inode/directory".to_string()));
        let text = fs::read_to_string(&fresh).unwrap();
        assert_eq!(default_in_text(&text, "inode/directory").as_deref(), Some("mide-files.desktop"));
        let _ = fs::remove_dir_all(&dir);
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
