//! Editor de Capturas do mide (B23), como a Marcação do Mac: abre uma imagem (a
//! captura que acabou de ser tirada, ou qualquer PNG/JPEG pelo "Abrir com") para
//! marcar e recortar.
//!
//! - **Selecionar** (V): clique numa forma para escolher; arrastar move, as alças azuis
//!   redimensionam (numa linha ou seta, as pontas), Delete apaga, setas andam 1 pixel.
//! - **Esboço** (S): o traço vira forma quando parece uma (linha, seta, retângulo,
//!   triângulo, oval), já escolhida para ajustar; **Desenho** (D): traço livre.
//! - **Formas**: retângulo, retângulo arredondado, oval, linha, seta, triângulo,
//!   estrela e balão de fala (arrastar define o tamanho); **Texto** (T): clique e digite.
//! - **Estilo do traço**, **Cor da borda**, **Preenchimento** e **Estilo do texto**: valem
//!   para a próxima forma e para a escolhida.
//! - **Girar**, **Recortar** (Ctrl+K: bolinhas, Enter aplica, Esc cancela); desfazer
//!   (Ctrl+Z) e refazer (Ctrl+Shift+Z, Ctrl+Y); **Salvar** (Ctrl+S), **Salvar como…**
//!   (Ctrl+Shift+S), **Copiar** (Ctrl+C), **Mover para a Lixeira** (Ctrl+Delete).
//!
//! As formas ficam editáveis até salvar ou copiar (só então entram nos pixels).
//!
//! Com `--miniatura <arquivo>`, é a miniatura que aparece no canto da tela depois de uma
//! captura (`thumbnail.rs`).

mod doc;
mod thumbnail;

use std::{
    error::Error,
    path::{Path, PathBuf},
};

use doc::Doc;
use mide::{
    annotate::{self, Shape, ShapeKind},
    control_client::ask_compositor,
    kit::{run, window_menu, App, ButtonStyle, Choice, FileChooser, Key, MenuGroup, MenuItem, Response, Ui, Window},
    markup::*,
    tr,
    ui::text::Text,
};
use tiny_skia::{Color, FilterQuality, Paint, PathBuilder, Pixmap, PixmapPaint, Rect, Stroke, Transform};

/// Ferramenta.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tool {
    Select,
    Sketch,
    Draw,
    Shape(ShapeKind),
    Text,
}

impl Tool {
    /// Nome da ferramenta no idioma da sessão.
    fn name(self) -> String {
        match self {
            Tool::Select => tr!("Select").into_owned(),
            Tool::Sketch => tr!("Sketch").into_owned(),
            Tool::Draw => tr!("Draw").into_owned(),
            Tool::Text => tr!("Text").into_owned(),
            Tool::Shape(k) => shape_name(k),
        }
    }
}

/// Formas do menu Formas.
const SHAPES: [ShapeKind; 8] = [
    ShapeKind::Rect,
    ShapeKind::RoundRect,
    ShapeKind::Oval,
    ShapeKind::Line,
    ShapeKind::Arrow,
    ShapeKind::Triangle,
    ShapeKind::Star,
    ShapeKind::Bubble,
];

/// Nome da forma no idioma da sessão.
fn shape_name(kind: ShapeKind) -> String {
    match kind {
        ShapeKind::Rect => tr!("Rectangle"),
        ShapeKind::RoundRect => tr!("Rounded Rectangle"),
        ShapeKind::Oval => tr!("Oval"),
        ShapeKind::Line => tr!("Line"),
        ShapeKind::Arrow => tr!("Arrow"),
        ShapeKind::Triangle => tr!("Triangle"),
        ShapeKind::Star => tr!("Star"),
        ShapeKind::Bubble => tr!("Speech Bubble"),
        _ => tr!("Shape"),
    }
    .into_owned()
}

/// Cores das marcações (borda, preenchimento e texto).
const PALETTE: [(&str, [u8; 3]); 9] = [
    ("vermelho", [255, 59, 48]),
    ("laranja", [255, 149, 0]),
    ("amarelo", [255, 204, 0]),
    ("verde", [52, 199, 89]),
    ("azul", [0, 122, 255]),
    ("roxo", [175, 82, 222]),
    ("cinza", [142, 142, 147]),
    ("preto", [20, 20, 22]),
    ("branco", [255, 255, 255]),
];

/// Espessuras do traço (vezes a padrão da imagem).
const WIDTHS: [f32; 5] = [0.5, 1.0, 1.5, 2.2, 3.2];

/// Arrasto em andamento, em pixels da imagem.
enum Drag {
    /// Desenhando à mão (Esboço e Desenho): os pontos por onde passou.
    Draw(Vec<(f32, f32)>),
    /// Forma nova a partir de `start`.
    NewShape { kind: ShapeKind, start: (f32, f32), end: (f32, f32) },
    /// Forma andando, mudando de tamanho por uma alça ou por uma ponta (linha, seta).
    Move { index: usize, start: (f32, f32), orig: Shape, began: bool },
    Resize { index: usize, handle: Handle, start: (f32, f32), orig_box: RectF, orig: Shape, began: bool },
    End { index: usize, which: usize, start: (f32, f32), orig: Shape, began: bool },
    /// Recorte novo, mudando de tamanho pela bolinha `handle` ou mudando de lugar.
    CropNew { start: (f32, f32) },
    CropResize { handle: Handle, orig: RectF, start: (f32, f32) },
    CropMove { orig: RectF, start: (f32, f32) },
}

/// Mudança de estilo pedida num popover (aplicada depois, com o editor livre).
type StyleChange = Box<dyn FnOnce(&mut Editor)>;

/// Popover aberto na barra de ferramentas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pop {
    Shapes,
    Style,
    Border,
    Fill,
    TextStyle,
}

enum Dialog {
    Open(FileChooser),
    SaveAs { chooser: FileChooser, then_close: bool },
    /// O nome escolhido ganhou ".png" e esse arquivo já existe: o seletor só perguntou
    /// pelo nome digitado, então a confirmação de substituir é aqui.
    ConfirmReplace { path: PathBuf, then_close: bool },
    ConfirmClose,
}

/// Onde a imagem aparece: `display` (pontos, na janela) mostra a parte `source`
/// (pixels da imagem) na escala `s`.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Mapping {
    display: RectF,
    source: RectF,
    s: f32,
}

impl Mapping {
    /// `source` cabendo em `area`, no meio, sem ampliar: no máximo `max` pontos por
    /// pixel da imagem (`1 / escala da tela`: um pixel da imagem num pixel da tela).
    fn fit(area: RectF, source: RectF, max: f32) -> Self {
        let s = (area.2 / source.2).min(area.3 / source.3).min(max).max(0.01);
        let (w, h) = (source.2 * s, source.3 * s);
        Self { display: ((area.0 + (area.2 - w) / 2.0).round(), (area.1 + (area.3 - h) / 2.0).round(), w, h), source, s }
    }

    fn to_image(self, p: (f32, f32)) -> (f32, f32) {
        (self.source.0 + (p.0 - self.display.0) / self.s, self.source.1 + (p.1 - self.display.1) / self.s)
    }

    fn to_screen(self, p: (f32, f32)) -> (f32, f32) {
        (self.display.0 + (p.0 - self.source.0) * self.s, self.display.1 + (p.1 - self.source.1) * self.s)
    }

    fn rect_to_screen(self, r: RectF) -> RectF {
        (self.display.0 + (r.0 - self.source.0) * self.s, self.display.1 + (r.1 - self.source.1) * self.s, r.2 * self.s, r.3 * self.s)
    }

    /// Dos pixels da imagem aos pixels do canvas (escala da tela `k`).
    fn transform(self, k: f32) -> Transform {
        let s = self.s * k;
        Transform::from_row(s, 0.0, 0.0, s, (self.display.0 - self.source.0 * self.s) * k, (self.display.1 - self.source.1 * self.s) * k)
    }
}

/// A imagem já reduzida para a tela (sem as formas, que são desenhadas por cima a cada
/// quadro), guardada até mudar a imagem, a parte mostrada ou o tamanho. Enquanto a
/// janela muda de tamanho, a anterior é só esticada; a redução boa é refeita quando o
/// tamanho para de mudar (`RESIZE_SETTLE`).
struct ViewCache {
    content: (usize, [i32; 4]),
    image: Pixmap,
    exact: bool,
    at: std::time::Instant,
}

const RESIZE_SETTLE: std::time::Duration = std::time::Duration::from_millis(150);

