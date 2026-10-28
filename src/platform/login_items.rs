//! Itens de início de sessão (Configurações → Itens de início): os apps e comandos que
//! abrem quando a sessão do mide começa, no formato freedesktop do autostart
//! (`~/.config/autostart/*.desktop`, que o mide e os outros desktops leem).
//!
//! As entradas do sistema (`/etc/xdg/autostart`) não são mexidas: desligar ou remover
//! uma delas grava no usuário um arquivo com o mesmo nome que a desliga (`Hidden=true`),
//! como manda a especificação.

use std::path::{Path, PathBuf};

/// Um item de início.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginItem {
    /// Nome do arquivo `.desktop` (a identidade do item nas pastas).
    pub file: String,
    pub name: String,
    pub exec: String,
    pub icon: Option<String>,
    pub enabled: bool,
    /// Vem do sistema (a remoção só desliga).
    pub system: bool,
}

/// Pasta de autostart do usuário.
pub fn user_dir() -> Option<PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(config.join("autostart"))
}

/// Pastas de autostart do sistema.
fn system_dirs() -> Vec<PathBuf> {
    let system = std::env::var("XDG_CONFIG_DIRS").ok().filter(|v| !v.is_empty()).unwrap_or_else(|| "/etc/xdg".into());
    system.split(':').filter(|d| !d.is_empty()).map(|d| PathBuf::from(d).join("autostart")).collect()
}

/// As chaves de `[Desktop Entry]`.
fn keys(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut inside = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            inside = line == "[Desktop Entry]";
            continue;
        }
        if let (true, Some((k, v))) = (inside, line.split_once('=')) {
            out.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    out
}

fn get<'a>(keys: &'a [(String, String)], key: &str) -> Option<&'a str> {
    keys.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

/// O nome no idioma da sessão (`Name[pt_BR]`, `Name[pt]`) ou o `Name`.
fn localized_name(keys: &[(String, String)]) -> Option<String> {
    let lang = std::env::var("LANG").unwrap_or_default();
    let full = lang.split('.').next().unwrap_or("").to_string();
    let short = full.split('_').next().unwrap_or("").to_string();
    [format!("Name[{full}]"), format!("Name[{short}]"), "Name".to_string()].iter().find_map(|k| get(keys, k).map(str::to_string))
}

/// Liga o item para o mide? (Desligado por `Hidden`, `X-GNOME-Autostart-enabled=false`
/// ou por não valer no mide.)
fn enabled(keys: &[(String, String)]) -> bool {
    let yes = |k: &str| get(keys, k).is_some_and(|v| v.eq_ignore_ascii_case("true"));
    let no = |k: &str| get(keys, k).is_some_and(|v| v.eq_ignore_ascii_case("false"));
    let list = |k: &str| get(keys, k).map(|v| v.split(';').filter(|s| !s.is_empty()).map(|s| s.to_lowercase()).collect::<Vec<_>>());
    !yes("Hidden") && !no("X-GNOME-Autostart-enabled") && list("OnlyShowIn").is_none_or(|l| l.iter().any(|d| d == "mide")) && list("NotShowIn").is_none_or(|l| !l.iter().any(|d| d == "mide"))
}

/// Os itens das pastas (o do usuário vale sobre o do sistema com o mesmo nome), por nome.
pub fn list() -> Vec<LoginItem> {
    list_in(user_dir().as_deref(), &system_dirs())
}

