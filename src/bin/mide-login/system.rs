//! O que a tela de login mostra do sistema: os usuários (AccountsService no D-Bus, ou
//! o `/etc/passwd`), as sessões instaladas (`wayland-sessions` e `xsessions`), os
//! layouts de teclado do sistema e a última escolha de cada usuário.

use std::path::{Path, PathBuf};

/// Um usuário que pode entrar.
#[derive(Debug, Clone, PartialEq)]
pub struct User {
    pub login: String,
    pub name: String,
    /// Foto do AccountsService, se houver.
    pub icon: Option<PathBuf>,
}

/// Usuários comuns do sistema (sem as contas de serviço), em ordem de nome.
pub fn users() -> Vec<User> {
    if let Some(list) = test_users() {
        return list;
    }
    let mut list = accounts_service().filter(|l| !l.is_empty()).unwrap_or_else(passwd_users);
    list.sort_by_key(|u| u.name.to_lowercase());
    list
}

/// Usuários dos testes (`MIDE_LOGIN_TEST_USERS="ana:Ana Souza;bruno:Bruno"`), só nas
/// compilações de desenvolvimento.
fn test_users() -> Option<Vec<User>> {
    if !cfg!(debug_assertions) {
        return None;
    }
    let text = std::env::var("MIDE_LOGIN_TEST_USERS").ok()?;
    Some(
        text.split(';')
            .filter_map(|item| {
                let (login, name) = item.split_once(':').unwrap_or((item, item));
                (!login.trim().is_empty()).then(|| User { login: login.trim().into(), name: name.trim().into(), icon: None })
            })
            .collect(),
    )
}

/// Pelo AccountsService (`org.freedesktop.Accounts`), como o GDM: nome completo e foto.
fn accounts_service() -> Option<Vec<User>> {
    let conn = zbus::blocking::Connection::system().ok()?;
    let accounts = zbus::blocking::Proxy::new(&conn, "org.freedesktop.Accounts", "/org/freedesktop/Accounts", "org.freedesktop.Accounts").ok()?;
    let paths: Vec<zbus::zvariant::OwnedObjectPath> = accounts.call("ListCachedUsers", &()).ok()?;
    let users = paths
        .into_iter()
        .filter_map(|path| {
            let user = zbus::blocking::Proxy::new(&conn, "org.freedesktop.Accounts", path, "org.freedesktop.Accounts.User").ok()?;
            if user.get_property::<bool>("SystemAccount").unwrap_or(false) {
                return None;
            }
            let login: String = user.get_property("UserName").ok()?;
            let real: String = user.get_property("RealName").unwrap_or_default();
            let icon: String = user.get_property("IconFile").unwrap_or_default();
            let icon = Some(PathBuf::from(icon)).filter(|p| p.is_file());
            Some(User { name: if real.trim().is_empty() { login.clone() } else { real }, login, icon })
        })
        .collect();
    Some(users)
}

/// Pelo `/etc/passwd`: UIDs entre `UID_MIN` e `UID_MAX` do `login.defs`, com um shell
/// de verdade.
fn passwd_users() -> Vec<User> {
    let defs = std::fs::read_to_string("/etc/login.defs").unwrap_or_default();
    let (min, max) = uid_range(&defs);
    let passwd = std::fs::read_to_string("/etc/passwd").unwrap_or_default();
    parse_passwd(&passwd, min, max)
}

fn uid_range(defs: &str) -> (u32, u32) {
    let value = |key: &str| {
        defs.lines()
            .map(str::trim)
            .filter(|l| !l.starts_with('#'))
            .find_map(|l| l.strip_prefix(key).and_then(|rest| rest.trim().parse::<u32>().ok()))
    };
    (value("UID_MIN").unwrap_or(1000), value("UID_MAX").unwrap_or(60000))
}

