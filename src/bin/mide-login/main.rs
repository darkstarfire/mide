//! Tela de login do mide (C5): o greeter do `greetd`, com o visual da tela de
//! bloqueio (relógio, data, foto e senha sobre o papel de parede desfocado). Roda
//! dentro do compositor em modo de login (`mide --login`), pelo protocolo de bloqueio:
//! o compositor desenha o fundo, cobre todas as telas e só ela recebe o teclado.
//!
//! Como no GDM: escolher o usuário (com foto, ou "Outro usuário"), a senha e as outras
//! perguntas do PAM (código de verificação, senha vencida), a sessão (a do mide é a
//! padrão; a última de cada usuário fica lembrada), o layout de teclado, e desligar,
//! reiniciar ou repousar. Quem confere a senha é o greetd, pelo PAM: o greeter só
//! repassa o que a pessoa digita e nunca guarda nada.

mod greetd;
mod system;

use std::{
    collections::HashMap,
    error::Error,
    path::PathBuf,
    sync::{mpsc, Arc, Mutex},
    time::{Duration, Instant},
};

use chrono::{Local, Timelike};
use mide::tr;
use mide::{
    kit::{run, App, Key, Ui, Waker, Window},
    ui::{
        draw::{fill_rounded, rgba},
        text::FontWeight,
    },
};
use tiny_skia::Pixmap;

use greetd::{AuthKind, ErrorType, Request, Response};
use system::{Memory, Session, User};

/// O que a tela está mostrando.
#[derive(Debug, Clone, PartialEq)]
enum Stage {
    /// A lista de usuários.
    Choose,
    /// "Outro usuário": digitar o nome.
    OtherName,
    /// Uma pergunta do PAM (senha, código): `secret` esconde o que se digita.
    Ask { prompt: String, secret: bool },
    /// Esperando o greetd.
    Waiting,
}

/// Para que serve a resposta esperada do greetd.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Pending {
    /// `create_session` ou a resposta a uma pergunta.
    Auth,
    /// `start_session`.
    Start,
    /// `cancel_session`; `retry`: começar de novo com o mesmo usuário (senha recusada).
    Cancel { retry: bool },
}

struct Login {
    users: Vec<User>,
    /// Usuário escolhido (`login`); vazio na lista.
    login: String,
    /// O nome completo e a foto de quem está entrando.
    name: String,
    icon: Option<PathBuf>,
    /// Destaque do teclado na lista (o último é "Outro usuário").
    highlight: usize,
    other: String,
    input: String,
    stage: Stage,
    /// Aviso embaixo do campo; `true` em vermelho.
    message: Option<(String, bool)>,
    shake: Option<Instant>,
    focus_input: bool,
    sessions: Vec<Session>,
    session: usize,
    /// A sessão foi escolhida na lista nesta vez (vence a lembrada do usuário).
    session_chosen: bool,
    layouts: Vec<mide::keyboard_layouts::Choice>,
    layout: usize,
    memory: Memory,
    avatars: HashMap<(PathBuf, u32), Option<Pixmap>>,
    client: Arc<Mutex<Option<greetd::Client>>>,
    pending: Option<(Pending, mpsc::Receiver<Result<Response, String>>)>,
    started: bool,
}

impl Login {
    fn new() -> Self {
        let users = system::users();
        let sessions = system::sessions();
        let layouts = system::keyboard_layouts();
        let memory = Memory::load();
        tracing::info!("login: {} usuário(s), {} sessão(ões), {} layout(s)", users.len(), sessions.len(), layouts.len());
        let highlight = memory.last_user.as_ref().and_then(|u| users.iter().position(|x| &x.login == u)).unwrap_or(0);
        let layout = mide::control_client::ask_compositor("layout")
            .and_then(|l| l.first().and_then(|n| n.trim().parse().ok()))
            .unwrap_or(0);
        Self {
            users,
            login: String::new(),
            name: String::new(),
            icon: None,
            highlight,
            other: String::new(),
            input: String::new(),
            stage: Stage::Choose,
            message: None,
            shake: None,
            focus_input: false,
            sessions,
            session: 0,
            session_chosen: false,
            layouts,
            layout,
            memory,
            avatars: HashMap::new(),
            client: Arc::new(Mutex::new(None)),
            pending: None,
            started: false,
        }
    }

