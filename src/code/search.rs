//! Buscar e substituir no projeto inteiro (`mide-code`, Ctrl+Shift+F): percorre os
//! arquivos de texto da pasta (os mesmos do "Ir para arquivo", sem pastas geradas) e
//! devolve as linhas com a busca; substituir grava cada arquivo de forma segura.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use super::{edit, project};

/// Arquivos maiores que isso ficam fora da busca.
const MAX_FILE: u64 = 4 * 1024 * 1024;

/// Uma linha encontrada.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    /// Caminho relativo à raiz do projeto.
    pub path: PathBuf,
    /// Linha e coluna (de 1; coluna em caracteres).
    pub line: usize,
    pub col: usize,
    /// O texto da linha (sem o recuo, cortado em 200 caracteres).
    pub text: String,
}

/// Texto do arquivo, se for texto UTF-8 de tamanho razoável.
fn read(path: &Path) -> Option<String> {
    if std::fs::metadata(path).ok()?.len() > MAX_FILE {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    if bytes.contains(&0) {
        return None;
    }
    String::from_utf8(bytes).ok()
}

/// Linhas de `text` com a busca (no máximo `limit`).
pub fn hits_in(rel: &Path, text: &str, query: &str, case: bool, limit: usize) -> Vec<Hit> {
    let mut out = Vec::new();
    if query.is_empty() {
        return out;
    }
    let needle = if case { query.to_string() } else { query.to_lowercase() };
    for (n, line) in text.lines().enumerate() {
        let hay = if case { line.to_string() } else { line.to_lowercase() };
        if let Some(i) = hay.find(&needle) {
            let col = hay[..i].chars().count() + 1;
            out.push(Hit { path: rel.to_path_buf(), line: n + 1, col, text: line.trim().chars().take(200).collect() });
            if out.len() >= limit {
                break;
            }
        }
    }
    out
}

/// Busca em todos os arquivos do projeto (até `limit` linhas no total).
pub fn search(root: &Path, query: &str, case: bool, limit: usize) -> Vec<Hit> {
    let mut out = Vec::new();
    if query.is_empty() {
        return out;
    }
    for rel in project::files(root, 20_000) {
        let Some(text) = read(&root.join(&rel)) else {
            continue;
        };
        out.extend(hits_in(&rel, &text, query, case, limit - out.len()));
        if out.len() >= limit {
            break;
        }
    }
    out
}

/// Substitui em todos os arquivos do projeto, menos os de `skip` (caminhos completos:
/// os abertos no editor, que o app troca no texto aberto). Devolve (arquivos,
/// substituições).
pub fn replace(root: &Path, query: &str, with: &str, case: bool, skip: &HashSet<PathBuf>) -> std::io::Result<(usize, usize)> {
    let (mut files, mut total) = (0, 0);
    for rel in project::files(root, 20_000) {
        let path = root.join(&rel);
        if skip.contains(&path) {
            continue;
        }
        let Some(mut text) = read(&path) else {
            continue;
        };
        let n = edit::replace_all(&mut text, query, with, case);
        if n > 0 {
            crate::files::write_atomic(&path, text.as_bytes())?;
            files += 1;
            total += n;
        }
    }
    Ok((files, total))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn busca_e_substitui_no_projeto() {
        let dir = std::env::temp_dir().join(format!("mide-busca-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::create_dir_all(dir.join("target")).unwrap();
        std::fs::write(dir.join("src/a.rs"), "fn alvo() {}\n  // Alvo de novo\n").unwrap();
        std::fs::write(dir.join("b.txt"), "nada aqui\n").unwrap();
        std::fs::write(dir.join("target/c.rs"), "alvo gerado\n").unwrap();
        std::fs::write(dir.join("bin.dat"), [0u8, 1, 2, b'a', b'l', b'v', b'o']).unwrap();
        let hits = search(&dir, "alvo", false, 100);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0], Hit { path: PathBuf::from("src/a.rs"), line: 1, col: 4, text: "fn alvo() {}".into() });
        assert_eq!(hits[1].line, 2);
        assert_eq!(hits[1].text, "// Alvo de novo");
        assert_eq!(search(&dir, "alvo", true, 100).len(), 1);
        assert_eq!(search(&dir, "alvo", false, 1).len(), 1);
        // Substituir, pulando um arquivo aberto.
        let skip: HashSet<PathBuf> = [dir.join("b.txt")].into_iter().collect();
        assert_eq!(replace(&dir, "alvo", "meta", false, &skip).unwrap(), (1, 2));
        assert_eq!(std::fs::read_to_string(dir.join("src/a.rs")).unwrap(), "fn meta() {}\n  // meta de novo\n");
        assert_eq!(std::fs::read_to_string(dir.join("target/c.rs")).unwrap(), "alvo gerado\n");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
