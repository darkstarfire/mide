//! As configurações do Code num arquivo que se edita à mão, como o `settings.json` do
//! Zed e do VS Code: `~/.config/mide/code-settings.json` (JSON com comentários). As
//! chaves são as do Zed quando ele tem a opção (`buffer_font_size`, `tab_size`, `lsp`,
//! `languages`...), para dar para copiar trechos de um para o outro. O arquivo vem
//! com todas as opções e os valores padrão (inclusive todos os servidores de
//! linguagem); as Preferências gravam nele, e o que se escreve nele vale ao salvar.
//! Atalhos ficam no `code-keymap.json`, no formato do `keymap.json` do Zed.

use std::path::PathBuf;

use serde_json::{json, Map, Value};

use crate::code::config::{Config, Server};

/// O arquivo das configurações.
pub fn path() -> Option<PathBuf> {
    Some(Config::path()?.with_file_name("code-settings.json"))
}

/// O arquivo dos atalhos (formato do `keymap.json` do Zed).
pub fn keymap_path() -> Option<PathBuf> {
    Some(Config::path()?.with_file_name("code-keymap.json"))
}

/// Nome de um servidor no arquivo: o programa sem a pasta (o rótulo, se tiver).
pub fn server_name(s: &Server) -> String {
    if !s.label.is_empty() {
        return s.label.clone();
    }
    s.command.rsplit('/').next().unwrap_or(&s.command).to_string()
}

/// Todas as opções, com os valores de `config` (os servidores inteiros).
pub fn to_json(config: &Config) -> Value {
    let mut lsp = Map::new();
    for s in &config.servers {
        let mut entry = json!({
            "languages": s.languages,
            "enabled": s.enabled,
            "binary": { "path": s.command, "arguments": s.args },
        });
        if !s.env.is_empty() {
            entry["binary"]["env"] = Value::Object(s.env.iter().map(|(k, v)| (k.clone(), json!(v))).collect());
        }
        if !s.init_options.is_null() {
            entry["initialization_options"] = s.init_options.clone();
        }
        if !s.settings.is_null() {
            entry["settings"] = s.settings.clone();
        }
        // O mesmo nome duas vezes (dois servidores com o mesmo programa): o segundo com
        // a linguagem no nome.
        let mut name = server_name(s);
        if lsp.contains_key(&name) {
            name = format!("{name} ({})", s.languages.join(", "));
        }
        lsp.insert(name, entry);
    }
    json!({
        "theme": match config.theme.as_str() { "light" => "light", "dark" => "dark", _ => "system" },
        "color_theme": config.color_theme,
        "icon_theme": config.icon_theme,
        "buffer_font_family": config.font,
        "buffer_font_size": config.size,
        "tab_size": config.tab_size,
        "hard_tabs": !config.soft_tabs,
        "gutter": { "line_numbers": config.line_numbers },
        "current_line_highlight": config.current_line,
        "show_completions_on_input": config.auto_complete,
        "completion_min_chars": config.complete_min_chars,
        "show_completion_documentation": config.complete_docs,
        "inlay_hints": { "enabled": config.inlay_hints },
        "format_on_save": if config.format_on_save { "on" } else { "off" },
        "project_panel": { "button": config.drawer, "default_width": config.drawer_width },
        "outline_panel": { "button": config.outline, "default_width": config.outline_width },
        "toolbar": { "breadcrumbs": config.breadcrumbs },
        "terminal": { "default_height": config.terminal_height },
        "lsp": Value::Object(lsp),
        "languages": {},
    })
}

