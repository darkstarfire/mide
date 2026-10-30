//! Arranjo das telas: onde cada uma fica no espaço de todas, a partir das
//! Configurações (`[monitors]`). Lógica pura, sem GPU, usada pelos backends.

use crate::config::Monitors;

/// Uma tela conectada: nome do conector e tamanho em pixels lógicos (modo ÷ escala).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Head {
    pub name: String,
    pub size: (i32, i32),
}

/// Posição (canto superior esquerdo) de cada tela ligada, na ordem recebida. Telas com
/// `x`/`y` nas Configurações ficam onde foram postas; as outras entram lado a lado, à
/// direita das já arranjadas, alinhadas no topo. A principal vem primeiro quando não
/// tem posição, então fica em (0, 0).
pub fn arrange(heads: &[Head], monitors: &Monitors) -> Vec<(String, (i32, i32))> {
    let enabled: Vec<&Head> =
        heads.iter().filter(|h| monitors.outputs.get(&h.name).is_none_or(|m| m.enabled)).collect();
    // Principal primeiro, depois a ordem dos conectores.
    let mut order: Vec<&Head> = enabled.clone();
    if let Some(primary) = monitors.primary.as_deref() {
        order.sort_by_key(|h| h.name != primary);
    }
    // Espelhadas: todas no mesmo lugar.
    if monitors.mirror {
        return enabled.iter().map(|h| (h.name.clone(), (0, 0))).collect();
    }
    let mut placed: Vec<(String, (i32, i32), (i32, i32))> = Vec::new();
    // Primeiro as que têm posição definida.
    for head in &order {
        if let Some(m) = monitors.outputs.get(&head.name) {
            if let (Some(x), Some(y)) = (m.x, m.y) {
                placed.push((head.name.clone(), (x, y), head.size));
            }
        }
    }
    for head in &order {
        if placed.iter().any(|(name, ..)| *name == head.name) {
            continue;
        }
        let right = placed.iter().map(|(_, (x, _), (w, _))| x + w).max().unwrap_or(0);
        let top = placed.iter().map(|(_, (_, y), _)| *y).min().unwrap_or(0);
        placed.push((head.name.clone(), (right, top), head.size));
    }
    // O arranjo começa em (0, 0): todas deslocadas juntas (a posição relativa fica).
    let min_x = placed.iter().map(|(_, (x, _), _)| *x).min().unwrap_or(0);
    let min_y = placed.iter().map(|(_, (_, y), _)| *y).min().unwrap_or(0);
    // Na ordem recebida (a dos conectores).
    enabled
        .iter()
        .filter_map(|h| placed.iter().find(|(name, ..)| *name == h.name).map(|(name, (x, y), _)| (name.clone(), (x - min_x, y - min_y))))
        .collect()
}

/// Tamanho lógico de uma tela: o modo dividido pela escala, arredondado.
pub fn logical_size(mode: (i32, i32), scale: f64) -> (i32, i32) {
    let scale = if scale.is_finite() && scale >= 0.5 { scale } else { 1.0 };
    ((mode.0 as f64 / scale).round() as i32, (mode.1 as f64 / scale).round() as i32)
}

/// Tempo que uma tela pode sumir sem que as janelas saiam dela. Monitores HDMI e DP
/// derrubam o sinal de "conectado" por um instante ao acordar do DPMS (o log mostrava
/// "tela HDMI-A-1 desligada" e ela de volta ~1 s depois); sem essa folga, as janelas
/// iam para a principal a cada vez que a tela apagava e acendia.
pub const UNPLUG_GRACE: std::time::Duration = std::time::Duration::from_secs(3);

/// O que fazer com uma tela ligada pelo que a placa diz agora.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presence {
    /// Conectada, como estava.
    Here,
    /// Sumiu há pouco e voltou: nada muda (a tela só é recomeçada).
    Back,
    /// Sumiu: continua ligada, olhar de novo daqui a esse tempo.
    Wait(std::time::Duration),
    /// Sumiu de vez (passou da folga): desligar.
    Gone,
}

