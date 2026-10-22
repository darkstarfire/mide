//! Regras do gerenciador de arquivos (sem interface): listar pastas, ordenar,
//! tamanhos e datas em português, criar pasta, renomear e tipos de arquivo.

use std::{
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

/// Um item de uma pasta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub size: u64,
    pub modified: SystemTime,
}

impl Entry {
    /// Nome do ícone do tema para o item (pelo tipo).
    pub fn icon_name(&self) -> &'static str {
        crate::places::icon_name_for(&crate::places::Item {
            path: self.path.clone(),
            name: self.name.clone(),
            is_dir: self.is_dir,
            modified: self.modified,
        })
    }

    /// É uma imagem que dá para mostrar em miniatura?
    pub fn is_image(&self) -> bool {
        !self.is_dir && crate::places::is_image(&self.path)
    }
}

/// Itens de `dir`: pastas primeiro, depois arquivos, em ordem alfabética (sem
/// diferenciar maiúsculas nem acentos). Ocultos (com ponto na frente) só com `hidden`.
pub fn list_dir(dir: &Path, hidden: bool) -> std::io::Result<Vec<Entry>> {
    let mut list: Vec<Entry> = fs::read_dir(dir)?
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if !hidden && name.starts_with('.') {
                return None;
            }
            let meta = e.metadata().ok()?;
            Some(Entry {
                is_dir: meta.is_dir(),
                size: if meta.is_dir() { 0 } else { meta.len() },
                modified: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                path: e.path(),
                name,
            })
        })
        .collect();
    list.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| crate::apps::fold(&a.name).cmp(&crate::apps::fold(&b.name))));
    Ok(list)
}

/// Itens cujo nome contém o texto (sem diferenciar maiúsculas nem acentos).
pub fn filter(entries: &[Entry], query: &str) -> Vec<Entry> {
    let q = crate::apps::fold(query.trim());
    if q.is_empty() {
        return entries.to_vec();
    }
    entries.iter().filter(|e| crate::apps::fold(&e.name).contains(&q)).cloned().collect()
}

/// Tamanho legível: "512 bytes", "3,2 KB", "14,8 MB", "1,1 GB".
pub fn size_text(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1000 {
        return if bytes == 1 { "1 byte".into() } else { format!("{bytes} bytes") };
    }
    let mut value = bytes as f64 / 1000.0;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    format!("{:.1} {}", value, UNITS[unit]).replace('.', ",")
}

/// Data de modificação: "hoje, 14:30", "ontem, 09:05", "12 de out." ou "12 de out. de 2024".
pub fn date_text(modified: SystemTime, now: SystemTime) -> String {
    use chrono::{DateTime, Datelike, Local};
    const MONTHS: [&str; 12] = ["jan.", "fev.", "mar.", "abr.", "mai.", "jun.", "jul.", "ago.", "set.", "out.", "nov.", "dez."];
    let when: DateTime<Local> = modified.into();
    let today: DateTime<Local> = now.into();
    let days = today.date_naive().signed_duration_since(when.date_naive()).num_days();
    match days {
        0 => format!("hoje, {}", when.format("%H:%M")),
        1 => format!("ontem, {}", when.format("%H:%M")),
        _ if when.year() == today.year() => format!("{} de {}", when.day(), MONTHS[when.month0() as usize]),
        _ => format!("{} de {} de {}", when.day(), MONTHS[when.month0() as usize], when.year()),
    }
}

/// Nome livre para uma pasta nova: "Nova pasta", "Nova pasta 2"...
pub fn new_folder_name(dir: &Path) -> String {
    let mut name = "Nova pasta".to_string();
    let mut n = 2;
    while dir.join(&name).exists() {
        name = format!("Nova pasta {n}");
        n += 1;
    }
    name
}

/// Cria uma pasta nova em `dir` e devolve o caminho dela.
pub fn create_folder(dir: &Path) -> std::io::Result<PathBuf> {
    let path = dir.join(new_folder_name(dir));
    fs::create_dir(&path)?;
    Ok(path)
}

