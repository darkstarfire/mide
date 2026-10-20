//! Escrita no `~/.config/mide/config.toml` pelas Configurações do sistema,
//! mantendo os comentários e a formatação que o usuário escreveu.

use std::{fs, path::PathBuf};

use toml_edit::{value, DocumentMut, Item, Table};

/// Caminho de `config.toml`, respeitando `XDG_CONFIG_HOME`.
pub fn config_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("mide").join("config.toml"))
}

/// Valor que dá para gravar.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
}

/// Grava `chave = valor` na seção, num texto TOML, sem mexer no resto. `None` em
/// `new` apaga a chave (volta ao padrão).
pub fn set_in_text(text: &str, section: &str, key: &str, new: Option<Value>) -> Result<String, String> {
    let mut doc: DocumentMut = text.parse().map_err(|e| format!("config.toml inválido: {e}"))?;
    if doc.get(section).is_none_or(|i| !i.is_table()) {
        doc[section] = Item::Table(Table::new());
    }
    let table = doc[section].as_table_mut().ok_or("seção inválida")?;
    match new {
        Some(Value::Bool(b)) => table[key] = value(b),
        Some(Value::Int(n)) => table[key] = value(n),
        Some(Value::Float(x)) => table[key] = value(x),
        Some(Value::Text(t)) => table[key] = value(t),
        None => {
            table.remove(key);
        }
    }
    // Seção que ficou vazia sai do arquivo.
    if doc[section].as_table().is_some_and(|t| t.is_empty()) {
        doc.remove(section);
    }
    Ok(doc.to_string())
}

/// Lê um valor da seção num texto TOML.
pub fn get_in_text(text: &str, section: &str, key: &str) -> Option<Value> {
    let doc: DocumentMut = text.parse().ok()?;
    let item = doc.get(section)?.get(key)?;
    if let Some(b) = item.as_bool() {
        return Some(Value::Bool(b));
    }
    if let Some(n) = item.as_integer() {
        return Some(Value::Int(n));
    }
    if let Some(x) = item.as_float() {
        return Some(Value::Float(x));
    }
    item.as_str().map(|s| Value::Text(s.to_string()))
}

/// Grava no arquivo do usuário (cria o arquivo se não existir).
pub fn set(section: &str, key: &str, new: Option<Value>) -> Result<(), String> {
    let path = config_path().ok_or("sem pasta de configuração")?;
    let text = fs::read_to_string(&path).unwrap_or_default();
    let updated = set_in_text(&text, section, key, new)?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    fs::write(&path, updated).map_err(|e| e.to_string())?;
    tracing::info!("config.toml: [{section}] {key} atualizado");
    Ok(())
}

/// Apaga a seção inteira do arquivo do usuário (volta tudo dela ao padrão).
pub fn clear_section(section: &str) -> Result<(), String> {
    let path = config_path().ok_or("sem pasta de configuração")?;
    let Ok(text) = fs::read_to_string(&path) else {
        return Ok(());
    };
    fs::write(&path, clear_section_in_text(&text, section)?).map_err(|e| e.to_string())?;
    tracing::info!("config.toml: [{section}] voltou ao padrão");
    Ok(())
}

/// Tira a seção de um texto TOML, sem mexer no resto.
pub fn clear_section_in_text(text: &str, section: &str) -> Result<String, String> {
    let mut doc: DocumentMut = text.parse().map_err(|e| format!("config.toml inválido: {e}"))?;
    doc.remove(section);
    Ok(doc.to_string())
}

/// Lê um valor do arquivo do usuário.
pub fn get(section: &str, key: &str) -> Option<Value> {
    let text = fs::read_to_string(config_path()?).ok()?;
    get_in_text(&text, section, key)
}

/// Um efeito de `[effects]` está ligado? (Os efeitos vêm ligados quando faltam.)
/// Pode ser chamado a cada quadro (ver `flag`).
pub fn effect(key: &str) -> bool {
    flag("effects", key, true)
}

