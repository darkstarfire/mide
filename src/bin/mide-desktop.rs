//! Área de trabalho do mide: os arquivos e pastas da pasta "Área de trabalho" como
//! ícones sobre o papel de parede, em colunas a partir do canto direito (como no
//! macOS). Feito com o mide-ui, numa camada de fundo (atrás das janelas).
//!
//! - **Discos** (pendrives, HDs externos, CDs e DVDs montados em `/media`, `/run/media`
//!   ou `/mnt`) aparecem primeiro, no canto de cima à direita, com "Ejetar".
//! - **Seleção** como no Finder: clique escolhe um, Ctrl+clique soma ou tira, Ctrl+A
//!   escolhe todos, e apertar no vazio e arrastar desenha o retângulo de seleção.
//! - **Arrastar** usa o arrasto do Wayland (o ícone vai por cima das janelas e o app
//!   sob o ponteiro recebe os arquivos). Soltar na própria área de trabalho muda o
//!   lugar (alinhado à grade; quem estava lá troca de lugar); soltar numa pasta move
//!   para dentro; segurar o arrasto em cima de uma pasta por um instante abre a pasta
//!   no Arquivos (pasta de mola, como no Finder), e o arrasto continua lá.
//! - **Onde aparece o que é criado**: pelo menu do botão direito (Nova pasta, Novo
//!   arquivo, Colar), no lugar do clique; pelo teclado ou por outro programa, no
//!   próximo lugar livre a partir do canto de cima à direita, de cima para baixo e da
//!   direita para a esquerda. Os lugares ficam em `~/.local/state/mide/desktop-positions`.
//! - **Novo arquivo** cria documentos válidos de cada tipo (`mide::templates`): texto,
//!   Markdown, Word, Writer, RTF, Excel, Calc, CSV, JSON e HTML.
//! - **Renomear** como no Finder: Enter (ou "Renomear", ou Ctrl+R) troca o nome embaixo
//!   do ícone por um campo com o nome sem a extensão escolhido. Abrir pelo teclado:
//!   Ctrl+O ou Ctrl+↓. Ctrl+C copia, Ctrl+V cola, Delete manda para a Lixeira.
//! - **Miniaturas** de fotos, PDFs e vídeos, com a borda branca.
//! - Programas (E8): atalhos `.desktop` com o nome e o ícone do app; AppImages,
//!   executáveis e scripts rodam com duplo clique (ou perguntam antes).
//!
//! `MIDE_DESKTOP_DIR` troca a pasta e `MIDE_DESKTOP_MOUNTINFO` a lista de montagens
//! (os testes usam arquivos temporários).

use std::{
    collections::{HashMap, HashSet},
    error::Error,
    path::{Path, PathBuf},
    sync::mpsc,
    time::{Duration, Instant, SystemTime},
};

use mide::{
    files::{self, Entry},
    kit::{run, App, Key, Theme, Tile, Ui, Window},
    places,
    programs::{self, Answer, Labels, Opened, Prompt},
};
use tiny_skia::Pixmap;

/// Ícones de 56 px; cada ladrilho ocupa 104 x 106.
const ICON: u32 = 56;
const CELL: (f32, f32) = (104.0, 106.0);
const MARGIN: f32 = 14.0;
/// Tempo segurando um arrasto em cima de uma pasta até ela abrir (pasta de mola).
const SPRING: Duration = Duration::from_millis(1100);

/// Disco montado (pendrive, HD externo, CD/DVD).
#[derive(Clone, Debug, PartialEq)]
struct Drive {
    device: String,
    mount: PathBuf,
    fstype: String,
}

/// Um ícone da área de trabalho: um arquivo ou pasta, ou um disco.
#[derive(Clone)]
struct Item {
    path: PathBuf,
    /// Chave do lugar guardado: o nome do arquivo, ou `disco:<ponto de montagem>`.
    key: String,
    name: String,
    icon: String,
    is_dir: bool,
    modified: SystemTime,
    drive: Option<Drive>,
    entry: Option<Entry>,
}

struct Desktop {
    dir: Option<PathBuf>,
    entries: Vec<Entry>,
    drives: Vec<Drive>,
    mtime: Option<SystemTime>,
    /// Itens escolhidos (caminhos).
    selected: Vec<PathBuf>,
    /// Renomear no lugar: o item, o nome sendo digitado e se o campo ainda precisa
    /// receber o foco.
    inline: Option<(PathBuf, String, bool)>,
    /// Área útil da tela (sem a barra e a dock), perguntada ao compositor.
    workarea: Option<(f32, f32, f32, f32)>,
    asked_at: Option<Instant>,
    /// Confirmação antes de rodar um programa (E8).
    prompt: Option<Prompt>,
    /// Nome e ícone dos atalhos `.desktop`, no lugar do nome do arquivo.
    labels: Labels,
    /// Lugar escolhido de cada ícone (chave → coluna a partir da direita, linha).
    cells: HashMap<String, (usize, usize)>,
    /// Ícone apertado: caminho, ponto onde o botão desceu e distância do ponteiro ao
    /// canto do ícone. Vira arrasto quando o ponteiro anda mais que alguns pontos.
    press: Option<Press>,
    /// Imagem do arrasto do ícone apertado, montada no clique: quando o ponteiro anda,
    /// o arrasto começa sem esperar o desenho (antes, uns 100 ms de atraso).
    press_icon: Option<Pixmap>,
    /// Arrasto do Wayland começado aqui: os caminhos, o ícone apertado e a distância
    /// do ponteiro ao canto dele.
    drag_out: Option<(Vec<PathBuf>, PathBuf, (f32, f32))>,
    /// O arrasto terminou (solto em outro app ou cancelado): se nenhum "soltou aqui"
    /// chegar logo depois, os ícones escondidos voltam ao lugar.
    drag_ended_at: Option<std::time::Instant>,
    /// Retângulo de seleção: onde começou, onde está, e a seleção de antes (com Ctrl,
    /// o retângulo soma a ela).
    band: mide::kit::SelectionBand,
    band_base: Option<Vec<PathBuf>>,
    /// Ponto do clique direito no vazio: o que o menu criar aparece ali.
    menu_point: Option<(f32, f32)>,
    /// Colar pedido (o texto da área de transferência chega num dos próximos quadros),
    /// com o lugar (ou `None`: o próximo livre).
    pending_paste: Option<Option<(f32, f32)>>,
    /// Pasta sob um arrasto, desde quando, e se já abriu (pasta de mola).
    spring: Option<(PathBuf, Instant, bool)>,
    /// Miniaturas (com a data do arquivo de quando foram feitas) e o carregador.
    thumbs: HashMap<PathBuf, (SystemTime, Option<Pixmap>)>,
    waiting: HashSet<PathBuf>,
    to_loader: mpsc::Sender<(PathBuf, SystemTime)>,
    from_loader: mpsc::Receiver<(PathBuf, SystemTime, Option<Pixmap>)>,
    /// Apps que abrem arquivos (para o "Abrir com"), lidos na primeira vez.
    apps: Option<Vec<mide::apps::App>>,
}

thread_local! {
    /// Medidor de texto da imagem do arrasto, criado uma vez só (criar e medir a primeira
    /// vez leva ~100 ms; `warm_drag_text` faz isso quando a mesa abre).
    static DRAG_TEXT: std::cell::RefCell<mide::ui::text::Text> = std::cell::RefCell::new(mide::ui::text::Text::new());
}

/// Prepara o medidor de texto do arrasto (e as fontes) antes do primeiro clique.
fn warm_drag_text() {
    DRAG_TEXT.with(|t| {
        t.borrow_mut().measure("Ag", 12.0, mide::ui::text::FontWeight::Regular);
    });
}