struct Editor {
    path: Option<PathBuf>,
    doc: Option<Doc>,
    tool: Tool,
    /// Estilo da próxima forma (e da escolhida, ao mudar).
    stroke: [u8; 3],
    fill: Option<[u8; 3]>,
    width_level: usize,
    dashed: bool,
    font_size: f32,
    /// Forma escolhida e, se for um texto sendo digitado, ele.
    selected: Option<usize>,
    editing_text: Option<usize>,
    cropping: Option<RectF>,
    drag: Option<Drag>,
    pop: Option<Pop>,
    pop_anchor: RectF,
    dialog: Option<Dialog>,
    message: Option<String>,
    view: Option<ViewCache>,
    last_mapping: Option<Mapping>,
    /// Zoom da imagem: 1 é a imagem inteira na janela; acima disso, ampliada em volta de
    /// `center` (ponto da imagem no meio da vista).
    zoom: f32,
    center: Option<(f32, f32)>,
    /// Pedidos de zoom do teclado e dos menus para o próximo desenho (fator, ou 0 para
    /// caber na janela, ou -1 para o tamanho real).
    zoom_request: Option<f32>,
    /// Medidas do texto (caixas de texto) e o resultado ao salvar.
    text: Text,
}

impl Editor {
    fn new(path: Option<PathBuf>) -> Self {
        let mut editor = Self {
            path: None,
            doc: None,
            tool: Tool::Sketch,
            stroke: PALETTE[0].1,
            fill: None,
            width_level: 1,
            dashed: false,
            font_size: 28.0,
            selected: None,
            editing_text: None,
            cropping: None,
            drag: None,
            pop: None,
            pop_anchor: (0.0, 0.0, 0.0, 0.0),
            dialog: None,
            message: None,
            view: None,
            last_mapping: None,
            zoom: 1.0,
            center: None,
            zoom_request: None,
            text: Text::new(),
        };
        match path {
            Some(path) => editor.load(path),
            None => editor.ask_open(),
        }
        editor
    }

    fn load(&mut self, path: PathBuf) {
        match doc::load(&path) {
            Ok(image) => {
                tracing::info!("editor da captura aberto: {} ({}x{})", path.display(), image.width(), image.height());
                self.doc = Some(Doc::new(image));
                self.path = Some(path);
                self.zoom = 1.0;
                self.center = None;
                self.message = None;
                self.cropping = None;
                self.drag = None;
                self.selected = None;
                self.editing_text = None;
                self.view = None;
            }
            Err(err) => {
                tracing::warn!("editor: não abriu {}: {err}", path.display());
                self.message = Some(tr!("Could not open “{name}”: {err}", name = path.display(), err = err));
            }
        }
    }

    fn name(&self) -> String {
        self.path.as_ref().and_then(|p| p.file_name()).map_or_else(|| tr!("No Image").into_owned(), |n| n.to_string_lossy().into_owned())
    }

    fn modified(&self) -> bool {
        self.doc.as_ref().is_some_and(Doc::modified)
    }

    fn ask_open(&mut self) {
        let dir = self.path.as_ref().and_then(|p| p.parent()).map(Path::to_path_buf).unwrap_or_else(default_dir);
        self.dialog = Some(Dialog::Open(FileChooser::open(&tr!("Open Image"), Some(&dir)).extensions(&["png", "jpg", "jpeg"])));
    }

    fn ask_save_as(&mut self, then_close: bool) {
        let dir = self.path.as_ref().and_then(|p| p.parent()).map(Path::to_path_buf).unwrap_or_else(default_dir);
        let name = self.path.as_ref().and_then(|p| p.file_name()).map_or_else(|| tr!("Screenshot.png").into_owned(), |n| n.to_string_lossy().into_owned());
        self.dialog = Some(Dialog::SaveAs { chooser: FileChooser::save(&tr!("Save As"), Some(&dir), &name), then_close });
    }

    /// Abre o arquivo escolhido aqui (sem imagem aberta) ou numa janela nova.
    fn open_chosen(&mut self, path: PathBuf) {
        if self.doc.is_none() {
            self.load(path);
            return;
        }
        match std::process::Command::new(sibling_program("mide-screenshot-editor")).arg(&path).spawn() {
            Ok(_) => tracing::info!("editor: abrindo em outra janela: {}", path.display()),
            Err(err) => self.message = Some(tr!("Could not open: {err}", err = err)),
        }
    }

    /// Salva no arquivo aberto (substitui); sem arquivo, pergunta onde.
    fn save(&mut self, then_close: bool) -> bool {
        match self.path.clone() {
            Some(path) => self.write_to(&path),
            None => {
                self.ask_save_as(then_close);
                false
            }
        }
    }

    fn write_to(&mut self, path: &Path) -> bool {
        let Some(doc) = self.doc.as_mut() else {
            return false;
        };
        let image = doc.result(&mut self.text);
        let written = doc::encode(&image, path).and_then(|bytes| mide::files::write_atomic(path, &bytes).map_err(|e| e.to_string()));
        match written {
            Ok(()) => {
                tracing::info!("editor: salvo {} ({}x{})", path.display(), image.width(), image.height());
                doc.mark_saved();
                self.message = None;
                true
            }
            Err(err) => {
                tracing::warn!("editor: não salvou {}: {err}", path.display());
                self.message = Some(tr!("Could not save: {err}", err = err));
                false
            }
        }
    }

    /// Põe o resultado na área de transferência (o compositor guarda: continua lá depois
    /// que o editor fecha).
    fn copy(&mut self) {
        let Some(doc) = self.doc.as_mut() else {
            return;
        };
        let image = doc.result(&mut self.text);
        let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        let tmp = runtime.join(format!("mide-editor-copia-{}.png", std::process::id()));
        let ok = image.save_png(&tmp).is_ok() && ask_compositor(&format!("copy-file image/png {}", tmp.display())).is_some_and(|r| r.first().map(String::as_str) == Some("ok"));
        let _ = std::fs::remove_file(&tmp);
        if ok {
            tracing::info!("editor: imagem copiada ({}x{})", image.width(), image.height());
            self.message = Some(tr!("Image copied. Paste it with Ctrl+V.").into_owned());
        } else {
            self.message = Some(tr!("The image was not copied: mide did not respond.").into_owned());
        }
    }

    /// Manda o arquivo para a Lixeira e fecha.
    fn trash(&mut self, ui: &mut Ui) {
        let Some(path) = self.path.clone() else {
            return;
        };
        match mide::places::move_to_trash(&path, &mide::places::trash_dir()) {
            Ok(name) => {
                tracing::info!("editor: movido para a Lixeira: {} ({name})", path.display());
                ui.close();
            }
            Err(err) => self.message = Some(tr!("Could not move to the Trash: {err}", err = err)),
        }
    }

    fn show_in_files(&self) {
        let Some(path) = &self.path else {
            return;
        };
        show_in_files(path);
    }

    /// Entra no recorte (a área começa no recorte atual) ou, já recortando, aplica.
    fn crop_command(&mut self) {
        match self.cropping {
            Some(_) => self.apply_crop(),
            None => {
                if let Some(doc) = &self.doc {
                    let c = doc.crop();
                    self.cropping = Some((c.x() as f32, c.y() as f32, c.width() as f32, c.height() as f32));
                    self.drag = None;
                    self.selected = None;
                    self.editing_text = None;
                    tracing::info!("editor: recortando");
                }
            }
        }
    }

    fn apply_crop(&mut self) {
        let (Some(area), Some(doc)) = (self.cropping.take(), self.doc.as_mut()) else {
            return;
        };
        self.drag = None;
        if doc.crop_to(area) {
            let c = doc.crop();
            tracing::info!("editor: recortado em {}x{} a partir de ({}, {})", c.width(), c.height(), c.x(), c.y());
        }
    }

    fn undo(&mut self, redo: bool) {
        self.cropping = None;
        self.drag = None;
        self.editing_text = None;
        if let Some(doc) = self.doc.as_mut() {
            let done = if redo { doc.redo() } else { doc.undo() };
            if done {
                tracing::info!("editor: {}", if redo { "refeito" } else { "desfeito" });
            }
            if self.selected.is_some_and(|i| i >= doc.state.shapes.len()) {
                self.selected = None;
            }
        }
    }

    fn set_tool(&mut self, tool: Tool) {
        if self.tool == tool && self.cropping.is_none() {
            return;
        }
        self.tool = tool;
        self.cropping = None;
        self.editing_text = None;
        if tool != Tool::Select {
            self.selected = None;
        }
        tracing::info!("editor: ferramenta {}", tool.name());
    }

    /// Muda o estilo da forma escolhida (com ponto de desfazer).
    fn restyle(&mut self, f: impl FnOnce(&mut Shape)) {
        let (Some(i), Some(doc)) = (self.selected, self.doc.as_mut()) else {
            return;
        };
        if i >= doc.state.shapes.len() {
            return;
        }
        doc.checkpoint();
        f(&mut doc.state.shapes[i]);
    }

    fn stroke_width(&self) -> f32 {
        self.doc.as_ref().map_or(4.0, Doc::stroke_width) * WIDTHS[self.width_level]
    }

    fn new_shape(&self, kind: ShapeKind, points: Vec<(f32, f32)>) -> Shape {
        let mut shape = Shape::new(kind, points, self.stroke, self.stroke_width());
        shape.dashed = self.dashed;
        if !matches!(kind, ShapeKind::Line | ShapeKind::Arrow | ShapeKind::Pen) {
            shape.fill = self.fill;
        }
        shape.font_size = self.font_size * self.doc.as_ref().map_or(1.0, |d| (d.width().max(d.height()) as f32 / 1600.0).max(1.0));
        shape
    }

