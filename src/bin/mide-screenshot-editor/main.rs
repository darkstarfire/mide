//! Editor de Capturas do mide (B23): abre uma imagem (a captura que acabou de ser
//! tirada, ou qualquer PNG/JPEG pelo "Abrir com") para recortar e marcar.
//!
//! - **Recortar** (Ctrl+K ou R): a área ajustável com bolinhas, como a da Captura;
//!   Enter (ou o botão Recortar) aplica, Esc cancela. O recorte não apaga nada: desfazer
//!   traz de volta o que estava fora.
//! - **Lápis** (L), **Seta** (S), **Quadrado** (Q) e **Círculo** (C), nas cores da
//!   Captura.
//! - Desfazer (Ctrl+Z) e refazer (Ctrl+Shift+Z, Ctrl+Y); **Salvar** (Ctrl+S) substitui o
//!   arquivo; **Salvar como…** (Ctrl+Shift+S); **Copiar** (Ctrl+C); **Mover para a
//!   Lixeira** (Ctrl+Delete). Os mesmos comandos nos menus da barra.
//!
//! Com `--miniatura <arquivo>`, é a miniatura que aparece no canto da tela depois de uma
//! captura (`thumbnail.rs`).

mod doc;
mod thumbnail;

use std::{
    error::Error,
    path::{Path, PathBuf},
};

use doc::{Doc, Mark, Op};
use mide::{
    control_client::ask_compositor,
    kit::{run, window_menu, App, Choice, FileChooser, Key, MenuGroup, MenuItem, Ui, Window},
    markup::*,
};
use tiny_skia::{Color, FillRule, FilterQuality, Paint, PathBuilder, Pixmap, PixmapPaint, Rect, Stroke, Transform};

/// Ferramenta de desenho.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tool {
    Pen,
    Arrow,
    Square,
    Circle,
}

impl Tool {
    /// Ferramenta, nome, ação dos menus, tecla e largura do botão.
    const ALL: [(Tool, &'static str, &'static str, char, f32); 4] = [
        (Tool::Pen, "Lápis", "lapis", 'l', 70.0),
        (Tool::Arrow, "Seta", "seta", 's', 64.0),
        (Tool::Square, "Quadrado", "quadrado", 'q', 86.0),
        (Tool::Circle, "Círculo", "circulo", 'c', 76.0),
    ];

    fn kind(self) -> MarkKind {
        match self {
            Tool::Pen => MarkKind::Pen,
            Tool::Arrow => MarkKind::Arrow,
            Tool::Square => MarkKind::Square,
            Tool::Circle => MarkKind::Circle,
        }
    }

    fn name(self) -> &'static str {
        Tool::ALL.iter().find(|t| t.0 == self).map_or("", |t| t.1)
    }
}

/// Arrasto em andamento, em pixels da imagem.
enum Drag {
    /// Desenhando: os pontos por onde passou.
    Draw(Vec<(f32, f32)>),
    /// Recorte novo a partir de `start`.
    CropNew { start: (f32, f32) },
    /// Recorte mudando de tamanho pela bolinha `handle`.
    CropResize { handle: Handle, orig: RectF, start: (f32, f32) },
    /// Recorte mudando de lugar.
    CropMove { orig: RectF, start: (f32, f32) },
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

    fn rect_to_screen(self, r: RectF) -> RectF {
        (self.display.0 + (r.0 - self.source.0) * self.s, self.display.1 + (r.1 - self.source.1) * self.s, r.2 * self.s, r.3 * self.s)
    }

    /// Dos pixels da imagem aos pixels do canvas (escala da tela `k`).
    fn transform(self, k: f32) -> Transform {
        let s = self.s * k;
        Transform::from_row(s, 0.0, 0.0, s, (self.display.0 - self.source.0 * self.s) * k, (self.display.1 - self.source.1 * self.s) * k)
    }
}