/// Retângulo de seleção: começo, ponto atual e a seleção de antes.

/// Ícone apertado: caminho, onde o botão desceu e distância do ponteiro ao canto.
type Press = (PathBuf, (f32, f32), (f32, f32));

/// Posições guardadas: uma linha por ícone, `coluna<TAB>linha<TAB>nome`.
fn cells_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("state")))?;
    Some(base.join("mide").join("desktop-positions"))
}

fn parse_cells(text: &str) -> HashMap<String, (usize, usize)> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, '\t');
            let col = parts.next()?.parse().ok()?;
            let row = parts.next()?.parse().ok()?;
            let name = parts.next().filter(|n| !n.is_empty())?;
            Some((name.to_string(), (col, row)))
        })
        .collect()
}

fn format_cells(cells: &HashMap<String, (usize, usize)>) -> String {
    let mut lines: Vec<String> = cells.iter().map(|(name, (c, r))| format!("{c}\t{r}\t{name}")).collect();
    lines.sort();
    lines.iter().map(|l| l.clone() + "\n").collect()
}

/// Lugar de cada item na grade de `cols` x `rows`: os que têm lugar guardado (dentro
/// da grade e ainda livre) ficam nele; os outros ocupam os primeiros livres, coluna
/// por coluna a partir da direita, de cima para baixo.
fn layout(names: &[String], saved: &HashMap<String, (usize, usize)>, cols: usize, rows: usize) -> Vec<(usize, usize)> {
    let mut taken = HashSet::new();
    let mut out: Vec<Option<(usize, usize)>> = names
        .iter()
        .map(|n| {
            let cell = saved.get(n).copied().filter(|&(c, r)| c < cols && r < rows && !taken.contains(&(c, r)))?;
            taken.insert(cell);
            Some(cell)
        })
        .collect();
    let mut next = 0usize;
    for slot in out.iter_mut().filter(|s| s.is_none()) {
        // Sem lugar livre na grade (muitos ícones): continua para a esquerda.
        loop {
            let cell = (next / rows, next % rows);
            next += 1;
            if !taken.contains(&cell) {
                taken.insert(cell);
                *slot = Some(cell);
                break;
            }
        }
    }
    out.into_iter().map(|c| c.unwrap_or((0, 0))).collect()
}

/// Célula livre mais perto de `want` (por distância na grade), dentro da grade.
fn nearest_free(want: (usize, usize), taken: &HashSet<(usize, usize)>, cols: usize, rows: usize) -> (usize, usize) {
    let mut best = None;
    for c in 0..cols.max(1) {
        for r in 0..rows.max(1) {
            if taken.contains(&(c, r)) {
                continue;
            }
            let d = (c as i64 - want.0 as i64).pow(2) + (r as i64 - want.1 as i64).pow(2);
            if best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, (c, r)));
            }
        }
    }
    best.map(|(_, cell)| cell).unwrap_or(want)
}

/// Discos montados para o usuário, lidos do `mountinfo`: tudo em `/media`, `/run/media`
/// e `/mnt` (pendrives, HDs externos, CDs/DVDs, partições montadas à mão).
fn read_drives() -> Vec<Drive> {
    let path = std::env::var_os("MIDE_DESKTOP_MOUNTINFO").map(PathBuf::from).unwrap_or_else(|| "/proc/self/mountinfo".into());
    std::fs::read_to_string(path).map(|t| parse_mountinfo(&t)).unwrap_or_default()
}

fn parse_mountinfo(text: &str) -> Vec<Drive> {
    // `\040` no ponto de montagem é um espaço (e `\011` tab, `\134` barra invertida).
    let unescape = |s: &str| s.replace("\\040", " ").replace("\\011", "\t").replace("\\134", "\\");
    let mut out: Vec<Drive> = Vec::new();
    for line in text.lines() {
        let Some((left, right)) = line.split_once(" - ") else {
            continue;
        };
        let fields: Vec<&str> = left.split(' ').collect();
        let rest: Vec<&str> = right.split(' ').collect();
        let (Some(mount), Some(fstype), Some(device)) = (fields.get(4), rest.first(), rest.get(1)) else {
            continue;
        };
        let mount = unescape(mount);
        let user_mount = ["/media/", "/run/media/", "/mnt/"].iter().any(|p| mount.starts_with(p));
        // Pastas de passagem (o tmpfs de /media/fabio, que só segura os discos) não
        // contam.
        if !user_mount || mount.trim_end_matches('/').matches('/').count() < 2 || matches!(*fstype, "tmpfs" | "autofs") {
            continue;
        }
        if out.iter().any(|d| d.mount == Path::new(&mount)) {
            continue;
        }
        out.push(Drive { device: device.to_string(), mount: PathBuf::from(mount), fstype: fstype.to_string() });
    }
    out
}

/// Ícone do tema para um disco: CD/DVD ou removível.
fn drive_icon(d: &Drive) -> &'static str {
    match d.fstype.as_str() {
        "iso9660" | "udf" => "media-optical",
        _ => "drive-removable-media",
    }
}

/// Miniatura para a área de trabalho: fotos, PDFs e vídeos com a borda branca (como
/// no Mac); a folha dos arquivos de texto já vem desenhada.
fn desktop_thumb(path: &Path) -> Option<Pixmap> {
    let base = mide::thumbnails::make(path, ICON)?;
    if files::is_text(path) {
        return Some(base);
    }
    let k = mide::ui::draw::scale().max(1.0);
    let side = base.width().max(base.height());
    let mut canvas = Pixmap::new(side, side)?;
    let border = (2.0 * k).round();
    let room = side as f32 - 2.0 * border - 2.0 * k;
    let s = (room / base.width().max(base.height()) as f32).min(1.0);
    let (w, h) = (base.width() as f32 * s, base.height() as f32 * s);
    let (x, y) = ((side as f32 - w) / 2.0, (side as f32 - h) / 2.0);
    // Sombra curta, borda branca e a foto.
    let shade = tiny_skia::Color::from_rgba8(0, 0, 0, 60);
    mide::ui::draw::fill_rounded(&mut canvas, (x - border) / k, (y - border + k) / k, (w + 2.0 * border) / k, (h + 2.0 * border) / k, 1.5, shade);
    mide::ui::draw::fill_rounded(&mut canvas, (x - border) / k, (y - border) / k, (w + 2.0 * border) / k, (h + 2.0 * border) / k, 1.5, tiny_skia::Color::WHITE);
    let paint = tiny_skia::PixmapPaint { quality: tiny_skia::FilterQuality::Bilinear, ..Default::default() };
    canvas.draw_pixmap(0, 0, base.as_ref(), &paint, tiny_skia::Transform::from_scale(s, s).post_translate(x, y), None);
    Some(canvas)
}

#[derive(Clone)]
enum Action {
    Open,
    OpenWith(String),
    Rename,
    Duplicate,
    Compress,
    Alias,
    Copy,
    CopyPath,
    Info,
    Tag(usize),
    Trash,
    Eject,
    NewFolderWithSelection,
    NewFolder,
    NewFile(&'static str),
    Paste,
    Terminal,
    ShowFolder,
    CleanUp,
    Wallpaper,
}

impl Desktop {
    fn new() -> Self {
        let (to_loader, jobs) = mpsc::channel::<(PathBuf, SystemTime)>();
        let (done, from_loader) = mpsc::channel();
        std::thread::spawn(move || {
            for (path, modified) in jobs {
                let pix = desktop_thumb(&path);
                if done.send((path, modified, pix)).is_err() {
                    break;
                }
            }
        });
        let mut app = Self {
            dir: places::desktop_dir(),
            entries: Vec::new(),
            drives: Vec::new(),
            mtime: None,
            selected: Vec::new(),
            inline: None,
            workarea: None,
            asked_at: None,
            prompt: None,
            labels: Labels::default(),
            cells: HashMap::new(),
            press: None,
            press_icon: None,
            drag_out: None,
            drag_ended_at: None,
            band: Default::default(),
            band_base: None,
            menu_point: None,
            pending_paste: None,
            spring: None,
            thumbs: HashMap::new(),
            waiting: HashSet::new(),
            to_loader,
            from_loader,
            apps: None,
        };
        app.cells = cells_path().and_then(|p| std::fs::read_to_string(p).ok()).map(|t| parse_cells(&t)).unwrap_or_default();
        app.reload();
        app
    }

