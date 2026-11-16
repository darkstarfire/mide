//! Loja (`mide-store`), como a App Store do Mac: os apps da própria distribuição (o apt
//! do mios), com nome, ícone, descrição e capturas do catálogo AppStream
//! (`mide::appstream`), e instalar, abrir e remover pelo PackageKit (`mide::packages`),
//! que pede a senha de administrador pelo polkit. Nada de catálogo próprio: a lista é a
//! dos pacotes da distribuição.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};

use mide::appstream::{installed_packages, Component};
use mide::kit::{run, window_menu, App, MenuGroup, MenuItem, Tile, Ui, Window};
use mide::packages::{self, Package, Progress};
use mide::tr;
use tiny_skia::Pixmap;

/// Seções da barra lateral: (nome, símbolo, categorias do freedesktop).
const SECTIONS: &[(&str, &str, &[&str])] = &[
    ("Discover", "starred-symbolic", &[]),
    ("Create", "applications-graphics-symbolic", &["Graphics", "2DGraphics", "3DGraphics", "Photography", "VectorGraphics", "RasterGraphics"]),
    ("Work", "x-office-document-symbolic", &["Office", "WordProcessor", "Spreadsheet", "Presentation", "Finance"]),
    ("Play", "applications-games-symbolic", &["Game"]),
    ("Develop", "applications-engineering-symbolic", &["Development", "IDE", "TextEditor"]),
    ("Internet", "web-browser-symbolic", &["Network", "WebBrowser", "Email", "Chat", "InstantMessaging"]),
    ("Music and Video", "applications-multimedia-symbolic", &["AudioVideo", "Audio", "Video", "Player", "Recorder"]),
    ("Utilities", "applications-utilities-symbolic", &["Utility", "System", "Settings", "Accessibility"]),
    ("Education and Science", "applications-science-symbolic", &["Education", "Science", "Math"]),
    ("Installed", "folder-download-symbolic", &[]),
];
const INSTALLED: usize = SECTIONS.len() - 1;

/// Destaques em Descobrir (os que existirem no catálogo).
const FEATURED: &[&str] = &[
    "gimp", "inkscape", "krita", "blender", "libreoffice-writer", "libreoffice-calc", "thunderbird", "vlc", "audacity", "obs-studio", "kdenlive",
    "darktable", "handbrake", "calibre", "transmission-gtk", "filezilla", "remmina", "shotwell", "rhythmbox", "scribus",
];

/// O que as threads mandam para a janela.
enum Msg {
    Catalog(Vec<Component>),
    Resolved(String, Vec<Package>, Option<packages::Details>),
    Screenshot(String, Option<Pixmap>),
    Progress(Progress),
    Done(String, Result<(), packages::Error>),
}

/// Página de um app aberta.
struct Page {
    package: String,
    available: Option<Package>,
    installed: Option<Package>,
    details: Option<packages::Details>,
    resolved: bool,
}

struct Store {
    section: usize,
    query: String,
    catalog: Vec<Component>,
    loaded: bool,
    installed: HashMap<String, String>,
    icons: HashMap<String, Option<Pixmap>>,
    shots: HashMap<String, Option<Pixmap>>,
    page: Option<Page>,
    busy: Option<(String, Option<Progress>, bool)>,
    message: Option<String>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    waker: Option<mide::kit::Waker>,
    started: bool,
}

impl Store {
    fn new() -> Self {
        let (tx, rx) = channel();
        Self {
            section: 0,
            query: String::new(),
            catalog: Vec::new(),
            loaded: false,
            installed: installed_packages(),
            icons: HashMap::new(),
            shots: HashMap::new(),
            page: None,
            busy: None,
            message: None,
            tx,
            rx,
            waker: None,
            started: false,
        }
    }

