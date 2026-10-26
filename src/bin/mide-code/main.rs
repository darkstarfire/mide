//! Code (`mide-code`): o editor de texto e código do mide, inspirado no TextMate do Mac
//! (só o comportamento como referência; tudo escrito aqui, com o mide-ui).
//!
//! - **Projeto**: abrir uma pasta (`mide-code pasta/` ou Arquivo → Abrir pasta) mostra a
//!   gaveta à esquerda com a árvore dos arquivos, como a do TextMate.
//! - **Abas**: um arquivo por aba; Ctrl+W fecha (pergunta se há alterações).
//! - **Ir para arquivo** (Ctrl+T, como o ⌘T do TextMate): busca aproximada pelo nome em
//!   todo o projeto. **Ir para linha** (Ctrl+L).
//! - **Cores do código** para mais de 25 linguagens (`mide::code::highlight`), e a
//!   linguagem escolhível em Visualizar → Linguagem.
//! - **LSP** (servidores de linguagem configuráveis em Preferências): erros e avisos
//!   sublinhados com a mensagem no fim da linha, completar (Ctrl+Espaço), ir para a
//!   definição (F12) e a dica do símbolo (Ctrl+K Ctrl+I → Ctrl+I).
//! - **Editar**: buscar e substituir (Ctrl+F), comentar linhas (Ctrl+/), duplicar linha
//!   (Ctrl+Shift+D), recuo com espaços ou tabulação.
//! - **Preferências** (Ctrl+,): fonte, tamanho, números de linha, linha atual,
//!   tabulação, gaveta e servidores de linguagem. Tudo pela interface.
//! - Arquivos soltos na janela abrem em abas.

use std::{
    collections::{HashMap, HashSet},
    error::Error,
    ops::Range,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use mide::code::{
    config::{installed, Config, Server},
    highlight,
    lang::{self, Language},
    lsp::{self, Client, Completion, Diagnostic, Event, Severity},
    project,
};
use mide::kit::{
    run,
    textarea::{line_of, Decorations, TextDoc},
    window_menu, App, Choice, FileChooser, Key, MenuGroup, MenuItem, TabAction, TextAreaLook, Ui, Window,
};

/// Arquivos maiores que isso não abrem (o histórico guarda cópias do texto).
const MAX_BYTES: u64 = 16 * 1024 * 1024;
/// Largura da gaveta de arquivos.
const DRAWER_W: f32 = 230.0;

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
    Prefs,
}

enum Dialog {
    Open(FileChooser),
    OpenFolder(FileChooser),
    SaveAs(FileChooser, bool),
    /// Fechar a aba com alterações não salvas.
    ConfirmClose(usize),
    /// Sair com abas alteradas.
    ConfirmQuit,
}

#[derive(Default)]
struct Find {
    query: String,
    replace: String,
    focus: bool,
}

