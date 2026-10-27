//! Regras do buscador do mide (sem interface): contas, índice de arquivos das pastas
//! pessoais e as seções das Configurações que aparecem nos resultados.

use std::path::{Path, PathBuf};

/// Resultado de uma conta, se `text` for uma expressão (`2+2`, `(3,5*4)/2`, `2^10`).
/// Precisa ter pelo menos um operador: um número sozinho não é conta.
pub fn calculate(text: &str) -> Option<f64> {
    let text = text.trim().replace(',', ".").replace('×', "*").replace('÷', "/");
    if !text.chars().any(|c| "+-*/^".contains(c)) || !text.chars().any(|c| c.is_ascii_digit()) {
        return None;
    }
    let tokens: Vec<char> = text.chars().filter(|c| !c.is_whitespace()).collect();
    let mut parser = Parser { tokens, pos: 0 };
    let value = parser.expr()?;
    (parser.pos == parser.tokens.len() && value.is_finite()).then_some(value)
}

/// Número para mostrar: sem casas quando é inteiro, até 10 casas sem zeros no fim,
/// vírgula como separador decimal.
pub fn number_text(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        return format!("{}", value as i64);
    }
    let text = format!("{value:.10}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    text.replace('.', ",")
}

/// Analisador de expressões: soma e subtração, depois multiplicação e divisão,
/// depois potência, depois números, parênteses e sinal.
struct Parser {
    tokens: Vec<char>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.tokens.get(self.pos).copied()
    }

    fn expr(&mut self) -> Option<f64> {
        let mut value = self.term()?;
        while let Some(op) = self.peek().filter(|c| *c == '+' || *c == '-') {
            self.pos += 1;
            let rhs = self.term()?;
            value = if op == '+' { value + rhs } else { value - rhs };
        }
        Some(value)
    }

    fn term(&mut self) -> Option<f64> {
        let mut value = self.power()?;
        while let Some(op) = self.peek().filter(|c| *c == '*' || *c == '/') {
            self.pos += 1;
            let rhs = self.power()?;
            value = if op == '*' { value * rhs } else { value / rhs };
        }
        Some(value)
    }

    fn power(&mut self) -> Option<f64> {
        let base = self.unary()?;
        if self.peek() == Some('^') {
            self.pos += 1;
            // Potência associa à direita: 2^3^2 = 2^9.
            let exp = self.power()?;
            return Some(base.powf(exp));
        }
        Some(base)
    }

    fn unary(&mut self) -> Option<f64> {
        match self.peek()? {
            '-' => {
                self.pos += 1;
                Some(-self.unary()?)
            }
            '+' => {
                self.pos += 1;
                self.unary()
            }
            '(' => {
                self.pos += 1;
                let value = self.expr()?;
                (self.peek() == Some(')')).then(|| self.pos += 1)?;
                Some(value)
            }
            c if c.is_ascii_digit() || c == '.' => {
                let start = self.pos;
                while self.peek().is_some_and(|c| c.is_ascii_digit() || c == '.') {
                    self.pos += 1;
                }
                self.tokens[start..self.pos].iter().collect::<String>().parse().ok()
            }
            _ => None,
        }
    }
}

/// Um arquivo ou pasta do índice: nome (já sem acentos e em minúsculas, para buscar)
/// e caminho.
#[derive(Debug, Clone)]
pub struct Indexed {
    pub folded: String,
    pub path: PathBuf,
    pub is_dir: bool,
}

/// Índice dos arquivos das pastas `roots`, até `depth` níveis e `limit` itens, sem
/// ocultos. Feito em segundo plano pelo buscador ao abrir.
pub fn index_files(roots: &[PathBuf], depth: usize, limit: usize) -> Vec<Indexed> {
    let mut out = Vec::new();
    let mut stack: Vec<(PathBuf, usize)> = roots.iter().map(|r| (r.clone(), 0)).collect();
    while let Some((dir, level)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            let path = entry.path();
            if is_dir && level + 1 < depth {
                stack.push((path.clone(), level + 1));
            }
            out.push(Indexed { folded: crate::apps::fold(&name), path, is_dir });
            if out.len() >= limit {
                return out;
            }
        }
    }
    out
}

/// Até `n` itens do índice com uma palavra do nome começando pela busca (como o
/// Spotlight: "cal" acha "calendario.txt" e "minha calculadora", não "scala"); os que
/// começam com ela primeiro, depois os de nome mais curto.
pub fn match_files<'a>(index: &'a [Indexed], query: &str, n: usize) -> Vec<&'a Indexed> {
    let q = crate::apps::fold(query.trim());
    if q.chars().count() < 2 {
        return Vec::new();
    }
    let mut found: Vec<&Indexed> = index.iter().filter(|i| word_starts_with(&i.folded, &q)).collect();
    found.sort_by_key(|i| (!i.folded.starts_with(&q), i.folded.len()));
    found.truncate(n);
    found
}

/// Alguma palavra de `name` (separadas por espaço, `-`, `_`, `.` ou mudança de letra
/// para número) começa com `q`?
pub fn word_starts_with(name: &str, q: &str) -> bool {
    name.match_indices(q).any(|(at, _)| {
        at == 0 || name[..at].chars().next_back().is_some_and(|c| !c.is_alphanumeric())
    })
}

