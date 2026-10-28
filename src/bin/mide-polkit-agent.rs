//! Agente de autenticação do polkit da sessão do mide (C7): quando um programa pede
//! permissão de administrador pelo polkit (`pkexec`, PackageKit, montar discos), o
//! `polkitd` chama este agente, que mostra a janela de senha do mide
//! (`mide-askpass --agent`) e entrega a senha ao `polkit-agent-helper-1` (o programa
//! setuid do polkit que confere a senha pelo PAM e avisa o `polkitd`).
//!
//! Sem a libpolkit-agent: o protocolo é o do D-Bus (`org.freedesktop.PolicyKit1`) e o do
//! helper, uma linha por mensagem (`mide::elevate::parse_helper_line`). A senha passa só
//! do pipe da janela para o pipe do helper: nunca por argumento, variável, log ou arquivo.
//!
//! Iniciado pelo compositor só na sessão do mide (num mide em janela dentro de outro
//! desktop, o agente de lá continua). Só em compilação de desenvolvimento, para os
//! testes: `MIDE_POLKIT_BUS` (endereço de um barramento de teste no lugar do sistema)
//! e `MIDE_POLKIT_HELPER` (um helper falso).

use std::{
    collections::HashMap,
    future::Future,
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    pin::Pin,
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    task::{Context, Poll, Waker},
    time::Duration,
};

use mide::elevate::{self, FromWindow, HelperLine, Identity, ToWindow};
use mide::tr;
use zbus::zvariant::{OwnedValue, Value};

const AGENT_PATH: &str = "/org/mide/PolicyKit1/AuthenticationAgent";
const AUTHORITY: &str = "org.freedesktop.PolicyKit1";
const AUTHORITY_PATH: &str = "/org/freedesktop/PolicyKit1/Authority";
const AUTHORITY_IFACE: &str = "org.freedesktop.PolicyKit1.Authority";

#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.freedesktop.PolicyKit1.Error")]
enum AgentError {
    #[zbus(error)]
    ZBus(zbus::Error),
    Failed(String),
    Cancelled(String),
    NotAuthorized(String),
}

/// Um pedido em andamento: cancelado pelo `CancelAuthentication` e os processos dele
/// (janela e helper), para encerrar.
#[derive(Default)]
struct Session {
    cancelled: AtomicBool,
    pids: Mutex<Vec<u32>>,
}

impl Session {
    fn track(&self, pid: u32) {
        if let Ok(mut pids) = self.pids.lock() {
            pids.push(pid);
        }
    }

    /// Tira o processo da lista **antes** do `wait`: depois de recolhido, o número pode
    /// ser de outro processo, e o `cancel` não pode mandar sinal a ele. O `cancel` segura
    /// a mesma trava enquanto manda os sinais, então nunca vê um número já recolhido.
    fn untrack(&self, pid: u32) {
        if let Ok(mut pids) = self.pids.lock() {
            pids.retain(|&p| p != pid);
        }
    }

    #[cfg(test)]
    fn tracked(&self) -> Vec<u32> {
        self.pids.lock().map(|p| p.clone()).unwrap_or_default()
    }

    fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        if let Ok(pids) = self.pids.lock() {
            for &pid in pids.iter() {
                if let Some(pid) = rustix::process::Pid::from_raw(pid as i32) {
                    let _ = rustix::process::kill_process(pid, rustix::process::Signal::TERM);
                }
            }
        }
    }

    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

/// Pedidos abertos, pelo cookie do polkit.
static SESSIONS: Mutex<Vec<(String, Arc<Session>)>> = Mutex::new(Vec::new());
/// Um pedido por vez: o segundo espera o primeiro terminar.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

/// Programa do mide ao lado deste (instalado ou em target/), ou pelo PATH.
fn sibling(name: &str) -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|d| d.join(name)))
        .filter(|p| p.exists())
        .unwrap_or_else(|| PathBuf::from(name))
}

