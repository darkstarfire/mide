//! Gaveta do projeto do Code: a árvore de pastas e arquivos à esquerda, com o nome do
//! projeto no topo, os arquivos abertos com fundo cinza leve (o da aba atual na cor de
//! destaque), o menu de clique direito (criar, renomear, duplicar, Lixeira, copiar o
//! caminho, mostrar no Arquivos, abrir no Terminal) e o começo do arrasto de um
//! arquivo para a borda do editor (que divide o editor).

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use mide::kit::{Key, Ui};

use super::{Code, Dialog, Drag, DragWhat};

/// Ação escolhida no menu de um item (ou do fundo) da gaveta.
#[derive(Debug, Clone)]
pub enum TreeAction {
    NewFile(PathBuf),
    NewFolder(PathBuf),
    Rename(PathBuf),
    Duplicate(PathBuf),
    Trash(PathBuf),
    CopyPath(PathBuf),
    Reveal(PathBuf),
    Terminal(PathBuf),
}

/// Linhas da árvore: (profundidade, caminho, é pasta), sem ocultos e pastas geradas.
fn rows(root: &Path, expanded: &HashSet<PathBuf>) -> Vec<(usize, PathBuf, bool)> {
    fn walk(dir: &Path, depth: usize, expanded: &HashSet<PathBuf>, rows: &mut Vec<(usize, PathBuf, bool)>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut list: Vec<(String, PathBuf, bool)> = entries
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                // Pasta de verdade (link para pasta também conta).
                let is_dir = e.path().is_dir();
                (!name.starts_with('.') && name != "target" && name != "node_modules").then(|| (name, e.path(), is_dir))
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
    let mut out = Vec::new();
    walk(root, 0, expanded, &mut out);
    out
}

/// Itens do menu de um arquivo ou pasta (`dir`: onde criar itens novos).
fn item_menu(m: &mut mide::kit::Menu, path: &Path, dir: &Path, chosen: &mut Option<TreeAction>) {
    if m.item_icon("document-new-symbolic", "Novo arquivo…", None) {
        *chosen = Some(TreeAction::NewFile(dir.to_path_buf()));
    }
    if m.item_icon("folder-new-symbolic", "Nova pasta…", None) {
        *chosen = Some(TreeAction::NewFolder(dir.to_path_buf()));
    }
    m.separator();
    if m.item("Renomear…") {
        *chosen = Some(TreeAction::Rename(path.to_path_buf()));
    }
    if m.item("Duplicar") {
        *chosen = Some(TreeAction::Duplicate(path.to_path_buf()));
    }
    if m.danger("Mover para a Lixeira") {
        *chosen = Some(TreeAction::Trash(path.to_path_buf()));
    }
    m.separator();
    folder_items(m, path, dir, chosen);
}

/// Itens que valem também para o fundo da gaveta (a pasta do projeto).
fn folder_items(m: &mut mide::kit::Menu, path: &Path, dir: &Path, chosen: &mut Option<TreeAction>) {
    if m.item("Copiar caminho") {
        *chosen = Some(TreeAction::CopyPath(path.to_path_buf()));
    }
    if m.item("Mostrar no Arquivos") {
        *chosen = Some(TreeAction::Reveal(path.to_path_buf()));
    }
    if m.item("Abrir no Terminal") {
        *chosen = Some(TreeAction::Terminal(dir.to_path_buf()));
    }
}

impl Code {
    /// Gaveta com a árvore do projeto, em (0, `top`) com `width` × `height`.
    pub(crate) fn drawer(&mut self, ui: &mut Ui, top: f32, width: f32, height: f32) {
        let Some(root) = self.root.clone() else {
            return;
        };
        let rows = rows(&root, &self.expanded);
        let current = self.docs.get(self.cur).and_then(|d| d.path.clone());
        let open: HashSet<PathBuf> = self.docs.iter().filter_map(|d| d.path.clone()).collect();
        let mut clicked: Option<(PathBuf, bool)> = None;
        let mut chosen: Option<TreeAction> = None;
        let mut pressed: Option<PathBuf> = None;
        let root_name = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| root.to_string_lossy().into_owned());
        ui.at(0.0, top, width, |ui| {
            ui.scroll_area_padded(height, 8.0, |ui| {
                // Nome do projeto, com margem (antes ficava colado na borda).
                ui.space(2.0);
                ui.row(|ui| {
                    ui.space_x(6.0);
                    ui.strong(&root_name);
                });
                ui.space(2.0);
                for (depth, path, is_dir) in &rows {
                    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let icon = if *is_dir {
                        "folder".to_string()
                    } else {
                        mide::places::icon_name_for(&mide::places::Item { path: path.clone(), name: name.clone(), is_dir: false, modified: std::time::SystemTime::UNIX_EPOCH }).to_string()
                    };
                    // A aba atual na cor de destaque; os outros abertos em cinza leve.
                    let selected = current.as_ref() == Some(path);
                    let opened = !selected && open.contains(path);
                    let (r, toggled) = ui.tree_row(*depth, *is_dir, self.expanded.contains(path), &icon, &name, &[], selected, opened);
                    if toggled || r.clicked() {
                        clicked = Some((path.clone(), *is_dir));
                    }
                    if r.hovered() && ui.input().pressed && !is_dir {
                        pressed = Some(path.clone());
                    }
                    let dir = if *is_dir { path.clone() } else { path.parent().map(Path::to_path_buf).unwrap_or_else(|| root.clone()) };
                    ui.context_menu(&r, |m| item_menu(m, path, &dir, &mut chosen));
                }
            });
            // Fundo da gaveta (fora dos itens): o menu da pasta do projeto.
            let bg = ui.background((0.0, top, width, height));
            ui.context_menu(&bg, |m| {
                if m.item_icon("document-new-symbolic", "Novo arquivo…", None) {
                    chosen = Some(TreeAction::NewFile(root.clone()));
                }
                if m.item_icon("folder-new-symbolic", "Nova pasta…", None) {
                    chosen = Some(TreeAction::NewFolder(root.clone()));
                }
                m.separator();
                folder_items(m, &root, &root, &mut chosen);
            });
        });
        if let Some(path) = pressed {
            // Começo de um arrasto possível (vira arrasto ao andar alguns pontos).
            self.drag = Some(Drag { what: DragWhat::Path(path), from: ui.input().pointer.unwrap_or_default(), active: false });
        }
        if let Some((path, is_dir)) = clicked {
            if is_dir {
                if !self.expanded.remove(&path) {
                    self.expanded.insert(path);
                }
            } else {
                self.open_path(path);
            }
            ui.request_repaint();
        }
        if let Some(action) = chosen {
            self.tree_action(action, ui);
        }
    }

    /// Faz a ação do menu da gaveta.
    pub(crate) fn tree_action(&mut self, action: TreeAction, ui: &mut Ui) {
        tracing::info!("gaveta: {action:?}");
        match action {
            TreeAction::NewFile(dir) => self.dialog = Some(Dialog::NewItem { dir, folder: false, name: String::new(), focus: true, error: None }),
            TreeAction::NewFolder(dir) => self.dialog = Some(Dialog::NewItem { dir, folder: true, name: String::new(), focus: true, error: None }),
            TreeAction::Rename(path) => {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                self.dialog = Some(Dialog::Rename { path, name, focus: true, error: None });
            }
            TreeAction::Duplicate(path) => match mide::files::duplicate(&path) {
                Ok(copy) => {
                    tracing::info!("duplicado: {}", copy.display());
                    self.say(format!("Duplicado como \"{}\"", copy.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()));
                }
                Err(err) => self.say(format!("Não foi possível duplicar: {err}")),
            },
            TreeAction::Trash(path) => match mide::places::move_to_trash(&path, &mide::places::trash_dir()) {
                Ok(_) => {
                    tracing::info!("para a Lixeira: {}", path.display());
                    // Abas dos arquivos que foram para a Lixeira (sem alterações) fecham.
                    let mut i = self.docs.len();
                    while i > 0 {
                        i -= 1;
                        if self.docs[i].path.as_ref().is_some_and(|p| p.starts_with(&path)) && !self.docs[i].text.modified {
                            self.close_tab(i, true);
                        }
                    }
                    self.say(format!("\"{}\" foi para a Lixeira", path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()));
                }
                Err(err) => self.say(format!("Não foi possível mover para a Lixeira: {err}")),
            },
            TreeAction::CopyPath(path) => {
                let text = path.to_string_lossy().into_owned();
                ui.copy(&text);
                mide::places::copy_text(&text);
                self.say("Caminho copiado");
            }
            TreeAction::Reveal(path) => show_in_files(&path),
            TreeAction::Terminal(dir) => {
                // Com o terminal integrado, abre nele; senão, o Terminal do mide.
                if !self.open_integrated_terminal(Some(dir.clone()), ui) {
                    mide::places::open_terminal(&dir);
                }
            }
        }
        ui.request_repaint();
    }

    /// Diálogo do nome de um item novo ou renomeado. Devolve se tratou um diálogo
    /// desses (os outros ficam com `dialogs`).
    pub(crate) fn name_dialog(&mut self, ui: &mut Ui) -> bool {
        let esc = ui.input().key(Key::Escape);
        let (title, value, focus, error) = match self.dialog.as_mut() {
            Some(Dialog::NewItem { folder, name, focus, error, .. }) => (if *folder { "Nova pasta" } else { "Novo arquivo" }, name, focus, error),
            Some(Dialog::Rename { name, focus, error, .. }) => ("Renomear", name, focus, error),
            _ => return false,
        };
        let mut ok = false;
        let mut cancel = esc;
        let first = std::mem::take(focus);
        ui.modal(380.0, |ui| {
            ui.strong(title);
            if first {
                // Ao renomear, o nome sem a extensão já vem escolhido (como no Finder).
                let end = value.rfind('.').filter(|&i| i > 0).unwrap_or(value.len());
                ui.focus_next_input_selecting(end);
            }
            ok |= ui.text_input(value, "Nome").submitted();
            if let Some(e) = error.as_ref() {
                ui.error_note(e);
            }
            match ui.dialog_buttons(&["Cancelar", if title == "Renomear" { "Renomear" } else { "Criar" }]) {
                Some(0) => cancel = true,
                Some(_) => ok = true,
                None => {}
            }
        });
        if cancel {
            self.dialog = None;
            return true;
        }
        if !ok {
            return true;
        }
        match self.dialog.take() {
            Some(Dialog::NewItem { dir, folder, name, .. }) => match mide::files::create_named(&dir, &name, folder) {
                Ok(path) => {
                    tracing::info!("criado: {}", path.display());
                    if folder {
                        self.expanded.insert(path);
                    } else {
                        self.open_path(path);
                    }
                    self.expanded.insert(dir);
                }
                Err(e) => self.dialog = Some(Dialog::NewItem { dir, folder, name, focus: false, error: Some(e) }),
            },
            Some(Dialog::Rename { path, name, .. }) => match mide::files::rename(&path, &name) {
                Ok(target) => {
                    tracing::info!("renomeado: {} → {}", path.display(), target.display());
                    // Abas e pastas abertas acompanham o nome novo.
                    for doc in &mut self.docs {
                        if let Some(p) = doc.path.as_mut().filter(|p| p.starts_with(&path)) {
                            let rest = p.strip_prefix(&path).map(Path::to_path_buf).unwrap_or_default();
                            *p = if rest.as_os_str().is_empty() { target.clone() } else { target.join(rest) };
                            doc.lsp_seen = None;
                        }
                    }
                    self.expanded = self.expanded.drain().map(|d| if d.starts_with(&path) { target.join(d.strip_prefix(&path).unwrap_or(Path::new(""))) } else { d }).collect();
                }
                Err(e) => self.dialog = Some(Dialog::Rename { path, name, focus: false, error: Some(e) }),
            },
            other => self.dialog = other,
        }
        ui.request_repaint();
        true
    }
}

/// Mostra o item no Arquivos do mide, escolhido (nos testes, só registra).
pub fn show_in_files(path: &Path) {
    if std::env::var_os("MIDE_OPEN_DRY_RUN").is_some() {
        tracing::info!("mostrar no Arquivos (teste): {}", path.display());
        return;
    }
    match std::process::Command::new(mide::places::mide_program("mide-files")).arg("--select").arg(path).spawn() {
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            tracing::info!("mostrando no Arquivos: {}", path.display());
        }
        Err(err) => tracing::warn!("não abriu o Arquivos: {err}"),
    }
}
