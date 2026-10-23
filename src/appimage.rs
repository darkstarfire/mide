//! AppImages instalados pelo mide (E9), no estilo do Mac: o AppImage é copiado para a
//! pasta de apps do mide (`~/Aplicativos`, visível como o "Aplicativos" do Mac) e
//! ganha um atalho `.desktop` em `$XDG_DATA_HOME/applications`, para aparecer no
//! launcher e na busca como os outros apps.
//!
//! - **Pasta de apps:** `~/Aplicativos` (o nome segue o idioma: "Aplicativos",
//!   "Aplicaciones" ou "Applications"; se uma delas já existe, vale ela). A primeira
//!   instalação grava a pasta escolhida (`$XDG_DATA_HOME/mide/pasta-de-apps`): trocar o
//!   idioma depois não muda a pasta, e as três variantes continuam reconhecidas como
//!   pasta de apps. Os testes trocam por `MIDE_APPS_DIR`.
//! - **O que vem de dentro do AppImage:** o `.desktop` e o ícone (`.DirIcon` ou o
//!   ícone citado no `.desktop`), extraídos pelo próprio AppImage
//!   (`--appimage-extract <padrão>`) numa pasta temporária. Isso roda o início do
//!   AppImage: só acontece depois de a pessoa escolher Instalar (que é confiar nele,
//!   como Abrir), e só nos tipos 1 e 2 (com a marca `AI` no ELF). Um ELF com a extensão
//!   `.AppImage` e sem a marca (o "tipo 0") não é extraído: para ele o
//!   `--appimage-extract` seria só um argumento, e o programa podia simplesmente abrir.
//!   O runtime do tipo 1 ignora o padrão e extrai tudo: a pasta temporária fica no
//!   cache do usuário (disco), não no `/tmp` (que pode ser tmpfs, na memória). Sem o que
//!   extrair, o nome e a versão vêm do nome do arquivo e o ícone é o genérico.
//! - **Atalho gerado:** `mide-appimage-<id>.desktop`, com o `.desktop` de dentro (nomes
//!   traduzidos, categorias, tipos de arquivo), `Exec` apontando para o AppImage
//!   copiado, `Icon` para o ícone copiado em `$XDG_DATA_HOME/mide/appimage-icons` e a
//!   marca `X-Mide-AppImage=<caminho>`, que o launcher usa para "Mover para a Lixeira".
//! - **Remover:** o AppImage vai para a Lixeira; o atalho e o ícone são apagados.

use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use crate::places;

/// Começo do nome dos atalhos gerados.
pub const DESKTOP_PREFIX: &str = "mide-appimage-";
/// Chave do atalho com o caminho do AppImage instalado.
pub const MARK_KEY: &str = "X-Mide-AppImage";
/// Tempo máximo de cada extração (um AppImage que não responde não trava o instalador).
const EXTRACT_TIMEOUT: Duration = Duration::from_secs(30);

/// Pastas do instalador. Os testes passam as suas, sem mexer no ambiente do processo.
#[derive(Debug, Clone)]
pub struct Dirs {
    /// Pasta de apps (`~/Aplicativos`).
    pub apps: PathBuf,
    /// Dados do usuário (`$XDG_DATA_HOME`): atalhos, ícones e a Lixeira.
    pub data: PathBuf,
}

impl Dirs {
    pub fn current() -> Self {
        Self { apps: apps_dir(), data: places::data_home() }
    }

    pub fn applications(&self) -> PathBuf {
        self.data.join("applications")
    }

    fn icons(&self) -> PathBuf {
        self.data.join("mide").join("appimage-icons")
    }

    fn trash(&self) -> PathBuf {
        self.data.join("Trash")
    }
}

/// Registro da pasta de apps escolhida na primeira instalação.
fn apps_dir_record(data: &Path) -> PathBuf {
    data.join("mide").join("pasta-de-apps")
}

/// Pasta de apps do mide: `MIDE_APPS_DIR` (testes), a gravada na primeira instalação ou
/// `~/Aplicativos` (ver [`folder_for`]).
pub fn apps_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("MIDE_APPS_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(dir);
    }
    if let Some(dir) = recorded_apps_dir(&places::data_home()) {
        return dir;
    }
    let lang = ["LC_ALL", "LC_MESSAGES", "LANG"].iter().find_map(|v| std::env::var(v).ok().filter(|l| !l.is_empty())).unwrap_or_default();
    folder_for(&places::home_dir(), &lang)
}

/// Nome da pasta de apps no idioma (`pt_BR.UTF-8` → "Aplicativos").
pub fn localized_folder(lang: &str) -> &'static str {
    match lang.get(..2) {
        Some("pt") => "Aplicativos",
        Some("es") => "Aplicaciones",
        _ => "Applications",
    }
}

/// A pasta de apps dentro de `home`: a do idioma, ou "Applications" se só ela existe
/// (de outro instalador de AppImages, como o AppImageLauncher), ou a do idioma, nova.
pub fn folder_for(home: &Path, lang: &str) -> PathBuf {
    let localized = home.join(localized_folder(lang));
    let english = home.join("Applications");
    if !localized.is_dir() && english.is_dir() {
        return english;
    }
    localized
}

/// A pasta gravada na primeira instalação, se ainda existe.
fn recorded_apps_dir(data: &Path) -> Option<PathBuf> {
    let text = fs::read_to_string(apps_dir_record(data)).ok()?;
    let dir = PathBuf::from(text.trim());
    (dir.is_absolute() && dir.is_dir()).then_some(dir)
}

