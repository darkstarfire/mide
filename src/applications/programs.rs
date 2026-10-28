//! Programas abertos com duplo clique no Arquivos e na área de trabalho (E8): atalhos
//! `.desktop` (abrem o app que descrevem), AppImages, executáveis e scripts.
//!
//! Segurança como nos outros desktops: o que veio da internet (com a marca
//! `user.xdg.origin.url` que os navegadores gravam, ou na pasta Downloads) ou não está
//! marcado como executável só roda depois de a pessoa confirmar ("Abrir <nome>? Este
//! programa veio de <origem>; abra só se confiar."). Confirmar marca o arquivo como
//! executável e guarda a confiança num registro do mide
//! (`$XDG_DATA_HOME/mide/programas-confiaveis`), com o tamanho e a data do arquivo: se o
//! arquivo muda, a confiança acaba e o mide pergunta de novo. Scripts de texto sempre
//! perguntam: Executar ou Abrir no editor.
//!
//! Atalhos `.desktop` fora das pastas de apps (`applications/`) só rodam e só mostram o
//! nome e o ícone do app depois de confiados, mesmo com a permissão de execução: num
//! pendrive exFAT/NTFS ou num zip extraído, todo arquivo aparece como executável, e um
//! `foto.jpg.desktop` com o ícone de imagem rodaria qualquer comando (o GNOME faz o
//! mesmo com o `metadata::trusted`). Programas em outro disco (pendrive, `/media`,
//! `/mnt`, outro sistema de arquivos que não o da pasta pessoal) também perguntam. Um
//! ELF com extensão de documento (`foto.jpg`, `relatorio.pdf`), uma biblioteca (`.so`,
//! sem `PT_INTERP`) ou um objeto (`.o`) abre como documento, nunca roda.
//!
//! Com `MIDE_OPEN_DRY_RUN` (testes), nada roda: o comando e a pasta vão para o log
//! ("executar (teste): ...").

use std::{
    collections::HashMap,
    fs,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant, SystemTime},
};

use crate::{kit::Ui, places};

/// Pastas que decidem a confiança: dados do usuário (registro e `applications/`),
/// Downloads e a pasta pessoal (o disco dela é o "de casa"). Os testes passam pastas
/// próprias, sem mexer no ambiente do processo.
#[derive(Debug, Clone)]
pub struct Dirs {
    pub data: PathBuf,
    pub downloads: PathBuf,
    pub home: PathBuf,
}

impl Dirs {
    /// As pastas de quem está usando (`XDG_DATA_HOME`, `user-dirs.dirs`, `HOME`).
    pub fn current() -> Self {
        Self { data: places::data_home(), downloads: places::downloads_dir(), home: places::home_dir() }
    }
}

/// Maior `.desktop` lido (um atalho de verdade tem poucos KB).
const MAX_LAUNCHER: u64 = 512 * 1024;

/// Extensões de documento: um arquivo com uma delas nunca roda com duplo clique, mesmo
/// sendo um ELF ou começando com `#!` (abre no app padrão, como antes do E8).
const DOCUMENT_EXTS: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "webp", "bmp", "tif", "tiff", "svg", "ico", "heic", "avif", "pdf", "txt", "md", "rtf", "doc", "docx", "odt", "ods", "odp", "xls", "xlsx",
    "ppt", "pptx", "csv", "epub", "html", "htm", "xml", "json", "mp3", "mp4", "m4a", "mkv", "webm", "avi", "mov", "ogg", "oga", "ogv", "opus", "flac", "wav", "zip", "tar",
    "gz", "tgz", "xz", "bz2", "zst", "7z", "rar", "iso", "so", "o", "a", "ko", "obj", "dll", "lib",
];

/// Nome de documento ou de biblioteca (`foto.jpg`, `libx.so`, `libx.so.1.2`, `main.o`).
fn document_name(path: &Path) -> bool {
    let name = path.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    let parts: Vec<&str> = name.split('.').collect();
    let ext = if parts.len() > 1 { parts[parts.len() - 1] } else { "" };
    DOCUMENT_EXTS.contains(&ext) || parts.iter().skip(1).any(|p| *p == "so")
}

/// Um ELF que é programa: `ET_EXEC`, ou `ET_DYN` com `PT_INTERP` (executável PIE).
/// Bibliotecas (`ET_DYN` sem intérprete), objetos (`ET_REL`) e despejos (`ET_CORE`)
/// não são; um cabeçalho que não dá para ler também não.
pub fn elf_is_program(path: &Path) -> bool {
    let Ok(mut file) = fs::File::open(path) else {
        return false;
    };
    let mut h = [0u8; 64];
    if file.read_exact(&mut h).is_err() || !h.starts_with(b"\x7fELF") {
        return false;
    }
    let little = match h[5] {
        1 => true,
        2 => false,
        _ => return false,
    };
    let u16_at = |b: &[u8], i: usize| if little { u16::from_le_bytes([b[i], b[i + 1]]) } else { u16::from_be_bytes([b[i], b[i + 1]]) };
    let u32_at = |b: &[u8], i: usize| {
        let x = [b[i], b[i + 1], b[i + 2], b[i + 3]];
        if little { u32::from_le_bytes(x) } else { u32::from_be_bytes(x) }
    };
    let u64_at = |b: &[u8], i: usize| {
        let mut x = [0u8; 8];
        x.copy_from_slice(&b[i..i + 8]);
        if little { u64::from_le_bytes(x) } else { u64::from_be_bytes(x) }
    };
    let (phoff, phentsize, phnum) = match h[4] {
        1 => (u32_at(&h, 28) as u64, u16_at(&h, 42) as u64, u16_at(&h, 44) as u64),
        2 => (u64_at(&h, 32), u16_at(&h, 54) as u64, u16_at(&h, 56) as u64),
        _ => return false,
    };
    match u16_at(&h, 16) {
        2 => true,
        3 => {
            // PT_INTERP (3) entre os cabeçalhos de programa.
            if !(4..=256).contains(&phentsize) || phnum == 0 || phnum > 256 {
                return false;
            }
            let mut table = vec![0u8; (phentsize * phnum) as usize];
            if file.seek(SeekFrom::Start(phoff)).is_err() || file.read_exact(&mut table).is_err() {
                return false;
            }
            table.chunks_exact(phentsize as usize).any(|e| u32_at(e, 0) == 3)
        }
        _ => false,
    }
}

