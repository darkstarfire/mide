//! Backend sem tela (testes): o mide desenha na GPU pelo render node
//! (`/dev/dri/renderD128`, sem sessão nem monitor), em texturas do tamanho de cada
//! tela virtual. Nenhuma janela aparece na tela de quem usa o computador, e a
//! orientação é a do DRM (sem inversão no eixo y).
//!
//! Telas: `MIDE_HEADLESS_OUTPUTS=1366x768,1920x1080` (nomes `HEADLESS-1`,
//! `HEADLESS-2`...); sem ela, uma tela do tamanho de `MIDE_WINIT_SIZE` (padrão
//! 1366x768). Cada uma aceita alguns modos comuns, para testar as Configurações →
//! Monitores (resolução, posição, principal, ligar e desligar).
//!
//! A entrada vem só do autoteste (`MIDE_SELFTEST`); os quadros saem no ritmo de 60 por
//! segundo enquanto há autoteste ou algo mudando.

use std::{cell::RefCell, error::Error, rc::Rc, time::Duration};

use smithay::{
    backend::{
        allocator::{gbm::GbmDevice, Fourcc},
        drm::DrmDeviceFd,
        egl::{EGLContext, EGLDisplay},
        renderer::{
            damage::OutputDamageTracker,
            gles::{GlesRenderer, GlesTexture},
            Bind, Offscreen,
        },
    },
    output::{Mode, Output, PhysicalProperties, Subpixel},
    reexports::calloop::{
        timer::{TimeoutAction, Timer},
        LoopHandle,
    },
    utils::{Buffer, DeviceFd, Size, Transform},
};

use crate::{
    render::{send_frames, Scene},
    state::{OutputInfo, State},
};

/// Modos que toda tela virtual aceita (além do dela).
const MODES: [(i32, i32, i32); 4] = [(1920, 1080, 60_000), (1366, 768, 60_000), (1280, 720, 60_000), (2560, 1440, 60_000)];

/// Uma tela virtual.
struct Head {
    name: String,
    /// Modo de quando o mide abriu (o "preferido").
    native: (i32, i32),
    output: Option<Output>,
    /// O global `wl_output` da tela ligada (removido ao desligar).
    global: Option<smithay::reexports::wayland_server::backend::GlobalId>,
    target: Option<GlesTexture>,
    damage_tracker: Option<OutputDamageTracker>,
    /// O alvo já tem um quadro (é sempre o mesmo: o conteúdo é o do quadro anterior,
    /// idade 1). Assim os testes desenham só o que mudou, como o DRM.
    drawn: bool,
}

struct Gpu {
    renderer: GlesRenderer,
    scene: Scene,
    heads: Vec<Head>,
}