    fn command(&mut self, action: &str, ui: &mut Ui) {
        if self.dialog.is_some() {
            return;
        }
        match action {
            "abrir" => self.ask_open(),
            "salvar" => {
                self.save(false);
            }
            "salvar-como" => self.ask_save_as(false),
            "copiar" => self.copy(),
            "lixeira" => self.trash(ui),
            "mostrar" => self.show_in_files(),
            "desfazer" => self.undo(false),
            "refazer" => self.undo(true),
            "recortar" => self.crop_command(),
            "girar-esquerda" | "girar-direita" => {
                self.cropping = None;
                if let Some(doc) = self.doc.as_mut() {
                    doc.rotate(action == "girar-direita");
                    self.view = None;
                    tracing::info!("editor: girado ({action})");
                }
            }
            "apagar" => {
                if let (Some(i), Some(doc)) = (self.selected.take(), self.doc.as_mut()) {
                    doc.remove(i);
                    self.editing_text = None;
                    tracing::info!("editor: forma apagada");
                }
            }
            "ampliar" => self.zoom_request = Some(1.25),
            "reduzir" => self.zoom_request = Some(0.8),
            "caber" => self.zoom_request = Some(0.0),
            "tamanho-real" => self.zoom_request = Some(-1.0),
            "selecionar" => self.set_tool(Tool::Select),
            "esboco" => self.set_tool(Tool::Sketch),
            "desenho" => self.set_tool(Tool::Draw),
            "texto" => self.set_tool(Tool::Text),
            other => {
                if let Some(kind) = SHAPES.iter().find(|k| shape_action(**k) == other) {
                    self.set_tool(Tool::Shape(*kind));
                }
            }
        }
    }

    /// Teclas do app (sem diálogo aberto).
    fn keys(&mut self, ui: &mut Ui) {
        let input = ui.input().clone();
        // Digitando num texto: as teclas são dele.
        if let Some(i) = self.editing_text {
            let mut edited = false;
            let mut text_now = None;
            if let Some(doc) = self.doc.as_mut() {
                if let Some(shape) = doc.state.shapes.get_mut(i) {
                    let mut t = shape.text.clone();
                    for (key, _, _) in input.key_events() {
                        match key {
                            Key::Backspace => {
                                t.pop();
                            }
                            Key::Enter => t.push('\n'),
                            Key::Escape => self.editing_text = None,
                            _ => {}
                        }
                    }
                    t.push_str(&input.text);
                    if t != shape.text {
                        text_now = Some(t);
                    }
                }
                if let Some(t) = text_now {
                    doc.state.shapes[i].text = t;
                    doc.touched();
                    edited = true;
                }
            }
            if edited {
                ui.request_repaint();
            }
            return;
        }
        let mut actions: Vec<&str> = Vec::new();
        match input.shortcut_shift("s") {
            Some(true) => actions.push("salvar-como"),
            Some(false) => actions.push("salvar"),
            None => {}
        }
        match input.shortcut_shift("z") {
            Some(true) => actions.push("refazer"),
            Some(false) => actions.push("desfazer"),
            None => {}
        }
        for (letter, action) in [("=", "ampliar"), ("+", "ampliar"), ("-", "reduzir"), ("0", "caber"), ("1", "tamanho-real")] {
            if input.shortcut(letter) {
                actions.push(action);
            }
        }
        for (letter, action) in [("y", "refazer"), ("o", "abrir"), ("c", "copiar"), ("k", "recortar")] {
            if input.shortcut(letter) {
                actions.push(action);
            }
        }
        let mut nudge = (0.0, 0.0);
        for (key, ctrl, _) in input.key_events() {
            match key {
                Key::Delete if ctrl => actions.push("lixeira"),
                Key::Delete | Key::Backspace if self.selected.is_some() => actions.push("apagar"),
                Key::Enter if self.cropping.is_some() => actions.push("recortar"),
                Key::Escape if self.cropping.is_some() => {
                    self.cropping = None;
                    self.drag = None;
                    tracing::info!("editor: recorte cancelado");
                }
                Key::Escape => self.selected = None,
                Key::Left => nudge.0 -= 1.0,
                Key::Right => nudge.0 += 1.0,
                Key::Up => nudge.1 -= 1.0,
                Key::Down => nudge.1 += 1.0,
                _ => {}
            }
        }
        if nudge != (0.0, 0.0) {
            self.restyle(|s| s.translate(nudge));
        }
        if !input.ctrl && !input.alt && !input.logo {
            for ch in input.text.chars() {
                match ch {
                    'r' => actions.push("recortar"),
                    'v' => actions.push("selecionar"),
                    's' => actions.push("esboco"),
                    'd' => actions.push("desenho"),
                    't' => actions.push("texto"),
                    _ => {}
                }
            }
        }
        for action in actions {
            self.command(action, ui);
        }
    }

    /// Barra de ferramentas do Tahoe, como a da Marcação do Mac.
    fn toolbar(&mut self, ui: &mut Ui) {
        let mut clicked: Option<String> = None;
        let mut pop: Option<(Pop, RectF)> = None;
        let cropping = self.cropping.is_some();
        let has_doc = self.doc.is_some();
        let (can_undo, can_redo) = self.doc.as_ref().map_or((false, false), |d| (d.can_undo(), d.can_redo()));
        let tool = self.tool;
        let (stroke, fill) = (self.stroke, self.fill);
        let open = self.pop;
        ui.toolbar(|ui| {
            if cropping {
                ui.toolbar_title(&tr!("Drag the handles to choose the area"), 360.0);
                ui.push_right(2.0 * 110.0 + 10.0);
                ui.sized(110.0, |ui| {
                    if ui.button(&tr!("Cancel")).clicked() {
                        clicked = Some("cancelar-recorte".into());
                    }
                });
                ui.sized(110.0, |ui| {
                    if ui.primary_button(&tr!("Crop")).clicked() {
                        clicked = Some("recortar".into());
                    }
                });
                return;
            }
            let shape_on = matches!(tool, Tool::Shape(_));
            let tools = [
                ("tool-pointer-symbolic", "↖", tool == Tool::Select, true),
                ("document-edit-symbolic", "✎", tool == Tool::Sketch, true),
                ("draw-freehand-symbolic", "✍", tool == Tool::Draw, true),
                ("draw-rectangle-symbolic", "▭", shape_on || open == Some(Pop::Shapes), true),
                ("draw-text-symbolic", "T", tool == Tool::Text, true),
            ];
            let picked = ui.toolbar_buttons(&tools);
            let group = ui.last_rect();
            segment_tips(ui, group, &[tr!("Select"), tr!("Sketch"), tr!("Draw"), tr!("Shapes"), tr!("Text")]);
            match picked {
                Some(0) => clicked = Some("selecionar".into()),
                Some(1) => clicked = Some("esboco".into()),
                Some(2) => clicked = Some("desenho".into()),
                Some(3) => pop = Some((Pop::Shapes, segment(group, 3))),
                Some(4) => clicked = Some("texto".into()),
                _ => {}
            }
            let styles = [
                ("format-justify-fill-symbolic", "≡", open == Some(Pop::Style), true),
                ("", "", open == Some(Pop::Border), true),
                ("", "", open == Some(Pop::Fill), true),
                ("format-text-larger-symbolic", "Aa", open == Some(Pop::TextStyle), true),
            ];
            let picked = ui.toolbar_buttons(&styles);
            let group = ui.last_rect();
            segment_tips(ui, group, &[tr!("Shape Style"), tr!("Border Color"), tr!("Fill Color"), tr!("Text Style")]);
            match picked {
                Some(0) => pop = Some((Pop::Style, segment(group, 0))),
                Some(1) => pop = Some((Pop::Border, segment(group, 1))),
                Some(2) => pop = Some((Pop::Fill, segment(group, 2))),
                Some(3) => pop = Some((Pop::TextStyle, segment(group, 3))),
                _ => {}
            }
            // As cores escolhidas nos botões de borda e de preenchimento.
            let (b, f) = (segment(group, 1), segment(group, 2));
            ui.paint(|canvas, (ox, oy)| {
                swatch(canvas, (b.0 - ox, b.1 - oy, b.2, b.3), Some(stroke), true);
                swatch(canvas, (f.0 - ox, f.1 - oy, f.2, f.3), fill, false);
            });
            let picked = ui.toolbar_buttons(&[
                ("object-rotate-left-symbolic", "⟲", false, has_doc),
                ("object-rotate-right-symbolic", "⟳", false, has_doc),
                ("image-crop-symbolic", "⌗", false, has_doc),
            ]);
            segment_tips(ui, ui.last_rect(), &[tr!("Rotate Left"), tr!("Rotate Right"), tr!("Crop")]);
            match picked {
                Some(0) => clicked = Some("girar-esquerda".into()),
                Some(1) => clicked = Some("girar-direita".into()),
                Some(2) => clicked = Some("recortar".into()),
                _ => {}
            }
            ui.push_right(84.0 + 10.0 + 90.0 + 10.0 + 90.0);
            let picked = ui.toolbar_buttons(&[("edit-undo-symbolic", "↶", false, can_undo), ("edit-redo-symbolic", "↷", false, can_redo)]);
            segment_tips(ui, ui.last_rect(), &[tr!("Undo"), tr!("Redo")]);
            match picked {
                Some(0) => clicked = Some("desfazer".into()),
                Some(1) => clicked = Some("refazer".into()),
                _ => {}
            }
            ui.sized(90.0, |ui| {
                if ui.button_styled(&tr!("Copy"), ButtonStyle::Glass).clicked() {
                    clicked = Some("copiar".into());
                }
            });
            ui.sized(90.0, |ui| {
                if ui.button_styled(&tr!("Save"), ButtonStyle::Prominent).clicked() {
                    clicked = Some("salvar".into());
                }
            });
        });
        if let Some((p, anchor)) = pop {
            self.pop = if self.pop == Some(p) { None } else { Some(p) };
            self.pop_anchor = anchor;
        }
        match clicked.as_deref() {
            Some("cancelar-recorte") => {
                self.cropping = None;
                self.drag = None;
            }
            Some(action) => self.command(action, ui),
            None => {}
        }
    }

