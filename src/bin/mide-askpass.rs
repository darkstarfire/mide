//! Janela de senha de administrador do mide (C7). Dois modos:
//!
//! - `mide-askpass "<pergunta do sudo>"`: o `SUDO_ASKPASS` do `mide-sudo`. Mostra a
//!   janela, escreve a senha digitada na saída padrão (que o sudo lê) e sai com 0;
//!   Cancelar, Esc ou fechar a janela saem com 1. O sudo chama de novo a cada tentativa:
//!   a contagem fica no arquivo de `MIDE_ASKPASS_STATE` (0600, criado pelo `mide-sudo`),
//!   e da segunda vez em diante a janela já abre com "Senha incorreta".
//! - `mide-askpass --agent`: a janela do agente do polkit (`mide-polkit-agent`). Recebe
//!   as mensagens pela entrada padrão e responde `pass <senha>` ou `cancel` pela saída
//!   padrão (`mide::elevate::{ToWindow, FromWindow}`); continua aberta entre as
//!   tentativas.
//!
//! Quem pede e o comando vêm de `MIDE_ELEVATE_TITLE`, `MIDE_ELEVATE_ICON` e
//! `MIDE_ELEVATE_COMMAND` (só para mostrar). A senha nunca vai para o log nem para
//! arquivo; o processo não gera dump de memória e apaga a senha antes de sair.

use std::{
    io::BufRead,
    os::fd::AsFd,
    path::PathBuf,
    sync::{mpsc, Arc, Mutex},
    time::{Duration, Instant},
};

use mide::{
    elevate::{self, FromWindow, ToWindow},
    kit::{run, App, Key, Ui, Waker, Window},
    ui::draw::{fill_rounded, rgba, tf},
};
use tiny_skia::{Paint, Pixmap, Stroke};

/// Espaço reservado para a senha no campo (bytes).
const PASSWORD_CAPACITY: usize = 1024;

/// Mensagens que chegam do agente (modo `--agent`), com o fim da entrada.
enum Incoming {
    Msg(ToWindow),
    Eof,
}

struct Ask {
    /// Quem pede ("XAMPP quer fazer alterações no sistema.").
    message: String,
    command: String,
    user: String,
    icon: Option<String>,
    password: String,
    error: Option<String>,
    info: Option<String>,
    /// Quando a senha foi recusada (o painel balança).
    shake: Option<Instant>,
    /// Modo do agente: a ligação com ele e se já pode responder.
    agent: Option<AgentLink>,
    /// Resposta final (modo do sudo): a senha ou `None` (cancelou).
    answer: Arc<Mutex<Option<String>>>,
}

struct AgentLink {
    rx: mpsc::Receiver<Incoming>,
    waker: Arc<Mutex<Option<Waker>>>,
    /// O agente pediu a senha (`ask`).
    ready: bool,
    /// Enter apertado antes de o agente pedir: manda assim que ele pedir.
    pending: bool,
    /// Senha enviada, esperando a conferência.
    busy: bool,
    placeholder: String,
}

impl Ask {
    fn send(&self, msg: &FromWindow) {
        // Direto no descritor (sem o buffer do stdout, que guardaria a senha) e a linha
        // apagada depois.
        if let Some(mut line) = msg.to_line() {
            let _ = elevate::write_all_fd(std::io::stdout().as_fd(), line.as_bytes());
            elevate::wipe(&mut line);
        }
    }

    fn submit_to_agent(&mut self) {
        let mut pass = std::mem::replace(&mut self.password, String::with_capacity(PASSWORD_CAPACITY));
        let msg = FromWindow::Pass(pass.clone());
        self.send(&msg);
        if let FromWindow::Pass(mut p) = msg {
            elevate::wipe(&mut p);
        }
        elevate::wipe(&mut pass);
        tracing::info!("senha enviada ao agente");
        if let Some(link) = self.agent.as_mut() {
            link.ready = false;
            link.pending = false;
            link.busy = true;
        }
        self.error = None;
    }

