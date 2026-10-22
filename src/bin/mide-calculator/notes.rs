//! Modo Notas matemáticas da Calculadora (Ctrl+4): texto livre em que a linha que
//! termina em "=" mostra o resultado depois dele ("aluguel × 12 =" → 14.400), com
//! variáveis ("aluguel = 1.200") e prosa antes da conta. As contas ficam em
//! `mide::calculator::notes` (sem tela); aqui ficam as notas abertas (`Session`), a
//! gravação, a lista, os menus e o desenho.

use std::{
    collections::{HashMap, HashSet},
    io,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime},
};

use mide::{
    calculator::{
        format::Style,
        notes::{
            self,
            store::{self, NoteFile},
            Context, LineEval, Shown,
        },
    },
    kit::{
        textarea::{line_of, line_starts, Change},
        textedit::Edit,
        Choice, Decorations, FileChooser, MenuGroup, MenuItem, Rgba, TextAreaLook, TextDoc, Theme, Ui, Waker,
    },
    settings_file::{self, Value},
    ui::text::{FontWeight, Text},
};

use super::{Calculator, Mode};

/// Espera depois da última tecla para gravar a nota.
const SAVE_AFTER: Duration = Duration::from_secs(1);

/// Tamanho da letra das notas.
const FONT: f32 = 15.0;

/// Ícone de cada nota na lista.
const NOTE_ICON: &str = "text-x-generic-symbolic";

/// Altura da linha de estado, embaixo da nota.
const STATUS_H: f32 = 34.0;

