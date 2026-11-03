//! Code (`mide-code`): o editor de texto e código do mide, inspirado no TextMate do Mac
//! (só o comportamento como referência; tudo escrito aqui, com o mide-ui).
//!
//! - **Projeto**: abrir uma pasta (`mide-code pasta/` ou Arquivo → Abrir pasta) mostra a
//!   gaveta à esquerda com a árvore dos arquivos (divisória arrastável, menu de clique
//!   direito para criar, renomear, duplicar, Lixeira, copiar o caminho, mostrar no
//!   Arquivos e abrir no Terminal). Aberto sem nada, volta ao projeto da última vez.
//! - **Abas e divisão**: um arquivo por aba; Ctrl+W fecha (pergunta se há alterações).
//!   O editor divide em dois painéis (Ctrl+\, menu Visualizar, ou arrastando um
//!   arquivo da gaveta ou uma aba até a borda do editor).
//! - **Ir para arquivo** (Ctrl+T, como o ⌘T do TextMate): busca aproximada pelo nome em
//!   todo o projeto. **Ir para linha** (Ctrl+L, aceita "linha:coluna").
//! - **Cores do código** para mais de 25 linguagens (`mide::code::highlight`), e a
//!   linguagem escolhível em Visualizar → Linguagem.
//! - **LSP** (servidores no `code-servers.toml`, aberto pelas Preferências e lido de
//!   novo ao salvar): erros e avisos sublinhados com a mensagem no fim da linha (F8
//!   vai ao próximo), completar (Alt+Espaço ou Ctrl+Espaço; Tab/Shift+Tab escolhem, Enter aceita,
//!   Esc fecha), ir para a definição (F12 ou Ctrl+clique) e a dica do símbolo (Ctrl+I).
//! - **Editar**: buscar e substituir no arquivo (Ctrl+F) e no projeto (Ctrl+Shift+F),
//!   comentar linhas (Ctrl+/), duplicar linha (Ctrl+Shift+D), mover linhas (Alt+↑/↓),
//!   salvar tudo (Ctrl+Alt+S), recuo com espaços ou tabulação.
//! - **Terminal integrado** (Ctrl+J ou Ctrl+`): o shell na pasta do projeto, num painel
//!   embaixo, com o emulador do Terminal do mide.
//! - **Preferências** (Ctrl+,): fonte, tamanho, números de linha, linha atual,
//!   tabulação, gaveta e servidores de linguagem.
//! - Arquivos soltos na janela abrem em abas.

mod ext_servers;
mod lsp_ui;
mod store_ui;
mod term;
mod tools;
mod tree;

