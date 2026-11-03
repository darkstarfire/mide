//! Servidores de linguagem dados por extensões do Zed com código (WebAssembly):
//! quando a linguagem não tem servidor no PATH, a extensão que atende a linguagem diz
//! o comando (e baixa o servidor pelo npm ou pelo GitHub, se preciso), numa thread à
//! parte; o servidor entra na lista e o Code segue como sempre.

use std::sync::mpsc::{channel, Receiver};

use super::*;
use mide::code::{extensions, wasm_ext::WasmExtension};

/// Pedidos às extensões em andamento (por linguagem) e as linguagens já perguntadas.
#[derive(Default)]
pub(crate) struct ExtServers {
    jobs: Vec<(&'static str, Receiver<Result<Server, String>>)>,
    asked: HashSet<&'static str>,
}

/// A extensão e o id do servidor que atendem a linguagem. Primeiro a extensão que
/// define a linguagem (a do PHP para o PHP), depois as que só ajudam nela (o Emmet
/// também atende PHP e HTML); dentro da extensão, a ordem do manifesto.
fn extension_for(lang: &Language) -> Option<(extensions::Extension, String)> {
    let mut best: Option<(u8, extensions::Extension, String)> = None;
    for ext in extensions::installed() {
        if !ext.dir.join("extension.wasm").exists() {
            continue;
        }
        let defines = ext.language_dirs().iter().any(|d| {
            std::fs::read_to_string(d.join("config.toml")).ok().and_then(|t| toml::from_str::<toml::Value>(&t).ok()).and_then(|c| c.get("name").and_then(|n| n.as_str()).map(|n| n.eq_ignore_ascii_case(lang.name))).unwrap_or(false)
        }) || ext.manifest.id.eq_ignore_ascii_case(lang.id);
        let rank = if defines { 0 } else { 1 };
        for (id, entry) in &ext.manifest.language_servers {
            let serves = entry.languages.iter().chain(std::iter::once(&entry.language)).any(|l| l.eq_ignore_ascii_case(lang.name) || mide::code::zed::language_id(l) == Some(lang.id));
            if serves && best.as_ref().is_none_or(|(r, _, _)| rank < *r) {
                best = Some((rank, ext.clone(), id.clone()));
                break;
            }
        }
    }
    best.map(|(_, e, id)| (e, id))
}

impl ExtServers {
    /// As extensões mudaram: as linguagens podem ser perguntadas de novo.
    pub(crate) fn reset(&mut self) {
        self.asked.clear();
    }
}

impl Code {
    /// Pergunta (uma vez por linguagem) a uma extensão com código qual servidor usar.
    pub(crate) fn ask_extension_for_server(&mut self, lang: &'static Language) {
        if !self.ext_servers.asked.insert(lang.id) {
            return;
        }
        let Some((ext, server_id)) = extension_for(lang) else {
            self.install_default_server(lang);
            return;
        };
        let root = self.root.clone().or_else(|| self.doc().path.as_ref().and_then(|p| p.parent().map(Path::to_path_buf))).unwrap_or_else(mide::places::home_dir);
        let (tx, rx) = channel();
        let lang_id = lang.id;
        self.say(tr!("Preparing {server} from the {ext} extension…", server = server_id, ext = ext.manifest.name));
        std::thread::spawn(move || {
            let result = (|| {
                let engine = wasmtime::Engine::default();
                let mut wasm = WasmExtension::load(&engine, &ext.manifest.id, &ext.dir, &root).map_err(|e| e.to_string())?;
                let (command, args, env) = wasm.language_server_command(&server_id)?;
                let mut server = Server::new(&[lang_id], &command, &[]);
                server.args = args;
                server.env = env;
                server.label = server_id.clone();
                server.init_options = wasm.initialization_options(&server_id).unwrap_or_default();
                server.settings = wasm.workspace_configuration(&server_id).unwrap_or_default();
                tracing::info!("extensão {}: servidor {server_id}: {command}", ext.manifest.id);
                Ok(server)
            })();
            let _ = tx.send(result);
        });
        self.ext_servers.jobs.push((lang_id, rx));
    }

    /// Sem extensão para a linguagem: o primeiro servidor padrão dela que vem do npm é
    /// instalado sozinho (como o Zed faz com o vtsls e os servidores do VS Code).
    fn install_default_server(&mut self, lang: &'static Language) {
        let Some(server) = self.config.servers.iter().find(|s| s.enabled && s.languages.iter().any(|l| l == lang.id) && mide::code::config::npm_package(&s.command).is_some()).cloned() else {
            return;
        };
        let (tx, rx) = channel();
        self.say(tr!("Installing {server} (language server)…", server = server.command));
        std::thread::spawn(move || {
            let result = mide::code::config::install_npm_server(&server.command).map(|path| {
                tracing::info!("servidor instalado pelo npm: {}", path.display());
                let mut s = server.clone();
                s.label = server.command.clone();
                s.command = path.to_string_lossy().into_owned();
                s
            });
            let _ = tx.send(result);
        });
        self.ext_servers.jobs.push((lang.id, rx));
    }

    /// Nenhum pedido às extensões esperando resposta.
    pub(crate) fn ext_servers_idle(&self) -> bool {
        self.ext_servers.jobs.is_empty()
    }

    /// Respostas das extensões: o servidor entra na frente da lista da linguagem.
    /// Devolve se algo mudou.
    pub(crate) fn poll_ext_servers(&mut self) -> bool {
        let mut changed = false;
        let mut done = Vec::new();
        for (k, (lang, rx)) in self.ext_servers.jobs.iter().enumerate() {
            if let Ok(result) = rx.try_recv() {
                done.push(k);
                changed = true;
                match result {
                    Ok(server) => {
                        self.config.servers.insert(0, server);
                        // As abas da linguagem abrem de novo no servidor novo.
                        for d in self.docs.iter_mut().filter(|d| d.lang.id == *lang) {
                            d.lsp_seen = None;
                        }
                    }
                    Err(err) => tracing::warn!("extensão: servidor de {lang}: {err}"),
                }
            }
        }
        for k in done.into_iter().rev() {
            self.ext_servers.jobs.remove(k);
        }
        changed
    }
}
