//! Pasta do projeto no `mide-code`: a lista dos arquivos (para o "Ir para arquivo",
//! como o ⌘T do TextMate) e a busca aproximada pelo nome.

use std::path::{Path, PathBuf};

/// Pastas que ficam de fora (geradas, de dependências ou do controle de versão).
const SKIP: &[&str] = &[".git", ".hg", ".svn", "target", "node_modules", "__pycache__", ".venv", "venv", "build", "dist", ".cache", ".idea", ".vscode"];

/// Arquivos do projeto (caminhos relativos à raiz), até `limit`, sem ocultos.
pub fn files(root: &Path, limit: usize) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || SKIP.contains(&name.as_str()) {
                continue;
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if kind.is_dir() {
                stack.push(path);
            } else if let Ok(rel) = path.strip_prefix(root) {
                out.push(rel.to_path_buf());
                if out.len() >= limit {
                    return out;
                }
            }
        }
    }
    out.sort();
    out
}

/// Nota da busca aproximada: as letras da busca aparecem em ordem no caminho (sem
/// diferenciar maiúsculas); vale mais quando caem no nome do arquivo, juntas e no
/// começo das palavras. `None`: não serve.
pub fn score(query: &str, path: &str) -> Option<i64> {
    if query.is_empty() {
        return Some(0);
    }
    let hay: Vec<char> = path.to_lowercase().chars().collect();
    let name_start = path.rfind('/').map_or(0, |i| path[..=i].chars().count());
    let mut score = 0i64;
    let mut at = 0usize;
    let mut last: Option<usize> = None;
    for q in query.to_lowercase().chars().filter(|c| !c.is_whitespace()) {
        let found = (at..hay.len()).find(|&i| hay[i] == q)?;
        score += 10;
        if found >= name_start {
            score += 8;
        }
        if last.is_some_and(|l| l + 1 == found) {
            score += 12;
        }
        if found == 0 || matches!(hay[found - 1], '/' | '_' | '-' | '.' | ' ') {
            score += 9;
        }
        last = Some(found);
        at = found + 1;
    }
    // Caminhos curtos primeiro, entre os que empatam.
    Some(score * 100 - hay.len() as i64)
}

/// Os melhores caminhos para a busca, do melhor para o pior.
pub fn best<'a>(query: &str, paths: &'a [PathBuf], limit: usize) -> Vec<&'a PathBuf> {
    let mut scored: Vec<(i64, &PathBuf)> = paths.iter().filter_map(|p| score(query, &p.to_string_lossy()).map(|s| (s, p))).collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1)));
    scored.into_iter().take(limit).map(|(_, p)| p).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn busca_aproximada() {
        let paths: Vec<PathBuf> = ["src/main.rs", "src/code/lsp.rs", "docs/manual.md", "src/bin/mide-code/main.rs"].iter().map(PathBuf::from).collect();
        assert_eq!(best("lsp", &paths, 5)[0], &PathBuf::from("src/code/lsp.rs"));
        assert_eq!(best("main", &paths, 5)[0], &PathBuf::from("src/main.rs"));
        assert_eq!(best("mcm", &paths, 5)[0], &PathBuf::from("src/bin/mide-code/main.rs"));
        assert!(score("xyz", "src/main.rs").is_none());
    }

    #[test]
    fn lista_sem_pastas_geradas() {
        let dir = std::env::temp_dir().join(format!("mide-projeto-{}", std::process::id()));
        for sub in ["src", "target/debug", ".git", "node_modules/x"] {
            std::fs::create_dir_all(dir.join(sub)).unwrap();
        }
        for f in ["src/a.rs", "target/debug/b", ".git/HEAD", "node_modules/x/c.js", "README.md", ".oculto"] {
            std::fs::write(dir.join(f), "x").unwrap();
        }
        let got = files(&dir, 100);
        assert_eq!(got, vec![PathBuf::from("README.md"), PathBuf::from("src/a.rs")]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
