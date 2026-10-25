//! Seletor de arquivos do mide-ui: "Abrir…", "Salvar como…" e "Escolher pasta…" num
//! diálogo por cima da janela do app, como o painel do macOS (um mini-Finder): barra
//! lateral com os favoritos e o computador, voltar/avançar, o nome da pasta, subir,
//! busca na pasta, lista com nome, tamanho e data (pastas primeiro) e "Nova pasta".
//!
//! O app guarda o seletor enquanto ele está aberto e chama `show` a cada quadro:
//!
//! ```ignore
//! if let Some(chooser) = self.abrir.as_mut() {
//!     match chooser.show(ui) {
//!         Some(Choice::Chosen(path)) => { self.abrir_arquivo(path); self.abrir = None; }
//!         Some(Choice::Cancelled) => self.abrir = None,
//!         None => {}
//!     }
//! }
//! ```
//!
//! Também é o seletor dos outros apps (portal `FileChooser`, B25): o `mide-chooser`
//! mostra o seletor ocupando a janela inteira (`embedded`), com vários arquivos
//! (`multiple`), filtros com nome (por padrão de nome ou tipo MIME), opções extras
//! (`choices`) e "salvar vários" (`save_files`).

use std::path::{Path, PathBuf};

use super::{input::Key, Ui};

/// O que o seletor escolhe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Um arquivo que existe.
    Open,
    /// Um nome novo (ou um arquivo para substituir) numa pasta.
    Save,
    /// Uma pasta.
    Folder,
    /// Uma pasta onde salvar vários arquivos com nomes já dados (`save_files`).
    SaveFiles,
}

/// Um padrão de um filtro: de nome (`*.png`, sem diferenciar maiúsculas) ou de tipo
/// MIME (`image/png`, `image/*`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pattern {
    Glob(String),
    Mime(String),
}

/// Filtro com nome ("Imagens"): um arquivo passa se casar com algum padrão.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filter {
    pub name: String,
    pub patterns: Vec<Pattern>,
}

impl Filter {
    /// O arquivo passa no filtro? Pastas sempre passam (para navegar).
    pub fn accepts(&self, path: &Path, is_dir: bool) -> bool {
        if is_dir || self.patterns.is_empty() {
            return true;
        }
        let name = path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
        let mut mime: Option<Option<String>> = None;
        self.patterns.iter().any(|p| match p {
            Pattern::Glob(glob) => crate::default_apps::glob_matches(&glob.to_lowercase(), &name),
            Pattern::Mime(want) => {
                let got = mime.get_or_insert_with(|| crate::default_apps::mime_for(path));
                let want = want.to_lowercase();
                match (got, want.strip_suffix("/*")) {
                    (_, None) if want == "*" || want == "*/*" || want == "application/octet-stream" => true,
                    (Some(got), Some(major)) => got.split('/').next() == Some(major),
                    (Some(got), None) => *got == want,
                    (None, _) => false,
                }
            }
        })
    }
}

/// Opção extra pedida pelo app (portal: `choices`): sem `options`, uma caixa de
/// marcar ("true"/"false"); com elas, uma lista.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtraChoice {
    pub id: String,
    pub label: String,
    /// (id, nome) de cada opção da lista.
    pub options: Vec<(String, String)>,
    /// O que está escolhido agora (id da opção, ou "true"/"false").
    pub value: String,
}

/// Resultado do seletor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    Chosen(PathBuf),
    Cancelled,
}

#[derive(Debug, Clone)]
struct Entry {
    name: String,
    path: PathBuf,
    is_dir: bool,
    size: Option<u64>,
    modified: Option<std::time::SystemTime>,
}

