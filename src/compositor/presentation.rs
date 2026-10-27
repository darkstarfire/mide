//! `wp_presentation` (presentation-time): avisa cada app de quando o quadro dele
//! apareceu na tela, com o relógio monotônico, a frequência da tela e o número do
//! vblank. O Chrome/Chromium (e o Firefox, o mpv, os jogos) usam esse aviso para
//! alinhar o próprio relógio de quadros ao vblank; sem ele, o Chrome segue um relógio
//! próprio que escorrega em relação à tela e a rolagem engasga.
//!
//! Fluxo: logo depois de desenhar uma tela, `collect` tira os pedidos de aviso das
//! superfícies que estão nela (janelas, popups, camadas, tela de bloqueio, ícone de
//! arrasto, cursor do app). No DRM, eles viajam junto com o quadro (dado do
//! `queue_buffer`) e saem como `presented` no vblank, com a hora da troca de página do
//! kernel; quadro que não chega à tela (tela refeita, erro, desligada) é descartado
//! (`discarded`, pelo `Drop` do Smithay). No backend sem tela e no winit, o quadro conta
//! como apresentado no fim do desenho (`present_now`).

use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime},
};

use smithay::{
    delegate_presentation,
    desktop::{
        layer_map_for_output,
        utils::{take_presentation_feedback_surface_tree, OutputPresentationFeedback},
        Window,
    },
    output::Output,
    reexports::{
        wayland_protocols::wp::presentation_time::server::wp_presentation_feedback::Kind,
        wayland_server::{protocol::wl_surface::WlSurface, DisplayHandle},
    },
    utils::{Clock, ClockSource, Monotonic},
    wayland::{
        compositor::SurfaceData,
        presentation::{PresentationState, Refresh},
    },
};

use crate::state::State;

/// Cria o global `wp_presentation`, no relógio monotônico (o mesmo do vblank do DRM).
pub fn init(display: &DisplayHandle) {
    // O global não some quando o `PresentationState` sai de escopo (ele não tem `Drop`).
    let _ = PresentationState::new::<State>(display, Monotonic::ID as u32);
}

delegate_presentation!(State);

/// Tira os pedidos de aviso das superfícies que aparecem em `output` neste quadro.
/// Janelas fora de todas as telas e as minimizadas não aparecem: os pedidos delas são
/// descartados na hora (o app não fica esperando um aviso que não viria).
pub fn collect(state: &State, output: &Output) -> OutputPresentationFeedback {
    let mut feedback = OutputPresentationFeedback::new(output);
    let here = |_: &WlSurface, _: &SurfaceData| Some(output.clone());
    let flags = |_: &WlSurface, _: &SurfaceData| Kind::empty();
    let mut hidden: Option<OutputPresentationFeedback> = None;
    for window in state.space.elements() {
        match primary_output(state, window) {
            // Janela em mais de uma tela: o aviso vem da principal dela.
            Some(primary) if &primary == output => window.take_presentation_feedback(&mut feedback, here, flags),
            Some(_) => {}
            None => {
                let discard = hidden.get_or_insert_with(|| OutputPresentationFeedback::new(output));
                window.take_presentation_feedback(discard, here, flags);
            }
        }
    }
    for minimized in &state.minimized {
        let discard = hidden.get_or_insert_with(|| OutputPresentationFeedback::new(output));
        minimized.window.take_presentation_feedback(discard, here, flags);
    }
    if let Some(mut discard) = hidden {
        discard.discarded();
    }
    for layer in layer_map_for_output(output).layers() {
        // Inclui os popups da camada.
        layer.take_presentation_feedback(&mut feedback, here, flags);
    }
    if let Some(surface) = state.lock.surface_for(output) {
        take_presentation_feedback_surface_tree(surface.wl_surface(), &mut feedback, here, flags);
    }
    if let Some((icon, _)) = &state.dnd_icon {
        take_presentation_feedback_surface_tree(icon, &mut feedback, here, flags);
    }
    if let smithay::input::pointer::CursorImageStatus::Surface(surface) = &state.cursor_status {
        take_presentation_feedback_surface_tree(surface, &mut feedback, here, flags);
    }
    feedback
}

/// A tela principal de uma janela: a primeira (na ordem das telas) que ela toca; `None`
/// se está fora de todas. Pela geometria, e não pelo `outputs_for_element` do Smithay,
/// que só se atualiza no `space.refresh` (uma janela recém-aberta ficaria sem tela no
/// primeiro quadro e perderia o aviso).
pub fn primary_output(state: &State, window: &Window) -> Option<Output> {
    let bbox = state.space.element_bbox(window)?;
    state
        .space
        .outputs()
        .find(|o| state.space.output_geometry(o).is_some_and(|g| g.overlaps(bbox)))
        .cloned()
}

