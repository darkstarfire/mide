//! Backend winit: roda o mide numa janela dentro da sessão atual.
//!
//! É o backend de desenvolvimento. Nele, as janelas dos apps aparecem dentro
//! de uma janela do mide, sem precisar de DRM nem de sair da sessão gráfica.

use std::{error::Error, time::Duration};

use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State as XdgState;
use smithay::{
    backend::{
        renderer::{
            damage::OutputDamageTracker,
            element::{
                surface::{render_elements_from_surface_tree, WaylandSurfaceRenderElement},
                utils::RescaleRenderElement,
                Kind,
            },
            gles::GlesRenderer,
        },
        winit::{self, WinitEvent},
    },
    desktop::{
        layer_map_for_output, space::render_output, utils::send_frames_surface_tree, PopupManager,
        Space, Window,
    },
    output::{Mode, Output, PhysicalProperties, Subpixel},
    reexports::calloop::{EventLoop, LoopHandle, LoopSignal},
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
    Textured=smithay::backend::renderer::gles::element::TextureShaderElement,
    /// Textura pronta, sem shader próprio (fundo desfocado do launcher).
    Blurred=smithay::backend::renderer::element::texture::TextureRenderElement<smithay::backend::renderer::gles::GlesTexture>,
    ClippedSurface=crate::clip::Clipped<WaylandSurfaceRenderElement<GlesRenderer>>,
    ClippedScaled=crate::clip::Clipped<RescaleRenderElement<WaylandSurfaceRenderElement<GlesRenderer>>>,
}

/// Taxa de atualização assumida para o modo da saída virtual (em mHz).
const REFRESH_MHZ: i32 = 60_000;

/// Cor de fundo do desktop se o papel de parede não puder ser desenhado.
const BACKGROUND: [f32; 4] = [0.12, 0.12, 0.14, 1.0];

