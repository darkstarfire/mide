//! Galeria do mide-ui no visual do Tahoe (`docs/tahoe.md`): janela de conteúdo
//! inteiro, barra lateral flutuante, barra de ferramentas e cada componente, para
//! conferir por captura de tela (claro e escuro).
//!
//! Rodar dentro do mide: `cargo run --example mide-ui-demo`

use mide::kit::{run, App, ButtonStyle, TabAction, Ui, Window, TAG_COLORS};

struct Demo {
    secao: usize,
    busca: String,
    visao: usize,
    // Controles
    notificacoes: bool,
    som: bool,
    volume: f32,
    tamanho: usize,
    marcado: bool,
    misto: bool,
    opcao: usize,
    copias: i64,
    nome: String,
    // Abas e popover
    abas: Vec<String>,
    aba: usize,
    popover: bool,
    popover_ancora: (f32, f32, f32, f32),
    // Tarefas
    tarefas: Vec<(String, bool)>,
    nova: String,
}

impl Default for Demo {
    fn default() -> Self {
        Self {
            secao: 1,
            busca: String::new(),
            visao: 0,
            notificacoes: true,
            som: false,
            volume: 60.0,
            tamanho: 1,
            marcado: true,
            misto: false,
            opcao: 0,
            copias: 2,
            nome: String::new(),
            abas: vec!["Documentos".into(), "Downloads".into(), "Projetos".into()],
            aba: 0,
            popover: false,
            popover_ancora: (0.0, 0.0, 0.0, 0.0),
            tarefas: vec![("Testar o mide-ui".into(), true), ("Criar meu primeiro app".into(), false)],
            nova: String::new(),
        }
    }
}

impl App for Demo {
    fn view(&mut self, ui: &mut Ui) {
        let secoes = [
            ("Componentes", ""),
            ("Botões", "input-mouse-symbolic"),
            ("Controles", "preferences-system-symbolic"),
            ("Indicadores", "dialog-information-symbolic"),
            ("Exemplo", ""),
            ("Tarefas", "view-list-symbolic"),
            ("Sobre", "help-about-symbolic"),
        ];
        ui.glass_sidebar();
        self.secao = ui.sidebar_area(&secoes, self.secao, |ui, secao| {
            let titulo = secoes[secao].0;
            let mut visao = self.visao;
            let mut busca = std::mem::take(&mut self.busca);
            ui.toolbar(|ui| {
                ui.toolbar_buttons(&[("go-previous-symbolic", "‹", false, false), ("go-next-symbolic", "›", false, false)]);
                let right = 124.0 + 10.0 + 180.0;
                let room = (ui.remaining_width() - right - 28.0).max(40.0);
                ui.toolbar_title(titulo, room);
                ui.push_right(right);
                if let Some(i) = ui.toolbar_buttons(&[
                    ("view-grid-symbolic", "▦", visao == 0, true),
                    ("view-list-symbolic", "☰", visao == 1, true),
                    ("view-column-symbolic", "▥", visao == 2, true),
                ]) {
                    visao = i;
                }
                ui.sized(180.0, |ui| {
                    ui.search_field(&mut busca, "Buscar");
                });
            });
            self.visao = visao;
            self.busca = busca;
            ui.page(|ui| match secao {
                1 => self.botoes(ui),
                2 => self.controles(ui),
                3 => self.indicadores(ui),
                5 => self.lista(ui),
                _ => sobre(ui),
            });
        });
        // Popover por cima de tudo (no fim do `view`).
        let ancora = self.popover_ancora;
        ui.popover(&mut self.popover, ancora, 260.0, |ui| {
            ui.strong("Popover");
            ui.note("Painel de vidro preso ao botão; fecha com clique fora ou Esc.");
            let mut ligado = true;
            ui.switch("Opção", &mut ligado);
        });
    }
}

impl Demo {
    fn botoes(&mut self, ui: &mut Ui) {
        ui.heading("Estilos (todos em cápsula)");
        ui.row(|ui| {
            ui.button_styled("Principal", ButtonStyle::Prominent);
            ui.button_styled("Comum", ButtonStyle::Bordered);
            ui.button_styled("Vidro", ButtonStyle::Glass);
            ui.button_styled("Só texto", ButtonStyle::Plain);
            ui.button_styled("Apagar", ButtonStyle::Destructive);
        });
        ui.heading("Barra de ferramentas");
        ui.row(|ui| {
            ui.toolbar_buttons(&[("view-dual-symbolic", "◨", true, true), ("folder-new-symbolic", "+", false, true), ("send-to-symbolic", "↗", false, true)]);
        });
    }

