//! Atalhos de apps fora do launcher: os apps fixados na dock (`~/.config/mide/dock.toml`)
//! e os atalhos na área de trabalho (uma cópia do `.desktop` do app na pasta da mesa).
//! O launcher usa para "Adicionar à dock" e "Adicionar à área de trabalho"; a dock lê e
//! grava a lista de fixados (e percebe quando o arquivo muda).

use std::{
    fs,
    path::{Path, PathBuf},
};

/// Apps fixados na dock quando ninguém mexeu nela ainda (como no macOS, as
/// Configurações do Sistema vêm fixadas).
pub const DEFAULT_PINNED: &[&str] = &["mide-terminal", "mide-calculator", "mide-settings"];

#[derive(serde::Deserialize, Default)]
#[serde(default)]
struct DockConfig {
    pinned: Vec<String>,
}

/// Onde fica a lista de fixados (`$XDG_CONFIG_HOME/mide/dock.toml`).
pub fn dock_config_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("mide").join("dock.toml"))
}

/// Ids dos apps fixados, na ordem da dock (sem o arquivo, os padrões).
pub fn pinned_ids() -> Vec<String> {
    let config = dock_config_path()
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|text| toml::from_str::<DockConfig>(&text).ok())
        .unwrap_or_default();
    if config.pinned.is_empty() {
        DEFAULT_PINNED.iter().map(|s| s.to_string()).collect()
    } else {
        config.pinned
    }
}

/// Grava a lista de fixados.
pub fn save_pinned(ids: &[String]) -> std::io::Result<()> {
    let path = dock_config_path().ok_or_else(|| std::io::Error::other("sem pasta de configuração"))?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let list = ids.iter().map(|id| format!("\"{id}\"")).collect::<Vec<_>>().join(", ");
    fs::write(&path, format!("# Apps fixados na dock, na ordem em que aparecem.\npinned = [{list}]\n"))
}

/// Fixa o app `id` no fim dos fixados da dock. `Ok(false)`: já estava.
pub fn pin_to_dock(id: &str) -> std::io::Result<bool> {
    let mut ids = pinned_ids();
    if ids.iter().any(|p| p == id) {
        return Ok(false);
    }
    ids.push(id.to_string());
    save_pinned(&ids)?;
    Ok(true)
}

/// O `.desktop` do app `id` (o do usuário antes dos do sistema), como na
/// especificação: `applications/<id>.desktop` nas pastas de dados, com os `-` do id
/// podendo ser subpastas.
pub fn desktop_file(id: &str) -> Option<PathBuf> {
    let home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")));
    let system = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    let dirs = home.into_iter().chain(system.split(':').filter(|d| !d.is_empty()).map(PathBuf::from));
    for dir in dirs {
        let apps = dir.join("applications");
        let plain = apps.join(format!("{id}.desktop"));
        if plain.is_file() {
            return Some(plain);
        }
        let nested = apps.join(format!("{}.desktop", id.replace('-', "/")));
        if nested.is_file() {
            return Some(nested);
        }
    }
    None
}

/// Cria na área de trabalho o atalho do app (cópia do `.desktop`) já pronto para só
/// executar: leitura e escrita para todos, executável e confiado (no registro dos
/// programas confiáveis), para a mesa mostrar o nome e o ícone do app e abrir sem
/// perguntar. Devolve o caminho do atalho; se já existe, acerta as permissões e a
/// confiança dele (atalhos criados antes ficavam com o nome do arquivo).
pub fn add_to_desktop(id: &str) -> std::io::Result<PathBuf> {
    let source = desktop_file(id).ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, format!("{id}.desktop não encontrado")))?;
    let dir = crate::places::desktop_dir().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "sem pasta da área de trabalho"))?;
    add_to_folder(&source, id, &dir, &crate::programs::Dirs::current())
}

/// Permissões do atalho: `rwxrwxrwx`.
const SHORTCUT_MODE: u32 = 0o777;

fn add_to_folder(source: &Path, id: &str, dir: &Path, dirs: &crate::programs::Dirs) -> std::io::Result<PathBuf> {
    let target = dir.join(format!("{id}.desktop"));
    if !target.exists() {
        fs::copy(source, &target)?;
    }
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&target, fs::Permissions::from_mode(SHORTCUT_MODE))?;
    // Por último: a confiança guarda o tamanho e a data do arquivo como estão agora.
    crate::programs::trust(dirs, &target)?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atalho_na_pasta() {
        let dir = std::env::temp_dir().join(format!("mide-atalho-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let source = dir.join("fonte.desktop");
        fs::write(&source, "[Desktop Entry]\nName=Teste\nExec=true\n").unwrap();
        let mesa = dir.join("mesa");
        fs::create_dir_all(&mesa).unwrap();
        let dirs = crate::programs::Dirs { data: dir.join("data"), downloads: dir.join("Downloads"), home: dir.clone() };
        let made = add_to_folder(&source, "teste", &mesa, &dirs).unwrap();
        assert_eq!(made, mesa.join("teste.desktop"));
        assert!(fs::read_to_string(&made).unwrap().contains("Name=Teste"));
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(&made).unwrap().permissions().mode() & 0o777, 0o777);
        // Confiado: a mesa mostra o nome e o ícone do app, sem perguntar ao abrir.
        assert!(crate::programs::is_trusted(&dirs, &made));
        // Atalho de antes (sem confiança, 0644): de novo acerta, sem copiar por cima.
        fs::set_permissions(&made, fs::Permissions::from_mode(0o644)).unwrap();
        fs::write(&made, "[Desktop Entry]\nName=Mexido\nExec=true\n").unwrap();
        assert_eq!(add_to_folder(&source, "teste", &mesa, &dirs).unwrap(), made);
        assert!(fs::read_to_string(&made).unwrap().contains("Name=Mexido"));
        assert_eq!(fs::metadata(&made).unwrap().permissions().mode() & 0o777, 0o777);
        assert!(crate::programs::is_trusted(&dirs, &made));
        fs::remove_dir_all(&dir).unwrap();
    }
}
