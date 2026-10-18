//! Demonstração do mide-ui: um app pequeno que usa todos os componentes.
//!
//! Rodar dentro do mide: `cargo run --example mide-ui-demo`

use mide::kit::{run, App, Ui, Window};

struct Demo {
    secao: usize,
    // Controles
    notificacoes: bool,
    modo_escuro: bool,
    volume: f32,
    nome: String,
    saudacao: String,
    // Tarefas
    tarefas: Vec<(String, bool)>,
    nova: String,
}

impl Default for Demo {
    fn default() -> Self {
        Self {
            secao: 0,
            notificacoes: true,
            modo_escuro: false,
            volume: 60.0,
            nome: String::new(),
            saudacao: String::new(),
            tarefas: vec![("Testar o mide-ui".into(), true), ("Criar meu primeiro app".into(), false)],
            nova: String::new(),
        }
    }
}

impl App for Demo {
    fn view(&mut self, ui: &mut Ui) {
        let secoes = [("Controles", "preferences-system"), ("Tarefas", "view-list"), ("Sobre", "help-about")];
        self.secao = ui.sidebar(&secoes, self.secao, |ui, secao| match secao {
            0 => self.controles(ui),
            1 => self.lista(ui),
            _ => sobre(ui),
        });
    }
}

impl Demo {
    fn controles(&mut self, ui: &mut Ui) {
        ui.title("Controles");
        ui.heading("Preferências");
        ui.card(|ui| {
            ui.switch("Notificações", &mut self.notificacoes);
            ui.switch("Modo escuro", &mut self.modo_escuro);
            ui.slider("Volume", &mut self.volume, 0.0, 100.0);
        });
        ui.heading("Seu nome");
        if ui.text_input(&mut self.nome, "Digite e aperte Enter").submitted() || ui.primary_button("Cumprimentar").clicked() {
            self.saudacao = if self.nome.trim().is_empty() {
                "Escreva um nome primeiro.".into()
            } else {
                format!("Olá, {}! Bem-vindo ao mide.", self.nome.trim())
            };
        }
        if !self.saudacao.is_empty() {
            ui.label(&self.saudacao);
        }
        ui.note(&format!(
            "Estado: notificações {}, modo escuro {}, volume {:.0}.",
            if self.notificacoes { "ligadas" } else { "desligadas" },
            if self.modo_escuro { "ligado" } else { "desligado" },
            self.volume
        ));
    }

    fn lista(&mut self, ui: &mut Ui) {
        ui.title("Tarefas");
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

fn sobre(ui: &mut Ui) {
    ui.title("Sobre o mide-ui");
    ui.label("O mide-ui é o jeito de criar apps com a mesma cara do mide: os componentes, as cores e o comportamento das Configurações do Sistema.");
    ui.label("Cada quadro, o app descreve a tela no `view`. Os componentes respondem se foram clicados ou mudaram, e o estado fica no próprio app.");
    ui.card(|ui| {
        ui.list_item("Componentes", "botão, interruptor, trilho, campo, lista");
        ui.list_item("Layout", "coluna, linha, cartão, página, barra lateral");
        ui.list_item("Tema", "cores, cantos e fontes num lugar só");
    });
    ui.note("Guia completo: docs/mide-ui.md");
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    run(Window::new("Demonstração do mide-ui").app_id("mide-ui-demo").size(760, 520).min_size(560, 380), Demo::default())
}
