//! Pastas especiais mostradas na dock: Downloads (itens recentes) e Lixeira.
//!
//! A pasta de Downloads vem de `user-dirs.dirs` (como no GNOME); a Lixeira segue a
//! especificação do freedesktop (`$XDG_DATA_HOME/Trash`, com `files/` e `info/`).

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::SystemTime,
};

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

fn config_home() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".config"))
}

/// Pasta dos dados do usuário (`$XDG_DATA_HOME`, ou `~/.local/share`).
pub fn data_home() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".local/share"))
}

/// Valor de `XDG_<NOME>_DIR` num texto no formato de `user-dirs.dirs`.
pub fn parse_user_dir(text: &str, key: &str, home: &Path) -> Option<PathBuf> {
    let wanted = format!("XDG_{key}_DIR=");
    let line = text.lines().map(str::trim).find(|l| l.starts_with(&wanted))?;
    let value = line[wanted.len()..].trim().trim_matches('"');
    if let Some(rest) = value.strip_prefix("$HOME") {
        Some(home.join(rest.trim_start_matches('/')))
    } else if value.starts_with('/') {
        Some(PathBuf::from(value))
    } else {
        None
    }
}

/// Pasta de Downloads do usuário.
pub fn downloads_dir() -> PathBuf {
    let home = home();
    fs::read_to_string(config_home().join("user-dirs.dirs"))
        .ok()
        .and_then(|text| parse_user_dir(&text, "DOWNLOAD", &home))
        .unwrap_or_else(|| home.join("Downloads"))
}

/// Pasta de Imagens do usuário (de `user-dirs.dirs`; senão `~/Imagens` ou `~/Pictures`).
pub fn pictures_dir() -> PathBuf {
    let home = home();
    fs::read_to_string(config_home().join("user-dirs.dirs"))
        .ok()
        .and_then(|text| parse_user_dir(&text, "PICTURES", &home))
        .or_else(|| [home.join("Imagens"), home.join("Pictures")].into_iter().find(|p| p.is_dir()))
        .unwrap_or_else(|| home.clone())
}

/// Pasta de vídeos do usuário (`XDG_VIDEOS_DIR`, ou Vídeos/Videos na pasta pessoal).
pub fn videos_dir() -> PathBuf {
    let home = home();
    fs::read_to_string(config_home().join("user-dirs.dirs"))
        .ok()
        .and_then(|text| parse_user_dir(&text, "VIDEOS", &home))
        .or_else(|| [home.join("Vídeos"), home.join("Videos")].into_iter().find(|p| p.is_dir()))
        .unwrap_or_else(|| home.join("Vídeos"))
}

/// Pasta da área de trabalho (os ícones na tela). `MIDE_DESKTOP_DIR` troca a pasta
/// (os testes usam uma pasta temporária). Sem pasta, `None` (não há o que mostrar).
pub fn desktop_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("MIDE_DESKTOP_DIR") {
        return Some(PathBuf::from(dir));
    }
    let home = home();
    fs::read_to_string(config_home().join("user-dirs.dirs"))
        .ok()
        .and_then(|text| parse_user_dir(&text, "DESKTOP", &home))
        .or_else(|| [home.join("Área de trabalho"), home.join("Desktop")].into_iter().find(|p| p.is_dir()))
        .filter(|p| *p != home && p.is_dir())
}

/// Pasta pessoal do usuário.
pub fn home_dir() -> PathBuf {
    home()
}

/// Pasta onde ficam os papéis de parede adicionados pelo usuário.
pub fn user_backgrounds_dir() -> PathBuf {
    data_home().join("backgrounds")
}

/// É uma imagem que o mide abre (PNG ou JPEG)?
pub fn is_image(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| matches!(e.to_lowercase().as_str(), "jpg" | "jpeg" | "png"))
}