    /// Manda um pedido ao greetd numa thread (a conexão é aberta na primeira vez).
    fn ask(&mut self, request: Request, kind: Pending, waker: Option<Waker>) {
        let (tx, rx) = mpsc::channel();
        let client = self.client.clone();
        std::thread::spawn(move || {
            let result = (|| {
                let mut guard = client.lock().map_err(|_| "greetd: conexão indisponível".to_string())?;
                if guard.is_none() {
                    *guard = Some(greetd::Client::connect().map_err(|e| e.to_string())?);
                }
                let answer = guard.as_mut().map(|c| c.send(&request)).ok_or("greetd: sem conexão")?;
                if answer.is_err() {
                    // Conexão quebrada: a próxima tentativa conecta de novo.
                    *guard = None;
                }
                answer.map_err(|e| e.to_string())
            })();
            let _ = tx.send(result);
            if let Some(w) = waker {
                w.wake();
            }
        });
        self.pending = Some((kind, rx));
    }

    /// Começa o login de `login` (a sessão lembrada dele vira a escolhida).
    fn choose(&mut self, login: String, waker: Option<Waker>) {
        let user = self.users.iter().find(|u| u.login == login).cloned();
        self.name = user.as_ref().map(|u| u.name.clone()).unwrap_or_else(|| login.clone());
        self.icon = user.and_then(|u| u.icon);
        if let Some(i) = self.memory.session_of(&login).and_then(|id| self.sessions.iter().position(|s| s.id == id)).filter(|_| !self.session_chosen) {
            self.session = i;
        }
        tracing::info!("login: create_session {login}");
        self.login = login.clone();
        self.input.clear();
        self.message = None;
        self.stage = Stage::Waiting;
        self.ask(Request::CreateSession { username: login }, Pending::Auth, waker);
    }

    /// Volta para a lista (desiste do login em andamento).
    fn back(&mut self, waker: Option<Waker>) {
        self.input.clear();
        self.message = None;
        if self.pending.is_none() {
            self.ask(Request::CancelSession, Pending::Cancel { retry: false }, waker);
        }
        self.stage = Stage::Choose;
        self.login.clear();
    }

