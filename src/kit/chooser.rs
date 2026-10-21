//! Seletor de arquivos do mide-ui: "Abrir…", "Salvar como…" e "Escolher pasta…" num
//! diálogo por cima da janela do app, como o do macOS (lugares do lado, a pasta atual
//! em lista, pastas primeiro).
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
}

pub struct FileChooser {
    mode: Mode,
    title: String,
    dir: PathBuf,
    entries: Vec<Entry>,
    selected: Option<usize>,
    /// Nome do arquivo (modo Salvar).
    name: String,
    /// Extensões aceitas no modo Abrir (vazio: todas).
    extensions: Vec<String>,
    show_hidden: bool,
    message: Option<String>,
    /// Pede o foco no campo do nome no primeiro quadro (modo Salvar).
    focus: bool,
    /// Pede confirmação para substituir este arquivo (modo Salvar).
    replace: Option<PathBuf>,
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
        self.extensions = extensions.iter().map(|e| e.to_lowercase()).collect();
        self.reload();
        self
    }

    fn new(mode: Mode, title: &str, dir: Option<&Path>, name: &str) -> Self {
        let dir = dir.filter(|d| d.is_dir()).map(Path::to_path_buf).unwrap_or_else(crate::places::home_dir);
        let mut chooser = Self {
            mode,
            title: title.into(),
            dir,
            entries: Vec::new(),
            selected: None,
            name: name.into(),
            extensions: Vec::new(),
            show_hidden: false,
            message: None,
            focus: mode == Mode::Save,
            replace: None,
        };
        chooser.reload();
        chooser
    }

    /// Pasta mostrada agora.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Entra numa pasta.
    pub fn go(&mut self, dir: PathBuf) {
        if dir.is_dir() {
            self.dir = dir;
            self.reload();
        } else {
            self.message = Some("Pasta não encontrada.".into());
        }
    }

    fn reload(&mut self) {
        self.selected = None;
        self.message = None;
        self.entries = list(&self.dir, self.mode, &self.extensions, self.show_hidden);
    }

    /// Desenha o diálogo; `Some` quando o usuário escolheu ou cancelou.
    pub fn show(&mut self, ui: &mut Ui) -> Option<Choice> {
        let esc = ui.input().key(Key::Escape);
        let enter = ui.input().key(Key::Enter);
        // Confirmação de substituir, por cima do seletor.
        if let Some(path) = self.replace.clone() {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let message = "Já existe um arquivo com esse nome nesta pasta. Substituir troca o conteúdo dele.";
            return match ui.alert(&format!("Substituir \"{name}\"?"), message, &["Cancelar", "Substituir"]) {
                Some(1) => Some(Choice::Chosen(path)),
                Some(_) => {
                    self.replace = None;
                    None
                }
                None => None,
            };
        }
        let mut action: Option<Action> = None;
        let (ww, wh) = ui.window_size();
        let width = (ww - 48.0).clamp(360.0, 720.0);
        let list_h = (wh * 0.62 - if self.mode == Mode::Save { 150.0 } else { 110.0 }).clamp(120.0, 380.0);
        let title = self.title.clone();
        ui.modal(width, |ui| {
            ui.strong(&title);
            // Voltar para a pasta de cima, o caminho e os lugares.
            ui.row(|ui| {
                let up = self.dir.parent().map(Path::to_path_buf);
                if ui.icon_button("go-up", "↑", up.is_some()).clicked() {
                    action = up.map(Action::Go);
                }
                // Caminho longo: fica o fim (a pasta atual), com reticências no começo.
                let shown = pretty_dir(&self.dir);
                let chars: Vec<char> = shown.chars().collect();
                let room = ((width - 80.0) / 8.0).max(12.0) as usize;
                let shown = if chars.len() > room { format!("…{}", chars[chars.len() - room + 1..].iter().collect::<String>()) } else { shown };
                ui.label(&shown);
            });
            ui.wrap(|ui| {
                for (label, dir) in places() {
                    let current = dir == self.dir;
                    let r = if current { ui.primary_button(label) } else { ui.button(label) };
                    if r.clicked() && !current {
                        action = Some(Action::Go(dir));
                    }
                }
            });
            let selected = self.selected;
            let entries = &self.entries;
            ui.scroll_area(list_h, |ui| {
                if entries.is_empty() {
                    ui.note(if self.mode == Mode::Folder { "Nenhuma pasta aqui." } else { "Pasta vazia." });
                }
                for (i, entry) in entries.iter().enumerate() {
                    let size = entry.size.map(crate::files::size_text).unwrap_or_default();
                    let icon = if entry.is_dir { "folder" } else { file_icon(&entry.path) };
                    let r = ui.list_row(icon, &entry.name, &[(&size, 90.0)], selected == Some(i));
                    if r.double_clicked() {
                        action = Some(if entry.is_dir { Action::Go(entry.path.clone()) } else { Action::Pick(entry.path.clone()) });
                    } else if r.clicked() {
                        action = Some(Action::Select(i));
                    }
                }
            });
            if self.mode == Mode::Save {
                if std::mem::take(&mut self.focus) {
                    ui.focus_next_input();
                }
                if ui.text_input(&mut self.name, "Nome do arquivo").submitted() {
                    action = Some(Action::Confirm);
                }
            }
            if let Some(message) = &self.message {
                ui.note(message);
            }
            ui.row(|ui| {
                let label = match self.mode {
                    Mode::Open => "Abrir",
                    Mode::Save => "Salvar",
                    Mode::Folder => "Escolher",
                };
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
        });
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
            Action::Select(i) => {
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
            Action::Confirm => self.confirm(),
        };
        ui.request_repaint();
        choice
    }

    /// Duplo clique num arquivo.
    fn pick(&mut self, path: PathBuf) -> Option<Choice> {
        match self.mode {
            Mode::Open => Some(Choice::Chosen(path)),
            Mode::Save => {
                self.name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                self.confirm()
            }
            Mode::Folder => None,
        }
    }

    /// Botão principal (ou Enter).
    fn confirm(&mut self) -> Option<Choice> {
        let selected = self.selected.and_then(|i| self.entries.get(i)).cloned();
        match self.mode {
            Mode::Open => match selected {
                Some(entry) if entry.is_dir => {
                    self.go(entry.path);
                    None
                }
                Some(entry) => Some(Choice::Chosen(entry.path)),
                None => {
                    self.message = Some("Escolha um arquivo.".into());
                    None
                }
            },
            Mode::Folder => Some(Choice::Chosen(selected.filter(|e| e.is_dir).map(|e| e.path).unwrap_or_else(|| self.dir.clone()))),
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
                Some(Choice::Chosen(path))
            }
        }
    }
}

