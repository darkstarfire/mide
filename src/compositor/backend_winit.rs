//! Backend winit: roda o mide numa janela dentro da sessão atual.
//!
//! É o backend de desenvolvimento. Nele, as janelas dos apps aparecem dentro
//! de uma janela do mide, sem precisar de DRM nem de sair da sessão gráfica.
//! Aqui ficam só a janela do anfitrião, a saída, os eventos e a apresentação do
//! quadro; o que é desenhado vem da cena (`crate::render`).

use std::{error::Error, time::Duration};

use smithay::{
    backend::{
        renderer::{damage::OutputDamageTracker, gles::GlesRenderer},
        winit::{self, WinitEvent},
    },
    output::{Mode, Output, PhysicalProperties, Subpixel},
    reexports::{
        calloop::{EventLoop, LoopHandle, LoopSignal},
        winit::window::CursorIcon,
    },
    utils::Transform,
};

use crate::{
    input::CursorShape,
    render::{send_frames, Scene},
    state::State,
};

/// Taxa de atualização assumida para o modo da saída virtual (em mHz).
const REFRESH_MHZ: i32 = 60_000;

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
    let selftest = std::rc::Rc::new(std::cell::RefCell::new(crate::selftest::SelfTest::from_env()));
    // Quando o anfitrião pede o último quadro (para o relógio de reserva abaixo).
    let last_redraw = std::rc::Rc::new(std::cell::Cell::new(std::time::Instant::now()));
    let mut scene = Scene::new(backend.renderer(), state)?;
    // Modo de jogo em tela cheia (A18): o scanout direto vale aqui?
    state.game.scanout_note = Some("não: mide em janela (winit), quem mostra é o desktop de baixo");

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
    // Com MIDE_SIM_DRM=1 (testes), a cena é desenhada como no DRM: sem inversão, numa
    // textura (como o buffer da tela), que depois é copiada para a janela.
    let transform = if sim_drm() {
        crate::clip::set_flipped(false);
        tracing::info!("simulando a orientação do DRM (MIDE_SIM_DRM)");
        Transform::Normal
    } else {
        Transform::Flipped180
    };
    output.change_current_state(Some(mode), Some(transform), None, Some((0, 0).into()));
    output.set_preferred(mode);
    state.space.map_output(&output, (0, 0));

    let mut damage_tracker = OutputDamageTracker::from_output(&output);
    let stop: LoopSignal = event_loop.get_signal();
    // Sem o primeiro pedido, o winit pode nunca enviar `Redraw` e a janela fica vazia.
    backend.window().request_redraw();
    // Com autoteste (testes), desenha sem parar: os roteiros contam quadros.
    let continuous = selftest.borrow().is_some();
    // A janela do anfitrião é usada também pelo relógio de reserva (pedir quadros).
    let backend = std::rc::Rc::new(std::cell::RefCell::new(backend));
    // Buffers dos apps na GPU (linux-dmabuf), se o EGL do desktop de baixo disser qual
    // é a placa (sem isso, o mide em janela fica sem o protocolo: é só desenvolvimento).
    {
        let mut host = backend.borrow_mut();
        let node = crate::dmabuf::render_node_of_egl(host.renderer().egl_context().display());
        let importer = backend.clone();
        crate::dmabuf::setup(
            state,
            host.renderer(),
            node,
            crate::dmabuf::access(importer, |b: &mut winit::WinitGraphicsBackend<GlesRenderer>| b.renderer()),
        );
    }
    {
        let (ping, source) = smithay::reexports::calloop::ping::make_ping()?;
        let host = backend.clone();
        loop_handle.insert_source(source, move |_, _, state: &mut State| {
            if state.redraw {
                if let Ok(backend) = host.try_borrow() {
                    backend.window().request_redraw();
                }
            }
        })?;
        state.waker = Some(ping);
    }

    // Relógio de reserva: o anfitrião (GNOME etc.) para de pedir quadros quando a
    // janela do mide fica escondida ou coberta. Sem isso, o autoteste e os apps
    // congelavam (os apps esperam o aviso de quadro). Com algo pendente e 100 ms sem
    // pedido, o relógio avança o autoteste e libera os apps, sem apresentar na tela
    // escondida. Sem nada pendente, só confere a cada 100 ms (quase sem custo).
    {
        let (selftest, last_redraw, output) = (selftest.clone(), last_redraw.clone(), output.clone());
        let host = backend.clone();
        let (busy_tick, idle_tick) = (Duration::from_millis(16), Duration::from_millis(100));
        loop_handle.insert_source(smithay::reexports::calloop::timer::Timer::from_duration(busy_tick), move |_, _, state| {
            if !state.redraw && !continuous {
                return smithay::reexports::calloop::timer::TimeoutAction::ToDuration(idle_tick);
            }
            // Pedido que ficou sem resposta: pede de novo ao anfitrião.
            if let Ok(backend) = host.try_borrow() {
                backend.window().request_redraw();
            }
            if last_redraw.get().elapsed() > Duration::from_millis(100) {
                tracing::trace!("anfitrião sem pedir quadros: seguindo pelo relógio");
                if let Some(test) = selftest.borrow_mut().as_mut() {
                    test.tick(state);
                }
                state.finish_animations(std::time::Instant::now());
                if let Err(err) = send_frames(state, &output) {
                    tracing::debug!("relógio de reserva: {err}");
                }
                // Continua só com algo em movimento; um commit novo acorda de novo.
                state.redraw = !state.animations.is_empty() || state.launcher_anim.is_some() || !state.ghosts.is_empty() || !state.appearing.is_empty();
            }
            smithay::reexports::calloop::timer::TimeoutAction::ToDuration(busy_tick)
        })?;
    }
    let (test_redraw, redraw_seen) = (selftest.clone(), last_redraw.clone());
    loop_handle.insert_source(winit_source, move |event, _, state| {
        let mut backend = backend.borrow_mut();
        match event {
            WinitEvent::Resized { size, .. } => {
                let mode = Mode {
                    size,
                    refresh: REFRESH_MHZ,
                };
                output.change_current_state(Some(mode), None, None, None);
                output.set_preferred(mode);
                state.output_resized();
                state.wake();
            }
            WinitEvent::Redraw => {
                redraw_seen.set(std::time::Instant::now());
                if let Some(test) = test_redraw.borrow_mut().as_mut() {
                    test.tick(state);
                }
                state.redraw = false;
                if let Err(err) = render(&mut backend, &output, &mut damage_tracker, state, &mut scene) {
                    tracing::warn!("falha ao desenhar frame: {err}");
                }
                // Só continua desenhando com algo em movimento (animações, papel de parede
                // trocando, janela fechando) ou no autoteste; senão espera algo mudar.
                if scene.moving(state) || continuous {
                    backend.window().request_redraw();
                    state.redraw = true;
                }
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
                backend.window().set_cursor(winit_cursor(state.cursor, &state.cursor_status));
                backend.window().request_redraw();
            }
            // Janela do mide perdeu o foco no desktop de baixo (Alt+Tab, Super): o
            // soltar das teclas vai para outra janela. Solta tudo, senão o Alt fica
            // "preso" e cada letra chega aos apps como Alt+letra.
            WinitEvent::Focus(false) => crate::input::release_all_keys(state),
            WinitEvent::Focus(true) => {}
        }
    })?;

    tracing::info!("saída winit criada ({}x{})", size.w, size.h);
    Ok(())
}