/// Cria a janela winit, registra a saída e agenda o desenho contínuo.
pub fn init(
    event_loop: &EventLoop<State>,
    loop_handle: &LoopHandle<State>,
    state: &mut State,
) -> Result<(), Box<dyn Error>> {
    // Por padrão a janela do winit fica sem moldura do desktop hospedeiro, para a
    // barra do próprio mide ser a única. MIDE_WINIT_DECORATIONS=1 mantém a moldura.
    let host_decorations = std::env::var("MIDE_WINIT_DECORATIONS").is_ok_and(|v| v == "1");
    let attributes = smithay::reexports::winit::window::WindowAttributes::default()
        .with_title("mide")
        .with_decorations(host_decorations)
        .with_min_inner_size(smithay::reexports::winit::dpi::LogicalSize::new(640.0, 400.0));
    // Tamanho inicial: 1366x768 (a resolução de tela mais comum no Brasil), ou
    // MIDE_WINIT_SIZE=LxA (os testes de interface usam 800x600).
    let attributes = match std::env::var("MIDE_WINIT_SIZE")
        .ok()
        .or_else(|| Some("1366x768".to_string()))
        .and_then(|v| v.split_once('x').map(|(w, h)| (w.parse::<f64>(), h.parse::<f64>())))
    {
        Some((Ok(w), Ok(h))) => {
            attributes.with_inner_size(smithay::reexports::winit::dpi::LogicalSize::new(w, h))
        }
        _ => attributes,
    };
    // Sessões de teste (MIDE_WINIT_FIXED): janela de tamanho fixo, para o anfitrião
    // não maximizar sozinho (o GNOME maximiza janelas grandes ao abrir) e as
    // coordenadas dos testes valerem.
    let attributes = if std::env::var_os("MIDE_WINIT_FIXED").is_some() {
        attributes.with_resizable(false)
    } else {
        attributes
    };
    let (mut backend, winit_source) = winit::init_from_attributes::<GlesRenderer>(attributes)?;
    let mut titles = crate::title::TitleRenderer::new();
    let selftest = std::rc::Rc::new(std::cell::RefCell::new(crate::selftest::SelfTest::from_env()));
    // Quando o anfitrião pede o último quadro (para o relógio de reserva abaixo).
    let last_redraw = std::rc::Rc::new(std::cell::Cell::new(std::time::Instant::now()));
    let shaders = decorations::Shaders::compile(backend.renderer())
        .map_err(|e| format!("falha ao compilar shaders da decoração: {e}"))?;
    let genie = crate::genie::Genie::compile(backend.renderer())
        .map_err(|e| format!("falha ao compilar o shader do efeito lâmpada: {e}"))?;
    let blur = crate::blur::Blur::compile(backend.renderer())
        .map_err(|e| format!("falha ao compilar os shaders do desfoque: {e}"))?;
    let wallpaper = crate::wallpaper::Wallpaper::new(backend.renderer(), state.config.desktop.wallpaper_path())
        .map_err(|e| format!("falha ao compilar o shader do papel de parede: {e}"))?;

    let size = backend.window_size();
    let output = Output::new(
        "mide-winit".to_string(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "mide".into(),
            model: "winit".into(),
        },
    );
    let mode = Mode {
        size,
        refresh: REFRESH_MHZ,
    };
    output.create_global::<State>(&state.display_handle);
    // O GL do winit tem a origem embaixo, então a saída precisa ser invertida.
    output.change_current_state(Some(mode), Some(Transform::Flipped180), None, Some((0, 0).into()));
    output.set_preferred(mode);
    state.space.map_output(&output, (0, 0));

    let mut damage_tracker = OutputDamageTracker::from_output(&output);
    let stop: LoopSignal = event_loop.get_signal();
    // Sem o primeiro pedido, o winit pode nunca enviar `Redraw` e a janela fica vazia.
    backend.window().request_redraw();

    // Relógio de reserva: o anfitrião (GNOME etc.) para de pedir quadros quando a
    // janela do mide fica escondida ou coberta. Sem isso, o autoteste e os apps
    // congelavam (os apps esperam o aviso de quadro). Passados 100 ms sem pedido, o
    // relógio avança o autoteste e libera os apps, sem apresentar na tela escondida.
    {
        let (selftest, last_redraw, output) = (selftest.clone(), last_redraw.clone(), output.clone());
        let tick = Duration::from_millis(16);
        loop_handle.insert_source(smithay::reexports::calloop::timer::Timer::from_duration(tick), move |_, _, state| {
            if last_redraw.get().elapsed() > Duration::from_millis(100) {
                tracing::trace!("anfitrião sem pedir quadros: seguindo pelo relógio");
                if let Some(test) = selftest.borrow_mut().as_mut() {
                    test.tick(state);
                }
                state.finish_animations(std::time::Instant::now());
                if let Err(err) = send_frames(state, &output) {
                    tracing::debug!("relógio de reserva: {err}");
                }
            }
            smithay::reexports::calloop::timer::TimeoutAction::ToDuration(tick)
        })?;
    }
    let (test_redraw, redraw_seen) = (selftest.clone(), last_redraw.clone());
    loop_handle.insert_source(winit_source, move |event, _, state| match event {
        WinitEvent::Resized { size, .. } => {
            let mode = Mode {
                size,
                refresh: REFRESH_MHZ,
            };
            output.change_current_state(Some(mode), None, None, None);
            output.set_preferred(mode);
            state.output_resized();
        }
        WinitEvent::Redraw => {
            redraw_seen.set(std::time::Instant::now());
            if let Some(test) = test_redraw.borrow_mut().as_mut() {
                test.tick(state);
            }
            if let Err(err) = render(&mut backend, &output, &mut damage_tracker, state, &shaders, &genie, &blur, &wallpaper, &mut titles) {
                tracing::warn!("falha ao desenhar frame: {err}");
            }
            backend.window().request_redraw();
        }
        WinitEvent::CloseRequested => {
            tracing::info!("janela do mide fechada, encerrando");
            stop.stop();
        }
        WinitEvent::Input(event) => {
            // A saída tem escala 1, então pixels físicos e lógicos coincidem.
            let physical = backend.window_size();
            let size = (physical.w, physical.h).into();
            crate::input::handle_event(state, event, size);
            backend.window().set_cursor(state.cursor_icon);
            backend.window().request_redraw();
        }
        WinitEvent::Focus(_) => {}
    })?;

    tracing::info!("saída winit criada ({}x{})", size.w, size.h);
    Ok(())
}

/// Elementos de render dos popups de todas as janelas, para ficarem acima delas.
fn popup_elements(
    renderer: &mut GlesRenderer,
    state: &State,
) -> Vec<WaylandSurfaceRenderElement<GlesRenderer>> {
    let mut elements = Vec::new();
    for window in state.space.elements() {
        let (Some(toplevel), Some(loc)) = (window.toplevel(), state.space.element_location(window))
        else {
            continue;
        };
        for (popup, popup_offset) in PopupManager::popups_for_surface(toplevel.wl_surface()) {
            let offset = popup_offset - popup.geometry().loc;
            let location: Point<i32, Physical> = (loc + offset).to_physical(1);
            elements.extend(render_elements_from_surface_tree(
                renderer,
                popup.wl_surface(),
                location,
                1.0,
                1.0,
                Kind::Unspecified,
            ));
        }
    }
    elements
}

