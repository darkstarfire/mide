//! Cena do compositor: monta o quadro (janelas, decorações, camadas, vidro, papel de
//! parede) e avisa os clientes que podem desenhar o próximo.
//!
//! Não depende de backend: recebe o renderizador e o alvo já preparados. O backend
//! (winit hoje, DRM depois) só cuida da janela ou da tela, dos eventos e de
//! apresentar o quadro.

use std::{error::Error, time::Duration};

use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State as XdgState;
use smithay::{
    backend::renderer::{
        damage::OutputDamageTracker,
        element::{
            surface::{render_elements_from_surface_tree, WaylandSurfaceRenderElement},
            utils::RescaleRenderElement,
            Kind,
        },
        gles::{GlesRenderer, GlesTarget},
    },
    desktop::{
        layer_map_for_output, utils::send_frames_surface_tree, PopupManager,
        Window,
    },
    output::Output,
    utils::{Logical, Physical, Point, Rectangle, Scale, Transform},
};

use crate::{
    decorations::{self},
    state::State,
};

smithay::backend::renderer::element::render_elements! {
    /// Elementos desenhados por cima das janelas: conteúdo de apps, camadas e barras.
    pub MideRenderElement<=GlesRenderer>;
    Surface=WaylandSurfaceRenderElement<GlesRenderer>,
    Shader=smithay::backend::renderer::gles::element::PixelShaderElement,
    Scaled=RescaleRenderElement<WaylandSurfaceRenderElement<GlesRenderer>>,
    Title=smithay::backend::renderer::element::memory::MemoryRenderBufferRenderElement<GlesRenderer>,
    /// Texturas com shader próprio: efeito lâmpada e papel de parede.
    Textured=crate::clip::Keyed<smithay::backend::renderer::gles::element::TextureShaderElement>,
    /// Textura pronta, sem shader próprio (fundo desfocado do launcher).
    Blurred=smithay::backend::renderer::element::texture::TextureRenderElement<smithay::backend::renderer::gles::GlesTexture>,
    ClippedSurface=crate::clip::Clipped<WaylandSurfaceRenderElement<GlesRenderer>>,
    ClippedScaled=crate::clip::Clipped<RescaleRenderElement<WaylandSurfaceRenderElement<GlesRenderer>>>,
    /// Foto de uma janela (transição de fechar) com os cantos arredondados dela.
    ClippedTexture=crate::clip::Clipped<smithay::backend::renderer::element::texture::TextureRenderElement<smithay::backend::renderer::gles::GlesTexture>>,
}

/// Quanto do papel de parede aparece atrás da tela de bloqueio (sobre o preto).
const LOCKED_DIM: f32 = 0.7;

/// Cor de fundo do desktop se o papel de parede não puder ser desenhado.
const BACKGROUND: [f32; 4] = [0.12, 0.12, 0.14, 1.0];

/// Recursos de GPU da cena, criados uma vez com o renderizador do backend.
pub struct Scene {
    shaders: decorations::Shaders,
    genie: crate::genie::Genie,
    blur: crate::blur::Blur,
    /// Papel de parede de cada tela (pelo nome da saída), com a textura no tamanho dela.
    wallpapers: std::collections::HashMap<String, crate::wallpaper::Wallpaper>,
    titles: crate::title::TitleRenderer,
    /// Cursor desenhado pelo mide (sem anfitrião que mostre o dele, como no DRM).
    cursors: Option<crate::cursor::Cursors>,
    /// Desenhar o cursor na cena (o DRM desliga quando o cursor está no plano de
    /// cursor da placa).
    pub draw_cursor: bool,
    /// Ícones do aviso de volume e brilho.
    osd_icons: crate::osd::Icons,
}

impl Scene {
    /// Compila os shaders e carrega o papel de parede da configuração.
    pub fn new(renderer: &mut GlesRenderer, state: &State) -> Result<Self, Box<dyn Error>> {
        let shaders = decorations::Shaders::compile(renderer)
            .map_err(|e| format!("falha ao compilar shaders da decoração: {e}"))?;
        let genie = crate::genie::Genie::compile(renderer)
            .map_err(|e| format!("falha ao compilar o shader do efeito lâmpada: {e}"))?;
        let blur = crate::blur::Blur::compile(renderer)
            .map_err(|e| format!("falha ao compilar os shaders do desfoque: {e}"))?;
        // Compila o shader do papel de parede já aqui, para o erro aparecer na partida.
        crate::wallpaper::Wallpaper::new(renderer, None).map_err(|e| format!("falha ao compilar o shader do papel de parede: {e}"))?;
        let _ = state;
        Ok(Self { shaders, genie, blur, wallpapers: Default::default(), titles: crate::title::TitleRenderer::new(), cursors: None, draw_cursor: true, osd_icons: Default::default() })
    }

    /// A cena desenha o cursor (backends sem anfitrião).
    pub fn with_cursor(mut self) -> Self {
        self.cursors = Some(crate::cursor::Cursors::new());
        self
    }

