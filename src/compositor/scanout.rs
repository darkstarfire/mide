//! Scanout direto no modo de jogo em tela cheia (A18): quando a janela em tela cheia
//! cobre a tela inteira e nada do mide aparece por cima, o buffer do app (dmabuf) vai
//! direto ao plano principal da placa, sem composição na GPU. Qualquer coisa que
//! precise da cena (menu do app, diálogo, aviso de volume, banner, barra revelada,
//! captura de tela, gravação, cursor desenhado pela cena, animação) volta à composição
//! normal no mesmo quadro.
//!
//! Aqui fica só a escolha (o que o backend DRM pode mandar à placa e, se não, por quê);
//! a entrega ao DRM fica no `backend_udev`.

use smithay::{
    backend::renderer::utils::{with_renderer_surface_state, Buffer},
    desktop::{layer_map_for_output, PopupManager},
    output::Output,
    utils::Transform,
    wayland::{
        compositor::{with_surface_tree_downward, TraversalAction},
        shell::wlr_layer::Layer,
    },
};

use crate::state::State;

/// Buffer do app pronto para ir à placa.
pub struct Candidate {
    pub buffer: Buffer,
    /// Tamanho do buffer em pixels.
    pub size: (i32, i32),
}

/// O buffer da janela em tela cheia da tela `output` pode ir direto à placa (modo de
/// pixels `mode`)? `Err` diz por que não (para o log).
pub fn candidate(state: &State, output: &Output, mode: (i32, i32)) -> Result<Candidate, String> {
    let Some(window) = state.game_window(output) else {
        return Err("sem modo de jogo".into());
    };
    let Some(toplevel) = window.toplevel() else {
        return Err("janela sem xdg_toplevel".into());
    };
    let surface = toplevel.wl_surface().clone();
    if state.animation_for(&surface).is_some() {
        return Err("animação da janela".into());
    }
    // Nada do mide por cima: aviso de volume, arrasto, prévias, encaixe, barra revelada,
    // capturas e gravações (que leem o quadro composto).
    let name = output.name();
    if state.osd.is_some() {
        return Err("aviso de volume ou brilho à mostra".into());
    }
    if state.dnd_icon.is_some() || state.preview.is_some() || state.snap.is_some() || !state.ghosts.is_empty() || !state.appearing.is_empty() || state.launcher_anim.is_some() {
        return Err("arrasto, prévia ou encaixe na tela".into());
    }
    if state.fs_reveal.progress(&name, std::time::Instant::now()) > 0.0 || state.fs_reveal.shown_on(&name) {
        return Err("barra revelada".into());
    }
    if state.recording.is_some() || state.screenshot_requests.iter().any(|(n, _)| *n == name) || state.screencopy.pending_for(output) {
        return Err("captura ou gravação da tela".into());
    }
    if state.screencopy.wants_cursor() {
        return Err("captura com o cursor".into());
    }
    // A janela de cima na tela (diálogos com pai também contam aqui).
    let screen = state.space.output_geometry(output).ok_or("tela fora do arranjo")?;
    let above = state
        .space
        .elements()
        .skip_while(|w| **w != window)
        .skip(1)
        .any(|w| state.space.element_bbox(w).is_some_and(|b| b.overlaps(screen)));
    if above {
        return Err("diálogo ou janela por cima".into());
    }
    if PopupManager::popups_for_surface(&surface).next().is_some() {
        return Err("menu do app aberto".into());
    }
    // Camadas à mostra (notificações, menus da barra, captura): só as escondidas pela
    // tela cheia e as de fundo podem existir.
    {
        let source = state.layer_output(output);
        let map = layer_map_for_output(&source);
        for layer in map.layers() {
            if matches!(layer.layer(), Layer::Background | Layer::Bottom) || state.layer_offset(output, layer).is_none() {
                continue;
            }
            return Err(format!("camada \"{}\" à mostra", layer.namespace()));
        }
    }
    // Transparência da janela inteira (`[opacity]`).
    let app_id = smithay::wayland::compositor::with_states(&surface, |states| {
        states
            .data_map
            .get::<smithay::wayland::shell::xdg::XdgToplevelSurfaceData>()
            .and_then(|d| d.lock().ok()?.app_id.clone())
    })
    .unwrap_or_default();
    if state.config.opacity.for_window(&app_id, true) < 0.999 {
        return Err("janela com transparência".into());
    }
    // Posição: a superfície começa no canto da tela (sem sombra do app em volta).
    let loc = state.space.element_location(&window).ok_or("janela fora do espaço")?;
    if loc != screen.loc || window.geometry().loc != (0, 0).into() {
        return Err("janela fora do canto da tela".into());
    }
    // Só a superfície principal com imagem (subsuperfícies com buffer precisam da cena).
    let mut with_buffer = 0;
    with_surface_tree_downward(
        &surface,
        (),
        |_, _, _| TraversalAction::DoChildren(()),
        // Os dados de cada superfície já vêm trancados aqui: `with_renderer_surface_state`
        // trancaria de novo (e o compositor travaria).
        |_, states, _| {
            let has = states
                .data_map
                .get::<smithay::backend::renderer::utils::RendererSurfaceStateUserData>()
                .and_then(|d| d.lock().ok().map(|r| r.buffer().is_some()))
                .unwrap_or(false);
            if has {
                with_buffer += 1;
            }
        },
        |_, _, _| true,
    );
    if with_buffer != 1 {
        return Err(format!("{with_buffer} superfície(s) com imagem"));
    }
    let scale = output.current_scale().fractional_scale();
    let checked = with_renderer_surface_state(&surface, |r| -> Result<Candidate, String> {
        let buffer = r.buffer().cloned().ok_or("sem buffer")?;
        if r.buffer_transform() != Transform::Normal {
            return Err("buffer girado".into());
        }
        let logical = r.buffer_size().ok_or("buffer sem tamanho")?;
        let size = (logical.w * r.buffer_scale(), logical.h * r.buffer_scale());
        if size != mode {
            return Err(format!("buffer {}x{} e tela {}x{}", size.0, size.1, mode.0, mode.1));
        }
        // Recorte ou escala pelo `wp_viewporter`: o buffer inteiro tem de cobrir a tela.
        if let Some(view) = r.view() {
            let dst = ((view.dst.w as f64 * scale).round() as i32, (view.dst.h as f64 * scale).round() as i32);
            let src_px = (view.src.size.w * r.buffer_scale() as f64, view.src.size.h * r.buffer_scale() as f64);
            let covers = (dst.0 - mode.0).abs() <= 1 && (dst.1 - mode.1).abs() <= 1;
            let whole = view.src.loc.x.abs() < 0.5 && view.src.loc.y.abs() < 0.5 && (src_px.0 - size.0 as f64).abs() < 1.0 && (src_px.1 - size.1 as f64).abs() < 1.0;
            if !covers || !whole {
                return Err("buffer recortado ou escalado".into());
            }
        }
        if smithay::wayland::dmabuf::get_dmabuf(&buffer).is_err() {
            return Err("buffer na memória (wl_shm), não na GPU".into());
        }
        Ok(Candidate { buffer, size })
    })
    .ok_or("superfície sem estado de desenho")??;
    Ok(checked)
}

