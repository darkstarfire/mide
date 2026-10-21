//! Buscador do mide (estilo Spotlight): uma barra flutuante sobre o vidro líquido,
//! aberta por atalho (`ctrl+space`, ou `toggle_search` no `config.toml`). Encontra
//! apps, seções das Configurações e arquivos das pastas pessoais, e faz contas.
//!
//! Teclado: digitar busca, setas escolhem, Enter abre, Esc fecha. Clicar fora fecha.
//! Feito com o mide-ui, como camada (`Window::overlay`).

use std::{
    error::Error,
    path::PathBuf,
    sync::mpsc::{channel, Receiver},
};

use mide::{
    apps::{self, App as DesktopApp},
    kit::{run, App, Key, Theme, Ui, Window},
    places,
    search::{self, Indexed},
};

/// Largura máxima do painel e quantos resultados cabem.
const PANEL_W: f32 = 680.0;
const MAX_ROWS: usize = 8;
/// Tamanho do ícone nas linhas de resultado (`Ui::result_row`).
const ROW_ICON: u32 = 28;

#[derive(Clone)]
enum Hit {
    Calc(String),
    App(usize),
    Setting(&'static str),
    File(PathBuf),
}

struct Row {
    hit: Hit,
    icon: String,
    title: String,
    detail: String,
}

struct Search {
    apps: Vec<DesktopApp>,
    index: Vec<Indexed>,
    index_rx: Option<Receiver<Vec<Indexed>>>,
    /// Ícones dos apps carregados em segundo plano (nome, imagem).
    icons_rx: Option<Receiver<(String, Option<tiny_skia::Pixmap>)>>,
    home: PathBuf,
    query: String,
    rows: Vec<Row>,
    /// Busca que gerou `rows` (refeitos só quando a busca muda).
    rows_for: Option<String>,
    selected: usize,
    started: bool,
    panel: Option<(f32, f32, f32, f32)>,
}

impl Search {
    fn new() -> Self {
        // Índice dos arquivos em segundo plano: a barra abre na hora.
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let start = std::time::Instant::now();
            let home = places::home_dir();
            let deep: Vec<PathBuf> = mide::files::favorites().into_iter().map(|(_, p, _)| p).filter(|p| *p != home).collect();
            let mut index = search::index_files(&deep, 4, 20_000);
            index.extend(search::index_files(&[home], 1, 2_000));
            tracing::debug!("índice de arquivos: {} itens em {:.0} ms", index.len(), start.elapsed().as_secs_f32() * 1000.0);
            let _ = tx.send(index);
        });
        let apps = apps::load();
        // Ícones dos apps carregados numa thread: na hora de digitar, já estão prontos.
        let names: Vec<String> = apps.iter().filter_map(|a| a.icon.clone()).collect();
        let (icon_tx, icons_rx) = channel();
        std::thread::spawn(move || {
            for name in names {
                let icon = mide::icons::load_icon(&name, ROW_ICON);
                if icon_tx.send((name, icon)).is_err() {
                    break;
                }
            }
        });
        Self {
            apps,
            icons_rx: Some(icons_rx),
            index: Vec::new(),
            index_rx: Some(rx),
            home: places::home_dir(),
            query: String::new(),
            rows: Vec::new(),
            rows_for: None,
            selected: 0,
            started: false,
            panel: None,
        }
    }

    /// Resultados da busca atual: conta, apps, Configurações e arquivos.
    fn refresh(&mut self) {
        if self.rows_for.as_deref() == Some(self.query.as_str()) {
            return;
        }
        self.rows_for = Some(self.query.clone());
        self.selected = 0;
        let mut rows = Vec::new();
        let q = self.query.trim();
        if q.is_empty() {
            self.rows = rows;
            return;
        }
        if let Some(value) = search::calculate(q) {
            let text = search::number_text(value);
            rows.push(Row { hit: Hit::Calc(text.clone()), icon: "mide-calculator".into(), title: format!("= {text}"), detail: "Enter copia o resultado".into() });
        }
        // Apps com uma palavra do nome começando pela busca vêm antes das
        // Configurações; os que só contêm a busca no meio de uma palavra, depois.
        let folded = apps::fold(q);
        let (strong, weak): (Vec<&DesktopApp>, Vec<&DesktopApp>) =
            apps::filter(&self.apps, q, None).into_iter().take(5).partition(|a| search::word_starts_with(&apps::fold(&a.name), &folded));
        let app_row = |app: &DesktopApp| {
            let index = self.apps.iter().position(|a| a.id == app.id).unwrap_or(0);
            let icon = app.icon.clone().unwrap_or_else(|| "application-x-executable".into());
            Row { hit: Hit::App(index), icon, title: app.name.clone(), detail: "Aplicativo".into() }
        };
        rows.extend(strong.into_iter().map(app_row));
        for (name, _, section, icon) in search::match_settings(q).into_iter().take(3) {
            rows.push(Row { hit: Hit::Setting(section), icon: (*icon).into(), title: (*name).into(), detail: "Configurações".into() });
        }
        rows.extend(weak.into_iter().map(app_row));
        let room = MAX_ROWS.saturating_sub(rows.len());
        for item in search::match_files(&self.index, q, room.min(5)) {
            let name = item.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let entry = mide::places::Item { path: item.path.clone(), name: name.clone(), is_dir: item.is_dir, modified: std::time::SystemTime::UNIX_EPOCH };
            rows.push(Row {
                hit: Hit::File(item.path.clone()),
                icon: places::icon_name_for(&entry).into(),
                title: name,
                detail: search::short_parent(&item.path, &self.home),
            });
        }
        rows.truncate(MAX_ROWS);
        self.rows = rows;
    }

    fn activate(&self, ui: &mut Ui, index: usize) {
        let Some(row) = self.rows.get(index) else {
            return;
        };
        match &row.hit {
            Hit::Calc(text) => places::copy_text(text),
            Hit::App(i) => {
                let app = &self.apps[*i];
                tracing::info!("buscador: abrir {}", app.name);
                if let Err(err) = apps::launch(app) {
                    tracing::warn!("falha ao abrir {}: {err}", app.name);
                }
            }
            Hit::Setting(section) => mide::system::open_settings(section),
            Hit::File(path) => places::open(&path.to_string_lossy()),
        }
        ui.close();
    }
}

