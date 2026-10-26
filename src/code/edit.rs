//! Edições de linhas do `mide-code`, sem interface (testáveis sozinhas): comentar,
//! duplicar e mover linhas, buscar a próxima/anterior ocorrência e ir para
//! "linha:coluna". Tudo em bytes do texto (como o `TextDoc` do mide-ui).

/// Começo e fim (sem o `\n`) das linhas que a seleção `a..b` toca. Uma seleção que
/// termina no começo de uma linha não leva essa linha (como nos editores de código).
pub fn line_block(text: &str, a: usize, b: usize) -> (usize, usize) {
    let (a, mut b) = (a.min(b).min(text.len()), a.max(b).min(text.len()));
    if b > a && text[..b].ends_with('\n') {
        b -= 1;
    }
    let start = text[..a].rfind('\n').map_or(0, |i| i + 1);
    let end = text[b..].find('\n').map_or(text.len(), |i| b + i);
    (start, end)
}

/// Comenta ou descomenta (com `prefix`, como `//` ou `#`) as linhas de `a..b`. Se todas
/// as linhas com texto já estão comentadas, tira o comentário; senão, comenta todas.
/// Devolve a nova posição de `a` e de `b` (seguindo o texto), ou `None` sem mudança.
pub fn toggle_comment(text: &mut String, a: usize, b: usize, prefix: &str) -> Option<(usize, usize)> {
    let (start, end) = line_block(text, a, b);
    let block = &text[start..end];
    let filled: Vec<&str> = block.split('\n').filter(|l| !l.trim().is_empty()).collect();
    if filled.is_empty() {
        return None;
    }
    let all_commented = filled.iter().all(|l| l.trim_start().starts_with(prefix));
    // Recuo comum (o menor), para o comentário ficar alinhado.
    let indent = filled.iter().map(|l| l.len() - l.trim_start().len()).min().unwrap_or(0);
    let mut out = Vec::new();
    for line in block.split('\n') {
        if line.trim().is_empty() {
            out.push(line.to_string());
        } else if all_commented {
            let lead = line.len() - line.trim_start().len();
            let rest = &line[lead + prefix.len()..];
            let rest = rest.strip_prefix(' ').unwrap_or(rest);
            out.push(format!("{}{rest}", &line[..lead]));
        } else {
            out.push(format!("{}{prefix} {}", &line[..indent], &line[indent..]));
        }
    }
    let new_block = out.join("\n");
    let map = |p: usize| -> usize {
        if p <= start {
            return p;
        }
        if p >= end {
            return (p as isize + new_block.len() as isize - block.len() as isize) as usize;
        }
        // Dentro do bloco: segue a linha (mesmo deslocamento da linha em que está).
        let line_i = text[start..p].matches('\n').count();
        let old_line_start = start + block.split('\n').take(line_i).map(|l| l.len() + 1).sum::<usize>();
        let new_line_start = start + out.iter().take(line_i).map(|l| l.len() + 1).sum::<usize>();
        let old_len = block.split('\n').nth(line_i).map_or(0, str::len);
        let new_len = out.get(line_i).map_or(0, String::len);
        let col = p - old_line_start;
        let col = if new_len >= old_len { col + (new_len - old_len) } else { col.saturating_sub(old_len - new_len) };
        new_line_start + col.min(new_len)
    };
    let (na, nb) = (map(a), map(b));
    text.replace_range(start..end, &new_block);
    Some((na, nb))
}

/// Duplica as linhas de `a..b` logo abaixo delas. Devolve quanto o cursor anda (o
/// tamanho do bloco mais o `\n`), para ele ir para a cópia.
pub fn duplicate_lines(text: &mut String, a: usize, b: usize) -> usize {
    let (start, end) = line_block(text, a, b);
    let block = text[start..end].to_string();
    text.insert_str(end, &format!("\n{block}"));
    block.len() + 1
}