    /// Relê a pasta e os discos (quando a pasta muda, ou depois de criar, renomear ou
    /// apagar).
    fn reload(&mut self) {
        self.drives = read_drives();
        let Some(dir) = &self.dir else {
            return;
        };
        self.mtime = std::fs::metadata(dir).and_then(|m| m.modified()).ok();
        self.entries = files::list_dir(dir, false).unwrap_or_default();
        self.selected.retain(|s| s.exists());
    }

    /// Discos e arquivos, na ordem da grade (discos primeiro: canto de cima à direita).
    fn items(&mut self) -> Vec<Item> {
        let mut out: Vec<Item> = self
            .drives
            .iter()
            .map(|d| Item {
                path: d.mount.clone(),
                key: format!("disco:{}", d.mount.display()),
                name: d.mount.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| d.device.clone()),
                icon: drive_icon(d).into(),
                is_dir: true,
                modified: SystemTime::UNIX_EPOCH,
                drive: Some(d.clone()),
                entry: None,
            })
            .collect();
        for e in &self.entries {
            let (name, icon) = self.labels.get(&e.path, e.modified).unwrap_or_else(|| (e.name.clone(), e.icon_name().to_string()));
            out.push(Item { path: e.path.clone(), key: e.name.clone(), name, icon, is_dir: e.is_dir, modified: e.modified, drive: None, entry: Some(e.clone()) });
        }
        out
    }

    /// Área útil (sem barra e dock), para os ícones não ficarem embaixo delas.
    fn ask_workarea(&mut self) {
        let parsed = mide::control_client::ask_compositor("workarea").and_then(|lines| {
            let n: Vec<f32> = lines.first()?.split_whitespace().filter_map(|v| v.parse().ok()).collect();
            (n.len() == 4).then(|| (n[0], n[1], n[2], n[3]))
        });
        self.workarea = parsed;
    }

    fn save_cells(&self) {
        if let Some(path) = cells_path() {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(path, format_cells(&self.cells));
        }
    }

    fn changed(&self) -> bool {
        self.dir.as_ref().is_some_and(|d| std::fs::metadata(d).and_then(|m| m.modified()).ok() != self.mtime)
    }

    /// Miniatura pronta de um arquivo (pedida ao carregador na primeira vez).
    fn thumb(&mut self, item: &Item) -> Option<&Pixmap> {
        let entry = item.entry.as_ref()?;
        if entry.is_dir || entry.path.extension().is_some_and(|e| e.eq_ignore_ascii_case("desktop") || e.eq_ignore_ascii_case("appimage")) {
            return None;
        }
        let fresh = self.thumbs.get(&item.path).is_some_and(|(m, _)| *m == item.modified);
        if !fresh && self.waiting.insert(item.path.clone()) {
            let _ = self.to_loader.send((item.path.clone(), item.modified));
        }
        self.thumbs.get(&item.path).and_then(|(_, p)| p.as_ref())
    }

    /// Começa a renomear no lugar: o nome embaixo do ícone vira um campo de texto.
    fn start_rename(&mut self, path: PathBuf) {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.selected = vec![path.clone()];
        self.inline = Some((path, name, true));
    }

    /// Termina de renomear no lugar (o ícone continua no mesmo lugar da grade).
    fn finish_rename(&mut self, path: PathBuf, name: String) {
        let current = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let name = name.trim();
        if name.is_empty() || name == current {
            return;
        }
        match files::rename(&path, name) {
            Ok(new_path) => {
                tracing::info!("área de trabalho: {} → {}", path.display(), new_path.display());
                if let Some(cell) = self.cells.remove(&current) {
                    self.cells.insert(name.to_string(), cell);
                    self.save_cells();
                }
                self.reload();
                self.selected = vec![new_path];
            }
            Err(err) => tracing::warn!("não foi renomeado: {err}"),
        }
    }

    /// Põe os itens novos (`paths`) a partir do ponto `point` (canto do ícone perto do
    /// clique), nos lugares livres mais perto; sem ponto, eles ficam com o próximo
    /// livre a partir do canto de cima à direita (sem lugar guardado).
    fn place_new(&mut self, paths: &[PathBuf], point: Option<(f32, f32)>, area: (f32, f32, f32, f32)) {
        let Some(point) = point else {
            return;
        };
        let (cols, rows) = Self::grid(area);
        let items = self.items();
        let new_keys: HashSet<String> = paths.iter().filter_map(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).collect();
        let others: Vec<&Item> = items.iter().filter(|i| !new_keys.contains(&i.key)).collect();
        let keys: Vec<String> = others.iter().map(|i| i.key.clone()).collect();
        let mut taken: HashSet<(usize, usize)> = layout(&keys, &self.cells, cols, rows).into_iter().collect();
        // Todos os outros ficam onde estão: guardam o lugar de agora (senão o novo, ao
        // entrar na ordem, empurraria os que não têm lugar guardado).
        for (item, cell) in others.iter().zip(layout(&keys, &self.cells, cols, rows)) {
            self.cells.entry(item.key.clone()).or_insert(cell);
        }
        let want = Self::nearest_cell((point.0 - CELL.0 / 2.0, point.1 - CELL.1 / 3.0), area);
        for key in paths.iter().filter_map(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()) {
            let cell = nearest_free(want, &taken, cols, rows);
            taken.insert(cell);
            self.cells.insert(key, cell);
        }
        self.save_cells();
    }

    /// Caminhos da ação: os escolhidos, se o item clicado está entre eles; senão, só ele.
    fn targets(&self, item: &Item) -> Vec<PathBuf> {
        if self.selected.contains(&item.path) && self.selected.len() > 1 {
            self.selected.iter().filter(|p| !self.drives.iter().any(|d| &d.mount == *p)).cloned().collect()
        } else {
            vec![item.path.clone()]
        }
    }