    /// Manda `msg` de uma thread e acorda a janela.
    fn sender(&self) -> impl Fn(Msg) + Send + Clone + 'static {
        let (tx, waker) = (self.tx.clone(), self.waker.clone());
        move |msg| {
            let _ = tx.send(msg);
            if let Some(w) = &waker {
                w.wake();
            }
        }
    }

    fn start(&mut self, ui: &Ui) {
        if self.started {
            return;
        }
        self.started = true;
        self.waker = ui.waker();
        let send = self.sender();
        std::thread::spawn(move || {
            let lang = std::env::var("LANG").unwrap_or_else(|_| "C".into());
            let mut apps = mide::appstream::load(&lang);
            apps.sort_by_key(|a| mide::apps::fold(&a.name));
            send(Msg::Catalog(apps));
        });
    }

    fn receive(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Catalog(apps) => {
                    tracing::info!("loja: {} apps no catálogo", apps.len());
                    self.catalog = apps;
                    self.loaded = true;
                }
                Msg::Resolved(name, list, details) => {
                    if let Some(page) = self.page.as_mut().filter(|p| p.package == name) {
                        page.installed = list.iter().find(|p| p.info == packages::Info::Installed).cloned();
                        page.available = list.iter().find(|p| p.info != packages::Info::Installed).cloned().or_else(|| page.installed.clone());
                        page.details = details;
                        page.resolved = true;
                    }
                }
                Msg::Screenshot(url, pix) => {
                    self.shots.insert(url, pix);
                }
                Msg::Progress(p) => {
                    if let Some((_, cur, _)) = self.busy.as_mut() {
                        *cur = Some(p);
                    }
                }
                Msg::Done(name, result) => {
                    let removing = self.busy.as_ref().is_some_and(|(_, _, r)| *r);
                    self.busy = None;
                    match result {
                        Ok(()) => tracing::info!("loja: {name} {}", if removing { "removido" } else { "instalado" }),
                        Err(packages::Error::NotAuthorized | packages::Error::Cancelled) => {}
                        Err(err) => self.message = Some(tr!("Couldn’t finish: {err}", err = err)),
                    }
                    self.installed = installed_packages();
                    if self.page.as_ref().is_some_and(|p| p.package == name) {
                        self.open_page(name);
                    }
                }
            }
        }
    }

    /// Abre a página de um app: pacote (instalado e disponível) e detalhes numa thread.
    fn open_page(&mut self, package: String) {
        tracing::info!("loja: página de {package}");
        self.page = Some(Page { package: package.clone(), available: None, installed: None, details: None, resolved: false });
        let send = self.sender();
        std::thread::spawn(move || {
            let list = packages::resolve(packages::FILTER_ARCH, std::slice::from_ref(&package)).unwrap_or_default();
            let details = list.iter().find(|p| p.info != packages::Info::Installed).or(list.first()).and_then(|p| packages::details(std::slice::from_ref(&p.id)).ok()).and_then(|d| d.into_iter().next());
            send(Msg::Resolved(package, list, details));
        });
        // A primeira captura de tela, baixada para o cache.
        let shot = self.page.as_ref().and_then(|p| self.component(&p.package)).and_then(|c| c.screenshots.first().cloned());
        if let Some(url) = shot.filter(|u| !self.shots.contains_key(u)) {
            self.shots.insert(url.clone(), None);
            let send = self.sender();
            std::thread::spawn(move || {
                let pix = download_image(&url);
                send(Msg::Screenshot(url, pix));
            });
        }
    }

    fn component(&self, package: &str) -> Option<&Component> {
        self.catalog.iter().find(|c| c.package == package)
    }

    /// Ícone de um app (o do catálogo, lido uma vez).
    fn icon(&mut self, c: &Component) -> Option<&Pixmap> {
        if !self.icons.contains_key(&c.package) {
            let pix = c.icon.as_ref().and_then(|p| image::open(p).ok()).map(|img| to_pixmap(&img.to_rgba8()));
            self.icons.insert(c.package.clone(), pix);
        }
        self.icons.get(&c.package).and_then(Option::as_ref)
    }

    /// Apps da seção atual (ou da busca).
    fn listed(&self) -> Vec<Component> {
        let query = mide::apps::fold(self.query.trim());
        if !query.is_empty() {
            return self
                .catalog
                .iter()
                .filter(|c| {
                    mide::apps::fold(&c.name).contains(&query) || mide::apps::fold(&c.summary).contains(&query) || c.package.contains(&query) || c.keywords.iter().any(|k| mide::apps::fold(k).contains(&query))
                })
                .take(200)
                .cloned()
                .collect();
        }
        match self.section {
            0 => FEATURED.iter().filter_map(|p| self.component(p).cloned()).collect(),
            INSTALLED => self.catalog.iter().filter(|c| self.installed.contains_key(&c.package)).cloned().collect(),
            s => {
                let cats = SECTIONS[s].2;
                self.catalog.iter().filter(|c| c.categories.iter().any(|x| cats.contains(&x.as_str()))).take(300).cloned().collect()
            }
        }
    }

    fn install(&mut self, pkg: Package) {
        let name = pkg.name().to_string();
        self.busy = Some((name.clone(), None, false));
        let send = self.sender();
        std::thread::spawn(move || {
            let progress = send.clone();
            let result = packages::install(std::slice::from_ref(&pkg.id), move |p| progress(Msg::Progress(p)));
            send(Msg::Done(name, result));
        });
    }

    fn remove(&mut self, pkg: Package) {
        let name = pkg.name().to_string();
        self.busy = Some((name.clone(), None, true));
        let send = self.sender();
        std::thread::spawn(move || {
            let progress = send.clone();
            let result = packages::remove(std::slice::from_ref(&pkg.id), move |p| progress(Msg::Progress(p)));
            send(Msg::Done(name, result));
        });
    }

    /// Abre o app instalado (pelo `.desktop` do catálogo).
    fn launch(&mut self, c: &Component) {
        let wanted = c.desktop_id.clone().unwrap_or_default();
        let app = mide::apps::load().into_iter().find(|a| format!("{}.desktop", a.id) == wanted || a.id == wanted.trim_end_matches(".desktop"));
        match app {
            Some(app) => {
                if let Err(err) = mide::apps::launch(&app) {
                    self.message = Some(tr!("Couldn’t open: {err}", err = err));
                }
            }
            None => self.message = Some(tr!("This app has no window to open.").into_owned()),
        }
    }

    fn grid(&mut self, ui: &mut Ui) {
        if !self.loaded {
            ui.progress_bar(None);
            ui.note(&tr!("Loading the catalog…"));
            return;
        }
        let list = self.listed();
        if list.is_empty() {
            ui.note(&if self.catalog.is_empty() { tr!("No catalog found. Install the appstream package and update the package list.") } else { tr!("Nothing here.") });
            return;
        }
        let mut open = None;
        ui.wrap(|ui| {
            for c in &list {
                if ui.tile_hidden(64) {
                    ui.tile_skip(64);
                    continue;
                }
                let installed = self.installed.contains_key(&c.package);
                let label = if installed { format!("{} ✓", c.name) } else { c.name.clone() };
                let r = match self.icon(c).cloned() {
                    Some(pix) => ui.tile(Tile::Image(&pix), &label, false, 64),
                    None => ui.tile(Tile::Icon(c.stock_icon.as_deref().unwrap_or("application-x-executable")), &label, false, 64),
                };
                ui.tooltip(&r, &c.summary);
                if r.clicked() {
                    open = Some(c.package.clone());
                }
            }
        });
        if let Some(p) = open {
            self.open_page(p);
        }
    }

    fn page_view(&mut self, ui: &mut Ui) {
        let Some(name) = self.page.as_ref().map(|p| p.package.clone()) else { return };
        let Some(c) = self.component(&name).cloned() else {
            self.page = None;
            return;
        };
        if ui.button(&tr!("‹ Back")).clicked() || ui.input().key(mide::kit::Key::Escape) {
            self.page = None;
            return;
        }
        let icon = self.icon(&c).cloned();
        let installed = self.installed.contains_key(&c.package);
        let (available, installed_pkg, details, resolved) = {
            let p = self.page.as_ref().unwrap();
            (p.available.clone(), p.installed.clone(), p.details.clone(), p.resolved)
        };
        let busy_here = self.busy.as_ref().filter(|(n, _, _)| *n == c.package).map(|(_, p, r)| (*p, *r));
        let mut action: Option<&str> = None;
        ui.card(|ui| {
            ui.columns(
                140.0,
                16.0,
                |ui| match &icon {
                    Some(pix) => {
                        ui.tile(Tile::Image(pix), "", false, 96);
                    }
                    None => ui.icon(c.stock_icon.as_deref().unwrap_or("application-x-executable"), 96),
                },
                |ui| {
                    ui.title(&c.name);
                    ui.note(&c.summary);
                    if !c.developer.is_empty() {
                        ui.note(&c.developer);
                    }
                    ui.row(|ui| match busy_here {
                        Some((p, removing)) => {
                            ui.sized(180.0, |ui| ui.progress_bar(p.and_then(|p| p.percent).map(|v| v as f32 / 100.0)));
                            ui.note(&if removing { tr!("Removing…") } else { tr!("Installing…") });
                        }
                        None if !resolved => ui.note(&tr!("Checking…")),
                        None if installed => {
                            if ui.primary_button(&tr!("Open")).clicked() {
                                action = Some("open");
                            }
                            if ui.button(&tr!("Remove")).clicked() {
                                action = Some("remove");
                            }
                        }
                        None => {
                            if available.is_some() && ui.primary_button(&tr!("Get")).clicked() {
                                action = Some("install");
                            }
                        }
                    });
                },
            );
        });
        match action {
            Some("install") => {
                if let Some(p) = available.clone() {
                    self.install(p);
                }
            }
            Some("remove") => {
                if let Some(p) = installed_pkg.clone() {
                    self.remove(p);
                }
            }
            Some("open") => self.launch(&c),
            _ => {}
        }
        // Captura de tela.
        if let Some(Some(shot)) = c.screenshots.first().and_then(|u| self.shots.get(u)).cloned() {
            let w = ui.available_width().min(720.0);
            let h = w * shot.height() as f32 / shot.width().max(1) as f32;
            let r = ui.region();
            let top = r.1 + r.3 - ui.remaining_height();
            let k = mide::ui::draw::scale();
            ui.paint(move |canvas, _| {
                let s = w * k / shot.width() as f32;
                let tf = tiny_skia::Transform::from_scale(s, s).post_translate(r.0 * k, top * k);
                canvas.draw_pixmap(0, 0, shot.as_ref(), &tiny_skia::PixmapPaint { quality: tiny_skia::FilterQuality::Bilinear, ..Default::default() }, tf, None);
            });
            ui.space(h + 12.0);
        }
        if !c.description.is_empty() {
            ui.card(|ui| {
                for para in c.description.split("\n\n") {
                    ui.label(para);
                }
            });
        }
        ui.card(|ui| {
            let version = available.as_ref().map(|p| p.version().to_string()).unwrap_or_else(|| "—".into());
            ui.key_value(&tr!("Version"), &version, 140.0);
            if let Some(v) = self.installed.get(&c.package) {
                ui.key_value(&tr!("Installed"), v, 140.0);
            }
            if let Some(d) = &details {
                if d.size > 0 {
                    ui.key_value(&tr!("Size"), &mide::processes::bytes_text(d.size as f64), 140.0);
                }
                if !d.license.is_empty() || !c.license.is_empty() {
                    ui.key_value(&tr!("License"), if c.license.is_empty() { &d.license } else { &c.license }, 140.0);
                }
            }
            ui.key_value(&tr!("Package"), &c.package, 140.0);
            if !c.homepage.is_empty() {
                ui.key_value(&tr!("Website"), &c.homepage, 140.0);
            }
        });
    }
}

