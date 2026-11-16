//! Boas-vindas do mios (`mios-welcome`), como o Assistente de Configuração do Mac: no
//! primeiro login do sistema instalado, apresenta o mios, deixa escolher a aparência
//! (clara ou escura) e o papel de parede, e some (não aparece de novo). No modo live
//! não abre: lá quem aparece é o instalador.

use mide::control_client::ask_compositor;
use mide::kit::{run, App, Tile, Ui, Window};
use mide::settings_file::{self, Value};
use mide::tr;

/// Arquivo que marca as boas-vindas como vistas.
fn done_marker() -> std::path::PathBuf {
    let config = std::env::var_os("XDG_CONFIG_HOME").map(std::path::PathBuf::from).unwrap_or_else(|| mide::places::home_dir().join(".config"));
    config.join("mide/welcome-done")
}

struct Welcome {
    page: usize,
    dark: bool,
}

impl Welcome {
    fn set_dark(&mut self, dark: bool) {
        self.dark = dark;
        let value = dark.then(|| Value::Text("dark".into()));
        if settings_file::set("appearance", "theme", value).is_ok() {
            ask_compositor("reload");
            mide::appearance::apply(true);
            tracing::info!("boas-vindas: aparência {}", if dark { "escura" } else { "clara" });
        }
    }

    fn finish(&self, ui: &mut Ui) {
        let marker = done_marker();
        if let Some(dir) = marker.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&marker, "1\n");
        tracing::info!("boas-vindas: concluídas");
        ui.close();
    }
}

impl App for Welcome {
    fn view(&mut self, ui: &mut Ui) {
        ui.toolbar(|ui| ui.toolbar_title(&tr!("Welcome"), 300.0));
        ui.page(|ui| {
            match self.page {
                0 => {
                    ui.title(&tr!("Welcome to mios"));
                    ui.label(&tr!("Your Mac inspired system is ready. Here are a few things to set up before you start."));
                    ui.card(|ui| {
                        ui.key_value("⌘ Space", &tr!("Search apps, files and settings"), 140.0);
                        ui.key_value("⌘ ⇧ Space", &tr!("Open the launcher with all apps"), 140.0);
                        ui.key_value("⌘ ⇧ 4", &tr!("Take a screenshot of an area"), 140.0);
                        ui.key_value(&tr!("Store"), &tr!("Install apps from the Store in the dock"), 140.0);
                    });
                }
                1 => {
                    ui.title(&tr!("Choose Your Look"));
                    ui.note(&tr!("You can change it later in Settings → Appearance."));
                    let mut light = false;
                    let mut dark = false;
                    ui.wrap(|ui| {
                        light = ui.tile(Tile::Icon("weather-clear"), &tr!("Light"), !self.dark, 96).clicked();
                        dark = ui.tile(Tile::Icon("weather-clear-night"), &tr!("Dark"), self.dark, 96).clicked();
                    });
                    if light {
                        self.set_dark(false);
                    }
                    if dark {
                        self.set_dark(true);
                    }
                }
                _ => {
                    ui.title(&tr!("All Set"));
                    ui.label(&tr!("Enjoy mios. Updates arrive automatically in Software Update."));
                }
            }
            ui.space(16.0);
            ui.row(|ui| {
                if self.page > 0 && ui.button(&tr!("Back")).clicked() {
                    self.page -= 1;
                }
                if self.page < 2 {
                    if ui.primary_button(&tr!("Continue")).clicked() {
                        self.page += 1;
                    }
                } else if ui.primary_button(&tr!("Start Using mios")).clicked() {
                    self.finish(ui);
                }
            });
        });
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _logging = mide::logging::init("mios-welcome", "mios_welcome=info,mide=info");
    mide::i18n::init();
    let force = std::env::args().any(|a| a == "--force");
    // No modo live, o instalador é quem recebe; depois de vistas, não aparecem mais.
    if !force && (std::path::Path::new("/run/live/medium").exists() || done_marker().exists()) {
        return Ok(());
    }
    let window = Window::new(&tr!("Welcome")).app_id("mios-welcome").size(640, 520).min_size(560, 460).full_size_content();
    run(window, Welcome { page: 0, dark: mide::kit::system_dark() })
}
