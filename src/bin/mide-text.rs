//! Editor de Texto do mide (E2): abre um arquivo de texto, edita e salva.
//!
//! Mínimo de propósito: um arquivo por janela, UTF-8, buscar e substituir, desfazer,
//! e aviso de alterações não salvas. Abre pelo Arquivos ou com `mide-text <arquivo>`.

use std::{
    error::Error,
    path::{Path, PathBuf},
};

use mide::kit::{run, textarea::TextDoc, textedit::Edit, App, Key, Ui, Window};

/// Arquivos maiores que isso não são abertos (o editor mínimo guarda cópias do texto
/// para desfazer).
const MAX_BYTES: u64 = 20 * 1024 * 1024;

#[derive(Default)]
struct Find {
    query: String,
    replace: String,
    focus: bool,
}

enum Dialog {
    /// "Salvar como": o caminho sendo digitado.
    SaveAs { path: String, focus: bool, then_close: bool },
    /// Fechar com alterações não salvas.
    ConfirmClose,
}

struct Editor {
    path: Option<PathBuf>,
    doc: TextDoc,
    /// O arquivo usava fim de linha do Windows (`\r\n`); volta assim ao salvar.
    crlf: bool,
    message: Option<String>,
    find: Option<Find>,
    dialog: Option<Dialog>,
    /// Primeiro quadro: o foco vai para o texto.
    start: bool,
}

impl Editor {
    fn open(path: Option<PathBuf>) -> Self {
        let mut editor = Self { path: None, doc: TextDoc::default(), crlf: false, message: None, find: None, dialog: None, start: true };
        if let Some(path) = path {
            match read_text(&path) {
                Ok((text, crlf)) => {
                    tracing::info!("aberto: {}", path.display());
                    editor.doc = TextDoc::new(text);
                    editor.crlf = crlf;
                    editor.path = Some(path);
                }
                // Arquivo que ainda não existe: começa vazio e cria ao salvar.
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => editor.path = Some(path),
                Err(err) => editor.message = Some(format!("Não foi possível abrir \"{}\": {err}", path.display())),
            }
        }
        editor
    }

    fn name(&self) -> String {
        self.path
            .as_ref()
            .and_then(|p| p.file_name())
            .map_or_else(|| "Sem título".to_string(), |n| n.to_string_lossy().into_owned())
    }

    /// Salva no caminho atual; sem caminho, pede um ("Salvar como").
    fn save(&mut self, then_close: bool) -> bool {
        let Some(path) = self.path.clone() else {
            self.ask_save_as(then_close);
            return false;
        };
        let text = if self.crlf { self.doc.text.replace('\n', "\r\n") } else { self.doc.text.clone() };
        match write_atomic(&path, text.as_bytes()) {
            Ok(()) => {
                tracing::info!("salvo: {}", path.display());
                self.doc.modified = false;
                self.message = None;
                true
            }
            Err(err) => {
                self.message = Some(format!("Não foi possível salvar: {err}"));
                false
            }
        }
    }

    fn ask_save_as(&mut self, then_close: bool) {
        let suggestion = match &self.path {
            Some(path) => path.to_string_lossy().into_owned(),
            None => default_dir().join("Sem título.txt").to_string_lossy().into_owned(),
        };
        self.dialog = Some(Dialog::SaveAs { path: suggestion, focus: true, then_close });
    }

    /// Próxima ocorrência da busca depois do cursor (volta ao começo no fim).
    fn find_next(&mut self, backwards: bool) {
        let Some(query) = self.find.as_ref().map(|f| f.query.clone()).filter(|q| !q.is_empty()) else {
            return;
        };
        let text = &self.doc.text;
        let (a, b) = self.doc.edit.selection().unwrap_or((self.doc.edit.cursor, self.doc.edit.cursor));
        let found = if backwards {
            text[..a].rfind(&query).or_else(|| text.rfind(&query))
        } else {
            text[b..].find(&query).map(|i| b + i).or_else(|| text.find(&query))
        };
        match found {
            Some(at) => {
                self.doc.edit = Edit { cursor: at + query.len(), anchor: Some(at) };
                self.message = None;
            }
            None => self.message = Some(format!("\"{query}\" não encontrado.")),
        }
    }

    fn replace_one(&mut self) {
        let Some(find) = &self.find else {
            return;
        };
        let (query, replace) = (find.query.clone(), find.replace.clone());
        if query.is_empty() {
            return;
        }
        if self.doc.edit.selected(&self.doc.text) == Some(query.as_str()) {
            self.doc.before_change(mide::kit::textarea::Change::Other);
            self.doc.edit.insert(&mut self.doc.text, &replace);
        }
        self.find_next(false);
    }

    fn replace_all(&mut self) {
        let Some(find) = &self.find else {
            return;
        };
        let (query, replace) = (find.query.clone(), find.replace.clone());
        if query.is_empty() {
            return;
        }
        let count = self.doc.text.matches(&query).count();
        if count > 0 {
            self.doc.before_change(mide::kit::textarea::Change::Other);
            self.doc.text = self.doc.text.replace(&query, &replace);
            self.doc.edit = Edit::at_end(&self.doc.text);
        }
        self.message = Some(format!("{count} substituição(ões)."));
    }

