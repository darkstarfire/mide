//! Extensões do Zed com código (WebAssembly, `wasm32-wasip2`): o mide é o "host" da
//! interface `zed:extension` (WIT `since_v0.6.0`, Apache-2.0, em
//! `data/zed-extension-api/`), escrito do zero. Serve o que as extensões de
//! linguagem usam: dizer o comando do servidor (e baixá-lo, pelo npm ou pelos
//! lançamentos do GitHub), as `initialization_options` e as configurações.
//!
//! Cada extensão roda com a própria pasta de trabalho
//! (`~/.local/share/mide/code/extensions/work/<id>/`, como no Zed): é nela que os
//! servidores baixados ficam.

use std::{
    path::{Path, PathBuf},
    process::Stdio,
};

use wasmtime::{
    component::{Component, Linker, Resource, ResourceTable},
    Engine, Store,
};
use wasmtime_wasi::{DirPerms, FilePerms, WasiCtx, WasiCtxBuilder, WasiView};

wasmtime::component::bindgen!({
    path: "data/zed-extension-api/since_v0.6.0",
    world: "extension",
});

use zed::extension::{github, http_client, nodejs, platform, process};

/// Estado do host de uma extensão: WASI, a tabela de recursos e o que ela pode ver.
pub struct HostState {
    wasi: WasiCtx,
    table: ResourceTable,
    work_dir: PathBuf,
    /// Pasta do projeto (a "worktree" do Zed).
    root: PathBuf,
    /// O `settings.json` do Zed (para `get-settings`).
    zed_settings: serde_json::Value,
    /// Respostas guardadas do `fetch-stream` (o corpo inteiro, em pedaços).
    streams: Vec<Option<Vec<u8>>>,
}

impl WasiView for HostState {
    fn table(&mut self) -> &mut ResourceTable {
        &mut self.table
    }
    fn ctx(&mut self) -> &mut WasiCtx {
        &mut self.wasi
    }
}

/// Pasta de trabalho de uma extensão.
pub fn work_dir(id: &str) -> Option<PathBuf> {
    Some(crate::code::extensions::root()?.join("work").join(id))
}

/// Comando de um servidor: programa, argumentos e ambiente.
pub type ServerCommand = (String, Vec<String>, Vec<(String, String)>);

/// Uma extensão com código carregada e pronta para responder.
pub struct WasmExtension {
    store: Store<HostState>,
    bindings: Extension,
    pub id: String,
}

impl WasmExtension {
    /// Carrega o `extension.wasm` da extensão `dir` (com o id `id`) para o projeto
    /// `root`.
    pub fn load(engine: &Engine, id: &str, dir: &Path, root: &Path) -> anyhow::Result<Self> {
        let work = work_dir(id).ok_or_else(|| anyhow::anyhow!("sem pasta de dados"))?;
        Self::load_in(engine, id, dir, root, work)
    }

    /// Como `load`, com a pasta de trabalho dada.
    pub fn load_in(engine: &Engine, id: &str, dir: &Path, root: &Path, work: PathBuf) -> anyhow::Result<Self> {
        let component = Component::from_file(engine, dir.join("extension.wasm"))?;
        std::fs::create_dir_all(&work)?;
        let work_str = work.to_string_lossy().into_owned();
        let wasi = WasiCtxBuilder::new()
            .inherit_stdio()
            .env("PWD", &work_str)
            .preopened_dir(&work, ".", DirPerms::all(), FilePerms::all())?
            .preopened_dir(&work, &work_str, DirPerms::all(), FilePerms::all())?
            .build();
        let state = HostState {
            wasi,
            table: ResourceTable::new(),
            work_dir: work,
            root: root.to_path_buf(),
            zed_settings: crate::code::zed::load().unwrap_or_default(),
            streams: Vec::new(),
        };
        let mut store = Store::new(engine, state);
        let mut linker: Linker<HostState> = Linker::new(engine);
        wasmtime_wasi::add_to_linker_sync(&mut linker)?;
        Extension::add_to_linker(&mut linker, |s: &mut HostState| s)?;
        let bindings = Extension::instantiate(&mut store, &component, &linker)?;
        bindings.call_init_extension(&mut store)?;
        Ok(Self { store, bindings, id: id.to_string() })
    }

    fn worktree(&mut self) -> Resource<Worktree> {
        Resource::new_borrow(0)
    }