/// Que tipo de programa é o arquivo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    /// Atalho `.desktop` (tipo Application ou Link).
    Desktop,
    /// AppImage (ELF com a marca `AI` ou extensão `.AppImage`).
    AppImage,
    /// Executável ELF.
    Binary,
    /// Script de texto (`#!` no começo, ou `.sh`/`.bash`). `shebang` diz se tem `#!`.
    Script { shebang: bool },
}

/// O arquivo é um programa que o mide roda com duplo clique? `None`: abre como
/// documento, com o app padrão.
pub fn kind_of(path: &Path) -> Option<Kind> {
    let meta = fs::metadata(path).ok()?;
    if !meta.is_file() {
        return None;
    }
    let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
    if ext == "desktop" {
        return read_launcher(path).map(|_| Kind::Desktop);
    }
    // Nome de documento ou de biblioteca: abre como documento, seja o que for por dentro.
    if document_name(path) {
        return None;
    }
    let mut head = [0u8; 16];
    let n = fs::File::open(path).and_then(|mut f| f.read(&mut head)).unwrap_or(0);
    let head = &head[..n];
    if head.starts_with(b"\x7fELF") {
        // Marca do AppImage (tipo 1 ou 2) no byte 8; a extensão também vale.
        let marked = head.len() >= 11 && &head[8..10] == b"AI" && matches!(head[10], 1 | 2);
        if marked || ext == "appimage" {
            return Some(Kind::AppImage);
        }
        return elf_is_program(path).then_some(Kind::Binary);
    }
    if head.starts_with(b"#!") {
        return Some(Kind::Script { shebang: true });
    }
    if matches!(ext.as_str(), "sh" | "bash") && crate::files::is_text(path) {
        return Some(Kind::Script { shebang: false });
    }
    None
}

/// O que um atalho `.desktop` abre.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// App: o comando já separado em argumentos, a pasta (`Path=`), se abre num
    /// terminal e o `TryExec` (programa que precisa existir).
    App { args: Vec<String>, dir: Option<PathBuf>, terminal: bool, try_exec: Option<String> },
    /// Endereço (`Type=Link`).
    Link(String),
}

/// Um atalho `.desktop` lido: nome e ícone (mostrados no lugar do nome do arquivo) e
/// o que ele abre.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launcher {
    pub name: String,
    pub icon: Option<String>,
    pub target: Target,
}

/// Lê um atalho `.desktop`. `None` se não é um atalho válido (sem `[Desktop Entry]`,
/// sem `Exec` num app, tipo que não abre nada, `Hidden=true`).
pub fn read_launcher(path: &Path) -> Option<Launcher> {
    // Só arquivos comuns e pequenos: um FIFO chamado `x.desktop` travaria a janela só de
    // abrir a pasta, e um de vários GB encheria a memória.
    let meta = fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_LAUNCHER {
        return None;
    }
    let mut text = String::new();
    fs::File::open(path).ok()?.take(MAX_LAUNCHER + 1).read_to_string(&mut text).ok()?;
    if text.len() as u64 > MAX_LAUNCHER {
        return None;
    }
    let locales = freedesktop_desktop_entry::get_languages_from_env();
    let entry = freedesktop_desktop_entry::DesktopEntry::from_str(path, &text, Some(&locales)).ok()?;
    entry.desktop_entry("Type")?;
    if entry.hidden() {
        return None;
    }
    let file_name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let name = entry.name(&locales).map(|n| n.trim().to_string()).filter(|n| !n.is_empty()).unwrap_or(file_name);
    let icon = entry.icon().map(|i| unescape(i).trim().to_string()).filter(|i| !i.is_empty());
    let target = match entry.type_() {
        Some("Application") => {
            let args = exec_args(&unescape(entry.exec()?), &name, icon.as_deref(), path)?;
            let dir = entry.path().map(|p| PathBuf::from(unescape(p))).filter(|p| p.is_absolute());
            let try_exec = entry.try_exec().map(unescape).filter(|t| !t.trim().is_empty());
            Target::App { args, dir, terminal: entry.terminal(), try_exec }
        }
        Some("Link") => Target::Link(unescape(entry.url()?)),
        _ => return None,
    };
    Some(Launcher { name, icon, target })
}

/// Desfaz os escapes dos valores de texto do `.desktop` (`\s`, `\n`, `\t`, `\r`, `\\`).
pub(crate) fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('s') => out.push(' '),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// Separa o `Exec` em argumentos, pela especificação: aspas duplas agrupam (dentro
/// delas, `\"`, `` \` ``, `\$` e `\\` são o caractere), códigos de arquivo (`%f`, `%U`...)
/// somem (nada foi passado), `%i` vira `--icon <ícone>`, `%c` o nome, `%k` o caminho
/// do atalho e `%%` um `%`. Aspas sem fechar ou comando vazio: `None`.
pub fn exec_args(exec: &str, name: &str, icon: Option<&str>, file: &Path) -> Option<Vec<String>> {
    let mut tokens: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut started = false;
    let mut quoted = false;
    let mut chars = exec.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            match c {
                '"' => quoted = false,
                '\\' if chars.peek().is_some_and(|n| matches!(n, '"' | '`' | '$' | '\\')) => current.push(chars.next()?),
                _ => current.push(c),
            }
        } else if c == '"' {
            quoted = true;
            started = true;
        } else if c.is_whitespace() {
            if started {
                tokens.push(std::mem::take(&mut current));
                started = false;
            }
        } else {
            current.push(c);
            started = true;
        }
    }
    if quoted {
        return None;
    }
    if started {
        tokens.push(current);
    }
    let mut args = Vec::new();
    for token in tokens {
        match token.as_str() {
            "%f" | "%F" | "%u" | "%U" | "%d" | "%D" | "%n" | "%N" | "%v" | "%m" => {}
            "%i" => {
                if let Some(icon) = icon {
                    args.push("--icon".to_string());
                    args.push(icon.to_string());
                }
            }
            "%c" => args.push(name.to_string()),
            "%k" => args.push(file.to_string_lossy().into_owned()),
            _ => args.push(token.replace("%%", "%")),
        }
    }
    // Pela especificação, o programa não pode ter `=` (seria uma variável de ambiente).
    (!args.is_empty() && !args[0].contains('=')).then_some(args)
}