    fn act(&mut self, action: Action, item: Option<Item>, area: (f32, f32, f32, f32), ui: &mut Ui) {
        let dir = self.dir.clone();
        let point = self.menu_point.take();
        match (action, item) {
            (Action::Open, Some(item)) => {
                for path in self.targets(&item) {
                    if path.is_dir() {
                        places::open(&path.to_string_lossy());
                        continue;
                    }
                    // Programas (.desktop, AppImage, executáveis, scripts) rodam ou
                    // perguntam antes.
                    match programs::open(&path) {
                        Opened::Done => {}
                        Opened::Ask(prompt) => self.prompt = Some(prompt),
                        Opened::Failed(err) => tracing::warn!("{err}"),
                    }
                }
            }
            (Action::OpenWith(id), Some(item)) => {
                if let Some(app) = self.apps.as_ref().and_then(|apps| apps.iter().find(|a| a.id == id)).cloned() {
                    for path in self.targets(&item) {
                        if let Err(err) = mide::apps::open_with(&app, &path) {
                            tracing::warn!("não abriu com {}: {err}", app.name);
                        }
                    }
                }
            }
            (Action::Rename, Some(item)) => self.start_rename(item.path.clone()),
            (Action::Duplicate, Some(item)) => {
                let mut made = Vec::new();
                for path in self.targets(&item) {
                    if let Ok(copy) = files::duplicate(&path) {
                        made.push(copy);
                    }
                }
                self.reload();
                self.selected = made;
            }
            (Action::Compress, Some(item)) => {
                let targets = self.targets(&item);
                let result = if targets.len() > 1 { files::compress_many(&targets) } else { files::compress(&targets[0]) };
                match result {
                    Ok(zip) => {
                        self.reload();
                        self.selected = vec![zip];
                    }
                    Err(err) => tracing::warn!("não foi comprimido: {err}"),
                }
            }
            (Action::Alias, Some(item)) => {
                // Atalho (link simbólico) ao lado, como o "Criar atalho" do Finder.
                if let Some(dir) = dir {
                    for path in self.targets(&item) {
                        let stem = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        let name = files::numbered_name(&dir, &format!("{stem} atalho"), "");
                        if let Err(err) = std::os::unix::fs::symlink(&path, dir.join(&name)) {
                            tracing::warn!("atalho não criado: {err}");
                        }
                    }
                    self.reload();
                }
            }
            (Action::Copy, Some(item)) => copy_files(&self.targets(&item)),
            (Action::CopyPath, Some(item)) => places::copy_text(&self.targets(&item).iter().map(|p| p.to_string_lossy().into_owned()).collect::<Vec<_>>().join("\n")),
            (Action::Info, Some(item)) => {
                let exe = sibling("mide-files");
                for path in self.targets(&item) {
                    let _ = std::process::Command::new(&exe).arg("--info").arg(&path).spawn();
                }
            }
            (Action::Tag(i), Some(item)) => {
                for path in self.targets(&item) {
                    if let Err(err) = files::toggle_tag(&path, i) {
                        tracing::warn!("etiqueta: {err}");
                    }
                }
            }
            (Action::Trash, Some(item)) => {
                for path in self.targets(&item) {
                    match places::move_to_trash(&path, &places::trash_dir()) {
                        Ok(_) => tracing::info!("para a Lixeira: {}", path.display()),
                        Err(err) => tracing::warn!("não foi para a Lixeira: {err}"),
                    }
                }
                self.reload();
            }
            (Action::Eject, Some(item)) => {
                if let Some(drive) = item.drive.clone() {
                    eject(drive);
                }
            }
            (Action::NewFolderWithSelection, Some(item)) => {
                // Como no Finder: os escolhidos vão para uma pasta nova, no lugar deles.
                if let Some(dir) = dir {
                    let targets = self.targets(&item);
                    let name = files::numbered_name(&dir, "Nova pasta com itens", "");
                    if let Ok(folder) = files::create_named(&dir, &name, true) {
                        let at = self.cells.get(&item.key).copied();
                        for path in &targets {
                            if let Err(err) = files::move_into(path, &folder) {
                                tracing::warn!("não foi movido: {err}");
                            }
                        }
                        if let Some(cell) = at {
                            self.cells.insert(name, cell);
                            self.save_cells();
                        }
                        self.reload();
                        self.start_rename(folder);
                    }
                }
            }
            // Como no Finder: cria com um nome livre (no lugar do clique) e já deixa o
            // nome para editar.
            (Action::NewFolder, _) => {
                if let Some(dir) = dir {
                    let name = files::numbered_name(&dir, "Nova pasta", "");
                    match files::create_named(&dir, &name, true) {
                        Ok(path) => {
                            tracing::info!("área de trabalho: {}", path.display());
                            self.place_new(std::slice::from_ref(&path), point, area);
                            self.reload();
                            self.start_rename(path);
                        }
                        Err(err) => tracing::warn!("não foi criado: {err}"),
                    }
                }
            }
            (Action::NewFile(ext), _) => {
                if let Some(dir) = dir {
                    match mide::templates::create(&dir, ext) {
                        Ok(path) => {
                            tracing::info!("área de trabalho: {}", path.display());
                            self.place_new(std::slice::from_ref(&path), point, area);
                            self.reload();
                            self.start_rename(path);
                        }
                        Err(err) => tracing::warn!("não foi criado: {err}"),
                    }
                }
            }
            (Action::Paste, _) => {
                self.pending_paste = Some(point);
                ui.request_paste();
            }
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
            // "Organizar": todos voltam à ordem a partir do canto de cima à direita.
            (Action::CleanUp, _) => {
                self.cells.clear();
                self.save_cells();
            }
            (Action::Wallpaper, _) => mide::system::open_settings("wallpaper"),
            _ => {}
        }
    }

    /// Cola os arquivos da área de transferência (lista de endereços `file://` ou
    /// caminhos, um por linha) na área de trabalho, no lugar pedido.
    fn paste_text(&mut self, text: &str, point: Option<(f32, f32)>, area: (f32, f32, f32, f32)) {
        let Some(dir) = self.dir.clone() else {
            return;
        };
        let mut paths = mide::kit::parse_uri_list(text);
        if paths.is_empty() {
            paths = text.lines().map(str::trim).filter(|l| l.starts_with('/')).map(PathBuf::from).collect();
        }
        let mut made = Vec::new();
        for path in paths.iter().filter(|p| p.exists()) {
            match files::copy_into(path, &dir) {
                Ok(copy) => made.push(copy),
                Err(err) => tracing::warn!("não foi colado: {err}"),
            }
        }
        if made.is_empty() {
            return;
        }
        tracing::info!("área de trabalho: colado(s) {} item(ns)", made.len());
        self.place_new(&made, point, area);
        self.reload();
        self.selected = made;
    }

    /// Tamanho da grade (colunas, linhas) que cabe na área.
    fn grid(area: (f32, f32, f32, f32)) -> (usize, usize) {
        let (_, _, width, height) = area;
        let cols = (((width - 2.0 * MARGIN) / CELL.0).floor() as usize).max(1);
        let rows = (((height - 2.0 * MARGIN) / CELL.1).floor() as usize).max(1);
        (cols, rows)
    }

    /// Canto da célula (coluna contada a partir da direita, linha a partir de cima).
    fn cell_pos(cell: (usize, usize), area: (f32, f32, f32, f32)) -> (f32, f32) {
        let (x, y, width, _) = area;
        (x + width - MARGIN - (cell.0 as f32 + 1.0) * CELL.0, y + MARGIN + cell.1 as f32 * CELL.1)
    }

    /// Célula mais perto de um ícone com o canto em `corner`.
    fn nearest_cell(corner: (f32, f32), area: (f32, f32, f32, f32)) -> (usize, usize) {
        let (x, y, width, _) = area;
        let (cols, rows) = Self::grid(area);
        let col = ((x + width - MARGIN - corner.0) / CELL.0 - 1.0).round().clamp(0.0, (cols - 1) as f32) as usize;
        let row = ((corner.1 - y - MARGIN) / CELL.1).round().clamp(0.0, (rows - 1) as f32) as usize;
        (col, row)
    }

