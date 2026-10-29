//! Onde está o menu de uma janela X11 (menu global dos apps pelo `xwayland-satellite`).
//!
//! No mide, um app X11 chega como janela Wayland do satellite: o processo da janela é o
//! do satellite e o `app_id` vem do `WM_CLASS`. O menu, porém, o app publica do jeito
//! do X11, ligado à janela X:
//!
//! - registrando a janela X (`RegisterWindow(<número da janela X>, objeto)`) no
//!   `com.canonical.AppMenu.Registrar` (Qt, Electron, Chrome, Firefox): o número da
//!   janela acha o registro certo mesmo com várias janelas no mesmo processo;
//! - em propriedades da janela X: `_GTK_UNIQUE_BUS_NAME` e `_GTK_MENUBAR_OBJECT_PATH`
//!   (`org.gtk.Menus`, como o GTK 3 e o LibreOffice fazem quando há um registrar no
//!   barramento; nesse caso o app tira a barra de menus de dentro da janela e só publica
//!   aqui) ou `_KDE_NET_WM_APPMENU_SERVICE_NAME`/`_KDE_NET_WM_APPMENU_OBJECT_PATH`
//!   (`dbusmenu`, Qt com a integração do KDE).
//!
//! A barra lê essas propriedades direto no display do satellite (o compositor não tem
//! código X11): acha a janela X com o mesmo título (e o `WM_CLASS` do `app_id`).

use x11rb::{
    connection::Connection,
    protocol::xproto::{Atom, AtomEnum, ConnectionExt, Window},
    rust_connection::RustConnection,
};

use crate::gmenu::GtkMenu;

/// O que uma janela X11 diz do próprio menu.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct XWindowMenu {
    /// Número da janela X (o mesmo do `RegisterWindow`).
    pub window: u32,
    /// Processo do app (`_NET_WM_PID`), 0 se ele não disse.
    pub pid: u32,
    /// Menu do GTK (`org.gtk.Menus`) pelas propriedades `_GTK_*`.
    pub gtk: Option<GtkMenu>,
    /// Menu `dbusmenu` pelas propriedades `_KDE_NET_WM_APPMENU_*`: serviço e objeto.
    pub kde: Option<(String, String)>,
}

/// O processo é o `xwayland-satellite` (a janela é de um app X11)?
pub fn is_xwayland_pid(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    let exe = std::fs::read_link(format!("/proc/{pid}/exe")).ok().and_then(|e| e.file_name().map(|n| n.to_string_lossy().into_owned()));
    // O `comm` corta em 15 letras ("xwayland-satell").
    let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).unwrap_or_default();
    exe.is_some_and(|e| e.starts_with("xwayland-satellite")) || comm.trim().starts_with("xwayland-sat")
}

/// Display X dos apps deste mide: o arquivo que o compositor grava ao lado do socket
/// (`$XDG_RUNTIME_DIR/<socket>.xdisplay`). Não usa o `DISPLAY` herdado: rodando o mide
/// numa janela, ele seria o do desktop de baixo.
pub fn display() -> Option<String> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")?;
    let socket = std::env::var("WAYLAND_DISPLAY").ok().or_else(|| std::env::var("MIDE_SOCKET").ok())?;
    let text = std::fs::read_to_string(std::path::Path::new(&runtime).join(format!("{socket}.xdisplay"))).ok()?;
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// Menu da janela X11 com este título (e `WM_CLASS` que bate com o `app_id`). `None`
/// sem display X ou sem janela com o título.
pub fn find(title: &str, app_id: &str) -> Option<XWindowMenu> {
    let display = display()?;
    let (conn, screen) = x11rb::connect(Some(&display)).ok()?;
    let root = conn.setup().roots.get(screen)?.root;
    let atoms = Atoms::new(&conn)?;
    let app = app_id.to_lowercase();
    let mut best: Option<(u8, XWindowMenu)> = None;
    for window in candidates(&conn, root, &atoms) {
        if window_title(&conn, window, &atoms).as_deref() != Some(title) {
            continue;
        }
        let class_ok = app.is_empty() || wm_class(&conn, window).iter().any(|c| c.to_lowercase() == app);
        let menu = read_window(&conn, window, &atoms);
        // Preferência: mesmo WM_CLASS e com menu; depois só um dos dois.
        let score = u8::from(class_ok) * 2 + u8::from(menu.gtk.is_some() || menu.kde.is_some());
        if best.as_ref().is_none_or(|(s, _)| score > *s) {
            best = Some((score, menu));
        }
    }
    best.map(|(_, m)| m)
}

