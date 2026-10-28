//! `mide-new`: cria um app nativo do mide pronto para compilar, com o framework
//! mide-ui (github.com/fabioaacarneiro/mide-ui).
//!
//! ```text
//! mide-new "Minhas Notas"            # cria ./minhas-notas
//! mide-new "Minhas Notas" --dir ~/Projetos
//! mide-new "Minhas Notas" --local    # usa o mide-ui da pasta ao lado do mide (desenvolvimento)
//! ```
//!
//! O projeto criado tem o `Cargo.toml`, o `src/main.rs` com uma janela no visual do
//! Tahoe (barra lateral de vidro com seções, barra de ferramentas, cartões, campo de
//! texto, lista, interruptor e menus na barra superior), o `.desktop` para o app
//! aparecer no launcher e na dock, e um README com como rodar e instalar.

use std::{
    error::Error,
    fs,
    path::{Path, PathBuf},
};

use mide::tr;

/// Onde o mide-ui está publicado (o repositório do mide).
const REPO: &str = "https://github.com/fabioaacarneiro/mide-ui";

/// Nome do pacote: minúsculas sem acento, palavras ligadas por hífen.
fn slug(name: &str) -> String {
    let folded = mide::apps::fold(name);
    let mut out = String::new();
    for c in folded.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() || out.starts_with(|c: char| c.is_ascii_digit()) { format!("app-{out}") } else { out }
}

/// Nome do tipo do app em Rust: `minhas-notas` → `MinhasNotas`.
fn type_name(slug: &str) -> String {
    slug.split('-')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let mut c = p.chars();
            c.next().map(|f| f.to_ascii_uppercase().to_string() + c.as_str()).unwrap_or_default()
        })
        .collect()
}

/// Os arquivos do projeto novo: caminho relativo e conteúdo.
pub fn project_files(display: &str, slug: &str, dependency: &str) -> Vec<(String, String)> {
    let ty = type_name(slug);
    let cargo = format!(
        "[package]\nname = \"{slug}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\n# Framework de apps nativos do mide.\n{dependency}\n\n# As dependências (desenho, texto, imagens) otimizadas também em desenvolvimento:\n# sem isso, o desenho da janela fica lento no `cargo run`.\n[profile.dev.package.\"*\"]\nopt-level = 2\n"
    );
    let main = MAIN_TEMPLATE.replace("{TYPE}", &ty).replace("{NAME}", display).replace("{SLUG}", slug);
    let desktop = format!(
        "[Desktop Entry]\nType=Application\nName={display}\nComment=App made with mide-ui\nExec={slug}\nIcon=application-x-executable\nCategories=Utility;\nStartupWMClass={slug}\n"
    );
    let readme = README_TEMPLATE.replace("{NAME}", display).replace("{SLUG}", slug);
    vec![
        ("Cargo.toml".into(), cargo),
        ("src/main.rs".into(), main),
        (format!("data/{slug}.desktop"), desktop),
        ("README.md".into(), readme),
        (".gitignore".into(), "/target\n".into()),
    ]
}

const MAIN_TEMPLATE: &str = r#"//! {NAME}: app nativo do mide, feito com o mide-ui.
//!
//! O app descreve a tela em `view` a cada quadro (como um jogo): o mide-ui cuida da
//! janela, do mouse, do teclado, do tema claro/escuro e da cor de destaque da sessão.
//! Guia: docs/mide-ui.md no repositório do mide.

use mide_ui::kit::{run, window_menu, App, MenuGroup, MenuItem, Ui, Window};

/// Seções da barra lateral: nome e ícone do tema.
const SECTIONS: &[(&str, &str)] = &[
    ("Home", "go-home-symbolic"),
    ("Notes", "accessories-text-editor-symbolic"),
    ("Settings", "preferences-system-symbolic"),
];

#[derive(Default)]
struct {TYPE} {
    section: usize,
    clicks: u32,
    notes: Vec<String>,
    draft: String,
    compact: bool,
}

impl {TYPE} {
    fn add_note(&mut self) {
        let text = self.draft.trim().to_string();
        if !text.is_empty() {
            self.notes.push(text);
            self.draft.clear();
        }
    }
}

