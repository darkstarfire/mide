//! Execução de um app do mide-ui: abre a janela (com a barra de título do mide),
//! cuida do mouse, do teclado e do desenho, e chama o `view` do app a cada quadro.

use std::{error::Error, num::NonZeroU32};

use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    data_device_manager::{
        data_device::{DataDevice, DataDeviceHandler},
        data_offer::{DataOfferHandler, DragOffer},
        data_source::{CopyPasteSource, DataSourceHandler},
        DataDeviceManagerState, WritePipe,
    },
    delegate_registry,
    output::{OutputHandler, OutputState},
    reexports::{
        calloop::{
            ping::{make_ping, Ping},
            timer::{TimeoutAction, Timer},
            EventLoop, LoopHandle, PostAction,
        },
        calloop_wayland_source::WaylandSource,
        client::{
            globals::registry_queue_init,
            protocol::{
                wl_data_device::WlDataDevice, wl_data_device_manager::DndAction, wl_data_source::WlDataSource, wl_keyboard,
                wl_output, wl_pointer, wl_seat, wl_shm, wl_surface,
            },
            Connection, QueueHandle,
        },
    },
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers, RawModifiers},
        pointer::{PointerEvent, PointerEventKind, PointerHandler},
        Capability, SeatHandler, SeatState,
    },
    shell::{
        wlr_layer::{Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface, LayerSurfaceConfigure},
        xdg::{
            window::{Window as XdgWindow, WindowConfigure, WindowDecorations, WindowHandler},
            XdgShell,
        },
        WaylandSurface,
    },
    shm::{slot::SlotPool, Shm, ShmHandler},
};
use tiny_skia::Pixmap;

use super::{
    input::{Key, KeyPress},
    theme::{color, Theme},
    ui::{Ctx, Ui},
};
use crate::ui::draw::to_argb8888;

/// Um app do mide-ui: descreve a tela no `view`, que é chamado a cada quadro.
pub trait App {
    fn view(&mut self, ui: &mut Ui);

    /// O usuário pediu para fechar a janela (botão da barra, atalho). Devolva `false`
    /// para continuar aberta (por exemplo, para perguntar se salva as alterações);
    /// depois, o app fecha com `ui.close()`.
    fn close_requested(&mut self) -> bool {
        true
    }

    /// Menus do app na barra superior do mide (vazio: os menus padrão do mide). Lido
    /// depois de cada quadro; o clique num item chega em `ui.input().menu`. Inclua
    /// [`window_menu`](super::window_menu) para o menu "Janela".
    fn menu_bar(&mut self) -> Vec<super::MenuGroup> {
        Vec::new()
    }
}

/// Janela do app: título, id (para a dock e o launcher) e tamanhos.
#[derive(Debug, Clone)]
pub struct Window {
    pub title: String,
    pub app_id: String,
    pub size: (u32, u32),
    pub min_size: (u32, u32),
    pub theme: Theme,
    /// O tema foi escolhido pelo app (`Window::theme`): não segue o claro/escuro do sistema.
    pub theme_fixed: bool,
    /// Camada por cima de tudo (`Window::overlay`), em vez de janela comum.
    pub overlay: Option<String>,
    /// Camada de fundo, atrás das janelas (`Window::desktop`).
    pub desktop: bool,
    /// Camada que desce do topo, por cima das janelas (`Window::dropdown`).
    pub dropdown: bool,
    /// Tela de bloqueio (`Window::lock`).
    pub lock: bool,
}

impl Window {
    /// Janela de 640x480 com o tema padrão do mide.
    pub fn new(title: &str) -> Self {
        Self {
            title: title.into(),
            app_id: title.to_lowercase().replace(' ', "-"),
            size: (640, 480),
            min_size: (360, 240),
            theme: Theme::system(),
            theme_fixed: false,
            overlay: None,
            desktop: false,
            dropdown: false,
            lock: false,
        }
    }

    /// Tela de bloqueio (protocolo `ext-session-lock-v1`): o compositor esconde tudo e
    /// só esta tela recebe teclado e mouse até `Ui::unlock`. O app desenha na primeira
    /// tela; as outras ficam escuras.
    pub fn lock(mut self) -> Self {
        self.lock = true;
        // Sobre o papel de parede, sempre com as cores claras.
        self.theme = Theme::default();
        self.theme_fixed = true;
        self
    }

    /// Camada por cima das janelas, na área útil (abaixo da barra), com o teclado só
    /// para ela enquanto aparece: para painéis que descem do topo (o terminal de
    /// atalho). O app desenha só a parte de cima e diz onde recebe cliques
    /// (`Ui::input_area`); o resto deixa passar. Esconde e volta com `Ui::set_hidden`.
    pub fn dropdown(mut self, namespace: &str) -> Self {
        self.overlay = Some(namespace.into());
        self.dropdown = true;
        self
    }

    /// Camada de fundo, atrás das janelas e na frente do papel de parede (a área de
    /// trabalho com ícones): transparente, entre a barra e a dock, e com o teclado só
    /// quando clicada.
    pub fn desktop(mut self, namespace: &str) -> Self {
        self.overlay = Some(namespace.into());
        self.desktop = true;
        self
    }