/// O helper do polkit: o instalado (setuid de root, conferido); `MIDE_POLKIT_HELPER`
/// só em compilação de desenvolvimento.
fn helper_path() -> Option<PathBuf> {
    if cfg!(debug_assertions)
        && let Some(fake) = std::env::var_os("MIDE_POLKIT_HELPER").filter(|v| !v.is_empty())
    {
        return Some(PathBuf::from(fake));
    }
    elevate::find_helper()
}

/// O que acontece durante um pedido: linhas do helper (da tentativa `n`), respostas da
/// janela e o fim de cada um.
enum Event {
    Helper(u32, HelperLine),
    HelperEnd(u32),
    Window(FromWindow),
    WindowEnd,
}

enum Outcome {
    Ok,
    Cancelled,
    Failed(String),
}

/// O pedido de autenticação recebido do polkit.
struct Request {
    message: String,
    command: String,
    user: String,
    cookie: String,
}

fn send_to_window(stdin: &mut std::process::ChildStdin, msg: ToWindow) {
    let _ = stdin.write_all(msg.to_line().as_bytes());
    let _ = stdin.flush();
}

fn kill(child: &mut Child, session: &Session) {
    session.untrack(child.id());
    let _ = child.kill();
    let _ = child.wait();
}

/// Espera um processo que já está terminando (fora da lista do `cancel` antes).
fn reap(child: &mut Child, session: &Session) {
    session.untrack(child.id());
    let _ = child.wait();
}

/// Abre o helper para a tentativa `n`: escreve o cookie e lê as linhas dele numa thread.
fn spawn_helper(helper: &PathBuf, req: &Request, n: u32, tx: &mpsc::Sender<Event>, session: &Session) -> std::io::Result<Child> {
    // Entrada num pipe (o helper recusa um terminal) e o usuário como único argumento.
    let mut child = Command::new(helper).arg(&req.user).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit()).spawn()?;
    session.track(child.id());
    if let Some(stdin) = child.stdin.as_mut() {
        stdin.write_all(format!("{}\n", req.cookie).as_bytes())?;
        stdin.flush()?;
    }
    let stdout = child.stdout.take().expect("saída do helper");
    let tx = tx.clone();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if tx.send(Event::Helper(n, elevate::parse_helper_line(&line))).is_err() {
                return;
            }
        }
        let _ = tx.send(Event::HelperEnd(n));
    });
    Ok(child)
}

