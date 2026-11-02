//! Git na margem do editor: compara o texto com a versão do último commit (`HEAD`)
//! e diz quais linhas são novas, mudadas ou têm linhas apagadas antes delas.

use std::path::Path;

/// O que mudou numa linha do texto atual.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineChange {
    Added,
    Modified,
    /// Linhas apagadas logo antes desta.
    DeletedAbove,
}

/// O arquivo como está no último commit (`None`: fora de um repositório, novo ou
/// sem o `git`).
pub fn head_text(path: &Path) -> Option<String> {
    let dir = path.parent()?;
    let name = path.file_name()?.to_string_lossy().into_owned();
    let out = std::process::Command::new("git").arg("-C").arg(dir).args(["show", &format!("HEAD:./{name}")]).stderr(std::process::Stdio::null()).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n"))
}

/// As linhas mudadas de `new` em relação a `old` (linha a partir de 0).
pub fn line_changes(old: &str, new: &str) -> Vec<(usize, LineChange)> {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    // Começo e fim iguais saem da conta (quase sempre sobra só um pedaço pequeno).
    let mut pre = 0;
    while pre < a.len() && pre < b.len() && a[pre] == b[pre] {
        pre += 1;
    }
    let mut suf = 0;
    while suf < a.len() - pre && suf < b.len() - pre && a[a.len() - 1 - suf] == b[b.len() - 1 - suf] {
        suf += 1;
    }
    let (ma, mb) = (&a[pre..a.len() - suf], &b[pre..b.len() - suf]);
    let mut out = Vec::new();
    if ma.is_empty() && mb.is_empty() {
        return out;
    }
    // Pedaço grande demais para comparar linha a linha: tudo mudado.
    if ma.len() * mb.len() > 4_000_000 {
        for k in 0..mb.len() {
            out.push((pre + k, LineChange::Modified));
        }
        return out;
    }
    // Maior subsequência comum (programação dinâmica), depois o caminho de volta.
    let (n, m) = (ma.len(), mb.len());
    let mut dp = vec![0u32; (n + 1) * (m + 1)];
    let at = |i: usize, j: usize| i * (m + 1) + j;
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[at(i, j)] = if ma[i] == mb[j] { dp[at(i + 1, j + 1)] + 1 } else { dp[at(i + 1, j)].max(dp[at(i, j + 1)]) };
        }
    }
    // Percorre juntando apagadas e novas de cada trecho diferente.
    let (mut i, mut j) = (0, 0);
    let (mut dels, mut adds): (usize, Vec<usize>) = (0, Vec::new());
    let flush = |dels: &mut usize, adds: &mut Vec<usize>, next_line: usize, out: &mut Vec<(usize, LineChange)>| {
        if adds.is_empty() {
            if *dels > 0 {
                out.push((next_line, LineChange::DeletedAbove));
            }
        } else {
            // Até `dels` das novas contam como mudadas; o resto, novas.
            for (k, line) in adds.iter().enumerate() {
                out.push((*line, if k < *dels { LineChange::Modified } else { LineChange::Added }));
            }
        }
        *dels = 0;
        adds.clear();
    };
    while i < n || j < m {
        if i < n && j < m && ma[i] == mb[j] {
            flush(&mut dels, &mut adds, pre + j, &mut out);
            i += 1;
            j += 1;
        } else if j < m && (i == n || dp[at(i, j + 1)] >= dp[at(i + 1, j)]) {
            adds.push(pre + j);
            j += 1;
        } else {
            dels += 1;
            i += 1;
        }
    }
    flush(&mut dels, &mut adds, pre + j, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn novas_mudadas_e_apagadas() {
        let old = "a\nb\nc\nd\n";
        assert_eq!(line_changes(old, "a\nb\nX\nc\nd\n"), vec![(2, LineChange::Added)]);
        assert_eq!(line_changes(old, "a\nB\nc\nd\n"), vec![(1, LineChange::Modified)]);
        assert_eq!(line_changes(old, "a\nc\nd\n"), vec![(1, LineChange::DeletedAbove)]);
        assert!(line_changes(old, old).is_empty());
    }
}
