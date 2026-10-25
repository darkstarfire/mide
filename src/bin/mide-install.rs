//! Instalador do mide (E9; o E6 vai usar a mesma janela para .deb e .rpm).
//!
//! - `mide-install <arquivo.AppImage>`: janela no estilo do Mac, com o ícone do app à
//!   esquerda, uma seta e a pasta Aplicativos à direita. Arrastar o ícone até a pasta
//!   instala (com o progresso na própria janela); o botão Instalar faz o mesmo (para
//!   quem não arrasta: teclado, acessibilidade). Se o app já está instalado, oferece
//!   substituir pela versão nova. No fim, Abrir roda o app instalado.
//! - `mide-install --remove <atalho.desktop>`: pergunta e move para a Lixeira um app
//!   instalado pelo mide (o "Mover para a Lixeira" do launcher).
//!
//! Um AppImage vindo da internet e ainda não confiado pede confirmação antes de tudo:
//! ler o nome e o ícone roda o próprio AppImage (`--appimage-extract`). A extração vai
//! para uma pasta nova e só do usuário (modo 0700, criada sem aceitar uma que já
//! exista) dentro do cache dele (`$XDG_CACHE_HOME/mide`), não num nome previsível no
//! `/tmp`, onde outro usuário podia criá-la antes e trocar o `.desktop` ou o ícone.

use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

use mide::{
    appimage::{self, Installed, Meta},
    files,
    kit::{run, App, Key, Ui, Window},
    programs::{self, Kind},
    ui::{
        draw::{blit, fill_rounded, rgba, scale, tf},
        text::FontWeight,
    },
};
use tiny_skia::{Paint, PathBuilder, Pixmap, Stroke};

/// Tamanho dos ícones do palco (pontos).
const ICON: f32 = 112.0;
/// Palco: começo e altura (pontos).
const STAGE_Y: f32 = 96.0;
const STAGE_H: f32 = 184.0;
const SIDE: f32 = 24.0;

/// Retângulo em pontos: x, y, largura, altura.
type Rect = (f32, f32, f32, f32);

/// O que a thread de trabalho devolve.
#[derive(Default)]
struct Shared {
    meta: Option<Meta>,
    progress: f32,
    result: Option<Result<Installed, String>>,
}

enum Stage {
    /// Veio da internet e ainda não foi confiado: a pergunta.
    Confirm(String),
    /// Lendo o nome e o ícone de dentro do AppImage.
    Reading,
    /// Pronto: arrastar ou Instalar.
    Ready,
    Copying,
    Done(Installed),
    Failed(String),
}

/// Pasta da extração, guardada também pelo `main` para apagar no fim.
type WorkDir = Arc<Mutex<Option<PathBuf>>>;

/// Cria a pasta temporária da extração: nova (falha se já existe), modo 0700, no cache
/// do usuário.
fn make_work_dir() -> std::io::Result<PathBuf> {
    use std::os::unix::fs::DirBuilderExt;
    let cache = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| mide::places::home_dir().join(".cache"));
    let base = cache.join("mide");
    std::fs::create_dir_all(&base)?;
    // Sobras de instaladores que já saíram sem limpar (encerrados à força).
    for entry in std::fs::read_dir(&base).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let pid = name.strip_prefix("instalar-").and_then(|r| r.split('-').next()).and_then(|p| p.parse::<u32>().ok());
        if let Some(pid) = pid.filter(|p| *p != std::process::id() && !Path::new(&format!("/proc/{p}")).exists()) {
            tracing::info!("instalador: apagando a pasta de um instalador que já saiu ({pid})");
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
    let mut last = None;
    for n in 0..100 {
        let dir = base.join(format!("instalar-{}-{n}", std::process::id()));
        match std::fs::DirBuilder::new().mode(0o700).create(&dir) {
            Ok(()) => return Ok(dir),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => last = Some(err),
            Err(err) => return Err(err),
        }
    }
    Err(last.unwrap_or_else(|| std::io::Error::other("sem pasta temporária")))
}

