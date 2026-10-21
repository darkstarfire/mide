//! Claro ou escuro na sessão do mide (Configurações → Aparência).
//!
//! Os apps do mide-ui seguem o `[appearance] theme` do `config.toml` na hora
//! (`kit::Theme::system`). Os apps GTK e Qt (que segue o GTK) leem do dconf: a sessão
//! do mide tem um perfil próprio (`mide-session`), com o banco do usuário por cima e um
//! banco só de leitura do mide que trava algumas chaves só nesta sessão. Aqui o mide
//! escreve nesse banco o esquema de cores e o tema do GTK; outras sessões (Cinnamon,
//! GNOME) não usam o perfil e não mudam. Apps já abertos não releem o banco: valem os
//! que abrirem depois (o portal é reiniciado para os apps GTK 4).

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

/// Pasta do perfil do dconf do mide, se a sessão usa o perfil (`DCONF_PROFILE` aponta
/// para `.../mide/dconf/profile`). Fora da sessão (testes, outra sessão), nada muda.
fn session_dconf() -> Option<PathBuf> {
    let profile = PathBuf::from(std::env::var_os("DCONF_PROFILE")?);
    let dir = profile.parent()?;
    (profile.file_name()? == "profile" && dir.file_name()? == "dconf" && dir.join("keyfiles").is_dir()).then(|| dir.to_path_buf())
}

/// Chaves do dconf para o tema (texto do keyfile e a lista de travas).
fn keyfile(dark: bool) -> (String, String) {
    let (scheme, gtk) = if dark { ("prefer-dark", "Adwaita-dark") } else { ("default", "Adwaita") };
    (
        format!("[org/gnome/desktop/interface]\ncolor-scheme='{scheme}'\ngtk-theme='{gtk}'\n"),
        "/org/gnome/desktop/interface/color-scheme\n/org/gnome/desktop/interface/gtk-theme\n".to_string(),
    )
}

/// Escreve o tema no banco da sessão e recompila. Devolve se mudou algo.
fn write_db(dir: &Path, dark: bool) -> Result<bool, String> {
    let (keys, locks) = keyfile(dark);
    let file = dir.join("keyfiles").join("01-aparencia");
    if fs::read_to_string(&file).is_ok_and(|old| old == keys) && dir.join("mide.db").exists() {
        return Ok(false);
    }
    fs::create_dir_all(dir.join("keyfiles").join("locks")).map_err(|e| e.to_string())?;
    fs::write(&file, keys).map_err(|e| e.to_string())?;
    fs::write(dir.join("keyfiles").join("locks").join("01-aparencia"), locks).map_err(|e| e.to_string())?;
    // Compila ao lado e troca de uma vez: quem abrir o banco no meio vê o antigo inteiro.
    let tmp = dir.join("mide.db.novo");
    let status = Command::new("dconf").arg("compile").arg(&tmp).arg(dir.join("keyfiles")).status().map_err(|e| format!("dconf: {e}"))?;
    if !status.success() {
        return Err(format!("dconf compile saiu com {status}"));
    }
    fs::rename(&tmp, dir.join("mide.db")).map_err(|e| e.to_string())?;
    Ok(true)
}

/// Aplica o tema da configuração aos apps GTK e Qt da sessão (no começo da sessão e
/// quando as Configurações trocam). `restart_portal`: reinicia o portal do GTK, que
/// passa o esquema de cores aos apps GTK 4/libadwaita.
pub fn apply(restart_portal: bool) {
    let Some(dir) = session_dconf() else {
        return;
    };
    let dark = crate::kit::system_dark();
    match write_db(&dir, dark) {
        Ok(true) => {
            tracing::info!("aparência: apps GTK e Qt no tema {}", if dark { "escuro" } else { "claro" });
            if restart_portal {
                let _ = Command::new("systemctl").args(["--user", "--no-block", "try-restart", "xdg-desktop-portal-gtk.service"]).status();
            }
        }
        Ok(false) => {}
        Err(err) => tracing::warn!("aparência: tema dos apps GTK não aplicado: {err}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn banco_do_tema_escuro() {
        let dir = std::env::temp_dir().join(format!("mide-aparencia-{}", std::process::id()));
        fs::create_dir_all(dir.join("keyfiles")).unwrap();
        let has_dconf = Command::new("dconf").arg("help").output().is_ok();
        if has_dconf {
            assert!(write_db(&dir, true).unwrap());
            assert!(fs::read_to_string(dir.join("keyfiles/01-aparencia")).unwrap().contains("prefer-dark"));
            assert!(dir.join("mide.db").exists());
            // O mesmo tema de novo não recompila.
            assert!(!write_db(&dir, true).unwrap());
            assert!(write_db(&dir, false).unwrap());
        }
        let _ = fs::remove_dir_all(&dir);
    }
}
