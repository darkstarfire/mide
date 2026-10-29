//! Onde está o menu de cada janela (menu global dos apps Wayland, B2):
//!
//! - `gtk_shell1`: apps GTK 3 com barra de menus (`GtkApplication`) dizem onde ela está
//!   no D-Bus (`org.gtk.Menus`/`org.gtk.Actions`) com `set_dbus_properties`. Avisado
//!   de que o mide mostra o menu na barra (capacidade `global_menu_bar`), o app tira
//!   a barra de menus de dentro da janela.
//! - `org_kde_kwin_appmenu`: apps Qt com a integração do KDE dizem onde está o menu
//!   `com.canonical.dbusmenu` (o mesmo formato do menu global do X11).
//!
//! A barra pergunta pelo canal de controle (`appmenu`) qual é o menu da janela em foco.
//!
//! Saída de segurança: com "Mostrar os menus dos apps na barra superior" desligado
//! (`[bar] app_menus = false`), o mide não anuncia o menu global (capacidades 0 no
//! `gtk_shell1`, inclusive para os apps já abertos, ao recarregar a configuração) e
//! não passa o menu à barra: os apps mantêm a barra de menus dentro da janela.

use std::sync::Mutex;

use smithay::{
    reexports::wayland_server::{protocol::wl_surface::WlSurface, Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource},
    wayland::compositor::with_states,
};

use crate::state::State;

#[allow(non_upper_case_globals, non_camel_case_types, unused_imports, dead_code, clippy::all)]
pub mod gtk {
    use smithay::reexports::wayland_server;
    use smithay::reexports::wayland_server::protocol::*;
    pub mod __interfaces {
        use smithay::reexports::wayland_server::backend as wayland_backend;
        use smithay::reexports::wayland_server::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("protocols/gtk-shell.xml");
    }
    use self::__interfaces::*;
    wayland_scanner::generate_server_code!("protocols/gtk-shell.xml");
}

#[allow(non_upper_case_globals, non_camel_case_types, unused_imports, dead_code, clippy::all)]
pub mod kde {
    use smithay::reexports::wayland_server;
    use smithay::reexports::wayland_server::protocol::*;
    pub mod __interfaces {
        use smithay::reexports::wayland_server::backend as wayland_backend;
        use smithay::reexports::wayland_server::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("protocols/kde-appmenu.xml");
    }
    use self::__interfaces::*;
    wayland_scanner::generate_server_code!("protocols/kde-appmenu.xml");
}

use gtk::{gtk_shell1::GtkShell1, gtk_surface1::GtkSurface1};
use kde::{org_kde_kwin_appmenu::OrgKdeKwinAppmenu, org_kde_kwin_appmenu_manager::OrgKdeKwinAppmenuManager};

/// Menu de uma janela, guardado na superfície dela.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppMenu {
    /// Menu do GTK: nome único no barramento, objetos da barra de menus, do app e da
    /// janela (as ações `app.` e `win.`).
    Gtk { bus: String, menubar: String, app: String, window: String },
    /// Menu `dbusmenu` (Qt/KDE): serviço e objeto.
    DbusMenu { service: String, path: String },
}

#[derive(Default)]
struct SurfaceMenu(Mutex<Option<AppMenu>>);

fn store(surface: &WlSurface, menu: Option<AppMenu>) {
    with_states(surface, |states| {
        states.data_map.insert_if_missing_threadsafe(SurfaceMenu::default);
        if let Some(slot) = states.data_map.get::<SurfaceMenu>() {
            *slot.0.lock().unwrap_or_else(|e| e.into_inner()) = menu;
        }
    });
}

/// Menu da janela desta superfície, se o app disse onde está.
pub fn menu_of(surface: &WlSurface) -> Option<AppMenu> {
    if !enabled() {
        return None;
    }
    with_states(surface, |states| states.data_map.get::<SurfaceMenu>().and_then(|m| m.0.lock().ok()?.clone()))
}

/// `gtk_shell1` de cada app ligado, para avisar quando a opção muda.
static SHELLS: Mutex<Vec<GtkShell1>> = Mutex::new(Vec::new());

/// Menus dos apps na barra superior ligados (`[bar] app_menus`, ligado por padrão).
pub fn enabled() -> bool {
    mide::settings_file::flag("bar", "app_menus", true)
}

/// Capacidades anunciadas aos apps GTK: o menu do app e a barra de menus ficam na
/// barra superior (o GTK lê como bits) ou nada (o app mostra a barra de menus dele).
fn capabilities() -> u32 {
    if enabled() { 1 | 2 } else { 0 }
}