struct Install {
    path: PathBuf,
    work: WorkDir,
    stage: Stage,
    shared: Arc<Mutex<Shared>>,
    meta: Option<Meta>,
    /// O mesmo app já instalado (oferece substituir).
    previous: Option<Installed>,
    /// Nome da pasta de apps ("Aplicativos").
    folder: String,
    /// Ícones do palco (app e pasta), com a escala em que foram carregados.
    icons: Option<(f32, Option<Pixmap>, Option<Pixmap>)>,
    /// Arrastando o ícone: distância do ponteiro ao canto do ícone.
    drag: Option<(f32, f32)>,
}

impl Install {
    fn new(path: PathBuf) -> Self {
        let folder = appimage::apps_dir().file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Aplicativos".into());
        let mut me = Self { path, work: WorkDir::default(), stage: Stage::Reading, shared: Arc::default(), meta: None, previous: None, folder, icons: None, drag: None };
        if appimage::appimage_type(&me.path).is_none() {
            me.stage = Stage::Failed(format!("\"{}\" não é um AppImage.", me.file_name()));
            return me;
        }
        match programs::reason_to_ask(&programs::Dirs::current(), &me.path, &Kind::AppImage) {
            Some(reason) => {
                tracing::info!("instalador: perguntar antes de ler {}: {reason}", me.path.display());
                me.stage = Stage::Confirm(reason);
            }
            None => me.start_reading(),
        }
        me
    }

    fn file_name(&self) -> String {
        self.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    }

    fn name(&self) -> String {
        self.meta.as_ref().map(|m| m.name.clone()).unwrap_or_else(|| appimage::name_from_file_name(&self.file_name()).0)
    }