/// Rótulos dos atalhos `.desktop` (nome e ícone do app), guardados por arquivo e data:
/// as janelas desenham a cada quadro e não podem reler o arquivo toda vez.
#[derive(Default)]
pub struct Labels {
    map: HashMap<PathBuf, (SystemTime, Option<(String, String)>)>,
    /// Pastas de teste (sem elas, as de quem está usando).
    dirs: Option<Dirs>,
}

impl Labels {
    /// Nome e ícone para mostrar no lugar do nome do arquivo, se for um atalho
    /// `.desktop` válido e confiável (numa pasta de apps ou confirmado pela pessoa). Os
    /// outros continuam com o nome do arquivo: um `foto.jpg.desktop` num pendrive
    /// podia se passar por uma foto.
    pub fn get(&mut self, path: &Path, modified: SystemTime) -> Option<(String, String)> {
        if !path.extension().is_some_and(|e| e.eq_ignore_ascii_case("desktop")) {
            return None;
        }
        if let Some((at, label)) = self.map.get(path)
            && *at == modified
        {
            return label.clone();
        }
        let dirs = self.dirs.clone().unwrap_or_else(Dirs::current);
        let label = read_launcher(path).filter(|_| in_app_dirs(&dirs, path) || is_trusted(&dirs, path)).map(|l| {
            // Ícone em XPM (o do Eclipse): vira PNG no cache, que o mide-ui mostra.
            let icon = l.icon.map(crate::applications::xpm::displayable_icon).unwrap_or_else(|| "application-x-executable".to_string());
            tracing::info!("atalho {}: \"{}\" (ícone {icon})", path.display(), l.name);
            (l.name, icon)
        });
        self.map.insert(path.to_path_buf(), (modified, label.clone()));
        label
    }

    /// Esquece o que foi lido (depois de confiar num atalho, ele passa a mostrar o nome).
    pub fn clear(&mut self) {
        self.map.clear();
    }
}

/// De onde veio um arquivo baixado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    /// Site gravado pelo navegador (`user.xdg.origin.url`).
    Site(String),
    /// Está na pasta Downloads (sem a marca).
    Downloads,
}

/// Endereço de onde o arquivo foi baixado (atributo estendido gravado pelos navegadores).
pub fn origin_url(path: &Path) -> Option<String> {
    let mut buf = vec![0u8; 4096];
    let n = rustix::fs::getxattr(path, "user.xdg.origin.url", &mut buf[..]).ok()?;
    let url = String::from_utf8_lossy(&buf[..n]).trim().to_string();
    (!url.is_empty()).then_some(url)
}

/// Só o site de um endereço ("https://ana@exemplo.com:8080/a" → "exemplo.com").
pub fn host_of(url: &str) -> String {
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    let host = authority.rsplit_once('@').map(|(_, h)| h).unwrap_or(authority);
    let host = match host.find(']') {
        Some(end) if host.starts_with('[') => &host[..=end],
        _ => host.split(':').next().unwrap_or(host),
    };
    if host.is_empty() {
        url.chars().take(60).collect()
    } else {
        host.to_string()
    }
}

/// O arquivo veio da internet? Pela marca do navegador ou por estar em Downloads.
pub fn internet_origin(dirs: &Dirs, path: &Path) -> Option<Origin> {
    if let Some(url) = origin_url(path) {
        return Some(Origin::Site(host_of(&url)));
    }
    let downloads = &dirs.downloads;
    let inside = |p: &Path| p.starts_with(downloads) || fs::canonicalize(downloads).is_ok_and(|d| p.starts_with(d));
    let real = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    (inside(path) || inside(&real)).then_some(Origin::Downloads)
}

/// Quem está usando pode executar o arquivo?
pub fn is_executable(path: &Path) -> bool {
    rustix::fs::access(path, rustix::fs::Access::EXEC_OK).is_ok()
}

/// Registro dos programas confiados pela pessoa.
fn trust_file(dirs: &Dirs) -> PathBuf {
    dirs.data.join("mide").join("programas-confiaveis")
}

/// Linha do registro de um arquivo: tamanho, data (em nanossegundos) e endereço.
fn trust_line(path: &Path) -> Option<String> {
    let real = fs::canonicalize(path).ok()?;
    let meta = fs::metadata(&real).ok()?;
    let nanos = meta.modified().ok()?.duration_since(SystemTime::UNIX_EPOCH).ok()?.as_nanos();
    Some(format!("{} {nanos} {}", meta.len(), places::file_uri(&real)))
}

/// A pessoa já confirmou este arquivo (e ele não mudou desde então)?
pub fn is_trusted(dirs: &Dirs, path: &Path) -> bool {
    let Some(line) = trust_line(path) else {
        return false;
    };
    fs::read_to_string(trust_file(dirs)).is_ok_and(|text| text.lines().any(|l| l == line))
}