fn mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Uma gravação automática em andamento, em outra thread (o `fsync` não segura a
/// janela): a nota, a versão e o texto gravados, e a data do arquivo quando terminar.
struct Pending {
    id: String,
    version: u64,
    text: String,
    done: std::sync::mpsc::Receiver<io::Result<Option<SystemTime>>>,
}

/// As notas abertas: a lista (a editada por último primeiro, com a nota nova que ainda
/// não virou arquivo), a aberta e o texto de cada uma enquanto o app está aberto (cada
/// uma com o seu desfazer).
pub struct Session {
    dir: PathBuf,
    pub list: Vec<NoteFile>,
    current: String,
    docs: HashMap<String, TextDoc>,
    /// Versão do `TextDoc` gravada (ou lida) e a data do arquivo naquela hora.
    saved: HashMap<String, (u64, Option<SystemTime>)>,
    pending: Option<Pending>,
    /// Notas que não são texto UTF-8 (outra codificação): abertas só para ler, nunca
    /// gravadas (gravar trocaria o conteúdo original pelo que dá para mostrar).
    unreadable: HashSet<String>,
}

impl Session {
    /// Abre a pasta: a nota `last` (a última aberta), senão a mais recente, senão uma
    /// nova.
    pub fn open(dir: PathBuf, last: Option<&str>, now: chrono::DateTime<chrono::Local>) -> Self {
        let list = store::list(&dir);
        let mut session = Self { dir, list, current: String::new(), docs: HashMap::new(), saved: HashMap::new(), pending: None, unreadable: HashSet::new() };
        let pick = last
            .filter(|id| session.list.iter().any(|n| n.id == *id))
            .map(str::to_string)
            .or_else(|| session.list.first().map(|n| n.id.clone()));
        match pick {
            Some(id) => {
                session.load(&id);
                session.current = id;
            }
            None => session.add_new(now),
        }
        session
    }

    fn path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.txt"))
    }

    /// A nota aberta.
    pub fn current(&self) -> &str {
        &self.current
    }

    /// Posição da nota aberta na lista.
    pub fn current_index(&self) -> usize {
        self.list.iter().position(|n| n.id == self.current).unwrap_or(0)
    }

    /// Título da nota aberta.
    pub fn title(&self) -> String {
        self.list.get(self.current_index()).map_or_else(|| store::title(""), |n| n.title.clone())
    }

    /// O texto da nota aberta (com o cursor e o desfazer dela).
    pub fn doc(&mut self) -> &mut TextDoc {
        if !self.docs.contains_key(&self.current) {
            let id = self.current.clone();
            self.load(&id);
        }
        self.docs.get_mut(&self.current).expect("nota carregada")
    }

    /// Lê a nota do disco (sem arquivo: vazia), com o cursor no fim. Um arquivo que não
    /// é UTF-8 abre como dá para mostrar e fica só para ler.
    fn load(&mut self, id: &str) {
        let path = self.path(id);
        let text = match std::fs::read(&path) {
            Ok(bytes) => match String::from_utf8(bytes) {
                Ok(text) => {
                    self.unreadable.remove(id);
                    text
                }
                Err(err) => {
                    self.unreadable.insert(id.to_string());
                    String::from_utf8_lossy(err.as_bytes()).into_owned()
                }
            },
            Err(_) => String::new(),
        }
        .replace("\r\n", "\n");
        let mut doc = TextDoc::new(text);
        doc.edit = Edit::at_end(&doc.text);
        self.saved.insert(id.to_string(), (doc.version, mtime(&path)));
        self.docs.insert(id.to_string(), doc);
    }

    /// A nota aberta mudou desde a última gravação.
    pub fn dirty(&self) -> bool {
        self.docs.get(&self.current).is_some_and(|d| self.saved.get(&self.current).is_none_or(|(v, _)| *v != d.version))
    }

    /// Nota nova ainda sem arquivo e sem texto.
    fn is_blank_new(&self, id: &str) -> bool {
        !self.path(id).exists() && self.docs.get(id).is_none_or(|d| d.text.trim().is_empty())
    }

    /// A nota aberta só pode ser lida (não é UTF-8): as mudanças não são gravadas.
    pub fn read_only(&self) -> bool {
        self.unreadable.contains(&self.current)
    }

    /// A versão e o texto da nota aberta para gravar, se ela mudou. Uma nota nova vazia
    /// não vira arquivo e uma nota só para ler não é gravada: ficam marcadas como
    /// gravadas e não há o que gravar. Se o arquivo mudou fora desde a última leitura
    /// (outra Calculadora aberta na mesma nota), o texto daqui vira uma nota nova, para
    /// nenhum dos dois se perder.
    fn next_save(&mut self) -> Option<(String, u64, String)> {
        if !self.dirty() {
            return None;
        }
        let mut id = self.current.clone();
        let (version, text) = {
            let doc = &self.docs[&id];
            (doc.version, doc.text.clone())
        };
        if self.unreadable.contains(&id) || (text.trim().is_empty() && !self.path(&id).exists()) {
            self.saved.insert(id, (version, None));
            return None;
        }
        let known = self.saved.get(&id).and_then(|(_, m)| *m);
        if known.is_some() && mtime(&self.path(&id)) != known {
            let doc = self.docs.remove(&id).expect("nota aberta");
            self.saved.remove(&id);
            self.add_new(chrono::Local::now());
            let copy = self.current.clone();
            tracing::warn!("calculadora: a nota {id} mudou fora; o texto daqui fica na nota {copy}");
            self.docs.insert(copy.clone(), doc);
            self.saved.insert(copy.clone(), (u64::MAX, None));
            id = copy;
        }
        Some((id, version, text))
    }

    /// A gravação terminou: a versão gravada, a data do arquivo e a nota no topo da
    /// lista, com o título do texto gravado.
    fn saved_now(&mut self, id: &str, version: u64, text: &str, modified: Option<SystemTime>) {
        tracing::debug!("calculadora: notas: salva {id}");
        self.saved.insert(id.to_string(), (version, modified));
        if let Some(i) = self.list.iter().position(|n| n.id == id) {
            let mut note = self.list.remove(i);
            note.title = store::title(text);
            note.modified = modified.unwrap_or_else(SystemTime::now);
            note.path = self.path(id);
            self.list.insert(0, note);
        }
    }

    fn finish(&mut self, pending: Pending, result: io::Result<Option<SystemTime>>) -> (String, io::Result<()>) {
        match result {
            Ok(modified) => {
                self.saved_now(&pending.id, pending.version, &pending.text, modified);
                (pending.id, Ok(()))
            }
            Err(err) => (pending.id, Err(err)),
        }
    }

    /// Espera a gravação automática em andamento (antes de outra gravação ou da
    /// Lixeira: nada é gravado fora de ordem nem depois de a nota sair).
    fn wait_pending(&mut self) -> Option<(String, io::Result<()>)> {
        let pending = self.pending.take()?;
        let result = pending.done.recv().unwrap_or_else(|_| Err(io::Error::other("a gravação parou")));
        Some(self.finish(pending, result))
    }

    /// A gravação automática terminou? Devolve a nota e o resultado uma vez, quando
    /// acaba.
    pub fn poll(&mut self) -> Option<(String, io::Result<()>)> {
        let received = self.pending.as_ref()?.done.try_recv();
        let result = match received {
            Ok(result) => result,
            Err(std::sync::mpsc::TryRecvError::Empty) => return None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Err(io::Error::other("a gravação parou")),
        };
        let pending = self.pending.take()?;
        Some(self.finish(pending, result))
    }

    /// Há uma gravação automática em andamento.
    pub fn saving(&self) -> bool {
        self.pending.is_some()
    }

    /// Grava a nota aberta agora, se mudou (esperando a gravação automática em
    /// andamento); `true` se gravou. Uma nota nova vazia não vira arquivo. A nota
    /// gravada sobe na lista, com o título novo.
    pub fn save(&mut self) -> io::Result<bool> {
        let waited = self.wait_pending();
        let Some((id, version, text)) = self.next_save() else {
            return match waited {
                Some((_, Err(err))) => Err(err),
                _ => Ok(false),
            };
        };
        let path = store::save(&self.dir, &id, &text)?;
        self.saved_now(&id, version, &text, mtime(&path));
        Ok(true)
    }

    /// Grava todas as notas com alterações (ao fechar): a aberta e as que não puderam
    /// ser gravadas ao trocar de nota. Devolve o primeiro erro.
    pub fn save_all(&mut self) -> io::Result<()> {
        let current = self.current.clone();
        let mut ids: Vec<String> = self.docs.keys().cloned().collect();
        ids.sort();
        let mut first = Ok(());
        for id in ids {
            self.current = id;
            if let Err(err) = self.save()
                && first.is_ok()
            {
                first = Err(err);
            }
        }
        // A cópia de uma nota mudada fora pode ter virado a aberta.
        if self.list.iter().any(|n| n.id == current) && self.docs.contains_key(&current) {
            self.current = current;
        }
        first
    }

    /// Grava a nota aberta numa thread (a gravação automática: o `fsync` não segura a
    /// janela); `waker` acorda a janela quando termina, e o `poll` aplica o resultado.
    /// `true` se começou.
    pub fn save_in_background(&mut self, waker: Option<Waker>) -> bool {
        if self.pending.is_some() {
            return false;
        }
        let Some((id, version, text)) = self.next_save() else {
            return false;
        };
        let (tx, done) = std::sync::mpsc::channel();
        let (dir, file_id, file_text) = (self.dir.clone(), id.clone(), text.clone());
        std::thread::spawn(move || {
            let result = store::save(&dir, &file_id, &file_text).map(|path| mtime(&path));
            let _ = tx.send(result);
            if let Some(waker) = waker {
                waker.wake();
            }
        });
        self.pending = Some(Pending { id, version, text, done });
        true
    }

    /// Põe uma nota nova (vazia, sem arquivo) no topo da lista e abre ela.
    fn add_new(&mut self, now: chrono::DateTime<chrono::Local>) {
        let base = store::new_id(&self.dir, now);
        let mut id = base.clone();
        let mut n = 2;
        while self.list.iter().any(|x| x.id == id) {
            id = format!("{base}-{n}");
            n += 1;
        }
        let path = self.path(&id);
        self.list.insert(0, NoteFile { id: id.clone(), path, title: store::title(""), modified: SystemTime::now() });
        let doc = TextDoc::new(String::new());
        self.saved.insert(id.clone(), (doc.version, None));
        self.docs.insert(id.clone(), doc);
        self.current = id;
    }

    /// Nova nota: grava a aberta e abre uma nova (se a aberta já é uma nova vazia,
    /// fica nela; se há outra nova vazia na lista, abre aquela).
    pub fn new_note(&mut self, now: chrono::DateTime<chrono::Local>) -> io::Result<()> {
        let saved = self.save();
        let current = self.current.clone();
        if !self.is_blank_new(&current) {
            match self.list.iter().map(|n| n.id.clone()).find(|id| self.is_blank_new(id)) {
                Some(id) => self.current = id,
                None => self.add_new(now),
            }
        }
        saved.map(|_| ())
    }

    /// Abre outra nota (grava a aberta antes). Uma nota sem alterações cujo arquivo
    /// mudou no disco (outro programa) é lida de novo.
    pub fn select(&mut self, id: &str) -> io::Result<()> {
        if id == self.current || !self.list.iter().any(|n| n.id == id) {
            return Ok(());
        }
        let saved = self.save();
        let stale = self.docs.get(id).is_some_and(|d| {
            let (version, modified) = self.saved.get(id).copied().unwrap_or((0, None));
            d.version == version && mtime(&self.path(id)) != modified
        });
        if !self.docs.contains_key(id) || stale {
            self.load(id);
        }
        self.current = id.to_string();
        saved.map(|_| ())
    }

    /// Manda a nota aberta para a Lixeira (a que ainda não virou arquivo só some) e
    /// abre a de baixo na lista (ou a de cima, se era a última), ou uma nova. Devolve o
    /// nome da que saiu.
    pub fn trash(&mut self, trash: &Path, now: chrono::DateTime<chrono::Local>) -> io::Result<String> {
        // A posição que a pessoa vê (gravar põe a nota no topo), e o que foi digitado por
        // último vai junto para a Lixeira.
        let at = self.current_index();
        self.save()?;
        let id = self.current.clone();
        let path = self.path(&id);
        if path.exists() {
            store::trash(&path, trash)?;
        }
        self.list.retain(|n| n.id != id);
        self.docs.remove(&id);
        self.saved.remove(&id);
        match self.list.get(at).or_else(|| self.list.last()).map(|n| n.id.clone()) {
            Some(next) => {
                if !self.docs.contains_key(&next) {
                    self.load(&next);
                }
                self.current = next;
            }
            None => self.add_new(now),
        }
        Ok(id)
    }
}