    /// Qual cursor mostrar agora (`None`: a cena não desenha cursor).
    pub fn cursor_plan(&mut self, state: &State) -> Option<crate::cursor::Plan<'_>> {
        self.cursors.as_mut().map(|c| c.plan(state))
    }

    /// Algo na tela está em movimento (animações, papel de parede trocando, janela
    /// fechando): o backend continua pedindo quadros.
    pub fn moving(&self, state: &State) -> bool {
        state.shake.scale(std::time::Instant::now()) > 1.001
            || !state.animations.is_empty()
            || state.launcher_anim.is_some()
            || self.wallpapers.values().any(|w| w.busy())
            || !state.ghosts.is_empty()
            || !state.appearing.is_empty()
            || state.preview.as_ref().is_some_and(|p| p.appearing())
            || self.genie.glass_since.borrow().values().any(|t| t.elapsed() < crate::genie::GLASS_FADE)
            || state.osd.as_ref().is_some_and(|o| o.moving(state.config.effects.transitions))
    }

    /// Desenha um quadro no alvo `framebuffer`: prévias, popups, camadas, janelas,
    /// vidro e fundo. Quem chama apresenta o quadro (esperando o ponto de sincronização
    /// devolvido, quando a apresentação precisa) e depois chama `send_frames`.
    ///
    /// `age` é a idade do buffer (quantos quadros atrás ele foi desenhado; 0 se não se
    /// sabe): com ela, só o que mudou desde então é redesenhado. Devolve também as
    /// áreas que mudaram (`None`: nada mudou).
    pub fn render(
        &mut self,
        renderer: &mut GlesRenderer,
        framebuffer: &mut GlesTarget<'_>,
        output: &Output,
        damage_tracker: &mut OutputDamageTracker,
        state: &mut State,
        age: usize,
    ) -> Result<(smithay::backend::renderer::sync::SyncPoint, Option<Vec<Rectangle<i32, Physical>>>), Box<dyn Error>> {
        let Self { shaders, genie, blur, wallpapers, titles, cursors, draw_cursor, osd_icons } = self;
        // Cada tela tem o seu papel de parede (textura no tamanho dela).
        let output_name = output.name();
        if !wallpapers.contains_key(&output_name) {
            let created = crate::wallpaper::Wallpaper::new(renderer, state.config.desktop.wallpaper_path())
                .map_err(|e| format!("papel de parede da tela {output_name}: {e}"))?;
            wallpapers.insert(output_name.clone(), created);
        }
        let Some(wallpaper) = wallpapers.get(&output_name) else {
            return Err("papel de parede sem tela".into());
        };
        // Posição da tela no espaço de todas as telas: o que tem posição global
        // (janelas, popups, cursor) é desenhado relativo a ela.
        let origin = state.space.output_geometry(output).map(|g| g.loc).unwrap_or_default();
        // Escala da tela: posições lógicas viram pixels multiplicando por ela.
        let scale = output.current_scale().fractional_scale();
        crate::clip::set_scale(scale);
        crate::decorations::begin_frame(&output_name);
        crate::decorations::set_dark(state.config.dark);
        state.popups.cleanup();
        let now = std::time::Instant::now();
        state.finish_animations(now);
        tracing::debug!("frame: {} janela(s) mapeada(s)", state.space.elements().count());
        // Da frente para trás: prévias das janelas, popups, camadas (barra, dock) e
        // depois as janelas.
        // Captura de tela pedida para esta tela: o quadro sai sem o cursor.
        let capturing = state.screenshot_requests.iter().any(|(name, _)| *name == output_name);
        let mut custom: Vec<MideRenderElement> = match cursors {
            Some(cursors) if !capturing && *draw_cursor => cursors.elements(renderer, state, origin),
            _ => Vec::new(),
        };
        // Ícone de um arrasto entre apps, logo atrás do cursor.
        if let Some((icon, offset)) = state.dnd_icon.as_ref().filter(|_| !capturing) {
            let at = crate::clip::phys(state.pointer_location.to_i32_round::<i32>() - origin + *offset);
            custom.extend(render_elements_from_surface_tree(renderer, icon, at, crate::clip::scale(), 1.0, Kind::Unspecified));
        }
        // Tela apagada (no DRM o monitor desliga; aqui, preto).
        if state.blanked {
            state.screencopy.fail_outputs(|o| o == output);
            let result = damage_tracker.render_output(renderer, framebuffer, age, &Vec::<MideRenderElement>::new(), [0.0, 0.0, 0.0, 1.0])?;
            dump_frame_if_requested(renderer, framebuffer, output)?;
            return Ok((result.sync, result.damage.cloned()));
        }
        // Aviso de volume e brilho: na frente de tudo (menos do cursor), também com a
        // tela bloqueada. Fica fora das capturas de tela (como o cursor); nas gravações
        // e nas capturas pedidas pelos apps (screencopy) aparece, como o cursor.
        let osd = if capturing { None } else { crate::osd::elements(renderer, state, output, shaders, titles, osd_icons) };
        let mut osd_glass = None;
        if let Some((elements, panel, alpha)) = osd {
            custom.extend(elements);
            if state.config.effects.panel_glass {
                osd_glass = Some(GlassRegion { at: custom.len(), rect: panel, radius: 14.0, name: "osd".into(), alpha });
            }
        }
        // Tela bloqueada: só a tela de bloqueio desta saída (sem ela, tudo escuro).
        // Nada do que está por trás aparece, nem num quadro.
        if state.lock.locked {
            if let Some(surface) = state.lock.surface_for(output) {
                let elements: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
                    render_elements_from_surface_tree(renderer, surface.wl_surface(), Point::from((0, 0)), crate::clip::scale(), 1.0, Kind::Unspecified);
                custom.extend(elements.into_iter().map(MideRenderElement::Surface));
                // Atrás da tela de bloqueio, o papel de parede desfocado e escurecido (o
                // mesmo desfoque do launcher, na GPU): o programa de bloqueio desenha só
                // o relógio e o campo, e cada tecla custa pouco. Só depois que ela
                // aparece: antes, a tela fica escura.
                let out = Rectangle::from_size(state.space.output_geometry(output).map(|g| g.size).unwrap_or_default());
                if let Some(el) = blur.wallpaper_element(renderer, wallpaper.texture(), out, LOCKED_DIM) {
                    custom.push(MideRenderElement::Blurred(el));
                }
            }
            let result = damage_tracker.render_output(renderer, framebuffer, age, &custom, [0.0, 0.0, 0.0, 1.0])?;
            dump_frame_if_requested(renderer, framebuffer, output)?;
            let damage = result.damage.cloned();
            crate::screencopy::serve(renderer, framebuffer, output, damage.as_deref(), state);
            crate::record::feed(renderer, framebuffer, output, state);
            return Ok((result.sync, damage));
        }
        // Prévias das janelas: só na tela da dock (a posição delas vem da dock).
        if layer_map_for_output(&state.layer_output(output)).layers().any(|l| l.namespace() == "mide-dock") {
            custom.extend(crate::preview::elements(renderer, state, origin, shaders, titles));
        }
        // Encaixe pelo topo: o painel de layouts na frente de tudo, a prévia atrás da
        // barra e da dock (na frente das janelas).
        let (snap_front, snap_back) = crate::snap::elements(state, output, shaders);
        custom.extend(snap_front);
        custom.extend(popup_elements(renderer, state, origin).into_iter().map(MideRenderElement::from));
        let before_layers = custom.len();
        let (layers, mut glass) = layer_elements(renderer, output, state, blur, wallpaper);
        for region in &mut glass {
            region.at += before_layers;
        }
        custom.extend(layers);
        custom.extend(snap_back);
        let before_windows = custom.len();
        let (windows, window_glass) = window_elements(renderer, state, output, origin, shaders, genie, wallpaper, titles);
        custom.extend(windows);
        glass.extend(window_glass.into_iter().map(|mut region| {
            region.at += before_windows;
            region
        }));
        glass.extend(osd_glass);
        // Vidro (dock, barra, menus): o que está atrás de cada região (tudo depois dela
        // na lista) é desenhado numa textura, desfocado e posto logo atrás da camada.
        // De trás para a frente: o vidro de um menu vê a dock já com o vidro dela.
        // Tamanho da tela em pixels (o vidro trabalha em pixels, não em pontos lógicos).
        let out_size = output.current_mode().map(|m| m.size.to_logical(1)).unwrap_or_default();
        // Animações (lâmpada, maximizar, launcher) mudam shaders sem mudar a geometria:
        // nelas o desfoque é refeito a cada quadro.
        // O papel de parede trocando (esmaecer) também: o vidro acompanha a transição.
        let animating = !state.animations.is_empty()
            || state.launcher_anim.is_some()
            || wallpaper.busy()
            || !state.ghosts.is_empty()
            || !state.appearing.is_empty()
            || state.preview.as_ref().is_some_and(|p| p.appearing());
        let strength = if state.config.effects.liquid_glass { 1.0 } else { 0.0 };
        let mut regions = glass;
        regions.sort_by(|a, b| b.at.cmp(&a.at));
        for region in regions {
            let at = region.at;
            let rect = region.rect;
            // A região em pixels da tela.
            let px = Rectangle::<i32, Logical>::new(
                (crate::clip::phys(rect.loc).x, crate::clip::phys(rect.loc).y).into(),
                ((rect.size.w as f64 * scale).round() as i32, (rect.size.h as f64 * scale).round() as i32).into(),
            );
            let src = blur_source(px, out_size);
            // A geração do papel de parede entra na assinatura: trocar a imagem refaz o vidro.
            let key = if animating {
                state.start_time.elapsed().as_nanos() as u64 ^ 0x9e37_79b9
            } else {
                backdrop_key(&custom[at..], px) ^ wallpaper.generation().wrapping_mul(0x9e37_79b9_7f4a_7c15)
            };
            let behind = &custom[at..];
            // O cache do desfoque é por tela (a mesma região pode aparecer em duas).
            let cache_name = format!("{output_name}:{}", region.name);
            let blurred = blur.region_blur(renderer, &cache_name, key, out_size, src, strength, |renderer, target| {
                crate::blur::render_region(renderer, behind, px, out_size, target)
            });
            match blurred {
                Some((blurred, size, id)) => {
                    let element = blur.composite_element(renderer, id, blurred, size, rect, region.radius, strength, region.alpha);
                    custom.insert(at, MideRenderElement::Textured(element));
                }
                None => tracing::trace!("vidro {}: área pequena ou falha ao desfocar", region.name),
            }
        }
        if std::env::var_os("MIDE_DAMAGE_DEBUG").is_some() {
            use smithay::backend::renderer::element::Element as _;
            for (i, e) in custom.iter().enumerate() {
                let g = e.geometry(Scale::from(scale));
                tracing::debug!("elemento {i}: {:?} commit {:?} em {}x{}+{}+{}", e.id(), e.current_commit(), g.size.w, g.size.h, g.loc.x, g.loc.y);
            }
        }
        // Direto no rastreador de dano: o `space::render_output` do Smithay acrescenta
        // sozinho as camadas da tela (barra, dock), que o mide já desenha aqui (e que
        // apareceriam atrás da tela de bloqueio).
        let result = damage_tracker.render_output(renderer, framebuffer, age, &custom, BACKGROUND)?;
        tracing::debug!("quadro do compositor: {:.2} ms", now.elapsed().as_secs_f32() * 1000.0);
        if let Some(mode) = output.current_mode() {
            let area: i64 = result.damage.map(|d| d.iter().map(|r| r.size.w as i64 * r.size.h as i64).sum()).unwrap_or(0);
            tracing::debug!("dano: {}% da tela ({} retângulo(s))", area * 100 / (mode.size.w as i64 * mode.size.h as i64).max(1), result.damage.map_or(0, |d| d.len()));
        }
        // Medição (`MIDE_GPU_TIMING=1`, tests/perf.sh): espera a GPU terminar o quadro
        // para saber o custo real do desenho (o tempo acima é só o da CPU).
        if std::env::var_os("MIDE_GPU_TIMING").is_some() {
            let _ = renderer.with_context(|gl| unsafe { gl.Finish() });
            tracing::debug!("quadro na GPU: {:.2} ms", now.elapsed().as_secs_f32() * 1000.0);
        }
        if capturing {
            let (mine, others): (Vec<_>, Vec<_>) = std::mem::take(&mut state.screenshot_requests).into_iter().partition(|(name, _)| *name == output_name);
            state.screenshot_requests = others;
            for (_, path) in mine {
                if let Err(err) = save_screenshot(renderer, framebuffer, output, path) {
                    tracing::warn!("captura de tela: {err}");
                }
            }
        }
        dump_frame_if_requested(renderer, framebuffer, output)?;
        // Capturas pedidas pelos apps (OBS e outros): saem deste quadro.
        let damage = result.damage.cloned();
        crate::screencopy::serve(renderer, framebuffer, output, damage.as_deref(), state);
        crate::record::feed(renderer, framebuffer, output, state);
        Ok((result.sync, damage))
    }
}