    /// O comando do servidor de linguagem `server_id` (programa, argumentos e
    /// ambiente), com o caminho relativo à pasta de trabalho já absoluto.
    pub fn language_server_command(&mut self, server_id: &str) -> Result<ServerCommand, String> {
        let wt = self.worktree();
        let cmd = self.bindings.call_language_server_command(&mut self.store, server_id, wt).map_err(|e| e.to_string())??;
        let work = self.store.data().work_dir.clone();
        let program = if Path::new(&cmd.command).is_absolute() || !cmd.command.contains('/') { cmd.command } else { work.join(&cmd.command).to_string_lossy().into_owned() };
        // Argumentos que são arquivos da pasta de trabalho (o `.js` do servidor) também
        // ficam absolutos.
        let args = cmd.args.into_iter().map(|a| if !a.starts_with('-') && !Path::new(&a).is_absolute() && work.join(&a).exists() { work.join(&a).to_string_lossy().into_owned() } else { a }).collect();
        Ok((program, args, cmd.env))
    }

    /// `initialization_options` do servidor (JSON), se a extensão tiver.
    pub fn initialization_options(&mut self, server_id: &str) -> Option<serde_json::Value> {
        let wt = self.worktree();
        let r = self.bindings.call_language_server_initialization_options(&mut self.store, server_id, wt).ok()?.ok()??;
        serde_json::from_str(&r).ok()
    }

    /// Configurações do servidor (`workspace/configuration`), se a extensão tiver.
    pub fn workspace_configuration(&mut self, server_id: &str) -> Option<serde_json::Value> {
        let wt = self.worktree();
        let r = self.bindings.call_language_server_workspace_configuration(&mut self.store, server_id, wt).ok()?.ok()??;
        serde_json::from_str(&r).ok()
    }
}

/// Roda um programa e devolve a saída (texto), ou o erro.
fn run(program: &str, args: &[&str], dir: Option<&Path>) -> Result<Vec<u8>, String> {
    let mut c = std::process::Command::new(program);
    c.args(args).stdin(Stdio::null());
    if let Some(d) = dir {
        c.current_dir(d);
    }
    let out = c.output().map_err(|e| format!("{program}: {e}"))?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        Err(format!("{program} {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim()))
    }
}

/// Pasta com o `node` e o `npm`: a do PATH; sem ela (a sessão gráfica não carrega o
/// `nvm` do terminal), a versão mais nova do `nvm` e, por último, o Node que o Zed
/// baixou (só leitura).
fn node_bin_dir() -> Option<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    if let Some(d) = std::env::split_paths(&path).find(|d| d.join("node").is_file() && d.join("npm").exists()) {
        return Some(d);
    }
    let home = PathBuf::from(std::env::var_os("HOME")?);
    let newest = |dir: PathBuf, sub: &str| -> Option<PathBuf> {
        let mut found: Vec<PathBuf> = std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path().join(sub)).filter(|b| b.join("node").is_file()).collect();
        found.sort_by(|a, b| {
            let v = |p: &PathBuf| p.parent().and_then(|x| x.file_name()).map(|n| n.to_string_lossy().trim_start_matches(['v', 'n', 'o', 'd', 'e', '-']).to_string()).unwrap_or_default();
            if crate::code::store::newer(&v(a), &v(b)) { std::cmp::Ordering::Greater } else { std::cmp::Ordering::Less }
        });
        found.pop()
    };
    let nvm = std::env::var_os("NVM_DIR").map(PathBuf::from).unwrap_or_else(|| home.join(".nvm"));
    newest(nvm.join("versions/node"), "bin").or_else(|| newest(home.join(".local/share/zed/node"), "bin")).or_else(|| std::env::split_paths(&path).find(|d| d.join("node").is_file()))
}

