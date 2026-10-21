//! Menus do GTK pelo D-Bus (`org.gtk.Menus` e `org.gtk.Actions`), para o menu global
//! dos apps GTK no Wayland (B2). O app diz ao mide onde está a barra de menus
//! (`gtk_shell1.set_dbus_properties`, veja `app_menus` no compositor); a barra lê os
//! menus daqui e manda as ações clicadas.
//!
//! Formato: cada menu é um grupo e um número; os itens têm `label`, `action` (`app.x`
//! ou `win.x`), `target` (o parâmetro da ação) e ligações para outros menus
//! (`:section`, um trecho que entra no mesmo menu com separadores, e `:submenu`).
//! Os itens viram `appmenu::Item` (o mesmo do `dbusmenu`), com o número da ação na
//! lista de ações devolvida junto.

use std::collections::HashMap;

use zbus::zvariant::{OwnedValue, Value};

use crate::appmenu::Item;

/// Onde está o menu de um app GTK.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GtkMenu {
    pub bus: String,
    pub menubar: String,
    pub app: String,
    pub window: String,
}

/// Uma ação do menu: nome com o prefixo (`app.salvar`) e o parâmetro.
#[derive(Debug, Clone)]
pub struct Action {
    pub name: String,
    pub target: Option<OwnedValue>,
}

type Menus = HashMap<(u32, u32), Vec<HashMap<String, OwnedValue>>>;

/// Lê a barra de menus inteira: os menus de cima (com os submenus dentro) e as ações.
pub fn read(conn: &zbus::blocking::Connection, menu: &GtkMenu) -> Option<(Vec<Item>, Vec<Action>)> {
    let proxy = zbus::blocking::Proxy::new(conn, menu.bus.clone(), menu.menubar.clone(), "org.gtk.Menus").ok()?;
    // Assina o grupo 0 e, enquanto aparecerem ligações para grupos novos, eles também.
    let mut menus: Menus = HashMap::new();
    let mut pending = vec![0u32];
    let mut subscribed = Vec::new();
    while let Some(group) = pending.pop() {
        if subscribed.contains(&group) || subscribed.len() > 64 {
            continue;
        }
        subscribed.push(group);
        let reply: Vec<(u32, u32, Vec<HashMap<String, OwnedValue>>)> = proxy.call("Start", &(vec![group],)).ok()?;
        for (g, m, items) in reply {
            for item in &items {
                for key in [":section", ":submenu"] {
                    if let Some((lg, _)) = item.get(key).and_then(link) {
                        if !subscribed.contains(&lg) {
                            pending.push(lg);
                        }
                    }
                }
            }
            menus.insert((g, m), items);
        }
    }
    let _: Result<(), _> = proxy.call("End", &(subscribed.clone(),));
    let states = describe(conn, menu);
    let mut actions = Vec::new();
    let top = build(&menus, (0, 0), &states, &mut actions, 0);
    Some((top, actions))
}

/// Manda a ação ao app (`app.` no objeto do app, `win.` no da janela).
pub fn activate(conn: &zbus::blocking::Connection, menu: &GtkMenu, action: &Action) {
    let (path, name) = match action.name.split_once('.') {
        Some(("win", name)) => (menu.window.clone(), name),
        Some((_, name)) => (menu.app.clone(), name),
        None => (menu.app.clone(), action.name.as_str()),
    };
    let Ok(proxy) = zbus::blocking::Proxy::new(conn, menu.bus.clone(), path, "org.gtk.Actions") else {
        return;
    };
    let params: Vec<Value<'_>> = action.target.iter().filter_map(|t| t.try_clone().ok().map(Value::from)).collect();
    let platform: HashMap<&str, Value<'_>> = HashMap::new();
    if let Err(err) = proxy.call::<_, _, ()>("Activate", &(name, params, platform)) {
        tracing::warn!("menu do app (GTK): ação {} não chegou: {err}", action.name);
    }
}