pub struct FileChooser {
    mode: Mode,
    title: String,
    dir: PathBuf,
    entries: Vec<Entry>,
    selected: Option<usize>,
    /// Mais itens escolhidos com Ctrl/Shift (`multiple`), além de `selected`.
    picked: Vec<usize>,
    multiple: bool,
    /// Nome do arquivo (modo Salvar).
    name: String,
    /// Filtros com nome (vazio: todos os arquivos) e o escolhido.
    filters: Vec<Filter>,
    filter: usize,
    choices: Vec<ExtraChoice>,
    /// Texto do botão principal (vazio: "Abrir", "Salvar", "Escolher").
    accept_label: String,
    /// Ocupa a janela inteira (o `mide-chooser`), em vez de um diálogo por cima do app.
    embedded: bool,
    /// Nomes dos arquivos do modo SaveFiles.
    save_names: Vec<String>,
    /// Caminhos escolhidos (todos), depois de `Choice::Chosen`.
    result: Vec<PathBuf>,
    show_hidden: bool,
    message: Option<String>,
    /// Pede o foco no campo do nome no primeiro quadro (modo Salvar).
    focus: bool,
    /// Pede confirmação para substituir este arquivo (modo Salvar).
    replace: Option<PathBuf>,
    /// Pastas visitadas (voltar e avançar, como no Finder).
    back: Vec<PathBuf>,
    forward: Vec<PathBuf>,
    /// Busca pelo nome na pasta mostrada.
    query: String,
    /// "Nova pasta": o nome sendo escrito.
    new_folder: Option<String>,
}

impl FileChooser {
    /// Escolher um arquivo para abrir, começando em `dir` (ou na pasta pessoal).
    pub fn open(title: &str, dir: Option<&Path>) -> Self {
        Self::new(Mode::Open, title, dir, "")
    }

    /// Escolher onde salvar, com o nome sugerido.
    pub fn save(title: &str, dir: Option<&Path>, name: &str) -> Self {
        Self::new(Mode::Save, title, dir, name)
    }

    /// Escolher uma pasta.
    pub fn folder(title: &str, dir: Option<&Path>) -> Self {
        Self::new(Mode::Folder, title, dir, "")
    }

    /// Só mostra (e aceita) arquivos com essas extensões, sem o ponto (`["txt", "md"]`).
    pub fn extensions(mut self, extensions: &[&str]) -> Self {
        let patterns = extensions.iter().map(|e| Pattern::Glob(format!("*.{}", e.to_lowercase()))).collect();
        self.filters = vec![Filter { name: String::new(), patterns }];
        self.filter = 0;
        self.reload();
        self
    }

    /// Salvar vários arquivos: escolhe a pasta, e cada nome vira `pasta/nome`.
    pub fn save_files(title: &str, dir: Option<&Path>, names: &[String]) -> Self {
        let mut chooser = Self::new(Mode::SaveFiles, title, dir, "");
        chooser.save_names = names.to_vec();
        chooser
    }

    /// Escolher vários arquivos de uma vez (Ctrl+clique e Shift+clique), no modo Abrir.
    pub fn multiple(mut self, multiple: bool) -> Self {
        self.multiple = multiple;
        self
    }

    /// Filtros com nome; `current` é o que começa escolhido.
    pub fn filters(mut self, filters: Vec<Filter>, current: usize) -> Self {
        self.filter = current.min(filters.len().saturating_sub(1));
        self.filters = filters;
        self.reload();
        self
    }

    /// Opções extras (caixas de marcar e listas) embaixo da lista.
    pub fn choices(mut self, choices: Vec<ExtraChoice>) -> Self {
        self.choices = choices;
        self
    }

    /// Texto do botão principal.
    pub fn accept_label(mut self, label: &str) -> Self {
        self.accept_label = label.replace('_', "");
        self
    }

    /// Ocupa a janela inteira, sem o fundo escurecido do diálogo.
    pub fn embedded(mut self, embedded: bool) -> Self {
        self.embedded = embedded;
        self
    }

    /// Todos os caminhos escolhidos (depois de `Choice::Chosen`, que traz o primeiro).
    pub fn chosen(&self) -> &[PathBuf] {
        &self.result
    }

