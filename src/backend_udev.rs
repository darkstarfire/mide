//! Backend DRM: o mide como sessão de verdade, direto na tela, sem outro desktop por
//! baixo (iniciado de um console de texto ou da tela de login).
//!
//! A sessão (libseat) dá acesso à placa de vídeo e aos dispositivos de entrada sem
//! root. Cada tela conectada é ligada pelo DRM/KMS e desenhada pela GPU (GBM + EGL +
//! GLES), com a troca de quadros no ritmo dela (vblank). Mouse, teclado e touchpad vêm
//! do libinput. O que é desenhado vem da cena (`crate::render`), a mesma do winit.
//!
//! Várias telas: cada conector com monitor vira uma saída, no modo escolhido nas
//! Configurações (`[monitors]`) ou no preferido, posicionada pelo `crate::outputs`.
//! Conectar ou desconectar um cabo refaz as telas (udev), e `outputs apply` no canal
//! de controle também (as Configurações usam).
//!
//! Limites: uma placa de vídeo.

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
        drm::{DrmDevice, DrmDeviceFd, DrmDeviceNotifier, DrmError, DrmEvent, GbmBufferedSurface},
        egl::{EGLContext, EGLDisplay},
        input::InputEvent,
        libinput::{LibinputInputBackend, LibinputSessionInterface},
        renderer::{damage::OutputDamageTracker, gles::GlesRenderer, Bind},
        session::{
            libseat::{LibSeatSession, LibSeatSessionNotifier},
            Event as SessionEvent, Session,
        },
        udev::{all_gpus, primary_gpu, UdevBackend, UdevEvent},
    },
    output::{Mode, Output, PhysicalProperties, Scale, Subpixel},
    reexports::{
        calloop::{Dispatcher, EventLoop, LoopHandle},
        drm::control::{connector, crtc, Device as ControlDevice, ModeTypeFlags},
        input::Libinput,
        rustix::fs::OFlags,
    },
    utils::{DeviceFd, Rectangle, Transform},
};

use crate::{
    render::{send_frames, Scene},
    state::{OutputInfo, State},
};

type Surface = GbmBufferedSurface<GbmAllocator<DrmDeviceFd>, ()>;
type DrmMode = smithay::reexports::drm::control::Mode;

/// Uma tela ligada.
struct Head {
    connector: connector::Handle,
    crtc: crtc::Handle,
    surface: Surface,
    output: Output,
    damage_tracker: OutputDamageTracker,
    /// Um quadro foi entregue e espera o vblank: o próximo só depois dele.
    pending: bool,
}

/// Placa de vídeo, telas e o que é preciso para desenhar.
struct Gpu {
    drm: DrmDevice,
    fd: DrmDeviceFd,
    gbm: GbmDevice<DrmDeviceFd>,
    renderer: GlesRenderer,
    formats: Vec<smithay::backend::allocator::Format>,
    heads: Vec<Head>,
    scene: Scene,
}

