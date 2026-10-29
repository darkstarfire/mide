//! Serviços do mide no D-Bus para os outros apps (B25), iniciados pelo compositor só
//! na sessão do mide:
//!
//! - `org.freedesktop.FileManager1`: "Mostrar na pasta" dos navegadores e de outros
//!   apps abre o Arquivos do mide (`ShowFolders`, `ShowItems` com o item escolhido e
//!   `ShowItemProperties`). O nome é pedido em tempo de execução, sem arquivo de
//!   ativação: fora da sessão do mide (Cinnamon, GNOME) quem atende continua o de lá.
//! - `org.freedesktop.impl.portal.FileChooser` (nome
//!   `org.freedesktop.impl.portal.desktop.mide`): o seletor de arquivos dos apps que
//!   usam o portal ("Abrir…", "Salvar como…"), mostrado pelo `mide-chooser`. O
//!   `xdg-desktop-portal` só usa este backend com `XDG_CURRENT_DESKTOP=mide`
//!   (`mide-portals.conf`).
//!
//! `mide-portal --portal-only` (ativação pelo D-Bus, se o portal pedir antes do
//! compositor iniciar este programa) atende só o seletor e sai depois de um tempo
//! sem pedidos.
//!
//! O programa sai sozinho quando o compositor que o abriu termina (o nome
//! `FileManager1` não fica preso no barramento depois da sessão).

use std::{
    collections::HashMap,
    ffi::OsString,
    future::Future,
    path::PathBuf,
    pin::Pin,
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    task::{Context, Poll, Waker},
    time::{Duration, Instant},
};

use mide::portal::{self, Method, ShowKind};
use zbus::{
    fdo::RequestNameFlags,
    zvariant::{OwnedObjectPath, OwnedValue},
    ObjectServer,
};

/// Programa do mide ao lado deste (instalado ou em target/), ou pelo PATH.
fn sibling(name: &str) -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|d| d.join(name)))
        .filter(|p| p.exists())
        .unwrap_or_else(|| PathBuf::from(name))
}

/// Pedidos abertos e o momento do último (para o `--portal-only` sair sem uso).
static PENDING: AtomicUsize = AtomicUsize::new(0);
static LAST_USE: Mutex<Option<Instant>> = Mutex::new(None);

fn touch() {
    if let Ok(mut last) = LAST_USE.lock() {
        *last = Some(Instant::now());
    }
}

struct FileManager;

impl FileManager {
    fn show(&self, kind: ShowKind, method: &str, uris: Vec<String>, startup_id: String) {
        tracing::info!("filemanager1: {method} {uris:?}");
        let (windows, refused) = portal::file_manager_windows(kind, &uris);
        for uri in refused {
            tracing::warn!("filemanager1: endereço que o Arquivos não abre: {uri}");
        }
        for args in windows {
            let mut command = Command::new(sibling("mide-files"));
            command.args(&args);
            // O token de ativação passa adiante: a janela nova pode receber o foco.
            if !startup_id.is_empty() {
                command.env("XDG_ACTIVATION_TOKEN", &startup_id).env("DESKTOP_STARTUP_ID", &startup_id);
            }
            match command.spawn() {
                Ok(mut child) => {
                    std::thread::spawn(move || {
                        let _ = child.wait();
                    });
                }
                Err(err) => tracing::warn!("filemanager1: Arquivos não abriu: {err}"),
            }
        }
    }
}

#[zbus::interface(name = "org.freedesktop.FileManager1")]
impl FileManager {
    fn show_folders(&self, uris: Vec<String>, startup_id: String) {
        self.show(ShowKind::Folders, "ShowFolders", uris, startup_id);
    }

    fn show_items(&self, uris: Vec<String>, startup_id: String) {
        self.show(ShowKind::Items, "ShowItems", uris, startup_id);
    }

