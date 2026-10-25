//! Bloqueio de tela (`ext-session-lock-v1`): enquanto a tela está bloqueada, o
//! compositor só desenha a superfície de bloqueio de cada tela (o `mide-lock`) e só
//! ela recebe teclado e mouse. Se o programa de bloqueio fechar ou travar, a tela
//! continua bloqueada (fica escura) e o mide abre outro.
//!
//! Quem pede o bloqueio: o atalho (Super+L), o logind (`loginctl lock-session`, antes
//! de suspender) e o tempo sem uso (`[lock] idle_minutes`).

use std::time::{Duration, Instant};

use smithay::{
    delegate_session_lock,
    reexports::wayland_server::Resource,
    output::Output,
    reexports::wayland_server::protocol::wl_output::WlOutput,
    utils::SERIAL_COUNTER,
    wayland::session_lock::{LockSurface, SessionLockHandler, SessionLockManagerState, SessionLocker},
};

use crate::state::State;

/// Estado do bloqueio.
#[derive(Default)]
pub struct Lock {
    /// A tela está bloqueada (do pedido do programa de bloqueio até o desbloqueio).
    pub locked: bool,
    /// Superfície de bloqueio de cada tela.
    pub surfaces: Vec<(Output, LockSurface)>,
    /// Quando o programa de bloqueio foi aberto pela última vez (para não abrir vários).
    spawned: Option<Instant>,
    /// Desbloqueios desde a partida (para o autoteste).
    pub unlocks: u32,
}

impl Lock {
    /// Superfície de bloqueio viva desta tela.
    pub fn surface_for(&self, output: &Output) -> Option<&LockSurface> {
        self.surfaces.iter().find(|(o, s)| o == output && s.alive()).map(|(_, s)| s)
    }

    /// Algum programa de bloqueio está mostrando a tela de bloqueio.
    pub fn client_alive(&self) -> bool {
        self.surfaces.iter().any(|(_, s)| s.alive())
    }
}

impl SessionLockHandler for State {
    fn lock_state(&mut self) -> &mut SessionLockManagerState {
        &mut self.session_lock_state
    }

    fn lock(&mut self, confirmation: SessionLocker) {
        tracing::info!("tela bloqueada");
        self.lock.locked = true;
        self.lock.surfaces.retain(|(_, s)| s.alive());
        // Ninguém mais recebe teclas até a tela de bloqueio aparecer.
        if let Some(keyboard) = self.seat.get_keyboard() {
            keyboard.set_focus(self, None, SERIAL_COUNTER.next_serial());
        }
        self.preview = None;
        confirmation.lock();
        self.wake();
    }

    fn unlock(&mut self) {
        tracing::info!("tela desbloqueada");
        self.lock.locked = false;
        self.lock.surfaces.clear();
        self.lock.unlocks += 1;
        self.last_activity = Instant::now();
        // O foco volta para a janela do topo.
        let top = self.space.elements().last().and_then(|w| w.toplevel().map(|t| t.wl_surface().clone()));
        if let Some(keyboard) = self.seat.get_keyboard() {
            keyboard.set_focus(self, top, SERIAL_COUNTER.next_serial());
        }
        self.wake();
    }

    fn new_surface(&mut self, surface: LockSurface, wl_output: WlOutput) {
        let Some(output) = Output::from_resource(&wl_output) else {
            return;
        };
        let size = self.space.output_geometry(&output).map(|g| g.size).unwrap_or_default();
        surface.with_pending_state(|s| s.size = Some((size.w.max(1) as u32, size.h.max(1) as u32).into()));
        surface.send_configure();
        // O teclado vai para a primeira tela de bloqueio do programa (é nela que ele
        // desenha o campo de senha); as outras telas só ficam cobertas.
        let main = !self.lock.client_alive();
        if main {
            if let Some(keyboard) = self.seat.get_keyboard() {
                keyboard.set_focus(self, Some(surface.wl_surface().clone()), SERIAL_COUNTER.next_serial());
            }
        }
        tracing::info!("tela de bloqueio na saída {}", output.name());
        self.lock.surfaces.retain(|(o, _)| o != &output);
        self.lock.surfaces.push((output, surface));
        self.wake();
    }
}

delegate_session_lock!(State);

impl smithay::wayland::idle_notify::IdleNotifierHandler for State {
    fn idle_notifier_state(&mut self) -> &mut smithay::wayland::idle_notify::IdleNotifierState<Self> {
        &mut self.idle_notifier_state
    }
}
smithay::delegate_idle_notify!(State);

impl smithay::wayland::idle_inhibit::IdleInhibitHandler for State {
    fn inhibit(&mut self, surface: smithay::reexports::wayland_server::protocol::wl_surface::WlSurface) {
        tracing::info!("um programa pediu para a tela não apagar");
        self.inhibitors.push(surface);
    }