/// Forma do ponteiro na janela do anfitrião: a das bordas das janelas do mide e, fora
/// delas, a que o app sob o ponteiro pediu pelo nome (`wp_cursor_shape` ou o tema, como o
/// de redimensionar nas divisórias do Code). Um cursor desenhado pelo app (superfície)
/// não passa para o anfitrião: fica a seta.
fn winit_cursor(shape: CursorShape, status: &smithay::input::pointer::CursorImageStatus) -> CursorIcon {
    match shape {
        CursorShape::Default => match status {
            smithay::input::pointer::CursorImageStatus::Named(icon) => *icon,
            _ => CursorIcon::Default,
        },
        CursorShape::ResizeHorizontal => CursorIcon::EwResize,
        CursorShape::ResizeVertical => CursorIcon::NsResize,
        CursorShape::ResizeNwse => CursorIcon::NwseResize,
        CursorShape::ResizeNesw => CursorIcon::NeswResize,
    }
}

/// Desenha um quadro da cena na janela do anfitrião, apresenta e envia os frame callbacks.
fn render(
    backend: &mut winit::WinitGraphicsBackend<GlesRenderer>,
    output: &Output,
    damage_tracker: &mut OutputDamageTracker,
    state: &mut State,
    scene: &mut Scene,
) -> Result<(), Box<dyn Error>> {
    // O app pede o ponteiro depois do movimento que o levou até ele: aplicado também a
    // cada quadro, a troca não espera o próximo movimento do mouse.
    backend.window().set_cursor(winit_cursor(state.cursor, &state.cursor_status));
    if sim_drm() {
        render_sim_drm(backend, output, damage_tracker, state, scene)?;
    } else {
        let size = backend.window_size();
        let (renderer, mut framebuffer) = backend.bind()?;
        // O `submit` do winit troca os buffers depois do desenho terminar (como antes da
        // cena ter saído daqui): o ponto de sincronização fica sem uso.
        let _sync = scene.render(renderer, &mut framebuffer, output, damage_tracker, state, 0)?;
        // As capturas atendidas no quadro (leitura dos pixels, cópia para dmabuf, gravação)
        // deixam o contexto do EGL sem a janela: sem religar, a troca de buffers falha.
        if state.screencopy.served_this_frame() || state.recording.is_some() {
            crate::screencopy::reattach_window(renderer, &mut framebuffer, size);
        }
    }
    // Apresenta o quadro inteiro. Com dano parcial, o buffer de trás pode ter pixels
    // de quadros antigos, e a diferença aparece como padrão pontilhado nas sombras.
    backend.submit(None)?;
    // Sem a hora do vblank do anfitrião: o quadro conta como apresentado ao entregá-lo.
    crate::presentation::present_now(crate::presentation::collect(state, output), output);

    send_frames(state, output)
}

