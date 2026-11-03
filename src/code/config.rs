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
    /// Configurações mandadas ao servidor (`workspace/configuration`), como o
    /// `lsp.<servidor>.settings` do Zed.
    #[serde(skip)]
    pub settings: serde_json::Value,
    /// `initializationOptions` do `initialize` (o `initialization_options` do Zed).
    #[serde(skip)]
    pub init_options: serde_json::Value,
    /// Variáveis de ambiente do servidor (o `binary.env` do Zed).
    #[serde(skip)]
    pub env: Vec<(String, String)>,
    /// Nome mostrado (o do servidor numa extensão, quando o programa é o `node`).
    #[serde(skip)]
    pub label: String,
}

impl Server {
    /// Chave do servidor rodando (o programa e os argumentos: dois servidores podem
    /// usar o mesmo `node`).
    /// Nome para mostrar: o dado pela extensão ou o programa.
    pub fn name(&self) -> &str {
        if self.label.is_empty() { self.command.rsplit('/').next().unwrap_or(&self.command) } else { &self.label }
    }

    pub fn key(&self) -> String {
        if self.args.is_empty() { self.command.clone() } else { format!("{} {}", self.command, self.args.join(" ")) }
    }

    pub fn new(languages: &[&str], command: &str, args: &[&str]) -> Self {
        Self {
            languages: languages.iter().map(|s| s.to_string()).collect(),
            command: command.into(),
            args: args.iter().map(|s| s.to_string()).collect(),
            enabled: true,
            settings: serde_json::Value::Null,
            init_options: serde_json::Value::Null,
            env: Vec::new(),
            label: String::new(),
        }
    }
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
    /// Completar abre sozinho enquanto se digita.
    pub auto_complete: bool,
    /// Letras digitadas antes de o completar abrir sozinho (1 a 5).
    pub complete_min_chars: usize,
    /// Documentação ao lado da sugestão escolhida.
    pub complete_docs: bool,
    /// Dicas em linha do servidor (tipos, nomes de parâmetro).
    pub inlay_hints: bool,
    /// Formatar pelo servidor antes de gravar.
    pub format_on_save: bool,
    /// Usar as configurações do Zed (`~/.config/zed/settings.json`): servidores de
    /// linguagem e, na primeira vez, fonte, recuo e as opções do editor.
    pub use_zed_settings: bool,
    /// Tema de cores do editor (de uma extensão do Zed; "": as cores do mide).
    pub color_theme: String,
    /// Tema de ícones de arquivos (de uma extensão do Zed; "": o Material embutido).
    pub icon_theme: String,
    /// Painel dos símbolos do arquivo à direita (como o outline do Neovim e do Zed).
    pub outline: bool,
    /// Largura do painel de símbolos (arrastando a divisória).
    pub outline_width: f32,
    /// Faixa com o caminho (pastas, arquivo e símbolos) em cima do texto.
    pub breadcrumbs: bool,
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
            auto_complete: true,
            complete_min_chars: 1,
            complete_docs: true,
            inlay_hints: true,
            format_on_save: false,
            use_zed_settings: true,
            color_theme: String::new(),
            icon_theme: String::new(),
            outline: true,
            outline_width: 270.0,
            breadcrumbs: true,
        }
    }
}