/// Abre a sessão, a placa de vídeo, as telas e a entrada, e agenda o desenho.
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
    let (drm, drm_notifier) = open_drm(&fd)?;
    let gbm = GbmDevice::new(fd.clone())?;

    // GPU: EGL sobre o GBM e o renderizador GLES (o mesmo tipo do winit, então a cena é igual).
    // SAFETY: o display EGL guarda uma cópia do dispositivo GBM, que fica vivo com ele.
    let egl = unsafe { EGLDisplay::new(gbm.clone())? };
    let context = EGLContext::new(&egl)?;
    let formats = context.dmabuf_render_formats().iter().copied().collect();
    // SAFETY: o contexto acabou de ser criado e só este renderizador o usa.
    let mut renderer = unsafe { GlesRenderer::new(context)? };
    // Sem inversão no eixo y (o winit inverte): recortes e vidro seguem isso.
    crate::clip::set_flipped(false);

    let scene = Scene::new(&mut renderer, state)?.with_cursor();
    let gpu = Rc::new(RefCell::new(Gpu { drm, fd, gbm, renderer, formats, heads: Vec::new(), scene }));
    state.session = Some(session.clone());

    // Telas: liga as conectadas e arranja. O ponteiro começa no meio da principal.
    refresh_heads(&mut gpu.borrow_mut(), state);
    if gpu.borrow().heads.is_empty() {
        return Err("nenhum monitor conectado".into());
    }
    if let Some(area) = state.primary_output().and_then(|o| state.space.output_geometry(&o)) {
        state.pointer_location =
            (area.loc.x as f64 + area.size.w as f64 / 2.0, area.loc.y as f64 + area.size.h as f64 / 2.0).into();
    }

    // Vblank: o quadro entregue foi para a tela; desenha o próximo se algo mudou.
    {
        let gpu = gpu.clone();
        loop_handle.insert_source(drm_notifier, move |event, _, state| match event {
            DrmEvent::VBlank(crtc) => {
                let mut gpu = gpu.borrow_mut();
                if let Some(head) = gpu.heads.iter_mut().find(|h| h.crtc == crtc) {
                    if let Err(err) = head.surface.frame_submitted() {
                        tracing::warn!("vblank: {err}");
                    }
                    head.pending = false;
                }
                if state.redraw || gpu.scene.moving(state) {
                    render(&mut gpu, state);
                }
            }
            DrmEvent::Error(err) => tracing::warn!("erro do DRM: {err}"),
        })?;
    }

    // Algo mudou (`state.wake`): desenha agora as telas que não esperam o vblank.
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

    // As Configurações mudaram as telas (`outputs apply`): refaz pela configuração.
    {
        let (ping, source) = smithay::reexports::calloop::ping::make_ping()?;
        let gpu = gpu.clone();
        loop_handle.insert_source(source, move |_, _, state: &mut State| {
            let mut gpu = gpu.borrow_mut();
            refresh_heads(&mut gpu, state);
            render(&mut gpu, state);
        })?;
        state.outputs_waker = Some(ping);
    }

    // Cabo conectado ou desconectado: a placa avisa pelo udev e as telas são refeitas.
    match UdevBackend::new(&seat) {
        Ok(udev) => {
            let gpu = gpu.clone();
            loop_handle.insert_source(udev, move |event, _, state| {
                if let UdevEvent::Changed { .. } = event {
                    tracing::info!("telas mudaram (cabo conectado ou desconectado)");
                    let mut gpu = gpu.borrow_mut();
                    refresh_heads(&mut gpu, state);
                    render(&mut gpu, state);
                }
            })?;
        }
        Err(err) => tracing::warn!("sem aviso de monitor conectado ou desconectado: {err}"),
    }

    // Entrada: libinput pela sessão (sem root).
    let mut libinput = Libinput::new_with_udev(LibinputSessionInterface::<LibSeatSession>::from(session.clone()));
    libinput.udev_assign_seat(&seat).map_err(|()| "libinput: não foi possível usar o assento")?;
    // Mouses e trackpads conectados, para reaplicar `[input]` quando muda.
    let devices: Rc<RefCell<Vec<smithay::reexports::input::Device>>> = Default::default();
    {
        let (ping, source) = smithay::reexports::calloop::ping::make_ping()?;
        let devices = devices.clone();
        loop_handle.insert_source(source, move |_, _, state: &mut State| {
            for device in devices.borrow_mut().iter_mut() {
                apply_input_config(device, &state.config.input);
            }
            tracing::info!("mouse e trackpad: configuração aplicada");
        })?;
        state.input_waker = Some(ping);
    }
    let input_devices = devices.clone();
    loop_handle.insert_source(LibinputInputBackend::new(libinput.clone()), move |event, _, state| {
        if let InputEvent::DeviceAdded { mut device } = event {
            apply_input_config(&mut device, &state.config.input);
            tracing::info!("dispositivo de entrada: {}", device.name());
            input_devices.borrow_mut().push(device);
            return;
        }
        if let InputEvent::DeviceRemoved { device } = &event {
            input_devices.borrow_mut().retain(|d| d != device);
        }
        // O ponteiro anda por todas as telas: a caixa que contém todas.
        let bounds = state
            .space
            .outputs()
            .filter_map(|o| state.space.output_geometry(o))
            .reduce(|a, b| a.merge(b))
            .unwrap_or_else(|| Rectangle::from_size((1366, 768).into()));
        crate::input::handle_event_in(state, event, bounds);
        state.wake();
    })?;

    // Troca de console: ao sair, as telas e a entrada são liberadas; ao voltar, retomadas.
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
                for head in &mut gpu.heads {
                    head.surface.reset_buffers();
                    head.pending = false;
                }
                // Um monitor pode ter sido trocado enquanto o mide estava em outro console.
                refresh_heads(&mut gpu, state);
                state.redraw = true;
                render(&mut gpu, state);
            }
        })?;
    }

    // Primeiro quadro.
    state.redraw = true;
    render(&mut gpu.borrow_mut(), state);
    tracing::info!("saída DRM criada ({} tela(s))", gpu.borrow().heads.len());
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