/// Lembra o último motivo registrado por tela, para o log sair uma vez por mudança.
#[derive(Debug, Default)]
pub struct Notes {
    last: std::collections::HashMap<String, String>,
}

impl Notes {
    /// Registra (nível info) se o estado do scanout da tela mudou: `None` = ativo.
    pub fn note(&mut self, output: &str, reason: Option<&str>) {
        let text = reason.unwrap_or("ativo").to_string();
        if self.last.get(output) == Some(&text) {
            return;
        }
        match reason {
            None => tracing::info!("scanout direto: ativo na tela {output} (buffer do app direto na placa)"),
            Some(r) => tracing::info!("scanout direto: não na tela {output} ({r}); quadro composto"),
        }
        self.last.insert(output.to_string(), text);
    }

    /// A tela saiu do modo de jogo: o próximo estado é registrado de novo.
    pub fn forget(&mut self, output: &str) {
        self.last.remove(output);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn motivo_registrado_uma_vez_por_mudanca() {
        let mut notes = Notes::default();
        notes.note("DP-1", Some("barra revelada"));
        assert_eq!(notes.last.get("DP-1").map(String::as_str), Some("barra revelada"));
        notes.note("DP-1", None);
        assert_eq!(notes.last.get("DP-1").map(String::as_str), Some("ativo"));
        notes.forget("DP-1");
        assert!(notes.last.is_empty());
    }
}
