//! Menus de verdade dos apps na barra superior (menu global, como no macOS).
//!
//! Apps que publicam a barra de menus pelo D-Bus (`com.canonical.dbusmenu`) avisam um
//! "registrar" (`com.canonical.AppMenu.Registrar`) qual objeto tem o menu de cada
//! janela: é o caminho do VS Code, Chrome, Firefox, LibreOffice e apps Qt/GTK com o
//! módulo appmenu, rodando pelo XWayland (no Wayland puro esses apps não publicam).
//!
//! A barra do mide é o registrar: guarda quem registrou (nome no barramento, processo)
//! e, para o app em foco, lê o menu (`GetLayout`) e manda os cliques (`Event`). O app
//! em foco é achado pelo processo que registrou: o nome dele (`/proc/<pid>/comm` ou o
//! executável) bate com o `app_id` da janela (`code`, `google-chrome`...).

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

/// Uma janela registrada.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Registration {
    pub window: u32,
    /// Nome único do app no barramento e o objeto do menu.
    pub sender: String,
    pub path: String,
    /// Nomes do processo (comando e executável), em minúsculas, para achar o app.
    pub names: Vec<String>,
    /// Processo que registrou (0 se não deu para saber).
    pub pid: u32,
}

/// Registros, compartilhados entre o serviço e a barra.
#[derive(Debug, Clone, Default)]
pub struct Registry(Arc<Mutex<Vec<Registration>>>);

impl Registry {
    /// Menu (nome no barramento e objeto) do app com este `app_id`, o registro mais novo.
    pub fn menu_for_app(&self, app_id: &str, pid: u32) -> Option<(String, String)> {
        let app = app_id.to_lowercase();
        if app.is_empty() {
            return None;
        }
        let list = self.0.lock().ok()?;
        // O registro do próprio processo da janela primeiro: com dois processos do mesmo
        // app (o terminal de atalho e o comum, por exemplo), o mais novo pode não ser o
        // da janela em foco.
        if pid > 0
            && let Some(r) = list.iter().rev().find(|r| r.pid == pid)
        {
            return Some((r.sender.clone(), r.path.clone()));
        }
        list.iter()
            .rev()
            .find(|r| r.names.iter().any(|n| matches_app(n, &app)))
            .map(|r| (r.sender.clone(), r.path.clone()))
    }

    fn register(&self, registration: Registration) {
        if let Ok(mut list) = self.0.lock() {
            list.retain(|r| r.window != registration.window);
            list.push(registration);
        }
    }

    fn unregister(&self, window: u32) {
        if let Ok(mut list) = self.0.lock() {
            list.retain(|r| r.window != window);
        }
    }

    /// Esquece os registros de um app que saiu do barramento.
    pub fn forget_sender(&self, sender: &str) {
        if let Ok(mut list) = self.0.lock() {
            list.retain(|r| r.sender != sender);
        }
    }
}

/// O nome do processo bate com o app: igual, ou um começa com o outro separado por
/// "-" ou "." (`google-chrome` e `chrome`, `org.kde.kate` e `kate`).
fn matches_app(name: &str, app: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    name == app
        || app.ends_with(&format!(".{name}"))
        || app.ends_with(&format!("-{name}"))
        || name.ends_with(&format!("-{app}"))
        || app.starts_with(&format!("{name}-"))
}

/// Nomes de um processo: o comando (`comm`) e o executável, em minúsculas.
fn process_names(pid: u32) -> Vec<String> {
    let mut names = Vec::new();
    if let Ok(comm) = std::fs::read_to_string(format!("/proc/{pid}/comm")) {
        names.push(comm.trim().to_lowercase());
    }
    if let Ok(exe) = std::fs::read_link(format!("/proc/{pid}/exe")) {
        if let Some(name) = exe.file_name() {
            names.push(name.to_string_lossy().to_lowercase());
        }
    }
    names.dedup();
    names
}

struct RegistrarService {
    registry: Registry,
}

#[zbus::interface(name = "com.canonical.AppMenu.Registrar")]
impl RegistrarService {
    async fn register_window(
        &self,
        window_id: u32,
        menu_object_path: OwnedObjectPath,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) {
        let Some(sender) = header.sender().map(|s| s.to_string()) else {
            return;
        };
        let pid = zbus::fdo::DBusProxy::new(conn)
            .await
            .ok()
            .map(|proxy| async move { proxy.get_connection_unix_process_id(header_sender(&sender)).await.ok() });
        let pid = match pid {
            Some(fut) => fut.await,
            None => None,
        };
        let names = pid.map(process_names).unwrap_or_default();
        let pid = pid.unwrap_or(0);
        tracing::info!("menu do app registrado: janela {window_id} de {} ({})", sender_short(&header), names.join(", "));
        self.registry.register(Registration {
            window: window_id,
            sender: header.sender().map(|s| s.to_string()).unwrap_or_default(),
            path: menu_object_path.to_string(),
            names,
            pid,
        });
    }

    async fn unregister_window(&self, window_id: u32) {
        self.registry.unregister(window_id);
    }

