//! Junta os catálogos de tradução (`locales/<idioma>/*.po`) numa lista embutida no
//! binário (`$OUT_DIR/catalogs.rs`, lida por `src/i18n.rs`): arquivo novo entra sozinho.

use std::{fmt::Write, fs, path::Path};

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("locales");
    println!("cargo:rerun-if-changed={}", root.display());
    let mut list = String::from("&[\n");
    let mut langs: Vec<_> = fs::read_dir(&root).into_iter().flatten().flatten().filter(|e| e.path().is_dir()).collect();
    langs.sort_by_key(|e| e.file_name());
    for lang in langs {
        println!("cargo:rerun-if-changed={}", lang.path().display());
        let mut files: Vec<_> = fs::read_dir(lang.path()).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "po")).collect();
        files.sort();
        for file in files {
            println!("cargo:rerun-if-changed={}", file.display());
            let _ = writeln!(list, "    ({:?}, include_str!({:?})),", lang.file_name().to_string_lossy(), file.display().to_string());
        }
    }
    list.push(']');
    let out = Path::new(&std::env::var("OUT_DIR").unwrap()).join("catalogs.rs");
    fs::write(out, list).unwrap();
}
