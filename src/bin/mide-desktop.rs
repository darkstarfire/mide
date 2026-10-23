//! Área de trabalho do mide: os arquivos e pastas da pasta "Área de trabalho" como
//! ícones sobre o papel de parede, em colunas a partir do canto direito (como no
//! macOS). Feito com o mide-ui, numa camada de fundo (atrás das janelas).
//!
//! Clique seleciona, duplo clique abre (pastas no Arquivos). Botão direito num ícone:
//! Abrir, Renomear, Duplicar, Copiar caminho e Mover para a Lixeira; no vazio: Nova
//! pasta, Novo arquivo, Abrir no Terminal, Mostrar no Arquivos e Mudar papel de
//! parede. Teclado (depois de clicar na área de trabalho): Enter abre, Delete manda
//! para a Lixeira, Ctrl+R renomeia.
//!
//! Programas (E8): atalhos `.desktop` aparecem com o nome e o ícone do app; atalhos,
//! AppImages, executáveis e scripts rodam com duplo clique (ou perguntam antes, como
//! no Arquivos: `mide::programs`).
//!
//! `MIDE_DESKTOP_DIR` troca a pasta (os testes usam uma pasta temporária).

use std::{error::Error, path::PathBuf, time::Duration};

use mide::{
    files::{self, Entry},
    kit::{run, App, Key, Theme, Tile, Ui, Window},
    places,
    programs::{self, Answer, Labels, Opened, Prompt},
};

/// Ícones de 56 px; cada ladrilho ocupa 100 x 102.
const ICON: u32 = 56;
const CELL: (f32, f32) = (104.0, 106.0);
const MARGIN: f32 = 14.0;

struct Desktop {
    dir: Option<PathBuf>,
    entries: Vec<Entry>,
    mtime: Option<std::time::SystemTime>,
    selected: Option<PathBuf>,
    dialog: Option<Dialog>,
    focus_dialog: bool,
    /// Área útil da tela (sem a barra e a dock), perguntada ao compositor.
    workarea: Option<(f32, f32, f32, f32)>,
    asked_at: Option<std::time::Instant>,
    /// Confirmação antes de rodar um programa (E8).
    prompt: Option<Prompt>,
    /// Nome e ícone dos atalhos `.desktop`, no lugar do nome do arquivo.
    labels: Labels,
}

/// Diálogo de criar ou renomear (o mesmo jeito do Arquivos).
struct Dialog {
    kind: DialogKind,
    suggestion: String,
    name: String,
    error: Option<String>,
}

enum DialogKind {
    NewFolder,
    NewFile,
    Rename(PathBuf),
}

#[derive(Clone, Copy)]
enum Action {
    Open,
    Rename,
    Duplicate,
    CopyPath,
    Trash,
    NewFolder,
    NewFile,
    Terminal,
    ShowFolder,
    Wallpaper,
}

impl Desktop {
    fn new() -> Self {
        let mut app = Self { dir: places::desktop_dir(), entries: Vec::new(), mtime: None, selected: None, dialog: None, focus_dialog: false, workarea: None, asked_at: None, prompt: None, labels: Labels::default() };
        app.reload();
        app
    }

    /// Relê a pasta (quando ela muda, ou depois de criar, renomear ou apagar).
    fn reload(&mut self) {
        let Some(dir) = &self.dir else {
            return;
        };
        self.mtime = std::fs::metadata(dir).and_then(|m| m.modified()).ok();
        self.entries = files::list_dir(dir, false).unwrap_or_default();
        if self.selected.as_ref().is_some_and(|s| !s.exists()) {
            self.selected = None;
        }
    }

    /// Área útil (sem barra e dock), para os ícones não ficarem embaixo delas.
    fn ask_workarea(&mut self) {
        let parsed = mide::control_client::ask_compositor("workarea").and_then(|lines| {
            let n: Vec<f32> = lines.first()?.split_whitespace().filter_map(|v| v.parse().ok()).collect();
            (n.len() == 4).then(|| (n[0], n[1], n[2], n[3]))
        });
        self.workarea = parsed;
    }

    fn changed(&self) -> bool {
        self.dir.as_ref().is_some_and(|d| std::fs::metadata(d).and_then(|m| m.modified()).ok() != self.mtime)
    }

    fn open_dialog(&mut self, kind: DialogKind, name: String) {
        self.dialog = Some(Dialog { kind, suggestion: name.clone(), name, error: None });
        self.focus_dialog = true;
    }

