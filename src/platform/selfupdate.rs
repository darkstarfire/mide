//! Atualização do mide pelo código-fonte, para quem não o tem pelo apt (Fedora, Arch,
//! Slackware e o PC de desenvolvimento): o app Atualização de Software vê a versão nova
//! publicada (a tag `vX.Y.Z` mais nova do GitHub), mostra o que mudou e atualiza sem
//! terminal.
//!
//! A versão nova é compilada numa cópia própria do projeto
//! (`~/.cache/mide/source`, nunca a pasta de trabalho de quem desenvolve), com poucos
//! núcleos e prioridade baixa, e instalada do mesmo jeito que foi instalada antes:
//! - pelo `./install.sh` (lista em `/var/lib/mide/installed-files` ou pacote do
//!   Slackware), com as escolhas guardadas em `/var/lib/mide/install-options`;
//! - pela cópia de desenvolvimento da tela de login (`/usr/local/lib/mide-login`, do
//!   `data/mide-login-install`).
//!
//! A senha de administrador é pedida pela janela do mide (`sudo -A` com o
//! `mide-askpass`). Quem tem o mide pelo apt (`Install::Package`) atualiza pelo
//! PackageKit, como qualquer pacote: nada daqui.

use std::{
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

/// O repositório do mide (`MIDE_UPDATE_REPO` troca, nos testes).
pub const REPO: &str = "https://github.com/fabioaacarneiro/mide";

pub fn repo() -> String {
    std::env::var("MIDE_UPDATE_REPO").ok().filter(|r| !r.is_empty()).unwrap_or_else(|| REPO.to_string())
}

/// Versão `X.Y.Z` (sem o "v" e sem sufixos como `+git1234567`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version(pub u32, pub u32, pub u32);

impl Version {
    pub fn parse(text: &str) -> Option<Self> {
        let core = text.trim().trim_start_matches('v');
        let core = core.split(['+', '-', '~']).next()?;
        let mut parts = core.split('.');
        let v = Version(parts.next()?.parse().ok()?, parts.next()?.parse().ok()?, parts.next()?.parse().ok()?);
        parts.next().is_none().then_some(v)
    }

    pub fn tag(&self) -> String {
        format!("v{self}")
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

/// A versão deste mide (a do programa que pergunta: todos vêm da mesma compilação).
/// `MIDE_UPDATE_CURRENT` troca, nos testes.
pub fn current() -> Version {
    std::env::var("MIDE_UPDATE_CURRENT")
        .ok()
        .and_then(|v| Version::parse(&v))
        .or_else(|| Version::parse(env!("CARGO_PKG_VERSION")))
        .unwrap_or(Version(0, 0, 0))
}

/// As escolhas da instalação pelo `./install.sh` (`/var/lib/mide/install-options`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Options {
    pub distro: String,
    pub prefix: String,
    pub x11: bool,
    pub icons: bool,
    pub power: bool,
    pub login: bool,
}

impl Options {
    pub fn parse(text: &str) -> Self {
        let get = |key: &str| text.lines().find_map(|l| l.trim().strip_prefix(key)?.strip_prefix('=')).map(|v| v.trim().to_string()).unwrap_or_default();
        Options { distro: get("DISTRO"), prefix: get("PREFIXO"), x11: get("X11") == "s", icons: get("ICONES") == "s", power: get("ENERGIA") == "s", login: get("LOGIN") == "s" }
    }

    /// Argumentos do `./install.sh` para repetir a instalação sem perguntas. Os temas do
    /// boot não são refeitos (o GRUB e o initramfs levam um tempo e quase nunca mudam).
    pub fn install_args(&self) -> Vec<String> {
        let mut args: Vec<String> = vec!["--install".into(), "--yes".into(), "--source".into(), "--no-boot-theme".into()];
        if !self.distro.is_empty() {
            args.extend(["--distro".into(), self.distro.clone()]);
        }
        if !self.prefix.is_empty() {
            args.extend(["--prefix".into(), self.prefix.clone()]);
        }
        let flag = |on: bool, yes: &str, no: &str| if on { yes.to_string() } else { no.to_string() };
        args.push(flag(self.x11, "--x11", "--no-x11"));
        args.push(flag(self.icons, "--icons", "--no-icons"));
        args.push(flag(self.power, "--power", "--no-power"));
        args.push(flag(self.login, "--login", "--no-login"));
        args
    }
}

/// Como o mide desta máquina foi instalado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Install {
    /// Pelo apt (o mios, ou o pacote pronto no Debian/Ubuntu): o PackageKit atualiza.
    Package,
    /// Pelo `./install.sh`, compilando (ou o pacote do Slackware).
    Source(Options),
    /// Cópia de desenvolvimento da tela de login (`data/mide-login-install`).
    Development,
    /// Rodando direto da pasta do projeto, ou não deu para saber: nada a fazer.
    Unknown,
}

impl Install {
    /// A atualização pelo código-fonte vale para esta instalação?
    pub fn from_source(&self) -> bool {
        matches!(self, Install::Source(_) | Install::Development)
    }
}

/// Como o mide foi instalado, olhando o sistema em `root` (`/`, ou outra pasta nos testes).
pub fn detect_in(root: &Path) -> Install {
    let status = std::fs::read_to_string(root.join("var/lib/dpkg/status")).unwrap_or_default();
    if dpkg_installed(&status, "mide") {
        return Install::Package;
    }
    let options = || Options::parse(&std::fs::read_to_string(root.join("var/lib/mide/install-options")).unwrap_or_default());
    if root.join("var/lib/mide/installed-files").is_file() || slackware_package(root) {
        return Install::Source(options());
    }
    if root.join("usr/local/lib/mide-login/mide").is_file() {
        return Install::Development;
    }
    Install::Unknown
}

pub fn detect() -> Install {
    detect_in(Path::new("/"))
}

/// O pacote está instalado, pelo `/var/lib/dpkg/status`?
fn dpkg_installed(status: &str, name: &str) -> bool {
    status.split("\n\n").any(|stanza| {
        let mut lines = stanza.lines();
        lines.clone().any(|l| l == format!("Package: {name}")) && lines.any(|l| l == "Status: install ok installed")
    })
}

fn slackware_package(root: &Path) -> bool {
    ["var/lib/pkgtools/packages", "var/log/packages"].iter().any(|d| {
        std::fs::read_dir(root.join(d)).into_iter().flatten().flatten().any(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            name.strip_prefix("mide-").is_some_and(|rest| rest.split('-').count() == 3 && rest.starts_with(|c: char| c.is_ascii_digit()))
        })
    })
}

