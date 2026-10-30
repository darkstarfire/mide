//! Tela de login do mide (C5): o greeter do `greetd`, no estilo da janela de início de
//! sessão do macOS Sonoma/Tahoe (data e hora grandes em cima, as fotos dos usuários
//! embaixo, o campo de senha de vidro, Desligar/Reiniciar/Repousar em botões redondos,
//! sessão, layout do teclado, Wi-Fi e bateria no canto de cima à direita). Roda
//! dentro do compositor em modo de login (`mide --login`), pelo protocolo de bloqueio:
//! o compositor desenha o fundo, cobre todas as telas e só ela recebe o teclado.
//!
//! Como no Mac e no GDM: lista de usuários (com foto, ou "Outro…") ou campos de nome e
//! senha (`[login] show = "name"`), a senha e as outras perguntas do PAM (código de
//! verificação, senha vencida), a dica de senha depois de três tentativas erradas, o
//! aviso de Caps Lock, a mensagem da tela de bloqueio, a sessão (a do mide é a padrão;
//! a última de cada usuário fica lembrada), o layout de teclado, e desligar, reiniciar
//! ou repousar. As opções vêm da seção `[login]` (Configurações → Usuários e grupos). Quem confere a senha é o greetd, pelo PAM: o greeter só
//! repassa o que a pessoa digita e nunca guarda nada.

mod greetd;
mod system;

use std::{
    error::Error,
    path::PathBuf,
    sync::{mpsc, Arc, Mutex},
    time::{Duration, Instant},
};

use mide::tr;
use mide::{
    kit::{run, App, Key, LockField, Ui, Waker, Window},
    lockscreen::{self, Avatars, LoginOptions},
    ui::text::FontWeight,
};

use greetd::{AuthKind, ErrorType, Request, Response};
use system::{Memory, Session, User};

/// O que a tela está mostrando.
#[derive(Debug, Clone, PartialEq)]
enum Stage {
    /// A lista de usuários.
    Choose,
    /// Campos de nome e senha (`[login] show = "name"`, ou "Outro…" na lista).
    Credentials,
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
    /// `cancel_session`, e o que fazer depois.
    Cancel(After),
}

/// Para onde a tela vai depois de cancelar o login no greetd.
#[derive(Debug, Clone, Copy, PartialEq)]
enum After {
    /// Começar de novo com o mesmo usuário (senha recusada).
    Retry,
    /// Voltar à lista de usuários.
    List,
    /// Voltar aos campos de nome e senha (senha recusada nesse modo).
    Credentials,
}

/// Menu aberto no canto de cima.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Menu {
    Session,
    Layout,
}