    /// Trata a resposta do greetd que chegou.
    fn answer(&mut self, kind: Pending, result: Result<Response, String>, ui: &mut Ui) {
        let waker = ui.waker();
        let response = match result {
            Ok(r) => r,
            Err(err) => {
                tracing::warn!("login: greetd não respondeu: {err}");
                self.message = Some((tr!("The login service is not responding").into_owned(), true));
                self.stage = Stage::Choose;
                return;
            }
        };
        match (kind, response) {
            (Pending::Auth, Response::AuthMessage { auth_message_type, auth_message }) => match auth_message_type {
                AuthKind::Secret | AuthKind::Visible => {
                    let secret = auth_message_type == AuthKind::Secret;
                    tracing::info!("login: pergunta do PAM ({}): {}", if secret { "secreta" } else { "visível" }, auth_message.trim());
                    self.stage = Stage::Ask { prompt: auth_message.trim().trim_end_matches(':').to_string(), secret };
                    self.input.clear();
                    self.focus_input = true;
                }
                AuthKind::Info | AuthKind::Error => {
                    tracing::info!("login: aviso do PAM: {}", auth_message.trim());
                    self.message = Some((auth_message.trim().to_string(), auth_message_type == AuthKind::Error));
                    self.ask(Request::PostAuthMessageResponse { response: None }, Pending::Auth, waker);
                }
            },
            (Pending::Auth, Response::Success) => {
                let Some(session) = self.sessions.get(self.session).cloned() else {
                    self.message = Some((tr!("No session to start").into_owned(), true));
                    return;
                };
                let (cmd, env) = session.command();
                tracing::info!("login: start_session '{}' para {}: {}", session.id, self.login, cmd.join(" "));
                self.stage = Stage::Waiting;
                self.ask(Request::StartSession { cmd, env }, Pending::Start, waker);
            }
            (Pending::Auth, Response::Error { error_type, description }) => {
                tracing::info!("login: recusado ({description})");
                self.message = Some(match error_type {
                    ErrorType::AuthError => (tr!("Incorrect password").into_owned(), true),
                    ErrorType::Error => (description, true),
                });
                self.shake = Some(Instant::now());
                // O greetd pede o cancelamento antes de outra tentativa.
                self.ask(Request::CancelSession, Pending::Cancel { retry: true }, waker);
            }
            (Pending::Start, Response::Success) => {
                if let Some(session) = self.sessions.get(self.session) {
                    tracing::info!("login: sessão '{}' iniciada para {}", session.id, self.login);
                    let (login, id) = (self.login.clone(), session.id.clone());
                    self.memory.remember(&login, &id);
                }
                self.started = true;
                ui.unlock();
            }
            (Pending::Start, Response::Error { description, .. }) => {
                tracing::warn!("login: a sessão não iniciou: {description}");
                self.message = Some((description, true));
                self.ask(Request::CancelSession, Pending::Cancel { retry: true }, waker);
            }
            (Pending::Cancel { retry }, _) => {
                if retry && !self.login.is_empty() {
                    let login = self.login.clone();
                    let message = self.message.take();
                    self.choose(login, waker);
                    self.message = message;
                } else {
                    self.stage = Stage::Choose;
                }
            }
            (_, other) => tracing::warn!("login: resposta inesperada do greetd: {other:?}"),
        }
    }

    /// Foto redonda de `path` em `px` pixels (guardada).
    fn avatar(&mut self, path: &PathBuf, px: u32) -> Option<&Pixmap> {
        self.avatars
            .entry((path.clone(), px))
            .or_insert_with(|| {
                let image = image::open(path).ok()?.to_rgba8();
                let image = image::imageops::resize(&image, px, px, image::imageops::FilterType::Triangle);
                let mut pixmap = Pixmap::new(px, px)?;
                for (i, p) in image.pixels().enumerate() {
                    let a = p[3] as u16;
                    let pm = |c: u8| ((c as u16 * a) / 255) as u8;
                    pixmap.pixels_mut()[i] = tiny_skia::PremultipliedColorU8::from_rgba(pm(p[0]), pm(p[1]), pm(p[2]), p[3]).unwrap_or(tiny_skia::PremultipliedColorU8::TRANSPARENT);
                }
                Some(pixmap)
            })
            .as_ref()
    }

    /// Desenha a foto (ou a inicial num círculo) de `size` pontos em (x, y).
    fn paint_avatar(&mut self, ui: &mut Ui, icon: Option<PathBuf>, name: &str, x: f32, y: f32, size: f32) {
        let k = mide::ui::draw::scale();
        let px = (size * k).round() as u32;
        let photo = icon.and_then(|p| self.avatar(&p, px).cloned());
        let initial = name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_else(|| "?".into());
        ui.paint_text(|pixmap, text, (ox, oy)| {
            let (x, y) = (ox + x, oy + y);
            match photo {
                Some(photo) => {
                    let shader = tiny_skia::Pattern::new(photo.as_ref(), tiny_skia::SpreadMode::Pad, tiny_skia::FilterQuality::Bilinear, 1.0, tiny_skia::Transform::from_translate(x * k, y * k));
                    let paint = tiny_skia::Paint { shader, anti_alias: true, ..Default::default() };
                    if let Some(circle) = tiny_skia::PathBuilder::from_circle((x + size / 2.0) * k, (y + size / 2.0) * k, size * k / 2.0) {
                        pixmap.fill_path(&circle, &paint, tiny_skia::FillRule::Winding, tiny_skia::Transform::identity(), None);
                    }
                }
                None => {
                    fill_rounded(pixmap, x, y, size, size, size / 2.0, rgba(255, 255, 255, 70));
                    let font = size * 0.44;
                    let w = text.measure(&initial, font, FontWeight::Semibold);
                    text.draw(pixmap, &initial, x + (size - w) / 2.0, y + (size - font * 1.2) / 2.0, font, FontWeight::Semibold, [255, 255, 255, 255]);
                }
            }
        });
    }

