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
//! Programas (E8): um atalho `.desktop` aparece com o nome e o ícone do app e abre o
//! app; AppImages, executáveis e scripts rodam na pasta deles. O que veio da internet
//! ou não está marcado como executável pergunta antes (`mide::programs`); scripts
//! perguntam Executar ou Abrir no editor.
//!
//! Teclado: Enter abre, Delete manda para a Lixeira, Backspace sobe uma pasta, setas
//! mudam a seleção, Ctrl+N cria uma pasta, Ctrl+H mostra os ocultos, Ctrl+F busca,
//! Ctrl+R renomeia, Ctrl+C copia, Ctrl+V cola e Ctrl+D duplica.
//!
//! Uso: `mide-files [pasta]` (ou `mide-files trash:///` para abrir a Lixeira). Um
//! arquivo abre a pasta dele com o arquivo escolhido. `mide-files --select a b ...`
//! abre a pasta dos itens com eles escolhidos e `mide-files --info item` mostra as
//! informações do item (o serviço `org.freedesktop.FileManager1` do `mide-portal`
//! usa as duas: "Mostrar na pasta" dos navegadores). Aceita caminhos e `file://`.

use std::{
    collections::{HashMap, HashSet},
    error::Error,
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    sync::mpsc::{channel, Receiver, Sender},
    time::SystemTime,
};

use mide::{
    files::{self, Entry},
    files_view::{self, ViewMemory},
    kit::{run, App, Key, Tile, Ui, Window},
    places,
    programs::{self, Answer, Labels, Opened, Prompt},
};
use tiny_skia::Pixmap;


#[derive(Debug, Clone, PartialEq)]
enum Place {
    Dir(PathBuf),
    Trash,
}