    fn toolbar(&mut self, ui: &mut Ui) {
        let (mut save, mut save_as, mut find) = (false, false, false);
        ui.toolbar(|ui| {
            ui.sized(90.0, |ui| save = ui.button("Salvar").clicked());
            ui.sized(130.0, |ui| save_as = ui.button("Salvar como…").clicked());
            ui.sized(90.0, |ui| find = ui.button("Buscar").clicked());
            let starts = self.doc.line_starts();
            let line = mide::kit::textarea::line_of(&starts, self.doc.edit.cursor);
            let column = self.doc.text[starts[line]..self.doc.edit.cursor].chars().count() + 1;
            let state = if self.doc.modified { " · editado" } else { "" };
            ui.label(&format!("Linha {}, coluna {column}{state}", line + 1));
        });
        if save {
            self.save(false);
        }
        if save_as {
            self.ask_save_as(false);
        }
        if find {
            self.toggle_find();
        }
    }

    fn toggle_find(&mut self) {
        self.find = match self.find.take() {
            Some(_) => None,
            None => {
                // Começa buscando o que está selecionado (uma linha só).
                let query = self.doc.edit.selected(&self.doc.text).filter(|s| !s.contains('\n')).unwrap_or("").to_string();
                Some(Find { query, focus: true, ..Default::default() })
            }
        };
    }

    fn find_bar(&mut self, ui: &mut Ui) {
        let Some(find) = self.find.as_mut() else {
            return;
        };
        let focus = std::mem::take(&mut find.focus);
        let (mut query, mut replace) = (find.query.clone(), find.replace.clone());
        let (mut next, mut prev, mut one, mut all, mut close) = (false, false, false, false, false);
        ui.toolbar(|ui| {
            if focus {
                ui.focus_next_input();
            }
            ui.sized(220.0, |ui| next = ui.text_input(&mut query, "Buscar").submitted());
            ui.sized(90.0, |ui| prev = ui.button("Anterior").clicked());
            ui.sized(90.0, |ui| next |= ui.button("Próxima").clicked());
            ui.sized(220.0, |ui| one = ui.text_input(&mut replace, "Substituir por").submitted());
            ui.sized(110.0, |ui| one |= ui.button("Substituir").clicked());
            ui.sized(130.0, |ui| all = ui.button("Substituir tudo").clicked());
            ui.sized(80.0, |ui| close = ui.button("Fechar").clicked());
        });
        if let Some(find) = self.find.as_mut() {
            find.query = query;
            find.replace = replace;
        }
        if close || ui.input().key(Key::Escape) {
            self.find = None;
            ui.focus_next_input();
            return;
        }
        if next || prev {
            self.find_next(prev);
        }
        if one {
            self.replace_one();
        }
        if all {
            self.replace_all();
        }
    }

    fn dialog(&mut self, ui: &mut Ui) {
        let Some(dialog) = self.dialog.as_mut() else {
            return;
        };
        let mut esc = ui.input().key(Key::Escape);
        match dialog {
            Dialog::SaveAs { path, focus, then_close } => {
                let (focus, then_close) = (std::mem::take(focus), *then_close);
                let mut text = path.clone();
                let (mut ok, mut cancel) = (false, false);
                ui.modal(460.0, |ui| {
                    ui.strong("Salvar como");
                    if focus {
                        ui.focus_next_input();
                    }
                    ok = ui.text_input(&mut text, "Caminho do arquivo").submitted();
                    ui.row(|ui| {
                        ui.push_right(2.0 * 110.0 + 8.0);
                        ui.sized(110.0, |ui| cancel = ui.button("Cancelar").clicked());
                        ui.sized(110.0, |ui| ok |= ui.primary_button("Salvar").clicked());
                    });
                });
                if let Some(Dialog::SaveAs { path, .. }) = self.dialog.as_mut() {
                    *path = text.clone();
                }
                if cancel || esc {
                    self.dialog = None;
                } else if ok {
                    let path = expand_home(text.trim());
                    if path.file_name().is_none() {
                        self.message = Some("Escreva o nome do arquivo.".into());
                        return;
                    }
                    self.path = Some(path);
                    self.dialog = None;
                    if self.save(false) && then_close {
                        ui.close();
                    }
                }
            }
            Dialog::ConfirmClose => {
                let name = self.name();
                let (mut save, mut discard, mut cancel) = (false, false, false);
                ui.modal(440.0, |ui| {
                    ui.strong(&format!("Salvar as alterações em \"{name}\"?"));
                    ui.note("Se não salvar, as alterações serão perdidas.");
                    ui.row(|ui| {
                        ui.push_right(3.0 * 120.0 + 16.0);
                        ui.sized(120.0, |ui| discard = ui.button("Não salvar").clicked());
                        ui.sized(120.0, |ui| cancel = ui.button("Cancelar").clicked());
                        ui.sized(120.0, |ui| save = ui.primary_button("Salvar").clicked());
                    });
                });
                esc |= cancel;
                if esc {
                    self.dialog = None;
                } else if discard {
                    tracing::info!("fechado sem salvar");
                    ui.close();
                } else if save {
                    self.dialog = None;
                    if self.save(true) {
                        ui.close();
                    }
                }
            }
        }
    }
}