    /// Ícones soltos na área de trabalho: dentro de uma pasta (se caíram em cima de
    /// uma) ou em lugares novos, mantendo a distância entre eles; quem estava no lugar
    /// do ícone apertado vai para o lugar antigo dele.
    fn drop_icons(&mut self, paths: &[PathBuf], pressed: &Path, corner: (f32, f32), pointer: (f32, f32), placed: &[(Item, (usize, usize))], area: (f32, f32, f32, f32)) {
        let target = placed.iter().find(|(i, cell)| {
            let (x, y) = Self::cell_pos(*cell, area);
            !paths.contains(&i.path) && i.is_dir && pointer.0 >= x && pointer.0 < x + CELL.0 && pointer.1 >= y && pointer.1 < y + CELL.1
        });
        if let Some((folder, _)) = target {
            for path in paths {
                match files::move_into(path, &folder.path) {
                    Ok(moved) => tracing::info!("área de trabalho: movido para {}", moved.display()),
                    Err(err) => tracing::warn!("não foi movido: {err}"),
                }
            }
            self.reload();
            return;
        }
        let (cols, rows) = Self::grid(area);
        let Some(from) = placed.iter().find(|(i, _)| i.path == pressed).map(|(_, c)| *c) else {
            return;
        };
        let to = Self::nearest_cell(corner, area);
        if from == to {
            return;
        }
        let moving: Vec<&(Item, (usize, usize))> = placed.iter().filter(|(i, _)| paths.contains(&i.path)).collect();
        // Os outros guardam o lugar de agora; quem está no destino do apertado (com um
        // só) troca com ele.
        let mut taken = HashSet::new();
        for (item, cell) in placed.iter().filter(|(i, _)| !paths.contains(&i.path)) {
            let cell = if moving.len() == 1 && *cell == to { from } else { *cell };
            self.cells.insert(item.key.clone(), cell);
            taken.insert(cell);
        }
        let (dc, dr) = (to.0 as i64 - from.0 as i64, to.1 as i64 - from.1 as i64);
        for (item, cell) in moving {
            let want = ((cell.0 as i64 + dc).clamp(0, cols as i64 - 1) as usize, (cell.1 as i64 + dr).clamp(0, rows as i64 - 1) as usize);
            let cell = nearest_free(want, &taken, cols, rows);
            taken.insert(cell);
            self.cells.insert(item.key.clone(), cell);
        }
        self.save_cells();
    }

    /// Imagem que segue o ponteiro no arrasto: o próprio ícone (opaco, como na mesa),
    /// com o nome embaixo na cápsula de escolhido e, com vários itens, a contagem num
    /// selo vermelho. O kit centra a imagem no ponteiro: o ícone é posto de modo que o
    /// ponto apertado (`grab`, relativo à célula) caia no centro, sem pulo ao começar.
    fn drag_icon(icon: &str, name: &str, count: usize, grab: (f32, f32)) -> Option<Pixmap> {
        let base = mide::icons::load_icon(icon, ICON).or_else(|| mide::icons::load_themed_icon(&[icon, "application-x-executable", "text-x-generic"], ICON))?;
        let (cw, ch) = (CELL.0 * 2.0, CELL.1 * 2.0);
        let mut canvas = Pixmap::new(cw as u32, ch as u32)?;
        // Canto da célula na imagem.
        let (ox, oy) = (cw / 2.0 - grab.0, ch / 2.0 - grab.1);
        let scale = (ICON as f32 / base.width().max(base.height()) as f32).min(1.0);
        let (w, h) = (base.width() as f32 * scale, base.height() as f32 * scale);
        let (ix, iy) = (ox + (CELL.0 - w) / 2.0, oy + 8.0 + (ICON as f32 - h) / 2.0);
        let paint = tiny_skia::PixmapPaint { quality: tiny_skia::FilterQuality::Bilinear, ..Default::default() };
        canvas.draw_pixmap(0, 0, base.as_ref(), &paint, tiny_skia::Transform::from_scale(scale, scale).post_translate(ix, iy), None);
        // Nome na cápsula de escolhido (cortado com reticências se não couber). O medidor
        // de texto é criado uma vez só: criar e medir a primeira vez leva ~100 ms.
        DRAG_TEXT.with(|text| Self::paint_drag_label(&mut text.borrow_mut(), &mut canvas, name, count, (ox, oy), (ix, iy, w)));
        Some(canvas)
    }

    /// O nome e o selo de contagem na imagem do arrasto.
    fn paint_drag_label(text: &mut mide::ui::text::Text, canvas: &mut Pixmap, name: &str, count: usize, (ox, oy): (f32, f32), (ix, iy, w): (f32, f32, f32)) {
        let size = 12.0;
        let max = CELL.0 - 8.0;
        let mut label = name.to_string();
        let fits = |t: &mut mide::ui::text::Text, l: &str| t.measure(l, size, mide::ui::text::FontWeight::Regular) <= max - 12.0;
        if !fits(text, &label) {
            let mut chars: Vec<char> = name.chars().collect();
            while chars.len() > 1 && !fits(text, &format!("{}…", chars.iter().collect::<String>())) {
                chars.pop();
            }
            label = format!("{}…", chars.iter().collect::<String>());
        }
        let tw = text.measure(&label, size, mide::ui::text::FontWeight::Regular) + 12.0;
        let (tx, ty) = (ox + (CELL.0 - tw) / 2.0, oy + 8.0 + ICON as f32 + 6.0);
        let [r, g, b, _] = mide::kit::system_accent(mide::kit::system_dark());
        mide::ui::draw::fill_rounded(canvas, tx, ty, tw, 19.0, 6.0, tiny_skia::Color::from_rgba8(r, g, b, 255));
        text.draw(canvas, &label, tx + 6.0, ty + 2.0, size, mide::ui::text::FontWeight::Regular, [255, 255, 255, 255]);
        if count > 1 {
            let badge = count.to_string();
            let bw = text.measure(&badge, 12.0, mide::ui::text::FontWeight::Semibold).max(8.0) + 12.0;
            let bx = ix + w - bw / 2.0;
            mide::ui::draw::fill_rounded(canvas, bx, iy - 6.0, bw, 20.0, 10.0, tiny_skia::Color::from_rgba8(255, 59, 48, 255));
            text.draw(canvas, &badge, bx + 6.0, iy - 3.5, 12.0, mide::ui::text::FontWeight::Semibold, [255, 255, 255, 255]);
        }
    }

    fn keys(&mut self, ui: &mut Ui, area: (f32, f32, f32, f32)) {
        if ui.typing() || self.inline.is_some() || self.prompt.is_some() {
            return;
        }
        let input = ui.input();
        // Como no Finder: Enter renomeia; abrir é ⌘O ou ⌘↓ (Ctrl+O, Ctrl+↓).
        let ctrl_down = input.key_events().iter().any(|(k, ctrl, _)| *k == Key::Down && *ctrl);
        let (enter, delete, rename) = (input.key(Key::Enter), input.key(Key::Delete), input.shortcut("r"));
        let (open, all, copy, paste) = (input.shortcut("o") || ctrl_down, input.shortcut("a"), input.shortcut("c"), input.shortcut("v"));
        let items = self.items();
        if all {
            self.selected = items.iter().map(|i| i.path.clone()).collect();
            ui.request_repaint();
        }
        if paste {
            // Pelo teclado, sem lugar: o próximo livre a partir do canto de cima à direita.
            self.act(Action::Paste, None, area, ui);
        }
        let chosen = self.selected.first().and_then(|s| items.iter().find(|i| &i.path == s)).cloned();
        if let Some(item) = chosen {
            if open {
                self.act(Action::Open, Some(item), area, ui);
            } else if copy {
                self.act(Action::Copy, Some(item), area, ui);
            } else if (enter || rename) && self.selected.len() == 1 && item.drive.is_none() {
                self.act(Action::Rename, Some(item), area, ui);
            } else if delete && item.drive.is_none() {
                self.act(Action::Trash, Some(item), area, ui);
            }
            ui.request_repaint();
        }
    }