/// Roda o `npm` da pasta do Node achada, com ela no PATH (o `npm` é um script que
/// chama o `node`).
fn npm(args: &[&str], dir: &Path) -> Result<Vec<u8>, String> {
    let bin = node_bin_dir().ok_or("o Node.js não está instalado (npm)")?;
    let npm = bin.join("npm");
    let npm = if npm.exists() { npm } else { PathBuf::from("npm") };
    let mut path = std::ffi::OsString::from(&bin);
    if let Some(old) = std::env::var_os("PATH") {
        path.push(":");
        path.push(old);
    }
    let out = std::process::Command::new(&npm).args(args).env("PATH", path).current_dir(dir).stdin(Stdio::null()).output().map_err(|e| format!("{}: {e}", npm.display()))?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        Err(format!("npm {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim()))
    }
}

/// Um caminho dentro da pasta de trabalho (sem sair dela com `..`).
fn inside(work: &Path, rel: &str) -> Result<PathBuf, String> {
    let rel = rel.trim_start_matches("./");
    if Path::new(rel).components().any(|c| matches!(c, std::path::Component::ParentDir)) {
        return Err("caminho fora da pasta da extensão".into());
    }
    let p = Path::new(rel);
    Ok(if p.is_absolute() && p.starts_with(work) { p.to_path_buf() } else { work.join(rel) })
}

/// Um lançamento do GitHub pela API (com os arquivos para baixar).
fn github_release(url: &str) -> Result<serde_json::Value, String> {
    let body = run("curl", &["-sSfL", "-H", "Accept: application/vnd.github+json", "-H", "User-Agent: mide-code", url], None)?;
    serde_json::from_slice(&body).map_err(|e| e.to_string())
}

fn to_release(v: &serde_json::Value) -> github::GithubRelease {
    github::GithubRelease {
        version: v["tag_name"].as_str().unwrap_or_default().to_string(),
        assets: v["assets"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|a| github::GithubReleaseAsset { name: a["name"].as_str().unwrap_or_default().to_string(), download_url: a["browser_download_url"].as_str().unwrap_or_default().to_string() })
            .collect(),
    }
}

impl ExtensionImports for HostState {
    fn get_settings(&mut self, _path: Option<SettingsLocation>, category: String, key: Option<String>) -> Result<String, String> {
        let key = key.unwrap_or_default();
        let v = match category.as_str() {
            "language" => {
                let tab = self.zed_settings["languages"][&key]["tab_size"].as_u64().or(self.zed_settings["tab_size"].as_u64()).unwrap_or(4).max(1);
                serde_json::json!({ "tab_size": tab })
            }
            "lsp" => {
                let e = &self.zed_settings["lsp"][&key];
                serde_json::json!({ "binary": e.get("binary"), "initialization_options": e.get("initialization_options"), "settings": e.get("settings") })
            }
            "context_servers" => serde_json::json!({ "command": null, "settings": self.zed_settings["context_servers"][&key]["settings"] }),
            other => return Err(format!("categoria desconhecida: {other}")),
        };
        Ok(v.to_string())
    }

    fn download_file(&mut self, url: String, file_path: String, file_type: DownloadedFileType) -> Result<(), String> {
        let dest = inside(&self.work_dir, &file_path)?;
        tracing::info!("extensão: baixando {url} para {}", dest.display());
        let tmp = self.work_dir.join(format!(".download-{}", std::process::id()));
        run("curl", &["-sSfL", "-o", &tmp.to_string_lossy(), &url], None)?;
        let result = match file_type {
            DownloadedFileType::Uncompressed => std::fs::rename(&tmp, &dest).map_err(|e| e.to_string()),
            DownloadedFileType::Gzip => run("gzip", &["-dc", &tmp.to_string_lossy()], None).and_then(|data| std::fs::write(&dest, data).map_err(|e| e.to_string())),
            DownloadedFileType::GzipTar => std::fs::create_dir_all(&dest).map_err(|e| e.to_string()).and_then(|_| run("tar", &["-xzf", &tmp.to_string_lossy(), "-C", &dest.to_string_lossy()], None).map(|_| ())),
            DownloadedFileType::Zip => std::fs::create_dir_all(&dest).map_err(|e| e.to_string()).and_then(|_| run("unzip", &["-oq", &tmp.to_string_lossy(), "-d", &dest.to_string_lossy()], None).map(|_| ())),
        };
        let _ = std::fs::remove_file(&tmp);
        result
    }

    fn make_file_executable(&mut self, filepath: String) -> Result<(), String> {
        use std::os::unix::fs::PermissionsExt;
        let p = inside(&self.work_dir, &filepath)?;
        let mut perm = std::fs::metadata(&p).map_err(|e| e.to_string())?.permissions();
        perm.set_mode(perm.mode() | 0o755);
        std::fs::set_permissions(&p, perm).map_err(|e| e.to_string())
    }

    fn set_language_server_installation_status(&mut self, name: String, status: LanguageServerInstallationStatus) {
        match status {
            LanguageServerInstallationStatus::Failed(why) => tracing::warn!("extensão: {name} não instalou: {why}"),
            LanguageServerInstallationStatus::Downloading => tracing::info!("extensão: baixando {name}"),
            LanguageServerInstallationStatus::CheckingForUpdate => tracing::info!("extensão: conferindo atualização de {name}"),
            LanguageServerInstallationStatus::None => {}
        }
    }
}

impl HostWorktree for HostState {
    fn id(&mut self, _w: Resource<Worktree>) -> u64 {
        0
    }
    fn root_path(&mut self, _w: Resource<Worktree>) -> String {
        self.root.to_string_lossy().into_owned()
    }
    fn read_text_file(&mut self, _w: Resource<Worktree>, path: String) -> Result<String, String> {
        if Path::new(&path).components().any(|c| matches!(c, std::path::Component::ParentDir)) {
            return Err("caminho fora do projeto".into());
        }
        std::fs::read_to_string(self.root.join(path)).map_err(|e| e.to_string())
    }
    fn which(&mut self, _w: Resource<Worktree>, name: String) -> Option<String> {
        crate::code::config::find_program(&name).map(|p| p.to_string_lossy().into_owned())
    }
    fn shell_env(&mut self, _w: Resource<Worktree>) -> Vec<(String, String)> {
        // O ambiente com o PATH do terminal (nvm, cargo, go...), como o Zed lê do shell.
        let path = crate::code::config::server_path_env().to_string_lossy().into_owned();
        std::env::vars().map(|(k, v)| if k == "PATH" { (k, path.clone()) } else { (k, v) }).collect()
    }
    fn drop(&mut self, _w: Resource<Worktree>) -> wasmtime::Result<()> {
        Ok(())
    }
}

impl HostProject for HostState {
    fn worktree_ids(&mut self, _p: Resource<Project>) -> Vec<u64> {
        vec![0]
    }
    fn drop(&mut self, _p: Resource<Project>) -> wasmtime::Result<()> {
        Ok(())
    }
}

impl HostKeyValueStore for HostState {
    fn insert(&mut self, _s: Resource<KeyValueStore>, _key: String, _value: String) -> Result<(), String> {
        Err("sem banco de documentação no mide".into())
    }
    fn drop(&mut self, _s: Resource<KeyValueStore>) -> wasmtime::Result<()> {
        Ok(())
    }
}

impl github::Host for HostState {
    fn latest_github_release(&mut self, repo: String, options: github::GithubReleaseOptions) -> Result<github::GithubRelease, String> {
        let list = github_release(&format!("https://api.github.com/repos/{repo}/releases?per_page=20"))?;
        let rel = list
            .as_array()
            .into_iter()
            .flatten()
            .find(|r| r["prerelease"].as_bool() == Some(options.pre_release) && (!options.require_assets || r["assets"].as_array().is_some_and(|a| !a.is_empty())))
            .ok_or_else(|| format!("nenhum lançamento em {repo}"))?;
        Ok(to_release(rel))
    }
    fn github_release_by_tag_name(&mut self, repo: String, tag: String) -> Result<github::GithubRelease, String> {
        Ok(to_release(&github_release(&format!("https://api.github.com/repos/{repo}/releases/tags/{tag}"))?))
    }
}

impl platform::Host for HostState {
    fn current_platform(&mut self) -> (platform::Os, platform::Architecture) {
        let arch = match std::env::consts::ARCH {
            "aarch64" => platform::Architecture::Aarch64,
            "x86" => platform::Architecture::X86,
            _ => platform::Architecture::X8664,
        };
        (platform::Os::Linux, arch)
    }
}

impl process::Host for HostState {
    fn run_command(&mut self, command: process::Command) -> Result<process::Output, String> {
        let out = std::process::Command::new(&command.command)
            .args(&command.args)
            .env("PATH", crate::code::config::server_path_env())
            .envs(command.env.iter().map(|(k, v)| (k, v)))
            .current_dir(&self.work_dir)
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("{}: {e}", command.command))?;
        Ok(process::Output { status: out.status.code(), stdout: out.stdout, stderr: out.stderr })
    }
}