    /// Filtro escolhido agora (índice em `filters`), se houver filtros.
    pub fn current_filter(&self) -> Option<usize> {
        (!self.filters.is_empty()).then_some(self.filter)
    }

    /// Valores das opções extras, como estão agora.
    pub fn choice_values(&self) -> Vec<(String, String)> {
        self.choices.iter().map(|c| (c.id.clone(), c.value.clone())).collect()
    }

    /// Modo do seletor.
    pub fn mode(&self) -> Mode {
        self.mode
    }

    fn new(mode: Mode, title: &str, dir: Option<&Path>, name: &str) -> Self {
        let dir = dir.filter(|d| d.is_dir()).map(Path::to_path_buf).unwrap_or_else(crate::places::home_dir);
        let mut chooser = Self {
            mode,
            title: title.into(),
            dir,
            entries: Vec::new(),
            selected: None,
            picked: Vec::new(),
            multiple: false,
            name: name.into(),
            filters: Vec::new(),
            filter: 0,
            choices: Vec::new(),
            accept_label: String::new(),
            embedded: false,
            save_names: Vec::new(),
            result: Vec::new(),
            show_hidden: false,
            message: None,
            focus: mode == Mode::Save,
            replace: None,
            back: Vec::new(),
            forward: Vec::new(),
            query: String::new(),
            new_folder: None,
        };
        chooser.reload();
        chooser
    }

    /// Pasta mostrada agora.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Entra numa pasta (a de antes vai para o "voltar").
    pub fn go(&mut self, dir: PathBuf) {
        if dir.is_dir() {
            if dir != self.dir {
                self.back.push(std::mem::replace(&mut self.dir, dir));
                self.forward.clear();
            }
            self.query.clear();
            self.reload();
        } else {
            self.message = Some("Pasta não encontrada.".into());
        }
    }

    fn reload(&mut self) {
        self.selected = None;
        self.picked.clear();
        self.message = None;
        self.entries = list(&self.dir, self.mode, self.filters.get(self.filter), self.show_hidden);
    }

    /// Índices escolhidos (o principal por último).
    fn picked_all(&self) -> Vec<usize> {
        let mut all = self.picked.clone();
        all.extend(self.selected.filter(|s| !all.contains(s)));
        all
    }