use std::{
    collections::{HashMap, HashSet},
    error::Error,
    ops::Range,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use mide::code::{
    config::{installed, parse_servers, Config, Server, DRAWER_MAX, DRAWER_MIN},
    edit, highlight,
    lang::{self, Language},
    lsp::{self, Client, Completion, Diagnostic, Event, Severity},
    panes::{self, Edge},
    project, search,
    session::Session,
};
use mide::i18n::trn;
use mide::tr;
use mide::kit::{
    run,
    textarea::{line_of, Decorations, TextDoc},
    window_menu, App, Choice, FileChooser, Key, MenuGroup, MenuItem, TabAction, TextAreaLook, Tool, Ui, Window,
};

/// Arquivos maiores que isso não abrem (o histórico guarda cópias do texto).
const MAX_BYTES: u64 = 16 * 1024 * 1024;
/// Id da janela filha das Preferências.
const PREFS: &str = "prefs";
/// Altura da barra de status.
const STATUS_H: f32 = 26.0;

/// Um documento aberto (uma aba).
struct Doc {
    path: Option<PathBuf>,
    text: TextDoc,
    lang: &'static Language,
    crlf: bool,
    /// Versão mandada ao servidor de linguagem (sobe a cada mudança).
    lsp_version: i32,
    /// `text.version` da última vez que o servidor soube do texto.
    lsp_seen: Option<u64>,
    diags: Vec<Diagnostic>,
    /// Cores do texto e a versão para a qual foram feitas.
    colors: (u64, Vec<Span>),
}

/// Trecho colorido do texto (bytes e cor).
type Span = (Range<usize>, [u8; 4]);

impl Doc {
    fn new(path: Option<PathBuf>, text: String, crlf: bool) -> Self {
        let first = text.lines().next().unwrap_or("").to_string();
        let lang = path.as_ref().map_or(lang::plain(), |p| lang::detect(p, &first));
        Self { path, text: TextDoc::new(text), lang, crlf, lsp_version: 1, lsp_seen: None, diags: Vec::new(), colors: (u64::MAX, Vec::new()) }
    }

    fn name(&self) -> String {
        self.path.as_ref().and_then(|p| p.file_name()).map_or_else(|| tr!("untitled").into_owned(), |n| n.to_string_lossy().into_owned())
    }

    fn title(&self) -> String {
        if self.text.modified { format!("• {}", self.name()) } else { self.name() }
    }
}

/// Painéis por cima do editor.
enum Overlay {
    None,
    /// Ir para arquivo: busca, a lista do projeto e o escolhido.
    GoToFile { query: String, files: Vec<PathBuf>, sel: usize, focus: bool },
    GoToLine { text: String, focus: bool },
    /// Buscar (e substituir) no projeto inteiro.
    ProjectFind(ProjectFind),
    /// Renomear símbolo (F2): o nome novo.
    Rename { text: String, focus: bool },
    /// Ações de código (Ctrl+.): a lista e a escolhida.
    Actions { actions: Vec<lsp::CodeAction>, sel: usize, go: bool },
    /// Ir para símbolo do arquivo (Ctrl+R).
    Symbols { query: String, items: Vec<lsp::Symbol>, sel: usize, focus: bool },
    /// Paleta de comandos (Ctrl+Shift+P).
    Palette { query: String, sel: usize, focus: bool, items: Vec<tools::PaletteItem> },
    /// Problemas das abas abertas (Ctrl+Shift+M).
    Problems { sel: usize, go: bool },
    /// Loja de extensões (Ctrl+Shift+X).
    Extensions { focus: bool },
}

#[derive(Default)]
struct ProjectFind {
    query: String,
    replace: String,
    case: bool,
    hits: Vec<search::Hit>,
    /// A busca que gerou `hits` (para "nenhum resultado").
    searched: Option<String>,
    focus: bool,
}

enum Dialog {
    Open(FileChooser),
    OpenFolder(FileChooser),
    SaveAs(FileChooser, bool),
    /// Fechar a aba com alterações não salvas.
    ConfirmClose(usize),
    /// Sair com abas alteradas.
    ConfirmQuit,
    /// Nome de um arquivo ou pasta novo (menu da gaveta).
    NewItem { dir: PathBuf, folder: bool, name: String, focus: bool, error: Option<String> },
    /// Nome novo de um item da gaveta.
    Rename { path: PathBuf, name: String, focus: bool, error: Option<String> },
}

#[derive(Default)]
struct Find {
    query: String,
    replace: String,
    case: bool,
    focus: bool,
}

/// O editor dividido em dois painéis: o documento de cada um (índices em `docs`), qual
/// tem o foco (o dele é sempre `Code::cur`) e a parte do primeiro.
struct Split {
    side_by_side: bool,
    ratio: f32,
    docs: [usize; 2],
    focus: usize,
}

/// Um arquivo ou aba sendo arrastado (para a borda do editor, que divide).
pub(crate) struct Drag {
    pub what: DragWhat,
    pub from: (f32, f32),
    /// Andou o bastante para ser arrasto (e não um clique).
    pub active: bool,
}

pub(crate) enum DragWhat {
    Doc(usize),
    Path(PathBuf),
}


pub(crate) struct Code {
    docs: Vec<Doc>,
    cur: usize,
    root: Option<PathBuf>,
    expanded: HashSet<PathBuf>,
    config: Config,
    /// Servidores rodando (pelo comando) e os que falharam (não tenta de novo).
    servers: HashMap<String, Client>,
    failed: HashSet<String>,
    overlay: Overlay,
    find: Option<Find>,
    dialog: Option<Dialog>,
    /// Completar: pedido, itens, escolhido e onde começa a palavra.
    completion: Option<(u64, Vec<Completion>, usize, usize)>,
    /// Pedido de documentação (`completionItem/resolve`) em andamento: o número e o
    /// rótulo do item.
    resolving: Option<(u64, String)>,
    hover: Option<(u64, String)>,
    definition: Option<u64>,
    message: Option<(String, Instant)>,
    start: bool,
    /// Edição nova no servidor: manda depois de uma pausa curta na digitação.
    last_edit: Option<Instant>,
    split: Option<Split>,
    drag: Option<Drag>,
    /// Terminal integrado (aberto ou não) e se tem o foco do teclado.
    terminal: Option<term::TermPanel>,
    terminal_open: bool,
    terminal_focus: bool,
    /// Última sessão gravada (para gravar só quando muda).
    saved_session: Option<Session>,
    /// Alt+Espaço pediu o completar: o espaço que a área de texto digitar sai.
    eat_space: bool,
    /// Assinatura, renomear, formatar, ações, símbolos e dicas em linha.
    lsp: lsp_ui::LspUi,
    /// Paleta, problemas, dica do ponteiro e Git na margem.
    tools: tools::Tools,
    /// Atalhos do `keymap.json` do Zed (com as configurações do Zed ligadas).
    zed_keys: Vec<mide::code::zed::Binding>,
    /// Tema de cores escolhido (de uma extensão do Zed) e os nomes dos temas de cores e
    /// de ícones instalados (para as Preferências).
    color_theme: Option<mide::code::themes::ColorTheme>,
    theme_names: (Vec<String>, Vec<String>),
    /// Realce por tree-sitter com as gramáticas das extensões do Zed.
    syntax: mide::code::syntax::Syntax,
    /// Servidores pedidos às extensões do Zed com código.
    ext_servers: ext_servers::ExtServers,
    /// Loja de extensões.
    store_ui: store_ui::StoreUi,
}

fn read_text(path: &Path) -> std::io::Result<(String, bool)> {
    if std::fs::metadata(path)?.len() > MAX_BYTES {
        return Err(std::io::Error::other(tr!("file too large for the editor").into_owned()));
    }
    let bytes = std::fs::read(path)?;
    let text = String::from_utf8(bytes).map_err(|_| std::io::Error::other(tr!("not UTF-8 text (binary file?)").into_owned()))?;
    let crlf = text.contains("\r\n");
    Ok((if crlf { text.replace("\r\n", "\n") } else { text }, crlf))
}

/// Prefixo de comentário para comentar linhas (com Ctrl+/).
fn comment_prefix(lang: &Language) -> &str {
    match lang.comment {
        "" => "#",
        c => c,
    }
}

impl Code {
    fn new(args: Vec<PathBuf>) -> Self {
        let zed_keys = if Config::load().use_zed_settings { mide::code::zed::keymap() } else { Vec::new() };
        let mut app = Self {
            docs: Vec::new(),
            cur: 0,
            root: None,
            expanded: HashSet::new(),
            config: Config::load(),
            servers: HashMap::new(),
            failed: HashSet::new(),
            overlay: Overlay::None,
            find: None,
            dialog: None,
            completion: None,
            resolving: None,
            lsp: lsp_ui::LspUi::default(),
            tools: tools::Tools::default(),
            zed_keys,
            color_theme: None,
            theme_names: (Vec::new(), Vec::new()),
            syntax: mide::code::syntax::Syntax::new(),
            ext_servers: ext_servers::ExtServers::default(),
            store_ui: store_ui::StoreUi::default(),
            hover: None,
            definition: None,
            message: None,
            start: true,
            last_edit: None,
            split: None,
            drag: None,
            terminal: None,
            terminal_open: false,
            terminal_focus: false,
            saved_session: None,
            eat_space: false,
        };
        if args.is_empty() {
            // Sem nada para abrir: volta ao projeto e às abas da última vez.
            if let Some(s) = Session::load() {
                tracing::info!("sessão anterior: {:?} com {} arquivo(s)", s.root, s.files.len());
                app.root = s.root.clone();
                app.expanded = s.expanded.iter().cloned().collect();
                for f in &s.files {
                    app.open_path(f.clone());
                }
                app.cur = s.current.min(app.docs.len().saturating_sub(1));
                app.saved_session = Some(s);
            }
        }
        for arg in args {
            if arg.is_dir() {
                app.root = std::fs::canonicalize(&arg).ok();
            } else {
                app.open_path(arg);
            }
        }
        if app.root.is_none() {
            // Arquivo de um projeto (com .git): a pasta dele vira a gaveta.
            app.root = app.docs.first().and_then(|d| d.path.as_ref()).and_then(|p| p.ancestors().skip(1).find(|a| a.join(".git").exists()).map(Path::to_path_buf));
        }
        if app.docs.is_empty() {
            app.docs.push(Doc::new(None, String::new(), false));
        }
        app.apply_themes();
        app
    }

    fn doc(&mut self) -> &mut Doc {
        let i = self.cur.min(self.docs.len() - 1);
        &mut self.docs[i]
    }

    fn say(&mut self, text: impl Into<String>) {
        let text = text.into();
        tracing::info!("aviso: {text}");
        self.message = Some((text, Instant::now()));
    }

    /// Abre um arquivo numa aba (ou vai para a aba dele, se já está aberto). A aba
    /// vazia e sem nome do começo é trocada por ele.
    fn open_path(&mut self, path: PathBuf) {
        let path = std::fs::canonicalize(&path).unwrap_or(path);
        if let Some(i) = self.docs.iter().position(|d| d.path.as_ref() == Some(&path)) {
            self.cur = i;
            return;
        }
        let doc = match read_text(&path) {
            Ok((text, crlf)) => Doc::new(Some(path.clone()), text, crlf),
            // Arquivo novo: começa vazio e é criado ao salvar.
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Doc::new(Some(path.clone()), String::new(), false),
            Err(err) => {
                self.say(tr!("Couldn’t open “{path}”: {err}", path = path.display(), err = err));
                return;
            }
        };
        tracing::info!("aberto: {} ({})", path.display(), doc.lang.name);
        let blank = self.docs.len() == 1 && self.docs[0].path.is_none() && !self.docs[0].text.modified && self.docs[0].text.text.is_empty() && self.split.is_none();
        if blank {
            self.docs[0] = doc;
            self.cur = 0;
        } else {
            self.docs.push(doc);
            self.cur = self.docs.len() - 1;
        }
        self.start = true;
    }

    /// Abre um arquivo e devolve o índice da aba, sem trocar a aba do painel em foco.
    fn open_index(&mut self, path: PathBuf) -> Option<usize> {
        let keep = self.cur;
        let canon = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
        self.open_path(path);
        let i = self.docs.iter().position(|d| d.path.as_ref() == Some(&canon))?;
        // A aba vazia do começo trocada pelo arquivo fica no índice 0 (o mesmo).
        self.cur = keep.min(self.docs.len() - 1);
        Some(i)
    }

    /// Salva a aba atual (sem nome: pergunta onde). Devolve se salvou.
    fn save(&mut self, then_close: bool) -> bool {
        if self.doc().path.is_none() {
            let dir = default_dir(self.root.as_deref());
            self.dialog = Some(Dialog::SaveAs(FileChooser::save(&tr!("Save As"), Some(&dir), &format!("{}.txt", tr!("untitled"))), then_close));
            return false;
        }
        self.save_index(self.cur.min(self.docs.len() - 1))
    }

    /// Grava a aba `i` (que tem caminho).
    fn save_index(&mut self, i: usize) -> bool {
        let Some(path) = self.docs[i].path.clone() else {
            return false;
        };
        let doc = &mut self.docs[i];
        let text = if doc.crlf { doc.text.text.replace('\n', "\r\n") } else { doc.text.text.clone() };
        match mide::files::write_atomic(&path, text.as_bytes()) {
            Ok(()) => {
                doc.text.modified = false;
                tracing::info!("salvo: {}", path.display());
                self.tools.git.remove(&path);
                // Arquivo de trechos gravado: lê os trechos de novo.
                if mide::code::snippets::user_dir().is_some_and(|d| path.starts_with(d)) {
                    self.tools.snippets.clear();
                }
                let cmd = self.config.server_for(doc.lang.id).map(|s| s.key());
                if let Some(client) = cmd.and_then(|c| self.servers.get_mut(&c)) {
                    client.did_save(&path);
                }
                // O arquivo dos servidores de linguagem: lê de novo e reinicia.
                if Config::servers_path().is_some_and(|p| p == path || std::fs::canonicalize(&p).ok().as_ref() == Some(&path)) {
                    self.reload_servers(&text);
                }
                true
            }
            Err(err) => {
                self.say(tr!("Couldn’t save: {err}", err = err));
                false
            }
        }
    }

    /// Salva todas as abas alteradas que têm arquivo.
    fn save_all(&mut self) {
        let mut n = 0;
        for i in 0..self.docs.len() {
            if self.docs[i].text.modified && self.docs[i].path.is_some() && self.save_index(i) {
                n += 1;
            }
        }
        let unnamed = self.docs.iter().filter(|d| d.text.modified && d.path.is_none()).count();
        if unnamed > 0 {
            self.say(tr!("{saved}; {unnamed} untitled (use Save As)", saved = trn("{n} file saved", "{n} files saved", n as u64), unnamed = unnamed));
        } else {
            self.say(trn("{n} file saved", "{n} files saved", n as u64));
        }
    }

    /// O `code-servers.toml` foi salvo: troca os servidores (com erro, mantém os de
    /// antes e mostra a linha errada na barra de status).
    fn reload_servers(&mut self, text: &str) {
        match parse_servers(text) {
            Ok(servers) => {
                tracing::info!("servidores de linguagem lidos: {}", servers.iter().map(|s| s.command.as_str()).collect::<Vec<_>>().join(", "));
                self.config.servers = servers;
                self.config.merge_default_servers();
                // Reinicia todos: o comando ou os argumentos podem ter mudado.
                self.servers.clear();
                self.failed.clear();
                for doc in &mut self.docs {
                    doc.lsp_seen = None;
                    doc.diags.clear();
                }
                let n = self.config.servers.len();
                self.say(tr!("Language servers updated ({n})", n = n));
            }
            Err(err) => self.say(tr!("Error in code-servers.toml, {err}", err = err)),
        }
    }

    fn close_tab(&mut self, i: usize, force: bool) {
        if i >= self.docs.len() {
            return;
        }
        if !force && self.docs[i].text.modified {
            self.cur = i;
            self.dialog = Some(Dialog::ConfirmClose(i));
            return;
        }
        let doc = self.docs.remove(i);
        if let (Some(path), Some(cmd)) = (&doc.path, self.config.server_for(doc.lang.id).map(|s| s.key()))
            && let Some(client) = self.servers.get_mut(&cmd)
        {
            client.did_close(path);
        }
        // Painel do outro lado com a aba fechada: a divisão acaba.
        if let Some(s) = self.split.as_mut() {
            let other = 1 - s.focus;
            if s.docs[other] == i {
                self.split = None;
            } else if s.docs[other] > i {
                s.docs[other] -= 1;
            }
        }
        if self.docs.is_empty() {
            self.docs.push(Doc::new(None, String::new(), false));
        }
        if i < self.cur {
            self.cur -= 1;
        }
        self.cur = self.cur.min(self.docs.len() - 1);
        if let Some(s) = self.split.as_mut() {
            s.docs[s.focus] = self.cur;
            if s.docs[0] == s.docs[1] && self.docs.len() == 1 {
                self.split = None;
            }
        }
        self.completion = None;
    }

    /// Divide o editor pondo a aba `doc` no lado `edge` (o painel dela fica com o foco).
    fn split_with(&mut self, doc: usize, edge: Edge) {
        let k = usize::from(edge.second());
        match self.split.as_mut() {
            Some(s) => {
                // Já dividido: o lado de lá recebe a aba (e a direção pode mudar).
                s.side_by_side = edge.side_by_side();
                s.docs[k] = doc;
                s.focus = k;
            }
            None => {
                let mut docs = [self.cur; 2];
                docs[k] = doc;
                self.split = Some(Split { side_by_side: edge.side_by_side(), ratio: 0.5, docs, focus: k });
            }
        }
        self.cur = doc;
        self.start = true;
        tracing::info!("editor dividido: {:?} com {}", edge, self.docs.get(doc).map(Doc::name).unwrap_or_default());
    }

    /// Servidor de linguagem da aba atual (inicia na primeira vez, se instalado).
    fn client_for_current(&mut self) -> Option<&mut Client> {
        let lang = self.doc().lang;
        let server = self.config.server_for(lang.id).cloned();
        // Sem servidor no PATH para a linguagem: uma extensão com código (do Zed) pode
        // dizer qual usar (e baixá-lo), sem travar a janela.
        let usable = server.as_ref().is_some_and(|s| installed(&s.command) && !self.failed.contains(&s.key()));
        if !usable {
            self.ask_extension_for_server(lang);
        }
        let server: Server = server?;
        let key = server.key();
        if self.failed.contains(&key) {
            return None;
        }
        if !self.servers.contains_key(&key) {
            if !installed(&server.command) {
                self.failed.insert(key);
                return None;
            }
            let root = self.root.clone().or_else(|| self.doc().path.as_ref().and_then(|p| p.parent().map(Path::to_path_buf))).unwrap_or_else(mide::places::home_dir);
            match Client::start_with(&server.command, &server.args, &root, &server.env, server.init_options.clone(), server.settings.clone()) {
                Ok(client) => {
                    tracing::info!("servidor de linguagem: {} ({})", server.command, root.display());
                    self.servers.insert(key.clone(), client);
                }
                Err(err) => {
                    self.say(tr!("Couldn’t start {command}: {err}", command = server.command, err = err));
                    self.failed.insert(key);
                    return None;
                }
            }
        }
        self.servers.get_mut(&key)
    }

    /// Mantém o servidor a par do texto da aba atual (abre e manda as mudanças depois
    /// de uma pausa curta na digitação).
    fn sync_lsp(&mut self) {
        let i = self.cur;
        let Some(path) = self.docs[i].path.clone() else {
            return;
        };
        let version = self.docs[i].text.version;
        let seen = self.docs[i].lsp_seen;
        if seen == Some(version) {
            return;
        }
        if seen.is_some() && self.last_edit.is_some_and(|t| t.elapsed() < Duration::from_millis(250)) {
            return;
        }
        let text = self.docs[i].text.text.clone();
        let lang = self.docs[i].lang.id;
        self.docs[i].lsp_version += 1;
        let v = self.docs[i].lsp_version;
        let Some(client) = self.client_for_current() else {
            return;
        };
        match seen {
            None => client.did_open(&path, lang, v, &text),
            Some(_) => client.did_change(&path, v, &text),
        }
        self.docs[i].lsp_seen = Some(version);
    }

    /// Respostas dos servidores: diagnósticos, completar, definição e dica.
    fn poll_lsp(&mut self) -> bool {
        let mut changed = false;
        let mut events = Vec::new();
        let mut dead = Vec::new();
        for (cmd, client) in &mut self.servers {
            for e in client.poll() {
                if let Event::Exited(_) = e {
                    dead.push(cmd.clone());
                }
                events.push(e);
            }
        }
        for cmd in dead {
            self.servers.remove(&cmd);
            self.failed.insert(cmd);
        }
        for event in events {
            changed = true;
            match event {
                Event::Diagnostics { uri, items } => {
                    if let Some(doc) = self.docs.iter_mut().find(|d| d.path.as_ref().is_some_and(|p| lsp::uri(p) == uri)) {
                        tracing::info!("diagnósticos: {} em {}", items.len(), doc.name());
                        doc.diags = items;
                    }
                }
                Event::Completions { id, items } => {
                    if let Some((want, list, sel, _)) = self.completion.as_mut()
                        && *want == id
                    {
                        tracing::info!("completar: {} item(ns)", items.len());
                        // As do servidor primeiro; as palavras do arquivo que ele não trouxe, depois.
                        let local = std::mem::take(list);
                        // Na ordem que o servidor sugere (`sortText`).
                        let mut merged = items;
                        merged.sort_by(|a, b| a.sort.cmp(&b.sort));
                        for word in local {
                            if !merged.iter().any(|m| m.label.trim_start_matches('$') == word.label) {
                                merged.push(word);
                            }
                        }
                        *list = merged;
                        *sel = 0;
                        // Filtra pela palavra já digitada.
                        self.filter_completion();
                    }
                }
                Event::Definition { id, uri, line, col } => {
                    if self.definition == Some(id) {
                        self.definition = None;
                        let path = mide::kit::parse_uri_list(&uri).into_iter().next().unwrap_or_default();
                        if !path.as_os_str().is_empty() {
                            tracing::info!("definição: {}:{}", path.display(), line + 1);
                            self.open_path(path);
                            let utf8 = self.client_for_current().map(|c| c.utf8).unwrap_or(true);
                            let doc = self.doc();
                            let at = lsp::to_offset(&doc.text.text, line, col, utf8);
                            doc.text.edit = mide::kit::textedit::Edit { cursor: at, anchor: None };
                            self.start = true;
                        }
                    }
                }
                Event::Resolved { id, doc, detail } => {
                    if let Some((want, label)) = self.resolving.take() {
                        if want == id
                            && let Some((_, items, _, _)) = self.completion.as_mut()
                            && let Some(item) = items.iter_mut().find(|c| c.label == label)
                        {
                            if !doc.is_empty() {
                                item.doc = doc;
                            }
                            if item.detail.is_empty() {
                                item.detail = detail;
                            }
                            // Já pedido: não pede de novo para o mesmo item.
                            item.raw = None;
                        } else if want != id {
                            self.resolving = Some((want, label));
                        }
                    }
                }
                Event::Hover { id, ref text } if self.mouse_hover_answer(id, text) => {}
                Event::Hover { id, text } => {
                    if self.hover.as_ref().is_some_and(|(want, _)| *want == id) {
                        self.hover = Some((id, text));
                    }
                }
                Event::Exited(text) => self.say(text),
                other => self.on_lsp_event(other),
            }
        }
        changed
    }

    /// Começo da palavra antes do cursor (para completar).
    fn word_start(text: &str, cursor: usize) -> usize {
        text[..cursor].char_indices().rev().take_while(|(_, c)| c.is_alphanumeric() || *c == '_').last().map_or(cursor, |(i, _)| i)
    }

    fn request_completion(&mut self) {
        self.complete(false);
    }

    /// Abre o completar: na hora, com as palavras do arquivo e da linguagem; com um
    /// servidor de linguagem, a resposta dele vem por cima (as dele primeiro). `auto`:
    /// aberto pela digitação (sem aviso quando não há nada a sugerir).
    fn complete(&mut self, auto: bool) {
        let cursor = self.doc().text.edit.cursor;
        let text = self.doc().text.text.clone();
        let path = self.doc().path.clone();
        let start = Self::word_start(&text, cursor);
        let lang = self.doc().lang;
        let mut local = local_completions(&text, start, cursor, lang.keywords.iter().map(|k| (*k, 14)).chain(lang.types.iter().map(|t| (*t, 7))));
        // Trechos do usuário e das extensões (`code-snippets/<linguagem>.json`) que
        // começam com o que foi digitado.
        let typed_word = text[start..cursor].to_lowercase();
        if !typed_word.is_empty() {
            let snippets = self.tools.snippets.entry(lang.id).or_insert_with(|| mide::code::snippets::for_language(lang.id));
            let mut found: Vec<Completion> = snippets
                .iter()
                .filter(|s| s.prefix.to_lowercase().starts_with(&typed_word))
                .map(|s| Completion { label: s.prefix.clone(), detail: s.name.clone(), insert: s.body.clone(), kind: 15, doc: s.description.clone(), snippet: true, ..Default::default() })
                .collect();
            found.append(&mut local);
            local = found;
        }
        // Sozinho, só com o mínimo de letras das Preferências (os gatilhos como `.` e
        // `->` abrem sem letra nenhuma).
        let typed = text[start..cursor].chars().count();
        if auto && typed > 0 && typed < self.config.complete_min_chars.clamp(1, 5) {
            return;
        }
        self.sync_now();
        let id = match (path, self.client_for_current()) {
            (Some(path), Some(client)) => client.completion(&path, &text, cursor).unwrap_or(0),
            _ => 0,
        };
        if id == 0 && local.is_empty() {
            if !auto {
                self.say(tr!("Completion needs a language server (Settings)."));
            }
            return;
        }
        tracing::info!("completar: pedido ({} do arquivo{})", local.len(), if id != 0 { ", servidor" } else { "" });
        self.completion = Some((id, local, 0, start));
    }

    /// A lista de completar acompanha a palavra digitada (e some fora dela).
    fn filter_completion(&mut self) {
        let i = self.cur.min(self.docs.len() - 1);
        let Some((id, items, sel, start)) = self.completion.as_mut() else {
            return;
        };
        // Já respondido (sem servidor, ou com itens na lista): sem nada que case, fecha
        // (a próxima letra pede de novo). Esperando o servidor, a lista fica aberta.
        let answered = *id == 0 || !items.is_empty();
        let doc = &self.docs[i].text;
        let cursor = doc.edit.cursor;
        if cursor < *start || doc.text[*start..cursor].contains(|c: char| !c.is_alphanumeric() && c != '_') {
            self.completion = None;
            return;
        }
        // Busca aproximada (as letras em ordem, em qualquer lugar), como no VS Code: os
        // que começam com o que foi digitado primeiro, na ordem de antes; depois os
        // outros, pela pontuação.
        let word = doc.text[*start..cursor].to_lowercase();
        if !word.is_empty() {
            let key = |it: &Completion| if it.filter.is_empty() { it.label.clone() } else { it.filter.clone() }.trim_start_matches('$').to_lowercase();
            let mut scored: Vec<(bool, i32, usize, Completion)> = std::mem::take(items)
                .into_iter()
                .enumerate()
                .filter_map(|(k, it)| {
                    let text = key(&it);
                    lsp_ui::fuzzy_score(&text, &word).map(|sc| (text.starts_with(&word), sc, k, it))
                })
                .collect();
            scored.sort_by(|a, b| b.0.cmp(&a.0).then(if a.0 { a.2.cmp(&b.2) } else { b.1.cmp(&a.1) }));
            *items = scored.into_iter().map(|(_, _, _, it)| it).collect();
        }
        if items.is_empty() && answered {
            self.completion = None;
            return;
        }
        *sel = (*sel).min(items.len().saturating_sub(1));
    }

    /// Põe o item escolhido no lugar da palavra.
    fn accept_completion(&mut self, item: Completion, start: usize) {
        let doc = &mut self.doc().text;
        let cursor = doc.edit.cursor;
        doc.before_change(mide::kit::textarea::Change::Other);
        let from = start.min(cursor);
        // Trecho (snippet): as marcas viram texto e a primeira parada fica escolhida.
        let (expanded, stop) = if item.snippet { lsp::expand_snippet(&item.insert) } else { (item.insert.clone(), None) };
        // `$nome` do PHP: o `$` já digitado não se repete.
        let dollar = from > 0 && doc.text[..from].ends_with('$') && expanded.starts_with('$');
        let insert = if dollar { &expanded[1..] } else { expanded.as_str() };
        let shift = if dollar { 1 } else { 0 };
        doc.text.replace_range(from..cursor, insert);
        doc.edit = match stop {
            Some((a, b)) if a >= shift => {
                let (a, b) = (from + a - shift, from + b - shift);
                mide::kit::textedit::Edit { cursor: b, anchor: (a != b).then_some(a) }
            }
            _ => mide::kit::textedit::Edit { cursor: from + insert.len(), anchor: None },
        };
        tracing::info!("completado: {}", item.label);
    }

    /// Manda o texto ao servidor agora (antes de um pedido).
    fn sync_now(&mut self) {
        self.last_edit = None;
        self.sync_lsp();
    }

    fn go_to_definition(&mut self) {
        let cursor = self.doc().text.edit.cursor;
        let text = self.doc().text.text.clone();
        let path = self.doc().path.clone();
        self.sync_now();
        if let (Some(path), Some(client)) = (path, self.client_for_current()) {
            self.definition = client.definition(&path, &text, cursor);
        } else {
            self.say(tr!("Go to Definition needs a language server (Settings)."));
        }
    }

    fn show_hover(&mut self) {
        let cursor = self.doc().text.edit.cursor;
        let text = self.doc().text.text.clone();
        let path = self.doc().path.clone();
        self.sync_now();
        if let (Some(path), Some(client)) = (path, self.client_for_current())
            && let Some(id) = client.hover(&path, &text, cursor)
        {
            self.hover = Some((id, String::new()));
        }
    }

    /// Vai ao próximo (ou anterior) erro ou aviso do servidor de linguagem.
    fn next_problem(&mut self, forward: bool) {
        let lang = self.doc().lang.id;
        let utf8 = self.config.server_for(lang).and_then(|s| self.servers.get(&s.key())).is_none_or(|c| c.utf8);
        let doc = self.doc();
        if doc.diags.is_empty() {
            self.say(tr!("No problems in this file"));
            return;
        }
        let cursor = doc.text.edit.cursor;
        let mut spots: Vec<(usize, String)> = doc.diags.iter().map(|d| (lsp::to_offset(&doc.text.text, d.start.0, d.start.1, utf8), d.message.clone())).collect();
        spots.sort_by_key(|s| s.0);
        let found = if forward { spots.iter().find(|s| s.0 > cursor).or(spots.first()) } else { spots.iter().rev().find(|s| s.0 < cursor).or(spots.last()) };
        if let Some((at, msg)) = found.cloned() {
            doc.text.edit = mide::kit::textedit::Edit { cursor: at, anchor: None };
            self.start = true;
            self.say(msg);
        }
    }

    /// Seleção (ou cursor) da aba atual, em ordem.
    fn selection(&mut self) -> (usize, usize) {
        let e = &self.doc().text.edit;
        e.selection().unwrap_or((e.cursor, e.cursor))
    }

    /// Comenta ou descomenta as linhas da seleção (ou a do cursor).
    fn toggle_comment(&mut self) {
        let prefix = comment_prefix(self.doc().lang).to_string();
        let (a, b) = self.selection();
        let doc = &mut self.doc().text;
        let mut text = doc.text.clone();
        let Some((na, nb)) = edit::toggle_comment(&mut text, a, b, &prefix) else {
            return;
        };
        doc.before_change(mide::kit::textarea::Change::Other);
        doc.text = text;
        let anchor = doc.edit.anchor.map(|_| if doc.edit.cursor == b { na } else { nb });
        let cursor = if doc.edit.cursor == b && a != b { nb } else { na };
        doc.edit = mide::kit::textedit::Edit { cursor, anchor };
    }

    fn duplicate_line(&mut self) {
        let (a, b) = self.selection();
        let doc = &mut self.doc().text;
        doc.before_change(mide::kit::textarea::Change::Other);
        let shift = edit::duplicate_lines(&mut doc.text, a, b);
        doc.edit = mide::kit::textedit::Edit { cursor: doc.edit.cursor + shift, anchor: doc.edit.anchor.map(|x| x + shift) };
    }

    /// Move as linhas da seleção uma linha para cima ou para baixo (Alt+↑/↓).
    fn move_lines(&mut self, up: bool) {
        let (a, b) = self.selection();
        let doc = &mut self.doc().text;
        let mut text = doc.text.clone();
        let Some(delta) = edit::move_lines(&mut text, a, b, up) else {
            return;
        };
        doc.before_change(mide::kit::textarea::Change::Other);
        doc.text = text;
        let mv = |x: usize| (x as isize + delta).max(0) as usize;
        doc.edit = mide::kit::textedit::Edit { cursor: mv(doc.edit.cursor), anchor: doc.edit.anchor.map(mv) };
    }

    fn go_to(&mut self, line: usize, col: usize) {
        let doc = &mut self.doc().text;
        let at = edit::offset_of(&doc.text, line, col);
        doc.edit = mide::kit::textedit::Edit { cursor: at, anchor: None };
        self.start = true;
    }

    /// Próxima (ou anterior) ocorrência da busca, a partir da seleção.
    fn find_step(&mut self, forward: bool) {
        let Some((query, case)) = self.find.as_ref().map(|f| (f.query.clone(), f.case)).filter(|(q, _)| !q.is_empty()) else {
            return;
        };
        let (a, b) = self.selection();
        let doc = &mut self.doc().text;
        match edit::find(&doc.text, &query, if forward { b } else { a }, forward, case) {
            Some(i) => {
                doc.edit = mide::kit::textedit::Edit { cursor: i + query.len(), anchor: Some(i) };
                self.start = true;
            }
            None => self.say(tr!("“{query}” not found", query = query)),
        }
    }

    /// Troca a ocorrência escolhida (se a seleção é a busca) e vai para a próxima.
    fn replace_one(&mut self) {
        let Some((q, r, case)) = self.find.as_ref().map(|f| (f.query.clone(), f.replace.clone(), f.case)) else {
            return;
        };
        let (a, b) = self.selection();
        let doc = &mut self.doc().text;
        let sel = &doc.text[a..b];
        let same = if case { sel == q } else { sel.to_lowercase() == q.to_lowercase() };
        if a != b && same {
            doc.before_change(mide::kit::textarea::Change::Other);
            doc.text.replace_range(a..b, &r);
            doc.edit = mide::kit::textedit::Edit { cursor: a + r.len(), anchor: None };
        }
        self.find_step(true);
    }

    fn replace_all(&mut self) {
        let Some((q, r, case)) = self.find.as_ref().map(|f| (f.query.clone(), f.replace.clone(), f.case)) else {
            return;
        };
        let doc = &mut self.doc().text;
        let mut text = doc.text.clone();
        let n = edit::replace_all(&mut text, &q, &r, case);
        if n > 0 {
            doc.before_change(mide::kit::textarea::Change::Other);
            doc.text = text;
            doc.edit = mide::kit::textedit::Edit { cursor: 0, anchor: None };
        }
        self.say(trn("{n} replacement", "{n} replacements", n as u64));
    }

    /// Substitui no projeto inteiro: nas abas abertas troca no texto (fica para salvar),
    /// nos outros arquivos grava direto.
    fn replace_in_project(&mut self, query: &str, with: &str, case: bool) {
        let Some(root) = self.root.clone() else {
            return;
        };
        let mut open = HashSet::new();
        let mut in_open = 0;
        for doc in &mut self.docs {
            let Some(path) = doc.path.clone().filter(|p| p.starts_with(&root)) else {
                continue;
            };
            open.insert(path);
            let mut text = doc.text.text.clone();
            let n = edit::replace_all(&mut text, query, with, case);
            if n > 0 {
                doc.text.before_change(mide::kit::textarea::Change::Other);
                doc.text.text = text;
                let end = doc.text.text.len();
                doc.text.edit = mide::kit::textedit::Edit { cursor: doc.text.edit.cursor.min(end), anchor: None };
                in_open += n;
            }
        }
        match search::replace(&root, query, with, case, &open) {
            Ok((files, n)) => self.say(tr!("{replacements} in {files} and {open} in open tabs", replacements = trn("{n} replacement", "{n} replacements", n as u64), files = trn("{n} file", "{n} files", files as u64), open = in_open)),
            Err(err) => self.say(tr!("Error while replacing: {err}", err = err)),
        }
    }

    /// Temas de cores e de ícones das Preferências (das extensões do Zed): lê os
    /// instalados, aplica os escolhidos e recolore as abas.
    fn apply_themes(&mut self) {
        let colors = mide::code::themes::color_themes();
        self.color_theme = colors.iter().find(|t| t.name == self.config.color_theme).cloned();
        mide::code::icons::set_theme(&self.config.icon_theme);
        let icons = mide::code::themes::icon_themes();
        self.theme_names = (colors.into_iter().map(|t| t.name).collect(), icons.into_iter().map(|t| t.name).collect());
        for d in &mut self.docs {
            d.colors.0 = u64::MAX;
        }
    }

    /// Cores do editor do tema escolhido, para a área de texto.
    fn editor_colors(&self) -> Option<mide::kit::TextAreaColors> {
        let t = self.color_theme.as_ref()?;
        Some(mide::kit::TextAreaColors { background: t.background, text: t.foreground, gutter: t.gutter.or(t.background), line_number: t.line_number, current_line: t.active_line })
    }

    /// Cores do código e erros da aba `i`, para a área de texto.
    fn decorations(&mut self, i: usize, dark: bool) -> Decorations {
        let theme = self.color_theme.as_ref().map(|t| t.syntax.clone());
        if self.docs[i].colors.0 != self.docs[i].text.version {
            // A gramática tree-sitter das extensões, se houver; senão, o realce próprio.
            let (text, lang) = (&self.docs[i].text.text, self.docs[i].lang);
            let spans = self.syntax.highlight(text, lang).unwrap_or_else(|| highlight::highlight(text, lang));
            let doc = &mut self.docs[i];
            // As cores do tema escolhido; o que ele não tem, as do mide.
            let pick = |k| theme.as_ref().and_then(|t| t.get(&k).copied()).unwrap_or_else(|| highlight::color(k, dark));
            doc.colors = (doc.text.version, spans.into_iter().map(|(r, k)| (r, pick(k))).collect());
        }
        let doc = &mut self.docs[i];
        let utf8 = self.config.server_for(doc.lang.id).and_then(|s| self.servers.get(&s.key())).is_none_or(|c| c.utf8);
        let mut deco = Decorations { colors: doc.colors.1.clone(), ..Default::default() };
        let mut seen_lines = HashSet::new();
        for d in &doc.diags {
            let a = lsp::to_offset(&doc.text.text, d.start.0, d.start.1, utf8);
            let mut b = lsp::to_offset(&doc.text.text, d.end.0, d.end.1, utf8);
            if b <= a {
                // Diagnóstico sem largura: sublinha até o fim da palavra.
                b = doc.text.text[a..].find(|c: char| !c.is_alphanumeric() && c != '_').map_or(doc.text.text.len(), |e| a + e.max(1));
            }
            if b > a {
                deco.underlines.push(a..b.min(doc.text.text.len()));
            }
            if seen_lines.insert(d.start.0) {
                let c = match d.severity {
                    Severity::Error => [215, 58, 73, 255],
                    Severity::Warning => [200, 140, 0, 255],
                    Severity::Info => [120, 120, 130, 255],
                };
                deco.trailing.push((d.start.0 as usize, d.message.clone(), c));
            }
        }
        deco.gutter = self.git_marks(i);
        deco
    }

    /// Abre (ou mostra) o terminal integrado, em `dir` ou na pasta do projeto. Devolve
    /// se ele está aberto.
    fn open_integrated_terminal(&mut self, dir: Option<PathBuf>, ui: &mut Ui) -> bool {
        let dir = dir.or_else(|| self.root.clone()).or_else(|| self.docs.get(self.cur).and_then(|d| d.path.as_ref()).and_then(|p| p.parent().map(Path::to_path_buf))).unwrap_or_else(mide::places::home_dir);
        let reuse = self.terminal.as_ref().is_some_and(|t| !t.ended && t.dir == dir);
        if !reuse {
            let waker = ui.waker();
            let wake = move || {
                if let Some(w) = &waker {
                    w.wake();
                }
            };
            match term::TermPanel::open(dir, wake) {
                Ok(t) => self.terminal = Some(t),
                Err(err) => {
                    self.say(tr!("Couldn’t open the terminal: {err}", err = err));
                    return false;
                }
            }
        }
        self.terminal_open = true;
        self.terminal_focus = true;
        ui.blur();
        true
    }

    /// Ação dos menus, atalhos e botões.
    fn command(&mut self, action: &str, ui: &mut Ui) {
        match action {
            "new" => {
                self.docs.push(Doc::new(None, String::new(), false));
                self.cur = self.docs.len() - 1;
                self.start = true;
            }
            "open" => {
                // Na pasta do arquivo aberto (ou do projeto).
                let dir = self.docs.get(self.cur).and_then(|d| d.path.as_ref()).and_then(|p| p.parent().map(Path::to_path_buf)).unwrap_or_else(|| default_dir(self.root.as_deref()));
                self.dialog = Some(Dialog::Open(FileChooser::open(&tr!("Open"), Some(&dir)).multiple(true)));
            }
            "open-folder" => {
                let dir = self.root.clone().unwrap_or_else(mide::places::home_dir);
                self.dialog = Some(Dialog::OpenFolder(FileChooser::folder(&tr!("Open Folder"), Some(&dir))));
            }
            "close-folder" => self.close_folder(),
            "save" => {
                // Formatar ao salvar (Preferências): grava quando a formatação chegar.
                if !(self.config.format_on_save && self.doc().path.is_some() && self.format_document(true)) {
                    self.save(false);
                }
            }
            "save-all" => self.save_all(),
            "save-as" => {
                let dir = self.doc().path.as_ref().and_then(|p| p.parent().map(Path::to_path_buf)).or_else(|| self.root.clone()).unwrap_or_else(mide::places::home_dir);
                let name = self.doc().name();
                self.dialog = Some(Dialog::SaveAs(FileChooser::save(&tr!("Save As"), Some(&dir), &name), false));
            }
            "close-tab" => self.close_tab(self.cur, false),
            "quit" => {
                if self.docs.iter().any(|d| d.text.modified) {
                    self.dialog = Some(Dialog::ConfirmQuit);
                } else {
                    ui.close();
                }
            }
            "find" => {
                self.find = match self.find.take() {
                    Some(_) => None,
                    None => {
                        let doc = &self.docs[self.cur.min(self.docs.len() - 1)].text;
                        let query = doc.edit.selected(&doc.text).filter(|s| !s.contains('\n')).unwrap_or("").to_string();
                        Some(Find { query, focus: true, ..Default::default() })
                    }
                };
            }
            "find-next" => self.find_step(true),
            "find-prev" => self.find_step(false),
            "find-project" => {
                if self.root.is_none() {
                    self.say(tr!("Open a folder (File → Open Folder) to search the project."));
                } else {
                    let doc = &self.docs[self.cur.min(self.docs.len() - 1)].text;
                    let query = doc.edit.selected(&doc.text).filter(|s| !s.contains('\n')).unwrap_or("").to_string();
                    self.overlay = Overlay::ProjectFind(ProjectFind { query, focus: true, ..Default::default() });
                }
            }
            "goto-file" => {
                let files = self.root.as_ref().map(|r| project::files(r, 20_000)).unwrap_or_default();
                if files.is_empty() {
                    self.say(tr!("Open a folder (File → Open Folder) to go to its files."));
                } else {
                    self.overlay = Overlay::GoToFile { query: String::new(), files, sel: 0, focus: true };
                }
            }
            "goto-line" => self.overlay = Overlay::GoToLine { text: String::new(), focus: true },
            "comment" => self.toggle_comment(),
            "duplicate" => self.duplicate_line(),
            "move-up" => self.move_lines(true),
            "move-down" => self.move_lines(false),
            "complete" => self.request_completion(),
            "rename" => self.rename_symbol(),
            "code-actions" => self.code_actions(),
            "format" => {
                self.format_document(false);
            }
            "symbols" => self.request_symbols(true),
            "palette" => {
                let items = self.palette_items();
                self.overlay = Overlay::Palette { query: String::new(), sel: 0, focus: true, items };
            }
            "problems" => self.overlay = Overlay::Problems { sel: 0, go: false },
            "extensions" => self.open_store(),
            "outline" => {
                self.config.outline = !self.config.outline;
                let _ = self.config.save();
            }
            "breadcrumbs" => {
                self.config.breadcrumbs = !self.config.breadcrumbs;
                let _ = self.config.save();
            }
            "signature" => self.request_signature(),
            "definition" => self.go_to_definition(),
            "hover" => self.show_hover(),
            "next-problem" => self.next_problem(true),
            "prev-problem" => self.next_problem(false),
            // Preferências numa janela filha de vidro (o compositor a centra sobre a
            // principal); de novo, fecha.
            "prefs" => {
                if ui.is_window_open(PREFS) {
                    ui.close_window(PREFS);
                } else {
                    ui.open_window(PREFS, Window::new(&tr!("Code Settings")).size(560, 470).min_size(480, 400).glass());
                }
            }
            "edit-servers" => {
                self.overlay = Overlay::None;
                match self.config.ensure_servers_file() {
                    Ok(path) => self.open_path(path),
                    Err(err) => self.say(tr!("Couldn’t create the language servers file: {err}", err = err)),
                }
            }
            "drawer" => {
                self.config.drawer = !self.config.drawer;
                let _ = self.config.save();
            }
            "split-right" | "split-down" => {
                // O outro painel mostra a aba seguinte (ou a mesma, com uma só).
                let other = if self.docs.len() > 1 { (self.cur + 1) % self.docs.len() } else { self.cur };
                self.split_with(other, if action == "split-right" { Edge::Right } else { Edge::Bottom });
            }
            "unsplit" => self.split = None,
            "terminal" => {
                if self.terminal_open && !self.terminal.as_ref().is_some_and(|t| t.ended) {
                    self.terminal_open = false;
                    self.terminal_focus = false;
                    self.start = true;
                } else {
                    self.terminal = self.terminal.take().filter(|t| !t.ended);
                    self.open_integrated_terminal(None, ui);
                }
            }
            "zoom-in" | "zoom-out" | "zoom-reset" => {
                self.config.size = match action {
                    "zoom-in" => (self.config.size + 1.0).min(32.0),
                    "zoom-out" => (self.config.size - 1.0).max(8.0),
                    _ => Config::default().size,
                };
                let _ = self.config.save();
            }
            "line-numbers" => {
                self.config.line_numbers = !self.config.line_numbers;
                let _ = self.config.save();
            }
            other => {
                if let Some(id) = other.strip_prefix("lang:") {
                    if let Some(l) = lang::by_id(id) {
                        let doc = self.doc();
                        doc.lang = l;
                        doc.colors.0 = u64::MAX;
                        doc.lsp_seen = None;
                        doc.diags.clear();
                    }
                } else if let Some(i) = other.strip_prefix("tab:").and_then(|n| n.parse::<usize>().ok())
                    && i < self.docs.len()
                {
                    self.cur = i;
                }
            }
        }
        ui.request_repaint();
    }

    /// Atalhos do teclado (com a área de texto: os de edição ficam com ela).
    fn shortcuts(&mut self, ui: &mut Ui) {
        let input = ui.input().clone();
        // Atalho com Shift (`Some(true)`) e sem (`Some(false)`); `None`: não apertado.
        let shift = |l: &str| input.shortcut_shift(l) == Some(true);
        let plain = |l: &str| input.shortcut_shift(l) == Some(false);
        let mut run: Vec<&str> = Vec::new();
        // O terminal integrado com o foco fica com as teclas (Ctrl+C, Ctrl+W...): só o
        // atalho que fecha o painel vale.
        let toggle_terminal = input.shortcut("j") || input.presses.iter().any(|p| p.ctrl && matches!(p.keysym, 0x60 | 0xfe50 | 0x27));
        if self.terminal_focus && self.terminal_open {
            if toggle_terminal {
                self.command("terminal", ui);
            }
            return;
        }
        if toggle_terminal {
            run.push("terminal");
        }
        if input.shortcut_alt("s") {
            run.push("save-all");
        } else if plain("s") {
            run.push("save");
        } else if shift("s") {
            run.push("save-as");
        }
        for (key, action) in [("n", "new"), ("w", "close-tab"), ("q", "quit"), ("t", "goto-file"), ("l", "goto-line"), ("/", "comment"), (",", "prefs"), ("=", "zoom-in"), ("+", "zoom-in"), ("-", "zoom-out"), ("0", "zoom-reset"), (".", "code-actions"), ("r", "symbols")] {
            if input.shortcut(key) {
                run.push(action);
            }
        }
        if plain("o") {
            run.push("open");
        } else if shift("o") {
            run.push("open-folder");
        }
        if plain("f") {
            run.push("find");
        } else if shift("f") {
            run.push("find-project");
        }
        if plain("g") {
            run.push("find-next");
        } else if shift("g") {
            run.push("find-prev");
        }
        if shift("d") {
            run.push("duplicate");
        }
        if shift("p") {
            run.push("palette");
        }
        if shift("m") {
            run.push("problems");
        }
        if shift("x") {
            run.push("extensions");
        }
        // Ctrl+I: dica do símbolo; Ctrl+Shift+I: formatar (como no VS Code).
        if plain("i") {
            run.push("hover");
        } else if shift("i") {
            run.push("format");
        }
        for p in &input.presses {
            match p.keysym {
                // Ctrl+Espaço ou Alt+Espaço: completar (na sessão do mide, Ctrl+Espaço
                // é o buscador e não chega aos apps).
                // Ctrl+Shift+Espaço: ajuda de assinatura.
                0x20 if p.ctrl && p.shift => run.push("signature"),
                0x20 if p.ctrl || p.alt => {
                    self.eat_space = p.alt;
                    run.push("complete");
                }
                // F12: definição; F2: renomear; F8 (Shift: anterior): próximo problema.
                0xffc9 => run.push("definition"),
                0xffbf => run.push("rename"),
                0xffc5 => run.push(if p.shift { "prev-problem" } else { "next-problem" }),
                // Ctrl+\ divide à direita (Ctrl+Shift+\: embaixo).
                0x5c | 0x7c if p.ctrl => run.push(if p.shift { "split-down" } else { "split-right" }),
                // Alt+↑/↓ movem as linhas (a área de texto não vê a seta).
                0xff52 | 0xff54 if p.alt && !p.ctrl => {
                    ui.take_key(if p.keysym == 0xff52 { Key::Up } else { Key::Down });
                    run.push(if p.keysym == 0xff52 { "move-up" } else { "move-down" });
                }
                _ => {}
            }
        }
        // Atalhos do `keymap.json` do Zed (rodam junto com os do Code).
        let zed: Vec<String> = self.zed_keys.iter().filter(|b| input.presses.iter().any(|p| p.ctrl == b.ctrl && p.shift == b.shift && p.alt == b.alt && key_matches(p.keysym, &b.key))).map(|b| b.action.clone()).collect();
        // Ctrl+Tab e Ctrl+Shift+Tab trocam de aba.
        if input.key_events().iter().any(|(k, ctrl, _)| *k == Key::Tab && *ctrl) && self.docs.len() > 1 {
            let back = input.key_events().iter().any(|(k, ctrl, sh)| *k == Key::Tab && *ctrl && *sh);
            self.cur = if back { (self.cur + self.docs.len() - 1) % self.docs.len() } else { (self.cur + 1) % self.docs.len() };
            ui.take_key(Key::Tab);
        }
        for action in run {
            self.command(action, ui);
        }
        for action in zed {
            self.command(&action, ui);
        }
    }

    fn overlays(&mut self, ui: &mut Ui) {
        let esc = ui.input().key(Key::Escape);
        if self.lsp_overlays(ui, esc) || self.tool_overlays(ui, esc) || self.store_overlay(ui, esc) {
            return;
        }
        match &mut self.overlay {
            Overlay::None => {}
            Overlay::GoToFile { query, files, sel, focus } => {
                let (up, down, enter) = (ui.take_key(Key::Up), ui.take_key(Key::Down), ui.input().key(Key::Enter));
                let mut chosen = None;
                let root = self.root.clone();
                ui.modal(560.0, |ui| {
                    ui.strong(&tr!("Go to File"));
                    if std::mem::take(focus) {
                        ui.focus_next_input();
                    }
                    ui.text_input(query, &tr!("File name (letters anywhere)"));
                    let best = project::best(query, files, 12);
                    if up {
                        *sel = sel.saturating_sub(1);
                    }
                    if down {
                        *sel = (*sel + 1).min(best.len().saturating_sub(1));
                    }
                    *sel = (*sel).min(best.len().saturating_sub(1));
                    for (i, rel) in best.iter().enumerate() {
                        let name = rel.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        let dir = rel.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
                        if i == *sel {
                            ui.join_selection(false, false);
                        }
                        if ui.list_row("text-x-generic", &name, &[(&dir, 260.0)], i == *sel).clicked() {
                            chosen = Some((*rel).clone());
                        }
                    }
                    if enter {
                        chosen = best.get(*sel).map(|p| (*p).clone());
                    }
                });
                if let (Some(rel), Some(root)) = (chosen, root) {
                    self.overlay = Overlay::None;
                    self.open_path(root.join(rel));
                } else if esc {
                    self.overlay = Overlay::None;
                }
                ui.request_repaint();
            }
            Overlay::GoToLine { text, focus } => {
                let mut go = None;
                ui.modal(320.0, |ui| {
                    ui.strong(&tr!("Go to Line"));
                    if std::mem::take(focus) {
                        ui.focus_next_input();
                    }
                    if ui.text_input(text, &tr!("Line (or line:column)")).submitted() {
                        go = edit::parse_goto(text);
                    }
                });
                if let Some((line, col)) = go {
                    self.overlay = Overlay::None;
                    self.go_to(line, col);
                } else if esc {
                    self.overlay = Overlay::None;
                }
            }
            Overlay::ProjectFind(_) => self.project_find(ui, esc),
            _ => {}
        }
    }

    /// Buscar e substituir no projeto (Ctrl+Shift+F): a lista das linhas; clique abre o
    /// arquivo na linha.
    fn project_find(&mut self, ui: &mut Ui, esc: bool) {
        let Overlay::ProjectFind(pf) = &mut self.overlay else {
            return;
        };
        let mut run_search = false;
        let mut replace = false;
        let mut chosen: Option<search::Hit> = None;
        let mut close = esc;
        ui.modal(680.0, |ui| {
            ui.strong(&tr!("Find in Project"));
            if std::mem::take(&mut pf.focus) {
                ui.focus_next_input();
            }
            run_search |= ui.text_input(&mut pf.query, &tr!("Find")).submitted();
            ui.text_input(&mut pf.replace, &tr!("Replace with"));
            ui.row(|ui| {
                if ui.checkbox(&tr!("Match case"), &mut pf.case).changed() {
                    run_search = true;
                }
                ui.push_right(2.0 * 170.0 + 8.0);
                ui.sized(170.0, |ui| replace |= ui.button(&tr!("Replace All")).clicked());
                ui.sized(170.0, |ui| run_search |= ui.primary_button(&tr!("Find")).clicked());
            });
            match &pf.searched {
                Some(q) if pf.hits.is_empty() => ui.note(&tr!("Nothing found for “{query}”.", query = q)),
                Some(_) if pf.hits.len() >= 500 => ui.note(&tr!("{lines} (the first 500)", lines = trn("{n} line", "{n} lines", pf.hits.len() as u64))),
                Some(_) => ui.note(&trn("{n} line", "{n} lines", pf.hits.len() as u64)),
                None => {}
            }
            if !pf.hits.is_empty() {
                ui.scroll_area(220.0, |ui| {
                    for hit in &pf.hits {
                        let label = format!("{}:{}", hit.path.display(), hit.line);
                        if ui.list_row("text-x-generic", &label, &[(&hit.text, 340.0)], false).clicked() {
                            chosen = Some(hit.clone());
                        }
                    }
                });
            }
            ui.row(|ui| {
                ui.push_right(110.0);
                ui.sized(110.0, |ui| close |= ui.button(&tr!("Close")).clicked());
            });
        });
        let (query, with, case) = (pf.query.clone(), pf.replace.clone(), pf.case);
        if replace && !query.is_empty() {
            self.replace_in_project(&query, &with, case);
            run_search = true;
        }
        if run_search && let (Some(root), Overlay::ProjectFind(pf)) = (self.root.clone(), &mut self.overlay) {
            pf.hits = search::search(&root, &pf.query, pf.case, 500);
            pf.searched = Some(pf.query.clone());
            tracing::info!("busca no projeto: \"{}\" → {} linha(s)", pf.query, pf.hits.len());
        }
        if let (Some(hit), Some(root)) = (chosen, self.root.clone()) {
            self.overlay = Overlay::None;
            self.open_path(root.join(&hit.path));
            self.go_to(hit.line, hit.col);
        } else if close {
            self.overlay = Overlay::None;
        }
        ui.request_repaint();
    }

    /// Preferências (janela filha `PREFS`): aparência do editor e servidores de
    /// linguagem. As mudanças valem na hora; o arquivo é gravado ao fechar a janela
    /// (`window_closed`).
    fn prefs(&mut self, ui: &mut Ui) {
        let c = &mut self.config;
        let mut close = ui.input().key(Key::Escape);
        let mut edit_servers = false;
        let fonts: Vec<String> = std::iter::once(tr!("System Font").into_owned()).chain(ui.mono_fonts()).collect();
        let mut font_i = c.font.as_ref().and_then(|f| fonts.iter().position(|x| x == f)).unwrap_or(0);
        let servers_running: Vec<String> = self.servers.keys().cloned().collect();
        let servers_file = "code-servers.toml";
        // Temas das extensões do Zed (o primeiro de cada lista é o do mide).
        let color_names: Vec<String> = std::iter::once(tr!("mide (default)").into_owned()).chain(self.theme_names.0.iter().cloned()).collect();
        let icon_names: Vec<String> = std::iter::once(tr!("Material Icon Theme (default)").into_owned()).chain(self.theme_names.1.iter().cloned()).collect();
        let mut color_i = color_names.iter().skip(1).position(|n| *n == c.color_theme).map_or(0, |p| p + 1);
        let mut icon_i = icon_names.iter().skip(1).position(|n| *n == c.icon_theme).map_or(0, |p| p + 1);
        let mut themes_changed = false;
        ui.page(|ui| {
            // Servidores primeiro (o mais procurado): um resumo (a lista fica no arquivo, editado no próprio Code).
            let running: Vec<&str> = c.servers.iter().filter(|s| s.enabled && servers_running.contains(&s.key())).map(|s| s.command.as_str()).collect();
            let ready: Vec<&str> = c.servers.iter().filter(|s| s.enabled && installed(&s.command) && !servers_running.contains(&s.key())).map(|s| s.command.as_str()).collect();
            let mut summary = tr!("{n} configured", n = c.servers.len());
            if !running.is_empty() {
                summary.push_str(&tr!(" · running: {list}", list = running.join(", ")));
            }
            if !ready.is_empty() {
                summary.push_str(&tr!(" · installed: {list}", list = ready.join(", ")));
            }
            ui.note(&tr!("Language servers (LSP): {summary}. They are listed in {file}, which takes effect when saved.", summary = summary, file = servers_file));
            ui.row(|ui| {
                edit_servers |= ui.button(&tr!("Edit Language Servers…")).clicked();
            });
            ui.card(|ui| {
                if ui.dropdown(&tr!("Font"), &fonts, &mut font_i).changed() {
                    c.font = (font_i > 0).then(|| fonts[font_i].clone());
                }
                if ui.dropdown(&tr!("Color Theme"), &color_names, &mut color_i).changed() {
                    c.color_theme = if color_i == 0 { String::new() } else { color_names[color_i].clone() };
                    themes_changed = true;
                }
                if ui.dropdown(&tr!("File Icon Theme"), &icon_names, &mut icon_i).changed() {
                    c.icon_theme = if icon_i == 0 { String::new() } else { icon_names[icon_i].clone() };
                    themes_changed = true;
                }
                let mut size = c.size;
                ui.slider(&tr!("Font Size"), &mut size, 9.0, 28.0);
                c.size = size.round();
                ui.switch(&tr!("Line numbers"), &mut c.line_numbers);
                ui.switch(&tr!("Highlight current line"), &mut c.current_line);
                ui.switch(&tr!("Indent with spaces (off: tabs)"), &mut c.soft_tabs);
                let mut tab = c.tab_size as f32;
                ui.slider(&tr!("Spaces per indent"), &mut tab, 1.0, 8.0);
                c.tab_size = tab.round() as usize;
                ui.switch(&tr!("Project drawer on the left"), &mut c.drawer);
            });
            // Completar: abre sozinho ao digitar (a partir de quantas letras) e mostra a
            // documentação ao lado, como no VS Code e no Zed.
            ui.card(|ui| {
                ui.switch(&tr!("Suggest completions while typing"), &mut c.auto_complete);
                let mut min = c.complete_min_chars.clamp(1, 5) as f32;
                ui.slider(&tr!("Letters before suggesting"), &mut min, 1.0, 5.0);
                c.complete_min_chars = min.round() as usize;
                ui.switch(&tr!("Show documentation next to the suggestion"), &mut c.complete_docs);
                ui.switch(&tr!("Inlay hints (types and parameter names)"), &mut c.inlay_hints);
                ui.switch(&tr!("Format on save"), &mut c.format_on_save);
            });
            // Quem vem do Zed: servidores (`lsp`, `languages`) e atalhos dele valem.
            if mide::code::zed::load().is_some() {
                ui.card(|ui| {
                    ui.switch(&tr!("Use Zed settings (language servers and keymap)"), &mut c.use_zed_settings);
                });
            }
            ui.row(|ui| {
                ui.push_right(110.0);
                ui.sized(110.0, |ui| close |= ui.primary_button(&mide::i18n::trc("settings", "Done")).clicked());
            });
        });
        if themes_changed {
            self.apply_themes();
        }
        if edit_servers {
            // O arquivo abre numa aba da janela principal.
            match self.config.ensure_servers_file() {
                Ok(path) => self.open_path(path),
                Err(err) => self.say(tr!("Couldn’t create the language servers file: {err}", err = err)),
            }
            close = true;
        }
        if close {
            ui.close();
        }
    }

    fn dialogs(&mut self, ui: &mut Ui) {
        if self.name_dialog(ui) {
            return;
        }
        let Some(dialog) = self.dialog.as_mut() else {
            return;
        };
        let esc = ui.input().key(Key::Escape);
        match dialog {
            Dialog::Open(chooser) => match chooser.show(ui) {
                Some(Choice::Chosen(_)) => {
                    let paths = chooser.chosen().to_vec();
                    self.dialog = None;
                    for p in paths {
                        self.open_path(p);
                    }
                }
                Some(Choice::Cancelled) => self.dialog = None,
                None => {}
            },
            Dialog::OpenFolder(chooser) => match chooser.show(ui) {
                Some(Choice::Chosen(path)) => {
                    self.dialog = None;
                    self.set_root(path);
                }
                Some(Choice::Cancelled) => self.dialog = None,
                None => {}
            },
            Dialog::SaveAs(chooser, then_close) => {
                let then_close = *then_close;
                match chooser.show(ui) {
                    Some(Choice::Chosen(path)) => {
                        self.dialog = None;
                        let first = self.doc().text.text.lines().next().unwrap_or("").to_string();
                        let doc = self.doc();
                        doc.path = Some(path.clone());
                        doc.lang = lang::detect(&path, &first);
                        doc.colors.0 = u64::MAX;
                        doc.lsp_seen = None;
                        if self.save(false) && then_close {
                            self.close_tab(self.cur, true);
                        }
                    }
                    Some(Choice::Cancelled) => self.dialog = None,
                    None => {}
                }
            }
            Dialog::ConfirmClose(i) => {
                let i = *i;
                let name = self.docs.get(i).map(Doc::name).unwrap_or_default();
                let mut choice = None;
                ui.modal(420.0, |ui| {
                    ui.strong(&tr!("Save the changes to “{name}”?", name = name));
                    ui.note(&tr!("Without saving, the changes are lost."));
                    ui.row(|ui| {
                        if ui.button(&tr!("Don’t Save")).clicked() {
                            choice = Some(0);
                        }
                        ui.push_right(2.0 * 110.0 + 8.0);
                        ui.sized(110.0, |ui| {
                            if ui.button(&tr!("Cancel")).clicked() {
                                choice = Some(1);
                            }
                        });
                        ui.sized(110.0, |ui| {
                            if ui.primary_button(&tr!("Save")).clicked() {
                                choice = Some(2);
                            }
                        });
                    });
                });
                match choice.or(esc.then_some(1)) {
                    Some(0) => {
                        self.dialog = None;
                        self.close_tab(i, true);
                    }
                    Some(1) => self.dialog = None,
                    Some(_) => {
                        self.dialog = None;
                        self.cur = i;
                        if self.save(true) {
                            self.close_tab(i, true);
                        }
                    }
                    None => {}
                }
            }
            Dialog::ConfirmQuit => {
                let n = self.docs.iter().filter(|d| d.text.modified).count();
                let mut choice = None;
                ui.modal(420.0, |ui| {
                    ui.strong(&trn("{n} file has unsaved changes", "{n} files have unsaved changes", n as u64));
                    ui.note(&tr!("Quitting without saving loses the changes."));
                    ui.row(|ui| {
                        ui.push_right(2.0 * 130.0 + 8.0);
                        ui.sized(130.0, |ui| {
                            if ui.button(&tr!("Cancel")).clicked() {
                                choice = Some(false);
                            }
                        });
                        ui.sized(130.0, |ui| {
                            if ui.primary_button(&tr!("Quit Without Saving")).clicked() {
                                choice = Some(true);
                            }
                        });
                    });
                });
                match choice.or(esc.then_some(false)) {
                    Some(true) => ui.close(),
                    Some(false) => self.dialog = None,
                    None => {}
                }
            }
            Dialog::NewItem { .. } | Dialog::Rename { .. } => {}
        }
    }

    /// Troca a pasta do projeto (a gaveta volta a aparecer).
    fn set_root(&mut self, path: PathBuf) {
        tracing::info!("projeto: {}", path.display());
        self.root = Some(path);
        self.expanded.clear();
        self.config.drawer = true;
        // Os servidores de linguagem rodam na pasta do projeto: começam de novo.
        self.servers.clear();
        self.failed.clear();
        for doc in &mut self.docs {
            doc.lsp_seen = None;
        }
    }

    /// Fecha a pasta do projeto: a gaveta some, os servidores de linguagem param e as
    /// abas dos arquivos dela fecham (as com alterações ficam abertas, para não perder
    /// nada). Na próxima abertura, o Code não reabre a pasta.
    fn close_folder(&mut self) {
        let Some(root) = self.root.take() else {
            return;
        };
        tracing::info!("projeto fechado: {}", root.display());
        let mut kept = 0;
        let mut i = self.docs.len();
        while i > 0 {
            i -= 1;
            if self.docs[i].path.as_ref().is_some_and(|p| p.starts_with(&root)) {
                if self.docs[i].text.modified {
                    kept += 1;
                } else {
                    self.close_tab(i, true);
                }
            }
        }
        self.expanded.clear();
        self.servers.clear();
        self.failed.clear();
        for doc in &mut self.docs {
            doc.lsp_seen = None;
        }
        let name = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if kept > 0 {
            self.say(tr!("Folder “{name}” closed; {tabs} with changes still open", name = name, tabs = trn("{n} tab", "{n} tabs", kept as u64)));
        } else {
            self.say(tr!("Folder “{name}” closed", name = name));
        }
    }

    /// Grava a sessão (projeto e abas) quando ela muda.
    fn keep_session(&mut self) {
        let s = Session {
            root: self.root.clone(),
            files: self.docs.iter().filter_map(|d| d.path.clone()).collect(),
            current: self.docs.iter().take(self.cur + 1).filter(|d| d.path.is_some()).count().saturating_sub(1),
            expanded: {
                let mut v: Vec<PathBuf> = self.expanded.iter().cloned().collect();
                v.sort();
                v
            },
        };
        if self.saved_session.as_ref() != Some(&s) {
            if let Err(err) = s.save() {
                tracing::warn!("sessão do Code não gravada: {err}");
            }
            self.saved_session = Some(s);
        }
    }

    /// Uma divisória do kit (gaveta ou terminal) acabou de ser arrastada: grava os
    /// tamanhos novos.
    fn divider_moved(&mut self) {
        tracing::info!("divisória: gaveta {:.0}, terminal {:.0}", self.config.drawer_width, self.config.terminal_height);
        if let Err(err) = self.config.save() {
            self.say(tr!("Settings not saved: {err}", err = err));
        }
    }

    /// Arrasto de um arquivo (gaveta) ou aba: mostra onde vai cair e, ao soltar perto
    /// de uma borda do editor, divide; no meio, abre no painel em foco.
    fn drag_step(&mut self, ui: &mut Ui, editor: (f32, f32, f32, f32)) {
        let input = ui.input().clone();
        let Some(drag) = self.drag.as_mut() else {
            return;
        };
        let p = input.pointer.unwrap_or(drag.from);
        if input.down {
            if !drag.active && ((p.0 - drag.from.0).abs() > 8.0 || (p.1 - drag.from.1).abs() > 8.0) {
                drag.active = true;
            }
            if drag.active {
                // Prévia: a metade do editor que o painel novo ocuparia.
                let accent = ui.theme().accent;
                let area = panes::drop_edge(editor, p).map_or(editor, |e| panes::edge_preview(editor, e));
                if input.over(editor) {
                    ui.paint(|pixmap, (ox, oy)| {
                        mide::ui::draw::fill_rounded(pixmap, ox + area.0 + 4.0, oy + area.1 + 4.0, area.2 - 8.0, area.3 - 8.0, 10.0, mide::ui::draw::rgba(accent[0], accent[1], accent[2], 48));
                    });
                }
                ui.request_repaint();
            }
            return;
        }
        let Some(drag) = self.drag.take() else {
            return;
        };
        if !drag.active || !input.over(editor) {
            return;
        }
        let edge = panes::drop_edge(editor, p);
        let doc = match drag.what {
            DragWhat::Doc(i) => Some(i),
            DragWhat::Path(path) => self.open_index(path),
        };
        let Some(doc) = doc.filter(|&d| d < self.docs.len()) else {
            return;
        };
        // Uma aba só, ou a própria aba do painel sem divisão: não há o que dividir.
        let lonely = self.docs.len() < 2 || (self.split.is_none() && doc == self.cur);
        match edge.filter(|_| !lonely) {
            Some(edge) => self.split_with(doc, edge),
            None => {
                self.cur = doc;
                self.start = true;
            }
        }
        ui.request_repaint();
    }

    /// Um painel do editor: a área de texto da aba `i` em `rect`, com o menu de
    /// contexto. Devolve a resposta da área de texto.
    fn editor_pane(&mut self, ui: &mut Ui, i: usize, rect: (f32, f32, f32, f32), dark: bool) -> mide::kit::TextAreaResponse {
        let deco = self.decorations(i, dark);
        let indent = self.config.indent();
        let (line_numbers, size, current_line) = (self.config.line_numbers, self.config.size, self.config.current_line);
        let mut deco = Some(deco);
        let mut decorate = move |_: &TextDoc| deco.take().unwrap_or_default();
        let inlays = self.inlays_for(i);
        let colors = self.editor_colors();
        let before = self.docs[i].text.version;
        let (len_before, cursor_before, modified_before) = (self.docs[i].text.text.len(), self.docs[i].text.edit.cursor, self.docs[i].text.modified);
        let mut out = None;
        ui.at(rect.0, rect.1, rect.2, |ui| {
            let look = TextAreaLook { line_numbers, size: Some(size), current_line, indent: &indent, decorate: Some(&mut decorate), inlays: &inlays, colors, ..Default::default() };
            out = Some(ui.text_area_with(&mut self.docs[i].text, rect.3, look));
        });
        let r = out.expect("área de texto");
        if i == self.cur && std::mem::take(&mut self.eat_space) {
            // O espaço do Alt+Espaço (o atalho do completar) não fica no texto.
            let doc = &mut self.docs[i].text;
            let c = doc.edit.cursor;
            if doc.text.len() == len_before + 1 && c == cursor_before + 1 && doc.text.get(c - 1..c) == Some(" ") {
                doc.text.replace_range(c - 1..c, "");
                doc.edit = mide::kit::textedit::Edit { cursor: c - 1, anchor: None };
                doc.modified = modified_before;
                doc.version += 1;
            }
        }
        if self.docs[i].text.version != before {
            self.last_edit = Some(Instant::now());
            if i == self.cur {
                self.filter_completion();
                // Digitando: o completar abre sozinho (uma letra a mais, ou um gatilho
                // como `.`, `->`, `::` e `$`).
                let doc = &self.docs[i].text;
                let typed = doc.text.len() > len_before && doc.edit.cursor > cursor_before && doc.edit.cursor - cursor_before <= 4;
                let before_cursor = doc.text[..doc.edit.cursor].to_string();
                let line_changed = doc.text.get(cursor_before.min(doc.edit.cursor)..doc.edit.cursor).is_none_or(|t| t.contains('\n'));
                if typed && self.config.auto_complete && self.completion.is_none() && auto_complete_trigger(&doc.text, doc.edit.cursor) {
                    self.complete(true);
                }
                if typed {
                    self.signature_after_typing(&before_cursor, line_changed);
                }
            }
        }
        // Ctrl+clique: vai à definição do que está sob o ponteiro (o F12 é do terminal
        // de atalho do mide e não chega aos apps).
        if r.response.clicked && ui.input().ctrl && i == self.cur
            && let Some(at) = r.pointer_index
        {
            self.docs[i].text.edit = mide::kit::textedit::Edit { cursor: at, anchor: None };
            self.go_to_definition();
        }
        // Menu de contexto do editor.
        let mut chosen = None;
        ui.context_menu(&r.response, |m| {
            for (action, label) in [("definition", tr!("Go to Definition")), ("hover", tr!("Show Info")), ("complete", tr!("Complete"))] {
                if m.item(&label) {
                    chosen = Some(action);
                }
            }
            m.separator();
            for (action, label) in [("comment", tr!("Comment Lines")), ("duplicate", tr!("Duplicate Line")), ("move-up", tr!("Move Line Up")), ("move-down", tr!("Move Line Down"))] {
                if m.item(&label) {
                    chosen = Some(action);
                }
            }
        });
        if let Some(a) = chosen {
            self.command(a, ui);
        }
        r
    }

    /// Popup de completar perto do cursor (abaixo da linha), com a janela de 10 itens
    /// que acompanha o escolhido.
    fn completion_popup(&mut self, ui: &mut Ui, area: (f32, f32, f32, f32)) {
        let Some((_, items, sel, start)) = &self.completion else {
            return;
        };
        if items.is_empty() {
            return;
        }
        let (sel, start) = (*sel, *start);
        let size = self.config.size;
        let line_numbers = self.config.line_numbers;
        let doc = &self.docs[self.cur].text;
        let starts = doc.line_starts();
        let line = line_of(&starts, doc.edit.cursor);
        let col = doc.text[starts[line]..doc.edit.cursor].chars().count() as f32;
        let line_h = (size * 1.5).round();
        let gutter = if line_numbers { size * 0.9 * 0.6 * starts.len().to_string().len().max(2) as f32 + 20.0 } else { 0.0 };
        let x = (area.0 + gutter + 8.0 + col * size * 0.6 - doc.scroll_x).min(area.0 + area.2 - 330.0).max(area.0 + 8.0);
        let y = area.1 + 6.0 + (line + 1) as f32 * line_h - doc.scroll;
        let first = panes::window_start(sel, items.len(), 10);
        // Ícone de cada tipo (função, variável, classe...), como no VS Code e no Zed.
        let shown: Vec<(String, String, String)> = items
            .iter()
            .skip(first)
            .take(10)
            .map(|c| {
                let icon = mide::code::icons::kind_icon(c.kind).unwrap_or_else(|| "text-x-generic".into());
                let detail = if c.detail.is_empty() { mide::i18n::tr(mide::code::icons::kind_name(c.kind)).into_owned() } else { c.detail.clone() };
                (icon, c.label.clone(), detail)
            })
            .collect();
        let chosen = items.get(sel).cloned();
        let width = 360.0;
        let py = y.min(area.1 + area.3 - 300.0).max(area.1);
        let mut clicked = None;
        ui.glass_panel(x, py, width, 10.0, |ui| {
            for (k, (icon, label, detail)) in shown.iter().enumerate() {
                let on = first + k == sel;
                if on {
                    ui.join_selection(false, false);
                }
                if ui.list_row(icon, label, &[(detail, 130.0)], on).clicked() {
                    clicked = Some(first + k);
                }
            }
        });
        // Documentação da sugestão escolhida ao lado (à direita, ou à esquerda se não
        // couber). Sem ela ainda, pede ao servidor (`completionItem/resolve`).
        if self.config.complete_docs
            && let Some(item) = chosen
        {
            if item.doc.is_empty()
                && let Some(raw) = item.raw.clone()
                && self.resolving.as_ref().is_none_or(|(_, l)| *l != item.label)
                && let Some(client) = self.client_for_current()
                && let Some(id) = client.resolve(raw)
            {
                self.resolving = Some((id, item.label.clone()));
            }
            let doc = doc_text(&item.doc);
            if !doc.is_empty() || (item.snippet && !item.insert.is_empty()) {
                let doc_w = 380.0_f32.min(area.2 * 0.45);
                let dx = if x + width + 6.0 + doc_w <= area.0 + area.2 { x + width + 6.0 } else { (x - 6.0 - doc_w).max(area.0) };
                ui.glass_panel(dx, py, doc_w, 10.0, |ui| {
                    if !item.detail.is_empty() {
                        ui.strong(&item.detail);
                    }
                    if !doc.is_empty() {
                        ui.label(&doc);
                    } else {
                        // Trecho sem documentação: o texto que vai entrar.
                        ui.note(&lsp::expand_snippet(&item.insert).0);
                    }
                });
            }
        }
        if let Some(k) = clicked
            && let Some(item) = self.completion.take().and_then(|(_, items, _, _)| items.into_iter().nth(k))
        {
            self.accept_completion(item, start);
        }
    }

    /// Teclas do popup de completar (tiradas da área de texto antes de ela vê-las):
    /// Tab e ↓ vão ao próximo, Shift+Tab e ↑ ao anterior, Enter aceita, Esc fecha.
    fn completion_keys(&mut self, ui: &mut Ui) {
        let Some((_, items, sel, start)) = self.completion.as_mut() else {
            return;
        };
        if ui.take_key(Key::Escape) {
            self.completion = None;
            return;
        }
        if items.is_empty() {
            return;
        }
        // Tab (0xff09) e Shift+Tab (que chega como ISO_Left_Tab, 0xfe20).
        let tabs: Vec<bool> = ui.input().presses.iter().filter(|p| !p.ctrl && matches!(p.keysym, 0xff09 | 0xfe20)).map(|p| p.shift || p.keysym == 0xfe20).collect();
        for shift in &tabs {
            *sel = panes::cycle(*sel, items.len(), !shift);
        }
        if !tabs.is_empty() {
            while ui.take_key(Key::Tab) {}
        }
        if ui.take_key(Key::Down) {
            *sel = panes::cycle(*sel, items.len(), true);
        }
        if ui.take_key(Key::Up) {
            *sel = panes::cycle(*sel, items.len(), false);
        }
        if ui.take_key(Key::Enter) {
            let (item, start) = (items.get(*sel).cloned(), *start);
            self.completion = None;
            if let Some(item) = item {
                self.accept_completion(item, start);
            }
        }
    }
}


impl App for Code {
    fn view(&mut self, ui: &mut Ui) {
        let doc_title = self.docs.get(self.cur).map(Doc::title).unwrap_or_default();
        ui.set_title(&format!("{doc_title} — Code"));
        ui.set_mono_font(self.config.font.as_deref());
        // O painel em foco mostra sempre a aba atual.
        if let Some(s) = self.split.as_mut() {
            s.docs[s.focus] = self.cur;
        }
        // Arquivos soltos na janela abrem em abas (pasta: vira o projeto).
        if let Some(paths) = ui.input().dropped.clone() {
            for p in paths {
                if p.is_dir() {
                    self.set_root(p);
                } else {
                    self.open_path(p);
                }
            }
        }
        for action in ui.input().menu.clone() {
            self.command(&action, ui);
        }
        let busy = self.dialog.is_some() || !matches!(self.overlay, Overlay::None);
        if !busy {
            self.shortcuts(ui);
        }
        self.overlay_keys(ui);
        if self.poll_lsp() | self.poll_ext_servers() {
            ui.request_repaint();
        }
        self.poll_store();
        if !self.ext_servers_idle() {
            ui.wake_after(Duration::from_millis(200));
        }
        if !self.servers.is_empty() {
            ui.wake_after(Duration::from_millis(120));
        }
        let dark = ui.theme().dark;
        let (win_w, win_h) = ui.window_size();
        let top = mide::kit::TOOLBAR_HEIGHT;
        let status_y = win_h - STATUS_H;
        let drawer_on = self.config.drawer && self.root.is_some();
        // Gaveta do projeto: a barra lateral de vidro do kit (flutuante, com os botões da
        // janela no topo, como a do Arquivos) e a divisória arrastável. A árvore é
        // desenhada dentro dela; o resto da janela é desenhado depois, nas coordenadas da
        // janela, e daqui só sai onde começa (`left`).
        let mut left = 0.0;
        if drawer_on {
            let mut width = self.config.drawer_width;
            let max = DRAWER_MAX.min(win_w - 300.0).max(DRAWER_MIN);
            let mut content_x = 0.0;
            let r = {
                // Os dois desenhos precisam do `self`: um `RefCell` empresta a um de
                // cada vez (o kit chama um depois do outro), como um mutex sem threads.
                let this = std::cell::RefCell::new(&mut *self);
                ui.sidebar_panel(
                    &mut width,
                    DRAWER_MIN,
                    max,
                    |ui| {
                        let (_, _, w, h) = ui.region();
                        this.borrow_mut().drawer(ui, 0.0, w, h);
                    },
                    |ui| content_x = ui.to_window(ui.region()).0,
                )
            };
            self.config.drawer_width = width;
            left = content_x;
            if r.submitted() {
                self.divider_moved();
            }
        }
        let width = win_w - left;
        // Barra de ferramentas (à direita da gaveta; sem ela, com os botões da janela):
        // grupos de botões de ícone, cada um com a dica.
        let mut clicked = None;
        let title = self.docs.get(self.cur).map(Doc::title).unwrap_or_default();
        let finding = self.find.is_some();
        let term_on = self.terminal_open;
        let outline_on = self.config.outline;
        ui.at(left, 0.0, width, |ui| {
            ui.toolbar(|ui| {
                if ui.tool_toggle("view-sidebar-start-symbolic", &tr!("Show or hide the project drawer"), drawer_on).clicked() {
                    clicked = Some("drawer");
                }
                ui.toolbar_title(&title, 360.0);
                ui.push_right(7.0 * mide::kit::TOOLBAR_SEG + 2.0 * 4.0 + 8.0 + 8.0);
                match ui.tool_group(&[
                    Tool::new("document-open-symbolic", &tr!("Open File (Ctrl+O)")),
                    Tool::new("folder-open-symbolic", &tr!("Open Folder (Ctrl+Shift+O)")),
                    Tool::new("document-save-symbolic", &tr!("Save (Ctrl+S)")),
                ]) {
                    Some(0) => clicked = Some("open"),
                    Some(1) => clicked = Some("open-folder"),
                    Some(2) => clicked = Some("save"),
                    _ => {}
                }
                match ui.tool_group(&[
                    Tool::new("edit-find-symbolic", &tr!("Find and Replace (Ctrl+F)")).on(finding),
                    Tool::new("utilities-terminal-symbolic", &tr!("Terminal (Ctrl+J)")).on(term_on),
                    Tool::new("view-sidebar-end-symbolic", &tr!("Symbols Panel")).on(outline_on),
                    Tool::new("preferences-system-symbolic", &tr!("Settings (Ctrl+,)")),
                ]) {
                    Some(0) => clicked = Some("find"),
                    Some(1) => clicked = Some("terminal"),
                    Some(2) => clicked = Some("outline"),
                    Some(3) => clicked = Some("prefs"),
                    _ => {}
                }
            });
        });
        if let Some(action) = clicked {
            self.command(action, ui);
        }
        // Tamanho das barras de cima do editor (abas e busca) do quadro anterior não é
        // conhecido antes de desenhar: a área do editor é medida depois delas.
        let input = ui.input().clone();
        let mut body_top = top;
        // Respiro entre a gaveta e o editor igual ao da gaveta até a borda da janela (o
        // recuo da barra lateral flutuante), e as abas longe da borda direita.
        let (cl, cw) = if drawer_on { (left + SIDE_GAP, width - SIDE_GAP) } else { (left, width) };
        ui.at(cl, top, cw - TABS_RIGHT_GAP, |ui| {
            // Abas (com mais de um arquivo); apertar numa começa um arrasto possível.
            // Abas sempre, mesmo com um arquivo só (como nos editores de código).
            if !self.docs.is_empty() {
                let titles: Vec<String> = self.docs.iter().map(Doc::name).collect();
                let modified: Vec<bool> = self.docs.iter().map(|d| d.text.modified).collect();
                // Ícone de cada arquivo na aba, como na árvore do projeto.
                let dark = ui.theme().dark;
                let icons: Vec<String> = self.docs.iter().map(|d| d.path.as_ref().and_then(|p| mide::code::icons::path_icon(p, false, false, !dark)).unwrap_or_default()).collect();
                let active_bg = self.color_theme.as_ref().and_then(|t| t.background);
                let (action, rects) = ui.editor_tabs(&titles, &icons, &modified, self.cur, active_bg);
                if input.pressed && self.drag.is_none() {
                    for (i, tab) in rects.iter().enumerate() {
                        // Fora do × (que fica à direita da aba).
                        if input.over(*tab) && !input.over((tab.0 + tab.2 - 28.0, tab.1, 28.0, tab.3)) {
                            self.drag = Some(Drag { what: DragWhat::Doc(i), from: input.pointer.unwrap_or_default(), active: false });
                        }
                    }
                }
                // Respiro entre as abas e o texto.
                ui.space(TABS_GAP);
                match action {
                    TabAction::Select(i) => self.cur = i,
                    TabAction::Close(i) => self.close_tab(i, false),
                    TabAction::New => self.command("new", ui),
                    TabAction::None => {}
                }
            }
            // Buscar e substituir no arquivo.
            if let Some(find) = self.find.as_mut() {
                let (mut next, mut prev, mut one, mut all, mut close) = (false, false, false, false, false);
                let doc = &self.docs[self.cur.min(self.docs.len() - 1)].text;
                let count = edit::count(&doc.text, &find.query, find.case);
                ui.row(|ui| {
                    if std::mem::take(&mut find.focus) {
                        ui.focus_next_input();
                    }
                    ui.sized(190.0, |ui| {
                        if ui.text_input(&mut find.query, &tr!("Find")).submitted() {
                            if ui.input().shift {
                                prev = true;
                            } else {
                                next = true;
                            }
                        }
                    });
                    ui.sized(170.0, |ui| {
                        if ui.text_input(&mut find.replace, &tr!("Replace with")).submitted() {
                            one = true;
                        }
                    });
                    let case = ui.checkbox("Aa", &mut find.case);
                    ui.tooltip(&case, &tr!("Match case"));
                    let r = ui.button("‹");
                    ui.tooltip(&r, &tr!("Find Previous"));
                    prev |= r.clicked();
                    let r = ui.button("›");
                    ui.tooltip(&r, &tr!("Find Next"));
                    next |= r.clicked();
                    one |= ui.button(&tr!("Replace")).clicked();
                    all |= ui.button(&mide::i18n::trc("replace", "All")).clicked();
                    let r = ui.button("×");
                    ui.tooltip(&r, &tr!("Close"));
                    close |= r.clicked();
                    if !find.query.is_empty() {
                        ui.note(&format!("{count}"));
                    }
                });
                if ui.input().key(Key::Escape) && ui.typing() {
                    close = true;
                }
                if next {
                    self.find_step(true);
                }
                if prev {
                    self.find_step(false);
                }
                if one {
                    self.replace_one();
                }
                if all {
                    self.replace_all();
                }
                if close {
                    self.find = None;
                    self.start = true;
                }
            }
            body_top = ui_cursor(ui);
        });
        body_top = body_top.max(top);
        // Editor em cima e terminal embaixo, com a divisória do kit (o ponteiro vira o
        // de redimensionar). Como na gaveta, os painéis são desenhados depois, nas
        // coordenadas da janela; aqui só se medem os retângulos.
        // Faixa do caminho (pastas › arquivo › símbolos) em cima do texto.
        if self.config.breadcrumbs && !self.docs.is_empty() {
            let crumbs = self.breadcrumb_line();
            self.breadcrumb_bar(ui, (cl, body_top, cw - EDGE_GAP, BREADCRUMB_H), &crumbs);
            body_top += BREADCRUMB_H;
        }
        // Painel de símbolos à direita.
        // Só com o que mostrar: servidor de linguagem para o arquivo ou símbolos lidos.
        let outline_w = if self.config.outline && self.outline_useful() { OUTLINE_W.min(cw * 0.35) } else { 0.0 };
        // Respiro em volta do editor: à direita e embaixo, para o texto não encostar na
        // borda da janela nem na barra de status.
        let body = (cl, body_top, (cw - EDGE_GAP - outline_w - if outline_w > 0.0 { SIDE_GAP } else { 0.0 }).max(120.0), (status_y - body_top - EDGE_GAP).max(60.0));
        if outline_w > 0.0 {
            let rect = (body.0 + body.2 + SIDE_GAP, body_top, outline_w, body.3);
            self.outline_panel(ui, rect);
        }
        let mut editor_rect = body;
        let mut term_rect = (cl, status_y, cw, 0.0);
        if self.terminal_open && self.terminal.is_some() {
            let mut height = self.config.terminal_height;
            let max = (body.3 - 120.0).max(80.0);
            let mut moved = None;
            ui.clip(body, |ui| {
                moved = Some(ui.split_vertical(
                    &mut height,
                    80.0,
                    max,
                    |ui| editor_rect = ui.to_window(ui.region()),
                    |ui| term_rect = ui.to_window(ui.region()),
                ));
            });
            self.config.terminal_height = height;
            if moved.is_some_and(|r| r.submitted()) {
                self.divider_moved();
            }
        }
        let term_h = term_rect.3;
        // Clique num painel do editor tira o foco do terminal; clique no terminal o dá.
        if input.pressed && self.dialog.is_none() && matches!(self.overlay, Overlay::None) {
            if term_h > 0.0 && input.over(term_rect) {
                if !self.terminal_focus {
                    self.terminal_focus = true;
                    ui.blur();
                }
            } else if input.over(editor_rect) {
                self.terminal_focus = false;
            }
        }
        // Painéis do editor (um ou dois), com a divisória do kit entre eles.
        let panes_rects: Vec<(usize, (f32, f32, f32, f32))> = match self.split.as_mut() {
            Some(s) => {
                let (mut a, mut b) = (editor_rect, editor_rect);
                let along = if s.side_by_side { editor_rect.2 } else { editor_rect.3 }.max(1.0);
                // O kit guarda o tamanho do primeiro painel (lado a lado) ou do de baixo.
                let mut size = if s.side_by_side { s.ratio } else { 1.0 - s.ratio } * along;
                let (min, max) = (along * 0.15, along * 0.85);
                let mut moved = false;
                ui.clip(editor_rect, |ui| {
                    let r = if s.side_by_side {
                        ui.split(&mut size, min, max, |ui| a = ui.to_window(ui.region()), |ui| b = ui.to_window(ui.region()))
                    } else {
                        ui.split_vertical(&mut size, min, max, |ui| a = ui.to_window(ui.region()), |ui| b = ui.to_window(ui.region()))
                    };
                    moved = r.changed();
                });
                if moved {
                    s.ratio = if s.side_by_side { size / along } else { 1.0 - size / along }.clamp(0.15, 0.85);
                }
                vec![(s.docs[0].min(self.docs.len() - 1), a), (s.docs[1].min(self.docs.len() - 1), b)]
            }
            None => vec![(self.cur.min(self.docs.len() - 1), editor_rect)],
        };
        // Clique num painel dá o foco a ele (a aba dele vira a atual).
        if let Some(s) = self.split.as_mut()
            && input.pressed
            && self.dialog.is_none()
        {
            for (k, (_, r)) in panes_rects.iter().enumerate() {
                if input.over(*r) && k != s.focus {
                    s.focus = k;
                    self.cur = s.docs[k];
                    self.completion = None;
                }
            }
        }
        let focus_pane = self.split.as_ref().map_or(0, |s| s.focus);
        if !self.terminal_focus {
            self.completion_keys(ui);
        }
        let start = std::mem::take(&mut self.start);
        let mut focused_area = editor_rect;
        for (k, (doc, rect)) in panes_rects.iter().enumerate() {
            if k == focus_pane && start && !self.terminal_focus {
                ui.focus_next_input();
            }
            let r = self.editor_pane(ui, *doc, *rect, dark);
            if k == focus_pane {
                focused_area = r.response.rect;
                focused_area = ui.to_window(focused_area);
            }
        }
        if let Some(s) = &self.split {
            // Marca do painel em foco (a divisória é do kit).
            let f = panes_rects[s.focus.min(1)].1;
            let t = ui.theme().clone();
            ui.paint(|pixmap, (ox, oy)| {
                mide::ui::draw::fill_rounded(pixmap, ox + f.0, oy + f.1, f.2, 2.0, 0.0, mide::ui::draw::rgba(t.accent[0], t.accent[1], t.accent[2], 200));
            });
        }
        if self.completion.is_none() {
            self.signature_popup(ui, focused_area);
        }
        self.mouse_hover(ui, focused_area);
        self.completion_popup(ui, focused_area);
        // Terminal integrado.
        if term_h > 0.0 {
            let size = self.config.size;
            let focused = self.terminal_focus;
            if let Some(panel) = self.terminal.as_mut() {
                if panel.read() {
                    tracing::debug!("terminal:\n{}", panel.screen_text());
                }
                if focused && self.dialog.is_none() && matches!(self.overlay, Overlay::None) {
                    let presses: Vec<_> = input.presses.iter().filter(|p| !(p.ctrl && (matches!(p.keysym, 0x60 | 0xfe50 | 0x27) || p.keysym == 0x6a))).cloned().collect();
                    // Ctrl+Shift+V cola (o Ctrl+V vai ao programa, como nos terminais).
                    if presses.iter().any(|p| p.ctrl && p.shift && (p.keysym == 0x56 || p.keysym == 0x76)) {
                        ui.request_paste();
                    }
                    let presses: Vec<_> = presses.into_iter().filter(|p| !(p.ctrl && p.shift && (p.keysym == 0x56 || p.keysym == 0x76))).collect();
                    panel.keys(&presses, input.paste.as_deref());
                }
                panel.draw(ui, term_rect, size, dark, focused);
            }
            ui.wake_after(Duration::from_millis(500));
        }
        // Rodapé (como a barra de status do TextMate): linha e coluna, linguagem,
        // recuo e o servidor de linguagem.
        let i = self.cur.min(self.docs.len() - 1);
        let (line, col) = {
            let doc = &self.docs[i].text;
            let starts = doc.line_starts();
            let l = line_of(&starts, doc.edit.cursor);
            (l + 1, doc.text[starts[l]..doc.edit.cursor].chars().count() + 1)
        };
        let lang = self.docs[i].lang;
        let lsp_state = match self.config.server_for(lang.id) {
            Some(s) if self.servers.get(&s.key()).is_some_and(|c| c.ready) => format!(" · {}", s.name()),
            Some(s) if self.failed.contains(&s.key()) || !installed(&s.command) => tr!(" · {command} not available", command = s.name()),
            Some(s) => tr!(" · {command} starting…", command = s.name()),
            None => String::new(),
        };
        let errors = self.docs[i].diags.iter().filter(|d| d.severity == Severity::Error).count();
        let warns = self.docs[i].diags.iter().filter(|d| d.severity == Severity::Warning).count();
        let problems = if errors + warns > 0 { format!(" · {}, {}", trn("{n} error", "{n} errors", errors as u64), trn("{n} warning", "{n} warnings", warns as u64)) } else { String::new() };
        // O caminho dos símbolos em volta do cursor (como os breadcrumbs do VS Code).
        let crumbs = match self.breadcrumbs() {
            c if c.is_empty() => String::new(),
            c => format!(" · {c}"),
        };
        let tabs = if self.config.soft_tabs { tr!("Spaces: {n}", n = self.config.tab_size) } else { mide::i18n::trc("indent", "Tabs").into_owned() };
        let note = match &self.message {
            Some((m, at)) if at.elapsed() < Duration::from_secs(6) => format!("  —  {m}"),
            _ => String::new(),
        };
        ui.at(left, status_y, win_w - left, |ui| {
            ui.status_bar(&format!("{}{crumbs} · {} · {tabs}{lsp_state}{problems}{note}", tr!("Line {line}, Column {col}", line = line, col = col), lang.label()));
        });
        // Dica do símbolo (Ctrl+I).
        if let Some((_, text)) = &self.hover
            && !text.is_empty()
        {
            let text: String = text.lines().take(12).collect::<Vec<_>>().join("\n");
            let mut close = ui.input().key(Key::Escape) || ui.input().pressed;
            ui.glass_panel(left + 40.0, top + 60.0, (win_w - left - 80.0).min(640.0), 14.0, |ui| {
                for l in text.lines() {
                    ui.label(l);
                }
                close |= ui.button(&tr!("Close")).clicked();
            });
            if close {
                self.hover = None;
            }
        }
        // Arrasto de arquivos/abas para as bordas do editor.
        self.drag_step(ui, editor_rect);
        self.overlays(ui);
        self.dialogs(ui);
        // O servidor de linguagem acompanha o texto.
        self.sync_lsp();
        self.refresh_lsp_extras();
        self.refresh_git();
        if self.last_edit.is_some_and(|t| t.elapsed() < Duration::from_millis(400)) {
            ui.wake_after(Duration::from_millis(300));
        }
        self.keep_session();
    }

    /// Janelas filhas: só as Preferências.
    fn window_view(&mut self, id: &str, ui: &mut Ui) {
        if id == PREFS {
            self.prefs(ui);
        }
    }

    /// Preferências fechadas: grava o que mudou.
    fn window_closed(&mut self, id: &str) {
        if id == PREFS {
            tracing::info!("preferências fechadas e gravadas");
            if let Err(err) = self.config.save() {
                self.say(tr!("Settings not saved: {err}", err = err));
            }
        }
    }

    /// Menus do app na barra superior (como os do TextMate).
    fn menu_bar(&mut self) -> Vec<MenuGroup> {
        let langs: Vec<MenuItem> = lang::LANGUAGES
            .iter()
            .map(|l| MenuItem::action(&format!("lang:{}", l.id), &l.label()).checked(self.docs.get(self.cur).is_some_and(|d| d.lang.id == l.id)))
            .collect();
        let tabs: Vec<MenuItem> = self.docs.iter().enumerate().map(|(i, d)| MenuItem::action(&format!("tab:{i}"), &d.title()).checked(i == self.cur)).collect();
        vec![
            MenuGroup {
                title: tr!("File").into(),
                items: vec![
                    MenuItem::action("new", &tr!("New")).shortcut("Ctrl+N"),
                    MenuItem::action("open", &tr!("Open…")).shortcut("Ctrl+O"),
                    MenuItem::action("open-folder", &tr!("Open Folder…")).shortcut("Ctrl+Shift+O"),
                    MenuItem::action("close-folder", &tr!("Close Folder")).enabled(self.root.is_some()),
                    MenuItem::Separator,
                    MenuItem::action("save", &tr!("Save")).shortcut("Ctrl+S"),
                    MenuItem::action("save-as", &tr!("Save As…")).shortcut("Ctrl+Shift+S"),
                    MenuItem::action("save-all", &tr!("Save All")).shortcut("Ctrl+Alt+S"),
                    MenuItem::Separator,
                    MenuItem::action("close-tab", &tr!("Close Tab")).shortcut("Ctrl+W"),
                    MenuItem::action("quit", &tr!("Quit")).shortcut("Ctrl+Q"),
                ],
            },
            MenuGroup {
                title: tr!("Edit").into(),
                items: vec![
                    MenuItem::action("find", &tr!("Find and Replace")).shortcut("Ctrl+F"),
                    MenuItem::action("find-next", &tr!("Find Next")).shortcut("Ctrl+G"),
                    MenuItem::action("find-prev", &tr!("Find Previous")).shortcut("Ctrl+Shift+G"),
                    MenuItem::action("find-project", &tr!("Find in Project…")).shortcut("Ctrl+Shift+F"),
                    MenuItem::Separator,
                    MenuItem::action("comment", &tr!("Comment Lines")).shortcut("Ctrl+/"),
                    MenuItem::action("duplicate", &tr!("Duplicate Line")).shortcut("Ctrl+Shift+D"),
                    MenuItem::action("move-up", &tr!("Move Line Up")).shortcut("Alt+↑"),
                    MenuItem::action("move-down", &tr!("Move Line Down")).shortcut("Alt+↓"),
                    MenuItem::Separator,
                    MenuItem::action("complete", &tr!("Complete")).shortcut("Alt+Space"),
                    MenuItem::action("rename", &tr!("Rename Symbol…")).shortcut("F2"),
                    MenuItem::action("code-actions", &tr!("Code Actions…")).shortcut("Ctrl+."),
                    MenuItem::action("format", &tr!("Format Document")).shortcut("Ctrl+Shift+I"),
                    MenuItem::action("prefs", &tr!("Settings…")).shortcut("Ctrl+,"),
                    MenuItem::action("edit-servers", &tr!("Language Servers…")),
                ],
            },
            MenuGroup {
                title: tr!("View").into(),
                items: vec![
                    MenuItem::action("palette", &tr!("Command Palette…")).shortcut("Ctrl+Shift+P"),
                    MenuItem::action("outline", &tr!("Symbols Panel")).checked(self.config.outline),
                    MenuItem::action("breadcrumbs", &tr!("Path Bar")).checked(self.config.breadcrumbs),
                    MenuItem::action("extensions", &tr!("Extensions…")).shortcut("Ctrl+Shift+X"),
                    MenuItem::Separator,
                    MenuItem::action("drawer", &tr!("File Drawer")).checked(self.config.drawer),
                    MenuItem::action("line-numbers", &tr!("Line Numbers")).checked(self.config.line_numbers),
                    MenuItem::action("terminal", &tr!("Terminal")).shortcut("Ctrl+J").checked(self.terminal_open),
                    MenuItem::Separator,
                    MenuItem::action("split-right", &tr!("Split Right")).shortcut("Ctrl+\\"),
                    MenuItem::action("split-down", &tr!("Split Down")).shortcut("Ctrl+Shift+\\"),
                    MenuItem::action("unsplit", &tr!("Merge Panes")),
                    MenuItem::Separator,
                    MenuItem::action("zoom-in", &tr!("Bigger Font")).shortcut("Ctrl+="),
                    MenuItem::action("zoom-out", &tr!("Smaller Font")).shortcut("Ctrl+-"),
                    MenuItem::action("zoom-reset", &tr!("Default Font Size")).shortcut("Ctrl+0"),
                    MenuItem::Separator,
                    MenuItem::Submenu { label: mide::i18n::trc("programming", "Language").into(), items: langs },
                ],
            },
            MenuGroup {
                title: tr!("Navigate").into(),
                items: vec![
                    MenuItem::action("goto-file", &tr!("Go to File…")).shortcut("Ctrl+T"),
                    MenuItem::action("goto-line", &tr!("Go to Line…")).shortcut("Ctrl+L"),
                    MenuItem::Separator,
                    MenuItem::action("definition", &tr!("Go to Definition (or Ctrl+Click)")).shortcut("F12"),
                    MenuItem::action("hover", &tr!("Show Symbol Info")).shortcut("Ctrl+I"),
                    MenuItem::action("symbols", &tr!("Go to Symbol…")).shortcut("Ctrl+R"),
                    MenuItem::action("problems", &tr!("Problems…")).shortcut("Ctrl+Shift+M"),
                    MenuItem::action("next-problem", &tr!("Next Problem")).shortcut("F8"),
                    MenuItem::action("prev-problem", &tr!("Previous Problem")).shortcut("Shift+F8"),
                ],
            },
            window_menu(tabs),
        ]
    }

    /// Fechar a janela com abas alteradas: pergunta antes.
    fn close_requested(&mut self) -> bool {
        if !self.docs.iter().any(|d| d.text.modified) {
            return true;
        }
        self.dialog = Some(Dialog::ConfirmQuit);
        false
    }
}

/// Onde o próximo item da área atual começa (na janela, para as áreas da raiz).
/// O texto acabou de ganhar algo que abre o completar? Uma letra de palavra, ou um
/// gatilho de membro (`.`, `->`, `::`) ou de variável (`$`).
/// Documentação do LSP (markdown) como texto para ler: sem as cercas de código, sem
/// `**`/`__`, títulos sem `#`, no máximo 24 linhas.
fn doc_text(doc: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    for line in doc.lines() {
        let t = line.trim_end();
        if t.trim_start().starts_with("```") {
            continue;
        }
        let t = t.trim_start_matches('#').trim_start_matches(' ').replace("**", "").replace("__", "").replace('`', "");
        // Uma linha vazia só entre parágrafos.
        if t.trim().is_empty() && out.last().is_none_or(|l| l.is_empty()) {
            continue;
        }
        out.push(t);
        if out.len() >= 24 {
            break;
        }
    }
    while out.last().is_some_and(|l| l.is_empty()) {
        out.pop();
    }
    out.join("\n")
}

/// A tecla `keysym` é a do nome do `keymap.json` do Zed ("p", "f2", "enter", ".")?
fn key_matches(keysym: u32, key: &str) -> bool {
    let named = match key {
        "enter" => Some(0xff0d),
        "tab" => Some(0xff09),
        "escape" => Some(0xff1b),
        "space" => Some(0x20),
        "backspace" => Some(0xff08),
        "delete" => Some(0xffff),
        "up" => Some(0xff52),
        "down" => Some(0xff54),
        "left" => Some(0xff51),
        "right" => Some(0xff53),
        "home" => Some(0xff50),
        "end" => Some(0xff57),
        "pageup" => Some(0xff55),
        "pagedown" => Some(0xff56),
        k if k.len() > 1 && k.starts_with('f') => k[1..].parse::<u32>().ok().filter(|n| (1..=24).contains(n)).map(|n| 0xffbe + n - 1),
        _ => None,
    };
    if let Some(code) = named {
        return keysym == code || (code == 0xff09 && keysym == 0xfe20);
    }
    // Letras e sinais: o keysym é o próprio caractere (maiúscula com Shift).
    let mut chars = key.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => char::from_u32(keysym).is_some_and(|k| k.to_lowercase().eq(c.to_lowercase())),
        _ => false,
    }
}

