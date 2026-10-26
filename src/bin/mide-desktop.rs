//! Área de trabalho do mide: os arquivos e pastas da pasta "Área de trabalho" como
//! ícones sobre o papel de parede, em colunas a partir do canto direito (como no
//! macOS). Feito com o mide-ui, numa camada de fundo (atrás das janelas).
//!
//! Clique seleciona, duplo clique abre (pastas no Arquivos). Botão direito num ícone:
//! Abrir, Renomear, Duplicar, Copiar caminho e Mover para a Lixeira; no vazio: Nova
//! pasta, Novo arquivo, Abrir no Terminal, Mostrar no Arquivos e Mudar papel de
//! parede. Teclado (depois de clicar na área de trabalho): Enter renomeia, Ctrl+O
//! abre, Delete manda para a Lixeira.
//!
//! Programas (E8): atalhos `.desktop` aparecem com o nome e o ícone do app; atalhos,
//! AppImages, executáveis e scripts rodam com duplo clique (ou perguntam antes, como
//! no Arquivos: `mide::programs`).
//!
//! Arrastar um ícone muda o lugar dele na grade (como no Mac, com "Alinhar à grade"):
//! o lugar fica guardado em `~/.local/state/mide/desktop-positions` e, se já houver
//! outro ícone lá, os dois trocam de lugar. Soltar em cima de uma pasta move o item
//! para dentro dela.
//!
//! Renomear é como no Finder: Enter (ou "Renomear", ou Ctrl+R) troca o nome embaixo
//! do ícone por um campo com o nome sem a extensão escolhido; Enter confirma, Esc
//! cancela, clicar fora confirma. Nova pasta e Novo arquivo criam com um nome livre e
//! já abrem o campo. Abrir pelo teclado: Ctrl+O ou Ctrl+↓ (⌘O/⌘↓). Enquanto o campo
//! está aberto, a camada vem para a frente das janelas.
//!
//! `MIDE_DESKTOP_DIR` troca a pasta (os testes usam uma pasta temporária).