    fn show_item_properties(&self, uris: Vec<String>, startup_id: String) {
        self.show(ShowKind::Properties, "ShowItemProperties", uris, startup_id);
    }
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

/// Roda `f` numa thread e espera o resultado sem prender o barramento.
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

/// Estado de um pedido aberto: o seletor (pid) e se o app já desistiu (`Close`).
#[derive(Default)]
struct RequestState {
    closed: bool,
    pid: Option<u32>,
}

/// Um pedido aberto (`org.freedesktop.impl.portal.Request`): `Close` fecha o seletor.
/// Se o `Close` chega antes de o seletor abrir, a marca `closed` impede que ele abra.
struct Request {
    state: Arc<Mutex<RequestState>>,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Request")]
impl Request {
    fn close(&self) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        state.closed = true;
        tracing::info!("seletor: pedido fechado pelo app");
        if let Some(pid) = state.pid.and_then(|p| rustix::process::Pid::from_raw(p as i32)) {
            let _ = rustix::process::kill_process(pid, rustix::process::Signal::TERM);
        }
    }
}

/// Abre o seletor, a não ser que o pedido já tenha sido fechado. A trava fica presa do
/// `spawn` até gravar o pid: um `Close` no meio espera e encontra o pid. `None`:
/// fechado antes de abrir.
fn spawn_chooser(state: &Mutex<RequestState>, args: &[OsString]) -> std::io::Result<Option<std::process::Child>> {
    let mut state = state.lock().map_err(|_| std::io::Error::other("trava do pedido"))?;
    if state.closed {
        return Ok(None);
    }
    let child = Command::new(sibling("mide-chooser")).args(args).stdout(Stdio::piped()).spawn()?;
    state.pid = Some(child.id());
    Ok(Some(child))
}

struct FileChooser;

impl FileChooser {
    #[allow(clippy::too_many_arguments)]
    async fn run(&self, server: &ObjectServer, method: Method, handle: OwnedObjectPath, app_id: String, parent: String, title: String, options: HashMap<String, OwnedValue>) -> (u32, HashMap<String, OwnedValue>) {
        PENDING.fetch_add(1, Ordering::SeqCst);
        touch();
        let req = portal::decode_request(method, &title, &options);
        tracing::info!("seletor: {method:?} de {app_id:?} (janela {parent:?}): {:?}", req.mode);
        let state: Arc<Mutex<RequestState>> = Arc::default();
        let exported = server.at(&handle, Request { state: state.clone() }).await.unwrap_or(false);
        let args: Vec<OsString> = portal::chooser_args(&req);
        let outcome = in_thread(move || spawn_chooser(&state, &args)?.map(|child| child.wait_with_output()).transpose()).await;
        if exported {
            let _ = server.remove::<Request, _>(&handle).await;
        }
        PENDING.fetch_sub(1, Ordering::SeqCst);
        touch();
        match outcome {
            Ok(None) => {
                tracing::info!("seletor: resposta 1 (fechado antes de abrir)");
                (1, HashMap::new())
            }
            Ok(Some(output)) if output.status.code() == Some(0) => {
                let out = portal::parse_output(&String::from_utf8_lossy(&output.stdout));
                tracing::info!("seletor: resposta 0 com {} arquivo(s)", out.paths.len());
                (0, portal::results(&req, &out))
            }
            Ok(Some(output)) if output.status.code() == Some(1) || output.status.code().is_none() => {
                tracing::info!("seletor: resposta 1 (cancelado)");
                (1, HashMap::new())
            }
            Ok(Some(output)) => {
                tracing::warn!("seletor: mide-chooser saiu com {}", output.status);
                (2, HashMap::new())
            }
            Err(err) => {
                tracing::warn!("seletor: mide-chooser não abriu: {err}");
                (2, HashMap::new())
            }
        }
    }
}

#[zbus::interface(name = "org.freedesktop.impl.portal.FileChooser")]
impl FileChooser {
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        4
    }

    #[allow(clippy::too_many_arguments)]
    async fn open_file(&self, #[zbus(object_server)] server: &ObjectServer, handle: OwnedObjectPath, app_id: String, parent_window: String, title: String, options: HashMap<String, OwnedValue>) -> (u32, HashMap<String, OwnedValue>) {
        self.run(server, Method::OpenFile, handle, app_id, parent_window, title, options).await
    }

