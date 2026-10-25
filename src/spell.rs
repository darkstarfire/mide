//! Verificação ortográfica com os dicionários do sistema (formato Hunspell: `.aff` e
//! `.dic` em `/usr/share/hunspell`), escrita aqui. Os mesmos dicionários do LibreOffice e
//! do Firefox, sem a biblioteca do Hunspell.
//!
//! Entende o que os dicionários de português e inglês usam: sufixos e prefixos com
//! condições, combinação de prefixo com sufixo, sufixos em duas camadas (afixo com
//! continuação), palavras proibidas, maiúsculas e palavras com hífen. Sugestões pelas
//! trocas comuns do dicionário (REP), pelas letras parecidas (MAP: acentos) e por uma
//! letra a mais, a menos, trocada ou de lugar. Palavras aprendidas ficam num arquivo do
//! usuário.

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

/// Condição de um afixo: uma sequência de letras ou grupos (`[abc]`, `[^abc]`, `.`).
#[derive(Debug, Clone)]
enum CondPart {
    Any,
    Set(Vec<char>, bool),
    Char(char),
}

#[derive(Debug, Clone)]
struct Affix {
    flag: char,
    /// O que sai da raiz e o que entra.
    strip: String,
    cond: Vec<CondPart>,
    /// Afixos que podem vir depois deste (duas camadas).
    cont: Vec<char>,
    cross: bool,
}

fn parse_cond(c: &str) -> Vec<CondPart> {
    if c == "." {
        return Vec::new();
    }
    let mut out = Vec::new();
    let chars: Vec<char> = c.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '.' => out.push(CondPart::Any),
            '[' => {
                let mut j = i + 1;
                let neg = chars.get(j) == Some(&'^');
                if neg {
                    j += 1;
                }
                let mut set = Vec::new();
                while j < chars.len() && chars[j] != ']' {
                    set.push(chars[j]);
                    j += 1;
                }
                out.push(CondPart::Set(set, neg));
                i = j;
            }
            ch => out.push(CondPart::Char(ch)),
        }
        i += 1;
    }
    out
}

fn part_matches(p: &CondPart, c: char) -> bool {
    match p {
        CondPart::Any => true,
        CondPart::Char(x) => *x == c,
        CondPart::Set(set, neg) => set.contains(&c) != *neg,
    }
}

impl Affix {
    /// A condição vale no fim (sufixo) ou no começo (prefixo) da raiz.
    fn cond_ok(&self, root: &str, suffix: bool) -> bool {
        if self.cond.is_empty() {
            return true;
        }
        let chars: Vec<char> = root.chars().collect();
        if chars.len() < self.cond.len() {
            return false;
        }
        let start = if suffix { chars.len() - self.cond.len() } else { 0 };
        self.cond.iter().enumerate().all(|(i, p)| part_matches(p, chars[start + i]))
    }
}

/// Um dicionário carregado.
pub struct Speller {
    words: HashMap<String, Vec<char>>,
    suffixes: HashMap<String, Vec<Affix>>,
    prefixes: HashMap<String, Vec<Affix>>,
    max_suffix: usize,
    max_prefix: usize,
    forbidden: Option<char>,
    nosuggest: Option<char>,
    needaffix: Option<char>,
    rep: Vec<(String, String)>,
    map: Vec<Vec<char>>,
    try_chars: Vec<char>,
    /// Palavras aprendidas (arquivo do usuário) e ignoradas nesta sessão.
    learned: HashSet<String>,
    ignored: HashSet<String>,
    user_file: Option<PathBuf>,
    pub language: String,
}

/// Arquivo das palavras aprendidas: `~/.local/share/mide/spelling-words.txt`.
pub fn user_words_file() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))?;
    Some(base.join("mide/spelling-words.txt"))
}

