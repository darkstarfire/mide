//! Arquivos do mide: gerenciador de arquivos no estilo do Finder, feito com o mide-ui.
//!
//! Barra lateral com os Favoritos e a Lixeira; barra de ferramentas com voltar e
//! avançar, o nome da pasta, ícones ou lista e busca; grade de ícones (com miniaturas
//! das imagens) ou lista com tamanho e data. Clique seleciona, duplo clique abre, o
//! botão direito abre o menu de contexto do item ou da pasta.
//!
//! Na lista, como no Finder: setinha para abrir uma pasta na própria lista (ou as
//! setas → e ←), linhas alternadas, colunas Modificado, Tamanho (das pastas também,
//! calculado em segundo plano) e Tipo, e o cabeçalho ordena.
//!
//! Teclado: Enter abre, Delete manda para a Lixeira, Backspace sobe uma pasta, setas
//! mudam a seleção, Ctrl+N cria uma pasta, Ctrl+H mostra os ocultos, Ctrl+F busca,
//! Ctrl+R renomeia, Ctrl+C copia, Ctrl+V cola e Ctrl+D duplica.
//!
//! Uso: `mide-files [pasta]` (ou `mide-files trash:///` para abrir a Lixeira).

use std::{
    collections::{HashMap, HashSet},
    error::Error,
    path::{Path, PathBuf},
    sync::mpsc::{channel, Receiver, Sender},
    time::SystemTime,
};

use mide::{
    files::{self, Entry},
    kit::{run, App, Key, Tile, Ui, Window},
    places,
};
use tiny_skia::Pixmap;

/// Lado das miniaturas e dos ícones da grade.
const TILE: u32 = 64;

#[derive(Clone, PartialEq)]
enum Place {
    Dir(PathBuf),
    Trash,
}

struct Files {
    place: Place,
    back: Vec<Place>,
    forward: Vec<Place>,
    entries: Vec<Entry>,
    selected: Option<PathBuf>,
    list_view: bool,
    hidden: bool,
    search: String,
    searching: bool,
    renaming: Option<(PathBuf, String)>,
    /// O campo do nome novo ainda precisa receber o foco.
    focus_rename: bool,
    message: Option<String>,
    favorites: Vec<(String, PathBuf, &'static str)>,
    thumbs: HashMap<PathBuf, Option<Pixmap>>,
    waiting: HashSet<PathBuf>,
    to_loader: Sender<PathBuf>,
    from_loader: Receiver<(PathBuf, Option<Pixmap>)>,
    /// Lista: pastas abertas na própria lista e o que há dentro delas.
    expanded: HashSet<PathBuf>,
    children: HashMap<PathBuf, Vec<Entry>>,
    /// Coluna ordenada (0 nome, 1 modificado, 2 tamanho, 3 tipo) e se é crescente.
    sort: (usize, bool),
    /// Tamanho das pastas (calculado numa thread; `None` = grande demais).
    sizes: HashMap<PathBuf, Option<u64>>,
    sizes_waiting: HashSet<PathBuf>,
    to_sizer: Sender<PathBuf>,
    from_sizer: Receiver<(PathBuf, Option<u64>)>,
    /// Item copiado (Copiar / Ctrl+C), para Colar em outra pasta.
    clipboard: Option<PathBuf>,
    /// Compactar e extrair rodam numa thread: o resultado chega aqui.
    jobs_tx: Sender<Result<PathBuf, String>>,
    jobs_rx: Receiver<Result<PathBuf, String>>,
    busy: usize,
}

impl Files {
    fn new(start: Place) -> Self {
        // Miniaturas carregam numa linha de execução à parte, para a janela não travar
        // numa pasta cheia de fotos grandes.
        let (to_loader, jobs) = channel::<PathBuf>();
        let (done, from_loader) = channel();
        std::thread::spawn(move || {
            for path in jobs {
                let thumb = mide::icons::load_icon_file(&path.to_string_lossy(), TILE);
                if done.send((path, thumb)).is_err() {
                    break;
                }
            }
        });
        let (to_sizer, size_jobs) = channel::<PathBuf>();
        let (size_done, from_sizer) = channel();
        std::thread::spawn(move || {
            for path in size_jobs {
                let size = files::dir_size(&path, 50_000);
                if size_done.send((path, size)).is_err() {
                    break;
                }
            }
        });
        let (jobs_tx, jobs_rx) = channel();
        let mut app = Self {
            place: start,
            expanded: HashSet::new(),
            children: HashMap::new(),
            sort: (0, true),
            sizes: HashMap::new(),
            sizes_waiting: HashSet::new(),
            to_sizer,
            from_sizer,
            clipboard: None,
            jobs_tx,
            jobs_rx,
            busy: 0,
            back: Vec::new(),
            forward: Vec::new(),
            entries: Vec::new(),
            selected: None,
            list_view: false,
            hidden: false,
            search: String::new(),
            searching: false,
            renaming: None,
            focus_rename: false,
            message: None,
            favorites: files::favorites(),
            thumbs: HashMap::new(),
            waiting: HashSet::new(),
            to_loader,
            from_loader,
        };
        app.reload();
        app
    }