/// Confia no arquivo: marca como executável (para quem pode ler) e guarda no registro.
pub fn trust(dirs: &Dirs, path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let meta = fs::metadata(path)?;
    let mode = meta.permissions().mode();
    let wanted = mode | ((mode & 0o444) >> 2);
    if wanted != mode {
        fs::set_permissions(path, fs::Permissions::from_mode(wanted))?;
        tracing::info!("marcado como executável: {}", path.display());
    }
    let line = trust_line(path).ok_or_else(|| std::io::Error::other("arquivo sem data"))?;
    let file = trust_file(dirs);
    let uri = line.splitn(3, ' ').nth(2).unwrap_or_default().to_string();
    let old = fs::read_to_string(&file).unwrap_or_default();
    // Uma linha por arquivo: a confiança de antes (de outra versão) sai.
    let mut text: String = old.lines().filter(|l| l.splitn(3, ' ').nth(2) != Some(uri.as_str())).map(|l| format!("{l}\n")).collect();
    text.push_str(&line);
    text.push('\n');
    if let Some(dir) = file.parent() {
        fs::create_dir_all(dir)?;
    }
    crate::files::write_atomic(&file, text.as_bytes())?;
    tracing::info!("confiado: {}", path.display());
    Ok(())
}

/// Atalho instalado numa pasta de apps do sistema ou do usuário (`applications/`): já
/// é confiável, como os do launcher.
fn in_app_dirs(dirs: &Dirs, path: &Path) -> bool {
    let system = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    std::iter::once(dirs.data.clone())
        .chain(system.split(':').filter(|d| !d.is_empty()).map(PathBuf::from))
        .any(|dir| path.starts_with(dir.join("applications")))
}

/// Pastas onde o sistema monta pendrives e outros discos.
const MOUNT_DIRS: &[&str] = &["/media", "/run/media", "/mnt"];

/// O arquivo está num pendrive ou em outro disco (montado em `/media`, `/run/media`,
/// `/mnt`, ou noutro sistema de arquivos que não o da pasta pessoal)?
pub fn on_other_disk(dirs: &Dirs, path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let real = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if MOUNT_DIRS.iter().any(|m| real.starts_with(m)) {
        return true;
    }
    match (fs::metadata(&real), fs::metadata(&dirs.home)) {
        (Ok(file), Ok(home)) => file.dev() != home.dev(),
        _ => false,
    }
}

/// Por que perguntar antes de rodar (o texto do diálogo), ou `None` se pode rodar.
pub fn reason_to_ask(dirs: &Dirs, path: &Path, kind: &Kind) -> Option<String> {
    if *kind == Kind::Desktop && in_app_dirs(dirs, path) {
        return None;
    }
    if is_trusted(dirs, path) {
        return None;
    }
    match internet_origin(dirs, path) {
        Some(Origin::Site(host)) => return Some(crate::tr!("This program came from {host}. Open it only if you trust it.", host = host)),
        Some(Origin::Downloads) => return Some(crate::tr!("This program came from the internet (it’s in Downloads). Open it only if you trust it.").into_owned()),
        None => {}
    }
    let dir = path.parent().map(home_short).unwrap_or_default();
    if on_other_disk(dirs, path) {
        return Some(crate::tr!("This program is in {dir}, on a USB drive or another disk. Open it only if you trust it.", dir = dir));
    }
    if !is_executable(path) {
        return Some(crate::tr!("This program came from {dir} and isn’t marked as executable. Open it only if you trust it.", dir = dir));
    }
    // Atalho fora das pastas de apps: a permissão de execução não basta (num pendrive
    // ou num zip extraído todo arquivo tem); só roda depois de confiado.
    if *kind == Kind::Desktop {
        return Some(crate::tr!("This shortcut is in {dir} and hasn’t been marked as trusted yet. Open it only if you trust it.", dir = dir));
    }
    None
}

/// Tempo depois de o diálogo aparecer em que o Enter não responde (a tecla que abriu
/// o arquivo, segurada, não confirma sozinha).
const KEY_GRACE: Duration = Duration::from_millis(400);

/// Caminho com `~` no lugar da pasta pessoal.
fn home_short(path: &Path) -> String {
    let home = places::home_dir();
    match path.strip_prefix(&home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".into(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

/// O comando que roda o programa: programa, argumentos, pasta de trabalho e o nome e
/// o ícone do app (para o ambiente dele).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    pub program: String,
    pub args: Vec<String>,
    pub dir: PathBuf,
    pub name: String,
    pub icon: Option<String>,
}

/// Programa que existe: caminho absoluto executável ou nome achado no `PATH`.
fn program_exists(program: &str) -> bool {
    if program.contains('/') {
        return is_executable(Path::new(program));
    }
    std::env::var_os("PATH").is_some_and(|paths| std::env::split_paths(&paths).any(|d| is_executable(&d.join(program))))
}

/// Monta o comando de um programa. Executáveis e scripts rodam na pasta deles; um app
/// de atalho, na `Path=` dele ou na pasta pessoal. Endereços (`Type=Link`) dão `Ok(None)`
/// e abrem no navegador por `places::open`.
pub fn launch_for(path: &Path, kind: &Kind) -> Result<Option<Launch>, String> {
    let file_name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let folder = path.parent().map(Path::to_path_buf).unwrap_or_else(places::home_dir);
    let own = |program: String, args: Vec<String>| Launch { program, args, dir: folder.clone(), name: file_name.clone(), icon: None };
    let path_text = path.to_string_lossy().into_owned();
    Ok(Some(match kind {
        Kind::AppImage | Kind::Binary | Kind::Script { shebang: true } => own(path_text, Vec::new()),
        Kind::Script { shebang: false } => own("sh".into(), vec![path_text]),
        Kind::Desktop => {
            let launcher = read_launcher(path).ok_or_else(|| crate::tr!("“{name}” isn’t a valid app shortcut.", name = file_name))?;
            match launcher.target {
                Target::Link(url) => {
                    places::open(&url);
                    return Ok(None);
                }
                Target::App { args, dir, terminal, try_exec } => {
                    if let Some(needed) = try_exec.filter(|t| !program_exists(t)) {
                        return Err(crate::tr!("“{name}” isn’t installed ({program} wasn’t found).", name = launcher.name, program = needed));
                    }
                    let (program, args) = if terminal {
                        ("mide-terminal".to_string(), std::iter::once("-e".to_string()).chain(args).collect())
                    } else {
                        let mut args = args.into_iter();
                        (args.next().unwrap_or_default(), args.collect())
                    };
                    Launch { program, args, dir: dir.filter(|d| d.is_dir()).unwrap_or_else(places::home_dir), name: launcher.name, icon: launcher.icon }
                }
            }
        }
    }))
}

/// Roda o programa num processo à parte (ou, nos testes, só registra no log).
pub fn run(path: &Path, kind: &Kind) -> Result<(), String> {
    let Some(launch) = launch_for(path, kind)? else {
        return Ok(());
    };
    let line = std::iter::once(launch.program.as_str()).chain(launch.args.iter().map(String::as_str)).collect::<Vec<_>>().join(" ");
    if std::env::var_os("MIDE_OPEN_DRY_RUN").is_some() {
        tracing::info!("executar (teste): {line} (pasta {})", launch.dir.display());
        return Ok(());
    }
    let mut cmd = Command::new(&launch.program);
    cmd.args(&launch.args).current_dir(&launch.dir).stdin(Stdio::null());
    crate::apps::launch_env(&mut cmd, &launch.name, launch.icon.as_deref());
    // Grupo de processos próprio: o programa não recebe os sinais da janela que o abriu.
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    match cmd.spawn() {
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            tracing::info!("executando {line} (pasta {})", launch.dir.display());
            Ok(())
        }
        Err(err) => Err(crate::tr!("Couldn’t open “{name}”: {err}", name = launch.name, err = err)),
    }
}