/// O que as contas da nota dependem: a nota, a versão do texto, Rad/Grau e o formato
/// dos números.
#[derive(Clone, PartialEq)]
struct CacheKey {
    id: String,
    /// A data das cotações: quando chegam outras, as contas com moeda mudam.
    rates: Option<chrono::NaiveDate>,
    version: u64,
    radians: bool,
    style: Style,
}

/// As opções da Calculadora que valem nas notas, com as cotações em uso.
pub struct NotesCtx {
    radians: bool,
    style: Style,
    rates: Option<std::rc::Rc<mide::calculator::currency::Rates>>,
}

impl NotesCtx {
    fn get(&self) -> Context<'_> {
        Context { radians: self.radians, style: self.style, rates: self.rates.as_deref() }
    }
}

/// Estado do modo Notas no app.
#[derive(Default)]
pub struct Notes {
    /// Aberta na primeira vez que o modo aparece.
    session: Option<Session>,
    /// As contas da nota aberta, refeitas só quando o texto ou as opções mudam.
    cache: Option<(CacheKey, Vec<LineEval>)>,
    /// O que já foi para o log, por linha (os testes acompanham por ele).
    logged: Vec<String>,
    /// Lista de notas à vista (Ctrl+T).
    pub list_shown: bool,
    /// Exportar com os resultados: o seletor de arquivos aberto.
    export: Option<FileChooser>,
    /// Última versão vista e quando ela mudou (para gravar 1 s depois da última tecla).
    seen: Option<(String, u64, Instant)>,
    /// Linha do clique direito (o menu de contexto guarda a de quando abriu).
    menu_line: Option<usize>,
    /// Modo de teclado para onde "Levar para a calculadora" volta.
    pub keypad: Option<Mode>,
    /// A nota deve receber o foco no próximo quadro (a lista mudou de tamanho ou
    /// apareceu: o kit guarda o foco pela posição).
    refocus: bool,
    /// Aviso aberto: o Exportar falhou (a mensagem).
    export_error: Option<String>,
    /// Aviso aberto: fechar a janela com uma nota que não foi gravada.
    pub close_error: Option<String>,
}

/// Mensagem de um erro de gravação para quem usa.
fn io_message(err: &io::Error) -> String {
    match err.kind() {
        io::ErrorKind::PermissionDenied => "Sem permissão para gravar nesta pasta.".into(),
        io::ErrorKind::StorageFull => "Não há espaço no disco.".into(),
        _ => err.to_string(),
    }
}

/// Cor entre duas (a das variáveis: entre o destaque e o texto).
fn mix(a: Rgba, b: Rgba) -> Rgba {
    let half = |i: usize| ((a[i] as u16 + b[i] as u16) / 2) as u8;
    [half(0), half(1), half(2), 255]
}

/// As decorações da nota: variáveis coloridas, erros sublinhados e o resultado depois
/// do fim da linha.
fn decorations(lines: &[LineEval], t: &Theme) -> Decorations {
    let mut deco = Decorations::default();
    let name = mix(t.accent, t.text);
    for (i, line) in lines.iter().enumerate() {
        for range in &line.names {
            deco.colors.push((range.clone(), name));
        }
        for problem in &line.problems {
            deco.underlines.push(problem.range.clone());
        }
        if let Some(shown) = &line.result {
            deco.trailing.push((i, shown.text.clone(), t.accent));
        }
    }
    deco
}

/// O que o log registra de uma linha: " = 360", ": erro: …" ou nada.
fn log_text(line: &LineEval) -> String {
    match (&line.result, line.problems.first()) {
        (Some(shown), _) => format!(" = {}", shown.text),
        (None, Some(problem)) => format!(": erro: {}", problem.message),
        _ => String::new(),
    }
}

/// Corta o fim do texto com "…" até caber em `width`.
fn fit_end(text: &mut Text, s: &str, size: f32, width: f32) -> String {
    if text.measure(s, size, FontWeight::Regular) <= width {
        return s.to_string();
    }
    let mut chars: Vec<char> = s.chars().collect();
    while !chars.is_empty() {
        chars.pop();
        let shown = format!("{}…", chars.iter().collect::<String>().trim_end());
        if text.measure(&shown, size, FontWeight::Regular) <= width {
            return shown;
        }
    }
    "…".into()
}

/// A linha do texto onde está a posição `at`.
fn line_at(text: &str, at: usize) -> usize {
    line_of(&line_starts(text), at)
}

