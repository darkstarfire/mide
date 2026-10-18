//! Regras do gerenciador de arquivos (sem interface): listar pastas, ordenar,
//! tamanhos e datas em português, criar pasta, renomear e tipos de arquivo.

use std::{
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

/// Um item de uma pasta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub size: u64,
    pub modified: SystemTime,
}

impl Entry {
    /// Nome do ícone do tema para o item (pelo tipo).
    pub fn icon_name(&self) -> &'static str {
        crate::places::icon_name_for(&crate::places::Item {
            path: self.path.clone(),
            name: self.name.clone(),
            is_dir: self.is_dir,
            modified: self.modified,
        })
    }

    /// É uma imagem que dá para mostrar em miniatura?
    pub fn is_image(&self) -> bool {
        !self.is_dir && crate::places::is_image(&self.path)
    }
}

/// Itens de `dir`: pastas primeiro, depois arquivos, em ordem alfabética (sem
/// diferenciar maiúsculas nem acentos). Ocultos (com ponto na frente) só com `hidden`.
pub fn list_dir(dir: &Path, hidden: bool) -> std::io::Result<Vec<Entry>> {
    let mut list: Vec<Entry> = fs::read_dir(dir)?
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if !hidden && name.starts_with('.') {
                return None;
            }
            let meta = e.metadata().ok()?;
            Some(Entry {
                is_dir: meta.is_dir(),
                size: if meta.is_dir() { 0 } else { meta.len() },
                modified: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                path: e.path(),
                name,
            })
        })
        .collect();
    list.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| crate::apps::fold(&a.name).cmp(&crate::apps::fold(&b.name))));
    Ok(list)
}

/// Itens cujo nome contém o texto (sem diferenciar maiúsculas nem acentos).
pub fn filter(entries: &[Entry], query: &str) -> Vec<Entry> {
    let q = crate::apps::fold(query.trim());
    if q.is_empty() {
        return entries.to_vec();
    }
    entries.iter().filter(|e| crate::apps::fold(&e.name).contains(&q)).cloned().collect()
}

/// Tamanho legível: "512 bytes", "3,2 KB", "14,8 MB", "1,1 GB".
pub fn size_text(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1000 {
        return if bytes == 1 { "1 byte".into() } else { format!("{bytes} bytes") };
    }
    let mut value = bytes as f64 / 1000.0;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    format!("{:.1} {}", value, UNITS[unit]).replace('.', ",")
}

/// Data de modificação: "hoje, 14:30", "ontem, 09:05", "12 de out." ou "12 de out. de 2024".
pub fn date_text(modified: SystemTime, now: SystemTime) -> String {
    use chrono::{DateTime, Datelike, Local};
    const MONTHS: [&str; 12] = ["jan.", "fev.", "mar.", "abr.", "mai.", "jun.", "jul.", "ago.", "set.", "out.", "nov.", "dez."];
    let when: DateTime<Local> = modified.into();
    let today: DateTime<Local> = now.into();
    let days = today.date_naive().signed_duration_since(when.date_naive()).num_days();
    match days {
        0 => format!("hoje, {}", when.format("%H:%M")),
        1 => format!("ontem, {}", when.format("%H:%M")),
        _ if when.year() == today.year() => format!("{} de {}", when.day(), MONTHS[when.month0() as usize]),
        _ => format!("{} de {} de {}", when.day(), MONTHS[when.month0() as usize], when.year()),
    }
}

/// Nome livre para uma pasta nova: "Nova pasta", "Nova pasta 2"...
pub fn new_folder_name(dir: &Path) -> String {
    let mut name = "Nova pasta".to_string();
    let mut n = 2;
    while dir.join(&name).exists() {
        name = format!("Nova pasta {n}");
        n += 1;
    }
    name
}

/// Cria uma pasta nova em `dir` e devolve o caminho dela.
pub fn create_folder(dir: &Path) -> std::io::Result<PathBuf> {
    let path = dir.join(new_folder_name(dir));
    fs::create_dir(&path)?;
    Ok(path)
}

/// Confere um nome novo: não vazio, sem "/" e sem ser "." ou "..".
pub fn valid_name(name: &str) -> Result<&str, &'static str> {
    let name = name.trim();
    if name.is_empty() {
        return Err("o nome não pode ficar vazio");
    }
    if name.contains('/') || name.contains('\0') {
        return Err("o nome não pode ter \"/\"");
    }
    if name == "." || name == ".." {
        return Err("esse nome não é permitido");
    }
    Ok(name)
}

