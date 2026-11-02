//! Configurações do Zed (`~/.config/zed/settings.json` e `keymap.json`) lidas pelo
//! Code, para quem vem do Zed não configurar tudo de novo. Só os formatos da
//! documentação do Zed servem de referência; nada do código dele é usado.

use std::path::PathBuf;

use serde_json::Value;

use crate::code::config::{Config, Server};

/// Pasta das configurações do Zed (`$XDG_CONFIG_HOME/zed` ou `~/.config/zed`).
pub fn config_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()).map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("zed"))
}

/// O `settings.json` do Zed, se existe e dá para ler.
pub fn load() -> Option<Value> {
    let text = std::fs::read_to_string(config_dir()?.join("settings.json")).ok()?;
    crate::code::jsonc::parse(&text).filter(Value::is_object)
}

/// Nome de um servidor no Zed para o programa que o mide roda (os que têm outro
/// nome no PATH).
pub fn server_command(zed_name: &str) -> &str {
    match zed_name {
        "json-language-server" => "vscode-json-language-server",
        "vscode-html-language-server" | "html-language-server" => "vscode-html-language-server",
        "vscode-css-language-server" | "css-language-server" => "vscode-css-language-server",
        "pyright" => "pyright-langserver",
        "basedpyright" => "basedpyright-langserver",
        "typescript-language-server" | "ts_ls" => "typescript-language-server",
        "eslint" => "vscode-eslint-language-server",
        "docker-langserver" | "dockerfile-language-server" => "docker-langserver",
        "omnisharp" => "OmniSharp",
        "kotlin-language-server" => "kotlin-language-server",
        other => other,
    }
}

/// Id do mide para o nome de uma linguagem no Zed ("PHP", "Shell Script", "C++").
pub fn language_id(zed_name: &str) -> Option<&'static str> {
    let n = zed_name.to_lowercase();
    let special = match n.as_str() {
        "shell script" | "bash" | "sh" => Some("shellscript"),
        "c++" => Some("cpp"),
        "c#" => Some("csharp"),
        "tsx" => Some("typescript"),
        "jsonc" => Some("json"),
        _ => None,
    };
    special.or_else(|| crate::code::lang::LANGUAGES.iter().find(|l| l.name.to_lowercase() == n || l.id == n).map(|l| l.id))
}