struct Code {
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
    /// Preferências: o servidor em edição (índice) e textos dos campos.
    pref_server: Option<usize>,
    pref_fields: (String, String, String),
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
            pref_server: None,
            pref_fields: Default::default(),
        };
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
        self.message = Some((text.into(), Instant::now()));
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
        let blank = self.docs.len() == 1 && self.docs[0].path.is_none() && !self.docs[0].text.modified && self.docs[0].text.text.is_empty();
        if blank {
            self.docs[0] = doc;
            self.cur = 0;
        } else {
            self.docs.push(doc);
            self.cur = self.docs.len() - 1;
        }
        self.start = true;
    }

    /// Salva a aba atual (sem nome: pergunta onde). Devolve se salvou.
    fn save(&mut self, then_close: bool) -> bool {
        let Some(path) = self.doc().path.clone() else {
            let dir = default_dir(self.root.as_deref());
            self.dialog = Some(Dialog::SaveAs(FileChooser::save("Salvar como", Some(&dir), "sem título.txt"), then_close));
            return false;
        };
        let doc = self.doc();
        let text = if doc.crlf { doc.text.text.replace('\n', "\r\n") } else { doc.text.text.clone() };
        match mide::files::write_atomic(&path, text.as_bytes()) {
            Ok(()) => {
                doc.text.modified = false;
                tracing::info!("salvo: {}", path.display());
                if let Some(client) = self.client_for_current() {
                    client.did_save(&path);
                }
                true
            }
            Err(err) => {
                self.say(format!("Não foi possível salvar: {err}"));
                false
            }
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
        if let (Some(path), Some(cmd)) = (&doc.path, self.config.server_for(doc.lang.id).map(|s| s.command.clone())) {
            if let Some(client) = self.servers.get_mut(&cmd) {
                client.did_close(path);
            }
        }
        if self.docs.is_empty() {
            self.docs.push(Doc::new(None, String::new(), false));
        }
        self.cur = self.cur.min(self.docs.len() - 1);
        self.completion = None;
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
                        doc.diags = items;
                    }
                }
                Event::Completions { id, items } => {
                    if let Some((want, list, sel, _)) = self.completion.as_mut() {
                        if *want == id {
                            *list = items;
                            *sel = 0;
                        }
                    }
                }
                Event::Definition { id, uri, line, col } => {
                    if self.definition == Some(id) {
                        self.definition = None;
                        let path = mide::kit::parse_uri_list(&uri).into_iter().next().unwrap_or_default();
                        if !path.as_os_str().is_empty() {
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
        if let (Some(path), Some(client)) = (path, self.client_for_current()) {
            if let Some(id) = client.hover(&path, &text, cursor) {
                self.hover = Some((id, String::new()));
            }
        }
    }

    /// Comenta ou descomenta as linhas da seleção (ou a do cursor).
    fn toggle_comment(&mut self) {
        let prefix = comment_prefix(self.doc().lang).to_string();
        let doc = &mut self.doc().text;
        let (a, b) = doc.edit.selection().unwrap_or((doc.edit.cursor, doc.edit.cursor));
        let starts = doc.line_starts();
        let (la, lb) = (line_of(&starts, a), line_of(&starts, b.saturating_sub(usize::from(b > a))));
        let lines: Vec<usize> = (la..=lb).collect();
        let all_commented = lines.iter().all(|&l| {
            let s = &doc.text[starts[l]..];
            let line = s.split('\n').next().unwrap_or("");
            line.trim().is_empty() || line.trim_start().starts_with(&prefix)
        });
        doc.before_change(mide::kit::textarea::Change::Other);
        // Do fim para o começo, para os índices das linhas de cima não mudarem.
        for &l in lines.iter().rev() {
            let start = starts[l];
            let line = doc.text[start..].split('\n').next().unwrap_or("").to_string();
            if line.trim().is_empty() {
                continue;
            }
            let indent = line.len() - line.trim_start().len();
            if all_commented {
                let rest = &line[indent + prefix.len()..];
                let drop = prefix.len() + usize::from(rest.starts_with(' '));
                doc.text.replace_range(start + indent..start + indent + drop, "");
            } else {
                doc.text.insert_str(start + indent, &format!("{prefix} "));
            }
        }
        let end = doc.text.len();
        doc.edit = mide::kit::textedit::Edit { cursor: doc.edit.cursor.min(end), anchor: None };
    }

    fn duplicate_line(&mut self) {
        let doc = &mut self.doc().text;
        let starts = doc.line_starts();
        let l = line_of(&starts, doc.edit.cursor);
        let start = starts[l];
        let end = doc.text[start..].find('\n').map_or(doc.text.len(), |i| start + i);
        let line = doc.text[start..end].to_string();
        doc.before_change(mide::kit::textarea::Change::Other);
        doc.text.insert_str(end, &format!("\n{line}"));
        doc.edit = mide::kit::textedit::Edit { cursor: doc.edit.cursor + line.len() + 1, anchor: None };
    }

    fn go_to_line(&mut self, line: usize) {
        let doc = &mut self.doc().text;
        let starts = doc.line_starts();
        let l = line.saturating_sub(1).min(starts.len() - 1);
        doc.edit = mide::kit::textedit::Edit { cursor: starts[l], anchor: None };
        self.start = true;
    }

    /// Próxima ocorrência da busca depois do cursor (volta ao começo no fim).
    fn find_next(&mut self) {
        let Some(query) = self.find.as_ref().map(|f| f.query.clone()).filter(|q| !q.is_empty()) else {
            return;
        };
        let doc = &mut self.doc().text;
        let from = doc.edit.selection().unwrap_or((doc.edit.cursor, doc.edit.cursor)).1;
        let at = doc.text[from..].find(&query).map(|i| from + i).or_else(|| doc.text.find(&query));
        match at {
            Some(i) => doc.edit = mide::kit::textedit::Edit { cursor: i + query.len(), anchor: Some(i) },
            None => self.say(format!("\"{query}\" não encontrado")),
        }
    }

    fn replace_all(&mut self) {
        let Some(find) = &self.find else {
            return;
        };
        if find.query.is_empty() {
            return;
        }
        let (q, r) = (find.query.clone(), find.replace.clone());
        let doc = &mut self.doc().text;
        let n = doc.text.matches(&q).count();
        if n > 0 {
            doc.before_change(mide::kit::textarea::Change::Other);
            doc.text = doc.text.replace(&q, &r);
            doc.edit = mide::kit::textedit::Edit { cursor: 0, anchor: None };
        }
        self.say(format!("{n} substituição(ões)"));
    }

    /// Cores do código e erros da aba atual, para a área de texto.
    fn decorations(&mut self, dark: bool) -> Decorations {
        let i = self.cur;
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
            "find-next" => self.find_next(),
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
            "complete" => self.request_completion(),
            "definition" => self.go_to_definition(),
            "hover" => self.show_hover(),
            "prefs" => self.overlay = if matches!(self.overlay, Overlay::Prefs) { Overlay::None } else { Overlay::Prefs },
            "drawer" => {
                self.config.drawer = !self.config.drawer;
                let _ = self.config.save();
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
                } else if let Some(i) = other.strip_prefix("tab:").and_then(|n| n.parse::<usize>().ok()) {
                    if i < self.docs.len() {
                        self.cur = i;
                    }
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
        let mut run: Vec<&str> = Vec::new();
        if input.shortcut_shift("s") == Some(false) {
            run.push("save");
        } else if shift("s") {
            run.push("save-as");
        }
        for (key, action) in [("n", "new"), ("o", "open"), ("w", "close-tab"), ("q", "quit"), ("f", "find"), ("g", "find-next"), ("t", "goto-file"), ("l", "goto-line"), ("/", "comment"), (",", "prefs"), ("i", "hover"), ("=", "zoom-in"), ("+", "zoom-in"), ("-", "zoom-out"), ("0", "zoom-reset")] {
            if input.shortcut(key) && !(key == "o" && shift("o")) && !(key == "d") {
                run.push(action);
            }
        }
        if shift("o") {
            run.retain(|a| *a != "open");
            run.push("open-folder");
        }
        if shift("d") {
            run.push("duplicate");
        }
        if input.presses.iter().any(|p| p.ctrl && p.keysym == 0x20) {
            run.push("complete");
        }
        if input.presses.iter().any(|p| p.keysym == 0xffc9) {
            run.push("definition");
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

    /// Gaveta com a árvore do projeto.
    fn drawer(&mut self, ui: &mut Ui, height: f32) {
        let Some(root) = self.root.clone() else {
            return;
        };
        let mut rows: Vec<(usize, PathBuf, bool)> = Vec::new();
        fn walk(dir: &Path, depth: usize, expanded: &HashSet<PathBuf>, rows: &mut Vec<(usize, PathBuf, bool)>) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            let mut list: Vec<(String, PathBuf, bool)> = entries
                .flatten()
                .filter_map(|e| {
                    let name = e.file_name().to_string_lossy().into_owned();
                    (!name.starts_with('.') && name != "target" && name != "node_modules").then(|| (name, e.path(), e.file_type().is_ok_and(|t| t.is_dir())))
                })
                .collect();
            list.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase())));
            for (_, path, is_dir) in list {
                let open = is_dir && expanded.contains(&path);
                rows.push((depth, path.clone(), is_dir));
                if open && rows.len() < 5000 {
                    walk(&path, depth + 1, expanded, rows);
                }
            }
        }
        walk(&root, 0, &self.expanded, &mut rows);
        let current = self.docs.get(self.cur).and_then(|d| d.path.clone());
        let mut clicked: Option<(PathBuf, bool, bool)> = None;
        let root_name = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        ui.scroll_area(height, |ui| {
            ui.note(&root_name.to_uppercase());
            for (depth, path, is_dir) in &rows {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let icon = if *is_dir {
                    "folder".to_string()
                } else {
                    mide::places::icon_name_for(&mide::places::Item { path: path.clone(), name: name.clone(), is_dir: false, modified: std::time::SystemTime::UNIX_EPOCH }).to_string()
                };
                let selected = current.as_ref() == Some(path);
                if selected {
                    ui.join_selection(false, false);
                }
                let (r, toggled) = ui.tree_row(*depth, *is_dir, self.expanded.contains(path), &icon, &name, &[], selected, false);
                if toggled || r.clicked() {
                    clicked = Some((path.clone(), *is_dir, toggled));
                }
            }
        });
        if let Some((path, is_dir, _)) = clicked {
            if is_dir {
                if !self.expanded.remove(&path) {
                    self.expanded.insert(path);
                }
            } else {
                self.open_path(path);
            }
            ui.request_repaint();
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
                    if ui.text_input(text, "Número da linha").submitted() {
                        go = text.trim().parse::<usize>().ok();
                    }
                });
                if let Some(line) = go {
                    self.overlay = Overlay::None;
                    self.go_to_line(line);
                } else if esc {
                    self.overlay = Overlay::None;
                }
            }
            Overlay::Prefs => self.prefs(ui, esc),
        }
    }

    /// Preferências: aparência do editor e servidores de linguagem.
    fn prefs(&mut self, ui: &mut Ui, esc: bool) {
        let mut c = self.config.clone();
        let mut close = esc;
        let mut edit_server: Option<Option<usize>> = None;
        let mut remove: Option<usize> = None;
        let mut save_server = false;
        let fonts: Vec<String> = std::iter::once("Fonte do sistema".to_string()).chain(ui.mono_fonts()).collect();
        let mut font_i = c.font.as_ref().and_then(|f| fonts.iter().position(|x| x == f)).unwrap_or(0);
        let editing = self.pref_server;
        let fields = &mut self.pref_fields;
        let servers_running: Vec<String> = self.servers.keys().cloned().collect();
        ui.modal(560.0, |ui| {
            ui.strong("Preferências do Code");
            ui.card(|ui| {
                if ui.dropdown("Fonte", &fonts, &mut font_i).changed() {
                    c.font = (font_i > 0).then(|| fonts[font_i].clone());
                }
                let mut size = c.size;
                ui.slider("Tamanho da fonte", &mut size, 9.0, 28.0);
                c.size = size.round();
                ui.switch_detail("Números de linha", "Na margem esquerda", &mut c.line_numbers);
                ui.switch_detail("Destacar a linha atual", "A linha do cursor com um fundo leve", &mut c.current_line);
                ui.switch_detail("Recuo com espaços", "Desligado, a tecla Tab insere uma tabulação", &mut c.soft_tabs);
                let mut tab = c.tab_size as f32;
                ui.slider("Espaços por recuo", &mut tab, 1.0, 8.0);
                c.tab_size = tab.round() as usize;
                ui.switch_detail("Gaveta de arquivos", "A árvore do projeto à esquerda", &mut c.drawer);
            });
            ui.strong("Servidores de linguagem (LSP)");
            ui.card(|ui| {
                for (i, s) in c.servers.iter_mut().enumerate() {
                    let state = if !installed(&s.command) {
                        "não instalado"
                    } else if servers_running.contains(&s.command) {
                        "rodando"
                    } else {
                        "instalado"
                    };
                    let label = format!("{} — {}", s.languages.join(", "), s.command);
                    ui.switch_detail(&label, &format!("{} {} · {state}", s.command, s.args.join(" ")), &mut s.enabled);
                    ui.row(|ui| {
                        if ui.button("Editar").clicked() {
                            edit_server = Some(Some(i));
                        }
                        if ui.button("Remover").clicked() {
                            remove = Some(i);
                        }
                    });
                }
                if ui.button("Adicionar servidor…").clicked() {
                    edit_server = Some(None);
                }
            });
            if let Some(i) = editing {
                ui.strong(if i == usize::MAX { "Novo servidor" } else { "Editar servidor" });
                ui.card(|ui| {
                    ui.text_input(&mut fields.0, "Linguagens (ex.: rust, python, go)");
                    ui.text_input(&mut fields.1, "Comando (ex.: rust-analyzer)");
                    ui.text_input(&mut fields.2, "Argumentos (separados por espaço)");
                    if ui.primary_button("Guardar servidor").clicked() {
                        save_server = true;
                    }
                });
            }
            ui.row(|ui| {
                ui.push_right(110.0);
                ui.sized(110.0, |ui| close |= ui.primary_button("Pronto").clicked());
            });
        });
        if let Some(which) = edit_server {
            match which.and_then(|i| c.servers.get(i)) {
                Some(s) => {
                    *fields = (s.languages.join(", "), s.command.clone(), s.args.join(" "));
                    self.pref_server = which;
                }
                None => {
                    *fields = Default::default();
                    self.pref_server = Some(usize::MAX);
                }
            }
        }
        if save_server {
            let server = Server {
                languages: fields.0.split([',', ' ']).filter(|s| !s.is_empty()).map(str::to_string).collect(),
                command: fields.1.trim().to_string(),
                args: fields.2.split_whitespace().map(str::to_string).collect(),
                enabled: true,
            };
            if !server.command.is_empty() && !server.languages.is_empty() {
                match self.pref_server {
                    Some(i) if i < c.servers.len() => c.servers[i] = server,
                    _ => c.servers.push(server),
                }
                self.failed.clear();
            }
            self.pref_server = None;
        }
        if let Some(i) = remove {
            if i < c.servers.len() {
                c.servers.remove(i);
            }
        }
        if c != self.config {
            // Servidor desligado ou trocado: para o que estava rodando.
            let wanted: HashSet<String> = c.servers.iter().filter(|s| s.enabled).map(|s| s.command.clone()).collect();
            self.servers.retain(|cmd, _| wanted.contains(cmd));
            for doc in &mut self.docs {
                doc.lsp_seen = None;
            }
            self.config = c;
            if let Err(err) = self.config.save() {
                self.say(format!("Preferências não gravadas: {err}"));
            }
        }
        if close {
            self.overlay = Overlay::None;
            self.pref_server = None;
        }
    }

    fn dialogs(&mut self, ui: &mut Ui) {
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
                    self.root = Some(path);
                    self.expanded.clear();
                    self.config.drawer = true;
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
        }
    }
}