fn list_in(user: Option<&Path>, system: &[PathBuf]) -> Vec<LoginItem> {
    let mut found: Vec<(String, PathBuf, bool)> = Vec::new();
    let add_dir = |dir: &Path, is_system: bool, found: &mut Vec<(String, PathBuf, bool)>| {
        let Ok(read) = std::fs::read_dir(dir) else {
            return;
        };
        let mut files: Vec<PathBuf> = read.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "desktop")).collect();
        files.sort();
        for path in files {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            match found.iter_mut().find(|(n, _, _)| *n == name) {
                // Já havia o do usuário: o do sistema só marca que existe lá também.
                Some(entry) => entry.2 |= is_system,
                None => found.push((name, path, is_system)),
            }
        }
    };
    if let Some(u) = user {
        add_dir(u, false, &mut found);
    }
    for d in system {
        add_dir(d, true, &mut found);
    }
    let mut out: Vec<LoginItem> = found
        .into_iter()
        .filter_map(|(file, path, system)| {
            let text = std::fs::read_to_string(&path).ok()?;
            let k = keys(&text);
            if get(&k, "Type").is_some_and(|t| t != "Application") {
                return None;
            }
            // Só do sistema e escondido de propósito no usuário: aparece desligado.
            let exec = get(&k, "Exec").map(crate::apps::clean_exec).unwrap_or_default();
            let name = localized_name(&k).unwrap_or_else(|| file.trim_end_matches(".desktop").to_string());
            Some(LoginItem { file, name, exec, icon: get(&k, "Icon").map(str::to_string), enabled: enabled(&k), system })
        })
        // Itens que não valem no mide nem foram tocados pelo usuário (de outro desktop:
        // OnlyShowIn=GNOME) ficam de fora da lista.
        .filter(|i| i.enabled || !i.system || user.is_some_and(|u| u.join(&i.file).exists()))
        .filter(|i| !i.exec.is_empty() || !i.enabled)
        .collect();
    out.sort_by_key(|i| crate::apps::fold(&i.name));
    out
}

