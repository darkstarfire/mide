//! Senha de administrador gráfica (C7): as regras, sem interface, do `mide-sudo` (pede a
//! senha numa janela para o `sudo -A`), do `mide-askpass` (a janela) e do
//! `mide-polkit-agent` (o agente de autenticação do polkit da sessão).
//!
//! Quem confere a senha é sempre o sudo/PAM ou o polkit; o mide só leva o que a pessoa
//! digitou, por pipes, até eles. A senha nunca vai para argumento de programa (aparece
//! em `/proc/*/cmdline`), variável de ambiente, log ou arquivo.
//!
//! **App gráfico como root:** o programa roda pelo X11 (o XWayland do mide), com só
//! `DISPLAY`/`XAUTHORITY` e o idioma. Ficam de fora `WAYLAND_DISPLAY`, `XDG_RUNTIME_DIR`
//! e `DBUS_SESSION_BUS_ADDRESS`: um app root com a pasta de execução do usuário criaria
//! arquivos de root nela (dconf, pulse) e teria a sessão D-Bus do usuário nas mãos.

use std::{
    ffi::{OsStr, OsString},
    io::{Read, Write},
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
};

/// O `sudo` de verdade (caminho fixo: nada de procurar no PATH).
pub const SUDO: &str = "/usr/bin/sudo";
/// O `env` que põe as variáveis escolhidas no programa que roda como root.
pub const ENV: &str = "/usr/bin/env";

/// Variáveis de ambiente para a janela de senha (só para mostrar: não são confiáveis,
/// e não precisam ser; quem confere a senha é o sudo).
pub const TITLE_VAR: &str = "MIDE_ELEVATE_TITLE";
pub const ICON_VAR: &str = "MIDE_ELEVATE_ICON";
pub const COMMAND_VAR: &str = "MIDE_ELEVATE_COMMAND";
/// Arquivo (0600, na pasta de execução) onde a janela conta as tentativas do sudo e
/// marca o cancelamento.
pub const STATE_VAR: &str = "MIDE_ASKPASS_STATE";

/// Tentativas de senha (as do `passwd_tries` padrão do sudo e as do agente do polkit).
pub const MAX_TRIES: u32 = 3;

/// Como o `mide-sudo` passa o ambiente ao programa.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvMode {
    /// `sudo -- /usr/bin/env K=V... programa`: funciona com o `env_reset` padrão e com a
    /// regra `%sudo ALL=(ALL:ALL) ALL`, sem `SETENV`.
    Env,
    /// `sudo --preserve-env=K,... -- programa`: segunda tentativa, quando a regra do
    /// sudoers só deixa rodar o próprio programa (recusa o `/usr/bin/env`).
    PreserveEnv,
}

/// Variáveis que o programa root recebe, a partir do ambiente de quem pediu: só a
/// tela X11 e o idioma, e os apps forçados ao X11. `xauthority` é a reserva quando
/// `XAUTHORITY` não está definida (o `~/.Xauthority` do usuário, se existir: o root tem
/// outra pasta pessoal e não o acharia).
pub fn root_env(env: &[(OsString, OsString)], xauthority: Option<&Path>) -> Vec<(OsString, OsString)> {
    let get = |name: &str| env.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone()).filter(|v| !v.is_empty());
    let mut out: Vec<(OsString, OsString)> = Vec::new();
    // Só tela local (`:N` ou `:N.M`) e Xauthority com caminho absoluto: o resto não
    // passa (o root leria o que o valor apontar).
    if let Some(display) = get("DISPLAY").filter(|d| x11_display_number(d).is_some()) {
        out.push(("DISPLAY".into(), display));
        let auth = get("XAUTHORITY").or_else(|| xauthority.map(|p| p.as_os_str().to_owned()));
        if let Some(auth) = auth.filter(|a| a.as_bytes().starts_with(b"/")) {
            out.push(("XAUTHORITY".into(), auth));
        }
    }
    // Apps GTK e Qt pelo X11: o root não tem acesso ao Wayland do usuário.
    out.push(("GDK_BACKEND".into(), "x11".into()));
    out.push(("QT_QPA_PLATFORM".into(), "xcb".into()));
    // Sem o barramento de acessibilidade do usuário (o root não alcança).
    out.push(("NO_AT_BRIDGE".into(), "1".into()));
    let mut lang: Vec<(OsString, OsString)> = env
        .iter()
        .filter(|(k, v)| {
            let k = k.as_bytes();
            (k == b"LANG" || k == b"LANGUAGE" || (k.starts_with(b"LC_") && k.len() > 3)) && safe_locale_value(v)
        })
        .cloned()
        .collect();
    lang.sort();
    out.extend(lang);
    out
}

/// Valor de idioma que pode ir para o root: o mesmo filtro do `env_check` do sudo
/// (nada de `/`, `%` nem `..`). Com `LC_ALL=/home/u/x`, a glibc do programa root
/// carregaria arquivos de locale de uma pasta do usuário.
fn safe_locale_value(v: &OsStr) -> bool {
    let b = v.as_bytes();
    !b.is_empty() && !b.contains(&b'/') && !b.contains(&b'%') && !b.windows(2).any(|w| w == b"..")
}

/// Número da tela X11 local (`:N` ou `:N.M`); outra forma (host remoto, caminho de
/// socket) dá `None`.
pub fn x11_display_number(display: &OsStr) -> Option<u32> {
    let rest = display.to_str()?.strip_prefix(':')?;
    let (num, screen) = match rest.split_once('.') {
        Some((n, s)) => (n, Some(s)),
        None => (rest, None),
    };
    let digits = |t: &str| !t.is_empty() && t.len() <= 6 && t.bytes().all(|c| c.is_ascii_digit());
    if !digits(num) || screen.is_some_and(|s| !digits(s)) {
        return None;
    }
    num.parse().ok()
}

