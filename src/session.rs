//! O mide como sessão de login (C2): iniciado pela tela de login pelo `mide-session`.
//!
//! Nessa hora o mide é o dono da sessão: publica o ambiente para os serviços do D-Bus e
//! do systemd (portais, notificações e outros apps ativados sob demanda precisam saber
//! o `WAYLAND_DISPLAY` e o `DISPLAY`) e inicia os apps de inicialização automática do
//! sistema e do usuário (padrão freedesktop de autostart).
//!
//! Só age com `MIDE_SESSION=1` (definido pelo `mide-session`): rodando numa janela, para
//! desenvolver, o mide não mexe na sessão de quem o abriu.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Command,
};

static ACTIVE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

/// Lê `MIDE_SESSION` uma vez, no começo do `main`, e a tira do ambiente: os apps abertos
/// dentro da sessão (terminal, editor) não a herdam. Sem isso, um mide de teste aberto
/// de um terminal da sessão se achava a sessão de login, publicava o socket de teste no
/// D-Bus/systemd do usuário e iniciava os apps automáticos de novo.
pub fn capture() {
    let active = std::env::var("MIDE_SESSION").is_ok_and(|v| v == "1");
    ACTIVE.get_or_init(|| active);
    // SAFETY: chamado no início do `main`, antes de qualquer outra thread existir.
    unsafe { std::env::remove_var("MIDE_SESSION") };
}

/// Apps automáticos abertos (pid e comando), passados ao mide reiniciado.
static STARTED: std::sync::Mutex<Vec<(u32, String)>> = std::sync::Mutex::new(Vec::new());

/// Lista dos apps automáticos abertos, para `MIDE_AUTOSTART` do mide reiniciado.
pub fn started_list() -> String {
    STARTED
        .lock()
        .map(|s| s.iter().map(|(pid, exec)| format!("{pid}\t{exec}")).collect::<Vec<_>>().join("\n"))
        .unwrap_or_default()
}

/// O mide foi iniciado como sessão de login?
pub fn active() -> bool {
    *ACTIVE.get_or_init(|| std::env::var("MIDE_SESSION").is_ok_and(|v| v == "1"))
}

/// Prepara a sessão: ambiente para o D-Bus/systemd e apps de inicialização automática.
pub fn start(socket: &str) {
    if !active() {
        return;
    }
    publish_environment(socket);
    // Reiniciado pelo `restart all`: os apps automáticos que continuam vivos (os que
    // não dependem do Wayland) não são abertos de novo.
    let still_running: Vec<(u32, String)> = std::env::var("MIDE_AUTOSTART")
        .unwrap_or_default()
        .lines()
        .filter_map(|line| line.split_once('\t'))
        .filter_map(|(pid, exec)| Some((pid.parse::<u32>().ok()?, exec.to_string())))
        .filter(|(pid, _)| std::path::Path::new(&format!("/proc/{pid}")).exists())
        .collect();
    // SAFETY: chamado na partida, antes das threads dos apps automáticos.
    unsafe { std::env::remove_var("MIDE_AUTOSTART") };
    let entries = autostart_entries(&autostart_dirs());
    tracing::info!("sessão: {} app(s) de inicialização automática", entries.len());
    for entry in entries {
        if let Some(running) = still_running.iter().find(|(_, exec)| *exec == entry.exec) {
            tracing::info!("sessão: {} continua aberto", entry.name);
            if let Ok(mut started) = STARTED.lock() {
                started.push(running.clone());
            }
            continue;
        }
        tracing::info!("sessão: iniciando {} ({})", entry.name, entry.exec);
        let mut command = Command::new("sh");
        command.arg("-c").arg(&entry.exec);
        crate::child_env(&mut command);
        match command.spawn() {
            Ok(mut child) => {
                if let Ok(mut started) = STARTED.lock() {
                    started.push((child.id(), entry.exec.clone()));
                }
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
            }
            Err(err) => tracing::warn!("sessão: {} não iniciou: {err}", entry.name),
        }
    }
}

/// Passa o ambiente da sessão aos serviços ativados pelo D-Bus e pelo systemd do
/// usuário (`dbus-update-activation-environment --systemd`).
fn publish_environment(socket: &str) {
    let mut vars = vec![
        format!("WAYLAND_DISPLAY={socket}"),
        "XDG_CURRENT_DESKTOP=mide".to_string(),
        "XDG_SESSION_TYPE=wayland".to_string(),
    ];
    if let Some(display) = crate::xwayland::display() {
        vars.push(format!("DISPLAY={display}"));
    }
    // A pasta de dados do mide vai junto: nela está `xdg-desktop-portal/mide-portals.conf`,
    // que diz ao portal quais implementações usar no desktop "mide".
    if let Some(dirs) = crate::data_dirs() {
        vars.push(format!("XDG_DATA_DIRS={dirs}"));
    }
    match Command::new("dbus-update-activation-environment").arg("--systemd").args(&vars).status() {
        Ok(status) if status.success() => tracing::info!("sessão: ambiente publicado ({})", vars.join(" ")),
        Ok(status) => tracing::warn!("sessão: dbus-update-activation-environment saiu com {status}"),
        Err(err) => tracing::warn!("sessão: dbus-update-activation-environment indisponível: {err}"),
    }
    // O portal pode ter sido iniciado antes do ambiente acima (por um app automático):
    // reiniciado, ele lê a configuração do mide e passa a oferecer as configurações do
    // sistema aos apps (layout dos botões, tema escuro, fontes).
    // Sem esperar (`--no-block`): o portal, ao subir, conversa com o próprio mide; se o
    // mide esperasse aqui, os dois ficariam presos (aconteceu no `restart all`).
    match Command::new("systemctl").args(["--user", "--no-block", "try-restart", "xdg-desktop-portal.service"]).status() {
        Ok(status) if status.success() => tracing::info!("sessão: portal do desktop reiniciado com a configuração do mide"),
        Ok(status) => tracing::warn!("sessão: reiniciar o portal saiu com {status}"),
        Err(err) => tracing::warn!("sessão: systemctl indisponível: {err}"),
    }
}

