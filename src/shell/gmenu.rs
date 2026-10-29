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
/// Resposta do `org.gtk.Menus.Start`: grupo, menu e itens.
type StartReply = Vec<(u32, u32, Vec<HashMap<String, OwnedValue>>)>;

/// A barra de menus lida: os menus de cima (com os submenus dentro), as ações dos itens
/// (o `id` de cada item é o número na lista) e, para cada menu de cima, as ações de
/// abrir submenu (`submenu-action`) dele e dos submenus dentro dele.
#[derive(Debug, Clone, Default)]
pub struct Menubar {
    pub items: Vec<Item>,
    pub actions: Vec<Action>,
    pub submenu_actions: Vec<Vec<String>>,
}

/// No máximo tantos grupos de menu por leitura. O LibreOffice tem uns 80 (um por
/// submenu); com o limite antigo de 64, os últimos lidos (o menu Arquivo inteiro)
/// ficavam vazios.
const MAX_GROUPS: usize = 1024;

/// Lê a barra de menus inteira: os menus de cima (com os submenus dentro) e as ações.
pub fn read(conn: &zbus::blocking::Connection, menu: &GtkMenu) -> Option<(Vec<Item>, Vec<Action>)> {
    read_menubar(conn, menu).ok().map(|m| (m.items, m.actions))
}

/// Como [`read`], com as ações de submenu e o motivo quando não dá para ler.
pub fn read_menubar(conn: &zbus::blocking::Connection, menu: &GtkMenu) -> Result<Menubar, String> {
    let proxy = zbus::blocking::Proxy::new(conn, menu.bus.clone(), menu.menubar.clone(), "org.gtk.Menus").map_err(|e| e.to_string())?;
    // Assina o grupo 0 e, enquanto aparecerem ligações para grupos novos, eles também,
    // todos os grupos novos de uma vez (uma ida ao app por nível, e não por grupo).
    let mut menus: Menus = HashMap::new();
    let mut subscribed: Vec<u32> = Vec::new();
    let mut wave = vec![0u32];
    while !wave.is_empty() && subscribed.len() < MAX_GROUPS {
        wave.truncate(MAX_GROUPS - subscribed.len());
        let mut next = Vec::new();
        // Em lotes de 16: um pedido grande demais pode passar do tempo de resposta
        // (300 ms) num app ocupado.
        for batch in wave.chunks(16) {
            subscribed.extend(batch);
            let reply: StartReply = match proxy.call("Start", &(batch.to_vec(),)) {
                Ok(reply) => reply,
                Err(err) => {
                    let _: Result<(), _> = proxy.call("End", &(subscribed.clone(),));
                    return Err(format!("org.gtk.Menus.Start: {err}"));
                }
            };
            for (g, m, items) in reply {
                for item in &items {
                    for key in [":section", ":submenu"] {
                        if let Some((lg, _)) = item.get(key).and_then(link)
                            && !subscribed.contains(&lg)
                            && !wave.contains(&lg)
                            && !next.contains(&lg)
                        {
                            next.push(lg);
                        }
                    }
                }
                menus.insert((g, m), items);
            }
        }
        wave = next;
    }
    let _: Result<(), _> = proxy.call("End", &(subscribed.clone(),));
    if menus.get(&(0, 0)).is_none_or(|m| m.is_empty()) {
        return Err("barra de menus vazia".into());
    }
    let states = describe(conn, menu);
    let mut actions = Vec::new();
    let mut submenu_actions = Vec::new();
    let top = build(&menus, (0, 0), &states, &mut actions, &mut submenu_actions, 0, 0);
    // Uma lista por menu de cima (separadores e itens sem nome ficam fora, como na barra).
    let mut per_top = Vec::new();
    let mut items = Vec::new();
    let mut lists = submenu_actions.into_iter();
    for item in top.into_iter().filter(|i| !i.separator) {
        let list = if item.id >= 0 { Vec::new() } else { lists.next().unwrap_or_default() };
        if !item.label.is_empty() {
            per_top.push(list);
            items.push(item);
        }
    }
    Ok(Menubar { items, actions, submenu_actions: per_top })
}

/// Abre (`true`) ou fecha os submenus: muda o estado das ações `submenu-action`, como
/// o GTK faz ao mostrar um submenu. Apps como o LibreOffice só preenchem o submenu (e
/// atualizam o que está ligado e marcado) quando recebem isso; ao fechar, o estado
/// volta a `false`.
pub fn set_submenus_open(conn: &zbus::blocking::Connection, menu: &GtkMenu, names: &[String], open: bool) {
    for name in names {
        let (path, action) = action_target(menu, name);
        let Ok(proxy) = zbus::blocking::Proxy::new(conn, menu.bus.clone(), path, "org.gtk.Actions") else {
            continue;
        };
        let platform: HashMap<&str, Value<'_>> = HashMap::new();
        if let Err(err) = proxy.call::<_, _, ()>("SetState", &(action, Value::from(open), platform)) {
            tracing::debug!("menu do app (GTK): submenu {name} não {}: {err}", if open { "aberto" } else { "fechado" });
        }
    }
}