/// Monitor conectado num conector: nome, modos, modo preferido e tamanho em mm.
struct Connected {
    handle: connector::Handle,
    name: String,
    modes: Vec<DrmMode>,
    preferred: DrmMode,
    mm: (u32, u32),
    encoders: Vec<smithay::reexports::drm::control::encoder::Handle>,
}

/// Conectores com monitor, na ordem da placa.
fn connected(fd: &DrmDeviceFd) -> Vec<Connected> {
    let Ok(resources) = fd.resource_handles() else {
        return Vec::new();
    };
    resources
        .connectors()
        .iter()
        .filter_map(|handle| {
            let info = fd.get_connector(*handle, true).ok()?;
            if info.state() != connector::State::Connected || info.modes().is_empty() {
                return None;
            }
            let preferred = info
                .modes()
                .iter()
                .find(|m| m.mode_type().contains(ModeTypeFlags::PREFERRED))
                .or_else(|| info.modes().first())
                .copied()?;
            Some(Connected {
                handle: *handle,
                name: format!("{}-{}", info.interface().as_str(), info.interface_id()),
                modes: info.modes().to_vec(),
                preferred,
                mm: info.size().unwrap_or((0, 0)),
                encoders: info.encoders().to_vec(),
            })
        })
        .collect()
}

/// Modo pedido nas Configurações (`1920x1080@60`), ou o preferido da tela. A
/// frequência é comparada em Hz arredondados (59,94 conta como 60).
fn choose_mode(c: &Connected, state: &State) -> DrmMode {
    let wanted = state.config.monitors.outputs.get(&c.name).and_then(|m| m.parsed_mode());
    let Some((w, h, hz)) = wanted else {
        return c.preferred;
    };
    let same_size: Vec<&DrmMode> = c.modes.iter().filter(|m| m.size() == (w as u16, h as u16)).collect();
    let fastest = same_size.iter().max_by_key(|m| m.vrefresh()).copied();
    let pick = match hz {
        Some(hz) => same_size.iter().find(|m| (m.vrefresh() as f64 - hz).abs() < 1.0).copied().or(fastest),
        None => fastest,
    };
    match pick {
        Some(mode) => *mode,
        None => {
            tracing::warn!("tela {}: modo {w}x{h} não existe, usando o preferido", c.name);
            c.preferred
        }
    }
}

