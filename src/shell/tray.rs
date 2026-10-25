//! Ícones de apps na barra (tray, B3): o protocolo `StatusNotifierItem` do D-Bus, que
//! Discord, Steam, Nextcloud, Telegram, o applet de rede e outros usam.
//!
//! A barra é o registro (`org.kde.StatusNotifierWatcher`): cada app avisa que tem um
//! ícone; a barra lê dele o ícone (pelo nome no tema ou a imagem que ele manda), o
//! título, o estado e o menu (`com.canonical.dbusmenu`, o mesmo formato do menu
//! global). Clicar ativa o app (`Activate`); o clique direito abre o menu dele.
//! Ícones "passivos" (o app pediu para sumir) não aparecem.

use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
};

use zbus::{object_server::SignalEmitter, zvariant::OwnedValue};

/// Um ícone registrado.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TrayItem {
    /// Nome no barramento (o registrado: único ou conhecido) e o objeto do ícone.
    pub bus: String,
    pub path: String,
    /// Dono único do nome (os sinais do app vêm dele).
    pub owner: String,
    pub id: String,
    pub title: String,
    /// `Active`, `Passive` (escondido) ou `NeedsAttention`.
    pub status: String,
    pub icon_name: String,
    /// Pasta extra de ícones do app.
    pub theme_path: String,
    /// Imagem mandada pelo app: largura, altura e pixels ARGB32 (big-endian).
    pub pixmap: Option<(i32, i32, Vec<u8>)>,
    /// Objeto do menu (`dbusmenu`), se houver.
    pub menu: Option<String>,
    /// O app só tem menu: o clique esquerdo também abre o menu.
    pub item_is_menu: bool,
}

impl TrayItem {
    fn key(&self) -> String {
        format!("{}{}", self.bus, self.path)
    }

    /// Aparece na barra (o app não pediu para esconder).
    pub fn visible(&self) -> bool {
        self.status != "Passive"
    }
}

/// Registro dos ícones, compartilhado com a barra. `version` muda a cada mudança (a
/// barra redesenha quando ela muda).
#[derive(Clone)]
pub struct Tray {
    items: Arc<Mutex<Vec<TrayItem>>>,
    version: Arc<AtomicU64>,
    conn: zbus::blocking::Connection,
}

impl Tray {
    pub fn items(&self) -> Vec<TrayItem> {
        self.items.lock().map(|i| i.iter().filter(|i| i.visible()).cloned().collect()).unwrap_or_default()
    }

    pub fn version(&self) -> u64 {
        self.version.load(Ordering::Relaxed)
    }

    /// Clique esquerdo: o app abre a janela dele (ou o que ele quiser).
    pub fn activate(&self, item: &TrayItem, x: i32, y: i32) {
        let (conn, bus, path) = (self.conn.clone(), item.bus.clone(), item.path.clone());
        std::thread::spawn(move || {
            let Ok(proxy) = zbus::blocking::Proxy::new(&conn, bus, path, "org.kde.StatusNotifierItem") else {
                return;
            };
            if let Err(err) = proxy.call::<_, _, ()>("Activate", &(x, y)) {
                tracing::debug!("tray: Activate falhou: {err}");
            }
        });
    }

    /// Itens do menu do ícone (lidos na hora: o app pode ter mudado o menu).
    pub fn menu(&self, item: &TrayItem) -> Option<Vec<crate::appmenu::Item>> {
        crate::appmenu::layout(&self.conn, &item.bus, item.menu.as_ref()?, 0)
    }

    /// Clique num item do menu do ícone.
    pub fn click(&self, item: &TrayItem, id: i32) {
        if let Some(menu) = &item.menu {
            crate::appmenu::click(&self.conn, &item.bus, menu, id);
        }
    }

    fn changed(&self) {
        self.version.fetch_add(1, Ordering::Relaxed);
    }

    /// Lê as propriedades do ícone e guarda (troca o que já havia com a mesma chave).
    fn refresh(&self, bus: &str, path: &str) {
        let Some(item) = read_item(&self.conn, bus, path) else {
            return;
        };
        if let Ok(mut items) = self.items.lock() {
            let key = item.key();
            match items.iter_mut().find(|i| i.key() == key) {
                Some(old) if *old == item => return,
                Some(old) => *old = item,
                None => {
                    tracing::info!("tray: ícone de {} ({})", if item.title.is_empty() { &item.id } else { &item.title }, item.bus);
                    items.push(item);
                }
            }
        }
        self.changed();
    }

    /// O app saiu do barramento: os ícones dele somem.
    fn forget_owner(&self, owner: &str) {
        let removed = self.items.lock().map(|mut items| {
            let before = items.len();
            items.retain(|i| i.owner != owner && i.bus != owner);
            before != items.len()
        });
        if removed.unwrap_or(false) {
            tracing::info!("tray: ícone de {owner} saiu");
            self.changed();
        }
    }
}