    #[allow(clippy::too_many_arguments)]
    async fn save_file(&self, #[zbus(object_server)] server: &ObjectServer, handle: OwnedObjectPath, app_id: String, parent_window: String, title: String, options: HashMap<String, OwnedValue>) -> (u32, HashMap<String, OwnedValue>) {
        self.run(server, Method::SaveFile, handle, app_id, parent_window, title, options).await
    }

    #[allow(clippy::too_many_arguments)]
    async fn save_files(&self, #[zbus(object_server)] server: &ObjectServer, handle: OwnedObjectPath, app_id: String, parent_window: String, title: String, options: HashMap<String, OwnedValue>) -> (u32, HashMap<String, OwnedValue>) {
        self.run(server, Method::SaveFiles, handle, app_id, parent_window, title, options).await
    }
}

fn main() {
    let _logging = mide::logging::init("mide-portal", "mide_portal=info");
    let portal_only = std::env::args().any(|a| a == "--portal-only");
    let conn = match zbus::blocking::Connection::session() {
        Ok(conn) => conn,
        Err(err) => {
            tracing::warn!("mide-portal: sem barramento da sessão: {err}");
            return;
        }
    };
    let mut serving = false;
    if conn.object_server().at(portal::PORTAL_PATH, FileChooser).is_ok() {
        match conn.request_name_with_flags(portal::PORTAL_BUS_NAME, RequestNameFlags::DoNotQueue.into()) {
            Ok(_) => {
                tracing::info!("seletor: pronto ({})", portal::PORTAL_BUS_NAME);
                serving = true;
            }
            Err(err) => tracing::info!("seletor: outro programa já atende ({err})"),
        }
    }
    if !portal_only && conn.object_server().at(portal::FILEMANAGER_PATH, FileManager).is_ok() {
        // Com outro gerenciador de arquivos já no nome (Nemo, Nautilus ativados antes),
        // pede para trocar; se ele não deixar, fica na fila e assume quando ele sair.
        match conn.request_name_with_flags(portal::FILEMANAGER_BUS_NAME, RequestNameFlags::ReplaceExisting.into()) {
            Ok(zbus::fdo::RequestNameReply::InQueue) => {
                tracing::info!("filemanager1: na fila (outro gerenciador de arquivos tem o nome até sair)");
                serving = true;
            }
            Ok(_) => {
                tracing::info!("filemanager1: pronto ({})", portal::FILEMANAGER_BUS_NAME);
                serving = true;
            }
            Err(err) => tracing::warn!("filemanager1: nome não obtido: {err}"),
        }
    }
    if !serving {
        return;
    }
    // Vive enquanto o compositor que o abriu viver (sem ele, a sessão acabou); no modo
    // de ativação, até ficar 10 minutos sem pedidos.
    let parent = std::os::unix::process::parent_id();
    touch();
    loop {
        std::thread::sleep(Duration::from_secs(2));
        if !portal_only && std::os::unix::process::parent_id() != parent {
            tracing::info!("mide-portal: o compositor saiu; encerrando");
            break;
        }
        let idle = LAST_USE.lock().ok().and_then(|l| *l).is_some_and(|t| t.elapsed() > Duration::from_secs(600));
        if portal_only && idle && PENDING.load(Ordering::SeqCst) == 0 {
            tracing::info!("mide-portal: sem pedidos há 10 minutos; encerrando");
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_antes_de_abrir_nao_abre_o_seletor() {
        // O `Close` chegou antes do `spawn`: o seletor não abre (e nenhum pid fica).
        let state = Mutex::new(RequestState { closed: true, pid: None });
        assert!(spawn_chooser(&state, &[]).unwrap().is_none());
        assert_eq!(state.lock().unwrap().pid, None);
        // O `Close` marca o pedido mesmo sem seletor aberto.
        let request = Request { state: Arc::default() };
        request.close();
        assert!(request.state.lock().unwrap().closed);
    }
}