use std::{collections::HashMap, error::Error, path::PathBuf, time::Duration};

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
    /// Renomear no lugar (Enter, como no Finder): o item, o nome sendo digitado e se o
    /// campo ainda precisa receber o foco.
    inline: Option<(PathBuf, String, bool)>,
    /// Área útil da tela (sem a barra e a dock), perguntada ao compositor.
    workarea: Option<(f32, f32, f32, f32)>,
    asked_at: Option<std::time::Instant>,
    /// Confirmação antes de rodar um programa (E8).
    prompt: Option<Prompt>,
    /// Nome e ícone dos atalhos `.desktop`, no lugar do nome do arquivo.
    labels: Labels,
    /// Lugar escolhido de cada ícone (nome do arquivo → coluna a partir da direita, linha).
    cells: HashMap<String, (usize, usize)>,
    /// Ícone apertado: caminho, ponto onde o botão desceu e distância do ponteiro ao
    /// canto do ícone. Vira arrasto quando o ponteiro anda mais que alguns pontos.
    press: Option<Press>,
    /// Ícone sendo arrastado pelo arrasto do Wayland (caminho e distância do ponteiro
    /// ao canto dele): o compositor desenha o ícone por cima das janelas, e quem recebe
    /// é a janela sob o ponteiro (o Editor de Texto abre o arquivo) ou a própria área
    /// de trabalho (muda o lugar do ícone).
    drag_out: Option<(PathBuf, (f32, f32))>,
}

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
    let mut taken = std::collections::HashSet::new();
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
        let mut app = Self { dir: places::desktop_dir(), entries: Vec::new(), mtime: None, selected: None, inline: None, workarea: None, asked_at: None, prompt: None, labels: Labels::default(), cells: HashMap::new(), press: None, drag_out: None };
        app.cells = cells_path().and_then(|p| std::fs::read_to_string(p).ok()).map(|t| parse_cells(&t)).unwrap_or_default();
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

    /// Começa a renomear no lugar: o nome embaixo do ícone vira um campo de texto.
    fn start_rename(&mut self, path: PathBuf) {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.selected = Some(path.clone());
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
                self.selected = Some(new_path);
            }
            Err(err) => tracing::warn!("não foi renomeado: {err}"),
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
            (Action::Rename, Some(e)) => self.start_rename(e.path.clone()),
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
            // Como no Finder: cria com um nome livre e já deixa o nome para editar.
            (Action::NewFolder, _) | (Action::NewFile, _) => {
                if let Some(dir) = dir {
                    let folder = matches!(action, Action::NewFolder);
                    let name = if folder { files::numbered_name(&dir, "Nova pasta", "") } else { files::numbered_name(&dir, "Novo arquivo", "txt") };
                    match files::create_named(&dir, &name, folder) {
                        Ok(path) => {
                            tracing::info!("área de trabalho: {}", path.display());
                            self.reload();
                            self.start_rename(path);
                        }
                        Err(err) => tracing::warn!("não foi criado: {err}"),
                    }
                }
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
            (Action::Wallpaper, _) => mide::system::open_settings("wallpaper"),
            _ => {}
        }
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

    /// Ícone solto: dentro de uma pasta (se caiu em cima de uma) ou numa célula nova;
    /// quem estava na célula vai para o lugar antigo do arrastado.
    fn drop_icon(&mut self, path: PathBuf, corner: (f32, f32), pointer: (f32, f32), placed: &[(Entry, (usize, usize))], area: (f32, f32, f32, f32)) {
        let target = placed.iter().find(|(e, cell)| {
            let (x, y) = Self::cell_pos(*cell, area);
            e.path != path && e.is_dir && pointer.0 >= x && pointer.0 < x + CELL.0 && pointer.1 >= y && pointer.1 < y + CELL.1
        });
        if let Some((folder, _)) = target {
            match files::move_into(&path, &folder.path) {
                Ok(moved) => tracing::info!("área de trabalho: movido para {}", moved.display()),
                Err(err) => tracing::warn!("não foi movido: {err}"),
            }
            self.reload();
            return;
        }
        let name = |p: &PathBuf| p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let Some(from) = placed.iter().find(|(e, _)| e.path == path).map(|(_, c)| *c) else {
            return;
        };
        let to = Self::nearest_cell(corner, area);
        if from == to {
            return;
        }
        // Quem está na célula de destino troca de lugar com o arrastado; os outros
        // ficam onde estão (todos passam a ter o lugar guardado).
        for (e, cell) in placed {
            let cell = if *cell == to { from } else { *cell };
            self.cells.insert(name(&e.path), cell);
        }
        self.cells.insert(name(&path), to);
        self.save_cells();
    }

    /// Imagem do ícone que segue o ponteiro no arrasto (um pouco transparente).
    fn drag_icon(icon: &str) -> Option<tiny_skia::Pixmap> {
        const SIZE: u32 = 72;
        let base = mide::icons::load_icon(icon, 64)?;
        let mut canvas = tiny_skia::Pixmap::new(SIZE, SIZE)?;
        let scale = (60.0 / base.width().max(base.height()) as f32).min(1.0);
        let (w, h) = (base.width() as f32 * scale, base.height() as f32 * scale);
        let paint = tiny_skia::PixmapPaint { opacity: 0.85, quality: tiny_skia::FilterQuality::Bilinear, ..Default::default() };
        let tf = tiny_skia::Transform::from_scale(scale, scale).post_translate((SIZE as f32 - w) / 2.0, (SIZE as f32 - h) / 2.0);
        canvas.draw_pixmap(0, 0, base.as_ref(), &paint, tf, None);
        Some(canvas)
    }

    fn keys(&mut self, ui: &mut Ui) {
        if ui.typing() || self.inline.is_some() || self.prompt.is_some() {
            return;
        }
        let input = ui.input();
        // Como no Finder: Enter renomeia; abrir é ⌘O ou ⌘↓ (Ctrl+O, Ctrl+↓).
        let ctrl_down = input.key_events().iter().any(|(k, ctrl, _)| *k == Key::Down && *ctrl);
        let (enter, delete, rename) = (input.key(Key::Enter), input.key(Key::Delete), input.shortcut("r"));
        let open = input.shortcut("o") || ctrl_down;
        let chosen = self.selected.as_ref().and_then(|s| self.entries.iter().find(|e| &e.path == s)).cloned();
        if let Some(entry) = chosen {
            if open {
                self.act(Action::Open, Some(entry));
            } else if enter || rename {
                self.act(Action::Rename, Some(entry));
            } else if delete {
                self.act(Action::Trash, Some(entry));
            }
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
        let (cols, rows) = Self::grid(area);
        let names: Vec<String> = self.entries.iter().map(|e| e.name.clone()).collect();
        let placed: Vec<(Entry, (usize, usize))> = self.entries.iter().cloned().zip(layout(&names, &self.cells, cols, rows)).collect();
        let mut action: Option<(Action, Option<Entry>)> = None;
        let mut select = None;
        // Arrasto: começa quando o ponteiro anda mais que 5 pontos com o botão no ícone,
        // e vira o arrasto do Wayland (por cima das janelas, para qualquer app).
        let (pointer, pressed, down, released, drop_hover) = {
            let input = ui.input();
            (input.pointer, input.pressed, input.down, input.released, input.drop_hover)
        };
        if let (Some((path, start, offset)), Some(p)) = (&self.press, pointer)
            && down
            && ((p.0 - start.0).powi(2) + (p.1 - start.1).powi(2)).sqrt() > 5.0
        {
            let icon = match self.entries.iter().find(|e| &e.path == path) {
                Some(e) => self.labels.get(&e.path, e.modified).map(|(_, icon)| icon).unwrap_or_else(|| e.icon_name().to_string()),
                None => "text-x-generic".into(),
            };
            tracing::info!("área de trabalho: arrastando {}", path.display());
            ui.start_drag(std::slice::from_ref(path), Self::drag_icon(&icon));
            self.drag_out = Some((path.clone(), *offset));
            self.selected = Some(path.clone());
            self.press = None;
        }
        // Renomear no lugar: o estado sai do app durante o desenho e volta depois.
        let mut inline = self.inline.take();
        let inline_was_focusing = inline.as_ref().is_some_and(|(_, _, f)| *f);
        let mut inline_done = false;
        for (entry, cell) in &placed {
            let (x, y) = Self::cell_pos(*cell, area);
            let selected = self.selected.as_ref() == Some(&entry.path);
            let (name, icon) = self.labels.get(&entry.path, entry.modified).unwrap_or_else(|| (entry.name.clone(), entry.icon_name().to_string()));
            // Pasta sob um arrasto (deste ou de outro app): destacada, como quem vai
            // receber o item.
            let dragging_this = self.drag_out.as_ref().is_some_and(|(p, _)| p == &entry.path);
            let drop_here = drop_hover && !dragging_this && entry.is_dir && pointer.is_some_and(|p| p.0 >= x && p.0 < x + CELL.0 && p.1 >= y && p.1 < y + CELL.1);
            let mut response = None;
            ui.at(x, y, CELL.0, |ui| {
                response = Some(ui.tile(Tile::Icon(&icon), &name, selected || drop_here, ICON));
            });
            let Some(r) = response else {
                continue;
            };
            // O nome vira um campo de texto embaixo do ícone (mais largo que ele, como no
            // Finder), com o nome sem a extensão já escolhido.
            if let Some((path, name, focus)) = inline.as_mut()
                && *path == entry.path
            {
                let width = 160.0;
                // Centrado no ícone, sem passar da borda da tela (coluna da direita).
                let fx = (r.rect.0 + (r.rect.2 - width) / 2.0).min(area.0 + area.2 - width - 6.0).max(area.0 + 6.0);
                let fy = r.rect.1 + 16.0 + ICON as f32 - 9.0;
                // O Enter que abriu o campo chega a ele no mesmo quadro: não confirma.
                let just_opened = std::mem::take(focus);
                if just_opened {
                    let stem = if entry.is_dir { name.len() } else { name.rfind('.').filter(|i| *i > 0).unwrap_or(name.len()) };
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
                self.press = Some((entry.path.clone(), p, (p.0 - x, p.1 - y)));
            }
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
        // Arquivos soltos na área de trabalho: o ícone arrastado daqui muda de lugar (ou
        // entra na pasta onde caiu); de outro app, são copiados para a pasta sob o
        // ponteiro ou para a área de trabalho.
        let dropped = ui.input().dropped.clone();
        let just_dropped = dropped.is_some();
        if let (Some(paths), Some(p)) = (dropped, pointer) {
            let own = self.drag_out.take().filter(|(path, _)| paths.len() == 1 && &paths[0] == path);
            if let Some((path, offset)) = own {
                self.drop_icon(path, (p.0 - offset.0, p.1 - offset.1), p, &placed, area);
            } else {
                let folder = placed.iter().find(|(e, cell)| {
                    let (x, y) = Self::cell_pos(*cell, area);
                    e.is_dir && p.0 >= x && p.0 < x + CELL.0 && p.1 >= y && p.1 < y + CELL.1
                });
                if let Some(dir) = folder.map(|(e, _)| e.path.clone()).or_else(|| self.dir.clone()) {
                    for path in &paths {
                        match files::copy_into(path, &dir) {
                            Ok(copy) => tracing::info!("área de trabalho: copiado {} → {}", path.display(), copy.display()),
                            Err(err) => tracing::warn!("não foi copiado: {err}"),
                        }
                    }
                }
                self.reload();
            }
            ui.request_repaint();
        }
        if let Some(path) = select {
            self.selected = Some(path);
            ui.request_repaint();
        }
        // Fundo: clique tira a seleção; botão direito abre o menu da área de trabalho.
        let area = ui.region();
        let bg = ui.background(area);
        if bg.clicked() && self.selected.is_some() && !just_dropped {
            self.selected = None;
            ui.request_repaint();
        }
        // Clique no papel de parede (espaço vazio): o compositor afasta as janelas para
        // mostrar a mesa, ou as traz de volta (como no macOS). Numa thread: o desenho
        // não espera a resposta.
        if bg.clicked() && !just_dropped && !renamed_now {
            std::thread::spawn(|| {
                let _ = mide::control_client::ask_compositor("reveal");
            });
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
}