    /// Conteúdo do popover aberto (Formas, Estilo, cores e texto).
    fn popover(&mut self, ui: &mut Ui) {
        let Some(pop) = self.pop else {
            return;
        };
        let mut open = true;
        let mut action: Option<String> = None;
        let mut style: Option<StyleChange> = None;
        let (stroke, fill, width_level, dashed, font_size) = (self.stroke, self.fill, self.width_level, self.dashed, self.font_size);
        let width = match pop {
            Pop::Shapes => 230.0,
            Pop::Style => 250.0,
            _ => 280.0,
        };
        ui.popover(&mut open, self.pop_anchor, width, |ui| match pop {
            Pop::Shapes => {
                for kind in SHAPES {
                    if ui.button_styled(&shape_name(kind), ButtonStyle::Plain).clicked() {
                        action = Some(shape_action(kind).to_string());
                    }
                }
            }
            Pop::Style => {
                ui.heading(&tr!("Line Width"));
                let mut level = width_level;
                if ui.segmented(&["1", "2", "3", "4", "5"], &mut level, None).changed() {
                    style = Some(Box::new(move |e: &mut Editor| {
                        e.width_level = level;
                        let w = e.stroke_width();
                        e.restyle(|s| s.width = w);
                    }));
                }
                let mut d = dashed;
                if ui.checkbox(&tr!("Dashed"), &mut d).changed() {
                    style = Some(Box::new(move |e: &mut Editor| {
                        e.dashed = d;
                        e.restyle(|s| s.dashed = d);
                    }));
                }
            }
            Pop::Border | Pop::Fill => {
                ui.heading(&if pop == Pop::Border { tr!("Border Color") } else { mide::i18n::trc("shape", "Fill") });
                let current = if pop == Pop::Border { Some(stroke) } else { fill };
                ui.row(|ui| {
                    for (_, c) in PALETTE {
                        let r = ui.tag_toggle([c[0], c[1], c[2], 255], current == Some(c));
                        if r.clicked() {
                            let is_border = pop == Pop::Border;
                            style = Some(Box::new(move |e: &mut Editor| {
                                if is_border {
                                    e.stroke = c;
                                    e.restyle(|s| s.stroke = c);
                                } else {
                                    e.fill = Some(c);
                                    e.restyle(|s| s.fill = Some(c));
                                }
                            }));
                        }
                    }
                });
                if pop == Pop::Fill && ui.button_styled(&tr!("No Fill"), ButtonStyle::Plain).clicked() {
                    style = Some(Box::new(|e: &mut Editor| {
                        e.fill = None;
                        e.restyle(|s| s.fill = None);
                    }));
                }
            }
            Pop::TextStyle => {
                ui.heading(&tr!("Text Size"));
                let mut size = font_size as i64;
                if ui.stepper(&mut size, 8, 200).changed() {
                    style = Some(Box::new(move |e: &mut Editor| {
                        e.font_size = size as f32;
                        let k = e.doc.as_ref().map_or(1.0, |d| (d.width().max(d.height()) as f32 / 1600.0).max(1.0));
                        e.restyle(|s| s.font_size = size as f32 * k);
                    }));
                }
                ui.note(&tr!("The text color is the border color."));
            }
        });
        if !open {
            self.pop = None;
        }
        if let Some(f) = style {
            f(self);
            ui.request_repaint();
        }
        if let Some(a) = action {
            self.pop = None;
            self.command(&a, ui);
        }
    }

    /// A imagem, as formas, o recorte e o desenho em andamento; o ponteiro na imagem.
    /// Zoom: Ctrl+roda (em volta do ponteiro), Ctrl +/−, Ctrl+0 (caber), Ctrl+1 (tamanho
    /// real) e os itens do menu Visualizar; ampliada, a roda desloca a imagem (com Shift,
    /// para os lados).
    fn zoom_input(&mut self, ui: &mut Ui, area: RectF, fit: Mapping) {
        let input = ui.input().clone();
        let max_zoom = 16.0;
        let old_s = fit.s * self.zoom;
        let mut want = self.zoom;
        let mut anchor = None;
        match self.zoom_request.take() {
            Some(f) if f > 0.0 => want = self.zoom * f,
            Some(f) if f == 0.0 => want = 1.0,
            Some(_) => want = (1.0 / mide::ui::draw::scale()) / fit.s,
            None => {}
        }
        if input.scroll != 0.0 && input.pointer.is_some_and(|p| inside(area, p)) {
            if input.ctrl {
                // Roda para cima amplia (cada passo de 15%).
                let steps = -input.scroll / 40.0;
                want = self.zoom * 1.15f32.powf(steps.clamp(-4.0, 4.0));
                anchor = input.pointer;
                ui.take_scroll();
            } else if self.zoom > 1.0 {
                let map = self.last_mapping.unwrap_or(fit);
                if let Some(c) = self.center.as_mut() {
                    let d = input.scroll / map.s;
                    if input.shift {
                        c.0 += d;
                    } else {
                        c.1 += d;
                    }
                }
                ui.take_scroll();
                ui.request_repaint();
            }
        }
        let want = want.clamp(1.0, max_zoom);
        if (want - self.zoom).abs() > 1e-4 {
            let map = self.last_mapping.unwrap_or(fit);
            let center = self.center.unwrap_or((map.source.0 + map.source.2 / 2.0, map.source.1 + map.source.3 / 2.0));
            let new_s = fit.s * want;
            // O ponto da imagem sob o ponteiro continua sob o ponteiro.
            self.center = Some(match anchor {
                Some(p) => {
                    let img = map.to_image(p);
                    (img.0 + (center.0 - img.0) * old_s / new_s, img.1 + (center.1 - img.1) * old_s / new_s)
                }
                None => center,
            });
            if want <= 1.0 {
                self.center = None;
            }
            self.zoom = want;
            tracing::info!("editor: zoom {:.0}%", want * fit.s * mide::ui::draw::scale() * 100.0);
            ui.request_repaint();
        }
    }

    /// Barra de baixo com o zoom (como a do Arquivos): o tamanho em % no meio e o
    /// deslizante à direita, de "caber na janela" (1x) a 16x, em escala logarítmica
    /// (cada pedaço do trilho multiplica o zoom pelo mesmo tanto).
    fn zoom_bar(&mut self, ui: &mut Ui, w: f32, h: f32) {
        const MAX: f32 = 16.0;
        let percent = self.last_mapping.map_or(100.0, |m| m.s * mide::ui::draw::scale() * 100.0);
        let mut value = self.zoom.max(1.0).ln() / MAX.ln();
        let mut changed = false;
        ui.at(0.0, h - 26.0, w, |ui| {
            let status = ui.status_bar(&tr!("Zoom: {percent}%", percent = format!("{percent:.0}")));
            ui.at(status.0 + status.2 - 156.0, status.1 + (status.3 - 18.0) / 2.0, 140.0, |ui| {
                changed = ui.mini_slider(140.0, &mut value, 0.0, 1.0).changed();
            });
        });
        if changed {
            let want = MAX.powf(value.clamp(0.0, 1.0));
            if (want - self.zoom).abs() > 1e-3 {
                self.zoom_request = Some(want / self.zoom.max(1e-3));
            }
            ui.request_repaint();
        }
    }