/// Seções das Configurações que o buscador encontra: nome, palavras-chave, seção e
/// ícone do tema.
pub const SETTINGS: &[(&str, &str, &str, &str)] = &[
    ("Wi-Fi", "wifi rede internet wireless", "wifi", "network-wireless"),
    ("Rede", "rede cabo cabeada ethernet internet endereco ip", "network", "network-wired"),
    ("Bluetooth", "bluetooth fone mouse teclado sem fio", "bluetooth", "bluetooth"),
    ("Som", "som volume audio alto-falante microfone", "sound", "audio-volume-high"),
    ("Papel de parede", "papel de parede fundo wallpaper imagem", "wallpaper", "preferences-desktop-wallpaper"),
    ("Dock", "dock icones tamanho barra de apps", "dock", "user-desktop"),
    ("Efeitos", "efeitos animacoes sombras desfoque vidro transparencia", "effects", "preferences-desktop-theme"),
    ("Atalhos de teclado", "atalhos teclado teclas combinacoes", "keyboard", "preferences-desktop-keyboard"),
    ("Sobre este computador", "sobre sistema versao computador memoria", "about", "help-about"),
];

/// Seções das Configurações que casam com a busca (pelo nome ou palavras-chave).
pub fn match_settings(query: &str) -> Vec<&'static (&'static str, &'static str, &'static str, &'static str)> {
    let q = crate::apps::fold(query.trim());
    if q.is_empty() {
        return Vec::new();
    }
    SETTINGS.iter().filter(|(name, keys, _, _)| crate::apps::fold(name).contains(&q) || keys.split(' ').any(|k| k.starts_with(&q))).collect()
}

/// Pasta que contém `path`, com `~` no lugar da pasta pessoal (detalhe dos resultados).
pub fn short_parent(path: &Path, home: &Path) -> String {
    let parent = path.parent().unwrap_or(path);
    match parent.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".into(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => parent.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contas_basicas() {
        assert_eq!(calculate("2+2"), Some(4.0));
        assert_eq!(calculate("2 + 3 * 4"), Some(14.0));
        assert_eq!(calculate("(2+3)*4"), Some(20.0));
        assert_eq!(calculate("3,5*2"), Some(7.0));
        assert_eq!(calculate("2^10"), Some(1024.0));
        assert_eq!(calculate("2^3^2"), Some(512.0));
        assert_eq!(calculate("-3+5"), Some(2.0));
        assert_eq!(calculate("10/4"), Some(2.5));
    }

    #[test]
    fn nao_e_conta() {
        assert_eq!(calculate("42"), None, "número sozinho");
        assert_eq!(calculate("firefox"), None);
        assert_eq!(calculate("2+"), None);
        assert_eq!(calculate("(2+3"), None);
        assert_eq!(calculate("1/0"), None, "infinito");
        assert_eq!(calculate("wi-fi"), None);
    }

    #[test]
    fn numeros_em_portugues() {
        assert_eq!(number_text(4.0), "4");
        assert_eq!(number_text(2.5), "2,5");
        assert_eq!(number_text(1.0 / 3.0), "0,3333333333");
        assert_eq!(number_text(-7.0), "-7");
    }

    #[test]
    fn indice_e_busca_de_arquivos() {
        let base = std::env::temp_dir().join(format!("mide-busca-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("Documentos/Relatórios")).unwrap();
        std::fs::create_dir_all(base.join(".oculta")).unwrap();
        std::fs::write(base.join("Documentos/Relatórios/relatorio anual.pdf"), "x").unwrap();
        std::fs::write(base.join("Documentos/notas.txt"), "x").unwrap();
        std::fs::write(base.join(".oculta/relatorio.txt"), "x").unwrap();
        let index = index_files(std::slice::from_ref(&base), 4, 100);
        assert!(index.iter().all(|i| !i.path.to_string_lossy().contains(".oculta")));
        let found: Vec<String> = match_files(&index, "relat", 5).iter().map(|i| i.folded.clone()).collect();
        // A pasta "Relatórios" (nome mais curto) vem antes do arquivo; sem acentos.
        assert_eq!(found, vec!["relatorios".to_string(), "relatorio anual.pdf".to_string()]);
        assert!(match_files(&index, "r", 5).is_empty(), "uma letra só não busca arquivos");
        assert_eq!(match_files(&index, "anual", 5).len(), 1, "começo de outra palavra vale");
        assert!(match_files(&index, "latorio", 5).is_empty(), "meio de palavra não vale");
        assert_eq!(index_files(std::slice::from_ref(&base), 4, 2).len(), 2, "limite");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn secoes_das_configuracoes() {
        let names: Vec<&str> = match_settings("volume").iter().map(|s| s.0).collect();
        assert_eq!(names, vec!["Som"]);
        assert_eq!(match_settings("wifi")[0].2, "wifi");
        assert_eq!(match_settings("papel")[0].0, "Papel de parede");
        assert!(match_settings("").is_empty());
    }

    #[test]
    fn pasta_curta() {
        let home = Path::new("/home/ana");
        assert_eq!(short_parent(Path::new("/home/ana/Documentos/a.txt"), home), "~/Documentos");
        assert_eq!(short_parent(Path::new("/home/ana/a.txt"), home), "~");
        assert_eq!(short_parent(Path::new("/etc/hosts"), home), "/etc");
    }
}