/// Abre o instalador do mide (`mide-install`) para o AppImage (ou, nos testes, só
/// registra no log).
pub fn start_installer(path: &Path) -> Result<(), String> {
    if std::env::var_os("MIDE_OPEN_DRY_RUN").is_some() {
        tracing::info!("instalar (teste): mide-install {}", path.display());
        return Ok(());
    }
    let program = places::mide_program("mide-install");
    match Command::new(&program).arg(path).stdin(Stdio::null()).spawn() {
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            tracing::info!("instalador aberto para {}", path.display());
            Ok(())
        }
        Err(err) => Err(crate::tr!("Couldn’t open the installer: {err}", err = err)),
    }
}

/// Resultado de abrir um arquivo com duplo clique.
pub enum Opened {
    /// Aberto (ou rodando).
    Done,
    /// Precisa perguntar antes: a janela mostra o diálogo.
    Ask(Prompt),
    /// Não deu certo: a mensagem para mostrar.
    Failed(String),
}

/// Abre um arquivo com duplo clique: documentos no app padrão; programas rodam, ou
/// pedem confirmação (vindos da internet, sem permissão de execução, ou scripts).
pub fn open(path: &Path) -> Opened {
    let Some(kind) = kind_of(path) else {
        places::open(&path.to_string_lossy());
        return Opened::Done;
    };
    let reason = reason_to_ask(&Dirs::current(), path, &kind);
    // AppImage fora da pasta de apps: oferece Instalar além de Abrir (E9).
    let install = kind == Kind::AppImage && !crate::appimage::is_managed(path);
    if reason.is_some() || install || matches!(kind, Kind::Script { .. }) {
        let why = reason.as_deref().unwrap_or(if install { "instalar ou abrir" } else { "script" });
        tracing::info!("perguntar antes de abrir {}: {why}", path.display());
        // Vindo da internet ou script: o Enter não confirma (só o clique no botão).
        let guarded = matches!(kind, Kind::Script { .. }) || (reason.is_some() && internet_origin(&Dirs::current(), path).is_some());
        return Opened::Ask(Prompt { path: path.to_path_buf(), kind, reason, install, fresh: true, guarded, opened: Instant::now() });
    }
    match run(path, &kind) {
        Ok(()) => Opened::Done,
        Err(err) => Opened::Failed(err),
    }
}

/// Diálogo de confirmação antes de rodar um programa.
pub struct Prompt {
    pub path: PathBuf,
    pub kind: Kind,
    /// Por que perguntar (vindo da internet, sem permissão); `None` num script ou
    /// AppImage já confiável.
    pub reason: Option<String>,
    /// Oferece "Instalar" (AppImage fora da pasta de apps, E9).
    pub install: bool,
    /// Aberto neste quadro: a tecla ou o clique que o abriu não conta como resposta.
    fresh: bool,
    /// Vindo da internet ou script: o Enter não escolhe o botão principal.
    guarded: bool,
    /// Quando o diálogo apareceu (ver [`KEY_GRACE`]).
    opened: Instant,
}

/// O que aconteceu no diálogo.
pub enum Answer {
    /// Cancelado: nada rodou.
    Cancelled,
    /// Rodou (ou abriu no editor).
    Done,
    /// Falhou: a mensagem para mostrar.
    Failed(String),
}