/// Avisa os apps GTK abertos da opção atual (depois do `reload`). O GTK 3 mostra ou
/// esconde a barra de menus da janela na hora (`gtk-shell-shows-menubar`).
pub fn refresh_capabilities() {
    let caps = capabilities();
    let mut shells = SHELLS.lock().unwrap_or_else(|e| e.into_inner());
    shells.retain(|s| s.is_alive());
    for shell in shells.iter() {
        shell.capabilities(caps);
    }
}

pub fn init(display: &DisplayHandle) {
    display.create_global::<State, GtkShell1, ()>(5, ());
    display.create_global::<State, OrgKdeKwinAppmenuManager, ()>(2, ());
}

impl GlobalDispatch<GtkShell1, ()> for State {
    fn bind(_: &mut State, _: &DisplayHandle, _: &Client, resource: New<GtkShell1>, _: &(), data_init: &mut DataInit<'_, State>) {
        let shell = data_init.init(resource, ());
        // O mide mostra o menu do app e a barra de menus na barra superior (se a opção
        // estiver ligada).
        shell.capabilities(capabilities());
        let mut shells = SHELLS.lock().unwrap_or_else(|e| e.into_inner());
        shells.retain(|s| s.is_alive());
        shells.push(shell);
    }
}

impl Dispatch<GtkShell1, ()> for State {
    fn request(_: &mut State, _: &Client, _: &GtkShell1, request: gtk::gtk_shell1::Request, _: &(), _: &DisplayHandle, data_init: &mut DataInit<'_, State>) {
        if let gtk::gtk_shell1::Request::GetGtkSurface { gtk_surface, surface } = request {
            data_init.init(gtk_surface, surface);
        }
    }
}

impl Dispatch<GtkSurface1, WlSurface> for State {
    fn request(state: &mut State, _: &Client, _: &GtkSurface1, request: gtk::gtk_surface1::Request, surface: &WlSurface, _: &DisplayHandle, _: &mut DataInit<'_, State>) {
        match request {
            gtk::gtk_surface1::Request::SetDbusProperties { menubar_path, window_object_path, application_object_path, unique_bus_name, .. } => {
                let menu = match (unique_bus_name, menubar_path) {
                    (Some(bus), Some(menubar)) if surface.is_alive() => Some(AppMenu::Gtk {
                        bus,
                        menubar,
                        app: application_object_path.unwrap_or_default(),
                        window: window_object_path.unwrap_or_default(),
                    }),
                    _ => None,
                };
                if menu.is_some() {
                    tracing::info!("menu do app (GTK): {menu:?}");
                }
                store(surface, menu);
            }
            // O app pede para vir para a frente (como o xdg-activation).
            gtk::gtk_surface1::Request::Present { .. } | gtk::gtk_surface1::Request::RequestFocus { .. } => {
                let id = state.window_for(surface).map(|w| crate::state::window_id(&w));
                if let Some(id) = id {
                    crate::control::focus_window_id(state, id);
                }
            }
            _ => {}
        }
    }
}

impl GlobalDispatch<OrgKdeKwinAppmenuManager, ()> for State {
    fn bind(_: &mut State, _: &DisplayHandle, _: &Client, resource: New<OrgKdeKwinAppmenuManager>, _: &(), data_init: &mut DataInit<'_, State>) {
        data_init.init(resource, ());
    }
}

impl Dispatch<OrgKdeKwinAppmenuManager, ()> for State {
    fn request(_: &mut State, _: &Client, _: &OrgKdeKwinAppmenuManager, request: kde::org_kde_kwin_appmenu_manager::Request, _: &(), _: &DisplayHandle, data_init: &mut DataInit<'_, State>) {
        if let kde::org_kde_kwin_appmenu_manager::Request::Create { id, surface } = request {
            data_init.init(id, surface);
        }
    }
}

impl Dispatch<OrgKdeKwinAppmenu, WlSurface> for State {
    fn request(_: &mut State, _: &Client, _: &OrgKdeKwinAppmenu, request: kde::org_kde_kwin_appmenu::Request, surface: &WlSurface, _: &DisplayHandle, _: &mut DataInit<'_, State>) {
        match request {
            kde::org_kde_kwin_appmenu::Request::SetAddress { service_name, object_path } => {
                tracing::info!("menu do app (KDE): {service_name} {object_path}");
                store(surface, Some(AppMenu::DbusMenu { service: service_name, path: object_path }));
            }
            kde::org_kde_kwin_appmenu::Request::Release => {
                if surface.is_alive() {
                    store(surface, None);
                }
            }
        }
    }
}