    /// Em vez de janela, uma camada transparente por cima de tudo, na tela inteira e
    /// com o teclado só para ela (buscador, painéis). `namespace` é o nome da camada
    /// (também usado para o vidro). O app desenha o que quiser e fecha com `ui.close()`.
    pub fn overlay(mut self, namespace: &str) -> Self {
        self.overlay = Some(namespace.into());
        self
    }

    pub fn app_id(mut self, id: &str) -> Self {
        self.app_id = id.into();
        self
    }

    pub fn size(mut self, w: u32, h: u32) -> Self {
        self.size = (w, h);
        self
    }

    pub fn min_size(mut self, w: u32, h: u32) -> Self {
        self.min_size = (w, h);
        self
    }

    pub fn theme(mut self, theme: Theme) -> Self {
        self.theme = theme;
        self.theme_fixed = true;
        self
    }
}

/// Acorda a janela de outra thread para desenhar um quadro (por exemplo, quando
/// chega saída de um programa). Pegue com [`Ui::waker`](super::Ui::waker).
#[derive(Clone)]
pub struct Waker(pub(super) Ping);

impl Waker {
    pub fn wake(&self) {
        self.0.ping();
    }
}

/// Abre a janela e roda o app até ela ser fechada.
const TICK: std::time::Duration = std::time::Duration::from_millis(40);

pub fn run(window: Window, app: impl App + 'static) -> Result<(), Box<dyn Error>> {
    let conn = Connection::connect_to_env()?;
    let (globals, event_queue) = registry_queue_init(&conn)?;
    let qh = event_queue.handle();
    let compositor = CompositorState::bind(&globals, &qh)?;
    let shm = Shm::bind(&globals, &qh)?;

    let surface = compositor.create_surface(&qh);
    let output_state = OutputState::new(&globals, &qh);
    let shell = if window.lock {
        use smithay_client_toolkit::session_lock::SessionLockState;
        let lock_state = SessionLockState::new(&globals, &qh);
        let session = lock_state.lock(&qh).map_err(|e| format!("o compositor não aceita bloqueio de tela: {e}"))?;
        let outputs: Vec<wl_output::WlOutput> = output_state.outputs().collect();
        let Some(first) = outputs.first() else {
            return Err("sem tela para bloquear".into());
        };
        // A primeira tela de bloqueio é a do app (o compositor dá o teclado a ela).
        let main = session.create_lock_surface(surface, first, &qh);
        let others = outputs[1..].iter().map(|o| (session.create_lock_surface(compositor.create_surface(&qh), o, &qh), None)).collect();
        Shell::Lock { session, surface: main, others }
    } else {
        match &window.overlay {
        Some(namespace) => {
            let layer_shell = LayerShell::bind(&globals, &qh)?;
            let kind = if window.desktop {
                Layer::Bottom
            } else if window.dropdown {
                Layer::Top
            } else {
                Layer::Overlay
            };
            let layer = layer_shell.create_layer_surface(&qh, surface, kind, Some(namespace.clone()), None);
            layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
            if window.desktop {
                // Fica na área útil (abaixo da barra, acima da dock) e só pega o teclado
                // quando clicada.
                layer.set_exclusive_zone(0);
                layer.set_keyboard_interactivity(KeyboardInteractivity::OnDemand);
            } else if window.dropdown {
                layer.set_exclusive_zone(0);
                layer.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
            } else {
                layer.set_exclusive_zone(-1);
                layer.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
            }
            layer.commit();
            Shell::Layer(layer, namespace.clone())
        }
        None => {
            let xdg_shell = XdgShell::bind(&globals, &qh)?;
            let xdg = xdg_shell.create_window(surface, WindowDecorations::RequestServer, &qh);
            xdg.set_title(window.title.clone());
            xdg.set_app_id(window.app_id.clone());
            xdg.set_min_size(Some(window.min_size));
            xdg.commit();
            Shell::Xdg(xdg)
        }
    }
    };

    let mut event_loop: EventLoop<Runner> = EventLoop::try_new()?;
    let mut runner = Runner {
        data_devices: DataDeviceManagerState::bind(&globals, &qh).ok(),
        data_device: None,
        copied: None,
        serial: 0,
        qh: qh.clone(),
        loop_handle: event_loop.handle(),
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state,
        pool: SlotPool::new((window.size.0 * window.size.1 * 4) as usize, &shm)?,
        shm,
        shell,
        glass: Default::default(),
        glass_on: None,
        glass_checks: 0,
        app_id: window.app_id.clone(),
        last_frame: None,
        pointer: None,
        keyboard: None,
        size: window.size,
        min_size: window.min_size,
        configured: false,
        dirty: false,
        exit: false,
        ctx: Ctx::new(window.theme.clone()),
        follow_theme: !window.theme_fixed,
        theme_checks: 0,
        title: window.title.clone(),
        app: Box::new(app),
        menus: None,
        maximized: false,
        compositor,
        hidden: false,
        others_drawn: None,
        test_hide: std::env::var_os("MIDE_KIT_TEST_HIDE").map(|_| None),
    };

    // Vidro de verdade só nas camadas de cima (o compositor desfoca atrás delas).
    runner.ctx.layer = matches!(runner.shell, Shell::Layer(..)) && !window.desktop;
    WaylandSource::new(conn.clone(), event_queue)
        .insert(event_loop.handle())
        .map_err(|e| e.to_string())?;
    // Quadros pedidos de outra thread (`Ui::waker`).
    let (ping, ping_source) = make_ping().map_err(|e| e.to_string())?;
    event_loop
        .handle()
        .insert_source(ping_source, |_, _, runner: &mut Runner| runner.dirty = true)
        .map_err(|e| e.to_string())?;
    runner.ctx.waker = Some(Waker(ping));
    // Quadros pedidos pelo app (`Ui::request_tick`), por exemplo enquanto miniaturas
    // carregam em segundo plano.
    event_loop
        .handle()
        .insert_source(Timer::from_duration(TICK), |_, _, runner: &mut Runner| {
            if runner.ctx.ticking {
                runner.dirty = true;
            }
            // Janela com vidro: a opção mudou nas Configurações? Redesenha (opaca ou
            // translúcida). Uma consulta por segundo (o arquivo só é lido se mudou).
            if let Some(on) = runner.glass_on {
                runner.glass_checks = (runner.glass_checks + 1) % 25;
                if runner.glass_checks == 0 && crate::control_client::panel_glass() != on {
                    runner.dirty = true;
                }
            }
            // Claro/escuro trocado nas Configurações: o app muda na hora.
            if runner.follow_theme {
                runner.theme_checks = (runner.theme_checks + 1) % 25;
                if runner.theme_checks == 0 && super::theme::system_dark() != runner.ctx.theme.dark {
                    runner.ctx.theme = Theme::system();
                    runner.last_frame = None;
                    runner.dirty = true;
                }
            }
            if runner.ctx.wake_at.is_some_and(|at| std::time::Instant::now() >= at) {
                runner.ctx.wake_at = None;
                runner.dirty = true;
            }
            // Animando (menu entrando, transições): no ritmo da tela; senão, mais devagar.
            TimeoutAction::ToDuration(if runner.ctx.ticking { std::time::Duration::from_millis(16) } else { TICK })
        })
        .map_err(|e| e.to_string())?;
    while !runner.exit {
        event_loop.dispatch(None, &mut runner)?;
        if runner.dirty && runner.configured {
            runner.frame();
            runner.clipboard();
        }
    }
    // O desbloqueio (e o resto) precisa chegar ao compositor antes de sair.
    let _ = conn.flush();
    Ok(())
}