/// Confere um nome novo: não vazio, sem "/" e sem ser "." ou "..".
pub fn valid_name(name: &str) -> Result<&str, &'static str> {
    let name = name.trim();
    if name.is_empty() {
        return Err("o nome não pode ficar vazio");
    }
    if name.contains('/') || name.contains('\0') {
        return Err("o nome não pode ter \"/\"");
    }
    if name == "." || name == ".." {
        return Err("esse nome não é permitido");
    }
    Ok(name)
}

/// O kernel segue este link? A mesma regra do `fs.protected_symlinks`: numa pasta com
/// "sticky bit" e escrita para todos (`/tmp`), só um link do próprio usuário ou do dono
/// da pasta. Senão, outro usuário podia plantar `/tmp/notas.txt -> ~/.bashrc`.
fn may_follow(link: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let Ok(meta) = fs::symlink_metadata(link) else {
        return false;
    };
    let me = rustix::process::geteuid().as_raw();
    let dir = link.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let Ok(dir_meta) = fs::metadata(dir) else {
        return false;
    };
    let shared = dir_meta.mode() & 0o1002 == 0o1002;
    !shared || meta.uid() == me || meta.uid() == dir_meta.uid()
}

/// O arquivo de verdade por trás de `path`: segue os links simbólicos, também um link
/// quebrado (grava no destino dele, como os editores fazem: um `~/.bashrc` que aponta
/// para um repositório de configurações continua link). Arquivo novo: o próprio
/// caminho. Um link que o kernel não seguiria (plantado por outro usuário numa pasta
/// compartilhada) dá erro de permissão.
fn real_path(path: &Path) -> std::io::Result<PathBuf> {
    let mut current = path.to_path_buf();
    for _ in 0..40 {
        match fs::symlink_metadata(&current) {
            Ok(meta) if meta.file_type().is_symlink() => {
                if !may_follow(&current) {
                    return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "link de outro usuário numa pasta compartilhada"));
                }
                let target = fs::read_link(&current)?;
                current = match current.parent() {
                    Some(dir) if target.is_relative() => dir.join(target),
                    _ => target,
                };
            }
            _ => return Ok(current),
        }
    }
    Err(std::io::Error::other("links simbólicos demais"))
}

/// Grava sem risco de perder o arquivo (o Editor de Texto e as notas da Calculadora):
/// escreve um temporário na pasta do arquivo de verdade (seguindo links simbólicos),
/// que nasce sem permissões a mais que o original, e troca de uma vez; o modo, o grupo
/// e o setuid do original são refeitos no temporário (no que o sistema de arquivos
/// deixar). O temporário tem nome único e é criado sem seguir nem truncar nada que já
/// exista; se algo falha, ele é apagado e o arquivo de antes continua como estava.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let real = real_path(path)?;
    let path = real.as_path();
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let name = path.file_name().ok_or_else(|| std::io::Error::other("caminho sem nome de arquivo"))?;
    let original = fs::metadata(path).ok();
    // Arquivo novo: o modo padrão (0666 menos a umask), como o `File::create`.
    let mode = original.as_ref().map_or(0o666, |m| m.permissions().mode() & 0o777);
    // Nome longo: o do temporário fica abaixo do limite de 255 bytes.
    let lossy = name.to_string_lossy();
    let mut cut = lossy.len().min(200);
    while !lossy.is_char_boundary(cut) {
        cut -= 1;
    }
    let (mut file, tmp) = {
        let mut tries = 0;
        loop {
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let tmp = dir.join(format!(".{}.{}-{n}.mide-tmp", &lossy[..cut], std::process::id()));
            match fs::OpenOptions::new().write(true).create_new(true).mode(mode).open(&tmp) {
                Ok(file) => break (file, tmp),
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists && tries < 100 => tries += 1,
                Err(err) => return Err(err),
            }
        }
    };
    let written = (|| {
        file.write_all(bytes)?;
        if let Some(meta) = &original {
            // O grupo do original (se o usuário é dele) e depois o modo exato: a escrita e
            // a troca de grupo tiram o setuid. Melhor esforço: alguns sistemas de arquivos
            // (FAT, FUSE) recusam, e o modo da criação já não é mais aberto que o original.
            let _ = std::os::unix::fs::fchown(&file, None, Some(meta.gid()));
            let _ = file.set_permissions(meta.permissions());
        }
        file.sync_all()
    })();
    drop(file);
    let result = written.and_then(|()| fs::rename(&tmp, path));
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// Renomeia sem sobrescrever: falha se já existir algo com o nome novo.
pub fn rename(path: &Path, new_name: &str) -> Result<PathBuf, String> {
    let name = valid_name(new_name)?;
    let target = path.with_file_name(name);
    if target == path {
        return Ok(target);
    }
    if target.exists() {
        return Err(format!("já existe \"{name}\" nesta pasta"));
    }
    fs::rename(path, &target).map_err(|e| e.to_string())?;
    Ok(target)
}

