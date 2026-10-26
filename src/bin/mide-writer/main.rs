//! Writer do mide (E2b, `mide-writer`): o editor de documentos, para quem escreve
//! (como o TextEdit e o Pages do Mac, no lugar do Word/Writer): documentos formatados
//! (RTF, DOCX e ODT, com páginas, fontes, cores, alinhamento, listas, imagens e as
//! normas ABNT, em `rich.rs`). Um documento por janela; abre pelo Arquivos ou com
//! `mide-writer <arquivo>`.
//!
//! A interface é sempre a de documento: um arquivo de texto simples abre como
//! documento (e volta a ser texto simples ao salvar como .txt); um arquivo vazio
//! (criado por "novo arquivo vazio") abre como documento novo; um arquivo que não dá
//! para ler mostra o aviso e abre um documento novo sem nome, para nunca gravar por
//! cima do original. Para editar código e textos rápidos, o `mide-code`.

mod rich;

use std::{
    error::Error,
    path::{Path, PathBuf},
};

use mide::kit::{run, textarea::TextDoc, textedit::Edit, window_menu, App, ButtonStyle, Choice, FileChooser, Key, MenuGroup, MenuItem, Ui, Window};

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
    /// "Abrir…": o seletor de arquivos.
    Open(FileChooser),
    /// "Inserir imagem…" do modo formatado.
    InsertImage(FileChooser),
    /// "Exportar como PDF…" e "Salvar PDF…" do Imprimir (com as páginas).
    ExportPdf(FileChooser, Option<(usize, usize)>),
    /// "Salvar como…": o seletor no modo salvar (`then_close`: fecha depois de salvar).
    SaveAs { chooser: FileChooser, then_close: bool },
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
    /// Modo formatado (RTF); sem ele, texto simples em `doc`.
    rich: Option<rich::Rich>,
}

/// Extensões abertas no modo formatado.
fn is_rich_path(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| ["rtf", "docx", "odt"].iter().any(|x| e.eq_ignore_ascii_case(x)))
}

/// Imagens que entram no documento (soltas na janela ou pelo "Inserir imagem").
fn is_picture(path: &Path) -> bool {
    matches!(ext_of(path).as_str(), "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp")
}

fn ext_of(path: &Path) -> String {
    path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase()
}

/// Lê um documento formatado pela extensão.
fn read_rich(path: &Path, data: &[u8]) -> Option<mide::richtext::Doc> {
    match ext_of(path).as_str() {
        "docx" => mide::richtext::docx::read(data),
        "odt" => mide::richtext::odt::read(data),
        _ => mide::richtext::rtf::read(data),
    }
}

impl Editor {
    fn open(path: Option<PathBuf>) -> Self {
        let mut editor = Self { path: None, doc: TextDoc::default(), crlf: false, message: None, find: None, dialog: None, start: true, rich: None };
        // Sempre a interface de documento (documento novo enquanto nada foi lido).
        editor.rich = Some(rich::Rich::new(mide::richtext::Doc::new()));
        let Some(path) = path else {
            return editor;
        };
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        match std::fs::read(&path) {
            // Vazio (criado por "novo arquivo vazio" de outro programa) ou ainda não
            // existe: documento novo, gravado nesse arquivo ao salvar.
            Ok(data) if data.is_empty() => editor.path = Some(path),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => editor.path = Some(path),
            Ok(data) if is_rich_path(&path) => match read_rich(&path, &data) {
                Some(doc) => {
                    tracing::info!("aberto (formatado): {} ({} parágrafos)", path.display(), doc.paras.len());
                    editor.rich = Some(rich::Rich::new(doc));
                    editor.path = Some(path);
                }
                // Não deu para ler: avisa e segue com um documento novo sem nome (salvar
                // pergunta onde; o original fica como está).
                None => {
                    tracing::warn!("não foi possível ler {}", path.display());
                    editor.message = Some(format!("Não foi possível ler \"{name}\" (arquivo danificado ou de outro formato). Você pode escrever num documento novo; o original não é alterado."));
                }
            },
            // Texto simples: entra como documento (ao salvar como .txt, volta a ser texto).
            Ok(_) => match read_text(&path) {
                Ok((text, crlf)) => {
                    tracing::info!("aberto (texto): {}", path.display());
                    editor.rich = Some(rich::Rich::new(mide::richtext::Doc::from_plain(&text, mide::richtext::CharStyle::default())));
                    editor.crlf = crlf;
                    editor.path = Some(path);
                }
                Err(err) => editor.message = Some(format!("Não foi possível ler \"{name}\": {err}. Você pode escrever num documento novo; o original não é alterado.")),
            },
            Err(err) => editor.message = Some(format!("Não foi possível abrir \"{name}\": {err}")),
        }
        editor
    }