/// Texto de uma entrada com as chaves trocadas (as outras ficam como estão).
fn with_keys(text: &str, set: &[(&str, &str)]) -> String {
    let mut out = String::new();
    let mut inside = false;
    let mut done: Vec<&str> = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            if inside {
                for (k, v) in set.iter().filter(|(k, _)| !done.contains(k)) {
                    out.push_str(&format!("{k}={v}\n"));
                }
                done.extend(set.iter().map(|(k, _)| *k));
            }
            inside = t == "[Desktop Entry]";
        } else if inside {
            if let Some((k, _)) = t.split_once('=') {
                if let Some((key, v)) = set.iter().find(|(key, _)| *key == k.trim()) {
                    out.push_str(&format!("{key}={v}\n"));
                    done.push(key);
                    continue;
                }
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    if inside {
        for (k, v) in set.iter().filter(|(k, _)| !done.contains(k)) {
            out.push_str(&format!("{k}={v}\n"));
        }
    }
    out
}

fn find_source(file: &str) -> Option<PathBuf> {
    user_dir().map(|u| u.join(file)).filter(|p| p.exists()).or_else(|| system_dirs().into_iter().map(|d| d.join(file)).find(|p| p.exists()))
}

/// Liga ou desliga um item (grava a versão do usuário).
pub fn set_enabled(file: &str, on: bool) -> std::io::Result<()> {
    let user = user_dir().ok_or_else(|| std::io::Error::other(crate::tr!("no configuration folder").into_owned()))?;
    let source = find_source(file).ok_or_else(|| std::io::Error::other(crate::tr!("item not found").into_owned()))?;
    let text = std::fs::read_to_string(&source)?;
    let value = if on { "true" } else { "false" };
    let hidden = if on { "false" } else { "true" };
    std::fs::create_dir_all(&user)?;
    let out = with_keys(&text, &[("X-GNOME-Autostart-enabled", value), ("Hidden", hidden)]);
    crate::files::write_atomic(&user.join(file), out.as_bytes())
}

/// Adiciona um app instalado (o `.desktop` dele) aos itens de início.
pub fn add_app(app: &crate::apps::App) -> std::io::Result<String> {
    let mut text = format!("[Desktop Entry]\nType=Application\nName={}\nExec={}\n", app.name, app.command);
    if let Some(icon) = &app.icon {
        text.push_str(&format!("Icon={icon}\n"));
    }
    if app.terminal {
        text.push_str("Terminal=true\n");
    }
    text.push_str("X-GNOME-Autostart-enabled=true\n");
    write_new(&format!("{}.desktop", app.id), &text)
}

/// Adiciona um comando (programa ou script) com um nome.
pub fn add_command(name: &str, command: &str) -> std::io::Result<String> {
    let slug: String = crate::apps::fold(name).chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect::<String>().trim_matches('-').to_string();
    let slug = if slug.is_empty() { "item".to_string() } else { slug };
    let text = format!("[Desktop Entry]\nType=Application\nName={name}\nExec={command}\nX-GNOME-Autostart-enabled=true\nX-Mide-Command=true\n");
    write_new(&format!("mide-{slug}.desktop"), &text)
}

fn write_new(file: &str, text: &str) -> std::io::Result<String> {
    let user = user_dir().ok_or_else(|| std::io::Error::other(crate::tr!("no configuration folder").into_owned()))?;
    std::fs::create_dir_all(&user)?;
    // Nome livre (dois comandos com o mesmo nome não se sobrescrevem).
    let stem = file.trim_end_matches(".desktop");
    let mut name = file.to_string();
    let mut n = 2;
    while user.join(&name).exists() {
        name = format!("{stem}-{n}.desktop");
        n += 1;
    }
    crate::files::write_atomic(&user.join(&name), text.as_bytes())?;
    Ok(name)
}

/// Remove o item: o do usuário é apagado; o do sistema fica desligado no usuário.
pub fn remove(file: &str) -> std::io::Result<()> {
    let user = user_dir().ok_or_else(|| std::io::Error::other(crate::tr!("no configuration folder").into_owned()))?;
    let in_system = system_dirs().iter().any(|d| d.join(file).exists());
    if in_system {
        return set_enabled(file, false);
    }
    match std::fs::remove_file(user.join(file)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lista_liga_e_desliga() {
        let base = std::env::temp_dir().join(format!("mide-login-{}", std::process::id()));
        let (user, sys) = (base.join("user"), base.join("sys"));
        std::fs::create_dir_all(&user).unwrap();
        std::fs::create_dir_all(&sys).unwrap();
        std::fs::write(sys.join("nm-applet.desktop"), "[Desktop Entry]\nType=Application\nName=Rede\nExec=nm-applet\n").unwrap();
        std::fs::write(sys.join("gnome-only.desktop"), "[Desktop Entry]\nType=Application\nName=Só GNOME\nExec=x\nOnlyShowIn=GNOME;\n").unwrap();
        std::fs::write(user.join("meu.desktop"), "[Desktop Entry]\nType=Application\nName=Meu script\nExec=/home/u/script.sh %u\n").unwrap();
        let items = list_in(Some(&user), std::slice::from_ref(&sys));
        let names: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, ["Meu script", "Rede"]);
        assert_eq!(items[0].exec, "/home/u/script.sh");
        assert!(items.iter().all(|i| i.enabled));
        assert!(items[1].system);
        // Desligar o do sistema: cópia no usuário com Hidden=true e o item desligado.
        let text = std::fs::read_to_string(sys.join("nm-applet.desktop")).unwrap();
        let off = with_keys(&text, &[("X-GNOME-Autostart-enabled", "false"), ("Hidden", "true")]);
        std::fs::write(user.join("nm-applet.desktop"), off).unwrap();
        let items = list_in(Some(&user), std::slice::from_ref(&sys));
        let rede = items.iter().find(|i| i.name == "Rede").unwrap();
        assert!(!rede.enabled && rede.system);
        // A troca de chaves mantém as outras e não duplica.
        let t = with_keys("[Desktop Entry]\nName=A\nHidden=false\n", &[("Hidden", "true")]);
        assert_eq!(t.matches("Hidden=").count(), 1);
        assert!(t.contains("Name=A") && t.contains("Hidden=true"));
        let _ = std::fs::remove_dir_all(&base);
    }
}
