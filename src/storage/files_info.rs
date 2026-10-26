//! Janela "Informações" do Arquivos (⌘I), como o "Obter Informações" do Finder: uma
//! janela por item, com o ícone, o nome, o tamanho e a data no topo, as etiquetas de
//! cor e as seções que abrem e fecham (Geral, Mais informações, Nome e extensão,
//! Comentários, Abrir com, Prévia, Compartilhamento e permissões).

use std::{
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::mpsc,
};

use tiny_skia::Pixmap;

use crate::{
    files,
    kit::{run, App, Ui, Window, TAG_COLORS},
};

/// Abre a janela de informações de `path`.
pub fn run_info(path: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let name = display_name(&path);
    // A mesma linha do "Obter informações" de dentro do Arquivos (a bateria confere as duas).
    tracing::info!("informações de {}", path.display());
    let app = Info::new(path);
    let window = Window::new(&format!("Informações de {name}")).app_id("mide-files-info").size(360, 640).min_size(320, 420);
    run(window, app)
}

fn display_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string())
}

/// Seções que abrem e fecham (Geral e Prévia começam abertas, como no Finder).
struct Sections {
    general: bool,
    more: bool,
    name: bool,
    comments: bool,
    open_with: bool,
    preview: bool,
    sharing: bool,
}

struct Info {
    path: PathBuf,
    meta: Option<std::fs::Metadata>,
    tags: Vec<usize>,
    name: String,
    comment: String,
    comment_saved: String,
    sections: Sections,
    /// Miniatura grande (gerada numa thread: vídeos e PDFs levam tempo).
    preview: Option<Pixmap>,
    preview_rx: Option<mpsc::Receiver<Option<Pixmap>>>,
    /// Apps que abrem o tipo do arquivo (o padrão primeiro) e o escolhido.
    apps: Vec<(String, String)>,
    app_index: usize,
    mime: Option<String>,
    message: Option<String>,
}

impl Info {
    fn new(path: PathBuf) -> Self {
        let (tx, rx) = mpsc::channel();
        let p = path.clone();
        std::thread::spawn(move || {
            let _ = tx.send(crate::thumbnails::make(&p, 256));
        });
        let mime = crate::default_apps::mime_for(&path);
        let all = crate::apps::load_handlers();
        let apps: Vec<(String, String)> = crate::default_apps::apps_for(&all, &path).into_iter().map(|a| (a.id.clone(), a.name.clone())).collect();
        let mut info = Self {
            name: display_name(&path),
            comment: files::comment_of(&path),
            comment_saved: String::new(),
            tags: files::tags_of(&path),
            meta: std::fs::metadata(&path).ok(),
            path,
            sections: Sections { general: true, more: false, name: false, comments: false, open_with: false, preview: true, sharing: false },
            preview: None,
            preview_rx: Some(rx),
            apps,
            app_index: 0,
            mime,
            message: None,
        };
        info.comment_saved = info.comment.clone();
        info
    }

    fn reload(&mut self) {
        self.meta = std::fs::metadata(&self.path).ok();
        self.tags = files::tags_of(&self.path);
    }