impl App for Search {
    fn view(&mut self, ui: &mut Ui) {
        // Índice de arquivos que acabou de chegar.
        if let Some(rx) = &self.index_rx {
            match rx.try_recv() {
                Ok(index) => {
                    self.index = index;
                    self.index_rx = None;
                    self.rows_for = None;
                    ui.request_repaint();
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => ui.request_tick(),
                Err(_) => self.index_rx = None,
            }
        }
        if let Some(rx) = &self.icons_rx {
            loop {
                match rx.try_recv() {
                    Ok((name, icon)) => ui.provide_icon(&name, ROW_ICON, icon),
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(_) => {
                        self.icons_rx = None;
                        break;
                    }
                }
            }
        }
        let input = ui.input();
        let (esc, enter, up, down) = (input.key(Key::Escape), input.key(Key::Enter), input.key(Key::Up), input.key(Key::Down));
        if esc {
            ui.close();
            return;
        }
        self.refresh();
        if down && !self.rows.is_empty() {
            self.selected = (self.selected + 1).min(self.rows.len() - 1);
            ui.request_repaint();
        }
        if up {
            self.selected = self.selected.saturating_sub(1);
            ui.request_repaint();
        }
        if enter {
            self.activate(ui, self.selected);
            return;
        }

        let (_, _, sw, sh) = ui.region();
        let width = PANEL_W.min(sw - 32.0);
        let (x, y) = ((sw - width) / 2.0, (sh * 0.2).round());
        let first = !self.started;
        self.started = true;
        let mut clicked = None;
        let selected = self.selected;
        let rows = &self.rows;
        let query = &mut self.query;
        let panel = ui.glass_panel(x, y, width, 18.0, |ui| {
            if first {
                ui.focus_next_input();
            }
            ui.text_input(query, "Buscar apps, ajustes, arquivos e contas");
            for (i, row) in rows.iter().enumerate() {
                if ui.result_row(&row.icon, &row.title, &row.detail, i == selected).clicked() {
                    clicked = Some(i);
                }
            }
        });
        self.panel = Some(panel);
        if let Some(i) = clicked {
            self.activate(ui, i);
            return;
        }
        // Clique fora do painel fecha (como o Spotlight).
        let area = ui.region();
        let outside = ui.background(area);
        if outside.clicked() && !ui.input().over(panel) {
            ui.close();
        }
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "mide_search=info".into()))
        .init();
    let mut theme = Theme::default();
    // Campo de busca mais alto e letra maior, como o Spotlight.
    theme.row = 44.0;
    theme.font = 15.0;
    run(Window::new("Buscar").app_id("mide-search").overlay("mide-search").theme(theme), Search::new())
}