/// A imagem já reduzida para a tela, guardada até mudar a versão, a parte ou o tamanho.
/// Enquanto a janela muda de tamanho, a anterior é só esticada (rápido); a redução
/// boa é refeita quando o tamanho para de mudar (`RESIZE_SETTLE`).
struct ViewCache {
    /// Versão do documento e a parte mostrada.
    content: (u64, [i32; 4]),
    image: Pixmap,
    /// `false`: esticada da anterior, ainda por refazer (desde `at`).
    exact: bool,
    at: std::time::Instant,
}

const RESIZE_SETTLE: std::time::Duration = std::time::Duration::from_millis(150);

struct Editor {
    path: Option<PathBuf>,
    doc: Option<Doc>,
    tool: Tool,
    color: usize,
    /// Recortando: a área escolhida (pixels da imagem).
    cropping: Option<RectF>,
    drag: Option<Drag>,
    dialog: Option<Dialog>,
    message: Option<String>,
    view: Option<ViewCache>,
    /// Onde a imagem apareceu no último quadro (para o log da bateria).
    last_mapping: Option<Mapping>,
}

impl Editor {
    fn new(path: Option<PathBuf>) -> Self {
        let mut editor = Self { path: None, doc: None, tool: Tool::Pen, color: 0, cropping: None, drag: None, dialog: None, message: None, view: None, last_mapping: None };
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
                self.message = None;
                self.cropping = None;
                self.drag = None;
                self.view = None;
            }
            Err(err) => {
                tracing::warn!("editor: não abriu {}: {err}", path.display());
                self.message = Some(format!("Não foi possível abrir \"{}\": {err}", path.display()));
            }
        }
    }

    fn name(&self) -> String {
        self.path.as_ref().and_then(|p| p.file_name()).map_or_else(|| "Sem imagem".to_string(), |n| n.to_string_lossy().into_owned())
    }

    fn modified(&self) -> bool {
        self.doc.as_ref().is_some_and(Doc::modified)
    }

    fn ask_open(&mut self) {
        let dir = self.path.as_ref().and_then(|p| p.parent()).map(Path::to_path_buf).unwrap_or_else(default_dir);
        self.dialog = Some(Dialog::Open(FileChooser::open("Abrir imagem", Some(&dir)).extensions(&["png", "jpg", "jpeg"])));
    }

    fn ask_save_as(&mut self, then_close: bool) {
        let dir = self.path.as_ref().and_then(|p| p.parent()).map(Path::to_path_buf).unwrap_or_else(default_dir);
        let name = self.path.as_ref().and_then(|p| p.file_name()).map_or_else(|| "Captura.png".to_string(), |n| n.to_string_lossy().into_owned());
        self.dialog = Some(Dialog::SaveAs { chooser: FileChooser::save("Salvar como", Some(&dir), &name), then_close });
    }

    /// Abre o arquivo escolhido aqui (sem imagem aberta) ou numa janela nova.
    fn open_chosen(&mut self, path: PathBuf) {
        if self.doc.is_none() {
            self.load(path);
            return;
        }
        match std::process::Command::new(sibling_program("mide-screenshot-editor")).arg(&path).spawn() {
            Ok(_) => tracing::info!("editor: abrindo em outra janela: {}", path.display()),
            Err(err) => self.message = Some(format!("Não foi possível abrir: {err}")),
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
        let image = doc.result();
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
                self.message = Some(format!("Não foi possível salvar: {err}"));
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
        let image = doc.result();
        let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        let tmp = runtime.join(format!("mide-editor-copia-{}.png", std::process::id()));
        let ok = image.save_png(&tmp).is_ok() && ask_compositor(&format!("copy-file image/png {}", tmp.display())).is_some_and(|r| r.first().map(String::as_str) == Some("ok"));
        let _ = std::fs::remove_file(&tmp);
        if ok {
            tracing::info!("editor: imagem copiada ({}x{})", image.width(), image.height());
            self.message = Some("Imagem copiada. Cole com Ctrl+V.".into());
        } else {
            self.message = Some("A imagem não foi copiada: o mide não respondeu.".into());
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
                mide::sounds::play("trash-empty");
                ui.close();
            }
            Err(err) => self.message = Some(format!("Não foi possível mover para a Lixeira: {err}")),
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
        if let Some(doc) = self.doc.as_mut() {
            let done = if redo { doc.redo() } else { doc.undo() };
            if done {
                tracing::info!("editor: {}", if redo { "refeito" } else { "desfeito" });
            }
        }
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
            other => {
                if let Some(t) = Tool::ALL.iter().find(|t| t.2 == other) {
                    self.tool = t.0;
                    self.cropping = None;
                    tracing::info!("editor: ferramenta {}", t.1);
                }
            }
        }
    }

    /// Teclas do app (sem diálogo aberto).
    fn keys(&mut self, ui: &mut Ui) {
        let input = ui.input().clone();
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
        for (letter, action) in [("y", "refazer"), ("o", "abrir"), ("c", "copiar"), ("k", "recortar")] {
            if input.shortcut(letter) {
                actions.push(action);
            }
        }
        for (key, ctrl, _) in input.key_events() {
            match key {
                Key::Delete if ctrl => actions.push("lixeira"),
                Key::Enter if self.cropping.is_some() => actions.push("recortar"),
                Key::Escape if self.cropping.is_some() => {
                    self.cropping = None;
                    self.drag = None;
                    tracing::info!("editor: recorte cancelado");
                }
                _ => {}
            }
        }
        if !input.ctrl && !input.alt && !input.logo {
            for ch in input.text.chars() {
                if ch == 'r' {
                    actions.push("recortar");
                } else if let Some(t) = Tool::ALL.iter().find(|t| t.3 == ch) {
                    actions.push(t.2);
                }
            }
        }
        for action in actions {
            self.command(action, ui);
        }
    }

    fn toolbar(&mut self, ui: &mut Ui) -> Vec<RectF> {
        let mut clicked: Option<&str> = None;
        let mut swatches = Vec::new();
        let cropping = self.cropping.is_some();
        let has_doc = self.doc.is_some();
        let (can_undo, can_redo) = self.doc.as_ref().map_or((false, false), |d| (d.can_undo(), d.can_redo()));
        let tool = self.tool;
        ui.toolbar(|ui| {
            if cropping {
                ui.strong("Arraste as bolinhas para escolher a área");
                ui.push_right(2.0 * 110.0 + 10.0);
                ui.sized(110.0, |ui| {
                    if ui.button("Cancelar").clicked() {
                        clicked = Some("cancelar-recorte");
                    }
                });
                ui.sized(110.0, |ui| {
                    if ui.primary_button("Recortar").clicked() {
                        clicked = Some("recortar");
                    }
                });
                return;
            }
            ui.sized(86.0, |ui| {
                if ui.button("Recortar").clicked() && has_doc {
                    clicked = Some("recortar");
                }
            });
            for (t, label, action, _, w) in Tool::ALL {
                ui.sized(w, |ui| {
                    let r = if tool == t { ui.primary_button(label) } else { ui.button(label) };
                    if r.clicked() {
                        clicked = Some(action);
                    }
                });
            }
            // As cores: círculos de 22 pontos, 26 entre os centros.
            ui.sized(COLORS.len() as f32 * 26.0, |ui| {
                ui.custom(28.0, |_, rect| {
                    swatches = (0..COLORS.len()).map(|i| (rect.0 + i as f32 * 26.0 + 2.0, rect.1 + 3.0, 22.0, 22.0)).collect();
                });
            });
            ui.push_right(2.0 * 32.0 + 2.0 * 80.0 + 3.0 * 10.0);
            if ui.icon_button("edit-undo-symbolic", "↶", can_undo).clicked() {
                clicked = Some("desfazer");
            }
            if ui.icon_button("edit-redo-symbolic", "↷", can_redo).clicked() {
                clicked = Some("refazer");
            }
            ui.sized(80.0, |ui| {
                if ui.button("Copiar").clicked() {
                    clicked = Some("copiar");
                }
            });
            ui.sized(80.0, |ui| {
                if ui.primary_button("Salvar").clicked() {
                    clicked = Some("salvar");
                }
            });
        });
        match clicked {
            Some("cancelar-recorte") => {
                self.cropping = None;
                self.drag = None;
            }
            Some(action) => self.command(action, ui),
            None => {}
        }
        swatches
    }

    /// Cores: desenha os círculos e troca a cor no clique.
    fn swatches(&mut self, ui: &mut Ui, swatches: &[RectF]) {
        if swatches.is_empty() {
            return;
        }
        if ui.input().released
            && self.dialog.is_none()
            && let Some(i) = ui.input().pointer.and_then(|p| swatches.iter().position(|s| inside(*s, p)))
        {
            self.color = i;
            tracing::info!("editor: cor {}", COLORS[i].0);
        }
        let chosen = self.color;
        let accent = ui.theme().accent;
        let line = ui.theme().shade(70);
        let rects = swatches.to_vec();
        ui.paint(|canvas, _| {
            let tf = mide::ui::draw::tf();
            for (i, (x, y, w, _)) in rects.iter().enumerate() {
                let [r, g, b] = COLORS[i].1;
                let c = (x + w / 2.0, y + w / 2.0);
                let mut paint = Paint { anti_alias: true, ..Default::default() };
                if let Some(path) = PathBuilder::from_circle(c.0, c.1, w / 2.0 - 2.0) {
                    paint.set_color(Color::from_rgba8(r, g, b, 255));
                    canvas.fill_path(&path, &paint, FillRule::Winding, tf, None);
                    paint.set_color(Color::from_rgba8(line[0], line[1], line[2], line[3]));
                    canvas.stroke_path(&path, &paint, &Stroke { width: 1.0, ..Default::default() }, tf, None);
                }
                if i == chosen
                    && let Some(ring) = PathBuilder::from_circle(c.0, c.1, w / 2.0 + 1.5)
                {
                    paint.set_color(Color::from_rgba8(accent[0], accent[1], accent[2], 255));
                    canvas.stroke_path(&ring, &paint, &Stroke { width: 2.0, ..Default::default() }, tf, None);
                }
            }
        });
    }

    /// A imagem, o recorte e o desenho em andamento; o ponteiro na área da imagem.
    fn canvas(&mut self, ui: &mut Ui, area: RectF) {
        let Some(doc) = self.doc.as_mut() else {
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
        let map = Mapping::fit(area, source, 1.0 / mide::ui::draw::scale());
        if self.last_mapping != Some(map) {
            let d = map.display;
            tracing::info!("editor: imagem em ({}, {}) {}x{} na escala {:.3}", d.0, d.1, d.2.round(), d.3.round(), map.s);
            self.last_mapping = Some(map);
        }
        let blocked = self.dialog.is_some() || ui.menu_open().is_some();
        if !blocked {
            self.pointer(ui, map, full);
        }
        let Some(doc) = self.doc.as_mut() else {
            return;
        };
        // A imagem reduzida, refeita só quando algo muda.
        let k = mide::ui::draw::scale();
        let (pw, ph) = (((map.display.2 * k).round() as u32).max(1), ((map.display.3 * k).round() as u32).max(1));
        let content = (doc.version, [source.0 as i32, source.1 as i32, source.2 as i32, source.3 as i32]);
        let now = std::time::Instant::now();
        // Mudou só o tamanho: estica a anterior e refaz quando parar.
        let stretched = self.view.as_ref().filter(|v| v.content == content && (v.image.width(), v.image.height()) != (pw, ph)).map(|v| stretch(&v.image, pw, ph));
        if let Some(image) = stretched {
            self.view = Some(ViewCache { content, image, exact: false, at: now });
        }
        let rebuild = self.view.as_ref().is_none_or(|v| v.content != content || (!v.exact && now.duration_since(v.at) >= RESIZE_SETTLE));
        if rebuild {
            let composed = doc.composed();
            let full = tiny_skia::IntRect::from_xywh(0, 0, composed.width(), composed.height());
            let part = tiny_skia::IntRect::from_xywh(content.1[0], content.1[1], source.2 as u32, source.3 as u32);
            // A imagem inteira vai direto (sem copiar); um recorte, só a parte dele.
            let image = match part.filter(|p| Some(*p) != full).and_then(|r| composed.clone_rect(r)) {
                Some(part) => shrink(&part, pw, ph),
                None => shrink(composed, pw, ph),
            };
            self.view = Some(ViewCache { content, image, exact: true, at: now });
        }
        if let Some(v) = self.view.as_ref().filter(|v| !v.exact) {
            ui.wake_after(RESIZE_SETTLE.saturating_sub(now.duration_since(v.at)));
        }
        let view = &self.view.as_ref().expect("acabou de ser feita").image;
        let accent = ui.theme().accent;
        let crop = self.cropping.map(|c| map.rect_to_screen(c));
        let drawing = match &self.drag {
            Some(Drag::Draw(points)) => Some((self.tool.kind(), COLORS[self.color].1, points.clone(), doc.stroke_width())),
            _ => None,
        };
        ui.paint(|canvas, _| {
            let d = map.display;
            mide::ui::draw::soft_shadow(canvas, d.0, d.1, d.2, d.3, 2.0, 14.0, 4.0, 50);
            mide::ui::draw::blit_px(canvas, view, mide::ui::draw::px(d.0), mide::ui::draw::px(d.1), 1.0);
            let tf = mide::ui::draw::tf();
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
            if let Some((kind, color, points, width)) = drawing {
                draw_mark(canvas, kind, color, &points, width, map.transform(mide::ui::draw::scale()));
            }
        });
    }

    fn pointer(&mut self, ui: &mut Ui, map: Mapping, full: RectF) {
        let input = ui.input().clone();
        let Some(p) = input.pointer else {
            return;
        };
        let img = map.to_image(p);
        if input.pressed {
            if let Some(crop) = self.cropping {
                let screen = map.rect_to_screen(crop);
                self.drag = Some(match handle_at(screen, p) {
                    Some(handle) => Drag::CropResize { handle, orig: crop, start: img },
                    None if inside(screen, p) => Drag::CropMove { orig: crop, start: img },
                    None if inside(map.display, p) => Drag::CropNew { start: clamp_point(img, full) },
                    None => return,
                });
            } else if inside(map.display, p) {
                self.drag = Some(Drag::Draw(vec![clamp_point(img, map.source)]));
            }
        }
        let delta = |start: (f32, f32)| (img.0 - start.0, img.1 - start.1);
        match &mut self.drag {
            Some(Drag::Draw(points)) => {
                let q = clamp_point(img, map.source);
                if self.tool == Tool::Pen {
                    // Um ponto novo a cada ponto da tela andado.
                    if points.last().is_none_or(|l| (l.0 - q.0).hypot(l.1 - q.1) * map.s >= 1.0) {
                        points.push(q);
                    }
                } else {
                    points.truncate(1);
                    points.push(q);
                }
            }
            Some(Drag::CropNew { start }) => {
                let r = normalized(*start, clamp_point(img, full));
                if r.2 >= 2.0 && r.3 >= 2.0 {
                    self.cropping = Some(r);
                }
            }
            Some(Drag::CropResize { handle, orig, start }) => self.cropping = Some(resize_area(*orig, *handle, delta(*start), full)),
            Some(Drag::CropMove { orig, start }) => self.cropping = Some(moved(*orig, delta(*start), full)),
            None => {}
        }
        if self.drag.is_some() {
            ui.request_repaint();
        }
        if input.released {
            match self.drag.take() {
                Some(Drag::Draw(points)) => self.finish_mark(points, map),
                Some(_) => {
                    if let Some(c) = self.cropping {
                        tracing::info!("editor: área do recorte {}x{} em ({}, {})", c.2.round(), c.3.round(), c.0.round(), c.1.round());
                    }
                }
                None => {}
            }
        }
    }

    fn finish_mark(&mut self, points: Vec<(f32, f32)>, map: Mapping) {
        let Some(doc) = self.doc.as_mut() else {
            return;
        };
        let (Some(a), Some(b)) = (points.first(), points.last()) else {
            return;
        };
        // Seta, quadrado e círculo precisam de tamanho (um clique solto não desenha).
        if self.tool != Tool::Pen && (a.0 - b.0).hypot(a.1 - b.1) * map.s < 4.0 {
            return;
        }
        let mark = Mark { kind: self.tool.kind(), color: COLORS[self.color].1, points, width: doc.stroke_width() };
        tracing::info!("editor: {} desenhado ({} pontos)", self.tool.name().to_lowercase(), mark.points.len());
        doc.push(Op::Mark(mark));
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
                match ui.alert(&format!("\"{name}\" já existe. Substituir?"), "O arquivo que está lá será substituído.", &["Cancelar", "Substituir"]) {
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
                match ui.alert(&format!("Salvar as alterações em \"{name}\"?"), "Se não salvar, as alterações serão perdidas.", &["Cancelar", "Não salvar", "Salvar"]) {
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

impl App for Editor {
    fn view(&mut self, ui: &mut Ui) {
        ui.set_title(&format!("{}{} — Editor de Capturas", if self.modified() { "• " } else { "" }, self.name()));
        for action in ui.input().menu.clone() {
            self.command(&action, ui);
        }
        if self.dialog.is_none() && !ui.typing() {
            self.keys(ui);
        }
        let swatches = self.toolbar(ui);
        self.swatches(ui, &swatches);
        if let Some(message) = &self.message {
            ui.note(message);
        }
        let (w, h) = ui.window_size();
        let top = h - ui.remaining_height();
        let area = (20.0, top + 10.0, (w - 40.0).max(1.0), (h - top - 30.0).max(1.0));
        if self.doc.is_some() {
            self.canvas(ui, area);
        } else if self.dialog.is_none() {
            ui.label("Nenhuma imagem aberta.");
            ui.sized(140.0, |ui| {
                if ui.primary_button("Abrir imagem…").clicked() {
                    self.ask_open();
                }
            });
        }
        self.dialog(ui);
    }

    fn menu_bar(&mut self) -> Vec<MenuGroup> {
        let has = self.doc.is_some();
        let has_file = has && self.path.is_some();
        let (undo, redo) = self.doc.as_ref().map_or((false, false), |d| (d.can_undo(), d.can_redo()));
        let mut tools = vec![MenuItem::action("recortar", "Recortar").shortcut("Ctrl+K").enabled(has), MenuItem::Separator];
        for (t, label, action, _, _) in Tool::ALL {
            tools.push(MenuItem::action(action, label).checked(self.tool == t && self.cropping.is_none()));
        }
        vec![
            MenuGroup {
                title: "Arquivo".into(),
                items: vec![
                    MenuItem::action("abrir", "Abrir…").shortcut("Ctrl+O"),
                    MenuItem::Separator,
                    MenuItem::action("salvar", "Salvar").shortcut("Ctrl+S").enabled(has),
                    MenuItem::action("salvar-como", "Salvar como…").shortcut("Ctrl+Shift+S").enabled(has),
                    MenuItem::Separator,
                    MenuItem::action("mostrar", "Mostrar no Arquivos").enabled(has_file),
                    MenuItem::action("lixeira", "Mover para a Lixeira").shortcut("Ctrl+Delete").enabled(has_file),
                ],
            },
            MenuGroup {
                title: "Editar".into(),
                items: vec![
                    MenuItem::action("desfazer", "Desfazer").shortcut("Ctrl+Z").enabled(undo),
                    MenuItem::action("refazer", "Refazer").shortcut("Ctrl+Shift+Z").enabled(redo),
                    MenuItem::Separator,
                    MenuItem::action("copiar", "Copiar").shortcut("Ctrl+C").enabled(has),
                ],
            },
            MenuGroup { title: "Ferramentas".into(), items: tools },
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

/// Pasta das capturas (Imagens/Capturas de tela), ou Imagens.
fn default_dir() -> PathBuf {
    let pictures = mide::places::pictures_dir();
    let shots = pictures.join("Capturas de tela");
    if shots.is_dir() { shots } else { pictures }
}

fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "mide_screenshot_editor=info".into()))
        .init();
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
    let window = Window::new("Editor de Capturas").app_id("mide-screenshot-editor").size(940, 600).min_size(620, 420);
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