/// O dicionário do idioma (de `LANG`, com reserva em português e inglês).
pub fn find_dictionary() -> Option<(PathBuf, PathBuf, String)> {
    let lang = std::env::var("LANG").unwrap_or_default();
    let code = lang.split('.').next().unwrap_or("").to_string();
    let dirs = ["/usr/share/hunspell", "/usr/share/myspell", "/usr/share/myspell/dicts"];
    for name in [code.as_str(), "pt_BR", "en_US"] {
        if name.is_empty() {
            continue;
        }
        for d in dirs {
            let aff = Path::new(d).join(format!("{name}.aff"));
            let dic = Path::new(d).join(format!("{name}.dic"));
            if aff.is_file() && dic.is_file() {
                return Some((aff, dic, name.to_string()));
            }
        }
    }
    None
}

impl Speller {
    /// Carrega o dicionário do sistema e as palavras aprendidas.
    pub fn load_default() -> Option<Self> {
        let (aff, dic, lang) = find_dictionary()?;
        let mut s = Self::load(&std::fs::read_to_string(aff).ok()?, &std::fs::read_to_string(dic).ok()?)?;
        s.language = lang;
        s.user_file = user_words_file();
        if let Some(text) = s.user_file.as_ref().and_then(|f| std::fs::read_to_string(f).ok()) {
            s.learned.extend(text.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string));
        }
        Some(s)
    }

    /// Lê o `.aff` e o `.dic` (em UTF-8).
    pub fn load(aff: &str, dic: &str) -> Option<Self> {
        let mut s = Self {
            words: HashMap::new(),
            suffixes: HashMap::new(),
            prefixes: HashMap::new(),
            max_suffix: 0,
            max_prefix: 0,
            forbidden: None,
            nosuggest: None,
            needaffix: None,
            rep: Vec::new(),
            map: Vec::new(),
            try_chars: Vec::new(),
            learned: HashSet::new(),
            ignored: HashSet::new(),
            user_file: None,
            language: String::new(),
        };
        let mut headers: HashMap<(bool, char), bool> = HashMap::new();
        for line in aff.lines() {
            let f: Vec<&str> = line.split_whitespace().collect();
            match f.as_slice() {
                ["TRY", chars, ..] => s.try_chars = chars.chars().collect(),
                ["FORBIDDENWORD", flag, ..] => s.forbidden = flag.chars().next(),
                ["NOSUGGEST", flag, ..] => s.nosuggest = flag.chars().next(),
                ["NEEDAFFIX", flag, ..] => s.needaffix = flag.chars().next(),
                ["REP", from, to, ..] => s.rep.push((from.to_string(), to.replace('_', " "))),
                ["MAP", chars] if chars.parse::<usize>().is_err() => s.map.push(chars.chars().filter(|c| *c != '(' && *c != ')').collect()),
                [kind @ ("SFX" | "PFX"), flag, cross, n] if n.parse::<usize>().is_ok() && (*cross == "Y" || *cross == "N") => {
                    if let Some(c) = flag.chars().next() {
                        headers.insert((*kind == "SFX", c), *cross == "Y");
                    }
                }
                [kind @ ("SFX" | "PFX"), flag, strip, add, rest @ ..] => {
                    let Some(flag) = flag.chars().next() else { continue };
                    let suffix = *kind == "SFX";
                    let (add, cont) = match add.split_once('/') {
                        Some((a, c)) => (a, c.chars().collect()),
                        None => (*add, Vec::new()),
                    };
                    let add = if add == "0" { "" } else { add };
                    let strip = if *strip == "0" { "" } else { *strip };
                    let cond = parse_cond(rest.first().copied().unwrap_or("."));
                    let a = Affix { flag, strip: strip.to_string(), cond, cont, cross: headers.get(&(suffix, flag)).copied().unwrap_or(false) };
                    let n = add.chars().count();
                    if suffix {
                        s.max_suffix = s.max_suffix.max(n);
                        s.suffixes.entry(add.to_string()).or_default().push(a);
                    } else {
                        s.max_prefix = s.max_prefix.max(n);
                        s.prefixes.entry(add.to_string()).or_default().push(a);
                    }
                }
                _ => {}
            }
        }
        for (i, line) in dic.lines().enumerate() {
            let line = line.trim_start_matches('\u{feff}').trim();
            if i == 0 && line.parse::<usize>().is_ok() || line.is_empty() {
                continue;
            }
            // "palavra/FLAGS" (a barra escapada "\/" faz parte da palavra).
            let line = line.split('\t').next().unwrap_or(line);
            let (word, flags) = match line.find('/').filter(|&i| i == 0 || !line[..i].ends_with('\\')) {
                Some(i) => (&line[..i], line[i + 1..].split_whitespace().next().unwrap_or("").chars().collect()),
                None => (line.split_whitespace().next().unwrap_or(line), Vec::new()),
            };
            let entry: &mut Vec<char> = s.words.entry(word.replace("\\/", "/")).or_default();
            entry.extend(flags);
        }
        (!s.words.is_empty()).then_some(s)
    }

    fn has_flag(&self, word: &str, flag: char) -> bool {
        self.words.get(word).is_some_and(|f| f.contains(&flag) && !self.forbidden.is_some_and(|x| f.contains(&x)))
    }

    /// A palavra (já na forma de busca) existe, sozinha ou com afixos.
    fn check_exact(&self, w: &str) -> bool {
        if let Some(flags) = self.words.get(w) {
            let forbidden = self.forbidden.is_some_and(|x| flags.contains(&x));
            let needs = self.needaffix.is_some_and(|x| flags.contains(&x));
            if forbidden {
                return false;
            }
            if !needs {
                return true;
            }
        }
        self.check_suffix(w, None, 0) || self.check_prefix(w)
    }

    /// Tira um sufixo: a raiz tem a marca dele (e, se `need` vier, a do prefixo também).
    /// `depth` 0 permite uma segunda camada de sufixo.
    fn check_suffix(&self, w: &str, need: Option<char>, depth: u8) -> bool {
        let chars: Vec<char> = w.chars().collect();
        for n in 0..=self.max_suffix.min(chars.len().saturating_sub(1)) {
            let add: String = chars[chars.len() - n..].iter().collect();
            let Some(list) = self.suffixes.get(&add) else {
                continue;
            };
            let stem: String = chars[..chars.len() - n].iter().collect();
            for a in list {
                if need.is_some() && !a.cross {
                    continue;
                }
                let root = format!("{stem}{}", a.strip);
                if !a.cond_ok(&root, true) {
                    continue;
                }
                if self.has_flag(&root, a.flag) && need.is_none_or(|p| self.has_flag(&root, p)) {
                    return true;
                }
                // Duas camadas: a raiz é ela mesma derivada de um sufixo que aceita este.
                if depth == 0 && need.is_none() && self.check_inner_suffix(&root, a.flag) {
                    return true;
                }
            }
        }
        false
    }

    /// `w` vem de uma raiz com um sufixo cuja continuação inclui `outer`.
    fn check_inner_suffix(&self, w: &str, outer: char) -> bool {
        let chars: Vec<char> = w.chars().collect();
        for n in 0..=self.max_suffix.min(chars.len().saturating_sub(1)) {
            let add: String = chars[chars.len() - n..].iter().collect();
            let Some(list) = self.suffixes.get(&add) else {
                continue;
            };
            let stem: String = chars[..chars.len() - n].iter().collect();
            for a in list.iter().filter(|a| a.cont.contains(&outer)) {
                let root = format!("{stem}{}", a.strip);
                if a.cond_ok(&root, true) && self.has_flag(&root, a.flag) {
                    return true;
                }
            }
        }
        false
    }

    fn check_prefix(&self, w: &str) -> bool {
        let chars: Vec<char> = w.chars().collect();
        for n in 1..=self.max_prefix.min(chars.len().saturating_sub(1)) {
            let add: String = chars[..n].iter().collect();
            let Some(list) = self.prefixes.get(&add) else {
                continue;
            };
            let rest: String = chars[n..].iter().collect();
            for a in list {
                let root = format!("{}{rest}", a.strip);
                if !a.cond_ok(&root, false) {
                    continue;
                }
                if self.has_flag(&root, a.flag) {
                    return true;
                }
                // Prefixo e sufixo juntos (os dois aceitam combinar).
                if a.cross && self.check_suffix(&root, Some(a.flag), 1) {
                    return true;
                }
            }
        }
        false
    }

    /// A palavra está certa? (Números, siglas com números e palavras aprendidas valem.)
    pub fn check(&self, word: &str) -> bool {
        let w = word.trim_matches(|c: char| c == '\'' || c == '’' || c == '-');
        if w.is_empty() || w.chars().any(|c| c.is_ascii_digit()) || self.learned.contains(w) || self.ignored.contains(w) {
            return true;
        }
        if self.check_exact(w) {
            return true;
        }
        let lower = w.to_lowercase();
        let mut cs = w.chars();
        let first_upper = cs.next().is_some_and(char::is_uppercase);
        let all_upper = w.chars().filter(|c| c.is_alphabetic()).all(char::is_uppercase);
        if (first_upper || all_upper) && self.check_exact(&lower) {
            return true;
        }
        if all_upper {
            let mut cap: String = lower.chars().take(1).flat_map(char::to_uppercase).collect();
            cap.extend(lower.chars().skip(1));
            if self.check_exact(&cap) {
                return true;
            }
        }
        if self.learned.contains(&lower) {
            return true;
        }
        // Palavras com hífen: cada parte certa (ex-aluno, guarda-chuva).
        if w.contains('-') {
            return w.split('-').filter(|p| !p.is_empty()).all(|p| self.check(p));
        }
        false
    }

    /// Até `max` sugestões para a palavra errada.
    pub fn suggest(&self, word: &str, max: usize) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let push = |cand: String, out: &mut Vec<String>| {
            if cand != word && !out.contains(&cand) && out.len() < max && self.check(&cand) && !self.nosuggest_word(&cand) {
                out.push(cand);
            }
        };
        // Trocas comuns do dicionário (REP) e letras parecidas (MAP: acentos).
        for (from, to) in &self.rep {
            let anchored_start = from.starts_with('^');
            let anchored_end = from.ends_with('$');
            let pat = from.trim_start_matches('^').trim_end_matches('$');
            if pat.is_empty() {
                continue;
            }
            for (i, _) in word.match_indices(pat) {
                if (anchored_start && i != 0) || (anchored_end && i + pat.len() != word.len()) {
                    continue;
                }
                push(format!("{}{to}{}", &word[..i], &word[i + pat.len()..]), &mut out);
            }
        }
        let chars: Vec<char> = word.chars().collect();
        for (i, c) in chars.iter().enumerate() {
            for group in self.map.iter().filter(|g| g.contains(c)) {
                for alt in group.iter().filter(|a| *a != c) {
                    let mut v = chars.clone();
                    v[i] = *alt;
                    push(v.into_iter().collect(), &mut out);
                }
            }
        }
        // Uma letra de lugar, faltando, sobrando ou trocada.
        for i in 0..chars.len().saturating_sub(1) {
            let mut v = chars.clone();
            v.swap(i, i + 1);
            push(v.into_iter().collect(), &mut out);
        }
        for i in 0..chars.len() {
            let mut v = chars.clone();
            v.remove(i);
            push(v.into_iter().collect(), &mut out);
        }
        let alphabet: Vec<char> = if self.try_chars.is_empty() { ('a'..='z').collect() } else { self.try_chars.clone() };
        for i in 0..=chars.len() {
            for a in &alphabet {
                let mut v = chars.clone();
                v.insert(i, *a);
                push(v.into_iter().collect(), &mut out);
            }
        }
        for i in 0..chars.len() {
            for a in &alphabet {
                if *a == chars[i] {
                    continue;
                }
                let mut v = chars.clone();
                v[i] = *a;
                push(v.into_iter().collect(), &mut out);
            }
        }
        // Duas palavras juntas por engano.
        for i in 1..chars.len() {
            let (a, b): (String, String) = (chars[..i].iter().collect(), chars[i..].iter().collect());
            if a.chars().count() > 1 && b.chars().count() > 1 && self.check(&a) && self.check(&b) {
                let cand = format!("{a} {b}");
                if !out.contains(&cand) && out.len() < max {
                    out.push(cand);
                }
            }
        }
        out
    }

    fn nosuggest_word(&self, w: &str) -> bool {
        self.nosuggest.is_some_and(|f| self.words.get(w).is_some_and(|fl| fl.contains(&f)))
    }

    /// "Aprender ortografia": a palavra passa a valer (e fica no arquivo do usuário).
    pub fn learn(&mut self, word: &str) {
        if self.learned.insert(word.to_string())
            && let Some(f) = &self.user_file {
                if let Some(dir) = f.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let mut list: Vec<&String> = self.learned.iter().collect();
                list.sort();
                let text: String = list.iter().map(|w| format!("{w}\n")).collect();
                let _ = std::fs::write(f, text);
            }
    }

    /// "Ignorar": vale até fechar o app.
    pub fn ignore(&mut self, word: &str) {
        self.ignored.insert(word.to_string());
    }
}