/// Baixa uma imagem (captura de tela) para o cache e a lê.
fn download_image(url: &str) -> Option<Pixmap> {
    let cache = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).unwrap_or_else(|| mide::places::home_dir().join(".cache")).join("mide/store");
    std::fs::create_dir_all(&cache).ok()?;
    let name: String = url.chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' { c } else { '_' }).collect();
    let file = cache.join(name);
    if !file.is_file() {
        let ok = std::process::Command::new("curl").args(["-sfL", "--max-time", "20", "-o"]).arg(&file).arg(url).status().is_ok_and(|s| s.success());
        if !ok {
            let _ = std::fs::remove_file(&file);
            return None;
        }
    }
    let img = image::open(&file).ok()?.to_rgba8();
    Some(to_pixmap(&img))
}

/// Imagem RGBA para pixmap (premultiplicado).
fn to_pixmap(img: &image::RgbaImage) -> Pixmap {
    let mut pixmap = Pixmap::new(img.width().max(1), img.height().max(1)).unwrap_or_else(|| Pixmap::new(1, 1).unwrap());
    for (dst, src) in pixmap.data_mut().as_chunks_mut::<4>().0.iter_mut().zip(img.as_raw().as_chunks::<4>().0) {
        let a = src[3] as u32;
        dst[0] = (src[0] as u32 * a / 255) as u8;
        dst[1] = (src[1] as u32 * a / 255) as u8;
        dst[2] = (src[2] as u32 * a / 255) as u8;
        dst[3] = src[3];
    }
    pixmap
}