pub fn init(loop_handle: &LoopHandle<State>, state: &mut State) -> Result<(), Box<dyn Error>> {
    let path = std::env::var("MIDE_HEADLESS_DEVICE").unwrap_or_else(|_| "/dev/dri/renderD128".into());
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|e| format!("sem acesso a {path}: {e}"))?;
    let fd = DrmDeviceFd::new(DeviceFd::from(std::os::fd::OwnedFd::from(file)));
    // O fd já é um render node: é a placa anunciada aos apps no linux-dmabuf.
    let node = crate::dmabuf::render_node(&fd);
    let gbm = GbmDevice::new(fd)?;
    // SAFETY: o display EGL guarda uma cópia do dispositivo GBM, que fica vivo com ele.
    let egl = unsafe { EGLDisplay::new(gbm)? };
    let context = EGLContext::new(&egl)?;
    // SAFETY: o contexto acabou de ser criado e só este renderizador o usa.
    let mut renderer = unsafe { GlesRenderer::new(context)? };
    // Como no DRM: o alvo não é invertido no eixo y.
    crate::clip::set_flipped(false);

    let sizes: Vec<(i32, i32)> = std::env::var("MIDE_HEADLESS_OUTPUTS")
        .ok()
        .map(|list| list.split(',').filter_map(parse_size).collect::<Vec<_>>())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| vec![std::env::var("MIDE_WINIT_SIZE").ok().as_deref().and_then(parse_size).unwrap_or((1366, 768))]);
    let heads = sizes
        .iter()
        .enumerate()
        .map(|(i, size)| Head { name: format!("HEADLESS-{}", i + 1), native: *size, output: None, global: None, target: None, damage_tracker: None, drawn: false })
        .collect();
    // `MIDE_SCENE_CURSOR=1`: a cena desenha o cursor, como no DRM sem o plano de cursor
    // (para medir o ponteiro agitado e o custo do cursor na cena).
    let scene = Scene::new(&mut renderer, state)?;
    let scene = if std::env::var_os("MIDE_SCENE_CURSOR").is_some() { scene.with_cursor() } else { scene };
    let gpu = Rc::new(RefCell::new(Gpu { renderer, scene, heads }));
    // Buffers dos apps na GPU (linux-dmabuf): importados neste renderizador.
    {
        let importer = gpu.clone();
        crate::dmabuf::setup(
            state,
            &gpu.borrow().renderer,
            node,
            crate::dmabuf::access(importer, |g: &mut Gpu| &mut g.renderer),
        );
    }
    refresh_heads(&mut gpu.borrow_mut(), state)?;
    tracing::info!("saída sem tela criada ({} tela(s), {path})", sizes.len());

    // As Configurações mudaram as telas (`outputs apply`).
    {
        let (ping, source) = smithay::reexports::calloop::ping::make_ping()?;
        let gpu = gpu.clone();
        loop_handle.insert_source(source, move |_, _, state: &mut State| {
            if let Err(err) = refresh_heads(&mut gpu.borrow_mut(), state) {
                tracing::warn!("telas sem tela: {err}");
            }
        })?;
        state.outputs_waker = Some(ping);
    }

    let mut selftest = crate::selftest::SelfTest::from_env();
    loop_handle.insert_source(Timer::from_duration(Duration::from_millis(16)), move |_, _, state| {
        let mut gpu = gpu.borrow_mut();
        let wanted = selftest.is_some() || state.redraw || gpu.scene.moving(state);
        if wanted {
            if let Some(test) = selftest.as_mut() {
                test.tick(state);
            }
            state.redraw = false;
            let Gpu { renderer, scene, heads } = &mut *gpu;
            for head in heads.iter_mut() {
                let (Some(output), Some(target), Some(damage_tracker)) = (&head.output, &mut head.target, &mut head.damage_tracker)
                else {
                    continue;
                };
                // `MIDE_FULL_DAMAGE=1`: tudo de novo a cada quadro (para comparar).
                let age = if head.drawn && std::env::var_os("MIDE_FULL_DAMAGE").is_none() { 1 } else { 0 };
                let result = (|| -> Result<(), Box<dyn Error>> {
                    let mut framebuffer = renderer.bind(target)?;
                    let _ = scene.render(renderer, &mut framebuffer, output, damage_tracker, state, age)?;
                    Ok(())
                })();
                head.drawn = result.is_ok();
                if let Err(err) = result {
                    tracing::warn!("falha ao desenhar a tela {}: {err}", head.name);
                }
                // Sem vblank: o quadro desenhado conta como apresentado agora (a
                // frequência anunciada é a do modo da tela virtual, 60 Hz). Quadro que
                // falhou: os pedidos ficam para o próximo.
                if head.drawn {
                    crate::presentation::present_now(crate::presentation::collect(state, output), output);
                }
                if let Err(err) = send_frames(state, output) {
                    tracing::debug!("avisos de quadro: {err}");
                }
            }
        }
        TimeoutAction::ToDuration(Duration::from_millis(16))
    })?;
    Ok(())
}