/// A superfície do app: janela comum, camada (com o nome dela) ou tela de bloqueio
/// (a do app e as das outras telas, que ficam escuras).
enum Shell {
    Xdg(XdgWindow),
    Layer(LayerSurface, String),
    Lock {
        session: smithay_client_toolkit::session_lock::SessionLock,
        surface: smithay_client_toolkit::session_lock::SessionLockSurface,
        /// As telas de bloqueio dos outros monitores e o tamanho de cada uma (depois
        /// da configuração): mostram a mesma tela do app, sem receber teclas.
        others: Vec<(smithay_client_toolkit::session_lock::SessionLockSurface, Option<(u32, u32)>)>,
    },
}

impl Shell {
    fn wl_surface(&self) -> &wl_surface::WlSurface {
        match self {
            Shell::Xdg(w) => w.wl_surface(),
            Shell::Layer(l, _) => l.wl_surface(),
            Shell::Lock { surface, .. } => surface.wl_surface(),
        }
    }

    fn commit(&self) {
        match self {
            Shell::Xdg(w) => w.commit(),
            Shell::Layer(l, _) => l.commit(),
            Shell::Lock { surface, .. } => surface.wl_surface().commit(),
        }
    }
}

struct Runner {
    /// Área de transferência: o gerenciador, o dispositivo do assento, o texto que este
    /// app copiou (com a origem oferecida aos outros) e o serial da última entrada
    /// (o compositor só aceita uma seleção pedida por uma entrada do usuário).
    data_devices: Option<DataDeviceManagerState>,
    data_device: Option<DataDevice>,
    copied: Option<(CopyPasteSource, String)>,
    serial: u32,
    qh: QueueHandle<Runner>,
    loop_handle: LoopHandle<'static, Runner>,
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    shm: Shm,
    pool: SlotPool,
    shell: Shell,
    /// Vidro pedido ao compositor (só em camadas).
    glass: crate::control_client::Glass,
    /// Estado do vidro no último quadro, se a janela usa vidro (`None` se não usa).
    glass_on: Option<bool>,
    glass_checks: u32,
    /// Segue o tema do sistema (verificado uma vez por segundo).
    follow_theme: bool,
    theme_checks: u32,
    app_id: String,
    /// Último quadro enviado (para não reenviar um igual).
    last_frame: Option<Pixmap>,
    pointer: Option<wl_pointer::WlPointer>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    size: (u32, u32),
    min_size: (u32, u32),
    configured: bool,
    dirty: bool,
    title: String,
    exit: bool,
    ctx: Ctx,
    app: Box<dyn App>,
    /// Menus publicados na barra superior.
    menus: Option<super::menubar::Published>,
    maximized: bool,
    compositor: CompositorState,
    /// A superfície foi escondida (`Ui::set_hidden`): sem imagem até voltar.
    hidden: bool,
    /// Quando as telas de bloqueio dos outros monitores foram desenhadas, e os tamanhos.
    others_drawn: Option<(std::time::Instant, Vec<(u32, u32)>)>,
    /// Teste de esconder e mostrar (`MIDE_KIT_TEST_HIDE`): quando a janela apareceu.
    test_hide: Option<Option<std::time::Instant>>,
}