/// Pastas favoritas da barra lateral: nome, caminho e ícone do tema.
pub fn favorites() -> Vec<(String, PathBuf, &'static str)> {
    let home = crate::places::home_dir();
    let user_dir = |key: &str, fallback: &str| -> PathBuf {
        let cfg = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".config"));
        fs::read_to_string(cfg.join("user-dirs.dirs"))
            .ok()
            .and_then(|t| crate::places::parse_user_dir(&t, key, &home))
            .unwrap_or_else(|| home.join(fallback))
    };
    let mut list = vec![("Pasta pessoal".to_string(), home.clone(), "user-home-symbolic")];
    for (key, fallback, label, icon) in [
        ("DESKTOP", "Área de trabalho", "Área de trabalho", "user-desktop-symbolic"),
        ("DOCUMENTS", "Documentos", "Documentos", "folder-documents-symbolic"),
        ("DOWNLOAD", "Downloads", "Downloads", "folder-download-symbolic"),
        ("PICTURES", "Imagens", "Imagens", "folder-pictures-symbolic"),
        ("MUSIC", "Músicas", "Músicas", "folder-music-symbolic"),
        ("VIDEOS", "Vídeos", "Vídeos", "folder-videos-symbolic"),
    ] {
        let path = user_dir(key, fallback);
        if path.is_dir() && path != home {
            list.push((label.to_string(), path, icon));
        }
    }
    list
}

/// Tipo do item em português, para a coluna "Tipo" da lista (como no Finder).
pub fn kind_text(entry: &Entry) -> String {
    if entry.is_dir {
        return "Pasta".into();
    }
    let ext = entry.path.extension().and_then(|e| e.to_str()).map(str::to_lowercase).unwrap_or_default();
    let kind = match ext.as_str() {
        "png" => "Imagem PNG",
        "jpg" | "jpeg" => "Imagem JPEG",
        "gif" => "Imagem GIF",
        "webp" => "Imagem WebP",
        "svg" => "Imagem SVG",
        "pdf" => "Documento PDF",
        "txt" => "Texto",
        "md" => "Texto Markdown",
        "odt" | "doc" | "docx" => "Documento de texto",
        "ods" | "xls" | "xlsx" | "csv" => "Planilha",
        "odp" | "ppt" | "pptx" => "Apresentação",
        "mp3" | "ogg" | "flac" | "wav" | "m4a" | "opus" => "Áudio",
        "mp4" | "mkv" | "webm" | "avi" | "mov" => "Vídeo",
        "zip" => "Arquivo ZIP",
        "gz" | "tgz" | "xz" | "bz2" | "7z" | "rar" | "tar" | "zst" => "Arquivo compactado",
        "rs" | "c" | "h" | "cpp" | "py" | "js" | "ts" | "go" | "java" | "sh" | "lua" | "vim" => "Código-fonte",
        "html" | "htm" => "Página HTML",
        "json" | "toml" | "yaml" | "yml" | "xml" | "ini" | "conf" => "Configuração",
        "desktop" => "Atalho de app",
        "deb" | "rpm" | "appimage" | "flatpakref" => "Pacote de instalação",
        "iso" | "img" => "Imagem de disco",
        "" => "Documento",
        _ => return format!("Documento {}", ext.to_uppercase()),
    };
    kind.into()
}