/// Servidores conhecidos (cada um só roda se estiver instalado).
pub fn default_servers() -> Vec<Server> {
    let server = Server::new;
    vec![
        server(&["rust"], "rust-analyzer", &[]),
        server(&["go"], "gopls", &[]),
        server(&["python"], "pylsp", &[]),
        server(&["python"], "pyright-langserver", &["--stdio"]),
        server(&["python"], "basedpyright-langserver", &["--stdio"]),
        server(&["c", "cpp"], "clangd", &[]),
        server(&["javascript", "typescript"], "typescript-language-server", &["--stdio"]),
        server(&["javascript", "typescript"], "vtsls", &["--stdio"]),
        server(&["shellscript"], "bash-language-server", &["start"]),
        server(&["php"], "intelephense", &["--stdio"]),
        server(&["php"], "phpactor", &["language-server"]),
        server(&["java"], "jdtls", &[]),
        server(&["kotlin"], "kotlin-language-server", &[]),
        server(&["swift"], "sourcekit-lsp", &[]),
        server(&["csharp"], "csharp-ls", &[]),
        server(&["csharp"], "OmniSharp", &["-lsp"]),
        server(&["html"], "vscode-html-language-server", &["--stdio"]),
        server(&["css"], "vscode-css-language-server", &["--stdio"]),
        server(&["json", "jsonc"], "vscode-json-language-server", &["--stdio"]),
        server(&["yaml"], "yaml-language-server", &["--stdio"]),
        server(&["toml"], "taplo", &["lsp", "stdio"]),
        server(&["markdown"], "marksman", &["server"]),
        server(&["lua"], "lua-language-server", &[]),
        server(&["ruby"], "ruby-lsp", &[]),
        server(&["ruby"], "solargraph", &["stdio"]),
        server(&["sql"], "sqls", &[]),
        server(&["dockerfile"], "docker-langserver", &["--stdio"]),
        server(&["xml"], "lemminx", &[]),
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
        config.merge_default_servers();
        config.drawer_width = config.drawer_width.clamp(DRAWER_MIN, DRAWER_MAX);
        // O `code-settings.json` (escrito à mão ou pelas Preferências) vale por cima.
        if let Some(Ok(v)) = crate::code::settings_json::load() {
            crate::code::settings_json::apply(&mut config, &v);
        }
        config.drawer_width = config.drawer_width.clamp(DRAWER_MIN, DRAWER_MAX);
        config
    }

    /// Os servidores padrão que faltam na lista entram no fim (versões novas trazem
    /// servidores novos; basta o programa estar instalado), menos para uma linguagem que
    /// o usuário atende com um servidor próprio (fora da lista padrão): o arquivo do
    /// usuário manda no que ele escreveu, e o resto já funciona sozinho.
    pub fn merge_default_servers(&mut self) {
        let defaults = default_servers();
        let custom = |s: &Server| !defaults.iter().any(|d| d.command == s.command);
        for server in default_servers() {
            let present = self.servers.iter().any(|s| s.command == server.command);
            let customized = self.servers.iter().any(|s| custom(s) && s.languages.iter().any(|l| server.languages.contains(l)));
            if !present && !customized {
                self.servers.push(server);
            }
        }
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
        crate::files::write_atomic(&path, format!("# Preferências do Code do mide (mudadas pela janela de Preferências).\n\n{body}").as_bytes())?;
        // O arquivo que se edita à mão acompanha.
        crate::code::settings_json::write(self).map(|_| ())
    }

    /// Recuo de uma tecla Tab.
    pub fn indent(&self) -> String {
        if self.soft_tabs { " ".repeat(self.tab_size.clamp(1, 16)) } else { "\t".into() }
    }

    /// Servidor ligado para uma linguagem (o primeiro da lista que a atende).
    /// Com mais de um para a linguagem (PHP: intelephense ou phpactor), o primeiro que
    /// está instalado.
    pub fn server_for(&self, language: &str) -> Option<&Server> {
        let mut candidates = self.servers.iter().filter(|s| s.enabled && s.languages.iter().any(|l| l == language)).peekable();
        let first = *candidates.peek()?;
        Some(candidates.find(|s| installed(&s.command)).unwrap_or(first))
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
    find_program(command).is_some()
}

/// Pastas onde os servidores de linguagem costumam ficar além do PATH: a sessão
/// gráfica não carrega o `.bashrc` (nvm, npm global, cargo, go, composer, pipx),
/// então o PATH dela é menor que o do terminal. A versão mais nova do nvm primeiro.
pub fn extra_dirs() -> Vec<PathBuf> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let nvm = std::env::var_os("NVM_DIR").map(PathBuf::from).unwrap_or_else(|| home.join(".nvm"));
    if let Ok(entries) = std::fs::read_dir(nvm.join("versions/node")) {
        let mut versions: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        versions.sort_by(|a, b| {
            let v = |p: &PathBuf| p.file_name().map(|n| n.to_string_lossy().trim_start_matches('v').to_string()).unwrap_or_default();
            if crate::code::store::newer(&v(a), &v(b)) { std::cmp::Ordering::Less } else { std::cmp::Ordering::Greater }
        });
        out.extend(versions.into_iter().map(|v| v.join("bin")));
    }
    for d in [
        ".local/bin",
        ".npm-global/bin",
        ".local/share/npm/bin",
        ".cargo/bin",
        "go/bin",
        ".config/composer/vendor/bin",
        ".composer/vendor/bin",
        ".local/share/pnpm",
        ".bun/bin",
        ".deno/bin",
        ".dotnet/tools",
        ".rbenv/shims",
        ".pyenv/shims",
        ".ghcup/bin",
    ] {
        out.push(home.join(d));
    }
    // Servidores em JavaScript que o mide instalou sozinho (`npm_package`) e os que o
    // Zed baixou (só leitura).
    if let Some(dir) = servers_dir() {
        out.push(dir.join("node_modules/.bin"));
    }
    if let Ok(entries) = std::fs::read_dir(home.join(".local/share/zed/languages")) {
        out.extend(entries.flatten().map(|e| e.path().join("node_modules/.bin")));
    }
    // O Node que o Zed baixou (só leitura), para servidores feitos em JavaScript.
    if let Ok(entries) = std::fs::read_dir(home.join(".local/share/zed/node")) {
        out.extend(entries.flatten().map(|e| e.path().join("bin")));
    }
    out.retain(|d| d.is_dir());
    out
}