    async fn get_menu_for_window(&self, window_id: u32) -> zbus::fdo::Result<(String, OwnedObjectPath)> {
        let list = self.registry.0.lock().map_err(|_| zbus::fdo::Error::Failed("registro indisponível".into()))?;
        let r = list.iter().find(|r| r.window == window_id).ok_or_else(|| zbus::fdo::Error::Failed("janela sem menu".into()))?;
        let path = OwnedObjectPath::try_from(r.path.clone()).map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        Ok((r.sender.clone(), path))
    }
}

fn header_sender(sender: &str) -> zbus::names::BusName<'_> {
    zbus::names::BusName::try_from(sender).unwrap_or_else(|_| zbus::names::BusName::try_from("org.freedesktop.DBus").expect("nome fixo válido"))
}

fn sender_short(header: &zbus::message::Header<'_>) -> String {
    header.sender().map(|s| s.to_string()).unwrap_or_default()
}

/// Atende como registrar no barramento da sessão, numa thread. Sem barramento, ou com
/// outro registrar já atendendo, não há menus dos apps (fica o menu padrão do mide).
pub fn start_registrar() -> Option<(Registry, zbus::blocking::Connection)> {
    start_registrar_with(Registry::default())
}

/// A conexão com o barramento ainda responde (uma chamada ao próprio barramento).
pub fn connection_alive(conn: &zbus::blocking::Connection) -> bool {
    zbus::blocking::fdo::DBusProxy::new(conn).and_then(|p| p.get_id().map_err(Into::into)).is_ok()
}

/// Esta conexão ainda é a dona do nome do registrar (outro programa pode ter tomado).
pub fn owns_registrar(conn: &zbus::blocking::Connection) -> bool {
    let Ok(proxy) = zbus::blocking::fdo::DBusProxy::new(conn) else {
        return false;
    };
    let Ok(name) = zbus::names::BusName::try_from("com.canonical.AppMenu.Registrar") else {
        return false;
    };
    match (proxy.get_name_owner(name), conn.unique_name()) {
        (Ok(owner), Some(me)) => owner.as_str() == me.as_str(),
        _ => false,
    }
}

/// O registrar de novo, guardando os registros que já existiam (a conexão antiga caiu).
pub fn start_registrar_with(registry: Registry) -> Option<(Registry, zbus::blocking::Connection)> {
    let service = RegistrarService { registry: registry.clone() };
    // Chamadas aos apps (ler o menu, clicar) desistem em 300 ms: um app travado não
    // trava a barra.
    let conn = zbus::blocking::connection::Builder::session()
        .ok()?
        .method_timeout(std::time::Duration::from_millis(300))
        .name("com.canonical.AppMenu.Registrar")
        .ok()?
        .serve_at("/com/canonical/AppMenu/Registrar", service)
        .ok()?
        .build();
    match conn {
        Ok(conn) => {
            tracing::info!("menus dos apps: registrar pronto");
            Some((registry, conn))
        }
        Err(err) => {
            tracing::info!("menus dos apps: registrar não iniciado ({err})");
            None
        }
    }
}

/// Item do menu de um app.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub id: i32,
    pub label: String,
    pub enabled: bool,
    pub separator: bool,
    /// Marcado (caixa de seleção ou opção).
    pub checked: Option<bool>,
    /// Atalho para mostrar à direita (`Ctrl+S`).
    pub shortcut: String,
    pub children: Vec<Item>,
}

/// Lê o menu (ou o submenu `parent`, com todos os níveis abaixo).
pub fn layout(conn: &zbus::blocking::Connection, sender: &str, path: &str, parent: i32) -> Option<Vec<Item>> {
    let proxy = zbus::blocking::Proxy::new(conn, sender.to_string(), path.to_string(), "com.canonical.dbusmenu").ok()?;
    // O app pode preencher o submenu na hora (é o que ele espera antes de mostrar).
    let _: Result<bool, _> = proxy.call("AboutToShow", &(parent,));
    let (_revision, root): (u32, OwnedValue) = proxy.call("GetLayout", &(parent, -1i32, Vec::<String>::new())).ok()?;
    let root = parse_node(&root)?;
    Some(root.children)
}

/// Avisa o app que o item foi clicado.
pub fn click(conn: &zbus::blocking::Connection, sender: &str, path: &str, id: i32) {
    let Ok(proxy) = zbus::blocking::Proxy::new(conn, sender.to_string(), path.to_string(), "com.canonical.dbusmenu") else {
        return;
    };
    let timestamp = 0u32;
    let data = Value::from(0i32);
    if let Err(err) = proxy.call::<_, _, ()>("Event", &(id, "clicked", data, timestamp)) {
        tracing::warn!("menu do app: clique não chegou: {err}");
    }
}

/// Lê um nó como a barra lê (para os testes de quem publica menus).
#[cfg(test)]
pub(crate) fn parse_for_test(value: &Value<'_>) -> Option<Item> {
    parse_node(value)
}