fn parse_size(text: &str) -> Option<(i32, i32)> {
    let (w, h) = text.trim().split_once('x')?;
    Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
}

/// Liga, desliga e redimensiona as telas virtuais pela configuração e arranja.
fn refresh_heads(gpu: &mut Gpu, state: &mut State) -> Result<(), Box<dyn Error>> {
    let monitors = state.config.monitors.clone();
    let any_enabled = gpu.heads.iter().any(|h| monitors.outputs.get(&h.name).is_none_or(|m| m.enabled));
    crate::outputs::sweep_globals(state);
    for (i, head) in gpu.heads.iter_mut().enumerate() {
        let config = monitors.outputs.get(&head.name);
        let enabled = config.is_none_or(|m| m.enabled) || (!any_enabled && i == 0);
        if !enabled {
            if head.output.take().is_some() {
                tracing::info!("tela {} desligada", head.name);
            }
            if let Some(id) = head.global.take() {
                crate::outputs::retire_global(state, id);
            }
            head.target = None;
            head.damage_tracker = None;
            continue;
        }
        let size = config
            .and_then(|m| m.parsed_mode())
            .map(|(w, h, _)| (w as i32, h as i32))
            .filter(|s| *s == head.native || MODES.iter().any(|(w, h, _)| (*w, *h) == *s))
            .unwrap_or(head.native);
        let global = &mut head.global;
        let output = head.output.get_or_insert_with(|| {
            let output = Output::new(
                head.name.clone(),
                PhysicalProperties { size: (0, 0).into(), subpixel: Subpixel::Unknown, make: "mide".into(), model: "headless".into() },
            );
            *global = Some(output.create_global::<State>(&state.display_handle));
            output
        });
        let mode = Mode { size: size.into(), refresh: 60_000 };
        let scale = crate::backend_udev::output_scale(config.map(|m| m.scale).unwrap_or(1.0));
        if (output.current_scale().fractional_scale() - scale.fractional_scale()).abs() > 0.001 {
            output.change_current_state(None, None, Some(scale), None);
            tracing::info!("tela {}: escala {}", head.name, scale.fractional_scale());
        }
        if output.current_mode() != Some(mode) || head.target.is_none() {
            output.change_current_state(Some(mode), Some(Transform::Normal), Some(scale), None);
            output.set_preferred(mode);
            head.target = Some(Offscreen::<GlesTexture>::create_buffer(&mut gpu.renderer, Fourcc::Abgr8888, Size::<i32, Buffer>::from(size))?);
            head.drawn = false;
            head.damage_tracker = Some(OutputDamageTracker::from_output(output));
            tracing::info!("tela {}: {}x{}", head.name, size.0, size.1);
        }
    }
    let live: Vec<Output> = gpu.heads.iter().filter_map(|h| h.output.clone()).collect();
    crate::outputs::place(state, &live);
    let primary = state.primary_output().map(|o| o.name());
    state.outputs_info = gpu
        .heads
        .iter()
        .map(|h| {
            let current = h.output.as_ref().and_then(|o| o.current_mode()).map(|m| (m.size.w, m.size.h)).unwrap_or(h.native);
            let mut modes = vec![(h.native.0, h.native.1, 60_000)];
            modes.extend(MODES.iter().filter(|m| (m.0, m.1) != h.native));
            OutputInfo {
                name: h.name.clone(),
                description: "tela virtual".into(),
                modes,
                current: (current.0, current.1, 60_000),
                position: h.output.as_ref().and_then(|o| state.space.output_geometry(o)).map(|g| (g.loc.x, g.loc.y)).unwrap_or((0, 0)),
                scale: monitors.outputs.get(&h.name).map(|m| m.scale).unwrap_or(1.0),
                enabled: h.output.is_some(),
                primary: primary.as_deref() == Some(h.name.as_str()),
            }
        })
        .collect();
    state.redraw = true;
    Ok(())
}