/// Elementos de render dos popups de todas as janelas, para ficarem acima delas.
fn popup_elements(
    renderer: &mut GlesRenderer,
    state: &State,
    origin: Point<i32, Logical>,
) -> Vec<WaylandSurfaceRenderElement<GlesRenderer>> {
    let mut elements = Vec::new();
    for window in state.space.elements() {
        let (Some(toplevel), Some(loc)) = (window.toplevel(), state.space.element_location(window))
        else {
            continue;
        };
        for (popup, popup_offset) in PopupManager::popups_for_surface(toplevel.wl_surface()) {
            let offset = popup_offset - popup.geometry().loc;
            // Menu que acabou de abrir: entra esmaecendo e descendo um pouco.
            let (alpha, dy) = state.appear(popup.wl_surface());
            let location: Point<i32, Physical> = crate::clip::phys(loc + offset - origin + Point::from((0, dy)));
            elements.extend(render_elements_from_surface_tree(
                renderer,
                popup.wl_surface(),
                location,
                crate::clip::scale(),
                alpha,
                Kind::Unspecified,
            ));
        }
    }
    elements
}

/// Elementos de todas as janelas, da frente para trás: contorno, máscaras dos
/// cantos, barra, conteúdo do app e sombra. Durante a animação de maximizar, o
/// conteúdo e a moldura seguem a interpolação entre o estado antigo e o novo.
#[allow(clippy::too_many_arguments)]
fn window_elements(
    renderer: &mut GlesRenderer,
    state: &State,
    output: &Output,
    origin: Point<i32, Logical>,
    shaders: &decorations::Shaders,
    genie: &crate::genie::Genie,
    wallpaper: &crate::wallpaper::Wallpaper,
    titles: &mut crate::title::TitleRenderer,
) -> (Vec<MideRenderElement>, Vec<GlassRegion>) {
    use smithay::backend::renderer::element::AsRenderElements;
    let mut glass = Vec::new();

    let now = std::time::Instant::now();

    // Primeira passada, de cima para baixo: posição, zoom e moldura de cada janela.
    struct Placed {
        window: Window,
        /// Origem da superfície: posição da janela menos o deslocamento da geometria,
        /// a mesma conta do `render_location` do Smithay, usada também no clique.
        surface_origin: Point<i32, Logical>,
        zoom: f64,
        alpha: f32,
        /// Transparência da janela inteira (`[opacity]`): conteúdo, barra e contorno.
        opacity: f32,
        frame: Rectangle<i32, Logical>,
        focused: bool,
        maximized: bool,
        animating: bool,
        /// Efeito lâmpada em andamento: área da janela, alvo e progresso (0 a 1).
        genie: Option<(Rectangle<i32, Logical>, Rectangle<i32, Logical>, f32)>,
    }
    let mut placed: Vec<Placed> = Vec::new();
    let keyboard_focus = state.seat.get_keyboard().and_then(|k| k.current_focus());
    // `elements()` vai do fundo para o topo; aqui percorremos do topo para o fundo.
    for window in state.space.elements().rev() {
        let (Some(toplevel), Some(loc)) = (window.toplevel(), state.space.element_location(window))
        else {
            continue;
        };
        // Mesa à mostra com várias telas: a janela afastada só aparece na tela dela.
        if state.reveal_home(window).is_some_and(|home| home != output.name()) {
            continue;
        }
        let state_flags = toplevel.current_state().states;
        // Foco pelo que o compositor decidiu (estado pendente), não pelo que o app já
        // confirmou: apps parados (VS Code, Electron) demoram a responder ao configure
        // e a barra ficava cinza com a janela em foco. A decoração é do compositor.
        let focused = toplevel.with_pending_state(|s| s.states.contains(XdgState::Activated));
        let maximized = state_flags.contains(XdgState::Maximized);
        let geo = window.geometry();

        // Posição, tamanho e zoom do conteúdo, interpolados se houver animação.
        let (draw_loc, zoom, frame_w, frame_h, alpha) = match state.animation_for(toplevel.wl_surface()) {
            Some(anim) => {
                let e = anim.eased(now);
                let lerp = |a: f32, b: f32| a + (b - a) * e;
                let loc = (
                    lerp(anim.from_loc.x as f32, anim.to_loc.x as f32).round() as i32,
                    lerp(anim.from_loc.y as f32, anim.to_loc.y as f32).round() as i32,
                );
                let zoom = lerp(anim.zoom_from, anim.zoom_to) as f64;
                let w = lerp(anim.from_size.w as f32, anim.to_size.w as f32).round() as i32;
                let h = lerp(anim.from_size.h as f32, anim.to_size.h as f32).round() as i32;
                (loc.into(), zoom, w, h, lerp(anim.alpha_from, anim.alpha_to))
            }
            None => (loc, 1.0, geo.size.w, geo.size.h, 1.0),
        };
        // A área visível começa no deslocamento da geometria (sombra do cliente, p. ex.).
        // No Space, a posição da janela já é o canto da área visível. A moldura e a
        // barra (se a janela usa decoração de servidor) se alinham a ela.
        let visible = draw_loc;
        let bar_h = crate::state::bar_height_for(window);
        // Daqui em diante, tudo relativo à tela que está sendo desenhada.
        let visible = visible - origin;
        let draw_loc = draw_loc - origin;
        let frame = Rectangle::new((visible.x, visible.y - bar_h).into(), (frame_w, bar_h + frame_h).into());
        let genie_info = state.animation_for(toplevel.wl_surface()).filter(|a| a.genie).map(|a| {
            let e = a.eased(now);
            let from = Rectangle::new(a.from_loc - origin, a.from_size);
            let to = Rectangle::new(a.to_loc - origin, a.to_size);
            // Minimizar: da janela ao alvo. Restaurar: do alvo à janela (progresso invertido).
            if a.hide_at_end { (from, to, e) } else { (to, from, 1.0 - e) }
        });
        placed.push(Placed {
            window: window.clone(),
            genie: genie_info,
            surface_origin: draw_loc - geo.loc,
            zoom,
            frame,
            focused,
            maximized,
            animating: state.animation_for(toplevel.wl_surface()).is_some(),
        alpha,
            // Sem foco pelo teclado de verdade (o "ativo" do xdg só muda quando o app
            // confirma, e alguns só confirmam quando redesenham).
            opacity: state.config.opacity.for_window(&app_id_of(window), keyboard_focus.as_ref() == Some(toplevel.wl_surface())),
        });
    }

    // As sombras das janelas são desenhadas logo atrás de cada uma (caem também sobre
    // as janelas de baixo); o fundo não recebe mais sombras por conta própria.
    let casters: Vec<decorations::Caster> = Vec::new();

    // A tela inteira, nas coordenadas dela (origem no canto).
    let output_rect = Rectangle::from_size(
        state.space.output_geometry(output).map(|g| g.size).unwrap_or_else(|| (800, 600).into()),
    );
    let output_height = output_rect.size.h as f32;

    // Segunda passada, de cima para baixo: cada janela desenha sobre as de baixo.
    // Sombra e máscaras recebem as molduras das janelas de baixo para não cobri-las.
    let mut elements = Vec::new();

    // Janelas que acabaram de fechar: a foto delas encolhe um pouco e esmaece, por
    // cima das outras, com os mesmos cantos arredondados da janela.
    {
        let snapshots = genie.snapshots.borrow();
        for (id, frame, start, ghost_radius) in &state.ghosts {
            let Some((texture, size)) = snapshots.get(id) else {
                continue;
            };
            if start.elapsed() < std::time::Duration::from_millis(20) {
                tracing::debug!("foto da janela fechada esmaecendo");
            }
            let t = (start.elapsed().as_secs_f32() / crate::state::CLOSE_FADE.as_secs_f32()).clamp(0.0, 1.0);
            let ease = t * t * (3.0 - 2.0 * t);
            let scale = 1.0 - 0.06 * ease;
            // O tamanho vem da foto: na hora em que a janela some, a geometria dela já
            // está zerada.
            let (w, h) = ((size.w as f32 * scale).round() as i32, (size.h as f32 * scale).round() as i32);
            let x = frame.loc.x - origin.x + (size.w - w) / 2;
            let y = frame.loc.y - origin.y + (size.h - h) / 2;
            use smithay::backend::renderer::Renderer as _;
            let radius = if state.config.effects.rounded_corners { *ghost_radius * scale } else { 0.0 };
            let clip = crate::clip::uniforms(Rectangle::new((x, y).into(), (w, h).into()), radius, output_rect.size.h as f32);
            let photo = smithay::backend::renderer::element::texture::TextureRenderElement::from_static_texture(
                smithay::backend::renderer::element::Id::new(),
                renderer.context_id(),
                crate::clip::phys((x, y).into()).to_f64(),
                texture.clone(),
                1,
                Transform::Normal,
                Some(1.0 - ease),
                Some(Rectangle::from_size((size.w as f64, size.h as f64).into())),
                Some((w, h).into()),
                None,
                Kind::Unspecified,
            );
            elements.push(MideRenderElement::ClippedTexture(crate::clip::Clipped::new(photo, shaders.clip_program(), clip)));
        }
    }
    // Fotos que não servem mais: a janela fechou e a foto já esmaeceu.
    genie.snapshots.borrow_mut().retain(|id, _| {
        state.close_requests.iter().any(|(w, _)| w == id)
            || state.ghosts.iter().any(|(w, _, _, _)| w == id)
            || state.commits.contains_key(id)
    });
    genie.photo_taken.borrow_mut().retain(|id, _| state.commits.contains_key(id));
    for p in placed.iter() {
        // A barra fica no topo da moldura, que já está na área visível.
        let bar_h = crate::state::bar_height_for(&p.window);
        let bar = Rectangle::new(p.frame.loc, (p.frame.size.w, bar_h).into());
        // Com a janela quase invisível (minimizando ou restaurando), a moldura não aparece.
        let decorated = (!p.maximized || p.animating) && p.alpha > 0.99 && p.genie.is_none();
        let phys: Point<i32, smithay::utils::Physical> = crate::clip::phys(p.surface_origin);
        // Moldura, barra e conteúdo com a opacidade da janela (`[opacity]`).
        decorations::set_alpha(p.opacity);

        // Janela maximizada ocupa a tela inteira: sem cantos, contorno nem sombra.
        // Cada efeito pode ser desligado em `[effects]` no config.toml.
        let effects = state.config.effects;
        if decorated && effects.rounded_corners {
            elements.push(MideRenderElement::from(decorations::outline_element(
                shaders, p.frame, p.focused, corner_radius(state, &p.window),
            )));
        }
        // Recorte arredondado pela moldura: vale para a barra e para o conteúdo, e os
        // cantos mostram o que está de fato atrás (desktop com sombra ou outra janela).
        let radius = if decorated && effects.rounded_corners {
            corner_radius(state, &p.window)
        } else {
            0.0
        };
        let clip_uniforms = crate::clip::uniforms(p.frame, radius, output_height);

        // Barra e título só existem nas janelas com decoração de servidor. Vêm antes do
        // conteúdo na lista, para ficarem na frente dele.
        if bar_h > 0 && p.genie.is_none() {
            let title = toplevel_title_of(&p.window);
            if let Some(text) = crate::title::element(renderer, titles, &title, p.focused, bar, p.opacity) {
                elements.push(MideRenderElement::Title(text));
            }
            elements.extend(
                decorations::bar_elements(
                    shaders,
                    bar,
                    p.focused,
                    decorations::button_at(bar, state.pointer_location - origin.to_f64()),
                    &clip_uniforms,
                )
                .into_iter()
                .map(MideRenderElement::from),
            );
        }

        // Efeito lâmpada: a janela inteira (barra, título e conteúdo) deformada
        // substitui o desenho normal durante a animação.
        if let Some((content_rect, target, progress)) = p.genie {
            // Na lâmpada a janela é uma foto: o vidro volta esmaecendo quando ela termina.
            genie.glass_since.borrow_mut().remove(&crate::state::window_id(&p.window));
            let bar_h = crate::state::bar_height_for(&p.window);
            let frame_rect = Rectangle::new(
                (content_rect.loc.x, content_rect.loc.y - bar_h).into(),
                (content_rect.size.w, content_rect.size.h + bar_h).into(),
            );
            // Barra e título em coordenadas da textura (origem no canto da moldura).
            let local_bar = Rectangle::new((0, 0).into(), (content_rect.size.w, bar_h).into());
            let no_clip = crate::clip::uniforms(Rectangle::default(), 0.0, 0.0);
            let bar_parts = if bar_h > 0 {
                decorations::bar_elements(shaders, local_bar, p.focused, None, &no_clip)
            } else {
                Vec::new()
            };
            let title = if bar_h > 0 {
                crate::title::element(renderer, titles, &toplevel_title_of(&p.window), p.focused, local_bar, 1.0)
            } else {
                None
            };
            let mut overlays: Vec<&dyn smithay::backend::renderer::element::RenderElement<GlesRenderer>> =
                Vec::new();
            if let Some(t) = title.as_ref() {
                overlays.push(t);
            }
            for part in &bar_parts {
                overlays.push(part);
            }
            let corner = if state.config.effects.rounded_corners {
                corner_radius(state, &p.window)
            } else {
                0.0
            };
            if let Some(element) = genie.element(
                renderer,
                &p.window,
                frame_rect,
                bar_h,
                &overlays,
                corner,
                target,
                progress,
                1.0,
            ) {
                elements.push(MideRenderElement::Textured(crate::clip::Keyed::new(smithay::backend::renderer::element::Id::new(), element)));
            }
            continue;
        }

        // Fechamento pedido: fotografa a janela (barra, título e conteúdo) para a
        // transição, até ela fechar.
        let id = crate::state::window_id(&p.window);
        // Foto para fechar: a cada quadro com o fechamento pedido; e depois de um commit
        // do app (no máximo a cada 300 ms, se ele muda sem parar), para quando o app
        // fecha sozinho.
        let changed = state.commits.get(&id).is_some_and(|at| genie.photo_taken.borrow().get(&id).is_none_or(|taken| at > taken && taken.elapsed() >= std::time::Duration::from_millis(300)));
        if state.config.effects.animations && (changed || state.close_requests.iter().any(|(w, _)| *w == id)) {
            genie.photo_taken.borrow_mut().insert(id, std::time::Instant::now());
            let local_bar = Rectangle::new((0, 0).into(), (p.frame.size.w, bar_h).into());
            let no_clip = crate::clip::uniforms(Rectangle::default(), 0.0, 0.0);
            let bar_parts = if bar_h > 0 { decorations::bar_elements(shaders, local_bar, p.focused, None, &no_clip) } else { Vec::new() };
            let title = if bar_h > 0 { crate::title::element(renderer, titles, &toplevel_title_of(&p.window), p.focused, local_bar, 1.0) } else { None };
            let mut overlays: Vec<&dyn smithay::backend::renderer::element::RenderElement<GlesRenderer>> = Vec::new();
            if let Some(t) = title.as_ref() {
                overlays.push(t);
            }
            for part in &bar_parts {
                overlays.push(part);
            }
            if let Some(texture) = crate::genie::snapshot(renderer, &p.window, p.frame.size, bar_h, &overlays) {
                genie.snapshots.borrow_mut().insert(id, (texture, p.frame.size));
            }
        }

        // Conteúdo do app, com zoom durante a animação.
        let content: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
            p.window.render_elements(renderer, phys, Scale::from(crate::clip::scale()), p.alpha * p.opacity);
        let program = shaders.clip_program();
        for element in content {
            if (p.zoom - 1.0).abs() > f64::EPSILON {
                let scaled = RescaleRenderElement::from_element(element, phys, p.zoom);
                elements.push(MideRenderElement::ClippedScaled(crate::clip::Clipped::new(
                    scaled,
                    program.clone(),
                    clip_uniforms.clone(),
                )));
            } else {
                elements.push(MideRenderElement::ClippedSurface(crate::clip::Clipped::new(
                    element,
                    program.clone(),
                    clip_uniforms.clone(),
                )));
            }
        }

        // Vidro pedido pela janela (a barra lateral do Arquivos), logo atrás do
        // conteúdo dela. Cobre também a faixa da barra de título acima e avança um raio
        // para dentro do conteúdo: os cantos arredondados do vidro coincidem com os da
        // janela ou ficam escondidos atrás das partes opacas.
        // Vidro da janela: entra esmaecendo depois de uma animação (lâmpada, zoom), em
        // vez de aparecer de uma vez quando ela termina.
        let id = crate::state::window_id(&p.window);
        let glass_ok = effects.panel_glass && p.alpha > 0.99 && (p.zoom - 1.0).abs() < f64::EPSILON;
        let glass_alpha = if glass_ok {
            let since = *genie.glass_since.borrow_mut().entry(id).or_insert(now);
            if effects.animations {
                let t = (since.elapsed().as_secs_f32() / crate::genie::GLASS_FADE.as_secs_f32()).clamp(0.0, 1.0);
                t * t * (3.0 - 2.0 * t)
            } else {
                1.0
            }
        } else {
            genie.glass_since.borrow_mut().remove(&id);
            0.0
        };
        if glass_ok {
            let key = glass_key(&p.window);
            let app_id = key.trim_start_matches("app:");
            let content_top = p.surface_origin.y + p.window.geometry().loc.y;
            for (i, (rect, _, _)) in state.glass.get(&key).into_iter().flatten().enumerate() {
                let mut r = Rectangle::new(p.surface_origin + rect.loc, rect.size);
                let pad = radius.ceil() as i32;
                if r.loc.y <= content_top {
                    r.size.h += r.loc.y - p.frame.loc.y;
                    r.loc.y = p.frame.loc.y;
                } else {
                    r.loc.y -= pad;
                    r.size.h += pad;
                }
                if r.loc.x > p.frame.loc.x {
                    r.loc.x -= pad;
                    r.size.w += pad;
                }
                r.size.w += pad;
                r.size.h += pad;
                if let Some(r) = r.intersection(p.frame) {
                    glass.push(GlassRegion { at: elements.len(), rect: r, radius, name: format!("app:{app_id}:{id}#{i}"), alpha: glass_alpha });
                }
            }
        }
        // Sombra logo atrás da janela: cai sobre o fundo e sobre as janelas de baixo.
        if decorated && effects.shadows && p.alpha > 0.99 {
            let strength = decorations::shadow_strength(p.focused) * p.opacity;
            elements.push(MideRenderElement::from(decorations::window_shadow_element(shaders, p.frame, radius.max(1.0), strength)));
        }
    }

    // Camadas de fundo (ícones da área de trabalho): atrás das janelas e na frente
    // do papel de parede.
    {
        let source = state.layer_output(output);
        let map = layer_map_for_output(&source);
        for layer in crate::state::back_layers(&map) {
            let Some(geo) = map.layer_geometry(&layer) else {
                continue;
            };
            let location: Point<i32, Physical> = crate::clip::phys(geo.loc);
            let surface: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
                render_elements_from_surface_tree(renderer, layer.wl_surface(), location, crate::clip::scale(), 1.0, Kind::Unspecified);
            elements.extend(surface.into_iter().map(MideRenderElement::from));
        }
    }

    // Daqui em diante, opaco (a opacidade era da última janela desenhada).
    decorations::set_alpha(1.0);

    // Fundo do desktop (papel de parede) com as sombras de todas as janelas, atrás
    // de tudo (fim da lista). Se a textura falhar, usa a cor lisa.
    // A configuração pode ter mudado (comando `reload`): segue o papel de parede dela.
    wallpaper.follow(state.config.desktop.wallpaper_path());
    let layers = wallpaper.element(renderer, output_rect, &casters);
    if layers.is_empty() {
        elements.push(MideRenderElement::from(decorations::desktop_element(shaders, output_rect, BACKGROUND, &casters)));
    }
    elements.extend(layers.into_iter().map(MideRenderElement::Textured));
    (elements, glass)
}

