//! Backend sem tela (testes): o mide desenha na GPU pelo render node
//! (`/dev/dri/renderD128`, sem sessão nem monitor), numa textura do tamanho de
//! `MIDE_WINIT_SIZE` (padrão 1366x768). Nenhuma janela aparece na tela de quem usa o
//! computador, e a orientação é a do DRM (sem inversão no eixo y).
//!
//! A entrada vem só do autoteste (`MIDE_SELFTEST`); os quadros saem no ritmo de 60 por
//! segundo enquanto há autoteste ou algo mudando.

use std::{error::Error, time::Duration};

use smithay::{
    backend::{
        allocator::{gbm::GbmDevice, Fourcc},
        drm::DrmDeviceFd,
        egl::{EGLContext, EGLDisplay},
        renderer::{damage::OutputDamageTracker, gles::GlesRenderer, Bind, Offscreen},
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
    state::State,
};

pub fn init(loop_handle: &LoopHandle<State>, state: &mut State) -> Result<(), Box<dyn Error>> {
    let path = std::env::var("MIDE_HEADLESS_DEVICE").unwrap_or_else(|_| "/dev/dri/renderD128".into());
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|e| format!("sem acesso a {path}: {e}"))?;
    let fd = DrmDeviceFd::new(DeviceFd::from(std::os::fd::OwnedFd::from(file)));
    let gbm = GbmDevice::new(fd)?;
    // SAFETY: o display EGL guarda uma cópia do dispositivo GBM, que fica vivo com ele.
    let egl = unsafe { EGLDisplay::new(gbm)? };
    let context = EGLContext::new(&egl)?;
    // SAFETY: o contexto acabou de ser criado e só este renderizador o usa.
    let mut renderer = unsafe { GlesRenderer::new(context)? };

    let (w, h) = std::env::var("MIDE_WINIT_SIZE")
        .ok()
        .and_then(|v| v.split_once('x').and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?))))
        .unwrap_or((1366, 768));
    let output = Output::new(
        "mide-headless".into(),
        PhysicalProperties { size: (0, 0).into(), subpixel: Subpixel::Unknown, make: "mide".into(), model: "headless".into() },
    );
    let mode = Mode { size: (w, h).into(), refresh: 60_000 };
    output.create_global::<State>(&state.display_handle);
    // Como no DRM: o alvo não é invertido no eixo y.
    output.change_current_state(Some(mode), Some(Transform::Normal), None, Some((0, 0).into()));
    output.set_preferred(mode);
    crate::clip::set_flipped(false);
    state.space.map_output(&output, (0, 0));

    let mut scene = Scene::new(&mut renderer, state)?;
    let mut target = Offscreen::<smithay::backend::renderer::gles::GlesTexture>::create_buffer(
        &mut renderer,
        Fourcc::Abgr8888,
        Size::<i32, Buffer>::from((w, h)),
    )?;
    let mut damage_tracker = OutputDamageTracker::from_output(&output);
    let mut selftest = crate::selftest::SelfTest::from_env();
    tracing::info!("saída sem tela criada ({w}x{h}, {path})");

    loop_handle.insert_source(Timer::from_duration(Duration::from_millis(16)), move |_, _, state| {
        let wanted = selftest.is_some() || state.redraw || scene.moving(state);
        if wanted {
            if let Some(test) = selftest.as_mut() {
                test.tick(state);
            }
            state.redraw = false;
            let result = (|| -> Result<(), Box<dyn Error>> {
                let mut framebuffer = renderer.bind(&mut target)?;
                let _sync = scene.render(&mut renderer, &mut framebuffer, &output, &mut damage_tracker, state)?;
                Ok(())
            })();
            if let Err(err) = result {
                tracing::warn!("falha ao desenhar quadro: {err}");
            }
            if let Err(err) = send_frames(state, &output) {
                tracing::debug!("avisos de quadro: {err}");
            }
        }
        TimeoutAction::ToDuration(Duration::from_millis(16))
    })?;
    Ok(())
}
