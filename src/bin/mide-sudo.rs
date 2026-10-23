//! Roda um programa como administrador pedindo a senha numa janela do mide (C7), para
//! usar em `.desktop` e scripts sem terminal:
//!
//! ```text
//! Exec=mide-sudo --name XAMPP /opt/lampp/manager-linux-x64.run
//! ```
//!
//! Uso: `mide-sudo [--name NOME] [--icon ICONE] [--gui] <programa> [argumentos]`.
//!
//! - Dentro de um terminal (entrada padrão é um terminal e sem `--gui`), é o `sudo`
//!   normal: a senha é pedida no próprio terminal.
//! - Sem terminal, roda `sudo -A` com o `mide-askpass` como `SUDO_ASKPASS`: a janela
//!   pede a senha, o sudo confere (PAM) e roda o programa. Errou a senha 3 vezes ou o
//!   usuário não pode usar o sudo: aviso nas notificações. Cancelar sai com 1, sem aviso.
//!
//! O programa recebe só a tela X11 (`DISPLAY`/`XAUTHORITY`), o idioma e os apps forçados
//! ao X11 (`mide::elevate::root_env`): nada do Wayland nem da sessão D-Bus do usuário.
//! Sai com o código do programa (ou do sudo).

use std::{
    ffi::OsString,
    io::{IsTerminal, Read, Write},
    os::unix::process::{CommandExt, ExitStatusExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Mutex,
};

use mide::elevate::{self, EnvMode, SudoFailure};

const USAGE: &str = "uso: mide-sudo [--name NOME] [--icon ICONE] [--gui] <programa> [argumentos]";

struct Request {
    name: Option<String>,
    icon: Option<String>,
    gui: bool,
    program: OsString,
    args: Vec<OsString>,
}

fn parse_args(mut args: Vec<OsString>) -> Result<Request, String> {
    let mut name = None;
    let mut icon = None;
    let mut gui = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].to_str() {
            Some("--name") | Some("--icon") => {
                let value = args.get(i + 1).ok_or_else(|| format!("{} sem valor", args[i].to_string_lossy()))?.to_string_lossy().into_owned();
                if args[i] == "--name" {
                    name = Some(value);
                } else {
                    icon = Some(value);
                }
                i += 2;
            }
            Some("--gui") => {
                gui = true;
                i += 1;
            }
            Some("--") => {
                i += 1;
                break;
            }
            Some("-h") | Some("--help") => return Err(USAGE.into()),
            _ => break,
        }
    }
    let mut rest = args.split_off(i).into_iter();
    let program = rest.next().ok_or_else(|| USAGE.to_string())?;
    Ok(Request { name, icon, gui, program, args: rest.collect() })
}

/// Programa do mide ao lado deste (instalado ou em target/), com caminho absoluto: o
/// sudo exige o `SUDO_ASKPASS` absoluto.
fn sibling(name: &str) -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let path = exe.parent()?.join(name);
    path.exists().then_some(path)
}

/// O `sudo`: sempre `/usr/bin/sudo`; `MIDE_SUDO_BIN` (o sudo falso dos testes) só nas
/// compilações de desenvolvimento.
fn sudo_bin() -> OsString {
    if cfg!(debug_assertions)
        && let Some(fake) = std::env::var_os("MIDE_SUDO_BIN").filter(|v| !v.is_empty())
    {
        return fake;
    }
    elevate::SUDO.into()
}

/// Pasta de execução do usuário (estado da janela e marcas da tela X11).
fn runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR").filter(|d| !d.is_empty()).map(PathBuf::from).unwrap_or_else(std::env::temp_dir)
}

/// O processo é um `mide-sudo` (a marca dele na tela X11 ainda vale)?
fn is_mide_sudo(pid: u32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/comm")).is_ok_and(|c| c.trim_end() == "mide-sudo")
}

/// Este `mide-sudo` participa da liberação da tela X11 para o root.
struct X11Hold {
    dir: PathBuf,
    display: u32,
}

fn xhost(arg: Option<&str>) -> Option<std::process::Output> {
    let mut cmd = Command::new("xhost");
    if let Some(arg) = arg {
        cmd.arg(arg);
    }
    cmd.stdin(Stdio::null()).stderr(Stdio::null()).output().ok().filter(|o| o.status.success())
}