impl nodejs::Host for HostState {
    fn node_binary_path(&mut self) -> Result<String, String> {
        node_bin_dir().map(|d| d.join("node").to_string_lossy().into_owned()).ok_or_else(|| "o Node.js não está instalado (node)".into())
    }
    fn npm_package_latest_version(&mut self, name: String) -> Result<String, String> {
        let out = npm(&["view", &name, "version", "--json"], &self.work_dir)?;
        let v: serde_json::Value = serde_json::from_slice(&out).map_err(|e| e.to_string())?;
        v.as_str().map(str::to_string).ok_or_else(|| format!("versão de {name} não encontrada"))
    }
    fn npm_package_installed_version(&mut self, name: String) -> Result<Option<String>, String> {
        let pkg = self.work_dir.join("node_modules").join(&name).join("package.json");
        let Ok(text) = std::fs::read_to_string(pkg) else {
            return Ok(None);
        };
        let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        Ok(v["version"].as_str().map(str::to_string))
    }
    fn npm_install_package(&mut self, name: String, version: String) -> Result<(), String> {
        tracing::info!("extensão: npm install {name}@{version}");
        let work = self.work_dir.to_string_lossy().into_owned();
        npm(&["install", "--prefix", &work, "--no-save", "--no-audit", "--no-fund", &format!("{name}@{version}")], &self.work_dir).map(|_| ())
    }
}