    fn canvas(&mut self, ui: &mut Ui, area: RectF) {
        let Some(doc) = self.doc.as_ref() else {
            return;
        };
        let full = (0.0, 0.0, doc.width() as f32, doc.height() as f32);
        let source = match self.cropping {
            Some(_) => full,
            None => {
                let c = doc.crop();
                (c.x() as f32, c.y() as f32, c.width() as f32, c.height() as f32)
            }
        };
        let native = 1.0 / mide::ui::draw::scale();
        let fit = Mapping::fit(area, source, native);
        let blocked_now = self.dialog.is_some() || ui.menu_open().is_some() || self.pop.is_some();
        if !blocked_now {
            self.zoom_input(ui, area, fit);
        }
        let map = zoomed(area, source, fit.s, self.zoom, &mut self.center);
        if self.last_mapping != Some(map) {
            let d = map.display;
            tracing::info!("editor: imagem em ({}, {}) {}x{} na escala {:.3}", d.0, d.1, d.2.round(), d.3.round(), map.s);
            self.last_mapping = Some(map);
        }
        let blocked = self.dialog.is_some() || ui.menu_open().is_some() || self.pop.is_some();
        if !blocked {
            self.pointer(ui, map, full);
        }
        let Some(doc) = self.doc.as_ref() else {
            return;
        };
        // A imagem reduzida (sem as formas), refeita só quando ela, a parte ou o tamanho
        // mudam.
        let k = mide::ui::draw::scale();
        let (pw, ph) = (((map.display.2 * k).round() as u32).max(1), ((map.display.3 * k).round() as u32).max(1));
        let content = (std::rc::Rc::as_ptr(&doc.state.base) as usize, [source.0 as i32, source.1 as i32, source.2 as i32, source.3 as i32]);
        let now = std::time::Instant::now();
        let stretched = self.view.as_ref().filter(|v| v.content == content && (v.image.width(), v.image.height()) != (pw, ph)).map(|v| stretch(&v.image, pw, ph));
        if let Some(image) = stretched {
            self.view = Some(ViewCache { content, image, exact: false, at: now });
        }
        let rebuild = self.view.as_ref().is_none_or(|v| v.content != content || (!v.exact && now.duration_since(v.at) >= RESIZE_SETTLE));
        if rebuild {
            let base = doc.state.base.as_ref();
            let whole = tiny_skia::IntRect::from_xywh(0, 0, base.width(), base.height());
            let part = tiny_skia::IntRect::from_xywh(content.1[0], content.1[1], source.2 as u32, source.3 as u32);
            let image = match part.filter(|p| Some(*p) != whole).and_then(|r| base.clone_rect(r)) {
                Some(part) => shrink(&part, pw, ph),
                None => shrink(base, pw, ph),
            };
            self.view = Some(ViewCache { content, image, exact: true, at: now });
        }
        if let Some(v) = self.view.as_ref().filter(|v| !v.exact) {
            ui.wake_after(RESIZE_SETTLE.saturating_sub(now.duration_since(v.at)));
        }
        let view = self.view.as_ref().expect("acabou de ser feita").image.clone();
        let accent = ui.theme().accent;
        let crop = self.cropping.map(|c| map.rect_to_screen(c));
        let shapes = doc.state.shapes.clone();
        // Desenho em andamento (traço à mão ou forma nova).
        let live: Option<Shape> = match &self.drag {
            Some(Drag::Draw(points)) => Some(self.new_shape(ShapeKind::Pen, points.clone())),
            Some(Drag::NewShape { kind, start, end }) => Some(self.new_shape(*kind, vec![*start, *end])),
            _ => None,
        };
        // A forma escolhida: a caixa e as alças (na tela).
        let selection = self.selected.and_then(|i| shapes.get(i)).map(|s| {
            let ends = matches!(s.kind, ShapeKind::Line | ShapeKind::Arrow).then(|| (map.to_screen(s.points[0]), map.to_screen(*s.points.last().unwrap_or(&s.points[0]))));
            (map.rect_to_screen(s.bounds(&mut self.text)), ends, s.kind)
        });
        let caret = self.editing_text.and_then(|i| shapes.get(i)).map(|s| {
            let b = s.bounds(&mut self.text);
            let last = s.text.split('\n').next_back().unwrap_or("");
            let lw = self.text.measure(last, s.font_size, mide::ui::text::FontWeight::Regular);
            let lines = s.text.split('\n').count().max(1) as f32;
            map.rect_to_screen((b.0 + lw + 1.0, b.1 + (lines - 1.0) * s.font_size * 1.3, 2.0, s.font_size * 1.2))
        });
        let tf_img = map.transform(k);
        ui.paint_text(|canvas, text, _| {
            let d = map.display;
            mide::ui::draw::soft_shadow(canvas, d.0, d.1, d.2, d.3, 2.0, 14.0, 4.0, 50);
            mide::ui::draw::blit_px(canvas, &view, mide::ui::draw::px(d.0), mide::ui::draw::px(d.1), 1.0);
            // As formas, recortadas na imagem mostrada.
            let clip = tiny_skia::Mask::new(canvas.width(), canvas.height()).and_then(|mut m| {
                let r = Rect::from_xywh(d.0, d.1, d.2, d.3)?;
                m.fill_path(&PathBuilder::from_rect(r), tiny_skia::FillRule::Winding, false, mide::ui::draw::tf());
                Some(m)
            });
            let mut layer = Pixmap::new(canvas.width(), canvas.height());
            if let Some(layer) = layer.as_mut() {
                for shape in shapes.iter().chain(live.iter()) {
                    annotate::draw(layer, shape, tf_img, text);
                }
                canvas.draw_pixmap(0, 0, layer.as_ref(), &PixmapPaint::default(), Transform::identity(), clip.as_ref());
            }
            let tf = mide::ui::draw::tf();
            if let Some((rect, ends, _)) = selection {
                match ends {
                    Some((a, b)) => {
                        for p in [a, b] {
                            handle_dot(canvas, p, accent, tf);
                        }
                    }
                    None => {
                        let mut paint = Paint { anti_alias: true, ..Default::default() };
                        paint.set_color(Color::from_rgba8(accent[0], accent[1], accent[2], 200));
                        if let Some(r) = Rect::from_xywh(rect.0, rect.1, rect.2.max(1.0), rect.3.max(1.0)) {
                            let stroke = Stroke { width: 1.0, dash: tiny_skia::StrokeDash::new(vec![4.0, 3.0], 0.0), ..Default::default() };
                            canvas.stroke_path(&PathBuilder::from_rect(r), &paint, &stroke, tf, None);
                        }
                        draw_handles_with(canvas, rect, accent, tf);
                    }
                }
            }
            if let Some(c) = caret {
                mide::ui::draw::fill_rounded(canvas, c.0, c.1, 1.5, c.3, 0.5, Color::from_rgba8(accent[0], accent[1], accent[2], 255));
            }
            if let Some(c) = crop {
                // Fora da área: escurecido.
                let mut dim = Paint::default();
                dim.set_color(Color::from_rgba8(0, 0, 0, 120));
                let (x1, y1) = (c.0 + c.2, c.1 + c.3);
                for (x, y, w, h) in [(d.0, d.1, d.2, c.1 - d.1), (d.0, y1, d.2, d.1 + d.3 - y1), (d.0, c.1, c.0 - d.0, c.3), (x1, c.1, d.0 + d.2 - x1, c.3)] {
                    if let Some(r) = Rect::from_xywh(x, y, w, h) {
                        canvas.fill_rect(r, &dim, tf, None);
                    }
                }
                outline_with(canvas, c, tf);
                draw_handles_with(canvas, c, accent, tf);
            }
        });
    }