/// Linha de comando do sudo (o primeiro item é o `sudo`). `askpass` acrescenta o `-A`
/// (a senha vem da janela, sem terminal). No modo `Env`, `LANGUAGE` sai do programa
/// quando o usuário não tinha (o `mide-sudo` põe `LANGUAGE=en` no sudo para entender
/// as mensagens dele) e o programa é procurado pelo `env` no PATH seguro do sudo.
pub fn sudo_command(sudo: &OsStr, program: &OsStr, args: &[OsString], vars: &[(OsString, OsString)], mode: EnvMode, askpass: bool) -> Vec<OsString> {
    let mut cmd: Vec<OsString> = vec![sudo.to_owned()];
    if askpass {
        cmd.push("-A".into());
    }
    match mode {
        EnvMode::Env => {
            cmd.push("--".into());
            cmd.push(ENV.into());
            if !vars.iter().any(|(k, _)| k == "LANGUAGE") {
                cmd.push("-u".into());
                cmd.push("LANGUAGE".into());
            }
            for (k, v) in vars {
                let mut kv = k.clone();
                kv.push("=");
                kv.push(v);
                cmd.push(kv);
            }
        }
        EnvMode::PreserveEnv => {
            let names: Vec<String> = vars.iter().map(|(k, _)| k.to_string_lossy().into_owned()).collect();
            cmd.push(format!("--preserve-env={}", names.join(",")).into());
            cmd.push("--".into());
        }
    }
    cmd.push(program.to_owned());
    cmd.extend(args.iter().cloned());
    cmd
}

/// Confere o programa pedido: vazio ou com `=` no nome (o `env` leria como variável)
/// é recusado. Caminho relativo com `/` vira absoluto (a pasta atual de quem pediu).
pub fn resolve_program(program: &OsStr, cwd: &Path) -> Result<OsString, String> {
    let bytes = program.as_bytes();
    if bytes.is_empty() {
        return Err("programa vazio".into());
    }
    if bytes.contains(&b'=') {
        return Err(format!("nome de programa com \"=\" não é aceito: {}", program.to_string_lossy()));
    }
    if bytes.starts_with(b"-") {
        return Err(format!("opção desconhecida: {}", program.to_string_lossy()));
    }
    if bytes.contains(&b'/') && !bytes.starts_with(b"/") {
        return Ok(cwd.join(program).into_os_string());
    }
    Ok(program.to_owned())
}

/// O comando como texto, para mostrar na janela (cortado se for longo).
pub fn command_text(program: &OsStr, args: &[OsString]) -> String {
    let mut parts = vec![quote(&program.to_string_lossy())];
    parts.extend(args.iter().map(|a| quote(&a.to_string_lossy())));
    let text = parts.join(" ");
    if text.chars().count() > 240 {
        let cut: String = text.chars().take(239).collect();
        format!("{cut}…")
    } else {
        text
    }
}

fn quote(s: &str) -> String {
    if !s.is_empty() && s.chars().all(|c| c.is_alphanumeric() || "-_./:=+,@%".contains(c)) {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

/// Por que o sudo falhou, pelas mensagens dele (em inglês: o `mide-sudo` roda o sudo
/// com `LANGUAGE=en`; as em português ficam como reserva).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SudoFailure {
    /// Errou a senha todas as vezes.
    WrongPassword,
    /// O usuário não pode usar o sudo.
    NotAllowed,
    /// O sudoers não deixa rodar o `/usr/bin/env` (regra só para o programa).
    EnvRefused,
    /// Nenhuma senha (a janela não abriu ou foi fechada).
    NoPassword,
}

pub fn classify_sudo_error(stderr: &str) -> Option<SudoFailure> {
    let low = stderr.to_lowercase();
    if low.contains("not allowed to execute") && low.contains(ENV) {
        Some(SudoFailure::EnvRefused)
    } else if low.contains("incorrect password attempt") || low.contains("tentativas de senha incorretas") || low.contains("tentativas incorretas de senha") {
        Some(SudoFailure::WrongPassword)
    } else if low.contains("not in the sudoers") || low.contains("not allowed to run sudo") || low.contains("not allowed to execute") || low.contains("não está no arquivo sudoers") {
        Some(SudoFailure::NotAllowed)
    } else if low.contains("no password was provided") || low.contains("a password is required") || low.contains("no askpass program") {
        Some(SudoFailure::NoPassword)
    } else {
        None
    }
}

/// Estado da janela de senha guardado entre as chamadas do sudo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AskState {
    pub attempts: u32,
    pub cancelled: bool,
}

pub fn parse_state(text: &str) -> AskState {
    let mut words = text.split_whitespace();
    let attempts = words.next().and_then(|w| w.parse().ok()).unwrap_or(0);
    AskState { attempts, cancelled: text.split_whitespace().any(|w| w == "cancel") }
}

pub fn format_state(state: AskState) -> String {
    if state.cancelled { format!("{} cancel\n", state.attempts) } else { format!("{}\n", state.attempts) }
}

/// Cria o arquivo de estado (0600, só se ainda não existe).
pub fn create_state(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(path)?;
    file.write_all(format_state(AskState::default()).as_bytes())
}

pub fn read_state(path: &Path) -> AskState {
    let mut text = String::new();
    if let Ok(f) = std::fs::File::open(path) {
        let _ = f.take(64).read_to_string(&mut text);
    }
    parse_state(&text)
}