/// Decide pela folga (`UNPLUG_GRACE`) se uma tela que a placa não mostra mais
/// conectada sai ou espera. `missing` guarda desde quando ela sumiu. Com as telas
/// apagadas a conta recomeça (um monitor dormindo pode se dizer desconectado; ele sai
/// só se continuar sumido depois de acender).
pub fn presence(missing: &mut Option<std::time::Instant>, connected: bool, blanked: bool, now: std::time::Instant) -> Presence {
    if connected {
        return if missing.take().is_some() { Presence::Back } else { Presence::Here };
    }
    if blanked {
        *missing = Some(now);
        return Presence::Wait(UNPLUG_GRACE);
    }
    let since = *missing.get_or_insert(now);
    let gone = now.saturating_duration_since(since);
    if gone >= UNPLUG_GRACE { Presence::Gone } else { Presence::Wait(UNPLUG_GRACE - gone) }
}

/// Olha as telas de novo daqui a `after` (o `outputs_waker` do backend refaz as telas).
pub fn recheck_later(state: &crate::state::State, after: std::time::Duration) {
    let Some(waker) = state.outputs_waker.clone() else {
        return;
    };
    let timer = smithay::reexports::calloop::timer::Timer::from_duration(after + std::time::Duration::from_millis(50));
    let _ = state.loop_handle.insert_source(timer, move |_, _, _| {
        waker.ping();
        smithay::reexports::calloop::timer::TimeoutAction::Drop
    });
}

/// Como a janela estava na tela de onde saiu.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Fit {
    /// Solta, na posição guardada.
    Free,
    /// Maximizada na área útil da tela.
    Maximized,
    /// Numa parte da área útil (metade, terço, quarto), em frações (x, y, largura,
    /// altura), e se encostava na borda esquerda.
    Part { frac: (f32, f32, f32, f32), left: bool },
}

/// Janela tirada de uma tela que saiu (desconectada ou desligada): de onde veio e onde
/// o mide a pôs. Quando uma tela com o mesmo nome volta e a janela continua onde o
/// mide a deixou, ela volta à tela dela, como no macOS. Genérico na janela para os
/// testes de unidade.
#[derive(Debug, Clone, PartialEq)]
pub struct Displaced<W> {
    pub window: W,
    /// Nome da tela de origem (o conector: `HDMI-A-1`).
    pub output: String,
    /// Posição da janela em relação ao canto da tela de origem.
    pub rel: (i32, i32),
    pub fit: Fit,
    /// Onde o mide pôs a janela na tela que a recebeu: se ela não está mais lá (o
    /// usuário a mudou de lugar), fica onde está.
    pub placed_at: (i32, i32),
}

/// Guarda de onde a janela veio. Uma janela já tirada de outra tela antes continua
/// com a origem primeira (só o lugar onde foi posta muda).
pub fn remember<W: PartialEq>(list: &mut Vec<Displaced<W>>, entry: Displaced<W>) {
    match list.iter_mut().find(|d| d.window == entry.window) {
        Some(old) => old.placed_at = entry.placed_at,
        None => list.push(entry),
    }
}

/// Tira da lista as janelas cuja tela de origem está em `live` (voltou).
pub fn returning<W>(list: &mut Vec<Displaced<W>>, live: &[String]) -> Vec<Displaced<W>> {
    let (back, stay): (Vec<_>, Vec<_>) = std::mem::take(list).into_iter().partition(|d| live.contains(&d.output));
    *list = stay;
    back
}

/// Frações de `rect` dentro de `area` (x, y, largura, altura).
pub fn fraction(rect: (i32, i32, i32, i32), area: (i32, i32, i32, i32)) -> (f32, f32, f32, f32) {
    let (w, h) = (area.2.max(1) as f32, area.3.max(1) as f32);
    ((rect.0 - area.0) as f32 / w, (rect.1 - area.1) as f32 / h, rect.2 as f32 / w, rect.3 as f32 / h)
}

/// O retângulo das frações `frac` dentro de `area` (o contrário de `fraction`).
pub fn from_fraction(frac: (f32, f32, f32, f32), area: (i32, i32, i32, i32)) -> (i32, i32, i32, i32) {
    let (w, h) = (area.2 as f32, area.3 as f32);
    (area.0 + (frac.0 * w).round() as i32, area.1 + (frac.1 * h).round() as i32, ((frac.2 * w).round() as i32).max(80), ((frac.3 * h).round() as i32).max(60))
}

/// Posição de volta numa tela em `out` (x, y, largura, altura): a mesma em relação
/// ao canto dela, sem deixar a janela (de tamanho `size`) sair da tela, se couber.
pub fn back_location(rel: (i32, i32), size: (i32, i32), out: (i32, i32, i32, i32)) -> (i32, i32) {
    let x = out.0 + rel.0.clamp(0, (out.2 - size.0).max(0));
    let y = out.1 + rel.1.clamp(0, (out.3 - size.1).max(0));
    (x, y)
}