    fn header(&mut self, ui: &mut Ui) {
        let Some(meta) = self.meta.clone() else {
            ui.note("O item não existe mais.");
            return;
        };
        let size = if meta.is_dir() { String::new() } else { files::size_text(meta.len()) };
        let modified = date_text(meta.modified().ok());
        let entry = files::Entry { name: self.name.clone(), path: self.path.clone(), is_dir: meta.is_dir(), size: meta.len(), modified: meta.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH) };
        let icon = entry.icon_name();
        ui.row(|ui| {
            ui.icon(icon, 48);
            let w = (ui.remaining_width() - 100.0).max(80.0);
            ui.sized(w, |ui| {
                ui.strong(&self.name);
            });
            ui.push_right(90.0);
            ui.sized(90.0, |ui| ui.strong(&size));
        });
        ui.note(&format!("Modificado: {modified}"));
    }

    fn tags_row(&mut self, ui: &mut Ui) {
        ui.heading("Etiquetas");
        let mut toggled = None;
        ui.row(|ui| {
            for (i, c) in TAG_COLORS.iter().enumerate() {
                let on = self.tags.contains(&i);
                let r = ui.tag_toggle(*c, on);
                if r.clicked() {
                    toggled = Some(i);
                }
                ui.tooltip(&r, files::TAGS[i].1);
            }
        });
        if let Some(i) = toggled {
            match files::toggle_tag(&self.path, i) {
                Ok(()) => tracing::info!("etiqueta {} em {}", files::TAGS[i].0, self.path.display()),
                Err(err) => self.message = Some(format!("Não foi possível marcar a etiqueta: {err}")),
            }
            self.reload();
        }
    }

    fn general(&mut self, ui: &mut Ui) {
        let Some(meta) = self.meta.clone() else {
            return;
        };
        let entry = files::Entry { name: self.name.clone(), path: self.path.clone(), is_dir: meta.is_dir(), size: meta.len(), modified: meta.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH) };
        let kind = files::kind_text(&entry);
        let size = if meta.is_dir() {
            let count = std::fs::read_dir(&self.path).map(|r| r.count()).unwrap_or(0);
            if count == 1 { "1 item".to_string() } else { format!("{count} itens") }
        } else {
            format!("{} bytes ({} no disco)", thousands(meta.len()), files::size_text(files::disk_usage(&meta)))
        };
        let place = self.path.parent().map(|p| p.display().to_string().replace('/', " › ").trim_start_matches(" › ").to_string()).unwrap_or_default();
        let created = date_text(meta.created().ok());
        let modified = date_text(meta.modified().ok());
        kv(ui, "Tipo", &kind);
        kv(ui, "Tamanho", &size);
        let r = kv(ui, "Onde", &place);
        let mut copy = false;
        ui.context_menu(&r, |m| copy = m.item("Copiar como nome do caminho"));
        if copy {
            ui.copy(&self.path.display().to_string());
        }
        kv(ui, "Criado", &created);
        kv(ui, "Modificado", &modified);
    }

    fn more(&mut self, ui: &mut Ui) {
        let Some(meta) = self.meta.clone() else {
            return;
        };
        kv(ui, "Último acesso", &date_text(meta.accessed().ok()));
        if let Ok((w, h)) = image::image_dimensions(&self.path) {
            kv(ui, "Dimensões", &format!("{w} × {h}"));
        }
        if let Some(mime) = &self.mime {
            kv(ui, "Tipo (MIME)", mime);
        }
    }

    fn name_section(&mut self, ui: &mut Ui) {
        if ui.text_input(&mut self.name, "Nome").submitted() {
            let current = display_name(&self.path);
            if self.name.trim().is_empty() || self.name == current {
                self.name = current;
            } else {
                match files::rename(&self.path, self.name.trim()) {
                    Ok(new_path) => {
                        tracing::info!("renomeado em Informações: {}", new_path.display());
                        self.path = new_path;
                        self.name = display_name(&self.path);
                        ui.set_title(&format!("Informações de {}", self.name));
                        self.reload();
                    }
                    Err(err) => {
                        self.message = Some(format!("Não foi possível renomear: {err}"));
                        self.name = current;
                    }
                }
            }
        }
    }

    fn comments(&mut self, ui: &mut Ui) {
        let r = ui.text_input(&mut self.comment, "Escreva um comentário");
        // Grava ao apertar Enter e quando o campo perde o foco.
        if (r.submitted() || !ui.typing()) && self.comment != self.comment_saved {
            match files::set_comment(&self.path, &self.comment) {
                Ok(()) => self.comment_saved = self.comment.clone(),
                Err(err) => self.message = Some(format!("Não foi possível gravar o comentário: {err}")),
            }
        }
    }

    fn open_with(&mut self, ui: &mut Ui) {
        if self.apps.is_empty() {
            ui.note("Nenhum app abre este tipo de arquivo.");
            return;
        }
        let names: Vec<String> = self.apps.iter().map(|(_, n)| n.clone()).collect();
        ui.card(|ui| {
            ui.dropdown("Abrir com", &names, &mut self.app_index);
        });
        ui.note("Use este app para abrir todos os documentos deste tipo.");
        if ui.button("Alterar todos…").clicked() {
            if let (Some(mime), Some((id, name))) = (&self.mime, self.apps.get(self.app_index)) {
                match crate::default_apps::set_for_mime(mime, &format!("{id}.desktop")) {
                    Ok(()) => self.message = Some(format!("{name} abre todos os arquivos deste tipo.")),
                    Err(err) => self.message = Some(format!("Não foi possível mudar o app padrão: {err}")),
                }
            }
        }
    }

    fn sharing(&mut self, ui: &mut Ui) {
        let Some(meta) = self.meta.clone() else {
            return;
        };
        const LEVELS: [&str; 3] = ["Ler e gravar", "Somente leitura", "Sem acesso"];
        let mode = meta.permissions().mode();
        let owner = files::account_name("/etc/passwd", meta.uid());
        let group = files::account_name("/etc/group", meta.gid());
        let level = |shift: u32| -> usize {
            let bits = (mode >> shift) & 0o6;
            if bits & 0o2 != 0 { 0 } else if bits & 0o4 != 0 { 1 } else { 2 }
        };
        let options: Vec<String> = LEVELS.iter().map(|s| s.to_string()).collect();
        let mut chosen = [level(6), level(3), level(0)];
        let before = chosen;
        ui.card(|ui| {
            ui.dropdown(&format!("{owner} (dono)"), &options, &mut chosen[0]);
            ui.dropdown(&format!("{group} (grupo)"), &options, &mut chosen[1]);
            ui.dropdown("todos", &options, &mut chosen[2]);
        });
        if chosen != before {
            let mut new_mode = mode & !0o666;
            for (i, shift) in [6u32, 3, 0].into_iter().enumerate() {
                let bits = match chosen[i] {
                    0 => 0o6,
                    1 => 0o4,
                    _ => 0,
                };
                new_mode |= bits << shift;
            }
            match std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(new_mode)) {
                Ok(()) => tracing::info!("permissões de {}: {:o}", self.path.display(), new_mode & 0o7777),
                Err(err) => self.message = Some(format!("Não foi possível mudar as permissões: {err}")),
            }
            self.reload();
        }
    }
}