    fn trash() -> PathBuf {
        places::trash_dir()
    }

    /// Lê de novo a pasta atual (depois de criar, renomear ou apagar).
    fn reload(&mut self) {
        let dir = match &self.place {
            Place::Dir(dir) => dir.clone(),
            Place::Trash => Self::trash().join("files"),
        };
        self.entries = match files::list_dir(&dir, self.hidden) {
            Ok(list) => list,
            Err(err) if self.place == Place::Trash && err.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(err) => {
                self.message = Some(format!("Não foi possível abrir a pasta: {err}"));
                Vec::new()
            }
        };
        self.sort_entries();
        // Pastas abertas na lista: relê o que há dentro (e esquece as que sumiram).
        self.expanded.retain(|p| p.is_dir());
        let expanded: Vec<PathBuf> = self.expanded.iter().cloned().collect();
        self.children.clear();
        for dir in expanded {
            let mut list = files::list_dir(&dir, self.hidden).unwrap_or_default();
            sort_list(&mut list, self.sort);
            self.children.insert(dir, list);
        }
        // Tamanhos mudam depois de copiar, apagar ou renomear.
        self.sizes.clear();
        if self.selected.as_ref().is_some_and(|s| !s.exists()) {
            self.selected = None;
        }
    }

    fn sort_entries(&mut self) {
        sort_list(&mut self.entries, self.sort);
        for list in self.children.values_mut() {
            sort_list(list, self.sort);
        }
    }

    /// Linhas da lista: cada item com o nível dele; dentro das pastas abertas, os itens
    /// delas logo abaixo, um nível para dentro. Na busca, só o nível de cima.
    fn rows(&self, shown: &[Entry]) -> Vec<(usize, Entry)> {
        fn push(rows: &mut Vec<(usize, Entry)>, list: &[Entry], depth: usize, expanded: &HashSet<PathBuf>, children: &HashMap<PathBuf, Vec<Entry>>) {
            for entry in list {
                rows.push((depth, entry.clone()));
                if entry.is_dir && expanded.contains(&entry.path) && depth < 12 {
                    if let Some(inner) = children.get(&entry.path) {
                        push(rows, inner, depth + 1, expanded, children);
                    }
                }
            }
        }
        let mut rows = Vec::new();
        let expanded = if self.search.trim().is_empty() { &self.expanded } else { &HashSet::new() };
        push(&mut rows, shown, 0, expanded, &self.children);
        rows
    }

    /// Abre ou fecha uma pasta na própria lista.
    fn toggle_expand(&mut self, path: &Path, open: bool) {
        if open {
            let mut list = files::list_dir(path, self.hidden).unwrap_or_default();
            sort_list(&mut list, self.sort);
            self.children.insert(path.to_path_buf(), list);
            self.expanded.insert(path.to_path_buf());
            tracing::info!("pasta aberta na lista: {}", path.display());
        } else {
            self.expanded.retain(|p| !p.starts_with(path));
        }
    }

    /// Texto do tamanho na lista: arquivos na hora, pastas quando a thread terminar.
    fn size_of(&mut self, entry: &Entry) -> String {
        if !entry.is_dir {
            return files::size_text(entry.size);
        }
        match self.sizes.get(&entry.path) {
            Some(Some(bytes)) => files::size_text(*bytes),
            Some(None) => "—".into(),
            None => {
                if self.sizes_waiting.insert(entry.path.clone()) {
                    let _ = self.to_sizer.send(entry.path.clone());
                }
                "…".into()
            }
        }
    }

