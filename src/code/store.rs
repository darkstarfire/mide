//! Loja de extensões do Code: busca, instala, atualiza e remove extensões do registro
//! público das extensões do Zed (o mesmo de onde o Zed baixa as extensões já
//! compiladas, `api.zed.dev`). Só entram as que usam a interface que o mide
//! implementa (`zed_extension_api` até a 0.7). Instaladas em
//! `~/.local/share/mide/code/extensions/<id>/`; as do próprio Zed não são tocadas.

use std::path::{Path, PathBuf};

use serde::Deserialize;

const API: &str = "https://api.zed.dev/extensions";
/// Maior versão da interface de extensões que o mide roda (`wasm_ext`).
pub const MAX_WASM_API: &str = "0.7.0";

/// Uma extensão do registro.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Listing {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub authors: Vec<String>,
    pub repository: String,
    pub wasm_api_version: Option<String>,
    pub provides: Vec<String>,
    pub download_count: u64,
}

impl Listing {
    /// O mide roda esta extensão (sem código, ou com a interface que ele tem).
    pub fn compatible(&self) -> bool {
        self.wasm_api_version.as_deref().is_none_or(|v| newer(MAX_WASM_API, v) || v == MAX_WASM_API)
    }
}

/// `a` é uma versão mais nova que `b` ("0.10.2" > "0.9.9").
pub fn newer(a: &str, b: &str) -> bool {
    let parts = |v: &str| v.trim_start_matches('v').split(['.', '-']).map(|p| p.parse::<u64>().unwrap_or(0)).collect::<Vec<_>>();
    let (x, y) = (parts(a), parts(b));
    for k in 0..x.len().max(y.len()) {
        let (p, q) = (x.get(k).copied().unwrap_or(0), y.get(k).copied().unwrap_or(0));
        if p != q {
            return p > q;
        }
    }
    false
}

#[derive(Deserialize)]
struct Page {
    data: Vec<Listing>,
}

fn curl(args: &[&str]) -> Result<Vec<u8>, String> {
    let out = std::process::Command::new("curl").args(["-sSfL", "--max-time", "60"]).args(args).output().map_err(|e| format!("curl: {e}"))?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// Busca no registro (vazio: as mais baixadas).
pub fn search(query: &str) -> Result<Vec<Listing>, String> {
    let q: String = query.chars().map(|c| if c.is_alphanumeric() || c == '-' { c } else { '+' }).collect();
    let url = format!("{API}?filter={q}&max_schema_version=1");
    let body = curl(&[&url])?;
    let mut list = serde_json::from_slice::<Page>(&body).map_err(|e| e.to_string())?.data;
    list.sort_by_key(|l| std::cmp::Reverse(l.download_count));
    Ok(list)
}

/// Pasta de uma extensão instalada pelo mide.
pub fn install_dir(id: &str) -> Option<PathBuf> {
    Some(crate::code::extensions::root()?.join(id))
}

/// Baixa e instala (ou atualiza) a extensão `id`. Devolve a pasta.
pub fn install(id: &str) -> Result<PathBuf, String> {
    if id.is_empty() || id.contains(['/', '.', ' ']) {
        return Err("id inválido".into());
    }
    let dest = install_dir(id).ok_or("sem pasta de dados")?;
    let parent = dest.parent().ok_or("sem pasta de dados")?.to_path_buf();
    std::fs::create_dir_all(&parent).map_err(|e| e.to_string())?;
    let tmp = parent.join(format!(".{id}.download"));
    let archive = parent.join(format!(".{id}.tar.gz"));
    let _ = std::fs::remove_dir_all(&tmp);
    let url = format!("{API}/{id}/download?min_schema_version=0&max_schema_version=1&min_wasm_api_version=0.0.0&max_wasm_api_version={MAX_WASM_API}");
    curl(&["-o", &archive.to_string_lossy(), &url])?;
    std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    let out = std::process::Command::new("tar").args(["-xzf", &archive.to_string_lossy(), "-C", &tmp.to_string_lossy()]).output().map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&archive);
    if !out.status.success() || !tmp.join("extension.toml").exists() {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(format!("pacote da extensão {id} inválido"));
    }
    // Troca a versão antiga pela nova (a pasta de trabalho, com os servidores
    // baixados, fica).
    let _ = std::fs::remove_dir_all(&dest);
    std::fs::rename(&tmp, &dest).map_err(|e| e.to_string())?;
    tracing::info!("loja: extensão {id} instalada em {}", dest.display());
    Ok(dest)
}

/// Remove uma extensão instalada pelo mide (as do Zed não).
pub fn uninstall(id: &str) -> Result<(), String> {
    let dir = install_dir(id).ok_or("sem pasta de dados")?;
    if !dir.join("extension.toml").exists() {
        return Err(format!("{id} não foi instalada pelo mide"));
    }
    std::fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
    if let Some(work) = crate::code::wasm_ext::work_dir(id) {
        let _ = std::fs::remove_dir_all(work);
    }
    Ok(())
}

/// Versão instalada de uma extensão e se é a do Zed (só leitura).
pub fn installed_version(id: &str) -> Option<(String, bool)> {
    crate::code::extensions::installed().into_iter().find(|e| e.manifest.id == id).map(|e| (e.manifest.version, e.from_zed))
}

/// A extensão está na pasta do mide?
pub fn is_mide_install(dir: &Path) -> bool {
    crate::code::extensions::root().is_some_and(|r| dir.starts_with(r))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versoes_e_compatibilidade() {
        assert!(newer("0.10.2", "0.9.9"));
        assert!(!newer("0.7.0", "0.7.0"));
        assert!(newer("1.0", "0.99.1"));
        let mut l = Listing { wasm_api_version: Some("0.7.0".into()), ..Default::default() };
        assert!(l.compatible());
        l.wasm_api_version = Some("0.8.0".into());
        assert!(!l.compatible());
        l.wasm_api_version = None;
        assert!(l.compatible());
    }
}