/// As contas de um texto, com a cache (só refaz quando a chave muda).
fn evaluate_cached(cache: &mut Option<(CacheKey, Vec<LineEval>)>, key: CacheKey, text: &str, ctx: &Context) -> Vec<LineEval> {
    if cache.as_ref().is_none_or(|(k, _)| *k != key) {
        *cache = Some((key, notes::evaluate(text, ctx)));
    }
    cache.as_ref().map(|(_, lines)| lines.clone()).unwrap_or_default()
}

impl Notes {
    /// `list_shown`: a lista de notas aparece (a opção `notes_list`).
    pub fn new(list_shown: bool) -> Self {
        Self { list_shown, ..Default::default() }
    }

    /// Abre as notas na primeira vez (a última aberta fica na opção `nota`).
    fn session(&mut self) -> &mut Session {
        self.session.get_or_insert_with(|| {
            let last = match settings_file::get("calculator", "nota") {
                Some(Value::Text(id)) => Some(id),
                _ => None,
            };
            let session = Session::open(store::notes_dir(), last.as_deref(), chrono::Local::now());
            tracing::debug!("calculadora: notas: {} nota(s), aberta {}", session.list.len(), session.current());
            session
        })
    }

    /// Grava a nota aberta agora (ao trocar de modo, de nota e ao fechar); `true` se
    /// gravou (a lista mudou: título e ordem).
    pub fn save_now(&mut self) -> bool {
        self.seen = None;
        let Some(session) = self.session.as_mut() else {
            return false;
        };
        let id = session.current().to_string();
        match session.save() {
            Ok(saved) => saved,
            Err(err) => {
                tracing::warn!("calculadora: nota {id} não gravada: {err}");
                false
            }
        }
    }

    /// As contas da nota aberta.
    fn lines(&mut self, ctx: &Context) -> Vec<LineEval> {
        let session = self.session();
        let id = session.current().to_string();
        let doc = session.doc();
        let key = CacheKey { id, rates: ctx.rates.map(|r| r.date), version: doc.version, radians: ctx.radians, style: ctx.style };
        let text = doc.text.clone();
        evaluate_cached(&mut self.cache, key, &text, ctx)
    }

    /// O resultado da linha `line` da nota aberta.
    fn result(&mut self, ctx: &Context, line: usize) -> Option<Shown> {
        self.lines(ctx).get(line).and_then(|l| l.result.clone())
    }

    /// O texto da nota aberta com os resultados escritos depois de cada "=".
    fn with_results(&mut self, ctx: &Context) -> String {
        let lines = self.lines(ctx);
        notes::with_results(&self.session().doc().text, &lines)
    }
}

impl Calculator {
    /// As opções das notas, com as cotações (um `Rc`: o contexto empresta deste valor,
    /// não do `self`, que continua livre para mudar).
    fn notes_context(&self) -> NotesCtx {
        NotesCtx { radians: self.engine.radians, style: self.engine.style, rates: self.money.rates.clone() }
    }

    /// Lembra a nota aberta (abre nela da próxima vez).
    fn remember_note(&mut self) {
        let id = self.notes.session().current().to_string();
        self.save("nota", Some(Value::Text(id)));
    }

    fn notes_new(&mut self) {
        self.notes.save_now();
        if let Err(err) = self.notes.session().new_note(chrono::Local::now()) {
            tracing::warn!("calculadora: nota não gravada: {err}");
        }
        tracing::debug!("calculadora: notas: nova nota, {} nota(s)", self.notes.session().list.len());
        self.remember_note();
    }

    fn notes_select(&mut self, id: &str) {
        self.notes.save_now();
        if let Err(err) = self.notes.session().select(id) {
            tracing::warn!("calculadora: nota não gravada: {err}");
        }
        tracing::debug!("calculadora: notas: aberta {id}");
        self.remember_note();
    }

    fn notes_trash(&mut self) {
        let trash = mide::places::trash_dir();
        match self.notes.session().trash(&trash, chrono::Local::now()) {
            Ok(id) => tracing::debug!("calculadora: notas: na Lixeira {id}"),
            Err(err) => tracing::warn!("calculadora: nota não foi para a Lixeira: {err}"),
        }
        self.notes.seen = None;
        self.remember_note();
    }

    /// Mostra ou esconde a lista. A nota muda de lugar no quadro (o kit guarda o foco
    /// pela posição): o foco volta para ela.
    fn toggle_notes_list(&mut self, ui: &mut Ui) {
        self.notes.list_shown = !self.notes.list_shown;
        self.save("notes_list", (!self.notes.list_shown).then_some(Value::Bool(false)));
        ui.focus_next_input();
        self.notes.refocus = true;
        ui.request_repaint();
    }

    /// Fechar a janela: grava as notas; se falhar, um aviso pergunta o que fazer.
    pub(super) fn notes_close(&mut self) -> bool {
        let Some(session) = self.notes.session.as_mut() else {
            return true;
        };
        match session.save_all() {
            Ok(()) => true,
            Err(err) => {
                tracing::warn!("calculadora: nota não gravada ao fechar: {err}");
                self.notes.close_error = Some(io_message(&err));
                false
            }
        }
    }

    /// Os avisos das notas (por cima de tudo, no fim do `view`).
    pub(super) fn notes_dialogs(&mut self, ui: &mut Ui) {
        if let Some(message) = self.notes.close_error.clone() {
            let text = format!("{message} A nota tem alterações que ainda não estão no disco.");
            match ui.alert("A nota não foi gravada", &text, &["Cancelar", "Fechar sem gravar", "Tentar de novo"]) {
                Some(1) => {
                    self.notes.close_error = None;
                    self.notes.session = None;
                    ui.close();
                }
                Some(2) => {
                    self.notes.close_error = None;
                    if self.notes_close() {
                        ui.close();
                    }
                }
                Some(_) => self.notes.close_error = None,
                None => {}
            }
        } else if let Some(message) = self.notes.export_error.clone()
            && ui.alert("A nota não foi exportada", &message, &["OK"]).is_some()
        {
            self.notes.export_error = None;
            ui.focus_next_input();
        }
    }

    /// Copia e registra no log (os testes conferem o que foi copiado).
    fn notes_copy(&self, ui: &mut Ui, text: &str) {
        tracing::debug!("calculadora: notas: copiado {text}");
        ui.copy(text);
    }