/// Pastas reconhecidas como pasta de apps: a atual, a gravada e as três variantes do
/// nome na pasta pessoal (trocar o idioma não "desinstala" nada).
pub fn managed_dirs(dirs: &Dirs) -> Vec<PathBuf> {
    let home = places::home_dir();
    let mut out = vec![dirs.apps.clone()];
    out.extend(recorded_apps_dir(&dirs.data));
    out.extend(["Aplicativos", "Aplicaciones", "Applications"].iter().map(|n| home.join(n)));
    out.iter().map(|d| fs::canonicalize(d).unwrap_or_else(|_| d.clone())).collect()
}

/// O arquivo está direto numa pasta de apps (ver [`managed_dirs`])?
fn in_managed_dir(dirs: &Dirs, path: &Path) -> bool {
    let parent = path.parent().map(|p| fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf()));
    parent.is_some_and(|p| managed_dirs(dirs).contains(&p))
}

/// O arquivo está na pasta de apps do mide (já instalado)?
pub fn is_managed(path: &Path) -> bool {
    in_managed_dir(&Dirs::current(), path)
}

/// O que se sabe de um AppImage antes de instalar.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Meta {
    /// Nome do app (do `.desktop` de dentro, ou do nome do arquivo).
    pub name: String,
    /// Identidade do app (nome do `.desktop` de dentro, sem a extensão): o mesmo app
    /// numa versão nova substitui o instalado.
    pub id: String,
    pub version: Option<String>,
    pub comment: Option<String>,
    /// Texto do `.desktop` de dentro do AppImage.
    pub desktop: Option<String>,
    /// Ícone extraído (na pasta temporária).
    pub icon: Option<PathBuf>,
    /// Tamanho do AppImage em bytes.
    pub size: u64,
}

/// Arquiteturas e sufixos comuns nos nomes de AppImage ("App-1.2-x86_64.AppImage").
const NAME_NOISE: &[&str] = &["x86_64", "x86-64", "amd64", "x64", "aarch64", "arm64", "armhf", "i386", "i686", "linux", "appimage", "universal"];

/// Nome e versão pelo nome do arquivo: "Krita-5.2.2-x86_64.AppImage" → ("Krita",
/// Some("5.2.2")); "Visual-Studio-Code.AppImage" → ("Visual Studio Code", None).
pub fn name_from_file_name(file_name: &str) -> (String, Option<String>) {
    let stem = match file_name.rsplit_once('.') {
        Some((stem, ext)) if ext.eq_ignore_ascii_case("appimage") && !stem.is_empty() => stem,
        _ => file_name,
    };
    let tokens: Vec<&str> = stem.split(['-', '_', ' ']).filter(|t| !t.is_empty()).collect();
    let is_version = |t: &str| {
        let t = t.strip_prefix(['v', 'V']).unwrap_or(t);
        t.starts_with(|c: char| c.is_ascii_digit()) && t.contains(|c: char| c == '.' || c.is_ascii_digit())
    };
    let mut name = Vec::new();
    let mut version = None;
    for t in &tokens {
        if NAME_NOISE.iter().any(|n| t.eq_ignore_ascii_case(n)) {
            continue;
        }
        if is_version(t) && !name.is_empty() {
            version = Some(t.trim_start_matches(['v', 'V']).to_string());
            break;
        }
        name.push(*t);
    }
    let name = if name.is_empty() { stem.to_string() } else { name.join(" ") };
    (name, version)
}

/// Identidade segura para nome de arquivo: letras, algarismos, `.`, `-` e `_`.
pub fn safe_id(text: &str) -> String {
    let id: String = text.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '-' }).collect();
    let id = id.trim_matches(['-', '.']).to_string();
    if id.is_empty() { "app".into() } else { id }
}

/// É um AppImage? Tipo 1 ou 2 pela marca `AI` do ELF; `Some(0)` para um ELF com a
/// extensão `.AppImage` e sem a marca.
pub fn appimage_type(path: &Path) -> Option<u8> {
    let mut head = [0u8; 11];
    fs::File::open(path).and_then(|mut f| f.read_exact(&mut head)).ok()?;
    if !head.starts_with(b"\x7fELF") {
        return None;
    }
    if &head[8..10] == b"AI" && matches!(head[10], 1 | 2) {
        return Some(head[10]);
    }
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case("appimage")).then_some(0)
}

/// Pede ao AppImage para extrair o que casa com `pattern` em `work/squashfs-root`.
/// Sem esperar mais que [`EXTRACT_TIMEOUT`]; a saída do AppImage é descartada.
fn extract(appimage: &Path, work: &Path, pattern: &str) -> bool {
    let mut cmd = Command::new(appimage);
    cmd.arg("--appimage-extract").arg(pattern).current_dir(work).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    cmd.env_remove("APPIMAGE_EXTRACT_AND_RUN");
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(err) => {
            tracing::warn!("AppImage {}: não deu para extrair {pattern}: {err}", appimage.display());
            return false;
        }
    };
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) => {}
            Err(_) => return false,
        }
        if start.elapsed() > EXTRACT_TIMEOUT {
            // O grupo inteiro (criado com `process_group(0)`): os filhos do runtime
            // também param de escrever na pasta temporária.
            match rustix::process::Pid::from_raw(child.id() as i32) {
                Some(pid) => {
                    let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
                }
                None => {
                    let _ = child.kill();
                }
            }
            let _ = child.wait();
            tracing::warn!("AppImage {}: a extração de {pattern} demorou demais", appimage.display());
            return false;
        }
        std::thread::sleep(Duration::from_millis(15));
    }
}

/// Arquivo dentro de `root` (links resolvidos, sem sair de `root`), se existir.
fn inside(root: &Path, path: &Path) -> Option<PathBuf> {
    let root = fs::canonicalize(root).ok()?;
    let real = fs::canonicalize(path).ok()?;
    (real.starts_with(&root) && real.is_file()).then_some(real)
}