/// Pasta onde o Code instala sozinho os servidores feitos em JavaScript.
pub fn servers_dir() -> Option<PathBuf> {
    Some(crate::code::extensions::root()?.parent()?.join("servers"))
}

/// Pacote do npm de um servidor de linguagem feito em JavaScript (o primeiro é o do
/// programa; os outros vão junto), para o Code instalar sozinho quando faltar, como
/// o Zed faz.
pub fn npm_package(command: &str) -> Option<&'static [&'static str]> {
    Some(match command {
        "typescript-language-server" => &["typescript-language-server", "typescript"],
        "vtsls" => &["@vtsls/language-server", "typescript"],
        "vscode-html-language-server" | "vscode-css-language-server" | "vscode-json-language-server" => &["vscode-langservers-extracted"],
        "yaml-language-server" => &["yaml-language-server"],
        "bash-language-server" => &["bash-language-server"],
        "intelephense" => &["intelephense"],
        "pyright-langserver" => &["pyright"],
        "docker-langserver" => &["dockerfile-language-server-nodejs"],
        _ => return None,
    })
}

/// Instala pelo npm o servidor `command` na pasta do Code (pode levar segundos: rodar
/// fora da janela). Devolve o caminho do programa instalado.
pub fn install_npm_server(command: &str) -> Result<PathBuf, String> {
    let packages = npm_package(command).ok_or_else(|| format!("{command} não vem do npm"))?;
    let dir = servers_dir().ok_or("sem pasta de dados")?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = server_path_env();
    let npm = std::env::split_paths(&path).map(|d| d.join("npm")).find(|p| p.is_file()).ok_or("o npm (Node.js) não está instalado")?;
    let out = std::process::Command::new(&npm)
        .args(["install", "--prefix"])
        .arg(&dir)
        .args(["--no-audit", "--no-fund", "--save=false"])
        .args(packages)
        .env("PATH", &path)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("npm falhou").to_string());
    }
    let bin = dir.join("node_modules/.bin").join(command);
    if bin.exists() { Ok(bin) } else { Err(format!("{command} não apareceu depois do npm install")) }
}

/// Caminho completo de um programa: pelo PATH e pelas pastas de `extra_dirs`.
pub fn find_program(command: &str) -> Option<PathBuf> {
    if command.contains('/') {
        let p = PathBuf::from(command);
        return p.is_file().then_some(p);
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path).chain(extra_dirs()).map(|d| d.join(command)).find(|p| p.is_file())
}

/// PATH para os servidores: o da sessão mais as pastas de `extra_dirs` (um servidor
/// feito em JavaScript chama o `node` pelo `#!/usr/bin/env node`).
pub fn server_path_env() -> std::ffi::OsString {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    for d in extra_dirs() {
        if !dirs.contains(&d) {
            dirs.push(d);
        }
    }
    std::env::join_paths(dirs).unwrap_or_default()
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
        let servers = vec![Server::new(&["rust"], "/tmp/falso.py", &["--stdio"])];
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