/// Pastas e imagens de `dir` (sem ocultos), pastas primeiro, em ordem alfabética.
pub fn folder_images(dir: &Path) -> Vec<(PathBuf, bool)> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut list: Vec<(PathBuf, bool)> = entries
        .flatten()
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .filter_map(|e| {
            let path = e.path();
            let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
            (is_dir || is_image(&path)).then_some((path, is_dir))
        })
        .collect();
    list.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.file_name().map(|n| n.to_string_lossy().to_lowercase()).cmp(&b.0.file_name().map(|n| n.to_string_lossy().to_lowercase()))));
    list
}

/// Copia uma imagem para os papéis de parede do usuário (nome repetido ganha " 2").
/// Devolve o caminho da cópia.
pub fn add_background(image: &Path) -> std::io::Result<PathBuf> {
    let dir = user_backgrounds_dir();
    fs::create_dir_all(&dir)?;
    let name = image.file_name().ok_or_else(|| std::io::Error::other("caminho sem nome"))?.to_string_lossy().into_owned();
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) => (s.to_string(), format!(".{e}")),
        None => (name.clone(), String::new()),
    };
    let mut target = dir.join(&name);
    let mut n = 2;
    while target.exists() {
        target = dir.join(format!("{stem} {n}{ext}"));
        n += 1;
    }
    fs::copy(image, &target)?;
    Ok(target)
}

/// Um item de uma pasta, para listar na dock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub modified: SystemTime,
}

/// Os `limit` itens mais recentes de `dir` (sem ocultos), do mais novo para o mais antigo.
pub fn recent_items(dir: &Path, limit: usize) -> Vec<Item> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut items: Vec<Item> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                return None;
            }
            let meta = entry.metadata().ok()?;
            Some(Item {
                path: entry.path(),
                name,
                is_dir: meta.is_dir(),
                modified: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            })
        })
        .collect();
    items.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| a.name.cmp(&b.name)));
    items.truncate(limit);
    items
}

/// Há quanto tempo, em texto curto ("agora", "há 5 min", "há 3 h", "há 2 dias").
pub fn age_text(modified: SystemTime, now: SystemTime) -> String {
    let secs = now.duration_since(modified).map(|d| d.as_secs()).unwrap_or(0);
    match secs {
        0..=59 => "agora".into(),
        60..=3599 => format!("há {} min", secs / 60),
        3600..=86_399 => format!("há {} h", secs / 3600),
        86_400..=172_799 => "ontem".into(),
        _ => format!("há {} dias", secs / 86_400),
    }
}

/// Nome do ícone do tema para um item, pela extensão.
pub fn icon_name_for(item: &Item) -> &'static str {
    if item.is_dir {
        return "folder";
    }
    let ext = item
        .path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" | "heic" => "image-x-generic",
        "mp4" | "mkv" | "webm" | "avi" | "mov" => "video-x-generic",
        "mp3" | "ogg" | "flac" | "wav" | "m4a" | "opus" => "audio-x-generic",
        "pdf" => "application-pdf",
        "zip" | "tar" | "gz" | "xz" | "bz2" | "7z" | "rar" | "zst" | "tgz" => "package-x-generic",
        "appimage" => "application-vnd.appimage",
        "deb" => "application-vnd.debian.binary-package",
        "rpm" => "application-x-rpm",
        "flatpakref" => "application-x-executable",
        "html" | "htm" => "text-html",
        "doc" | "docx" | "odt" | "rtf" => "x-office-document",
        "xls" | "xlsx" | "ods" | "csv" => "x-office-spreadsheet",
        "ppt" | "pptx" | "odp" => "x-office-presentation",
        _ => "text-x-generic",
    }
}

/// Pasta da Lixeira do usuário.
pub fn trash_dir() -> PathBuf {
    data_home().join("Trash")
}

/// Quantos itens há na Lixeira.
pub fn trash_count(trash: &Path) -> usize {
    fs::read_dir(trash.join("files")).map(|d| d.count()).unwrap_or(0)
}