/// Versões publicadas na saída do `git ls-remote --tags --refs` (só as `vX.Y.Z`).
pub fn parse_tags(output: &str) -> Vec<Version> {
    let mut list: Vec<Version> = output.lines().filter_map(|l| l.split_whitespace().nth(1)?.strip_prefix("refs/tags/v")).filter_map(Version::parse).collect();
    list.sort();
    list.dedup();
    list
}

/// A versão mais nova publicada (rede: `git ls-remote`, sem baixar o projeto).
pub fn latest(repo: &str) -> Result<Option<Version>, String> {
    let out = Command::new("git")
        .args(["ls-remote", "--tags", "--refs", repo, "v*"])
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(parse_tags(&String::from_utf8_lossy(&out.stdout)).pop())
}

/// Onde fica a cópia do projeto usada nas atualizações (`MIDE_UPDATE_DIR` troca).
pub fn source_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("MIDE_UPDATE_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(dir);
    }
    let cache = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".cache"));
    cache.join("mide/source")
}

fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    } else {
        Err(format!("git {}: {}", args.first().copied().unwrap_or(""), String::from_utf8_lossy(&out.stderr).trim()))
    }
}

/// Deixa a cópia do projeto em `dir` com as tags de `repo` (só o histórico: os arquivos
/// vêm na hora de trocar de versão, sem baixar todas as imagens de todas as versões).
pub fn fetch(dir: &Path, repo: &str) -> Result<(), String> {
    if !dir.join(".git").is_dir() {
        if let Some(parent) = dir.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let _ = std::fs::remove_dir_all(dir);
        let out = Command::new("git")
            .args(["clone", "--quiet", "--filter=blob:none", "--no-checkout", repo])
            .arg(dir)
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("git: {e}"))?;
        if !out.status.success() {
            return Err(format!("git clone: {}", String::from_utf8_lossy(&out.stderr).trim()));
        }
    }
    git(dir, &["remote", "set-url", "origin", repo])?;
    git(dir, &["fetch", "--quiet", "--tags", "--force", "--prune", "origin"])?;
    Ok(())
}