    /// Desenha o diálogo; `Some` quando o usuário escolheu ou cancelou.
    pub fn show(&mut self, ui: &mut Ui) -> Option<Choice> {
        let esc = ui.input().key(Key::Escape);
        let enter = ui.input().key(Key::Enter);
        // Confirmação de substituir, por cima do seletor.
        if let Some(path) = self.replace.clone() {
            // O aviso não tem campo: o Enter e o Esc são dele (o nome perde o foco).
            ui.blur();
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let taken = if self.mode == Mode::SaveFiles { self.result.iter().filter(|p| p.exists()).count() } else { 1 };
            let (title, message) = if taken > 1 {
                (format!("Substituir {taken} arquivos?"), "Já existem arquivos com esses nomes nesta pasta. Substituir troca o conteúdo deles.".to_string())
            } else {
                (format!("Substituir \"{name}\"?"), "Já existe um arquivo com esse nome nesta pasta. Substituir troca o conteúdo dele.".to_string())
            };
            return match ui.alert(&title, &message, &["Cancelar", "Substituir"]) {
                Some(1) => {
                    if self.mode != Mode::SaveFiles {
                        self.result = vec![path.clone()];
                    }
                    Some(Choice::Chosen(self.result.first().cloned().unwrap_or(path)))
                }
                Some(_) => {
                    // De volta ao nome, para corrigir.
                    self.replace = None;
                    ui.focus_next_input();
                    None
                }
                None => None,
            };
        }
        let mut action: Option<Action> = None;
        let (ww, wh) = ui.window_size();
        let width = if self.embedded { ww } else { (ww - 48.0).clamp(420.0, 820.0) };
        let row = ui.theme().row + ui.theme().spacing;
        // Linhas embaixo da lista: filtros, opções, aviso, nova pasta e botões.
        let extra = (self.filters.len() > 1) as usize as f32 * row
            + self.choices.len() as f32 * row
            + (self.mode == Mode::SaveFiles) as usize as f32 * 24.0
            + self.message.is_some() as usize as f32 * 24.0
            + self.new_folder.is_some() as usize as f32 * row;
        let top_rows = if self.mode == Mode::Save { row } else { 0.0 };
        let list_h = (wh * 0.72 - 150.0 - extra - top_rows).clamp(160.0, 420.0);
        let title = self.title.clone();
        let (ctrl, shift) = (ui.input().ctrl, ui.input().shift);
        let embedded = self.embedded;
        let sidebar_w = if width < 560.0 { 0.0 } else { 172.0 };
        let body = |ui: &mut Ui| {
            if !embedded {
                ui.strong(&title);
            }
            // Salvar: o nome em cima, como no Mac.
            if self.mode == Mode::Save {
                if std::mem::take(&mut self.focus) {
                    ui.focus_next_input();
                }
                ui.form_row("Salvar como:", 110.0, |ui| {
                    if ui.text_input(&mut self.name, "Nome do arquivo").submitted() {
                        action = Some(Action::Confirm);
                    }
                });
            }
            let list_h = if embedded { (ui.remaining_height() - extra - row - 2.0 * row - ui.theme().padding).max(140.0) } else { list_h };
            let picked = self.picked_all();
            let entries = &self.entries;
            let folders_only = matches!(self.mode, Mode::Folder | Mode::SaveFiles);
            let dir = self.dir.clone();
            let query = &mut self.query;
            let (has_back, has_forward) = (!self.back.is_empty(), !self.forward.is_empty());
            let sidebar = |ui: &mut Ui, action: &mut Option<Action>| {
                ui.heading("Favoritos");
                for (label, icon, place) in places() {
                    if label == "Computador" {
                        ui.heading("Locais");
                    }
                    let current = place == dir;
                    if ui.list_row(icon, label, &[], current).clicked() && !current {
                        *action = Some(Action::Go(place));
                    }
                }
            };
            let mut main = |ui: &mut Ui, action: &mut Option<Action>| {
                // Barra: voltar/avançar, o nome da pasta, subir e a busca.
                ui.row(|ui| {
                    match ui.toolbar_buttons(&[("go-previous-symbolic", "‹", false, has_back), ("go-next-symbolic", "›", false, has_forward)]) {
                        Some(0) => *action = Some(Action::Back),
                        Some(1) => *action = Some(Action::Forward),
                        _ => {}
                    }
                    let up = dir.parent().map(Path::to_path_buf);
                    if ui.toolbar_buttons(&[("go-up-symbolic", "↑", false, up.is_some())]).is_some() {
                        *action = up.map(Action::Go);
                    }
                    let name = dir.file_name().map_or_else(|| "Computador".to_string(), |n| n.to_string_lossy().into_owned());
                    let name = if dir == crate::places::home_dir() { "Pasta pessoal".to_string() } else { name };
                    ui.toolbar_title(&name, 220.0);
                    ui.push_right(190.0);
                    ui.sized(190.0, |ui| {
                        ui.search_field(query, "Buscar");
                    });
                });
                let q = crate::apps::fold(query.trim());
                let shown: Vec<usize> = (0..entries.len()).filter(|&i| q.is_empty() || crate::apps::fold(&entries[i].name).contains(&q)).collect();
                ui.list_header("Nome", &[("Tamanho", 90.0), ("Modificado", 130.0)], (0, true));
                ui.scroll_area(list_h, |ui| {
                    if shown.is_empty() {
                        ui.note(if !q.is_empty() {
                            "Nada com esse nome aqui."
                        } else if folders_only {
                            "Nenhuma pasta aqui."
                        } else {
                            "Pasta vazia."
                        });
                    }
                    for &i in &shown {
                        let entry = &entries[i];
                        let size = entry.size.map(crate::files::size_text).unwrap_or_else(|| "—".into());
                        let date = entry.modified.map(|m| crate::files::date_text(m, std::time::SystemTime::now())).unwrap_or_default();
                        let icon = if entry.is_dir { "folder" } else { file_icon(&entry.path) };
                        let r = ui.list_row(icon, &entry.name, &[(&size, 90.0), (&date, 130.0)], picked.contains(&i));
                        if r.double_clicked() {
                            *action = Some(if entry.is_dir { Action::Go(entry.path.clone()) } else { Action::Pick(entry.path.clone()) });
                        } else if r.clicked() {
                            *action = Some(Action::Select(i));
                        }
                    }
                });
            };
            if sidebar_w > 0.0 {
                let mut a1 = None;
                let mut a2 = None;
                ui.columns(sidebar_w, 16.0, |ui| sidebar(ui, &mut a1), |ui| main(ui, &mut a2));
                action = a2.or(a1).or(action.take());
            } else {
                let mut a2 = None;
                main(ui, &mut a2);
                action = a2.or(action.take());
            }
            if self.mode == Mode::SaveFiles {
                let n = self.save_names.len();
                ui.note(&if n == 1 { "1 arquivo será salvo na pasta escolhida.".to_string() } else { format!("{n} arquivos serão salvos na pasta escolhida.") });
            }
            if self.filters.len() > 1 {
                let names: Vec<String> = self.filters.iter().map(|f| f.name.clone()).collect();
                let mut current = self.filter;
                if ui.dropdown("Mostrar", &names, &mut current).changed() {
                    action = Some(Action::Filter(current));
                }
            }
            for choice in &mut self.choices {
                if choice.options.is_empty() {
                    let mut on = choice.value == "true";
                    if ui.switch(&choice.label, &mut on).changed() {
                        choice.value = on.to_string();
                    }
                } else {
                    let names: Vec<String> = choice.options.iter().map(|(_, n)| n.clone()).collect();
                    let mut current = choice.options.iter().position(|(id, _)| *id == choice.value).unwrap_or(0);
                    if ui.dropdown(&choice.label, &names, &mut current).changed() {
                        choice.value = choice.options[current].0.clone();
                    }
                }
            }
            // Nova pasta: o nome numa linha, com Criar.
            if let Some(name) = self.new_folder.as_mut() {
                ui.form_row("Nova pasta:", 110.0, |ui| {
                    ui.sized(260.0, |ui| {
                        if ui.text_input(name, "sem título").submitted() {
                            action = Some(Action::CreateFolder);
                        }
                    });
                    ui.sized(90.0, |ui| {
                        if ui.button_styled("Criar", super::ButtonStyle::Bordered).clicked() {
                            action = Some(Action::CreateFolder);
                        }
                    });
                });
            }
            if let Some(message) = &self.message {
                ui.note(message);
            }
            ui.row(|ui| {
                let label = match self.mode {
                    _ if !self.accept_label.is_empty() => self.accept_label.as_str(),
                    Mode::Open => "Abrir",
                    Mode::Save | Mode::SaveFiles => "Salvar",
                    Mode::Folder => "Escolher",
                };
                // "Nova pasta" à esquerda (onde se salva ou escolhe uma pasta).
                if self.mode != Mode::Open && self.new_folder.is_none() {
                    ui.sized(120.0, |ui| {
                        if ui.button_styled("Nova pasta", super::ButtonStyle::Bordered).clicked() {
                            action = Some(Action::NewFolder);
                        }
                    });
                }
                ui.push_right(2.0 * 110.0 + 8.0);
                ui.sized(110.0, |ui| {
                    if ui.button("Cancelar").clicked() {
                        action = Some(Action::Cancel);
                    }
                });
                ui.sized(110.0, |ui| {
                    if ui.primary_button(label).clicked() {
                        action = Some(Action::Confirm);
                    }
                });
            });
        };
        if embedded {
            ui.page(body);
        } else {
            ui.modal(width, body);
        }
        if action.is_none() && esc && self.new_folder.is_some() {
            self.new_folder = None;
            ui.request_repaint();
            return None;
        }
        if action.is_none() {
            if esc {
                action = Some(Action::Cancel);
            } else if enter && !ui.typing() {
                action = Some(Action::Confirm);
            }
        }
        let choice = match action? {
            Action::Cancel => Some(Choice::Cancelled),
            Action::Go(dir) => {
                self.go(dir);
                None
            }
            Action::Filter(i) => {
                self.filter = i;
                let dir = self.dir.clone();
                self.go(dir);
                None
            }
            Action::Select(i) => {
                // Vários (Abrir com `multiple`): Ctrl junta ou tira; Shift pega do
                // principal até o clicado.
                if self.multiple && self.mode == Mode::Open && (ctrl || shift) {
                    if shift {
                        let from = self.selected.unwrap_or(i);
                        let (a, b) = (from.min(i), from.max(i));
                        self.picked = (a..=b).filter(|j| *j != i).collect();
                    } else if let Some(k) = self.picked_all().iter().position(|j| *j == i) {
                        let mut all = self.picked_all();
                        all.remove(k);
                        self.selected = all.pop();
                        self.picked = all;
                        ui.request_repaint();
                        return None;
                    } else {
                        self.picked = self.picked_all();
                    }
                } else {
                    self.picked.clear();
                }
                self.selected = Some(i);
                // Salvar: clicar num arquivo usa o nome dele.
                if let (Mode::Save, Some(entry)) = (self.mode, self.entries.get(i)) {
                    if !entry.is_dir {
                        self.name = entry.name.clone();
                    }
                }
                None
            }
            Action::Pick(path) => self.pick(path),
            Action::Back => {
                if let Some(prev) = self.back.pop() {
                    self.forward.push(std::mem::replace(&mut self.dir, prev));
                    self.query.clear();
                    self.reload();
                }
                None
            }
            Action::Forward => {
                if let Some(next) = self.forward.pop() {
                    self.back.push(std::mem::replace(&mut self.dir, next));
                    self.query.clear();
                    self.reload();
                }
                None
            }
            Action::NewFolder => {
                self.new_folder = Some("sem título".into());
                ui.focus_next_input_selecting("sem título".len());
                None
            }
            Action::CreateFolder => {
                let name = self.new_folder.take().unwrap_or_default();
                let name = name.trim();
                if name.is_empty() || name.contains('/') {
                    self.message = Some("Nome de pasta inválido.".into());
                    return None;
                }
                let path = self.dir.join(name);
                match std::fs::create_dir(&path) {
                    Ok(()) => self.go(path),
                    Err(err) => self.message = Some(format!("Não foi possível criar a pasta: {err}")),
                }
                None
            }
            Action::Confirm => self.confirm(),
        };
        ui.request_repaint();
        choice
    }