/// Nó do `GetLayout`: `(id, propriedades, filhos)`.
fn parse_node(value: &Value<'_>) -> Option<Item> {
    let value = match value {
        Value::Value(inner) => inner.as_ref(),
        other => other,
    };
    let Value::Structure(st) = value else {
        return None;
    };
    // Campos às vezes vêm embrulhados num variant.
    let field = |i: usize| -> Option<&Value<'_>> {
        match st.fields().get(i)? {
            Value::Value(inner) => Some(inner.as_ref()),
            other => Some(other),
        }
    };
    let id = match field(0)? {
        Value::I32(n) => *n,
        _ => return None,
    };
    let props: HashMap<String, OwnedValue> = match field(1)? {
        Value::Dict(d) => d.try_clone().ok().and_then(|d| HashMap::<String, OwnedValue>::try_from(d).ok()).unwrap_or_default(),
        _ => HashMap::new(),
    };
    let children: Vec<Item> = match field(2) {
        Some(Value::Array(a)) => a.iter().filter_map(parse_node).collect(),
        _ => Vec::new(),
    };
    let text = |key: &str| props.get(key).and_then(|v| <&str>::try_from(&**v).ok().map(str::to_string));
    let flag = |key: &str, default: bool| props.get(key).and_then(|v| bool::try_from(&**v).ok()).unwrap_or(default);
    let visible = flag("visible", true);
    if !visible {
        return None;
    }
    let separator = text("type").as_deref() == Some("separator");
    let checked = text("toggle-type").filter(|t| !t.is_empty()).map(|_| {
        props.get("toggle-state").and_then(|v| i32::try_from(&**v).ok()).unwrap_or(0) == 1
    });
    Some(Item {
        id,
        label: clean_label(&text("label").unwrap_or_default()),
        enabled: flag("enabled", true),
        separator,
        checked,
        shortcut: props.get("shortcut").map(|v| shortcut_text(v)).unwrap_or_default(),
        children,
    })
}

/// Tira o "_" das teclas de acesso (`_Arquivo` → `Arquivo`; `__` vira `_`).
pub fn clean_label(label: &str) -> String {
    let mut out = String::new();
    let mut chars = label.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '_' {
            if chars.peek() == Some(&'_') {
                out.push('_');
                chars.next();
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// Atalho do dbusmenu (`[["Control", "s"]]`) como texto (`Ctrl+S`).
fn shortcut_text(value: &Value<'_>) -> String {
    let value = match value {
        Value::Value(inner) => inner.as_ref(),
        other => other,
    };
    let Value::Array(combos) = value else {
        return String::new();
    };
    let Some(Value::Array(keys)) = combos.iter().next() else {
        return String::new();
    };
    let keys: Vec<&str> = keys.iter().filter_map(|k| <&str>::try_from(k).ok()).collect();
    // Com ⌘ (Super), como no macOS: símbolos e sem "+" (⌘⇧D).
    let mac = keys.contains(&"Super");
    let names: Vec<String> = keys
        .iter()
        .map(|k| match *k {
            "Control" if mac => "⌃".to_string(),
            "Shift" if mac => "⇧".to_string(),
            "Alt" if mac => "⌥".to_string(),
            "Super" => "⌘".to_string(),
            "Control" => "Ctrl".to_string(),
            "Return" | "KP_Enter" => "Enter".to_string(),
            "Page_Up" => "PgUp".to_string(),
            "Page_Down" => "PgDn".to_string(),
            "Up" => "↑".to_string(),
            "Down" => "↓".to_string(),
            "Left" => "←".to_string(),
            "Right" => "→".to_string(),
            "Delete" => "Del".to_string(),
            other if other.chars().count() == 1 => other.to_uppercase(),
            other => other.to_string(),
        })
        .collect();
    names.join(if mac { "" } else { "+" })
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::zvariant::{Array, Dict, Signature, StructureBuilder};

    fn node(id: i32, label: &str, children: Vec<Value<'static>>) -> Value<'static> {
        let mut props = Dict::new(&Signature::Str, &Signature::Variant);
        props.append(Value::from("label".to_string()), Value::Value(Box::new(Value::from(label.to_string())))).unwrap();
        let mut array = Array::new(&Signature::Variant);
        for child in children {
            array.append(Value::Value(Box::new(child))).unwrap();
        }
        StructureBuilder::new().add_field(id).add_field(Value::Dict(props)).add_field(Value::Array(array)).build().unwrap().into()
    }

    #[test]
    fn le_o_menu_do_app() {
        let root = node(0, "", vec![node(1, "_Arquivo", vec![node(3, "_Salvar", vec![])]), node(2, "_Editar", vec![])]);
        let item = parse_node(&root).unwrap();
        let labels: Vec<&str> = item.children.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(labels, ["Arquivo", "Editar"]);
        assert_eq!(item.children[0].children[0].label, "Salvar");
        assert_eq!(item.children[0].children[0].id, 3);
        assert_eq!(clean_label("Salvar __como"), "Salvar _como");
    }

    #[test]
    fn processo_bate_com_o_app() {
        assert!(matches_app("code", "code"));
        assert!(matches_app("chrome", "google-chrome"));
        assert!(matches_app("kate", "org.kde.kate"));
        assert!(!matches_app("code", "foot"));
        assert!(!matches_app("", "code"));
    }
}