/// Tela desligada ou desconectada: o `wl_output` dela some para os apps (sem isso o
/// global ficava anunciado e, ao religar, os apps viam duas saídas com o mesmo nome).
/// Primeiro só escondido (um app que ainda vai ligar-se a ele não recebe erro); a
/// remoção de vez fica para a próxima mudança de telas (`sweep_globals`).
pub fn retire_global(state: &mut crate::state::State, id: smithay::reexports::wayland_server::backend::GlobalId) {
    state.display_handle.disable_global::<crate::state::State>(id.clone());
    state.retired_output_globals.push(id);
}

/// Remove de vez os globais escondidos numa mudança de telas anterior.
pub fn sweep_globals(state: &mut crate::state::State) {
    for id in std::mem::take(&mut state.retired_output_globals) {
        state.display_handle.remove_global::<crate::state::State>(id);
    }
}

/// Aplica o arranjo às telas ligadas (`live`): posição de cada uma no espaço, telas
/// que sumiram fora, janelas perdidas (numa tela que saiu) trazidas para a principal
/// e, se a principal mudou, barra e dock reabertas nela.
pub fn place(state: &mut crate::state::State, live: &[smithay::output::Output]) {
    use smithay::desktop::layer_map_for_output;
    state.screencopy.fail_outputs(|o| !live.contains(o));
    // Telas que saem: antes de tirá-las do espaço, guarda como estava cada janela
    // nelas (para voltar quando a tela voltar).
    let mut leaving_windows: Vec<Displaced<smithay::desktop::Window>> = Vec::new();
    let mut left_names: Vec<String> = Vec::new();
    for output in state.space.outputs().cloned().collect::<Vec<_>>() {
        if !live.contains(&output) {
            tracing::info!("tela {} saiu", output.name());
            if let (Some(geo), Some(area)) = (state.space.output_geometry(&output), state.usable_area_of(&output)) {
                leaving_windows.extend(windows_on(state, &output.name(), geo, area));
            }
            left_names.push(output.name());
            state.space.unmap_output(&output);
        }
    }
    // Escala de cada tela: a das Configurações, ou, espelhando, a que deixa a largura
    // igual à da principal (o mesmo conteúdo em telas de tamanhos diferentes).
    let primary_name = state.config.monitors.primary.clone().filter(|p| live.iter().any(|o| o.name() == *p)).or_else(|| live.first().map(|o| o.name()));
    // Tamanho da tela já girado (tela interna em retrato, pelo sensor).
    let mode_of = |o: &smithay::output::Output| o.current_mode().map(|m| o.current_transform().transform_size(m.size)).map(|m| (m.w, m.h)).unwrap_or((1366, 768));
    let configured = |name: &str| state.config.monitors.outputs.get(name).map(|m| m.scale).filter(|s| s.is_finite() && *s >= 0.5).unwrap_or(1.0);
    let primary_width = primary_name.as_ref().and_then(|p| live.iter().find(|o| o.name() == *p)).map(|o| logical_size(mode_of(o), configured(&o.name())).0);
    for output in live {
        let name = output.name();
        let wanted = match (state.config.monitors.mirror, primary_width) {
            (true, Some(width)) if Some(&name) != primary_name.as_ref() && width > 0 => mode_of(output).0 as f64 / width as f64,
            _ => configured(&name),
        };
        if (output.current_scale().fractional_scale() - wanted).abs() > 0.001 {
            tracing::info!("tela {name}: escala {wanted:.3}");
            output.change_current_state(None, None, Some(smithay::output::Scale::Fractional(wanted)), None);
        }
    }
    let heads: Vec<Head> = live
        .iter()
        .map(|o| {
            Head { name: o.name(), size: logical_size(mode_of(o), o.current_scale().fractional_scale()) }
        })
        .collect();
    for (name, pos) in arrange(&heads, &state.config.monitors) {
        let Some(output) = live.iter().find(|o| o.name() == name) else {
            continue;
        };
        output.change_current_state(None, None, None, Some(pos.into()));
        state.space.map_output(output, pos);
        layer_map_for_output(output).arrange();
        tracing::info!("tela {name} em ({}, {})", pos.0, pos.1);
    }
    // Janelas fora de todas as telas (a tela delas saiu): para a área útil da principal.
    if let Some(area) = state.usable_area() {
        let lost: Vec<_> = state
            .space
            .elements()
            .filter(|w| {
                state.space.element_location(w).is_some_and(|loc| {
                    let center = loc + smithay::utils::Point::from((w.geometry().size.w / 2, 10));
                    state.space.output_under(center.to_f64()).next().is_none()
                })
            })
            .cloned()
            .collect();
        for (i, window) in lost.into_iter().enumerate() {
            let offset = 32 * i as i32;
            let loc = area.loc + smithay::utils::Point::from((48 + offset, 48 + offset));
            let origin = leaving_windows.iter().position(|d| d.window == window).map(|i| leaving_windows.swap_remove(i));
            let toplevel = window.toplevel().cloned();
            // Maximizada ou numa parte da tela: o mesmo na principal (como no macOS);
            // solta: num canto da área útil, em escada.
            match (origin.as_ref().map(|d| d.fit), toplevel) {
                (Some(Fit::Maximized), Some(t)) => state.maximize_in(&t, area),
                (Some(Fit::Part { frac, left }), Some(t)) => {
                    let r = from_fraction(frac, (area.loc.x, area.loc.y, area.size.w, area.size.h));
                    state.snap_to(&t, smithay::utils::Rectangle::new((r.0, r.1).into(), (r.2, r.3).into()), left);
                }
                _ => state.space.map_element(window.clone(), loc, false),
            }
            if let Some(mut origin) = origin {
                let at = state.space.element_location(&window).unwrap_or(loc);
                origin.placed_at = (at.x, at.y);
                tracing::info!("janela da tela {} levada para a principal em ({}, {}); volta quando a tela voltar", origin.output, at.x, at.y);
                remember(&mut state.displaced, origin);
            }
        }
    }
    bring_back(state, live, &left_names);
    // Barra, dock e área de trabalho ficam na principal: se ela mudou (nas
    // Configurações ou porque a tela deles saiu), eles são reabertos nela.
    let primary = state.primary_output().map(|o| o.name());
    match &state.shell_output {
        None => state.shell_output = primary,
        Some(current) if primary.is_some() && primary.as_ref() != Some(current) => {
            tracing::info!("tela principal agora é {}: reabrindo barra e dock nela", primary.as_deref().unwrap_or("-"));
            state.shell_output = primary;
            crate::restart_shell();
        }
        _ => {}
    }
    state.wake();
}