    fn start_reading(&mut self) {
        self.stage = Stage::Reading;
        let (path, work_slot, shared) = (self.path.clone(), self.work.clone(), self.shared.clone());
        std::thread::spawn(move || {
            let meta = match make_work_dir() {
                Ok(work) => {
                    if let Ok(mut slot) = work_slot.lock() {
                        *slot = Some(work.clone());
                    }
                    appimage::inspect(&path, &work)
                }
                Err(err) => {
                    tracing::warn!("instalador: sem pasta temporária ({err})");
                    let (name, version) = appimage::name_from_file_name(&path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
                    Meta { id: appimage::safe_id(&name.to_lowercase()), name, version, ..Default::default() }
                }
            };
            if let Ok(mut s) = shared.lock() {
                s.meta = Some(meta);
            }
        });
    }

    fn start_install(&mut self, how: &str) {
        let Some(meta) = self.meta.clone() else {
            return;
        };
        tracing::info!("instalador: instalar \"{}\" ({how})", meta.name);
        self.stage = Stage::Copying;
        let (src, shared) = (self.path.clone(), self.shared.clone());
        std::thread::spawn(move || {
            let progress_to = shared.clone();
            let result = appimage::install(&src, &meta, &appimage::Dirs::current(), &mut |p| {
                if let Ok(mut s) = progress_to.lock() {
                    s.progress = p;
                }
            });
            if let Ok(mut s) = shared.lock() {
                s.result = Some(result.map_err(|e| e.to_string()));
            }
        });
    }

    /// O que a thread já terminou.
    fn poll(&mut self, ui: &mut Ui) {
        match self.stage {
            Stage::Reading => {
                let meta = self.shared.lock().ok().and_then(|mut s| s.meta.take());
                match meta {
                    Some(meta) => {
                        self.previous = appimage::installed(&appimage::Dirs::current(), &meta.id);
                        tracing::info!(
                            "instalador: pronto para instalar \"{}\" (versão {}; já instalado: {})",
                            meta.name,
                            meta.version.as_deref().unwrap_or("?"),
                            self.previous.as_ref().map_or("não".into(), |p| format!("versão {}", p.version.as_deref().unwrap_or("?")))
                        );
                        ui.set_title(&format!("Instalar {}", meta.name));
                        self.meta = Some(meta);
                        self.icons = None;
                        self.stage = Stage::Ready;
                        ui.request_repaint();
                    }
                    None => ui.wake_after(Duration::from_millis(50)),
                }
            }
            Stage::Copying => {
                let result = self.shared.lock().ok().and_then(|mut s| s.result.take());
                match result {
                    Some(Ok(done)) => {
                        mide::sounds::play("complete");
                        self.stage = Stage::Done(done);
                    }
                    Some(Err(err)) => {
                        tracing::warn!("instalador: falhou: {err}");
                        self.stage = Stage::Failed(format!("Não foi possível instalar: {err}"));
                    }
                    None => ui.wake_after(Duration::from_millis(40)),
                }
                ui.request_repaint();
            }
            _ => {}
        }
    }

    /// Ícones do app e da pasta, no tamanho em pixels da tela atual.
    fn ensure_icons(&mut self) {
        let k = scale();
        if self.icons.as_ref().is_some_and(|(s, _, _)| *s == k) {
            return;
        }
        let px = (ICON * k).round() as u32;
        let app = self
            .meta
            .as_ref()
            .and_then(|m| m.icon.as_ref())
            .and_then(|p| mide::icons::load_icon_file(&p.to_string_lossy(), px))
            .or_else(|| mide::icons::load_themed_icon(&["application-x-executable", "application-x-executable-symbolic"], px));
        let folder = mide::icons::load_themed_icon(&["folder"], px);
        self.icons = Some((k, app, folder));
    }

    /// Retângulos (pontos) do ícone do app e da pasta no palco.
    fn spots(w: f32) -> (Rect, Rect) {
        let inner = w - 2.0 * SIDE;
        let y = STAGE_Y + 20.0;
        let app = (SIDE + inner * 0.25 - ICON / 2.0, y, ICON, ICON);
        let folder = (SIDE + inner * 0.75 - ICON / 2.0, y, ICON, ICON);
        (app, folder)
    }

    fn open_installed(&self, done: &Installed) {
        match programs::run(&done.desktop, &Kind::Desktop) {
            Ok(()) => tracing::info!("instalador: abrindo \"{}\"", done.name),
            Err(err) => tracing::warn!("instalador: {err}"),
        }
    }
}

fn inflate(r: Rect, by: f32) -> Rect {
    (r.0 - by, r.1 - by, r.2 + 2.0 * by, r.3 + 2.0 * by)
}

/// Seta do palco, do ícone do app até a pasta.
/// Seta grossa do instalador (como nas imagens de instalação do Mac): haste e ponta
/// preenchidas, na cor dada.
fn draw_arrow(pixmap: &mut Pixmap, x0: f32, x1: f32, y: f32, c: [u8; 4]) {
    let mut paint = Paint { anti_alias: true, ..Default::default() };
    paint.set_color(rgba(c[0], c[1], c[2], c[3]));
    let head = 30.0;
    let stroke = Stroke { width: 7.0, line_cap: tiny_skia::LineCap::Round, ..Default::default() };
    let mut pb = PathBuilder::new();
    pb.move_to(x0, y);
    pb.line_to(x1 - head + 4.0, y);
    if let Some(path) = pb.finish() {
        pixmap.stroke_path(&path, &paint, &stroke, tf(), None);
    }
    let mut pb = PathBuilder::new();
    pb.move_to(x1 - head, y - 16.0);
    pb.line_to(x1, y);
    pb.line_to(x1 - head, y + 16.0);
    pb.close();
    if let Some(path) = pb.finish() {
        pixmap.fill_path(&path, &paint, tiny_skia::FillRule::Winding, tf(), None);
    }
}

impl App for Install {
    fn view(&mut self, ui: &mut Ui) {
        self.poll(ui);
        let t = ui.theme().clone();
        let (_, _, w, h) = ui.region();
        let name = self.name();
        let input = ui.input().clone();

        // Pergunta de confiança antes de rodar o AppImage para ler o que tem dentro. O
        // Enter não confirma (só o clique em Continuar).
        if let Stage::Confirm(reason) = &self.stage {
            let reason = reason.clone();
            let enter = input.key(Key::Enter);
            match ui.alert(&format!("Instalar \"{}\"?", self.file_name()), &format!("{reason} Para instalar, o mide lê o nome e o ícone de dentro dele."), &["Cancelar", "Continuar"]) {
                Some(1) if enter => tracing::info!("instalador: Enter ignorado (escolha pelo botão)"),
                Some(1) => match programs::trust(&programs::Dirs::current(), &self.path) {
                    Ok(()) => self.start_reading(),
                    Err(err) => self.stage = Stage::Failed(format!("Não foi possível marcar \"{}\" como executável: {err}", self.file_name())),
                },
                Some(_) => {
                    tracing::info!("instalador: cancelado");
                    ui.close();
                }
                None => {}
            }
            return;
        }

        // Título e detalhes.
        let detail = {
            let mut parts = Vec::new();
            if let Some(v) = self.meta.as_ref().and_then(|m| m.version.clone()) {
                parts.push(format!("Versão {v}"));
            }
            parts.push("AppImage".to_string());
            if let Some(size) = self.meta.as_ref().map(|m| m.size).filter(|s| *s > 0) {
                parts.push(files::size_text(size));
            }
            parts.join(" · ")
        };
        // Como nas imagens de instalação do Mac: título grande no meio e, embaixo, o
        // que fazer (sem botões: arrastar é o jeito de instalar).
        let instruction = match &self.stage {
            Stage::Ready if self.previous.is_some() => format!("Arraste o app para a pasta {} para substituir a versão instalada", self.folder),
            Stage::Ready | Stage::Reading => format!("Arraste o app para a pasta {}", self.folder),
            _ => detail.clone(),
        };
        let title = format!("Instalar {name}");
        ui.paint_text(|pixmap, text, _| {
            let tw = text.measure(&title, 24.0, FontWeight::Bold);
            text.draw(pixmap, &title, (w - tw) / 2.0, 26.0, 24.0, FontWeight::Bold, t.text);
            let iw = text.measure(&instruction, 14.0, FontWeight::Regular);
            text.draw(pixmap, &instruction, (w - iw) / 2.0, 62.0, 14.0, FontWeight::Regular, t.text_secondary);
        });

        // Palco: o ícone, a seta e a pasta.
        self.ensure_icons();
        let (app_spot, folder_spot) = Self::spots(w);
        let ready = matches!(self.stage, Stage::Ready);
        if ready && input.pressed && input.over(inflate(app_spot, 6.0)) {
            let (px, py) = input.pointer.unwrap_or_default();
            self.drag = Some((px - app_spot.0, py - app_spot.1));
            tracing::info!("instalador: arrastando o ícone");
        }
        let over_folder = self.drag.is_some() && input.over(inflate(folder_spot, 24.0));
        let mut dropped = false;
        if self.drag.is_some() && (input.released || !input.down) {
            self.drag = None;
            if over_folder && ready {
                dropped = true;
            } else {
                tracing::info!("instalador: ícone solto fora da pasta");
            }
        }
        if self.drag.is_some() {
            ui.request_repaint();
        }
        let done = matches!(self.stage, Stage::Done(_));
        let progress = match self.stage {
            Stage::Copying => self.shared.lock().map(|s| s.progress).unwrap_or(0.0),
            Stage::Done(_) => 1.0,
            _ => 0.0,
        };
        let icons = self.icons.clone();
        let drag = self.drag.zip(input.pointer);
        let folder = self.folder.clone();
        let stage_name = name.clone();
        ui.paint_text(|pixmap, text, _| {
            let (sx, sw) = (SIDE, w - 2.0 * SIDE);
            let _ = (sx, sw);
            // A pasta acende com o ícone em cima dela (ou depois de instalar).
            if over_folder || done {
                let a = t.accent;
                fill_rounded(pixmap, folder_spot.0 - 12.0, folder_spot.1 - 10.0, ICON + 24.0, ICON + 20.0, 16.0, rgba(a[0], a[1], a[2], 48));
            }
            let arrow_y = app_spot.1 + ICON / 2.0;
            draw_arrow(pixmap, app_spot.0 + ICON + 26.0, folder_spot.0 - 26.0, arrow_y, t.accent);
            if let Some((_, app_icon, folder_icon)) = &icons {
                if let Some(f) = folder_icon {
                    blit(pixmap, f, folder_spot.0 as i32, folder_spot.1 as i32, 1.0);
                }
                if let Some(a) = app_icon {
                    // Enquanto arrasta, o lugar do ícone fica apagado e ele segue o ponteiro.
                    let (opacity, at) = match drag {
                        Some(_) => (0.3, (app_spot.0, app_spot.1)),
                        None if done => (0.3, (app_spot.0, app_spot.1)),
                        None => (1.0, (app_spot.0, app_spot.1)),
                    };
                    blit(pixmap, a, at.0 as i32, at.1 as i32, opacity);
                }
            }
            // Nomes embaixo dos ícones.
            let size = t.font;
            let mut label = |s: &str, center: f32, text: &mut mide::ui::text::Text| {
                let mut shown = s.to_string();
                while text.measure(&shown, size, FontWeight::Regular) > 170.0 && shown.chars().count() > 2 {
                    let mut chars: Vec<char> = shown.trim_end_matches('…').chars().collect();
                    chars.pop();
                    shown = format!("{}…", chars.into_iter().collect::<String>());
                }
                let lw = text.measure(&shown, size, FontWeight::Regular);
                text.draw(pixmap, &shown, center - lw / 2.0, app_spot.1 + ICON + 12.0, size, FontWeight::Regular, t.text);
            };
            label(&stage_name, app_spot.0 + ICON / 2.0, text);
            label(&folder, folder_spot.0 + ICON / 2.0, text);
            // Ícone sendo arrastado, por cima de tudo.
            if let (Some(((ox, oy), (px, py))), Some((_, Some(a), _))) = (drag, &icons) {
                blit(pixmap, a, (px - ox) as i32, (py - oy) as i32, 0.9);
            }
        });

        // Texto, progresso e botões embaixo do palco.
        let mut chosen = None;
        let previous = self.previous.clone();
        let new_version = self.meta.as_ref().and_then(|m| m.version.clone());
        let folder = self.folder.clone();
        ui.at(SIDE, STAGE_Y + STAGE_H + 14.0, w - 2.0 * SIDE, |ui| match &self.stage {
            Stage::Reading => {
                ui.centered_note("Lendo o AppImage…");
            }
            Stage::Ready => {
                // Sem botões (arrastar instala); só a versão já instalada, se houver.
                if let Some(old) = &previous {
                    let note = match (&old.version, &new_version) {
                        (Some(o), Some(n)) if o == n => format!("Esta versão já está em {folder}."),
                        (Some(o), Some(n)) => format!("Instalada: {o}. Esta: {n}."),
                        _ => format!("{name} já está em {folder}."),
                    };
                    ui.centered_note(&note);
                }
            }
            Stage::Copying => {
                let pct = (progress * 100.0).round() as u32;
                ui.label(&format!("Instalando… {pct}%"));
                let a = t.accent;
                let c = t.control;
                ui.custom(10.0, |pixmap, rect| {
                    fill_rounded(pixmap, rect.0, rect.1 + 2.0, rect.2, 6.0, 3.0, rgba(c[0], c[1], c[2], c[3]));
                    fill_rounded(pixmap, rect.0, rect.1 + 2.0, (rect.2 * progress).max(6.0), 6.0, 3.0, rgba(a[0], a[1], a[2], a[3]));
                });
            }
            // Sem botões (como as imagens de instalação do Mac): a janela fecha pela luz
            // vermelha ou Esc; Enter abre o app instalado.
            Stage::Done(_) => {
                ui.centered_note(&format!("{name} foi instalado em {folder} e já aparece no launcher e na busca."));
            }
            Stage::Failed(err) => {
                ui.error_note(err);
            }
            Stage::Confirm(_) => {}
        });
        let _ = h;
        // Teclado: Enter é o botão principal; Esc fecha (menos durante a cópia).
        if chosen.is_none() && input.key(Key::Enter) && !input.repeat && matches!(self.stage, Stage::Done(_)) {
            chosen = Some(1);
        }
        if chosen.is_none() && input.key(Key::Escape) && !matches!(self.stage, Stage::Copying) {
            chosen = Some(0);
        }
        if dropped {
            self.start_install("arrastado para a pasta");
            ui.request_repaint();
            return;
        }
        match (chosen, &self.stage) {
            (Some(1), Stage::Ready) => {
                self.start_install("botão");
                ui.request_repaint();
            }
            (Some(1), Stage::Done(done)) => {
                let done = done.clone();
                self.open_installed(&done);
                ui.close();
            }
            (Some(_), _) => {
                tracing::info!("instalador: fechado");
                ui.close();
            }
            (None, _) => {}
        }
    }