/// Superfícies de camada (barra, dock, buscador) que ficam acima das janelas.
/// As camadas de fundo ainda não são desenhadas: ficam para quando o desktop tiver papel de parede.
fn layer_elements(
    renderer: &mut GlesRenderer,
    output: &Output,
    state: &State,
    blur: &crate::blur::Blur,
    wallpaper: &crate::wallpaper::Wallpaper,
) -> (Vec<MideRenderElement>, Vec<GlassRegion>) {
    let source = state.layer_output(output);
    let map = layer_map_for_output(&source);
    let mut elements: Vec<MideRenderElement> = Vec::new();
    let mut glass = Vec::new();
    // Do topo para o fundo, como a lista de render.
    for layer in crate::state::front_layers(&map).iter() {
        let Some(geo) = map.layer_geometry(layer) else {
            continue;
        };
        // Menus da barra e da dock que acabaram de abrir: entram esmaecendo.
        let (appear, dy) = state.appear(layer.wl_surface());
        let location: Point<i32, Physical> = crate::clip::phys(geo.loc + Point::from((0, dy)));
        // O launcher abre e fecha com esmaecer e zoom feitos aqui (GPU).
        let (alpha, zoom) = if layer.namespace() == "mide-launcher" { state.launcher_look() } else { (appear, 1.0) };
        // Popups da camada (menus de painéis de terceiros), na frente dela.
        for (popup, offset) in PopupManager::popups_for_surface(layer.wl_surface()) {
            let at = crate::clip::phys(geo.loc + offset - popup.geometry().loc);
            let popup_elements: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
                render_elements_from_surface_tree(renderer, popup.wl_surface(), at, crate::clip::scale(), alpha, Kind::Unspecified);
            elements.extend(popup_elements.into_iter().map(MideRenderElement::from));
        }
        let surface: Vec<WaylandSurfaceRenderElement<GlesRenderer>> = render_elements_from_surface_tree(
            renderer,
            layer.wl_surface(),
            location,
            crate::clip::scale(),
            alpha,
            Kind::Unspecified,
        );
        if (zoom - 1.0).abs() > f64::EPSILON {
            let center: Point<i32, Physical> = crate::clip::phys((geo.loc.x + geo.size.w / 2, geo.loc.y + geo.size.h / 2).into());
            elements.extend(surface.into_iter().map(|e| MideRenderElement::Scaled(RescaleRenderElement::from_element(e, center, zoom))));
        } else {
            elements.extend(surface.into_iter().map(MideRenderElement::from));
        }
        match layer.namespace() {
            // Atrás do launcher, o papel de parede desfocado (como o Launchpad).
            "mide-launcher" => {
                // Pastas (ícones e o painel aberto) em vidro líquido, como no macOS: o
                // vidro vê o papel de parede desfocado e acompanha o zoom e o esmaecer.
                if state.config.effects.liquid_glass {
                    let center = (geo.loc.x as f64 + geo.size.w as f64 / 2.0, geo.loc.y as f64 + geo.size.h as f64 / 2.0);
                    for (i, (rect, radius, region_alpha)) in state.glass.get("mide-launcher").into_iter().flatten().enumerate() {
                        let x = center.0 + (geo.loc.x as f64 + rect.loc.x as f64 - center.0) * zoom;
                        let y = center.1 + (geo.loc.y as f64 + rect.loc.y as f64 - center.1) * zoom;
                        let r = Rectangle::new(
                            (x.round() as i32, y.round() as i32).into(),
                            ((rect.size.w as f64 * zoom).round() as i32, (rect.size.h as f64 * zoom).round() as i32).into(),
                        );
                        if let Some(r) = r.intersection(geo) {
                            glass.push(GlassRegion { at: elements.len(), rect: r, radius: *radius * zoom as f32, name: format!("mide-launcher#{i}"), alpha: alpha * region_alpha });
                        }
                    }
                }
                if !state.config.effects.launcher_blur {
                    continue;
                }
                let out = Rectangle::new((0, 0).into(), geo.size);
                if let Some(el) = blur.wallpaper_element(renderer, wallpaper.texture(), out, state.launcher_look().0) {
                    elements.push(MideRenderElement::Blurred(el));
                }
            }
            "mide-dock" if state.config.effects.dock_blur => {
                // Só a parte na tela (a dock que se esconde desliza para fora); a faixa
                // escondida, pequena demais, fica sem vidro.
                let screen = output.current_mode().map(|m| Rectangle::from_size(m.size.to_logical(1))).unwrap_or(geo);
                if let Some(rect) = geo.intersection(screen) {
                    glass.push(GlassRegion { at: elements.len(), rect, radius: 15.0, name: "mide-dock".into(), alpha: 1.0 });
                }
            }
            namespace if state.config.effects.panel_glass => {
                // Regiões pedidas pela camada (barra, menus, notificações), relativas a ela.
                for (i, (rect, radius, region_alpha)) in state.glass.get(namespace).into_iter().flatten().enumerate() {
                    let on_screen = Rectangle::new(geo.loc + rect.loc, rect.size);
                    if let Some(rect) = on_screen.intersection(geo) {
                        glass.push(GlassRegion { at: elements.len(), rect, radius: *radius, name: format!("{namespace}#{i}"), alpha: appear * region_alpha });
                    }
                }
            }
            _ => {}
        }
    }
    (elements, glass)
}