impl Runner {
    /// Um quadro: chama o `view` do app e apresenta. Se algo mudou (um clique trocou
    /// o estado), desenha de novo para a tela mostrar o estado novo.
    fn frame(&mut self) {
        let start = std::time::Instant::now();
        self.dirty = false;
        // Cliques nos menus da barra: os da "Janela" são da janela; o resto, do app.
        if let Some(menus) = &self.menus {
            while let Ok(action) = menus.clicks.try_recv() {
                match (action.as_str(), &self.shell) {
                    (super::menubar::MINIMIZE, Shell::Xdg(xdg)) => xdg.set_minimized(),
                    (super::menubar::ZOOM, Shell::Xdg(xdg)) => {
                        if self.maximized { xdg.unset_maximized() } else { xdg.set_maximized() }
                    }
                    _ => self.ctx.input.menu.push(action),
                }
            }
        }
        let mut pixmap = None;
        for _ in 0..2 {
            let Some(mut canvas) = Pixmap::new(self.size.0, self.size.1) else {
                return;
            };
            // Camadas são transparentes: o app desenha só os painéis.
            if matches!(self.shell, Shell::Xdg(_)) {
                canvas.fill(color(self.ctx.theme.background));
            }
            self.ctx.begin_frame();
            {
                let mut ui = Ui::root(&mut canvas, &mut self.ctx);
                self.app.view(&mut ui);
            }
            // Menus de contexto ficam por cima de tudo.
            if let Some(overlay) = self.ctx.take_overlay() {
                let (alpha, dy) = self.ctx.overlay_look();
                let paint = tiny_skia::PixmapPaint { opacity: alpha, ..Default::default() };
                canvas.draw_pixmap(0, dy, overlay.as_ref(), &paint, tiny_skia::Transform::identity(), None);
            }
            if let Some(title) = self.ctx.title.take() {
                if title != self.title {
                    if let Shell::Xdg(xdg) = &self.shell {
                        xdg.set_title(title.clone());
                    }
                    self.title = title;
                }
            }
            let again = self.ctx.repaint;
            self.ctx.end_frame();
            pixmap = Some(canvas);
            if !again {
                break;
            }
        }
        // Senha aceita: desbloqueia e fecha (o app de bloqueio termina aqui).
        if std::mem::take(&mut self.ctx.unlock_request) {
            if let Shell::Lock { session, .. } = &self.shell {
                session.unlock();
            }
            self.exit = true;
            return;
        }
        // Teste do compositor (`MIDE_KIT_TEST_HIDE=1`): a janela se esconde 2 s depois
        // de aparecer e volta 2 s depois, como o Chromium em segundo plano.
        if self.configured || self.hidden {
            if let Some(phase) = self.test_hide.as_mut() {
                let at = *phase.get_or_insert_with(std::time::Instant::now);
                let t = at.elapsed();
                if t >= std::time::Duration::from_secs(4) && self.hidden {
                    self.ctx.hide_request = Some(false);
                    self.test_hide = None;
                } else if t >= std::time::Duration::from_secs(2) && !self.hidden {
                    self.ctx.hide_request = Some(true);
                }
                if self.test_hide.is_some() {
                    let next = std::time::Instant::now() + std::time::Duration::from_millis(250);
                    self.ctx.wake_at = Some(self.ctx.wake_at.map_or(next, |w| w.min(next)));
                }
            }
        }
        // Esconder e mostrar: sem imagem a superfície some; para voltar, um commit vazio
        // pede outra configuração ao compositor.
        match self.ctx.hide_request.take() {
            Some(true) if !self.hidden => {
                self.hidden = true;
                let surface = self.shell.wl_surface();
                surface.attach(None, 0, 0);
                self.shell.commit();
                self.last_frame = None;
            }
            Some(false) if self.hidden => {
                self.hidden = false;
                self.configured = false;
                self.shell.commit();
            }
            _ => {}
        }
        let groups = self.app.menu_bar();
        match &mut self.menus {
            Some(menus) => menus.update(groups),
            None if !groups.is_empty() => self.menus = super::menubar::Published::start(groups, self.ctx.waker.clone()),
            None => {}
        }
        let Some(pixmap) = pixmap else {
            return;
        };
        if self.ctx.close_requested {
            self.exit = true;
            return;
        }
        if self.hidden || !self.configured {
            return;
        }
        self.draw_other_lock_screens();
        if self.exit {
            return;
        }
        // Painéis de vidro (`ui.glass_panel`): o compositor desenha o vidro atrás.
        if let (Shell::Layer(_, namespace), true) = (&self.shell, self.ctx.layer) {
            let rects = if crate::control_client::panel_glass() { self.ctx.glass.clone() } else { Vec::new() };
            self.glass.set(namespace, &rects);
        } else if matches!(self.shell, Shell::Xdg(_)) {
            // Janela: vidro só onde o app pediu (barra lateral de vidro), pelo app_id.
            let rects = if crate::control_client::panel_glass() { self.ctx.glass.clone() } else { Vec::new() };
            self.glass.set(&format!("app:{}", self.app_id), &rects);
        }
        self.glass_on = self.ctx.glass_capable.then(crate::control_client::panel_glass);
        let (w, h) = self.size;
        // Quadro igual ao último enviado (por exemplo, um quadro agendado sem mudança):
        // não manda de novo, para o compositor não refazer o que está atrás (vidro).
        if self.last_frame.as_ref().is_some_and(|last| last.width() == w && last.height() == h && last.data() == pixmap.data()) {
            return;
        }
        let Ok((buffer, canvas)) = self.pool.create_buffer(w as i32, h as i32, w as i32 * 4, wl_shm::Format::Argb8888) else {
            return;
        };
        to_argb8888(&pixmap, canvas);
        tracing::debug!("quadro do app: {:.2} ms", start.elapsed().as_secs_f32() * 1000.0);
        let surface = self.shell.wl_surface();
        // Só onde o app desenha recebe cliques (`Ui::input_area`).
        match self.ctx.input_region.take() {
            Some((x, y, rw, rh)) => {
                if let Ok(region) = smithay_client_toolkit::compositor::Region::new(&self.compositor) {
                    region.add(x as i32, y as i32, rw as i32, rh as i32);
                    surface.set_input_region(Some(region.wl_region()));
                }
            }
            None => surface.set_input_region(None),
        }
        // Só a parte que mudou desde o último quadro conta como dano: o compositor
        // copia e redesenha só ela (digitar num campo não reenvia a janela inteira).
        let (dx, dy, dw, dh) = self.last_frame.as_ref().and_then(|last| changed_area(last, &pixmap)).unwrap_or((0, 0, w, h));
        surface.damage_buffer(dx as i32, dy as i32, dw as i32, dh as i32);
        if buffer.attach_to(surface).is_ok() {
            self.shell.commit();
        }
        self.last_frame = Some(pixmap);
    }
}