fn parse_passwd(passwd: &str, min: u32, max: u32) -> Vec<User> {
    passwd
        .lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split(':').collect();
            let uid: u32 = f.get(2)?.parse().ok()?;
            let shell = f.get(6)?;
            if uid < min || uid > max || shell.ends_with("nologin") || shell.ends_with("false") {
                return None;
            }
            let real = f.get(4)?.split(',').next().unwrap_or("").trim();
            Some(User { login: f[0].into(), name: if real.is_empty() { f[0].into() } else { real.into() }, icon: None })
        })
        .collect()
}

/// Uma sessão instalada.
#[derive(Debug, Clone, PartialEq)]
pub struct Session {
    /// Nome do arquivo sem `.desktop` (o que fica lembrado).
    pub id: String,
    pub name: String,
    /// Comando (já sem os códigos `%f` e afins).
    pub exec: Vec<String>,
    pub wayland: bool,
    /// `DesktopNames` (vai em `XDG_CURRENT_DESKTOP`).
    pub desktop_names: String,
}

impl Session {
    /// Comando e ambiente do `start_session` do greetd. Sessões X11 sobem pelo `startx`.
    pub fn command(&self) -> (Vec<String>, Vec<String>) {
        let mut env = vec![
            format!("XDG_SESSION_TYPE={}", if self.wayland { "wayland" } else { "x11" }),
            format!("XDG_SESSION_DESKTOP={}", self.id),
        ];
        if !self.desktop_names.is_empty() {
            env.push(format!("XDG_CURRENT_DESKTOP={}", self.desktop_names.trim_end_matches(';').replace(';', ":")));
        }
        let cmd = if self.wayland {
            self.exec.clone()
        } else {
            let mut cmd = vec!["startx".to_string(), "/usr/bin/env".to_string()];
            cmd.extend(self.exec.iter().cloned());
            cmd
        };
        (cmd, env)
    }

    /// A sessão do mide (a padrão).
    pub fn is_mide(&self) -> bool {
        self.id == "mide" || self.exec.first().is_some_and(|e| e.ends_with("mide-session"))
    }
}

/// Sessões das pastas `wayland-sessions` e `xsessions` (`MIDE_LOGIN_SESSIONS_DIRS` troca
/// as pastas, separadas por `:`), com a do mide primeiro. Sessões X11 só com o `startx`.
pub fn sessions() -> Vec<Session> {
    let dirs: Vec<(PathBuf, bool)> = match std::env::var("MIDE_LOGIN_SESSIONS_DIRS") {
        Ok(list) => list.split(':').filter(|d| !d.is_empty()).map(|d| (PathBuf::from(d), !d.contains("xsessions"))).collect(),
        Err(_) => {
            let data = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
            data.split(':')
                .filter(|d| !d.is_empty())
                .flat_map(|d| [(Path::new(d).join("wayland-sessions"), true), (Path::new(d).join("xsessions"), false)])
                .collect()
        }
    };
    let has_startx = which("startx");
    let mut list: Vec<Session> = Vec::new();
    for (dir, wayland) in dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut files: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "desktop")).collect();
        files.sort();
        for path in files {
            let Some(session) = std::fs::read_to_string(&path).ok().and_then(|text| parse_session(&path, &text, wayland)) else {
                continue;
            };
            if (!session.wayland && !has_startx) || list.iter().any(|s| s.id == session.id) {
                continue;
            }
            list.push(session);
        }
    }
    list.sort_by_key(|s| (!s.is_mide(), s.name.to_lowercase()));
    list
}

