//! Exemplo e teste de `Window::on_every_output` (mide-ui): uma camada escurecida em
//! cada tela, com o nome da tela no meio. Cliques e arrastos vão para o log em
//! coordenadas globais (do desktop), em qualquer tela e atravessando de uma para a
//! outra (a pega implícita do compositor mantém o ponteiro com a superfície onde o
//! botão foi apertado). Esc fecha.
//!
//! Rodar dentro do mide: `RUST_LOG=kit_telas=info cargo run --example kit-telas`

use std::collections::HashSet;

use mide::kit::{run, App, Key, Ui, Window};

#[derive(Default)]
struct Telas {
    /// Onde o botão foi apertado (global).
    inicio: Option<(f32, f32)>,
    /// Telas já desenhadas (para registrar uma vez cada).
    desenhadas: HashSet<String>,
}

impl App for Telas {
    fn view(&mut self, ui: &mut Ui) {
        let tela = ui.screen();
        if self.desenhadas.insert(tela.name.clone()) {
            tracing::info!(
                "desenhou a tela {} em ({}, {}) {}x{}{}{}",
                tela.name,
                tela.origin.0,
                tela.origin.1,
                tela.size.0,
                tela.size.1,
                if tela.primary { " principal" } else { "" },
                if ui.mirror() { " (passe extra)" } else { "" }
            );
        }
        // Desenho, em todas as telas: fundo escuro e o nome da tela.
        ui.paint(|canvas, _| canvas.fill(tiny_skia::Color::from_rgba8(0, 0, 0, 90)));
        ui.title(&format!("Tela {}", tela.name));
        if let Some(p) = ui.pointer_global() {
            ui.label(&format!("ponteiro global: ({:.0}, {:.0})", p.0, p.1));
        }
        // Lógica, só no passe da tela principal.
        if ui.mirror() {
            return;
        }
        let input = ui.input().clone();
        if input.key(Key::Escape) {
            ui.close();
            return;
        }
        let global = ui.pointer_global();
        if input.pressed {
            self.inicio = global;
        }
        if input.released
            && let (Some(a), Some(b)) = (self.inicio.take(), global)
        {
            let telas = ui.screens();
            let nome = mide::kit::screens::screen_at(&telas, b).map_or("-".to_string(), |i| telas[i].name.clone());
            if (a.0 - b.0).abs() < 3.0 && (a.1 - b.1).abs() < 3.0 {
                tracing::info!("clique global ({:.0}, {:.0}) na tela {nome}", b.0, b.1);
            } else {
                tracing::info!("arrasto global de ({:.0}, {:.0}) a ({:.0}, {:.0}), soltou na tela {nome}", a.0, a.1, b.0, b.1);
            }
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "kit_telas=info,mide=info".into()))
        .init();
    run(Window::new("Telas").overlay("kit-telas").on_every_output().fixed_scale(), Telas::default())
}