/// Pastas de autostart, da mais importante para a menos: a do usuário e depois as do
/// sistema (`XDG_CONFIG_DIRS`, padrão `/etc/xdg`).
fn autostart_dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let user = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| home.map(|h| h.join(".config")));
    let system = std::env::var("XDG_CONFIG_DIRS").ok().filter(|v| !v.is_empty()).unwrap_or_else(|| "/etc/xdg".into());
    user.into_iter()
        .chain(system.split(':').filter(|d| !d.is_empty()).map(PathBuf::from))
        .map(|d| d.join("autostart"))
        .collect()
}

/// App de inicialização automática que vale para o mide.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Autostart {
    pub name: String,
    pub exec: String,
}

/// Entradas `.desktop` das pastas de autostart que devem iniciar no mide. Um arquivo
/// com o mesmo nome numa pasta mais importante substitui (ou, com `Hidden=true`,
/// desliga) o das outras. Valem `OnlyShowIn`, `NotShowIn`, `TryExec` e
/// `X-GNOME-Autostart-enabled`.
pub fn autostart_entries(dirs: &[PathBuf]) -> Vec<Autostart> {
    let mut chosen: BTreeMap<String, PathBuf> = BTreeMap::new();
    for dir in dirs {
        let Ok(read) = std::fs::read_dir(dir) else {
            continue;
        };
        for file in read.flatten() {
            let path = file.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()).filter(|n| n.ends_with(".desktop")) else {
                continue;
            };
            chosen.entry(name.to_string()).or_insert(path);
        }
    }
    chosen.values().filter_map(|path| parse_entry(path)).collect()
}

/// Lê uma entrada e decide se ela inicia no mide.
fn parse_entry(path: &Path) -> Option<Autostart> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut keys: BTreeMap<&str, &str> = BTreeMap::new();
    let mut in_entry = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if let (true, Some((k, v))) = (in_entry, line.split_once('=')) {
            keys.entry(k.trim()).or_insert(v.trim());
        }
    }
    let list = |key: &str| keys.get(key).map(|v| v.split(';').filter(|s| !s.is_empty()).map(str::to_string).collect::<Vec<_>>());
    let yes = |key: &str| keys.get(key).is_some_and(|v| v.eq_ignore_ascii_case("true"));
    if keys.get("Type").is_some_and(|t| *t != "Application") || yes("Hidden") {
        return None;
    }
    if keys.get("X-GNOME-Autostart-enabled").is_some_and(|v| v.eq_ignore_ascii_case("false")) {
        return None;
    }
    if list("OnlyShowIn").is_some_and(|l| !l.iter().any(|d| d.eq_ignore_ascii_case("mide"))) {
        return None;
    }
    if list("NotShowIn").is_some_and(|l| l.iter().any(|d| d.eq_ignore_ascii_case("mide"))) {
        return None;
    }
    if let Some(try_exec) = keys.get("TryExec") {
        if !program_exists(try_exec) {
            return None;
        }
    }
    let exec = mide::apps::clean_exec(keys.get("Exec")?);
    if exec.is_empty() {
        return None;
    }
    let name = keys.get("Name").map_or_else(|| exec.clone(), |n| n.to_string());
    Some(Autostart { name, exec })
}

/// O programa existe (caminho absoluto ou no PATH)?
fn program_exists(program: &str) -> bool {
    let path = Path::new(program);
    if path.is_absolute() {
        return path.is_file();
    }
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(program).is_file()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, name: &str, body: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join(name), format!("[Desktop Entry]\nType=Application\n{body}")).unwrap();
    }

    #[test]
    fn regras_de_autostart() {
        let base = std::env::temp_dir().join(format!("mide-autostart-{}", std::process::id()));
        let (user, system) = (base.join("user"), base.join("system"));
        write(&system, "comum.desktop", "Name=Comum\nExec=comum --flag %U");
        write(&system, "so-gnome.desktop", "Name=Só GNOME\nExec=x\nOnlyShowIn=GNOME;");
        write(&system, "nao-mide.desktop", "Name=Não mide\nExec=x\nNotShowIn=mide;KDE;");
        write(&system, "desligado.desktop", "Name=Desligado\nExec=x\nX-GNOME-Autostart-enabled=false");
        write(&system, "sem-programa.desktop", "Name=Sem programa\nExec=x\nTryExec=/nao/existe/programa");
        write(&system, "escondido.desktop", "Name=Sistema\nExec=sistema");
        write(&user, "escondido.desktop", "Name=Usuário\nExec=x\nHidden=true");
        write(&system, "trocado.desktop", "Name=Sistema\nExec=sistema");
        write(&user, "trocado.desktop", "Name=Do usuário\nExec=do-usuario\nOnlyShowIn=mide;");
        let found = autostart_entries(&[user, system]);
        let names: Vec<&str> = found.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, vec!["Comum", "Do usuário"]);
        assert_eq!(found[0].exec, "comum --flag");
        std::fs::remove_dir_all(&base).unwrap();
    }
}