/// Lê um `.desktop` de sessão (sem as escondidas e as de `TryExec` ausente).
pub fn parse_session(path: &Path, text: &str, wayland: bool) -> Option<Session> {
    let lang = std::env::var("LANG").unwrap_or_default();
    let lang = lang.split('.').next().unwrap_or("").to_string();
    let short = lang.split('_').next().unwrap_or("").to_string();
    let mut main = false;
    let (mut name, mut localized, mut exec, mut names) = (String::new(), None::<(u8, String)>, String::new(), String::new());
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            main = line == "[Desktop Entry]";
            continue;
        }
        if !main {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        match key {
            "Name" => name = value.into(),
            "Exec" => exec = value.into(),
            "DesktopNames" => names = value.into(),
            "Hidden" | "NoDisplay" if value == "true" => return None,
            "TryExec" if !which(value) => return None,
            _ if key == format!("Name[{lang}]") => localized = Some((2, value.into())),
            _ if key == format!("Name[{short}]") && localized.as_ref().is_none_or(|(rank, _)| *rank < 1) => localized = Some((1, value.into())),
            _ => {}
        }
    }
    let exec: Vec<String> = split_exec(&exec);
    if exec.is_empty() {
        return None;
    }
    let id = path.file_stem()?.to_string_lossy().into_owned();
    Some(Session { name: localized.map(|(_, n)| n).unwrap_or(name), id, exec, wayland, desktop_names: names })
}

/// Separa o `Exec` em palavras (aspas duplas) e tira os códigos de campo (`%f`, `%U`...).
fn split_exec(exec: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let (mut quoted, mut escaped, mut started) = (false, false, false);
    for c in exec.chars() {
        if escaped {
            word.push(c);
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == '"' {
            quoted = !quoted;
            started = true;
        } else if c.is_whitespace() && !quoted {
            if started || !word.is_empty() {
                words.push(std::mem::take(&mut word));
            }
            started = false;
        } else {
            word.push(c);
        }
    }
    if started || !word.is_empty() {
        words.push(word);
    }
    words.into_iter().filter(|w| !(w.len() == 2 && w.starts_with('%'))).collect()
}

/// O programa existe (caminho ou no PATH)?
fn which(program: &str) -> bool {
    if program.contains('/') {
        return Path::new(program).is_file();
    }
    std::env::var("PATH").unwrap_or_default().split(':').any(|dir| Path::new(dir).join(program).is_file())
}

/// Layouts de teclado do sistema (`/etc/default/keyboard`, `XKBLAYOUT` com vários
/// separados por vírgula): o compositor de login usa os mesmos, na mesma ordem.
pub fn keyboard_layouts() -> Vec<mide::keyboard_layouts::Choice> {
    let text = std::fs::read_to_string("/etc/default/keyboard").unwrap_or_default();
    let value = |key: &str| {
        text.lines()
            .filter_map(|l| l.trim().strip_prefix(key)?.strip_prefix('='))
            .map(|v| v.trim().trim_matches('"').trim_matches('\'').to_string())
            .next()
            .unwrap_or_default()
    };
    let (layouts, variants) = (value("XKBLAYOUT"), value("XKBVARIANT"));
    let variants: Vec<&str> = variants.split(',').collect();
    layouts
        .split(',')
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty())
        .map(|(i, l)| mide::keyboard_layouts::Choice { layout: l.trim().into(), variant: variants.get(i).map(|v| v.trim().to_string()).unwrap_or_default() })
        .collect()
}

/// Última escolha: o último usuário e a sessão de cada um. Fica na pasta de estado do
/// usuário do greeter; sem permissão de escrita, só não lembra.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Memory {
    pub last_user: Option<String>,
    pub sessions: Vec<(String, String)>,
}