    /// Pasta onde criar e colar: a de cima, ou a pasta aberta na lista escolhida.
    fn target_dir(&self) -> Option<PathBuf> {
        match &self.place {
            Place::Dir(dir) => Some(dir.clone()),
            Place::Trash => None,
        }
    }

    fn new_file(&mut self) {
        let Some(dir) = self.target_dir() else {
            return;
        };
        match files::create_file(&dir) {
            Ok(path) => {
                tracing::info!("arquivo criado: {}", path.display());
                self.reload();
                self.selected = Some(path.clone());
                // O nome vem todo selecionado: digitar "notas.md" já define a extensão.
                self.start_rename(path);
            }
            Err(err) => self.message = Some(format!("Não foi possível criar o arquivo: {err}")),
        }
    }

    fn duplicate(&mut self, path: &Path) {
        match files::duplicate(path) {
            Ok(copy) => {
                tracing::info!("duplicado: {}", copy.display());
                self.reload();
                self.selected = Some(copy);
            }
            Err(err) => self.message = Some(format!("Não foi possível duplicar: {err}")),
        }
    }

    fn paste(&mut self) {
        let (Some(item), Some(dir)) = (self.clipboard.clone(), self.target_dir()) else {
            return;
        };
        match files::copy_into(&item, &dir) {
            Ok(copy) => {
                tracing::info!("colado: {}", copy.display());
                self.reload();
                self.selected = Some(copy);
            }
            Err(err) => self.message = Some(format!("Não foi possível colar: {err}")),
        }
    }

    /// Compactar ou extrair numa thread (pastas grandes levam tempo).
    fn run_job(&mut self, path: PathBuf, extract: bool) {
        let tx = self.jobs_tx.clone();
        self.busy += 1;
        self.message = Some(if extract { "Extraindo…".into() } else { "Compactando…".into() });
        std::thread::spawn(move || {
            let result = if extract { files::extract(&path) } else { files::compress(&path) };
            let _ = tx.send(result.map_err(|e| e.to_string()));
        });
    }

    fn go(&mut self, place: Place) {
        if place == self.place {
            return;
        }
        let old = std::mem::replace(&mut self.place, place);
        self.back.push(old);
        self.forward.clear();
        self.after_move();
    }

    fn after_move(&mut self) {
        self.selected = None;
        self.search.clear();
        self.searching = false;
        self.renaming = None;
        self.message = None;
        self.reload();
        match &self.place {
            Place::Dir(dir) => tracing::info!("pasta aberta: {}", dir.display()),
            Place::Trash => tracing::info!("pasta aberta: Lixeira"),
        }
    }

    fn go_back(&mut self) {
        if let Some(place) = self.back.pop() {
            let old = std::mem::replace(&mut self.place, place);
            self.forward.push(old);
            self.after_move();
        }
    }

    fn go_forward(&mut self) {
        if let Some(place) = self.forward.pop() {
            let old = std::mem::replace(&mut self.place, place);
            self.back.push(old);
            self.after_move();
        }
    }

    fn go_up(&mut self) {
        if let Place::Dir(dir) = &self.place {
            if let Some(parent) = dir.parent() {
                let parent = parent.to_path_buf();
                let came_from = dir.clone();
                self.go(Place::Dir(parent));
                self.selected = Some(came_from);
            }
        }
    }

    fn title(&self) -> String {
        match &self.place {
            Place::Trash => "Lixeira".into(),
            Place::Dir(dir) => {
                if let Some((name, _, _)) = self.favorites.iter().find(|(_, p, _)| p == dir) {
                    return name.clone();
                }
                dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "/".into())
            }
        }
    }

    /// Itens mostrados agora (com a busca aplicada).
    fn shown(&self) -> Vec<Entry> {
        files::filter(&self.entries, &self.search)
    }

    fn open(&mut self, entry: &Entry) {
        if self.place == Place::Trash {
            // Na Lixeira nada abre: primeiro restaura.
            self.message = Some(format!("\"{}\" está na Lixeira. Restaure para abrir.", entry.name));
            return;
        }
        if entry.is_dir {
            self.go(Place::Dir(entry.path.clone()));
        } else {
            places::open(&entry.path.to_string_lossy());
        }
    }

