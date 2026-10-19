//! Backend DRM: o mide como sessão de verdade, direto na tela, sem outro desktop por
//! baixo (iniciado de um console de texto).
//!
//! A sessão (libseat) dá acesso à placa de vídeo e aos dispositivos de entrada sem
//! root. A tela é ligada pelo DRM/KMS e desenhada pela GPU (GBM + EGL + GLES), com a
//! troca de quadros no ritmo do monitor (vblank). Mouse, teclado e touchpad vêm do
//! libinput. O que é desenhado vem da cena (`crate::render`), a mesma do backend winit.
//!
//! Limites desta primeira versão: uma placa de vídeo e uma tela (a primeira conectada,
//! no modo preferido), sem ligar e desligar monitores com o mide aberto.

use std::{
    cell::RefCell,
    error::Error,
    io,
    path::PathBuf,
    rc::Rc,
    time::{Duration, Instant},
};

use smithay::{
    backend::{
        allocator::{
            gbm::{GbmAllocator, GbmBufferFlags, GbmDevice},
            Fourcc,
        },
        drm::{DrmDevice, DrmDeviceFd, DrmError, DrmEvent, DrmDeviceNotifier, GbmBufferedSurface},
        egl::{EGLContext, EGLDisplay},
        input::InputEvent,
        libinput::{LibinputInputBackend, LibinputSessionInterface},
        renderer::{damage::OutputDamageTracker, gles::GlesRenderer, Bind},
        session::{
            libseat::{LibSeatSession, LibSeatSessionNotifier},
            Event as SessionEvent, Session,
        },
        udev::{all_gpus, primary_gpu},
    },
    output::{Mode, Output, PhysicalProperties, Subpixel},
    reexports::{
        calloop::{Dispatcher, EventLoop, LoopHandle},
        drm::control::{connector, crtc, Device as ControlDevice, ModeTypeFlags},
        input::Libinput,
        rustix::fs::OFlags,
    },
    utils::{DeviceFd, Rectangle, Size, Transform},
};

use crate::{
    render::{send_frames, Scene},
    state::State,
};

/// Placa de vídeo e tela em uso, com o que é preciso para desenhar.
struct Gpu {
    drm: DrmDevice,
    renderer: GlesRenderer,
    surface: GbmBufferedSurface<GbmAllocator<DrmDeviceFd>, ()>,
    output: Output,
    damage_tracker: OutputDamageTracker,
    scene: Scene,
    /// Um quadro foi entregue e espera o vblank: o próximo só depois dele.
    pending: bool,
}