    fn cancel(&mut self, ui: &mut Ui) {
        tracing::info!("pedido de senha cancelado");
        if self.agent.is_some() {
            self.send(&FromWindow::Cancel);
        } else if let Some(state) = state_path() {
            let _ = elevate::mark_cancelled(&state);
        }
        elevate::wipe(&mut self.password);
        ui.close();
    }

    /// Mensagens do agente que chegaram.
    fn poll_agent(&mut self, ui: &mut Ui) {
        let Some(link) = self.agent.as_mut() else {
            return;
        };
        if let Ok(mut w) = link.waker.lock()
            && w.is_none()
        {
            *w = ui.waker();
        }
        let mut close = false;
        let mut submit = false;
        while let Ok(incoming) = link.rx.try_recv() {
            match incoming {
                Incoming::Eof | Incoming::Msg(ToWindow::Close) => close = true,
                Incoming::Msg(ToWindow::Message(t)) => self.message = t,
                Incoming::Msg(ToWindow::Command(t)) => self.command = t,
                Incoming::Msg(ToWindow::User(t)) => self.user = t,
                Incoming::Msg(ToWindow::Info(t)) => self.info = Some(t).filter(|t| !t.is_empty()),
                Incoming::Msg(ToWindow::Error(t)) => {
                    self.error = Some(t);
                    self.shake = Some(Instant::now());
                    link.busy = false;
                }
                Incoming::Msg(ToWindow::Busy) => link.busy = true,
                Incoming::Msg(ToWindow::Ask(prompt)) => {
                    link.ready = true;
                    link.busy = false;
                    link.placeholder = placeholder_for(&prompt);
                    submit = link.pending && !self.password.is_empty();
                }
            }
            ui.request_repaint();
        }
        if close {
            tracing::info!("o agente fechou a janela");
            elevate::wipe(&mut self.password);
            ui.close();
        } else if submit {
            self.submit_to_agent();
        }
    }
}

/// A pergunta do PAM como dica do campo ("Senha:" → "Senha").
fn placeholder_for(prompt: &str) -> String {
    let p = prompt.trim().trim_end_matches(':').trim();
    if p.is_empty() || p.eq_ignore_ascii_case("password") { "Senha".into() } else { p.to_string() }
}