/// Estado das ações (ligada; marcada, se for de liga-desliga), pelo nome com prefixo.
fn describe(conn: &zbus::blocking::Connection, menu: &GtkMenu) -> HashMap<String, (bool, Option<bool>)> {
    let mut out = HashMap::new();
    for (prefix, path) in [("app", &menu.app), ("win", &menu.window)] {
        if path.is_empty() {
            continue;
        }
        let Ok(proxy) = zbus::blocking::Proxy::new(conn, menu.bus.clone(), path.clone(), "org.gtk.Actions") else {
            continue;
        };
        let all: HashMap<String, (bool, zbus::zvariant::Signature, Vec<OwnedValue>)> = match proxy.call("DescribeAll", &()) {
            Ok(all) => all,
            Err(_) => continue,
        };
        for (name, (enabled, _, state)) in all {
            let checked = state.first().and_then(|v| bool::try_from(&**v).ok());
            out.insert(format!("{prefix}.{name}"), (enabled, checked));
        }
    }
    out
}

/// Ligação para outro menu: `(grupo, menu)`.
fn link(value: &OwnedValue) -> Option<(u32, u32)> {
    let value: &Value<'_> = value;
    let value = match value {
        Value::Value(inner) => inner.as_ref(),
        other => other,
    };
    let Value::Structure(st) = value else {
        return None;
    };
    let fields = st.fields();
    Some((u32::try_from(fields.first()?).ok()?, u32::try_from(fields.get(1)?).ok()?))
}

/// Itens do menu `at`, com as seções desfeitas (separadas) e os submenus dentro.
fn build(menus: &Menus, at: (u32, u32), states: &HashMap<String, (bool, Option<bool>)>, actions: &mut Vec<Action>, depth: usize) -> Vec<Item> {
    let Some(items) = menus.get(&at) else {
        return Vec::new();
    };
    if depth > 8 {
        return Vec::new();
    }
    let mut out: Vec<Item> = Vec::new();
    let text = |item: &HashMap<String, OwnedValue>, key: &str| item.get(key).and_then(|v| <&str>::try_from(&**v).ok().map(str::to_string));
    for item in items {
        if let Some(section) = item.get(":section").and_then(link) {
            let inner = build(menus, section, states, actions, depth + 1);
            if inner.is_empty() {
                continue;
            }
            if !out.is_empty() && !out.last().is_some_and(|i| i.separator) {
                out.push(separator());
            }
            out.extend(inner);
            out.push(separator());
            continue;
        }
        let label = crate::appmenu::clean_label(&text(item, "label").unwrap_or_default());
        if let Some(sub) = item.get(":submenu").and_then(link) {
            let children = build(menus, sub, states, actions, depth + 1);
            out.push(Item { id: -1, label, enabled: true, separator: false, checked: None, shortcut: String::new(), children });
            continue;
        }
        let Some(action) = text(item, "action") else {
            continue;
        };
        let (enabled, checked) = states.get(&action).copied().unwrap_or((true, None));
        let shortcut = text(item, "accel").map(|a| accel_text(&a)).unwrap_or_default();
        actions.push(Action { name: action, target: item.get("target").and_then(|t| t.try_clone().ok()) });
        out.push(Item { id: (actions.len() - 1) as i32, label, enabled, separator: false, checked, shortcut, children: Vec::new() });
    }
    while out.last().is_some_and(|i| i.separator) {
        out.pop();
    }
    out
}

fn separator() -> Item {
    Item { id: -1, label: String::new(), enabled: false, separator: true, checked: None, shortcut: String::new(), children: Vec::new() }
}

/// Atalho do GTK (`<Primary><Shift>s`) como texto (`Ctrl+Shift+S`).
fn accel_text(accel: &str) -> String {
    let mut parts = Vec::new();
    let mut rest = accel;
    while let Some(start) = rest.find('<') {
        let Some(end) = rest[start..].find('>') else {
            break;
        };
        parts.push(match &rest[start + 1..start + end] {
            "Primary" | "Control" | "Ctrl" => "Ctrl",
            "Shift" => "Shift",
            "Alt" => "Alt",
            "Super" => "Super",
            other => other,
        }
        .to_string());
        rest = &rest[start + end + 1..];
    }
    let key = rest.trim();
    if !key.is_empty() {
        parts.push(if key.chars().count() == 1 { key.to_uppercase() } else { key.to_string() });
    }
    parts.join("+")
}

#[cfg(test)]
mod tests {
    use super::accel_text;

    #[test]
    fn atalhos_do_gtk() {
        assert_eq!(accel_text("<Primary>s"), "Ctrl+S");
        assert_eq!(accel_text("<Primary><Shift>z"), "Ctrl+Shift+Z");
        assert_eq!(accel_text("F11"), "F11");
    }
}