/// Objeto e nome sem prefixo de uma ação (`app.` no objeto do app, `win.` no da janela).
fn action_target<'a>(menu: &GtkMenu, name: &'a str) -> (String, &'a str) {
    match name.split_once('.') {
        Some(("win", name)) => (menu.window.clone(), name),
        Some((_, name)) => (menu.app.clone(), name),
        None => (menu.app.clone(), name),
    }
}

/// Manda a ação ao app (`app.` no objeto do app, `win.` no da janela).
pub fn activate(conn: &zbus::blocking::Connection, menu: &GtkMenu, action: &Action) {
    let (path, name) = action_target(menu, &action.name);
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

/// Itens do menu `at`, com as seções desfeitas (separadas) e os submenus dentro. No
/// nível de cima (`depth` 0), cada submenu junta em `submenus` uma lista com as ações
/// de abrir dele e dos submenus de dentro.
fn build(menus: &Menus, at: (u32, u32), states: &HashMap<String, (bool, Option<bool>)>, actions: &mut Vec<Action>, submenus: &mut Vec<Vec<String>>, depth: usize, nest: usize) -> Vec<Item> {
    let Some(items) = menus.get(&at) else {
        return Vec::new();
    };
    // `nest` conta também as seções (uma seção que aponta para si mesma não trava).
    if depth > 8 || nest > 24 {
        return Vec::new();
    }
    let mut out: Vec<Item> = Vec::new();
    let text = |item: &HashMap<String, OwnedValue>, key: &str| item.get(key).and_then(|v| <&str>::try_from(&**v).ok().map(str::to_string));
    for item in items {
        if let Some(section) = item.get(":section").and_then(link) {
            // Seções não descem um nível de menu: os submenus de dentro delas na barra
            // de cima ainda são menus de cima.
            let inner = build(menus, section, states, actions, submenus, depth + usize::from(depth > 0), nest + 1);
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
            let opener = text(item, "submenu-action");
            if depth == 0 {
                submenus.push(opener.into_iter().collect());
            } else if let (Some(opener), Some(list)) = (opener, submenus.last_mut()) {
                list.push(opener);
            }
            let children = build(menus, sub, states, actions, submenus, depth + 1, nest + 1);
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
    use super::*;

    fn item(pairs: &[(&str, Value<'static>)]) -> HashMap<String, OwnedValue> {
        pairs.iter().map(|(k, v)| (k.to_string(), OwnedValue::try_from(v.try_clone().unwrap()).unwrap())).collect()
    }

    /// Barra como a do LibreOffice: seções no topo, menus com `submenu-action` e um
    /// submenu dentro de outro.
    #[test]
    fn barra_com_secoes_e_submenu_action() {
        let mut menus: Menus = HashMap::new();
        menus.insert((0, 0), vec![item(&[(":section", Value::from((0u32, 1u32)))])]);
        menus.insert(
            (0, 1),
            vec![
                item(&[("label", Value::from("_Arquivo")), ("submenu-action", Value::from("win.m-1")), (":submenu", Value::from((1u32, 0u32)))]),
                item(&[("label", Value::from("_Ajuda")), ("submenu-action", Value::from("win.m-9")), (":submenu", Value::from((2u32, 0u32)))]),
            ],
        );
        menus.insert(
            (1, 0),
            vec![
                item(&[("label", Value::from("_Abrir")), ("action", Value::from("win.abrir")), ("accel", Value::from("<Primary>o"))]),
                item(&[("label", Value::from("Recentes")), ("submenu-action", Value::from("win.m-2")), (":submenu", Value::from((3u32, 0u32)))]),
            ],
        );
        menus.insert((3, 0), vec![item(&[("label", Value::from("a.odt")), ("action", Value::from("win.r1"))])]);
        menus.insert((2, 0), vec![item(&[("label", Value::from("Sobre")), ("action", Value::from("win.sobre"))])]);
        let mut actions = Vec::new();
        let mut submenus = Vec::new();
        let top = build(&menus, (0, 0), &HashMap::new(), &mut actions, &mut submenus, 0, 0);
        let labels: Vec<&str> = top.iter().filter(|i| !i.separator).map(|i| i.label.as_str()).collect();
        assert_eq!(labels, ["Arquivo", "Ajuda"]);
        assert_eq!(submenus, vec![vec!["win.m-1".to_string(), "win.m-2".to_string()], vec!["win.m-9".to_string()]]);
        assert_eq!(top[0].children[0].shortcut, "Ctrl+O");
        assert_eq!(top[0].children[1].children[0].label, "a.odt");
        assert_eq!(actions.len(), 3);
    }

    /// Uma seção que aponta para ela mesma não trava a leitura.
    #[test]
    fn secao_em_ciclo() {
        let mut menus: Menus = HashMap::new();
        menus.insert((0, 0), vec![item(&[(":section", Value::from((0u32, 0u32)))])]);
        let top = build(&menus, (0, 0), &HashMap::new(), &mut Vec::new(), &mut Vec::new(), 0, 0);
        assert!(top.is_empty());
    }

    #[test]
    fn atalhos_do_gtk() {
        assert_eq!(accel_text("<Primary>s"), "Ctrl+S");
        assert_eq!(accel_text("<Primary><Shift>z"), "Ctrl+Shift+Z");
        assert_eq!(accel_text("F11"), "F11");
    }
}
