//! Suspensão, do jeito do GNOME e do KDE: o mide segura um inibidor "delay" do logind
//! para o sono (`Inhibit("sleep", ..., "delay")`). Quando o logind avisa que vai
//! suspender (`PrepareForSleep(true)`), o mide bloqueia a tela, espera o quadro com a
//! tela de bloqueio chegar ao monitor (o vblank dele) e só então solta o inibidor. Daí
//! até a volta, nenhum quadro novo vai à placa: uma troca de página em voo no momento
//! da suspensão é a receita do vblank perdido e da tela congelada.
//!
//! Este módulo guarda só a lógica (testável sem placa nem logind); a thread do logind
//! fica em `lock::watch_logind` e quem confere o quadro entregue é o backend DRM.

use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Espera máxima pela tela de bloqueio na tela antes de liberar a suspensão (a thread do
/// logind solta o inibidor sozinha em `RELEASE_MAX`, mesmo com o laço parado).
pub const PREPARE_MAX: Duration = Duration::from_millis(1800);
/// Limite da thread do logind para soltar o inibidor sem resposta do laço: a suspensão
/// nunca fica presa pelo mide.
pub const RELEASE_MAX: Duration = Duration::from_secs(2);
/// "Dormindo" há tanto tempo de relógio monotônico (que para durante a suspensão) sem
/// aviso de volta: a suspensão não aconteceu (ou o aviso se perdeu) e as telas voltam.
pub const ASLEEP_MAX: Duration = Duration::from_secs(10);

/// Fase da suspensão vista pelo compositor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Uso normal.
    Awake,
    /// O logind avisou que vai suspender: desenhando a tela de bloqueio.
    Preparing { since: Instant },
    /// A tela de bloqueio foi entregue (ou o tempo acabou) e o inibidor foi solto:
    /// nenhum quadro novo até a volta.
    Asleep { since: Instant },
}

/// O que mudou numa conferência (`Sleep::poll`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    None,
    /// Pronto para suspender: o inibidor foi solto.
    Slept,
    /// Ficou "dormindo" tempo demais sem a volta: as telas voltam a desenhar.
    Expired,
}

/// Estado da suspensão, guardado no `State`.
pub struct Sleep {
    phase: Phase,
    /// Conta os pedidos de suspensão: o quadro com a tela de bloqueio de cada tela é
    /// marcado com o número do pedido em que foi entregue.
    generation: u64,
    /// Canal para a thread do logind soltar o inibidor.
    release: Option<mpsc::Sender<()>>,
}

impl Default for Sleep {
    fn default() -> Self {
        Self { phase: Phase::Awake, generation: 0, release: None }
    }
}

