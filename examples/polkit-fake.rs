//! `polkitd` falso para os testes do agente do polkit do mide (C7). Nunca fala com o
//! polkit de verdade: roda num barramento de teste (o da sessão de um
//! `dbus-run-session`), toma o nome `org.freedesktop.PolicyKit1`, aceita o registro do
//! agente e depois faz os pedidos dados nos argumentos, registrando as respostas.
//!
//! Uso: `polkit-fake <arquivo-de-saída> <passo>...`, com os passos:
//! - `begin COOKIE`: chama `BeginAuthentication` e espera a resposta;
//! - `cancel COOKIE MS`: chama `BeginAuthentication` e, depois de MS milissegundos,
//!   `CancelAuthentication`;
//! - `intruso COOKIE`: um outro processo (outra conexão, sem o nome do polkit) chama
//!   `BeginAuthentication` (o agente tem de recusar sem abrir janela);
//! - `intruso-cancel COOKIE MS`: o polkit chama `BeginAuthentication` e, depois de MS
//!   milissegundos, o outro processo chama `CancelAuthentication` (recusado: o pedido
//!   continua).
//!
//! Cada linha do arquivo: `registrado sessão=... caminho=...`, `COOKIE: ok` ou
//! `COOKIE: erro NOME`.

use std::{
    collections::HashMap,
    io::Write,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use zbus::zvariant::{OwnedValue, Value};

#[derive(Default)]
struct Registered {
    agent: Option<(String, String)>,
}

struct Authority {
    state: Arc<Mutex<Registered>>,
    out: Arc<Mutex<std::fs::File>>,
}

fn log(out: &Mutex<std::fs::File>, line: &str) {
    println!("{line}");
    if let Ok(mut f) = out.lock() {
        let _ = writeln!(f, "{line}");
    }
}

#[zbus::interface(name = "org.freedesktop.PolicyKit1.Authority")]
impl Authority {
    fn register_authentication_agent_with_options(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        subject: (String, HashMap<String, OwnedValue>),
        locale: String,
        object_path: String,
        options: HashMap<String, OwnedValue>,
    ) {
        let sender = header.sender().map(|s| s.to_string()).unwrap_or_default();
        let session = subject.1.get("session-id").and_then(|v| v.downcast_ref::<String>().ok()).unwrap_or_default();
        let fallback = options.get("fallback").and_then(|v| v.downcast_ref::<bool>().ok());
        log(&self.out, &format!("registrado tipo={} sessão={session} caminho={object_path} idioma={} fallback={fallback:?}", subject.0, !locale.is_empty()));
        if let Ok(mut s) = self.state.lock() {
            s.agent = Some((sender, object_path));
        }
    }

    fn unregister_authentication_agent(&self, _subject: (String, HashMap<String, OwnedValue>), object_path: String) {
        log(&self.out, &format!("desregistrado caminho={object_path}"));
    }

    fn authentication_agent_response2(&self, uid: u32, cookie: String, _identity: (String, HashMap<String, OwnedValue>)) {
        log(&self.out, &format!("resposta do helper uid={uid} cookie={cookie}"));
    }
}

fn result(cookie: &str, reply: zbus::Result<zbus::Message>) -> String {
    match reply {
        Ok(_) => format!("{cookie}: ok"),
        Err(zbus::Error::MethodError(name, _, _)) => format!("{cookie}: erro {name}"),
        Err(err) => format!("{cookie}: erro {err}"),
    }
}

fn begin(conn: &zbus::blocking::Connection, agent: &(String, String), cookie: &str) -> String {
    let mut details: HashMap<&str, &str> = HashMap::new();
    details.insert("program", "/usr/bin/true");
    details.insert("command_line", "/usr/bin/true --teste-do-agente");
    let mut uid: HashMap<&str, Value> = HashMap::new();
    uid.insert("uid", Value::from(rustix::process::getuid().as_raw()));
    let identities = vec![("unix-user", uid)];
    let reply = conn.call_method(
        Some(agent.0.as_str()),
        agent.1.as_str(),
        Some("org.freedesktop.PolicyKit1.AuthenticationAgent"),
        "BeginAuthentication",
        &("org.mide.teste.rodar", "Autenticação é necessária para rodar o teste do mide", "", details, cookie, identities),
    );
    result(cookie, reply)
}

fn cancel(conn: &zbus::blocking::Connection, agent: &(String, String), cookie: &str) -> zbus::Result<zbus::Message> {
    conn.call_method(Some(agent.0.as_str()), agent.1.as_str(), Some("org.freedesktop.PolicyKit1.AuthenticationAgent"), "CancelAuthentication", &(cookie))
}

/// Outra conexão no mesmo barramento, sem o nome do polkit (um processo qualquer).
fn intruder() -> zbus::blocking::Connection {
    zbus::blocking::Connection::session().expect("barramento de teste")
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(out_path) = args.first() else {
        eprintln!("uso: polkit-fake <arquivo-de-saída> <passo>...");
        std::process::exit(2);
    };
    let out = Arc::new(Mutex::new(std::fs::OpenOptions::new().create(true).append(true).open(out_path).expect("arquivo de saída")));
    let state: Arc<Mutex<Registered>> = Arc::default();
    let conn = zbus::blocking::connection::Builder::session()
        .and_then(|b| b.name("org.freedesktop.PolicyKit1"))
        .and_then(|b| b.serve_at("/org/freedesktop/PolicyKit1/Authority", Authority { state: state.clone(), out: out.clone() }))
        .and_then(|b| b.build())
        .expect("barramento de teste");
    log(&out, "polkitd falso pronto");
    let deadline = Instant::now() + Duration::from_secs(20);
    let agent = loop {
        if let Some(agent) = state.lock().ok().and_then(|s| s.agent.clone()) {
            break agent;
        }
        if Instant::now() > deadline {
            log(&out, "nenhum agente registrado");
            std::process::exit(1);
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let mut steps = args[1..].iter();
    while let Some(step) = steps.next() {
        match step.as_str() {
            "begin" => {
                let cookie = steps.next().cloned().unwrap_or_default();
                let result = begin(&conn, &agent, &cookie);
                log(&out, &result);
            }
            "cancel" => {
                let cookie = steps.next().cloned().unwrap_or_default();
                let ms: u64 = steps.next().and_then(|m| m.parse().ok()).unwrap_or(1000);
                let (c2, a2, k2) = (conn.clone(), agent.clone(), cookie.clone());
                let waiting = std::thread::spawn(move || begin(&c2, &a2, &k2));
                std::thread::sleep(Duration::from_millis(ms));
                let _ = cancel(&conn, &agent, &cookie);
                log(&out, &format!("{cookie}: cancelamento pedido"));
                let result = waiting.join().unwrap_or_default();
                log(&out, &result);
            }
            "intruso" => {
                let cookie = steps.next().cloned().unwrap_or_default();
                let result = begin(&intruder(), &agent, &cookie);
                log(&out, &format!("intruso {result}"));
            }
            "intruso-cancel" => {
                let cookie = steps.next().cloned().unwrap_or_default();
                let ms: u64 = steps.next().and_then(|m| m.parse().ok()).unwrap_or(1000);
                let (c2, a2, k2) = (conn.clone(), agent.clone(), cookie.clone());
                let waiting = std::thread::spawn(move || begin(&c2, &a2, &k2));
                std::thread::sleep(Duration::from_millis(ms));
                let r = cancel(&intruder(), &agent, &cookie);
                log(&out, &format!("intruso {}", result(&format!("{cookie}-cancelamento"), r)));
                let result = waiting.join().unwrap_or_default();
                log(&out, &result);
            }
            other => log(&out, &format!("passo desconhecido: {other}")),
        }
    }
    log(&out, "fim");
}