    /// Criar só com "Criar": sem mudar o nome, o próximo livre ("Nova pasta (1)"...).
    fn confirm_dialog(&mut self) {
        let (Some(dialog), Some(dir)) = (self.dialog.as_ref(), self.dir.clone()) else {
            return;
        };
        let result = match &dialog.kind {
            DialogKind::Rename(path) => files::rename(path, &dialog.name),
            kind => {
                let folder = matches!(kind, DialogKind::NewFolder);
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
                tracing::info!("área de trabalho: {}", path.display());
                self.dialog = None;
                self.reload();
                self.selected = Some(path);
            }
            Err(err) => {
                if let Some(d) = self.dialog.as_mut() {
                    d.error = Some(err);
                }
            }
        }
    }

    fn act(&mut self, action: Action, entry: Option<Entry>) {
        let dir = self.dir.clone();
        match (action, entry) {
            (Action::Open, Some(e)) if e.is_dir => places::open(&e.path.to_string_lossy()),
            // Programas (.desktop, AppImage, executáveis, scripts) rodam ou perguntam antes.
            (Action::Open, Some(e)) => match programs::open(&e.path) {
                Opened::Done => {}
                Opened::Ask(prompt) => self.prompt = Some(prompt),
                Opened::Failed(err) => tracing::warn!("{err}"),
            },
            (Action::Rename, Some(e)) => self.open_dialog(DialogKind::Rename(e.path.clone()), e.name.clone()),
            (Action::Duplicate, Some(e)) => {
                if let Ok(copy) = files::duplicate(&e.path) {
                    self.reload();
                    self.selected = Some(copy);
                }
            }
            (Action::CopyPath, Some(e)) => places::copy_text(&e.path.to_string_lossy()),
            (Action::Trash, Some(e)) => {
                match places::move_to_trash(&e.path, &places::trash_dir()) {
                    Ok(_) => tracing::info!("para a Lixeira: {}", e.path.display()),
                    Err(err) => tracing::warn!("não foi para a Lixeira: {err}"),
                }
                self.reload();
            }
            (Action::NewFolder, _) => self.open_dialog(DialogKind::NewFolder, "Nova pasta".into()),
            (Action::NewFile, _) => self.open_dialog(DialogKind::NewFile, "Novo arquivo.txt".into()),
            (Action::Terminal, _) => {
                if let Some(dir) = dir {
                    places::open_terminal(&dir);
                }
            }
            (Action::ShowFolder, _) => {
                if let Some(dir) = dir {
                    places::open(&dir.to_string_lossy());
                }
            }
            (Action::Wallpaper, _) => mide::system::open_settings("wallpaper"),
            _ => {}
        }
    }

    /// Posição do ícone `i`: colunas de cima para baixo, a partir da direita.
    fn place(i: usize, area: (f32, f32, f32, f32)) -> (f32, f32) {
        let (x, y, width, height) = area;
        let rows = (((height - 2.0 * MARGIN) / CELL.1).floor() as usize).max(1);
        let (col, row) = (i / rows, i % rows);
        (x + width - MARGIN - (col as f32 + 1.0) * CELL.0, y + MARGIN + row as f32 * CELL.1)
    }

    fn keys(&mut self, ui: &mut Ui) {
        if ui.typing() || self.dialog.is_some() || self.prompt.is_some() {
            return;
        }
        let input = ui.input();
        let (enter, delete, rename) = (input.key(Key::Enter), input.key(Key::Delete), input.shortcut("r"));
        let chosen = self.selected.as_ref().and_then(|s| self.entries.iter().find(|e| &e.path == s)).cloned();
        if let Some(entry) = chosen {
            if enter {
                self.act(Action::Open, Some(entry));
            } else if delete {
                self.act(Action::Trash, Some(entry));
            } else if rename {
                self.act(Action::Rename, Some(entry));
            }
            ui.request_repaint();
        }
    }

    fn dialog_view(&mut self, ui: &mut Ui) {
        let Some(dialog) = self.dialog.as_ref() else {
            return;
        };
        let (title, action) = match dialog.kind {
            DialogKind::NewFolder => ("Nova pasta", "Criar"),
            DialogKind::NewFile => ("Novo arquivo", "Criar"),
            DialogKind::Rename(_) => ("Renomear", "Renomear"),
        };
        let focus = std::mem::take(&mut self.focus_dialog);
        let mut name = dialog.name.clone();
        let error = dialog.error.clone();
        let mut done = false;
        let mut cancel = ui.input().key(Key::Escape);
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
            self.dialog = None;
            ui.request_repaint();
        } else if done {
            self.confirm_dialog();
            ui.request_repaint();
        }
    }
}