impl Memory {
    fn path() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
        Some(base.join("mide-login/state"))
    }

    pub fn load() -> Self {
        Self::path().and_then(|p| std::fs::read_to_string(p).ok()).map(|t| Self::parse(&t)).unwrap_or_default()
    }

    pub fn parse(text: &str) -> Self {
        let mut memory = Self::default();
        for line in text.lines() {
            if let Some(user) = line.strip_prefix("last_user=") {
                memory.last_user = Some(user.trim().to_string()).filter(|u| !u.is_empty());
            } else if let Some((user, session)) = line.strip_prefix("session.").and_then(|r| r.split_once('=')) {
                memory.sessions.push((user.trim().into(), session.trim().into()));
            }
        }
        memory
    }

    pub fn text(&self) -> String {
        let mut out = String::new();
        if let Some(user) = &self.last_user {
            out.push_str(&format!("last_user={user}\n"));
        }
        for (user, session) in &self.sessions {
            out.push_str(&format!("session.{user}={session}\n"));
        }
        out
    }

    pub fn session_of(&self, user: &str) -> Option<&str> {
        self.sessions.iter().find(|(u, _)| u == user).map(|(_, s)| s.as_str())
    }

    /// Guarda o usuário e a sessão que entraram.
    pub fn remember(&mut self, user: &str, session: &str) {
        self.last_user = Some(user.to_string());
        self.sessions.retain(|(u, _)| u != user);
        self.sessions.push((user.to_string(), session.to_string()));
        let Some(path) = Self::path() else {
            return;
        };
        let saved = path.parent().map(std::fs::create_dir_all).transpose().and_then(|_| std::fs::write(&path, self.text()));
        if let Err(err) = saved {
            tracing::info!("login: última escolha não guardada ({}): {err}", path.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usuarios_comuns_do_passwd() {
        let passwd = "root:x:0:0:root:/root:/bin/bash\nfabio:x:1000:1000:Fabio Carneiro,,,:/home/fabio:/bin/bash\nnobody:x:65534:65534:nobody:/nonexistent:/usr/sbin/nologin\nsvc:x:1001:1001::/srv:/usr/sbin/nologin\nana:x:1002:1002::/home/ana:/bin/zsh\n";
        let users = parse_passwd(passwd, 1000, 60000);
        assert_eq!(users.iter().map(|u| (u.login.as_str(), u.name.as_str())).collect::<Vec<_>>(), [("fabio", "Fabio Carneiro"), ("ana", "ana")]);
        assert_eq!(uid_range("# x\nUID_MIN\t\t 500\nUID_MAX 1999\n"), (500, 1999));
        assert_eq!(uid_range(""), (1000, 60000));
    }

    #[test]
    fn sessoes_do_desktop() {
        let text = "[Desktop Entry]\nName=mide\nName[pt_BR]=mide (pt)\nExec=mide-session --x \"a b\" %U\nDesktopNames=mide;\n[Desktop Action y]\nName=nada\n";
        let s = parse_session(Path::new("/usr/share/wayland-sessions/mide.desktop"), text, true).unwrap();
        assert_eq!(s.id, "mide");
        assert_eq!(s.exec, ["mide-session", "--x", "a b"]);
        assert!(s.is_mide());
        let (cmd, env) = s.command();
        assert_eq!(cmd, ["mide-session", "--x", "a b"]);
        assert!(env.contains(&"XDG_SESSION_TYPE=wayland".to_string()));
        assert!(env.contains(&"XDG_CURRENT_DESKTOP=mide".to_string()));
        let x = parse_session(Path::new("/usr/share/xsessions/cinnamon.desktop"), "[Desktop Entry]\nName=Cinnamon\nExec=cinnamon-session-cinnamon\n", false).unwrap();
        assert_eq!(x.command().0, ["startx", "/usr/bin/env", "cinnamon-session-cinnamon"]);
        assert!(parse_session(Path::new("h.desktop"), "[Desktop Entry]\nName=H\nExec=h\nHidden=true\n", true).is_none());
        assert!(parse_session(Path::new("t.desktop"), "[Desktop Entry]\nName=T\nExec=t\nTryExec=/nao/existe/mide\n", true).is_none());
    }

    #[test]
    fn memoria_da_ultima_escolha() {
        let mut m = Memory::parse("last_user=ana\nsession.ana=mide\nsession.bruno=niri\n");
        assert_eq!(m.last_user.as_deref(), Some("ana"));
        assert_eq!(m.session_of("bruno"), Some("niri"));
        m.sessions.retain(|(u, _)| u != "ana");
        m.sessions.push(("ana".into(), "cinnamon".into()));
        assert_eq!(Memory::parse(&m.text()), m);
    }
}