/// Aplica o arquivo às configurações (só as chaves que existem nele).
pub fn apply(config: &mut Config, v: &Value) {
    let b = |v: &Value| v.as_bool();
    if let Some(t) = v["theme"].as_str().or_else(|| v["theme"]["mode"].as_str()) {
        config.theme = match t {
            "light" => "light".into(),
            "dark" => "dark".into(),
            _ => "auto".into(),
        };
    }
    if let Some(t) = v["color_theme"].as_str() {
        config.color_theme = t.into();
    }
    if let Some(t) = v["icon_theme"].as_str() {
        config.icon_theme = t.into();
    }
    match &v["buffer_font_family"] {
        Value::String(f) if !f.is_empty() => config.font = Some(f.clone()),
        Value::Null if v.get("buffer_font_family").is_some() => config.font = None,
        _ => {}
    }
    if let Some(n) = v["buffer_font_size"].as_f64() {
        config.size = (n as f32).clamp(8.0, 32.0);
    }
    if let Some(n) = v["tab_size"].as_u64() {
        config.tab_size = (n as usize).clamp(1, 16);
    }
    if let Some(x) = b(&v["hard_tabs"]) {
        config.soft_tabs = !x;
    }
    if let Some(x) = b(&v["gutter"]["line_numbers"]) {
        config.line_numbers = x;
    }
    if let Some(x) = b(&v["current_line_highlight"]) {
        config.current_line = x;
    }
    if let Some(x) = b(&v["show_completions_on_input"]) {
        config.auto_complete = x;
    }
    if let Some(n) = v["completion_min_chars"].as_u64() {
        config.complete_min_chars = (n as usize).clamp(1, 5);
    }
    if let Some(x) = b(&v["show_completion_documentation"]) {
        config.complete_docs = x;
    }
    if let Some(x) = b(&v["inlay_hints"]["enabled"]) {
        config.inlay_hints = x;
    }
    if let Some(f) = v["format_on_save"].as_str() {
        config.format_on_save = f == "on";
    }
    if let Some(x) = b(&v["project_panel"]["button"]) {
        config.drawer = x;
    }
    if let Some(w) = v["project_panel"]["default_width"].as_f64() {
        config.drawer_width = w as f32;
    }
    if let Some(x) = b(&v["outline_panel"]["button"]) {
        config.outline = x;
    }
    if let Some(w) = v["outline_panel"]["default_width"].as_f64() {
        config.outline_width = w as f32;
    }
    if let Some(x) = b(&v["toolbar"]["breadcrumbs"]) {
        config.breadcrumbs = x;
    }
    if let Some(h) = v["terminal"]["default_height"].as_f64() {
        config.terminal_height = h as f32;
    }
    // Servidores: a lista do arquivo, na ordem dele (a primeira instalada de cada
    // linguagem vale); os padrões que faltam entram no fim, menos para linguagens que
    // o arquivo atende com um servidor próprio.
    if let Some(lsp) = v["lsp"].as_object()
        && lsp.values().any(|e| e.get("languages").is_some())
    {
        let mut servers = Vec::new();
        for (name, e) in lsp {
            let langs: Vec<&str> = e["languages"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
            if langs.is_empty() {
                continue;
            }
            let command = e["binary"]["path"].as_str().unwrap_or(name.split(' ').next().unwrap_or(name));
            let args: Vec<&str> = e["binary"]["arguments"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
            let mut s = Server::new(&langs, command, &args);
            s.enabled = e["enabled"].as_bool().unwrap_or(true);
            s.env = e["binary"]["env"].as_object().map(|o| o.iter().filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_string()))).collect()).unwrap_or_default();
            s.init_options = e["initialization_options"].clone();
            s.settings = e["settings"].clone();
            servers.push(s);
        }
        config.servers = servers;
        config.merge_default_servers();
    }
    // `lsp` no formato do Zed (sem `languages`) e `languages.<Nome>.language_servers`.
    crate::code::zed::apply(config, v, false);
}

/// Lê o arquivo (`None` se não existe; `Err` com a linha se tem erro).
pub fn load() -> Option<Result<Value, String>> {
    let text = std::fs::read_to_string(path()?).ok()?;
    Some(parse(&text))
}

/// Lê o texto do arquivo, dizendo a linha do erro.
pub fn parse(text: &str) -> Result<Value, String> {
    let stripped = crate::code::jsonc::strip(text);
    serde_json::from_str::<Value>(&stripped).map_err(|e| crate::tr!("line {line}: {err}", line = e.line(), err = e))
}

/// Texto do arquivo: o cabeçalho explicando e todas as opções.
pub fn text(config: &Config) -> String {
    let body = serde_json::to_string_pretty(&to_json(config)).unwrap_or_else(|_| "{}".into());
    format!(
        "// Configurações do Code do mide. Edite e salve: valem na hora.\n\
         // As chaves seguem as do Zed (https://zed.dev/docs/configuring-zed) quando ele\n\
         // tem a opção, então dá para copiar trechos das configurações do Zed para cá.\n\
         // \"lsp\": servidores de linguagem (\"languages\" são os ids do Code: php,\n\
         // javascript, rust...; \"binary.path\" é o programa, achado também fora do PATH).\n\
         // \"languages\": por linguagem, como no Zed (\"language_servers\", \"tab_size\").\n\
         {body}\n"
    )
}