    /// A lista de usuários (e "Outro usuário"), centrada em `y`.
    fn user_list(&mut self, ui: &mut Ui, w: f32, y: f32) {
        const TILE: f32 = 132.0;
        const AVATAR: f32 = 88.0;
        let count = self.users.len() + 1;
        let x0 = (w - TILE * count as f32) / 2.0;
        // Teclado: setas trocam, Enter escolhe.
        if !ui.mirror() {
            if ui.take_key(Key::Left) {
                self.highlight = self.highlight.saturating_sub(1);
            }
            if ui.take_key(Key::Right) {
                self.highlight = (self.highlight + 1).min(count - 1);
            }
        }
        let enter = !ui.mirror() && ui.take_key(Key::Enter);
        let pointer = ui.pointer();
        let clicked = ui.input().released;
        let mut chosen = enter.then_some(self.highlight);
        for i in 0..count {
            let tx = x0 + i as f32 * TILE;
            let (icon, name) = match self.users.get(i) {
                Some(u) => (u.icon.clone(), u.name.clone()),
                None => (None, tr!("Other…").into_owned()),
            };
            let over = pointer.is_some_and(|(px, py)| px >= tx && px < tx + TILE && py >= y && py < y + AVATAR + 40.0);
            if over {
                self.highlight = i;
                if clicked {
                    chosen = Some(i);
                }
            }
            if i == self.highlight {
                // Foco visível (teclado e ponteiro): anel claro em volta da foto.
                let ring = AVATAR + 10.0;
                let (rx, ry) = (tx + (TILE - ring) / 2.0, y - 5.0);
                ui.paint_text(|pixmap, _, (ox, oy)| fill_rounded(pixmap, ox + rx, oy + ry, ring, ring, ring / 2.0, rgba(255, 255, 255, 110)));
            }
            self.paint_avatar(ui, icon, if i < self.users.len() { &name } else { "+" }, tx + (TILE - AVATAR) / 2.0, y, AVATAR);
            ui.paint_text(|pixmap, text, (ox, oy)| {
                let nw = text.measure(&name, 15.0, FontWeight::Semibold).min(TILE - 8.0);
                text.draw(pixmap, &name, ox + tx + (TILE - nw) / 2.0, oy + y + AVATAR + 12.0, 15.0, FontWeight::Semibold, [255, 255, 255, 255]);
            });
        }
        if let Some(i) = chosen.filter(|_| !ui.mirror()) {
            match self.users.get(i).map(|u| u.login.clone()) {
                Some(login) => self.choose(login, ui.waker()),
                None => {
                    self.stage = Stage::OtherName;
                    self.other.clear();
                    self.focus_input = true;
                    self.message = None;
                }
            }
        }
    }