    fn close_requested(&mut self) -> bool {
        // Fechar no meio da cópia não deixa nada pela metade: a cópia termina na thread
        // (o arquivo só aparece inteiro); a janela espera.
        !matches!(self.stage, Stage::Copying)
    }
}

/// Pergunta antes de mover para a Lixeira um app instalado pelo mide.
struct Remove {
    desktop: PathBuf,
    info: Option<Installed>,
    icon: Option<(f32, Option<Pixmap>)>,
    error: Option<String>,
}

impl App for Remove {
    fn view(&mut self, ui: &mut Ui) {
        let t = ui.theme().clone();
        let (_, _, w, _) = ui.region();
        let input = ui.input().clone();
        let Some(info) = self.info.clone() else {
            ui.at(SIDE, 20.0, w - 2.0 * SIDE, |ui| {
                ui.title("Não é um app instalado pelo mide");
                ui.note(&self.desktop.display().to_string());
                ui.space(6.0);
                if ui.dialog_buttons(&["Fechar"]).is_some() || input.key(Key::Escape) || input.key(Key::Enter) {
                    ui.close();
                }
            });
            return;
        };
        let size = 56.0;
        let k = scale();
        if self.icon.as_ref().is_none_or(|(s, _)| *s != k) {
            let px = (size * k).round() as u32;
            let pix = info.icon.as_deref().and_then(|i| mide::icons::load_icon(i, px)).or_else(|| mide::icons::load_themed_icon(&["application-x-executable"], px));
            self.icon = Some((k, pix));
        }
        if let Some((_, Some(pix))) = &self.icon {
            let pix = pix.clone();
            ui.paint(|pixmap, _| blit(pixmap, &pix, SIDE as i32, 22, 1.0));
        }
        let x = SIDE + size + 16.0;
        let mut chosen = None;
        let folder = info.appimage.parent().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        ui.at(x, 16.0, (w - x - SIDE).max(160.0), |ui| {
            ui.title(&format!("Mover \"{}\" para a Lixeira?", info.name));
            ui.note(&format!("O AppImage sai de {folder} e vai para a Lixeira; o app sai do launcher e da busca."));
            if let Some(err) = &self.error {
                ui.error_note(err);
            }
            ui.space(6.0);
            chosen = if self.error.is_some() { ui.dialog_buttons(&["Fechar"]).map(|_| 0) } else { ui.dialog_buttons(&["Cancelar", "Mover para a Lixeira"]) };
        });
        let _ = t;
        if chosen.is_none() && input.key(Key::Escape) {
            chosen = Some(0);
        }
        if chosen.is_none() && input.key(Key::Enter) && self.error.is_none() {
            chosen = Some(1);
        }
        match chosen {
            Some(1) => match appimage::uninstall(&self.desktop, &appimage::Dirs::current()) {
                Ok(_) => {
                    ui.close();
                }
                Err(err) => {
                    tracing::warn!("remover {}: {err}", self.desktop.display());
                    self.error = Some(format!("Não foi possível remover: {err}"));
                    ui.request_repaint();
                }
            },
            Some(_) => {
                tracing::info!("remover {}: cancelado", self.desktop.display());
                ui.close();
            }
            None => {}
        }
    }
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "mide_install=info,mide=info".into()))
        .init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.as_slice() {
        [flag, desktop] if flag == "--remove" => {
            let desktop = PathBuf::from(desktop);
            let info = appimage::read_installed(&desktop).filter(|_| desktop.file_name().is_some_and(|n| n.to_string_lossy().starts_with(appimage::DESKTOP_PREFIX)));
            tracing::info!("remover {}: {}", desktop.display(), info.as_ref().map_or("não é do mide".to_string(), |i| format!("\"{}\" ({})", i.name, i.appimage.display())));
            let window = Window::new("Mover para a Lixeira").app_id("mide-install").size(500, 168).min_size(420, 150);
            run(window, Remove { desktop, info, icon: None, error: None })
        }
        [file] if !file.starts_with('-') => {
            let path = Path::new(file.strip_prefix("file://").unwrap_or(file));
            let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
            let install = Install::new(path);
            let work = install.work.clone();
            let window = Window::new(&format!("Instalar {}", install.name())).app_id("mide-install").size(620, 340).min_size(520, 330);
            let result = run(window, install);
            if let Some(dir) = work.lock().ok().and_then(|mut w| w.take()) {
                let _ = std::fs::remove_dir_all(&dir);
            }
            result
        }
        _ => {
            eprintln!("uso: mide-install <arquivo.AppImage> | mide-install --remove <atalho.desktop>");
            std::process::exit(2);
        }
    };
    if let Err(err) = result {
        eprintln!("mide-install: {err}");
        std::process::exit(1);
    }
}