    /// "Levar para a calculadora": volta ao último modo de teclado com o número.
    fn notes_take(&mut self, ui: &mut Ui, value: f64) {
        let mode = self.notes.keypad.unwrap_or(Mode::Basic);
        self.set_mode(ui, mode);
        self.put(value, None);
    }

    /// Ações dos menus da barra no modo Notas; `true` se a ação era das notas.
    pub(super) fn notes_menu_action(&mut self, ui: &mut Ui, action: &str) -> bool {
        // Com o seletor do Exportar (ou um aviso) aberto, o que mexe na nota espera; o
        // Colar vai para o campo do nome.
        let busy = self.notes.export.is_some() || self.notes.export_error.is_some() || self.notes.close_error.is_some();
        if busy && matches!(action, "nota-nova" | "nota-lixeira" | "nota-exportar" | "notas-lista" | "desfazer" | "refazer" | "cortar" | "copiar" | "selecionar-tudo" | "basica" | "cientifica" | "programador") {
            return true;
        }
        let owned = self.notes_context();
        let ctx = owned.get();
        match action {
            "nota-nova" => {
                self.notes_new();
                ui.focus_next_input();
                ui.request_repaint();
            }
            "nota-lixeira" => {
                self.notes_trash();
                ui.focus_next_input();
                ui.request_repaint();
            }
            "nota-exportar" => {
                let name = format!("{}.txt", self.notes.session().title().replace('/', "-"));
                self.notes.export = Some(FileChooser::save("Exportar a nota com os resultados", None, &name));
            }
            "notas-lista" => self.toggle_notes_list(ui),
            "desfazer" => {
                self.notes.session().doc().undo();
            }
            "refazer" => {
                self.notes.session().doc().redo();
            }
            "cortar" => {
                let doc = self.notes.session().doc();
                if let Some(cut) = doc.edit.selected(&doc.text).map(str::to_string) {
                    doc.before_change(Change::Other);
                    doc.edit.delete_selection(&mut doc.text);
                    self.notes_copy(ui, &cut);
                }
            }
            "copiar" => {
                let doc = self.notes.session().doc();
                if let Some(text) = doc.edit.selected(&doc.text).map(str::to_string) {
                    self.notes_copy(ui, &text);
                }
            }
            "colar" => ui.request_paste(),
            "selecionar-tudo" => {
                let doc = self.notes.session().doc();
                doc.edit = Edit::all(&doc.text);
            }
            "copiar-resultado" => {
                let doc = self.notes.session().doc();
                let line = line_at(&doc.text, doc.edit.cursor);
                if let Some(shown) = self.notes.result(&ctx, line) {
                    self.notes_copy(ui, &shown.text);
                }
            }
            "copiar-nota" => {
                let text = self.notes.with_results(&ctx);
                self.notes_copy(ui, &text);
            }
            "radianos" => self.set_radians(!self.engine.radians),
            _ => return false,
        }
        true
    }

    /// Menus da barra no modo Notas: Arquivo e Editar (o Visualizar é o de sempre).
    pub(super) fn notes_menus(&mut self) -> Vec<MenuGroup> {
        let owned = self.notes_context();
        let ctx = owned.get();
        let doc = self.notes.session().doc();
        let (undo, redo) = (doc.can_undo(), doc.can_redo());
        let selection = doc.edit.selection().is_some_and(|(a, b)| a != b);
        let line = line_at(&doc.text, doc.edit.cursor);
        let has_result = self.notes.result(&ctx, line).is_some();
        vec![
            MenuGroup {
                title: "Arquivo".into(),
                items: vec![
                    MenuItem::action("nota-nova", "Nova nota").shortcut("Ctrl+N"),
                    MenuItem::action("nota-exportar", "Exportar com os resultados…"),
                    MenuItem::Separator,
                    MenuItem::action("nota-lixeira", "Mover a nota para a Lixeira"),
                ],
            },
            MenuGroup {
                title: "Editar".into(),
                items: vec![
                    MenuItem::action("desfazer", "Desfazer").shortcut("Ctrl+Z").enabled(undo),
                    MenuItem::action("refazer", "Refazer").shortcut("Ctrl+Shift+Z").enabled(redo),
                    MenuItem::Separator,
                    MenuItem::action("cortar", "Cortar").shortcut("Ctrl+X").enabled(selection),
                    MenuItem::action("copiar", "Copiar").shortcut("Ctrl+C").enabled(selection),
                    MenuItem::action("colar", "Colar").shortcut("Ctrl+V"),
                    MenuItem::action("selecionar-tudo", "Selecionar tudo").shortcut("Ctrl+A"),
                    MenuItem::Separator,
                    MenuItem::action("copiar-resultado", "Copiar o resultado da linha").enabled(has_result),
                    MenuItem::action("copiar-nota", "Copiar a nota com os resultados"),
                ],
            },
        ]
    }

    /// O modo Notas: a lista (Ctrl+T), a barra de ferramentas, a nota e a linha de
    /// estado.
    pub(super) fn notes_view(&mut self, ui: &mut Ui) {
        // Primeiro quadro das notas (o app abriu nelas): o texto já recebe as teclas.
        if self.notes.session.is_none() || std::mem::take(&mut self.notes.refocus) {
            ui.focus_next_input();
        }
        // Cotações que chegaram: as contas com moeda são refeitas (a data entra na cache).
        if self.money.poll() {
            ui.request_repaint();
        }
        // Atalhos do modo (os de edição ficam com a área de texto); com o seletor do
        // Exportar ou um aviso aberto, as teclas são dele.
        if self.notes.export.is_none() && self.notes.export_error.is_none() && self.notes.close_error.is_none() {
            let input = ui.input().clone();
            for (key, mode) in [("1", Mode::Basic), ("2", Mode::Scientific), ("3", Mode::Programmer)] {
                if input.shortcut(key) {
                    self.set_mode(ui, mode);
                    return;
                }
            }
            if input.shortcut("n") {
                self.notes_new();
                ui.focus_next_input();
                ui.request_repaint();
            }
            if input.shortcut("t") {
                self.toggle_notes_list(ui);
            }
        }
        self.notes_screen(ui);
        // Exportar: o seletor de arquivos por cima de tudo, por último (um modal no
        // começo empurraria a nota para baixo, e o Enter que confirma o nome entraria na
        // nota no mesmo quadro). A nota volta a receber as teclas no quadro seguinte.
        if let Some(chooser) = self.notes.export.as_mut() {
            match chooser.show(ui) {
                Some(Choice::Chosen(path)) => {
                    self.notes.export = None;
                    let text = self.notes.with_results(&self.notes_context().get());
                    match mide::files::write_atomic(&path, text.as_bytes()) {
                        Ok(()) => {
                            tracing::debug!("calculadora: notas: exportada {}", path.display());
                            ui.focus_next_input();
                        }
                        Err(err) => {
                            tracing::warn!("calculadora: nota não exportada: {err}");
                            self.notes.export_error = Some(format!("{} ({})", io_message(&err), path.display()));
                            ui.blur();
                            ui.request_repaint();
                        }
                    }
                }
                Some(Choice::Cancelled) => {
                    self.notes.export = None;
                    ui.focus_next_input();
                }
                None => {}
            }
        }
        let was_open = self.money.is_open();
        self.money_dialogs(ui);
        // A janela das cotações fechou: a nota volta a receber as teclas.
        if was_open && !self.money.is_open() {
            self.notes.refocus = true;
            ui.request_repaint();
        }
        self.notes_dialogs(ui);
    }