/// Formato de uma imagem de ícone pelo começo do arquivo: "png" ou "svg".
fn icon_format(path: &Path) -> Option<&'static str> {
    let mut head = [0u8; 512];
    let n = fs::File::open(path).and_then(|mut f| f.read(&mut head)).ok()?;
    let head = &head[..n];
    if head.starts_with(b"\x89PNG") {
        return Some("png");
    }
    let text = String::from_utf8_lossy(head);
    (text.contains("<svg") || text.trim_start().starts_with("<?xml")).then_some("svg")
}

/// Lê o nome, a versão e o ícone de um AppImage, extraindo o `.desktop` e o ícone de
/// dentro dele em `work` (pasta temporária vazia). Só os tipos 1 e 2 são extraídos
/// (ver o começo do módulo); os outros ficam com o nome do arquivo.
pub fn inspect(appimage: &Path, work: &Path) -> Meta {
    let can_extract = matches!(appimage_type(appimage), Some(1 | 2));
    if !can_extract {
        tracing::info!("AppImage {}: sem a marca AI (tipo 0), nada é extraído", appimage.display());
    }
    inspect_with(appimage, work, can_extract)
}

fn inspect_with(appimage: &Path, work: &Path, can_extract: bool) -> Meta {
    let extract = |appimage: &Path, work: &Path, pattern: &str| can_extract && extract(appimage, work, pattern);
    let file_name = appimage.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let (name, version) = name_from_file_name(&file_name);
    let mut meta = Meta { id: safe_id(&name.to_lowercase()), name, version, size: fs::metadata(appimage).map(|m| m.len()).unwrap_or(0), ..Default::default() };
    let root = work.join("squashfs-root");
    if can_extract && !extract(appimage, work, "*.desktop") {
        tracing::info!("AppImage {}: sem o .desktop de dentro (nome pelo arquivo)", appimage.display());
    }
    // O .desktop de dentro fica na raiz do AppImage.
    let desktop = fs::read_dir(&root).ok().and_then(|entries| {
        let mut found: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "desktop")).collect();
        found.sort();
        found.into_iter().find_map(|p| inside(&root, &p).map(|real| (p, real)))
    });
    let mut icon_name = None;
    if let Some((path, real)) = desktop
        && let Ok(text) = fs::read_to_string(&real)
        && text.len() <= 512 * 1024
    {
        let locales = freedesktop_desktop_entry::get_languages_from_env();
        if let Ok(entry) = freedesktop_desktop_entry::DesktopEntry::from_str(&path, &text, Some(&locales)) {
            if let Some(n) = entry.name(&locales).map(|n| n.trim().to_string()).filter(|n| !n.is_empty() && !n.contains('\n')) {
                meta.name = n;
            }
            meta.comment = entry.comment(&locales).map(|c| c.trim().to_string()).filter(|c| !c.is_empty());
            if let Some(v) = entry.desktop_entry("X-AppImage-Version").map(str::trim).filter(|v| !v.is_empty()) {
                meta.version = Some(v.to_string());
            }
            icon_name = entry.icon().map(str::trim).filter(|i| !i.is_empty() && !i.contains('/')).map(str::to_string);
        }
        if let Some(stem) = path.file_stem() {
            meta.id = safe_id(&stem.to_string_lossy());
        }
        meta.desktop = Some(text);
    }
    // Ícone: o citado no .desktop (SVG antes de PNG), senão o .DirIcon.
    let mut candidates = Vec::new();
    if let Some(icon) = &icon_name {
        if !icon.contains(['*', '?', '[']) {
            extract(appimage, work, &format!("{icon}.*"));
        }
        candidates.extend(["svg", "png"].iter().map(|ext| root.join(format!("{icon}.{ext}"))));
    }
    extract(appimage, work, ".DirIcon");
    candidates.push(root.join(".DirIcon"));
    meta.icon = candidates.iter().find_map(|c| inside(&root, c).filter(|p| icon_format(p).is_some()));
    tracing::info!(
        "AppImage {}: \"{}\" (id {}, versão {}, ícone {})",
        appimage.display(),
        meta.name,
        meta.id,
        meta.version.as_deref().unwrap_or("?"),
        meta.icon.as_ref().map_or("genérico".into(), |p| p.display().to_string())
    );
    meta
}

/// Um app instalado pelo mide.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    /// O atalho `mide-appimage-<id>.desktop`.
    pub desktop: PathBuf,
    /// O AppImage na pasta de apps.
    pub appimage: PathBuf,
    pub name: String,
    pub version: Option<String>,
    /// O ícone copiado (caminho absoluto) ou o nome do ícone.
    pub icon: Option<String>,
}

/// Valor de uma chave sem tradução no grupo `[Desktop Entry]`.
fn entry_value(text: &str, key: &str) -> Option<String> {
    let mut main = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            main = line == "[Desktop Entry]";
            continue;
        }
        if main && let Some((k, v)) = line.split_once('=') && k.trim() == key {
            return Some(v.trim().to_string());
        }
    }
    None
}

/// Lê um atalho gerado pelo instalador (com a marca `X-Mide-AppImage`).
pub fn read_installed(desktop: &Path) -> Option<Installed> {
    let text = fs::read_to_string(desktop).ok()?;
    let appimage = PathBuf::from(unescape_value(&entry_value(&text, MARK_KEY)?));
    Some(Installed {
        desktop: desktop.to_path_buf(),
        appimage,
        name: entry_value(&text, "Name").unwrap_or_default(),
        version: entry_value(&text, "X-AppImage-Version").filter(|v| !v.is_empty()),
        icon: entry_value(&text, "Icon").map(|i| unescape_value(&i)),
    })
}