impl Runner {
    /// Tela de bloqueio nos outros monitores: o mesmo `view` do app, desenhado no
    /// tamanho de cada um e sem entrada (as teclas são só da tela principal).
    fn draw_other_lock_screens(&mut self) {
        let targets: Vec<(wl_surface::WlSurface, (u32, u32))> = match &self.shell {
            Shell::Lock { others, .. } => others.iter().filter_map(|(s, size)| size.map(|sz| (s.wl_surface().clone(), sz))).collect(),
            _ => return,
        };
        if targets.is_empty() {
            return;
        }
        // Nos outros monitores só o relógio muda: uma vez por segundo basta (a cada
        // tecla, o custo era o de desenhar todas as telas). Monitor novo: na hora.
        let sizes: Vec<(u32, u32)> = targets.iter().map(|(_, size)| *size).collect();
        if self.others_drawn.as_ref().is_some_and(|(at, last)| at.elapsed() < std::time::Duration::from_secs(1) && *last == sizes) {
            return;
        }
        self.others_drawn = Some((std::time::Instant::now(), sizes));
        let input = std::mem::take(&mut self.ctx.input);
        let (unlock, close) = (self.ctx.unlock_request, self.ctx.close_requested);
        self.ctx.mirror = true;
        for (surface, (w, h)) in targets {
            let Some(mut canvas) = Pixmap::new(w, h) else {
                continue;
            };
            self.ctx.begin_frame();
            {
                let mut ui = Ui::root(&mut canvas, &mut self.ctx);
                self.app.view(&mut ui);
            }
            let _ = self.ctx.take_overlay();
            self.ctx.end_frame();
            let Ok((buffer, data)) = self.pool.create_buffer(w as i32, h as i32, w as i32 * 4, wl_shm::Format::Argb8888) else {
                continue;
            };
            to_argb8888(&canvas, data);
            surface.damage_buffer(0, 0, w as i32, h as i32);
            if buffer.attach_to(&surface).is_ok() {
                surface.commit();
            }
        }
        // O que importa (teclas, desbloquear) é da tela principal. Um desbloqueio pedido
        // aqui também vale (nunca se perde).
        self.ctx.mirror = false;
        self.ctx.input = input;
        self.ctx.unlock_request |= unlock;
        self.ctx.close_requested = close;
        if self.ctx.unlock_request {
            if let Shell::Lock { session, .. } = &self.shell {
                session.unlock();
            }
            self.exit = true;
        }
    }
}