/// Apaga para sempre tudo o que está na Lixeira (`files/` e `info/`). Devolve
/// quantos itens foram apagados.
pub fn empty_trash(trash: &Path) -> std::io::Result<usize> {
    let mut removed = 0;
    for sub in ["files", "info", "expunged"] {
        let Ok(entries) = fs::read_dir(trash.join(sub)) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            // Links não são seguidos: só o próprio link é apagado.
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            if is_dir {
                fs::remove_dir_all(&path)?;
            } else {
                fs::remove_file(&path)?;
            }
            if sub == "files" {
                removed += 1;
            }
        }
    }
    if removed > 0 {
        crate::sounds::play("trash-empty");
    }
    Ok(removed)
}

/// Codifica um caminho para o campo `Path=` do `.trashinfo` (como numa URL).
fn encode_path(path: &Path) -> String {
    let mut out = String::new();
    for byte in path.to_string_lossy().as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => out.push(*byte as char),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Move um arquivo ou pasta para a Lixeira, pela especificação do freedesktop:
/// o item vai para `files/` e um `.trashinfo` em `info/` guarda o caminho
/// original e a data, para poder restaurar. Nomes repetidos ganham " 2", " 3"...
/// Devolve o nome usado na Lixeira.
pub fn move_to_trash(path: &Path, trash: &Path) -> std::io::Result<String> {
    let path = fs::canonicalize(path)?;
    let files = trash.join("files");
    let info = trash.join("info");
    fs::create_dir_all(&files)?;
    fs::create_dir_all(&info)?;
    let original = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or_else(|| std::io::Error::other("caminho sem nome"))?;
    let (stem, ext) = match original.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem.to_string(), format!(".{ext}")),
        _ => (original.clone(), String::new()),
    };
    let mut name = original.clone();
    let mut n = 2;
    // `create_new` reserva o nome no info/ sem corrida com outro programa.
    let mut info_file = loop {
        match fs::File::options().write(true).create_new(true).open(info.join(format!("{name}.trashinfo"))) {
            Ok(file) if !files.join(&name).exists() => break file,
            Ok(_) => {
                let _ = fs::remove_file(info.join(format!("{name}.trashinfo")));
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(err) => return Err(err),
        }
        name = format!("{stem} {n}{ext}");
        n += 1;
    };
    let date = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S");
    use std::io::Write;
    write!(info_file, "[Trash Info]\nPath={}\nDeletionDate={date}\n", encode_path(&path))?;
    if let Err(err) = fs::rename(&path, files.join(&name)) {
        let _ = fs::remove_file(info.join(format!("{name}.trashinfo")));
        return Err(err);
    }
    crate::sounds::play("trash-move");
    Ok(name)
}

/// Um item da Lixeira: o nome em `files/`, de onde veio e quando foi apagado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrashItem {
    pub name: String,
    pub original: PathBuf,
    pub deleted: SystemTime,
    pub is_dir: bool,
}

/// Desfaz a codificação `%XX` do campo `Path=` do `.trashinfo`.
fn decode_path(value: &str) -> String {
    String::from_utf8_lossy(&percent_decode(value)).into_owned()
}