/// Vidro atrás de uma camada ou janela: posição na lista de render (logo depois dos
/// elementos dela, ou seja, atrás dela), retângulo na tela, raio dos cantos e nome (para
/// o cache do desfoque).
struct GlassRegion {
    at: usize,
    rect: Rectangle<i32, Logical>,
    radius: f32,
    name: String,
    /// Opacidade (o menu que entra esmaecendo leva o vidro junto).
    alpha: f32,
}

/// Região da dock dentro da textura da cena, nas coordenadas da textura. A cena é
/// desenhada com a mesma orientação da tela (invertida no eixo y no OpenGL).
/// Assinatura do que está atrás da dock: muda quando algo ali muda (posição, tamanho,
/// opacidade, conteúdo novo de uma janela). Elementos de shader são recriados a cada
/// quadro com id novo, então deles entra só a geometria e a opacidade (por isso, com
/// uma animação em curso, quem chama não usa o cache).
fn backdrop_key(elements: &[MideRenderElement], area: Rectangle<i32, Logical>) -> u64 {
    use smithay::backend::renderer::element::Element;
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (area.loc.x, area.loc.y, area.size.w, area.size.h).hash(&mut h);
    let region: Rectangle<i32, smithay::utils::Physical> = Rectangle::new((area.loc.x, area.loc.y).into(), (area.size.w, area.size.h).into());
    for element in elements {
        let g = element.geometry(smithay::utils::Scale::from(crate::clip::scale()));
        if !g.overlaps(region) {
            continue;
        }
        (g.loc.x, g.loc.y, g.size.w, g.size.h, element.alpha().to_bits()).hash(&mut h);
        match element {
            MideRenderElement::Shader(_) | MideRenderElement::Textured(_) | MideRenderElement::Blurred(_) => 0u8.hash(&mut h),
            other => {
                other.id().hash(&mut h);
                format!("{:?}", other.current_commit()).hash(&mut h);
            }
        }
    }
    h.finish()
}

