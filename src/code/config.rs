//! Preferências do `mide-code`, editadas na janela de Preferências (nunca à mão):
//! fonte, tamanho, números de linha, linha atual, tabulação, tema e os servidores de
//! linguagem (LSP), cada um com o comando, os argumentos e as linguagens que atende.
//!
//! Ficam em `~/.config/mide/code.toml` (respeitando `XDG_CONFIG_HOME`).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Um servidor de linguagem.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Server {
    /// Linguagens atendidas (ids do LSP: `rust`, `python`...).
    pub languages: Vec<String>,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Família da fonte de largura fixa (`None`: a do sistema).
    pub font: Option<String>,
    pub size: f32,
    pub line_numbers: bool,
    pub current_line: bool,
    /// Espaços por recuo; com `soft_tabs` falso, a tecla Tab insere `\t`.
    pub tab_size: usize,
    pub soft_tabs: bool,
    /// "auto" (segue o sistema), "light" ou "dark".
    pub theme: String,
    /// Mostrar a barra lateral com as pastas do projeto.
    pub drawer: bool,
    pub servers: Vec<Server>,
}

impl Default for Config {
    fn default() -> Self {
        let server = |langs: &[&str], command: &str, args: &[&str]| Server { languages: langs.iter().map(|s| s.to_string()).collect(), command: command.into(), args: args.iter().map(|s| s.to_string()).collect(), enabled: true };
        Self {
            font: None,
            size: 13.0,
            line_numbers: true,
            current_line: true,
            tab_size: 4,
            soft_tabs: true,
            theme: "auto".into(),
            drawer: true,
            servers: vec![
                server(&["rust"], "rust-analyzer", &[]),
                server(&["go"], "gopls", &[]),
                server(&["python"], "pylsp", &[]),
                server(&["c", "cpp"], "clangd", &[]),
                server(&["javascript", "typescript"], "typescript-language-server", &["--stdio"]),
                server(&["shellscript"], "bash-language-server", &["start"]),
            ],
        }
    }
}

impl Config {
    pub fn path() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        Some(base.join("mide").join("code.toml"))
    }

    pub fn load() -> Self {
        Self::path().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| toml::from_str(&t).ok()).unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        let path = Self::path().ok_or_else(|| std::io::Error::other("sem pasta de configuração"))?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let body = toml::to_string(self).map_err(std::io::Error::other)?;
        crate::files::write_atomic(&path, format!("# Preferências do Code do mide (mudadas pela janela de Preferências).\n\n{body}").as_bytes())
    }

    /// Recuo de uma tecla Tab.
    pub fn indent(&self) -> String {
        if self.soft_tabs { " ".repeat(self.tab_size.clamp(1, 16)) } else { "\t".into() }
    }

    /// Servidor ligado para uma linguagem (o primeiro da lista que a atende).
    pub fn server_for(&self, language: &str) -> Option<&Server> {
        self.servers.iter().find(|s| s.enabled && s.languages.iter().any(|l| l == language))
    }
}

/// O programa existe no PATH (ou é um caminho executável)?
pub fn installed(command: &str) -> bool {
    if command.contains('/') {
        return std::path::Path::new(command).is_file();
    }
    std::env::var_os("PATH").is_some_and(|paths| std::env::split_paths(&paths).any(|d| d.join(command).is_file()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn padrao_e_ida_e_volta() {
        let c = Config::default();
        assert_eq!(c.server_for("cpp").map(|s| s.command.as_str()), Some("clangd"));
        assert_eq!(c.indent(), "    ");
        let text = toml::to_string(&c).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(back, c);
        // Arquivo antigo, só com o tamanho: o resto vem do padrão.
        let partial: Config = toml::from_str("size = 15.0").unwrap();
        assert_eq!(partial.size, 15.0);
        assert!(partial.line_numbers);
        let tabs = Config { soft_tabs: false, ..Config::default() };
        assert_eq!(tabs.indent(), "\t");
    }
}