/// Elementos de todas as janelas, da frente para trás: contorno, máscaras dos
/// cantos, barra, conteúdo do app e sombra. Durante a animação de maximizar, o
/// conteúdo e a moldura seguem a interpolação entre o estado antigo e o novo.
fn window_elements(
    renderer: &mut GlesRenderer,
    state: &State,
    output: &Output,
    shaders: &decorations::Shaders,
    genie: &crate::genie::Genie,
    wallpaper: &crate::wallpaper::Wallpaper,
    titles: &mut crate::title::TitleRenderer,
) -> Vec<MideRenderElement> {
    use smithay::backend::renderer::element::AsRenderElements;

    let now = std::time::Instant::now();

    // Primeira passada, de cima para baixo: posição, zoom e moldura de cada janela.
    struct Placed {
        window: Window,
        /// Origem da superfície: posição da janela menos o deslocamento da geometria,
        /// a mesma conta do `render_location` do Smithay, usada também no clique.
        surface_origin: Point<i32, Logical>,
        zoom: f64,
        alpha: f32,
        frame: Rectangle<i32, Logical>,
        focused: bool,
        maximized: bool,
        animating: bool,
        /// Efeito lâmpada em andamento: área da janela, alvo e progresso (0 a 1).
        genie: Option<(Rectangle<i32, Logical>, Rectangle<i32, Logical>, f32)>,
    }
    let mut placed: Vec<Placed> = Vec::new();
    // `elements()` vai do fundo para o topo; aqui percorremos do topo para o fundo.
    for window in state.space.elements().rev() {
        let (Some(toplevel), Some(loc)) = (window.toplevel(), state.space.element_location(window))
        else {
            continue;
        };
        let state_flags = toplevel.current_state().states;
        let focused = state_flags.contains(XdgState::Activated);
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
        let frame = Rectangle::new((visible.x, visible.y - bar_h).into(), (frame_w, bar_h + frame_h).into());
        let genie_info = state.animation_for(toplevel.wl_surface()).filter(|a| a.genie).map(|a| {
            let e = a.eased(now);
            let from = Rectangle::new(a.from_loc, a.from_size);
            let to = Rectangle::new(a.to_loc, a.to_size);
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
        });
    }

    // Janelas que projetam sombra no desktop: as 8 de cima, com moldura (não
    // maximizadas e visíveis). Com sombras desligadas, a lista fica vazia.
    let casters: Vec<decorations::Caster> = if state.config.effects.shadows {
        placed
            .iter()
            .filter(|p| (!p.maximized || p.animating) && p.alpha > 0.99 && p.genie.is_none())
            .take(8)
            .map(|p| decorations::Caster {
                frame: p.frame,
                strength: decorations::shadow_strength(p.focused),
            })
            .collect()
    } else {
        Vec::new()
    };

    let output_rect = state
        .space
        .output_geometry(output)
        .unwrap_or_else(|| Rectangle::new((0, 0).into(), (800, 600).into()));
    let output_height = output_rect.size.h as f32;

    // Segunda passada, de cima para baixo: cada janela desenha sobre as de baixo.
    // Sombra e máscaras recebem as molduras das janelas de baixo para não cobri-las.
    let mut elements = Vec::new();
    for p in placed.iter() {
        // A barra fica no topo da moldura, que já está na área visível.
        let bar_h = crate::state::bar_height_for(&p.window);
        let bar = Rectangle::new(p.frame.loc, (p.frame.size.w, bar_h).into());
        // Com a janela quase invisível (minimizando ou restaurando), a moldura não aparece.
        let decorated = (!p.maximized || p.animating) && p.alpha > 0.99 && p.genie.is_none();
        let phys: Point<i32, smithay::utils::Physical> = p.surface_origin.to_physical(1);

        // Janela maximizada ocupa a tela inteira: sem cantos, contorno nem sombra.
        // Cada efeito pode ser desligado em `[effects]` no config.toml.
        let effects = state.config.effects;
        if decorated && effects.rounded_corners {
            elements.push(MideRenderElement::from(decorations::outline_element(
                shaders, p.frame, p.focused,
            )));
        }
        // Recorte arredondado pela moldura: vale para a barra e para o conteúdo, e os
        // cantos mostram o que está de fato atrás (desktop com sombra ou outra janela).
        let radius = if decorated && effects.rounded_corners {
            decorations::CORNER_RADIUS as f32
        } else {
            0.0
        };
        let clip_uniforms = crate::clip::uniforms(p.frame, radius, output_height);

        // Barra e título só existem nas janelas com decoração de servidor. Vêm antes do
        // conteúdo na lista, para ficarem na frente dele.
        if bar_h > 0 && p.genie.is_none() {
            let title = toplevel_title_of(&p.window);
            if let Some(text) = crate::title::element(renderer, titles, &title, p.focused, bar) {
                elements.push(MideRenderElement::Title(text));
            }
            elements.extend(
                decorations::bar_elements(
                    shaders,
                    bar,
                    p.focused,
                    decorations::button_at(bar, state.pointer_location),
                    &clip_uniforms,
                )
                .into_iter()
                .map(MideRenderElement::from),
            );
        }

        // Efeito lâmpada: a janela inteira (barra, título e conteúdo) deformada
        // substitui o desenho normal durante a animação.
        if let Some((content_rect, target, progress)) = p.genie {
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
                crate::title::element(renderer, titles, &toplevel_title_of(&p.window), p.focused, local_bar)
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
                decorations::CORNER_RADIUS as f32
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
                elements.push(MideRenderElement::Textured(element));
            }
            continue;
        }

        // Conteúdo do app, com zoom durante a animação.
        let content: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
            p.window.render_elements(renderer, phys, Scale::from(1.0), p.alpha);
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

    }

    // Fundo do desktop (papel de parede) com as sombras de todas as janelas, atrás
    // de tudo (fim da lista). Se a textura falhar, usa a cor lisa.
    // A configuração pode ter mudado (comando `reload`): segue o papel de parede dela.
    wallpaper.follow(state.config.desktop.wallpaper_path());
    match wallpaper.element(renderer, output_rect, &casters) {
        Some(element) => elements.push(MideRenderElement::Textured(element)),
        None => elements.push(MideRenderElement::from(decorations::desktop_element(
            shaders,
            output_rect,
            BACKGROUND,
            &casters,
        ))),
    }
    elements
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
    let map = layer_map_for_output(output);
    let mut elements: Vec<MideRenderElement> = Vec::new();
    let mut glass = Vec::new();
    // Do topo para o fundo, como a lista de render.
    for layer in crate::state::front_layers(&map).iter() {
        let Some(geo) = map.layer_geometry(layer) else {
            continue;
        };
        let location: Point<i32, Physical> = geo.loc.to_physical(1);
        // O launcher abre e fecha com esmaecer e zoom feitos aqui (GPU).
        let (alpha, zoom) = if layer.namespace() == "mide-launcher" { state.launcher_look() } else { (1.0, 1.0) };
        let surface: Vec<WaylandSurfaceRenderElement<GlesRenderer>> = render_elements_from_surface_tree(
            renderer,
            layer.wl_surface(),
            location,
            1.0,
            alpha,
            Kind::Unspecified,
        );
        if (zoom - 1.0).abs() > f64::EPSILON {
            let center: Point<i32, Physical> = (geo.loc.x + geo.size.w / 2, geo.loc.y + geo.size.h / 2).into();
            elements.extend(surface.into_iter().map(|e| MideRenderElement::Scaled(RescaleRenderElement::from_element(e, center, zoom))));
        } else {
            elements.extend(surface.into_iter().map(MideRenderElement::from));
        }
        match layer.namespace() {
            // Atrás do launcher, o papel de parede desfocado (como o Launchpad).
            "mide-launcher" if state.config.effects.launcher_blur => {
                let out = Rectangle::new((0, 0).into(), geo.size);
                if let Some(el) = blur.wallpaper_element(renderer, wallpaper.texture(), out, state.launcher_look().0) {
                    elements.push(MideRenderElement::Blurred(el));
                }
            }
            "mide-dock" if state.config.effects.dock_blur => {
                glass.push(GlassRegion { at: elements.len(), rect: geo, radius: 15.0, name: "mide-dock".into() });
            }
            namespace if state.config.effects.panel_glass => {
                // Regiões pedidas pela camada (barra, menus, notificações), relativas a ela.
                for (i, (rect, radius)) in state.glass.get(namespace).into_iter().flatten().enumerate() {
                    let on_screen = Rectangle::new(geo.loc + rect.loc, rect.size);
                    if let Some(rect) = on_screen.intersection(geo) {
                        glass.push(GlassRegion { at: elements.len(), rect, radius: *radius, name: format!("{namespace}#{i}") });
                    }
                }
            }
            _ => {}
        }
    }
    (elements, glass)
}