/// Refaz as telas pela placa e pela configuração: liga as conectadas e ligadas,
/// desliga as que sumiram ou foram desligadas, troca o modo das que mudaram, arranja
/// as posições e atualiza a lista das Configurações.
fn refresh_heads(gpu: &mut Gpu, state: &mut State) {
    let found = connected(&gpu.fd);
    let monitors = state.config.monitors.clone();
    let enabled = |name: &str| monitors.outputs.get(name).is_none_or(|m| m.enabled);
    // Nunca desliga todas: se nenhuma ficaria ligada, a primeira fica.
    let any_enabled = found.iter().any(|c| enabled(&c.name));
    let keep: Vec<connector::Handle> = found
        .iter()
        .enumerate()
        .filter(|(i, c)| enabled(&c.name) || (!any_enabled && *i == 0))
        .map(|(_, c)| c.handle)
        .collect();

    // Telas que sumiram ou foram desligadas.
    gpu.heads.retain(|head| {
        let stays = keep.contains(&head.connector);
        if !stays {
            tracing::info!("tela {} desligada", head.output.name());
        }
        stays
    });

    for c in found.iter().filter(|c| keep.contains(&c.handle)) {
        let mode = choose_mode(c, state);
        let scale = monitors.outputs.get(&c.name).map(|m| m.scale).unwrap_or(1.0);
        if let Some(head) = gpu.heads.iter_mut().find(|h| h.connector == c.handle) {
            // Já ligada: troca o modo se mudou (o modo em uso é o da saída; o da
            // superfície GBM pode vir zerado antes do primeiro quadro).
            let in_use = head.output.current_mode().map(|m| (m.size.w, m.size.h, m.refresh));
            let wanted = (mode.size().0 as i32, mode.size().1 as i32, mode.vrefresh() as i32 * 1000);
            if in_use != Some(wanted) {
                match head.surface.use_mode(mode) {
                    Ok(()) => {
                        tracing::info!("tela {}: {}x{} a {} Hz", c.name, mode.size().0, mode.size().1, mode.vrefresh());
                        head.damage_tracker = OutputDamageTracker::from_output(&head.output);
                        head.pending = false;
                    }
                    Err(err) => {
                        // Fica no modo de antes (só a escala pode mudar).
                        tracing::warn!("tela {}: modo não aplicado: {err}", c.name);
                        head.output.change_current_state(None, None, Some(output_scale(scale)), None);
                        continue;
                    }
                }
            }
            set_output_mode(&head.output, mode, scale);
            continue;
        }
        match create_head(gpu, state, c, mode, scale) {
            Ok(head) => {
                tracing::info!("tela {}: {}x{} a {} Hz", c.name, mode.size().0, mode.size().1, mode.vrefresh());
                gpu.heads.push(head);
            }
            Err(err) => tracing::warn!("tela {} não ligada: {err}", c.name),
        }
    }

    let live: Vec<Output> = gpu.heads.iter().map(|h| h.output.clone()).collect();
    crate::outputs::place(state, &live);

    // Lista para as Configurações: todas as conectadas, ligadas ou não.
    let primary = state.primary_output().map(|o| o.name());
    state.outputs_info = found
        .iter()
        .map(|c| {
            let head = gpu.heads.iter().find(|h| h.connector == c.handle);
            let preferred = (c.preferred.size().0 as i32, c.preferred.size().1 as i32, c.preferred.vrefresh() as i32 * 1000);
            let current = head
                .and_then(|h| h.output.current_mode())
                .map(|m| (m.size.w, m.size.h, m.refresh))
                .unwrap_or(preferred);
            let mut modes: Vec<(i32, i32, i32)> =
                c.modes.iter().map(|m| (m.size().0 as i32, m.size().1 as i32, m.vrefresh() as i32 * 1000)).collect();
            modes.dedup();
            OutputInfo {
                name: c.name.clone(),
                description: format!("{}x{} mm", c.mm.0, c.mm.1),
                modes,
                current,
                position: head.and_then(|h| state.space.output_geometry(&h.output)).map(|g| (g.loc.x, g.loc.y)).unwrap_or((0, 0)),
                scale: monitors.outputs.get(&c.name).map(|m| m.scale).unwrap_or(1.0),
                enabled: head.is_some(),
                primary: primary.as_deref() == Some(c.name.as_str()),
            }
        })
        .collect();
    state.redraw = true;
}