struct Files {
    place: Place,
    back: Vec<Place>,
    forward: Vec<Place>,
    entries: Vec<Entry>,
    /// Itens escolhidos (Ctrl+clique e Shift+clique escolhem vários); o último é o
    /// principal (teclas, Shift+clique a partir dele).
    selection: Vec<PathBuf>,
    list_view: bool,
    /// Colunas (como no Finder): cada pasta escolhida abre a próxima coluna; vale mais
    /// que `list_view`.
    column_view: bool,
    /// Lado dos ícones da grade (zoom: Ctrl+roda, Ctrl +/−, controle na barra de status).
    icon_size: u32,
    /// Painel de prévia à direita (Finder: Visualizar → Mostrar Pré-visualização).
    preview_pane: bool,
    /// Visualização Rápida (Espaço): o item mostrado grande por cima da janela.
    quick_look: Option<PathBuf>,
    /// Seleção por retângulo: onde o botão foi apertado no vazio, e depois de andar, o
    /// retângulo (início, ponta) com a seleção de antes (Ctrl/Shift somam a ela).
    lasso_press: Option<(f32, f32)>,
    lasso: Option<((f32, f32), (f32, f32), Vec<PathBuf>)>,
    /// Colunas: o que há em cada pasta aberta nelas (relido ao recarregar).
    col_cache: HashMap<PathBuf, Vec<Entry>>,
    /// Painel de prévia: as informações do item mostrado (lidas uma vez por item).
    preview_info: Option<(PathBuf, Vec<(&'static str, String)>)>,
    /// Espaço livre no disco da pasta atual (barra de status).
    free_space: Option<u64>,
    /// O zoom mudou pelo controle da barra de status: grava quando soltar.
    zoom_dirty: bool,
    hidden: bool,
    search: String,
    searching: bool,
    /// Diálogo aberto (nova pasta, novo arquivo ou renomear).
    dialog: Option<Dialog>,
    /// O campo do nome ainda precisa receber o foco.
    focus_rename: bool,
    message: Option<String>,
    favorites: Vec<(String, PathBuf, &'static str)>,
    /// Miniaturas por (arquivo, tamanho): imagens, folhas de texto e as dos geradores
    /// do sistema (`mide::thumbnails`).
    thumbs: HashMap<(PathBuf, u32), Option<Pixmap>>,
    waiting: HashSet<(PathBuf, u32)>,
    to_loader: Sender<(PathBuf, u32)>,
    from_loader: Receiver<((PathBuf, u32), Option<Pixmap>)>,
    /// Lista: pastas abertas na própria lista e o que há dentro delas.
    expanded: HashSet<PathBuf>,
    children: HashMap<PathBuf, Vec<Entry>>,
    /// Transições (`[effects] transitions`, "Transições suaves"): pasta que acabou de abrir na lista e
    /// troca entre ícones e lista, com o momento em que começaram.
    expanding: Option<(PathBuf, std::time::Instant, bool)>,
    view_switched: Option<std::time::Instant>,
    /// Coluna ordenada (0 nome, 1 modificado, 2 tamanho, 3 tipo) e se é crescente.
    sort: (usize, bool),
    /// Tamanho das pastas (calculado numa thread; `None` = grande demais).
    sizes: HashMap<PathBuf, Option<u64>>,
    sizes_waiting: HashSet<PathBuf>,
    to_sizer: Sender<PathBuf>,
    from_sizer: Receiver<(PathBuf, Option<u64>)>,
    /// Itens copiados (Copiar / Ctrl+C), para Colar em outra pasta.
    clipboard: Vec<PathBuf>,
    /// Apps instalados (para "Abrir com"), carregados numa thread ao abrir.
    apps: Vec<mide::apps::App>,
    apps_rx: Option<Receiver<Vec<mide::apps::App>>>,
    /// Arrastar itens: onde o botão foi apertado (e em qual item); depois de andar um
    /// pouco, vira um arrasto com os itens escolhidos.
    press: Option<(PathBuf, (f32, f32))>,
    drag: Option<Vec<PathBuf>>,
    /// Arrasto que saiu da janela (para outro app, um site ou de volta para cá): os
    /// itens, até o compositor avisar que acabou.
    drag_out: Option<Vec<PathBuf>>,
    /// Arquivo do "Abrir com…" (a lista dos apps aparece por cima).
    open_with: Option<PathBuf>,
    /// Compactar e extrair rodam numa thread: o resultado chega aqui.
    jobs_tx: Sender<Result<PathBuf, String>>,
    jobs_rx: Receiver<Result<PathBuf, String>>,
    busy: usize,
    /// Item que a lista ou a grade deve rolar até mostrar (aberto com `--select`, ou
    /// escolhido pelo teclado).
    reveal: Option<PathBuf>,
    /// "Obter informações": o item e as linhas (rótulo, valor).
    info: Option<(PathBuf, Vec<(&'static str, String)>)>,
    /// Confirmação antes de rodar um programa (E8): vindo da internet, sem permissão
    /// de execução, ou script (Executar ou Abrir no editor).
    prompt: Option<Prompt>,
    /// Nome e ícone dos atalhos `.desktop`, mostrados no lugar do nome do arquivo.
    labels: Labels,
}

impl Files {
    fn new(start: Place) -> Self {
        // Miniaturas carregam numa linha de execução à parte, para a janela não travar
        // numa pasta cheia de fotos grandes.
        // Duas linhas de execução: um gerador lento (vídeo, PDF) não segura as fotos.
        let (to_loader, jobs) = channel::<(PathBuf, u32)>();
        let (done, from_loader) = channel();
        let jobs = std::sync::Arc::new(std::sync::Mutex::new(jobs));
        for _ in 0..2 {
            let (jobs, done) = (jobs.clone(), done.clone());
            std::thread::spawn(move || loop {
                let job = jobs.lock().ok().and_then(|rx| rx.recv().ok());
                let Some((path, size)) = job else {
                    break;
                };
                let thumb = mide::thumbnails::make(&path, size);
                if done.send(((path, size), thumb)).is_err() {
                    break;
                }
            });
        }
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
            expanding: None,
            view_switched: None,
            children: HashMap::new(),
            sort: (0, true),
            sizes: HashMap::new(),
            sizes_waiting: HashSet::new(),
            to_sizer,
            from_sizer,
            clipboard: Vec::new(),
            apps: Vec::new(),
            apps_rx: Some({
                let (tx, rx) = channel();
                std::thread::spawn(move || {
                    let _ = tx.send(mide::apps::load_handlers());
                });
                rx
            }),
            press: None,
            drag: None,
            drag_out: None,
            open_with: None,
            jobs_tx,
            jobs_rx,
            busy: 0,
            reveal: None,
            info: None,
            prompt: None,
            labels: Labels::default(),
            back: Vec::new(),
            forward: Vec::new(),
            entries: Vec::new(),
            selection: Vec::new(),
            list_view: false,
            column_view: false,
            icon_size: files_view::ICON_DEFAULT,
            preview_pane: false,
            quick_look: None,
            lasso_press: None,
            lasso: None,
            col_cache: HashMap::new(),
            preview_info: None,
            free_space: None,
            zoom_dirty: false,
            hidden: false,
            search: String::new(),
            searching: false,
            dialog: None,
            focus_rename: false,
            message: None,
            favorites: files::favorites(),
            thumbs: HashMap::new(),
            waiting: HashSet::new(),
            to_loader,
            from_loader,
        };
        app.apply_saved_view();
        app.reload();
        app
    }

    /// Chave da pasta atual na memória das visualizações.
    fn view_key(&self) -> String {
        match &self.place {
            Place::Dir(dir) => files_view::key_for_path(dir),
            Place::Trash => files_view::TRASH_KEY.to_string(),
        }
    }

    /// Visualização lembrada da pasta atual (B21): a dela ou o padrão (a última
    /// escolha). Lê o disco a cada pasta: outra janela pode ter mudado algo.
    fn apply_saved_view(&mut self) {
        let key = self.view_key();
        let view = ViewMemory::load().view_for(&key);
        if self.place != Place::Trash {
            self.list_view = view.list;
            self.column_view = view.columns;
        }
        self.icon_size = view.icon_size;
        self.sort = (view.sort_column, view.ascending);
        self.view_switched = None;
        tracing::info!("visualização de {key}: {}", view_text(self.list_view || self.place == Place::Trash, self.sort));
    }

    /// Guarda a visualização da pasta atual (e, fora da Lixeira, como padrão).
    fn save_view(&self) {
        let key = self.view_key();
        let in_trash = self.place == Place::Trash;
        let view = files_view::View {
            list: self.list_view || in_trash,
            sort_column: self.sort.0,
            ascending: self.sort.1,
            columns: self.column_view && !in_trash,
            icon_size: self.icon_size,
        };
        match ViewMemory::record(&key, view, !in_trash) {
            Ok(_) => tracing::info!("visualização guardada para {key}: {}", view_text(view.list, self.sort)),
            Err(err) => tracing::warn!("não foi possível guardar a visualização: {err}"),
        }
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
        self.col_cache.clear();
        self.preview_info = None;
        self.free_space = rustix::fs::statvfs(&dir).ok().map(|v| v.f_bavail.saturating_mul(v.f_frsize));
        self.selection.retain(|s| s.exists());
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
            if mide::settings_file::effect("transitions") {
                self.expanding = Some((path.to_path_buf(), std::time::Instant::now(), true));
            }
            tracing::info!("pasta aberta na lista: {}", path.display());
        } else if mide::settings_file::effect("transitions") {
            // Fecha com a animação: as linhas de dentro saem de baixo para cima e a
            // pasta fecha de verdade no fim (`content`).
            self.expanding = Some((path.to_path_buf(), std::time::Instant::now(), false));
            tracing::info!("pasta fechada na lista: {}", path.display());
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

    /// Novo arquivo: diálogo com "Novo arquivo.txt" todo selecionado (digitar
    /// "notas.md" já define a extensão).
    fn new_file(&mut self) {
        if let Some(dir) = self.target_dir() {
            self.open_dialog(DialogKind::NewFile(dir), "Novo arquivo.txt".into());
        }
    }

    fn duplicate(&mut self, path: &Path) {
        match files::duplicate(path) {
            Ok(copy) => {
                tracing::info!("duplicado: {}", copy.display());
                self.reload();
                self.select_one(copy);
            }
            Err(err) => self.message = Some(format!("Não foi possível duplicar: {err}")),
        }
    }

    fn paste(&mut self) {
        let Some(dir) = self.target_dir() else {
            return;
        };
        let mut pasted = Vec::new();
        for item in self.clipboard.clone() {
            match files::copy_into(&item, &dir) {
                Ok(copy) => {
                    tracing::info!("colado: {}", copy.display());
                    pasted.push(copy);
                }
                Err(err) => self.message = Some(format!("Não foi possível colar \"{}\": {err}", item.display())),
            }
        }
        self.reload();
        if !pasted.is_empty() {
            self.selection = pasted;
        }
    }

    /// O item principal da seleção (o último escolhido).
    fn primary(&self) -> Option<&PathBuf> {
        self.selection.last()
    }

    fn select_one(&mut self, path: PathBuf) {
        self.selection = vec![path];
    }

    /// Itens de uma ação feita sobre `entry`: a seleção inteira, se ele faz parte dela;
    /// senão, só ele.
    fn targets(&self, entry: &Entry) -> Vec<PathBuf> {
        if self.selection.len() > 1 && self.selection.contains(&entry.path) {
            self.selection.clone()
        } else {
            vec![entry.path.clone()]
        }
    }

    /// Clique num item com Ctrl (junta ou tira da seleção), Shift (do principal até ele,
    /// na ordem da tela) ou sem nada (só ele).
    fn click_select(&mut self, path: PathBuf, ctrl: bool, shift: bool, order: &[PathBuf]) {
        if ctrl {
            if let Some(i) = self.selection.iter().position(|p| *p == path) {
                self.selection.remove(i);
            } else {
                self.selection.push(path);
            }
        } else if shift {
            let from = self.primary().and_then(|p| order.iter().position(|o| o == p));
            let to = order.iter().position(|o| *o == path);
            match (from, to) {
                (Some(a), Some(b)) if a <= b => self.selection = order[a..=b].to_vec(),
                (Some(a), Some(b)) => self.selection = order[b..=a].iter().rev().cloned().collect(),
                _ => self.select_one(path),
            }
        } else {
            self.select_one(path);
        }
    }

    /// Move (ou copia, com Ctrl) os itens arrastados para a pasta.
    fn drop_into(&mut self, paths: &[PathBuf], dir: &Path, copy: bool) {
        let mut done = 0;
        for path in paths {
            let result = if copy { files::copy_into(path, dir) } else { files::move_into(path, dir) };
            match result {
                Ok(target) => {
                    tracing::info!("{}: {} → {}", if copy { "copiado" } else { "movido" }, path.display(), target.display());
                    done += 1;
                }
                Err(err) => self.message = Some(format!("Não foi possível {} \"{}\": {err}", if copy { "copiar" } else { "mover" }, path.display())),
            }
        }
        if done > 0 {
            let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| dir.display().to_string());
            let what = if done == 1 { "1 item".to_string() } else { format!("{done} itens") };
            self.message = Some(format!("{what} {} para \"{name}\".", if copy { "copiado(s)" } else { "movido(s)" }));
        }
        self.reload();
    }

    /// Compactar vários itens num .zip só (numa thread).
    fn compress_job(&mut self, paths: Vec<PathBuf>) {
        let tx = self.jobs_tx.clone();
        self.busy += 1;
        self.message = Some("Compactando…".into());
        std::thread::spawn(move || {
            let _ = tx.send(files::compress_many(&paths).map_err(|e| e.to_string()));
        });
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
        self.selection.clear();
        self.search.clear();
        self.searching = false;
        self.dialog = None;
        self.message = None;
        self.apply_saved_view();
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
                self.select_one(came_from);
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
            self.open_file(&entry.path);
        }
    }

    /// Abre um arquivo: documentos no app padrão; programas (.desktop, AppImage,
    /// executáveis, scripts) rodam ou pedem confirmação antes (E8).
    fn open_file(&mut self, path: &Path) {
        match programs::open(path) {
            Opened::Done => {}
            Opened::Ask(prompt) => {
                // Um diálogo de cada vez: abrindo vários, só o primeiro pergunta.
                if self.prompt.is_none() {
                    self.prompt = Some(prompt);
                }
            }
            Opened::Failed(err) => self.message = Some(err),
        }
    }

    /// Diálogo de confirmação de um programa, por cima de tudo.
    fn prompt_view(&mut self, ui: &mut Ui) {
        let Some(prompt) = self.prompt.as_mut() else {
            return;
        };
        let Some(answer) = prompt.view(ui) else {
            return;
        };
        self.prompt = None;
        match answer {
            Answer::Cancelled => {}
            Answer::Done => {
                self.message = None;
                // Um atalho confiado agora passa a mostrar o nome do app.
                self.labels.clear();
            }
            Answer::Failed(err) => self.message = Some(err),
        }
    }

    /// Nome e ícone mostrados para um item: os do app num atalho `.desktop`.
    fn label(&mut self, entry: &Entry) -> (String, String) {
        self.labels.get(&entry.path, entry.modified).unwrap_or_else(|| (entry.name.clone(), entry.icon_name().to_string()))
    }

    fn move_to_trash(&mut self, path: &Path) {
        match places::move_to_trash(path, &Self::trash()) {
            Ok(_) => tracing::info!("para a Lixeira: {}", path.display()),
            Err(err) => self.message = Some(format!("Não foi possível mover para a Lixeira: {err}")),
        }
        self.reload();
    }

    /// Nova pasta: abre o diálogo com o nome sugerido (nada é criado ainda).
    fn new_folder(&mut self) {
        if let Some(dir) = self.target_dir() {
            self.open_dialog(DialogKind::NewFolder(dir), "Nova pasta".into());
        }
    }

    fn start_rename(&mut self, path: PathBuf) {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.open_dialog(DialogKind::Rename(path), name);
    }

    fn open_dialog(&mut self, kind: DialogKind, name: String) {
        self.dialog = Some(Dialog { kind, suggestion: name.clone(), name, error: None });
        self.focus_rename = true;
    }

    /// "Criar" ou "Renomear" no diálogo. Para criar: sem mudar o nome sugerido, usa o
    /// próximo livre ("Nova pasta", "Nova pasta (1)"...); com outro nome, exatamente
    /// ele (sem sobrescrever nada). Com erro, o diálogo continua aberto e mostra.
    fn confirm_dialog(&mut self) {
        let Some(dialog) = self.dialog.as_ref() else {
            return;
        };
        let result = match &dialog.kind {
            DialogKind::Rename(path) => files::rename(path, &dialog.name),
            DialogKind::NewFolder(dir) | DialogKind::NewFile(dir) => {
                let dir = dir.clone();
                let folder = matches!(dialog.kind, DialogKind::NewFolder(_));
                let name = if dialog.name.trim() == dialog.suggestion {
                    if folder { files::numbered_name(&dir, "Nova pasta", "") } else { files::numbered_name(&dir, "Novo arquivo", "txt") }
                } else {
                    dialog.name.trim().to_string()
                };
                files::create_named(&dir, &name, folder)
            }
        };
        match result {
            Ok(path) => {
                tracing::info!("pronto: {}", path.display());
                self.dialog = None;
                // Criado dentro de uma pasta da lista: ela abre para mostrar o item novo.
                if let Some(parent) = path.parent().filter(|p| Some(p.to_path_buf()) != self.target_dir()) {
                    self.expanded.insert(parent.to_path_buf());
                }
                self.reload();
                self.select_one(path);
                self.message = None;
            }
            Err(err) => {
                if let Some(d) = self.dialog.as_mut() {
                    d.error = Some(err);
                }
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

    /// Miniatura de um arquivo no tamanho `size`, pedindo o carregamento na primeira
    /// vez (enquanto não chega, o ícone do tipo fica no lugar).
    /// Imagem que segue o ponteiro num arrasto para fora: a miniatura (ou o ícone) do
    /// primeiro item e, com vários, a contagem num selo.
    fn drag_icon(&mut self, paths: &[PathBuf]) -> Option<Pixmap> {
        const SIZE: u32 = 72;
        let first = paths.first()?;
        let entry = self.entries.iter().find(|e| &e.path == first).cloned();
        let base = self
            .thumbs
            .iter()
            .find(|((p, _), img)| p == first && img.is_some())
            .and_then(|(_, img)| img.clone())
            .or_else(|| {
                let name = entry.as_ref().map(|e| e.icon_name()).unwrap_or(if first.is_dir() { "folder" } else { "text-x-generic" });
                mide::icons::load_icon(name, 64)
            })?;
        let mut canvas = Pixmap::new(SIZE, SIZE)?;
        let scale = (60.0 / base.width().max(base.height()) as f32).min(1.0);
        let (w, h) = (base.width() as f32 * scale, base.height() as f32 * scale);
        let paint = tiny_skia::PixmapPaint { opacity: 0.85, quality: tiny_skia::FilterQuality::Bilinear, ..Default::default() };
        let tf = tiny_skia::Transform::from_scale(scale, scale).post_translate((SIZE as f32 - w) / 2.0, (SIZE as f32 - h) / 2.0);
        canvas.draw_pixmap(0, 0, base.as_ref(), &paint, tf, None);
        if paths.len() > 1 {
            let label = paths.len().to_string();
            let mut text = mide::ui::text::Text::new();
            let tw = text.measure(&label, 12.0, mide::ui::text::FontWeight::Semibold).max(8.0) + 12.0;
            let x = SIZE as f32 - tw - 1.0;
            mide::ui::draw::fill_rounded(&mut canvas, x, 1.0, tw, 20.0, 10.0, tiny_skia::Color::from_rgba8(255, 59, 48, 255));
            text.draw(&mut canvas, &label, x + 6.0, 3.5, 12.0, mide::ui::text::FontWeight::Semibold, [255, 255, 255, 255]);
        }
        Some(canvas)
    }

    fn thumb(&mut self, path: &Path, size: u32) -> Option<&Pixmap> {
        let key = (path.to_path_buf(), size);
        if !self.thumbs.contains_key(&key) && self.waiting.insert(key.clone()) {
            let _ = self.to_loader.send(key.clone());
        }
        // Cache limitado: zoom de um lado para o outro numa pasta grande.
        if self.thumbs.len() > 3000 {
            let size_now = size;
            self.thumbs.retain(|(_, s), _| *s == size_now);
        }
        self.thumbs.get(&key).and_then(Option::as_ref)
    }

    /// Quem ganha miniatura: arquivos comuns, menos os atalhos `.desktop` (mostram o
    /// ícone do app).
    fn wants_thumb(entry: &Entry) -> bool {
        !entry.is_dir && !entry.path.extension().is_some_and(|e| e.eq_ignore_ascii_case("desktop"))
    }

    /// Muda o tamanho dos ícones (zoom), em passos de 16 pontos.
    fn zoom(&mut self, steps: i32) {
        let size = (self.icon_size as i32 + 16 * steps).clamp(files_view::ICON_MIN as i32, files_view::ICON_MAX as i32) as u32;
        if size != self.icon_size {
            self.icon_size = size;
            tracing::info!("ícones com {size} pontos");
            self.save_view();
        }
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
        let (copy, paste, dup, select_all) = (input.shortcut("c"), input.shortcut("v"), input.shortcut("d"), input.shortcut("a"));
        let info = input.shortcut("i");
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
        if ui.typing() || self.dialog.is_some() || self.open_with.is_some() || self.info.is_some() || self.prompt.is_some() {
            return;
        }
        // Espaço: Visualização Rápida (abre e fecha). Ctrl +/−/0: zoom dos ícones.
        let input = ui.input();
        let space = input.text == " ";
        let (zoom_in, zoom_out, zoom_reset) = (input.shortcut("=") || input.shortcut("+"), input.shortcut("-"), input.shortcut("0"));
        if space {
            self.quick_look = if self.quick_look.is_some() { None } else { self.primary().cloned() };
            ui.request_repaint();
        }
        if zoom_in || zoom_out {
            self.zoom(if zoom_in { 1 } else { -1 });
            ui.request_repaint();
        }
        if zoom_reset && self.icon_size != files_view::ICON_DEFAULT {
            self.icon_size = files_view::ICON_DEFAULT;
            self.save_view();
            ui.request_repaint();
        }
        if info {
            if let Some(path) = self.primary().cloned() {
                self.show_info(&path);
                ui.request_repaint();
            }
        }
        let current = self.primary().and_then(|s| shown.iter().position(|e| &e.path == s));
        if step != 0 && !shown.is_empty() {
            let next = match current {
                Some(i) => (i as isize + step).clamp(0, shown.len() as isize - 1) as usize,
                None => 0,
            };
            self.select_one(shown[next].path.clone());
            self.reveal = Some(shown[next].path.clone());
            ui.request_repaint();
        }
        // Ctrl+A: tudo o que está à mostra.
        if select_all && !shown.is_empty() {
            self.selection = shown.iter().map(|e| e.path.clone()).collect();
            ui.request_repaint();
        }
        let chosen = current.map(|i| shown[i].clone());
        if list && self.mode() == 1 && (left || right) {
            if let Some(entry) = chosen.as_ref().filter(|e| e.is_dir) {
                self.toggle_expand(&entry.path, right);
                ui.request_repaint();
            }
        }
        if self.place != Place::Trash {
            if copy && !self.selection.is_empty() {
                self.clipboard = self.selection.clone();
                tracing::info!("copiado(s): {} item(ns)", self.clipboard.len());
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
                    for path in self.targets(entry) {
                        self.move_to_trash(&path);
                    }
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

    /// Colunas: ← volta para a pasta de cima (na coluna anterior) e → entra na pasta
    /// escolhida (o primeiro item dela).
    fn column_keys(&mut self, ui: &mut Ui, dirs: &[PathBuf]) {
        let (left, right) = (ui.input().key(Key::Left), ui.input().key(Key::Right));
        let Some(primary) = self.primary().cloned() else {
            return;
        };
        if left {
            if let Some(parent) = primary.parent().filter(|p| dirs.first().is_some_and(|base| p != base && p.starts_with(base))) {
                self.select_one(parent.to_path_buf());
                ui.request_repaint();
            }
        }
        if right && primary.is_dir() {
            if let Some(first) = self.column_list(&primary).first() {
                self.select_one(first.path.clone());
                ui.request_repaint();
            }
        }
    }

    /// Barra de ferramentas no estilo do Finder: voltar e avançar, o nome da pasta e,
    /// à direita, as visualizações (ícones, lista, colunas), o painel de prévia, nova
    /// pasta e a busca.
    fn toolbar(&mut self, ui: &mut Ui) {
        let mut back = false;
        let mut forward = false;
        let mut mode = None;
        let mut preview = false;
        let mut new_folder = false;
        let can_back = !self.back.is_empty();
        let can_forward = !self.forward.is_empty();
        let title = self.title();
        let current = self.mode();
        let in_trash = self.place == Place::Trash;
        let has_items = !self.entries.is_empty();
        let preview_on = self.preview_pane;
        let mut empty_trash = false;
        let mut search = std::mem::take(&mut self.search);
        ui.toolbar(|ui| {
            back = ui.icon_button("go-previous-symbolic", "‹", can_back).clicked();
            forward = ui.icon_button("go-next-symbolic", "›", can_forward).clicked();
            ui.space(4.0);
            ui.strong(&title);
            // Cada botão ocupa 32 pontos mais o espaçamento da barra.
            let step = 32.0 + ui.theme().spacing;
            let right = if in_trash { step + 110.0 } else { 5.0 * step + 200.0 };
            ui.push_right(right);
            if in_trash {
                preview = ui.icon_toggle("view-dual-symbolic", "◨", preview_on).clicked();
                ui.sized(110.0, |ui| empty_trash = has_items && ui.button("Esvaziar").clicked());
            } else {
                for (i, (icon, fallback)) in [("view-grid-symbolic", "▦"), ("view-list-symbolic", "☰"), ("view-column-symbolic", "▥")].into_iter().enumerate() {
                    if ui.icon_toggle(icon, fallback, current == i).clicked() {
                        mode = Some(i);
                    }
                }
                preview = ui.icon_toggle("view-dual-symbolic", "◨", preview_on).clicked();
                new_folder = ui.icon_button("folder-new-symbolic", "+", true).clicked();
                ui.sized(200.0, |ui| {
                    ui.text_input(&mut search, "Buscar");
                });
            }
        });
        if search != self.search {
            self.selection.clear();
        }
        self.search = search;
        if back {
            self.go_back();
        }
        if forward {
            self.go_forward();
        }
        if preview {
            self.preview_pane = !self.preview_pane;
        }
        if new_folder {
            self.new_folder();
        }
        if let Some(mode) = mode {
            self.set_mode(mode);
        }
        if empty_trash {
            match places::empty_trash(&Self::trash()) {
                Ok(n) => tracing::info!("Lixeira esvaziada: {n} itens"),
                Err(err) => self.message = Some(format!("Não foi possível esvaziar a Lixeira: {err}")),
            }
            self.reload();
        }
    }

    /// Visualização atual: 0 ícones, 1 lista, 2 colunas (a Lixeira é sempre lista).
    fn mode(&self) -> usize {
        if self.place == Place::Trash || (self.list_view && !self.column_view) {
            1
        } else if self.column_view {
            2
        } else {
            0
        }
    }

    fn set_mode(&mut self, mode: usize) {
        if mode == self.mode() || self.place == Place::Trash {
            return;
        }
        if mide::settings_file::effect("transitions") {
            self.view_switched = Some(std::time::Instant::now());
        }
        self.list_view = mode == 1;
        self.column_view = mode == 2;
        tracing::info!("visualização: {}", ["ícones", "lista", "colunas"][mode.min(2)]);
        self.save_view();
        // O item escolhido continua à vista na outra visualização.
        self.reveal = self.primary().cloned();
    }

    /// Diálogo (modal, vidro branco no meio da janela): nome, Cancelar e Criar.
    /// Cancelar (ou Esc) não cria nada; Enter confirma.
    /// "Abrir com…": os apps que abrem o tipo do arquivo (o padrão primeiro); clicar abre.
    fn open_with_view(&mut self, ui: &mut Ui) {
        let Some(path) = self.open_with.clone() else {
            return;
        };
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let apps = mide::default_apps::apps_for(&self.apps, &path);
        let default = mide::default_apps::mime_for(&path).and_then(|m| mide::default_apps::current(&m)).map(|d| d.trim_end_matches(".desktop").to_string());
        let mut chosen = None;
        let mut cancel = ui.input().key(Key::Escape);
        ui.modal(420.0, |ui| {
            ui.strong(&format!("Abrir \"{}\" com", short(&name)));
            if apps.is_empty() {
                ui.note(if self.apps_rx.is_some() { "Procurando apps…" } else { "Nenhum app instalado abre este tipo de arquivo." });
            }
            ui.scroll_area((apps.len() as f32 * 42.0).clamp(42.0, 300.0), |ui| {
                for app in &apps {
                    let detail = if default.as_deref() == Some(app.id.as_str()) { "padrão" } else { "" };
                    if ui.list_item(&app.name, detail).clicked() {
                        chosen = Some((*app).clone());
                    }
                }
            });
            ui.row(|ui| {
                ui.push_right(110.0);
                ui.sized(110.0, |ui| cancel |= ui.button("Cancelar").clicked());
            });
        });
        if let Some(app) = chosen {
            tracing::info!("abrir com {}: {}", app.name, path.display());
            if let Err(err) = mide::apps::open_with(&app, &path) {
                self.message = Some(format!("Não foi possível abrir com {}: {err}", app.name));
            }
            self.open_with = None;
            ui.request_repaint();
        } else if cancel {
            self.open_with = None;
            ui.request_repaint();
        }
    }

    /// Abre a janela de informações do item (lida agora do disco).
    fn show_info(&mut self, path: &Path) {
        match files::info_lines(path) {
            Ok(lines) => {
                tracing::info!("informações de {}", path.display());
                self.info = Some((path.to_path_buf(), lines));
            }
            Err(err) => self.message = Some(format!("Não foi possível ler \"{}\": {err}", path.display())),
        }
    }

    /// Informações do item: nome, tipo, tamanho, local, data e permissões.
    fn info_view(&mut self, ui: &mut Ui) {
        let Some((path, lines)) = &self.info else {
            return;
        };
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string());
        let mut close = ui.input().key(Key::Escape) || ui.input().key(Key::Enter);
        ui.modal(440.0, |ui| {
            ui.strong(&format!("Informações de \"{}\"", short(&name)));
            ui.card(|ui| {
                for (label, value) in lines {
                    ui.row(|ui| {
                        ui.sized(110.0, |ui| ui.note(label));
                        ui.label(value);
                    });
                }
            });
            ui.row(|ui| {
                ui.push_right(110.0);
                ui.sized(110.0, |ui| close |= ui.primary_button("OK").clicked());
            });
        });
        if close {
            self.info = None;
            ui.request_repaint();
        }
    }

    fn dialog_view(&mut self, ui: &mut Ui) {
        let Some(dialog) = self.dialog.as_mut() else {
            return;
        };
        let (title, action) = match dialog.kind {
            DialogKind::NewFolder(_) => ("Nova pasta", "Criar"),
            DialogKind::NewFile(_) => ("Novo arquivo", "Criar"),
            DialogKind::Rename(_) => ("Renomear", "Renomear"),
        };
        let focus = std::mem::take(&mut self.focus_rename);
        let mut done = false;
        let mut cancel = ui.input().key(Key::Escape);
        let error = dialog.error.clone();
        let mut name = dialog.name.clone();
        ui.modal(380.0, |ui| {
            ui.strong(title);
            if focus {
                ui.focus_next_input();
            }
            done = ui.text_input(&mut name, "Nome").submitted();
            if let Some(err) = &error {
                ui.note(err);
            }
            ui.row(|ui| {
                ui.push_right(2.0 * 110.0 + 8.0);
                ui.sized(110.0, |ui| cancel |= ui.button("Cancelar").clicked());
                ui.sized(110.0, |ui| done |= ui.primary_button(action).clicked());
            });
        });
        if let Some(d) = self.dialog.as_mut() {
            if d.name != name {
                d.error = None;
            }
            d.name = name;
        }
        if cancel {
            tracing::info!("diálogo cancelado: nada criado");
            self.dialog = None;
            ui.request_repaint();
        } else if done {
            self.confirm_dialog();
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
                    self.select_one(path);
                }
                Err(err) => self.message = Some(format!("Não deu certo: {err}")),
            }
            ui.request_repaint();
        }
        if self.busy > 0 || !self.sizes_waiting.is_empty() {
            ui.request_tick();
        }
        // Miniaturas que chegaram.
        while let Ok((key, thumb)) = self.from_loader.try_recv() {
            self.waiting.remove(&key);
            self.thumbs.insert(key, thumb);
            ui.request_repaint();
        }
        if !self.waiting.is_empty() {
            ui.request_tick();
        }
        if let Some(rx) = &self.apps_rx {
            match rx.try_recv() {
                Ok(apps) => {
                    self.apps = apps;
                    self.apps_rx = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => ui.request_tick(),
                Err(_) => self.apps_rx = None,
            }
        }
        self.toolbar(ui);
        let mode = self.mode();
        // Zoom dos ícones: Ctrl+roda (como a pinça do trackpad no Finder).
        if mode == 0 && ui.input().ctrl {
            let scroll = ui.take_scroll();
            if scroll.abs() > 0.5 {
                self.zoom(if scroll < 0.0 { 1 } else { -1 });
                ui.request_repaint();
            }
        }
        let size = self.icon_size;
        let shown = self.shown();
        let in_list = mode == 1;
        // Área do conteúdo: sobra o rodapé (caminho e status) e, com a prévia, a faixa
        // da direita.
        let region = ui.region();
        let top = region.1 + region.3 - ui.remaining_height();
        let footer_h = 2.0 * (24.0 + ui.theme().spacing);
        let body_h = (ui.remaining_height() - footer_h).max(80.0);
        let preview_w = if self.preview_pane && region.2 > 620.0 { 270.0 } else { 0.0 };
        let columns = ((region.2 - preview_w - 2.0 * ui.theme().padding) / (size as f32 + 44.0 + ui.theme().spacing)).max(1.0) as usize;
        // Na lista, as teclas andam pelas linhas (inclusive as de dentro das pastas).
        let rows = if in_list { self.rows(&shown) } else { Vec::new() };
        let row_entries: Vec<Entry> = rows.iter().map(|(_, e)| e.clone()).collect();
        // Colunas: uma lista por pasta do caminho até o item escolhido.
        let col_dirs = if mode == 2 { self.column_dirs() } else { Vec::new() };
        let mut col_lists: Vec<Vec<Entry>> = Vec::new();
        for dir in &col_dirs {
            let list = if self.place == Place::Dir(dir.clone()) { shown.clone() } else { self.column_list(dir) };
            col_lists.push(list);
        }
        // Nas colunas, ↑ e ↓ andam na coluna do item escolhido.
        let key_list: Vec<Entry> = if mode == 2 {
            let primary = self.primary().cloned();
            col_lists.iter().rev().find(|l| primary.as_ref().is_some_and(|p| l.iter().any(|e| &e.path == p))).cloned().unwrap_or_else(|| shown.clone())
        } else if in_list {
            row_entries.clone()
        } else {
            shown.clone()
        };
        self.keys(ui, &key_list, columns, mode != 0);
        if mode == 2 && !ui.typing() && self.dialog.is_none() {
            self.column_keys(ui, &col_dirs);
        }
        // Rótulos (nome e ícone) do que está à vista: os atalhos mostram o app. Só os
        // `.desktop` entram (os outros usam o nome e o ícone da própria entrada, sem
        // copiar textos a cada quadro numa pasta grande).
        let mut labels: HashMap<PathBuf, (String, String)> = HashMap::new();
        for entry in shown.iter().chain(rows.iter().map(|(_, e)| e)) {
            let shortcut = entry.path.extension().is_some_and(|e| e.eq_ignore_ascii_case("desktop"));
            if shortcut && !labels.contains_key(&entry.path) {
                let label = self.label(entry);
                labels.insert(entry.path.clone(), label);
            }
        }
        let label_of = |entry: &Entry| labels.get(&entry.path).cloned().unwrap_or_else(|| (entry.name.clone(), entry.icon_name().to_string()));
        let mut sizes: HashMap<PathBuf, String> = HashMap::new();
        if in_list {
            for (_, entry) in &rows {
                let text = self.size_of(entry);
                sizes.insert(entry.path.clone(), text);
            }
        }
        let mut toggles: Vec<(PathBuf, bool)> = Vec::new();
        let mut expand_done = false;
        let mut sort_by = None;
        let clipboard = !self.clipboard.is_empty();
        let (ctrl, shift) = (ui.input().ctrl, ui.input().shift);
        let (pressed, pointer) = (ui.input().pressed, ui.input().pointer);
        // Arrasto de dentro ou chegando de outro app (arquivos sobre a janela).
        let dragging = self.drag.clone().or_else(|| ui.input().drop_hover.then(|| self.drag_out.clone().unwrap_or_default()));
        let mut press = None;
        let mut drop_target: Option<PathBuf> = None;
        let in_trash = self.place == Place::Trash;
        let now = SystemTime::now();
        let mut action: Option<(Action, Entry)> = None;
        let mut empty_action = None;
        let mut wanted: Vec<PathBuf> = Vec::new();
        let mut rects: Vec<(PathBuf, (f32, f32, f32, f32))> = Vec::new();
        let ctrl_shift = ctrl || shift;
        ui.at(region.0, top, region.2 - preview_w, |ui| {
        ui.page_sized(body_h, |ui| {
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
            let selection = self.selection.clone();
            let mut select = None;
            let reveal = self.reveal.clone();
            let mut revealed = false;
            let mut item = |ui: &mut Ui, entry: &Entry, r: mide::kit::Response| {
                rects.push((entry.path.clone(), r.rect));
                if reveal.as_ref() == Some(&entry.path) {
                    ui.reveal(r.rect);
                    revealed = true;
                }
                if dragging.is_none() {
                    if r.clicked() {
                        select = Some((entry.path.clone(), false));
                    }
                    if r.secondary_clicked() {
                        select = Some((entry.path.clone(), true));
                    }
                    if r.double_clicked() {
                        action = Some((Action::Open, entry.clone()));
                    }
                    // Apertou no item: pode virar um arrasto.
                    if pressed && r.hovered() {
                        press = pointer.map(|p| (entry.path.clone(), p));
                    }
                } else if let Some(paths) = &dragging {
                    // Arrastando: uma pasta (que não está sendo arrastada) recebe os itens.
                    if entry.is_dir && !in_trash && !paths.contains(&entry.path) && ui.over(r.rect) {
                        drop_target = Some(entry.path.clone());
                        let rect = r.rect;
                        let accent = ui.theme().accent;
                        ui.paint(|canvas, _| {
                            if let Some(path) = mide::ui::draw::rounded_rect(rect.0 + 1.0, rect.1 + 1.0, rect.2 - 2.0, rect.3 - 2.0, 8.0) {
                                let mut paint = tiny_skia::Paint::default();
                                paint.anti_alias = true;
                                paint.set_color(tiny_skia::Color::from_rgba8(accent[0], accent[1], accent[2], 255));
                                canvas.stroke_path(&path, &paint, &tiny_skia::Stroke { width: 2.0, ..Default::default() }, mide::ui::draw::tf(), None);
                            }
                        });
                    }
                }
                let multi = selection.len() > 1 && selection.contains(&entry.path);
                let count = selection.len();
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
                        if m.item(&if multi { format!("Abrir {count} itens") } else { "Abrir".into() }) {
                            chosen = Some(Action::Open);
                        }
                        // Abrir com…: a lista dos apps que abrem o tipo do arquivo.
                        if !multi && !entry.is_dir && m.item("Abrir com…") {
                            chosen = Some(Action::OpenWith);
                        }
                        if !multi && entry.is_dir && m.item("Abrir no Terminal") {
                            chosen = Some(Action::Terminal);
                        }
                        if !multi && entry.is_dir {
                            m.separator();
                            if m.item("Nova pasta aqui dentro") {
                                chosen = Some(Action::NewFolderIn);
                            }
                            if m.item("Novo arquivo aqui dentro") {
                                chosen = Some(Action::NewFileIn);
                            }
                        }
                        m.separator();
                        if !multi && m.item("Renomear…") {
                            chosen = Some(Action::Rename);
                        }
                        if m.item(&if multi { format!("Duplicar {count} itens") } else { "Duplicar".into() }) {
                            chosen = Some(Action::Duplicate);
                        }
                        if multi {
                            if m.item(&format!("Comprimir {count} itens")) {
                                chosen = Some(Action::Compress);
                            }
                        } else if is_zip {
                            if m.item("Extrair aqui") {
                                chosen = Some(Action::Extract);
                            }
                        } else if m.item(&format!("Comprimir \"{}\"", short(&entry.name))) {
                            chosen = Some(Action::Compress);
                        }
                        m.separator();
                        if m.item(&if multi { format!("Copiar {count} itens") } else { "Copiar".into() }) {
                            chosen = Some(Action::Copy);
                        }
                        if !multi && m.item("Copiar caminho") {
                            chosen = Some(Action::CopyPath);
                        }
                        if !multi && m.item("Obter informações") {
                            chosen = Some(Action::Info);
                        }
                        m.separator();
                        if m.danger(&if multi { format!("Mover {count} itens para a Lixeira") } else { "Mover para a Lixeira".into() }) {
                            chosen = Some(Action::Trash);
                        }
                        chosen
                    };
                    if let Some(chosen) = chosen {
                        action = Some((chosen, entry.clone()));
                    }
                });
            };
            if mode == 2 {
                // Colunas (como no Finder): uma lista por pasta, lado a lado; as últimas
                // ficam à vista quando não cabem todas.
                let r = ui.region();
                let col_top = r.1 + r.3 - ui.remaining_height();
                let col_w = 230.0f32;
                let fit = ((r.2 / col_w).floor() as usize).max(1);
                let skip = col_lists.len().saturating_sub(fit);
                let col_h = (r.1 + r.3 - col_top - 2.0).max(60.0);
                let line = ui.theme().line;
                for (ci, list) in col_lists.iter().enumerate().skip(skip) {
                    let x = r.0 + (ci - skip) as f32 * col_w;
                    ui.at(x, col_top, col_w - 8.0, |ui| {
                        ui.scroll_area(col_h, |ui| {
                            for entry in list {
                                let selected = selection.contains(&entry.path) || col_dirs.contains(&entry.path);
                                let (name, icon) = label_of(entry);
                                let arrow = if entry.is_dir { "›" } else { "" };
                                let (r, _) = ui.tree_row(0, false, false, &icon, &name, &[(arrow, 18.0)], selected, false);
                                item(ui, entry, r);
                            }
                        });
                    });
                    ui.paint(|canvas, _| {
                        mide::ui::draw::fill_rounded(canvas, x + col_w - 4.0, col_top, 1.0, col_h, 0.0, tiny_skia::Color::from_rgba8(line[0], line[1], line[2], line[3]));
                    });
                }
                ui.space(col_h);
            } else if in_list {
                // Lista no estilo do Finder: cabeçalho que ordena e linhas alternadas,
                // com a setinha das pastas.
                let widths = [("Modificado", 165.0), ("Tamanho", 85.0), ("Tipo", 150.0)];
                if !shown.is_empty() {
                    sort_by = ui.list_header("Nome", &widths, self.sort);
                }
                // Pasta abrindo ou fechando na lista (sanfona): quantas linhas de dentro
                // aparecem agora; a linha que está entrando é um espaço que cresce, e
                // as de baixo descem (ou sobem) junto.
                let accordion = self.expanding.as_ref().map(|(folder, at, opening)| {
                    let n = rows.iter().filter(|(_, e)| e.path.starts_with(folder) && e.path != *folder).count();
                    let duration = (0.12 + 0.015 * n as f32).clamp(mide::motion::LIST_EXPAND.as_secs_f32(), 0.4);
                    let p = (at.elapsed().as_secs_f32() / duration).clamp(0.0, 1.0);
                    let k = mide::motion::ease_out(p);
                    let shown_rows = if *opening { k } else { 1.0 - k } * n as f32;
                    (folder.clone(), *opening, shown_rows, p >= 1.0)
                });
                if let Some((_, _, _, done)) = &accordion {
                    expand_done = *done;
                }
                let mut order = 0usize;
                for (i, (depth, entry)) in rows.iter().enumerate() {
                    if let Some((folder, _, shown_rows, _)) = &accordion {
                        if entry.path != *folder && entry.path.starts_with(folder) {
                            let j = order as f32;
                            order += 1;
                            if j + 1.0 > *shown_rows {
                                if j < *shown_rows {
                                    ui.space((shown_rows - j) * 26.0);
                                }
                                continue;
                            }
                        }
                    }
                    let selected = selection.contains(&entry.path);
                    let date = files::date_text(entry.modified, now);
                    let size = sizes.get(&entry.path).cloned().unwrap_or_default();
                    let kind = files::kind_text(entry);
                    let closing = self.expanding.as_ref().is_some_and(|(f, _, opening)| !opening && *f == entry.path);
                    let open = self.expanded.contains(&entry.path) && !closing;
                    let expandable = entry.is_dir && !in_trash;
                    let cols = [(date.as_str(), widths[0].1), (size.as_str(), widths[1].1), (kind.as_str(), widths[2].1)];
                    let (name, icon) = label_of(entry);
                    let (r, toggled) = ui.tree_row(*depth, expandable, open, &icon, &name, &cols, selected, i % 2 == 1);
                    // Itens da pasta que acabou de abrir: aparecem esmaecendo, de cima
                    // para baixo (cada linha começa um pouco depois da anterior).
                    if let Some((folder, at, true)) = &self.expanding {
                        if entry.path != *folder && entry.path.starts_with(folder) {
                            let order = rows.iter().take(i).filter(|(_, e)| e.path.starts_with(folder) && e.path != *folder).count();
                            let delay = order as f32 * mide::motion::LIST_STAGGER.as_secs_f32();
                            let t = ((at.elapsed().as_secs_f32() - delay) / mide::motion::LIST_EXPAND.as_secs_f32()).clamp(0.0, 1.0);
                            veil(ui, r.rect, 1.0 - mide::motion::ease_out_soft(t));
                        }
                    }
                    if toggled {
                        toggles.push((entry.path.clone(), !open));
                    } else {
                        item(ui, entry, r);
                    }
                }
            } else {
                // Ícones com miniaturas (imagens, texto, PDF, vídeo...): só as que
                // aparecem são pedidas; até chegar, o ícone do tipo.
                let view = ui.region();
                let thumbs = &self.thumbs;
                ui.wrap(|ui| {
                    for entry in &shown {
                        let selected = selection.contains(&entry.path);
                        let (name, icon) = label_of(entry);
                        let wants = Self::wants_thumb(entry);
                        let cached = if wants { thumbs.get(&(entry.path.clone(), size)) } else { None };
                        let image = match cached {
                            Some(Some(pix)) => Tile::Image(pix),
                            _ => Tile::Icon(&icon),
                        };
                        let r = ui.tile(image, &name, selected, size);
                        if wants && cached.is_none() && r.rect.1 + r.rect.3 >= view.1 - 200.0 && r.rect.1 <= view.1 + view.3 + 200.0 {
                            wanted.push(entry.path.clone());
                        }
                        item(ui, entry, r);
                    }
                });
            }
            if revealed {
                self.reveal = None;
            }
            if let Some((path, secondary)) = select {
                if secondary {
                    // Botão direito num item de fora da seleção escolhe só ele; num de
                    // dentro, a ação vale para todos.
                    if !self.selection.contains(&path) {
                        self.select_one(path);
                    }
                } else {
                    let order: Vec<PathBuf> = if in_list { rows.iter().map(|(_, e)| e.path.clone()).collect() } else { shown.iter().map(|e| e.path.clone()).collect() };
                    self.click_select(path, ctrl, shift, &order);
                }
            }
            // Arrasto em andamento: os itens seguem o ponteiro.
            if let (Some(paths), Some((px, py))) = (&self.drag, ui.pointer()) {
                let label = if paths.len() == 1 {
                    paths[0].file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
                } else {
                    format!("{} itens", paths.len())
                };
                let (accent, on_accent) = (ui.theme().accent, ui.theme().on_accent);
                ui.paint_text(|canvas, text, _| {
                    let w = text.measure(&label, 12.5, mide::ui::text::FontWeight::Semibold) + 20.0;
                    mide::ui::draw::fill_rounded(canvas, px + 14.0, py + 10.0, w, 24.0, 12.0, tiny_skia::Color::from_rgba8(accent[0], accent[1], accent[2], 230));
                    text.draw(canvas, &label, px + 24.0, py + 14.0, 12.5, mide::ui::text::FontWeight::Semibold, on_accent);
                });
            }
            // Fundo da pasta: clique no vazio tira a seleção; o botão direito abre o
            // menu da pasta.
            let area = ui.region();
            // Troca entre ícones e lista: o conteúdo novo aparece esmaecendo.
            if let Some(at) = self.view_switched {
                let t = mide::motion::progress(at, mide::motion::VIEW_SWITCH);
                veil(ui, area, 0.9 * (1.0 - mide::motion::ease_out_soft(t)));
            }
            // Seleção por retângulo (como no Finder): apertar no vazio e arrastar
            // escolhe o que o retângulo toca; com Ctrl ou Shift, soma à seleção.
            let ptr = ui.pointer();
            let (pressed_now, down_now) = (ui.input().pressed, ui.input().down);
            let mut lasso_used = self.lasso.is_some();
            if pressed_now && press.is_none() && dragging.is_none() && ui.menu_open().is_none() && mode != 2 {
                if let Some(p) = ptr.filter(|p| ui.over(area) && p.0 < area.0 + area.2 - 16.0) {
                    self.lasso_press = Some(p);
                }
            }
            if let (Some(start), Some(p), true) = (self.lasso_press, ptr, down_now) {
                if self.lasso.is_none() && ((p.0 - start.0).powi(2) + (p.1 - start.1).powi(2)).sqrt() > 4.0 {
                    let base = if ctrl_shift { self.selection.clone() } else { Vec::new() };
                    self.lasso = Some((start, p, base));
                }
                if let Some(lasso) = self.lasso.as_mut() {
                    lasso.1 = p;
                }
            }
            if let Some((a, b, base)) = self.lasso.clone() {
                let (x0, y0, x1, y1) = (a.0.min(b.0), a.1.min(b.1), a.0.max(b.0), a.1.max(b.1));
                let mut chosen = base;
                for (path, r) in &rects {
                    if r.0 < x1 && r.0 + r.2 > x0 && r.1 < y1 && r.1 + r.3 > y0 && !chosen.contains(path) {
                        chosen.push(path.clone());
                    }
                }
                self.selection = chosen;
                lasso_used = true;
                let accent = ui.theme().accent;
                ui.paint(|canvas, _| {
                    let (w, h) = ((x1 - x0).max(1.0), (y1 - y0).max(1.0));
                    mide::ui::draw::fill_rounded(canvas, x0, y0, w, h, 2.0, tiny_skia::Color::from_rgba8(accent[0], accent[1], accent[2], 40));
                    if let Some(path) = mide::ui::draw::rounded_rect(x0 + 0.5, y0 + 0.5, w - 1.0, h - 1.0, 2.0) {
                        let mut paint = tiny_skia::Paint::default();
                        paint.anti_alias = true;
                        paint.set_color(tiny_skia::Color::from_rgba8(accent[0], accent[1], accent[2], 200));
                        canvas.stroke_path(&path, &paint, &tiny_skia::Stroke { width: 1.0, ..Default::default() }, mide::ui::draw::tf(), None);
                    }
                });
                ui.request_repaint();
            }
            if !down_now {
                if self.lasso.take().is_some() {
                    tracing::info!("seleção por retângulo: {} item(ns)", self.selection.len());
                }
                self.lasso_press = None;
            }
            let bg = ui.background(area);
            if bg.clicked() && !lasso_used {
                self.selection.clear();
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
        });
        for path in wanted {
            self.thumb(&path, size);
        }
        if !self.waiting.is_empty() {
            ui.request_tick();
        }
        // Painel de prévia à direita, separado por uma linha fina.
        if preview_w > 0.0 {
            ui.at(region.0 + region.2 - preview_w, top, preview_w, |ui| self.preview_panel(ui, body_h));
            let line = ui.theme().line;
            let x = region.0 + region.2 - preview_w;
            ui.paint(|canvas, _| mide::ui::draw::fill_rounded(canvas, x, top, 1.0, body_h, 0.0, tiny_skia::Color::from_rgba8(line[0], line[1], line[2], line[3])));
        }
        ui.space(body_h);
        // Rodapé do Finder: barra de caminho (clicar vai até a pasta) e barra de status
        // com o zoom dos ícones.
        let parts = self.path_parts();
        let names: Vec<(String, String)> = parts.iter().map(|(n, i, _)| (n.clone(), i.clone())).collect();
        if let Some(i) = ui.path_bar(&names) {
            let target = parts[i].2.clone();
            if target.is_dir() {
                self.go(Place::Dir(target));
            } else {
                self.select_one(target);
            }
            ui.request_repaint();
        }
        let count = if mode == 2 { col_lists.first().map_or(0, Vec::len) } else { shown.len() };
        let status = ui.status_bar(&self.status_text(count));
        if mode == 0 {
            let mut value = self.icon_size as f32;
            let mut changed = false;
            ui.at(status.0 + status.2 - 140.0, status.1 + 3.0, 128.0, |ui| {
                changed = ui.mini_slider(128.0, &mut value, files_view::ICON_MIN as f32, files_view::ICON_MAX as f32).changed();
            });
            if changed {
                let snapped = (((value / 16.0).round() * 16.0) as u32).clamp(files_view::ICON_MIN, files_view::ICON_MAX);
                if snapped != self.icon_size {
                    self.icon_size = snapped;
                    self.zoom_dirty = true;
                }
                ui.request_repaint();
            }
            if self.zoom_dirty && !ui.input().down {
                self.zoom_dirty = false;
                self.save_view();
            }
        }
        for (path, open) in toggles {
            self.toggle_expand(&path, open);
            ui.request_repaint();
        }
        // Arrastar e soltar: apertou num item e andou mais de 6 pontos com o botão
        // segurado → arrasta a seleção (ou só ele); soltou numa pasta → move (com Ctrl,
        // copia).
        if let Some(p) = press {
            self.press = Some(p);
        }
        let input = ui.input();
        let (down, released, ctrl_now) = (input.down, input.released, input.ctrl);
        if let (Some((path, (x0, y0))), Some((x, y))) = (self.press.clone(), pointer) {
            if down && self.drag.is_none() && ((x - x0).powi(2) + (y - y0).powi(2)).sqrt() > 6.0 && !in_trash {
                if !self.selection.contains(&path) {
                    self.select_one(path.clone());
                }
                tracing::info!("arrastando {} item(ns)", self.selection.len());
                self.drag = Some(self.selection.clone());
            }
        }
        // Arrastando para fora da janela: vira um arrasto entre apps (outro app, um site
        // no navegador, a área de trabalho), com os arquivos como `text/uri-list`.
        let (win_w, win_h) = ui.window_size();
        let outside = pointer.is_none_or(|(x, y)| x < 0.0 || y < 0.0 || x >= win_w || y >= win_h);
        if down && outside && self.drag.is_some() {
            if let Some(paths) = self.drag.take() {
                tracing::info!("arrastando {} item(ns) para fora da janela", paths.len());
                let icon = self.drag_icon(&paths);
                ui.start_drag(&paths, icon);
                self.drag_out = Some(paths);
                self.press = None;
            }
        }
        if ui.input().drag_ended {
            self.drag_out = None;
        }
        // Arquivos soltos na janela: numa pasta da lista ou na pasta aberta. Do próprio
        // Arquivos, move (com Ctrl, copia), como dentro da janela; de outro app, copia.
        if let Some(paths) = ui.input().dropped.clone() {
            let own = self.drag_out.as_ref().is_some_and(|out| *out == paths);
            let dir = drop_target.clone().or_else(|| match &self.place {
                Place::Dir(dir) => Some(dir.clone()),
                _ => None,
            });
            match dir {
                Some(dir) if own && paths.iter().all(|p| p.parent() == Some(dir.as_path())) => {}
                Some(dir) => self.drop_into(&paths, &dir, !own || ctrl_now),
                None => self.message = Some("Solte os arquivos numa pasta.".into()),
            }
            ui.request_repaint();
        }
        if self.drag.is_some() {
            ui.request_repaint();
        }
        if released || !down {
            if let Some(paths) = self.drag.take() {
                match drop_target {
                    Some(dir) => self.drop_into(&paths, &dir, ctrl_now),
                    None => tracing::info!("arrasto solto fora de uma pasta"),
                }
                ui.request_repaint();
            }
            self.press = None;
        }
        // Transições em andamento: pedem quadros até acabar.
        if expand_done || self.expanding.as_ref().is_some_and(|(_, at, _)| at.elapsed().as_secs_f32() > 0.6) {
            // Fechando: a pasta sai da lista agora que as linhas sumiram.
            if let Some((folder, _, false)) = self.expanding.take() {
                self.expanded.retain(|p| !p.starts_with(&folder));
            }
        }
        if self.view_switched.is_some_and(|at| at.elapsed().as_secs_f32() > 0.22) {
            self.view_switched = None;
        }
        if self.expanding.is_some() || self.view_switched.is_some() {
            ui.request_tick();
        }
        if let Some(column) = sort_by {
            // Clicar na coluna já ordenada inverte a ordem.
            self.sort = if self.sort.0 == column { (column, !self.sort.1) } else { (column, true) };
            self.sort_entries();
            self.save_view();
            ui.request_repaint();
        }
        if let Some((action, entry)) = action {
            match action {
                Action::Open if self.targets(&entry).len() > 1 => {
                // Vários: arquivos abrem cada um no app padrão; com pastas, entra na primeira.
                let targets = self.targets(&entry);
                for path in targets.iter().filter(|p| !p.is_dir()) {
                    self.open_file(path);
                }
                if let Some(dir) = targets.into_iter().find(|p| p.is_dir()) {
                    self.go(Place::Dir(dir));
                }
            }
            Action::Open => self.open(&entry),
            Action::OpenWith => self.open_with = Some(entry.path.clone()),
                Action::Rename => self.start_rename(entry.path.clone()),
                Action::CopyPath => places::copy_text(&entry.path.to_string_lossy()),
                Action::Info => self.show_info(&entry.path),
                Action::Terminal => places::open_terminal(&entry.path),
                Action::Duplicate => {
                    for path in self.targets(&entry) {
                        self.duplicate(&path);
                    }
                }
                Action::NewFolderIn => self.open_dialog(DialogKind::NewFolder(entry.path.clone()), "Nova pasta".into()),
                Action::NewFileIn => self.open_dialog(DialogKind::NewFile(entry.path.clone()), "Novo arquivo.txt".into()),
                Action::Compress => {
                    let targets = self.targets(&entry);
                    if targets.len() > 1 {
                        self.compress_job(targets);
                    } else {
                        self.run_job(entry.path.clone(), false);
                    }
                }
                Action::Extract => self.run_job(entry.path.clone(), true),
                Action::Copy => {
                    self.clipboard = self.targets(&entry);
                    tracing::info!("copiado(s): {} item(ns)", self.clipboard.len());
                }
                Action::Trash => {
                    for path in self.targets(&entry) {
                        self.move_to_trash(&path);
                    }
                }
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

    /// Pastas mostradas nas colunas: a pasta atual e, até o item escolhido, cada pasta
    /// do caminho dele (como no Finder).
    fn column_dirs(&self) -> Vec<PathBuf> {
        let Place::Dir(base) = &self.place else {
            return Vec::new();
        };
        let mut dirs = vec![base.clone()];
        if let Some(rel) = self.primary().and_then(|p| p.strip_prefix(base).ok()) {
            let mut cur = base.clone();
            for part in rel.components() {
                cur.push(part);
                if cur.is_dir() {
                    dirs.push(cur.clone());
                }
            }
        }
        dirs
    }

    /// O que há numa pasta das colunas (guardado até recarregar).
    fn column_list(&mut self, dir: &Path) -> Vec<Entry> {
        if let Some(list) = self.col_cache.get(dir) {
            return list.clone();
        }
        let mut list = files::list_dir(dir, self.hidden).unwrap_or_default();
        sort_list(&mut list, self.sort);
        self.col_cache.insert(dir.to_path_buf(), list.clone());
        list
    }

    /// Barra de caminho: do disco até a pasta atual (e o item escolhido, como no
    /// Finder). Cada pedaço: nome, ícone e para onde vai.
    fn path_parts(&self) -> Vec<(String, String, PathBuf)> {
        let dir = match &self.place {
            Place::Trash => return vec![("Lixeira".into(), "user-trash".into(), Self::trash())],
            Place::Dir(dir) => dir.clone(),
        };
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let mut parts = Vec::new();
        let mut cur = PathBuf::new();
        for comp in dir.components() {
            cur.push(comp);
            let (name, icon) = if cur == Path::new("/") {
                ("Computador".to_string(), "drive-harddisk")
            } else if Some(&cur) == home.as_ref() {
                (cur.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), "user-home")
            } else {
                (cur.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), "folder")
            };
            parts.push((name, icon.to_string(), cur.clone()));
        }
        if let Some(sel) = self.primary().filter(|p| p.parent() == Some(dir.as_path())) {
            let name = sel.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let icon = self.entries.iter().find(|e| &e.path == sel).map(|e| e.icon_name()).unwrap_or("text-x-generic");
            parts.push((name, icon.to_string(), sel.clone()));
        }
        parts
    }

    /// Texto da barra de status: itens, escolhidos e espaço livre.
    fn status_text(&self, count: usize) -> String {
        let items = if count == 1 { "1 item".to_string() } else { format!("{count} itens") };
        let chosen = self.selection.len();
        let mut text = if chosen > 0 { format!("{chosen} de {items} escolhido(s)") } else { items };
        if let Some(free) = self.free_space {
            text.push_str(&format!(", {} disponíveis", files::size_text(free)));
        }
        text
    }

    /// Painel de prévia (à direita, como no Finder): miniatura grande, nome, tipo e
    /// tamanho, e as informações do item.
    fn preview_panel(&mut self, ui: &mut Ui, height: f32) {
        let Some(path) = self.primary().cloned() else {
            ui.scroll_area_padded(height, 16.0, |ui| {
                ui.space(40.0);
                ui.note("Nenhum item escolhido");
            });
            return;
        };
        if self.preview_info.as_ref().is_none_or(|(p, _)| *p != path) {
            self.preview_info = Some((path.clone(), files::info_lines(&path).unwrap_or_default()));
        }
        let entry = self.entries.iter().chain(self.col_cache.values().flatten()).find(|e| e.path == path).cloned();
        let icon = entry.as_ref().map(|e| e.icon_name()).unwrap_or("text-x-generic");
        let image = if entry.as_ref().is_some_and(Self::wants_thumb) { self.thumb_clone(&path, 220) } else { None };
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let kind = entry.as_ref().map(files::kind_text).unwrap_or_default();
        let size = entry.as_ref().filter(|e| !e.is_dir).map(|e| files::size_text(e.size)).unwrap_or_default();
        let lines = self.preview_info.as_ref().map(|(_, l)| l.clone()).unwrap_or_default();
        let mut quick = false;
        ui.scroll_area_padded(height, 16.0, |ui| {
            ui.space(8.0);
            ui.image_centered(image.as_ref(), icon, 220);
            ui.strong(&name);
            ui.note(&if size.is_empty() { kind.clone() } else { format!("{kind} - {size}") });
            ui.separator();
            ui.heading("Informações");
            for (label, value) in &lines {
                ui.row(|ui| {
                    ui.sized(96.0, |ui| ui.note(label));
                    ui.label(value);
                });
            }
            ui.space(8.0);
            quick = ui.button("Visualização Rápida").clicked();
        });
        if quick {
            self.quick_look = Some(path);
        }
    }

    /// Visualização Rápida (Espaço, como no Finder): o item grande no meio da janela.
    /// Espaço ou Esc fecham; as setas trocam de item.
    fn quick_look_view(&mut self, ui: &mut Ui) {
        let Some(path) = self.quick_look.clone() else {
            return;
        };
        // A seleção mudou pelas setas: mostra o item novo.
        if let Some(p) = self.primary().cloned() {
            if p != path {
                self.quick_look = Some(p);
                ui.request_repaint();
                return;
            }
        }
        let (ww, wh) = ui.window_size();
        let side = ((ww.min(wh) as f32) * 0.62).clamp(160.0, 640.0) as u32;
        let entry = self.entries.iter().chain(self.col_cache.values().flatten()).find(|e| e.path == path).cloned();
        let icon = entry.as_ref().map(|e| e.icon_name()).unwrap_or("text-x-generic");
        let image = if entry.as_ref().is_some_and(Self::wants_thumb) { self.thumb_clone(&path, side) } else { None };
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let mut close = ui.input().key(Key::Escape);
        let mut open = false;
        ui.modal(side as f32 + 48.0, |ui| {
            ui.row(|ui| {
                ui.strong(&short(&name));
                ui.push_right(110.0);
                ui.sized(110.0, |ui| open = ui.button("Abrir").clicked());
            });
            ui.image_centered(image.as_ref(), icon, side);
        });
        if open {
            if let Some(entry) = entry {
                self.open(&entry);
            }
            close = true;
        }
        if close {
            self.quick_look = None;
            ui.request_repaint();
        }
        if self.waiting.iter().any(|(p, _)| *p == path) {
            ui.request_tick();
        }
    }

    fn thumb_clone(&mut self, path: &Path, size: u32) -> Option<Pixmap> {
        self.thumb(path, size).cloned()
    }
}

/// Diálogo do Arquivos: o que ele faz, o nome sugerido, o nome digitado e o erro.
struct Dialog {
    kind: DialogKind,
    suggestion: String,
    name: String,
    error: Option<String>,
}

/// O que o diálogo faz; criar leva a pasta onde criar.
enum DialogKind {
    NewFolder(PathBuf),
    NewFile(PathBuf),
    Rename(PathBuf),
}

#[derive(Clone, Copy)]
enum Action {
    Open,
    /// Abrir com…: escolher o app numa lista.
    OpenWith,
    NewFolderIn,
    NewFileIn,
    Terminal,
    Rename,
    Duplicate,
    Compress,
    Extract,
    Copy,
    Paste,
    NewFile,
    CopyPath,
    /// Obter informações (Ctrl+I).
    Info,
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
        ui.glass_sidebar();
        let chosen = ui.sidebar_area(&refs, current, |ui, _| self.content(ui));
        // Diálogo por cima de tudo (a janela inteira escurece atrás).
        self.dialog_view(ui);
        self.open_with_view(ui);
        self.info_view(ui);
        self.prompt_view(ui);
        self.quick_look_view(ui);
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

/// Texto do log: "lista, ordem modificado decrescente".
fn view_text(list: bool, (column, ascending): (usize, bool)) -> String {
    let column = ["nome", "modificado", "tamanho", "tipo"].get(column).copied().unwrap_or("nome");
    format!("{}, ordem {column} {}", if list { "lista" } else { "ícones" }, if ascending { "crescente" } else { "decrescente" })
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
    // `args_os`: nomes que não são UTF-8 (ex.: Latin-1 vindo do "Mostrar na pasta") chegam
    // com os bytes originais; `args()` entraria em pânico com eles.
    let start = parse_args(std::env::args_os().skip(1).collect());
    let mut app = Files::new(start.place);
    if !start.select.is_empty() {
        // Itens ocultos pedidos: os ocultos aparecem.
        if start.select.iter().any(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'))) {
            app.hidden = true;
            app.reload();
        }
        for path in &start.select {
            tracing::info!("selecionado {}", path.display());
        }
        app.reveal = start.select.first().cloned();
        app.selection = start.select;
    }
    if let Some(path) = start.info {
        app.show_info(&path);
    }
    let window = Window::new("Arquivos").app_id("mide-files").size(900, 520).min_size(560, 360);
    run(window, app)
}

/// Como o Arquivos abre: a pasta, os itens escolhidos e o item das informações.
#[derive(Debug, PartialEq)]
struct Start {
    place: Place,
    select: Vec<PathBuf>,
    info: Option<PathBuf>,
}

/// Lê os argumentos: `[pasta|arquivo|trash:///]`, `--select itens...` e `--info item`.
/// Itens que não existem ficam de fora; sem nada que exista, a pasta pessoal.
fn parse_args(args: Vec<OsString>) -> Start {
    let to_path = |arg: &OsStr| -> Option<PathBuf> {
        let path = match arg.to_str() {
            Some(text) => places::path_from_uri(text).or_else(|| (!text.contains("://")).then(|| PathBuf::from(text)))?,
            // Não é UTF-8: só pode ser um caminho (os endereços chegam codificados).
            None => PathBuf::from(arg),
        };
        let path = if path.is_relative() { std::env::current_dir().ok()?.join(path) } else { path };
        // Sem barra no fim nem `.`: a mesma pasta tem uma chave só (B21).
        let path = files_view::normalize(&path);
        path.symlink_metadata().is_ok().then_some(path)
    };
    let mut place = None;
    let mut select = Vec::new();
    let mut info = None;
    let mut args = args.into_iter();
    let mut selecting = false;
    while let Some(arg) = args.next() {
        if arg == "--select" {
            selecting = true;
        } else if arg == "--info" {
            info = args.next().and_then(|a| to_path(&a));
        } else if arg.to_str().is_some_and(|a| a.starts_with("trash:")) && place.is_none() && select.is_empty() {
            place = Some(Place::Trash);
        } else if let Some(path) = to_path(&arg) {
            if !selecting && place.is_none() && path.is_dir() {
                place = Some(Place::Dir(path));
            } else if selecting || place.is_none() {
                select.push(path);
            }
        }
    }
    // Itens escolhidos (ou o das informações): a pasta é a do primeiro; os de outras
    // pastas ficam de fora (o serviço abre uma janela por pasta).
    let anchor = select.first().cloned().or_else(|| if place.is_none() { info.clone() } else { None });
    if let Some(first) = anchor {
        let parent = first.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("/"));
        select.retain(|p| p.parent() == Some(parent.as_path()));
        if select.is_empty() {
            select.push(first);
        }
        place = Some(Place::Dir(parent));
    }
    Start { place: place.unwrap_or_else(|| Place::Dir(places::home_dir())), select, info }
}

/// Véu da cor do fundo sobre `rect` (área atual), com opacidade `k` (0 a 1): o que
/// está embaixo aparece conforme o véu some.
fn veil(ui: &mut Ui, rect: (f32, f32, f32, f32), k: f32) {
    if k <= 0.01 {
        return;
    }
    let bg = ui.theme().background;
    ui.paint(|canvas, offset| {
        if let Some(r) = tiny_skia::Rect::from_xywh(offset.0 + rect.0, offset.1 + rect.1, rect.2, rect.3) {
            let mut paint = tiny_skia::Paint::default();
            paint.set_color(tiny_skia::Color::from_rgba8(bg[0], bg[1], bg[2], (k.clamp(0.0, 1.0) * 255.0) as u8));
            canvas.fill_rect(r, &paint, mide::ui::draw::tf(), None);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argumentos_de_abrir() {
        let dir = std::env::temp_dir().join(format!("mide-files-args-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub pasta")).unwrap();
        std::fs::write(dir.join("a.txt"), "a").unwrap();
        std::fs::write(dir.join("b.txt"), "b").unwrap();
        std::fs::write(dir.join("sub pasta/c.txt"), "c").unwrap();
        let s = |v: &[&str]| parse_args(v.iter().map(OsString::from).collect());
        let d = dir.display().to_string();
        // Pasta (também como file:// com %20).
        assert_eq!(s(&[&d]).place, Place::Dir(dir.clone()));
        let uri = places::file_uri(&dir.join("sub pasta"));
        assert_eq!(s(&[&uri]), Start { place: Place::Dir(dir.join("sub pasta")), select: vec![], info: None });
        // Arquivo: abre a pasta dele com ele escolhido.
        assert_eq!(s(&[&format!("{d}/a.txt")]), Start { place: Place::Dir(dir.clone()), select: vec![dir.join("a.txt")], info: None });
        // --select com vários da mesma pasta; um de outra pasta fica de fora.
        let got = s(&["--select", &format!("{d}/a.txt"), &format!("{d}/b.txt"), &format!("{d}/sub pasta/c.txt")]);
        assert_eq!(got.place, Place::Dir(dir.clone()));
        assert_eq!(got.select, vec![dir.join("a.txt"), dir.join("b.txt")]);
        // --select de uma pasta: a de cima, com a pasta escolhida (não entra nela).
        assert_eq!(s(&["--select", &format!("{d}/sub pasta/")]).select, vec![dir.join("sub pasta")]);
        // --info: pasta do item, com ele escolhido.
        let got = s(&["--info", &format!("{d}/b.txt")]);
        assert_eq!((got.place, got.info), (Place::Dir(dir.clone()), Some(dir.join("b.txt"))));
        // Nada que exista: pasta pessoal.
        assert_eq!(s(&["--select", "/nao/existe"]).place, Place::Dir(places::home_dir()));
        assert_eq!(s(&["trash:///"]).place, Place::Trash);
        // Nome que não é UTF-8 (Latin-1, "é.txt"): abre a pasta com ele escolhido.
        {
            use std::os::unix::ffi::OsStrExt;
            let latin1 = dir.join(OsStr::from_bytes(b"\xe9.txt"));
            std::fs::write(&latin1, "x").unwrap();
            let got = parse_args(vec![OsString::from("--select"), latin1.clone().into_os_string()]);
            assert_eq!((got.place, got.select), (Place::Dir(dir.clone()), vec![latin1]));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