/// Vidro atrás de uma camada: posição na lista de render (logo depois dos elementos
/// da camada, ou seja, atrás dela), retângulo na tela, raio dos cantos e nome (para
/// o cache do desfoque).
struct GlassRegion {
    at: usize,
    rect: Rectangle<i32, Logical>,
    radius: f32,
    name: String,
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
        let g = element.geometry(smithay::utils::Scale::from(1.0));
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
    let flip = std::env::var("MIDE_BLUR_FLIP").map(|v| v != "0").unwrap_or(true);
    let y = if flip { out.h - area.loc.y - area.size.h } else { area.loc.y };
    Rectangle::new((area.loc.x as f64, y as f64).into(), (area.size.w as f64, area.size.h as f64).into())
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
    std::fs::write(format!("{path}.{w}x{h}.rgba"), bytes)?;
    tracing::info!("frame salvo para depuração ({w}x{h})");
    Ok(())
}

/// Desenha um quadro: fundo, camadas, janelas, e envia os frame callbacks.
fn render(
    backend: &mut winit::WinitGraphicsBackend<GlesRenderer>,
    output: &Output,
    damage_tracker: &mut OutputDamageTracker,
    state: &mut State,
    shaders: &decorations::Shaders,
    genie: &crate::genie::Genie,
    blur: &crate::blur::Blur,
    wallpaper: &crate::wallpaper::Wallpaper,
    titles: &mut crate::title::TitleRenderer,
) -> Result<(), Box<dyn Error>> {
    state.popups.cleanup();
    let now = std::time::Instant::now();
    state.finish_animations(now);
    tracing::debug!("frame: {} janela(s) mapeada(s)", state.space.elements().count());
    {
        let (renderer, mut framebuffer) = backend.bind()?;
        // Da frente para trás: prévias das janelas, popups, camadas (barra, dock) e
        // depois as janelas.
        let mut custom: Vec<MideRenderElement> = crate::preview::elements(renderer, state, shaders, titles);
        custom.extend(popup_elements(renderer, state).into_iter().map(MideRenderElement::from));
        let before_layers = custom.len();
        let (layers, glass) = layer_elements(renderer, output, state, blur, wallpaper);
        custom.extend(layers);
        let windows = window_elements(renderer, state, output, shaders, genie, wallpaper, titles);
        custom.extend(windows);
        // Vidro (dock, barra, menus): o que está atrás de cada região (tudo depois dela
        // na lista) é desenhado numa textura, desfocado e posto logo atrás da camada.
        // De trás para a frente: o vidro de um menu vê a dock já com o vidro dela.
        let out_size = output.current_mode().map(|m| m.size.to_logical(1)).unwrap_or_default();
        // Animações (lâmpada, maximizar, launcher) mudam shaders sem mudar a geometria:
        // nelas o desfoque é refeito a cada quadro.
        let animating = !state.animations.is_empty() || state.launcher_anim.is_some();
        let strength = if state.config.effects.liquid_glass { 1.0 } else { 0.0 };
        let mut regions = glass;
        regions.sort_by(|a, b| b.at.cmp(&a.at));
        for region in regions {
            let at = before_layers + region.at;
            let rect = region.rect;
            let src = blur_source(rect, out_size);
            let key = if animating { state.start_time.elapsed().as_nanos() as u64 ^ 0x9e37_79b9 } else { backdrop_key(&custom[at..], rect) };
            let behind = &custom[at..];
            let blurred = blur.region_blur(renderer, &region.name, key, out_size, src, strength, |renderer, target| {
                crate::blur::render_region(renderer, behind, rect, out_size, target)
            });
            match blurred {
                Some((blurred, size)) => {
                    let element = blur.composite_element(renderer, blurred, size, rect, region.radius, strength);
                    custom.insert(at, MideRenderElement::Textured(element));
                }
                None => tracing::trace!("vidro {}: área pequena ou falha ao desfocar", region.name),
            }
        }
        let result = render_output(
            output,
            renderer,
            &mut framebuffer,
            1.0,
            0,
            Vec::<&Space<Window>>::new(),
            &custom,
            damage_tracker,
            BACKGROUND,
        )?;
        tracing::debug!("quadro do compositor: {:.2} ms", now.elapsed().as_secs_f32() * 1000.0);
        dump_frame_if_requested(renderer, &framebuffer, output)?;
        drop(result);
    }
    // Apresenta o quadro inteiro. Com dano parcial, o buffer de trás pode ter pixels
    // de quadros antigos, e a diferença aparece como padrão pontilhado nas sombras.
    backend.submit(None)?;

    send_frames(state, output)
}

/// Avisa os clientes (janelas, camadas e popups) que podem desenhar o próximo quadro.
fn send_frames(state: &mut State, output: &Output) -> Result<(), Box<dyn Error>> {
    let elapsed = state.start_time.elapsed();
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
    state.space.refresh();
    state.display_handle.flush_clients()?;
    Ok(())
}

/// Título da janela (vazio se o app não definiu um).
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