/// Liga uma tela: CRTC livre, superfície GBM e a saída Wayland.
fn create_head(gpu: &mut Gpu, state: &mut State, c: &Connected, mode: DrmMode, scale: f64) -> Result<Head, Box<dyn Error>> {
    let resources = gpu.fd.resource_handles()?;
    let used: Vec<crtc::Handle> = gpu.heads.iter().map(|h| h.crtc).collect();
    let crtc = c
        .encoders
        .iter()
        .filter_map(|e| gpu.fd.get_encoder(*e).ok())
        .flat_map(|e| resources.filter_crtcs(e.possible_crtcs()))
        .find(|crtc| !used.contains(crtc))
        .ok_or("nenhuma CRTC livre")?;
    let surface = gpu.drm.create_surface(crtc, mode, &[c.handle])?;
    let allocator = GbmAllocator::new(gpu.gbm.clone(), GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT);
    // Formato opaco primeiro: a tela não usa o canal alfa.
    let surface = GbmBufferedSurface::new(surface, allocator, &[Fourcc::Xrgb8888, Fourcc::Argb8888], gpu.formats.iter().copied())
        .map_err(|e| format!("não foi possível preparar a tela {}: {e}", c.name))?;
    let output = Output::new(
        c.name.clone(),
        PhysicalProperties {
            size: (c.mm.0 as i32, c.mm.1 as i32).into(),
            subpixel: Subpixel::Unknown,
            make: "mide".into(),
            model: c.name.clone(),
        },
    );
    output.create_global::<State>(&state.display_handle);
    set_output_mode(&output, mode, scale);
    let damage_tracker = OutputDamageTracker::from_output(&output);
    Ok(Head { connector: c.handle, crtc, surface, output, damage_tracker, pending: false })
}

/// Modo e escala da saída Wayland (o que os apps veem).
fn set_output_mode(output: &Output, mode: DrmMode, scale: f64) {
    let (w, h) = mode.size();
    let wl_mode = Mode { size: (w as i32, h as i32).into(), refresh: mode.vrefresh() as i32 * 1000 };
    output.change_current_state(Some(wl_mode), Some(Transform::Normal), Some(output_scale(scale)), None);
    output.set_preferred(wl_mode);
}

/// Desenha um quadro em cada tela que não espera vblank e o entrega (aparece no
/// próximo vblank dela).
fn render(gpu: &mut Gpu, state: &mut State) {
    if !gpu.drm.is_active() {
        return;
    }
    let wanted = state.redraw;
    state.redraw = false;
    let mut again = false;
    let Gpu { renderer, heads, scene, .. } = gpu;
    for head in heads.iter_mut() {
        if head.pending {
            // Esta tela desenha no próximo vblank dela.
            again |= wanted;
            continue;
        }
        let Head { surface, output, damage_tracker, .. } = head;
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
            Ok(()) => head.pending = true,
            Err(err) => {
                tracing::warn!("falha ao desenhar a tela {}: {err}", head.output.name());
                again = true;
            }
        }
        if let Err(err) = send_frames(state, &head.output) {
            tracing::debug!("avisos de quadro: {err}");
        }
    }
    // Algo em movimento (ou uma tela que ficou para depois): o próximo vblank desenha.
    if again || gpu.scene.moving(state) {
        state.redraw = true;
    }
}

/// Escala da tela para o Wayland: inteira quando dá (1, 2), senão fracionária (1,25).
pub fn output_scale(scale: f64) -> Scale {
    let scale = if scale.is_finite() && scale >= 0.5 { scale } else { 1.0 };
    if (scale - scale.round()).abs() < 0.001 {
        Scale::Integer(scale.round() as i32)
    } else {
        Scale::Fractional(scale)
    }
}

/// Velocidade, aceleração, rolagem natural e toque para clicar (`[input]`) num mouse ou
/// trackpad. Trackpad é o que aceita toque (`tap`).
fn apply_input_config(device: &mut smithay::reexports::input::Device, input: &crate::config::Input) {
    use smithay::reexports::input::AccelProfile;
    let touchpad = device.config_tap_finger_count() > 0;
    if touchpad {
        let _ = device.config_tap_set_enabled(input.tap_to_click);
    }
    if device.config_accel_is_available() {
        let _ = device.config_accel_set_speed(input.pointer_speed.clamp(-1.0, 1.0));
        if !touchpad {
            let profile = if input.mouse_acceleration { AccelProfile::Adaptive } else { AccelProfile::Flat };
            let _ = device.config_accel_set_profile(profile);
        }
    }
    if device.config_scroll_has_natural_scroll() {
        let natural = if touchpad { input.touchpad_natural_scroll } else { input.mouse_natural_scroll };
        let _ = device.config_scroll_set_natural_scroll_enabled(natural);
    }
}