/// Move as linhas de `a..b` uma linha para cima (`up`) ou para baixo. Devolve o
/// deslocamento da seleção (negativo para cima), ou `None` na primeira/última linha.
pub fn move_lines(text: &mut String, a: usize, b: usize, up: bool) -> Option<isize> {
    let (start, end) = line_block(text, a, b);
    let block = text[start..end].to_string();
    if up {
        if start == 0 {
            return None;
        }
        let prev_start = text[..start - 1].rfind('\n').map_or(0, |i| i + 1);
        let prev = text[prev_start..start - 1].to_string();
        text.replace_range(prev_start..end, &format!("{block}\n{prev}"));
        Some(-((prev.len() + 1) as isize))
    } else {
        if end >= text.len() {
            return None;
        }
        let next_end = text[end + 1..].find('\n').map_or(text.len(), |i| end + 1 + i);
        let next = text[end + 1..next_end].to_string();
        text.replace_range(start..next_end, &format!("{next}\n{block}"));
        Some((next.len() + 1) as isize)
    }
}

/// Próxima ocorrência de `query` a partir de `from` (volta ao começo no fim), ou a
/// anterior antes de `from` (volta ao fim). `case`: diferencia maiúsculas.
pub fn find(text: &str, query: &str, from: usize, forward: bool, case: bool) -> Option<usize> {
    if query.is_empty() {
        return None;
    }
    let (hay, needle) = if case { (text.to_string(), query.to_string()) } else { (text.to_lowercase(), query.to_lowercase()) };
    // Minúsculas podem mudar o tamanho em bytes (raro): aí busca com diferença.
    let (hay, needle) = if hay.len() == text.len() { (hay, needle) } else { (text.to_string(), query.to_string()) };
    let from = from.min(hay.len());
    if forward {
        hay[from..].find(&needle).map(|i| from + i).or_else(|| hay.find(&needle))
    } else {
        hay[..from].rfind(&needle).or_else(|| hay.rfind(&needle))
    }
}

/// Quantas vezes `query` aparece.
pub fn count(text: &str, query: &str, case: bool) -> usize {
    if query.is_empty() {
        return 0;
    }
    if case {
        text.matches(query).count()
    } else {
        text.to_lowercase().matches(&query.to_lowercase()).count()
    }
}

/// Troca todas as ocorrências (com ou sem diferença de maiúsculas). Devolve quantas.
pub fn replace_all(text: &mut String, query: &str, with: &str, case: bool) -> usize {
    if query.is_empty() {
        return 0;
    }
    let (hay, needle) = if case { (text.clone(), query.to_string()) } else { (text.to_lowercase(), query.to_lowercase()) };
    // Minúsculas que mudam o tamanho em bytes (raro): troca com diferença.
    let (hay, needle) = if hay.len() == text.len() && needle.len() == query.len() { (hay, needle) } else { (text.clone(), query.to_string()) };
    let starts: Vec<usize> = hay.match_indices(&needle).map(|(i, _)| i).collect();
    if starts.is_empty() {
        return 0;
    }
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for &i in &starts {
        out.push_str(&text[at..i]);
        out.push_str(with);
        at = i + needle.len();
    }
    out.push_str(&text[at..]);
    *text = out;
    starts.len()
}

/// "12" ou "12:5" (linha e coluna, contando de 1) → (linha, coluna).
pub fn parse_goto(s: &str) -> Option<(usize, usize)> {
    let mut parts = s.trim().splitn(2, [':', ',']);
    let line = parts.next()?.trim().parse::<usize>().ok()?;
    let col = match parts.next() {
        Some(c) => c.trim().parse::<usize>().ok()?,
        None => 1,
    };
    Some((line.max(1), col.max(1)))
}