/// Aplica as configurações do Zed: servidores sempre; o editor (fonte, recuo,
/// formatar ao salvar...) só com `first_run`.
pub fn apply(config: &mut Config, zed: &Value, first_run: bool) {
    // `lsp`: binário, argumentos, ambiente, `initialization_options` e `settings` de
    // cada servidor.
    if let Some(lsp) = zed["lsp"].as_object() {
        for (name, entry) in lsp {
            let command = server_command(name).to_string();
            let path = entry["binary"]["path"].as_str().map(str::to_string);
            let args: Option<Vec<String>> = entry["binary"]["arguments"].as_array().map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect());
            let env: Vec<(String, String)> = entry["binary"]["env"].as_object().map(|o| o.iter().filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_string()))).collect()).unwrap_or_default();
            let matches = |s: &Server| s.command == command || s.command.rsplit('/').next() == Some(command.as_str()) || path.as_deref().is_some_and(|p| p == s.command);
            if let Some(server) = config.servers.iter_mut().find(|s| matches(s)) {
                if let Some(p) = &path {
                    server.command = p.clone();
                }
                if let Some(a) = &args {
                    server.args = a.clone();
                }
                if !env.is_empty() {
                    server.env = env.clone();
                }
                server.settings = entry["settings"].clone();
                server.init_options = entry["initialization_options"].clone();
            } else if let Some(p) = &path {
                // Servidor que o mide não conhece, com o caminho dado: entra para as
                // linguagens que o `languages` do Zed liga a ele.
                let langs: Vec<String> = zed["languages"]
                    .as_object()
                    .map(|o| o.iter().filter(|(_, v)| v["language_servers"].as_array().is_some_and(|a| a.iter().any(|x| x.as_str() == Some(name)))).filter_map(|(l, _)| language_id(l).map(str::to_string)).collect())
                    .unwrap_or_default();
                if !langs.is_empty() {
                    let mut s = Server::new(&[], p, &[]);
                    s.languages = langs;
                    s.args = args.clone().unwrap_or_default();
                    s.env = env.clone();
                    s.settings = entry["settings"].clone();
                    s.init_options = entry["initialization_options"].clone();
                    config.servers.push(s);
                }
            }
        }
    }
    // `languages.<Nome>.language_servers`: a ordem de preferência, `!nome` desliga e
    // `...` são os outros.
    if let Some(langs) = zed["languages"].as_object() {
        for (lang_name, entry) in langs {
            let Some(id) = language_id(lang_name) else {
                continue;
            };
            let Some(list) = entry["language_servers"].as_array() else {
                continue;
            };
            let names: Vec<&str> = list.iter().filter_map(Value::as_str).collect();
            for name in names.iter().filter_map(|n| n.strip_prefix('!')) {
                let command = server_command(name);
                for s in config.servers.iter_mut().filter(|s| s.command.ends_with(command) && s.languages.iter().any(|l| l == id)) {
                    s.languages.retain(|l| l != id);
                }
            }
            // Os escolhidos para a frente da lista, na ordem dada.
            let wanted: Vec<&str> = names.iter().filter(|n| !n.starts_with('!') && **n != "...").map(|n| server_command(n)).collect();
            for command in wanted.iter().rev() {
                if let Some(i) = config.servers.iter().position(|s| s.command.ends_with(command) && s.languages.iter().any(|l| l == id)) {
                    let s = config.servers.remove(i);
                    config.servers.insert(0, s);
                }
            }
        }
    }
    if !first_run {
        return;
    }
    if let Some(font) = zed["buffer_font_family"].as_str().filter(|f| !f.is_empty() && *f != "Zed Plex Mono" && *f != ".ZedMono") {
        config.font = Some(font.to_string());
    }
    if let Some(size) = zed["buffer_font_size"].as_f64() {
        config.size = (size as f32).clamp(9.0, 28.0);
    }
    if let Some(tab) = zed["tab_size"].as_u64() {
        config.tab_size = (tab as usize).clamp(1, 16);
    }
    if let Some(hard) = zed["hard_tabs"].as_bool() {
        config.soft_tabs = !hard;
    }
    if let Some(f) = zed["format_on_save"].as_str() {
        config.format_on_save = f == "on";
    }
    if let Some(on) = zed["inlay_hints"]["enabled"].as_bool() {
        config.inlay_hints = on;
    }
    if let Some(on) = zed["show_completions_on_input"].as_bool() {
        config.auto_complete = on;
    }
    if let Some(on) = zed["show_completion_documentation"].as_bool() {
        config.complete_docs = on;
    }
    if let Some(on) = zed["gutter"]["line_numbers"].as_bool() {
        config.line_numbers = on;
    }
    if let Some(mode) = zed["theme"]["mode"].as_str() {
        config.theme = match mode {
            "light" => "light".into(),
            "dark" => "dark".into(),
            _ => "auto".into(),
        };
    }
}

/// Atalho do `keymap.json` do Zed ("ctrl-shift-p") e a ação do mide equivalente.
#[derive(Debug, Clone, PartialEq)]
pub struct Binding {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    /// Nome da tecla em minúsculas ("p", "f2", "enter", ".").
    pub key: String,
    pub action: String,
}

/// Ação do mide para uma ação do Zed (as que o Code tem).
pub fn action_for(zed_action: &str) -> Option<&'static str> {
    Some(match zed_action {
        "command_palette::Toggle" => "palette",
        "file_finder::Toggle" => "goto-file",
        "go_to_line::Toggle" => "goto-line",
        "outline::Toggle" | "project_symbols::Toggle" => "symbols",
        "editor::Rename" => "rename",
        "editor::Format" => "format",
        "editor::ToggleCodeActions" => "code-actions",
        "editor::GoToDefinition" => "definition",
        "editor::Hover" => "hover",
        "editor::ShowCompletions" => "complete",
        "editor::ShowSignatureHelp" => "signature",
        "editor::ToggleComments" => "comment",
        "editor::DuplicateLineDown" | "editor::DuplicateSelection" => "duplicate",
        "editor::MoveLineUp" => "move-up",
        "editor::MoveLineDown" => "move-down",
        "editor::GoToDiagnostic" => "next-problem",
        "editor::GoToPreviousDiagnostic" | "editor::GoToPrevDiagnostic" => "prev-problem",
        "diagnostics::Deploy" => "problems",
        "buffer_search::Deploy" => "find",
        "pane::DeploySearch" | "workspace::NewSearch" => "find-project",
        "workspace::Save" => "save",
        "workspace::SaveAs" => "save-as",
        "workspace::SaveAll" => "save-all",
        "workspace::NewFile" => "new",
        "workspace::Open" => "open",
        "pane::CloseActiveItem" => "close-tab",
        "workspace::ToggleLeftDock" | "project_panel::ToggleFocus" => "drawer",
        "terminal_panel::ToggleFocus" | "workspace::ToggleBottomDock" => "terminal",
        "pane::SplitRight" => "split-right",
        "pane::SplitDown" => "split-down",
        "zed::IncreaseBufferFontSize" => "zoom-in",
        "zed::DecreaseBufferFontSize" => "zoom-out",
        "zed::ResetBufferFontSize" => "zoom-reset",
        "zed::OpenSettings" => "prefs",
        _ => return None,
    })
}