impl App for Desktop {
    fn view(&mut self, ui: &mut Ui) {
        // A pasta pode mudar por fora (Arquivos, terminal): confere de tempos em tempos.
        if self.changed() {
            self.reload();
        }
        // A área útil muda quando a dock muda de tamanho ou passa a se esconder
        // (pergunta no máximo a cada 1,5 s).
        if self.asked_at.is_none_or(|at| at.elapsed() > Duration::from_millis(1500)) {
            self.ask_workarea();
            self.asked_at = Some(std::time::Instant::now());
        }
        ui.wake_after(Duration::from_secs(2));
        self.keys(ui);
        let region = ui.region();
        let area = self.workarea.unwrap_or(region);
        let mut action: Option<(Action, Option<Entry>)> = None;
        let mut select = None;
        let entries = self.entries.clone();
        for (i, entry) in entries.iter().enumerate() {
            let (x, y) = Self::place(i, area);
            let selected = self.selected.as_ref() == Some(&entry.path);
            let mut response = None;
            let (name, icon) = self.labels.get(&entry.path, entry.modified).unwrap_or_else(|| (entry.name.clone(), entry.icon_name().to_string()));
            ui.at(x, y, CELL.0, |ui| {
                response = Some(ui.tile(Tile::Icon(&icon), &name, selected, ICON));
            });
            let Some(r) = response else {
                continue;
            };
            if r.clicked() || r.secondary_clicked() {
                select = Some(entry.path.clone());
            }
            if r.double_clicked() {
                action = Some((Action::Open, Some(entry.clone())));
            }
            ui.context_menu(&r, |m| {
                let mut chosen = None;
                if m.item("Abrir") {
                    chosen = Some(Action::Open);
                }
                m.separator();
                if m.item("Renomear…") {
                    chosen = Some(Action::Rename);
                }
                if m.item("Duplicar") {
                    chosen = Some(Action::Duplicate);
                }
                if m.item("Copiar caminho") {
                    chosen = Some(Action::CopyPath);
                }
                m.separator();
                if m.danger("Mover para a Lixeira") {
                    chosen = Some(Action::Trash);
                }
                if let Some(chosen) = chosen {
                    action = Some((chosen, Some(entry.clone())));
                }
            });
        }
        if let Some(path) = select {
            self.selected = Some(path);
            ui.request_repaint();
        }
        // Fundo: clique tira a seleção; botão direito abre o menu da área de trabalho.
        let area = ui.region();
        let bg = ui.background(area);
        if bg.clicked() && self.selected.is_some() {
            self.selected = None;
            ui.request_repaint();
        }
        let has_dir = self.dir.is_some();
        ui.context_menu(&bg, |m| {
            let mut chosen = None;
            if has_dir {
                if m.item("Nova pasta") {
                    chosen = Some(Action::NewFolder);
                }
                if m.item("Novo arquivo") {
                    chosen = Some(Action::NewFile);
                }
                m.separator();
                if m.item("Abrir no Terminal") {
                    chosen = Some(Action::Terminal);
                }
                if m.item("Mostrar no Arquivos") {
                    chosen = Some(Action::ShowFolder);
                }
                m.separator();
            }
            if m.item("Mudar papel de parede…") {
                chosen = Some(Action::Wallpaper);
            }
            if let Some(chosen) = chosen {
                action = Some((chosen, None));
            }
        });
        if let Some((a, entry)) = action {
            self.act(a, entry);
            ui.request_repaint();
        }
        self.dialog_view(ui);
        if let Some(answer) = self.prompt.as_mut().and_then(|p| p.view(ui)) {
            self.prompt = None;
            match answer {
                Answer::Done => self.labels.clear(),
                Answer::Cancelled => {}
                Answer::Failed(err) => tracing::warn!("{err}"),
            }
        }
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "mide_desktop=info".into()))
        .init();
    let mut theme = Theme::default();
    // Nomes brancos (com sombra) sobre o papel de parede.
    theme.tile_text = [255, 255, 255, 255];
    run(Window::new("Área de trabalho").app_id("mide-desktop").desktop("mide-desktop").theme(theme), Desktop::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icones_em_colunas_a_partir_da_direita() {
        // Área útil de 800 x 500 abaixo de uma barra de 28 px: 4 linhas por coluna.
        let area = (0.0, 28.0, 800.0, 500.0);
        let (x0, y0) = Desktop::place(0, area);
        assert_eq!((x0, y0), (800.0 - MARGIN - CELL.0, 28.0 + MARGIN));
        let (x1, y1) = Desktop::place(1, area);
        assert_eq!(x1, x0);
        assert!(y1 > y0);
        // Quinto ícone: segunda coluna, à esquerda da primeira, de novo em cima.
        let (x4, y4) = Desktop::place(4, area);
        assert!(x4 < x0);
        assert_eq!(y4, 28.0 + MARGIN);
    }
}