impl Prompt {
    /// Desenha o diálogo (chamar no fim do `view`, enquanto ele existir). Devolve a
    /// resposta quando a pessoa escolhe; aí a janela fecha o diálogo.
    pub fn view(&mut self, ui: &mut Ui) -> Option<Answer> {
        let name = self.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let script = matches!(self.kind, Kind::Script { .. });
        use crate::tr;
        let (title, buttons): (String, Vec<String>) = if script {
            (tr!("Run “{name}” or open it in the editor?", name = name), vec![tr!("Cancel").into_owned(), tr!("Open in Editor").into_owned(), tr!("Run").into_owned()])
        } else if self.install {
            (tr!("Install or open “{name}”?", name = name), vec![tr!("Cancel").into_owned(), tr!("Install").into_owned(), tr!("Open").into_owned()])
        } else {
            (tr!("Open “{name}”?", name = name), vec![tr!("Cancel").into_owned(), tr!("Open").into_owned()])
        };
        let buttons: Vec<&str> = buttons.iter().map(String::as_str).collect();
        let folder = crate::appimage::apps_dir().file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let message = match (&self.reason, self.install) {
            (Some(reason), true) => tr!("{reason} Install puts the app in {folder}, in the launcher and in search.", reason = reason, folder = folder),
            (None, true) => tr!("Install puts the app in {folder}, in the launcher and in search. Open only runs this copy.", folder = folder),
            (Some(reason), false) => reason.clone(),
            (None, false) => tr!("This is a script: running it runs its commands.").into_owned(),
        };
        let enter = ui.input().key(crate::kit::Key::Enter);
        let repeat = ui.input().repeat;
        let chosen = ui.alert(&title, &message, &buttons);
        if std::mem::take(&mut self.fresh) {
            ui.request_repaint();
            return None;
        }
        // Enter: nunca confirma o que veio da internet nem um script; nos outros, não vale
        // a repetição da tecla segurada nem o Enter logo depois de o diálogo aparecer.
        let chosen = match chosen {
            Some(c) if enter && c != 0 && (self.guarded || repeat || self.opened.elapsed() < KEY_GRACE) => {
                tracing::info!("abrir {}: Enter ignorado (escolha pelo botão)", self.path.display());
                if !self.guarded {
                    ui.wake_after(KEY_GRACE);
                }
                None
            }
            other => other,
        };
        let chosen = chosen?;
        ui.request_repaint();
        let run_it = chosen + 1 == buttons.len();
        if chosen == 0 {
            tracing::info!("abrir {}: cancelado", self.path.display());
            return Some(Answer::Cancelled);
        }
        if !run_it && self.install {
            // Instalar também é confiar: o instalador roda o AppImage para ler o nome e
            // o ícone de dentro dele.
            if self.reason.is_some()
                && let Err(err) = trust(&Dirs::current(), &self.path)
            {
                return Some(Answer::Failed(tr!("Couldn’t mark “{name}” as executable: {err}", name = name, err = err)));
            }
            return Some(match start_installer(&self.path) {
                Ok(()) => Answer::Done,
                Err(err) => Answer::Failed(err),
            });
        }
        if !run_it {
            tracing::info!("abrir {} no editor", self.path.display());
            places::open(&self.path.to_string_lossy());
            return Some(Answer::Done);
        }
        // Com motivo (internet, sem permissão): confirmar marca como executável e lembra.
        if self.reason.is_some()
            && let Err(err) = trust(&Dirs::current(), &self.path)
        {
            return Some(Answer::Failed(tr!("Couldn’t mark “{name}” as executable: {err}", name = name, err = err)));
        }
        Some(match run(&self.path, &self.kind) {
            Ok(()) => Answer::Done,
            Err(err) => Answer::Failed(err),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mide-programs-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn chmod(path: &Path, mode: u32) {
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
    }

    /// ELF de 64 bits mínimo: cabeçalho com o tipo e um cabeçalho de programa (PT_INTERP
    /// ou PT_LOAD).
    fn elf(e_type: u16, interp: bool) -> Vec<u8> {
        let mut b = vec![0u8; 64 + 56];
        b[..4].copy_from_slice(b"\x7fELF");
        b[4] = 2;
        b[5] = 1;
        b[6] = 1;
        b[16..18].copy_from_slice(&e_type.to_le_bytes());
        b[32..40].copy_from_slice(&64u64.to_le_bytes());
        b[54..56].copy_from_slice(&56u16.to_le_bytes());
        b[56..58].copy_from_slice(&1u16.to_le_bytes());
        b[64..68].copy_from_slice(&(if interp { 3u32 } else { 1 }).to_le_bytes());
        b
    }

    /// FIFO e arquivo enorme com o nome de atalho: não são lidos (nem travam).
    #[test]
    fn atalho_que_nao_e_arquivo_comum() {
        let dir = temp_dir("fifo");
        let fifo = dir.join("x.desktop");
        if Command::new("mkfifo").arg(&fifo).status().is_ok_and(|s| s.success()) {
            assert_eq!(read_launcher(&fifo), None);
            assert_eq!(kind_of(&fifo), None);
            let d = Dirs { data: dir.join("data"), downloads: dir.join("dl"), home: dir.clone() };
            let mut labels = Labels { dirs: Some(d), ..Default::default() };
            assert_eq!(labels.get(&fifo, SystemTime::UNIX_EPOCH), None);
        }
        let big = dir.join("grande.desktop");
        let mut text = String::from("[Desktop Entry]\nType=Application\nName=X\nExec=x\n");
        text.push_str(&format!("Comment={}\n", "a".repeat(600 * 1024)));
        fs::write(&big, text).unwrap();
        assert_eq!(read_launcher(&big), None);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn exec_com_aspas_e_codigos() {
        let f = Path::new("/a/b.desktop");
        let a = |s: &str| exec_args(s, "Meu App", Some("meu-icone"), f);
        assert_eq!(a("firefox %u"), Some(vec!["firefox".into()]));
        assert_eq!(a(r#""/opt/Meu App/app" --x %F"#), Some(vec!["/opt/Meu App/app".into(), "--x".into()]));
        assert_eq!(a(r#"sh -c "echo \"oi\" \$HOME""#), Some(vec!["sh".into(), "-c".into(), "echo \"oi\" $HOME".into()]));
        assert_eq!(a("app %i %c %k 100%%"), Some(vec!["app".into(), "--icon".into(), "meu-icone".into(), "Meu App".into(), "/a/b.desktop".into(), "100%".into()]));
        assert_eq!(a(r#"app "sem fim"#), None);
        assert_eq!(a("%f"), None);
        assert_eq!(a("VAR=1 app"), None);
        assert_eq!(a(r#"app """#), Some(vec!["app".into(), "".into()]));
        // Escapes do texto do .desktop (\s) vêm antes das aspas: o espaço só fica no
        // argumento entre aspas.
        assert_eq!(exec_args(&unescape(r#""/opt/x\sy/app" a\sb"#), "", None, f), Some(vec!["/opt/x y/app".into(), "a".into(), "b".into()]));
    }

    #[test]
    fn site_do_endereco() {
        assert_eq!(host_of("https://github.com/a/b.AppImage"), "github.com");
        assert_eq!(host_of("https://ana@exemplo.com:8080/x?y"), "exemplo.com");
        assert_eq!(host_of("http://[::1]:80/x"), "[::1]");
    }

    #[test]
    fn tipos_de_programa() {
        let dir = temp_dir("tipos");
        let write = |name: &str, bytes: &[u8]| {
            let p = dir.join(name);
            fs::write(&p, bytes).unwrap();
            p
        };
        assert_eq!(kind_of(&write("a.sh", b"#!/bin/sh\necho oi\n")), Some(Kind::Script { shebang: true }));
        assert_eq!(kind_of(&write("b.sh", b"echo oi\n")), Some(Kind::Script { shebang: false }));
        assert_eq!(kind_of(&write("c.txt", b"oi\n")), None);
        assert_eq!(kind_of(&write("prog", &elf(2, false))), Some(Kind::Binary));
        assert_eq!(kind_of(&write("pie", &elf(3, true))), Some(Kind::Binary));
        // Biblioteca (ET_DYN sem intérprete), objeto e cabeçalho cortado: documentos.
        assert_eq!(kind_of(&write("semextlib", &elf(3, false))), None);
        assert_eq!(kind_of(&write("main", &elf(1, false))), None);
        assert_eq!(kind_of(&write("cortado", b"\x7fELF\x02\x01\x01\0\0\0\0\0\0\0\0\0")), None);
        // Nome de documento ou de biblioteca nunca roda, mesmo sendo programa.
        assert_eq!(kind_of(&write("foto.jpg", &elf(2, false))), None);
        assert_eq!(kind_of(&write("relatorio.PDF", &elf(3, true))), None);
        assert_eq!(kind_of(&write("libx.so.1.2", &elf(3, true))), None);
        assert_eq!(kind_of(&write("libx.so", &elf(3, true))), None);
        assert_eq!(kind_of(&write("foto.png", b"\x7fELF\x02\x01\x01\0AI\x02\0\0\0\0\0")), None);
        assert_eq!(kind_of(&write("nota.txt", b"#!/bin/sh\necho oi\n")), None);
        // Versão no nome não é extensão de documento.
        assert_eq!(kind_of(&write("jogo-1.2", &elf(3, true))), Some(Kind::Binary));
        assert_eq!(kind_of(&write("App.AppImage", b"\x7fELF\x02\x01\x01\0\0\0\0\0\0\0\0\0")), Some(Kind::AppImage));
        assert_eq!(kind_of(&write("semext", b"\x7fELF\x02\x01\x01\0AI\x02\0\0\0\0\0")), Some(Kind::AppImage));
        // .AppImage que não é ELF não roda.
        assert_eq!(kind_of(&write("falso.AppImage", b"oi")), None);
        assert_eq!(kind_of(&write("app.desktop", b"[Desktop Entry]\nType=Application\nName=X\nExec=x\n")), Some(Kind::Desktop));
        assert_eq!(kind_of(&write("ruim.desktop", b"nada\n")), None);
        assert_eq!(kind_of(&dir), None);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn atalho_desktop_lido() {
        let dir = temp_dir("atalho");
        let p = dir.join("editor.desktop");
        fs::write(&p, "[Desktop Entry]\nType=Application\nName=Meu Editor\nName[pt_BR]=Meu Editor BR\nIcon=accessories-text-editor\nExec=\"/opt/Meu Editor/bin\" %F\nPath=/tmp\nTerminal=false\n").unwrap();
        let l = read_launcher(&p).unwrap();
        assert!(l.name.starts_with("Meu Editor"));
        assert_eq!(l.icon.as_deref(), Some("accessories-text-editor"));
        assert_eq!(l.target, Target::App { args: vec!["/opt/Meu Editor/bin".into()], dir: Some("/tmp".into()), terminal: false, try_exec: None });
        let link = dir.join("site.desktop");
        fs::write(&link, "[Desktop Entry]\nType=Link\nName=Site\nURL=https://exemplo.com\n").unwrap();
        assert_eq!(read_launcher(&link).unwrap().target, Target::Link("https://exemplo.com".into()));
        // Comando no terminal: mide-terminal -e; pasta: a do Path=.
        let t = dir.join("top.desktop");
        fs::write(&t, "[Desktop Entry]\nType=Application\nName=Top\nExec=htop -d 5\nTerminal=true\n").unwrap();
        let launch = launch_for(&t, &Kind::Desktop).unwrap().unwrap();
        assert_eq!((launch.program.as_str(), launch.args.clone()), ("mide-terminal", vec!["-e".to_string(), "htop".into(), "-d".into(), "5".into()]));
        // TryExec que não existe: não roda, com aviso.
        let missing = dir.join("falta.desktop");
        fs::write(&missing, "[Desktop Entry]\nType=Application\nName=Falta\nExec=x\nTryExec=/nao/existe/programa\n").unwrap();
        assert!(launch_for(&missing, &Kind::Desktop).unwrap_err().contains("não está instalado"));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn executavel_roda_na_pasta_dele() {
        let dir = temp_dir("pasta");
        let p = dir.join("ola.sh");
        fs::write(&p, "#!/bin/sh\necho oi\n").unwrap();
        let launch = launch_for(&p, &Kind::Script { shebang: true }).unwrap().unwrap();
        assert_eq!(launch.program, p.to_string_lossy());
        assert_eq!(launch.dir, dir);
        let launch = launch_for(&p, &Kind::Script { shebang: false }).unwrap().unwrap();
        assert_eq!((launch.program.as_str(), launch.args.clone()), ("sh", vec![p.to_string_lossy().into_owned()]));
        fs::remove_dir_all(&dir).unwrap();
    }

    /// Pergunta, confiança e registro, com pastas de teste.
    #[test]
    fn confianca_marca_executavel_e_lembra() {
        crate::i18n::set_language("pt_BR");
        let base = temp_dir("confianca");
        fs::create_dir_all(base.join("dl")).unwrap();
        let d = Dirs { data: base.join("data"), downloads: base.join("dl"), home: base.clone() };
        let kind = Kind::Binary;
        // Executável local: roda sem perguntar.
        let local = base.join("prog");
        fs::write(&local, b"\x7fELF").unwrap();
        chmod(&local, 0o755);
        assert_eq!(reason_to_ask(&d, &local, &kind), None);
        // Sem permissão de execução: pergunta, com a pasta.
        chmod(&local, 0o644);
        assert!(reason_to_ask(&d, &local, &kind).unwrap().contains("não está marcado como executável"));
        // Confiar marca como executável (para quem lê) e lembra.
        trust(&d, &local).unwrap();
        assert_eq!(fs::metadata(&local).unwrap().permissions().mode() & 0o777, 0o755);
        assert!(is_trusted(&d, &local));
        assert_eq!(reason_to_ask(&d, &local, &kind), None);
        // Em Downloads: pergunta mesmo executável; confiado, não pergunta mais; se o
        // arquivo muda, a confiança acaba.
        let down = base.join("dl/App.AppImage");
        fs::write(&down, b"\x7fELF").unwrap();
        chmod(&down, 0o755);
        assert!(reason_to_ask(&d, &down, &Kind::AppImage).unwrap().contains("Downloads"));
        trust(&d, &down).unwrap();
        assert_eq!(reason_to_ask(&d, &down, &Kind::AppImage), None);
        fs::write(&down, b"\x7fELF outro").unwrap();
        assert!(reason_to_ask(&d, &down, &Kind::AppImage).is_some());
        trust(&d, &down).unwrap();
        let registry = fs::read_to_string(trust_file(&d)).unwrap();
        assert_eq!(registry.lines().count(), 2, "uma linha por arquivo: {registry}");
        // Marca do navegador: o site aparece (se o sistema de arquivos aceita xattr).
        let site = base.join("baixado");
        fs::write(&site, b"\x7fELF").unwrap();
        chmod(&site, 0o755);
        if rustix::fs::setxattr(&site, "user.xdg.origin.url", b"https://exemplo.com/baixado", rustix::fs::XattrFlags::empty()).is_ok() {
            assert_eq!(reason_to_ask(&d, &site, &kind).unwrap(), "Este programa veio de exemplo.com; abra só se confiar.");
        }
        // Atalho .desktop numa pasta de apps: confiável; o rótulo esconde o nome de um
        // atalho baixado até confiar.
        let apps = base.join("data/applications");
        fs::create_dir_all(&apps).unwrap();
        let installed = apps.join("x.desktop");
        fs::write(&installed, "[Desktop Entry]\nType=Application\nName=X\nExec=x\n").unwrap();
        assert_eq!(reason_to_ask(&d, &installed, &Kind::Desktop), None);
        // Atalho executável numa pasta comum (pendrive, zip extraído): a permissão não
        // basta; pergunta e mostra o nome do arquivo até confiar.
        let loose = base.join("foto.jpg.desktop");
        fs::write(&loose, "[Desktop Entry]\nType=Application\nName=foto.jpg\nIcon=image-x-generic\nExec=sh -c x\n").unwrap();
        chmod(&loose, 0o755);
        assert!(reason_to_ask(&d, &loose, &Kind::Desktop).unwrap().contains("ainda não foi marcado como confiável"));
        let mut loose_labels = Labels { dirs: Some(d.clone()), ..Default::default() };
        let loose_time = fs::metadata(&loose).unwrap().modified().unwrap();
        assert_eq!(loose_labels.get(&loose, loose_time), None);
        trust(&d, &loose).unwrap();
        assert_eq!(reason_to_ask(&d, &loose, &Kind::Desktop), None);
        loose_labels.clear();
        assert_eq!(loose_labels.get(&loose, loose_time), Some(("foto.jpg".into(), "image-x-generic".into())));
        // O atalho de uma pasta de apps mostra o app sem confirmação.
        let installed_time = fs::metadata(&installed).unwrap().modified().unwrap();
        assert_eq!(loose_labels.get(&installed, installed_time), Some(("X".into(), "application-x-executable".into())));
        // Em outro disco que não o da pasta pessoal (aqui, a "pessoal" no /proc):
        // pergunta mesmo executável.
        let elsewhere = Dirs { home: PathBuf::from("/proc"), ..d.clone() };
        let other = base.join("outro-disco");
        fs::write(&other, b"\x7fELF").unwrap();
        chmod(&other, 0o755);
        assert!(reason_to_ask(&elsewhere, &other, &Kind::Binary).unwrap().contains("num pendrive ou em outro disco"));
        assert_eq!(reason_to_ask(&d, &other, &Kind::Binary), None);
        let fake = base.join("dl/foto.jpg.desktop");
        fs::write(&fake, "[Desktop Entry]\nType=Application\nName=foto.jpg\nIcon=image-x-generic\nExec=x\n").unwrap();
        let modified = fs::metadata(&fake).unwrap().modified().unwrap();
        let mut labels = Labels { dirs: Some(d.clone()), ..Default::default() };
        assert_eq!(labels.get(&fake, modified), None);
        trust(&d, &fake).unwrap();
        labels.clear();
        assert_eq!(labels.get(&fake, modified), Some(("foto.jpg".into(), "image-x-generic".into())));
        fs::remove_dir_all(&base).unwrap();
    }
}
