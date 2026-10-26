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

mod term;
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
        self.path.as_ref().and_then(|p| p.file_name()).map_or_else(|| "sem título".to_string(), |n| n.to_string_lossy().into_owned())
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
}

fn read_text(path: &Path) -> std::io::Result<(String, bool)> {
    if std::fs::metadata(path)?.len() > MAX_BYTES {
        return Err(std::io::Error::other("arquivo grande demais para o editor"));
    }
    let bytes = std::fs::read(path)?;
    let text = String::from_utf8(bytes).map_err(|_| std::io::Error::other("não é texto UTF-8 (arquivo binário?)"))?;
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
                self.say(format!("Não foi possível abrir \"{}\": {err}", path.display()));
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
            self.dialog = Some(Dialog::SaveAs(FileChooser::save("Salvar como", Some(&dir), "sem título.txt"), then_close));
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
                let cmd = self.config.server_for(doc.lang.id).map(|s| s.command.clone());
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
                self.say(format!("Não foi possível salvar: {err}"));
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
            self.say(format!("{n} arquivo(s) salvo(s); {unnamed} sem nome (use Salvar como)"));
        } else {
            self.say(format!("{n} arquivo(s) salvo(s)"));
        }
    }

    /// O `code-servers.toml` foi salvo: troca os servidores (com erro, mantém os de
    /// antes e mostra a linha errada na barra de status).
    fn reload_servers(&mut self, text: &str) {
        match parse_servers(text) {
            Ok(servers) => {
                tracing::info!("servidores de linguagem lidos: {}", servers.iter().map(|s| s.command.as_str()).collect::<Vec<_>>().join(", "));
                self.config.servers = servers;
                // Reinicia todos: o comando ou os argumentos podem ter mudado.
                self.servers.clear();
                self.failed.clear();
                for doc in &mut self.docs {
                    doc.lsp_seen = None;
                    doc.diags.clear();
                }
                let n = self.config.servers.len();
                self.say(format!("Servidores de linguagem atualizados ({n})"));
            }
            Err(err) => self.say(format!("Erro no code-servers.toml, {err}")),
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
        if let (Some(path), Some(cmd)) = (&doc.path, self.config.server_for(doc.lang.id).map(|s| s.command.clone()))
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
        let lang = self.doc().lang.id;
        let server: Server = self.config.server_for(lang)?.clone();
        if self.failed.contains(&server.command) {
            return None;
        }
        if !self.servers.contains_key(&server.command) {
            if !installed(&server.command) {
                self.failed.insert(server.command.clone());
                return None;
            }
            let root = self.root.clone().or_else(|| self.doc().path.as_ref().and_then(|p| p.parent().map(Path::to_path_buf))).unwrap_or_else(mide::places::home_dir);
            match Client::start(&server.command, &server.args, &root) {
                Ok(client) => {
                    tracing::info!("servidor de linguagem: {} ({})", server.command, root.display());
                    self.servers.insert(server.command.clone(), client);
                }
                Err(err) => {
                    self.say(format!("Não foi possível iniciar {}: {err}", server.command));
                    self.failed.insert(server.command.clone());
                    return None;
                }
            }
        }
        self.servers.get_mut(&server.command)
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
                        *list = items;
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
                Event::Hover { id, text } => {
                    if self.hover.as_ref().is_some_and(|(want, _)| *want == id) {
                        self.hover = Some((id, text));
                    }
                }
                Event::Exited(text) => self.say(text),
            }
        }
        changed
    }

    /// Começo da palavra antes do cursor (para completar).
    fn word_start(text: &str, cursor: usize) -> usize {
        text[..cursor].char_indices().rev().take_while(|(_, c)| c.is_alphanumeric() || *c == '_').last().map_or(cursor, |(i, _)| i)
    }

    fn request_completion(&mut self) {
        let cursor = self.doc().text.edit.cursor;
        let text = self.doc().text.text.clone();
        let path = self.doc().path.clone();
        self.sync_now();
        let (Some(path), Some(client)) = (path, self.client_for_current()) else {
            self.say("Completar precisa de um servidor de linguagem (Preferências).");
            return;
        };
        if let Some(id) = client.completion(&path, &text, cursor) {
            self.completion = Some((id, Vec::new(), 0, Self::word_start(&text, cursor)));
        }
    }

    /// A lista de completar acompanha a palavra digitada (e some fora dela).
    fn filter_completion(&mut self) {
        let i = self.cur.min(self.docs.len() - 1);
        let Some((_, items, sel, start)) = self.completion.as_mut() else {
            return;
        };
        let doc = &self.docs[i].text;
        let cursor = doc.edit.cursor;
        if cursor < *start || doc.text[*start..cursor].contains(|c: char| !c.is_alphanumeric() && c != '_') {
            self.completion = None;
            return;
        }
        let word = doc.text[*start..cursor].to_lowercase();
        if !word.is_empty() {
            items.retain(|it| it.label.to_lowercase().starts_with(&word));
        }
        *sel = (*sel).min(items.len().saturating_sub(1));
    }

    /// Põe o item escolhido no lugar da palavra.
    fn accept_completion(&mut self, item: Completion, start: usize) {
        let doc = &mut self.doc().text;
        let cursor = doc.edit.cursor;
        doc.before_change(mide::kit::textarea::Change::Other);
        doc.text.replace_range(start.min(cursor)..cursor, &item.insert);
        let at = start.min(cursor) + item.insert.len();
        doc.edit = mide::kit::textedit::Edit { cursor: at, anchor: None };
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
            self.say("Ir para a definição precisa de um servidor de linguagem (Preferências).");
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
        let utf8 = self.config.server_for(lang).and_then(|s| self.servers.get(&s.command)).is_none_or(|c| c.utf8);
        let doc = self.doc();
        if doc.diags.is_empty() {
            self.say("Nenhum problema neste arquivo");
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
            None => self.say(format!("\"{query}\" não encontrado")),
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
        self.say(format!("{n} substituição(ões)"));
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
            Ok((files, n)) => self.say(format!("{} substituição(ões) em {files} arquivo(s) e {in_open} nas abas abertas", n)),
            Err(err) => self.say(format!("Erro ao substituir: {err}")),
        }
    }

    /// Cores do código e erros da aba `i`, para a área de texto.
    fn decorations(&mut self, i: usize, dark: bool) -> Decorations {
        let doc = &mut self.docs[i];
        if doc.colors.0 != doc.text.version {
            let spans = highlight::highlight(&doc.text.text, doc.lang);
            doc.colors = (doc.text.version, spans.into_iter().map(|(r, k)| (r, highlight::color(k, dark))).collect());
        }
        let utf8 = self.config.server_for(doc.lang.id).and_then(|s| self.servers.get(&s.command)).is_none_or(|c| c.utf8);
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
                    self.say(format!("Não foi possível abrir o terminal: {err}"));
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
                self.dialog = Some(Dialog::Open(FileChooser::open("Abrir", Some(&dir)).multiple(true)));
            }
            "open-folder" => {
                let dir = self.root.clone().unwrap_or_else(mide::places::home_dir);
                self.dialog = Some(Dialog::OpenFolder(FileChooser::folder("Abrir pasta", Some(&dir))));
            }
            "save" => {
                self.save(false);
            }
            "save-all" => self.save_all(),
            "save-as" => {
                let dir = self.doc().path.as_ref().and_then(|p| p.parent().map(Path::to_path_buf)).or_else(|| self.root.clone()).unwrap_or_else(mide::places::home_dir);
                let name = self.doc().name();
                self.dialog = Some(Dialog::SaveAs(FileChooser::save("Salvar como", Some(&dir), &name), false));
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
                    self.say("Abra uma pasta (Arquivo → Abrir pasta) para buscar no projeto.");
                } else {
                    let doc = &self.docs[self.cur.min(self.docs.len() - 1)].text;
                    let query = doc.edit.selected(&doc.text).filter(|s| !s.contains('\n')).unwrap_or("").to_string();
                    self.overlay = Overlay::ProjectFind(ProjectFind { query, focus: true, ..Default::default() });
                }
            }
            "goto-file" => {
                let files = self.root.as_ref().map(|r| project::files(r, 20_000)).unwrap_or_default();
                if files.is_empty() {
                    self.say("Abra uma pasta (Arquivo → Abrir pasta) para ir para os arquivos dela.");
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
                    ui.open_window(PREFS, Window::new("Preferências do Code").size(560, 470).min_size(480, 400).glass());
                }
            }
            "edit-servers" => {
                self.overlay = Overlay::None;
                match self.config.ensure_servers_file() {
                    Ok(path) => self.open_path(path),
                    Err(err) => self.say(format!("Não foi possível criar o arquivo dos servidores: {err}")),
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
        for (key, action) in [("n", "new"), ("w", "close-tab"), ("q", "quit"), ("t", "goto-file"), ("l", "goto-line"), ("/", "comment"), (",", "prefs"), ("i", "hover"), ("=", "zoom-in"), ("+", "zoom-in"), ("-", "zoom-out"), ("0", "zoom-reset")] {
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
        for p in &input.presses {
            match p.keysym {
                // Ctrl+Espaço ou Alt+Espaço: completar (na sessão do mide, Ctrl+Espaço
                // é o buscador e não chega aos apps).
                0x20 if p.ctrl || p.alt => {
                    self.eat_space = p.alt;
                    run.push("complete");
                }
                // F12: definição; F8 (Shift: anterior): próximo problema.
                0xffc9 => run.push("definition"),
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
        // Ctrl+Tab e Ctrl+Shift+Tab trocam de aba.
        if input.key_events().iter().any(|(k, ctrl, _)| *k == Key::Tab && *ctrl) && self.docs.len() > 1 {
            let back = input.key_events().iter().any(|(k, ctrl, sh)| *k == Key::Tab && *ctrl && *sh);
            self.cur = if back { (self.cur + self.docs.len() - 1) % self.docs.len() } else { (self.cur + 1) % self.docs.len() };
            ui.take_key(Key::Tab);
        }
        for action in run {
            self.command(action, ui);
        }
    }

    fn overlays(&mut self, ui: &mut Ui) {
        let esc = ui.input().key(Key::Escape);
        match &mut self.overlay {
            Overlay::None => {}
            Overlay::GoToFile { query, files, sel, focus } => {
                let (up, down, enter) = (ui.take_key(Key::Up), ui.take_key(Key::Down), ui.input().key(Key::Enter));
                let mut chosen = None;
                let root = self.root.clone();
                ui.modal(560.0, |ui| {
                    ui.strong("Ir para arquivo");
                    if std::mem::take(focus) {
                        ui.focus_next_input();
                    }
                    ui.text_input(query, "Nome do arquivo (letras em qualquer parte)");
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
                    ui.strong("Ir para a linha");
                    if std::mem::take(focus) {
                        ui.focus_next_input();
                    }
                    if ui.text_input(text, "Linha (ou linha:coluna)").submitted() {
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
            ui.strong("Buscar no projeto");
            if std::mem::take(&mut pf.focus) {
                ui.focus_next_input();
            }
            run_search |= ui.text_input(&mut pf.query, "Buscar").submitted();
            ui.text_input(&mut pf.replace, "Substituir por");
            ui.row(|ui| {
                if ui.checkbox("Diferenciar maiúsculas", &mut pf.case).changed() {
                    run_search = true;
                }
                ui.push_right(2.0 * 170.0 + 8.0);
                ui.sized(170.0, |ui| replace |= ui.button("Substituir em tudo").clicked());
                ui.sized(170.0, |ui| run_search |= ui.primary_button("Buscar").clicked());
            });
            match &pf.searched {
                Some(q) if pf.hits.is_empty() => ui.note(&format!("Nada encontrado para \"{q}\".")),
                Some(_) => ui.note(&format!("{} linha(s){}", pf.hits.len(), if pf.hits.len() >= 500 { " (as primeiras 500)" } else { "" })),
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
                ui.sized(110.0, |ui| close |= ui.button("Fechar").clicked());
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
        let fonts: Vec<String> = std::iter::once("Fonte do sistema".to_string()).chain(ui.mono_fonts()).collect();
        let mut font_i = c.font.as_ref().and_then(|f| fonts.iter().position(|x| x == f)).unwrap_or(0);
        let servers_running: Vec<String> = self.servers.keys().cloned().collect();
        let servers_file = "code-servers.toml";
        ui.page(|ui| {
            ui.card(|ui| {
                if ui.dropdown("Fonte", &fonts, &mut font_i).changed() {
                    c.font = (font_i > 0).then(|| fonts[font_i].clone());
                }
                let mut size = c.size;
                ui.slider("Tamanho da fonte", &mut size, 9.0, 28.0);
                c.size = size.round();
                ui.switch("Números de linha", &mut c.line_numbers);
                ui.switch("Destacar a linha atual", &mut c.current_line);
                ui.switch("Recuo com espaços (desligado: tabulação)", &mut c.soft_tabs);
                let mut tab = c.tab_size as f32;
                ui.slider("Espaços por recuo", &mut tab, 1.0, 8.0);
                c.tab_size = tab.round() as usize;
                ui.switch("Gaveta do projeto à esquerda", &mut c.drawer);
            });
            // Servidores: um resumo (a lista fica no arquivo, editado no próprio Code).
            let running: Vec<&str> = c.servers.iter().filter(|s| s.enabled && servers_running.contains(&s.command)).map(|s| s.command.as_str()).collect();
            let ready: Vec<&str> = c.servers.iter().filter(|s| s.enabled && installed(&s.command) && !servers_running.contains(&s.command)).map(|s| s.command.as_str()).collect();
            let mut summary = format!("{} configurado(s)", c.servers.len());
            if !running.is_empty() {
                summary.push_str(&format!(" · rodando: {}", running.join(", ")));
            }
            if !ready.is_empty() {
                summary.push_str(&format!(" · instalados: {}", ready.join(", ")));
            }
            ui.note(&format!("Servidores de linguagem (LSP): {summary}. Ficam no {servers_file}, que vale ao salvar."));
            ui.row(|ui| {
                edit_servers |= ui.button("Editar servidores de linguagem…").clicked();
                ui.push_right(110.0);
                ui.sized(110.0, |ui| close |= ui.primary_button("Pronto").clicked());
            });
        });
        if edit_servers {
            // O arquivo abre numa aba da janela principal.
            match self.config.ensure_servers_file() {
                Ok(path) => self.open_path(path),
                Err(err) => self.say(format!("Não foi possível criar o arquivo dos servidores: {err}")),
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
                    ui.strong(&format!("Salvar as alterações de \"{name}\"?"));
                    ui.note("Sem salvar, as alterações se perdem.");
                    ui.row(|ui| {
                        if ui.button("Não salvar").clicked() {
                            choice = Some(0);
                        }
                        ui.push_right(2.0 * 110.0 + 8.0);
                        ui.sized(110.0, |ui| {
                            if ui.button("Cancelar").clicked() {
                                choice = Some(1);
                            }
                        });
                        ui.sized(110.0, |ui| {
                            if ui.primary_button("Salvar").clicked() {
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
                    ui.strong(&format!("{n} arquivo(s) com alterações não salvas"));
                    ui.note("Sair sem salvar perde as alterações.");
                    ui.row(|ui| {
                        ui.push_right(2.0 * 130.0 + 8.0);
                        ui.sized(130.0, |ui| {
                            if ui.button("Cancelar").clicked() {
                                choice = Some(false);
                            }
                        });
                        ui.sized(130.0, |ui| {
                            if ui.primary_button("Sair sem salvar").clicked() {
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
            self.say(format!("Preferências não gravadas: {err}"));
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
        let before = self.docs[i].text.version;
        let (len_before, cursor_before, modified_before) = (self.docs[i].text.text.len(), self.docs[i].text.edit.cursor, self.docs[i].text.modified);
        let mut out = None;
        ui.at(rect.0, rect.1, rect.2, |ui| {
            let look = TextAreaLook { line_numbers, size: Some(size), current_line, indent: &indent, decorate: Some(&mut decorate), ..Default::default() };
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
            for (action, label) in [("definition", "Ir para a definição"), ("hover", "Mostrar informações"), ("complete", "Completar")] {
                if m.item(label) {
                    chosen = Some(action);
                }
            }
            m.separator();
            for (action, label) in [("comment", "Comentar linhas"), ("duplicate", "Duplicar linha"), ("move-up", "Mover linha para cima"), ("move-down", "Mover linha para baixo")] {
                if m.item(label) {
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
        let shown: Vec<(String, String)> = items.iter().skip(first).take(10).map(|c| (c.label.clone(), c.detail.clone())).collect();
        let mut clicked = None;
        ui.glass_panel(x, y.min(area.1 + area.3 - 300.0).max(area.1), 320.0, 10.0, |ui| {
            for (k, (label, detail)) in shown.iter().enumerate() {
                let on = first + k == sel;
                if on {
                    ui.join_selection(false, false);
                }
                if ui.list_row("code-function-symbolic", label, &[(detail, 120.0)], on).clicked() {
                    clicked = Some(first + k);
                }
            }
        });
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
        if self.poll_lsp() {
            ui.request_repaint();
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
        ui.at(left, 0.0, width, |ui| {
            ui.toolbar(|ui| {
                if ui.tool_toggle("view-sidebar-start-symbolic", "Mostrar ou esconder a gaveta do projeto", drawer_on).clicked() {
                    clicked = Some("drawer");
                }
                ui.toolbar_title(&title, 360.0);
                ui.push_right(2.0 * (3.0 * 40.0 + 4.0) + 8.0 + 8.0);
                match ui.tool_group(&[
                    Tool::new("document-open-symbolic", "Abrir arquivo (Ctrl+O)"),
                    Tool::new("folder-open-symbolic", "Abrir pasta (Ctrl+Shift+O)"),
                    Tool::new("document-save-symbolic", "Salvar (Ctrl+S)"),
                ]) {
                    Some(0) => clicked = Some("open"),
                    Some(1) => clicked = Some("open-folder"),
                    Some(2) => clicked = Some("save"),
                    _ => {}
                }
                match ui.tool_group(&[
                    Tool::new("edit-find-symbolic", "Buscar e substituir (Ctrl+F)").on(finding),
                    Tool::new("utilities-terminal-symbolic", "Terminal (Ctrl+J)").on(term_on),
                    Tool::new("preferences-system-symbolic", "Preferências (Ctrl+,)"),
                ]) {
                    Some(0) => clicked = Some("find"),
                    Some(1) => clicked = Some("terminal"),
                    Some(2) => clicked = Some("prefs"),
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
        ui.at(left, top, width, |ui| {
            // Abas (com mais de um arquivo); apertar numa começa um arrasto possível.
            if self.docs.len() > 1 {
                let titles: Vec<String> = self.docs.iter().map(Doc::title).collect();
                let action = ui.tab_bar(&titles, self.cur);
                let bar = ui.last_rect();
                let track_w = bar.2 - 28.0 - 8.0;
                let tw = (track_w - 4.0) / titles.len() as f32;
                if input.pressed && self.drag.is_none() {
                    for i in 0..titles.len() {
                        let tab = (bar.0 + 2.0 + i as f32 * tw, bar.1 + 3.0, tw, 26.0);
                        // Fora do × (que fica à esquerda da aba).
                        if input.over(tab) && !input.over((tab.0, tab.1, 24.0, tab.3)) {
                            self.drag = Some(Drag { what: DragWhat::Doc(i), from: input.pointer.unwrap_or_default(), active: false });
                        }
                    }
                }
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
                        if ui.text_input(&mut find.query, "Buscar").submitted() {
                            if ui.input().shift {
                                prev = true;
                            } else {
                                next = true;
                            }
                        }
                    });
                    ui.sized(170.0, |ui| {
                        if ui.text_input(&mut find.replace, "Substituir por").submitted() {
                            one = true;
                        }
                    });
                    ui.checkbox("Aa", &mut find.case);
                    prev |= ui.button("‹").clicked();
                    next |= ui.button("›").clicked();
                    one |= ui.button("Substituir").clicked();
                    all |= ui.button("Todos").clicked();
                    close |= ui.button("×").clicked();
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
        let body = (left, body_top, width, (status_y - body_top).max(60.0));
        let mut editor_rect = body;
        let mut term_rect = (left, status_y, width, 0.0);
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
            Some(s) if self.servers.get(&s.command).is_some_and(|c| c.ready) => format!(" · {}", s.command),
            Some(s) if self.failed.contains(&s.command) || !installed(&s.command) => format!(" · {} não disponível", s.command),
            Some(s) => format!(" · {} iniciando…", s.command),
            None => String::new(),
        };
        let errors = self.docs[i].diags.iter().filter(|d| d.severity == Severity::Error).count();
        let warns = self.docs[i].diags.iter().filter(|d| d.severity == Severity::Warning).count();
        let problems = if errors + warns > 0 { format!(" · {errors} erro(s), {warns} aviso(s)") } else { String::new() };
        let tabs = if self.config.soft_tabs { format!("Espaços: {}", self.config.tab_size) } else { "Tabulação".into() };
        let note = match &self.message {
            Some((m, at)) if at.elapsed() < Duration::from_secs(6) => format!("  —  {m}"),
            _ => String::new(),
        };
        ui.at(left, status_y, win_w - left, |ui| {
            ui.status_bar(&format!("Linha {line}, coluna {col} · {} · {tabs}{lsp_state}{problems}{note}", lang.name));
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
                close |= ui.button("Fechar").clicked();
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
                self.say(format!("Preferências não gravadas: {err}"));
            }
        }
    }

    /// Menus do app na barra superior (como os do TextMate).
    fn menu_bar(&mut self) -> Vec<MenuGroup> {
        let langs: Vec<MenuItem> = lang::LANGUAGES
            .iter()
            .map(|l| MenuItem::action(&format!("lang:{}", l.id), l.name).checked(self.docs.get(self.cur).is_some_and(|d| d.lang.id == l.id)))
            .collect();
        let tabs: Vec<MenuItem> = self.docs.iter().enumerate().map(|(i, d)| MenuItem::action(&format!("tab:{i}"), &d.title()).checked(i == self.cur)).collect();
        vec![
            MenuGroup {
                title: "Arquivo".into(),
                items: vec![
                    MenuItem::action("new", "Novo").shortcut("Ctrl+N"),
                    MenuItem::action("open", "Abrir…").shortcut("Ctrl+O"),
                    MenuItem::action("open-folder", "Abrir pasta…").shortcut("Ctrl+Shift+O"),
                    MenuItem::Separator,
                    MenuItem::action("save", "Salvar").shortcut("Ctrl+S"),
                    MenuItem::action("save-as", "Salvar como…").shortcut("Ctrl+Shift+S"),
                    MenuItem::action("save-all", "Salvar tudo").shortcut("Ctrl+Alt+S"),
                    MenuItem::Separator,
                    MenuItem::action("close-tab", "Fechar aba").shortcut("Ctrl+W"),
                    MenuItem::action("quit", "Sair").shortcut("Ctrl+Q"),
                ],
            },
            MenuGroup {
                title: "Editar".into(),
                items: vec![
                    MenuItem::action("find", "Buscar e substituir").shortcut("Ctrl+F"),
                    MenuItem::action("find-next", "Buscar próximo").shortcut("Ctrl+G"),
                    MenuItem::action("find-prev", "Buscar anterior").shortcut("Ctrl+Shift+G"),
                    MenuItem::action("find-project", "Buscar no projeto…").shortcut("Ctrl+Shift+F"),
                    MenuItem::Separator,
                    MenuItem::action("comment", "Comentar linhas").shortcut("Ctrl+/"),
                    MenuItem::action("duplicate", "Duplicar linha").shortcut("Ctrl+Shift+D"),
                    MenuItem::action("move-up", "Mover linha para cima").shortcut("Alt+↑"),
                    MenuItem::action("move-down", "Mover linha para baixo").shortcut("Alt+↓"),
                    MenuItem::Separator,
                    MenuItem::action("complete", "Completar").shortcut("Alt+Space"),
                    MenuItem::action("prefs", "Preferências…").shortcut("Ctrl+,"),
                    MenuItem::action("edit-servers", "Servidores de linguagem…"),
                ],
            },
            MenuGroup {
                title: "Visualizar".into(),
                items: vec![
                    MenuItem::action("drawer", "Gaveta de arquivos").checked(self.config.drawer),
                    MenuItem::action("line-numbers", "Números de linha").checked(self.config.line_numbers),
                    MenuItem::action("terminal", "Terminal").shortcut("Ctrl+J").checked(self.terminal_open),
                    MenuItem::Separator,
                    MenuItem::action("split-right", "Dividir à direita").shortcut("Ctrl+\\"),
                    MenuItem::action("split-down", "Dividir embaixo").shortcut("Ctrl+Shift+\\"),
                    MenuItem::action("unsplit", "Juntar os painéis"),
                    MenuItem::Separator,
                    MenuItem::action("zoom-in", "Aumentar a fonte").shortcut("Ctrl+="),
                    MenuItem::action("zoom-out", "Diminuir a fonte").shortcut("Ctrl+-"),
                    MenuItem::action("zoom-reset", "Tamanho normal").shortcut("Ctrl+0"),
                    MenuItem::Separator,
                    MenuItem::Submenu { label: "Linguagem".into(), items: langs },
                ],
            },
            MenuGroup {
                title: "Navegar".into(),
                items: vec![
                    MenuItem::action("goto-file", "Ir para arquivo…").shortcut("Ctrl+T"),
                    MenuItem::action("goto-line", "Ir para a linha…").shortcut("Ctrl+L"),
                    MenuItem::Separator,
                    MenuItem::action("definition", "Ir para a definição (ou Ctrl+clique)").shortcut("F12"),
                    MenuItem::action("hover", "Mostrar informações do símbolo").shortcut("Ctrl+I"),
                    MenuItem::action("next-problem", "Próximo problema").shortcut("F8"),
                    MenuItem::action("prev-problem", "Problema anterior").shortcut("Shift+F8"),
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
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "mide_code=info".into()))
        .init();
    let args: Vec<PathBuf> = std::env::args().skip(1).filter(|a| !a.starts_with('-')).map(PathBuf::from).collect();
    let window = Window::new("Code").app_id("mide-code").size(1040, 640).min_size(560, 360).full_size_content().accepts_files();
    run(window, Code::new(args))
}