impl Sleep {
    pub fn set_release(&mut self, release: mpsc::Sender<()>) {
        self.release = Some(release);
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// Número do pedido de suspensão em andamento (0 antes do primeiro).
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Esperando a tela de bloqueio ser entregue.
    pub fn preparing(&self) -> bool {
        matches!(self.phase, Phase::Preparing { .. })
    }

    /// Pode mandar quadros à placa (não está "dormindo").
    pub fn frames_allowed(&self) -> bool {
        !matches!(self.phase, Phase::Asleep { .. })
    }

    /// O logind avisou que vai suspender.
    pub fn prepare(&mut self, now: Instant) {
        self.generation += 1;
        self.phase = Phase::Preparing { since: now };
    }

    /// Voltou da suspensão (aviso do logind, relógio ou entrada). Devolve se estava
    /// suspendendo ou dormindo.
    pub fn wake(&mut self) -> bool {
        std::mem::replace(&mut self.phase, Phase::Awake) != Phase::Awake
    }

    /// Confere a fase: `ready` diz se a tela de bloqueio já foi entregue a todos os
    /// monitores (ou não há o que esperar).
    pub fn poll(&mut self, now: Instant, ready: bool) -> Change {
        match self.phase {
            Phase::Awake => Change::None,
            Phase::Preparing { since } => {
                let late = now.saturating_duration_since(since) >= PREPARE_MAX;
                if !ready && !late {
                    return Change::None;
                }
                if late && !ready {
                    tracing::warn!("suspensão: a tela de bloqueio não chegou ao monitor a tempo; liberando assim mesmo");
                } else {
                    tracing::info!("suspensão: tela de bloqueio na tela; liberando o sistema para suspender");
                }
                self.phase = Phase::Asleep { since: now };
                if let Some(release) = &self.release {
                    let _ = release.send(());
                }
                Change::Slept
            }
            Phase::Asleep { since } => {
                if now.saturating_duration_since(since) < ASLEEP_MAX {
                    return Change::None;
                }
                tracing::warn!("suspensão: sem aviso de volta em {} s; as telas voltam a desenhar", ASLEEP_MAX.as_secs());
                self.phase = Phase::Awake;
                Change::Expired
            }
        }
    }
}

/// Pronto para suspender: sem bloqueio pedido, basta não haver troca de quadro em voo;
/// com ele, a tela bloqueada e o quadro com a tela de bloqueio entregue em cada monitor
/// (`lock_presented`); com as telas apagadas, basta a tela estar bloqueada.
pub fn ready(lock_wanted: bool, locked: bool, blanked: bool, flips_pending: bool, lock_presented: bool) -> bool {
    if blanked {
        return !lock_wanted || locked;
    }
    if !lock_wanted {
        return !flips_pending;
    }
    locked && lock_presented && !flips_pending
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn libera_so_com_a_tela_de_bloqueio_entregue() {
        let t0 = Instant::now();
        let (tx, rx) = mpsc::channel();
        let mut sleep = Sleep::default();
        sleep.set_release(tx);
        assert!(sleep.frames_allowed());
        sleep.prepare(t0);
        assert_eq!(sleep.generation(), 1);
        assert!(sleep.preparing() && sleep.frames_allowed());
        // Ainda sem a tela de bloqueio: espera.
        assert_eq!(sleep.poll(t0 + Duration::from_millis(100), false), Change::None);
        assert!(rx.try_recv().is_err());
        // Entregue: solta o inibidor e para de mandar quadros.
        assert_eq!(sleep.poll(t0 + Duration::from_millis(200), true), Change::Slept);
        assert!(rx.try_recv().is_ok());
        assert!(!sleep.frames_allowed());
        // A volta (logind ou relógio) devolve os quadros.
        assert!(sleep.wake());
        assert!(sleep.frames_allowed());
        assert!(!sleep.wake());
    }

    #[test]
    fn nunca_segura_a_suspensao_mais_que_o_limite() {
        let t0 = Instant::now();
        let (tx, rx) = mpsc::channel();
        let mut sleep = Sleep::default();
        sleep.set_release(tx);
        sleep.prepare(t0);
        assert_eq!(sleep.poll(t0 + PREPARE_MAX - Duration::from_millis(1), false), Change::None);
        assert_eq!(sleep.poll(t0 + PREPARE_MAX, false), Change::Slept);
        assert!(rx.try_recv().is_ok());
        assert!(PREPARE_MAX < RELEASE_MAX);
    }

    #[test]
    fn suspensao_cancelada_ou_sem_aviso_de_volta_nao_deixa_a_tela_parada() {
        let t0 = Instant::now();
        let mut sleep = Sleep::default();
        // Cancelada: o PrepareForSleep(false) chega logo depois (o relógio não avança).
        sleep.prepare(t0);
        sleep.poll(t0, true);
        assert!(sleep.wake());
        assert!(sleep.frames_allowed());
        // Sem aviso nenhum: depois de `ASLEEP_MAX` de relógio monotônico, volta sozinho.
        sleep.prepare(t0);
        sleep.poll(t0, true);
        assert_eq!(sleep.poll(t0 + ASLEEP_MAX - Duration::from_millis(1), true), Change::None);
        assert_eq!(sleep.poll(t0 + ASLEEP_MAX, true), Change::Expired);
        assert!(sleep.frames_allowed());
        // Outro pedido: número novo (o quadro de bloqueio do pedido anterior não vale).
        sleep.prepare(t0);
        assert_eq!(sleep.generation(), 3);
    }

    #[test]
    fn quando_esta_pronto_para_suspender() {
        // Sem bloqueio pedido: só falta não haver troca de quadro em voo.
        assert!(ready(false, false, false, false, false));
        assert!(!ready(false, false, false, true, false));
        // Com bloqueio: bloqueada, quadro de bloqueio entregue e nada em voo.
        assert!(!ready(true, false, false, false, false));
        assert!(!ready(true, true, false, false, false));
        assert!(!ready(true, true, false, true, true));
        assert!(ready(true, true, false, false, true));
        // Telas apagadas: nada vai à tela; basta estar bloqueada.
        assert!(ready(true, true, true, true, false));
        assert!(!ready(true, false, true, false, false));
    }
}