/// Nome livre em `dir` a partir de `stem` e da extensão `ext` (pode ser vazia):
/// "nome.txt", "nome 2.txt", "nome 3.txt"...
pub fn unique_name(dir: &Path, stem: &str, ext: &str) -> String {
    let with = |s: &str| if ext.is_empty() { s.to_string() } else { format!("{s}.{ext}") };
    let mut name = with(stem);
    let mut n = 2;
    while dir.join(&name).exists() {
        name = with(&format!("{stem} {n}"));
        n += 1;
    }
    name
}

/// Cria um arquivo vazio "Novo arquivo.txt" (ou com número) e devolve o caminho. O
/// Arquivos já abre o nome para trocar, com a extensão que a pessoa quiser.
pub fn create_file(dir: &Path) -> std::io::Result<PathBuf> {
    let path = dir.join(unique_name(dir, "Novo arquivo", "txt"));
    fs::OpenOptions::new().write(true).create_new(true).open(&path)?;
    Ok(path)
}

/// Separa "foto.jpg" em ("foto", "jpg"); pastas e nomes sem ponto ficam sem extensão.
fn split_name(path: &Path) -> (String, String) {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if path.is_dir() {
        return (name, String::new());
    }
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem.to_string(), ext.to_string()),
        _ => (name, String::new()),
    }
}

/// Copia um arquivo ou uma pasta inteira (com tudo dentro) para `target`.
fn copy_recursive(src: &Path, target: &Path) -> std::io::Result<()> {
    if src.is_dir() {
        fs::create_dir(target)?;
        for entry in fs::read_dir(src)?.flatten() {
            copy_recursive(&entry.path(), &target.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        fs::copy(src, target).map(|_| ())
    }
}

/// Duplica o item na mesma pasta: "foto cópia.jpg", "foto cópia 2.jpg"...
pub fn duplicate(path: &Path) -> std::io::Result<PathBuf> {
    let dir = path.parent().ok_or_else(|| std::io::Error::other("sem pasta"))?;
    let (stem, ext) = split_name(path);
    let target = dir.join(unique_name(dir, &format!("{stem} cópia"), &ext));
    copy_recursive(path, &target)?;
    Ok(target)
}

/// Copia o item para a pasta `dir` (Colar). Com o mesmo nome lá, ganha número.
pub fn copy_into(path: &Path, dir: &Path) -> std::io::Result<PathBuf> {
    if dir.starts_with(path) {
        return Err(std::io::Error::other("não dá para copiar uma pasta para dentro dela mesma"));
    }
    let (stem, ext) = split_name(path);
    let target = dir.join(unique_name(dir, &stem, &ext));
    copy_recursive(path, &target)?;
    Ok(target)
}

/// Move o item para a pasta `dir` (arrastar e soltar). Com o mesmo nome lá, ganha
/// número. Entre discos diferentes, copia e apaga o original.
pub fn move_into(path: &Path, dir: &Path) -> std::io::Result<PathBuf> {
    if dir.starts_with(path) {
        return Err(std::io::Error::other("não dá para mover uma pasta para dentro dela mesma"));
    }
    if path.parent() == Some(dir) {
        return Ok(path.to_path_buf());
    }
    let (stem, ext) = split_name(path);
    let target = dir.join(unique_name(dir, &stem, &ext));
    if fs::rename(path, &target).is_err() {
        copy_recursive(path, &target)?;
        if path.is_dir() {
            fs::remove_dir_all(path)?;
        } else {
            fs::remove_file(path)?;
        }
    }
    Ok(target)
}

/// Tamanho de uma pasta com tudo dentro (sem seguir atalhos). Para depois de `limit`
/// itens, para não demorar em pastas enormes: devolve `None` nesse caso.
pub fn dir_size(path: &Path, limit: usize) -> Option<u64> {
    let mut total = 0;
    let mut seen = 0;
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            seen += 1;
            if seen > limit {
                return None;
            }
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if meta.is_dir() {
                stack.push(entry.path());
            } else {
                total += meta.len();
            }
        }
    }
    Some(total)
}

/// Compacta um arquivo ou pasta num .zip ao lado dele ("foto.zip", "Projetos.zip",
/// com número se já existir). Feito pelo próprio mide, sem programa externo.
pub fn compress(path: &Path) -> std::io::Result<PathBuf> {
    compress_many(std::slice::from_ref(&path.to_path_buf()))
}