/// Grava o estado (o arquivo já existe: criado pelo `mide-sudo` com 0600).
pub fn write_state(path: &Path, state: AskState) -> std::io::Result<()> {
    let mut file = std::fs::OpenOptions::new().write(true).truncate(true).open(path)?;
    file.write_all(format_state(state).as_bytes())
}

/// Mais uma chamada da janela: devolve o número desta tentativa (1 na primeira).
pub fn bump_attempt(path: &Path) -> std::io::Result<u32> {
    let mut state = read_state(path);
    state.attempts += 1;
    state.cancelled = false;
    write_state(path, state)?;
    Ok(state.attempts)
}

pub fn mark_cancelled(path: &Path) -> std::io::Result<()> {
    let mut state = read_state(path);
    state.cancelled = true;
    write_state(path, state)
}

/// O que fazer com a tela X11 ao abrir um programa root (`X11Lock::join`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum X11Grant {
    /// Ainda não liberada: liberar (`xhost +si:localuser:root`); a marca já está feita.
    Add,
    /// Já liberada por outro `mide-sudo` (aberto, ou morto sem desfazer): só a marca.
    Join,
    /// Liberada por outra pessoa ou programa: não mexer (nem desfazer no fim).
    Skip,
}

/// Liberação da tela X11 para o root dividida entre os `mide-sudo` abertos ao mesmo
/// tempo: cada um deixa uma marca (`mide-sudo-x11-<tela>-<pid>`, na pasta de execução)
/// e só o último a sair desfaz. Marca de um `mide-sudo` que morreu sem desfazer (SIGKILL,
/// sessão caindo) é recolhida pelo próximo, que fica com a liberação e a desfaz no fim.
/// Tudo sob um `flock` (`mide-sudo-x11.lock`), solto quando o `X11Lock` sai de cena.
pub struct X11Lock {
    _file: std::fs::File,
    dir: PathBuf,
    display: u32,
}

impl X11Lock {
    pub fn acquire(dir: &Path, display: u32) -> std::io::Result<Self> {
        use std::os::unix::fs::OpenOptionsExt;
        let file = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).mode(0o600).open(dir.join("mide-sudo-x11.lock"))?;
        rustix::fs::flock(&file, rustix::fs::FlockOperation::LockExclusive)?;
        Ok(Self { _file: file, dir: dir.to_owned(), display })
    }

    fn prefix(&self) -> String {
        format!("mide-sudo-x11-{}-", self.display)
    }

    fn mark(&self, pid: u32) -> PathBuf {
        self.dir.join(format!("{}{pid}", self.prefix()))
    }

    /// Marcas dos outros: quantas estão vivas e se havia alguma morta (apagada aqui).
    fn others(&self, me: u32, alive: &dyn Fn(u32) -> bool) -> (usize, bool) {
        let prefix = self.prefix();
        let (mut live, mut stale) = (0, false);
        if let Ok(entries) = std::fs::read_dir(&self.dir) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let Some(pid) = name.to_str().and_then(|n| n.strip_prefix(prefix.as_str())).and_then(|p| p.parse::<u32>().ok()) else {
                    continue;
                };
                if pid == me {
                    continue;
                }
                if alive(pid) {
                    live += 1;
                } else {
                    stale = true;
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
        (live, stale)
    }

    /// Entra na liberação. `root_allowed`: o `xhost` já lista o root.
    pub fn join(&self, me: u32, root_allowed: bool, alive: &dyn Fn(u32) -> bool) -> std::io::Result<X11Grant> {
        let (live, stale) = self.others(me, alive);
        let grant = match (root_allowed, live > 0 || stale) {
            (true, false) => return Ok(X11Grant::Skip),
            (true, true) => X11Grant::Join,
            (false, _) => X11Grant::Add,
        };
        std::fs::write(self.mark(me), b"")?;
        Ok(grant)
    }

    /// Sai da liberação: `true` quando é o último (desfazer com `xhost -si:localuser:root`).
    pub fn leave(&self, me: u32, alive: &dyn Fn(u32) -> bool) -> bool {
        let _ = std::fs::remove_file(self.mark(me));
        self.others(me, alive).0 == 0
    }
}

/// Linha do `polkit-agent-helper-1` (o programa setuid do polkit que conversa com o
/// PAM). O texto depois do prefixo vem com escapes de C (`\n`, `\\`, `\NNN`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HelperLine {
    /// Pergunta sem mostrar o que se digita (a senha).
    PromptEchoOff(String),
    /// Pergunta mostrando o que se digita (raro: usuário, código).
    PromptEchoOn(String),
    Error(String),
    Info(String),
    Success,
    Failure,
    Other(String),
}

pub fn parse_helper_line(line: &str) -> HelperLine {
    let line = line.trim_end_matches(['\n', '\r']);
    let rest = |prefix: &str| line.strip_prefix(prefix).map(|r| unescape_c(r.strip_prefix(' ').unwrap_or(r)));
    if let Some(t) = rest("PAM_PROMPT_ECHO_OFF") {
        HelperLine::PromptEchoOff(t)
    } else if let Some(t) = rest("PAM_PROMPT_ECHO_ON") {
        HelperLine::PromptEchoOn(t)
    } else if let Some(t) = rest("PAM_ERROR_MSG") {
        HelperLine::Error(t)
    } else if let Some(t) = rest("PAM_TEXT_INFO") {
        HelperLine::Info(t)
    } else if line == "SUCCESS" {
        HelperLine::Success
    } else if line == "FAILURE" {
        HelperLine::Failure
    } else {
        HelperLine::Other(line.to_string())
    }
}

