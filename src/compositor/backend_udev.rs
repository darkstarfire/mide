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

/// Cada quadro entregue leva os pedidos de aviso de apresentação (`wp_presentation`)
/// das superfícies desenhadas nele, devolvidos no vblank.
type Surface = GbmBufferedSurface<GbmAllocator<DrmDeviceFd>, smithay::desktop::utils::OutputPresentationFeedback>;
type DrmMode = smithay::reexports::drm::control::Mode;

/// Uma tela ligada.
struct Head {
    connector: connector::Handle,
    crtc: crtc::Handle,
    surface: Surface,
    output: Output,
    /// O global `wl_output` desta tela (removido quando ela é desligada).
    global: smithay::reexports::wayland_server::backend::GlobalId,
    damage_tracker: OutputDamageTracker,
    /// Um quadro foi entregue e espera o vblank: o próximo só depois dele.
    pending: bool,
    /// Quando o quadro que espera o vblank foi entregue (para achar o vblank perdido).
    queued_at: Instant,
    /// Tela do próprio notebook (eDP/LVDS), onde o PSR2 pede a tela inteira como dano.
    internal: bool,
    /// Cursor no plano de cursor da placa: imagem enviada, nome dela e posição.
    cursor: HwCursor,
    /// Desde quando a placa não mostra mais o monitor conectado: a tela continua
    /// ligada (sem desenhar) pela folga de `outputs::UNPLUG_GRACE`, porque monitores
    /// derrubam o "conectado" por um instante ao acordar do DPMS.
    missing: Option<Instant>,
    /// Pedido de suspensão (`State::sleep`) cujo quadro com a tela de bloqueio foi
    /// entregue e espera o vblank (0: o quadro em voo não tem a tela de bloqueio).
    lock_queued: u64,
    /// Pedido de suspensão cujo quadro com a tela de bloqueio já está na tela.
    lock_shown: u64,
    /// Scanout direto do modo de jogo (A18): o buffer do app na placa.
    scanout: ScanoutHead,
    /// Feedback de scanout do linux-dmabuf mandado à janela em tela cheia desta tela.
    /// Fica fora do `ScanoutHead`, que é zerado a cada troca de modo: a superfície
    /// que recebeu o feedback de scanout precisa receber o padrão de volta.
    feedback: ScanoutFeedback,
}

/// Feedback do linux-dmabuf de uma tela no modo de jogo (A18).
#[derive(Default)]
struct ScanoutFeedback {
    /// Feedback com a tranche de scanout do plano principal desta tela (`Some(None)`:
    /// já calculado, sem formato em comum entre o plano e o renderizador).
    built: Option<Option<smithay::wayland::dmabuf::DmabufFeedback>>,
    /// Superfície que está com o feedback de scanout.
    surface: Option<smithay::reexports::wayland_server::protocol::wl_surface::WlSurface>,
}

/// Buffer de um app mandado direto ao plano principal (scanout direto, A18). Enquanto
/// existe, o app não recebe o `release` do buffer (a placa ainda o lê).
struct Scanned {
    _buffer: smithay::backend::renderer::utils::Buffer,
    _fb: Rc<smithay::backend::drm::gbm::GbmFramebuffer>,
    feedback: Option<smithay::desktop::utils::OutputPresentationFeedback>,
}

/// Estado do scanout direto de uma tela.
#[derive(Default)]
struct ScanoutHead {
    /// Trocado na placa, esperando o vblank.
    pending: Option<Scanned>,
    /// Na tela agora (solto quando outro quadro aparece).
    shown: Option<Scanned>,
    /// O último quadro entregue foi do app (o próximo composto redesenha tudo).
    active: bool,
    /// Framebuffers da placa já criados para os buffers do app (os jogos alternam 2 ou 3).
    fbs: Vec<(smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer, Rc<smithay::backend::drm::gbm::GbmFramebuffer>)>,
    /// Buffers que a placa recusou (não são testados de novo a cada quadro).
    refused: Vec<smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer>,
}

/// Cursor de hardware de uma tela (comandos de cursor do DRM): a imagem vai uma vez
/// e cada movimento só muda a posição, sem redesenhar a cena.
#[derive(Default)]
struct HwCursor {
    buffer: Option<smithay::reexports::gbm::BufferObject<()>>,
    /// Segundo buffer: a imagem nova é escrita nele e só então mostrada, nunca no que a
    /// placa está lendo (a animação do ponteiro agitado troca a imagem a cada quadro).
    spare: Option<smithay::reexports::gbm::BufferObject<()>>,
    /// Nome da imagem enviada (`None`: escondido).
    shown: Option<String>,
    position: Option<(i32, i32)>,
    /// O driver recusou: esta tela usa o cursor desenhado na cena.
    failed: bool,
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
    /// Telas apagadas (DPMS desligado).
    blanked: bool,
    /// Tamanho do plano de cursor da placa (64x64 na maioria).
    cursor_size: (u32, u32),
    /// Último motivo registrado do scanout direto em cada tela.
    scanout_notes: crate::scanout::Notes,
    /// `MIDE_NO_SCANOUT=1`: nunca manda o buffer do app direto à placa (diagnóstico).
    scanout_off: bool,
    /// Nó primário da placa (`/dev/dri/cardN`): o alvo da tranche de scanout.
    primary_dev: Option<libc::dev_t>,
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
    // Render node da placa (anunciado aos apps no linux-dmabuf).
    let node = crate::dmabuf::render_node(&fd);

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
    // Modo de jogo em tela cheia (A18): o scanout direto vale aqui?
    state.game.scanout_note = Some("conforme o buffer do app (ver \"scanout direto:\" a seguir)");
    let primary_dev = primary_node(&fd);
    let gpu = Rc::new(RefCell::new(Gpu { drm, fd, gbm, renderer, formats, heads: Vec::new(), scene, blanked: false, cursor_size: (64, 64), scanout_notes: Default::default(), scanout_off: std::env::var_os("MIDE_NO_SCANOUT").is_some(), primary_dev }));
    state.session = Some(session.clone());
    // Buffers dos apps na GPU (linux-dmabuf): importados neste renderizador. A janela em
    // tela cheia no modo de jogo recebe, só ela, a tranche de scanout (A18, `update_feedback`).
    {
        let node = node.or_else(|| crate::dmabuf::render_node_of_egl(gpu.borrow().renderer.egl_context().display()));
        let importer = gpu.clone();
        crate::dmabuf::setup(
            state,
            &gpu.borrow().renderer,
            node,
            crate::dmabuf::access(importer, |g: &mut Gpu| &mut g.renderer),
        );
    }