    /// Duplo clique num arquivo.
    fn pick(&mut self, path: PathBuf) -> Option<Choice> {
        match self.mode {
            Mode::Open => {
                self.result = vec![path.clone()];
                Some(Choice::Chosen(path))
            }
            Mode::Save => {
                self.name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                self.confirm()
            }
            Mode::Folder | Mode::SaveFiles => None,
        }
    }

    /// Botão principal (ou Enter).
    fn confirm(&mut self) -> Option<Choice> {
        let selected = self.selected.and_then(|i| self.entries.get(i)).cloned();
        match self.mode {
            Mode::Open => {
                // Vários: os arquivos escolhidos, na ordem da lista; só pastas: entra
                // na principal.
                let mut all = self.picked_all();
                all.sort_unstable();
                let files: Vec<PathBuf> = all.iter().filter_map(|i| self.entries.get(*i)).filter(|e| !e.is_dir).map(|e| e.path.clone()).collect();
                match (files.first().cloned(), selected) {
                    (Some(first), _) => {
                        self.result = files;
                        Some(Choice::Chosen(first))
                    }
                    (None, Some(entry)) if entry.is_dir => {
                        self.go(entry.path);
                        None
                    }
                    _ => {
                        self.message = Some("Escolha um arquivo.".into());
                        None
                    }
                }
            }
            Mode::Folder => {
                let dir = selected.filter(|e| e.is_dir).map(|e| e.path).unwrap_or_else(|| self.dir.clone());
                self.result = vec![dir.clone()];
                Some(Choice::Chosen(dir))
            }
            Mode::SaveFiles => {
                let dir = selected.filter(|e| e.is_dir).map(|e| e.path).unwrap_or_else(|| self.dir.clone());
                self.result = self.save_names.iter().map(|n| dir.join(n)).collect();
                if let Some(taken) = self.result.iter().find(|p| p.exists()).cloned() {
                    self.replace = Some(taken);
                    return None;
                }
                Some(Choice::Chosen(self.result.first().cloned().unwrap_or(dir)))
            }
            Mode::Save => {
                let name = self.name.trim();
                if name.is_empty() || name.contains('/') || name == "." || name == ".." {
                    self.message = Some(if name.contains('/') { "O nome não pode ter \"/\".".into() } else { "Escreva o nome do arquivo.".into() });
                    return None;
                }
                let path = self.dir.join(name);
                if path.is_dir() {
                    self.go(path);
                    return None;
                }
                if path.exists() {
                    self.replace = Some(path);
                    return None;
                }
                self.result = vec![path.clone()];
                Some(Choice::Chosen(path))
            }
        }
    }
}