/// O app `id` já instalado.
pub fn installed(dirs: &Dirs, id: &str) -> Option<Installed> {
    read_installed(&desktop_path(dirs, id))
}

fn desktop_path(dirs: &Dirs, id: &str) -> PathBuf {
    dirs.applications().join(format!("{DESKTOP_PREFIX}{id}.desktop"))
}

/// Escapa um valor de texto do `.desktop` (a barra invertida).
fn escape_value(value: &str) -> String {
    value.replace('\\', "\\\\")
}

fn unescape_value(value: &str) -> String {
    value.replace("\\\\", "\\")
}

/// O programa do `Exec`: entre aspas (com `"`, `` ` ``, `$` e `\` escapados, como pede a
/// especificação) e já escapado como texto do `.desktop`.
fn exec_program(path: &Path) -> String {
    let mut quoted = String::from("\"");
    for c in path.to_string_lossy().chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            quoted.push('\\');
        }
        quoted.push(c);
    }
    quoted.push('"');
    escape_value(&quoted)
}

/// Troca o programa (o primeiro argumento) de um `Exec`, mantendo o resto.
fn replace_program(exec: &str, program: &str) -> String {
    let exec = exec.trim_start();
    let rest = if let Some(after) = exec.strip_prefix('"') {
        // Fim das aspas (pulando os escapes).
        let mut end = None;
        let mut escaped = false;
        for (i, c) in after.char_indices() {
            match c {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => {
                    end = Some(i + 1);
                    break;
                }
                _ => {}
            }
        }
        end.map_or("", |e| &after[e..])
    } else {
        exec.find(char::is_whitespace).map_or("", |i| &exec[i..])
    };
    format!("{program}{}", rest.trim_end())
}

/// Chaves que o atalho gerado não leva: o `TryExec` (o programa agora é o AppImage),
/// o `DBusActivatable` (o app não está registrado no D-Bus) e o que esconderia o app.
const DROPPED_KEYS: &[&str] = &["TryExec", "DBusActivatable", "Hidden", "NoDisplay", "OnlyShowIn", "NotShowIn", MARK_KEY];

/// Texto do atalho gerado: o `.desktop` de dentro do AppImage (traduções, categorias,
/// tipos de arquivo) com `Exec` e `Icon` apontando para o que foi instalado, ou um
/// atalho novo quando o AppImage não traz um.
pub fn desktop_text(meta: &Meta, appimage: &Path, icon: Option<&Path>) -> String {
    let program = exec_program(appimage);
    let icon_value = icon.map(|p| escape_value(&p.to_string_lossy())).unwrap_or_else(|| "application-x-executable".into());
    let mark = format!("{MARK_KEY}={}", escape_value(&appimage.to_string_lossy()));
    let clean = |s: &str| s.replace(['\n', '\r'], " ");
    let Some(original) = &meta.desktop else {
        let mut text = format!("[Desktop Entry]\nType=Application\nName={}\nExec={program}\nIcon={icon_value}\nTerminal=false\nCategories=Utility;\n", clean(&meta.name));
        if let Some(c) = &meta.comment {
            text.push_str(&format!("Comment={}\n", clean(c)));
        }
        if let Some(v) = &meta.version {
            text.push_str(&format!("X-AppImage-Version={}\n", clean(v)));
        }
        text.push_str(&mark);
        text.push('\n');
        return text;
    };
    let mut out = String::new();
    let mut group = String::new();
    let (mut has_exec, mut has_type, mut has_name, mut has_version) = (false, false, false, false);
    let mut main_end = None;
    for line in original.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            if group == "[Desktop Entry]" && main_end.is_none() {
                main_end = Some(out.len());
            }
            group = trimmed.to_string();
            out.push_str(trimmed);
            out.push('\n');
            continue;
        }
        let main = group == "[Desktop Entry]";
        let Some((key, value)) = trimmed.split_once('=') else {
            out.push_str(line);
            out.push('\n');
            continue;
        };
        let key = key.trim();
        let base = key.split('[').next().unwrap_or(key);
        if DROPPED_KEYS.contains(&base) {
            continue;
        }
        match base {
            "Exec" => {
                has_exec |= main;
                out.push_str(&format!("{key}={}\n", safe_exec(&replace_program(value, &program), &program)));
            }
            "Icon" => out.push_str(&format!("{key}={icon_value}\n")),
            _ => {
                if main {
                    has_type |= base == "Type";
                    has_name |= key == "Name";
                    has_version |= key == "X-AppImage-Version";
                }
                out.push_str(line);
                out.push('\n');
            }
        }
    }
    if group == "[Desktop Entry]" && main_end.is_none() {
        main_end = Some(out.len());
    }
    // O que faltar no grupo principal, e a marca do mide, no fim dele.
    let mut extra = String::new();
    if !has_type {
        extra.push_str("Type=Application\n");
    }
    if !has_name {
        extra.push_str(&format!("Name={}\n", clean(&meta.name)));
    }
    if !has_exec {
        extra.push_str(&format!("Exec={program}\n"));
    }
    if !has_version && let Some(v) = &meta.version {
        extra.push_str(&format!("X-AppImage-Version={}\n", clean(v)));
    }
    extra.push_str(&mark);
    extra.push('\n');
    match main_end {
        Some(at) => out.insert_str(at, &extra),
        // Sem o grupo principal: o atalho é montado do zero.
        None => return desktop_text(&Meta { desktop: None, ..meta.clone() }, appimage, icon),
    }
    out
}