    /// Menu de um arquivo ou pasta (como o do Finder).
    fn item_menu(m: &mut mide::kit::Menu, item: &Item, count: usize, apps: &[(String, String, String)], tags: &[usize]) -> Option<Action> {
        let mut chosen = None;
        let multi = count > 1;
        if multi && m.item(&format!("Nova pasta com a seleção ({count} itens)")) {
            chosen = Some(Action::NewFolderWithSelection);
        }
        if m.item(&if multi { format!("Abrir {count} itens") } else { "Abrir".into() }) {
            chosen = Some(Action::Open);
        }
        if !item.is_dir && !apps.is_empty() {
            m.submenu(None, "Abrir com", |m| {
                for (i, (id, name, icon)) in apps.iter().enumerate() {
                    let label = if i == 0 { format!("{name} (padrão)") } else { name.clone() };
                    if m.item_icon(icon, &label, None) {
                        chosen = Some(Action::OpenWith(id.clone()));
                    }
                    if i == 0 && apps.len() > 1 {
                        m.separator();
                    }
                }
            });
        }
        m.separator();
        if m.item(&if multi { format!("Mover {count} itens para a Lixeira") } else { "Mover para a Lixeira".into() }) {
            chosen = Some(Action::Trash);
        }
        m.separator();
        if m.item("Obter informações") {
            chosen = Some(Action::Info);
        }
        if !multi && m.item("Renomear…") {
            chosen = Some(Action::Rename);
        }
        if m.item(&if multi { format!("Comprimir {count} itens") } else { format!("Comprimir \"{}\"", item.name) }) {
            chosen = Some(Action::Compress);
        }
        if m.item("Duplicar") {
            chosen = Some(Action::Duplicate);
        }
        if m.item("Criar atalho") {
            chosen = Some(Action::Alias);
        }
        m.separator();
        if m.item(&if multi { format!("Copiar {count} itens") } else { format!("Copiar \"{}\"", item.name) }) {
            chosen = Some(Action::Copy);
        }
        if m.item("Copiar como caminho") {
            chosen = Some(Action::CopyPath);
        }
        m.separator();
        let marked: Vec<bool> = (0..files::TAGS.len()).map(|i| tags.contains(&i)).collect();
        if let Some(i) = m.tags("Etiquetas…", &marked) {
            chosen = Some(Action::Tag(i));
        }
        chosen
    }
}

/// Copia arquivos: a lista `file://` vai para a área de transferência (o Arquivos e a
/// área de trabalho colam; outros apps recebem os endereços).
fn copy_files(paths: &[PathBuf]) {
    let text: Vec<String> = paths.iter().map(|p| mide::thumbnails::file_uri(p)).collect();
    tracing::info!("copiado(s): {} item(ns)", paths.len());
    places::copy_text(&text.join("\n"));
}

/// Desmonta o disco (e desliga, se puder) numa thread: o desenho não espera.
fn eject(drive: Drive) {
    std::thread::spawn(move || {
        let ok = std::process::Command::new("udisksctl").args(["unmount", "-b", &drive.device]).status().is_ok_and(|s| s.success());
        if ok {
            let _ = std::process::Command::new("udisksctl").args(["power-off", "-b", &drive.device]).status();
            tracing::info!("ejetado: {}", drive.mount.display());
        } else if !std::process::Command::new("gio").arg("mount").arg("-u").arg(&drive.mount).status().is_ok_and(|s| s.success()) {
            tracing::warn!("não foi possível ejetar {}", drive.mount.display());
        }
    });
}

/// Programa irmão (ao lado deste executável), ou o do PATH.
fn sibling(name: &str) -> PathBuf {
    std::env::current_exe().ok().and_then(|exe| exe.parent().map(|d| d.join(name))).filter(|p| p.exists()).unwrap_or_else(|| PathBuf::from(name))
}

