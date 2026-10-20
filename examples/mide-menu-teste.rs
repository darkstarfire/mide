//! App de teste do menu global (testes de interface): uma janela do mide-ui que
//! publica a barra de menus pelo D-Bus (`com.canonical.dbusmenu`), como o VS Code e o
//! Chrome fazem no X11, e a registra no registrar da barra. O clique num item é
//! gravado no arquivo de `MENU_TESTE_ARQUIVO` (o número do item).
//!
//! Menus: "Projeto" (Compilar = 11, Limpar = 12) e "Ajuda" (Sobre = 21).

use std::collections::HashMap;

use mide::kit::{run, App, Ui, Window};
use zbus::zvariant::{Array, Dict, OwnedValue, Signature, StructureBuilder, Value};

fn node(id: i32, label: &str, children: Vec<Value<'static>>) -> Value<'static> {
    let mut props = Dict::new(&Signature::Str, &Signature::Variant);
    props.append(Value::from("label".to_string()), Value::Value(Box::new(Value::from(label.to_string())))).expect("rótulo");
    if !children.is_empty() {
        props
            .append(Value::from("children-display".to_string()), Value::Value(Box::new(Value::from("submenu".to_string()))))
            .expect("submenu");
    }
    let mut array = Array::new(&Signature::Variant);
    for child in children {
        array.append(Value::Value(Box::new(child))).expect("filho");
    }
    StructureBuilder::new().append_field(Value::I32(id)).append_field(Value::Dict(props)).append_field(Value::Array(array)).build().expect("nó").into()
}

fn menu() -> Value<'static> {
    node(
        0,
        "",
        vec![
            node(1, "_Projeto", vec![node(11, "_Compilar", vec![]), node(12, "_Limpar", vec![])]),
            node(2, "A_juda", vec![node(21, "_Sobre", vec![])]),
        ],
    )
}

fn find(value: &Value<'static>, id: i32) -> Option<Value<'static>> {
    let Value::Structure(st) = value else {
        return None;
    };
    if st.fields().first() == Some(&Value::I32(id)) {
        return Some(value.try_clone().ok()?);
    }
    if let Some(Value::Array(children)) = st.fields().get(2) {
        for child in children.iter() {
            if let Value::Value(inner) = child {
                if let Some(found) = find(&inner.try_clone().ok()?, id) {
                    return Some(found);
                }
            }
        }
    }
    None
}

struct DbusMenu;

#[zbus::interface(name = "com.canonical.dbusmenu")]
impl DbusMenu {
    fn get_layout(&self, parent_id: i32, _depth: i32, _props: Vec<String>) -> (u32, OwnedValue) {
        let root = find(&menu(), parent_id).unwrap_or_else(menu);
        (1, OwnedValue::try_from(root).expect("valor"))
    }

    fn about_to_show(&self, _id: i32) -> bool {
        false
    }

    fn event(&self, id: i32, event_id: String, _data: OwnedValue, _timestamp: u32) {
        if event_id == "clicked" {
            if let Ok(path) = std::env::var("MENU_TESTE_ARQUIVO") {
                let _ = std::fs::write(path, id.to_string());
            }
        }
    }

    #[zbus(property)]
    fn version(&self) -> u32 {
        3
    }

    #[zbus(property)]
    fn status(&self) -> String {
        "normal".into()
    }

    fn get_group_properties(&self, _ids: Vec<i32>, _props: Vec<String>) -> Vec<(i32, HashMap<String, OwnedValue>)> {
        Vec::new()
    }
}

struct Teste;

impl App for Teste {
    fn view(&mut self, ui: &mut Ui) {
        ui.title("Menu de teste");
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // O menu fica no barramento enquanto a janela existir.
    let conn = zbus::blocking::connection::Builder::session()?.serve_at("/MenuBar", DbusMenu)?.build()?;
    let path = zbus::zvariant::ObjectPath::try_from("/MenuBar")?;
    let registrar = zbus::blocking::Proxy::new(&conn, "com.canonical.AppMenu.Registrar", "/com/canonical/AppMenu/Registrar", "com.canonical.AppMenu.Registrar")?;
    // A barra pode ainda não estar pronta: tenta por alguns segundos.
    for _ in 0..30 {
        if registrar.call::<_, _, ()>("RegisterWindow", &(1u32, &path)).is_ok() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    run(Window::new("Menu de teste").app_id("mide-menu-teste").size(400, 200), Teste)
}