struct Login {
    users: Vec<User>,
    /// Usuário escolhido (`login`); vazio na lista.
    login: String,
    /// O nome completo e a foto de quem está entrando.
    name: String,
    icon: Option<PathBuf>,
    hint: String,
    /// Senhas recusadas seguidas deste usuário (a dica aparece na terceira).
    failures: u32,
    hint_logged: bool,
    /// Destaque do teclado na lista (o último é "Outro usuário").
    highlight: usize,
    /// Nome digitado nos campos de nome e senha.
    other: String,
    input: String,
    /// Nos campos de nome e senha: o foco está no nome (`true`) ou na senha.
    name_focus: bool,
    /// Senha digitada junto com o nome: vai na primeira pergunta secreta do PAM.
    queued: Option<String>,
    /// O login começou pelos campos de nome e senha.
    from_credentials: bool,
    options: LoginOptions,
    status: lockscreen::Status,
    menu: Option<(Menu, (f32, f32, f32, f32))>,
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
    avatars: Avatars,
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
        let options = LoginOptions::load();
        tracing::info!(
            "login: {} usuário(s), {} sessão(ões), {} layout(s); {}",
            users.len(),
            sessions.len(),
            layouts.len(),
            if options.list { "lista de usuários" } else { "nome e senha" }
        );
        let highlight = memory.last_user.as_ref().and_then(|u| users.iter().position(|x| &x.login == u)).unwrap_or(0);
        let layout = mide::control_client::ask_compositor("layout")
            .and_then(|l| l.first().and_then(|n| n.trim().parse().ok()))
            .unwrap_or(0);
        Self {
            users,
            login: String::new(),
            name: String::new(),
            icon: None,
            hint: String::new(),
            failures: 0,
            hint_logged: false,
            highlight,
            other: String::new(),
            input: String::new(),
            name_focus: true,
            queued: None,
            from_credentials: false,
            stage: if options.list { Stage::Choose } else { Stage::Credentials },
            options,
            status: lockscreen::Status::start(),
            menu: None,
            message: None,
            shake: None,
            focus_input: false,
            sessions,
            session: 0,
            session_chosen: false,
            layouts,
            layout,
            memory,
            avatars: Avatars::default(),
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
        if login != self.login {
            self.failures = 0;
        }
        self.name = user.as_ref().map(|u| u.name.clone()).unwrap_or_else(|| login.clone());
        self.hint = user.as_ref().map(|u| u.hint.clone()).unwrap_or_default();
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

    /// Volta para a lista ou para os campos de nome e senha (desiste do login em
    /// andamento).
    fn back(&mut self, waker: Option<Waker>) {
        self.input.clear();
        self.message = None;
        self.queued = None;
        self.from_credentials = false;
        if self.pending.is_none() {
            self.ask(Request::CancelSession, Pending::Cancel(After::List), waker);
        }
        self.stage = if self.options.list { Stage::Choose } else { Stage::Credentials };
        self.name_focus = true;
        self.login.clear();
    }

    /// Mostra os campos de nome e senha ("Outro…").
    fn credentials(&mut self) {
        self.stage = Stage::Credentials;
        self.other.clear();
        self.input.clear();
        self.name_focus = true;
        self.message = None;
    }

    /// Trata a resposta do greetd que chegou.
    fn answer(&mut self, kind: Pending, result: Result<Response, String>, ui: &mut Ui) {
        let waker = ui.waker();
        let response = match result {
            Ok(r) => r,
            Err(err) => {
                tracing::warn!("login: greetd não respondeu: {err}");
                self.message = Some((tr!("The login service is not responding").into_owned(), true));
                self.stage = if self.from_credentials || !self.options.list { Stage::Credentials } else { Stage::Choose };
                return;
            }
        };
        match (kind, response) {
            (Pending::Auth, Response::AuthMessage { auth_message_type, auth_message }) => match auth_message_type {
                AuthKind::Secret | AuthKind::Visible => {
                    let secret = auth_message_type == AuthKind::Secret;
                    tracing::info!("login: pergunta do PAM ({}): {}", if secret { "secreta" } else { "visível" }, auth_message.trim());
                    // A senha digitada junto com o nome responde a primeira pergunta secreta.
                    if let Some(password) = self.queued.take().filter(|_| secret) {
                        self.ask(Request::PostAuthMessageResponse { response: Some(password) }, Pending::Auth, waker);
                        return;
                    }
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
                // Como no Mac, a senha errada só balança o campo; outros erros aparecem.
                self.message = match error_type {
                    ErrorType::AuthError => None,
                    ErrorType::Error => Some((description, true)),
                };
                self.failures += 1;
                self.shake = Some(Instant::now());
                self.queued = None;
                // O greetd pede o cancelamento antes de outra tentativa.
                let after = if self.from_credentials { After::Credentials } else { After::Retry };
                self.ask(Request::CancelSession, Pending::Cancel(after), waker);
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
                self.ask(Request::CancelSession, Pending::Cancel(After::Retry), waker);
            }
            (Pending::Cancel(after), _) => match after {
                After::Retry if !self.login.is_empty() => {
                    let login = self.login.clone();
                    let message = self.message.take();
                    self.choose(login, waker);
                    self.message = message;
                }
                After::Credentials => {
                    self.stage = Stage::Credentials;
                    self.input.clear();
                    self.name_focus = false;
                    // Para a dica e o ✓ do nome digitado.
                    self.from_credentials = true;
                }
                _ => {
                    if self.stage == Stage::Waiting {
                        self.stage = if self.options.list { Stage::Choose } else { Stage::Credentials };
                    }
                }
            },
            (_, other) => tracing::warn!("login: resposta inesperada do greetd: {other:?}"),
        }
    }

    /// A lista de usuários (e "Outro…"), com as fotos no alto `y`.
    fn user_list(&mut self, ui: &mut Ui, w: f32, y: f32) {
        const TILE: f32 = 148.0;
        const AVATAR: f32 = 88.0;
        let count = self.users.len() + 1;
        let x0 = (w - TILE * count as f32) / 2.0;
        // Teclado: setas trocam, Enter escolhe.
        if !ui.mirror() && self.menu.is_none() {
            if ui.take_key(Key::Left) {
                self.highlight = self.highlight.saturating_sub(1);
            }
            if ui.take_key(Key::Right) {
                self.highlight = (self.highlight + 1).min(count - 1);
            }
        }
        let enter = !ui.mirror() && self.menu.is_none() && ui.take_key(Key::Enter);
        let pointer = ui.pointer();
        let clicked = ui.input().released && self.menu.is_none();
        let mut chosen = enter.then_some(self.highlight);
        for i in 0..count {
            let tx = x0 + i as f32 * TILE;
            let (icon, name, label) = match self.users.get(i) {
                Some(u) => (u.icon.clone(), u.name.clone(), u.name.clone()),
                None => (None, String::new(), tr!("Other…").into_owned()),
            };
            let over = pointer.is_some_and(|(px, py)| px >= tx && px < tx + TILE && py >= y - 6.0 && py < y + AVATAR + 40.0);
            if over {
                self.highlight = i;
                if clicked {
                    chosen = Some(i);
                }
            }
            let ax = tx + (TILE - AVATAR) / 2.0;
            if i == self.highlight {
                // Foco visível (teclado e ponteiro): anel claro em volta da foto.
                let ring = AVATAR + 8.0;
                ui.paint_text(|pixmap, _, (ox, oy)| {
                    mide::ui::draw::fill_rounded(pixmap, ox + ax - 4.0, oy + y - 4.0, ring, ring, ring / 2.0, mide::ui::draw::rgba(255, 255, 255, 120));
                });
            }
            self.avatars.paint(ui, icon.as_deref(), &name, ax, y, AVATAR, over);
            let label = fit(ui, &label, 15.0, TILE - 10.0);
            lockscreen::centered(ui, &label, tx + TILE / 2.0, y + AVATAR + 12.0, 15.0, FontWeight::Semibold, lockscreen::WHITE);
        }
        if over_any(pointer, x0, y, TILE * count as f32, AVATAR + 40.0) {
            ui.request_tick();
        }
        if let Some(i) = chosen.filter(|_| !ui.mirror()) {
            match self.users.get(i).map(|u| u.login.clone()) {
                Some(login) => self.choose(login, ui.waker()),
                None => self.credentials(),
            }
        }
    }

    /// Quem está entrando: foto, nome e o campo da pergunta do PAM, com o alto da foto
    /// em `y`. `dx` é o balanço da senha recusada.
    fn user_view(&mut self, ui: &mut Ui, cx: f32, y: f32, dx: f32) {
        const AVATAR: f32 = 100.0;
        let icon = self.icon.clone();
        let name = self.name.clone();
        self.avatars.paint(ui, icon.as_deref(), &name, cx - AVATAR / 2.0, y, AVATAR, false);
        lockscreen::centered(ui, &name, cx, y + AVATAR + 12.0, 20.0, FontWeight::Semibold, lockscreen::WHITE);
        let field_y = y + AVATAR + 50.0;
        let (placeholder, secret) = match &self.stage {
            Stage::Ask { prompt, secret } => {
                let lower = prompt.to_lowercase();
                let generic = *secret && (lower.starts_with("pass") || lower.starts_with("senha"));
                (if generic { tr!("Enter Password").into_owned() } else { prompt.clone() }, *secret)
            }
            _ => (tr!("Enter Password").into_owned(), true),
        };
        let busy = self.stage == Stage::Waiting;
        let mut submitted = false;
        let typing = !ui.mirror() && self.menu.is_none();
        ui.at(cx - FIELD_W / 2.0 + dx, field_y, FIELD_W, |ui| {
            let look = LockField { secret, focused: typing, busy, submit: true };
            submitted = ui.lock_field(&mut self.input, &placeholder, look).submitted;
        });
        self.below_field(ui, cx, field_y + 46.0);
        if submitted && !ui.mirror() && self.pending.is_none() && matches!(self.stage, Stage::Ask { .. }) {
            let response = std::mem::take(&mut self.input);
            self.stage = Stage::Waiting;
            self.message = None;
            self.ask(Request::PostAuthMessageResponse { response: Some(response) }, Pending::Auth, ui.waker());
        }
    }

    /// Campos de nome e senha (modo "nome e senha" ou "Outro…"), com o alto da silhueta
    /// em `y`.
    fn credentials_view(&mut self, ui: &mut Ui, cx: f32, y: f32, dx: f32) {
        const AVATAR: f32 = 100.0;
        self.avatars.paint(ui, None, "", cx - AVATAR / 2.0, y, AVATAR, false);
        let field_y = y + AVATAR + 22.0;
        let busy = self.pending.is_some() || self.stage == Stage::Waiting;
        let typing = !ui.mirror() && self.menu.is_none() && !busy;
        // Tab troca de campo; Enter no nome vai para a senha.
        if typing && ui.take_key(Key::Tab) {
            self.name_focus = !self.name_focus;
        }
        let (mut name_r, mut pass_r) = (None, None);
        ui.at(cx - FIELD_W / 2.0, field_y, FIELD_W, |ui| {
            let look = LockField { secret: false, focused: typing && self.name_focus, busy: false, submit: false };
            name_r = Some(ui.lock_field(&mut self.other, &tr!("Name"), look));
        });
        ui.at(cx - FIELD_W / 2.0 + dx, field_y + 46.0, FIELD_W, |ui| {
            let look = LockField { secret: true, focused: typing && !self.name_focus, busy, submit: true };
            pass_r = Some(ui.lock_field(&mut self.input, &tr!("Password"), look));
        });
        let (name_r, pass_r) = (name_r.unwrap_or_default(), pass_r.unwrap_or_default());
        if typing {
            if name_r.clicked {
                self.name_focus = true;
            }
            if pass_r.clicked {
                self.name_focus = false;
            }
            if name_r.submitted {
                self.name_focus = false;
            }
        }
        self.below_field(ui, cx, field_y + 92.0);
        let login = self.other.trim().to_string();
        if typing && pass_r.submitted && !login.is_empty() {
            tracing::info!("login: nome e senha para {login}");
            self.queued = Some(std::mem::take(&mut self.input));
            self.from_credentials = true;
            self.choose(login, ui.waker());
        } else if typing && pass_r.submitted {
            self.name_focus = true;
        }
    }

    /// Embaixo do campo: o aviso do PAM ou do greetd, ou a dica de senha depois de três
    /// tentativas erradas (como no Mac).
    fn below_field(&mut self, ui: &mut Ui, cx: f32, y: f32) {
        if self.failures >= 3 && self.options.password_hints && self.hint.is_empty() && !self.login.is_empty() && !ui.mirror() {
            // Nome digitado à mão: a dica vem do AccountsService (uma vez).
            self.hint = lockscreen::account(&self.login).map(|a| a.hint).filter(|h| !h.is_empty()).unwrap_or_else(|| " ".into());
        }
        let hint = self.hint.trim();
        if let Some((message, error)) = self.message.clone() {
            let color = if error { [255, 214, 214, 255] } else { [255, 255, 255, 235] };
            lockscreen::centered(ui, &message, cx, y, 13.0, FontWeight::Semibold, color);
        } else if self.failures >= 3 && self.options.password_hints && !hint.is_empty() {
            let text = tr!("Hint: {hint}", hint = hint);
            if !self.hint_logged && !ui.mirror() {
                self.hint_logged = true;
                tracing::info!("login: dica de senha mostrada");
            }
            lockscreen::centered(ui, &text, cx, y, 13.0, FontWeight::Semibold, [255, 255, 255, 235]);
        }
    }

    /// Botões de baixo: Cancelar quando dá para voltar; senão Desligar, Reiniciar e
    /// Repousar (se as opções mostram).
    fn bottom(&mut self, ui: &mut Ui, w: f32, h: f32, can_back: bool) {
        let y = h - 98.0;
        if !self.options.message.is_empty() {
            let message = self.options.message.clone();
            let my = if can_back || self.options.power_buttons { y - 34.0 } else { h - 48.0 };
            lockscreen::centered(ui, &message, w / 2.0, my, 13.0, FontWeight::Semibold, [255, 255, 255, 230]);
        }
        if can_back {
            if lockscreen::circle_button(ui, "go-previous-symbolic", &tr!("Cancel"), w / 2.0, y) && self.menu.is_none() {
                self.back(ui.waker());
            }
            return;
        }
        if !self.options.power_buttons {
            return;
        }
        let actions = [
            ("system-shutdown-symbolic", tr!("Shut Down").into_owned(), mide::system::Power::Shutdown, "desligar"),
            ("system-reboot-symbolic", tr!("Restart").into_owned(), mide::system::Power::Restart, "reiniciar"),
            ("system-suspend-symbolic", tr!("Sleep").into_owned(), mide::system::Power::Sleep, "repousar"),
        ];
        let step = 104.0;
        let x0 = w / 2.0 - step * (actions.len() as f32 - 1.0) / 2.0;
        for (i, (symbol, label, action, log)) in actions.into_iter().enumerate() {
            if lockscreen::circle_button(ui, symbol, &label, x0 + i as f32 * step, y) && self.menu.is_none() {
                tracing::info!("login: {log}");
                mide::system::power(action);
            }
        }
    }

    /// Canto de cima à direita: sessão e layout do teclado (cada um abre um menu), Wi-Fi
    /// e bateria.
    fn status_bar(&mut self, ui: &mut Ui, w: f32) {
        let status = self.status.get();
        let mut right = w - 10.0;
        if let Some((percent, charging)) = status.battery {
            right = lockscreen::status_battery(ui, percent, charging, right);
        }
        if let Some(signal) = status.wifi {
            right = lockscreen::status_wifi(ui, signal, right);
        }
        let clicked = ui.input().released && !ui.mirror();
        let pointer = ui.pointer();
        let hit = |r: (f32, f32, f32, f32)| pointer.is_some_and(|(px, py)| px >= r.0 && px < r.0 + r.2 && py >= r.1 && py < r.1 + r.3);
        if self.options.input_menu && self.layouts.len() > 1 {
            let name = self.layouts.get(self.layout).map(|l| l.name()).unwrap_or_default();
            let open = matches!(self.menu, Some((Menu::Layout, _)));
            let rect = lockscreen::status_text(ui, &name, right - 4.0, open);
            if clicked && hit(rect) {
                self.menu = if open { None } else { Some((Menu::Layout, rect)) };
            }
            right = rect.0;
        }
        if self.sessions.len() > 1 {
            let name = self.sessions.get(self.session).map(|s| s.name.clone()).unwrap_or_default();
            let open = matches!(self.menu, Some((Menu::Session, _)));
            let rect = lockscreen::status_text(ui, &name, right - 4.0, open);
            if clicked && hit(rect) {
                self.menu = if open { None } else { Some((Menu::Session, rect)) };
            }
        }
    }

    /// O menu aberto do canto de cima, por cima de tudo.
    fn open_menu(&mut self, ui: &mut Ui) {
        let Some((kind, anchor)) = self.menu else {
            return;
        };
        if !ui.mirror() && ui.take_key(Key::Escape) {
            self.menu = None;
            return;
        }
        let (items, selected): (Vec<String>, usize) = match kind {
            Menu::Session => (self.sessions.iter().map(|s| s.name.clone()).collect(), self.session),
            Menu::Layout => (self.layouts.iter().map(|l| l.name()).collect(), self.layout),
        };
        match lockscreen::menu(ui, anchor, &items, selected) {
            Some(Some(i)) => {
                self.menu = None;
                match kind {
                    Menu::Session => {
                        tracing::info!("login: sessão escolhida: {}", items.get(i).cloned().unwrap_or_default());
                        if i != self.session {
                            self.session_chosen = true;
                        }
                        self.session = i;
                    }
                    Menu::Layout => {
                        mide::control_client::ask_compositor(&format!("layout {i}"));
                        tracing::info!("login: layout de teclado {i}");
                        self.layout = i;
                    }
                }
            }
            Some(None) => self.menu = None,
            None => {}
        }
    }
}

/// Largura dos campos de senha e de nome.
const FIELD_W: f32 = 240.0;

/// O ponteiro está na área.
fn over_any(pointer: Option<(f32, f32)>, x: f32, y: f32, w: f32, h: f32) -> bool {
    pointer.is_some_and(|(px, py)| px >= x && px < x + w && py >= y && py < y + h)
}

/// `s` cortado com reticências para caber em `width` pontos.
fn fit(ui: &mut Ui, s: &str, size: f32, width: f32) -> String {
    if lockscreen::measure(ui, s, size, FontWeight::Semibold) <= width {
        return s.to_string();
    }
    let mut out: String = s.to_string();
    while out.chars().count() > 1 && lockscreen::measure(ui, &format!("{out}…"), size, FontWeight::Semibold) > width {
        out.pop();
    }
    format!("{}…", out.trim_end())
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
            // Um usuário só: já começa nele, como o Mac e o GDM.
            if self.options.list && self.stage == Stage::Choose && self.login.is_empty() && self.users.len() == 1 && self.pending.is_none() && self.message.is_none() {
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

        let cx = w / 2.0;
        lockscreen::clock(ui, cx, (h * 0.075).round());
        self.status_bar(ui, w);
        // O conteúdo fica na metade de baixo, como no Sonoma e no Tahoe.
        let content = (h * 0.50).round();
        let can_back = match self.stage {
            Stage::Choose => false,
            Stage::Credentials => self.options.list,
            _ => self.options.list && (self.users.len() > 1 || self.from_credentials),
        };
        let credentials = self.stage == Stage::Credentials || (self.from_credentials && self.queued.is_some());
        if self.stage == Stage::Choose {
            self.user_list(ui, w, (h * 0.56).round());
        } else if credentials {
            self.credentials_view(ui, cx, (content - 24.0).round(), dx);
        } else {
            self.user_view(ui, cx, content, dx);
            // Esc volta à lista.
            if can_back && !ui.mirror() && self.menu.is_none() && ui.take_key(Key::Escape) {
                self.back(ui.waker());
            }
        }
        if self.stage == Stage::Credentials && can_back && !ui.mirror() && self.menu.is_none() && ui.take_key(Key::Escape) {
            self.back(ui.waker());
        }
        self.bottom(ui, w, h, can_back);
        self.open_menu(ui);
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let _logging = mide::logging::init("mide-login", "mide_login=info");
    run(Window::new(&mide::i18n::trc("window title", "Login")).app_id("mide-login").lock(), Login::new())
}
