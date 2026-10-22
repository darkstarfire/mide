//! Arquivos das notas da Calculadora: uma nota por arquivo de texto em
//! `$XDG_DATA_HOME/mide/calculadora/notas`, com o nome pela data de criação
//! ("20261022-153012.txt"). O arquivo guarda só o que foi digitado, em UTF-8 com "\n":
//! os resultados são recalculados ao abrir.

use std::{
    fs, io,
    path::{Path, PathBuf},
    time::SystemTime,
};

/// Pasta das notas.
pub fn notes_dir() -> PathBuf {
    crate::places::data_home().join("mide/calculadora/notas")
}

/// Uma nota na pasta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteFile {
    pub id: String,
    pub path: PathBuf,
    pub title: String,
    pub modified: SystemTime,
}

/// As notas da pasta, a editada por último primeiro (empate: a criada por último).
pub fn list(dir: &Path) -> Vec<NoteFile> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut notes: Vec<NoteFile> = entries
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            let id = path.file_name()?.to_str()?.strip_suffix(".txt")?.to_string();
            if id.is_empty() || id.starts_with('.') {
                return None;
            }
            let meta = e.metadata().ok().filter(|m| m.is_file())?;
            let text = fs::read_to_string(&path).unwrap_or_default();
            Some(NoteFile { id, title: title(&text), modified: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH), path })
        })
        .collect();
    notes.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| b.id.cmp(&a.id)));
    notes
}

/// Nome de uma nota nova, pela hora ("20261022-153012"); se já existe, "-2", "-3"...
pub fn new_id(dir: &Path, now: chrono::DateTime<chrono::Local>) -> String {
    let base = now.format("%Y%m%d-%H%M%S").to_string();
    let free = |id: &str| !dir.join(format!("{id}.txt")).exists();
    if free(&base) {
        return base;
    }
    (2..).map(|n| format!("{base}-{n}")).find(|id| free(id)).unwrap_or(base)
}

/// Título da nota na lista: a primeira linha com texto, sem espaços nas pontas e com
/// até 40 caracteres ("…" se passa); sem texto, "Nota nova".
pub fn title(text: &str) -> String {
    let Some(line) = text.lines().map(str::trim).find(|l| !l.is_empty()) else {
        return "Nota nova".into();
    };
    if line.chars().count() <= 40 {
        return line.to_string();
    }
    let cut: String = line.chars().take(39).collect();
    format!("{}…", cut.trim_end())
}

/// Grava a nota sem risco de perder a anterior (temporário e troca); cria a pasta.
pub fn save(dir: &Path, id: &str, text: &str) -> io::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let path = dir.join(format!("{id}.txt"));
    crate::files::write_atomic(&path, text.as_bytes())?;
    Ok(path)
}

/// Manda a nota para a Lixeira (dá para recuperar pelo Arquivos).
pub fn trash(path: &Path, trash: &Path) -> io::Result<()> {
    crate::places::move_to_trash(path, trash).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mide-notas-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn nome_novo_nao_repete() {
        let dir = temp("nome");
        let now = chrono::Local.with_ymd_and_hms(2026, 10, 22, 15, 30, 12).unwrap();
        let id = new_id(&dir, now);
        assert_eq!(id, "20261022-153012");
        save(&dir, &id, "a").unwrap();
        let id2 = new_id(&dir, now);
        assert_eq!(id2, "20261022-153012-2");
        save(&dir, &id2, "b").unwrap();
        assert_eq!(new_id(&dir, now), "20261022-153012-3");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn titulo() {
        assert_eq!(title(""), "Nota nova");
        assert_eq!(title("\n   \n  Viagem a Salvador  \nhotel = 3 × 180"), "Viagem a Salvador");
        let long = "a".repeat(50);
        assert_eq!(title(&long), format!("{}…", "a".repeat(39)));
        assert_eq!(title(&"é".repeat(40)), "é".repeat(40));
    }

    #[test]
    fn gravar_listar_e_lixeira() {
        let dir = temp("lista");
        let trash_dir = temp("lixeira");
        // Gravação atômica, com acentos; sem sobras do temporário na pasta.
        let a = save(&dir, "20261022-100000", "Orçamento\nluz = 120 =").unwrap();
        assert_eq!(fs::read_to_string(&a).unwrap(), "Orçamento\nluz = 120 =");
        save(&dir, "20261022-110000", "Mercado").unwrap();
        fs::write(dir.join("leia-me.md"), "não é nota").unwrap();
        // A editada por último vem primeiro.
        std::thread::sleep(std::time::Duration::from_millis(20));
        save(&dir, "20261022-100000", "Orçamento novo").unwrap();
        let notes = list(&dir);
        let ids: Vec<&str> = notes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, ["20261022-100000", "20261022-110000"]);
        assert_eq!(notes[0].title, "Orçamento novo");
        assert!(fs::read_dir(&dir).unwrap().flatten().all(|e| !e.file_name().to_string_lossy().starts_with('.')));
        // Lixeira de teste: o arquivo vai para files/ com o .trashinfo em info/.
        trash(&notes[1].path, &trash_dir).unwrap();
        assert_eq!(list(&dir).len(), 1);
        assert!(trash_dir.join("files/20261022-110000.txt").exists());
        assert!(trash_dir.join("info/20261022-110000.txt.trashinfo").exists());
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&trash_dir);
    }
}
