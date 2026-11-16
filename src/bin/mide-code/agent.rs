//! Painel do Claude no Code: uma conversa com o Claude Code instalado na máquina, que
//! trabalha no projeto aberto. Entra com a conta do Claude (`claude auth login
//! --claudeai`, a assinatura; sem chave de API) e conversa com o `claude -p` pelo
//! formato `stream-json`. Cada ferramenta que o Claude quer usar (escrever, rodar um
//! comando...) pede licença aqui (`--permission-prompt-tool stdio`): Permitir, Negar ou
//! Permitir sempre nesta conversa.
//!
//! Outros agentes podem entrar depois: o painel só depende dos eventos daqui.

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{channel, Receiver, Sender};

use mide::kit::Ui;
use mide::tr;
use serde_json::{json, Value};

/// Uma linha da conversa.
#[derive(Clone, Debug, PartialEq)]
pub enum Line {
    User(String),
    Text(String),
    /// Ferramenta usada: nome, o que (arquivo, comando...) e se deu certo (`None` = rodando).
    Tool { id: String, name: String, detail: String, ok: Option<bool> },
    Note(String),
}

/// Pedido de licença para usar uma ferramenta.
#[derive(Clone, Debug)]
pub struct Permission {
    pub id: String,
    pub tool: String,
    pub description: String,
    pub input: Value,
}

/// Situação do Claude Code na máquina.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Checking,
    /// O `claude` não está instalado.
    Missing,
    LoggedOut,
    Ready,
}

enum Event {
    Status(Status),
    Json(Value),
    Exited,
}

pub struct Agent {
    pub status: Status,
    pub lines: Vec<Line>,
    pub pending: Vec<Permission>,
    pub busy: bool,
    pub input: String,
    pub model: String,
    pub cost: f64,
    session: Option<String>,
    always: HashSet<String>,
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    tx: Sender<Event>,
    rx: Receiver<Event>,
    waker: Option<mide::kit::Waker>,
    checked: bool,
    login_started: Option<std::time::Instant>,
    tools: HashMap<String, usize>,
    /// Quantas linhas (e pedidos) o painel já mostrou: com novas, rola para o fim.
    shown: usize,
}

/// O programa `claude` (no PATH ou em `~/.local/bin`, onde o instalador oficial põe).
fn claude_bin() -> Option<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .chain([mide::places::home_dir().join(".local/bin")])
        .map(|d| d.join("claude"))
        .find(|p| p.is_file())
}

/// O que mostrar de uma ferramenta: o arquivo, o comando, o padrão buscado...
fn tool_detail(input: &Value) -> String {
    for key in ["file_path", "path", "command", "pattern", "url", "query", "description", "notebook_path"] {
        if let Some(s) = input.get(key).and_then(Value::as_str) {
            return s.lines().next().unwrap_or(s).to_string();
        }
    }
    String::new()
}

impl Agent {
    pub fn new() -> Self {
        let (tx, rx) = channel();
        Self {
            status: Status::Checking,
            lines: Vec::new(),
            pending: Vec::new(),
            busy: false,
            input: String::new(),
            model: String::new(),
            cost: 0.0,
            session: None,
            always: HashSet::new(),
            child: None,
            stdin: None,
            tx,
            rx,
            waker: None,
            checked: false,
            login_started: None,
            tools: HashMap::new(),
            shown: 0,
        }
    }