fn auto_complete_trigger(text: &str, cursor: usize) -> bool {
    let before = &text[..cursor];
    let Some(c) = before.chars().next_back() else {
        return false;
    };
    if c.is_alphanumeric() || c == '_' {
        // Não no meio de um número.
        let word: String = before.chars().rev().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
        return !word.chars().last().is_some_and(|c| c.is_ascii_digit());
    }
    before.ends_with('.') && !before.ends_with("..") || before.ends_with("->") || before.ends_with("::") || c == '$'
}

/// Sugestões sem servidor: as palavras do arquivo (três letras ou mais, as mais
/// frequentes primeiro) e as palavras-chave da linguagem que começam com o que foi
/// digitado (de `start` a `cursor`), sem a própria palavra.
fn local_completions<'a>(text: &str, start: usize, cursor: usize, keywords: impl Iterator<Item = (&'a str, u8)>) -> Vec<Completion> {
    let typed = text[start..cursor].to_lowercase();
    let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let mut word_at: Option<usize> = None;
    for (i, c) in text.char_indices().chain(std::iter::once((text.len(), ' '))) {
        match (word_at, is_word(c) && i < text.len()) {
            (None, true) => word_at = Some(i),
            (Some(at), false) => {
                word_at = None;
                let word = &text[at..i];
                // A palavra que está sendo digitada não conta.
                if at != start && word.chars().count() >= 3 && !word.starts_with(|c: char| c.is_ascii_digit()) {
                    *counts.entry(word).or_default() += 1;
                }
            }
            _ => {}
        }
    }
    let mut words: Vec<(&str, usize)> = counts.into_iter().filter(|(w, _)| w.to_lowercase().starts_with(&typed) && w.to_lowercase() != typed).collect();
    words.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    let mut out: Vec<Completion> = Vec::new();
    for (kw, kind) in keywords {
        if !typed.is_empty() && kw.to_lowercase().starts_with(&typed) && kw.to_lowercase() != typed && !out.iter().any(|c| c.label == kw) {
            let detail = if kind == 14 { tr!("keyword") } else { tr!("type") }.into_owned();
            out.push(Completion { label: kw.into(), detail, insert: kw.into(), kind, ..Default::default() });
        }
    }
    for (w, _) in words.into_iter().take(40) {
        if !out.iter().any(|c| c.label == w) {
            out.push(Completion { label: w.into(), insert: w.into(), ..Default::default() });
        }
    }
    out
}