    fn uninhibit(&mut self, surface: smithay::reexports::wayland_server::protocol::wl_surface::WlSurface) {
        self.inhibitors.retain(|s| s != &surface);
    }
}
smithay::delegate_idle_inhibit!(State);

/// Abre o programa de bloqueio (se ainda não há um mostrando a tela).
pub fn request(state: &mut State, reason: &str) {
    if state.lock.locked && state.lock.client_alive() {
        return;
    }
    if state.lock.spawned.is_some_and(|t| t.elapsed() < Duration::from_secs(3)) {
        return;
    }
    tracing::info!("bloqueio pedido: {reason}");
    state.lock.spawned = Some(Instant::now());
    crate::input::spawn("mide-lock");
}

/// Duração de `n` minutos da configuração (`MIDE_IDLE_UNIT=seconds` faz os testes
/// contarem em segundos).
fn minutes(n: u32) -> Duration {
    if std::env::var("MIDE_IDLE_UNIT").as_deref() == Ok("seconds") { Duration::from_secs(n as u64) } else { Duration::from_secs(n as u64 * 60) }
}

/// A cada volta do laço: reabre o programa de bloqueio se ele fechou com a tela
/// bloqueada, e, depois do tempo sem uso, bloqueia e apaga a tela (a não ser que um
/// programa, como um vídeo tocando, peça para não).
pub fn tick(state: &mut State) {
    if state.lock.locked && !state.lock.client_alive() {
        request(state, "o programa de bloqueio fechou");
    }
    // Pedidos de "não apague" de superfícies que já não existem não contam.
    state.inhibitors.retain(|s| s.is_alive());
    let inhibited = !state.inhibitors.is_empty();
    state.idle_notifier_state.set_is_inhibited(inhibited);
    if inhibited || !state.idle_lock {
        return;
    }
    let idle = state.last_activity.elapsed();
    let lock_after = state.config.lock.idle_minutes;
    if !state.lock.locked && lock_after > 0 && idle >= minutes(lock_after) {
        request(state, "tempo sem uso");
    }
    // Apagar: o tempo escolhido, ou 1 minuto com a tela bloqueada.
    let blank_after = if state.lock.locked { Some(1) } else { Some(state.config.lock.blank_minutes).filter(|m| *m > 0) };
    if let Some(blank_after) = blank_after {
        if !state.blanked && idle >= minutes(blank_after) {
            tracing::info!("tela apagada (sem uso)");
            state.blanked = true;
            state.wake();
        }
    }
    // Suspender depois do tempo escolhido em Configurações → Energia (uma vez por
    // período sem uso).
    let suspend_after = state.config.power.suspend_minutes;
    if suspend_after > 0 && !state.idle_suspended && idle >= minutes(suspend_after) {
        tracing::info!("suspendendo (sem uso há {suspend_after} min)");
        state.idle_suspended = true;
        mide::system::power(mide::system::Power::Sleep);
    }
}

/// Pedidos do logind: "Lock" da sessão (`loginctl lock-session`) e "PrepareForSleep"
/// (bloquear antes de suspender). Numa thread; cada pedido chega pelo canal.
pub fn watch_logind(sender: smithay::reexports::calloop::channel::Sender<&'static str>) {
    std::thread::Builder::new()
        .name("logind".into())
        .spawn(move || {
            let Ok(conn) = zbus::blocking::Connection::system() else {
                return;
            };
            let Ok(manager) = zbus::blocking::Proxy::new(&conn, "org.freedesktop.login1", "/org/freedesktop/login1", "org.freedesktop.login1.Manager") else {
                return;
            };
            let session: Option<zbus::zvariant::OwnedObjectPath> = manager.call("GetSessionByPID", &(std::process::id(),)).ok();
            let sleep = sender.clone();
            let sleep_conn = conn.clone();
            std::thread::spawn(move || {
                let Ok(sleep_proxy) = zbus::blocking::Proxy::new(&sleep_conn, "org.freedesktop.login1", "/org/freedesktop/login1", "org.freedesktop.login1.Manager") else {
                    return;
                };
                let Ok(signals) = sleep_proxy.receive_signal("PrepareForSleep") else {
                    return;
                };
                // `true`: vai suspender (bloqueia antes); `false`: acordou (as telas
                // refazem o estado: uma troca de quadro em andamento pode ter se perdido).
                for signal in signals {
                    let going = signal.body().deserialize::<bool>().unwrap_or(false);
                    if sleep.send(if going { "suspender" } else { "acordou" }).is_err() {
                        break;
                    }
                }
            });
            let Some(session) = session else {
                tracing::info!("logind: sem sessão para este processo (o Lock não chega)");
                return;
            };
            let Ok(proxy) = zbus::blocking::Proxy::new(&conn, "org.freedesktop.login1", session, "org.freedesktop.login1.Session") else {
                return;
            };
            let Ok(signals) = proxy.receive_signal("Lock") else {
                return;
            };
            for _ in signals {
                if sender.send("logind").is_err() {
                    break;
                }
            }
        })
        .ok();
}
