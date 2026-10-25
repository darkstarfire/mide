//! Layouts de teclado do mide (Configurações → Teclado e o seletor na barra superior).
//!
//! A escolha fica em `[keyboard] layouts`, uma lista separada por vírgulas de
//! `layout` ou `layout:variante` do xkb (`"br,us:intl"`). Com mais de um, o compositor
//! monta um mapa com um grupo por layout e troca entre eles (Ctrl+Super+Espaço ou pela
//! barra), sem recarregar.

/// Layouts oferecidos: nome, layout e variante do xkb.
pub const LAYOUTS: &[(&str, &str, &str)] = &[
    ("Português (Brasil, ABNT2)", "br", ""),
    ("Inglês (EUA)", "us", ""),
    ("Inglês (EUA, internacional, com acentos)", "us", "intl"),
    ("Inglês (EUA, internacional AltGr)", "us", "altgr-intl"),
    ("Inglês (Reino Unido)", "gb", ""),
    ("Português (Portugal)", "pt", ""),
    ("Espanhol (Espanha)", "es", ""),
    ("Espanhol (América Latina)", "latam", ""),
    ("Francês", "fr", ""),
    ("Alemão", "de", ""),
    ("Italiano", "it", ""),
];

/// Um layout escolhido: `layout` e `variante` (vazia para a padrão).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub layout: String,
    pub variant: String,
}

impl Choice {
    /// Como fica no `config.toml` (`us:intl`).
    pub fn id(&self) -> String {
        if self.variant.is_empty() { self.layout.clone() } else { format!("{}:{}", self.layout, self.variant) }
    }

    /// Nome para a tela (o da lista, ou o código).
    pub fn name(&self) -> String {
        LAYOUTS
            .iter()
            .find(|(_, l, v)| *l == self.layout && *v == self.variant)
            .map(|(n, ..)| n.to_string())
            .unwrap_or_else(|| self.id())
    }

    /// Sigla para a barra ("BR", "US", "US-I" para o internacional).
    pub fn short(&self) -> String {
        let base = self.layout.to_uppercase();
        match self.variant.as_str() {
            "" => base,
            "intl" | "altgr-intl" => format!("{base}-I"),
            other => format!("{base}-{}", other.chars().next().unwrap_or(' ').to_uppercase()),
        }
    }
}

/// Lê a lista do `config.toml` (`"br,us:intl"`).
pub fn parse(text: &str) -> Vec<Choice> {
    text.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| {
            let (layout, variant) = s.split_once(':').unwrap_or((s, ""));
            Choice { layout: layout.trim().to_string(), variant: variant.trim().to_string() }
        })
        .collect()
}

/// Escreve a lista para o `config.toml`.
pub fn join(list: &[Choice]) -> String {
    list.iter().map(Choice::id).collect::<Vec<_>>().join(",")
}

/// Os layouts escolhidos nas Configurações (vazio: o do sistema). Aceita também o
/// formato antigo, de um layout só (`layout` e `variant`).
pub fn configured() -> Vec<Choice> {
    use crate::settings_file::{cached, Value};
    if let Some(Value::Text(list)) = cached("keyboard", "layouts") {
        return parse(&list);
    }
    match (cached("keyboard", "layout"), cached("keyboard", "variant")) {
        (Some(Value::Text(layout)), variant) if !layout.is_empty() => {
            let variant = match variant {
                Some(Value::Text(v)) => v,
                _ => String::new(),
            };
            vec![Choice { layout, variant }]
        }
        _ => Vec::new(),
    }
}

/// Layout do sistema (`/etc/default/keyboard`), o primeiro da lista quando o usuário
/// acrescenta outro; "us" se não der para ler.
pub fn system() -> Choice {
    let text = std::fs::read_to_string("/etc/default/keyboard").unwrap_or_default();
    let value = |key: &str| {
        text.lines()
            .filter_map(|l| l.trim().strip_prefix(key)?.strip_prefix('='))
            .map(|v| v.trim().trim_matches('"').trim_matches('\'').split(',').next().unwrap_or("").to_string())
            .next()
            .unwrap_or_default()
    };
    let layout = value("XKBLAYOUT");
    Choice { layout: if layout.is_empty() { "us".into() } else { layout }, variant: value("XKBVARIANT") }
}

/// O seletor aparece na barra: mais de um layout e a opção não desligada.
pub fn show_in_bar() -> bool {
    configured().len() > 1 && crate::settings_file::flag("keyboard", "show_layout", true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lista_de_layouts() {
        let list = parse("br, us:intl,,de");
        assert_eq!(list.len(), 3);
        assert_eq!(list[1], Choice { layout: "us".into(), variant: "intl".into() });
        assert_eq!(join(&list), "br,us:intl,de");
        assert_eq!(list[0].short(), "BR");
        assert_eq!(list[1].short(), "US-I");
        assert_eq!(list[1].name(), "Inglês (EUA, internacional, com acentos)");
    }
}
