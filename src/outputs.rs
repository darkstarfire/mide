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

/// Aplica o arranjo às telas ligadas (`live`): posição de cada uma no espaço, telas
/// que sumiram fora, janelas perdidas (numa tela que saiu) trazidas para a principal
/// e, se a principal mudou, barra e dock reabertas nela.
pub fn place(state: &mut crate::state::State, live: &[smithay::output::Output]) {
    use smithay::desktop::layer_map_for_output;
    for output in state.space.outputs().cloned().collect::<Vec<_>>() {
        if !live.contains(&output) {
            tracing::info!("tela {} saiu", output.name());
            state.space.unmap_output(&output);
        }
    }
    let heads: Vec<Head> = live
        .iter()
        .map(|o| {
            let mode = o.current_mode().map(|m| (m.size.w, m.size.h)).unwrap_or((1366, 768));
            Head { name: o.name(), size: logical_size(mode, o.current_scale().fractional_scale()) }
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
            state.space.map_element(window, loc, false);
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Monitor;

    fn head(name: &str, w: i32, h: i32) -> Head {
        Head { name: name.into(), size: (w, h) }
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
}