    {
        use smithay::reexports::drm::{Device as _, DriverCapability};
        let mut g = gpu.borrow_mut();
        let w = g.drm.get_driver_capability(DriverCapability::CursorWidth).unwrap_or(64) as u32;
        let h = g.drm.get_driver_capability(DriverCapability::CursorHeight).unwrap_or(64) as u32;
        g.cursor_size = (w.clamp(32, 512), h.clamp(32, 512));
        tracing::info!("plano de cursor: {}x{}", g.cursor_size.0, g.cursor_size.1);
    }

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
        loop_handle.insert_source(drm_notifier, move |event, meta, state| match event {
            DrmEvent::VBlank(crtc) => {
                crate::watchdog::phase("vblank");
                let mut gpu = gpu.borrow_mut();
                if let Some(head) = gpu.heads.iter_mut().find(|h| h.crtc == crtc) {
                    // Scanout direto (A18): o buffer do app está na tela; o anterior é
                    // solto (o app recebe o `release` dele).
                    if let Some(mut scanned) = head.scanout.pending.take() {
                        if let Some(feedback) = scanned.feedback.take() {
                            let (time, hw_clock) = crate::presentation::vblank_time(meta.as_ref());
                            let seq = meta.as_ref().map_or(0, |m| u64::from(m.sequence));
                            crate::presentation::present_vblank(feedback, &head.output, time, hw_clock, seq);
                        }
                        head.scanout.shown = Some(scanned);
                        head.lock_shown = head.lock_queued;
                    } else {
                    head.scanout.shown = None;
                    match head.surface.frame_submitted() {
                        // O quadro está na tela: os apps dele recebem a hora da troca
                        // de página (`wp_presentation`), a frequência e o número do vblank.
                        Ok(Some(feedback)) => {
                            let (time, hw_clock) = crate::presentation::vblank_time(meta.as_ref());
                            let seq = meta.as_ref().map_or(0, |m| u64::from(m.sequence));
                            crate::presentation::present_vblank(feedback, &head.output, time, hw_clock, seq);
                            head.lock_shown = head.lock_queued;
                        }
                        // Vblank de um quadro que o mide já tinha dado por perdido (a
                        // tela foi recomeçada): só registra.
                        Ok(None) => tracing::debug!("tela {}: vblank sem quadro esperando", head.output.name()),
                        Err(err) => tracing::warn!("tela {}: vblank: {err}", head.output.name()),
                    }
                    }
                    head.pending = false;
                }
                if state.redraw || gpu.scene.moving(state) {
                    render(&mut gpu, state);
                }
                check_sleep(&mut gpu, state);
            }
            DrmEvent::Error(err) => tracing::warn!("erro do DRM: {err}"),
        })?;
    }

    // Algo mudou (`state.wake`): desenha agora as telas que não esperam o vblank.
    {
        let (ping, source) = smithay::reexports::calloop::ping::make_ping()?;
        let gpu = gpu.clone();
        loop_handle.insert_source(source, move |_, _, state: &mut State| {
            crate::watchdog::phase("desenho pedido");
            if let Ok(mut gpu) = gpu.try_borrow_mut() {
                if state.redraw {
                    render(&mut gpu, state);
                }
                check_sleep(&mut gpu, state);
            }
        })?;
        state.waker = Some(ping);
    }

    // Rede de segurança, a cada meio segundo: (1) a volta da suspensão percebida pelo
    // relógio (o tempo suspenso conta no CLOCK_BOOTTIME e não no MONOTONIC), sem
    // depender do aviso do logind, que pode não chegar; (2) a tela que espera há mais
    // de 1 s o vblank de um quadro: o aviso se perdeu (suspensão, driver) e, sem isto,
    // ela nunca mais desenharia, congelada no último quadro.
    {
        use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
        const TICK: Duration = Duration::from_millis(500);
        let gpu = gpu.clone();
        let mut slept = suspended_time();
        loop_handle.insert_source(Timer::from_duration(TICK), move |_, _, state: &mut State| {
            crate::watchdog::phase("relógio das telas");
            let now_slept = suspended_time();
            if resumed_by_clock(slept, now_slept) {
                tracing::info!("volta da suspensão percebida pelo relógio ({:.0} s suspenso)", (now_slept - slept).as_secs_f32());
                state.sleep.wake();
                state.resumed = true;
                state.activity();
                state.redraw = true;
            }
            slept = now_slept;
            let Ok(mut gpu) = gpu.try_borrow_mut() else {
                return TimeoutAction::ToDuration(TICK);
            };
            if gpu.drm.is_active() && !gpu.blanked && state.sleep.frames_allowed() {
                for head in gpu.heads.iter_mut().filter(|h| h.pending && h.queued_at.elapsed() > VBLANK_LOST) {
                    tracing::warn!(
                        tela = %head.output.name(),
                        segundos = head.queued_at.elapsed().as_secs_f32(),
                        cursor_ampliado = state.shake.scale(Instant::now()) > 1.001,
                        "vblank não chegou depois de entregar o quadro; recomeçando a tela"
                    );
                    restart_head(head);
                    state.redraw = true;
                }
            }
            if state.redraw {
                render(&mut gpu, state);
            }
            check_sleep(&mut gpu, state);
            TimeoutAction::ToDuration(TICK)
        })?;
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
                    crate::watchdog::phase("udev (telas)");
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
    let input_gpu = gpu.clone();
    let input_loop = loop_handle.clone();
    let shake_clock: Rc<std::cell::Cell<bool>> = Default::default();
    loop_handle.insert_source(LibinputInputBackend::new(libinput.clone()), move |event, _, state| {
        crate::watchdog::phase("entrada");
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
        // Só o mouse andou (nada mais pediu quadro, o cursor não mudou de forma, nada
        // segue o ponteiro): com o cursor de hardware, basta mover o cursor da placa,
        // sem desenhar quadro nenhum.
        let motion = matches!(event, InputEvent::PointerMotion { .. } | InputEvent::PointerMotionAbsolute { .. });
        let before = (state.redraw, cursor_key(state));
        crate::input::handle_event_in(state, event, bounds);
        // O ícone de um arrasto entre apps é desenhado na cena junto do ponteiro: com
        // ele, todo movimento pede quadro (não basta mover o cursor da placa).
        let follows_pointer = state.preview.is_some()
            || state.dnd_icon.is_some()
            || state.seat.get_pointer().is_some_and(|p| p.is_grabbed())
            || over_title_bar(state)
            || state.screencopy.wants_cursor();
        if only_cursor_moved(motion, before.0 || state.redraw, follows_pointer, before.1 == cursor_key(state)) {
            let mut gpu = input_gpu.borrow_mut();
            let moved = !gpu.blanked && state.sleep.frames_allowed() && move_hw_cursors(&mut gpu, state);
            state.cursor_plane = moved;
            drop(gpu);
            if moved {
                start_shake_clock(&input_loop, &input_gpu, &shake_clock, state);
                return;
            }
        }
        start_shake_clock(&input_loop, &input_gpu, &shake_clock, state);
        state.wake();
    })?;

    // Troca de console: ao sair, as telas e a entrada são liberadas; ao voltar, retomadas.
    {
        let gpu = gpu.clone();
        let mut libinput = libinput.clone();
        loop_handle.insert_source(notifier, move |event, _, state| match event {
            SessionEvent::PauseSession => {
                crate::watchdog::phase("sessão pausada");
                tracing::info!("sessão pausada (outro console)");
                libinput.suspend();
                gpu.borrow_mut().drm.pause();
            }
            SessionEvent::ActivateSession => {
                crate::watchdog::phase("sessão de volta");
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
                    head.scanout = ScanoutHead::default();
                    // Outro programa usou a placa: o cursor de hardware vai de novo.
                    head.cursor = HwCursor::default();
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

    // Telas que sumiram ou foram desligadas. Desligada nas Configurações sai na hora;
    // monitor que a placa não mostra mais conectado espera a folga (ao acordar do
    // DPMS, HDMI e DP somem por ~1 s e voltam: antes isso levava as janelas da tela
    // para a principal a cada vez que as telas apagavam).
    crate::outputs::sweep_globals(state);
    let mut retired = Vec::new();
    let mut recheck: Option<Duration> = None;
    let now = Instant::now();
    let blanked = gpu.blanked;
    gpu.heads.retain_mut(|head| {
        let present = found.iter().any(|c| c.handle == head.connector);
        if present && !keep.contains(&head.connector) {
            tracing::info!("tela {} desligada", head.output.name());
            retired.push(head.global.clone());
            return false;
        }
        match crate::outputs::presence(&mut head.missing, present, blanked, now) {
            crate::outputs::Presence::Here => true,
            crate::outputs::Presence::Back => {
                tracing::info!("monitor {} voltou: a tela continua como estava", head.output.name());
                // O monitor pode ter perdido o modo e o cursor: refaz tudo no próximo quadro.
                if let Err(err) = head.surface.surface().reset_state() {
                    tracing::warn!("monitor {}: estado não reavaliado: {err}", head.output.name());
                }
                head.surface.reset_buffers();
                head.pending = false;
                head.cursor = HwCursor::default();
                true
            }
            crate::outputs::Presence::Wait(after) => {
                tracing::info!("monitor {} sumiu: esperando {:.1} s antes de tirar a tela", head.output.name(), after.as_secs_f32());
                recheck = Some(recheck.map_or(after, |r| r.min(after)));
                true
            }
            crate::outputs::Presence::Gone => {
                tracing::info!("tela {} desligada (monitor desconectado)", head.output.name());
                retired.push(head.global.clone());
                false
            }
        }
    });
    if let Some(after) = recheck {
        crate::outputs::recheck_later(state, after);
    }
    for id in retired {
        crate::outputs::retire_global(state, id);
    }

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
    let global = output.create_global::<State>(&state.display_handle);
    set_output_mode(&output, mode, scale);
    let damage_tracker = OutputDamageTracker::from_output(&output);
    // Tela do próprio notebook (onde fica o PSR2): eDP, LVDS ou DSI.
    let internal = ["eDP", "LVDS", "DSI"].iter().any(|t| c.name.starts_with(t));
    Ok(Head { connector: c.handle, crtc, surface, output, global, damage_tracker, pending: false, queued_at: Instant::now(), internal, cursor: HwCursor::default(), missing: None, lock_queued: 0, lock_shown: 0, scanout: ScanoutHead::default(), feedback: ScanoutFeedback::default() })
}

/// Modo e escala da saída Wayland (o que os apps veem).
fn set_output_mode(output: &Output, mode: DrmMode, scale: f64) {
    let (w, h) = mode.size();
    let wl_mode = Mode { size: (w as i32, h as i32).into(), refresh: mode.vrefresh() as i32 * 1000 };
    output.change_current_state(Some(wl_mode), Some(Transform::Normal), Some(output_scale(scale)), None);
    output.set_preferred(wl_mode);
}

/// O ponteiro está sobre a barra de título de uma janela (os botões dela acendem com
/// o mouse em cima: precisa de quadro). Com folga, para não perder a saída.
fn over_title_bar(state: &State) -> bool {
    let p = state.pointer_location;
    state.space.elements().any(|w| {
        let bar_h = crate::state::bar_height_for(w);
        let Some(loc) = state.space.element_location(w) else {
            return false;
        };
        let size = w.geometry().size;
        bar_h > 0 && p.x >= (loc.x - 8) as f64 && p.x <= (loc.x + size.w + 8) as f64 && p.y >= (loc.y - bar_h - 8) as f64 && p.y <= (loc.y + 8) as f64
    })
}

/// O evento só mexeu o cursor (basta mover o cursor da placa, sem quadro novo)? Só
/// quando foi movimento do mouse, ninguém pediu quadro, nada segue o ponteiro na cena
/// (ícone de arrasto, janela sendo movida, prévia, gravação) e o cursor não mudou de forma.
fn only_cursor_moved(motion: bool, redraw: bool, follows_pointer: bool, same_cursor: bool) -> bool {
    motion && !redraw && !follows_pointer && same_cursor
}

/// Forma do cursor agora (para saber se ela mudou com o movimento).
fn cursor_key(state: &State) -> String {
    format!("{:?} {:?}", state.cursor, state.cursor_status)
}

/// Intervalo do relógio do ponteiro agitado (~120 trocas de imagem por segundo no
/// máximo; a imagem só vai à placa quando o tamanho em pixels muda).
const SHAKE_TICK: Duration = Duration::from_millis(8);

/// Anima o ponteiro agitado no plano de cursor da placa: um relógio troca a imagem do
/// cursor enquanto a mola anda, sem a cena desenhar quadro nenhum. Antes, a animação
/// compunha a tela inteira em GLES a cada vblank e travava a tela em placas Intel
/// (Chromebook). Se o plano não serve (imagem grande demais, driver recusou), a cena
/// volta a desenhar o cursor, como antes. Um relógio só por vez (`running`).
fn start_shake_clock(handle: &LoopHandle<'_, State>, gpu: &Rc<RefCell<Gpu>>, running: &Rc<std::cell::Cell<bool>>, state: &State) {
    use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
    if running.get() || !state.shake.animating(Instant::now()) {
        return;
    }
    running.set(true);
    let (gpu, flag) = (gpu.clone(), running.clone());
    let inserted = handle.insert_source(Timer::from_duration(SHAKE_TICK), move |_, _, state: &mut State| {
        crate::watchdog::phase("cursor agitado");
        let animating = state.shake.animating(Instant::now());
        let Ok(mut gpu) = gpu.try_borrow_mut() else {
            return TimeoutAction::ToDuration(SHAKE_TICK);
        };
        // Tela apagada ou suspendendo: o próximo quadro de verdade acerta o cursor.
        let moved = !gpu.blanked && state.sleep.frames_allowed() && move_hw_cursors(&mut gpu, state);
        drop(gpu);
        state.cursor_plane = moved;
        if !moved {
            // A cena assume (o `moving` dela segue a animação até o fim).
            state.wake();
        }
        if animating && moved {
            return TimeoutAction::ToDuration(SHAKE_TICK);
        }
        flag.set(false);
        TimeoutAction::Drop
    });
    if inserted.is_err() {
        running.set(false);
    }
}

/// Move o cursor de hardware de todas as telas. Devolve se todas usam o cursor de
/// hardware (senão, alguma precisa de um quadro novo com o cursor desenhado).
fn move_hw_cursors(gpu: &mut Gpu, state: &State) -> bool {
    let Gpu { heads, scene, drm, gbm, cursor_size, .. } = gpu;
    let mut all = true;
    for head in heads.iter_mut() {
        all &= hw_cursor(drm, gbm, *cursor_size, head, scene, state);
    }
    all
}

/// Põe o cursor desta tela no plano de cursor da placa. Devolve se deu (a cena não
/// desenha o cursor); `false` quando a cena precisa desenhar: cursor de um app
/// (superfície), imagem maior que o plano, driver que recusa ou `MIDE_SW_CURSOR=1`.
#[allow(deprecated)]
fn hw_cursor(drm: &DrmDevice, gbm: &GbmDevice<DrmDeviceFd>, plane: (u32, u32), head: &mut Head, scene: &mut Scene, state: &State) -> bool {
    use smithay::reexports::gbm::BufferObjectFlags;
    let hide = |head: &mut Head| {
        if head.cursor.shown.take().is_some() {
            let _ = drm.set_cursor2(head.crtc, None::<&smithay::reexports::gbm::BufferObject<()>>, (0, 0));
        }
    };
    if head.cursor.failed || std::env::var_os("MIDE_SW_CURSOR").is_some() {
        return false;
    }
    let Some(area) = state.space.output_geometry(&head.output) else {
        return false;
    };
    // Cursor desenhado pelo app (VS Code, navegadores, GTK): também vai para o plano de
    // hardware, lido do buffer dele. Sem isso, cada movimento do mouse redesenhava a tela
    // inteira, e um quadro lento atrasava o ponteiro.
    let app_cursor = match &state.cursor_status {
        smithay::input::pointer::CursorImageStatus::Surface(surface)
            if state.cursor == crate::input::CursorShape::Default && state.shake.scale(std::time::Instant::now()) <= 1.001 =>
        {
            client_cursor(surface, head.output.current_scale().fractional_scale())
        }
        _ => None,
    };
    let owned: Vec<u8>;
    let (name, pixels, size, hotspot): (String, &[u8], (u32, u32), (i32, i32)) = match app_cursor {
        Some((key, data, size, hotspot)) => {
            owned = data;
            (key, &owned, size, hotspot)
        }
        None => {
            let Some(plan) = scene.cursor_plan(state, head.output.current_scale().fractional_scale()) else {
                return false;
            };
            match plan {
                crate::cursor::Plan::Hidden => {
                    hide(head);
                    return true;
                }
                crate::cursor::Plan::Surface => {
                    hide(head);
                    return false;
                }
                crate::cursor::Plan::Image { name, pixels, size, hotspot } => (name, pixels, size, hotspot),
            }
        }
    };
    if size.0 > plane.0 || size.1 > plane.1 {
        hide(head);
        return false;
    }
    // O ponteiro está em outra tela: aqui, nada (a outra mostra).
    let pointer = state.pointer_location;
    let inside = pointer.x >= area.loc.x as f64
        && pointer.y >= area.loc.y as f64
        && pointer.x < (area.loc.x + area.size.w) as f64
        && pointer.y < (area.loc.y + area.size.h) as f64;
    if !inside {
        hide(head);
        return true;
    }
    if head.cursor.shown.as_deref() != Some(name.as_str()) {
        let result = (|| -> std::io::Result<()> {
            if head.cursor.spare.is_none() {
                head.cursor.spare = Some(gbm.create_buffer_object::<()>(plane.0, plane.1, Fourcc::Argb8888, BufferObjectFlags::CURSOR | BufferObjectFlags::WRITE)?);
            }
            // RGBA pré-multiplicado do tema para ARGB8888 (B, G, R, A na memória), no
            // canto do plano; o resto transparente.
            let mut data = vec![0u8; (plane.0 * plane.1 * 4) as usize];
            for y in 0..size.1 {
                for x in 0..size.0 {
                    let s = ((y * size.0 + x) * 4) as usize;
                    let d = ((y * plane.0 + x) * 4) as usize;
                    data[d..d + 4].copy_from_slice(&[pixels[s + 2], pixels[s + 1], pixels[s], pixels[s + 3]]);
                }
            }
            let buffer = head.cursor.spare.as_mut().ok_or_else(|| std::io::Error::other("sem buffer"))?;
            buffer.write(&data)?;
            drm.set_cursor2(head.crtc, Some(&*buffer), hotspot)?;
            // O mostrado vira o atual; o anterior fica livre para a próxima imagem.
            std::mem::swap(&mut head.cursor.buffer, &mut head.cursor.spare);
            Ok(())
        })();
        if let Err(err) = result {
            tracing::warn!("tela {}: cursor de hardware recusado ({err}); usando o desenhado", head.output.name());
            head.cursor.failed = true;
            head.cursor.buffer = None;
            head.cursor.spare = None;
            return false;
        }
        head.cursor.shown = Some(name);
        head.cursor.position = None;
    }
    // Posição do canto da imagem, em pixels da tela (como o cursor desenhado).
    let scale = head.output.current_scale().fractional_scale();
    let x = ((pointer.x - area.loc.x as f64) * scale).round() as i32 - hotspot.0;
    let y = ((pointer.y - area.loc.y as f64) * scale).round() as i32 - hotspot.1;
    if head.cursor.position != Some((x, y)) {
        if let Err(err) = drm.move_cursor(head.crtc, (x, y)) {
            tracing::warn!("tela {}: cursor de hardware não moveu ({err}); usando o desenhado", head.output.name());
            hide(head);
            head.cursor.failed = true;
            return false;
        }
        head.cursor.position = Some((x, y));
    }
    true
}

/// Imagem do cursor de um app (buffer de memória compartilhada): chave que muda com a
/// imagem, pixels em RGBA pré-multiplicado, tamanho e ponto quente em pixels. `None`
/// quando o plano de hardware não serve (escala do buffer diferente da tela, buffer da
/// GPU, formato desconhecido): a cena desenha.
type AppCursor = (String, Vec<u8>, (u32, u32), (i32, i32));

fn client_cursor(surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface, scale: f64) -> Option<AppCursor> {
    use smithay::reexports::wayland_server::protocol::wl_shm::Format;
    use std::hash::{Hash, Hasher};
    let (buffer, buffer_scale) = smithay::backend::renderer::utils::with_renderer_surface_state(surface, |s| (s.buffer().cloned(), s.buffer_scale()))?;
    let buffer = buffer?;
    if (buffer_scale as f64 - scale).abs() > 0.01 {
        return None;
    }
    let hotspot = smithay::wayland::compositor::with_states(surface, |states| {
        states.data_map.get::<smithay::input::pointer::CursorImageSurfaceData>().and_then(|d| d.lock().ok().map(|a| a.hotspot))
    })?;
    let (pixels, w, h) = smithay::wayland::shm::with_buffer_contents(&buffer, |ptr, len, data| {
        let (w, h, stride, offset) = (data.width.max(0) as usize, data.height.max(0) as usize, data.stride.max(0) as usize, data.offset.max(0) as usize);
        let opaque = match data.format {
            Format::Argb8888 => false,
            Format::Xrgb8888 => true,
            _ => return None,
        };
        if w == 0 || h == 0 || stride < w * 4 || offset + stride * h > len {
            return None;
        }
        // SAFETY: a área foi conferida contra o tamanho do mapeamento acima.
        let src = unsafe { std::slice::from_raw_parts(ptr.add(offset), stride * h) };
        let mut out = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let s = y * stride + x * 4;
                let d = (y * w + x) * 4;
                // Na memória, B, G, R, A; aqui, R, G, B, A (como os do tema).
                out[d] = src[s + 2];
                out[d + 1] = src[s + 1];
                out[d + 2] = src[s];
                out[d + 3] = if opaque { 255 } else { src[s + 3] };
            }
        }
        Some((out, w as u32, h as u32))
    })
    .ok()??;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    pixels.hash(&mut hasher);
    let key = format!("app:{:x}", hasher.finish());
    Some((key, pixels, (w, h), (hotspot.x * buffer_scale, hotspot.y * buffer_scale)))
}

/// Liga ou desliga os monitores (propriedade DPMS dos conectores: 0 ligado, 3
/// desligado; o kernel aceita também com o modo atômico).
fn set_dpms(gpu: &Gpu, on: bool) {
    for head in &gpu.heads {
        let Ok(props) = gpu.drm.get_properties(head.connector) else {
            continue;
        };
        let (handles, _) = props.as_props_and_values();
        for &prop in handles {
            let is_dpms = gpu.drm.get_property(prop).is_ok_and(|info| info.name().to_bytes() == b"DPMS");
            if is_dpms {
                match gpu.drm.set_property(head.connector, prop, if on { 0 } else { 3 }) {
                    Ok(()) => tracing::info!("monitor {}: {}", head.output.name(), if on { "ligado" } else { "desligado (DPMS)" }),
                    Err(err) => tracing::warn!("monitor {}: DPMS não mudou: {err}", head.output.name()),
                }
            }
        }
    }
}

/// Tempo que o computador passou suspenso desde que ligou: a diferença entre o relógio
/// que conta a suspensão (`CLOCK_BOOTTIME`) e o que não conta (`CLOCK_MONOTONIC`).
fn suspended_time() -> Duration {
    let read = |clock| {
        let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
        // SAFETY: `ts` é um timespec válido e o relógio existe no Linux.
        unsafe { libc::clock_gettime(clock, &mut ts) };
        Duration::new(ts.tv_sec as u64, ts.tv_nsec as u32)
    };
    read(libc::CLOCK_BOOTTIME).saturating_sub(read(libc::CLOCK_MONOTONIC))
}

/// Recomeça as telas depois de o estado delas mudar por fora (DPMS, suspensão): sem
/// quadro pendente, estado do CRTC reavaliado (o próximo quadro refaz o modo), buffers
/// e cursor de novo.
fn reset_heads(gpu: &mut Gpu) {
    for head in gpu.heads.iter_mut() {
        restart_head(head);
    }
}

/// Espera máxima pelo vblank de um quadro entregue antes de dá-lo por perdido (a 60 Hz
/// são 16 ms; um segundo inteiro só acontece quando o aviso do kernel não vem mais).
const VBLANK_LOST: Duration = Duration::from_secs(1);

/// Recomeça uma tela: o quadro em voo é dado por apresentado, o estado do CRTC é
/// reavaliado e os buffers e o cursor vão de novo.
///
/// Dar o quadro em voo por apresentado é o essencial: o `GbmBufferedSurface` do Smithay
/// guarda esse quadro até o `frame_submitted` e, enquanto ele existe, `queue_buffer` só
/// enfileira o próximo, sem mandá-lo à placa (e devolve `Ok`). Sem isso, depois de um
/// vblank perdido a tela nunca mais trocaria de quadro, mesmo "recomeçada".
fn restart_head(head: &mut Head) {
    if head.pending {
        match head.surface.frame_submitted() {
            Ok(Some(_)) => tracing::info!("tela {}: quadro em voo dado por apresentado", head.output.name()),
            Ok(None) => {}
            Err(err) => tracing::warn!("tela {}: quadro na fila não foi entregue: {err}", head.output.name()),
        }
    }
    head.pending = false;
    head.scanout = ScanoutHead::default();
    // O estado do CRTC mudou por fora do Smithay: sem reavaliar, em alguns drivers
    // o quadro falha e a tela fica preta.
    if let Err(err) = head.surface.surface().reset_state() {
        tracing::warn!("monitor {}: estado não reavaliado: {err}", head.output.name());
    }
    head.surface.reset_buffers();
    // O monitor religado pode ter perdido o cursor: manda de novo.
    head.cursor = HwCursor::default();
}

/// O tempo suspenso (BOOTTIME − MONOTONIC) cresceu mais de 2 s desde a última
/// conferência: o computador dormiu e voltou.
fn resumed_by_clock(before: Duration, now: Duration) -> bool {
    now.saturating_sub(before) > Duration::from_secs(2)
}

/// Tela de bloqueio desta saída já tem imagem (o programa de bloqueio desenhou).
fn lock_drawn(state: &State, output: &Output) -> bool {
    state.lock.locked
        && state.lock.surface_for(output).is_some_and(|s| {
            smithay::backend::renderer::utils::with_renderer_surface_state(s.wl_surface(), |r| r.buffer().is_some()).unwrap_or(false)
        })
}

/// Suspensão em andamento (`crate::sleep`): confere se a tela de bloqueio já está em
/// todos os monitores e, se estiver (ou o tempo acabou), libera o sistema para dormir;
/// dormindo há tempo demais sem aviso de volta, as telas voltam.
fn check_sleep(gpu: &mut Gpu, state: &mut State) {
    if state.sleep.phase() == crate::sleep::Phase::Awake {
        return;
    }
    let generation = state.sleep.generation();
    let heads = gpu.heads.iter().filter(|h| h.missing.is_none());
    let flips_pending = heads.clone().any(|h| h.pending);
    let lock_presented = heads.clone().all(|h| h.lock_shown == generation);
    let ready = crate::sleep::ready(
        state.config.lock.on_suspend,
        state.lock.locked && state.space.outputs().all(|o| lock_drawn(state, o)),
        gpu.blanked || !gpu.drm.is_active(),
        flips_pending,
        lock_presented,
    );
    if crate::sleep::Change::Expired == state.sleep.poll(Instant::now(), ready) {
        state.resumed = true;
        state.redraw = true;
        render(gpu, state);
    }
}

/// Desenha um quadro em cada tela que não espera vblank e o entrega (aparece no
/// próximo vblank dela).
fn render(gpu: &mut Gpu, state: &mut State) {
    if !gpu.drm.is_active() {
        return;
    }
    // Suspendendo, com a tela de bloqueio já no monitor: nada vai à placa até a volta
    // (o pedido de quadro fica guardado para ela).
    if !state.sleep.frames_allowed() {
        state.render_stalled = false;
        return;
    }
    // Quadro demorado atrasa o mouse (o laço não lê a entrada enquanto desenha): fica
    // registrado, no máximo uma vez a cada 5 s, para achar a causa.
    let started = std::time::Instant::now();
    let _slow = SlowFrame { started, shake_scale: state.shake.scale(std::time::Instant::now()) };
    crate::watchdog::phase("render");
    // Tela apagada (tempo sem uso): desliga os monitores e não desenha; ao acender,
    // liga de novo e recomeça os quadros (o que esperava o vblank não vai chegar).
    if state.blanked != gpu.blanked {
        gpu.blanked = state.blanked;
        set_dpms(gpu, !state.blanked);
        if !state.blanked {
            reset_heads(gpu);
            state.redraw = true;
            // Monitor que sumiu com as telas apagadas: a folga começa agora, ao acender.
            let mut any_missing = false;
            for head in gpu.heads.iter_mut().filter(|h| h.missing.is_some()) {
                head.missing = Some(Instant::now());
                any_missing = true;
            }
            if any_missing {
                crate::outputs::recheck_later(state, crate::outputs::UNPLUG_GRACE);
            }
        }
    }
    // Voltou da suspensão: uma troca de quadro em andamento pode ter se perdido (o mide
    // esperaria o vblank dela para sempre, com a tela parada).
    if std::mem::take(&mut state.resumed) && !gpu.blanked {
        tracing::info!("telas refeitas depois da suspensão");
        reset_heads(gpu);
        state.redraw = true;
    }
    if gpu.blanked {
        // Apagada: nada a desenhar. O pedido é atendido aqui (senão o próximo `wake`,
        // vendo um quadro já pedido, não avisaria, e a tela não acenderia mais).
        state.redraw = false;
        state.render_stalled = false;
        return;
    }
    let wanted = state.redraw;
    state.redraw = false;
    let mut again = false;
    let mut plane_ok = true;
    let Gpu { renderer, heads, scene, drm, gbm, cursor_size, fd, scanout_notes, scanout_off, primary_dev, .. } = gpu;
    for head in heads.iter_mut() {
        // Monitor sumido (na folga antes de a tela sair): nada a entregar ao conector.
        if head.missing.is_some() {
            continue;
        }
        if head.pending {
            // Esta tela desenha no próximo vblank dela.
            again |= wanted;
            continue;
        }
        // Cursor no plano de hardware quando dá; numa gravação de tela que pede o
        // cursor, a cena o desenha (o plano de hardware não entra na imagem).
        scene.draw_cursor = if state.screencopy.wants_cursor() {
            if head.cursor.shown.take().is_some() {
                #[allow(deprecated)]
                let _ = drm.set_cursor2(head.crtc, None::<&smithay::reexports::gbm::BufferObject<()>>, (0, 0));
            }
            true
        } else {
            !hw_cursor(drm, gbm, *cursor_size, head, scene, state)
        };
        plane_ok &= !scene.draw_cursor;
        // Suspendendo: este quadro conta como "tela de bloqueio entregue" se ela já
        // tem imagem (o vblank dele libera a suspensão).
        let lock_frame = if state.sleep.preparing() && lock_drawn(state, &head.output) { state.sleep.generation() } else { 0 };
        // Modo de jogo (A18): o buffer do app direto à placa, sem compor, quando dá.
        update_feedback(head, state, *primary_dev, *scanout_off);
        if try_scanout(head, fd, gbm, state, scanout_notes, *scanout_off, scene.draw_cursor) {
            head.pending = true;
            head.queued_at = Instant::now();
            head.lock_queued = lock_frame;
            if let Err(err) = send_frames(state, &head.output) {
                tracing::debug!("avisos de quadro: {err}");
            }
            continue;
        }
        // Voltando do scanout direto: o primeiro quadro composto redesenha a tela inteira
        // (os buffers do mide não sabem o que o app mostrou).
        if std::mem::take(&mut head.scanout.active) {
            head.surface.reset_buffer_ages();
            head.scanout.fbs.clear();
            head.scanout.refused.clear();
        }
        let Head { surface, output, damage_tracker, internal, .. } = head;
        let internal = *internal;
        let result = (|| -> Result<(), Box<dyn Error>> {
            // Com a idade do buffer, a GPU só redesenha o que mudou desde que ele foi
            // usado (`MIDE_FULL_DAMAGE=1` volta a redesenhar tudo).
            let (mut dmabuf, age) = surface.next_buffer()?;
            let full = std::env::var_os("MIDE_FULL_DAMAGE").is_some();
            let age = if full { 0 } else { age as usize };
            let (sync, damage) = {
                let mut target = renderer.bind(&mut dmabuf)?;
                scene.render(renderer, &mut target, output, damage_tracker, state, age)?
            };
            // Ao kernel, só as áreas que mudaram; na tela do notebook (eDP), a tela
            // inteira: com PSR2 (Intel), ela atualizaria só uma faixa e o resto ficaria
            // com o quadro antigo.
            let size = output.current_mode().map(|m| m.size).unwrap_or_default();
            let clips = match damage {
                Some(rects) if !internal && !full && !rects.is_empty() => rects,
                _ => vec![Rectangle::from_size(size)],
            };
            // Os pedidos de aviso das superfícies deste quadro vão com ele até o vblank
            // (se o quadro não chegar à tela, o Smithay os descarta).
            let feedback = crate::presentation::collect(state, output);
            surface.queue_buffer(Some(sync), Some(clips), feedback)?;
            Ok(())
        })();
        match result {
            Ok(()) => {
                head.pending = true;
                head.queued_at = Instant::now();
                head.lock_queued = lock_frame;
            }
            Err(err) => {
                // Com a nova tentativa a cada evento, o aviso sai no máximo a cada 5 s.
                use std::sync::atomic::{AtomicU64, Ordering};
                static LAST: AtomicU64 = AtomicU64::new(0);
                let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
                if now.saturating_sub(LAST.swap(now, Ordering::Relaxed)) >= 5 {
                    tracing::warn!("falha ao entregar o quadro da tela {} à placa (tentando de novo): {err}", head.output.name());
                }
                again = true;
            }
        }
        if let Err(err) = send_frames(state, &head.output) {
            tracing::debug!("avisos de quadro: {err}");
        }
    }
    state.cursor_plane = plane_ok;
    // Algo em movimento (ou uma tela que ficou para depois): o próximo vblank desenha.
    if again || gpu.scene.moving(state) {
        state.redraw = true;
    }
    // Quadro pedido e nenhuma tela esperando vblank (a entrega falhou): nenhum vblank
    // vai chamar o desenho de novo. O próximo `wake` (cliente, entrada) avisa mesmo com o
    // pedido já feito, e o relógio das telas tenta a cada meio segundo. Antes disso, a
    // tela ficava parada até algo de fora (a volta da suspensão) chamar o desenho.
    let waiting = gpu.heads.iter().any(|h| h.pending && h.missing.is_none());
    state.render_stalled = state.redraw && !waiting;
}

/// Scanout direto (A18): com a tela no modo de jogo e o buffer da janela em tela cheia
/// servindo (dmabuf no tamanho da tela, nada do mide por cima, cursor no plano da
/// placa), manda o buffer direto ao plano principal. Devolve se a troca foi pedida;
/// senão, a tela é composta como sempre (e o motivo vai ao log, uma vez por mudança).
fn try_scanout(head: &mut Head, fd: &DrmDeviceFd, gbm: &GbmDevice<DrmDeviceFd>, state: &mut State, notes: &mut crate::scanout::Notes, off: bool, scene_cursor: bool) -> bool {
    use smithay::backend::drm::{PlaneConfig, PlaneState};
    let name = head.output.name();
    if state.game_window(&head.output).is_none() {
        notes.forget(&name);
        return false;
    }
    let drm_surface = head.surface.surface();
    let mode = drm_surface.pending_mode().size();
    let mode = (mode.0 as i32, mode.1 as i32);
    let candidate = if off {
        Err("desligado por MIDE_NO_SCANOUT".to_string())
    } else if scene_cursor {
        Err("cursor desenhado pela cena".to_string())
    } else if drm_surface.is_legacy() {
        Err("driver sem DRM atômico".to_string())
    } else if drm_surface.commit_pending() {
        Err("troca de modo pendente".to_string())
    } else {
        crate::scanout::candidate(state, &head.output, mode)
    };
    let candidate = match candidate {
        Ok(c) => c,
        Err(reason) => {
            notes.note(&name, Some(&reason));
            return false;
        }
    };
    let wl_buffer: &smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer = &candidate.buffer;
    if head.scanout.refused.iter().any(|b| b == wl_buffer) {
        return false;
    }
    let fb = match head.scanout.fbs.iter().find(|(b, _)| b == wl_buffer) {
        Some((_, fb)) => fb.clone(),
        None => {
        // O plano principal aceita o formato e o modificador do buffer? (`IN_FORMATS`
        // da placa). Com alfa (ARGB), vale a versão opaca (XRGB) do mesmo modificador:
        // a janela cobre a tela e o fundo do modo de jogo é preto, então ignorar o alfa
        // dá a mesma imagem da composição. Sem a opaca no plano, vai o formato com alfa
        // (o plano mistura com o preto do CRTC, também igual à composição).
        let plane_formats = &drm_surface.plane_info().formats;
        let format = smithay::wayland::dmabuf::get_dmabuf(wl_buffer).map(smithay::backend::allocator::Buffer::format).ok();
        let use_opaque = match format {
            Some(format) => {
                let opaque = smithay::backend::allocator::format::get_opaque(format.code)
                    .map(|code| smithay::backend::allocator::Format { code, modifier: format.modifier })
                    .filter(|f| plane_formats.contains(f));
                if opaque.is_none() && !plane_formats.contains(&format) {
                    notes.note(&name, Some(&format!(
                        "o plano principal não aceita o buffer: {} ({} formato(s) no plano); o app deveria realocar pelo feedback de scanout",
                        describe_format(&format),
                        plane_formats.iter().count()
                    )));
                    head.scanout.refused.push(wl_buffer.clone());
                    return false;
                }
                opaque.is_some()
            }
            None => true,
        };
        match smithay::backend::drm::gbm::framebuffer_from_wayland_buffer(fd, gbm, wl_buffer, use_opaque) {
            Ok(Some(fb)) => {
                let fb = Rc::new(fb);
                head.scanout.fbs.push((wl_buffer.clone(), fb.clone()));
                if head.scanout.fbs.len() > 4 {
                    head.scanout.fbs.remove(0);
                }
                fb
            }
            Ok(None) => {
                notes.note(&name, Some("buffer sem formato para a placa (sem modificador)"));
                head.scanout.refused.push(wl_buffer.clone());
                return false;
            }
            Err(err) => {
                let what = format.map(|f| describe_format(&f)).unwrap_or_else(|| "buffer EGL".into());
                notes.note(&name, Some(&format!(
                    "a placa não importou o buffer {what}: {} ({} formato(s) no plano)",
                    error_chain(&err),
                    plane_formats.iter().count()
                )));
                head.scanout.refused.push(wl_buffer.clone());
                return false;
            }
        }
        }
    };
    let (w, h) = candidate.size;
    let plane = PlaneState {
        handle: drm_surface.plane(),
        config: Some(PlaneConfig {
            src: Rectangle::from_size((w as f64, h as f64).into()),
            dst: Rectangle::from_size((w, h).into()),
            transform: Transform::Normal,
            alpha: 1.0,
            damage_clips: None,
            fb: *AsRef::<smithay::reexports::drm::control::framebuffer::Handle>::as_ref(&*fb),
            fence: None,
        }),
    };
    if let Err(err) = drm_surface.test_state([plane.clone()], false) {
        let what = smithay::wayland::dmabuf::get_dmabuf(wl_buffer).map(|d| describe_format(&smithay::backend::allocator::Buffer::format(d))).unwrap_or_default();
        notes.note(&name, Some(&format!("a placa recusou o buffer {what} no plano principal ({})", error_chain(&err))));
        head.scanout.refused.push(wl_buffer.clone());
        if head.scanout.refused.len() > 8 {
            head.scanout.refused.remove(0);
        }
        return false;
    }
    if let Err(err) = drm_surface.page_flip([plane], true) {
        notes.note(&name, Some(&format!("troca de quadro recusada ({err})")));
        return false;
    }
    let feedback = crate::presentation::collect(state, &head.output);
    head.scanout.pending = Some(Scanned { _buffer: candidate.buffer.clone(), _fb: fb, feedback: Some(feedback) });
    head.scanout.active = true;
    notes.note(&name, None);
    true
}

/// Feedback do linux-dmabuf da janela em tela cheia (A18): com o modo de jogo na tela,
/// a superfície dela recebe a tranche de scanout do plano principal (o app realoca os
/// buffers num formato que a placa mostra direto); ao sair do modo, volta ao padrão.
/// Superfícies que não pediram feedback próprio não recebem nada (o Smithay ignora).
fn update_feedback(head: &mut Head, state: &State, primary_dev: Option<libc::dev_t>, off: bool) {
    let drm_surface = head.surface.surface();
    let want = if off || drm_surface.is_legacy() || primary_dev.is_none() {
        None
    } else {
        state.game_window(&head.output).and_then(|w| w.toplevel().map(|t| t.wl_surface().clone()))
    };
    if head.feedback.surface != want
        && let Some(old) = head.feedback.surface.take()
        && smithay::reexports::wayland_server::Resource::is_alive(&old)
    {
        state.dmabuf.set_surface_feedback(&old, None);
    }
    let (Some(surface), Some(target)) = (want, primary_dev) else {
        return;
    };
    if head.feedback.built.is_none() {
        let plane = &drm_surface.plane_info().formats;
        let built = state.dmabuf.scanout_feedback(target, plane);
        let name = head.output.name();
        match &built {
            Some((_, n)) => tracing::info!(
                "scanout direto: tela {name}: feedback de scanout com {n} formato(s) (de {} no plano principal)",
                plane.iter().count()
            ),
            None => tracing::info!(
                "scanout direto: tela {name}: sem feedback de scanout (nenhum dos {} formato(s) do plano principal é importado pelo renderizador, ou sem linux-dmabuf)",
                plane.iter().count()
            ),
        }
        head.feedback.built = Some(built.map(|(f, _)| f));
    }
    if let Some(Some(feedback)) = &head.feedback.built {
        state.dmabuf.set_surface_feedback(&surface, Some(feedback));
    }
    head.feedback.surface = Some(surface);
}

/// Nó primário (`cardN`) da placa aberta em `fd`.
fn primary_node(fd: &DrmDeviceFd) -> Option<libc::dev_t> {
    use smithay::backend::drm::{DrmNode, NodeType};
    let node = DrmNode::from_file(fd).ok()?;
    if node.ty() == NodeType::Primary {
        return Some(node.dev_id());
    }
    node.node_with_type(NodeType::Primary).and_then(Result::ok).map(|n| n.dev_id())
}

/// Formato de um buffer para o log: fourcc e modificador (nome e número).
fn describe_format(format: &smithay::backend::allocator::Format) -> String {
    format!("{:?} modificador {:?} (0x{:016x})", format.code, format.modifier, u64::from(format.modifier))
}

/// Mensagem de erro com as causas (o Smithay põe o erro do kernel na `source`).
fn error_chain(err: &dyn Error) -> String {
    let mut text = err.to_string();
    let mut source = err.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}

/// Avisa no log, ao sair de `render`, se o quadro passou de 12 ms.
struct SlowFrame {
    started: std::time::Instant,
    shake_scale: f32,
}

impl Drop for SlowFrame {
    fn drop(&mut self) {
        use std::sync::atomic::{AtomicU64, Ordering};
        static LAST: AtomicU64 = AtomicU64::new(0);
        let ms = self.started.elapsed().as_secs_f32() * 1000.0;
        if ms > 12.0 {
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
            if now.saturating_sub(LAST.swap(now, Ordering::Relaxed)) >= 5 {
                tracing::warn!(
                    milissegundos = ms,
                    cursor_ampliado = self.shake_scale > 1.001,
                    escala_cursor = self.shake_scale,
                    "quadro demorado (o ponteiro pode ter travado)"
                );
            }
        }
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

#[cfg(test)]
mod tests {
    use super::{only_cursor_moved, resumed_by_clock};
    use std::time::Duration;

    #[test]
    fn volta_da_suspensao_pelo_relogio() {
        let s = Duration::from_secs;
        // Tempo suspenso parado ou com ruído: não dormiu.
        assert!(!resumed_by_clock(s(100), s(100)));
        assert!(!resumed_by_clock(s(100), s(102)));
        // Cresceu mais de 2 s entre duas conferências: dormiu e voltou.
        assert!(resumed_by_clock(s(100), Duration::from_millis(102_001)));
        assert!(resumed_by_clock(s(0), s(3600)));
        // Relógio que "volta" (não acontece, mas não pode disparar).
        assert!(!resumed_by_clock(s(100), s(50)));
    }

    #[test]
    fn arrasto_com_icone_pede_quadro_a_cada_movimento() {
        // Só o mouse andou: move o cursor da placa, sem quadro.
        assert!(only_cursor_moved(true, false, false, true));
        // Algo segue o ponteiro (ícone do arrasto, janela movida): quadro novo.
        assert!(!only_cursor_moved(true, false, true, true));
        // Alguém pediu quadro, o cursor mudou de forma ou não foi movimento.
        assert!(!only_cursor_moved(true, true, false, true));
        assert!(!only_cursor_moved(true, false, false, false));
        assert!(!only_cursor_moved(false, false, false, true));
    }
}