/// Renomeia sem sobrescrever: falha se já existir algo com o nome novo.
pub fn rename(path: &Path, new_name: &str) -> Result<PathBuf, String> {
    let name = valid_name(new_name)?;
    let target = path.with_file_name(name);
    if target == path {
        return Ok(target);
    }
    if target.exists() {
        return Err(format!("já existe \"{name}\" nesta pasta"));
    }
    fs::rename(path, &target).map_err(|e| e.to_string())?;
    Ok(target)
}

/// Pastas favoritas da barra lateral: nome, caminho e ícone do tema.
pub fn favorites() -> Vec<(String, PathBuf, &'static str)> {
    let home = crate::places::home_dir();
    let user_dir = |key: &str, fallback: &str| -> PathBuf {
        let cfg = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".config"));
        fs::read_to_string(cfg.join("user-dirs.dirs"))
            .ok()
            .and_then(|t| crate::places::parse_user_dir(&t, key, &home))
            .unwrap_or_else(|| home.join(fallback))
    };
    let mut list = vec![("Pasta pessoal".to_string(), home.clone(), "user-home")];
    for (key, fallback, label, icon) in [
        ("DESKTOP", "Área de trabalho", "Área de trabalho", "user-desktop"),
        ("DOCUMENTS", "Documentos", "Documentos", "folder-documents"),
        ("DOWNLOAD", "Downloads", "Downloads", "folder-download"),
        ("PICTURES", "Imagens", "Imagens", "folder-pictures"),
        ("MUSIC", "Músicas", "Músicas", "folder-music"),
        ("VIDEOS", "Vídeos", "Vídeos", "folder-videos"),
    ] {
        let path = user_dir(key, fallback);
        if path.is_dir() && path != home {
            list.push((label.to_string(), path, icon));
        }
    }
    list
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mide-files-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn lista_pastas_primeiro_sem_ocultos() {
        let dir = temp("lista");
        fs::create_dir(dir.join("zeta")).unwrap();
        fs::write(dir.join("Água.txt"), "1").unwrap();
        fs::write(dir.join("banana.pdf"), "12").unwrap();
        fs::write(dir.join(".oculto"), "x").unwrap();
        let names: Vec<String> = list_dir(&dir, false).unwrap().into_iter().map(|e| e.name).collect();
        assert_eq!(names, vec!["zeta", "Água.txt", "banana.pdf"]);
        assert_eq!(list_dir(&dir, true).unwrap().len(), 4);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn tamanhos_em_portugues() {
        assert_eq!(size_text(1), "1 byte");
        assert_eq!(size_text(512), "512 bytes");
        assert_eq!(size_text(3_200), "3,2 KB");
        assert_eq!(size_text(14_800_000), "14,8 MB");
        assert_eq!(size_text(1_100_000_000), "1,1 GB");
    }

    #[test]
    fn datas_relativas() {
        let now = SystemTime::now();
        assert!(date_text(now, now).starts_with("hoje, "));
        assert!(date_text(now - Duration::from_secs(86_400), now).starts_with("ontem, "));
        let old = SystemTime::UNIX_EPOCH + Duration::from_secs(1_600_000_000);
        assert!(date_text(old, now).ends_with("de 2020"));
    }

    #[test]
    fn pasta_nova_com_nome_livre_e_renomear() {
        let dir = temp("nova");
        let a = create_folder(&dir).unwrap();
        let b = create_folder(&dir).unwrap();
        assert!(a.ends_with("Nova pasta"));
        assert!(b.ends_with("Nova pasta 2"));
        let renamed = rename(&a, "  Fotos  ").unwrap();
        assert!(renamed.ends_with("Fotos"));
        assert!(rename(&b, "Fotos").is_err());
        assert!(rename(&b, "a/b").is_err());
        assert!(rename(&b, "").is_err());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn busca_ignora_acentos() {
        let e = |n: &str| Entry { name: n.into(), path: n.into(), is_dir: false, size: 0, modified: SystemTime::UNIX_EPOCH };
        let list = vec![e("Relatório.pdf"), e("foto.jpg")];
        assert_eq!(filter(&list, "relatorio").len(), 1);
        assert_eq!(filter(&list, "").len(), 2);
    }
}