/// Desfaz os escapes de C (`g_strescape`): `\n`, `\t`, `\\`, `\"` e octais `\NNN`.
fn unescape_c(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'\\' || i + 1 >= bytes.len() {
            out.push(bytes[i]);
            i += 1;
            continue;
        }
        let c = bytes[i + 1];
        i += 2;
        match c {
            b'n' => out.push(b'\n'),
            b't' => out.push(b'\t'),
            b'r' => out.push(b'\r'),
            b'b' => out.push(8),
            b'f' => out.push(12),
            b'v' => out.push(11),
            b'0'..=b'7' => {
                let mut v = (c - b'0') as u32;
                let mut n = 1;
                while n < 3 && i < bytes.len() && (b'0'..=b'7').contains(&bytes[i]) {
                    v = v * 8 + (bytes[i] - b'0') as u32;
                    i += 1;
                    n += 1;
                }
                out.push(v as u8);
            }
            other => out.push(other),
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Uma identidade que o polkit aceita para autorizar (`a(sa{sv})` do `BeginAuthentication`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Identity {
    User(u32),
    Group(u32),
}

/// Usuários de um grupo: os listados no `/etc/group` e os que têm o grupo como principal.
fn group_members(gid: u32, passwd: &str, group: &str) -> Vec<String> {
    let mut members: Vec<String> = Vec::new();
    for line in group.lines() {
        let f: Vec<&str> = line.split(':').collect();
        if f.len() >= 4 && f[2].parse::<u32>().ok() == Some(gid) {
            members.extend(f[3].split(',').map(str::trim).filter(|m| !m.is_empty()).map(String::from));
        }
    }
    for line in passwd.lines() {
        let f: Vec<&str> = line.split(':').collect();
        if f.len() >= 4 && f[3].parse::<u32>().ok() == Some(gid) && !members.iter().any(|m| m == f[0]) {
            members.push(f[0].to_string());
        }
    }
    members
}

/// Nome de login do uid no `/etc/passwd`.
pub fn user_name(uid: u32, passwd: &str) -> Option<String> {
    passwd.lines().find_map(|line| {
        let f: Vec<&str> = line.split(':').collect();
        (f.len() >= 3 && f[2].parse::<u32>().ok() == Some(uid)).then(|| f[0].to_string())
    })
}

fn uid_of(name: &str, passwd: &str) -> Option<u32> {
    passwd.lines().find_map(|line| {
        let f: Vec<&str> = line.split(':').collect();
        (f.len() >= 3 && f[0] == name).then(|| f[2].parse().ok()).flatten()
    })
}

/// Com que usuário autenticar: o próprio usuário, se ele estiver entre as identidades
/// (direto ou num grupo, como o `sudo` do Debian); senão o primeiro usuário comum
/// listado; o root por último.
pub fn choose_identity(ids: &[Identity], me: u32, passwd: &str, group: &str) -> Option<String> {
    let mut candidates: Vec<u32> = Vec::new();
    for id in ids {
        match *id {
            Identity::User(uid) => candidates.push(uid),
            Identity::Group(gid) => candidates.extend(group_members(gid, passwd, group).iter().filter_map(|m| uid_of(m, passwd))),
        }
    }
    if candidates.contains(&me) {
        return user_name(me, passwd);
    }
    candidates
        .iter()
        .find(|&&uid| uid != 0)
        .or_else(|| candidates.iter().find(|&&uid| uid == 0))
        .and_then(|&uid| user_name(uid, passwd))
}

/// Onde as distribuições põem o `polkit-agent-helper-1`.
pub const HELPER_PATHS: [&str; 4] = [
    "/usr/lib/polkit-1/polkit-agent-helper-1",
    "/usr/libexec/polkit-agent-helper-1",
    "/usr/lib/policykit-1/polkit-agent-helper-1",
    "/usr/libexec/polkit-1/polkit-agent-helper-1",
];

/// O helper do polkit instalado: dono root e com o bit setuid (um arquivo qualquer no
/// lugar dele não recebe a senha).
pub fn find_helper() -> Option<PathBuf> {
    use std::os::unix::fs::MetadataExt;
    HELPER_PATHS.iter().map(PathBuf::from).find(|p| {
        std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.uid() == 0 && m.mode() & 0o4000 != 0)
    })
}

/// Um helper instalado sem o bit setuid: polkit novo em que o helper roda pelo socket
/// do systemd (`polkit-agent-helper.socket`), ainda não suportado pelo agente do mide
/// (para o aviso dizer o motivo certo).
pub fn helper_without_setuid() -> Option<PathBuf> {
    use std::os::unix::fs::MetadataExt;
    HELPER_PATHS.iter().map(PathBuf::from).find(|p| std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.mode() & 0o4000 == 0))
}

/// Mensagens do agente do polkit para a janela (`mide-askpass --agent`, pela entrada
/// padrão dela), uma por linha.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToWindow {
    /// Quem pede (o texto do polkit: "Autenticação é necessária para...").
    Message(String),
    /// O comando (do `pkexec`) ou a ação.
    Command(String),
    /// Usuário cuja senha é pedida.
    User(String),
    /// Pode digitar: o texto é a pergunta do PAM ("Senha:").
    Ask(String),
    /// Erro (senha incorreta): o campo balança e o texto fica em vermelho.
    Error(String),
    /// Aviso do PAM.
    Info(String),
    /// Conferindo (o campo fica esperando).
    Busy,
    Close,
}

