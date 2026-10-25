//! Janela comum (sem conteúdo inteiro) com menus: no mide, a barra de título é do
//! compositor e os menus vão para a barra superior; fora do mide (ou com
//! `MIDE_UI_STANDALONE=1`), o mide-ui desenha a barra, com os menus dentro dela.

use mide_ui::kit::{run, window_menu, App, MenuGroup, MenuItem, Ui, Window};

#[derive(Default)]
struct Comum {
    cliques: u32,
    ultimo_menu: String,
}

impl App for Comum {
    fn view(&mut self, ui: &mut Ui) {
        for action in ui.input().menu.clone() {
            self.ultimo_menu = action;
        }
        ui.page(|ui| {
            ui.title("Janela comum");
            ui.card(|ui| {
                ui.label(&format!("{} clique(s). Último menu: {}", self.cliques, if self.ultimo_menu.is_empty() { "nenhum" } else { &self.ultimo_menu }));
                if ui.primary_button("Clicar").clicked() {
                    self.cliques += 1;
                }
            });
        });
    }

    fn menu_bar(&mut self) -> Vec<MenuGroup> {
        vec![
            MenuGroup { title: "Arquivo".into(), items: vec![MenuItem::action("novo", "Novo").shortcut("Ctrl+N"), MenuItem::Separator, MenuItem::action("sair", "Sair")] },
            MenuGroup { title: "Editar".into(), items: vec![MenuItem::action("copiar", "Copiar"), MenuItem::action("colar", "Colar")] },
            window_menu(Vec::new()),
        ]
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    run(Window::new("Janela comum").app_id("janela-comum").size(560, 360), Comum::default())
}