    fn move_to_trash(&mut self, path: &Path) {
        match places::move_to_trash(path, &Self::trash()) {
            Ok(_) => tracing::info!("para a Lixeira: {}", path.display()),
            Err(err) => self.message = Some(format!("Não foi possível mover para a Lixeira: {err}")),
        }
        self.reload();
    }

    fn new_folder(&mut self) {
        let Place::Dir(dir) = &self.place else {
            return;
        };
        match files::create_folder(dir) {
            Ok(path) => {
                tracing::info!("pasta criada: {}", path.display());
                self.reload();
                self.selected = Some(path.clone());
                // Como no Finder: a pasta nova já entra com o nome para trocar.
                self.start_rename(path);
            }
            Err(err) => self.message = Some(format!("Não foi possível criar a pasta: {err}")),
        }
    }

    fn start_rename(&mut self, path: PathBuf) {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.renaming = Some((path, name));
        self.focus_rename = true;
    }

    fn finish_rename(&mut self) {
        let Some((path, name)) = self.renaming.take() else {
            return;
        };
        match files::rename(&path, &name) {
            Ok(target) => {
                tracing::info!("renomeado: {} -> {}", path.display(), target.display());
                self.reload();
                self.selected = Some(target);
                self.message = None;
            }
            Err(err) => {
                self.message = Some(format!("Não foi possível renomear: {err}"));
                self.renaming = Some((path, name));
            }
        }
    }

    fn restore(&mut self, entry: &Entry) {
        match places::restore_from_trash(&Self::trash(), &entry.name) {
            Ok(to) => tracing::info!("restaurado: {}", to.display()),
            Err(err) => self.message = Some(format!("Não foi possível restaurar: {err}")),
        }
        self.reload();
    }

    fn delete_forever(&mut self, entry: &Entry) {
        match places::delete_from_trash(&Self::trash(), &entry.name) {
            Ok(()) => tracing::info!("apagado da Lixeira: {}", entry.name),
            Err(err) => self.message = Some(format!("Não foi possível apagar: {err}")),
        }
        self.reload();
    }

    /// Miniatura de uma imagem, pedindo o carregamento na primeira vez.
    fn thumb(&mut self, path: &Path) -> Option<&Pixmap> {
        if !self.thumbs.contains_key(path) && self.waiting.insert(path.to_path_buf()) {
            let _ = self.to_loader.send(path.to_path_buf());
        }
        self.thumbs.get(path).and_then(Option::as_ref)
    }

    /// Teclas da janela (quando não há um campo de texto com o foco).
    fn keys(&mut self, ui: &mut Ui, shown: &[Entry], columns: usize, list: bool) {
        let input = ui.input();
        let (enter, delete, backspace) = (input.key(Key::Enter), input.key(Key::Delete), input.key(Key::Backspace));
        // Na lista, ← e → fecham e abrem a pasta escolhida (como no Finder).
        let (left, right) = (input.key(Key::Left), input.key(Key::Right));
        let moves = if list {
            vec![(Key::Up, -1isize), (Key::Down, 1)]
        } else {
            vec![(Key::Left, -1isize), (Key::Right, 1), (Key::Up, -(columns as isize)), (Key::Down, columns as isize)]
        };
        let step: isize = moves.iter().filter(|(k, _)| input.key(*k)).map(|(_, d)| *d).sum();
        let (new, hidden, find, rename) = (input.shortcut("n"), input.shortcut("h"), input.shortcut("f"), input.shortcut("r"));
        let (copy, paste, dup) = (input.shortcut("c"), input.shortcut("v"), input.shortcut("d"));
        if hidden {
            self.hidden = !self.hidden;
            self.reload();
            ui.request_repaint();
        }
        if find {
            self.searching = true;
            ui.focus_next_input();
        }
        if new {
            self.new_folder();
            ui.request_repaint();
        }
        if ui.typing() || self.renaming.is_some() {
            return;
        }
        let current = self.selected.as_ref().and_then(|s| shown.iter().position(|e| &e.path == s));
        if step != 0 && !shown.is_empty() {
            let next = match current {
                Some(i) => (i as isize + step).clamp(0, shown.len() as isize - 1) as usize,
                None => 0,
            };
            self.selected = Some(shown[next].path.clone());
            ui.request_repaint();
        }
        let chosen = current.map(|i| shown[i].clone());
        if list && (left || right) {
            if let Some(entry) = chosen.as_ref().filter(|e| e.is_dir) {
                self.toggle_expand(&entry.path, right);
                ui.request_repaint();
            }
        }
        if self.place != Place::Trash {
            if copy {
                if let Some(entry) = &chosen {
                    self.clipboard = Some(entry.path.clone());
                    tracing::info!("copiado: {}", entry.path.display());
                }
            }
            if paste {
                self.paste();
                ui.request_repaint();
            }
            if dup {
                if let Some(entry) = &chosen {
                    self.duplicate(&entry.path);
                    ui.request_repaint();
                }
            }
        }
        if enter {
            if let Some(entry) = &chosen {
                self.open(entry);
                ui.request_repaint();
            }
        }
        if delete {
            if let Some(entry) = &chosen {
                if self.place == Place::Trash {
                    self.delete_forever(entry);
                } else {
                    self.move_to_trash(&entry.path);
                }
                ui.request_repaint();
            }
        }
        if rename && self.place != Place::Trash {
            if let Some(entry) = chosen {
                self.start_rename(entry.path);
                ui.request_repaint();
            }
        }
        if backspace {
            self.go_up();
            ui.request_repaint();
        }
    }