    fn pointer(&mut self, ui: &mut Ui, map: Mapping, full: RectF) {
        let input = ui.input().clone();
        let Some(p) = input.pointer else {
            return;
        };
        let img = map.to_image(p);
        let tol = 6.0 / map.s;
        if input.pressed {
            if let Some(crop) = self.cropping {
                let screen = map.rect_to_screen(crop);
                self.drag = Some(match handle_at(screen, p) {
                    Some(handle) => Drag::CropResize { handle, orig: crop, start: img },
                    None if inside(screen, p) => Drag::CropMove { orig: crop, start: img },
                    None if inside(map.display, p) => Drag::CropNew { start: clamp_point(img, full) },
                    None => return,
                });
            } else if inside(map.display, p) || self.selected.is_some() {
                self.press(img, p, map, tol);
            }
        }
        let delta = |start: (f32, f32)| (img.0 - start.0, img.1 - start.1);
        let mut changed = false;
        match &mut self.drag {
            Some(Drag::Draw(points)) => {
                let q = clamp_point(img, map.source);
                if points.last().is_none_or(|l| (l.0 - q.0).hypot(l.1 - q.1) * map.s >= 1.0) {
                    points.push(q);
                }
            }
            Some(Drag::NewShape { end, .. }) => *end = clamp_point(img, map.source),
            Some(Drag::CropNew { start }) => {
                let r = normalized(*start, clamp_point(img, full));
                if r.2 >= 2.0 && r.3 >= 2.0 {
                    self.cropping = Some(r);
                }
            }
            Some(Drag::CropResize { handle, orig, start }) => self.cropping = Some(resize_area(*orig, *handle, delta(*start), full)),
            Some(Drag::CropMove { orig, start }) => self.cropping = Some(moved(*orig, delta(*start), full)),
            Some(Drag::Move { index, start, orig, began }) | Some(Drag::Resize { index, start, orig, began, .. }) | Some(Drag::End { index, start, orig, began, .. }) => {
                let d = delta(*start);
                if !*began && (d.0.hypot(d.1) * map.s) < 2.0 {
                    // Ainda não andou: um clique só escolhe.
                } else {
                    let first = !*began;
                    *began = true;
                    let (index, orig) = (*index, orig.clone());
                    let mut shape = orig.clone();
                    if let Some(Drag::Resize { handle, orig_box, .. }) = &self.drag {
                        let new_box = annotate::resize_box(*orig_box, *handle, d);
                        shape.fit_to(*orig_box, new_box);
                    } else if let Some(Drag::End { which, .. }) = &self.drag {
                        let i = if *which == 0 { 0 } else { shape.points.len() - 1 };
                        shape.points[i] = (orig.points[i].0 + d.0, orig.points[i].1 + d.1);
                    } else {
                        shape.translate(d);
                    }
                    if let Some(doc) = self.doc.as_mut() {
                        if first {
                            doc.checkpoint();
                        } else {
                            doc.touched();
                        }
                        if let Some(s) = doc.state.shapes.get_mut(index) {
                            *s = shape;
                        }
                    }
                    changed = true;
                }
            }
            None => {}
        }
        if self.drag.is_some() || changed {
            ui.request_repaint();
        }
        if input.released {
            match self.drag.take() {
                Some(Drag::Draw(points)) => self.finish_stroke(points, map),
                Some(Drag::NewShape { kind, start, end }) => {
                    if (start.0 - end.0).hypot(start.1 - end.1) * map.s >= 4.0 {
                        let shape = self.new_shape(kind, vec![start, end]);
                        if let Some(doc) = self.doc.as_mut() {
                            self.selected = Some(doc.add(shape));
                            tracing::info!("editor: {} desenhado", Tool::Shape(kind).name().to_lowercase());
                        }
                    }
                }
                Some(Drag::CropNew { .. } | Drag::CropResize { .. } | Drag::CropMove { .. }) => {
                    if let Some(c) = self.cropping {
                        tracing::info!("editor: área do recorte {}x{} em ({}, {})", c.2.round(), c.3.round(), c.0.round(), c.1.round());
                    }
                }
                _ => {}
            }
        }
    }

    /// Apertou na imagem: escolher/mover/redimensionar uma forma, começar um desenho,
    /// uma forma nova ou um texto.
    fn press(&mut self, img: (f32, f32), p: (f32, f32), map: Mapping, tol: f32) {
        let Some(doc) = self.doc.as_ref() else {
            return;
        };
        // Alças da forma escolhida primeiro.
        if let Some(i) = self.selected.filter(|i| *i < doc.state.shapes.len()) {
            let shape = &doc.state.shapes[i];
            if matches!(shape.kind, ShapeKind::Line | ShapeKind::Arrow) {
                let ends = [shape.points[0], *shape.points.last().unwrap_or(&shape.points[0])];
                if let Some(which) = ends.iter().position(|e| {
                    let s = map.to_screen(*e);
                    (s.0 - p.0).hypot(s.1 - p.1) <= HANDLE_HIT
                }) {
                    self.drag = Some(Drag::End { index: i, which, start: img, orig: shape.clone(), began: false });
                    return;
                }
            } else {
                let b = shape.bounds(&mut self.text);
                if let Some(handle) = handle_at(map.rect_to_screen(b), p) {
                    self.drag = Some(Drag::Resize { index: i, handle, start: img, orig_box: b, orig: shape.clone(), began: false });
                    return;
                }
            }
        }
        // Clique numa forma (a de cima primeiro), fora do desenho à mão: escolhe e move.
        let drawing = matches!(self.tool, Tool::Sketch | Tool::Draw);
        if !drawing || self.tool == Tool::Select {
            let hit = doc.state.shapes.iter().enumerate().rev().find(|(_, s)| s.hit(img, tol, &mut self.text)).map(|(i, s)| (i, s.clone()));
            if let Some((i, shape)) = hit {
                // Clique num texto com a ferramenta Texto: volta a digitar nele.
                if self.tool == Tool::Text && shape.kind == ShapeKind::Text {
                    self.editing_text = Some(i);
                }
                self.selected = Some(i);
                self.drag = Some(Drag::Move { index: i, start: img, orig: shape, began: false });
                return;
            }
        }
        self.selected = None;
        self.editing_text = None;
        if !inside(map.display, p) {
            return;
        }
        match self.tool {
            Tool::Select => {}
            Tool::Sketch | Tool::Draw => self.drag = Some(Drag::Draw(vec![img])),
            Tool::Shape(kind) => self.drag = Some(Drag::NewShape { kind, start: img, end: img }),
            Tool::Text => {
                let mut shape = self.new_shape(ShapeKind::Text, vec![img]);
                shape.fill = None;
                if let Some(doc) = self.doc.as_mut() {
                    let i = doc.add(shape);
                    self.selected = Some(i);
                    self.editing_text = Some(i);
                    tracing::info!("editor: caixa de texto");
                }
            }
        }
    }

    /// Fim de um traço à mão: no Esboço, vira a forma reconhecida (já escolhida para
    /// ajustar); senão, fica o desenho livre.
    fn finish_stroke(&mut self, points: Vec<(f32, f32)>, map: Mapping) {
        let recognized = if self.tool == Tool::Sketch {
            // O reconhecimento trabalha em pontos da tela (o tamanho do traço na tela).
            let screen: Vec<(f32, f32)> = points.iter().map(|p| map.to_screen(*p)).collect();
            annotate::recognize(&screen).map(|(kind, pts)| (kind, pts.into_iter().map(|q| map.to_image(q)).collect::<Vec<_>>()))
        } else {
            None
        };
        // Cabeça de seta num segundo traço, perto da ponta da linha que acabou de ser
        // feita: a linha vira seta (como no Mac).
        if self.tool == Tool::Sketch && !matches!(recognized, Some((ShapeKind::Line | ShapeKind::Rect | ShapeKind::Oval | ShapeKind::Star, _))) {
            let screen: Vec<(f32, f32)> = points.iter().map(|p| map.to_screen(*p)).collect();
            if let Some(doc) = self.doc.as_mut() {
                let last = doc.state.shapes.len().checked_sub(1);
                let line = last.and_then(|i| doc.state.shapes.get(i).filter(|s| s.kind == ShapeKind::Line).map(|s| (i, s.points[0], s.points[s.points.len() - 1])));
                if let Some((i, a, b)) = line
                    && let Some(end) = annotate::head_for_line(&screen, map.to_screen(a), map.to_screen(b))
                {
                    doc.checkpoint();
                    let shape = &mut doc.state.shapes[i];
                    shape.kind = ShapeKind::Arrow;
                    shape.points = if end == 1 { vec![a, b] } else { vec![b, a] };
                    self.selected = Some(i);
                    tracing::info!("editor: cabeça desenhada: a linha virou seta");
                    return;
                }
            }
        }
        let shape = match recognized {
            Some((kind, pts)) => {
                tracing::info!("editor: esboço reconhecido como {kind:?}");
                self.new_shape(kind, pts)
            }
            None => self.new_shape(ShapeKind::Pen, points),
        };
        if let Some(doc) = self.doc.as_mut() {
            let i = doc.add(shape);
            if recognized_kind_selectable(&doc.state.shapes[i]) {
                self.selected = Some(i);
            }
        }
    }

    fn dialog(&mut self, ui: &mut Ui) {
        let Some(dialog) = self.dialog.as_mut() else {
            return;
        };
        match dialog {
            Dialog::Open(chooser) => match chooser.show(ui) {
                Some(Choice::Chosen(path)) => {
                    self.dialog = None;
                    self.open_chosen(path);
                }
                Some(Choice::Cancelled) => {
                    self.dialog = None;
                    // Aberto sem imagem e sem escolher nenhuma: não há o que editar.
                    if self.doc.is_none() {
                        ui.close();
                    }
                }
                None => {}
            },
            Dialog::SaveAs { chooser, then_close } => {
                let then_close = *then_close;
                match chooser.show(ui) {
                    Some(Choice::Chosen(chosen)) => {
                        self.dialog = None;
                        let path = with_image_extension(chosen.clone());
                        if path != chosen && path.exists() {
                            self.dialog = Some(Dialog::ConfirmReplace { path, then_close });
                        } else if self.write_to(&path) {
                            self.path = Some(path);
                            if then_close {
                                ui.close();
                            }
                        }
                    }
                    Some(Choice::Cancelled) => self.dialog = None,
                    None => {}
                }
            }
            Dialog::ConfirmReplace { path, then_close } => {
                let (path, then_close) = (path.clone(), *then_close);
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                match ui.alert(&tr!("“{name}” already exists. Replace it?", name = name), &tr!("The file that is there will be replaced."), &[&tr!("Cancel"), &tr!("Replace")]) {
                    Some(0) => self.dialog = None,
                    Some(_) => {
                        self.dialog = None;
                        if self.write_to(&path) {
                            self.path = Some(path);
                            if then_close {
                                ui.close();
                            }
                        }
                    }
                    None => {}
                }
            }
            Dialog::ConfirmClose => {
                let name = self.name();
                match ui.alert(&tr!("Save the changes to “{name}”?", name = name), &tr!("Your changes will be lost if you don’t save them."), &[&tr!("Cancel"), &tr!("Don’t Save"), &tr!("Save")]) {
                    Some(0) => self.dialog = None,
                    Some(1) => {
                        tracing::info!("editor: fechado sem salvar");
                        ui.close();
                    }
                    Some(_) => {
                        self.dialog = None;
                        if self.save(true) {
                            ui.close();
                        }
                    }
                    None => {}
                }
            }
        }
    }
}