fn section_name(i: usize) -> String {
    // Contexto próprio: "Play" aqui é jogar (não tocar música), "Work" é trabalhar.
    use mide::i18n::trc;
    match i {
        0 => trc("store", "Discover"),
        1 => trc("store", "Create"),
        2 => trc("store", "Work"),
        3 => trc("store", "Play"),
        4 => trc("store", "Develop"),
        5 => trc("store", "Internet"),
        6 => trc("store", "Music and Video"),
        7 => trc("store", "Utilities"),
        8 => trc("store", "Education and Science"),
        _ => trc("store", "Installed"),
    }
    .into_owned()
}

impl App for Store {
    fn view(&mut self, ui: &mut Ui) {
        self.start(ui);
        self.receive();
        for action in ui.input().menu.clone() {
            match action.as_str() {
                "quit" => ui.close(),
                "updates" => {
                    let _ = std::process::Command::new("mide-updates").spawn();
                }
                _ => {}
            }
        }
        let names: Vec<String> = (0..SECTIONS.len()).map(section_name).collect();
        let items: Vec<(&str, &str, Option<[u8; 3]>)> = names.iter().zip(SECTIONS).map(|(n, s)| (n.as_str(), s.1, None)).collect();
        let current = self.section;
        let searching = !self.query.trim().is_empty();
        let mut query = std::mem::take(&mut self.query);
        let chosen = ui.sidebar_search(&items, if searching { usize::MAX } else { current }, &mut query, &tr!("Search"), |ui, _| {
            if self.page.is_some() {
                self.page_view(ui);
            } else {
                ui.title(&if searching { tr!("Results") .into_owned() } else { section_name(current) });
                self.grid(ui);
            }
        });
        if !query.trim().is_empty() && query != self.query && self.page.is_some() && !searching {
            self.page = None;
        }
        self.query = query;
        if chosen != current && chosen < SECTIONS.len() {
            self.section = chosen;
            self.query.clear();
            self.page = None;
        }
        if let Some(message) = self.message.clone()
            && ui.alert(&message, "", &[&tr!("OK")]).is_some()
        {
            self.message = None;
        }
        if let Some((name, _, _)) = &self.busy {
            ui.status_bar(&tr!("Working on {name}…", name = name));
        }
    }

    fn menu_bar(&mut self) -> Vec<MenuGroup> {
        vec![
            MenuGroup {
                title: tr!("Store").into_owned(),
                items: vec![
                    MenuItem::action("updates", &tr!("Software Update…")),
                    MenuItem::Separator,
                    MenuItem::action("quit", &tr!("Quit Store")).shortcut("Ctrl+Q"),
                ],
            },
            window_menu(Vec::new()),
        ]
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _logging = mide::logging::init("mide-store", "mide_store=info,mide=info");
    mide::i18n::init();
    let window = Window::new(&tr!("Store")).app_id("mide-store").size(1100, 720).min_size(760, 520).full_size_content();
    run(window, Store::new())
}