    /// A lista (se aparece) e a nota.
    fn notes_screen(&mut self, ui: &mut Ui) {
        let t = ui.theme().clone();
        if self.notes.list_shown {
            let session = self.notes.session();
            let titles: Vec<String> = session.list.iter().map(|n| n.title.clone()).collect();
            // Os ids na ordem desenhada (a gravação pode reordenar a lista no meio).
            let ids: Vec<String> = session.list.iter().map(|n| n.id.clone()).collect();
            let selected = session.current_index() + 1;
            let mut items: Vec<(&str, &str)> = vec![("Notas", "")];
            items.extend(titles.iter().map(|title| (title.as_str(), NOTE_ICON)));
            let mut taken = false;
            let chosen = ui.sidebar_area(&items, selected, |ui, _| taken = self.notes_page(ui, &t));
            if taken {
                return;
            }
            if chosen != selected && chosen > 0 {
                let id = ids.get(chosen - 1).cloned();
                if let Some(id) = id {
                    self.notes_select(&id);
                    ui.focus_next_input();
                    ui.request_repaint();
                }
            }
        } else {
            self.notes_page(ui, &t);
        }
    }

    /// A nota em si: barra de ferramentas, texto e linha de estado. `true` se a nota
    /// levou um número para a calculadora (o modo mudou).
    fn notes_page(&mut self, ui: &mut Ui, t: &Theme) -> bool {
        let owned = self.notes_context();
        let ctx = owned.get();
        let (_, _, _, h) = ui.region();
        let title = self.notes.session().title();
        let mut clicked: Option<&str> = None;
        ui.toolbar(|ui| {
            if ui.icon_button("view-list-symbolic", "☰", true).clicked() {
                clicked = Some("lista");
            }
            let (_, _, inner, _) = ui.region();
            ui.sized((inner - 130.0).max(60.0), |ui| ui.strong(&title));
            ui.push_right(76.0);
            if ui.icon_button("document-new-symbolic", "+", true).clicked() {
                clicked = Some("nova");
            }
            if ui.icon_button("user-trash-symbolic", "🗑", true).clicked() {
                clicked = Some("lixeira");
            }
        });
        match clicked {
            Some("lista") => self.toggle_notes_list(ui),
            Some("nova") => {
                self.notes_new();
                self.notes.refocus = true;
            }
            Some("lixeira") => {
                self.notes_trash();
                self.notes.refocus = true;
            }
            _ => {}
        }
        if clicked.is_some() {
            ui.request_repaint();
        }
        // A nota ocupa o resto, menos a linha de estado embaixo. As contas vêm da
        // cache, já com a tecla deste quadro.
        let area_h = (h - 52.0 - STATUS_H - 12.0).max(80.0);
        let Notes { session, cache, .. } = &mut self.notes;
        let session = session.as_mut().expect("notas abertas");
        let id = session.current().to_string();
        let mut decorate = |d: &TextDoc| {
            let key = CacheKey { id: id.clone(), rates: ctx.rates.map(|r| r.date), version: d.version, radians: ctx.radians, style: ctx.style };
            decorations(&evaluate_cached(cache, key, &d.text, &ctx), t)
        };
        let look = TextAreaLook {
            line_numbers: false,
            sans: true,
            size: Some(FONT),
            placeholder: "Escreva uma conta e termine com “=” (por exemplo, 120 × 3 =)",
            decorate: Some(&mut decorate),
        };
        let r = ui.text_area_with(session.doc(), area_h, look);
        // Clique direito: o menu do resultado da linha (a de quando abriu) e o de editar.
        if r.response.secondary_clicked {
            let text = &self.notes.session().doc().text;
            self.notes.menu_line = r.pointer_index.map(|i| line_at(text, i));
        }
        let menu_result = self.notes.menu_line.and_then(|line| self.notes.result(&ctx, line));
        let mut chosen: Option<&str> = None;
        ui.context_menu(&r.response, |m| {
            if let Some(shown) = &menu_result {
                if m.item(&format!("Copiar “{}”", shown.text)) {
                    chosen = Some("copiar-este");
                }
                if m.item("Copiar só o número") {
                    chosen = Some("copiar-numero");
                }
                if m.item("Levar para a calculadora") {
                    chosen = Some("levar");
                }
                m.separator();
            }
            if m.item("Cortar") {
                chosen = Some("cortar");
            }
            if m.item("Copiar") {
                chosen = Some("copiar");
            }
            if m.item("Colar") {
                chosen = Some("colar");
            }
            if m.item("Selecionar tudo") {
                chosen = Some("selecionar-tudo");
            }
        });
        match (chosen, menu_result) {
            (Some("copiar-este"), Some(shown)) => self.notes_copy(ui, &shown.text),
            (Some("copiar-numero"), Some(shown)) => self.notes_copy(ui, &shown.number),
            (Some("levar"), Some(shown)) => {
                self.notes_take(ui, shown.value);
                return true;
            }
            (Some(action), _) => {
                self.notes_menu_action(ui, action);
            }
            _ => {}
        }
        // Linha de estado: o erro da linha do cursor e Graus/Radianos.
        let lines = self.notes.lines(&ctx);
        let doc = self.notes.session().doc();
        let line = line_at(&doc.text, doc.edit.cursor);
        let read_only = self.notes.session().read_only();
        // Uma nota que converte moedas pede as cotações (a busca, ou a pergunta da
        // primeira vez, como no conversor).
        let needs_rates = notes::needs_rates(&lines);
        if needs_rates {
            let online = self.currency_online();
            self.money.ensure(online, false, ui.waker());
        }
        let error = if read_only {
            Some("Esta nota não está em UTF-8: dá para ler, mas as mudanças não são gravadas.".to_string())
        } else {
            lines.get(line).and_then(|l| l.problems.first()).map(|p| p.message.clone())
        };
        // Sem erro na linha do cursor, a fonte e a data das cotações em uso (em cinza).
        let info = (error.is_none() && needs_rates).then(|| format!("Cotações: {}", self.money.band(self.currency_online())));
        let message = error.clone().or(info);
        let is_error = error.is_some();
        let mut toggle = false;
        let mut button = None;
        ui.row(|ui| {
            ui.push_right(100.0);
            ui.sized(100.0, |ui| {
                let r = ui.button(if ctx.radians { "Radianos" } else { "Graus" });
                toggle = r.clicked();
                button = Some(r.rect);
            });
        });
        if toggle {
            self.set_radians(!ctx.radians);
        }
        // O erro, à esquerda do botão e no meio da altura dele, em vermelho como o
        // sublinhado.
        if let (Some(message), Some((bx, by, _, bh))) = (message, button) {
            let (x, _, _, _) = ui.region();
            let size = t.font_small;
            let color = if is_error { t.danger } else { t.text_secondary };
            ui.paint_text(|pixmap, text, (ox, oy)| {
                let shown = fit_end(text, &message, size, bx - x - 24.0);
                text.draw(pixmap, &shown, ox + x + 12.0, oy + by + (bh - size * 1.25) / 2.0, size, FontWeight::Regular, color);
            });
        }
        // Log: só o que mudou em cada linha.
        let texts: Vec<String> = lines.iter().map(log_text).collect();
        for (i, text) in texts.iter().enumerate() {
            if !text.is_empty() && self.notes.logged.get(i) != Some(text) {
                tracing::debug!("calculadora: notas: linha {}{text}", i + 1);
            }
        }
        self.notes.logged = texts;
        self.autosave(ui);
        false
    }

