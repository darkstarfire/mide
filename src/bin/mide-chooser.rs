//! Seletor de arquivos do mide para os outros apps (B25): uma janela do mide-ui com o
//! `FileChooser` ocupando a janela inteira. É aberta pelo `mide-portal` a cada pedido
//! do portal `FileChooser` ("Abrir…", "Salvar como…" do navegador e dos apps GTK/Qt).
//!
//! Argumentos: os de `mide::portal::chooser_args` (`--mode open|folder|save|save-files`,
//! `--title`, `--multiple`, `--folder`, `--name`, `--accept`, `--filter`/`--glob`/`--mime`,
//! `--current-filter`, `--choice`/`--option`, `--save-name`).
//!
//! Saída: os escolhidos no stdout (`mide::portal::format_output`) e o código de saída
//! 0 (escolheu), 1 (cancelou ou fechou a janela) ou 2 (erro: argumentos, sem compositor).

use std::{cell::RefCell, rc::Rc};
use mide::tr;

use mide::{
    kit::{run, App, Choice, FileChooser, Ui, Window},
    portal::{self, ChooserMode, ChooserOutput},
};

struct Chooser {
    chooser: FileChooser,
    done: Rc<RefCell<Option<ChooserOutput>>>,
}

impl App for Chooser {
    fn view(&mut self, ui: &mut Ui) {
        match self.chooser.show(ui) {
            Some(Choice::Chosen(_)) => {
                let out = ChooserOutput {
                    paths: self.chooser.chosen().to_vec(),
                    filter: self.chooser.current_filter(),
                    choices: self.chooser.choice_values(),
                };
                for path in &out.paths {
                    tracing::info!("seletor: escolhido {}", path.display());
                }
                *self.done.borrow_mut() = Some(out);
                ui.close();
            }
            Some(Choice::Cancelled) => {
                tracing::info!("seletor: cancelado");
                ui.close();
            }
            None => {}
        }
    }
}

fn main() {
    // O log vai para o stderr: o stdout é a resposta para o mide-portal.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "mide_chooser=info".into()))
        .init();
    let req = match portal::parse_chooser_args(std::env::args_os().skip(1).collect()) {
        Ok(req) => req,
        Err(err) => {
            eprintln!("mide-chooser: {err}");
            std::process::exit(2);
        }
    };
    let folder = req.folder.as_deref();
    let name = req.name.clone().unwrap_or_default();
    let mut chooser = match req.mode {
        ChooserMode::Open => FileChooser::open(&req.title, folder).multiple(req.multiple),
        ChooserMode::Folder => FileChooser::folder(&req.title, folder),
        ChooserMode::Save => FileChooser::save(&req.title, folder, &name),
        ChooserMode::SaveFiles => FileChooser::save_files(&req.title, folder, &req.files),
    };
    if !req.filters.is_empty() {
        chooser = chooser.filters(req.filters.clone(), req.current_filter.unwrap_or(0));
    }
    if let Some(label) = &req.accept_label {
        chooser = chooser.accept_label(label);
    }
    chooser = chooser.choices(req.choices.clone()).embedded(true);
    tracing::info!("seletor: {} em {}", req.title, chooser.dir().display());
    let done = Rc::new(RefCell::new(None));
    let title = if req.title.is_empty() { tr!("Choose File").into_owned() } else { req.title.clone() };
    let window = Window::new(&title).app_id("mide-chooser").size(720, 500).min_size(420, 360);
    if let Err(err) = run(window, Chooser { chooser, done: done.clone() }) {
        eprintln!("mide-chooser: {err}");
        std::process::exit(2);
    }
    let out = done.borrow_mut().take();
    match out {
        Some(out) => print!("{}", portal::format_output(&out)),
        None => std::process::exit(1),
    }
}