/// Bytes de um texto com `%XX` (como numa URL) já decodificados.
fn percent_decode(value: &str) -> Vec<u8> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = if bytes[i] == b'%' { value.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok()) } else { None };
        if let Some(byte) = hex {
            out.push(byte);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

/// Endereço `file://` de um caminho, com `%XX` nos bytes fora do conjunto seguro
/// (espaços, acentos, `#`, `?`), byte a byte: nomes que não são UTF-8 também voltam iguais.
pub fn file_uri(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut out = String::from("file://");
    for byte in path.as_os_str().as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' | b'!' | b'$' | b'&' | b'\'' | b'(' | b')' | b'*' | b'+' | b',' | b';' | b'=' | b':' | b'@' => {
                out.push(*byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Caminho de um endereço `file://` (com ou sem `localhost`) ou de um caminho comum
/// (começando com `/`). Outros endereços (`trash:`, `http:`, `sftp:`) dão `None`.
pub fn path_from_uri(uri: &str) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    if uri.starts_with('/') {
        return Some(PathBuf::from(uri));
    }
    let rest = uri.strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    if !rest.starts_with('/') {
        return None;
    }
    // Sem a parte `?...` e `#...` (não fazem parte do caminho num endereço de arquivo).
    let rest = rest.split(['?', '#']).next().unwrap_or(rest);
    Some(PathBuf::from(std::ffi::OsStr::from_bytes(&percent_decode(rest))))
}

/// Lê um `.trashinfo`: caminho original e data em que foi apagado.
fn read_trashinfo(path: &Path) -> Option<(PathBuf, Option<SystemTime>)> {
    let text = fs::read_to_string(path).ok()?;
    let original = text.lines().find_map(|l| l.strip_prefix("Path=")).map(|p| PathBuf::from(decode_path(p.trim())))?;
    let deleted = text
        .lines()
        .find_map(|l| l.strip_prefix("DeletionDate="))
        .and_then(|d| chrono::NaiveDateTime::parse_from_str(d.trim(), "%Y-%m-%dT%H:%M:%S").ok())
        .and_then(|d| d.and_local_timezone(chrono::Local).single())
        .map(SystemTime::from);
    Some((original, deleted))
}

/// Os `limit` itens apagados mais recentemente, do mais novo para o mais antigo.
pub fn trash_items(trash: &Path, limit: usize) -> Vec<TrashItem> {
    let Ok(entries) = fs::read_dir(trash.join("files")) else {
        return Vec::new();
    };
    let mut items: Vec<TrashItem> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let info = trash.join("info").join(format!("{name}.trashinfo"));
            let (original, deleted) = read_trashinfo(&info).unwrap_or_else(|| (PathBuf::from(&name), None));
            let deleted = deleted
                .or_else(|| fs::metadata(&info).and_then(|m| m.modified()).ok())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            Some(TrashItem {
                is_dir: entry.file_type().map(|t| t.is_dir()).unwrap_or(false),
                name,
                original,
                deleted,
            })
        })
        .collect();
    items.sort_by(|a, b| b.deleted.cmp(&a.deleted).then_with(|| a.name.cmp(&b.name)));
    items.truncate(limit);
    items
}

/// Devolve um item da Lixeira para onde ele estava. Se já existir algo com o mesmo
/// nome lá, o restaurado ganha " 2", " 3"... Devolve o caminho final.
pub fn restore_from_trash(trash: &Path, name: &str) -> std::io::Result<PathBuf> {
    let info = trash.join("info").join(format!("{name}.trashinfo"));
    let (original, _) = read_trashinfo(&info).ok_or_else(|| std::io::Error::other("item sem .trashinfo"))?;
    // Caminho relativo não diz para onde voltar: restaurar criaria o arquivo na
    // pasta atual de quem chamou.
    if !original.is_absolute() {
        return Err(std::io::Error::other("caminho original inválido no .trashinfo"));
    }
    if let Some(parent) = original.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut target = original.clone();
    let mut n = 2;
    while target.exists() {
        let stem = original.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let ext = original.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
        target = original.with_file_name(format!("{stem} {n}{ext}"));
        n += 1;
    }
    fs::rename(trash.join("files").join(name), &target)?;
    let _ = fs::remove_file(&info);
    Ok(target)
}

/// Apaga para sempre um item da Lixeira.
pub fn delete_from_trash(trash: &Path, name: &str) -> std::io::Result<()> {
    let path = trash.join("files").join(name);
    let is_dir = fs::symlink_metadata(&path)?.is_dir();
    if is_dir {
        fs::remove_dir_all(&path)?;
    } else {
        fs::remove_file(&path)?;
    }
    let _ = fs::remove_file(trash.join("info").join(format!("{name}.trashinfo")));
    Ok(())
}

/// Copia um texto para a área de transferência: o compositor guarda o texto (continua
/// lá mesmo que o app feche); sem o mide, o `wl-copy`. Com `MIDE_OPEN_DRY_RUN`
/// definido (testes), também registra no log e não usa o `wl-copy`.
pub fn copy_text(text: &str) {
    // Nos testes, registra e copia no mide de teste (copiar não mexe em nada fora dele).
    let dry_run = std::env::var_os("MIDE_OPEN_DRY_RUN").is_some();
    if dry_run {
        tracing::info!("copiar (teste): {text}");
    }
    // Uma linha só no canal de controle.
    let line = text.replace(['\n', '\r'], " ");
    if crate::control_client::ask_compositor(&format!("copy {line}")).is_some() {
        tracing::info!("copiado: {text}");
        return;
    }
    if dry_run {
        return;
    }
    match Command::new("wl-copy").arg("--").arg(text).spawn() {
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            tracing::info!("copiado: {text}");
        }
        Err(err) => tracing::warn!("falha ao copiar (instale wl-clipboard): {err}"),
    }
}