impl App for Editor {
    fn view(&mut self, ui: &mut Ui) {
        let title = format!("{}{} — Editor de Texto", if self.doc.modified { "• " } else { "" }, self.name());
        ui.set_title(&title);
        // Atalhos do app: Ctrl+S salva (com Shift, "Salvar como"), Ctrl+F busca.
        let (save, find) = (ui.input().shortcut_shift("s"), ui.input().shortcut("f"));
        if self.dialog.is_none() {
            match save {
                Some(true) => self.ask_save_as(false),
                Some(false) => {
                    self.save(false);
                }
                None => {}
            }
            if find {
                self.toggle_find();
            }
        }
        self.toolbar(ui);
        self.find_bar(ui);
        if let Some(message) = &self.message {
            ui.note(message);
        }
        if std::mem::take(&mut self.start) {
            ui.focus_next_input();
        }
        let height = ui.remaining_height();
        ui.text_area(&mut self.doc, height);
        self.dialog(ui);
    }

    fn close_requested(&mut self) -> bool {
        if !self.doc.modified {
            return true;
        }
        self.dialog = Some(Dialog::ConfirmClose);
        false
    }
}

/// Lê o arquivo como texto UTF-8, sem `\r\n` (devolve se usava `\r\n`).
fn read_text(path: &Path) -> std::io::Result<(String, bool)> {
    let meta = std::fs::metadata(path)?;
    if meta.len() > MAX_BYTES {
        return Err(std::io::Error::other("arquivo grande demais para o editor (mais de 20 MB)"));
    }
    let bytes = std::fs::read(path)?;
    let text = String::from_utf8(bytes).map_err(|_| std::io::Error::other("não é um arquivo de texto (UTF-8)"))?;
    let crlf = text.contains("\r\n");
    Ok((if crlf { text.replace("\r\n", "\n") } else { text }, crlf))
}

/// Grava sem risco de perder o arquivo: escreve um temporário na mesma pasta, mantém
/// as permissões do original e troca de uma vez.
fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let name = path.file_name().ok_or_else(|| std::io::Error::other("caminho sem nome de arquivo"))?;
    let tmp = dir.join(format!(".{}.mide-text", name.to_string_lossy()));
    {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    if let Ok(meta) = std::fs::metadata(path) {
        let _ = std::fs::set_permissions(&tmp, meta.permissions());
    }
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

fn expand_home(path: &str) -> PathBuf {
    match (path.strip_prefix("~/"), std::env::var_os("HOME")) {
        (Some(rest), Some(home)) => PathBuf::from(home).join(rest),
        _ => PathBuf::from(path),
    }
}

/// Pasta sugerida para um arquivo novo: Documentos, ou a pasta pessoal.
fn default_dir() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    ["Documentos", "Documents"].iter().map(|d| home.join(d)).find(|d| d.is_dir()).unwrap_or(home)
}

fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "mide_text=info".into()))
        .init();
    let path = std::env::args().nth(1).map(PathBuf::from);
    let editor = Editor::open(path);
    let window = Window::new("Editor de Texto").app_id("mide-text").size(860, 620).min_size(480, 320);
    run(window, editor)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn salvar_mantem_permissoes_e_crlf() {
        let dir = std::env::temp_dir().join(format!("mide-text-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("nota.txt");
        std::fs::write(&path, "a\r\nb\r\n").unwrap();
        let (text, crlf) = read_text(&path).unwrap();
        assert_eq!((text.as_str(), crlf), ("a\nb\n", true));
        let mut editor = Editor::open(Some(path.clone()));
        editor.doc.type_text("x");
        assert!(editor.save(false));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "xa\r\nb\r\n");
        assert!(!editor.doc.modified);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn binario_nao_abre() {
        let dir = std::env::temp_dir().join(format!("mide-text-bin-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("x.bin");
        std::fs::write(&path, [0xff, 0xfe, 0x00]).unwrap();
        let editor = Editor::open(Some(path));
        assert!(editor.message.is_some());
        assert!(editor.path.is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn buscar_e_substituir() {
        let mut editor = Editor::open(None);
        editor.doc = TextDoc::new("gato e gato e rato".into());
        editor.find = Some(Find { query: "gato".into(), replace: "cão".into(), focus: false });
        editor.find_next(false);
        assert_eq!(editor.doc.edit.selected(&editor.doc.text), Some("gato"));
        editor.replace_one();
        assert_eq!(editor.doc.text, "cão e gato e rato");
        assert_eq!(editor.doc.edit.selected(&editor.doc.text), Some("gato"));
        editor.replace_all();
        assert_eq!(editor.doc.text, "cão e cão e rato");
        assert!(editor.doc.undo());
        assert_eq!(editor.doc.text, "cão e gato e rato");
    }
}