/// Argumento do `Exec` de dentro do AppImage que pode ficar no atalho: códigos de
/// arquivo (`%F`) e opções simples (`--nova-janela`, `--x=1`). O launcher roda o `Exec`
/// por um shell: aspas, `$`, `;`, `|` e afins de dentro do AppImage não passam.
fn safe_arg(arg: &str) -> bool {
    !arg.is_empty() && arg.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '%' | '-' | '_' | '.' | '/' | '=' | ':' | ',' | '+' | '@'))
}

/// `Exec` gerado: o programa (já entre aspas) e só os argumentos seguros de dentro.
fn safe_exec(exec: &str, program: &str) -> String {
    let rest = exec.strip_prefix(program).unwrap_or("");
    let mut out = program.to_string();
    for arg in rest.split_whitespace() {
        if safe_arg(arg) {
            out.push(' ');
            out.push_str(arg);
        } else {
            tracing::info!("atalho do AppImage: argumento do Exec deixado de fora: {arg}");
        }
    }
    out
}

/// Nome do AppImage na pasta de apps ("Meu App.AppImage"). O nome vem de dentro do
/// AppImage e vai para o `Exec` do atalho, que o launcher roda por um shell: só letras,
/// algarismos, espaço (um só entre as palavras), `.`, `-`, `_` e parênteses; o resto
/// vira `-`.
fn file_name_for(name: &str) -> String {
    let base: String = name.chars().map(|c| if c.is_alphanumeric() || matches!(c, ' ' | '.' | '-' | '_' | '(' | ')') { c } else { '-' }).collect();
    let base = base.split_whitespace().collect::<Vec<_>>().join(" ");
    let base = base.trim_start_matches(['.', '-']);
    format!("{}.AppImage", if base.is_empty() { "App" } else { base })
}

/// Copia o AppImage com o progresso (de 0 a 1) para um temporário escondido na mesma
/// pasta e troca pelo destino só no fim (sem ficar meio copiado), executável.
fn copy_with_progress(src: &Path, dest: &Path, progress: &mut dyn FnMut(f32)) -> std::io::Result<()> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let dir = dest.parent().ok_or_else(|| std::io::Error::other("destino sem pasta"))?;
    let name = dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = dir.join(format!(".{name}.{}.parte", std::process::id()));
    let total = fs::metadata(src)?.len().max(1);
    let result = (|| {
        let mut input = fs::File::open(src)?;
        let mut output = fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o755).open(&tmp)?;
        let mut buf = vec![0u8; 1 << 20];
        let mut done = 0u64;
        loop {
            let n = input.read(&mut buf)?;
            if n == 0 {
                break;
            }
            output.write_all(&buf[..n])?;
            done += n as u64;
            progress((done as f64 / total as f64).min(1.0) as f32);
        }
        output.sync_all()?;
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o755))?;
        fs::rename(&tmp, dest)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// Instala o AppImage: copia para a pasta de apps (com o progresso), guarda o ícone e
/// grava o atalho. Se o app já está instalado (mesma identidade), substitui: o
/// AppImage antigo vai para a Lixeira se o nome mudou.
pub fn install(src: &Path, meta: &Meta, dirs: &Dirs, progress: &mut dyn FnMut(f32)) -> std::io::Result<Installed> {
    fs::create_dir_all(&dirs.apps)?;
    // A primeira instalação grava a pasta: trocar o idioma depois não muda a pasta.
    let record = apps_dir_record(&dirs.data);
    if !record.exists() {
        if let Some(dir) = record.parent() {
            fs::create_dir_all(dir)?;
        }
        let dir = fs::canonicalize(&dirs.apps).unwrap_or_else(|_| dirs.apps.clone());
        crate::files::write_atomic(&record, format!("{}\n", dir.display()).as_bytes())?;
    }
    let previous = installed(dirs, &meta.id);
    let real = |p: &Path| fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let src_real = real(src);
    let wanted = dirs.apps.join(file_name_for(&meta.name));
    let is_previous = |p: &Path| previous.as_ref().is_some_and(|old| real(&old.appimage) == real(p));
    // O nome é de outro arquivo (outro app, ou algo que a pessoa pôs lá): número no fim.
    let dest = if !wanted.exists() || is_previous(&wanted) || real(&wanted) == src_real {
        wanted
    } else {
        let stem = file_name_for(&meta.name).trim_end_matches(".AppImage").to_string();
        let name = crate::files::numbered_name(&dirs.apps, &stem, "AppImage");
        dirs.apps.join(name)
    };
    if real(&dest) == src_real {
        // Já está na pasta de apps: só a integração.
        progress(1.0);
    } else {
        copy_with_progress(src, &dest, progress)?;
    }
    if let Some(old) = &previous
        && old.appimage.exists()
        && real(&old.appimage) != real(&dest)
        && old.appimage.starts_with(&dirs.apps)
    {
        match places::move_to_trash(&old.appimage, &dirs.trash()) {
            Ok(_) => tracing::info!("versão anterior na Lixeira: {}", old.appimage.display()),
            Err(err) => tracing::warn!("versão anterior ficou em {}: {err}", old.appimage.display()),
        }
    }
    let icon = match meta.icon.as_deref().and_then(|p| icon_format(p).map(|f| (p, f))) {
        Some((path, format)) => {
            fs::create_dir_all(dirs.icons())?;
            // Ícone antigo em outro formato sai.
            for ext in ["png", "svg"] {
                let _ = fs::remove_file(dirs.icons().join(format!("{}.{ext}", meta.id)));
            }
            let target = dirs.icons().join(format!("{}.{format}", meta.id));
            crate::files::write_atomic(&target, &fs::read(path)?)?;
            Some(target)
        }
        None => None,
    };
    let desktop = desktop_path(dirs, &meta.id);
    fs::create_dir_all(dirs.applications())?;
    crate::files::write_atomic(&desktop, desktop_text(meta, &dest, icon.as_deref()).as_bytes())?;
    tracing::info!("instalado: \"{}\" em {} (atalho {})", meta.name, dest.display(), desktop.display());
    Ok(Installed {
        desktop,
        appimage: dest,
        name: meta.name.clone(),
        version: meta.version.clone(),
        icon: Some(icon.map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|| "application-x-executable".into())),
    })
}