    /// Sessão, layout de teclado e energia, embaixo.
    fn bottom_bar(&mut self, ui: &mut Ui, w: f32, h: f32) {
        let y = h - 72.0;
        // Cartões claros atrás das listas (o texto do kit é escuro).
        let card = |ui: &mut Ui, x: f32, width: f32| ui.paint_text(move |pixmap, _, (ox, oy)| fill_rounded(pixmap, ox + x - 8.0, oy + y - 6.0, width + 16.0, 44.0, 14.0, rgba(255, 255, 255, 205)));
        if self.sessions.len() > 1 {
            card(ui, 24.0, 280.0);
            let names: Vec<String> = self.sessions.iter().map(|s| s.name.clone()).collect();
            let mut session = self.session;
            ui.at(24.0, y, 280.0, |ui| {
                if ui.dropdown(&tr!("Session"), &names, &mut session).changed() {
                    tracing::info!("login: sessão escolhida: {}", names.get(session).cloned().unwrap_or_default());
                }
            });
            if session != self.session {
                self.session_chosen = true;
            }
            self.session = session;
        }
        if self.layouts.len() > 1 {
            card(ui, w - 24.0 - 240.0, 240.0);
            let names: Vec<String> = self.layouts.iter().map(|l| l.name()).collect();
            let mut layout = self.layout;
            ui.at(w - 24.0 - 240.0, y, 240.0, |ui| {
                if ui.dropdown(&tr!("Keyboard"), &names, &mut layout).changed() {
                    mide::control_client::ask_compositor(&format!("layout {layout}"));
                    tracing::info!("login: layout de teclado {layout}");
                }
            });
            self.layout = layout;
        }
        // Energia, no meio, como na tela de login do Mac.
        let actions = [
            (tr!("Sleep").into_owned(), mide::system::Power::Sleep, "repousar"),
            (tr!("Restart").into_owned(), mide::system::Power::Restart, "reiniciar"),
            (tr!("Shut Down").into_owned(), mide::system::Power::Shutdown, "desligar"),
        ];
        let bw = 120.0;
        let x0 = (w - bw * actions.len() as f32 - 12.0 * (actions.len() - 1) as f32) / 2.0;
        for (i, (label, action, log)) in actions.into_iter().enumerate() {
            let x = x0 + i as f32 * (bw + 12.0);
            ui.at(x, y, bw, |ui| {
                // Cápsula de vidro: legível sobre o papel de parede (o botão comum sumia).
                if ui.button_styled(&label, mide::kit::ButtonStyle::Glass).clicked() && !ui.mirror() {
                    tracing::info!("login: {log}");
                    mide::system::power(action);
                }
            });
        }
    }
}