/// O que mudou de `from` até `to` (os títulos dos commits, do mais novo ao mais velho).
/// Sem a tag de `from` (instalação mais antiga que a cópia conhece), as últimas mudanças.
pub fn changes(dir: &Path, from: Version, to: Version) -> Vec<String> {
    let range = format!("{}..{}", from.tag(), to.tag());
    let text = git(dir, &["log", "--format=%s", "--no-merges", &range]).or_else(|_| git(dir, &["log", "--format=%s", "--no-merges", "-n", "15", &to.tag()])).unwrap_or_default();
    text.lines().filter(|l| !l.trim().is_empty()).map(|l| l.trim().to_string()).collect()
}

/// Troca a cópia do projeto para a versão `to`.
pub fn checkout(dir: &Path, to: Version) -> Result<(), String> {
    git(dir, &["-c", "advice.detachedHead=false", "checkout", "--quiet", "--force", &to.tag()])?;
    Ok(())
}

/// Em que passo a atualização está.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    Downloading,
    /// Compilando: pacotes prontos e o total (aproximado: os já compilados antes não contam).
    Building { done: usize, total: usize },
    /// Esperando a senha e instalando no sistema.
    Installing,
}

/// Quantos pacotes o cargo compila, pelo `Cargo.lock` (mais o próprio mide).
pub fn crate_count(lock: &str) -> usize {
    lock.lines().filter(|l| l.trim() == "[[package]]").count().max(1)
}

/// Núcleos para compilar: metade da máquina, no máximo 4 (o computador continua usável).
pub fn build_jobs() -> usize {
    let n = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2);
    (n / 2).clamp(1, 4)
}

/// O `mide-askpass` ao lado deste programa (o sudo exige o caminho absoluto).
fn askpass() -> Option<PathBuf> {
    let beside = std::env::current_exe().ok()?.parent()?.join("mide-askpass");
    if beside.is_file() {
        return Some(beside);
    }
    ["/usr/local/bin/mide-askpass", "/usr/bin/mide-askpass"].iter().map(PathBuf::from).find(|p| p.is_file())
}

/// Roda `cmd` mandando cada linha da saída (stdout e stderr juntos) a `line`; devolve o
/// fim da saída se falhar.
fn run_lines(mut cmd: Command, mut line: impl FnMut(&str)) -> Result<(), String> {
    // stderr na mesma saída: o cargo escreve o "Compiling" lá.
    let (reader, writer) = std::io::pipe().map_err(|e| e.to_string())?;
    let writer2 = writer.try_clone().map_err(|e| e.to_string())?;
    cmd.stdin(Stdio::null()).stdout(writer).stderr(writer2);
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    drop(cmd);
    let mut tail: Vec<String> = Vec::new();
    for l in BufReader::new(reader).lines().map_while(Result::ok) {
        line(&l);
        tail.push(l);
        if tail.len() > 12 {
            tail.remove(0);
        }
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(tail.join("\n"))
    }
}