    fn toolbar(&mut self, ui: &mut Ui) {
        let mut back = false;
        let mut forward = false;
        let mut toggle = None;
        let can_back = !self.back.is_empty();
        let can_forward = !self.forward.is_empty();
        let title = self.title();
        let list_view = self.list_view;
        let in_trash = self.place == Place::Trash;
        let has_items = !self.entries.is_empty();
        let mut empty_trash = false;
        let mut search = std::mem::take(&mut self.search);
        ui.toolbar(|ui| {
            back = ui.icon_button("go-previous", "‹", can_back).clicked();
            forward = ui.icon_button("go-next", "›", can_forward).clicked();
            ui.space(4.0);
            ui.strong(&title);
            let right = if in_trash { 110.0 } else { 2.0 * 32.0 + 220.0 + 2.0 * 8.0 };
            ui.push_right(right);
            if in_trash {
                ui.sized(110.0, |ui| empty_trash = has_items && ui.button("Esvaziar").clicked());
            } else {
                if ui.icon_toggle("view-grid-symbolic", "▦", !list_view).clicked() {
                    toggle = Some(false);
                }
                let list = ui.icon_toggle("view-list-symbolic", "☰", list_view);
                if list.clicked() {
                    toggle = Some(true);
                }
                ui.sized(220.0, |ui| {
                    ui.text_input(&mut search, "Buscar");
                });
            }
        });
        if search != self.search {
            self.selected = None;
        }
        self.search = search;
        if back {
            self.go_back();
        }
        if forward {
            self.go_forward();
        }
        if let Some(list) = toggle {
            self.list_view = list;
        }
        if empty_trash {
            match places::empty_trash(&Self::trash()) {
                Ok(n) => tracing::info!("Lixeira esvaziada: {n} itens"),
                Err(err) => self.message = Some(format!("Não foi possível esvaziar a Lixeira: {err}")),
            }
            self.reload();
        }
    }

    fn rename_bar(&mut self, ui: &mut Ui) {
        let Some((_, mut name)) = self.renaming.clone() else {
            return;
        };
        let mut done = false;
        let mut cancel = ui.input().key(Key::Escape);
        ui.heading("Renomear");
        ui.card(|ui| {
            if std::mem::take(&mut self.focus_rename) {
                ui.focus_next_input();
            }
            done = ui.text_input(&mut name, "Nome novo").submitted();
            ui.row(|ui| {
                done |= ui.primary_button("Renomear").clicked();
                cancel |= ui.button("Cancelar").clicked();
            });
        });
        if let Some((_, current)) = self.renaming.as_mut() {
            *current = name;
        }
        if cancel {
            self.renaming = None;
            self.message = None;
            ui.request_repaint();
        } else if done {
            self.finish_rename();
            ui.request_repaint();
        }
    }