    fn controles(&mut self, ui: &mut Ui) {
        ui.heading("Interruptores e deslizante");
        ui.card(|ui| {
            ui.switch("Notificações", &mut self.notificacoes);
            ui.switch_detail("Efeitos sonoros", "Tocar um som nos alertas", &mut self.som);
            ui.slider("Volume", &mut self.volume, 0.0, 100.0);
        });
        ui.heading("Controle segmentado");
        ui.segmented(&["Pequeno", "Médio", "Grande"], &mut self.tamanho, Some(320.0));
        ui.heading("Caixas de seleção e opções");
        ui.row(|ui| {
            ui.checkbox("Mostrar extensões", &mut self.marcado);
            ui.checkbox("Mostrar ocultos", &mut self.misto);
        });
        ui.row(|ui| {
            for (i, nome) in ["Ícones", "Lista", "Colunas"].iter().enumerate() {
                if ui.radio(nome, self.opcao == i).clicked() {
                    self.opcao = i;
                }
            }
        });
        ui.heading("Passo e campo de texto");
        ui.row(|ui| {
            ui.stepper(&mut self.copias, 1, 99);
            ui.sized(240.0, |ui| {
                ui.text_input(&mut self.nome, "Seu nome");
            });
        });
    }

    fn lista(&mut self, ui: &mut Ui) {
        ui.row(|ui| {
            ui.note(&format!("{} de {} concluídas", self.tarefas.iter().filter(|t| t.1).count(), self.tarefas.len()));
        });
        ui.card(|ui| {
            let mut alternar = None;
            for (i, (nome, feita)) in self.tarefas.iter().enumerate() {
                let detalhe = if *feita { "✓ feita" } else { "a fazer" };
                if ui.list_item(nome, detalhe).clicked() {
                    alternar = Some(i);
                }
            }
            if let Some(i) = alternar {
                self.tarefas[i].1 = !self.tarefas[i].1;
            }
        });
        ui.heading("Nova tarefa");
        let enviar = ui.text_input(&mut self.nova, "O que precisa ser feito?").submitted();
        ui.row(|ui| {
            let adicionar = ui.primary_button("Adicionar").clicked();
            if (adicionar || enviar) && !self.nova.trim().is_empty() {
                self.tarefas.push((self.nova.trim().to_string(), false));
                self.nova.clear();
            }
            if ui.button("Limpar concluídas").clicked() {
                self.tarefas.retain(|t| !t.1);
            }
        });
    }
}

impl Demo {
    fn indicadores(&mut self, ui: &mut Ui) {
    ui.heading("Abas");
    match ui.tab_bar(&self.abas, self.aba) {
        TabAction::Select(i) => self.aba = i,
        TabAction::Close(i) => {
            self.abas.remove(i);
            self.aba = self.aba.min(self.abas.len().saturating_sub(1));
        }
        TabAction::New => {
            self.abas.push(format!("Aba {}", self.abas.len() + 1));
            self.aba = self.abas.len() - 1;
        }
        TabAction::None => {}
    }
    ui.heading("Popover e dica");
    ui.row(|ui| {
        let r = ui.button("Abrir popover");
        if r.clicked() {
            self.popover = !self.popover;
        }
        self.popover_ancora = ui.to_window(r.rect);
        let d = ui.button_styled("Passe o ponteiro", ButtonStyle::Glass);
        ui.tooltip(&d, "Uma dica do Tahoe");
    });
    ui.heading("Progresso");
    ui.progress_bar(Some(0.62));
    ui.progress_bar(None);
    ui.heading("Selos e etiquetas");
    ui.row(|ui| {
        ui.badge("3");
        ui.badge("12");
        ui.badge("Novo");
        ui.tag_dots(&TAG_COLORS[..3]);
        ui.tag_dots(&[TAG_COLORS[4]]);
    });
    }
}

fn sobre(ui: &mut Ui) {
    ui.label("O mide-ui é o jeito de criar apps com a mesma cara do mide, no visual do macOS Tahoe: janela de conteúdo inteiro, barra lateral flutuante de vidro e controles em cápsula.");
    ui.card(|ui| {
        ui.list_item("Componentes", "botões, interruptor, deslizante, segmentado, caixas, rádio, passo");
        ui.list_item("Estrutura", "barra lateral, barra de ferramentas, busca, página, cartão");
        ui.list_item("Mapa do visual", "docs/tahoe.md");
    });
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    run(
        Window::new("Demonstração do mide-ui").app_id("mide-ui-demo").size(900, 560).min_size(640, 400).full_size_content(),
        Demo::default(),
    )
}