/// Atualiza o mide para `to`: baixa, compila e instala, contando os passos a `progress`.
/// Não reinicia nada: o mide novo vale ao entrar de novo na sessão (ou ao reiniciá-la).
pub fn update(install: &Install, to: Version, mut progress: impl FnMut(Step)) -> Result<(), String> {
    if !install.from_source() {
        return Err("this mide is not updated from source".into());
    }
    let dir = source_dir();
    progress(Step::Downloading);
    fetch(&dir, &repo())?;
    checkout(&dir, to)?;
    let total = crate_count(&std::fs::read_to_string(dir.join("Cargo.lock")).unwrap_or_default());
    let mut done = 0usize;
    progress(Step::Building { done, total });
    let jobs = build_jobs().to_string();
    let askpass = askpass().ok_or("mide-askpass not found")?;
    let elevate = |cmd: &mut Command| {
        cmd.env("SUDO_ASKPASS", &askpass).env(crate::elevate::TITLE_VAR, "mide").env(crate::elevate::COMMAND_VAR, format!("mide {to}")).env(crate::elevate::ICON_VAR, "system-software-update");
    };
    let mut on_line = |l: &str, progress: &mut dyn FnMut(Step)| {
        let t = l.trim_start();
        if t.starts_with("Compiling ") {
            done += 1;
            progress(Step::Building { done: done.min(total), total });
        } else if t.contains("==> Copying to") || t.contains("==> Copiando para") || t.starts_with("==> removepkg") || t.contains("Slackware package") || t.contains("Pacote do Slackware") {
            progress(Step::Installing);
        }
    };
    match install {
        Install::Source(options) => {
            // O próprio ./install.sh da versão nova, com as mesmas escolhas.
            let mut cmd = Command::new("bash");
            cmd.arg(dir.join("install.sh")).args(options.install_args()).current_dir(&dir).env("CARGO_BUILD_JOBS", &jobs).env("LANG", "C.UTF-8");
            elevate(&mut cmd);
            run_lines(cmd, |l| on_line(l, &mut progress))?;
        }
        Install::Development => {
            // Só os intermediários do próprio mide saem antes (o cargo guarda uma cópia por
            // versão); as dependências já compiladas ficam.
            let _ = Command::new("cargo").args(["clean", "--release", "-p", "mide", "-p", "mide-ui"]).current_dir(&dir).stdout(Stdio::null()).stderr(Stdio::null()).status();
            let mut cmd = Command::new("nice");
            cmd.args(["-n", "19", "cargo", "build", "--release", "--locked", "-j", &jobs]).current_dir(&dir);
            run_lines(cmd, |l| on_line(l, &mut progress))?;
            progress(Step::Installing);
            let mut cmd = Command::new("sudo");
            cmd.arg("-A").arg("bash").arg(dir.join("data/mide-login-install")).current_dir(&dir);
            elevate(&mut cmd);
            run_lines(cmd, |_| {})?;
        }
        Install::Package | Install::Unknown => unreachable!(),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versoes() {
        assert_eq!(Version::parse("0.10.105"), Some(Version(0, 10, 105)));
        assert_eq!(Version::parse("v1.2.3"), Some(Version(1, 2, 3)));
        assert_eq!(Version::parse("0.10.103+git1a2b3c4"), Some(Version(0, 10, 103)));
        assert_eq!(Version::parse("1.0"), None);
        assert_eq!(Version::parse("1.2.3.4"), None);
        assert!(Version(0, 10, 105) > Version(0, 10, 99));
        assert!(Version(0, 11, 0) > Version(0, 10, 200));
        assert_eq!(Version(0, 10, 5).tag(), "v0.10.5");
    }

    #[test]
    fn tags_do_ls_remote() {
        let out = "abc\trefs/tags/v0.10.9\nabd\trefs/tags/v0.10.104\nabe\trefs/tags/mios-1.0\nabf\trefs/tags/iso\nabg\trefs/tags/v0.10.10\n";
        let tags = parse_tags(out);
        assert_eq!(tags.last(), Some(&Version(0, 10, 104)));
        assert_eq!(tags.len(), 3);
    }

    #[test]
    fn opcoes_da_instalacao() {
        let o = Options::parse("DISTRO=fedora\nPREFIXO=/usr/local\nX11=s\nICONES=n\nENERGIA=s\nLOGIN=s\n");
        assert_eq!(o.distro, "fedora");
        assert!(o.x11 && !o.icons && o.power && o.login);
        let args = o.install_args().join(" ");
        assert_eq!(args, "--install --yes --source --no-boot-theme --distro fedora --prefix /usr/local --x11 --no-icons --power --login");
    }

    #[test]
    fn tipo_de_instalacao() {
        let root = std::env::temp_dir().join(format!("mide-selfupdate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(detect_in(&root), Install::Unknown);
        std::fs::create_dir_all(root.join("usr/local/lib/mide-login")).unwrap();
        std::fs::write(root.join("usr/local/lib/mide-login/mide"), "").unwrap();
        assert_eq!(detect_in(&root), Install::Development);
        std::fs::create_dir_all(root.join("var/lib/mide")).unwrap();
        std::fs::write(root.join("var/lib/mide/installed-files"), "/usr/local/bin/mide\n").unwrap();
        std::fs::write(root.join("var/lib/mide/install-options"), "DISTRO=arch\nLOGIN=s\n").unwrap();
        match detect_in(&root) {
            Install::Source(o) => assert!(o.distro == "arch" && o.login),
            other => panic!("{other:?}"),
        }
        std::fs::create_dir_all(root.join("var/lib/dpkg")).unwrap();
        std::fs::write(root.join("var/lib/dpkg/status"), "Package: mide-x\nStatus: install ok installed\n\nPackage: mide\nStatus: deinstall ok config-files\n").unwrap();
        assert!(matches!(detect_in(&root), Install::Source(_)), "pacote removido não conta");
        std::fs::write(root.join("var/lib/dpkg/status"), "Package: libc6\nStatus: install ok installed\n\nPackage: mide\nVersion: 1\nStatus: install ok installed\n").unwrap();
        assert_eq!(detect_in(&root), Install::Package);
        std::fs::remove_dir_all(&root).unwrap();
        // Pacote do Slackware.
        std::fs::create_dir_all(root.join("var/lib/pkgtools/packages")).unwrap();
        std::fs::write(root.join("var/lib/pkgtools/packages/mide-0.10.105-x86_64-1_mide"), "").unwrap();
        assert!(matches!(detect_in(&root), Install::Source(_)));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn contagem_de_pacotes() {
        assert_eq!(crate_count("[[package]]\nname = \"a\"\n\n[[package]]\nname = \"b\"\n"), 2);
        assert_eq!(crate_count(""), 1);
        assert!((1..=4).contains(&build_jobs()));
    }

    /// Repositório de verdade (local): a versão mais nova, o que mudou e a troca de versão.
    #[test]
    fn repositorio_com_tags() {
        if Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let base = std::env::temp_dir().join(format!("mide-selfupdate-git-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let origem = base.join("origem");
        std::fs::create_dir_all(&origem).unwrap();
        let g = |args: &[&str]| {
            let ok = Command::new("git").arg("-C").arg(&origem).args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"]).args(args).output().unwrap();
            assert!(ok.status.success(), "{args:?}: {}", String::from_utf8_lossy(&ok.stderr));
        };
        g(&["init", "-q"]);
        for (i, versao) in ["0.1.0", "0.1.1", "0.1.2"].iter().enumerate() {
            std::fs::write(origem.join("VERSION"), versao).unwrap();
            g(&["add", "."]);
            g(&["commit", "-q", "-m", &format!("mudança {i}")]);
            g(&["tag", &format!("v{versao}")]);
        }
        g(&["tag", "mios-1.0"]);
        let repo = format!("file://{}", origem.display());
        assert_eq!(latest(&repo).unwrap(), Some(Version(0, 1, 2)));
        let copia = base.join("copia");
        fetch(&copia, &repo).unwrap();
        assert_eq!(changes(&copia, Version(0, 1, 0), Version(0, 1, 2)), vec!["mudança 2", "mudança 1"]);
        // Versão instalada que a cópia não conhece: as últimas mudanças.
        assert_eq!(changes(&copia, Version(0, 0, 9), Version(0, 1, 1)), vec!["mudança 1", "mudança 0"]);
        checkout(&copia, Version(0, 1, 1)).unwrap();
        assert_eq!(std::fs::read_to_string(copia.join("VERSION")).unwrap(), "0.1.1");
        // Buscar de novo (a cópia já existe) traz a tag nova.
        std::fs::write(origem.join("VERSION"), "0.2.0").unwrap();
        g(&["commit", "-q", "-am", "versão nova"]);
        g(&["tag", "v0.2.0"]);
        fetch(&copia, &repo).unwrap();
        checkout(&copia, Version(0, 2, 0)).unwrap();
        assert_eq!(std::fs::read_to_string(copia.join("VERSION")).unwrap(), "0.2.0");
        std::fs::remove_dir_all(&base).unwrap();
    }
}