    fn content(&mut self, ui: &mut Ui) {
        // Tamanhos de pastas e tarefas (compactar, extrair) que terminaram.
        while let Ok((path, size)) = self.from_sizer.try_recv() {
            self.sizes_waiting.remove(&path);
            self.sizes.insert(path, size);
            ui.request_repaint();
        }
        while let Ok(result) = self.jobs_rx.try_recv() {
            self.busy = self.busy.saturating_sub(1);
            match result {
                Ok(path) => {
                    tracing::info!("pronto: {}", path.display());
                    self.message = None;
                    self.reload();
                    self.selected = Some(path);
                }
                Err(err) => self.message = Some(format!("Não deu certo: {err}")),
            }
            ui.request_repaint();
        }
        if self.busy > 0 || !self.sizes_waiting.is_empty() {
            ui.request_tick();
        }
        // Miniaturas que chegaram.
        while let Ok((path, thumb)) = self.from_loader.try_recv() {
            self.waiting.remove(&path);
            self.thumbs.insert(path, thumb);
            ui.request_repaint();
        }
        if !self.waiting.is_empty() {
            ui.request_tick();
        }
        self.toolbar(ui);
        let shown = self.shown();
        let in_list = self.list_view || self.place == Place::Trash;
        let columns = ((ui.available_width() - 2.0 * ui.theme().padding) / (TILE as f32 + 44.0 + ui.theme().spacing)).max(1.0) as usize;
        // Na lista, as teclas andam pelas linhas (inclusive as de dentro das pastas).
        let rows = if in_list { self.rows(&shown) } else { Vec::new() };
        let row_entries: Vec<Entry> = rows.iter().map(|(_, e)| e.clone()).collect();
        self.keys(ui, if in_list { &row_entries } else { &shown }, columns, in_list);
        let mut sizes: HashMap<PathBuf, String> = HashMap::new();
        if in_list {
            for (_, entry) in &rows {
                let text = self.size_of(entry);
                sizes.insert(entry.path.clone(), text);
            }
        }
        let mut toggles: Vec<(PathBuf, bool)> = Vec::new();
        let mut sort_by = None;
        let clipboard = self.clipboard.is_some();
        let in_trash = self.place == Place::Trash;
        let now = SystemTime::now();
        let mut action: Option<(Action, Entry)> = None;
        let mut empty_action = None;
        ui.page(|ui| {
            self.rename_bar(ui);
            if let Some(message) = &self.message {
                ui.note(message);
            }
            if shown.is_empty() {
                ui.note(match (in_trash, self.search.is_empty()) {
                    (true, _) => "A Lixeira está vazia.",
                    (_, false) => "Nada encontrado.",
                    _ => "Esta pasta está vazia.",
                });
            }
            let mut select = None;
            let mut item = |ui: &mut Ui, entry: &Entry, r: mide::kit::Response| {
                if r.clicked() || r.secondary_clicked() {
                    select = Some(entry.path.clone());
                }
                if r.double_clicked() {
                    action = Some((Action::Open, entry.clone()));
                }
                ui.context_menu(&r, |m| {
                    let chosen = if in_trash {
                        if m.item("Restaurar") {
                            Some(Action::Restore)
                        } else if m.danger("Apagar para sempre") {
                            Some(Action::Delete)
                        } else {
                            None
                        }
                    } else {
                        let is_zip = entry.path.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip"));
                        let mut chosen = None;
                        if m.item("Abrir") {
                            chosen = Some(Action::Open);
                        }
                        if entry.is_dir && m.item("Abrir no Terminal") {
                            chosen = Some(Action::Terminal);
                        }
                        m.separator();
                        if m.item("Renomear…") {
                            chosen = Some(Action::Rename);
                        }
                        if m.item("Duplicar") {
                            chosen = Some(Action::Duplicate);
                        }
                        if is_zip {
                            if m.item("Extrair aqui") {
                                chosen = Some(Action::Extract);
                            }
                        } else if m.item(&format!("Comprimir \"{}\"", short(&entry.name))) {
                            chosen = Some(Action::Compress);
                        }
                        m.separator();
                        if m.item("Copiar") {
                            chosen = Some(Action::Copy);
                        }
                        if m.item("Copiar caminho") {
                            chosen = Some(Action::CopyPath);
                        }
                        m.separator();
                        if m.danger("Mover para a Lixeira") {
                            chosen = Some(Action::Trash);
                        }
                        chosen
                    };
                    if let Some(chosen) = chosen {
                        action = Some((chosen, entry.clone()));
                    }
                });
            };
            let current = self.selected.clone();
            let thumbs: Vec<Option<Pixmap>> = if self.list_view || in_trash {
                Vec::new()
            } else {
                // A miniatura de cada imagem vem do cache; o que falta é pedido.
                shown.iter().map(|e| if e.is_image() { self.thumb_clone(&e.path) } else { None }).collect()
            };
            if in_list {
                // Lista no estilo do Finder: cabeçalho que ordena e linhas alternadas,
                // com a setinha das pastas.
                let widths = [("Modificado", 165.0), ("Tamanho", 85.0), ("Tipo", 150.0)];
                if !shown.is_empty() {
                    sort_by = ui.list_header("Nome", &widths, self.sort);
                }
                for (i, (depth, entry)) in rows.iter().enumerate() {
                    let selected = current.as_ref() == Some(&entry.path);
                    let date = files::date_text(entry.modified, now);
                    let size = sizes.get(&entry.path).cloned().unwrap_or_default();
                    let kind = files::kind_text(entry);
                    let open = self.expanded.contains(&entry.path);
                    let expandable = entry.is_dir && !in_trash;
                    let cols = [(date.as_str(), widths[0].1), (size.as_str(), widths[1].1), (kind.as_str(), widths[2].1)];
                    let (r, toggled) = ui.tree_row(*depth, expandable, open, entry.icon_name(), &entry.name, &cols, selected, i % 2 == 1);
                    if toggled {
                        toggles.push((entry.path.clone(), !open));
                    } else {
                        item(ui, entry, r);
                    }
                }
            } else {
                ui.wrap(|ui| {
                    for (entry, thumb) in shown.iter().zip(&thumbs) {
                        let selected = current.as_ref() == Some(&entry.path);
                        let image = match thumb {
                            Some(pix) => Tile::Image(pix),
                            None => Tile::Icon(entry.icon_name()),
                        };
                        let r = ui.tile(image, &entry.name, selected, TILE);
                        item(ui, entry, r);
                    }
                });
            }
            if let Some(path) = select {
                self.selected = Some(path);
            }
            // Fundo da pasta: clique no vazio tira a seleção; o botão direito abre o
            // menu da pasta.
            let area = ui.region();
            let bg = ui.background(area);
            if bg.clicked() {
                self.selected = None;
                ui.request_repaint();
            }
            ui.context_menu(&bg, |m| {
                if in_trash {
                    if m.danger("Esvaziar a Lixeira") {
                        empty_action = Some(Action::EmptyTrash);
                    }
                } else {
                    if m.item("Nova pasta") {
                        empty_action = Some(Action::NewFolder);
                    }
                    if m.item("Novo arquivo") {
                        empty_action = Some(Action::NewFile);
                    }
                    if clipboard && m.item("Colar") {
                        empty_action = Some(Action::Paste);
                    }
                    m.separator();
                    if m.item("Abrir no Terminal") {
                        empty_action = Some(Action::Terminal);
                    }
                    if m.item(if self.hidden { "Esconder arquivos ocultos" } else { "Mostrar arquivos ocultos" }) {
                        empty_action = Some(Action::ToggleHidden);
                    }
                    if m.item("Copiar caminho da pasta") {
                        empty_action = Some(Action::CopyFolder);
                    }
                }
            });
        });
        for (path, open) in toggles {
            self.toggle_expand(&path, open);
            ui.request_repaint();
        }
        if let Some(column) = sort_by {
            // Clicar na coluna já ordenada inverte a ordem.
            self.sort = if self.sort.0 == column { (column, !self.sort.1) } else { (column, true) };
            self.sort_entries();
            ui.request_repaint();
        }
        if let Some((action, entry)) = action {
            match action {
                Action::Open => self.open(&entry),
                Action::Rename => self.start_rename(entry.path.clone()),
                Action::CopyPath => places::copy_text(&entry.path.to_string_lossy()),
                Action::Terminal => places::open_terminal(&entry.path),
                Action::Duplicate => self.duplicate(&entry.path),
                Action::Compress => self.run_job(entry.path.clone(), false),
                Action::Extract => self.run_job(entry.path.clone(), true),
                Action::Copy => {
                    tracing::info!("copiado: {}", entry.path.display());
                    self.clipboard = Some(entry.path.clone());
                }
                Action::Trash => self.move_to_trash(&entry.path),
                Action::Restore => self.restore(&entry),
                Action::Delete => self.delete_forever(&entry),
                _ => {}
            }
            ui.request_repaint();
        }
        if let Some(action) = empty_action {
            match action {
                Action::NewFolder => self.new_folder(),
                Action::NewFile => self.new_file(),
                Action::Paste => self.paste(),
                Action::Terminal => {
                    if let Some(dir) = self.target_dir() {
                        places::open_terminal(&dir);
                    }
                }
                Action::ToggleHidden => {
                    self.hidden = !self.hidden;
                    self.reload();
                }
                Action::CopyFolder => {
                    if let Place::Dir(dir) = &self.place {
                        places::copy_text(&dir.to_string_lossy());
                    }
                }
                Action::EmptyTrash => {
                    if let Err(err) = places::empty_trash(&Self::trash()) {
                        self.message = Some(format!("Não foi possível esvaziar a Lixeira: {err}"));
                    }
                    self.reload();
                }
                _ => {}
            }
            ui.request_repaint();
        }
    }