/// Propriedades de um ícone (`org.kde.StatusNotifierItem`).
fn read_item(conn: &zbus::blocking::Connection, bus: &str, path: &str) -> Option<TrayItem> {
    let props = zbus::blocking::fdo::PropertiesProxy::builder(conn).destination(bus.to_string()).ok()?.path(path.to_string()).ok()?.build().ok()?;
    let interface = zbus::names::InterfaceName::try_from("org.kde.StatusNotifierItem").ok()?;
    let all: HashMap<String, OwnedValue> = props.get_all(interface.into()).ok()?;
    let text = |key: &str| all.get(key).and_then(|v| <&str>::try_from(&**v).ok().map(str::to_string)).unwrap_or_default();
    let menu = all.get("Menu").and_then(|v| zbus::zvariant::ObjectPath::try_from(&**v).ok().map(|p| p.to_string())).filter(|p| p != "/" && !p.is_empty());
    let item_is_menu = all.get("ItemIsMenu").and_then(|v| bool::try_from(&**v).ok()).unwrap_or(false);
    // A imagem do app: entre as do tamanho mais próximo de 44 px (22 em tela 2x).
    let pixmap = all.get("IconPixmap").and_then(|v| {
        let images: Vec<(i32, i32, Vec<u8>)> = v.try_clone().ok().and_then(|v| Vec::<(i32, i32, Vec<u8>)>::try_from(v).ok())?;
        images.into_iter().filter(|(w, h, d)| *w > 0 && *h > 0 && d.len() >= (*w * *h * 4) as usize).min_by_key(|(w, _, _)| (w - 44).abs())
    });
    let owner = zbus::blocking::fdo::DBusProxy::new(conn)
        .ok()
        .and_then(|d| d.get_name_owner(zbus::names::BusName::try_from(bus).ok()?).ok())
        .map(|o| o.to_string())
        .unwrap_or_else(|| bus.to_string());
    Some(TrayItem {
        bus: bus.to_string(),
        path: path.to_string(),
        owner,
        id: text("Id"),
        title: text("Title"),
        status: text("Status"),
        icon_name: text("IconName"),
        theme_path: text("IconThemePath"),
        pixmap,
        menu,
        item_is_menu,
    })
}

struct Watcher {
    tray: Tray,
}

#[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
impl Watcher {
    async fn register_status_notifier_item(
        &self,
        service: &str,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) {
        let sender = header.sender().map(|s| s.to_string()).unwrap_or_default();
        // Uns mandam o nome no barramento (KDE), outros só o objeto (Ayatana).
        let (bus, path) = if service.starts_with('/') { (sender, service.to_string()) } else { (service.to_string(), "/StatusNotifierItem".to_string()) };
        let tray = self.tray.clone();
        let (b, p) = (bus.clone(), path.clone());
        // Ler as propriedades é uma chamada ao app: fora do atendimento do registro.
        std::thread::spawn(move || tray.refresh(&b, &p));
        let _ = Self::status_notifier_item_registered(&emitter, &format!("{bus}{path}")).await;
    }

    async fn register_status_notifier_host(&self, _service: &str) {}

    #[zbus(property)]
    async fn registered_status_notifier_items(&self) -> Vec<String> {
        self.tray.items.lock().map(|i| i.iter().map(TrayItem::key).collect()).unwrap_or_default()
    }

    #[zbus(property)]
    async fn is_status_notifier_host_registered(&self) -> bool {
        true
    }

    #[zbus(property)]
    async fn protocol_version(&self) -> i32 {
        0
    }

    #[zbus(signal)]
    async fn status_notifier_item_registered(emitter: &SignalEmitter<'_>, service: &str) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn status_notifier_item_unregistered(emitter: &SignalEmitter<'_>, service: &str) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn status_notifier_host_registered(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

/// Começa o registro dos ícones no barramento da sessão. Sem barramento, ou com outro
/// registro já atendendo (outro painel), a barra fica sem tray.
pub fn start() -> Option<Tray> {
    let conn = zbus::blocking::connection::Builder::session().ok()?.method_timeout(std::time::Duration::from_millis(500)).build().ok()?;
    let tray = Tray { items: Arc::default(), version: Arc::default(), conn: conn.clone() };
    conn.object_server().at("/StatusNotifierWatcher", Watcher { tray: tray.clone() }).ok()?;
    if let Err(err) = conn.request_name("org.kde.StatusNotifierWatcher") {
        tracing::info!("tray: registro não iniciado ({err})");
        return None;
    }
    // Ícones que mudam (NewIcon, NewStatus...) e apps que saem do barramento.
    let watch = tray.clone();
    std::thread::spawn(move || {
        let rule = zbus::MatchRule::builder().msg_type(zbus::message::Type::Signal).build();
        let Ok(messages) = zbus::blocking::MessageIterator::for_match_rule(rule, &watch.conn, Some(64)) else {
            return;
        };
        for message in messages.flatten() {
            let header = message.header();
            let (Some(interface), Some(member)) = (header.interface(), header.member()) else {
                continue;
            };
            match (interface.as_str(), member.as_str()) {
                ("org.kde.StatusNotifierItem", _) => {
                    let sender = header.sender().map(|s| s.to_string()).unwrap_or_default();
                    let path = header.path().map(|p| p.to_string()).unwrap_or_default();
                    let known: Vec<(String, String)> =
                        watch.items.lock().map(|i| i.iter().filter(|i| i.owner == sender && i.path == path).map(|i| (i.bus.clone(), i.path.clone())).collect()).unwrap_or_default();
                    for (bus, path) in known {
                        watch.refresh(&bus, &path);
                    }
                }
                ("org.freedesktop.DBus", "NameOwnerChanged") => {
                    if let Ok((name, old, new)) = message.body().deserialize::<(String, String, String)>() {
                        if new.is_empty() && !old.is_empty() {
                            watch.forget_owner(&old);
                            watch.forget_owner(&name);
                        }
                    }
                }
                _ => {}
            }
        }
    });
    tracing::info!("tray: registro pronto");
    Some(tray)
}