struct Atoms {
    client_list: Atom,
    net_wm_name: Atom,
    net_wm_pid: Atom,
    gtk_bus: Atom,
    gtk_menubar: Atom,
    gtk_app: Atom,
    gtk_window: Atom,
    kde_service: Atom,
    kde_path: Atom,
}

impl Atoms {
    fn new(conn: &RustConnection) -> Option<Atoms> {
        let names = [
            "_NET_CLIENT_LIST",
            "_NET_WM_NAME",
            "_NET_WM_PID",
            "_GTK_UNIQUE_BUS_NAME",
            "_GTK_MENUBAR_OBJECT_PATH",
            "_GTK_APPLICATION_OBJECT_PATH",
            "_GTK_WINDOW_OBJECT_PATH",
            "_KDE_NET_WM_APPMENU_SERVICE_NAME",
            "_KDE_NET_WM_APPMENU_OBJECT_PATH",
        ];
        // Pede todos e só depois espera as respostas (uma ida ao servidor X).
        let cookies: Vec<_> = names.iter().map(|n| conn.intern_atom(false, n.as_bytes()).ok()).collect();
        let mut atoms = Vec::new();
        for cookie in cookies {
            atoms.push(cookie?.reply().ok()?.atom);
        }
        Some(Atoms {
            client_list: atoms[0],
            net_wm_name: atoms[1],
            net_wm_pid: atoms[2],
            gtk_bus: atoms[3],
            gtk_menubar: atoms[4],
            gtk_app: atoms[5],
            gtk_window: atoms[6],
            kde_service: atoms[7],
            kde_path: atoms[8],
        })
    }
}

/// Janelas de app: a lista do gerenciador (`_NET_CLIENT_LIST`) e, junto, as filhas da
/// raiz e as netas (gerenciadores que põem a janela do app dentro de uma moldura).
fn candidates(conn: &RustConnection, root: Window, atoms: &Atoms) -> Vec<Window> {
    let mut out: Vec<Window> = Vec::new();
    if let Some(list) = conn
        .get_property(false, root, atoms.client_list, AtomEnum::WINDOW, 0, 4096)
        .ok()
        .and_then(|c| c.reply().ok())
        .and_then(|r| r.value32().map(|v| v.collect::<Vec<u32>>()))
    {
        out.extend(list);
    }
    if let Some(tree) = conn.query_tree(root).ok().and_then(|c| c.reply().ok()) {
        for child in tree.children {
            if !out.contains(&child) {
                out.push(child);
            }
            if let Some(sub) = conn.query_tree(child).ok().and_then(|c| c.reply().ok()) {
                out.extend(sub.children.into_iter().filter(|w| *w != child));
            }
        }
    }
    out.dedup();
    out
}

fn text_property(conn: &RustConnection, window: Window, atom: Atom) -> Option<String> {
    let reply = conn.get_property(false, window, atom, AtomEnum::ANY, 0, 1024).ok()?.reply().ok()?;
    if reply.format != 8 || reply.value.is_empty() {
        return None;
    }
    Some(String::from_utf8_lossy(&reply.value).trim_end_matches('\0').to_string())
}

fn window_title(conn: &RustConnection, window: Window, atoms: &Atoms) -> Option<String> {
    text_property(conn, window, atoms.net_wm_name).or_else(|| text_property(conn, window, AtomEnum::WM_NAME.into()))
}

/// As duas partes do `WM_CLASS` (instância e classe).
fn wm_class(conn: &RustConnection, window: Window) -> Vec<String> {
    let Some(text) = text_property(conn, window, AtomEnum::WM_CLASS.into()) else {
        return Vec::new();
    };
    text.split('\0').filter(|s| !s.is_empty()).map(str::to_string).collect()
}

fn read_window(conn: &RustConnection, window: Window, atoms: &Atoms) -> XWindowMenu {
    let get = |atom| text_property(conn, window, atom).filter(|s| !s.is_empty());
    let gtk = match (get(atoms.gtk_bus), get(atoms.gtk_menubar)) {
        (Some(bus), Some(menubar)) => Some(GtkMenu {
            bus,
            menubar,
            app: get(atoms.gtk_app).unwrap_or_default(),
            window: get(atoms.gtk_window).unwrap_or_default(),
        }),
        _ => None,
    };
    let kde = match (get(atoms.kde_service), get(atoms.kde_path)) {
        (Some(service), Some(path)) => Some((service, path)),
        _ => None,
    };
    let pid = conn
        .get_property(false, window, atoms.net_wm_pid, AtomEnum::CARDINAL, 0, 1)
        .ok()
        .and_then(|c| c.reply().ok())
        .and_then(|r| r.value32().and_then(|mut v| v.next()))
        .unwrap_or(0);
    XWindowMenu { window, pid, gtk, kde }
}