/// Compacta vários itens da mesma pasta num .zip só, "Arquivo.zip" (como o Finder);
/// com um item só, o .zip leva o nome dele.
pub fn compress_many(paths: &[PathBuf]) -> std::io::Result<PathBuf> {
    use std::io::Write;
    let first = paths.first().ok_or_else(|| std::io::Error::other("nada para compactar"))?;
    let dir = first.parent().ok_or_else(|| std::io::Error::other("sem pasta"))?;
    let stem = if paths.len() == 1 { split_name(first).0 } else { "Arquivo".to_string() };
    let target = dir.join(unique_name(dir, &stem, "zip"));
    let file = fs::File::create(&target)?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let root = dir;
    let mut stack: Vec<PathBuf> = paths.iter().rev().cloned().collect();
    while let Some(item) = stack.pop() {
        let name = item.strip_prefix(root).map_err(std::io::Error::other)?.to_string_lossy().replace('\\', "/");
        if item.is_dir() {
            zip.add_directory(format!("{name}/"), options).map_err(std::io::Error::other)?;
            let mut children: Vec<PathBuf> = fs::read_dir(&item)?.flatten().map(|e| e.path()).collect();
            children.sort();
            stack.extend(children.into_iter().rev());
        } else {
            zip.start_file(name, options).map_err(std::io::Error::other)?;
            zip.write_all(&fs::read(&item)?)?;
        }
    }
    zip.finish().map_err(std::io::Error::other)?;
    Ok(target)
}

/// Extrai um .zip numa pasta nova ao lado dele (com o nome do arquivo, ou com número
/// se já existir). Caminhos que tentam sair da pasta são ignorados.
pub fn extract(path: &Path) -> std::io::Result<PathBuf> {
    let dir = path.parent().ok_or_else(|| std::io::Error::other("sem pasta"))?;
    let (stem, _) = split_name(path);
    let target = dir.join(unique_name(dir, &stem, ""));
    let mut archive = zip::ZipArchive::new(fs::File::open(path)?).map_err(std::io::Error::other)?;
    fs::create_dir(&target)?;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(std::io::Error::other)?;
        let Some(inner) = file.enclosed_name() else {
            continue;
        };
        let out = target.join(inner);
        if file.is_dir() {
            fs::create_dir_all(&out)?;
        } else {
            if let Some(parent) = out.parent() {
                fs::create_dir_all(parent)?;
            }
            std::io::copy(&mut file, &mut fs::File::create(&out)?)?;
        }
    }
    Ok(target)
}

/// Nome livre para um item novo: "Nova pasta", "Nova pasta (1)", "Nova pasta (2)"...
/// (`ext` vazia para pastas; com extensão: "Novo arquivo.txt", "Novo arquivo (1).txt").
pub fn numbered_name(dir: &Path, stem: &str, ext: &str) -> String {
    let with = |s: &str| if ext.is_empty() { s.to_string() } else { format!("{s}.{ext}") };
    let mut name = with(stem);
    let mut n = 1;
    while taken_name(dir, &name).is_some() {
        name = with(&format!("{stem} ({n})"));
        n += 1;
    }
    name
}

/// Nome já usado na pasta, sem diferenciar maiúsculas de minúsculas (como no macOS:
/// "Nova Pasta" e "Nova pasta" são o mesmo nome). Devolve o nome como está no disco.
pub fn taken_name(dir: &Path, name: &str) -> Option<String> {
    if dir.join(name).exists() {
        return Some(name.to_string());
    }
    let wanted = name.to_lowercase();
    fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .find(|existing| existing.to_lowercase() == wanted)
}

/// Cria um item novo com o nome dado (pasta ou arquivo vazio). Não sobrescreve: com
/// o nome já usado, devolve o erro em português.
pub fn create_named(dir: &Path, name: &str, folder: bool) -> Result<PathBuf, String> {
    let name = valid_name(name)?;
    let path = dir.join(name);
    if let Some(existing) = taken_name(dir, name) {
        return Err(format!("já existe \"{existing}\" nesta pasta"));
    }
    let result = if folder { fs::create_dir(&path) } else { fs::OpenOptions::new().write(true).create_new(true).open(&path).map(|_| ()) };
    result.map_err(|e| e.to_string())?;
    Ok(path)
}