/// Grava as configurações (mantendo as chaves que o mide não conhece).
pub fn write(config: &Config) -> std::io::Result<PathBuf> {
    let p = path().ok_or_else(|| std::io::Error::other("sem pasta de configuração"))?;
    let mut v = to_json(config);
    if let Some(Ok(Value::Object(old))) = load()
        && let Value::Object(new) = &mut v
    {
        for (k, val) in &old {
            if !new.contains_key(k) {
                new.insert(k.clone(), val.clone());
            }
        }
        // As linguagens escritas pelo usuário ficam (o Code não as gera).
        if let Some(langs) = old.get("languages").filter(|l| l.as_object().is_some_and(|o| !o.is_empty())) {
            new.insert("languages".into(), langs.clone());
        }
    }
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let body = serde_json::to_string_pretty(&v).unwrap_or_else(|_| "{}".into());
    let full = text(config).split_once('{').map(|(head, _)| format!("{head}{body}\n")).unwrap_or(body);
    crate::files::write_atomic(&p, full.as_bytes())?;
    Ok(p)
}

/// Garante que o arquivo existe (com todas as opções) e devolve o caminho.
pub fn ensure(config: &Config) -> std::io::Result<PathBuf> {
    match path() {
        Some(p) if p.exists() => Ok(p),
        _ => write(config),
    }
}

/// Garante o arquivo de atalhos, com exemplos comentados, e devolve o caminho.
pub fn ensure_keymap() -> std::io::Result<PathBuf> {
    let p = keymap_path().ok_or_else(|| std::io::Error::other("sem pasta de configuração"))?;
    if !p.exists() {
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = "// Atalhos do Code do mide, no formato do keymap.json do Zed: as teclas\n\
                    // (\"ctrl-shift-p\", \"alt-enter\", \"f2\") e a ação do Zed que o Code tem.\n\
                    [\n  {\n    \"context\": \"Editor\",\n    \"bindings\": {\n      \"ctrl-shift-p\": \"command_palette::Toggle\",\n      \"f2\": \"editor::Rename\",\n      \"ctrl-.\": \"editor::ToggleCodeActions\",\n      \"ctrl-shift-i\": \"editor::Format\",\n      \"ctrl-r\": \"outline::Toggle\",\n      \"ctrl-t\": \"file_finder::Toggle\"\n    }\n  }\n]\n";
        crate::files::write_atomic(&p, text.as_bytes())?;
    }
    Ok(p)
}

/// Copia para as configurações do Code as do Zed (`~/.config/zed/settings.json`): o
/// editor, os servidores (`lsp`) e as linguagens. Devolve se havia o que copiar.
pub fn import_zed(config: &mut Config) -> bool {
    let Some(zed) = crate::code::zed::load() else {
        return false;
    };
    crate::code::zed::apply(config, &zed, true);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ida_e_volta_com_todas_as_opcoes() {
        let mut c = Config::default();
        c.size = 15.0;
        c.tab_size = 2;
        c.format_on_save = true;
        let v = to_json(&c);
        // Todos os servidores padrão no arquivo.
        assert_eq!(v["lsp"].as_object().unwrap().len(), c.servers.len());
        assert_eq!(v["lsp"]["intelephense"]["languages"][0], "php");
        let mut back = Config::default();
        apply(&mut back, &v);
        assert_eq!(back.size, 15.0);
        assert_eq!(back.tab_size, 2);
        assert!(back.format_on_save);
        assert_eq!(back.servers.len(), c.servers.len());
    }

    #[test]
    fn servidor_escrito_a_mao() {
        let text = r#"{
            // comentário
            "tab_size": 3,
            "lsp": { "meu-php": { "languages": ["php"], "binary": { "path": "/opt/php-ls", "arguments": ["--stdio"] } } },
        }"#;
        let v = parse(text).unwrap();
        let mut c = Config::default();
        apply(&mut c, &v);
        assert_eq!(c.tab_size, 3);
        assert_eq!(c.server_for("php").unwrap().command, "/opt/php-ls");
        assert!(parse("{ \"tab_size\": }").unwrap_err().contains('1'));
    }
}