fn blur_source(area: Rectangle<i32, Logical>, out: smithay::utils::Size<i32, Logical>) -> Rectangle<f64, Logical> {
    let flip = std::env::var("MIDE_BLUR_FLIP").map(|v| v != "0").unwrap_or(crate::clip::flipped());
    let y = if flip { out.h - area.loc.y - area.size.h } else { area.loc.y };
    Rectangle::new((area.loc.x as f64, y as f64).into(), (area.size.w as f64, area.size.h as f64).into())
}

/// Salva o quadro em PNG (numa thread: codificar leva alguns ms). Grava com outro nome
/// e renomeia, para quem espera o arquivo só o ver pronto.
fn save_screenshot(
    renderer: &mut GlesRenderer,
    framebuffer: &smithay::backend::renderer::gles::GlesTarget<'_>,
    output: &Output,
    path: std::path::PathBuf,
) -> Result<(), Box<dyn Error>> {
    use smithay::backend::renderer::ExportMem;
    let mode = output.current_mode().ok_or("tela sem modo")?;
    let (w, h) = (mode.size.w, mode.size.h);
    let mapping = renderer.copy_framebuffer(framebuffer, Rectangle::from_size((w, h).into()), smithay::backend::allocator::Fourcc::Abgr8888)?;
    let bytes = renderer.map_texture(&mapping)?.to_vec();
    let flipped = crate::clip::flipped();
    std::thread::spawn(move || {
        let row = w as usize * 4;
        // No winit a imagem vem de baixo para cima.
        let mut pixels: Vec<u8> = if flipped { bytes.chunks(row).rev().flatten().copied().collect() } else { bytes };
        // Opaca: o fundo do quadro não tem transparência de verdade.
        for px in pixels.chunks_mut(4) {
            px[3] = 255;
        }
        let Some(image) = image::RgbaImage::from_raw(w as u32, h as u32, pixels) else {
            return;
        };
        let tmp = path.with_extension("png.parcial");
        match image.save_with_format(&tmp, image::ImageFormat::Png).and_then(|_| std::fs::rename(&tmp, &path).map_err(Into::into)) {
            Ok(()) => tracing::info!("captura de tela salva: {}", path.display()),
            Err(err) => tracing::warn!("captura de tela não salva: {err}"),
        }
    });
    Ok(())
}