impl App for Desktop {
    fn view(&mut self, ui: &mut Ui) {
        // A pasta pode mudar por fora (Arquivos, terminal) e discos entram e saem:
        // confere de tempos em tempos.
        if self.changed() || self.asked_at.is_none_or(|at| at.elapsed() > Duration::from_millis(1500)) && read_drives() != self.drives {
            self.reload();
        }
        // A área útil muda quando a dock muda de tamanho ou passa a se esconder
        // (pergunta no máximo a cada 1,5 s).
        if self.asked_at.is_none_or(|at| at.elapsed() > Duration::from_millis(1500)) {
            self.ask_workarea();
            self.asked_at = Some(Instant::now());
        }
        // Miniaturas que ficaram prontas.
        while let Ok((path, modified, pix)) = self.from_loader.try_recv() {
            self.waiting.remove(&path);
            self.thumbs.insert(path, (modified, pix));
            ui.request_repaint();
        }
        if !self.waiting.is_empty() {
            ui.request_tick();
        }
        ui.wake_after(Duration::from_secs(2));
        let region = ui.region();
        let area = self.workarea.unwrap_or(region);
        self.keys(ui, area);
        // Texto colado chegou: arquivos para a área de trabalho.
        if let Some(text) = ui.input().paste.clone()
            && let Some(point) = self.pending_paste.take()
        {
            self.paste_text(&text, point, area);
            ui.request_repaint();
        }
        let (cols, rows) = Self::grid(area);
        let items = self.items();
        let keys: Vec<String> = items.iter().map(|i| i.key.clone()).collect();
        let placed: Vec<(Item, (usize, usize))> = items.into_iter().zip(layout(&keys, &self.cells, cols, rows)).collect();
        let mut action: Option<(Action, Option<Item>)> = None;
        let mut select: Option<(PathBuf, bool)> = None;
        let (pointer, pressed, down, released, drop_hover, ctrl) = {
            let input = ui.input();
            (input.pointer, input.pressed, input.down, input.released, input.drop_hover, input.ctrl)
        };
        // Arrasto: começa quando o ponteiro anda mais que 5 pontos com o botão num ícone,
        // e vira o arrasto do Wayland (por cima das janelas, para qualquer app). Leva os
        // escolhidos, se o apertado está entre eles.
        if let (Some((path, start, offset)), Some(p)) = (self.press.clone(), pointer)
            && down
            && ((p.0 - start.0).powi(2) + (p.1 - start.1).powi(2)).sqrt() > 5.0
        {
            let drives: Vec<PathBuf> = self.drives.iter().map(|d| d.mount.clone()).collect();
            let mut paths: Vec<PathBuf> = if self.selected.contains(&path) { self.selected.clone() } else { vec![path.clone()] };
            paths.retain(|p| !drives.contains(p));
            if !paths.is_empty() {
                let (icon, name) = placed
                    .iter()
                    .find(|(i, _)| i.path == path)
                    .map(|(i, _)| (i.icon.clone(), i.name.clone()))
                    .unwrap_or_else(|| ("text-x-generic".into(), String::new()));
                tracing::info!("área de trabalho: arrastando {} item(ns)", paths.len());
                let image = self.press_icon.take().or_else(|| Self::drag_icon(&icon, &name, paths.len(), offset));
                ui.start_drag(&paths, image);
                self.drag_ended_at = None;
                if !self.selected.contains(&path) {
                    self.selected = vec![path.clone()];
                }
                self.drag_out = Some((paths, path, offset));
            }
            self.press = None;
        }
        // Renomear no lugar: o estado sai do app durante o desenho e volta depois.
        let mut inline = self.inline.take();
        let inline_was_focusing = inline.as_ref().is_some_and(|(_, _, f)| *f);
        let mut inline_done = false;
        let mut spring_over: Option<PathBuf> = None;
        let mut over_item = false;
        // "Abrir com": os apps que abrem arquivos, lidos uma vez, quando um menu abre.
        if ui.menu_open().is_some() && self.apps.is_none() {
            self.apps = Some(mide::apps::load_handlers());
        }
        for (item, cell) in &placed {
            let (x, y) = Self::cell_pos(*cell, area);
            let selected = self.selected.contains(&item.path);
            let inside = |p: (f32, f32)| p.0 >= x && p.0 < x + CELL.0 && p.1 >= y && p.1 < y + CELL.1;
            // Pasta sob um arrasto (deste ou de outro app): destacada, como quem vai
            // receber o item; segurando, abre (pasta de mola).
            let dragging_this = self.drag_out.as_ref().is_some_and(|(paths, ..)| paths.contains(&item.path));
            let drop_here = drop_hover && !dragging_this && item.is_dir && pointer.is_some_and(inside);
            // O ícone arrastado sai do lugar (é ele que segue o ponteiro); volta se o
            // arrasto for cancelado.
            if dragging_this {
                continue;
            }
            if drop_here {
                spring_over = Some(item.path.clone());
            }
            if pointer.is_some_and(inside) {
                over_item = true;
            }
            let thumb = self.thumb(item).cloned();
            let mut response = None;
            ui.at(x, y, CELL.0, |ui| {
                let image = match &thumb {
                    Some(pix) => Tile::Image(pix),
                    None => Tile::Icon(&item.icon),
                };
                response = Some(ui.tile(image, &item.name, selected || drop_here, ICON));
            });
            let Some(r) = response else {
                continue;
            };
            // O nome vira um campo de texto embaixo do ícone (mais largo que ele, como no
            // Finder), com o nome sem a extensão já escolhido.
            if let Some((path, name, focus)) = inline.as_mut()
                && *path == item.path
            {
                let width = 160.0;
                // Centrado no ícone, sem passar da borda da tela (coluna da direita).
                let fx = (r.rect.0 + (r.rect.2 - width) / 2.0).min(area.0 + area.2 - width - 6.0).max(area.0 + 6.0);
                let fy = r.rect.1 + 16.0 + ICON as f32 - 9.0;
                // O Enter que abriu o campo chega a ele no mesmo quadro: não confirma.
                let just_opened = std::mem::take(focus);
                if just_opened {
                    let stem = if item.is_dir { name.len() } else { name.rfind('.').filter(|i| *i > 0).unwrap_or(name.len()) };
                    ui.focus_next_input_selecting(stem);
                }
                ui.at(fx, fy, width, |ui| {
                    if ui.text_input(name, "").submitted() && !just_opened {
                        inline_done = true;
                    }
                });
            }
            if r.hovered()
                && pressed
                && self.press.is_none()
                && let Some(p) = pointer
            {
                self.press = Some((item.path.clone(), p, (p.0 - x, p.1 - y)));
                let count = if self.selected.contains(&item.path) { self.selected.iter().filter(|s| !self.drives.iter().any(|d| &d.mount == *s)).count().max(1) } else { 1 };
                self.press_icon = Self::drag_icon(&item.icon, &item.name, count, (p.0 - x, p.1 - y));
            }
            if r.clicked() {
                select = Some((item.path.clone(), ctrl));
            }
            if r.secondary_clicked() && !selected {
                select = Some((item.path.clone(), false));
            }
            if r.double_clicked() {
                action = Some((Action::Open, Some(item.clone())));
            }
            // Menu: discos têm o deles; arquivos e pastas, o do Finder.
            let count = if selected { self.selected.len() } else { 1 };
            if item.drive.is_some() {
                let name = item.name.clone();
                ui.context_menu(&r, |m| {
                    let mut chosen = None;
                    if m.item("Abrir") {
                        chosen = Some(Action::Open);
                    }
                    m.separator();
                    if m.item(&format!("Ejetar \"{name}\"")) {
                        chosen = Some(Action::Eject);
                    }
                    if let Some(chosen) = chosen {
                        action = Some((chosen, Some(item.clone())));
                    }
                });
            } else {
                let apps: Vec<(String, String, String)> = if ui.menu_open().is_some() {
                    self.apps
                        .as_ref()
                        .map(|apps| {
                            mide::default_apps::apps_for(apps, &item.path)
                                .into_iter()
                                .take(12)
                                .map(|a| (a.id.clone(), a.name.clone(), a.icon.clone().unwrap_or_else(|| "application-x-executable".into())))
                                .collect()
                        })
                        .unwrap_or_default()
                } else {
                    Vec::new()
                };
                let tags = files::tags_of(&item.path);
                ui.context_menu(&r, |m| {
                    if let Some(chosen) = Self::item_menu(m, item, count, &apps, &tags) {
                        action = Some((chosen, Some(item.clone())));
                    }
                });
            }
        }
        // Pasta de mola: segurando o arrasto em cima de uma pasta, ela abre no Arquivos
        // (uma vez por pasta).
        match (&spring_over, &mut self.spring) {
            (Some(folder), Some((f, at, fired))) if f == folder => {
                if !*fired && at.elapsed() >= SPRING {
                    *fired = true;
                    tracing::info!("pasta de mola: {}", folder.display());
                    places::open(&folder.to_string_lossy());
                }
                ui.request_tick();
            }
            (Some(folder), _) => {
                self.spring = Some((folder.clone(), Instant::now(), false));
                ui.request_tick();
            }
            (None, _) => self.spring = None,
        }
        // Renomear no lugar: Enter confirma, Esc cancela e clicar fora (o campo perde o
        // foco) também confirma, como no Finder.
        let mut renamed_now = false;
        if let Some((path, name, focusing)) = inline {
            let escape = ui.input().key(Key::Escape);
            let lost = !inline_was_focusing && !focusing && !ui.typing();
            if escape {
                ui.blur();
                renamed_now = true;
            } else if inline_done || lost {
                ui.blur();
                self.finish_rename(path, name);
                renamed_now = true;
            } else {
                self.inline = Some((path, name, focusing));
            }
            ui.request_repaint();
        }
        // Botão solto sem arrastar: só esquece o aperto (clique comum).
        if released || !down {
            self.press = None;
        }
        // Arquivos soltos na área de trabalho: os arrastados daqui mudam de lugar (ou
        // entram na pasta onde caíram); de outro app, são copiados para a pasta sob o
        // ponteiro ou para a área de trabalho, no lugar onde caíram.
        let dropped = ui.input().dropped.clone();
        let just_dropped = dropped.is_some();
        // Arrasto terminado sem cair aqui (outro app ou cancelado): depois de um instante
        // (o "soltou aqui" pode chegar um pouco depois do fim), os ícones voltam.
        if ui.input().drag_ended && self.drag_out.is_some() {
            self.drag_ended_at = Some(std::time::Instant::now());
            ui.wake_after(std::time::Duration::from_millis(400));
        }
        if self.drag_ended_at.is_some_and(|t| t.elapsed() >= std::time::Duration::from_millis(350)) && !just_dropped {
            self.drag_ended_at = None;
            if self.drag_out.take().is_some() {
                tracing::info!("área de trabalho: arrasto terminou fora daqui, ícones de volta");
            }
        }
        if let (Some(paths), Some(p)) = (dropped, pointer) {
            let own = self.drag_out.take().filter(|(out, ..)| out.len() == paths.len() && paths.iter().all(|p| out.contains(p)));
            if let Some((paths, pressed_path, offset)) = own {
                self.drop_icons(&paths, &pressed_path, (p.0 - offset.0, p.1 - offset.1), p, &placed, area);
            } else {
                let folder = placed.iter().find(|(i, cell)| {
                    let (x, y) = Self::cell_pos(*cell, area);
                    i.is_dir && p.0 >= x && p.0 < x + CELL.0 && p.1 >= y && p.1 < y + CELL.1
                });
                let into_desktop = folder.is_none();
                if let Some(dir) = folder.map(|(i, _)| i.path.clone()).or_else(|| self.dir.clone()) {
                    let mut made = Vec::new();
                    for path in &paths {
                        match files::copy_into(path, &dir) {
                            Ok(copy) => {
                                tracing::info!("área de trabalho: copiado {} → {}", path.display(), copy.display());
                                made.push(copy);
                            }
                            Err(err) => tracing::warn!("não foi copiado: {err}"),
                        }
                    }
                    if into_desktop {
                        self.place_new(&made, Some(p), area);
                    }
                }
                self.reload();
            }
            ui.request_repaint();
        }
        if let Some((path, add)) = select {
            if add {
                if let Some(i) = self.selected.iter().position(|s| *s == path) {
                    self.selected.remove(i);
                } else {
                    self.selected.push(path);
                }
            } else {
                self.selected = vec![path];
            }
            ui.request_repaint();
        }
        // Fundo: apertar e arrastar desenha o retângulo de seleção; clique tira a
        // seleção e mostra a mesa; botão direito abre o menu da área de trabalho.
        let whole = ui.region();
        let bg = ui.background(whole);
        // O retângulo é o componente do mide-ui (o mesmo do Arquivos).
        let can_start = bg.hovered() && !over_item && self.inline.is_none();
        let band = self.band.update(ui, whole, can_start);
        if self.band.pressed() && self.band_base.is_none() {
            self.band_base = Some(if ctrl { self.selected.clone() } else { Vec::new() });
        }
        if let Some(r) = band {
            let mut chosen = self.band_base.clone().unwrap_or_default();
            for (item, cell) in &placed {
                // O ícone e o nome (a parte do ladrilho que aparece).
                let (x, y) = Self::cell_pos(*cell, area);
                if mide::kit::band_touches(r, (x + 18.0, y + 2.0, CELL.0 - 36.0, CELL.1 - 12.0)) && !chosen.contains(&item.path) {
                    chosen.push(item.path.clone());
                }
            }
            self.selected = chosen;
        }
        if !self.band.pressed() {
            self.band_base = None;
        }
        let band_click = self.band.active() || self.band.finished();
        if bg.clicked() && !self.selected.is_empty() && !just_dropped && !band_click && !ctrl {
            self.selected.clear();
            ui.request_repaint();
        }
        // Clique no papel de parede (espaço vazio): o compositor afasta as janelas para
        // mostrar a mesa, ou as traz de volta (como no macOS). Numa thread: o desenho
        // não espera a resposta.
        if bg.clicked() && !just_dropped && !renamed_now && !band_click {
            std::thread::spawn(|| {
                let _ = mide::control_client::ask_compositor("reveal");
            });
        }
        // Clique direito no vazio: o que o menu criar aparece ali.
        if bg.secondary_clicked() {
            self.menu_point = pointer;
        }
        let has_dir = self.dir.is_some();
        ui.context_menu(&bg, |m| {
            let mut chosen = None;
            if has_dir {
                if m.item("Nova pasta") {
                    chosen = Some(Action::NewFolder);
                }
                m.submenu(None, "Novo arquivo", |m| {
                    for (ext, label, _, icon) in mide::templates::KINDS {
                        if m.item_icon(icon, label, None) {
                            chosen = Some(Action::NewFile(ext));
                        }
                    }
                });
                if m.item("Colar") {
                    chosen = Some(Action::Paste);
                }
                m.separator();
                if m.item("Abrir no Terminal") {
                    chosen = Some(Action::Terminal);
                }
                if m.item("Mostrar no Arquivos") {
                    chosen = Some(Action::ShowFolder);
                }
                m.separator();
                if m.item("Organizar") {
                    chosen = Some(Action::CleanUp);
                }
            }
            if m.item("Mudar papel de parede…") {
                chosen = Some(Action::Wallpaper);
            }
            if let Some(chosen) = chosen {
                action = Some((chosen, None));
            }
        });
        if let Some((a, item)) = action {
            self.act(a, item, area, ui);
            ui.request_repaint();
        }
        // Renomeando ou com uma confirmação aberta: a camada vem para a frente das
        // janelas e pega o teclado (o nome digitado nunca fica escondido); depois, volta
        // para trás delas.
        ui.set_raised(self.inline.is_some() || self.prompt.is_some());
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
    // Nomes brancos (com sombra) sobre o papel de parede.
    let theme = Theme { tile_text: [255, 255, 255, 255], ..Theme::default() };
    warm_drag_text();
    run(Window::new("Área de trabalho").app_id("mide-desktop").desktop("mide-desktop").accepts_files().theme(theme), Desktop::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icones_em_colunas_a_partir_da_direita() {
        // Área útil de 800 x 500 abaixo de uma barra de 28 px: 4 linhas por coluna.
        let area = (0.0, 28.0, 800.0, 500.0);
        let (_, rows) = Desktop::grid(area);
        assert_eq!(rows, 4);
        let names: Vec<String> = (0..5).map(|i| format!("f{i}")).collect();
        let cells = layout(&names, &HashMap::new(), 7, rows);
        assert_eq!(cells[0], (0, 0));
        assert_eq!(cells[1], (0, 1));
        // Quinto ícone: segunda coluna, à esquerda da primeira, de novo em cima.
        assert_eq!(cells[4], (1, 0));
        let (x0, y0) = Desktop::cell_pos(cells[0], area);
        assert_eq!((x0, y0), (800.0 - MARGIN - CELL.0, 28.0 + MARGIN));
        assert!(Desktop::cell_pos(cells[4], area).0 < x0);
        assert_eq!(Desktop::nearest_cell((x0 - 3.0, y0 + 40.0), area), (0, 0));
        assert_eq!(Desktop::nearest_cell((x0 - 100.0, y0 + 110.0), area), (1, 1));
    }

    #[test]
    fn lugares_guardados() {
        let mut saved = HashMap::new();
        saved.insert("b".to_string(), (2, 1));
        // Lugar fora da grade (tela menor) é ignorado; lugar ocupado também.
        saved.insert("c".to_string(), (9, 9));
        let names = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let cells = layout(&names, &saved, 4, 3);
        assert_eq!(cells, vec![(0, 0), (2, 1), (0, 1)]);
        let text = format_cells(&saved);
        assert_eq!(parse_cells(&text), saved);
        assert_eq!(parse_cells("1\t2\tcom espaço e\ttab"), HashMap::from([("com espaço e\ttab".to_string(), (1, 2))]));
    }

    #[test]
    fn lugar_livre_mais_perto() {
        let taken: HashSet<(usize, usize)> = [(3, 2), (3, 3)].into_iter().collect();
        assert_eq!(nearest_free((5, 5), &taken, 8, 6), (5, 5));
        let near = nearest_free((3, 2), &taken, 8, 6);
        assert!(!taken.contains(&near));
        assert!((near.0 as i64 - 3).abs() + (near.1 as i64 - 2).abs() == 1);
    }

    #[test]
    fn discos_do_mountinfo() {
        let text = "\
36 25 8:1 / / rw,relatime shared:1 - ext4 /dev/sda1 rw
80 36 8:17 / /media/fabio/PEN\\040DRIVE rw,nosuid shared:40 - vfat /dev/sdb1 rw
81 36 11:0 / /run/media/fabio/UBUNTU_24 ro shared:41 - iso9660 /dev/sr0 ro
82 36 0:50 / /media/fabio rw shared:42 - tmpfs tmpfs rw
83 36 8:33 / /mnt/backup rw shared:43 - ext4 /dev/sdc1 rw
";
        let d = parse_mountinfo(text);
        assert_eq!(d.len(), 3);
        assert_eq!(d[0].mount, PathBuf::from("/media/fabio/PEN DRIVE"));
        assert_eq!(drive_icon(&d[1]), "media-optical");
        assert_eq!(d[2].device, "/dev/sdc1");
    }
}