/// Remove um app instalado pelo mide: o AppImage vai para a Lixeira e o atalho e o
/// ícone são apagados. Só atalhos gerados pelo instalador (`mide-appimage-*.desktop`
/// com a marca).
pub fn uninstall(desktop: &Path, dirs: &Dirs) -> std::io::Result<Installed> {
    let is_ours = desktop.file_name().is_some_and(|n| n.to_string_lossy().starts_with(DESKTOP_PREFIX));
    let info = read_installed(desktop).filter(|_| is_ours).ok_or_else(|| std::io::Error::other("não é um app instalado pelo mide"))?;
    // Só um AppImage numa pasta de apps vai para a Lixeira: a marca do atalho pode ter
    // sido trocada para apontar para qualquer arquivo.
    if info.appimage.is_file() && !in_managed_dir(dirs, &info.appimage) {
        tracing::warn!("AppImage fora da pasta de apps fica onde está: {}", info.appimage.display());
    } else if info.appimage.is_file() {
        if appimage_type(&info.appimage).is_none() {
            return Err(std::io::Error::other(format!("{} não é um AppImage", info.appimage.display())));
        }
        places::move_to_trash(&info.appimage, &dirs.trash())?;
        tracing::info!("AppImage na Lixeira: {}", info.appimage.display());
    }
    // Só o ícone copiado pelo instalador (caminho real dentro da pasta de ícones: `..`
    // e links para fora não valem).
    let icons = fs::canonicalize(dirs.icons()).ok();
    if let Some(icon) = info.icon.as_deref().and_then(|i| fs::canonicalize(i).ok()).filter(|p| icons.as_ref().is_some_and(|d| p.starts_with(d))) {
        let _ = fs::remove_file(icon);
    }
    fs::remove_file(desktop)?;
    tracing::info!("removido: \"{}\" ({})", info.name, desktop.display());
    Ok(info)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mide-appimage-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn nome_e_versao_pelo_arquivo() {
        assert_eq!(name_from_file_name("Krita-5.2.2-x86_64.AppImage"), ("Krita".into(), Some("5.2.2".into())));
        assert_eq!(name_from_file_name("Visual-Studio-Code.AppImage"), ("Visual Studio Code".into(), None));
        assert_eq!(name_from_file_name("balenaEtcher-1.18.11-x64.AppImage"), ("balenaEtcher".into(), Some("1.18.11".into())));
        assert_eq!(name_from_file_name("app_v2.0_amd64.appimage"), ("app".into(), Some("2.0".into())));
        assert_eq!(name_from_file_name("2048.AppImage"), ("2048".into(), None));
    }

    #[test]
    fn pasta_de_apps_no_idioma() {
        let home = temp_dir("pasta");
        assert_eq!(folder_for(&home, "pt_BR.UTF-8"), home.join("Aplicativos"));
        assert_eq!(folder_for(&home, "es_ES.UTF-8"), home.join("Aplicaciones"));
        assert_eq!(folder_for(&home, "C"), home.join("Applications"));
        // Só a "Applications" existe (de outro instalador): vale ela.
        fs::create_dir_all(home.join("Applications")).unwrap();
        assert_eq!(folder_for(&home, "pt_BR.UTF-8"), home.join("Applications"));
        fs::create_dir_all(home.join("Aplicativos")).unwrap();
        assert_eq!(folder_for(&home, "pt_BR.UTF-8"), home.join("Aplicativos"));
        fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn exec_troca_so_o_programa() {
        let p = Path::new("/home/ana/Aplicativos/Meu App.AppImage");
        let prog = exec_program(p);
        assert_eq!(prog, "\"/home/ana/Aplicativos/Meu App.AppImage\"");
        assert_eq!(replace_program("AppRun %U", &prog), format!("{prog} %U"));
        assert_eq!(replace_program(r#""/tmp/.mount_x/app bin" --new-window %F"#, &prog), format!("{prog} --new-window %F"));
        assert_eq!(replace_program("app", &prog), prog);
        // Com $ e aspas no caminho: escapados pela especificação.
        assert_eq!(exec_program(Path::new("/a/$x\"y")), r#""/a/\\$x\\"y""#);
        // O Exec gerado volta igual pelo leitor do E8.
        let args = crate::programs::exec_args(&crate::programs::unescape(&format!("{prog} %U")), "", None, Path::new("/x")).unwrap();
        assert_eq!(args, vec![p.to_string_lossy().into_owned()]);
    }

    #[test]
    fn atalho_gerado_do_desktop_de_dentro() {
        let meta = Meta {
            name: "Editor".into(),
            id: "org.exemplo.Editor".into(),
            version: Some("2.1".into()),
            desktop: Some("[Desktop Entry]\nType=Application\nName=Editor\nName[pt_BR]=Editor BR\nExec=AppRun %F\nTryExec=AppRun\nIcon=editor\nNoDisplay=false\nCategories=Office;\n\n[Desktop Action nova]\nName=Nova janela\nExec=AppRun --nova\n".into()),
            ..Default::default()
        };
        let text = desktop_text(&meta, Path::new("/h/Aplicativos/Editor.AppImage"), Some(Path::new("/h/.local/share/mide/appimage-icons/org.exemplo.Editor.png")));
        assert!(text.contains("Name[pt_BR]=Editor BR\n"), "{text}");
        assert!(text.contains("Exec=\"/h/Aplicativos/Editor.AppImage\" %F\n"), "{text}");
        assert!(text.contains("Exec=\"/h/Aplicativos/Editor.AppImage\" --nova\n"), "{text}");
        assert!(text.contains("Icon=/h/.local/share/mide/appimage-icons/org.exemplo.Editor.png\n"), "{text}");
        assert!(!text.contains("TryExec") && !text.contains("NoDisplay"), "{text}");
        // A marca e a versão no grupo principal (antes da ação).
        let mark = text.find("X-Mide-AppImage=/h/Aplicativos/Editor.AppImage").unwrap();
        assert!(mark < text.find("[Desktop Action nova]").unwrap());
        assert!(text.contains("X-AppImage-Version=2.1\n"));
        // Sem .desktop de dentro: atalho novo, com o ícone genérico.
        let bare = desktop_text(&Meta { name: "Jogo".into(), id: "jogo".into(), ..Default::default() }, Path::new("/h/A/Jogo.AppImage"), None);
        assert!(bare.starts_with("[Desktop Entry]\nType=Application\nName=Jogo\nExec=\"/h/A/Jogo.AppImage\"\nIcon=application-x-executable\n"), "{bare}");
    }

    /// Instalar, substituir por outra versão e remover, com pastas de teste.
    #[test]
    fn instalar_substituir_e_remover() {
        let base = temp_dir("instalar");
        let dirs = Dirs { apps: base.join("Aplicativos"), data: base.join("data") };
        let src = base.join("Editor-1.0-x86_64.AppImage");
        fs::write(&src, b"\x7fELF\x02\x01\x01\0AI\x02\0\0\0\0\0versao1").unwrap();
        let icon = base.join("icone");
        fs::write(&icon, b"\x89PNG\r\n\x1a\nfalso").unwrap();
        let meta = Meta { name: "Editor".into(), id: "org.exemplo.Editor".into(), version: Some("1.0".into()), icon: Some(icon.clone()), ..Default::default() };
        let mut seen = Vec::new();
        let done = install(&src, &meta, &dirs, &mut |p| seen.push(p)).unwrap();
        assert_eq!(done.appimage, dirs.apps.join("Editor.AppImage"));
        assert_eq!(seen.last().copied(), Some(1.0));
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(&done.appimage).unwrap().permissions().mode() & 0o777, 0o755);
        assert_eq!(fs::read(&done.appimage).unwrap(), fs::read(&src).unwrap());
        let found = installed(&dirs, "org.exemplo.Editor").unwrap();
        assert_eq!(found.appimage, done.appimage);
        assert_eq!(found.version.as_deref(), Some("1.0"));
        assert!(found.icon.as_deref().unwrap().ends_with("appimage-icons/org.exemplo.Editor.png"));
        // Outra versão: o mesmo arquivo é trocado; nada vai para a Lixeira.
        let src2 = base.join("Editor-2.0.AppImage");
        fs::write(&src2, b"\x7fELF\x02\x01\x01\0AI\x02\0\0\0\0\0versao2").unwrap();
        let meta2 = Meta { version: Some("2.0".into()), ..meta.clone() };
        let again = install(&src2, &meta2, &dirs, &mut |_| {}).unwrap();
        assert_eq!(again.appimage, done.appimage);
        assert!(fs::read(&again.appimage).unwrap().ends_with(b"versao2"));
        assert_eq!(installed(&dirs, "org.exemplo.Editor").unwrap().version.as_deref(), Some("2.0"));
        assert_eq!(places::trash_count(&dirs.trash()), 0);
        // Outro app com o mesmo nome não pisa no primeiro.
        let other = install(&src2, &Meta { id: "outro".into(), icon: None, ..meta.clone() }, &dirs, &mut |_| {}).unwrap();
        assert_eq!(other.appimage, dirs.apps.join("Editor (1).AppImage"));
        // A primeira instalação gravou a pasta de apps.
        assert_eq!(recorded_apps_dir(&dirs.data), Some(fs::canonicalize(&dirs.apps).unwrap()));
        // Com a pasta atual em outro lugar (idioma trocado), a gravada continua valendo.
        let other_lang = Dirs { apps: base.join("Applications"), ..dirs.clone() };
        assert!(in_managed_dir(&other_lang, &done.appimage));
        assert!(!in_managed_dir(&other_lang, &src));
        // Remover: AppImage na Lixeira, atalho e ícone apagados.
        let gone = uninstall(&found.desktop, &dirs).unwrap();
        assert_eq!(gone.appimage, done.appimage);
        assert!(!done.appimage.exists() && !found.desktop.exists());
        assert!(!Path::new(found.icon.as_deref().unwrap()).exists());
        assert_eq!(places::trash_count(&dirs.trash()), 1);
        // Marca trocada para um arquivo fora da pasta de apps e ícone com `..`: o atalho
        // sai, mas o arquivo e o "ícone" ficam onde estão.
        let outside = base.join("importante.AppImage");
        fs::write(&outside, b"\x7fELF\x02\x01\x01\0AI\x02\0\0\0\0\0").unwrap();
        let victim = base.join("vitima.png");
        fs::write(&victim, b"x").unwrap();
        fs::create_dir_all(dirs.icons()).unwrap();
        let forged = dirs.applications().join(format!("{DESKTOP_PREFIX}falso.desktop"));
        fs::write(&forged, format!("[Desktop Entry]\nType=Application\nName=F\nExec=x\nIcon={}/../../../vitima.png\n{MARK_KEY}={}\n", dirs.icons().display(), outside.display())).unwrap();
        uninstall(&forged, &dirs).unwrap();
        assert!(outside.exists() && victim.exists() && !forged.exists());
        assert_eq!(places::trash_count(&dirs.trash()), 1);
        // Um atalho comum não é removido por aqui.
        let plain = dirs.applications().join("firefox.desktop");
        fs::write(&plain, "[Desktop Entry]\nType=Application\nName=F\nExec=firefox\n").unwrap();
        assert!(uninstall(&plain, &dirs).is_err());
        assert!(plain.exists());
        fs::remove_dir_all(&base).unwrap();
    }

    /// Lê o .desktop e o ícone pelo `--appimage-extract` de um AppImage falso (um
    /// script que responde como o de verdade); um `.DirIcon` apontando para fora não vale.
    #[test]
    fn le_o_que_tem_dentro() {
        use std::os::unix::fs::PermissionsExt;
        let base = temp_dir("dentro");
        let fake = base.join("Editor-1.0-x86_64.AppImage");
        fs::write(
            &fake,
            "#!/bin/sh\n[ \"$1\" = --appimage-extract ] || exit 1\nmkdir -p squashfs-root\ncase \"$2\" in\n\
             '*.desktop') printf '[Desktop Entry]\\nType=Application\\nName=Editor Bonito\\nIcon=editor\\nExec=AppRun\\nX-AppImage-Version=1.0.3\\n' > squashfs-root/org.ex.Editor.desktop ;;\n\
             'editor.*') printf '<svg xmlns=\"http://www.w3.org/2000/svg\"/>' > squashfs-root/editor.svg ;;\n\
             '.DirIcon') ln -sf /etc/hostname squashfs-root/.DirIcon ;;\nesac\n",
        )
        .unwrap();
        fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
        let work = base.join("work");
        fs::create_dir_all(&work).unwrap();
        let meta = inspect_with(&fake, &work, true);
        assert_eq!((meta.name.as_str(), meta.id.as_str(), meta.version.as_deref()), ("Editor Bonito", "org.ex.Editor", Some("1.0.3")));
        assert!(meta.icon.as_ref().unwrap().ends_with("editor.svg"), "{:?}", meta.icon);
        assert!(meta.desktop.as_deref().unwrap().contains("Name=Editor Bonito"));
        // Sem nada dentro (o AppImage não extrai): nome e versão pelo arquivo.
        let mute = base.join("Jogo-2.5.AppImage");
        fs::write(&mute, "#!/bin/sh\nexit 1\n").unwrap();
        fs::set_permissions(&mute, fs::Permissions::from_mode(0o755)).unwrap();
        let work2 = base.join("work2");
        fs::create_dir_all(&work2).unwrap();
        let meta = inspect(&mute, &work2);
        assert_eq!((meta.name.as_str(), meta.id.as_str(), meta.version.as_deref(), meta.icon.as_ref()), ("Jogo", "jogo", Some("2.5"), None));
        fs::remove_dir_all(&base).unwrap();
    }

    /// Sem a marca AI (tipo 0, ou nem ELF), o AppImage não é rodado para extrair.
    #[test]
    fn tipo_0_nao_roda() {
        use std::os::unix::fs::PermissionsExt;
        let base = temp_dir("tipo0");
        let marker = base.join("rodou");
        let fake = base.join("Jogo-1.0.AppImage");
        fs::write(&fake, format!("#!/bin/sh\ntouch '{}'\n", marker.display())).unwrap();
        fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
        let work = base.join("work");
        fs::create_dir_all(&work).unwrap();
        let meta = inspect(&fake, &work);
        assert_eq!((meta.name.as_str(), meta.version.as_deref()), ("Jogo", Some("1.0")));
        assert!(!marker.exists(), "o AppImage sem a marca rodou");
        fs::remove_dir_all(&base).unwrap();
    }

    /// Nome e `Exec` do atalho não levam nada que o shell do launcher interprete.
    #[test]
    fn nome_e_exec_sem_shell() {
        assert_eq!(file_name_for("$(id>/tmp/x)"), "(id--tmp-x).AppImage");
        assert_eq!(file_name_for("Meu  App \"2\" `x`"), "Meu App -2- -x-.AppImage");
        assert_eq!(file_name_for("..oculto"), "oculto.AppImage");
        assert_eq!(file_name_for("Editor (Beta) 2.0"), "Editor (Beta) 2.0.AppImage");
        let meta = Meta {
            name: "X".into(),
            id: "x".into(),
            desktop: Some("[Desktop Entry]\nType=Application\nName=X\nExec=AppRun %U; touch $(id) \"a b\" --nova-janela\n".into()),
            ..Default::default()
        };
        let text = desktop_text(&meta, Path::new("/h/A/X.AppImage"), None);
        assert!(text.contains("Exec=\"/h/A/X.AppImage\" touch --nova-janela\n"), "{text}");
    }

    #[test]
    fn tipo_de_appimage() {
        let dir = temp_dir("tipo");
        let p = dir.join("x");
        fs::write(&p, b"\x7fELF\x02\x01\x01\0AI\x02\0\0").unwrap();
        assert_eq!(appimage_type(&p), Some(2));
        fs::write(&p, b"\x7fELF\x02\x01\x01\0AI\x01\0\0").unwrap();
        assert_eq!(appimage_type(&p), Some(1));
        fs::write(&p, b"#!/bin/sh\necho oi\n").unwrap();
        assert_eq!(appimage_type(&p), None);
        fs::remove_dir_all(&dir).unwrap();
    }
}