/// Respiro do editor até a borda direita da janela e até a barra de status.
const EDGE_GAP: f32 = 10.0;
/// Altura da faixa do caminho em cima do texto.
const BREADCRUMB_H: f32 = 24.0;
/// Largura do painel de símbolos.
const OUTLINE_W: f32 = 270.0;
/// Respiro entre a gaveta do projeto e o editor (o recuo da barra lateral flutuante).
const SIDE_GAP: f32 = 8.0;
/// Respiro à direita da barra de abas (o + não encosta na borda da janela), como o
/// da barra de ferramentas.
const TABS_RIGHT_GAP: f32 = 12.0;
/// Espaço entre as abas e o texto.
const TABS_GAP: f32 = 6.0;

fn ui_cursor(ui: &Ui) -> f32 {
    let r = ui.region();
    r.1 + r.3 - ui.remaining_height()
}

/// Pasta sugerida para um arquivo novo: a do projeto, Documentos, ou a pasta pessoal.
fn default_dir(root: Option<&Path>) -> PathBuf {
    if let Some(root) = root {
        return root.to_path_buf();
    }
    let home = mide::places::home_dir();
    ["Documentos", "Documents"].iter().map(|d| home.join(d)).find(|d| d.is_dir()).unwrap_or(home)
}

fn main() -> Result<(), Box<dyn Error>> {
    let _logging = mide::logging::init("mide-code", "mide_code=info");
    let args: Vec<PathBuf> = std::env::args().skip(1).filter(|a| !a.starts_with('-')).map(PathBuf::from).collect();
    let window = Window::new("Code").app_id("mide-code").size(1040, 640).min_size(560, 360).full_size_content().accepts_files();
    run(window, Code::new(args))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completar_abre_ao_digitar() {
        let t = "$teste = PD";
        assert!(auto_complete_trigger(t, t.len()));
        let t = "obj.";
        assert!(auto_complete_trigger(t, t.len()));
        let t = "$this->";
        assert!(auto_complete_trigger(t, t.len()));
        let t = "Config::";
        assert!(auto_complete_trigger(t, t.len()));
        let t = "$";
        assert!(auto_complete_trigger(t, t.len()));
        // Número, espaço, intervalo (`..`) e `-` sozinho não abrem.
        for t in ["x = 12", "a ", "0..", "a -"] {
            assert!(!auto_complete_trigger(t, t.len()), "{t}");
        }
    }

    #[test]
    fn palavras_do_arquivo_e_da_linguagem() {
        let text = "$appName = 1;\n$appDescription = 2;\n$appName;\n$teste = app";
        let cursor = text.len();
        let start = Code::word_start(text, cursor);
        let got = local_completions(text, start, cursor, ["array", "abstract", "as"].into_iter().map(|k| (k, 14)));
        let labels: Vec<&str> = got.iter().map(|c| c.label.as_str()).collect();
        // Palavras-chave primeiro; das palavras, a mais usada antes; nunca a própria palavra.
        assert_eq!(labels, ["appName", "appDescription"]);
        let t = "ação açúcar aç";
        let got = local_completions(t, Code::word_start(t, t.len()), t.len(), std::iter::empty());
        assert_eq!(got.iter().map(|c| c.label.as_str()).collect::<Vec<_>>(), ["ação", "açúcar"]);
    }
}
