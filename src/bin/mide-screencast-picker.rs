//! Escolha da tela para gravar ou compartilhar (A19). O portal de captura
//! (`xdg-desktop-portal-wlr`) abre esta janela quando um app (OBS, navegador numa
//! videochamada) pede a tela; o monitor escolhido sai na saída padrão (o nome dele,
//! como `HDMI-A-1`), que o portal lê. Cancelar não escreve nada: o app recebe a recusa.

use std::error::Error;
use mide::tr;

use mide::{
    control_client::ask_compositor,
    displays::Display,
    kit::{run, App, Ui, Window},
};

struct Picker {
    displays: Vec<Display>,
    chosen: usize,
}

impl App for Picker {
    fn view(&mut self, ui: &mut Ui) {
        ui.page(|ui| {
            ui.title(&tr!("Share Screen"));
            ui.note(&tr!("An app asked to record or stream the screen. Choose the display:"));
            if self.displays.is_empty() {
                ui.note(&tr!("No display found."));
            }
            let mut chosen = self.chosen;
            ui.card(|ui| {
                for (i, display) in self.displays.iter().enumerate() {
                    let name = if display.description.is_empty() { display.name.clone() } else { format!("{} ({})", display.description, display.name) };
                    let size = format!("{}×{}", display.current.0, display.current.1);
                    let main = if display.primary { tr!("main") } else { "".into() };
                    if ui.list_row("video-display", &name, &[(size.as_str(), 110.0), (&main, 90.0)], i == chosen).clicked() {
                        chosen = i;
                    }
                }
            });
            self.chosen = chosen;
            let mut share = false;
            ui.row(|ui| {
                if ui.button(&tr!("Cancel")).clicked() {
                    tracing::info!("compartilhar a tela: cancelado");
                    ui.close();
                }
                if ui.primary_button(&tr!("Share")).clicked() {
                    share = true;
                }
            });
            if share {
                if let Some(screen) = self.displays.get(self.chosen) {
                    tracing::info!("compartilhar a tela: {}", screen.name);
                    println!("{}", screen.name);
                }
                ui.close();
            }
        });
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let _logging = mide::logging::init("mide-screencast-picker", "mide_screencast_picker=info");
    let displays: Vec<Display> = ask_compositor("outputs")
        .unwrap_or_default()
        .iter()
        .filter_map(|line| mide::displays::parse_line(line))
        .filter(|d| d.enabled)
        .collect();
    // Começa pelo monitor principal.
    let chosen = displays.iter().position(|d| d.primary).unwrap_or(0);
    let window = Window::new(&tr!("Share Screen")).app_id("mide-screencast-picker").size(520, 280).min_size(420, 240);
    run(window, Picker { displays, chosen })
}