impl CompositorHandler for Runner {
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: i32) {}
    fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: wl_output::Transform) {}
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {}
    fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
    fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
}

impl OutputHandler for Runner {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl WindowHandler for Runner {
    fn request_close(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &XdgWindow) {
        if self.app.close_requested() {
            self.exit = true;
        } else {
            self.dirty = true;
        }
    }

    fn configure(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &XdgWindow, configure: WindowConfigure, _: u32) {
        self.maximized = configure.is_maximized();
        let (w, h) = configure.new_size;
        if let Some(w) = w.map(NonZeroU32::get) {
            self.size.0 = w.max(self.min_size.0);
        }
        if let Some(h) = h.map(NonZeroU32::get) {
            self.size.1 = h.max(self.min_size.1);
        }
        self.configured = true;
        self.dirty = true;
    }
}

impl smithay_client_toolkit::session_lock::SessionLockHandler for Runner {
    fn locked(&mut self, _: &Connection, _: &QueueHandle<Self>, _: smithay_client_toolkit::session_lock::SessionLock) {
        tracing::debug!("tela bloqueada pelo compositor");
    }

    /// O compositor recusou o bloqueio (outro programa já bloqueou): sai.
    fn finished(&mut self, _: &Connection, _: &QueueHandle<Self>, _: smithay_client_toolkit::session_lock::SessionLock) {
        tracing::info!("bloqueio recusado ou encerrado pelo compositor");
        self.exit = true;
    }

    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        surface: smithay_client_toolkit::session_lock::SessionLockSurface,
        configure: smithay_client_toolkit::session_lock::SessionLockSurfaceConfigure,
        _: u32,
    ) {
        let (w, h) = configure.new_size;
        if &surface.wl_surface() == &self.shell.wl_surface() {
            if w > 0 && h > 0 {
                self.size = (w, h);
            }
            self.configured = true;
            self.dirty = true;
            return;
        }
        // Outra tela: guarda o tamanho; o próximo quadro desenha nela também.
        if let Shell::Lock { others, .. } = &mut self.shell {
            if let Some(entry) = others.iter_mut().find(|(s, _)| s.wl_surface() == surface.wl_surface()) {
                entry.1 = Some((w.max(1), h.max(1)));
            }
        }
        self.dirty = true;
    }
}

impl LayerShellHandler for Runner {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        self.exit = true;
    }

    fn configure(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface, configure: LayerSurfaceConfigure, _: u32) {
        let (w, h) = configure.new_size;
        if w > 0 && h > 0 {
            self.size = (w, h);
        }
        self.configured = true;
        self.dirty = true;
    }
}

impl SeatHandler for Runner {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
    fn new_capability(&mut self, _: &Connection, qh: &QueueHandle<Self>, seat: wl_seat::WlSeat, capability: Capability) {
        if capability == Capability::Pointer && self.pointer.is_none() {
            self.pointer = self.seat_state.get_pointer(qh, &seat).ok();
        }
        if capability == Capability::Keyboard && self.keyboard.is_none() {
            self.keyboard = self.seat_state.get_keyboard(qh, &seat, None).ok();
        }
        if self.data_device.is_none() {
            self.data_device = self.data_devices.as_ref().map(|m| m.get_data_device(qh, &seat));
        }
    }
    fn remove_capability(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat, capability: Capability) {
        if capability == Capability::Pointer {
            if let Some(p) = self.pointer.take() {
                p.release();
            }
        }
        if capability == Capability::Keyboard {
            if let Some(k) = self.keyboard.take() {
                k.release();
            }
        }
    }
    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl PointerHandler for Runner {
    fn pointer_frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_pointer::WlPointer, events: &[PointerEvent]) {
        const BTN_LEFT: u32 = 0x110;
        for event in events {
            if &event.surface != self.shell.wl_surface() {
                continue;
            }
            let pos = (event.position.0 as f32, event.position.1 as f32);
            let input = &mut self.ctx.input;
            match event.kind {
                PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => input.pointer = Some(pos),
                PointerEventKind::Leave { .. } => input.pointer = None,
                PointerEventKind::Press { button, serial, .. } if button == BTN_LEFT => {
                    self.serial = serial;
                    input.pointer = Some(pos);
                    input.pressed = true;
                    input.down = true;
                }
                PointerEventKind::Press { button, .. } if button == 0x111 => {
                    input.pointer = Some(pos);
                    input.secondary = true;
                }
                PointerEventKind::Release { button, .. } if button == BTN_LEFT => {
                    input.pointer = Some(pos);
                    input.released = true;
                    input.down = false;
                }
                PointerEventKind::Axis { vertical, .. } => {
                    let steps = vertical.value120 as f32 / 120.0;
                    input.scroll += if steps != 0.0 { steps * 40.0 } else if vertical.discrete != 0 { vertical.discrete as f32 * 40.0 } else { vertical.absolute as f32 };
                }
                _ => continue,
            }
            self.dirty = true;
        }
    }
}