impl App for Info {
    fn view(&mut self, ui: &mut Ui) {
        if let Some(rx) = &self.preview_rx {
            match rx.try_recv() {
                Ok(pix) => {
                    self.preview = pix;
                    self.preview_rx = None;
                    ui.request_repaint();
                }
                Err(mpsc::TryRecvError::Empty) => ui.request_tick(),
                Err(_) => self.preview_rx = None,
            }
        }
        if ui.input().key(crate::kit::Key::Escape) && !ui.typing() {
            ui.close();
        }
        ui.page(|ui| {
            self.header(ui);
            self.tags_row(ui);
            if let Some(message) = &self.message {
                ui.note(message);
            }
            let mut s = std::mem::replace(&mut self.sections, Sections { general: false, more: false, name: false, comments: false, open_with: false, preview: false, sharing: false });
            ui.disclosure("Geral", "", &mut s.general);
            if s.general {
                self.general(ui);
            }
            ui.disclosure("Mais informações", "", &mut s.more);
            if s.more {
                self.more(ui);
            }
            ui.disclosure("Nome e extensão", "", &mut s.name);
            if s.name {
                self.name_section(ui);
            }
            ui.disclosure("Comentários", "", &mut s.comments);
            if s.comments {
                self.comments(ui);
            }
            ui.disclosure("Abrir com", "", &mut s.open_with);
            if s.open_with {
                self.open_with(ui);
            }
            ui.disclosure("Prévia", "", &mut s.preview);
            if s.preview {
                let icon = if self.meta.as_ref().is_some_and(|m| m.is_dir()) { "folder" } else { "text-x-generic" };
                ui.image_centered(self.preview.as_ref(), icon, 200);
            }
            ui.disclosure("Compartilhamento e permissões", "", &mut s.sharing);
            if s.sharing {
                self.sharing(ui);
            }
            self.sections = s;
        });
    }
}

/// Linha "chave: valor" das seções (a chave à direita numa coluna, como no Finder).
fn kv(ui: &mut Ui, key: &str, value: &str) -> crate::kit::Response {
    ui.key_value(&format!("{key}:"), value, 104.0)
}

fn date_text(t: Option<std::time::SystemTime>) -> String {
    match t {
        Some(t) => chrono::DateTime::<chrono::Local>::from(t).format("%d/%m/%Y às %H:%M").to_string(),
        None => "—".into(),
    }
}

/// Número com ponto de milhar ("1.073.741.824").
fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push('.');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn milhar() {
        assert_eq!(super::thousands(1_073_741_824), "1.073.741.824");
        assert_eq!(super::thousands(999), "999");
        assert_eq!(super::thousands(1000), "1.000");
    }
}