/// Intervalo entre quadros da tela (o `refresh` do protocolo), pela frequência do modo
/// atual em milihertz; `Unknown` sem modo ou com frequência zero.
pub fn refresh_of(output: &Output) -> Refresh {
    match output.current_mode().map(|m| m.refresh) {
        Some(mhz) if mhz > 0 => Refresh::fixed(refresh_interval(mhz as u32)),
        _ => Refresh::Unknown,
    }
}

/// Duração de um quadro numa tela de `mhz` milihertz (60000 → 16,666… ms).
pub fn refresh_interval(mhz: u32) -> Duration {
    Duration::from_nanos(1_000_000_000_000 / u64::from(mhz.max(1)))
}

/// Hora de uma troca de página do DRM no relógio monotônico. O kernel manda a hora no
/// relógio monotônico (o normal) ou, em drivers antigos, no de tempo real: esta é
/// convertida pela diferença entre os dois relógios agora. Devolve também se a hora veio
/// do hardware (o aviso leva `hw_clock`).
pub fn vblank_time(meta: Option<&smithay::backend::drm::DrmEventMetadata>) -> (Duration, bool) {
    use smithay::backend::drm::DrmEventTime;
    let now: Duration = Clock::<Monotonic>::new().now().into();
    match meta.map(|m| m.time) {
        Some(DrmEventTime::Monotonic(t)) if !t.is_zero() => (t, true),
        Some(DrmEventTime::Realtime(t)) => {
            let ago = SystemTime::now().duration_since(t).unwrap_or(Duration::ZERO);
            (now.saturating_sub(ago), true)
        }
        _ => (now, false),
    }
}

/// O quadro chegou à tela no vblank: avisa os apps com a hora, a frequência e o número
/// do vblank (`seq`), com as marcas de sincronia vertical e de conclusão pelo hardware.
pub fn present_vblank(mut feedback: OutputPresentationFeedback, output: &Output, time: Duration, hw_clock: bool, seq: u64) {
    let mut flags = Kind::Vsync | Kind::HwCompletion;
    if hw_clock {
        flags |= Kind::HwClock;
    }
    feedback.presented::<_, Monotonic>(time, refresh_of(output), seq, flags);
}

/// Backends sem vblank (sem tela, winit): o quadro conta como apresentado agora, com um
/// contador de quadros próprio da tela.
pub fn present_now(mut feedback: OutputPresentationFeedback, output: &Output) {
    let now: Duration = Clock::<Monotonic>::new().now().into();
    let counter = output.user_data().get_or_insert_threadsafe(|| AtomicU64::new(0));
    let seq = counter.fetch_add(1, Ordering::Relaxed) + 1;
    feedback.presented::<_, Monotonic>(now, refresh_of(output), seq, Kind::empty());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intervalo_da_frequencia() {
        assert_eq!(refresh_interval(60_000), Duration::from_nanos(16_666_666));
        assert_eq!(refresh_interval(144_000), Duration::from_nanos(6_944_444));
        assert_eq!(refresh_interval(59_940), Duration::from_nanos(16_683_350));
        // Frequência zero não divide por zero.
        assert!(refresh_interval(0) > Duration::ZERO);
    }

    #[test]
    fn hora_do_vblank_monotonica_ou_convertida() {
        use smithay::backend::drm::{DrmEventMetadata, DrmEventTime};
        let mono = DrmEventMetadata { time: DrmEventTime::Monotonic(Duration::from_secs(5)), sequence: 1 };
        assert_eq!(vblank_time(Some(&mono)), (Duration::from_secs(5), true));
        // Tempo real de 10 ms atrás vira o monotônico de agora menos 10 ms.
        let real = DrmEventMetadata { time: DrmEventTime::Realtime(SystemTime::now() - Duration::from_millis(10)), sequence: 1 };
        let now: Duration = Clock::<Monotonic>::new().now().into();
        let (t, hw) = vblank_time(Some(&real));
        assert!(hw);
        assert!(now.saturating_sub(t) >= Duration::from_millis(9) && now.saturating_sub(t) < Duration::from_millis(500));
        // Sem a hora do kernel: agora, sem `hw_clock`.
        let (t, hw) = vblank_time(None);
        assert!(!hw && t >= now);
    }
}