impl KeyboardHandler for Runner {
    fn enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: &wl_surface::WlSurface, _: u32, _: &[u32], _: &[Keysym]) {
        self.ctx.input.focused = true;
        self.dirty = true;
    }
    fn leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: &wl_surface::WlSurface, _: u32) {
        self.ctx.input.focused = false;
        // Sem foco, nenhuma tecla continua "segurada" para o app.
        self.ctx.input.ctrl = false;
        self.ctx.input.shift = false;
        self.ctx.input.alt = false;
        self.ctx.input.logo = false;
        self.dirty = true;
    }

    fn press_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, serial: u32, event: KeyEvent) {
        self.serial = serial;
        self.key(event);
    }

    fn repeat_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, event: KeyEvent) {
        self.key(event);
    }

    fn release_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, _: KeyEvent) {}

    fn update_modifiers(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, modifiers: Modifiers, _: RawModifiers, _: u32) {
        self.ctx.input.ctrl = modifiers.ctrl;
        self.ctx.input.shift = modifiers.shift;
        self.ctx.input.alt = modifiers.alt;
        self.ctx.input.logo = modifiers.logo;
    }
}

impl Runner {
    fn key(&mut self, event: KeyEvent) {
        let input = &mut self.ctx.input;
        input.presses.push(KeyPress {
            keysym: event.keysym.raw(),
            text: event.utf8.clone(),
            ctrl: input.ctrl,
            shift: input.shift,
            alt: input.alt,
            logo: input.logo,
        });
        let key = match event.keysym {
            Keysym::BackSpace => Some(Key::Backspace),
            Keysym::Return | Keysym::KP_Enter => Some(Key::Enter),
            Keysym::Escape => Some(Key::Escape),
            Keysym::Tab => Some(Key::Tab),
            Keysym::Left => Some(Key::Left),
            Keysym::Right => Some(Key::Right),
            Keysym::Up => Some(Key::Up),
            Keysym::Down => Some(Key::Down),
            Keysym::Delete => Some(Key::Delete),
            Keysym::Home | Keysym::KP_Home => Some(Key::Home),
            Keysym::End | Keysym::KP_End => Some(Key::End),
            Keysym::Page_Up | Keysym::KP_Page_Up => Some(Key::PageUp),
            Keysym::Page_Down | Keysym::KP_Page_Down => Some(Key::PageDown),
            _ => None,
        };
        match key {
            Some(k) => {
                input.keys.push(k);
                input.key_mods.push((input.ctrl, input.shift));
            }
            None if input.ctrl => {
                // Ctrl+letra vira atalho, não texto. Com Ctrl, o texto da tecla é um
                // caractere de controle (Ctrl+D = 0x04): vale o símbolo da tecla.
                if let Some(c) = event.keysym.key_char().filter(|c| c.is_alphanumeric()) {
                    input.shortcuts.push(c.to_lowercase().to_string());
                    input.shortcut_shift.push(input.shift);
                }
            }
            None => {
                if let Some(text) = event.utf8.filter(|t| t.chars().all(|c| !c.is_control())) {
                    input.text.push_str(&text);
                }
            }
        }
        self.dirty = true;
    }
}

/// Tipos de texto oferecidos e aceitos na área de transferência, do preferido ao último.
const TEXT_MIMES: [&str; 4] = ["text/plain;charset=utf-8", "UTF8_STRING", "text/plain", "TEXT"];

impl Runner {
    /// Atende o que o quadro pediu: pôr texto na área de transferência (Ctrl+C/X) ou
    /// ler o que está nela (Ctrl+V). A leitura é feita no laço de eventos, sem travar:
    /// quem tem o texto pode ser este mesmo app.
    fn clipboard(&mut self) {
        if let Some(text) = self.ctx.clipboard_out.take() {
            if let (Some(manager), Some(device)) = (&self.data_devices, &self.data_device) {
                let source = manager.create_copy_paste_source(&self.qh, TEXT_MIMES);
                source.set_selection(device, self.serial);
                tracing::debug!("área de transferência: copiado ({} bytes, serial {})", text.len(), self.serial);
                self.copied = Some((source, text));
            } else {
                tracing::debug!("área de transferência: sem dispositivo de dados");
            }
        }
        if !std::mem::take(&mut self.ctx.paste_requested) {
            return;
        }
        if let Some((_, text)) = &self.copied {
            self.ctx.input.paste = Some(text.clone());
            self.dirty = true;
            return;
        }
        let Some(offer) = self.data_device.as_ref().and_then(|d| d.data().selection_offer()) else {
            tracing::debug!("área de transferência: nada para colar");
            return;
        };
        let Some(mime) = offer.with_mime_types(|offered| TEXT_MIMES.iter().find(|m| offered.iter().any(|o| o == *m)).map(|m| m.to_string())) else {
            return;
        };
        tracing::debug!("área de transferência: colando ({mime})");
        let Ok(pipe) = offer.receive(mime) else {
            return;
        };
        let mut bytes = Vec::new();
        let _ = self.loop_handle.insert_source(pipe, move |_, file, runner| {
            use std::io::Read;
            let mut chunk = [0u8; 4096];
            loop {
                // SAFETY: o arquivo não é fechado aqui (o calloop é dono dele).
                match unsafe { file.get_mut() }.read(&mut chunk) {
                    Ok(0) => {
                        tracing::debug!("área de transferência: colado ({} bytes)", bytes.len());
                        runner.ctx.input.paste = Some(String::from_utf8_lossy(&bytes).into_owned());
                        runner.dirty = true;
                        return PostAction::Remove;
                    }
                    Ok(n) => bytes.extend_from_slice(&chunk[..n]),
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => return PostAction::Continue,
                    Err(_) => return PostAction::Remove,
                }
            }
        });
    }
}