/// Libera a tela X11 para o root (`xhost +si:localuser:root`) enquanto o programa roda:
/// o XWayland do mide não usa Xauthority. Vários `mide-sudo` ao mesmo tempo dividem a
/// liberação (`elevate::X11Lock`): só o último a sair desfaz. Liberada por outro
/// programa: não mexe.
fn grant_x11_root() -> Option<X11Hold> {
    let display = elevate::x11_display_number(&std::env::var_os("DISPLAY")?)?;
    let dir = runtime_dir();
    let lock = match elevate::X11Lock::acquire(&dir, display) {
        Ok(lock) => lock,
        Err(err) => {
            tracing::warn!("tela X11: trava indisponível ({err}); sem liberar para o root");
            return None;
        }
    };
    let text = String::from_utf8_lossy(&xhost(None)?.stdout).to_lowercase();
    if text.contains("access control disabled") {
        return None;
    }
    let me = std::process::id();
    let hold = X11Hold { dir: dir.clone(), display };
    match lock.join(me, text.contains("si:localuser:root"), &is_mide_sudo) {
        Ok(elevate::X11Grant::Skip) => None,
        Ok(elevate::X11Grant::Join) => {
            tracing::info!("tela X11 já liberada para o root por outro mide-sudo");
            Some(hold)
        }
        Ok(elevate::X11Grant::Add) => {
            if xhost(Some("+si:localuser:root")).is_some() {
                tracing::info!("tela X11 liberada para o root enquanto o programa roda");
                Some(hold)
            } else {
                lock.leave(me, &is_mide_sudo);
                None
            }
        }
        Err(err) => {
            tracing::warn!("tela X11: marca não gravada ({err}); sem liberar para o root");
            None
        }
    }
}

/// Sai da liberação; o último desfaz (sem a trava, desfaz assim mesmo: mais seguro).
fn release_x11_root(hold: X11Hold) {
    let last = match elevate::X11Lock::acquire(&hold.dir, hold.display) {
        Ok(lock) => lock.leave(std::process::id(), &is_mide_sudo),
        Err(_) => true,
    };
    if last {
        let _ = xhost(Some("-si:localuser:root"));
        tracing::info!("tela X11 fechada de novo para o root");
    }
}

/// O que desfazer no fim (também quando o `mide-sudo` é encerrado por sinal).
#[derive(Default)]
struct Cleanup {
    x11: Option<X11Hold>,
    state: Option<PathBuf>,
}

static CLEANUP: Mutex<Option<Cleanup>> = Mutex::new(None);
/// Grupo de processos do sudo em andamento (o sudo, a janela de senha que ele abriu),
/// para repassar um sinal recebido.
static CHILD_GROUP: Mutex<Option<i32>> = Mutex::new(None);

fn cleanup() {
    let Some(c) = CLEANUP.lock().ok().and_then(|mut c| c.take()) else {
        return;
    };
    if let Some(hold) = c.x11 {
        release_x11_root(hold);
    }
    if let Some(state) = c.state {
        let _ = std::fs::remove_file(state);
    }
}

/// SIGTERM, SIGINT e SIGHUP: com o sudo rodando, o sinal vai para o grupo dele (o sudo
/// repassa ao programa, a janela de senha fecha) e o `main` desfaz tudo quando ele
/// terminar; antes disso, desfaz aqui e sai. Os sinais ficam bloqueados nas threads e
/// uma thread própria espera por eles (`sigwait`); o sudo é aberto com a máscara limpa
/// (`pre_exec` no `run_sudo`: o `spawn` herda a máscara).
fn handle_signals() {
    // SAFETY: chamadas da libc com um sigset_t local, antes de criar outras threads.
    let set = unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        for sig in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
            libc::sigaddset(&mut set, sig);
        }
        if libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut()) != 0 {
            return;
        }
        set
    };
    std::thread::spawn(move || {
        loop {
            let mut sig: libc::c_int = 0;
            // SAFETY: `set` é um sigset_t válido e `sig` uma saída local.
            if unsafe { libc::sigwait(&set, &mut sig) } != 0 {
                continue;
            }
            tracing::info!("sinal {sig} recebido");
            let group = CHILD_GROUP.lock().ok().and_then(|g| *g);
            match group {
                Some(pgid) => {
                    // SAFETY: kill com um grupo de processos (número negativo).
                    unsafe { libc::kill(-pgid, sig) };
                }
                None => {
                    cleanup();
                    std::process::exit(128 + sig);
                }
            }
        }
    });
}