/// Conversa com a janela e com o helper até a senha ser aceita, a pessoa desistir ou
/// as tentativas acabarem.
fn authenticate(req: &Request, session: &Session) -> Outcome {
    let Some(helper) = helper_path() else {
        match elevate::helper_without_setuid() {
            // Polkit com o helper pelo socket do systemd (sem setuid): ainda não suportado.
            Some(p) => tracing::warn!("{} não é setuid (polkit com o helper pelo socket do systemd, ainda não suportado pelo mide)", p.display()),
            None => tracing::warn!("polkit-agent-helper-1 não encontrado (setuid de root em {:?})", elevate::HELPER_PATHS),
        }
        return Outcome::Failed("polkit-agent-helper-1 não encontrado".into());
    };
    let mut window = match Command::new(sibling("mide-askpass")).arg("--agent").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit()).spawn() {
        Ok(w) => w,
        Err(err) => {
            tracing::warn!("a janela de senha não abriu: {err}");
            return Outcome::Failed(format!("a janela de senha não abriu: {err}"));
        }
    };
    session.track(window.id());
    let (tx, rx) = mpsc::channel::<Event>();
    let mut to_window = window.stdin.take().expect("entrada da janela");
    let mut from_window = window.stdout.take().expect("saída da janela");
    {
        let tx = tx.clone();
        std::thread::spawn(move || {
            // Sem `BufReader`: o buffer dele guardaria a senha sem ser apagado. A linha é
            // lida byte a byte num `Vec` apagado depois de cada mensagem.
            let mut line: Vec<u8> = Vec::new();
            while let Ok(true) = elevate::read_secret_line(&mut from_window, &mut line) {
                let msg = std::str::from_utf8(&line).ok().and_then(FromWindow::parse);
                elevate::wipe_bytes(&mut line);
                if let Some(msg) = msg
                    && tx.send(Event::Window(msg)).is_err()
                {
                    return;
                }
            }
            let _ = tx.send(Event::WindowEnd);
        });
    }
    send_to_window(&mut to_window, ToWindow::Message(req.message.clone()));
    send_to_window(&mut to_window, ToWindow::Command(req.command.clone()));
    send_to_window(&mut to_window, ToWindow::User(req.user.clone()));

    let finish = |window: &mut Child, to_window: &mut std::process::ChildStdin, outcome: Outcome| {
        send_to_window(to_window, ToWindow::Close);
        session.untrack(window.id());
        // A janela sai sozinha com o `close`; se travar, é encerrada.
        for _ in 0..40 {
            if window.try_wait().ok().flatten().is_some() {
                return outcome;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        let _ = window.kill();
        let _ = window.wait();
        outcome
    };

    for attempt in 1..=elevate::MAX_TRIES {
        if session.is_cancelled() {
            return finish(&mut window, &mut to_window, Outcome::Cancelled);
        }
        let mut child = match spawn_helper(&helper, req, attempt, &tx, session) {
            Ok(c) => c,
            Err(err) => {
                tracing::warn!("o helper do polkit não abriu: {err}");
                return finish(&mut window, &mut to_window, Outcome::Failed(format!("o helper do polkit não abriu: {err}")));
            }
        };
        let mut asked = false;
        let result = loop {
            let Ok(event) = rx.recv_timeout(Duration::from_millis(200)) else {
                if session.is_cancelled() {
                    break None;
                }
                continue;
            };
            match event {
                Event::Window(FromWindow::Cancel) | Event::WindowEnd => break None,
                Event::Window(FromWindow::Pass(mut pass)) => {
                    if !asked {
                        elevate::wipe(&mut pass);
                        continue;
                    }
                    asked = false;
                    let ok = child.stdin.as_mut().is_some_and(|stdin| {
                        // Capacidade exata: a linha não cresce (sem cópias soltas).
                        let mut line = String::with_capacity(pass.len() + 1);
                        line.push_str(&pass);
                        line.push('\n');
                        let r = stdin.write_all(line.as_bytes()).and_then(|_| stdin.flush()).is_ok();
                        elevate::wipe(&mut line);
                        r
                    });
                    elevate::wipe(&mut pass);
                    if !ok {
                        tracing::warn!("o helper do polkit fechou antes da senha");
                    }
                    send_to_window(&mut to_window, ToWindow::Busy);
                }
                Event::Helper(n, _) | Event::HelperEnd(n) if n != attempt => {}
                Event::Helper(_, HelperLine::PromptEchoOff(prompt)) | Event::Helper(_, HelperLine::PromptEchoOn(prompt)) => {
                    asked = true;
                    send_to_window(&mut to_window, ToWindow::Ask(prompt));
                }
                Event::Helper(_, HelperLine::Error(text)) => send_to_window(&mut to_window, ToWindow::Info(text)),
                Event::Helper(_, HelperLine::Info(text)) => send_to_window(&mut to_window, ToWindow::Info(text)),
                Event::Helper(_, HelperLine::Success) => break Some(true),
                Event::Helper(_, HelperLine::Failure) | Event::HelperEnd(_) => break Some(false),
                Event::Helper(_, HelperLine::Other(line)) => tracing::debug!("helper: linha desconhecida ({} bytes)", line.len()),
            }
        };
        match result {
            None => {
                kill(&mut child, session);
                tracing::info!("autenticação cancelada");
                return finish(&mut window, &mut to_window, Outcome::Cancelled);
            }
            Some(true) => {
                reap(&mut child, session);
                tracing::info!("senha aceita pelo polkit");
                return finish(&mut window, &mut to_window, Outcome::Ok);
            }
            Some(false) => {
                reap(&mut child, session);
                tracing::info!("senha recusada (tentativa {attempt} de {})", elevate::MAX_TRIES);
                if attempt < elevate::MAX_TRIES {
                    send_to_window(&mut to_window, ToWindow::Error(tr!("Incorrect password. Try again.").into_owned()));
                }
            }
        }
    }
    finish(&mut window, &mut to_window, Outcome::Failed("senha incorreta".into()))
}

/// Resposta de uma thread para um método assíncrono do D-Bus (um "oneshot" mínimo).
struct Reply<T> {
    slot: Arc<Mutex<(Option<T>, Option<Waker>)>>,
}

impl<T> Future for Reply<T> {
    type Output = T;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<T> {
        let mut slot = self.slot.lock().expect("trava da resposta");
        match slot.0.take() {
            Some(value) => Poll::Ready(value),
            None => {
                slot.1 = Some(cx.waker().clone());
                Poll::Pending
            }
        }
    }
}

fn in_thread<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Reply<T> {
    let slot: Arc<Mutex<(Option<T>, Option<Waker>)>> = Arc::default();
    let out = slot.clone();
    std::thread::spawn(move || {
        let value = f();
        if let Ok(mut slot) = out.lock() {
            slot.0 = Some(value);
            if let Some(waker) = slot.1.take() {
                waker.wake();
            }
        }
    });
    Reply { slot }
}

fn value_u32(map: &HashMap<String, OwnedValue>, key: &str) -> Option<u32> {
    map.get(key).and_then(|v| v.downcast_ref::<u32>().ok())
}

/// Identidades do polkit (`unix-user` com `uid`, `unix-group` com `gid`).
fn identities(list: &[(String, HashMap<String, OwnedValue>)]) -> Vec<Identity> {
    list.iter()
        .filter_map(|(kind, details)| match kind.as_str() {
            "unix-user" => value_u32(details, "uid").map(Identity::User),
            "unix-group" => value_u32(details, "gid").map(Identity::Group),
            _ => None,
        })
        .collect()
}

/// Quem chamou é o `polkitd` (o dono atual de `org.freedesktop.PolicyKit1`)? A política
/// do D-Bus da maioria das distribuições já só deixa ele chamar o agente, mas não todas:
/// sem isto, um processo qualquer abriria janelas de senha com o texto que quisesse ou
/// cancelaria pedidos de verdade.
async fn from_polkit(conn: &zbus::Connection, hdr: &zbus::message::Header<'_>) -> bool {
    let Some(sender) = hdr.sender() else {
        return false;
    };
    let Ok(dbus) = zbus::fdo::DBusProxy::new(conn).await else {
        return false;
    };
    let Ok(name) = zbus::names::BusName::try_from(AUTHORITY) else {
        return false;
    };
    dbus.get_name_owner(name).await.is_ok_and(|owner| owner.as_str() == sender.as_str())
}

struct Agent;

#[zbus::interface(name = "org.freedesktop.PolicyKit1.AuthenticationAgent")]
impl Agent {
    #[allow(clippy::too_many_arguments)]
    async fn begin_authentication(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
        action_id: String,
        message: String,
        icon_name: String,
        details: HashMap<String, String>,
        cookie: String,
        identities: Vec<(String, HashMap<String, OwnedValue>)>,
    ) -> Result<(), AgentError> {
        let _ = icon_name;
        if !from_polkit(conn, &hdr).await {
            tracing::warn!("pedido de autenticação recusado: não veio do polkit ({:?})", hdr.sender().map(|s| s.to_string()));
            return Err(AgentError::NotAuthorized("só o polkit pode pedir autenticação".into()));
        }
        tracing::info!("pedido de autenticação: {action_id}");
        let ids = self::identities(&identities);
        let passwd = std::fs::read_to_string("/etc/passwd").unwrap_or_default();
        let group = std::fs::read_to_string("/etc/group").unwrap_or_default();
        let me = rustix::process::getuid().as_raw();
        let Some(user) = elevate::choose_identity(&ids, me, &passwd, &group) else {
            tracing::warn!("nenhuma identidade conhecida para {action_id}");
            return Err(AgentError::Failed("nenhuma identidade para autenticar".into()));
        };
        let command = details.get("command_line").or_else(|| details.get("program")).cloned().unwrap_or_else(|| action_id.clone());
        let session = Arc::new(Session::default());
        if let Ok(mut sessions) = SESSIONS.lock() {
            sessions.push((cookie.clone(), session.clone()));
        }
        let req = Request { message, command, user, cookie: cookie.clone() };
        let thread_session = session.clone();
        let outcome = in_thread(move || {
            let _turn = ONE_AT_A_TIME.lock();
            if thread_session.is_cancelled() {
                return Outcome::Cancelled;
            }
            authenticate(&req, &thread_session)
        })
        .await;
        if let Ok(mut sessions) = SESSIONS.lock() {
            sessions.retain(|(_, s)| !Arc::ptr_eq(s, &session));
        }
        match outcome {
            Outcome::Ok => Ok(()),
            Outcome::Cancelled => Err(AgentError::Cancelled("a autenticação foi cancelada".into())),
            Outcome::Failed(why) => Err(AgentError::Failed(why)),
        }
    }

    async fn cancel_authentication(&self, #[zbus(header)] hdr: zbus::message::Header<'_>, #[zbus(connection)] conn: &zbus::Connection, cookie: String) -> Result<(), AgentError> {
        if !from_polkit(conn, &hdr).await {
            tracing::warn!("cancelamento recusado: não veio do polkit ({:?})", hdr.sender().map(|s| s.to_string()));
            return Err(AgentError::NotAuthorized("só o polkit pode cancelar".into()));
        }
        tracing::info!("o polkit cancelou um pedido");
        let found: Vec<Arc<Session>> = SESSIONS.lock().map(|s| s.iter().filter(|(c, _)| *c == cookie).map(|(_, s)| s.clone()).collect()).unwrap_or_default();
        for session in found {
            session.cancel();
        }
        Ok(())
    }
}

/// O barramento do polkit: o do sistema; `MIDE_POLKIT_BUS` (um barramento de teste)
/// só em compilação de desenvolvimento.
fn connect() -> zbus::Result<(zbus::blocking::Connection, bool)> {
    if cfg!(debug_assertions)
        && let Some(address) = std::env::var("MIDE_POLKIT_BUS").ok().filter(|a| !a.is_empty())
    {
        return Ok((zbus::blocking::connection::Builder::address(address.as_str())?.build()?, true));
    }
    Ok((zbus::blocking::Connection::system()?, false))
}

/// A sessão de login deste processo: pelo logind (ou elogind) e, de reserva,
/// `XDG_SESSION_ID`.
fn session_id(conn: &zbus::blocking::Connection, test_bus: bool) -> Option<String> {
    if !test_bus {
        let by_pid = conn
            .call_method(Some("org.freedesktop.login1"), "/org/freedesktop/login1", Some("org.freedesktop.login1.Manager"), "GetSessionByPID", &(std::process::id()))
            .ok()
            .and_then(|reply| reply.body().deserialize::<zbus::zvariant::OwnedObjectPath>().ok());
        if let Some(path) = by_pid {
            let id = conn
                .call_method(Some("org.freedesktop.login1"), path.as_str(), Some("org.freedesktop.DBus.Properties"), "Get", &("org.freedesktop.login1.Session", "Id"))
                .ok()
                .and_then(|reply| reply.body().deserialize::<OwnedValue>().ok())
                .and_then(|v| v.downcast_ref::<String>().ok());
            if id.is_some() {
                return id;
            }
        }
    }
    std::env::var("XDG_SESSION_ID").ok().filter(|s| !s.is_empty())
}

fn subject(session: &str) -> (&'static str, HashMap<&'static str, Value<'_>>) {
    let mut details = HashMap::new();
    details.insert("session-id", Value::from(session));
    ("unix-session", details)
}

fn locale() -> String {
    ["LC_ALL", "LC_MESSAGES", "LANG"].iter().find_map(|v| std::env::var(v).ok().filter(|s| !s.is_empty())).unwrap_or_else(|| "C".into())
}

fn register(conn: &zbus::blocking::Connection, session: &str) -> zbus::Result<()> {
    let mut options: HashMap<&str, Value> = HashMap::new();
    options.insert("fallback", Value::from(false));
    conn.call_method(Some(AUTHORITY), AUTHORITY_PATH, Some(AUTHORITY_IFACE), "RegisterAuthenticationAgentWithOptions", &(subject(session), locale(), AGENT_PATH, options))?;
    Ok(())
}

fn main() {
    // Sem dump de memória (a senha passa por aqui) e ninguém do mesmo usuário lê a
    // memória pelo ptrace sem ser root.
    let _ = rustix::process::set_dumpable_behavior(rustix::process::DumpableBehavior::NotDumpable);
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "mide_polkit_agent=info".into()))
        .init();
    let (conn, test_bus) = match connect() {
        Ok(c) => c,
        Err(err) => {
            tracing::warn!("agente do polkit: sem o barramento do sistema: {err}");
            return;
        }
    };
    if let Err(err) = conn.object_server().at(AGENT_PATH, Agent) {
        tracing::warn!("agente do polkit: objeto não publicado: {err}");
        return;
    }
    let Some(session) = session_id(&conn, test_bus) else {
        tracing::warn!("agente do polkit: sessão de login desconhecida (sem logind e sem XDG_SESSION_ID)");
        return;
    };
    // Outro agente na mesma sessão (o anterior saindo num `restart shell`): tenta mais
    // algumas vezes; com um agente de terceiros de pé, desiste com um aviso.
    let mut registered = false;
    for attempt in 1..=5 {
        match register(&conn, &session) {
            Ok(()) => {
                registered = true;
                break;
            }
            Err(err) if attempt < 5 => {
                tracing::debug!("agente do polkit: registro recusado ({err}); tentando de novo");
                std::thread::sleep(Duration::from_millis(600));
            }
            Err(err) => tracing::warn!("agente do polkit: registro recusado na sessão {session}: {err} (outro agente de pé?)"),
        }
    }
    if !registered {
        return;
    }
    tracing::info!("agente do polkit pronto na sessão {session}");
    // Vive enquanto quem o abriu (o compositor) viver.
    let parent = std::os::unix::process::parent_id();
    loop {
        std::thread::sleep(Duration::from_secs(2));
        if std::os::unix::process::parent_id() != parent {
            tracing::info!("agente do polkit: o compositor saiu; encerrando");
            break;
        }
    }
    let _ = conn.call_method(Some(AUTHORITY), AUTHORITY_PATH, Some(AUTHORITY_IFACE), "UnregisterAuthenticationAgent", &(subject(&session), AGENT_PATH));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn processo_recolhido_sai_da_lista_do_cancelamento() {
        let session = Session::default();
        let mut child = Command::new("true").spawn().expect("true");
        session.track(child.id());
        assert_eq!(session.tracked(), [child.id()]);
        reap(&mut child, &session);
        // Depois do wait, o número pode ser de outro processo: o cancelamento não o vê.
        assert!(session.tracked().is_empty());
        let mut child = Command::new("sleep").arg("5").spawn().expect("sleep");
        session.track(child.id());
        session.cancel();
        assert!(session.is_cancelled());
        // O cancelamento encerrou o processo (TERM) e o kill tira da lista.
        let status = child.wait().expect("wait");
        assert!(!status.success());
        session.untrack(child.id());
        assert!(session.tracked().is_empty());
    }
}
