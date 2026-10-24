//! Leitor e escritor de XML mínimos (para o DOCX e o ODT), escritos aqui: elementos,
//! atributos, texto, entidades, comentários, CDATA e instruções.

/// Um elemento: nome com o prefixo ("w:p"), atributos e filhos.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Element {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Node>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Element(Element),
    Text(String),
}

impl Element {
    /// O valor do atributo (pelo nome completo, "w:val").
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    /// Os filhos que são elementos.
    pub fn elements(&self) -> impl Iterator<Item = &Element> {
        self.children.iter().filter_map(|n| match n {
            Node::Element(e) => Some(e),
            Node::Text(_) => None,
        })
    }

    /// O primeiro filho com o nome.
    pub fn child(&self, name: &str) -> Option<&Element> {
        self.elements().find(|e| e.name == name)
    }

    /// Descendente pelo caminho de nomes ("w:body/w:p").
    pub fn path(&self, path: &str) -> Option<&Element> {
        path.split('/').try_fold(self, |e, part| e.child(part))
    }

    /// Todos os descendentes com o nome (em ordem), sem entrar neles.
    pub fn find_all<'a>(&'a self, name: &str, out: &mut Vec<&'a Element>) {
        for e in self.elements() {
            if e.name == name {
                out.push(e);
            } else {
                e.find_all(name, out);
            }
        }
    }

    /// O texto de dentro (todos os níveis).
    pub fn text(&self) -> String {
        let mut s = String::new();
        for n in &self.children {
            match n {
                Node::Text(t) => s.push_str(t),
                Node::Element(e) => s.push_str(&e.text()),
            }
        }
        s
    }
}

fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest.find(';') else {
            out.push_str(rest);
            return out;
        };
        let ent = &rest[1..end];
        let ch = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            e if e.starts_with("#x") || e.starts_with("#X") => u32::from_str_radix(&e[2..], 16).ok().and_then(char::from_u32),
            e if e.starts_with('#') => e[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match ch {
            Some(c) => out.push(c),
            None => out.push_str(&rest[..=end]),
        }
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    out
}

/// Escapa texto para dentro de um elemento ou atributo.
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            // Caracteres de controle não são permitidos no XML (só tab, \n e \r).
            c if (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') => {}
            c => out.push(c),
        }
    }
    out
}

/// Lê o XML e devolve o elemento raiz.
pub fn parse(src: &str) -> Option<Element> {
    let mut stack: Vec<Element> = vec![Element::default()];
    let mut rest = src;
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix("<!--") {
            rest = r.find("-->").map_or("", |i| &r[i + 3..]);
        } else if let Some(r) = rest.strip_prefix("<![CDATA[") {
            let end = r.find("]]>").unwrap_or(r.len());
            stack.last_mut()?.children.push(Node::Text(r[..end].to_string()));
            rest = r.get(end + 3..).unwrap_or("");
        } else if rest.starts_with("<?") || rest.starts_with("<!") {
            rest = rest.find('>').map_or("", |i| &rest[i + 1..]);
        } else if let Some(r) = rest.strip_prefix("</") {
            let end = r.find('>')?;
            rest = &r[end + 1..];
            let done = stack.pop()?;
            if stack.is_empty() {
                return Some(done);
            }
            stack.last_mut()?.children.push(Node::Element(done));
        } else if let Some(r) = rest.strip_prefix('<') {
            // Fim da tag, fora de aspas.
            let mut quote = None;
            let mut end = None;
            for (i, c) in r.char_indices() {
                match (quote, c) {
                    (None, '"' | '\'') => quote = Some(c),
                    (Some(q), c) if c == q => quote = None,
                    (None, '>') => {
                        end = Some(i);
                        break;
                    }
                    _ => {}
                }
            }
            let end = end?;
            let mut inner = &r[..end];
            rest = &r[end + 1..];
            let self_closing = inner.ends_with('/');
            if self_closing {
                inner = &inner[..inner.len() - 1];
            }
            let name_end = inner.find(|c: char| c.is_whitespace()).unwrap_or(inner.len());
            let mut el = Element { name: inner[..name_end].to_string(), ..Default::default() };
            let mut a = inner[name_end..].trim_start();
            while let Some(eq) = a.find('=') {
                let key = a[..eq].trim().to_string();
                let after = a[eq + 1..].trim_start();
                let q = after.chars().next()?;
                let close = after[1..].find(q)? + 1;
                el.attrs.push((key, unescape(&after[1..close])));
                a = after[close + 1..].trim_start();
            }
            if self_closing {
                stack.last_mut()?.children.push(Node::Element(el));
            } else {
                stack.push(el);
            }
        } else {
            let end = rest.find('<').unwrap_or(rest.len());
            let text = &rest[..end];
            if !text.is_empty() {
                stack.last_mut()?.children.push(Node::Text(unescape(text)));
            }
            rest = &rest[end..];
        }
    }
    // Sem fechar a raiz: devolve o que foi lido.
    let root = stack.into_iter().next()?;
    root.elements().next().cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_xml_com_prefixos_e_entidades() {
        let x = r#"<?xml version="1.0"?><!-- c --><w:document a="1"><w:body><w:p><w:r><w:t xml:space="preserve"> Olá &amp; &#233;&lt;</w:t></w:r><w:br/></w:p></w:body></w:document>"#;
        let root = parse(x).expect("xml");
        assert_eq!(root.name, "w:document");
        assert_eq!(root.attr("a"), Some("1"));
        let p = root.path("w:body/w:p").expect("p");
        assert_eq!(p.text(), " Olá & é<");
        assert!(p.child("w:br").is_some());
        assert_eq!(escape("a<b&\"c\""), "a&lt;b&amp;&quot;c&quot;");
    }
}