/// Depuração: com `MIDE_DUMP_FRAME=/caminho`, salva o frame `MIDE_DUMP_AT` (padrão 120)
/// como bytes RGBA crus, com a largura e altura no nome do arquivo.
fn dump_frame_if_requested(
    renderer: &mut GlesRenderer,
    framebuffer: &smithay::backend::renderer::gles::GlesTarget<'_>,
    output: &Output,
) -> Result<(), Box<dyn Error>> {
    use smithay::backend::renderer::ExportMem;
    use std::sync::atomic::{AtomicU32, Ordering};
    static FRAMES: AtomicU32 = AtomicU32::new(0);

    let Ok(path) = std::env::var("MIDE_DUMP_FRAME") else {
        return Ok(());
    };
    // Com várias telas, só uma é salva: a de `MIDE_DUMP_OUTPUT`, ou a primeira desenhada.
    static FIRST: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let wanted = std::env::var("MIDE_DUMP_OUTPUT").unwrap_or_else(|_| FIRST.get_or_init(|| output.name()).clone());
    if output.name() != wanted {
        return Ok(());
    }
    let at: u32 = std::env::var("MIDE_DUMP_AT").ok().and_then(|v| v.parse().ok()).unwrap_or(120);
    if FRAMES.fetch_add(1, Ordering::Relaxed) != at {
        return Ok(());
    }
    let Some(mode) = output.current_mode() else {
        return Ok(());
    };
    let (w, h) = (mode.size.w, mode.size.h);
    let region = Rectangle::from_size((w, h).into());
    let mapping = renderer.copy_framebuffer(framebuffer, region, smithay::backend::allocator::Fourcc::Abgr8888)?;
    let bytes = renderer.map_texture(&mapping)?;
    // O arquivo sai sempre na ordem do OpenGL do winit (de baixo para cima), que os
    // roteiros de teste desviram; alvos sem inversão (DRM, headless) são virados aqui.
    let bytes = if crate::clip::flipped() {
        bytes.to_vec()
    } else {
        let row = w as usize * 4;
        bytes.chunks(row).rev().flatten().copied().collect()
    };
    std::fs::write(format!("{path}.{w}x{h}.rgba"), bytes)?;
    tracing::info!("frame salvo para depuração ({w}x{h})");
    Ok(())
}