enum Action {
    Back,
    Forward,
    NewFolder,
    CreateFolder,
    Filter(usize),
    Go(PathBuf),
    Select(usize),
    Pick(PathBuf),
    Confirm,
    Cancel,
}

/// Itens da pasta: pastas primeiro, depois arquivos, por nome (sem diferenciar
/// maiúsculas). Nos modos de pasta, só pastas; com um filtro, só os arquivos dele.
fn list(dir: &Path, mode: Mode, filter: Option<&Filter>, hidden: bool) -> Vec<Entry> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut entries: Vec<Entry> = read
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if !hidden && name.starts_with('.') {
                return None;
            }
            let path = e.path();
            let meta = std::fs::metadata(&path).ok()?;
            let is_dir = meta.is_dir();
            if !is_dir && matches!(mode, Mode::Folder | Mode::SaveFiles) {
                return None;
            }
            if filter.is_some_and(|f| !f.accepts(&path, is_dir)) {
                return None;
            }
            Some(Entry { name, path, is_dir, size: (!is_dir).then(|| meta.len()), modified: meta.modified().ok() })
        })
        .collect();
    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| crate::apps::fold(&a.name).cmp(&crate::apps::fold(&b.name))));
    entries
}

/// Lugares da barra lateral: os favoritos (pasta pessoal e as pastas do usuário que
/// existem) e o computador.
fn places() -> Vec<(&'static str, &'static str, PathBuf)> {
    let home = crate::places::home_dir();
    let mut out = vec![("Pasta pessoal", "user-home-symbolic", home.clone())];
    if let Some(desktop) = crate::places::desktop_dir() {
        out.push(("Área de trabalho", "user-desktop-symbolic", desktop));
    }
    for (label, icon, key) in [("Documentos", "folder-documents-symbolic", "DOCUMENTS"), ("Downloads", "folder-download-symbolic", ""), ("Imagens", "folder-pictures-symbolic", ""), ("Vídeos", "folder-videos-symbolic", ""), ("Músicas", "folder-music-symbolic", "MUSIC")] {
        let dir = match label {
            "Downloads" => crate::places::downloads_dir(),
            "Imagens" => crate::places::pictures_dir(),
            "Vídeos" => crate::places::videos_dir(),
            _ => match user_dir(&home, key) {
                Some(d) => d,
                None => continue,
            },
        };
        if dir.is_dir() && !out.iter().any(|(_, _, d)| *d == dir) {
            out.push((label, icon, dir));
        }
    }
    out.push(("Computador", "computer-symbolic", PathBuf::from("/")));
    out
}