/// Extensões de arquivos de texto conhecidas (abrem no Editor de Texto do mide).
const TEXT_EXTENSIONS: &[&str] = &[
    "txt", "md", "markdown", "rst", "log", "csv", "tsv", "json", "toml", "yaml", "yml", "ini", "conf", "cfg", "xml",
    "html", "htm", "css", "js", "ts", "jsx", "tsx", "rs", "go", "py", "rb", "c", "h", "cpp", "hpp", "java", "kt", "sh",
    "bash", "zsh", "fish", "lua", "sql", "desktop", "env", "gitignore", "mod", "sum", "lock",
];

/// O arquivo é texto? Pela extensão conhecida ou, sem ela, pelo começo do conteúdo
/// (UTF-8 sem bytes nulos). Pastas e arquivos grandes demais (mais de 20 MB) não.
pub fn is_text(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() || meta.len() > 20 * 1024 * 1024 {
        return false;
    }
    let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    if ext.as_deref().is_some_and(|e| TEXT_EXTENSIONS.contains(&e)) {
        return true;
    }
    if ext.is_some() {
        return false;
    }
    // Sem extensão (README, Makefile, .bashrc...): olha o começo.
    let mut head = vec![0u8; 4096];
    let Ok(n) = std::fs::File::open(path).and_then(|mut f| std::io::Read::read(&mut f, &mut head)) else {
        return false;
    };
    let head = &head[..n];
    !head.contains(&0) && (std::str::from_utf8(head).is_ok() || n == 4096)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconhece_arquivos_de_texto() {
        let dir = std::env::temp_dir().join(format!("mide-texto-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let write = |name: &str, bytes: &[u8]| {
            let p = dir.join(name);
            std::fs::write(&p, bytes).unwrap();
            p
        };
        assert!(is_text(&write("notas.md", b"# oi")));
        assert!(is_text(&write("README", "texto com acento: ação".as_bytes())));
        assert!(!is_text(&write("foto.png", b"\x89PNG\0\0")));
        assert!(!is_text(&write("binario", b"ELF\0\0\0")));
        assert!(!is_text(&dir));
        std::fs::remove_dir_all(&dir).unwrap();
    }
    use std::time::Duration;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mide-files-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn gravacao_segura_mantem_permissoes_e_nao_deixa_sobras() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp("gravar");
        let path = dir.join("notas.txt");
        // Arquivo novo e depois por cima do antigo, com acentos.
        write_atomic(&path, "primeira versão".as_bytes()).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        write_atomic(&path, "segunda versão, ação".as_bytes()).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "segunda versão, ação");
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        // O temporário não fica na pasta.
        let names: Vec<String> = fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["notas.txt"]);
        // Pasta que não existe: erro, e nada é criado.
        assert!(write_atomic(&dir.join("nao-existe/x.txt"), b"x").is_err());
        // Uma sobra de outra gravação com o nome antigo do temporário não atrapalha nem é
        // apagada.
        fs::write(dir.join(".notas.txt.mide-tmp"), "sobra").unwrap();
        write_atomic(&path, b"terceira").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "terceira");
        assert_eq!(fs::read_to_string(dir.join(".notas.txt.mide-tmp")).unwrap(), "sobra");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn gravacao_segura_segue_links_simbolicos() {
        use std::os::unix::fs::symlink;
        let dir = temp("links");
        // Link para um arquivo (como um ~/.bashrc de um repositório de configurações):
        // o link continua link e o arquivo de verdade recebe o texto.
        fs::write(dir.join("real.txt"), "antigo").unwrap();
        symlink("real.txt", dir.join("link.txt")).unwrap();
        write_atomic(&dir.join("link.txt"), b"novo").unwrap();
        assert!(fs::symlink_metadata(dir.join("link.txt")).unwrap().file_type().is_symlink());
        assert_eq!(fs::read_to_string(dir.join("real.txt")).unwrap(), "novo");
        // Link quebrado: grava no destino dele.
        symlink("ainda-nao.txt", dir.join("quebrado.txt")).unwrap();
        write_atomic(&dir.join("quebrado.txt"), b"criado").unwrap();
        assert!(fs::symlink_metadata(dir.join("quebrado.txt")).unwrap().file_type().is_symlink());
        assert_eq!(fs::read_to_string(dir.join("ainda-nao.txt")).unwrap(), "criado");
        // Nenhum temporário sobrou.
        assert!(fs::read_dir(&dir).unwrap().flatten().all(|e| !e.file_name().to_string_lossy().ends_with(".mide-tmp")));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn gravacao_segura_com_nome_longo_e_setuid() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp("longo");
        // Nome de 240 bytes: o temporário não passa do limite, em várias gravações.
        let path = dir.join(format!("{}.txt", "ç".repeat(118)));
        for i in 0..12 {
            write_atomic(&path, format!("versão {i}").as_bytes()).unwrap();
        }
        assert_eq!(fs::read_to_string(&path).unwrap(), "versão 11");
        // O modo com setuid volta depois da escrita.
        let script = dir.join("roda.sh");
        fs::write(&script, "#!/bin/sh").unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o4755)).unwrap();
        write_atomic(&script, b"#!/bin/sh\necho oi").unwrap();
        assert_eq!(fs::metadata(&script).unwrap().permissions().mode() & 0o7777, 0o4755);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn lista_pastas_primeiro_sem_ocultos() {
        let dir = temp("lista");
        fs::create_dir(dir.join("zeta")).unwrap();
        fs::write(dir.join("Água.txt"), "1").unwrap();
        fs::write(dir.join("banana.pdf"), "12").unwrap();
        fs::write(dir.join(".oculto"), "x").unwrap();
        let names: Vec<String> = list_dir(&dir, false).unwrap().into_iter().map(|e| e.name).collect();
        assert_eq!(names, vec!["zeta", "Água.txt", "banana.pdf"]);
        assert_eq!(list_dir(&dir, true).unwrap().len(), 4);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn tamanhos_em_portugues() {
        assert_eq!(size_text(1), "1 byte");
        assert_eq!(size_text(512), "512 bytes");
        assert_eq!(size_text(3_200), "3,2 KB");
        assert_eq!(size_text(14_800_000), "14,8 MB");
        assert_eq!(size_text(1_100_000_000), "1,1 GB");
    }

    #[test]
    fn datas_relativas() {
        let now = SystemTime::now();
        assert!(date_text(now, now).starts_with("hoje, "));
        assert!(date_text(now - Duration::from_secs(86_400), now).starts_with("ontem, "));
        let old = SystemTime::UNIX_EPOCH + Duration::from_secs(1_600_000_000);
        assert!(date_text(old, now).ends_with("de 2020"));
    }

    #[test]
    fn pasta_nova_com_nome_livre_e_renomear() {
        let dir = temp("nova");
        let a = create_folder(&dir).unwrap();
        let b = create_folder(&dir).unwrap();
        assert!(a.ends_with("Nova pasta"));
        assert!(b.ends_with("Nova pasta 2"));
        let renamed = rename(&a, "  Fotos  ").unwrap();
        assert!(renamed.ends_with("Fotos"));
        assert!(rename(&b, "Fotos").is_err());
        assert!(rename(&b, "a/b").is_err());
        assert!(rename(&b, "").is_err());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn busca_ignora_acentos() {
        let e = |n: &str| Entry { name: n.into(), path: n.into(), is_dir: false, size: 0, modified: SystemTime::UNIX_EPOCH };
        let list = vec![e("Relatório.pdf"), e("foto.jpg")];
        assert_eq!(filter(&list, "relatorio").len(), 1);
        assert_eq!(filter(&list, "").len(), 2);
    }

    #[test]
    fn tipos_em_portugues() {
        let entry = |name: &str, is_dir: bool| Entry { name: name.into(), path: PathBuf::from(name), is_dir, size: 0, modified: SystemTime::UNIX_EPOCH };
        assert_eq!(kind_text(&entry("Fotos", true)), "Pasta");
        assert_eq!(kind_text(&entry("a.PNG", false)), "Imagem PNG");
        assert_eq!(kind_text(&entry("a.pdf", false)), "Documento PDF");
        assert_eq!(kind_text(&entry("a.xyz", false)), "Documento XYZ");
        assert_eq!(kind_text(&entry("LEIAME", false)), "Documento");
    }

    #[test]
    fn novo_arquivo_duplicar_e_colar() {
        let dir = temp("copias");
        let a = create_file(&dir).unwrap();
        assert_eq!(a.file_name().unwrap(), "Novo arquivo.txt");
        assert_eq!(create_file(&dir).unwrap().file_name().unwrap(), "Novo arquivo 2.txt");
        fs::write(dir.join("foto.jpg"), "x").unwrap();
        assert_eq!(duplicate(&dir.join("foto.jpg")).unwrap().file_name().unwrap(), "foto cópia.jpg");
        assert_eq!(duplicate(&dir.join("foto.jpg")).unwrap().file_name().unwrap(), "foto cópia 2.jpg");
        fs::create_dir_all(dir.join("Pasta/dentro")).unwrap();
        fs::write(dir.join("Pasta/dentro/n.txt"), "abc").unwrap();
        let copy = duplicate(&dir.join("Pasta")).unwrap();
        assert_eq!(fs::read_to_string(copy.join("dentro/n.txt")).unwrap(), "abc");
        fs::create_dir(dir.join("Outra")).unwrap();
        let pasted = copy_into(&dir.join("foto.jpg"), &dir.join("Outra")).unwrap();
        assert_eq!(pasted, dir.join("Outra/foto.jpg"));
        assert!(copy_into(&dir.join("Pasta"), &dir.join("Pasta/dentro")).is_err(), "pasta dentro dela mesma");
        assert_eq!(dir_size(&dir.join("Pasta"), 100), Some(3));
        assert_eq!(dir_size(&dir, 2), None, "passou do limite");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn compactar_e_extrair_zip() {
        let dir = temp("zip");
        fs::create_dir_all(dir.join("Projeto/src")).unwrap();
        fs::write(dir.join("Projeto/src/main.rs"), "fn main() {}").unwrap();
        fs::write(dir.join("Projeto/LEIAME.md"), "oi").unwrap();
        let zip = compress(&dir.join("Projeto")).unwrap();
        assert_eq!(zip.file_name().unwrap(), "Projeto.zip");
        fs::remove_dir_all(dir.join("Projeto")).unwrap();
        let out = extract(&zip).unwrap();
        assert_eq!(out.file_name().unwrap(), "Projeto");
        assert_eq!(fs::read_to_string(out.join("Projeto/src/main.rs")).unwrap(), "fn main() {}");
        assert_eq!(fs::read_to_string(out.join("Projeto/LEIAME.md")).unwrap(), "oi");
        // Extrair de novo não sobrescreve: pasta com número.
        assert_eq!(extract(&zip).unwrap().file_name().unwrap(), "Projeto 2");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn nomes_numerados_e_criar_com_nome() {
        let dir = temp("numerados");
        assert_eq!(numbered_name(&dir, "Nova pasta", ""), "Nova pasta");
        create_named(&dir, "Nova pasta", true).unwrap();
        assert_eq!(numbered_name(&dir, "Nova pasta", ""), "Nova pasta (1)");
        create_named(&dir, "Nova pasta (1)", true).unwrap();
        assert_eq!(numbered_name(&dir, "Nova pasta", ""), "Nova pasta (2)");
        assert_eq!(numbered_name(&dir, "Novo arquivo", "txt"), "Novo arquivo.txt");
        let f = create_named(&dir, "notas.md", false).unwrap();
        assert!(f.is_file());
        assert!(create_named(&dir, "notas.md", false).unwrap_err().contains("já existe"));
        assert!(create_named(&dir, "a/b", true).is_err());
        // Maiúsculas e minúsculas contam como o mesmo nome (como no macOS).
        create_named(&dir, "Pasta Nova", true).unwrap();
        assert_eq!(numbered_name(&dir, "Pasta nova", ""), "Pasta nova (1)");
        assert!(create_named(&dir, "pasta nova", true).unwrap_err().contains("Pasta Nova"));
        fs::remove_dir_all(&dir).unwrap();
    }
}