/// Abre a sessão, a placa de vídeo, a tela e a entrada, e agenda o desenho.
pub fn init(event_loop: &EventLoop<State>, loop_handle: &LoopHandle<State>, state: &mut State) -> Result<(), Box<dyn Error>> {
    let _ = event_loop;
    let (mut session, notifier) =
        LibSeatSession::new().map_err(|e| format!("sem sessão (rode o mide de um console de texto, logado): {e}"))?;
    let seat = session.seat();
    tracing::info!("sessão aberta no assento {seat}");
    let notifier = wait_active(&session, notifier)?;

    // Placa de vídeo: a principal do assento, ou a primeira encontrada.
    let gpu_path: PathBuf = primary_gpu(&seat)?
        .or_else(|| all_gpus(&seat).ok()?.into_iter().next())
        .ok_or("nenhuma placa de vídeo encontrada")?;
    tracing::info!("placa de vídeo: {}", gpu_path.display());
    let fd = session
        .open(&gpu_path, OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK)
        .map_err(|e| format!("não foi possível abrir {}: {e}", gpu_path.display()))?;
    let fd = DrmDeviceFd::new(DeviceFd::from(fd));
    let (mut drm, drm_notifier) = open_drm(&fd)?;
    let gbm = GbmDevice::new(fd.clone())?;

    // GPU: EGL sobre o GBM e o renderizador GLES (o mesmo tipo do winit, então a cena é igual).
    // SAFETY: o display EGL guarda uma cópia do dispositivo GBM, que fica vivo com ele.
    let egl = unsafe { EGLDisplay::new(gbm.clone())? };
    let context = EGLContext::new(&egl)?;
    let formats = context.dmabuf_render_formats().clone();
    // SAFETY: o contexto acabou de ser criado e só este renderizador o usa.
    let mut renderer = unsafe { GlesRenderer::new(context)? };

    // Tela: o primeiro conector ligado a um monitor, no modo preferido.
    let (connector, crtc, drm_mode, name, mm) = pick_connector(&fd)?;
    let (w, h) = drm_mode.size();
    tracing::info!("tela {name}: {w}x{h} a {} Hz", drm_mode.vrefresh());
    let surface = drm.create_surface(crtc, drm_mode, &[connector])?;
    let allocator = GbmAllocator::new(gbm.clone(), GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT);
    // Formato opaco primeiro: a tela não usa o canal alfa.
    let surface = GbmBufferedSurface::new(surface, allocator, &[Fourcc::Xrgb8888, Fourcc::Argb8888], formats.iter().copied())
        .map_err(|e| format!("não foi possível preparar a tela {name}: {e}"))?;

    let output = Output::new(
        name.clone(),
        PhysicalProperties { size: (mm.0 as i32, mm.1 as i32).into(), subpixel: Subpixel::Unknown, make: "mide".into(), model: name },
    );
    let mode = Mode { size: (w as i32, h as i32).into(), refresh: drm_mode.vrefresh() as i32 * 1000 };
    output.create_global::<State>(&state.display_handle);
    output.change_current_state(Some(mode), Some(Transform::Normal), None, Some((0, 0).into()));
    // Sem inversão no eixo y (o winit inverte): recortes e vidro seguem isso.
    crate::clip::set_flipped(false);
    output.set_preferred(mode);
    state.space.map_output(&output, (0, 0));
    // O ponteiro começa no meio da tela.
    state.pointer_location = (w as f64 / 2.0, h as f64 / 2.0).into();

    let scene = Scene::new(&mut renderer, state)?.with_cursor();
    let damage_tracker = OutputDamageTracker::from_output(&output);
    let gpu = Rc::new(RefCell::new(Gpu { drm, renderer, surface, output: output.clone(), damage_tracker, scene, pending: false }));
    state.session = Some(session.clone());

    // Vblank: o quadro entregue foi para a tela; desenha o próximo se algo mudou.
    {
        let gpu = gpu.clone();
        loop_handle.insert_source(drm_notifier, move |event, _, state| match event {
            DrmEvent::VBlank(_) => {
                let mut gpu = gpu.borrow_mut();
                if let Err(err) = gpu.surface.frame_submitted() {
                    tracing::warn!("vblank: {err}");
                }
                gpu.pending = false;
                if state.redraw || gpu.scene.moving(state) {
                    render(&mut gpu, state);
                }
            }
            DrmEvent::Error(err) => tracing::warn!("erro do DRM: {err}"),
        })?;
    }

    // Algo mudou (`state.wake`): desenha agora, se não houver quadro esperando o vblank.
    {
        let (ping, source) = smithay::reexports::calloop::ping::make_ping()?;
        let gpu = gpu.clone();
        loop_handle.insert_source(source, move |_, _, state: &mut State| {
            if state.redraw {
                if let Ok(mut gpu) = gpu.try_borrow_mut() {
                    render(&mut gpu, state);
                }
            }
        })?;
        state.waker = Some(ping);
    }

    // Entrada: libinput pela sessão (sem root).
    let mut libinput = Libinput::new_with_udev(LibinputSessionInterface::<LibSeatSession>::from(session.clone()));
    libinput.udev_assign_seat(&seat).map_err(|()| "libinput: não foi possível usar o assento")?;
    {
        let output = output.clone();
        loop_handle.insert_source(LibinputInputBackend::new(libinput.clone()), move |event, _, state| {
            if let InputEvent::DeviceAdded { mut device } = event {
                // Touchpad: toque para clicar, como nos outros desktops.
                if device.config_tap_finger_count() > 0 {
                    let _ = device.config_tap_set_enabled(true);
                }
                tracing::info!("dispositivo de entrada: {}", device.name());
                return;
            }
            let size = output.current_mode().map(|m| m.size.to_logical(1)).unwrap_or(Size::from((w as i32, h as i32)));
            crate::input::handle_event(state, event, size);
            state.wake();
        })?;
    }

    // Troca de console: ao sair, a tela e a entrada são liberadas; ao voltar, retomadas.
    {
        let gpu = gpu.clone();
        let mut libinput = libinput.clone();
        loop_handle.insert_source(notifier, move |event, _, state| match event {
            SessionEvent::PauseSession => {
                tracing::info!("sessão pausada (outro console)");
                libinput.suspend();
                gpu.borrow_mut().drm.pause();
            }
            SessionEvent::ActivateSession => {
                tracing::info!("sessão de volta");
                if libinput.resume().is_err() {
                    tracing::warn!("libinput não voltou");
                }
                let mut gpu = gpu.borrow_mut();
                if let Err(err) = gpu.drm.activate(false) {
                    tracing::warn!("a tela não voltou: {err}");
                }
                gpu.surface.reset_buffers();
                gpu.pending = false;
                state.redraw = true;
                render(&mut gpu, state);
            }
        })?;
    }

    // Primeiro quadro.
    state.redraw = true;
    render(&mut gpu.borrow_mut(), state);
    tracing::info!("saída DRM criada ({w}x{h})");
    Ok(())
}