/// Testes: simular a orientação do backend DRM (`MIDE_SIM_DRM=1`).
fn sim_drm() -> bool {
    std::env::var("MIDE_SIM_DRM").is_ok_and(|v| v == "1")
}

/// Como no DRM: a cena vai para uma textura sem inversão (o quadro salvo por
/// `MIDE_DUMP_FRAME` sai dela, de cabeça para baixo em relação à janela), e a textura
/// é copiada para a janela, invertida como o GL do winit precisa.
fn render_sim_drm(
    backend: &mut winit::WinitGraphicsBackend<GlesRenderer>,
    output: &Output,
    damage_tracker: &mut OutputDamageTracker,
    state: &mut State,
    scene: &mut Scene,
) -> Result<(), Box<dyn Error>> {
    use smithay::backend::renderer::{Bind, Frame, Offscreen, Renderer};
    use smithay::utils::{Buffer, Rectangle, Size};
    let size = backend.window_size();
    let mut texture: smithay::backend::renderer::gles::GlesTexture = backend
        .renderer()
        .create_buffer(smithay::backend::allocator::Fourcc::Abgr8888, Size::<i32, Buffer>::from((size.w, size.h)))?;
    {
        let renderer = backend.renderer();
        let mut target = renderer.bind(&mut texture)?;
        let _sync = scene.render(renderer, &mut target, output, damage_tracker, state, 0)?;
    }
    let (renderer, mut framebuffer) = backend.bind()?;
    let mut frame = renderer.render(&mut framebuffer, size, Transform::Flipped180)?;
    let whole = Rectangle::from_size(size);
    frame.render_texture_from_to(
        &texture,
        Rectangle::from_size((size.w as f64, size.h as f64).into()),
        whole,
        &[whole],
        &[],
        Transform::Normal,
        1.0,
        None,
        &[],
    )?;
    let _ = frame.finish()?;
    Ok(())
}