    fn thumb_clone(&mut self, path: &Path) -> Option<Pixmap> {
        self.thumb(path).cloned()
    }
}

#[derive(Clone, Copy)]
enum Action {
    Open,
    Terminal,
    Rename,
    Duplicate,
    Compress,
    Extract,
    Copy,
    Paste,
    NewFile,
    CopyPath,
    Trash,
    Restore,
    Delete,
    NewFolder,
    ToggleHidden,
    CopyFolder,
    EmptyTrash,
}

impl App for Files {
    fn view(&mut self, ui: &mut Ui) {
        ui.set_title(&self.title());
        let mut items: Vec<(String, &str)> = vec![("Favoritos".into(), "")];
        items.extend(self.favorites.iter().map(|(name, _, icon)| (name.clone(), *icon)));
        items.push(("Lugares".into(), ""));
        items.push(("Computador".into(), "drive-harddisk-symbolic"));
        items.push(("Lixeira".into(), "user-trash-symbolic"));
        let favorites = self.favorites.len();
        let current = match &self.place {
            Place::Trash => favorites + 3,
            Place::Dir(dir) if dir == Path::new("/") => favorites + 2,
            Place::Dir(dir) => self.favorites.iter().position(|(_, p, _)| p == dir).map(|i| i + 1).unwrap_or(usize::MAX),
        };
        let refs: Vec<(&str, &str)> = items.iter().map(|(n, i)| (n.as_str(), *i)).collect();
        let chosen = ui.sidebar_area(&refs, current, |ui, _| self.content(ui));
        if chosen != current {
            let place = if chosen == favorites + 3 {
                Place::Trash
            } else if chosen == favorites + 2 {
                Place::Dir(PathBuf::from("/"))
            } else {
                Place::Dir(self.favorites[chosen - 1].1.clone())
            };
            self.go(place);
            ui.request_repaint();
        }
    }
}