/// Formas que ficam escolhidas depois de desenhadas (o desenho livre não).
fn recognized_kind_selectable(shape: &Shape) -> bool {
    shape.kind != ShapeKind::Pen
}

/// Ação do menu de uma forma ("forma-retangulo"...).
fn shape_action(kind: ShapeKind) -> &'static str {
    match kind {
        ShapeKind::Rect => "forma-retangulo",
        ShapeKind::RoundRect => "forma-arredondado",
        ShapeKind::Oval => "forma-oval",
        ShapeKind::Line => "forma-linha",
        ShapeKind::Arrow => "forma-seta",
        ShapeKind::Triangle => "forma-triangulo",
        ShapeKind::Star => "forma-estrela",
        ShapeKind::Bubble => "forma-balao",
        ShapeKind::Pen | ShapeKind::Text => "",
    }
}

/// Retângulo de um botão `i` num grupo de `toolbar_buttons` (40 pontos cada).
fn segment(group: RectF, i: usize) -> RectF {
    (group.0 + 2.0 + i as f32 * 40.0, group.1, 40.0, group.3)
}

/// Bolinha de cor num botão da barra (borda: anel; preenchimento: cheia; sem cor: risco).
fn swatch(canvas: &mut Pixmap, r: RectF, color: Option<[u8; 3]>, ring: bool) {
    let (cx, cy) = (r.0 + r.2 / 2.0, r.1 + r.3 / 2.0);
    let tf = mide::ui::draw::tf();
    let mut paint = Paint { anti_alias: true, ..Default::default() };
    match color {
        Some(c) => {
            paint.set_color(Color::from_rgba8(c[0], c[1], c[2], 255));
            if let Some(circle) = PathBuilder::from_circle(cx, cy, 8.0) {
                if ring {
                    canvas.stroke_path(&circle, &paint, &Stroke { width: 3.0, ..Default::default() }, tf, None);
                } else {
                    canvas.fill_path(&circle, &paint, tiny_skia::FillRule::Winding, tf, None);
                }
            }
        }
        None => {
            paint.set_color(Color::from_rgba8(140, 140, 146, 255));
            if let Some(circle) = PathBuilder::from_circle(cx, cy, 8.0) {
                canvas.stroke_path(&circle, &paint, &Stroke { width: 1.2, ..Default::default() }, tf, None);
            }
            let mut pb = PathBuilder::new();
            pb.move_to(cx - 5.5, cy + 5.5);
            pb.line_to(cx + 5.5, cy - 5.5);
            if let Some(line) = pb.finish() {
                paint.set_color(Color::from_rgba8(255, 59, 48, 255));
                canvas.stroke_path(&line, &paint, &Stroke { width: 1.5, ..Default::default() }, tf, None);
            }
        }
    }
}

/// Alça redonda (ponta de linha e seta).
fn handle_dot(canvas: &mut Pixmap, p: (f32, f32), accent: [u8; 4], tf: Transform) {
    if let Some(c) = PathBuilder::from_circle(p.0, p.1, HANDLE_RADIUS) {
        let mut paint = Paint { anti_alias: true, ..Default::default() };
        paint.set_color(Color::from_rgba8(accent[0], accent[1], accent[2], 255));
        canvas.fill_path(&c, &paint, tiny_skia::FillRule::Winding, tf, None);
        paint.set_color(Color::WHITE);
        canvas.stroke_path(&c, &paint, &Stroke { width: 1.5, ..Default::default() }, tf, None);
    }
}

impl App for Editor {
    fn view(&mut self, ui: &mut Ui) {
        ui.set_title(&tr!("{name} — Screenshot Editor", name = format!("{}{}", if self.modified() { "• " } else { "" }, self.name())));
        for action in ui.input().menu.clone() {
            self.command(&action, ui);
        }
        // Imagem solta na janela: abre (nesta, se está vazia; senão, numa nova).
        if let Some(paths) = ui.input().dropped.clone() {
            for path in paths.into_iter().filter(|p| mide::places::is_image(p)) {
                self.open_chosen(path);
            }
            ui.request_repaint();
        }
        if self.dialog.is_none() && !ui.typing() && self.pop.is_none() {
            self.keys(ui);
        }
        self.toolbar(ui);
        if let Some(message) = &self.message {
            ui.row(|ui| {
                ui.space_x(20.0);
                ui.note(message);
            });
        }
        let (w, h) = ui.window_size();
        let top = h - ui.remaining_height();
        let area = (20.0, top + 10.0, (w - 40.0).max(1.0), (h - top - 30.0).max(1.0));
        if self.doc.is_some() {
            self.canvas(ui, area);
            self.zoom_bar(ui, w, h);
        } else if self.dialog.is_none() {
            ui.label(&tr!("No image open."));
            ui.sized(140.0, |ui| {
                if ui.primary_button(&tr!("Open Image…")).clicked() {
                    self.ask_open();
                }
            });
        }
        self.popover(ui);
        self.dialog(ui);
    }