fn user_dir(home: &Path, key: &str) -> Option<PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".config"));
    let text = std::fs::read_to_string(config.join("user-dirs.dirs")).ok()?;
    crate::places::parse_user_dir(&text, key, home)
}

fn file_icon(path: &Path) -> &'static str {
    let item = crate::places::Item { path: path.to_path_buf(), name: String::new(), is_dir: false, modified: std::time::SystemTime::UNIX_EPOCH };
    crate::places::icon_name_for(&item)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pastas_primeiro_e_filtro() {
        let dir = std::env::temp_dir().join(format!("mide-seletor-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("Zeta")).unwrap();
        std::fs::write(dir.join("b.txt"), "x").unwrap();
        std::fs::write(dir.join("A.md"), "x").unwrap();
        std::fs::write(dir.join(".oculto"), "x").unwrap();
        let names = |entries: Vec<Entry>| entries.into_iter().map(|e| e.name).collect::<Vec<_>>();
        assert_eq!(names(list(&dir, Mode::Open, None, false)), ["Zeta", "A.md", "b.txt"]);
        let txt = Filter { name: "Textos".into(), patterns: vec![Pattern::Glob("*.TXT".into())] };
        assert_eq!(names(list(&dir, Mode::Open, Some(&txt), false)), ["Zeta", "b.txt"]);
        assert_eq!(names(list(&dir, Mode::Folder, None, false)), ["Zeta"]);
        let markdown = Filter { name: "Markdown".into(), patterns: vec![Pattern::Mime("text/markdown".into())] };
        assert_eq!(names(list(&dir, Mode::Open, Some(&markdown), false)), ["Zeta", "A.md"]);
        let any_text = Filter { name: "Textos".into(), patterns: vec![Pattern::Mime("text/*".into())] };
        assert_eq!(names(list(&dir, Mode::Open, Some(&any_text), false)), ["Zeta", "A.md", "b.txt"]);
        let images = Filter { name: "Imagens".into(), patterns: vec![Pattern::Mime("image/*".into()), Pattern::Glob("*.xcf".into())] };
        assert_eq!(names(list(&dir, Mode::Open, Some(&images), false)), ["Zeta"]);
        let mut save = FileChooser::save("Salvar", Some(&dir), "b.txt");
        // Nome que já existe: pede para substituir em vez de escolher.
        assert_eq!(save.confirm(), None);
        assert_eq!(save.replace, Some(dir.join("b.txt")));
        save.replace = None;
        save.name = "novo.txt".into();
        assert_eq!(save.confirm(), Some(Choice::Chosen(dir.join("novo.txt"))));
        assert_eq!(save.chosen(), [dir.join("novo.txt")]);
        // Vários para abrir: os arquivos escolhidos, sem as pastas, na ordem da lista.
        let mut open = FileChooser::open("Abrir", Some(&dir)).multiple(true);
        open.selected = Some(2);
        open.picked = vec![0, 1];
        assert_eq!(open.confirm(), Some(Choice::Chosen(dir.join("A.md"))));
        assert_eq!(open.chosen(), [dir.join("A.md"), dir.join("b.txt")]);
        // Salvar vários: cada nome na pasta; um que já existe pede para substituir.
        let mut many = FileChooser::save_files("Salvar", Some(&dir), &["c.txt".into(), "d.txt".into()]);
        assert_eq!(many.confirm(), Some(Choice::Chosen(dir.join("c.txt"))));
        assert_eq!(many.chosen(), [dir.join("c.txt"), dir.join("d.txt")]);
        let mut many = FileChooser::save_files("Salvar", Some(&dir), &["b.txt".into()]);
        assert_eq!(many.confirm(), None);
        assert_eq!(many.replace, Some(dir.join("b.txt")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