/// Ordena uma lista pela coluna (0 nome, 1 modificado, 2 tamanho, 3 tipo). Por nome,
/// pastas primeiro (como o Finder); nas outras colunas, tudo junto.
fn sort_list(list: &mut [Entry], (column, ascending): (usize, bool)) {
    list.sort_by(|a, b| {
        let order = match column {
            1 => a.modified.cmp(&b.modified),
            2 => a.size.cmp(&b.size),
            3 => files::kind_text(a).cmp(&files::kind_text(b)),
            _ => b.is_dir.cmp(&a.is_dir).then_with(|| mide::apps::fold(&a.name).cmp(&mide::apps::fold(&b.name))),
        };
        let order = order.then_with(|| mide::apps::fold(&a.name).cmp(&mide::apps::fold(&b.name)));
        if ascending { order } else { order.reverse() }
    });
}

/// Nome curto para os menus ("Comprimir "um nome muito…"").
fn short(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    if chars.len() <= 24 {
        name.to_string()
    } else {
        format!("{}…", chars[..22].iter().collect::<String>())
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "mide_files=info".into()))
        .init();
    let start = match std::env::args().nth(1) {
        Some(arg) if arg.starts_with("trash:") => Place::Trash,
        Some(arg) => {
            let path = PathBuf::from(arg.strip_prefix("file://").unwrap_or(&arg));
            if path.is_dir() {
                Place::Dir(path)
            } else {
                Place::Dir(places::home_dir())
            }
        }
        None => Place::Dir(places::home_dir()),
    };
    let window = Window::new("Arquivos").app_id("mide-files").size(900, 520).min_size(560, 360);
    run(window, Files::new(start))
}