/// Janelas numa tela (pelo ponto do meio do topo, como a conta das janelas perdidas),
/// com o jeito que estavam nela.
fn windows_on(
    state: &crate::state::State,
    name: &str,
    geo: smithay::utils::Rectangle<i32, smithay::utils::Logical>,
    area: smithay::utils::Rectangle<i32, smithay::utils::Logical>,
) -> Vec<Displaced<smithay::desktop::Window>> {
    use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State as XdgState;
    state
        .space
        .elements()
        .filter_map(|w| {
            let loc = state.space.element_location(w)?;
            let size = w.geometry().size;
            let top = loc + smithay::utils::Point::from((size.w / 2, 10));
            if !geo.contains(top) {
                return None;
            }
            let maximized = w.toplevel().is_some_and(|t| t.current_state().states.contains(XdgState::Maximized));
            let tiled = w.toplevel().is_some_and(|t| state.tiled.contains(t.wl_surface()));
            let fit = if maximized {
                Fit::Maximized
            } else if tiled {
                // O encaixe põe o conteúdo abaixo da barra de título, no topo da parte.
                let bar = crate::state::bar_height_for(w);
                let rect = (loc.x, loc.y - bar, size.w, size.h + bar);
                let left = w.toplevel().is_some_and(|t| t.current_state().states.contains(XdgState::TiledLeft));
                Fit::Part { frac: fraction(rect, (area.loc.x, area.loc.y, area.size.w, area.size.h)), left }
            } else {
                Fit::Free
            };
            Some(Displaced { window: w.clone(), output: name.to_string(), rel: (loc.x - geo.loc.x, loc.y - geo.loc.y), fit, placed_at: (loc.x, loc.y) })
        })
        .collect()
}