fn state_path() -> Option<PathBuf> {
    std::env::var_os(elevate::STATE_VAR).filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// Cadeado desenhado pelo mide (sem depender do tema de ícones), em pontos.
fn draw_lock(pixmap: &mut Pixmap, x: f32, y: f32, size: f32, body: [u8; 4], shackle: [u8; 4], detail: [u8; 4]) {
    let k = size / 56.0;
    // Alça.
    let mut pb = tiny_skia::PathBuilder::new();
    let (cx, top, r) = (x + 28.0 * k, y + 6.0 * k, 12.0 * k);
    pb.move_to(cx - r, y + 26.0 * k);
    pb.line_to(cx - r, top + r);
    pb.cubic_to(cx - r, top + r * 0.45, cx - r * 0.55, top, cx, top);
    pb.cubic_to(cx + r * 0.55, top, cx + r, top + r * 0.45, cx + r, top + r);
    pb.line_to(cx + r, y + 26.0 * k);
    if let Some(path) = pb.finish() {
        let mut paint = Paint { anti_alias: true, ..Default::default() };
        paint.set_color(rgba(shackle[0], shackle[1], shackle[2], shackle[3]));
        pixmap.stroke_path(&path, &paint, &Stroke { width: 5.0 * k, ..Default::default() }, tf(), None);
    }
    // Corpo e buraco da chave.
    fill_rounded(pixmap, x + 8.0 * k, y + 24.0 * k, 40.0 * k, 30.0 * k, 7.0 * k, rgba(body[0], body[1], body[2], body[3]));
    fill_rounded(pixmap, cx - 3.5 * k, y + 32.0 * k, 7.0 * k, 7.0 * k, 3.5 * k, rgba(detail[0], detail[1], detail[2], detail[3]));
    fill_rounded(pixmap, cx - 1.75 * k, y + 37.0 * k, 3.5 * k, 9.0 * k, 1.5 * k, rgba(detail[0], detail[1], detail[2], detail[3]));
}

impl App for Ask {
    fn view(&mut self, ui: &mut Ui) {
        self.poll_agent(ui);
        let t = ui.theme().clone();
        let shaking = self.shake.is_some_and(|s| s.elapsed() < Duration::from_millis(450));
        if shaking {
            ui.request_tick();
        }
        let dx = self.shake.filter(|_| shaking).map_or(0.0, |s| {
            let s = s.elapsed().as_secs_f32();
            (s * 40.0).sin() * 10.0 * (1.0 - s / 0.45)
        });
        // O agente pode responder a qualquer momento: olha a fila de vez em quando
        // (além do aviso da thread que lê a entrada).
        if self.agent.is_some() {
            ui.wake_after(Duration::from_millis(250));
        }
        let (_, _, w, _) = ui.region();
        let pad = 20.0;
        let icon = 56.0;
        let accent = t.accent;
        let shackle = t.text_secondary;
        let hole = t.on_accent;
        let app_icon = self.icon.clone();
        ui.at(pad + dx, pad, icon, |ui| {
            ui.custom(icon, |pixmap, rect| draw_lock(pixmap, rect.0, rect.1, icon, accent, shackle, hole));
        });
        if let Some(name) = app_icon {
            // O ícone do app que pede, pequeno, no canto do cadeado.
            ui.at(pad + dx + icon - 20.0, pad + icon - 20.0, 26.0, |ui| ui.icon(&name, 26));
        }
        let x = pad + icon + 16.0;
        let width = (w - x - pad).max(120.0);
        let mut ok = false;
        let mut cancel = false;
        let busy = self.agent.as_ref().is_some_and(|l| l.busy);
        let placeholder = match &self.agent {
            Some(l) if l.busy => "Conferindo…".to_string(),
            Some(l) => l.placeholder.clone(),
            None => "Senha".to_string(),
        };
        let error = self.error.is_some();
        ui.at(x + dx, pad - 4.0, width, |ui| {
            ui.title("Digite sua senha");
            ui.label(&self.message);
            let who = if self.user.is_empty() { "Digite a senha de administrador para permitir.".to_string() } else { format!("Digite a senha de {} para permitir.", self.user) };
            ui.note(&who);
            if !self.command.is_empty() {
                ui.note(&format!("Comando: {}", self.command));
            }
            if let Some(info) = &self.info {
                ui.note(info);
            }
            if let Some(err) = &self.error {
                ui.error_note(err);
            }
            if busy {
                let mut empty = String::new();
                let _ = ui.password_field(&mut empty, &placeholder, false);
            } else if ui.password_field(&mut self.password, &placeholder, error).submitted() {
                ok = true;
            }
            ui.space(4.0);
            match ui.dialog_buttons(&["Cancelar", "OK"]) {
                Some(0) => cancel = true,
                Some(_) => ok = true,
                None => {}
            }
        });
        if ui.input().key(Key::Escape) {
            cancel = true;
        }
        if cancel {
            self.cancel(ui);
            return;
        }
        if ok && !busy && !self.password.is_empty() {
            match self.agent.as_mut() {
                Some(link) if !link.ready => link.pending = true,
                Some(_) => self.submit_to_agent(),
                None => {
                    tracing::info!("senha digitada");
                    if let Ok(mut answer) = self.answer.lock() {
                        *answer = Some(std::mem::take(&mut self.password));
                    }
                    ui.close();
                }
            }
            ui.request_repaint();
        }
    }

    fn close_requested(&mut self) -> bool {
        // Fechar pela barra é cancelar.
        tracing::info!("janela de senha fechada");
        if self.agent.is_some() {
            self.send(&FromWindow::Cancel);
        } else if let Some(state) = state_path() {
            let _ = elevate::mark_cancelled(&state);
        }
        elevate::wipe(&mut self.password);
        true
    }
}

fn main() {
    // O log vai para o stderr: o stdout é a senha (para o sudo) ou o protocolo do agente.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "mide_askpass=info".into()))
        .init();
    // Sem dump de memória (a senha estaria nele) e ninguém do mesmo usuário lê a memória
    // pelo ptrace sem ser root.
    let _ = rustix::process::set_dumpable_behavior(rustix::process::DumpableBehavior::NotDumpable);
    let agent_mode = std::env::args_os().nth(1).is_some_and(|a| a == "--agent");
    let env_text = |name: &str| std::env::var(name).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let title = env_text(elevate::TITLE_VAR);
    let message = match &title {
        Some(app) => format!("{app} quer fazer alterações no sistema."),
        None => "Um programa quer fazer alterações no sistema.".into(),
    };
    let answer: Arc<Mutex<Option<String>>> = Arc::default();
    let mut ask = Ask {
        message,
        command: env_text(elevate::COMMAND_VAR).unwrap_or_default(),
        user: elevate::current_user(),
        icon: env_text(elevate::ICON_VAR),
        // Reservada antes: o campo não cresce enquanto se digita (cada realocação
        // deixaria uma cópia da senha solta na memória).
        password: String::with_capacity(PASSWORD_CAPACITY),
        error: None,
        info: None,
        shake: None,
        agent: None,
        answer: answer.clone(),
    };
    if agent_mode {
        let (tx, rx) = mpsc::channel();
        let waker: Arc<Mutex<Option<Waker>>> = Arc::default();
        let thread_waker = waker.clone();
        std::thread::spawn(move || {
            let stdin = std::io::stdin();
            for line in stdin.lock().lines() {
                let Ok(line) = line else { break };
                if let Some(msg) = ToWindow::parse(&line)
                    && tx.send(Incoming::Msg(msg)).is_err()
                {
                    return;
                }
                if let Some(w) = thread_waker.lock().ok().and_then(|w| w.clone()) {
                    w.wake();
                }
            }
            let _ = tx.send(Incoming::Eof);
            if let Some(w) = thread_waker.lock().ok().and_then(|w| w.clone()) {
                w.wake();
            }
        });
        ask.agent = Some(AgentLink { rx, waker, ready: false, pending: false, busy: false, placeholder: "Senha".into() });
    } else if let Some(state) = state_path() {
        // Da segunda chamada do sudo em diante, a anterior foi recusada.
        match elevate::bump_attempt(&state) {
            Ok(n) if n > 1 => {
                ask.error = Some("Senha incorreta. Tente de novo.".into());
                ask.shake = Some(Instant::now());
                tracing::info!("tentativa {n}: a anterior foi recusada");
            }
            Ok(_) => {}
            Err(err) => tracing::warn!("estado das tentativas indisponível: {err}"),
        }
    }
    let window = Window::new("Senha de administrador").app_id("mide-auth").size(480, 262).min_size(400, 240);
    if let Err(err) = run(window, ask) {
        eprintln!("mide-askpass: {err}");
        std::process::exit(1);
    }
    if agent_mode {
        return;
    }
    let taken = answer.lock().ok().and_then(|mut a| a.take());
    match taken {
        Some(mut password) => {
            // Direto no descritor, de um buffer apagado depois (o do stdout não seria).
            let mut line: Vec<u8> = Vec::with_capacity(password.len() + 1);
            line.extend_from_slice(password.as_bytes());
            line.push(b'\n');
            let ok = elevate::write_all_fd(std::io::stdout().as_fd(), &line);
            elevate::wipe_bytes(&mut line);
            elevate::wipe(&mut password);
            if ok.is_err() {
                std::process::exit(1);
            }
        }
        None => std::process::exit(1),
    }
}