    /// Gravação automática: 1 s depois da última tecla, numa thread (o `fsync` não
    /// segura a janela), que acorda a janela quando termina.
    fn autosave(&mut self, ui: &mut Ui) {
        // Com o botão do mouse apertado, a lista não muda de ordem embaixo do clique.
        let held = ui.input().down;
        let session = self.notes.session();
        if let Some((id, result)) = if held { None } else { session.poll() } {
            match result {
                // Gravou: o título e a ordem da lista mudam já neste quadro.
                Ok(()) => ui.request_repaint(),
                // Falhou: tenta de novo 1 s depois (a espera recomeça).
                Err(err) => {
                    tracing::warn!("calculadora: nota {id} não gravada: {err}");
                    self.notes.seen = None;
                }
            }
        }
        let session = self.notes.session();
        let (id, version) = (session.current().to_string(), session.doc().version);
        if self.notes.seen.as_ref().is_none_or(|(i, v, _)| *i != id || *v != version) {
            self.notes.seen = Some((id, version, Instant::now()));
        }
        let session = self.notes.session();
        if session.saving() {
            // Sem `Waker` (ou se ele falhar), confere de novo daqui a pouco.
            ui.wake_after(Duration::from_millis(100));
        } else if session.dirty() {
            let left = self.notes.seen.as_ref().map_or(Duration::ZERO, |(_, _, at)| SAVE_AFTER.saturating_sub(at.elapsed()));
            if left.is_zero() {
                let waker = ui.waker();
                if self.notes.session().save_in_background(waker) {
                    ui.wake_after(Duration::from_millis(100));
                }
            } else {
                ui.wake_after(left);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mide-calc-notas-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn now(s: u32) -> chrono::DateTime<chrono::Local> {
        chrono::Local.with_ymd_and_hms(2026, 10, 22, 10, 0, s).unwrap()
    }

    fn files(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        names
    }

    #[test]
    fn nota_nova_vazia_nao_grava() {
        let dir = temp("vazia");
        let mut s = Session::open(dir.clone(), None, now(0));
        assert_eq!(s.list.len(), 1);
        assert!(!s.save().unwrap());
        s.doc().type_text("   ");
        assert!(!s.save().unwrap());
        assert!(files(&dir).is_empty());
        // Com texto, vira arquivo.
        s.doc().type_text("2+2=");
        assert!(s.save().unwrap());
        assert_eq!(files(&dir), ["20261022-100000.txt"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn trocar_de_nota_grava_a_anterior_e_a_editada_sobe() {
        let dir = temp("trocar");
        store::save(&dir, "20261021-090000", "antiga").unwrap();
        std::thread::sleep(Duration::from_millis(20));
        store::save(&dir, "20261021-100000", "recente").unwrap();
        let mut s = Session::open(dir.clone(), None, now(0));
        // Abre a mais recente.
        assert_eq!(s.current(), "20261021-100000");
        s.doc().type_text(" editada");
        s.select("20261021-090000").unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("20261021-100000.txt")).unwrap(), "recente editada");
        assert_eq!(s.doc().text, "antiga");
        // A antiga editada sobe para o topo da lista.
        std::thread::sleep(Duration::from_millis(20));
        s.doc().type_text("!");
        s.save().unwrap();
        assert_eq!(s.list[0].id, "20261021-090000");
        assert_eq!(s.title(), "antiga!");
        // Reabrir lembra a última aberta.
        let s2 = Session::open(dir.clone(), Some("20261021-100000"), now(0));
        assert_eq!(s2.current(), "20261021-100000");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn nova_nota_reaproveita_a_vazia() {
        let dir = temp("nova");
        let mut s = Session::open(dir.clone(), None, now(0));
        s.doc().type_text("primeira");
        s.new_note(now(1)).unwrap();
        assert_eq!(s.list.len(), 2);
        assert_eq!(s.current(), "20261022-100001");
        // Outra nova com a aberta vazia: fica na mesma.
        s.new_note(now(2)).unwrap();
        assert_eq!(s.list.len(), 2);
        assert_eq!(files(&dir), ["20261022-100000.txt"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn lixeira_abre_a_proxima_ou_uma_nova() {
        let dir = temp("lixeira");
        let trash = temp("lixeira-trash");
        store::save(&dir, "a", "um").unwrap();
        std::thread::sleep(Duration::from_millis(20));
        store::save(&dir, "b", "dois").unwrap();
        let mut s = Session::open(dir.clone(), None, now(0));
        assert_eq!(s.current(), "b");
        assert_eq!(s.trash(&trash, now(1)).unwrap(), "b");
        assert_eq!(s.current(), "a");
        assert!(trash.join("files/b.txt").exists());
        assert_eq!(s.trash(&trash, now(2)).unwrap(), "a");
        // Sem notas: uma nova vazia (sem arquivo).
        assert_eq!(s.list.len(), 1);
        assert!(s.doc().text.is_empty());
        assert!(files(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&trash);
    }

    #[test]
    fn arquivo_mudado_fora_e_relido() {
        let dir = temp("fora");
        store::save(&dir, "a", "um").unwrap();
        store::save(&dir, "b", "dois").unwrap();
        let mut s = Session::open(dir.clone(), Some("a"), now(0));
        s.select("b").unwrap();
        // Outro programa muda a nota "a", que não tem alteração aqui: relida ao voltar.
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(dir.join("a.txt"), "um, mudado fora").unwrap();
        s.select("a").unwrap();
        assert_eq!(s.doc().text, "um, mudado fora");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mudada_fora_com_alteracao_aqui_vira_copia() {
        let dir = temp("copia");
        store::save(&dir, "a", "um").unwrap();
        let mut s = Session::open(dir.clone(), Some("a"), now(0));
        // Outra Calculadora grava a mesma nota; aqui também houve alteração.
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(dir.join("a.txt"), "um, da outra janela").unwrap();
        s.doc().type_text(", daqui");
        assert!(s.save().unwrap());
        // Nenhum dos dois textos se perde: o daqui vai para uma nota nova.
        assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "um, da outra janela");
        assert_ne!(s.current(), "a");
        assert_eq!(std::fs::read_to_string(dir.join(format!("{}.txt", s.current()))).unwrap(), "um, daqui");
        assert_eq!(s.list.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn nota_que_nao_e_utf8_fica_so_para_ler() {
        let dir = temp("latin1");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("velha.txt"), b"Or\xe7amento = 10").unwrap();
        let mut s = Session::open(dir.clone(), None, now(0));
        assert!(s.read_only());
        assert!(s.doc().text.starts_with("Or"));
        s.doc().type_text("!");
        assert!(!s.save().unwrap());
        // O arquivo continua com os bytes de antes.
        assert_eq!(std::fs::read(dir.join("velha.txt")).unwrap(), b"Or\xe7amento = 10");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn gravacao_automatica_em_outra_thread() {
        let dir = temp("thread");
        let mut s = Session::open(dir.clone(), None, now(0));
        let id = s.current().to_string();
        s.doc().type_text("2+2=");
        assert!(s.save_in_background(None));
        assert!(s.saving());
        // Outra não começa enquanto a primeira não termina.
        assert!(!s.save_in_background(None));
        let mut done = None;
        for _ in 0..400 {
            if let Some(result) = s.poll() {
                done = Some(result);
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let (saved_id, result) = done.expect("a gravação terminou");
        assert!(result.is_ok());
        assert_eq!(saved_id, id);
        assert!(!s.saving() && !s.dirty());
        assert_eq!(s.title(), "2+2=");
        // Digitar durante a gravação: a gravação direta espera a automática e grava o
        // texto novo por último.
        s.doc().type_text("x");
        assert!(s.save_in_background(None));
        s.doc().type_text("y");
        assert!(s.save().unwrap());
        assert!(!s.saving() && !s.dirty());
        assert_eq!(std::fs::read_to_string(dir.join(format!("{id}.txt"))).unwrap(), "2+2=xy");
        // A Lixeira espera a gravação em andamento: nada volta para a pasta depois.
        let trash = temp("thread-trash");
        s.doc().type_text("z");
        assert!(s.save_in_background(None));
        s.trash(&trash, now(1)).unwrap();
        assert_eq!(std::fs::read_to_string(trash.join(format!("files/{id}.txt"))).unwrap(), "2+2=xyz");
        assert!(files(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&trash);
    }

    #[test]
    fn cache_cores_e_log() {
        assert_eq!(mix([0, 0, 255, 255], [255, 255, 255, 255]), [127, 127, 255, 255]);
        let ctx = Context { radians: false, style: Style::default(), rates: None };
        let text = "120 × 3 =\nx = 2\ny * 2 =";
        let key = |version: u64, radians: bool| CacheKey { id: "a".into(), rates: None, version, radians, style: Style::default() };
        let mut cache = None;
        let lines = evaluate_cached(&mut cache, key(1, false), text, &ctx);
        assert_eq!(log_text(&lines[0]), " = 360");
        assert_eq!(log_text(&lines[1]), "");
        assert!(log_text(&lines[2]).starts_with(": erro: "));
        // A mesma chave não refaz as contas (outro texto com a chave igual: a cache vale).
        assert_eq!(evaluate_cached(&mut cache, key(1, false), "", &ctx), lines);
        // Outra versão, Rad ou formato: refaz.
        assert!(evaluate_cached(&mut cache, key(2, false), "", &ctx).iter().all(|l| l.result.is_none()));
        assert_eq!(evaluate_cached(&mut cache, key(2, true), text, &ctx), lines);
        let mut other = key(2, true);
        other.style = Style { decimals: Some(2), grouping: false };
        assert!(evaluate_cached(&mut cache, other, "", &ctx).iter().all(|l| l.result.is_none()));
        let t = Theme::default();
        let deco = decorations(&lines, &t);
        assert_eq!(deco.trailing, [(0, "360".to_string(), t.accent)]);
        assert_eq!(deco.underlines.len(), 1);
        assert_eq!(notes::with_results("120 × 3 =", &lines[..1]), "120 × 3 = 360");
    }
}