/// Byte da linha e coluna (de 1; a coluna em caracteres, presa ao fim da linha).
pub fn offset_of(text: &str, line: usize, col: usize) -> usize {
    let mut start = 0;
    for _ in 1..line {
        match text[start..].find('\n') {
            Some(i) => start += i + 1,
            None => break,
        }
    }
    let end = text[start..].find('\n').map_or(text.len(), |i| start + i);
    text[start..end].char_indices().nth(col.saturating_sub(1)).map_or(end, |(i, _)| start + i)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comentar_e_descomentar() {
        let mut t = String::from("fn a() {\n    x();\n\n    y();\n}\n");
        let (a, b) = (t.find("x").unwrap(), t.find("y").unwrap());
        let (na, _) = toggle_comment(&mut t, a, b, "//").unwrap();
        assert_eq!(t, "fn a() {\n    // x();\n\n    // y();\n}\n");
        assert_eq!(&t[na..na + 1], "x");
        let y = t.find("y").unwrap();
        toggle_comment(&mut t, a, y, "//").unwrap();
        assert_eq!(t, "fn a() {\n    x();\n\n    y();\n}\n");
        // Linha só de espaços: nada a fazer.
        let mut blank = String::from("   \n");
        assert!(toggle_comment(&mut blank, 0, 0, "#").is_none());
        // Recuos diferentes: o comentário fica no menor recuo.
        let mut mixed = String::from("a\n  b\n");
        let end = mixed.len();
        toggle_comment(&mut mixed, 0, end, "#");
        assert_eq!(mixed, "# a\n#   b\n");
    }

    #[test]
    fn bloco_de_linhas() {
        let t = "um\ndois\ntres";
        assert_eq!(line_block(t, 4, 4), (3, 7));
        // Seleção até o começo da linha seguinte não leva essa linha.
        assert_eq!(line_block(t, 0, 3), (0, 2));
        assert_eq!(line_block(t, 0, t.len()), (0, t.len()));
    }

    #[test]
    fn duplicar_e_mover() {
        let mut t = String::from("a\nb\nc");
        assert_eq!(duplicate_lines(&mut t, 2, 2), 2);
        assert_eq!(t, "a\nb\nb\nc");
        let mut t = String::from("a\nb\nc");
        assert_eq!(move_lines(&mut t, 2, 2, true), Some(-2));
        assert_eq!(t, "b\na\nc");
        assert_eq!(move_lines(&mut t, 0, 0, true), None);
        assert_eq!(move_lines(&mut t, 0, 0, false), Some(2));
        assert_eq!(t, "a\nb\nc");
        // Última linha (sem \n no fim) não desce; a penúltima desce até ela.
        assert_eq!(move_lines(&mut t, 4, 4, false), None);
        assert_eq!(move_lines(&mut t, 2, 2, false), Some(2));
        assert_eq!(t, "a\nc\nb");
        // Duas linhas juntas.
        let mut t = String::from("1\n2\n3\n4");
        assert_eq!(move_lines(&mut t, 2, 5, false), Some(2));
        assert_eq!(t, "1\n4\n2\n3");
    }

    #[test]
    fn buscar_e_substituir() {
        let t = "Abc abc ABC";
        assert_eq!(find(t, "abc", 1, true, false), Some(4));
        assert_eq!(find(t, "abc", 9, true, false), Some(0));
        assert_eq!(find(t, "abc", 5, true, true), Some(4));
        assert_eq!(find(t, "abc", 4, false, false), Some(0));
        assert_eq!(find(t, "abc", 0, false, false), Some(8));
        assert_eq!(count(t, "abc", false), 3);
        assert_eq!(count(t, "abc", true), 1);
        let mut s = t.to_string();
        assert_eq!(replace_all(&mut s, "abc", "x", false), 3);
        assert_eq!(s, "x x x");
        let mut s = String::from("aaa");
        assert_eq!(replace_all(&mut s, "a", "aa", true), 3);
        assert_eq!(s, "aaaaaa");
    }

    #[test]
    fn ir_para_linha_e_coluna() {
        assert_eq!(parse_goto(" 12 "), Some((12, 1)));
        assert_eq!(parse_goto("3:7"), Some((3, 7)));
        assert_eq!(parse_goto("x"), None);
        let t = "ação\nbê\n";
        assert_eq!(offset_of(t, 1, 3), "aç".len());
        assert_eq!(offset_of(t, 2, 1), t.find('b').unwrap());
        assert_eq!(offset_of(t, 2, 99), t.find("ê").unwrap() + "ê".len());
        assert_eq!(offset_of(t, 99, 1), t.len());
    }
}