/// Devolve às telas que voltaram as janelas tiradas delas, se continuam onde o mide as
/// pôs (o usuário não as mudou de lugar nem de estado). Minimizadas esperam.
fn bring_back(state: &mut crate::state::State, live: &[smithay::output::Output], just_left: &[String]) {
    use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State as XdgState;
    state.displaced.retain(|d| smithay::utils::IsAlive::alive(&d.window));
    let names: Vec<String> = live.iter().map(|o| o.name()).filter(|n| !just_left.contains(n)).collect();
    for d in returning(&mut state.displaced, &names) {
        let Some(output) = live.iter().find(|o| o.name() == d.output) else {
            continue;
        };
        let (Some(geo), Some(area)) = (state.space.output_geometry(output), state.usable_area_of(output)) else {
            state.displaced.push(d);
            continue;
        };
        let Some(loc) = state.space.element_location(&d.window) else {
            // Minimizada (fora do espaço): fica no registro.
            state.displaced.push(d);
            continue;
        };
        let toplevel = d.window.toplevel().cloned();
        let maximized = toplevel.as_ref().is_some_and(|t| t.current_state().states.contains(XdgState::Maximized));
        let tiled = toplevel.as_ref().is_some_and(|t| state.tiled.contains(t.wl_surface()));
        let untouched = match d.fit {
            Fit::Free => (loc.x, loc.y) == d.placed_at && !maximized && !tiled,
            Fit::Maximized => maximized,
            Fit::Part { .. } => tiled,
        };
        if !untouched {
            tracing::info!("tela {} voltou; a janela que veio dela foi mexida e fica onde está", d.output);
            continue;
        }
        match (d.fit, toplevel) {
            (Fit::Maximized, Some(t)) => state.maximize_in(&t, area),
            (Fit::Part { frac, left }, Some(t)) => {
                let r = from_fraction(frac, (area.loc.x, area.loc.y, area.size.w, area.size.h));
                state.snap_to(&t, smithay::utils::Rectangle::new((r.0, r.1).into(), (r.2, r.3).into()), left);
            }
            _ => {
                let size = d.window.geometry().size;
                let (x, y) = back_location(d.rel, (size.w, size.h), (geo.loc.x, geo.loc.y, geo.size.w, geo.size.h));
                state.space.map_element(d.window.clone(), (x, y), false);
            }
        }
        let at = state.space.element_location(&d.window).unwrap_or(loc);
        tracing::info!("janela de volta à tela {} em ({}, {})", d.output, at.x, at.y);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Monitor;

    fn head(name: &str, w: i32, h: i32) -> Head {
        Head { name: name.into(), size: (w, h) }
    }

    #[test]
    fn espelhadas_no_mesmo_lugar() {
        let heads = [head("eDP-1", 1366, 768), head("HDMI-A-1", 1920, 1080)];
        let monitors = Monitors { mirror: true, ..Default::default() };
        assert_eq!(arrange(&heads, &monitors), vec![("eDP-1".into(), (0, 0)), ("HDMI-A-1".into(), (0, 0))]);
    }

    #[test]
    fn lado_a_lado_sem_configuracao() {
        let heads = [head("eDP-1", 1366, 768), head("HDMI-A-1", 1920, 1080)];
        let got = arrange(&heads, &Monitors::default());
        assert_eq!(got, vec![("eDP-1".into(), (0, 0)), ("HDMI-A-1".into(), (1366, 0))]);
    }

    #[test]
    fn principal_primeiro_e_posicao_configurada() {
        let heads = [head("DP-1", 1920, 1080), head("DP-2", 2560, 1440)];
        let mut monitors = Monitors { primary: Some("DP-2".into()), ..Default::default() };
        // Sem posição: a principal em (0, 0) e a outra à direita.
        let got = arrange(&heads, &monitors);
        assert_eq!(got, vec![("DP-1".into(), (2560, 0)), ("DP-2".into(), (0, 0))]);
        // Com posição: cada uma onde foi posta (a de 1080 alinhada embaixo, à esquerda).
        monitors.outputs.insert("DP-1".into(), Monitor { x: Some(0), y: Some(360), ..Default::default() });
        monitors.outputs.insert("DP-2".into(), Monitor { x: Some(1920), y: Some(0), ..Default::default() });
        let got = arrange(&heads, &monitors);
        assert_eq!(got, vec![("DP-1".into(), (0, 360)), ("DP-2".into(), (1920, 0))]);
        // Só uma com posição (longe da origem): o arranjo é deslocado para (0, 0).
        monitors.outputs.remove("DP-2");
        monitors.outputs.insert("DP-1".into(), Monitor { x: Some(1920), y: Some(200), ..Default::default() });
        let got = arrange(&heads, &monitors);
        assert_eq!(got, vec![("DP-1".into(), (0, 0)), ("DP-2".into(), (1920, 0))]);
    }

    #[test]
    fn tela_desligada_fica_de_fora() {
        let heads = [head("eDP-1", 1366, 768), head("HDMI-A-1", 1920, 1080)];
        let mut monitors = Monitors::default();
        monitors.outputs.insert("eDP-1".into(), Monitor { enabled: false, ..Default::default() });
        assert_eq!(arrange(&heads, &monitors), vec![("HDMI-A-1".into(), (0, 0))]);
    }

    #[test]
    fn tamanho_logico_com_escala() {
        assert_eq!(logical_size((2560, 1440), 1.25), (2048, 1152));
        assert_eq!(logical_size((1920, 1080), 2.0), (960, 540));
        assert_eq!(logical_size((1920, 1080), 0.0), (1920, 1080));
    }

    #[test]
    fn folga_ao_sumir_e_voltar() {
        use std::time::{Duration, Instant};
        let t0 = Instant::now();
        let mut missing = None;
        assert_eq!(presence(&mut missing, true, false, t0), Presence::Here);
        // Some: espera a folga inteira.
        assert_eq!(presence(&mut missing, false, false, t0), Presence::Wait(UNPLUG_GRACE));
        // Volta em 1 s (o monitor acordando do DPMS): nada sai.
        assert_eq!(presence(&mut missing, true, false, t0 + Duration::from_secs(1)), Presence::Back);
        assert_eq!(missing, None);
        // Some e não volta: sai depois da folga.
        presence(&mut missing, false, false, t0);
        assert_eq!(presence(&mut missing, false, false, t0 + Duration::from_secs(1)), Presence::Wait(UNPLUG_GRACE - Duration::from_secs(1)));
        assert_eq!(presence(&mut missing, false, false, t0 + UNPLUG_GRACE), Presence::Gone);
    }

    #[test]
    fn tela_apagada_nao_sai() {
        use std::time::{Duration, Instant};
        let t0 = Instant::now();
        let mut missing = None;
        // Dormindo, sumida por muito tempo: espera.
        presence(&mut missing, false, true, t0);
        assert_eq!(presence(&mut missing, false, true, t0 + Duration::from_secs(60)), Presence::Wait(UNPLUG_GRACE));
        // Acendeu e continua sumida: a folga conta a partir da última vez apagada.
        let t1 = t0 + Duration::from_secs(61);
        assert_eq!(presence(&mut missing, false, false, t1), Presence::Wait(UNPLUG_GRACE - Duration::from_secs(1)));
        assert_eq!(presence(&mut missing, true, false, t1 + Duration::from_secs(1)), Presence::Back);
    }

    fn displaced(window: u32, output: &str) -> Displaced<u32> {
        Displaced { window, output: output.into(), rel: (0, 30), fit: Fit::Maximized, placed_at: (0, 28) }
    }

    #[test]
    fn registro_de_onde_veio() {
        let mut list = Vec::new();
        remember(&mut list, displaced(1, "HDMI-A-1"));
        remember(&mut list, displaced(2, "DP-1"));
        // Tirada de novo (de outra tela): a origem primeira fica, o lugar muda.
        remember(&mut list, Displaced { placed_at: (48, 76), ..displaced(1, "eDP-1") });
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].output, "HDMI-A-1");
        assert_eq!(list[0].placed_at, (48, 76));
        // Volta só a da tela que voltou.
        let back = returning(&mut list, &["eDP-1".into(), "HDMI-A-1".into()]);
        assert_eq!(back.iter().map(|d| d.window).collect::<Vec<_>>(), vec![1]);
        assert_eq!(list.iter().map(|d| d.window).collect::<Vec<_>>(), vec![2]);
    }

    #[test]
    fn fracoes_e_posicao_de_volta() {
        // Metade direita de uma área 2560x1440 em (1920, 0) vira a metade direita de
        // uma área 1366x740 em (0, 28).
        let frac = fraction((3200, 0, 1280, 1440), (1920, 0, 2560, 1440));
        assert_eq!(frac, (0.5, 0.0, 0.5, 1.0));
        assert_eq!(from_fraction(frac, (0, 28, 1366, 740)), (683, 28, 683, 740));
        // Mesma posição em relação ao canto, sem sair da tela.
        assert_eq!(back_location((100, 76), (800, 600), (1920, 0, 2560, 1440)), (2020, 76));
        assert_eq!(back_location((2400, 76), (800, 600), (1920, 0, 2560, 1440)), (3680, 76));
        assert_eq!(back_location((-50, -10), (800, 600), (1920, 0, 2560, 1440)), (1920, 0));
    }
}