/// Respostas da janela para o agente (pela saída padrão dela).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FromWindow {
    Pass(String),
    Cancel,
}

/// Escapa `\`, quebras de linha e o retorno (o protocolo é de uma linha por mensagem).
pub fn escape_field(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    escape_into(&mut out, s);
    out
}

fn escape_into(out: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
}

/// O contrário de `escape_field`. O resultado nunca é maior que a entrada: com a
/// capacidade reservada no começo, a `String` não cresce (crescer deixaria cópias da
/// senha na memória solta).
pub fn unescape_field(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

impl ToWindow {
    pub fn to_line(&self) -> String {
        let (word, text) = match self {
            ToWindow::Message(t) => ("message", t.as_str()),
            ToWindow::Command(t) => ("command", t.as_str()),
            ToWindow::User(t) => ("user", t.as_str()),
            ToWindow::Ask(t) => ("ask", t.as_str()),
            ToWindow::Error(t) => ("error", t.as_str()),
            ToWindow::Info(t) => ("info", t.as_str()),
            ToWindow::Busy => return "busy\n".into(),
            ToWindow::Close => return "close\n".into(),
        };
        format!("{word} {}\n", escape_field(text))
    }

    pub fn parse(line: &str) -> Option<Self> {
        let line = line.trim_end_matches(['\n', '\r']);
        let (word, rest) = line.split_once(' ').unwrap_or((line, ""));
        let text = unescape_field(rest);
        Some(match word {
            "message" => ToWindow::Message(text),
            "command" => ToWindow::Command(text),
            "user" => ToWindow::User(text),
            "ask" => ToWindow::Ask(text),
            "error" => ToWindow::Error(text),
            "info" => ToWindow::Info(text),
            "busy" => ToWindow::Busy,
            "close" => ToWindow::Close,
            _ => return None,
        })
    }
}

impl FromWindow {
    /// Linha para o agente. Senha com quebra de linha não tem como seguir pelo
    /// protocolo (nem pelo helper do polkit): `None`.
    pub fn to_line(&self) -> Option<String> {
        match self {
            FromWindow::Pass(p) if p.contains(['\n', '\r']) => None,
            FromWindow::Pass(p) => {
                // Capacidade do pior caso (tudo escapado): a linha não cresce e não deixa
                // cópias da senha pelo caminho. Quem chama apaga a linha com `wipe`.
                let mut line = String::with_capacity(5 + 2 * p.len() + 1);
                line.push_str("pass ");
                escape_into(&mut line, p);
                line.push('\n');
                Some(line)
            }
            FromWindow::Cancel => Some("cancel\n".into()),
        }
    }

    pub fn parse(line: &str) -> Option<Self> {
        let line = line.strip_suffix('\n').unwrap_or(line);
        if line == "cancel" {
            return Some(FromWindow::Cancel);
        }
        line.strip_prefix("pass ").map(|p| FromWindow::Pass(unescape_field(p)))
    }
}

/// Apaga a senha da memória antes de soltar a `String` (o melhor possível: cópias
/// feitas pelas bibliotecas no caminho não são alcançadas).
pub fn wipe(s: &mut String) {
    // SAFETY: só zeros (UTF-8 válido) são escritos, e a String fica vazia em seguida.
    let bytes = unsafe { s.as_mut_vec() };
    for b in bytes.iter_mut() {
        // Escrita volátil: o compilador não pode tirar a limpeza.
        unsafe { std::ptr::write_volatile(b, 0) };
    }
    bytes.clear();
}

/// Apaga bytes (senha) antes de soltar o `Vec`.
pub fn wipe_bytes(v: &mut Vec<u8>) {
    for b in v.iter_mut() {
        // SAFETY: ponteiro válido para um byte do próprio Vec.
        unsafe { std::ptr::write_volatile(b, 0) };
    }
    v.clear();
}

/// Tamanho máximo de uma linha do protocolo da janela (a senha vem nela).
pub const MAX_LINE: usize = 8192;

/// Lê uma linha (sem o `\n`) byte a byte, sem buffer intermediário, para a linha que
/// traz a senha ficar só em `line` (que quem chama apaga com `wipe_bytes`). Linha maior
/// que `MAX_LINE` é descartada inteira. `false` no fim da entrada.
pub fn read_secret_line(r: &mut impl Read, line: &mut Vec<u8>) -> std::io::Result<bool> {
    wipe_bytes(line);
    line.reserve(MAX_LINE);
    let mut too_long = false;
    let mut byte = [0u8; 1];
    loop {
        match r.read(&mut byte) {
            Ok(0) => {
                if too_long {
                    wipe_bytes(line);
                }
                return Ok(!line.is_empty());
            }
            Ok(_) => {
                if byte[0] == b'\n' {
                    if too_long {
                        too_long = false;
                        wipe_bytes(line);
                        continue;
                    }
                    return Ok(true);
                }
                if line.len() < MAX_LINE {
                    line.push(byte[0]);
                } else {
                    too_long = true;
                }
                // SAFETY: o próprio byte do buffer local (pode ser da senha).
                unsafe { std::ptr::write_volatile(&mut byte[0], 0) };
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => {
                wipe_bytes(line);
                return Err(e);
            }
        }
    }
}

/// Escreve direto no descritor (sem o buffer do `stdout` da biblioteca padrão, que
/// guardaria uma cópia da senha).
pub fn write_all_fd(fd: std::os::fd::BorrowedFd<'_>, mut bytes: &[u8]) -> std::io::Result<()> {
    while !bytes.is_empty() {
        match rustix::io::write(fd, bytes) {
            Ok(0) => return Err(std::io::ErrorKind::WriteZero.into()),
            Ok(n) => bytes = &bytes[n..],
            Err(rustix::io::Errno::INTR) => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

/// Nome de login de quem está usando (do `/etc/passwd`, ou `USER`).
pub fn current_user() -> String {
    let uid = rustix::process::getuid().as_raw();
    std::fs::read_to_string("/etc/passwd")
        .ok()
        .and_then(|p| user_name(uid, &p))
        .or_else(|| std::env::var("USER").ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
        pairs.iter().map(|(k, v)| (OsString::from(k), OsString::from(v))).collect()
    }

    fn names(vars: &[(OsString, OsString)]) -> Vec<String> {
        vars.iter().map(|(k, _)| k.to_string_lossy().into_owned()).collect()
    }

    #[test]
    fn ambiente_do_root_so_com_o_necessario() {
        let user = env(&[
            ("DISPLAY", ":1"),
            ("XAUTHORITY", "/run/user/1000/xauth"),
            ("WAYLAND_DISPLAY", "mide-0"),
            ("XDG_RUNTIME_DIR", "/run/user/1000"),
            ("DBUS_SESSION_BUS_ADDRESS", "unix:path=/run/user/1000/bus"),
            ("HOME", "/home/fabio"),
            ("LANG", "pt_BR.UTF-8"),
            ("LC_TIME", "pt_BR.UTF-8"),
            ("LC_", "x"),
            ("SUDO_ASKPASS", "/usr/bin/mide-askpass"),
            ("MIDE_ASKPASS_STATE", "/run/user/1000/mide-askpass-1"),
        ]);
        let vars = root_env(&user, None);
        let n = names(&vars);
        assert_eq!(n, ["DISPLAY", "XAUTHORITY", "GDK_BACKEND", "QT_QPA_PLATFORM", "NO_AT_BRIDGE", "LANG", "LC_TIME"]);
        for proibida in ["WAYLAND_DISPLAY", "XDG_RUNTIME_DIR", "DBUS_SESSION_BUS_ADDRESS", "HOME", "SUDO_ASKPASS", "MIDE_ASKPASS_STATE"] {
            assert!(!n.contains(&proibida.to_string()), "{proibida} não pode ir para o root");
        }
    }

    #[test]
    fn xauthority_de_reserva_so_com_tela() {
        let p = Path::new("/home/fabio/.Xauthority");
        let vars = root_env(&env(&[("DISPLAY", ":0")]), Some(p));
        assert!(vars.contains(&("XAUTHORITY".into(), "/home/fabio/.Xauthority".into())));
        // Sem DISPLAY, nada de X11.
        let vars = root_env(&env(&[("LANG", "C")]), Some(p));
        assert!(!names(&vars).iter().any(|n| n == "DISPLAY" || n == "XAUTHORITY"));
    }

    #[test]
    fn tela_e_idioma_filtrados() {
        // Idioma com caminho, % ou .. não vai (como o env_check do sudo).
        let user = env(&[
            ("DISPLAY", ":1"),
            ("LANG", "pt_BR.UTF-8"),
            ("LC_ALL", "/home/u/x"),
            ("LC_TIME", "pt_BR%s"),
            ("LC_CTYPE", "../../home/u/x"),
            ("LANGUAGE", "pt_BR:pt"),
        ]);
        let n = names(&root_env(&user, None));
        assert!(n.contains(&"LANG".into()) && n.contains(&"LANGUAGE".into()));
        for fora in ["LC_ALL", "LC_TIME", "LC_CTYPE"] {
            assert!(!n.contains(&fora.to_string()), "{fora} não pode ir para o root");
        }
        // Só tela local; Xauthority só absoluto.
        for ruim in ["host:0", "/tmp/.X11-unix/X0", ":", ":a", ":1.", "unix:0"] {
            assert!(!names(&root_env(&env(&[("DISPLAY", ruim)]), None)).contains(&"DISPLAY".into()), "{ruim}");
        }
        assert_eq!(x11_display_number(OsStr::new(":12.0")), Some(12));
        assert_eq!(x11_display_number(OsStr::new(":0")), Some(0));
        let vars = root_env(&env(&[("DISPLAY", ":0"), ("XAUTHORITY", "xauth-relativo")]), None);
        assert!(!names(&vars).contains(&"XAUTHORITY".into()));
    }

    #[test]
    fn tela_x11_dividida_entre_os_mide_sudo() {
        let dir = std::env::temp_dir().join(format!("mide-x11-teste-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let vivos = |pid: u32| pid == 10 || pid == 20;
        let marcas = || {
            let mut v: Vec<String> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| !n.ends_with(".lock")).collect();
            v.sort();
            v
        };
        // Liberada por outro programa: não mexe.
        assert_eq!(X11Lock::acquire(&dir, 1).unwrap().join(10, true, &vivos).unwrap(), X11Grant::Skip);
        assert!(marcas().is_empty());
        // Primeiro libera; o segundo só entra; o primeiro a sair não desfaz.
        assert_eq!(X11Lock::acquire(&dir, 1).unwrap().join(10, false, &vivos).unwrap(), X11Grant::Add);
        assert_eq!(X11Lock::acquire(&dir, 1).unwrap().join(20, true, &vivos).unwrap(), X11Grant::Join);
        assert!(!X11Lock::acquire(&dir, 1).unwrap().leave(10, &|p| p == 20));
        assert!(X11Lock::acquire(&dir, 1).unwrap().leave(20, &|_| false));
        assert!(marcas().is_empty());
        // Um mide-sudo morto sem desfazer (marca do pid 99): o próximo fica com a
        // liberação e desfaz no fim.
        std::fs::write(dir.join("mide-sudo-x11-1-99"), b"").unwrap();
        assert_eq!(X11Lock::acquire(&dir, 1).unwrap().join(10, true, &vivos).unwrap(), X11Grant::Join);
        assert_eq!(marcas(), ["mide-sudo-x11-1-10"]);
        assert!(X11Lock::acquire(&dir, 1).unwrap().leave(10, &vivos));
        // Outra tela não conta.
        std::fs::write(dir.join("mide-sudo-x11-2-20"), b"").unwrap();
        assert_eq!(X11Lock::acquire(&dir, 1).unwrap().join(10, true, &vivos).unwrap(), X11Grant::Skip);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn linha_secreta_lida_sem_sobras() {
        let mut entrada: &[u8] = b"pass abc\\n\ncancel\nsem-fim";
        let mut line = Vec::new();
        assert!(read_secret_line(&mut entrada, &mut line).unwrap());
        assert_eq!(line, b"pass abc\\n");
        assert!(read_secret_line(&mut entrada, &mut line).unwrap());
        assert_eq!(line, b"cancel");
        assert!(read_secret_line(&mut entrada, &mut line).unwrap());
        assert_eq!(line, b"sem-fim");
        assert!(!read_secret_line(&mut entrada, &mut line).unwrap());
        // Linha grande demais é descartada inteira; a seguinte vem certa.
        let mut grande = vec![b'x'; MAX_LINE + 10];
        grande.extend_from_slice(b"\ncancel\n");
        let mut entrada: &[u8] = &grande;
        assert!(read_secret_line(&mut entrada, &mut line).unwrap());
        assert_eq!(line, b"cancel");
        wipe_bytes(&mut line);
        assert!(line.is_empty());
        // A linha da senha é feita sem crescer (capacidade do pior caso).
        let l = FromWindow::Pass("a\\b".repeat(50)).to_line().unwrap();
        assert!(l.capacity() > 5 + 2 * 150);
        assert_eq!(FromWindow::parse(&l), Some(FromWindow::Pass("a\\b".repeat(50))));
    }

    #[test]
    fn comando_do_sudo_com_env() {
        let vars = env(&[("DISPLAY", ":1"), ("LANG", "pt_BR.UTF-8")]);
        let cmd = sudo_command(OsStr::new(SUDO), OsStr::new("/opt/lampp/manager-linux-x64.run"), &["--x".into()], &vars, EnvMode::Env, true);
        let cmd: Vec<String> = cmd.iter().map(|s| s.to_string_lossy().into_owned()).collect();
        assert_eq!(cmd, [SUDO, "-A", "--", ENV, "-u", "LANGUAGE", "DISPLAY=:1", "LANG=pt_BR.UTF-8", "/opt/lampp/manager-linux-x64.run", "--x"]);
        // Com LANGUAGE do usuário, ele vai como está (sem o -u).
        let vars = env(&[("LANGUAGE", "pt_BR:pt")]);
        let cmd = sudo_command(OsStr::new(SUDO), OsStr::new("id"), &[], &vars, EnvMode::Env, false);
        let cmd: Vec<String> = cmd.iter().map(|s| s.to_string_lossy().into_owned()).collect();
        assert_eq!(cmd, [SUDO, "--", ENV, "LANGUAGE=pt_BR:pt", "id"]);
    }

    #[test]
    fn comando_do_sudo_com_preserve_env() {
        let vars = env(&[("DISPLAY", ":1"), ("GDK_BACKEND", "x11")]);
        let cmd = sudo_command(OsStr::new(SUDO), OsStr::new("gparted"), &[], &vars, EnvMode::PreserveEnv, true);
        let cmd: Vec<String> = cmd.iter().map(|s| s.to_string_lossy().into_owned()).collect();
        assert_eq!(cmd, [SUDO, "-A", "--preserve-env=DISPLAY,GDK_BACKEND", "--", "gparted"]);
    }

    #[test]
    fn programa_conferido() {
        let cwd = Path::new("/home/fabio/xampp");
        assert_eq!(resolve_program(OsStr::new("./manager.run"), cwd).unwrap(), OsString::from("/home/fabio/xampp/./manager.run"));
        assert_eq!(resolve_program(OsStr::new("gparted"), cwd).unwrap(), OsString::from("gparted"));
        assert!(resolve_program(OsStr::new("A=b"), cwd).is_err());
        assert!(resolve_program(OsStr::new(""), cwd).is_err());
        assert!(resolve_program(OsStr::new("--rm"), cwd).is_err());
    }

    #[test]
    fn texto_do_comando() {
        assert_eq!(command_text(OsStr::new("/usr/bin/id"), &["-u".into()]), "/usr/bin/id -u");
        assert_eq!(command_text(OsStr::new("sh"), &["-c".into(), "echo oi".into()]), "sh -c 'echo oi'");
        let longo = command_text(OsStr::new("x"), &["a".repeat(400).into()]);
        assert!(longo.ends_with('…') && longo.chars().count() == 240);
    }

    #[test]
    fn erros_do_sudo() {
        assert_eq!(classify_sudo_error("sudo: 3 incorrect password attempts\n"), Some(SudoFailure::WrongPassword));
        assert_eq!(classify_sudo_error("fabio is not in the sudoers file.\n"), Some(SudoFailure::NotAllowed));
        assert_eq!(
            classify_sudo_error("Sorry, user fabio is not allowed to execute '/usr/bin/env DISPLAY=:1 x' as root on pc.\n"),
            Some(SudoFailure::EnvRefused)
        );
        assert_eq!(classify_sudo_error("Sorry, user fabio is not allowed to execute '/usr/bin/x' as root on pc.\n"), Some(SudoFailure::NotAllowed));
        assert_eq!(classify_sudo_error("sudo: no password was provided\nsudo: a password is required\n"), Some(SudoFailure::NoPassword));
        assert_eq!(classify_sudo_error("erro qualquer do programa\n"), None);
    }

    #[test]
    fn estado_das_tentativas() {
        let dir = std::env::temp_dir().join(format!("mide-elevate-teste-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("estado");
        let _ = std::fs::remove_file(&path);
        create_state(&path).unwrap();
        // Só o dono lê.
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        // Não sobrescreve um que já existe.
        assert!(create_state(&path).is_err());
        assert_eq!(bump_attempt(&path).unwrap(), 1);
        assert_eq!(bump_attempt(&path).unwrap(), 2);
        mark_cancelled(&path).unwrap();
        assert_eq!(read_state(&path), AskState { attempts: 2, cancelled: true });
        assert_eq!(parse_state(""), AskState::default());
        assert_eq!(parse_state("lixo"), AskState::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn linhas_do_helper_do_polkit() {
        assert_eq!(parse_helper_line("PAM_PROMPT_ECHO_OFF Senha: \n"), HelperLine::PromptEchoOff("Senha: ".into()));
        assert_eq!(parse_helper_line("PAM_PROMPT_ECHO_ON Login:"), HelperLine::PromptEchoOn("Login:".into()));
        assert_eq!(parse_helper_line("PAM_ERROR_MSG Conta\\nbloqueada"), HelperLine::Error("Conta\nbloqueada".into()));
        assert_eq!(parse_helper_line("PAM_TEXT_INFO Toque no leitor \\303\\251"), HelperLine::Info("Toque no leitor é".into()));
        assert_eq!(parse_helper_line("SUCCESS\n"), HelperLine::Success);
        assert_eq!(parse_helper_line("FAILURE"), HelperLine::Failure);
        assert_eq!(parse_helper_line("outra"), HelperLine::Other("outra".into()));
    }

    const PASSWD: &str = "root:x:0:0:root:/root:/bin/bash\nfabio:x:1000:1000:Fabio,,,:/home/fabio:/bin/bash\nana:x:1001:1001::/home/ana:/bin/bash\nbeto:x:1002:27::/home/beto:/bin/sh\n";
    const GROUP: &str = "root:x:0:\nsudo:x:27:ana,fabio\nfabio:x:1000:\n";

    #[test]
    fn identidade_escolhida() {
        // O próprio usuário, direto ou pelo grupo sudo.
        assert_eq!(choose_identity(&[Identity::User(1001), Identity::User(1000)], 1000, PASSWD, GROUP).as_deref(), Some("fabio"));
        assert_eq!(choose_identity(&[Identity::Group(27)], 1000, PASSWD, GROUP).as_deref(), Some("fabio"));
        // Grupo principal também conta (beto tem o 27 como principal).
        assert_eq!(choose_identity(&[Identity::Group(27)], 1002, PASSWD, GROUP).as_deref(), Some("beto"));
        // Fora do grupo: o primeiro membro comum; o root por último.
        assert_eq!(choose_identity(&[Identity::Group(27)], 1003, PASSWD, GROUP).as_deref(), Some("ana"));
        assert_eq!(choose_identity(&[Identity::User(0), Identity::User(1001)], 1000, PASSWD, GROUP).as_deref(), Some("ana"));
        assert_eq!(choose_identity(&[Identity::User(0)], 1000, PASSWD, GROUP).as_deref(), Some("root"));
        assert_eq!(choose_identity(&[], 1000, PASSWD, GROUP), None);
    }

    #[test]
    fn protocolo_da_janela() {
        let msgs = [
            ToWindow::Message("Autenticação é necessária\npara rodar".into()),
            ToWindow::Command("/usr/bin/true --x \\ y".into()),
            ToWindow::User("fabio".into()),
            ToWindow::Ask("Senha: ".into()),
            ToWindow::Error("Senha incorreta".into()),
            ToWindow::Info("".into()),
            ToWindow::Busy,
            ToWindow::Close,
        ];
        for m in msgs {
            let line = m.to_line();
            assert!(line.ends_with('\n') && line.matches('\n').count() == 1, "{line:?}");
            assert_eq!(ToWindow::parse(&line), Some(m));
        }
        assert_eq!(ToWindow::parse("desconhecida x"), None);
        let pass = FromWindow::Pass("a\\b ç".into());
        assert_eq!(FromWindow::parse(&pass.to_line().unwrap()), Some(pass));
        assert_eq!(FromWindow::parse("cancel\n"), Some(FromWindow::Cancel));
        assert_eq!(FromWindow::parse("outra"), None);
        // Senha com quebra de linha não segue.
        assert_eq!(FromWindow::Pass("a\nb".into()).to_line(), None);
    }

    #[test]
    fn senha_apagada() {
        let mut s = String::from("segredo");
        wipe(&mut s);
        assert!(s.is_empty());
    }

    #[test]
    fn helper_procurado_nos_lugares_certos() {
        // Só conferir que um helper encontrado é setuid de root (nesta máquina pode não haver).
        if let Some(p) = find_helper() {
            use std::os::unix::fs::MetadataExt;
            let m = std::fs::metadata(p).unwrap();
            assert!(m.uid() == 0 && m.mode() & 0o4000 != 0);
        }
    }
}