impl App for {TYPE} {
    fn view(&mut self, ui: &mut Ui) {
        // Itens dos menus da barra superior (veja `menu_bar`).
        for action in ui.input().menu.clone() {
            match action.as_str() {
                "new-note" => self.section = 1,
                "quit" => ui.close(),
                _ => {}
            }
        }
        // Janela do Tahoe: barra lateral flutuante de vidro, com os botões da janela.
        ui.glass_sidebar();
        let current = self.section;
        self.section = ui.sidebar_area(SECTIONS, current, |ui, section| {
            ui.toolbar(|ui| ui.toolbar_title(SECTIONS[section].0, 320.0));
            ui.page(|ui| match section {
                0 => {
                    ui.title("{NAME}");
                    ui.card(|ui| {
                        ui.label(&format!("You clicked {} time(s).", self.clicks));
                        if ui.primary_button("Click Me").clicked() {
                            self.clicks += 1;
                        }
                    });
                    ui.note("Edit src/main.rs and run it again with cargo run.");
                }
                1 => {
                    ui.title("Notes");
                    ui.card(|ui| {
                        if ui.text_input(&mut self.draft, "Write a note and press Enter").submitted() {
                            self.add_note();
                        }
                        if ui.button("Add").clicked() {
                            self.add_note();
                        }
                    });
                    if !self.notes.is_empty() {
                        ui.card(|ui| {
                            for note in &self.notes {
                                ui.list_item(note, "");
                            }
                        });
                    }
                }
                _ => {
                    ui.title("Settings");
                    ui.card(|ui| {
                        ui.switch_detail("Compact Mode", "An example switch", &mut self.compact);
                    });
                }
            });
        });
    }

    /// Menus do app na barra superior do mide (como no Mac).
    fn menu_bar(&mut self) -> Vec<MenuGroup> {
        vec![
            MenuGroup {
                title: "File".into(),
                items: vec![
                    MenuItem::action("new-note", "New Note").shortcut("Ctrl+N"),
                    MenuItem::Separator,
                    MenuItem::action("quit", "Quit").shortcut("Ctrl+Q"),
                ],
            },
            window_menu(Vec::new()),
        ]
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let window = Window::new("{NAME}").app_id("{SLUG}").size(820, 520).min_size(560, 360).full_size_content();
    run(window, {TYPE}::default())
}
"#;

const README_TEMPLATE: &str = r#"# {NAME}

Native [mide](https://github.com/fabioaacarneiro/mide) app, made with the mide-ui
framework.

## Run

Inside a mide session (or mide in a window, for development):

```sh
cargo run
```

## Install for your user

```sh
cargo build --release
install -Dm755 target/release/{SLUG} ~/.local/bin/{SLUG}
install -Dm644 data/{SLUG}.desktop ~/.local/share/applications/{SLUG}.desktop
```

The app shows up in the launcher and can be pinned to the dock.

## Next steps

- Available components: `docs/mide-ui.md` in the mide repository and the gallery
  (`cargo run -p mide-ui --example mide-ui-demo` in the mide repository).
- Look: follow the Tahoe map in `docs/tahoe.md`.
"#;

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    let (mut name, mut dir, mut local) = (None::<String>, PathBuf::from("."), false);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--dir" => dir = args.next().map(PathBuf::from).ok_or_else(|| tr!("--dir needs a folder").into_owned())?,
            "--local" => local = true,
            "-h" | "--help" => {
                println!("{}", tr!("usage: mide-new \"App Name\" [--dir FOLDER] [--local]"));
                return Ok(());
            }
            other => name = Some(other.to_string()),
        }
    }
    let Some(display) = name else {
        eprintln!("{}", tr!("usage: mide-new \"App Name\" [--dir FOLDER] [--local]"));
        std::process::exit(2);
    };
    let slug = slug(&display);
    let target = dir.join(&slug);
    if target.exists() {
        return Err(tr!("{path} already exists", path = target.display()).into());
    }
    // `--local`: o mide-ui clonado ao lado do mide (para quem desenvolve os dois).
    let dependency = if local {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap_or(Path::new("..")).join("mide-ui");
        format!("mide-ui = {{ path = \"{}\" }}", path.display())
    } else {
        format!("mide-ui = {{ git = \"{REPO}\" }}")
    };
    for (rel, text) in project_files(&display, &slug, &dependency) {
        let path = target.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, text)?;
    }
    println!("{}", tr!("App “{name}” created in {path}", name = display, path = target.display()));
    println!("{}", tr!("To run it:  cd {path} && cargo run", path = target.display()));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nomes_do_projeto() {
        assert_eq!(slug("Minhas Notas"), "minhas-notas");
        assert_eq!(slug("Calculadora de Preços!"), "calculadora-de-precos");
        assert_eq!(slug("3D"), "app-3d");
        assert_eq!(type_name("minhas-notas"), "MinhasNotas");
    }

    #[test]
    fn arquivos_do_projeto() {
        let files = project_files("Minhas Notas", "minhas-notas", "mide-ui = { path = \"x\" }");
        let get = |n: &str| files.iter().find(|(p, _)| p == n).map(|(_, t)| t.clone()).unwrap();
        assert!(get("Cargo.toml").contains("name = \"minhas-notas\""));
        assert!(get("Cargo.toml").contains("[profile.dev.package.\"*\"]"));
        let main = get("src/main.rs");
        assert!(main.contains("struct MinhasNotas") && main.contains(".app_id(\"minhas-notas\")"));
        assert!(!main.contains("{TYPE}") && !main.contains("{NAME}"));
        assert!(get("data/minhas-notas.desktop").contains("Exec=minhas-notas"));
    }
}