/// Itens da direita da barra superior que podem ser escondidos (`[bar]` no
/// `config.toml`): chave, nome e explicação. O relógio fica sempre.
pub const BAR_ITEMS: &[(&str, &str, &str)] = &[
    ("gear", "Configurações", "Engrenagem que abre as Configurações do Sistema"),
    ("sound", "Som", "Volume e saída de som"),
    ("wifi", "Wi-Fi", "Redes sem fio"),
    ("bluetooth", "Bluetooth", "Aparelhos sem fio"),
    ("battery", "Bateria", "Carga e tempo restante (só em notebooks)"),
    ("control", "Central de controle", "Wi-Fi, Bluetooth, Não perturbe, brilho e som num painel só"),
];

/// Um item da barra superior está à mostra? (Todos aparecem quando faltam.)
pub fn bar_item(key: &str) -> bool {
    flag("bar", key, true)
}

/// Opção ligada/desligada de `[section]`, com o valor padrão quando falta. Guarda o
/// arquivo lido e só relê quando ele muda: pode ser chamado a cada quadro.
pub fn flag(section: &str, key: &str, default: bool) -> bool {
    use std::sync::Mutex;
    type Cache = Option<(Option<std::time::SystemTime>, String)>;
    static CACHE: Mutex<Cache> = Mutex::new(None);
    let path = config_path();
    let mtime = path.as_ref().and_then(|p| fs::metadata(p).ok()?.modified().ok());
    let Ok(mut cache) = CACHE.lock() else {
        return default;
    };
    if cache.as_ref().is_none_or(|(m, _)| *m != mtime) {
        let text = path.and_then(|p| fs::read_to_string(p).ok()).unwrap_or_default();
        *cache = Some((mtime, text));
    }
    let text = cache.as_ref().map(|(_, t)| t.as_str()).unwrap_or("");
    match get_in_text(text, section, key) {
        Some(Value::Bool(on)) => on,
        _ => default,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grava_mantendo_comentarios() {
        let text = "# Meus atalhos\n[keybindings]\n\"ctrl+shift+t\" = \"launch:foot\" # terminal\n";
        let out = set_in_text(text, "effects", "lamp", Some(Value::Bool(false))).unwrap();
        assert!(out.contains("# Meus atalhos"));
        assert!(out.contains("# terminal"));
        assert!(out.contains("[effects]"));
        assert_eq!(get_in_text(&out, "effects", "lamp"), Some(Value::Bool(false)));
        // Trocar o valor de novo e apagar.
        let out = set_in_text(&out, "effects", "lamp", Some(Value::Bool(true))).unwrap();
        assert_eq!(get_in_text(&out, "effects", "lamp"), Some(Value::Bool(true)));
        let out = set_in_text(&out, "effects", "lamp", None).unwrap();
        assert_eq!(get_in_text(&out, "effects", "lamp"), None);
        assert!(!out.contains("[effects]"));
    }

    #[test]
    fn limpar_secao_mantem_o_resto() {
        let text = "[effects]\nlamp = false\n\n[keybindings]\n\"ctrl+shift+q\" = \"none\"\n";
        let out = clear_section_in_text(text, "keybindings").unwrap();
        assert!(!out.contains("keybindings"));
        assert_eq!(get_in_text(&out, "effects", "lamp"), Some(Value::Bool(false)));
    }

    #[test]
    fn numero_inteiro() {
        let out = set_in_text("", "dock", "icon_size", Some(Value::Int(48))).unwrap();
        assert_eq!(get_in_text(&out, "dock", "icon_size"), Some(Value::Int(48)));
    }

    #[test]
    fn texto_e_arquivo_vazio() {
        let out = set_in_text("", "desktop", "wallpaper", Some(Value::Text("/img/a.jpg".into()))).unwrap();
        assert_eq!(get_in_text(&out, "desktop", "wallpaper"), Some(Value::Text("/img/a.jpg".into())));
    }

    #[test]
    fn toml_invalido_nao_e_sobrescrito() {
        assert!(set_in_text("isso = = errado", "effects", "lamp", Some(Value::Bool(true))).is_err());
    }
}
