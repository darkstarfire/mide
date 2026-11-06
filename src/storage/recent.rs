//! Recentes (barra lateral do Arquivos, como no Finder): os arquivos abertos há pouco,
//! do `~/.local/share/recently-used.xbel` do freedesktop, que os apps GTK, o Firefox, o
//! LibreOffice e o próprio mide (ao abrir pelo Arquivos) atualizam.
//!
//! O formato é XML ("XBEL"); só o atributo `href` e o `modified` de cada `bookmark`
//! interessam aqui, então a leitura é uma busca simples nos atributos, sem um leitor de
//! XML completo.

use std::path::{Path, PathBuf};

use crate::places;

/// Caminho do arquivo dos recentes.
pub fn xbel_path() -> Option<PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .map(|d| d.join("recently-used.xbel"))
}

/// Os `limit` arquivos (e pastas) mais recentes que ainda existem, do mais novo para o
/// mais velho.
pub fn recent_files(limit: usize) -> Vec<PathBuf> {
    let Some(text) = xbel_path().and_then(|p| std::fs::read_to_string(p).ok()) else {
        return Vec::new();
    };
    let mut items = parse(&text);
    // Datas ISO 8601 em UTC: a ordem do texto é a ordem do tempo.
    items.sort_by(|a, b| b.1.cmp(&a.1));
    let mut out: Vec<PathBuf> = Vec::new();
    for (path, _) in items {
        if out.len() >= limit {
            break;
        }
        if !out.contains(&path) && path.exists() {
            out.push(path);
        }
    }
    out
}

/// Pares (caminho, `modified`) dos marcadores de arquivos locais.
pub fn parse(text: &str) -> Vec<(PathBuf, String)> {
    let mut out = Vec::new();
    for chunk in text.split("<bookmark ").skip(1) {
        let head = chunk.split('>').next().unwrap_or("");
        let Some(href) = attr(head, "href") else {
            continue;
        };
        let Some(path) = places::path_from_uri(&unescape(&href)) else {
            continue;
        };
        let when = attr(head, "modified").or_else(|| attr(head, "visited")).or_else(|| attr(head, "added")).unwrap_or_default();
        out.push((path, when));
    }
    out
}

/// Valor de `name="..."` dentro da marca.
fn attr(head: &str, name: &str) -> Option<String> {
    let key = format!(" {name}=\"");
    let start = format!(" {head}").find(&key)? + key.len() - 1;
    let rest = &head[start..];
    Some(rest[..rest.find('"')?].to_string())
}

fn unescape(s: &str) -> String {
    s.replace("&amp;", "&").replace("&apos;", "'").replace("&quot;", "\"").replace("&lt;", "<").replace("&gt;", ">")
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Registra `path` como aberto agora pelo app `app` (o Arquivos chama ao abrir um
/// arquivo): atualiza a data se já está na lista, senão acrescenta.
pub fn add(path: &Path, app: &str) {
    let Some(file) = xbel_path() else {
        return;
    };
    let text = std::fs::read_to_string(&file).unwrap_or_default();
    let updated = add_to(&text, path, app, &chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string());
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // Grava num temporário e troca, para um app lendo ao mesmo tempo não ver o meio.
    let tmp = file.with_extension("xbel.mide-tmp");
    if std::fs::write(&tmp, updated).is_ok() {
        let _ = std::fs::rename(&tmp, &file);
    }
}

/// O texto do XBEL com `path` acrescentado ou com a data atualizada para `now`.
pub fn add_to(text: &str, path: &Path, app: &str, now: &str) -> String {
    let href = escape(&places::file_uri(path));
    let mut text = if text.contains("<xbel") {
        text.to_string()
    } else {
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<xbel version=\"1.0\"\n      xmlns:bookmark=\"http://www.freedesktop.org/standards/desktop-bookmarks\"\n      xmlns:mime=\"http://www.freedesktop.org/standards/shared-mime-info\"\n>\n</xbel>\n".to_string()
    };
    let needle = format!("href=\"{href}\"");
    if let Some(at) = text.find(&needle) {
        // Já está: troca o `modified` e o `visited` dessa marca.
        let end = at + text[at..].find('>').unwrap_or(0);
        let head = text[at..end].to_string();
        let mut new_head = head.clone();
        for key in ["modified", "visited"] {
            if let Some(old) = attr(&format!(" {head}"), key) {
                new_head = new_head.replace(&format!("{key}=\"{old}\""), &format!("{key}=\"{now}\""));
            }
        }
        text.replace_range(at..end, &new_head);
        return text;
    }
    let mime = crate::thumbnails::mime_for(path).unwrap_or_else(|| "application/octet-stream".into());
    let entry = format!(
        "  <bookmark href=\"{href}\" added=\"{now}\" modified=\"{now}\" visited=\"{now}\">\n    <info>\n      <metadata owner=\"http://freedesktop.org\">\n        <mime:mime-type type=\"{mime}\"/>\n        <bookmark:applications>\n          <bookmark:application name=\"{app}\" exec=\"&apos;{app} %u&apos;\" modified=\"{now}\" count=\"1\"/>\n        </bookmark:applications>\n      </metadata>\n    </info>\n  </bookmark>\n",
        mime = escape(&mime),
        app = escape(app),
    );
    match text.rfind("</xbel>") {
        Some(at) => text.insert_str(at, &entry),
        None => text.push_str(&entry),
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_e_atualiza_o_xbel() {
        let text = "<?xml version=\"1.0\"?>\n<xbel version=\"1.0\">\n  <bookmark href=\"file:///tmp/a%20b.txt\" added=\"2026-01-01T00:00:00Z\" modified=\"2026-01-02T00:00:00Z\" visited=\"2026-01-02T00:00:00Z\">\n  </bookmark>\n  <bookmark href=\"https://example.com/\" modified=\"2026-01-03T00:00:00Z\"/>\n</xbel>\n";
        let items = parse(text);
        assert_eq!(items, vec![(PathBuf::from("/tmp/a b.txt"), "2026-01-02T00:00:00Z".to_string())]);
        // Atualizar uma que já existe troca as datas; uma nova entra antes do fim.
        let t = add_to(text, Path::new("/tmp/a b.txt"), "mide-files", "2026-11-05T10:00:00Z");
        assert!(t.contains("modified=\"2026-11-05T10:00:00Z\" visited=\"2026-11-05T10:00:00Z\""));
        let t = add_to(&t, Path::new("/tmp/novo.txt"), "mide-files", "2026-11-05T11:00:00Z");
        let items = parse(&t);
        assert_eq!(items.len(), 2);
        assert!(t.trim_end().ends_with("</xbel>"));
        assert_eq!(items[1], (PathBuf::from("/tmp/novo.txt"), "2026-11-05T11:00:00Z".to_string()));
        // Sem arquivo ainda: cria o XBEL inteiro.
        let fresh = add_to("", Path::new("/tmp/x.txt"), "mide-files", "2026-11-05T12:00:00Z");
        assert!(fresh.starts_with("<?xml") && parse(&fresh).len() == 1);
    }
}
