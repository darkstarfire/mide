//! App de teste do tray (testes de interface): publica um ícone `StatusNotifierItem`
//! (um quadrado laranja, pela imagem) com um menu `dbusmenu` ("Abrir" = 1, "Sair" =
//! 2), como o Discord, o Steam e o Nextcloud. Grava em `TRAY_TESTE_ARQUIVO` o que
//! aconteceu: `ativado` (clique esquerdo) ou `clique N` (item do menu).

use std::collections::HashMap;

use zbus::{
    object_server::SignalEmitter,
    zvariant::{Array, Dict, ObjectPath, OwnedObjectPath, OwnedValue, Signature, StructureBuilder, Value},
};

fn record(text: &str) {
    if let Ok(path) = std::env::var("TRAY_TESTE_ARQUIVO") {
        let _ = std::fs::write(path, text);
    }
}

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

struct Menu;

#[zbus::interface(name = "com.canonical.dbusmenu")]
impl Menu {
    fn get_layout(&self, _parent_id: i32, _depth: i32, _props: Vec<String>) -> (u32, OwnedValue) {
        let root = node(0, "", vec![node(1, "Abrir", vec![]), node(2, "Sair", vec![])]);
        (1, OwnedValue::try_from(root).expect("valor"))
    }

    fn about_to_show(&self, _id: i32) -> bool {
        false
    }

    fn event(&self, id: i32, event_id: String, _data: OwnedValue, _timestamp: u32) {
        if event_id == "clicked" {
            record(&format!("clique {id}"));
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

struct Item;

#[zbus::interface(name = "org.kde.StatusNotifierItem")]
impl Item {
    fn activate(&self, _x: i32, _y: i32) {
        record("ativado");
    }

    fn secondary_activate(&self, _x: i32, _y: i32) {}

    fn context_menu(&self, _x: i32, _y: i32) {}

    fn scroll(&self, _delta: i32, _orientation: String) {}

    #[zbus(property)]
    fn id(&self) -> String {
        "mide-tray-teste".into()
    }

    #[zbus(property)]
    fn title(&self) -> String {
        "Teste do tray".into()
    }

    #[zbus(property)]
    fn category(&self) -> String {
        "ApplicationStatus".into()
    }

    #[zbus(property)]
    fn status(&self) -> String {
        "Active".into()
    }

    #[zbus(property)]
    fn icon_name(&self) -> String {
        String::new()
    }

    /// Um quadrado laranja de 22 px (ARGB32, big-endian).
    #[zbus(property)]
    fn icon_pixmap(&self) -> Vec<(i32, i32, Vec<u8>)> {
        let pixels: Vec<u8> = (0..22 * 22).flat_map(|_| [255u8, 255, 138, 31]).collect();
        vec![(22, 22, pixels)]
    }

    #[zbus(property)]
    fn menu(&self) -> OwnedObjectPath {
        OwnedObjectPath::try_from("/MenuBar").expect("caminho")
    }

    #[zbus(property)]
    fn item_is_menu(&self) -> bool {
        false
    }

    #[zbus(signal)]
    async fn new_icon(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let name = format!("org.kde.StatusNotifierItem-{}-1", std::process::id());
    let conn = zbus::blocking::connection::Builder::session()?
        .name(name.as_str())?
        .serve_at("/StatusNotifierItem", Item)?
        .serve_at("/MenuBar", Menu)?
        .build()?;
    let watcher = zbus::blocking::Proxy::new(&conn, "org.kde.StatusNotifierWatcher", "/StatusNotifierWatcher", "org.kde.StatusNotifierWatcher")?;
    // A barra pode ainda não estar pronta: tenta por alguns segundos.
    for _ in 0..50 {
        if watcher.call::<_, _, ()>("RegisterStatusNotifierItem", &(name.as_str(),)).is_ok() {
            println!("ícone registrado");
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    let _ = ObjectPath::try_from("/StatusNotifierItem");
    // Tempo de vida (`TRAY_TESTE_SEGUNDOS`, padrão 60): depois o app sai e o ícone some.
    let secs = std::env::var("TRAY_TESTE_SEGUNDOS").ok().and_then(|s| s.parse().ok()).unwrap_or(60);
    std::thread::sleep(std::time::Duration::from_secs(secs));
    Ok(())
}
