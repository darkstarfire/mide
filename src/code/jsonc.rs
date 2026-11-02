//! JSON com comentários (`//` e `/* */`) e vírgulas sobrando, como o `settings.json`
//! do Zed e do VS Code e os arquivos de trechos: tira o que o JSON comum não aceita.

/// O texto sem comentários e sem vírgulas antes de `}` ou `]` (fora das strings).
pub fn strip(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    let mut in_str = false;
    while i < chars.len() {
        let c = chars[i];
        if in_str {
            out.push(c);
            if c == '\\' && i + 1 < chars.len() {
                out.push(chars[i + 1]);
                i += 2;
                continue;
            }
            if c == '"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        match c {
            '"' => {
                in_str = true;
                out.push(c);
            }
            '/' if chars.get(i + 1) == Some(&'/') => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
                continue;
            }
            '/' if chars.get(i + 1) == Some(&'*') => {
                i += 2;
                while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                    i += 1;
                }
                i += 2;
                continue;
            }
            ',' => {
                // Vírgula sobrando: o próximo caractere que não é espaço fecha a lista.
                let next = chars[i + 1..].iter().find(|c| !c.is_whitespace());
                if !matches!(next, Some('}') | Some(']')) {
                    out.push(c);
                }
            }
            _ => out.push(c),
        }
        i += 1;
    }
    out
}

/// Lê um JSON com comentários.
pub fn parse(text: &str) -> Option<serde_json::Value> {
    serde_json::from_str(&strip(text)).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tira_comentarios_e_virgulas() {
        let t = "{\n // c\n \"a\": \"http://x\", /* b */ \"l\": [1, 2,],\n}";
        let v = parse(t).unwrap();
        assert_eq!(v["a"], "http://x");
        assert_eq!(v["l"].as_array().unwrap().len(), 2);
    }
}