/// Avisa os clientes (janelas, camadas e popups) que podem desenhar o próximo quadro.
pub fn send_frames(state: &mut State, output: &Output) -> Result<(), Box<dyn Error>> {
    let elapsed = state.start_time.elapsed();
    crate::activation::update_scales(state, output);
    for window in state.space.elements() {
        window.send_frame(output, elapsed, Some(Duration::ZERO), |_, _| Some(output.clone()));
    }
    // Camadas e popups também precisam do frame callback para redesenhar depois de
    // receber teclas ou cliques.
    for layer in layer_map_for_output(output).layers() {
        layer.send_frame(output, elapsed, Some(Duration::ZERO), |_, _| Some(output.clone()));
    }
    for window in state.space.elements() {
        let Some(toplevel) = window.toplevel() else {
            continue;
        };
        for (popup, _) in PopupManager::popups_for_surface(toplevel.wl_surface()) {
            send_frames_surface_tree(
                popup.wl_surface(),
                output,
                elapsed,
                Some(Duration::ZERO),
                |_, _| Some(output.clone()),
            );
        }
    }
    // Tela de bloqueio desta saída.
    if let Some(surface) = state.lock.surface_for(output) {
        send_frames_surface_tree(surface.wl_surface(), output, elapsed, Some(Duration::ZERO), |_, _| Some(output.clone()));
    }
    if let Some((icon, _)) = &state.dnd_icon {
        send_frames_surface_tree(icon, output, elapsed, Some(Duration::ZERO), |_, _| Some(output.clone()));
    }
    // Cursor pedido por um app (desenhado pelo mide no DRM): também pode ser animado.
    if let smithay::input::pointer::CursorImageStatus::Surface(surface) = &state.cursor_status {
        send_frames_surface_tree(surface, output, elapsed, Some(Duration::ZERO), |_, _| Some(output.clone()));
    }
    state.space.refresh();
    state.display_handle.flush_clients()?;
    Ok(())
}

/// Raio dos cantos de uma janela: o pedido pelo app (`window_style app:<id> radius=N`, os
/// apps de conteúdo inteiro do mide-ui) ou o padrão.
pub fn corner_radius(state: &State, window: &Window) -> f32 {
    radius_for(&state.window_radius, &app_id_of(window))
}

/// O raio pedido por `app_id` ou o padrão: o mesmo para a janela viva e para a foto dela ao fechar.
pub fn radius_for(requested: &std::collections::HashMap<String, i32>, app_id: &str) -> f32 {
    requested.get(app_id).copied().unwrap_or(decorations::CORNER_RADIUS) as f32
}

#[cfg(test)]
mod radius_tests {
    use super::radius_for;
    use std::collections::HashMap;

    #[test]
    fn raio_do_app_ou_o_padrao() {
        let mut requested = HashMap::new();
        requested.insert("mide-text".to_string(), 24);
        assert_eq!(radius_for(&requested, "mide-text"), 24.0);
        // Um app que não pediu (cliente de terceiros) fica com o padrão de 16.
        assert_eq!(radius_for(&requested, "foot"), 16.0);
        assert_eq!(radius_for(&requested, ""), 16.0);
        assert_eq!(radius_for(&HashMap::new(), "mide-text"), 16.0);
    }
}

/// Título da janela (vazio se o app não definiu um).
/// `app_id` da janela (vazio sem ele).
fn app_id_of(window: &Window) -> String {
    window
        .toplevel()
        .and_then(|t| {
            smithay::wayland::compositor::with_states(t.wl_surface(), |s| {
                s.data_map
                    .get::<smithay::wayland::shell::xdg::XdgToplevelSurfaceData>()
                    .and_then(|d| d.lock().ok().and_then(|a| a.app_id.clone()))
            })
        })
        .unwrap_or_default()
}

pub fn window_title(window: &Window) -> String {
    toplevel_title_of(window)
}

fn toplevel_title_of(window: &Window) -> String {
    window
        .toplevel()
        .map(|t| toplevel_title(t))
        .unwrap_or_default()
}

/// Título que o app definiu para a janela (vazio se não definiu).
fn toplevel_title(toplevel: &smithay::wayland::shell::xdg::ToplevelSurface) -> String {
    use smithay::wayland::compositor::with_states;
    use smithay::wayland::shell::xdg::XdgToplevelSurfaceData;
    with_states(toplevel.wl_surface(), |states| {
        states
            .data_map
            .get::<XdgToplevelSurfaceData>()
            .and_then(|data| data.lock().ok().and_then(|attrs| attrs.title.clone()))
    })
    .unwrap_or_default()
}

/// Nome das regiões de vidro de uma janela no canal de controle: `app:<app_id>` para a
/// janela sem pai e `app:<app_id>><título>` para as filhas (Preferências de um app do
/// mide-ui), que têm o mesmo `app_id` da principal. No título, espaço vira `%20` e `%`
/// vira `%25`, como o mide-ui manda (as palavras do canal são separadas por espaço).
fn glass_key(window: &smithay::desktop::Window) -> String {
    let Some(toplevel) = window.toplevel() else {
        return "app:".into();
    };
    let (app_id, title) = smithay::wayland::compositor::with_states(toplevel.wl_surface(), |s| {
        s.data_map
            .get::<smithay::wayland::shell::xdg::XdgToplevelSurfaceData>()
            .and_then(|d| d.lock().ok().map(|a| (a.app_id.clone().unwrap_or_default(), a.title.clone().unwrap_or_default())))
            .unwrap_or_default()
    });
    if toplevel.parent().is_some() {
        format!("app:{app_id}>{}", encode_glass_title(&title))
    } else {
        format!("app:{app_id}")
    }
}

/// Título no nome do vidro: espaços como `%20` e `%` como `%25`.
pub(crate) fn encode_glass_title(title: &str) -> String {
    let mut out = String::with_capacity(title.len());
    for c in title.chars() {
        match c {
            '%' => out.push_str("%25"),
            c if c.is_whitespace() => out.push_str("%20"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod glass_key_tests {
    #[test]
    fn titulo_codificado() {
        assert_eq!(super::encode_glass_title("Preferências do Code"), "Preferências%20do%20Code");
        assert_eq!(super::encode_glass_title("50% zoom"), "50%25%20zoom");
    }
}