/// Vindo da tela de login, a sessão pode ainda não estar ativa (o console é trocado
/// logo depois): sem isso a tela não é nossa. Espera ela ficar ativa, até 10 s.
fn wait_active(session: &LibSeatSession, notifier: LibSeatSessionNotifier) -> Result<LibSeatSessionNotifier, Box<dyn Error>> {
    if session.is_active() {
        return Ok(notifier);
    }
    tracing::info!("esperando a sessão ficar ativa");
    let mut waiting: EventLoop<()> = EventLoop::try_new()?;
    let dispatcher = Dispatcher::new(notifier, |_, _, _: &mut ()| {});
    let token = waiting.handle().register_dispatcher(dispatcher.clone())?;
    let deadline = Instant::now() + Duration::from_secs(10);
    while !session.is_active() && Instant::now() < deadline {
        waiting.dispatch(Some(Duration::from_millis(100)), &mut ())?;
    }
    waiting.handle().remove(token);
    drop(waiting);
    if !session.is_active() {
        return Err("a sessão não ficou ativa em 10 s (outro programa está com a tela?)".into());
    }
    tracing::info!("sessão ativa");
    Ok(dispatcher.into_source_inner())
}

/// Assume a placa de vídeo. Quem estava com a tela antes (o servidor X da tela de
/// login) pode demorar um instante para soltá-la: tenta de novo por até 3 s.
fn open_drm(fd: &DrmDeviceFd) -> Result<(DrmDevice, DrmDeviceNotifier), Box<dyn Error>> {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match DrmDevice::new(fd.clone(), true) {
            Err(DrmError::Access(err)) if err.source.kind() == io::ErrorKind::PermissionDenied && Instant::now() < deadline => {
                tracing::info!("a placa de vídeo ainda está ocupada, tentando de novo");
                std::thread::sleep(Duration::from_millis(100));
            }
            result => return Ok(result?),
        }
    }
}

/// Desenha um quadro e o entrega à tela (aparece no próximo vblank).
fn render(gpu: &mut Gpu, state: &mut State) {
    if gpu.pending || !gpu.drm.is_active() {
        return;
    }
    state.redraw = false;
    let Gpu { renderer, surface, output, damage_tracker, scene, .. } = gpu;
    let result = (|| -> Result<(), Box<dyn Error>> {
        let (mut dmabuf, _age) = surface.next_buffer()?;
        let sync = {
            let mut target = renderer.bind(&mut dmabuf)?;
            scene.render(renderer, &mut target, output, damage_tracker, state)?
        };
        // A tela inteira como área alterada: sem isso, telas de notebook com PSR2
        // (Intel) atualizam só uma faixa e o resto fica com o quadro antigo.
        let size = output.current_mode().map(|m| m.size).unwrap_or_default();
        surface.queue_buffer(Some(sync), Some(vec![Rectangle::from_size(size)]), ())?;
        Ok(())
    })();
    match result {
        Ok(()) => {
            gpu.pending = true;
            // Algo em movimento: o próximo vblank desenha de novo.
            if gpu.scene.moving(state) {
                state.redraw = true;
            }
        }
        Err(err) => {
            tracing::warn!("falha ao desenhar quadro: {err}");
            state.redraw = true;
        }
    }
    if let Err(err) = send_frames(state, &gpu.output) {
        tracing::debug!("avisos de quadro: {err}");
    }
}

/// Primeiro conector com monitor, o modo preferido dele, uma CRTC livre que o
/// alimenta, o nome (`eDP-1`, `HDMI-A-1`...) e o tamanho físico em mm.
fn pick_connector(
    fd: &DrmDeviceFd,
) -> Result<(connector::Handle, crtc::Handle, smithay::reexports::drm::control::Mode, String, (u32, u32)), Box<dyn Error>> {
    let resources = fd.resource_handles()?;
    for handle in resources.connectors() {
        let info = fd.get_connector(*handle, true)?;
        if info.state() != connector::State::Connected {
            continue;
        }
        let Some(mode) = info
            .modes()
            .iter()
            .find(|m| m.mode_type().contains(ModeTypeFlags::PREFERRED))
            .or_else(|| info.modes().first())
            .copied()
        else {
            continue;
        };
        let crtc = info
            .encoders()
            .iter()
            .filter_map(|e| fd.get_encoder(*e).ok())
            .flat_map(|e| resources.filter_crtcs(e.possible_crtcs()))
            .next();
        let Some(crtc) = crtc else {
            continue;
        };
        let name = format!("{}-{}", info.interface().as_str(), info.interface_id());
        return Ok((*handle, crtc, mode, name, info.size().unwrap_or((0, 0))));
    }
    Err("nenhum monitor conectado".into())
}