    fn name(&self) -> String {
        self.path
            .as_ref()
            .and_then(|p| p.file_name())
            .map_or_else(|| "Sem título".to_string(), |n| n.to_string_lossy().into_owned())
    }

    fn modified(&self) -> bool {
        match &self.rich {
            Some(r) => r.modified(),
            None => self.doc.modified,
        }
    }

    /// Salva no caminho atual; sem caminho, pede um ("Salvar como").
    fn save(&mut self, then_close: bool) -> bool {
        let Some(path) = self.path.clone() else {
            self.ask_save_as(then_close);
            return false;
        };
        if let Some(r) = self.rich.as_mut() {
            // A célula em edição volta para a tabela antes de gravar.
            r.leave_cell();
            // Formatado: pela extensão (DOCX, ODT, RTF; num .txt, o texto simples).
            let data: Vec<u8> = match ext_of(&path).as_str() {
                "docx" => mide::richtext::docx::write(&r.doc),
                "odt" => mide::richtext::odt::write(&r.doc),
                "rtf" | "" => mide::richtext::rtf::write(&r.doc).into_bytes(),
                _ => r.doc.all_plain().into_bytes(),
            };
            return match mide::files::write_atomic(&path, &data) {
                Ok(()) => {
                    tracing::info!("salvo (formatado): {}", path.display());
                    r.mark_saved();
                    self.message = None;
                    true
                }
                Err(err) => {
                    self.message = Some(format!("Não foi possível salvar: {err}"));
                    false
                }
            };
        }
        let text = if self.crlf { self.doc.text.replace('\n', "\r\n") } else { self.doc.text.clone() };
        match mide::files::write_atomic(&path, text.as_bytes()) {
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
        let (dir, name) = match &self.path {
            Some(path) => (path.parent().map(Path::to_path_buf), path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()),
            None => (Some(default_dir()), if self.rich.is_some() { "Sem título.docx" } else { "Sem título.txt" }.to_string()),
        };
        self.dialog = Some(Dialog::SaveAs { chooser: FileChooser::save("Salvar como", dir.as_deref(), &name), then_close });
    }

    fn ask_open(&mut self) {
        let dir = self.path.as_ref().and_then(|p| p.parent()).map(Path::to_path_buf).unwrap_or_else(default_dir);
        self.dialog = Some(Dialog::Open(FileChooser::open("Abrir", Some(&dir))));
    }

    /// Abre o arquivo escolhido: nesta janela, se ela está sem título e sem nada
    /// digitado; senão, numa janela nova (um documento por janela, como no Mac).
    fn open_chosen(&mut self, path: PathBuf) {
        let empty = match &self.rich {
            Some(r) => !r.modified() && r.doc.all_plain().is_empty(),
            None => !self.doc.modified && self.doc.text.is_empty(),
        };
        if self.path.is_none() && empty {
            let opened = Editor::open(Some(path));
            self.path = opened.path;
            self.doc = opened.doc;
            self.crlf = opened.crlf;
            self.message = opened.message;
            self.rich = opened.rich;
            self.start = true;
            return;
        }
        let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("mide-writer"));
        match std::process::Command::new(exe).arg(&path).spawn() {
            Ok(mut child) => {
                tracing::info!("abrindo em outra janela: {}", path.display());
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
            }
            Err(err) => self.message = Some(format!("Não foi possível abrir: {err}")),
        }
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

    /// Barra de ferramentas do Tahoe, como a do TextEdit: luzes, o nome do documento
    /// (com "Editado") e, à direita, as cápsulas de abrir/salvar e de buscar.
    fn toolbar(&mut self, ui: &mut Ui) {
        let mut clicked = None;
        let title = if self.modified() { format!("{} — Editado", self.name()) } else { self.name() };
        let finding = self.find.is_some() || self.rich.as_ref().is_some_and(|r| r.find.is_some());
        ui.toolbar(|ui| {
            ui.toolbar_title(&title, 420.0);
            ui.push_right(2.0 * 40.0 + 4.0 + 10.0 + 40.0 + 4.0);
            match ui.toolbar_buttons(&[("document-open-symbolic", "↥", false, true), ("document-save-symbolic", "↓", false, true)]) {
                Some(0) => clicked = Some("abrir"),
                Some(1) => clicked = Some("salvar"),
                _ => {}
            }
            if ui.toolbar_buttons(&[("edit-find-symbolic", "⌕", finding, true)]).is_some() {
                clicked = Some("buscar");
            }
        });
        match clicked {
            Some("abrir") => self.ask_open(),
            Some("salvar") => {
                self.save(false);
            }
            Some("buscar") => match self.rich.as_mut() {
                Some(r) => r.toggle_find(),
                None => self.toggle_find(),
            },
            _ => {}
        }
    }

    /// Rodapé com a linha e a coluna do cursor.
    fn status(&self, ui: &mut Ui) {
        let starts = self.doc.line_starts();
        let line = mide::kit::textarea::line_of(&starts, self.doc.edit.cursor);
        let column = self.doc.text[starts[line]..self.doc.edit.cursor].chars().count() + 1;
        let lines = starts.len();
        ui.status_bar(&format!("Linha {}, coluna {column} · {lines} {}", line + 1, if lines == 1 { "linha" } else { "linhas" }));
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
        // Barra de busca do Tahoe: busca em cápsula, anterior/próxima numa cápsula,
        // substituir e "OK" para fechar.
        ui.space(4.0);
        ui.row(|ui| {
            ui.space_x(12.0);
            if focus {
                ui.focus_next_input();
            }
            ui.sized(240.0, |ui| next = ui.search_field(&mut query, "Buscar").submitted());
            match ui.toolbar_buttons(&[("go-up-symbolic", "‹", false, true), ("go-down-symbolic", "›", false, true)]) {
                Some(0) => prev = true,
                Some(1) => next = true,
                _ => {}
            }
            ui.space_x(8.0);
            ui.sized(220.0, |ui| one = ui.search_field(&mut replace, "Substituir por").submitted());
            ui.sized(100.0, |ui| one |= ui.button_styled("Substituir", ButtonStyle::Bordered).clicked());
            ui.sized(70.0, |ui| all = ui.button_styled("Tudo", ButtonStyle::Bordered).clicked());
            ui.sized(60.0, |ui| close = ui.button_styled("OK", ButtonStyle::Prominent).clicked());
        });
        ui.space(6.0);
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

    /// Tela do modo formatado: barra de ferramentas, a do TextEdit e as páginas.
    fn rich_view(&mut self, ui: &mut Ui, save: Option<bool>, open: bool) {
        if self.dialog.is_none() {
            if open {
                self.ask_open();
            }
            match save {
                Some(true) => self.ask_save_as(false),
                Some(false) => {
                    self.save(false);
                }
                None => {}
            }
            if ui.input().shortcut_shift("t") == Some(true) {
                self.to_plain();
                return;
            }
        }
        self.toolbar(ui);
        let top = mide::kit::TOOLBAR_HEIGHT;
        if let Some(r) = self.rich.as_mut() {
            if self.dialog.is_some() {
                // Com o seletor aberto, as páginas só aparecem (sem teclado).
                ui.blur();
            }
            r.view(ui, top);
            if std::mem::take(&mut r.wants_image) {
                let dir = mide::places::pictures_dir();
                self.dialog = Some(Dialog::InsertImage(FileChooser::open("Inserir imagem", Some(&dir)).extensions(&["png", "jpg", "jpeg", "webp", "gif", "bmp"])));
            }
            if let Some(range) = r.wants_pdf_path.take() {
                let dir = self.path.as_ref().and_then(|p| p.parent()).map(Path::to_path_buf).unwrap_or_else(default_dir);
                let stem = self.path.as_ref().and_then(|p| p.file_stem()).map_or_else(|| "Sem título".to_string(), |n| n.to_string_lossy().into_owned());
                self.dialog = Some(Dialog::ExportPdf(FileChooser::save("Exportar como PDF", Some(&dir), &format!("{stem}.pdf")), range));
            }
            if let Some(m) = r.message.take() {
                self.message = Some(m);
            }
        }
        if let Some(message) = self.message.clone() {
            let (w, h) = ui.window_size();
            ui.at(16.0, h - rich::STATUS_H - 30.0, w - 32.0, |ui| ui.note(&message));
        }
        self.dialog(ui);
    }

    /// Formatar → Converter em texto simples (perde a formatação).
    fn to_plain(&mut self) {
        if let Some(r) = self.rich.take() {
            self.doc = TextDoc::new(r.doc.all_plain());
            self.doc.modified = true;
            self.start = true;
            if self.path.as_ref().is_some_and(|p| is_rich_path(p)) {
                self.path = self.path.as_ref().map(|p| p.with_extension("txt"));
            }
            tracing::info!("editor: convertido em texto simples");
        }
    }

    /// Formatar → Converter em texto formatado.
    fn to_rich(&mut self) {
        let doc = mide::richtext::Doc::from_plain(&self.doc.text, mide::richtext::CharStyle::default());
        let mut r = rich::Rich::new(doc);
        r.undo_point();
        self.rich = Some(r);
        if let Some(p) = &self.path {
            self.path = Some(p.with_extension("docx"));
        }
        tracing::info!("editor: convertido em texto formatado");
    }

    fn dialog(&mut self, ui: &mut Ui) {
        let Some(dialog) = self.dialog.as_mut() else {
            return;
        };
        let mut esc = ui.input().key(Key::Escape);
        match dialog {
            Dialog::Open(chooser) => match chooser.show(ui) {
                Some(Choice::Chosen(path)) => {
                    self.dialog = None;
                    self.open_chosen(path);
                }
                Some(Choice::Cancelled) => self.dialog = None,
                None => {}
            },
            Dialog::ExportPdf(chooser, range) => {
                let range = *range;
                match chooser.show(ui) {
                    Some(Choice::Chosen(path)) => {
                        self.dialog = None;
                        let path = if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf")) { path } else { path.with_extension("pdf") };
                        if let Some(r) = self.rich.as_mut() {
                            r.pdf_request = Some((path, range));
                        }
                    }
                    Some(Choice::Cancelled) => self.dialog = None,
                    None => {}
                }
            }
            Dialog::InsertImage(chooser) => match chooser.show(ui) {
                Some(Choice::Chosen(path)) => {
                    self.dialog = None;
                    if let Some(r) = self.rich.as_mut() {
                        r.insert_image(&path);
                    }
                }
                Some(Choice::Cancelled) => self.dialog = None,
                None => {}
            },
            Dialog::SaveAs { chooser, then_close } => {
                let then_close = *then_close;
                match chooser.show(ui) {
                    Some(Choice::Chosen(path)) => {
                        self.path = Some(path);
                        self.dialog = None;
                        if self.save(false) && then_close {
                            ui.close();
                        }
                    }
                    Some(Choice::Cancelled) => self.dialog = None,
                    None => {}
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
        let title = format!("{}{} — Writer", if self.modified() { "• " } else { "" }, self.name());
        ui.set_title(&title);
        // Atalhos do app: Ctrl+S salva (com Shift, "Salvar como"), Ctrl+O abre, Ctrl+F
        // busca. Itens dos menus do app na barra superior também.
        let (save, find, open) = (ui.input().shortcut_shift("s"), ui.input().shortcut("f"), ui.input().shortcut("o"));
        // Arquivos soltos na janela (da área de trabalho, do Arquivos, de outro app):
        // imagem num documento formatado entra no texto, como no TextEdit; o resto abre
        // (nesta janela, se ela está vazia; senão, numa janela nova).
        if let Some(paths) = ui.input().dropped.clone() {
            for path in paths {
                match self.rich.as_mut() {
                    Some(r) if is_picture(&path) => r.insert_image(&path),
                    _ => self.open_chosen(path),
                }
            }
            ui.request_repaint();
        }
        for action in ui.input().menu.clone() {
            if let Some(r) = self.rich.as_mut()
                && r.command(&action, ui) {
                    continue;
                }
            match action.as_str() {
                "abrir" if self.dialog.is_none() => self.ask_open(),
                "salvar" if self.dialog.is_none() => {
                    self.save(false);
                }
                "salvar-como" if self.dialog.is_none() => self.ask_save_as(false),
                "buscar" => self.toggle_find(),
                "nova" => {
                    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("mide-writer"));
                    if let Ok(mut child) = std::process::Command::new(exe).spawn() {
                        std::thread::spawn(move || {
                            let _ = child.wait();
                        });
                    }
                }
                _ => {}
            }
        }
        if self.dialog.is_none() && open {
            self.ask_open();
        }
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
        if self.rich.is_some() {
            self.rich_view(ui, save, open);
            return;
        }
        if ui.input().shortcut_shift("t") == Some(true) {
            self.to_rich();
            return;
        }
        self.toolbar(ui);
        self.find_bar(ui);
        if let Some(message) = &self.message {
            ui.row(|ui| {
                ui.space_x(16.0);
                ui.note(message);
            });
        }
        if std::mem::take(&mut self.start) {
            ui.focus_next_input();
        }
        // O rodapé (26) e o espaço entre a área de texto e ele.
        let height = ui.remaining_height() - 26.0 - ui.theme().spacing;
        ui.text_area(&mut self.doc, height);
        self.status(ui);
        self.dialog(ui);
    }

    fn menu_bar(&mut self) -> Vec<MenuGroup> {
        if let Some(r) = &self.rich {
            let mut groups = vec![
                MenuGroup {
                    title: "Arquivo".into(),
                    items: vec![
                        MenuItem::action("nova", "Novo").shortcut("Ctrl+N"),
                        MenuItem::action("abrir", "Abrir…").shortcut("Ctrl+O"),
                        MenuItem::Separator,
                        MenuItem::action("salvar", "Salvar").shortcut("Ctrl+S"),
                        MenuItem::action("salvar-como", "Salvar como…").shortcut("Ctrl+Shift+S"),
                        MenuItem::action("exportar-pdf", "Exportar como PDF…"),
                        MenuItem::Separator,
                        MenuItem::action("configurar-pagina", "Configurar página…").shortcut("Ctrl+Shift+P"),
                        MenuItem::action("imprimir", "Imprimir…").shortcut("Ctrl+P"),
                    ],
                },
                MenuGroup {
                    title: "Editar".into(),
                    items: vec![
                        MenuItem::action("desfazer", "Desfazer").shortcut("Ctrl+Z"),
                        MenuItem::action("refazer", "Refazer").shortcut("Ctrl+Shift+Z"),
                        MenuItem::Separator,
                        MenuItem::action("recortar", "Recortar").shortcut("Ctrl+X"),
                        MenuItem::action("copiar", "Copiar").shortcut("Ctrl+C"),
                        MenuItem::action("colar", "Colar").shortcut("Ctrl+V"),
                        MenuItem::action("selecionar-tudo", "Selecionar tudo").shortcut("Ctrl+A"),
                        MenuItem::Separator,
                        MenuItem::action("buscar", "Buscar e substituir…").shortcut("Ctrl+F"),
                        MenuItem::action("ortografia", "Verificar ortografia ao digitar").checked(r.spell_on),
                    ],
                },
            ];
            groups.extend(r.menus());
            groups.push(window_menu(Vec::new()));
            return groups;
        }
        vec![
            MenuGroup {
                title: "Arquivo".into(),
                items: vec![
                    MenuItem::action("nova", "Nova janela").shortcut("Ctrl+N"),
                    MenuItem::action("abrir", "Abrir…").shortcut("Ctrl+O"),
                    MenuItem::Separator,
                    MenuItem::action("salvar", "Salvar").shortcut("Ctrl+S"),
                    MenuItem::action("salvar-como", "Salvar como…").shortcut("Ctrl+Shift+S"),
                ],
            },
            MenuGroup { title: "Editar".into(), items: vec![MenuItem::action("buscar", "Buscar e substituir…").shortcut("Ctrl+F")] },
            MenuGroup { title: "Formatar".into(), items: vec![MenuItem::action("texto-formatado", "Converter em texto formatado").shortcut("Ctrl+Shift+T")] },
            window_menu(Vec::new()),
        ]
    }

    fn close_requested(&mut self) -> bool {
        if !self.modified() {
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
    let window = Window::new("Writer").app_id("mide-writer").size(1040, 660).min_size(560, 360).full_size_content().accepts_files();
    run(window, editor)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn salvar_mantem_permissoes_e_crlf() {
        let dir = std::env::temp_dir().join(format!("mide-writer-{}", std::process::id()));
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
        let dir = std::env::temp_dir().join(format!("mide-writer-bin-{}", std::process::id()));
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
