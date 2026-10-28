//! Preferências do `mide-code`, editadas na janela de Preferências: fonte, tamanho,
//! números de linha, linha atual, tabulação, tema e a largura da gaveta, em
//! `~/.config/mide/code.toml` (respeitando `XDG_CONFIG_HOME`).
//!
//! Os servidores de linguagem (LSP), cada um com o comando, os argumentos e as
//! linguagens que atende, ficam num arquivo próprio, `~/.config/mide/code-servers.toml`,
//! que as Preferências abrem no próprio Code (pedido do usuário, E11): ao salvar, o Code
//! lê de novo e reinicia os servidores. Sem esse arquivo valem os servidores antigos do
//! `code.toml` (versões antes do E11) ou os padrões.

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
    /// Largura da gaveta (arrastando a divisória).
    pub drawer_width: f32,
    /// Altura do terminal integrado (arrastando a divisória).
    pub terminal_height: f32,
    /// Servidores de linguagem: lidos de `code-servers.toml` (só lidos daqui em
    /// arquivos antigos; não são mais gravados no `code.toml`).
    #[serde(skip_serializing)]
    pub servers: Vec<Server>,
}

/// O arquivo dos servidores: uma tabela `[[server]]` para cada um.
#[derive(Debug, Default, Serialize, Deserialize)]
struct ServersFile {
    #[serde(default)]
    server: Vec<Server>,
}

/// Menor e maior largura da gaveta.
pub const DRAWER_MIN: f32 = 150.0;
pub const DRAWER_MAX: f32 = 600.0;

impl Default for Config {
    fn default() -> Self {
        Self {
            font: None,
            size: 13.0,
            line_numbers: true,
            current_line: true,
            tab_size: 4,
            soft_tabs: true,
            theme: "auto".into(),
            drawer: true,
            drawer_width: 230.0,
            terminal_height: 220.0,
            servers: default_servers(),
        }
    }
}

/// Servidores conhecidos (cada um só roda se estiver instalado).
pub fn default_servers() -> Vec<Server> {
    let server = |langs: &[&str], command: &str, args: &[&str]| Server { languages: langs.iter().map(|s| s.to_string()).collect(), command: command.into(), args: args.iter().map(|s| s.to_string()).collect(), enabled: true };
    vec![
        server(&["rust"], "rust-analyzer", &[]),
        server(&["go"], "gopls", &[]),
        server(&["python"], "pylsp", &[]),
        server(&["c", "cpp"], "clangd", &[]),
        server(&["javascript", "typescript"], "typescript-language-server", &["--stdio"]),
        server(&["shellscript"], "bash-language-server", &["start"]),
    ]
}

impl Config {
    pub fn path() -> Option<PathBuf> {
        Some(config_dir()?.join("code.toml"))
    }

    /// Arquivo dos servidores de linguagem (`code-servers.toml`).
    pub fn servers_path() -> Option<PathBuf> {
        Some(config_dir()?.join("code-servers.toml"))
    }

    /// Preferências e servidores (estes do `code-servers.toml`, se existir e estiver
    /// certo).
    pub fn load() -> Self {
        let mut config: Self = Self::path().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| toml::from_str(&t).ok()).unwrap_or_default();
        if let Some(Ok(servers)) = Self::servers_path().and_then(|p| std::fs::read_to_string(p).ok()).map(|t| parse_servers(&t)) {
            config.servers = servers;
        }
        config.drawer_width = config.drawer_width.clamp(DRAWER_MIN, DRAWER_MAX);
        config
    }

    /// Garante que o arquivo dos servidores existe (com os servidores atuais e a
    /// explicação no topo) e devolve o caminho dele, para abrir no editor.
    pub fn ensure_servers_file(&self) -> std::io::Result<PathBuf> {
        let path = Self::servers_path().ok_or_else(|| std::io::Error::other("sem pasta de configuração"))?;
        if !path.exists() {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            crate::files::write_atomic(&path, servers_text(&self.servers).as_bytes())?;
        }
        Ok(path)
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

/// Pasta de configuração do mide (`$XDG_CONFIG_HOME/mide` ou `~/.config/mide`).
fn config_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("mide"))
}

/// Lê o texto do `code-servers.toml`. O erro vem no idioma da sessão, com a linha
/// (para a barra de status do Code mostrar ao salvar).
pub fn parse_servers(text: &str) -> Result<Vec<Server>, String> {
    let file: ServersFile = toml::from_str(text).map_err(|e| {
        let line = e.span().map(|s| text[..s.start.min(text.len())].matches('\n').count() + 1);
        let what = e.message().to_string();
        match line {
            Some(l) => crate::tr!("line {line}: {error}", line = l, error = what),
            None => what,
        }
    })?;
    for (i, s) in file.server.iter().enumerate() {
        if s.command.trim().is_empty() {
            return Err(crate::tr!("server {n} has no \"command\"", n = i + 1));
        }
        if s.languages.is_empty() {
            return Err(crate::tr!("server {n} ({command}) has no \"languages\"", n = i + 1, command = s.command));
        }
    }
    Ok(file.server)
}

/// Texto do `code-servers.toml` com os servidores dados e a explicação no topo.
pub fn servers_text(servers: &[Server]) -> String {
    let body = toml::to_string(&ServersFile { server: servers.to_vec() }).unwrap_or_default();
    format!(
        "# Servidores de linguagem (LSP) do Code do mide.\n\
         #\n\
         # Um bloco [[server]] para cada servidor:\n\
         #   languages: linguagens atendidas (ids do LSP: rust, go, python, c, cpp,\n\
         #              javascript, typescript, shellscript, ...)\n\
         #   command:   o programa (no PATH ou o caminho completo)\n\
         #   args:      argumentos (ex.: [\"--stdio\"])\n\
         #   enabled:   false desliga sem apagar\n\
         #\n\
         # Ao salvar (Ctrl+S), o Code lê este arquivo de novo e reinicia os servidores.\n\
         # Para uma linguagem vale o primeiro servidor ligado da lista.\n\n{body}"
    )
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
        // Os servidores não vão mais para o code.toml (têm arquivo próprio).
        assert!(!text.contains("rust-analyzer"));
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(back, Config { servers: default_servers(), ..c.clone() });
        // Arquivo antigo, só com o tamanho: o resto vem do padrão.
        let partial: Config = toml::from_str("size = 15.0").unwrap();
        assert_eq!(partial.size, 15.0);
        assert!(partial.line_numbers);
        let tabs = Config { soft_tabs: false, ..Config::default() };
        assert_eq!(tabs.indent(), "\t");
    }

    #[test]
    fn arquivo_dos_servidores() {
        let servers = vec![Server { languages: vec!["rust".into()], command: "/tmp/falso.py".into(), args: vec!["--stdio".into()], enabled: true }];
        let text = servers_text(&servers);
        assert!(text.starts_with("# Servidores de linguagem"));
        assert_eq!(parse_servers(&text).unwrap(), servers);
        // Sem `args` nem `enabled`: os padrões.
        let short = parse_servers("[[server]]\nlanguages = [\"go\"]\ncommand = \"gopls\"\n").unwrap();
        assert!(short[0].enabled && short[0].args.is_empty());
        // Erros com a linha.
        let err = parse_servers("[[server]]\nlanguages = [\"go\"]\ncommand = gopls\n").unwrap_err();
        assert!(err.contains(" 3: "), "{err}");
        assert!(parse_servers("[[server]]\nlanguages = []\ncommand = \"x\"\n").unwrap_err().contains("languages"));
        // Arquivo vazio: nenhum servidor.
        assert!(parse_servers("").unwrap().is_empty());
    }
}