/// Arquivo de estado da janela (0600) na pasta de execução do usuário.
fn state_file() -> Option<PathBuf> {
    let dir = runtime_dir();
    // Sobras de um mide-sudo encerrado à força (o pid não existe mais).
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(pid) = name.to_str().and_then(|n| n.strip_prefix("mide-askpass-")).and_then(|p| p.parse::<u32>().ok()) else {
                continue;
            };
            if !Path::new(&format!("/proc/{pid}")).exists() {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    let path = dir.join(format!("mide-askpass-{}", std::process::id()));
    let _ = std::fs::remove_file(&path);
    elevate::create_state(&path).ok().map(|_| path)
}

/// Roda o sudo com a janela e devolve o status e o fim do stderr dele (o stderr passa
/// adiante, para quem chamou ver também).
#[allow(clippy::too_many_arguments)]
fn run_sudo(cmd: &[OsString], vars: &[(OsString, OsString)], mode: EnvMode, askpass: &Path, state: Option<&Path>, req: &Request, title: &str, command_text: &str) -> std::io::Result<(std::process::ExitStatus, String)> {
    let mut command = Command::new(&cmd[0]);
    command.args(&cmd[1..]).stdin(Stdio::null()).stderr(Stdio::piped());
    command.env("SUDO_ASKPASS", askpass);
    command.env(elevate::TITLE_VAR, title).env(elevate::COMMAND_VAR, command_text);
    match &req.icon {
        Some(icon) => command.env(elevate::ICON_VAR, icon),
        None => command.env_remove(elevate::ICON_VAR),
    };
    if let Some(state) = state {
        command.env(elevate::STATE_VAR, state);
    }
    match mode {
        // Mensagens do sudo em inglês, para entender por que falhou; o programa recebe o
        // LANGUAGE do usuário (ou nenhum) pelo `env`.
        EnvMode::Env => {
            command.env("LANGUAGE", "en");
        }
        // Com --preserve-env, o sudo copia as variáveis do próprio ambiente.
        EnvMode::PreserveEnv => {
            for (k, v) in vars {
                command.env(k, v);
            }
        }
    }
    // Grupo próprio: um sinal recebido pelo mide-sudo vai para o sudo e a janela dele.
    command.process_group(0);
    // O sudo começa com os sinais desbloqueados (a máscara é herdada no `spawn`: sem
    // isto, o TERM repassado ficaria bloqueado no sudo e na janela).
    // SAFETY: só funções seguras entre o fork e o exec (sigemptyset, pthread_sigmask).
    unsafe {
        command.pre_exec(|| {
            let mut set: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut set);
            libc::pthread_sigmask(libc::SIG_SETMASK, &set, std::ptr::null_mut());
            Ok(())
        });
    }
    let mut child = {
        let mut group = CHILD_GROUP.lock().unwrap_or_else(|e| e.into_inner());
        let child = command.spawn()?;
        *group = Some(child.id() as i32);
        child
    };
    let mut stderr = child.stderr.take().expect("stderr do sudo");
    let reader = std::thread::spawn(move || {
        let mut tail: Vec<u8> = Vec::new();
        let mut buf = [0u8; 4096];
        let mut out = std::io::stderr();
        while let Ok(n) = stderr.read(&mut buf) {
            if n == 0 {
                break;
            }
            let _ = out.write_all(&buf[..n]);
            tail.extend_from_slice(&buf[..n]);
            if tail.len() > 16 * 1024 {
                tail.drain(..tail.len() - 16 * 1024);
            }
        }
        String::from_utf8_lossy(&tail).into_owned()
    });
    let status = child.wait();
    if let Ok(mut group) = CHILD_GROUP.lock() {
        *group = None;
    }
    let status = status?;
    let tail = reader.join().unwrap_or_default();
    Ok((status, tail))
}

fn exit_code(status: std::process::ExitStatus) -> i32 {
    status.code().unwrap_or_else(|| 128 + status.signal().unwrap_or(1))
}

