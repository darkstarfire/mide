//! Arquivos do mide: gerenciador de arquivos no estilo do Finder, feito com o mide-ui.
//!
//! Barra lateral com os Favoritos e a Lixeira; barra de ferramentas com voltar e
//! avançar, o nome da pasta, ícones ou lista e busca; grade de ícones (com miniaturas
//! das imagens) ou lista com tamanho e data. Clique seleciona, duplo clique abre, o
//! botão direito abre o menu de contexto do item ou da pasta.
//!
//! Teclado: Enter abre, Delete manda para a Lixeira, Backspace sobe uma pasta, setas
//! mudam a seleção, Ctrl+N cria uma pasta, Ctrl+H mostra os ocultos, Ctrl+F busca,
//! Ctrl+R renomeia.
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
        let mut app = Self {
            place: start,
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
        if self.selected.as_ref().is_some_and(|s| !self.entries.iter().any(|e| &e.path == s)) {
            self.selected = None;
        }
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
    fn keys(&mut self, ui: &mut Ui, shown: &[Entry], columns: usize) {
        let input = ui.input();
        let (enter, delete, backspace) = (input.key(Key::Enter), input.key(Key::Delete), input.key(Key::Backspace));
        let moves = [(Key::Left, -1isize), (Key::Right, 1), (Key::Up, -(columns as isize)), (Key::Down, columns as isize)];
        let step: isize = moves.iter().filter(|(k, _)| input.key(*k)).map(|(_, d)| *d).sum();
        let (new, hidden, find, rename) = (input.shortcut("n"), input.shortcut("h"), input.shortcut("f"), input.shortcut("r"));
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
        let columns = ((ui.available_width() - 2.0 * ui.theme().padding) / (TILE as f32 + 44.0 + ui.theme().spacing)).max(1.0) as usize;
        self.keys(ui, &shown, if self.list_view || self.place == Place::Trash { 1 } else { columns });
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
                    } else if m.item("Abrir") {
                        Some(Action::Open)
                    } else if m.item("Renomear…") {
                        Some(Action::Rename)
                    } else if m.item("Copiar caminho") {
                        Some(Action::CopyPath)
                    } else {
                        m.separator();
                        m.danger("Mover para a Lixeira").then_some(Action::Trash)
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
            if self.list_view || in_trash {
                for entry in &shown {
                    let selected = current.as_ref() == Some(&entry.path);
                    let size = if entry.is_dir { "—".to_string() } else { files::size_text(entry.size) };
                    let date = files::date_text(entry.modified, now);
                    let r = ui.list_row(entry.icon_name(), &entry.name, &[(&date, 170.0), (&size, 90.0)], selected);
                    item(ui, entry, r);
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
                    if m.item(if self.hidden { "Esconder arquivos ocultos" } else { "Mostrar arquivos ocultos" }) {
                        empty_action = Some(Action::ToggleHidden);
                    }
                    m.separator();
                    if m.item("Copiar caminho da pasta") {
                        empty_action = Some(Action::CopyFolder);
                    }
                }
            });
        });
        if let Some((action, entry)) = action {
            match action {
                Action::Open => self.open(&entry),
                Action::Rename => self.start_rename(entry.path.clone()),
                Action::CopyPath => places::copy_text(&entry.path.to_string_lossy()),
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
    Rename,
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
        items.push(("Computador".into(), "drive-harddisk"));
        items.push(("Lixeira".into(), "user-trash"));
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