impl App for Code {
    fn view(&mut self, ui: &mut Ui) {
        let doc_title = self.docs.get(self.cur).map(Doc::title).unwrap_or_default();
        ui.set_title(&format!("{doc_title} — Code"));
        ui.set_mono_font(self.config.font.as_deref());
        // Arquivos soltos na janela abrem em abas (pasta: vira o projeto).
        if let Some(paths) = ui.input().dropped.clone() {
            for p in paths {
                if p.is_dir() {
                    self.root = Some(p);
                    self.config.drawer = true;
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
        // Barra de ferramentas: título e botões (gaveta, abrir, salvar, buscar, preferências).
        let mut clicked = None;
        let title = self.docs.get(self.cur).map(Doc::title).unwrap_or_default();
        let drawer_on = self.config.drawer && self.root.is_some();
        let finding = self.find.is_some();
        ui.toolbar(|ui| {
            if ui.toolbar_buttons(&[("view-sidebar-start-symbolic", "▤", drawer_on, true)]).is_some() {
                clicked = Some("drawer");
            }
            ui.toolbar_title(&title, 360.0);
            ui.push_right(3.0 * 40.0 + 2.0 * 44.0 + 14.0);
            match ui.toolbar_buttons(&[("document-open-symbolic", "↥", false, true), ("folder-open-symbolic", "▱", false, true), ("document-save-symbolic", "↓", false, true)]) {
                Some(0) => clicked = Some("open"),
                Some(1) => clicked = Some("open-folder"),
                Some(2) => clicked = Some("save"),
                _ => {}
            }
            if ui.toolbar_buttons(&[("edit-find-symbolic", "⌕", finding, true)]).is_some() {
                clicked = Some("find");
            }
            if ui.toolbar_buttons(&[("preferences-system-symbolic", "⚙", false, true)]).is_some() {
                clicked = Some("prefs");
            }
        });
        if let Some(action) = clicked {
            self.command(action, ui);
        }
        let (win_w, win_h) = ui.window_size();
        let top = mide::kit::TOOLBAR_HEIGHT;
        let status_h = 26.0;
        let body_h = (win_h - top - status_h).max(60.0);
        let left = if drawer_on { DRAWER_W } else { 0.0 };
        if drawer_on {
            ui.at(0.0, top, DRAWER_W, |ui| self.drawer(ui, body_h));
        }
        let width = win_w - left;
        ui.at(left, top, width, |ui| {
            // Abas (com mais de um arquivo).
            if self.docs.len() > 1 {
                let titles: Vec<String> = self.docs.iter().map(Doc::title).collect();
                match ui.tab_bar(&titles, self.cur) {
                    TabAction::Select(i) => self.cur = i,
                    TabAction::Close(i) => self.close_tab(i, false),
                    TabAction::New => self.command("new", ui),
                    TabAction::None => {}
                }
            }
            // Buscar e substituir.
            if let Some(find) = self.find.as_mut() {
                let mut next = false;
                let mut all = false;
                let mut close = false;
                ui.row(|ui| {
                    if std::mem::take(&mut find.focus) {
                        ui.focus_next_input();
                    }
                    ui.sized(220.0, |ui| next |= ui.text_input(&mut find.query, "Buscar").submitted());
                    ui.sized(220.0, |ui| {
                        ui.text_input(&mut find.replace, "Substituir por");
                    });
                    next |= ui.button("Próximo").clicked();
                    all |= ui.button("Substituir todos").clicked();
                    close |= ui.button("Fechar").clicked();
                });
                if next {
                    self.find_next();
                }
                if all {
                    self.replace_all();
                }
                if close {
                    self.find = None;
                }
            }
            // Popup de completar: ↑/↓ escolhem, Enter/Tab entram, Esc fecha (tiradas da
            // área de texto antes de ela ver as teclas).
            let mut accept = None;
            if let Some((_, items, sel, _)) = self.completion.as_mut() {
                if !items.is_empty() {
                    if ui.take_key(Key::Down) {
                        *sel = (*sel + 1).min(items.len() - 1);
                    }
                    if ui.take_key(Key::Up) {
                        *sel = sel.saturating_sub(1);
                    }
                    if ui.take_key(Key::Enter) || ui.take_key(Key::Tab) {
                        accept = items.get(*sel).cloned();
                    }
                }
                if ui.take_key(Key::Escape) {
                    self.completion = None;
                }
            }
            if let (Some(item), Some((_, _, _, start))) = (accept, self.completion.take()) {
                let doc = &mut self.doc().text;
                let cursor = doc.edit.cursor;
                doc.before_change(mide::kit::textarea::Change::Other);
                doc.text.replace_range(start.min(cursor)..cursor, &item.insert);
                let at = start.min(cursor) + item.insert.len();
                doc.edit = mide::kit::textedit::Edit { cursor: at, anchor: None };
            }
            let deco = self.decorations(dark);
            let start = std::mem::take(&mut self.start);
            if start {
                ui.focus_next_input();
            }
            let editor_h = ui.remaining_height().max(60.0);
            let indent = self.config.indent();
            let (line_numbers, size, current_line) = (self.config.line_numbers, self.config.size, self.config.current_line);
            let mut deco = Some(deco);
            let mut decorate = move |_: &TextDoc| deco.take().unwrap_or_default();
            let i = self.cur.min(self.docs.len() - 1);
            let before = self.docs[i].text.version;
            let look = TextAreaLook { line_numbers, size: Some(size), current_line, indent: &indent, decorate: Some(&mut decorate), ..Default::default() };
            let r = ui.text_area_with(&mut self.docs[i].text, editor_h, look);
            if self.docs[i].text.version != before {
                self.last_edit = Some(Instant::now());
                // Digitando: a lista de completar acompanha a palavra (e some fora dela).
                if let Some((_, items, _, start)) = self.completion.as_mut() {
                    let doc = &self.docs[i].text;
                    let cursor = doc.edit.cursor;
                    if cursor < *start || doc.text[*start..cursor].contains(|c: char| !c.is_alphanumeric() && c != '_') {
                        self.completion = None;
                    } else {
                        let word = doc.text[*start..cursor].to_lowercase();
                        items.retain(|it| it.label.to_lowercase().starts_with(&word) || word.is_empty());
                    }
                }
            }
            // Menu de contexto do editor.
            ui.context_menu(&r.response, |m| {
                let mut chosen = None;
                for (action, label) in [("definition", "Ir para a definição"), ("hover", "Mostrar informações"), ("complete", "Completar")] {
                    if m.item(label) {
                        chosen = Some(action);
                    }
                }
                m.separator();
                for (action, label) in [("comment", "Comentar linhas"), ("duplicate", "Duplicar linha")] {
                    if m.item(label) {
                        chosen = Some(action);
                    }
                }
                if let Some(a) = chosen {
                    self.pending_menu(a);
                }
            });
            // Lista de completar perto do cursor (abaixo da linha).
            if let Some((_, items, sel, _)) = &self.completion {
                if !items.is_empty() {
                    let doc = &self.docs[i].text;
                    let starts = doc.line_starts();
                    let line = line_of(&starts, doc.edit.cursor);
                    let col = doc.text[starts[line]..doc.edit.cursor].chars().count() as f32;
                    let line_h = (size * 1.5).round();
                    let gutter = if line_numbers { size * 0.9 * 0.6 * starts.len().to_string().len().max(2) as f32 + 20.0 } else { 0.0 };
                    let x = (r.response.rect.0 + gutter + 8.0 + col * size * 0.6 - doc.scroll_x).min(width - 320.0).max(8.0);
                    let y = r.response.rect.1 + 6.0 + (line + 1) as f32 * line_h - doc.scroll;
                    let shown: Vec<(String, String)> = items.iter().take(10).map(|c| (c.label.clone(), c.detail.clone())).collect();
                    let sel = *sel;
                    ui.glass_panel(x, y.min(r.response.rect.1 + r.response.rect.3 - 200.0), 320.0, 10.0, |ui| {
                        for (k, (label, detail)) in shown.iter().enumerate() {
                            if k == sel {
                                ui.join_selection(false, false);
                            }
                            ui.list_row("code-function-symbolic", label, &[(detail, 120.0)], k == sel);
                        }
                    });
                }
            }
        });
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
        ui.at(left, win_h - status_h, win_w - left, |ui| {
            ui.status_bar(&format!("Linha {line}, coluna {col} · {} · {tabs}{lsp_state}{problems}{note}", lang.name));
        });
        // Dica do símbolo (Ctrl+I).
        if let Some((_, text)) = &self.hover {
            if !text.is_empty() {
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
        }
        self.overlays(ui);
        self.dialogs(ui);
        // O servidor de linguagem acompanha o texto.
        self.sync_lsp();
        if self.last_edit.is_some_and(|t| t.elapsed() < Duration::from_millis(400)) {
            ui.wake_after(Duration::from_millis(300));
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
                    MenuItem::Separator,
                    MenuItem::action("comment", "Comentar linhas").shortcut("Ctrl+/"),
                    MenuItem::action("duplicate", "Duplicar linha").shortcut("Ctrl+Shift+D"),
                    MenuItem::Separator,
                    MenuItem::action("complete", "Completar").shortcut("Ctrl+Space"),
                    MenuItem::action("prefs", "Preferências…").shortcut("Ctrl+,"),
                ],
            },
            MenuGroup {
                title: "Visualizar".into(),
                items: vec![
                    MenuItem::action("drawer", "Gaveta de arquivos").checked(self.config.drawer),
                    MenuItem::action("line-numbers", "Números de linha").checked(self.config.line_numbers),
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
                    MenuItem::action("definition", "Ir para a definição").shortcut("F12"),
                    MenuItem::action("hover", "Mostrar informações do símbolo").shortcut("Ctrl+I"),
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

/// Pasta sugerida para um arquivo novo: a do projeto, Documentos, ou a pasta pessoal.
fn default_dir(root: Option<&Path>) -> PathBuf {
    if let Some(root) = root {
        return root.to_path_buf();
    }
    let home = mide::places::home_dir();
    ["Documentos", "Documents"].iter().map(|d| home.join(d)).find(|d| d.is_dir()).unwrap_or(home)
}

impl Code {
    /// Ação do menu de contexto (feita fora do desenho do menu).
    fn pending_menu(&mut self, action: &str) {
        match action {
            "definition" => self.go_to_definition(),
            "hover" => self.show_hover(),
            "complete" => self.request_completion(),
            "comment" => self.toggle_comment(),
            "duplicate" => self.duplicate_line(),
            _ => {}
        }
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "mide_code=info".into()))
        .init();
    let args: Vec<PathBuf> = std::env::args().skip(1).filter(|a| !a.starts_with('-')).map(PathBuf::from).collect();
    let window = Window::new("Code").app_id("mide-code").size(1040, 640).min_size(560, 360).full_size_content().accepts_files();
    run(window, Code::new(args))
}