/// As palavras de um texto (bytes de início e fim): letras, com apóstrofo e hífen no
/// meio.
pub fn words(text: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    for (k, &(i, c)) in chars.iter().enumerate() {
        let inner = (c == '-' || c == '\'' || c == '’') && start.is_some() && chars.get(k + 1).is_some_and(|(_, n)| n.is_alphabetic());
        if c.is_alphanumeric() || inner {
            if start.is_none() {
                start = Some(i);
            }
        } else if let Some(s) = start.take() {
            out.push((s, i));
        }
    }
    if let Some(s) = start {
        out.push((s, text.len()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const AFF: &str = "SET UTF-8\nFLAG UTF-8\nTRY aeiosrnmtcdulpvgbfhqjxzçãéáíóúâêô\nFORBIDDENWORD !\nREP 1\nREP ss ç\nMAP 1\nMAP aáã\nSFX A Y 2\nSFX A 0 s [aeiou]\nSFX A ão ões ão\nSFX B Y 1\nSFX B 0 mente/A .\nPFX R Y 1\nPFX R 0 re .\n";
    const DIC: &str = "6\ncasa/AR\ncoração/A\nrápido/B\nfazer/R\nnão\nerrado/!\n";

    #[test]
    fn verifica_com_afixos() {
        let s = Speller::load(AFF, DIC).unwrap();
        assert!(s.check("casa"));
        assert!(s.check("casas"));
        assert!(s.check("Casas"));
        assert!(s.check("CASAS"));
        assert!(s.check("corações"));
        assert!(s.check("rapidamente") == false);
        assert!(s.check("rápidomente"));
        assert!(s.check("rápidomentes"), "duas camadas: mente e depois s");
        assert!(s.check("refazer"));
        assert!(s.check("recasas"), "prefixo com sufixo");
        assert!(!s.check("casaz"));
        assert!(!s.check("errado"), "palavra proibida");
        assert!(s.check("não-casa"));
        assert!(s.check("2026"));
    }

    #[test]
    fn sugere() {
        let s = Speller::load(AFF, DIC).unwrap();
        assert!(s.suggest("csa", 5).contains(&"casa".to_string()));
        assert!(s.suggest("cassa", 5).contains(&"casa".to_string()));
        assert!(s.suggest("nao", 5).contains(&"não".to_string()), "{:?}", s.suggest("nao", 5));
        assert_eq!(words("Olá, guarda-chuva d'água 12x."), vec![(0, 4), (6, 18), (19, 26), (27, 30)]);
    }

    /// Com o dicionário de verdade do sistema (se houver).
    #[test]
    #[ignore]
    fn dicionario_do_sistema() {
        let t = std::time::Instant::now();
        let s = Speller::load_default().expect("dicionário");
        eprintln!("carregado em {:?} ({})", t.elapsed(), s.language);
        for w in ["casa", "casas", "coração", "corações", "rapidamente", "infelizmente", "computadores", "trabalhando", "reescrever", "água", "Brasil", "guarda-chuva", "metodologia", "acadêmico", "introdução"] {
            assert!(s.check(w), "deveria estar certa: {w}");
        }
        for w in ["casaa", "trabalhano", "metodolojia", "intrudução"] {
            assert!(!s.check(w), "deveria estar errada: {w}");
        }
        eprintln!("sugestões para 'metodolojia': {:?}", s.suggest("metodolojia", 5));
        eprintln!("sugestões para 'intrudução': {:?}", s.suggest("intrudução", 5));
    }
}