enum Action {
    Go(PathBuf),
    Select(usize),
    Pick(PathBuf),
    Confirm,
    Cancel,
}

/// Itens da pasta: pastas primeiro, depois arquivos, por nome (sem diferenciar
/// maiúsculas). No modo Pasta, só pastas; com extensões, só os arquivos delas.
fn list(dir: &Path, mode: Mode, extensions: &[String], hidden: bool) -> Vec<Entry> {
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
            if !is_dir && mode == Mode::Folder {
                return None;
            }
            if !is_dir && !extensions.is_empty() {
                let ext = path.extension().map(|x| x.to_string_lossy().to_lowercase()).unwrap_or_default();
                if !extensions.contains(&ext) {
                    return None;
                }
            }
            Some(Entry { name, path, is_dir, size: (!is_dir).then(|| meta.len()) })
        })
        .collect();
    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| crate::apps::fold(&a.name).cmp(&crate::apps::fold(&b.name))));
    entries
}

/// Lugares do lado: pasta pessoal e as pastas do usuário que existem.
fn places() -> Vec<(&'static str, PathBuf)> {
    let home = crate::places::home_dir();
    let mut out = vec![("Pasta pessoal", home.clone())];
    if let Some(desktop) = crate::places::desktop_dir() {
        out.push(("Área de trabalho", desktop));
    }
    for (label, key, fallback) in [("Documentos", "DOCUMENTS", "Documentos"), ("Downloads", "", ""), ("Imagens", "", "")] {
        let dir = match label {
            "Downloads" => crate::places::downloads_dir(),
            "Imagens" => crate::places::pictures_dir(),
            _ => user_dir(&home, key).unwrap_or_else(|| home.join(fallback)),
        };
        if dir.is_dir() && !out.iter().any(|(_, d)| *d == dir) {
            out.push((label, dir));
        }
    }
    out
}

fn user_dir(home: &Path, key: &str) -> Option<PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".config"));
    let text = std::fs::read_to_string(config.join("user-dirs.dirs")).ok()?;
    crate::places::parse_user_dir(&text, key, home)
}

/// Caminho com `~` no lugar da pasta pessoal.
fn pretty_dir(dir: &Path) -> String {
    let home = crate::places::home_dir();
    match dir.strip_prefix(&home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".into(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => dir.display().to_string(),
    }
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
        assert_eq!(names(list(&dir, Mode::Open, &[], false)), ["Zeta", "A.md", "b.txt"]);
        assert_eq!(names(list(&dir, Mode::Open, &["txt".into()], false)), ["Zeta", "b.txt"]);
        assert_eq!(names(list(&dir, Mode::Folder, &[], false)), ["Zeta"]);
        let mut save = FileChooser::save("Salvar", Some(&dir), "b.txt");
        // Nome que já existe: pede para substituir em vez de escolher.
        assert_eq!(save.confirm(), None);
        assert_eq!(save.replace, Some(dir.join("b.txt")));
        save.replace = None;
        save.name = "novo.txt".into();
        assert_eq!(save.confirm(), Some(Choice::Chosen(dir.join("novo.txt"))));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