impl App for Login {
    fn view(&mut self, ui: &mut Ui) {
        let (_, _, w, h) = ui.region();
        // Respostas do greetd: só na tela principal (as outras são cópias).
        if !ui.mirror() {
            if let Some((kind, rx)) = self.pending.take() {
                match rx.try_recv() {
                    Ok(result) => self.answer(kind, result, ui),
                    Err(mpsc::TryRecvError::Empty) => self.pending = Some((kind, rx)),
                    Err(mpsc::TryRecvError::Disconnected) => self.answer(kind, Err("thread do greetd caiu".into()), ui),
                }
            }
            if self.started {
                return;
            }
            // Um usuário só: já começa nele, como o GDM.
            if self.stage == Stage::Choose && self.login.is_empty() && self.users.len() == 1 && self.pending.is_none() && self.message.is_none() {
                let login = self.users[0].login.clone();
                self.choose(login, ui.waker());
            }
        }
        ui.wake_after(Duration::from_secs(1));
        let shaking = self.shake.is_some_and(|t| t.elapsed() < Duration::from_millis(450));
        if shaking {
            ui.request_tick();
        }
        let dx = self.shake.filter(|_| shaking).map_or(0.0, |t| {
            let s = t.elapsed().as_secs_f32();
            (s * 40.0).sin() * 12.0 * (1.0 - s / 0.45)
        });

        let now = Local::now();
        let time = format!("{:02}:{:02}", now.hour(), now.minute());
        let date = mide::calendar::long_date(now.date_naive());
        let (cx, top) = (w / 2.0, h * 0.12);
        ui.paint_text(|pixmap, text, (ox, oy)| {
            let tw = text.measure(&time, 84.0, FontWeight::Semibold);
            text.draw(pixmap, &time, ox + cx - tw / 2.0, oy + top, 84.0, FontWeight::Semibold, [255, 255, 255, 255]);
            let dw = text.measure(&date, 20.0, FontWeight::Regular);
            text.draw(pixmap, &date, ox + cx - dw / 2.0, oy + top + 110.0, 20.0, FontWeight::Regular, [255, 255, 255, 220]);
        });

        let field_w = 260.0;
        let field_y = h * 0.58;
        if self.stage == Stage::Choose {
            self.user_list(ui, w, h * 0.40);
        } else {
            // Quem está entrando: foto e nome, acima do campo.
            let avatar = 96.0;
            let (icon, name) = match self.stage {
                Stage::OtherName => (None, tr!("Other…").into_owned()),
                _ => (self.icon.clone(), self.name.clone()),
            };
            self.paint_avatar(ui, icon, &name, cx - avatar / 2.0, field_y - 150.0, avatar);
            ui.paint_text(|pixmap, text, (ox, oy)| {
                let nw = text.measure(&name, 17.0, FontWeight::Semibold);
                text.draw(pixmap, &name, ox + cx - nw / 2.0, oy + field_y - 40.0, 17.0, FontWeight::Semibold, [255, 255, 255, 255]);
            });
            if self.focus_input {
                ui.focus_next_input();
                self.focus_input = false;
            }
            let waker = ui.waker();
            let stage = self.stage.clone();
            let mut submitted = false;
            ui.at(cx - field_w / 2.0 + dx, field_y, field_w, |ui| match &stage {
                Stage::OtherName => submitted = ui.text_input(&mut self.other, &tr!("Name")).submitted(),
                Stage::Ask { prompt, secret } => {
                    let lower = prompt.to_lowercase();
                    let placeholder = if *secret && (lower.starts_with("pass") || lower.starts_with("senha")) {
                        tr!("Enter Password").into_owned()
                    } else {
                        prompt.clone()
                    };
                    submitted = if *secret { ui.password_input(&mut self.input, &placeholder) } else { ui.text_input(&mut self.input, &placeholder) }.submitted();
                }
                _ => {
                    let mut empty = String::new();
                    let _ = ui.password_input(&mut empty, &tr!("Checking…"));
                }
            });
            // Voltar à lista (Esc ou o botão), com mais de um usuário.
            let can_back = self.users.len() > 1 || self.stage == Stage::OtherName;
            if can_back {
                let mut back = !ui.mirror() && ui.take_key(Key::Escape);
                ui.at(cx - field_w / 2.0 - 52.0, field_y, 40.0, |ui| {
                    back |= ui.button_styled("←", mide::kit::ButtonStyle::Glass).clicked();
                });
                if back {
                    self.back(waker.clone());
                }
            }
            if submitted && !ui.mirror() {
                match self.stage.clone() {
                    Stage::OtherName if !self.other.trim().is_empty() => {
                        let login = self.other.trim().to_string();
                        self.choose(login, waker);
                    }
                    Stage::Ask { .. } if self.pending.is_none() => {
                        let response = std::mem::take(&mut self.input);
                        self.stage = Stage::Waiting;
                        self.message = None;
                        self.ask(Request::PostAuthMessageResponse { response: Some(response) }, Pending::Auth, waker);
                    }
                    _ => {}
                }
            }
        }
        if let Some((message, error)) = self.message.clone() {
            let color = if error { [255, 220, 220, 255] } else { [255, 255, 255, 230] };
            ui.paint_text(|pixmap, text, (ox, oy)| {
                let mw = text.measure(&message, 14.0, FontWeight::Regular);
                text.draw(pixmap, &message, ox + cx - mw / 2.0, oy + field_y + 52.0, 14.0, FontWeight::Regular, color);
            });
        }
        self.bottom_bar(ui, w, h);
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let _logging = mide::logging::init("mide-login", "mide_login=info");
    run(Window::new(&mide::i18n::trc("window title", "Login")).app_id("mide-login").lock(), Login::new())
}