    fn menu_bar(&mut self) -> Vec<MenuGroup> {
        let has = self.doc.is_some();
        let has_file = has && self.path.is_some();
        let (undo, redo) = self.doc.as_ref().map_or((false, false), |d| (d.can_undo(), d.can_redo()));
        let mut tools = vec![
            MenuItem::action("recortar", &tr!("Crop")).shortcut("Ctrl+K").enabled(has),
            MenuItem::action("girar-esquerda", &tr!("Rotate Left")).enabled(has),
            MenuItem::action("girar-direita", &tr!("Rotate Right")).enabled(has),
            MenuItem::Separator,
        ];
        for (tool, action) in [(Tool::Select, "selecionar"), (Tool::Sketch, "esboco"), (Tool::Draw, "desenho"), (Tool::Text, "texto")] {
            tools.push(MenuItem::action(action, &tool.name()).checked(self.tool == tool && self.cropping.is_none()));
        }
        tools.push(MenuItem::Separator);
        for kind in SHAPES {
            tools.push(MenuItem::action(shape_action(kind), &shape_name(kind)).checked(self.tool == Tool::Shape(kind) && self.cropping.is_none()));
        }
        vec![
            MenuGroup {
                title: tr!("File").into(),
                items: vec![
                    MenuItem::action("abrir", &tr!("Open…")).shortcut("Ctrl+O"),
                    MenuItem::Separator,
                    MenuItem::action("salvar", &tr!("Save")).shortcut("Ctrl+S").enabled(has),
                    MenuItem::action("salvar-como", &tr!("Save As…")).shortcut("Ctrl+Shift+S").enabled(has),
                    MenuItem::Separator,
                    MenuItem::action("mostrar", &tr!("Show in Files")).enabled(has_file),
                    MenuItem::action("lixeira", &tr!("Move to Trash")).shortcut("Ctrl+Delete").enabled(has_file),
                ],
            },
            MenuGroup {
                title: tr!("Edit").into(),
                items: vec![
                    MenuItem::action("desfazer", &tr!("Undo")).shortcut("Ctrl+Z").enabled(undo),
                    MenuItem::action("refazer", &tr!("Redo")).shortcut("Ctrl+Shift+Z").enabled(redo),
                    MenuItem::Separator,
                    MenuItem::action("copiar", &tr!("Copy")).shortcut("Ctrl+C").enabled(has),
                ],
            },
            MenuGroup {
                title: tr!("View").into(),
                items: vec![
                    MenuItem::action("ampliar", &tr!("Zoom In")).shortcut("Ctrl+=").enabled(has),
                    MenuItem::action("reduzir", &tr!("Zoom Out")).shortcut("Ctrl+-").enabled(has && self.zoom > 1.0),
                    MenuItem::action("tamanho-real", &tr!("Actual Size")).shortcut("Ctrl+1").enabled(has),
                    MenuItem::action("caber", &tr!("Zoom to Fit")).shortcut("Ctrl+0").enabled(has && self.zoom > 1.0),
                ],
            },
            MenuGroup { title: tr!("Tools").into(), items: tools },
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

/// `src` reduzida para `w`×`h` pixels: metade por vez (média de 2x2 pelo filtro
/// bilinear) enquanto passa do dobro, e o resto de uma vez; sem serrilhado numa
/// captura grande reduzida para uma miniatura.
pub fn shrink(src: &Pixmap, w: u32, h: u32) -> Pixmap {
    // As metades começam da imagem emprestada: só as reduzidas são novas.
    let mut owned: Option<Pixmap> = None;
    loop {
        let cur = owned.as_ref().unwrap_or(src);
        if cur.width() < 2 * w.max(1) || cur.height() < 2 * h.max(1) {
            break;
        }
        let Some(mut half) = Pixmap::new(cur.width().div_ceil(2), cur.height().div_ceil(2)) else {
            break;
        };
        let paint = PixmapPaint { quality: FilterQuality::Bilinear, ..Default::default() };
        half.draw_pixmap(0, 0, cur.as_ref(), &paint, Transform::from_scale(0.5, 0.5), None);
        owned = Some(half);
    }
    match owned {
        Some(cur) if (cur.width(), cur.height()) == (w, h) => cur,
        Some(cur) => stretch(&cur, w, h),
        None => stretch(src, w, h),
    }
}

/// A vista com zoom: em 1, a imagem inteira cabendo em `area` (escala `fit_s`); acima
/// disso, só o pedaço visível de `whole`, em volta de `center` (que fica preso para a
/// vista não passar da imagem).
fn zoomed(area: RectF, whole: RectF, fit_s: f32, zoom: f32, center: &mut Option<(f32, f32)>) -> Mapping {
    let s = fit_s * zoom.max(1.0);
    let (vw, vh) = ((area.2 / s).min(whole.2), (area.3 / s).min(whole.3));
    let c = center.unwrap_or((whole.0 + whole.2 / 2.0, whole.1 + whole.3 / 2.0));
    let cx = c.0.clamp(whole.0 + vw / 2.0, whole.0 + whole.2 - vw / 2.0);
    let cy = c.1.clamp(whole.1 + vh / 2.0, whole.1 + whole.3 - vh / 2.0);
    if zoom > 1.0 {
        *center = Some((cx, cy));
    }
    let source = (cx - vw / 2.0, cy - vh / 2.0, vw, vh);
    let (w, h) = (vw * s, vh * s);
    Mapping { display: ((area.0 + (area.2 - w) / 2.0).round(), (area.1 + (area.3 - h) / 2.0).round(), w, h), source, s }
}

/// `src` esticada (ou encolhida) para `w`×`h` pixels de uma vez, pelo filtro bilinear.
fn stretch(src: &Pixmap, w: u32, h: u32) -> Pixmap {
    let Some(mut out) = Pixmap::new(w.max(1), h.max(1)) else {
        return src.clone();
    };
    if (src.width(), src.height()) == (out.width(), out.height()) {
        return src.clone();
    }
    let paint = PixmapPaint { quality: FilterQuality::Bilinear, ..Default::default() };
    out.draw_pixmap(0, 0, src.as_ref(), &paint, Transform::from_scale(out.width() as f32 / src.width() as f32, out.height() as f32 / src.height() as f32), None);
    out
}

/// O nome com uma extensão que o editor grava (`.png`, `.jpg`, `.jpeg`); com outra ou
/// sem nenhuma, ganha `.png` (o formato gravado), em vez de um PNG com nome de outro
/// formato.
fn with_image_extension(path: PathBuf) -> PathBuf {
    let ext = path.extension().and_then(|e| e.to_str()).map(str::to_lowercase);
    if matches!(ext.as_deref(), Some("png" | "jpg" | "jpeg")) {
        return path;
    }
    let mut name = path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(".png");
    path.with_file_name(name)
}

/// Mostra o arquivo selecionado no Arquivos do mide (nos testes, só registra).
pub fn show_in_files(path: &Path) {
    if std::env::var_os("MIDE_OPEN_DRY_RUN").is_some() {
        tracing::info!("mostrar no Arquivos (teste): {}", path.display());
        return;
    }
    match std::process::Command::new(sibling_program("mide-files")).arg("--select").arg(path).spawn() {
        Ok(_) => tracing::info!("mostrando no Arquivos: {}", path.display()),
        Err(err) => tracing::warn!("não abriu o Arquivos: {err}"),
    }
}

/// Dica de cada botão de um grupo da barra (`toolbar_buttons`): o grupo não tem dicas
/// próprias, então cada segmento ganha uma resposta com o ponteiro em cima (I2).
fn segment_tips(ui: &mut Ui, group: RectF, tips: &[std::borrow::Cow<'_, str>]) {
    let pointer = ui.input().pointer;
    for (i, tip) in tips.iter().enumerate() {
        let r = segment(group, i);
        let (x, y, w, h) = ui.to_window(r);
        let hovered = pointer.is_some_and(|(px, py)| px >= x && px < x + w && py >= y && py < y + h);
        // Número próprio da dica, longe dos números dos componentes do quadro.
        let id = tip.bytes().fold(0xD1CA_0000_0000_u64, |h, b| h.wrapping_mul(31).wrapping_add(u64::from(b)));
        ui.tooltip(&Response { hovered, rect: r, id, ..Default::default() }, tip);
    }
}

/// Pasta das capturas (Imagens/Capturas de tela), ou Imagens.
fn default_dir() -> PathBuf {
    let pictures = mide::places::pictures_dir();
    let shots = pictures.join("Capturas de tela");
    if shots.is_dir() { shots } else { pictures }
}

fn main() -> Result<(), Box<dyn Error>> {
    let _logging = mide::logging::init("mide-screenshot-editor", "mide_screenshot_editor=info");
    // `args_os`: nomes que não são UTF-8 também abrem.
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    if args.first().is_some_and(|a| a == "--miniatura") {
        let path = args.get(1).map(PathBuf::from).ok_or("uso: mide-screenshot-editor --miniatura <arquivo>")?;
        return thumbnail::run_thumbnail(path);
    }
    let path = args.first().map(|a| {
        let text = a.to_string_lossy();
        mide::places::path_from_uri(&text).unwrap_or_else(|| PathBuf::from(a))
    });
    mide::i18n::init();
    let window = Window::new(&tr!("Screenshot Editor")).app_id("mide-screenshot-editor").size(1000, 640).min_size(760, 440).full_size_content().accepts_files();
    run(window, Editor::new(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imagem_cabe_no_meio_sem_ampliar() {
        // Imagem pequena: tamanho real, no meio.
        let m = Mapping::fit((20.0, 60.0, 940.0, 600.0), (0.0, 0.0, 400.0, 300.0), 1.0);
        assert_eq!(m.s, 1.0);
        assert_eq!(m.display, (290.0, 210.0, 400.0, 300.0));
        // Tela 2x: 400x300 pixels ocupam 200x150 pontos (um pixel por pixel da tela).
        let m = Mapping::fit((20.0, 60.0, 940.0, 600.0), (0.0, 0.0, 400.0, 300.0), 1.0 / 2.0);
        assert_eq!(m.s, 0.5);
        assert_eq!(m.display, (390.0, 285.0, 200.0, 150.0));
        // Em 1,25: 320x240 pontos.
        let m = Mapping::fit((20.0, 60.0, 940.0, 600.0), (0.0, 0.0, 400.0, 300.0), 1.0 / 1.25);
        assert_eq!((m.display.2, m.display.3), (320.0, 240.0));
        // Grande: reduzida para caber na altura.
        let m = Mapping::fit((0.0, 0.0, 1000.0, 500.0), (100.0, 0.0, 1366.0, 768.0), 1.0);
        assert!((m.display.3 - 500.0).abs() < 0.01);
        let back = m.to_image((m.display.0, m.display.1));
        assert!((back.0 - 100.0).abs() < 0.01 && back.1.abs() < 0.01);
        let r = m.rect_to_screen((100.0, 0.0, 1366.0, 768.0));
        assert!((r.2 - m.display.2).abs() < 0.01);
    }

    #[test]
    fn reduzir_sem_mudar_a_cor() {
        let mut src = Pixmap::new(1366, 768).unwrap();
        src.fill(Color::from_rgba8(10, 132, 255, 255));
        let small = shrink(&src, 180, 101);
        assert_eq!((small.width(), small.height()), (180, 101));
        let p = small.pixel(90, 50).unwrap();
        assert_eq!((p.red(), p.green(), p.blue(), p.alpha()), (10, 132, 255, 255));
        // Sem reduzir (já do tamanho), e esticar a reduzida (redimensionando a janela).
        assert_eq!(shrink(&small, 180, 101).data(), small.data());
        let big = stretch(&small, 360, 202);
        assert_eq!((big.width(), big.height()), (360, 202));
        assert_eq!(big.pixel(180, 100).unwrap().blue(), 255);
    }

    #[test]
    fn salvar_como_com_extensao_de_imagem() {
        let p = |s: &str| with_image_extension(PathBuf::from(s));
        assert_eq!(p("/x/Captura"), PathBuf::from("/x/Captura.png"));
        assert_eq!(p("/x/a.webp"), PathBuf::from("/x/a.webp.png"));
        assert_eq!(p("/x/a.bmp"), PathBuf::from("/x/a.bmp.png"));
        assert_eq!(p("/x/a.png"), PathBuf::from("/x/a.png"));
        assert_eq!(p("/x/a.JPG"), PathBuf::from("/x/a.JPG"));
        assert_eq!(p("/x/a.jpeg"), PathBuf::from("/x/a.jpeg"));
    }
}