fn main() {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "mide_sudo=info,mide::notify=info".into()))
        .init();
    let req = match parse_args(std::env::args_os().skip(1).collect()) {
        Ok(req) => req,
        Err(msg) => {
            eprintln!("{msg}");
            std::process::exit(2);
        }
    };
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
    let program = match elevate::resolve_program(&req.program, &cwd) {
        Ok(p) => p,
        Err(msg) => {
            eprintln!("mide-sudo: {msg}");
            std::process::exit(2);
        }
    };
    let env: Vec<(OsString, OsString)> = std::env::vars_os().collect();
    let home_xauth = std::env::var_os("HOME").map(|h| Path::new(&h).join(".Xauthority")).filter(|p| p.is_file());
    let vars = elevate::root_env(&env, home_xauth.as_deref());
    let sudo = sudo_bin();

    // Num terminal, o sudo de sempre (a senha no terminal).
    if std::io::stdin().is_terminal() && !req.gui {
        let cmd = elevate::sudo_command(&sudo, &program, &req.args, &vars, EnvMode::Env, false);
        let err = Command::new(&cmd[0]).args(&cmd[1..]).exec();
        eprintln!("mide-sudo: o sudo não abriu: {err}");
        std::process::exit(1);
    }

    let Some(askpass) = sibling("mide-askpass") else {
        eprintln!("mide-sudo: mide-askpass não encontrado ao lado do mide-sudo");
        mide::notify::send_and_wait("Senha de administrador", "Não foi possível pedir a senha", "O mide-askpass não está instalado.");
        std::process::exit(1);
    };
    // Nome de quem pede: --name, o app aberto pelo launcher ou pela dock, ou o programa.
    let title = req
        .name
        .clone()
        .or_else(|| std::env::var("MIDE_APP_NAME").ok().filter(|n| !n.trim().is_empty()))
        .unwrap_or_else(|| Path::new(&program).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
    let req = Request { icon: req.icon.clone().or_else(|| std::env::var("MIDE_APP_ICON").ok().filter(|i| !i.trim().is_empty())), ..req };
    let command_text = elevate::command_text(&program, &req.args);
    tracing::info!("pedindo a senha de administrador para: {command_text}");

    handle_signals();
    let state = state_file();
    if let Ok(mut c) = CLEANUP.lock() {
        *c = Some(Cleanup { state: state.clone(), ..Default::default() });
    }
    let granted = grant_x11_root();
    if let Ok(mut c) = CLEANUP.lock()
        && let Some(c) = c.as_mut()
    {
        c.x11 = granted;
    }
    let mut mode = EnvMode::Env;
    let result = loop {
        let cmd = elevate::sudo_command(&sudo, &program, &req.args, &vars, mode, true);
        let result = run_sudo(&cmd, &vars, mode, &askpass, state.as_deref(), &req, &title, &command_text);
        // Regra do sudoers só para o programa (sem o `env`): tenta de novo pelo
        // --preserve-env, com a contagem de tentativas do zero.
        if mode == EnvMode::Env
            && let Ok((status, tail)) = &result
            && !status.success()
            && elevate::classify_sudo_error(tail) == Some(SudoFailure::EnvRefused)
        {
            tracing::info!("o sudoers não deixa rodar o env: tentando com --preserve-env");
            if let Some(state) = &state {
                let _ = elevate::write_state(state, elevate::AskState::default());
            }
            mode = EnvMode::PreserveEnv;
            continue;
        }
        break result;
    };
    let asked = state.as_deref().map(elevate::read_state).unwrap_or_default();
    cleanup();
    let (status, tail) = match result {
        Ok(r) => r,
        Err(err) => {
            eprintln!("mide-sudo: o sudo não abriu: {err}");
            mide::notify::send_and_wait("Senha de administrador", "Não foi possível pedir a senha", &format!("O sudo não abriu: {err}"));
            std::process::exit(1);
        }
    };
    let code = exit_code(status);
    if status.success() {
        tracing::info!("{title}: terminou (código 0)");
        std::process::exit(0);
    }
    if asked.cancelled {
        tracing::info!("{title}: cancelado na janela de senha");
        std::process::exit(code.max(1));
    }
    let user = elevate::current_user();
    match elevate::classify_sudo_error(&tail) {
        Some(SudoFailure::WrongPassword) => {
            tracing::info!("{title}: senha incorreta {} vezes", asked.attempts.max(elevate::MAX_TRIES));
            mide::notify::send_and_wait("Senha de administrador", "Senha incorreta", &format!("{title} não foi aberto: a senha de {user} foi recusada {} vezes.", asked.attempts.max(elevate::MAX_TRIES)));
        }
        Some(SudoFailure::NotAllowed) | Some(SudoFailure::EnvRefused) => {
            tracing::info!("{title}: {user} não pode usar o sudo para este programa");
            mide::notify::send_and_wait("Senha de administrador", "Sem permissão de administrador", &format!("{user} não pode usar o sudo para abrir {title}."));
        }
        Some(SudoFailure::NoPassword) => {
            tracing::info!("{title}: nenhuma senha foi dada");
        }
        None => tracing::info!("{title}: terminou com o código {code}"),
    }
    std::process::exit(code);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    #[test]
    fn argumentos() {
        let r = parse_args(args(&["--name", "XAMPP", "/opt/lampp/manager-linux-x64.run"])).unwrap();
        assert_eq!(r.name.as_deref(), Some("XAMPP"));
        assert_eq!(r.program, "/opt/lampp/manager-linux-x64.run");
        assert!(r.args.is_empty() && !r.gui);
        let r = parse_args(args(&["--gui", "--icon", "xampp", "--", "--estranho", "-x"])).unwrap();
        assert!(r.gui);
        assert_eq!(r.icon.as_deref(), Some("xampp"));
        assert_eq!(r.program, "--estranho");
        assert_eq!(r.args, args(&["-x"]));
        // As opções do programa ficam com ele.
        let r = parse_args(args(&["gparted", "--name", "x"])).unwrap();
        assert_eq!(r.program, "gparted");
        assert_eq!(r.args, args(&["--name", "x"]));
        assert!(parse_args(args(&[])).is_err());
        assert!(parse_args(args(&["--name"])).is_err());
    }
}