/// Atalhos do `keymap.json` do Zed que o Code entende (contexto do editor ou geral).
pub fn keymap() -> Vec<Binding> {
    let Some(text) = config_dir().and_then(|d| std::fs::read_to_string(d.join("keymap.json")).ok()) else {
        return Vec::new();
    };
    parse_keymap(&text)
}

pub fn parse_keymap(text: &str) -> Vec<Binding> {
    let Some(v) = crate::code::jsonc::parse(text) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for block in v.as_array().into_iter().flatten() {
        let context = block["context"].as_str().unwrap_or_default();
        // Só o editor e o geral (o resto é de painéis que o Code não tem).
        if !(context.is_empty() || context.starts_with("Editor") || context.starts_with("Workspace")) {
            continue;
        }
        for (keys, action) in block["bindings"].as_object().into_iter().flatten() {
            let name = match action {
                Value::String(a) => a.as_str(),
                Value::Array(a) => a.first().and_then(Value::as_str).unwrap_or_default(),
                _ => continue,
            };
            let Some(action) = action_for(name) else {
                continue;
            };
            // Só atalhos de uma tecla (sem sequências como "ctrl-k ctrl-s").
            if keys.contains(' ') {
                continue;
            }
            let mut b = Binding { ctrl: false, shift: false, alt: false, key: String::new(), action: action.to_string() };
            let parts: Vec<&str> = keys.split('-').collect();
            for (k, part) in parts.iter().enumerate() {
                match *part {
                    "ctrl" | "cmd" | "super" if k + 1 < parts.len() => b.ctrl = true,
                    "shift" if k + 1 < parts.len() => b.shift = true,
                    "alt" if k + 1 < parts.len() => b.alt = true,
                    // `ctrl--` termina em "-" (a tecla é o próprio hífen).
                    "" => b.key = "-".into(),
                    key => b.key = key.to_lowercase(),
                }
            }
            if !b.key.is_empty() {
                out.push(b);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn servidores_e_editor_do_zed() {
        let zed = crate::code::jsonc::parse(
            r#"{
            // comentário
            "buffer_font_size": 15,
            "tab_size": 2,
            "format_on_save": "on",
            "lsp": {
                "intelephense": { "binary": { "path": "/opt/intelephense", "arguments": ["--stdio"] }, "initialization_options": { "licenceKey": "x" }, "settings": { "intelephense": { "files": { "maxSize": 5 } } } }
            },
            "languages": { "PHP": { "language_servers": ["phpactor", "!intelephense", "..."] } },
        }"#,
        )
        .unwrap();
        let mut config = Config::default();
        apply(&mut config, &zed, true);
        assert_eq!(config.size, 15.0);
        assert_eq!(config.tab_size, 2);
        assert!(config.format_on_save);
        let inte = config.servers.iter().find(|s| s.command == "/opt/intelephense").unwrap();
        assert_eq!(inte.init_options["licenceKey"], "x");
        // `!intelephense` tira o PHP dele; o phpactor vem primeiro.
        assert!(!inte.languages.iter().any(|l| l == "php"));
        assert_eq!(config.servers[0].command, "phpactor");
        // Sem `first_run`, o editor não muda.
        let mut other = Config::default();
        apply(&mut other, &zed, false);
        assert_eq!(other.size, Config::default().size);
    }

    #[test]
    fn atalhos_do_zed() {
        let b = parse_keymap(r#"[{ "context": "Editor", "bindings": { "ctrl-shift-r": "editor::Rename", "ctrl-k ctrl-s": "zed::OpenKeymap", "alt-enter": ["editor::ToggleCodeActions", {}] } }, { "context": "Terminal", "bindings": { "ctrl-p": "command_palette::Toggle" } }]"#);
        assert_eq!(b.len(), 2);
        assert!(b.iter().any(|x| x.ctrl && x.shift && x.key == "r" && x.action == "rename"));
        assert!(b.iter().any(|x| x.alt && x.key == "enter" && x.action == "code-actions"));
        assert_eq!(language_id("Shell Script"), Some("shellscript"));
        assert_eq!(language_id("PHP"), Some("php"));
    }
}