    fn send_event(&self) -> impl Fn(Event) + Send + Clone + 'static {
        let (tx, waker) = (self.tx.clone(), self.waker.clone());
        move |e| {
            let _ = tx.send(e);
            if let Some(w) = &waker {
                w.wake();
            }
        }
    }

    /// Confere se o `claude` existe e se já entrou com a conta.
    fn check(&mut self) {
        self.checked = true;
        let send = self.send_event();
        std::thread::spawn(move || {
            let Some(bin) = claude_bin() else {
                send(Event::Status(Status::Missing));
                return;
            };
            let out = Command::new(bin).args(["auth", "status", "--json"]).output();
            let logged = out.ok().and_then(|o| serde_json::from_slice::<Value>(&o.stdout).ok()).and_then(|v| v.get("loggedIn").and_then(Value::as_bool)).unwrap_or(false);
            send(Event::Status(if logged { Status::Ready } else { Status::LoggedOut }));
        });
    }

    /// Entra com a conta do Claude (abre o navegador; a assinatura, sem chave de API).
    pub fn login(&mut self) {
        if let Some(bin) = claude_bin() {
            tracing::info!("claude: entrando com a conta");
            let _ = Command::new(bin).args(["auth", "login", "--claudeai"]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
            self.login_started = Some(std::time::Instant::now());
        }
    }

    /// Começa a sessão do `claude` na pasta `cwd` (continua a conversa, se havia uma).
    fn start(&mut self, cwd: &Path) -> bool {
        let Some(bin) = claude_bin() else {
            self.status = Status::Missing;
            return false;
        };
        let mut cmd = Command::new(bin);
        cmd.args(["-p", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose", "--permission-prompt-tool", "stdio"]);
        if let Some(s) = &self.session {
            cmd.args(["--resume", s]);
        }
        cmd.current_dir(cwd).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                self.lines.push(Line::Note(tr!("Couldn’t start Claude Code: {err}", err = e)));
                return false;
            }
        };
        self.stdin = child.stdin.take();
        let stdout = child.stdout.take();
        let send = self.send_event();
        std::thread::spawn(move || {
            if let Some(out) = stdout {
                for line in std::io::BufReader::new(out).lines().map_while(Result::ok) {
                    if let Ok(v) = serde_json::from_str::<Value>(&line) {
                        send(Event::Json(v));
                    }
                }
            }
            send(Event::Exited);
        });
        tracing::info!("claude: sessão aberta em {}", cwd.display());
        self.child = Some(child);
        true
    }

    fn write(&mut self, v: &Value) {
        if let Some(stdin) = self.stdin.as_mut() {
            let _ = writeln!(stdin, "{v}");
            let _ = stdin.flush();
        }
    }

    /// Manda a mensagem do campo.
    pub fn send(&mut self, cwd: &Path) {
        let text = self.input.trim().to_string();
        if text.is_empty() || self.busy {
            return;
        }
        if self.child.is_none() && !self.start(cwd) {
            return;
        }
        self.input.clear();
        self.lines.push(Line::User(text.clone()));
        self.busy = true;
        self.write(&json!({"type": "user", "message": {"role": "user", "content": text}}));
    }

    /// Para o que o Claude está fazendo.
    pub fn stop(&mut self) {
        self.write(&json!({"type": "control_request", "request_id": format!("mide-{}", std::process::id()), "request": {"subtype": "interrupt"}}));
    }

    /// Responde um pedido de licença.
    pub fn answer(&mut self, index: usize, allow: bool, always: bool) {
        if index >= self.pending.len() {
            return;
        }
        let p = self.pending.remove(index);
        if always {
            self.always.insert(p.tool.clone());
        }
        self.reply(&p, allow);
    }

    fn reply(&mut self, p: &Permission, allow: bool) {
        let response = if allow {
            json!({"behavior": "allow", "updatedInput": p.input})
        } else {
            json!({"behavior": "deny", "message": "The user denied this in the mide Code."})
        };
        self.write(&json!({"type": "control_response", "response": {"subtype": "success", "request_id": p.id, "response": response}}));
    }

    /// Conversa nova (a anterior fica no histórico do Claude Code).
    pub fn new_conversation(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
        }
        self.stdin = None;
        self.session = None;
        self.lines.clear();
        self.pending.clear();
        self.busy = false;
        self.tools.clear();
    }

    /// Lê o que chegou do `claude`.
    pub fn poll(&mut self, ui: &Ui) {
        if self.waker.is_none() {
            self.waker = ui.waker();
        }
        if !self.checked {
            self.check();
        }
        // Depois de mandar entrar, confere de novo a cada pouco (até 3 minutos).
        if self.status == Status::LoggedOut && self.login_started.is_some_and(|t| t.elapsed().as_secs() < 180) && !self.checked {
            self.check();
        }
        while let Ok(e) = self.rx.try_recv() {
            match e {
                Event::Status(s) => {
                    self.status = s;
                    if s == Status::LoggedOut && self.login_started.is_some_and(|t| t.elapsed().as_secs() < 180) {
                        // Ainda não entrou: confere de novo daqui a pouco.
                        let send = self.send_event();
                        std::thread::spawn(move || {
                            std::thread::sleep(std::time::Duration::from_secs(3));
                            let logged = claude_bin()
                                .and_then(|b| Command::new(b).args(["auth", "status", "--json"]).output().ok())
                                .and_then(|o| serde_json::from_slice::<Value>(&o.stdout).ok())
                                .and_then(|v| v.get("loggedIn").and_then(Value::as_bool))
                                .unwrap_or(false);
                            send(Event::Status(if logged { Status::Ready } else { Status::LoggedOut }));
                        });
                    }
                }
                Event::Exited => {
                    self.child = None;
                    self.stdin = None;
                    if self.busy {
                        self.lines.push(Line::Note(tr!("Claude Code stopped.").into_owned()));
                    }
                    self.busy = false;
                    self.pending.clear();
                }
                Event::Json(v) => self.handle(v),
            }
        }
    }

    fn handle(&mut self, v: Value) {
        match v.get("type").and_then(Value::as_str).unwrap_or_default() {
            "system" if v.get("subtype").and_then(Value::as_str) == Some("init") => {
                self.session = v.get("session_id").and_then(Value::as_str).map(str::to_string);
                self.model = v.get("model").and_then(Value::as_str).unwrap_or_default().to_string();
            }
            "assistant" => {
                for block in v.pointer("/message/content").and_then(Value::as_array).cloned().unwrap_or_default() {
                    match block.get("type").and_then(Value::as_str) {
                        Some("text") => {
                            let text = block.get("text").and_then(Value::as_str).unwrap_or_default().trim().to_string();
                            if !text.is_empty() {
                                self.lines.push(Line::Text(text));
                            }
                        }
                        Some("tool_use") => {
                            let id = block.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
                            let name = block.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
                            let detail = block.get("input").map(tool_detail).unwrap_or_default();
                            self.tools.insert(id.clone(), self.lines.len());
                            self.lines.push(Line::Tool { id, name, detail, ok: None });
                        }
                        _ => {}
                    }
                }
            }
            "user" => {
                for block in v.pointer("/message/content").and_then(Value::as_array).cloned().unwrap_or_default() {
                    if block.get("type").and_then(Value::as_str) == Some("tool_result") {
                        let id = block.get("tool_use_id").and_then(Value::as_str).unwrap_or_default();
                        let ok = !block.get("is_error").and_then(Value::as_bool).unwrap_or(false);
                        if let Some(Line::Tool { ok: state, .. }) = self.tools.get(id).and_then(|&i| self.lines.get_mut(i)) {
                            *state = Some(ok);
                        }
                    }
                }
            }
            "control_request" => {
                let request = v.get("request").cloned().unwrap_or_default();
                if request.get("subtype").and_then(Value::as_str) == Some("can_use_tool") {
                    let p = Permission {
                        id: v.get("request_id").and_then(Value::as_str).unwrap_or_default().to_string(),
                        tool: request.get("tool_name").and_then(Value::as_str).unwrap_or_default().to_string(),
                        description: request.get("description").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| request.get("input").map(tool_detail).unwrap_or_default()),
                        input: request.get("input").cloned().unwrap_or(json!({})),
                    };
                    if self.always.contains(&p.tool) {
                        self.reply(&p, true);
                    } else {
                        self.pending.push(p);
                    }
                }
            }
            "result" => {
                self.busy = false;
                self.cost += v.get("total_cost_usd").and_then(Value::as_f64).unwrap_or(0.0);
                if v.get("is_error").and_then(Value::as_bool) == Some(true) {
                    let text = v.get("result").and_then(Value::as_str).unwrap_or_default();
                    self.lines.push(Line::Note(tr!("Claude Code reported an error: {err}", err = text)));
                }
            }
            _ => {}
        }
    }
}