/// Abre o Terminal do mide na pasta `dir`. Com
/// `MIDE_OPEN_DRY_RUN` definido (testes), só registra no log.
pub fn open_terminal(dir: &Path) {
    if std::env::var_os("MIDE_OPEN_DRY_RUN").is_some() {
        tracing::info!("terminal (teste): {}", dir.display());
        return;
    }
    let quoted = format!("'{}'", dir.to_string_lossy().replace('\'', "'\\''"));
    let command = format!("mide-terminal -D {quoted}");
    match Command::new("sh").arg("-c").arg(&command).spawn() {
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            tracing::info!("terminal em {}", dir.display());
        }
        Err(err) => tracing::warn!("falha ao abrir o terminal: {err}"),
    }
}

/// Um programa do mide: o da mesma pasta do programa atual (o build em uso), senão o
/// do `PATH`.
pub fn mide_program(name: &str) -> PathBuf {
    std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join(name))).filter(|p| p.exists()).unwrap_or_else(|| PathBuf::from(name))
}

/// Abre um arquivo, pasta ou endereço (como `trash:///`) com o app padrão.
/// Com `MIDE_OPEN_DRY_RUN` definido (testes), só registra no log.
pub fn open(target: &str) {
    if std::env::var_os("MIDE_OPEN_DRY_RUN").is_some() {
        tracing::info!("abrir (teste): {target}");
        return;
    }
    // Pastas e a Lixeira abrem no Arquivos do mide, e arquivos de texto no Editor de
    // Texto do mide (quando instalados).
    // Se o usuário escolheu outro app padrão (Configurações → Apps padrão), vale ele.
    let path = Path::new(target.strip_prefix("file://").unwrap_or(target));
    let chosen_other = |mime: &str, ours: &str| crate::default_apps::user_choice(mime).is_some_and(|id| id != ours);
    let native = if target.starts_with("trash:") || (path.is_dir() && !chosen_other("inode/directory", "mide-files.desktop")) {
        Some(("mide-files", "Arquivos"))
    } else if crate::files::is_text(path) && !chosen_other("text/plain", "mide-text.desktop") {
        Some(("mide-text", "Editor de Texto"))
    } else {
        None
    };
    if let Some((name, app)) = native {
        let sibling = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join(name))).filter(|p| p.exists());
        let program = sibling.unwrap_or_else(|| PathBuf::from(name));
        let arg = if target.starts_with("trash:") { target } else { path.to_str().unwrap_or(target) };
        if let Ok(mut child) = Command::new(program).arg(arg).spawn() {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            tracing::info!("abrindo {target} no {app}");
            return;
        }
    }
    let quoted = format!("'{}'", target.replace('\'', "'\\''"));
    let command = format!("gio open {quoted} || xdg-open {quoted}");
    match Command::new("sh").arg("-c").arg(&command).spawn() {
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            tracing::info!("abrindo {target}");
        }
        Err(err) => tracing::warn!("falha ao abrir {target}: {err}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mide-places-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn enderecos_de_arquivo() {
        use std::os::unix::ffi::OsStrExt;
        let path = Path::new("/home/ana/Área de trabalho/nota #1.txt");
        let uri = file_uri(path);
        assert_eq!(uri, "file:///home/ana/%C3%81rea%20de%20trabalho/nota%20%231.txt");
        assert_eq!(path_from_uri(&uri).as_deref(), Some(path));
        assert_eq!(path_from_uri("file://localhost/tmp/a%20b").as_deref(), Some(Path::new("/tmp/a b")));
        assert_eq!(path_from_uri("/tmp/x").as_deref(), Some(Path::new("/tmp/x")));
        assert_eq!(path_from_uri("trash:///a"), None);
        assert_eq!(path_from_uri("https://x/a"), None);
        // %XX no fim do texto também decodifica; nome que não é UTF-8 volta igual.
        assert_eq!(path_from_uri("file:///tmp/a%41").as_deref(), Some(Path::new("/tmp/aA")));
        let odd = PathBuf::from(std::ffi::OsStr::from_bytes(b"/tmp/\xff.txt"));
        assert_eq!(path_from_uri(&file_uri(&odd)), Some(odd));
    }

    #[test]
    fn pasta_de_downloads_do_user_dirs() {
        let home = Path::new("/home/ana");
        let text = "# comentário\nXDG_DESKTOP_DIR=\"$HOME/Área de trabalho\"\nXDG_DOWNLOAD_DIR=\"$HOME/Baixados\"\n";
        assert_eq!(parse_user_dir(text, "DOWNLOAD", home), Some(PathBuf::from("/home/ana/Baixados")));
        assert_eq!(parse_user_dir("XDG_DOWNLOAD_DIR=\"/dados/dl\"", "DOWNLOAD", home), Some(PathBuf::from("/dados/dl")));
        assert_eq!(parse_user_dir("", "DOWNLOAD", home), None);
    }

    #[test]
    fn recentes_do_mais_novo_sem_ocultos() {
        let dir = temp_dir("recentes");
        for (i, name) in ["velho.txt", "meio.pdf", "novo.png", ".oculto"].iter().enumerate() {
            let path = dir.join(name);
            fs::write(&path, "x").unwrap();
            let time = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000 + i as u64 * 100);
            fs::File::options().write(true).open(&path).unwrap().set_modified(time).unwrap();
        }
        let names: Vec<String> = recent_items(&dir, 10).into_iter().map(|i| i.name).collect();
        assert_eq!(names, vec!["novo.png", "meio.pdf", "velho.txt"]);
        assert_eq!(recent_items(&dir, 2).len(), 2);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn idade_em_texto() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000_000);
        let ago = |s: u64| now - Duration::from_secs(s);
        assert_eq!(age_text(ago(5), now), "agora");
        assert_eq!(age_text(ago(300), now), "há 5 min");
        assert_eq!(age_text(ago(7200), now), "há 2 h");
        assert_eq!(age_text(ago(90_000), now), "ontem");
        assert_eq!(age_text(ago(3 * 86_400), now), "há 3 dias");
    }

    #[test]
    fn icone_pela_extensao() {
        let item = |name: &str, is_dir: bool| Item {
            path: PathBuf::from(name),
            name: name.into(),
            is_dir,
            modified: SystemTime::UNIX_EPOCH,
        };
        assert_eq!(icon_name_for(&item("foto.JPG", false)), "image-x-generic");
        assert_eq!(icon_name_for(&item("doc.pdf", false)), "application-pdf");
        assert_eq!(icon_name_for(&item("pasta", true)), "folder");
        assert_eq!(icon_name_for(&item("semext", false)), "text-x-generic");
    }

    #[test]
    fn esvaziar_lixeira_apaga_arquivos_e_info() {
        let trash = temp_dir("lixeira");
        fs::create_dir_all(trash.join("files/pasta")).unwrap();
        fs::write(trash.join("files/pasta/dentro.txt"), "x").unwrap();
        fs::write(trash.join("files/a.txt"), "x").unwrap();
        fs::create_dir_all(trash.join("info")).unwrap();
        fs::write(trash.join("info/a.txt.trashinfo"), "x").unwrap();
        fs::write(trash.join("info/pasta.trashinfo"), "x").unwrap();
        assert_eq!(trash_count(&trash), 2);
        assert_eq!(empty_trash(&trash).unwrap(), 2);
        assert_eq!(trash_count(&trash), 0);
        assert_eq!(fs::read_dir(trash.join("info")).unwrap().count(), 0);
        // As pastas da Lixeira continuam existindo.
        assert!(trash.join("files").is_dir());
        fs::remove_dir_all(&trash).unwrap();
    }

    #[test]
    fn mover_para_a_lixeira_com_trashinfo_e_nome_repetido() {
        let base = temp_dir("mover");
        let trash = base.join("Trash");
        fs::create_dir_all(base.join("dl")).unwrap();
        fs::write(base.join("dl/nota fiscal.pdf"), "1").unwrap();
        let name = move_to_trash(&base.join("dl/nota fiscal.pdf"), &trash).unwrap();
        assert_eq!(name, "nota fiscal.pdf");
        assert!(!base.join("dl/nota fiscal.pdf").exists());
        assert!(trash.join("files/nota fiscal.pdf").exists());
        let info = fs::read_to_string(trash.join("info/nota fiscal.pdf.trashinfo")).unwrap();
        assert!(info.starts_with("[Trash Info]\nPath=/"));
        assert!(info.contains("nota%20fiscal.pdf\n"));
        assert!(info.contains("DeletionDate="));
        // Outro arquivo com o mesmo nome ganha " 2".
        fs::write(base.join("dl/nota fiscal.pdf"), "2").unwrap();
        assert_eq!(move_to_trash(&base.join("dl/nota fiscal.pdf"), &trash).unwrap(), "nota fiscal 2.pdf");
        assert_eq!(trash_count(&trash), 2);
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn lixeira_lista_restaura_e_apaga() {
        let base = temp_dir("restaurar");
        let trash = base.join("Trash");
        fs::create_dir_all(base.join("docs")).unwrap();
        fs::write(base.join("docs/carta.txt"), "1").unwrap();
        fs::write(base.join("docs/foto antiga.png"), "2").unwrap();
        move_to_trash(&base.join("docs/carta.txt"), &trash).unwrap();
        move_to_trash(&base.join("docs/foto antiga.png"), &trash).unwrap();
        let items = trash_items(&trash, 10);
        assert_eq!(items.len(), 2);
        let foto = items.iter().find(|i| i.name == "foto antiga.png").unwrap();
        assert!(foto.original.ends_with("docs/foto antiga.png"));
        // Restaurar devolve ao lugar de origem; com o nome ocupado, ganha " 2".
        let back = restore_from_trash(&trash, "foto antiga.png").unwrap();
        assert!(back.ends_with("docs/foto antiga.png"));
        assert!(base.join("docs/foto antiga.png").exists());
        fs::write(base.join("docs/carta.txt"), "novo").unwrap();
        let back = restore_from_trash(&trash, "carta.txt").unwrap();
        assert!(back.ends_with("docs/carta 2.txt"));
        assert_eq!(trash_count(&trash), 0);
        // Apagar permanentemente tira de files/ e de info/.
        move_to_trash(&base.join("docs/carta 2.txt"), &trash).unwrap();
        delete_from_trash(&trash, "carta 2.txt").unwrap();
        assert_eq!(trash_count(&trash), 0);
        assert_eq!(fs::read_dir(trash.join("info")).unwrap().count(), 0);
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn pasta_com_imagens_e_subpastas() {
        let dir = temp_dir("imagens");
        fs::create_dir_all(dir.join("Férias")).unwrap();
        fs::write(dir.join("b.png"), "x").unwrap();
        fs::write(dir.join("A.jpg"), "x").unwrap();
        fs::write(dir.join("notas.txt"), "x").unwrap();
        fs::write(dir.join(".oculta.png"), "x").unwrap();
        let names: Vec<String> = folder_images(&dir).iter().map(|(p, _)| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, vec!["Férias", "A.jpg", "b.png"]);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn lixeira_inexistente_conta_zero() {
        assert_eq!(trash_count(Path::new("/caminho/que/nao/existe")), 0);
        assert_eq!(empty_trash(Path::new("/caminho/que/nao/existe")).unwrap(), 0);
    }
}