impl DataDeviceHandler for Runner {
    fn enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice, _: f64, _: f64, _: &wl_surface::WlSurface) {}
    fn leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {}
    fn motion(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice, _: f64, _: f64) {}
    fn selection(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {}
    fn drop_performed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {}
}

impl DataOfferHandler for Runner {
    fn source_actions(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &mut DragOffer, _: DndAction) {}
    fn selected_action(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &mut DragOffer, _: DndAction) {}
}

impl DataSourceHandler for Runner {
    fn accept_mime(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource, _: Option<String>) {}

    /// Outro app (ou este) quer o texto copiado: escreve no canal que ele mandou.
    fn send_request(&mut self, _: &Connection, _: &QueueHandle<Self>, source: &WlDataSource, _: String, mut fd: WritePipe) {
        use std::io::Write;
        if let Some((copied, text)) = &self.copied {
            if copied.inner() == source {
                tracing::debug!("área de transferência: entregando o texto copiado");
                let _ = fd.write_all(text.as_bytes());
            }
        }
    }

    /// Outro app copiou algo: o texto deste deixa de ser o da área de transferência.
    fn cancelled(&mut self, _: &Connection, _: &QueueHandle<Self>, source: &WlDataSource) {
        if self.copied.as_ref().is_some_and(|(copied, _)| copied.inner() == source) {
            self.copied = None;
        }
    }

    fn dnd_dropped(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource) {}
    fn dnd_finished(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource) {}
    fn action(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource, _: DndAction) {}
}

impl ShmHandler for Runner {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

delegate_registry!(Runner);
smithay_client_toolkit::delegate_dispatch2!(Runner);

impl ProvidesRegistryState for Runner {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }

    registry_handlers![OutputState, SeatState];
}

/// Retângulo (x, y, largura, altura) que cobre os pixels diferentes entre dois quadros
/// do mesmo tamanho; `None` se o tamanho mudou.
fn changed_area(old: &Pixmap, new: &Pixmap) -> Option<(u32, u32, u32, u32)> {
    if old.width() != new.width() || old.height() != new.height() {
        return None;
    }
    let (w, h) = (new.width() as usize, new.height() as usize);
    let stride = w * 4;
    let (a, b) = (old.data(), new.data());
    let row = |y: usize| &a[y * stride..(y + 1) * stride] != &b[y * stride..(y + 1) * stride];
    let top = (0..h).find(|&y| row(y))?;
    let bottom = (top..h).rev().find(|&y| row(y)).unwrap_or(top);
    let (mut left, mut right) = (w, 0);
    for y in top..=bottom {
        let (ra, rb) = (&a[y * stride..(y + 1) * stride], &b[y * stride..(y + 1) * stride]);
        if ra == rb {
            continue;
        }
        if let Some(x) = (0..left.min(w)).find(|&x| ra[x * 4..x * 4 + 4] != rb[x * 4..x * 4 + 4]) {
            left = x;
        }
        if let Some(x) = (right..w).rev().find(|&x| ra[x * 4..x * 4 + 4] != rb[x * 4..x * 4 + 4]) {
            right = right.max(x);
        }
    }
    let left = left.min(right);
    Some((left as u32, top as u32, (right - left + 1) as u32, (bottom - top + 1) as u32))
}

#[cfg(test)]
mod tests {
    use super::changed_area;
    use tiny_skia::Pixmap;

    #[test]
    fn dano_so_onde_mudou() {
        let old = Pixmap::new(100, 80).unwrap();
        let mut new = old.clone();
        new.fill_rect(tiny_skia::Rect::from_xywh(10.0, 20.0, 5.0, 3.0).unwrap(), &tiny_skia::Paint::default(), tiny_skia::Transform::identity(), None);
        assert_eq!(changed_area(&old, &new), Some((10, 20, 5, 3)));
        assert_eq!(changed_area(&old, &old), None);
        assert_eq!(changed_area(&old, &Pixmap::new(50, 80).unwrap()), None);
    }
}