impl Drop for Agent {
    fn drop(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
        }
    }
}

/// Texto encurtado para caber numa linha do painel.
fn short(s: &str, max: usize) -> String {
    if s.chars().count() > max {
        format!("{}…", s.chars().take(max - 1).collect::<String>())
    } else {
        s.to_string()
    }
}

/// Nome da ferramenta para mostrar.
fn tool_name(name: &str) -> String {
    match name {
        "Read" => tr!("Read").into_owned(),
        "Write" => tr!("Write").into_owned(),
        "Edit" | "MultiEdit" => tr!("Edit").into_owned(),
        "Bash" => tr!("Run").into_owned(),
        "Grep" | "Glob" => tr!("Search").into_owned(),
        "WebFetch" | "WebSearch" => tr!("Web").into_owned(),
        other => other.to_string(),
    }
}

impl crate::Code {
    /// O painel do Claude no retângulo `rect` (à direita do editor).
    pub(crate) fn agent_panel(&mut self, ui: &mut Ui, rect: (f32, f32, f32, f32)) {
        self.agent.poll(ui);
        let t = ui.theme().clone();
        let bg = self.color_theme.as_ref().and_then(|c| c.background).unwrap_or(t.card);
        ui.paint(|pixmap, (ox, oy)| {
            mide::ui::draw::fill_rounded(pixmap, ox + rect.0, oy + rect.1, rect.2, rect.3, 10.0, mide::ui::draw::rgba(bg[0], bg[1], bg[2], 255));
        });
        let cwd = self.root.clone().unwrap_or_else(mide::places::home_dir);
        let mut action: Option<&str> = None;
        let mut answer: Option<(usize, bool, bool)> = None;
        let agent = &mut self.agent;
        ui.at(rect.0 + 10.0, rect.1 + 8.0, rect.2 - 20.0, |ui| {
            ui.row(|ui| {
                ui.strong("Claude");
                if !agent.model.is_empty() {
                    ui.note(&agent.model);
                }
                if agent.status == Status::Ready && ui.button(&tr!("New Conversation")).clicked() {
                    action = Some("new");
                }
            });
            match agent.status {
                Status::Checking => ui.note(&tr!("Looking for Claude Code…")),
                Status::Missing => {
                    ui.note(&tr!("Claude Code isn’t installed. Install it in the terminal with:"));
                    ui.note("curl -fsSL https://claude.ai/install.sh | bash");
                    if ui.button(&tr!("Check Again")).clicked() {
                        action = Some("check");
                    }
                }
                Status::LoggedOut => {
                    ui.note(&tr!("Sign in with your Claude account (subscription, no API key). The browser opens to finish."));
                    if ui.primary_button(&tr!("Sign In with Claude")).clicked() {
                        action = Some("login");
                    }
                }
                Status::Ready => {
                    // O que fica embaixo da conversa: o pedido de licença (um de cada
                    // vez), "trabalhando" e o campo.
                    let reserved = 60.0 + 52.0 + if agent.busy { 44.0 } else { 0.0 } + if agent.pending.is_empty() { 0.0 } else { 180.0 };
                    let list_h = (rect.3 - reserved).max(80.0);
                    let now = agent.lines.len() + agent.busy as usize;
                    let keep = (now != agent.shown).then_some((1.0e9, 1.0e9));
                    agent.shown = now;
                    let prefix = format!("{}/", cwd.display());
                    ui.scroll_area_showing(list_h, keep, |ui| {
                        if agent.lines.is_empty() {
                            ui.note(&tr!("Ask Claude to explain, change or create code in this project. Each change asks for your permission."));
                        }
                        for line in &agent.lines {
                            match line {
                                Line::User(text) => {
                                    ui.space(6.0);
                                    ui.strong(&tr!("You"));
                                    ui.label(text);
                                }
                                Line::Text(text) => ui.label(text),
                                Line::Tool { name, detail, ok, .. } => {
                                    let mark = match ok {
                                        None => "…",
                                        Some(true) => "✓",
                                        Some(false) => "✕",
                                    };
                                    // Caminhos do projeto relativos a ele.
                                    let detail = detail.strip_prefix(&prefix).unwrap_or(detail);
                                    ui.note(&format!("{mark} {} {}", tool_name(name), short(detail, 80)));
                                }
                                Line::Note(text) => ui.note(text),
                            }
                        }
                    });
                    if let Some(p) = agent.pending.first() {
                        ui.card(|ui| {
                            ui.strong(&tr!("Allow {tool}?", tool = tool_name(&p.tool)));
                            let what = p.description.strip_prefix(&prefix).unwrap_or(&p.description);
                            ui.note(&short(what, 200));
                            ui.row(|ui| {
                                if ui.button(&tr!("Deny")).clicked() {
                                    answer = Some((0, false, false));
                                }
                                if ui.primary_button(&tr!("Allow")).clicked() {
                                    answer = Some((0, true, false));
                                }
                            });
                            if ui.button(&tr!("Always in This Conversation")).clicked() {
                                answer = Some((0, true, true));
                            }
                        });
                    }
                    if agent.busy {
                        ui.row(|ui| {
                            ui.note(&tr!("Claude is working…"));
                            if ui.button(&tr!("Stop")).clicked() {
                                action = Some("stop");
                            }
                        });
                    }
                    if ui.text_input(&mut agent.input, &tr!("Message Claude")).submitted() {
                        action = Some("send");
                    }
                }
            }
        });
        if let Some((i, allow, always)) = answer {
            self.agent.answer(i, allow, always);
        }
        match action {
            Some("new") => self.agent.new_conversation(),
            Some("check") => self.agent.checked = false,
            Some("login") => self.agent.login(),
            Some("stop") => self.agent.stop(),
            Some("send") => self.agent.send(&cwd),
            _ => {}
        }
        // Enquanto o login no navegador não termina, confere de novo.
        if self.agent.status == Status::LoggedOut && self.agent.login_started.is_some_and(|t| t.elapsed().as_secs() < 180) {
            ui.wake_after(std::time::Duration::from_secs(3));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eventos_do_claude_viram_a_conversa() {
        let mut a = Agent::new();
        a.handle(json!({"type": "system", "subtype": "init", "session_id": "s1", "model": "claude-x"}));
        a.handle(json!({"type": "assistant", "message": {"content": [{"type": "text", "text": "Vou criar."}, {"type": "tool_use", "id": "t1", "name": "Write", "input": {"file_path": "/p/a.txt", "content": "oi"}}]}}));
        a.handle(json!({"type": "control_request", "request_id": "r1", "request": {"subtype": "can_use_tool", "tool_name": "Write", "input": {"file_path": "/p/a.txt"}, "description": "a.txt"}}));
        assert_eq!(a.pending.len(), 1);
        a.handle(json!({"type": "user", "message": {"content": [{"type": "tool_result", "tool_use_id": "t1", "is_error": false}]}}));
        a.handle(json!({"type": "result", "total_cost_usd": 0.01, "is_error": false}));
        assert_eq!(a.session.as_deref(), Some("s1"));
        assert_eq!(a.model, "claude-x");
        assert_eq!(a.lines[0], Line::Text("Vou criar.".into()));
        assert_eq!(a.lines[1], Line::Tool { id: "t1".into(), name: "Write".into(), detail: "/p/a.txt".into(), ok: Some(true) });
        assert!(!a.busy);
        // "Sempre nesta conversa": o próximo pedido da mesma ferramenta passa sozinho.
        a.answer(0, true, true);
        a.handle(json!({"type": "control_request", "request_id": "r2", "request": {"subtype": "can_use_tool", "tool_name": "Write", "input": {}}}));
        assert!(a.pending.is_empty());
    }
}