impl http_client::Host for HostState {
    fn fetch(&mut self, req: http_client::HttpRequest) -> Result<http_client::HttpResponse, String> {
        let body = fetch(&req)?;
        Ok(http_client::HttpResponse { headers: Vec::new(), body })
    }
    fn fetch_stream(&mut self, req: http_client::HttpRequest) -> Result<Resource<http_client::HttpResponseStream>, String> {
        let body = fetch(&req)?;
        self.streams.push(Some(body));
        Ok(Resource::new_own((self.streams.len() - 1) as u32))
    }
}

impl http_client::HostHttpResponseStream for HostState {
    fn next_chunk(&mut self, s: Resource<http_client::HttpResponseStream>) -> Result<Option<Vec<u8>>, String> {
        Ok(self.streams.get_mut(s.rep() as usize).and_then(Option::take))
    }
    fn drop(&mut self, s: Resource<http_client::HttpResponseStream>) -> wasmtime::Result<()> {
        if let Some(slot) = self.streams.get_mut(s.rep() as usize) {
            *slot = None;
        }
        Ok(())
    }
}

fn fetch(req: &http_client::HttpRequest) -> Result<Vec<u8>, String> {
    let method = match req.method {
        http_client::HttpMethod::Get => "GET",
        http_client::HttpMethod::Head => "HEAD",
        http_client::HttpMethod::Post => "POST",
        http_client::HttpMethod::Put => "PUT",
        http_client::HttpMethod::Delete => "DELETE",
        http_client::HttpMethod::Options => "OPTIONS",
        http_client::HttpMethod::Patch => "PATCH",
    };
    let mut args: Vec<String> = vec!["-sSf".into(), "-X".into(), method.into()];
    if !matches!(req.redirect_policy, http_client::RedirectPolicy::NoFollow) {
        args.push("-L".into());
    }
    for (k, v) in &req.headers {
        args.push("-H".into());
        args.push(format!("{k}: {v}"));
    }
    let body_file = match &req.body {
        Some(b) => {
            let f = std::env::temp_dir().join(format!("mide-ext-body-{}", std::process::id()));
            std::fs::write(&f, b).map_err(|e| e.to_string())?;
            args.push("--data-binary".into());
            args.push(format!("@{}", f.display()));
            Some(f)
        }
        None => None,
    };
    args.push(req.url.clone());
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let out = run("curl", &refs, None);
    if let Some(f) = body_file {
        let _ = std::fs::remove_file(f);
    }
    out
}

impl zed::extension::dap::Host for HostState {
    fn resolve_tcp_template(&mut self, template: zed::extension::dap::TcpArgumentsTemplate) -> Result<zed::extension::dap::TcpArguments, String> {
        // Porta livre pedida ao sistema quando a extensão não diz qual.
        let port = match template.port {
            Some(p) => p,
            None => std::net::TcpListener::bind("127.0.0.1:0").and_then(|l| l.local_addr()).map(|a| a.port()).map_err(|e| e.to_string())?,
        };
        Ok(zed::extension::dap::TcpArguments { host: template.host.unwrap_or(u32::from(std::net::Ipv4Addr::LOCALHOST)), port, timeout: template.timeout })
    }
}

impl zed::extension::common::Host for HostState {}
impl zed::extension::context_server::Host for HostState {}
impl zed::extension::lsp::Host for HostState {}
impl zed::extension::slash_command::Host for HostState {}

#[cfg(test)]
mod tests {
    use super::*;

    /// Roda a extensão PHP instalada no Zed desta máquina e pede o comando do
    /// intelephense (pode baixar pelo npm: rodar à mão com `--ignored`).
    #[test]
    #[ignore]
    fn extensao_php_do_zed() {
        let dir = PathBuf::from(std::env::var("HOME").unwrap()).join(".local/share/zed/extensions/installed/php");
        if !dir.join("extension.wasm").exists() {
            return;
        }
        let work = std::env::temp_dir().join("mide-ext-php-teste");
        let engine = Engine::default();
        let mut ext = WasmExtension::load_in(&engine, "php", &dir, Path::new("/tmp"), work).expect("extensão carregada");
        let r = ext.language_server_command("intelephense");
        eprintln!("comando: {r:?}");
        assert!(r.is_ok(), "{r:?}");
    }
}
