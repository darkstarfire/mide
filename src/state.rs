//! Estado global do compositor e os handlers dos protocolos Wayland.
//!
//! Cada protocolo mínimo que o mide anuncia precisa de um estado (`*State`)
//! e de um handler que o implementa. As macros `delegate_*!` ligam os dois.

use std::{error::Error, time::{Duration, Instant}};

use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode;

use crate::{config::Config, decorations::BAR_HEIGHT};
use smithay::wayland::compositor::with_states;

use smithay::{
    backend::renderer::utils::on_commit_buffer_handler,
    delegate_compositor, delegate_data_device, delegate_layer_shell, delegate_output,
    delegate_seat, delegate_shm, delegate_virtual_keyboard_manager, delegate_xdg_decoration,
    delegate_xdg_shell,
    desktop::{
        layer_map_for_output, LayerSurface as DesktopLayerSurface, PopupKind, PopupManager,
        Space, Window,
    },
    output::Output,
    input::{keyboard::XkbConfig, Seat, SeatHandler, SeatState},
    reexports::wayland_server::{
        backend::{ClientData, ClientId, DisconnectReason},
        protocol::{wl_buffer, wl_output::WlOutput, wl_seat, wl_surface::WlSurface},
        Client, DisplayHandle, Resource,
    },
    utils::{Logical, Point, Serial, Size},
    wayland::{
        buffer::BufferHandler,
        compositor::{CompositorClientState, CompositorHandler, CompositorState},
        output::OutputHandler,
        virtual_keyboard::VirtualKeyboardManagerState,
        shell::wlr_layer::{
            Layer, LayerSurface, WlrLayerShellHandler, WlrLayerShellState,
        },
        shell::xdg::decoration::{XdgDecorationHandler, XdgDecorationState},
        selection::{
            data_device::{
                set_data_device_focus, ClientDndGrabHandler, DataDeviceHandler,
                DataDeviceState, ServerDndGrabHandler,
            },
            SelectionHandler,
        },
        shell::xdg::{
            PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
        },
        shm::{ShmHandler, ShmState},
    },
};

/// Estado de uma camada: já recebeu a configuração inicial, e está aparecendo (com
/// imagem).
#[derive(Default)]
struct LayerFlags {
    configured: std::sync::atomic::AtomicBool,
    mapped: std::sync::atomic::AtomicBool,
}

/// Estado de cada cliente Wayland conectado.
#[derive(Default)]
pub struct ClientState {
    pub compositor_state: CompositorClientState,
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {
        tracing::info!("cliente conectado");
    }

    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {
        tracing::info!("cliente desconectado");
    }
}

/// Redimensionamento em andamento: a janela segue o ponteiro pela borda ou canto pressionado.
#[derive(Debug, Clone)]
pub struct ResizeGrab {
    pub window: Window,
    pub edges: crate::decorations::Edges,
    pub start_pointer: Point<f64, Logical>,
    pub start_loc: Point<i32, Logical>,
    pub start_size: Size<i32, Logical>,
}

// Durações das animações: todas em `mide::motion` (a tabela do `docs/design.md`).
/// Maximizar, restaurar, metade da tela.
pub const ANIMATION_DURATION: Duration = mide::motion::WINDOW_MOVE;
/// Efeito lâmpada (minimizar e restaurar): mais longo para ser legível.
pub const GENIE_DURATION: Duration = mide::motion::GENIE;
/// Abrir e fechar o launcher.
pub const LAUNCHER_OPEN: Duration = mide::motion::LAUNCHER_OPEN;
pub const LAUNCHER_CLOSE: Duration = mide::motion::LAUNCHER_CLOSE;

/// Animação de maximizar: a janela vai da posição e do tamanho antigos aos novos.
#[derive(Debug, Clone)]
pub struct WindowAnimation {
    pub window: Window,
    pub surface: WlSurface,
    pub from_loc: Point<i32, Logical>,
    pub from_size: Size<i32, Logical>,
    pub to_loc: Point<i32, Logical>,
    pub to_size: Size<i32, Logical>,
    /// Zoom do conteúdo no início e no fim (1.0 = tamanho natural).
    pub zoom_from: f32,
    pub zoom_to: f32,
    /// Opacidade no início e no fim.
    pub alpha_from: f32,
    pub alpha_to: f32,
    /// Ao terminar, a janela sai do espaço (minimizar).
    pub hide_at_end: bool,
    /// Usa o efeito lâmpada em vez de escala (minimizar e restaurar).
    pub genie: bool,
    pub duration: Duration,
    pub start: Instant,
}

/// Janela minimizada: fora do espaço, guarda onde estava para voltar.
#[derive(Debug, Clone)]
pub struct MinimizedWindow {
    pub window: Window,
    pub loc: Point<i32, Logical>,
    pub size: Size<i32, Logical>,
}

impl WindowAnimation {
    /// Progresso de 0 a 1: desaceleração cúbica, ou aceleração e desaceleração no efeito lâmpada.
    pub fn eased(&self, now: Instant) -> f32 {
        let t = (now.duration_since(self.start).as_secs_f32() / self.duration.as_secs_f32())
            .clamp(0.0, 1.0);
        if self.genie {
            // Lâmpada: acelera e desacelera suave, para o funil ser visível do começo ao fim.
            mide::motion::ease_in_out(t)
        } else {
            mide::motion::ease_out(t)
        }
    }

    pub fn finished(&self, now: Instant) -> bool {
        now.duration_since(self.start) >= self.duration
    }
}

/// Estado global do compositor.
pub struct State {
    pub display_handle: DisplayHandle,
    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub shm_state: ShmState,
    pub seat_state: SeatState<Self>,
    pub data_device_state: DataDeviceState,
    /// Janelas mapeadas sobre as saídas.
    pub space: Space<Window>,
    /// Início do compositor, usado como referência de tempo para os frame callbacks.
    pub start_time: Instant,
    /// Medição de resposta: última entrada (clique, tecla, roda), quando foi e o
    /// cliente que a recebeu. No próximo commit desse cliente, registra o tempo
    /// (`resposta: ... ms`, em nível debug, lido pelo `tests/perf.sh`).
    /// Regiões de vidro pedidas pelas camadas (`glass` no canal de controle): pelo
    /// nome da camada, retângulos relativos a ela e o raio dos cantos.
    pub glass: std::collections::HashMap<String, Vec<(smithay::utils::Rectangle<i32, smithay::utils::Logical>, f32, f32)>>,
    pub input_mark: Option<(Instant, &'static str, smithay::reexports::wayland_server::backend::ClientId)>,
    /// Posição atual do cursor, em coordenadas globais.
    pub pointer_location: Point<f64, Logical>,
    /// Onde o botão esquerdo foi pressionado pela última vez (início de arrastos).
    pub press_location: Option<Point<f64, Logical>>,
    /// Janela sendo arrastada pelo cursor, enquanto o botão esquerdo está pressionado.
    pub move_grab: Option<MoveGrab>,
    /// Posição de cada janela antes de maximizar, para restaurar depois.
    pub restore_positions: Vec<(WlSurface, Point<i32, Logical>)>,
    /// Tamanho de cada janela antes de maximizar, para pedir de volta ao restaurar.
    pub restore_sizes: Vec<(WlSurface, Size<i32, Logical>)>,
    /// Janelas numa metade da tela (Super+← e Super+→).
    pub tiled: Vec<WlSurface>,
    /// Forma do ponteiro (redimensionar, padrão); o backend a mostra.
    pub cursor: crate::input::CursorShape,
    /// Redimensionamento pela borda em andamento.
    pub resize_grab: Option<ResizeGrab>,
    /// Animações em andamento (ex.: maximizar).
    pub animations: Vec<WindowAnimation>,
    /// Pedidos de fechar feitos pelo mide (janela e quando): enquanto o app não fecha,
    /// o compositor fotografa a janela para a transição de fechar.
    pub close_requests: Vec<(u64, Instant)>,
    /// Janelas que acabaram de fechar: a foto delas esmaece (número, moldura, início).
    pub ghosts: Vec<(u64, smithay::utils::Rectangle<i32, Logical>, Instant)>,
    /// Janelas minimizadas, da mais antiga para a mais recente.
    pub minimized: Vec<MinimizedWindow>,
    /// Teclas no estilo Mac: Ctrl e ⌘ de verdade apertados e a troca feita em cada
    /// tecla apertada (o soltar usa a mesma).
    pub mac_keys: MacKeys,
    /// Janelas que o app escondeu sem fechar (buffer nulo; o Chromium faz isso ao
    /// continuar em segundo plano), com a posição onde estavam. Voltam quando o app
    /// desenhar de novo.
    pub hidden: Vec<(Window, Point<i32, Logical>)>,
    /// Janelas que já mostraram um quadro (as outras ainda estão abrindo).
    pub shown: std::collections::HashSet<u64>,
    /// Ícones da dock informados por ela: ids do app e retângulo relativo à dock.
    pub dock_icons: Vec<(Vec<String>, smithay::utils::Rectangle<i32, Logical>)>,
    /// Prévias das janelas de um app, pedidas pela dock (ponteiro sobre o ícone).
    pub preview: Option<crate::preview::Preview>,
    /// Apps abertos pela dock (ids e quando): a primeira janela deles sai do ícone
    /// com o efeito lâmpada.
    pub pending_open: Vec<(Vec<String>, Instant)>,
    /// Animação do launcher feita aqui, na GPU: início e se está abrindo (ou fechando).
    pub launcher_anim: Option<(Instant, bool)>,
    /// O launcher já mostrou o primeiro quadro (a animação de abrir começa nele).
    pub launcher_seen: bool,
    /// O launcher terminou de esmaecer ao fechar e fica invisível até a camada sumir
    /// (sem isso, ele reaparecia inteiro por alguns quadros: uma piscada).
    pub launcher_gone: bool,
    /// Algo pode ter mudado na tela (commit de um cliente, comando, entrada): pede um
    /// quadro. Parado, sem nada mudando, o compositor não desenha.
    pub redraw: bool,
    /// Acorda o laço de desenho na hora (`wake`), sem esperar o relógio.
    pub waker: Option<smithay::reexports::calloop::ping::Ping>,
    /// Telas conhecidas pelo backend (conectadas, ligadas ou não), para as
    /// Configurações (`outputs` no canal de controle).
    pub outputs_info: Vec<OutputInfo>,
    /// Pede ao backend para refazer as telas pela configuração (`outputs apply`).
    pub outputs_waker: Option<smithay::reexports::calloop::ping::Ping>,
    /// Pede ao backend para reaplicar o mouse e o trackpad (`[input]`, depois do `reload`).
    pub input_waker: Option<smithay::reexports::calloop::ping::Ping>,
    /// Tela onde estão a barra, a dock e a área de trabalho (para reabri-los quando a
    /// tela principal muda).
    pub shell_output: Option<String>,
    /// Captura de tela pedida (`screenshot <arquivo>`): o próximo quadro da tela
    /// principal é salvo em PNG nesse arquivo, sem o cursor.
    pub screenshot_request: Option<std::path::PathBuf>,
    /// Menus que acabaram de aparecer (camadas de menu e popups), com o momento:
    /// entram esmaecendo (`[effects] menu_animation`).
    pub appearing: Vec<(WlSurface, Instant)>,
    /// Cursor pedido pelo app sob o ponteiro (desenhado pelo mide no DRM).
    pub cursor_status: smithay::input::pointer::CursorImageStatus,
    /// Sessão do backend DRM (trocar de console com Ctrl+Alt+F1…F12).
    pub session: Option<smithay::backend::session::libseat::LibSeatSession>,
    /// Gravação de atalho em andamento (tela Teclado das Configurações).
    pub shortcut_capture: Option<ShortcutCapture>,
    /// Autoteste: finge o Ctrl pressionado (o teste não tem teclado real).
    pub test_ctrl: bool,
    /// Popups (menus, listas) rastreados até serem mapeados e destruídos.
    pub popups: PopupManager,
    /// Superfícies de camada (barra, dock, buscador) do protocolo wlr-layer-shell.
    pub layer_shell_state: WlrLayerShellState,
    pub session_lock_state: smithay::wayland::session_lock::SessionLockManagerState,
    /// Bloqueio de tela (`crate::lock`).
    pub lock: crate::lock::Lock,
    /// Última entrada do usuário (para bloquear depois do tempo sem uso).
    pub last_activity: Instant,
    /// Bloquear sozinho depois do tempo sem uso (só na sessão de verdade).
    pub idle_lock: bool,
    /// Aviso de "usuário ocioso" para os programas (`ext-idle-notify-v1`).
    pub idle_notifier_state: smithay::wayland::idle_notify::IdleNotifierState<State>,
    /// Programas que pedem para a tela não apagar nem bloquear (vídeo tocando). O
    /// estado só precisa existir (o protocolo fica anunciado).
    #[allow(dead_code)]
    pub idle_inhibit_state: smithay::wayland::idle_inhibit::IdleInhibitManagerState,
    pub inhibitors: Vec<WlSurface>,
    /// A tela está apagada (DPMS desligado no DRM; preto nos outros backends).
    pub blanked: bool,
    /// Já pediu para suspender por tempo sem uso (`[power] suspend_minutes`); volta a
    /// valer depois de qualquer uso.
    pub idle_suspended: bool,
    /// O computador voltou da suspensão: as telas refazem o estado no próximo quadro.
    pub resumed: bool,
    /// Pedido para a barra (lido por ela com `bar_request`), como a confirmação de
    /// desligar do botão de ligar.
    pub bar_request: Option<&'static str>,
    /// Capturas de tela pedidas pelos apps (`wlr-screencopy`).
    pub screencopy: crate::screencopy::Screencopy,
    /// Pedidos dos apps para trazer uma janela para a frente (`xdg-activation`).
    pub xdg_activation_state: smithay::wayland::xdg_activation::XdgActivationState,
    /// Estado da decoração de servidor (barra de título). Guardado para manter o global vivo.
    #[allow(dead_code)]
    pub decoration_state: XdgDecorationState,
    /// Verdadeiro enquanto o botão esquerdo está pressionado sobre uma barra de título.
    pub title_press: bool,
    /// Último clique numa barra de título, para detectar o duplo clique.
    pub last_title_click: Option<(Window, Instant)>,
    /// Configuração carregada na inicialização (atalhos de teclado).
    pub config: Config,
    /// Pedido para encerrar o event loop, feito por atalho.
    pub quit_requested: bool,
    /// Reiniciar o compositor com o programa novo (`restart all`), sem sair da sessão.
    pub reexec_requested: bool,
    pub seat: Seat<Self>,
}

/// Gravação de atalho pedida pelas Configurações (`shortcuts capture`): a próxima
/// combinação apertada fica em `combo` (ou `cancel`, com Esc), sem disparar atalhos
/// nem chegar ao app. Expira sozinha, para o teclado não ficar preso se as
/// Configurações fecharem no meio.
pub struct ShortcutCapture {
    pub started: Instant,
    pub combo: Option<String>,
}

impl ShortcutCapture {
    pub const TIMEOUT: Duration = Duration::from_secs(20);

    pub fn expired(&self) -> bool {
        self.started.elapsed() > Self::TIMEOUT
    }
}

/// Arrasto de janela: guarda a janela e a distância entre o cursor e a origem dela.
pub struct MoveGrab {
    pub window: Window,
    pub offset: Point<f64, Logical>,
}

impl State {
    pub fn new(display_handle: DisplayHandle, config: Config, loop_handle: smithay::reexports::calloop::LoopHandle<'static, State>) -> Result<Self, Box<dyn Error>> {
        // Teclado virtual: deixa ferramentas como o wtype digitarem nos testes automáticos.
        VirtualKeyboardManagerState::new::<Self, _>(&display_handle, |_| true);
        // Pedidos de recorte e escala de superfície (wp_viewporter) e posição lógica das
        // saídas (xdg-output): o XWayland (xwayland-satellite) precisa dos dois.
        smithay::wayland::viewporter::ViewporterState::new::<Self>(&display_handle);
        smithay::wayland::output::OutputManagerState::new_with_xdg_output::<Self>(&display_handle);
        // Captura de tela pelos apps (OBS e outros, pelo portal wlr).
        crate::screencopy::init(&display_handle);
        // Onde está o menu de cada janela (apps GTK e Qt no Wayland).
        crate::app_menus::init(&display_handle);
        // Escala exata da tela para cada janela (nítidas em 1,25, 1,5...).
        smithay::wayland::fractional_scale::FractionalScaleManagerState::new::<Self>(&display_handle);
        let xdg_activation_state = smithay::wayland::xdg_activation::XdgActivationState::new::<Self>(&display_handle);
        let compositor_state = CompositorState::new::<Self>(&display_handle);
        let xdg_shell_state = XdgShellState::new::<Self>(&display_handle);
        let shm_state = ShmState::new::<Self>(&display_handle, vec![]);
        let data_device_state = DataDeviceState::new::<Self>(&display_handle);
        let layer_shell_state = WlrLayerShellState::new::<Self>(&display_handle);
        let session_lock_state = smithay::wayland::session_lock::SessionLockManagerState::new::<Self, _>(&display_handle, |_| true);
        let idle_notifier_state = smithay::wayland::idle_notify::IdleNotifierState::new(&display_handle, loop_handle);
        let idle_inhibit_state = smithay::wayland::idle_inhibit::IdleInhibitManagerState::new::<Self>(&display_handle);
        let decoration_state = XdgDecorationState::new::<Self>(&display_handle);

        let mut seat_state = SeatState::new();
        let mut seat = seat_state.new_wl_seat(&display_handle, "seat0");
        // Layout do sistema (ABNT2 etc.), também sem desktop anfitrião (DRM).
        let kb = config.keyboard.effective();
        if !kb.layout.is_empty() {
            tracing::info!("teclado: layout {} {}", kb.layout, kb.variant);
        }
        let xkb = XkbConfig { rules: "", model: &kb.model, layout: &kb.layout, variant: &kb.variant, options: kb.options.clone() };
        seat.add_keyboard(xkb, 200, 25)?;
        seat.add_pointer();

        Ok(Self {
            display_handle,
            compositor_state,
            xdg_shell_state,
            shm_state,
            seat_state,
            data_device_state,
            space: Space::default(),
            start_time: Instant::now(),
            input_mark: None,
            glass: std::collections::HashMap::new(),
            pointer_location: (0.0, 0.0).into(),
            press_location: None,
            move_grab: None,
            restore_positions: Vec::new(),
            restore_sizes: Vec::new(),
            tiled: Vec::new(),
            cursor: Default::default(),
            animations: Vec::new(),
            close_requests: Vec::new(),
            ghosts: Vec::new(),
            minimized: Vec::new(),
            hidden: Vec::new(),
            mac_keys: MacKeys::default(),
            shown: Default::default(),
            dock_icons: Vec::new(),
            preview: None,
            pending_open: Vec::new(),
            launcher_anim: None,
            launcher_seen: false,
            launcher_gone: false,
            redraw: true,
            waker: None,
            outputs_info: Vec::new(),
            screenshot_request: None,
            appearing: Vec::new(),
            outputs_waker: None,
            input_waker: None,
            shell_output: None,
            cursor_status: smithay::input::pointer::CursorImageStatus::default_named(),
            session: None,
            shortcut_capture: None,
            test_ctrl: false,
            resize_grab: None,
            popups: PopupManager::default(),
            layer_shell_state,
            session_lock_state,
            lock: Default::default(),
            last_activity: Instant::now(),
            idle_lock: false,
            idle_notifier_state,
            idle_inhibit_state,
            inhibitors: Vec::new(),
            blanked: false,
            idle_suspended: false,
            resumed: false,
            bar_request: None,
            screencopy: Default::default(),
            xdg_activation_state,
            decoration_state,
            title_press: false,
            last_title_click: None,
            config,
            quit_requested: false,
            reexec_requested: false,
            seat,
        })
    }

    /// Camada (barra, buscador) cuja superfície é `surface`, se existir.
    pub fn layer_for(&self, surface: &WlSurface) -> Option<DesktopLayerSurface> {
        self.space.outputs().find_map(|output| {
            layer_map_for_output(output)
                .layers()
                .find(|l| l.wl_surface() == surface)
                .cloned()
        })
    }

    /// Reorganiza as camadas da saída quando uma camada faz commit. Sem isso, a
    /// configuração inicial da camada nunca é enviada ao cliente.
    pub fn arrange_layer_if_any(&mut self, surface: &WlSurface) {
        use std::sync::atomic::Ordering;
        for output in self.space.outputs().cloned().collect::<Vec<_>>() {
            let before = self.usable_area();
            let mut map = layer_map_for_output(&output);
            let Some(layer) = map.layers().find(|l| l.wl_surface() == surface).cloned() else {
                continue;
            };
            map.arrange();
            drop(map);
            // A barra pode chegar depois das primeiras janelas: elas descem para
            // baixo dela, e as maximizadas se ajustam à área nova.
            if self.usable_area() != before {
                self.fit_windows_to_area();
            }
            // A configuração inicial vem uma vez, em resposta ao primeiro commit.
            // O arrange só envia configurações depois dessa. Uma camada que se escondeu
            // (commit sem imagem depois de aparecer) volta a esperar a configuração
            // inicial: o próximo commit dela recebe uma (o terminal de atalho some e
            // volta assim).
            layer.user_data().insert_if_missing_threadsafe(LayerFlags::default);
            let Some(flags) = layer.user_data().get::<LayerFlags>() else {
                return;
            };
            let has_buffer = smithay::backend::renderer::utils::with_renderer_surface_state(surface, |s| s.buffer().is_some()).unwrap_or(false);
            if has_buffer {
                flags.mapped.store(true, Ordering::Relaxed);
            } else if flags.mapped.swap(false, Ordering::Relaxed) {
                flags.configured.store(false, Ordering::Relaxed);
                return;
            }
            if !flags.configured.swap(true, Ordering::Relaxed) {
                layer.layer_surface().send_configure();
            }
            return;
        }
    }

    /// Dá o foco do teclado a uma camada que aceita foco. Usado no clique e, para as
    /// camadas que exigem o teclado, no commit (antes disso o cliente ainda não está
    /// pronto para receber teclas).
    pub fn focus_layer_if_allowed(&mut self, surface: &WlSurface) {
        let Some(layer) = self.layer_for(surface) else {
            return;
        };
        if !layer.can_receive_keyboard_focus() {
            return;
        }
        let Some(keyboard) = self.seat.get_keyboard() else {
            return;
        };
        if keyboard.current_focus().as_ref() == Some(surface) {
            return;
        }
        tracing::info!("foco do teclado: camada");
        let serial = smithay::utils::SERIAL_COUNTER.next_serial();
        keyboard.set_focus(self, Some(surface.clone()), serial);
    }

    /// Canto superior esquerdo da área visível da janela. No `Space` do Smithay, a
    /// posição da janela já é esse canto; a superfície fica em posição menos o
    /// deslocamento da geometria (`render_location`). A moldura e a barra se alinham aqui.
    pub fn visible_origin(&self, window: &Window) -> Option<Point<i32, Logical>> {
        self.space.element_location(window)
    }

    /// Marca uma entrada entregue a `surface` para medir a resposta do app.
    /// O usuário mexeu (tecla, clique, movimento): conta o tempo sem uso de novo,
    /// avisa os programas e acende a tela apagada.
    pub fn activity(&mut self) {
        self.last_activity = Instant::now();
        self.idle_suspended = false;
        let seat = self.seat.clone();
        self.idle_notifier_state.notify_activity(&seat);
        if self.blanked {
            tracing::info!("tela acesa");
            self.blanked = false;
            // Acorda o desenho sempre: o `wake` só avisa quando não havia quadro pedido, e
            // com a tela apagada um pedido antigo pode ter ficado (sem vblank, ninguém o
            // atenderia e a tela não acenderia mais).
            self.redraw = true;
            if let Some(waker) = &self.waker {
                waker.ping();
            }
        }
    }

    pub fn mark_input(&mut self, kind: &'static str, surface: Option<WlSurface>) {
        use smithay::reexports::wayland_server::Resource;
        self.activity();
        if let Some(client) = surface.and_then(|s| s.client()) {
            self.input_mark = Some((Instant::now(), kind, client.id()));
        }
    }

    /// Animação em andamento da superfície, se houver.
    pub fn animation_for(&self, surface: &WlSurface) -> Option<&WindowAnimation> {
        self.animations.iter().find(|a| &a.surface == surface)
    }

    /// Janela mapeada cuja superfície raiz é `surface`, se existir.
    pub fn window_for(&self, surface: &WlSurface) -> Option<Window> {
        self.space
            .elements()
            .find(|w| w.toplevel().is_some_and(|t| t.wl_surface() == surface))
            .cloned()
    }
}

impl CompositorHandler for State {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        &client.get_data::<ClientState>().unwrap().compositor_state
    }

    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);
        if let Some((at, kind, client)) = &self.input_mark {
            use smithay::reexports::wayland_server::Resource;
            if surface.client().is_some_and(|c| &c.id() == client) {
                tracing::debug!("resposta ({kind}): {:.1} ms", at.elapsed().as_secs_f32() * 1000.0);
                self.input_mark = None;
            }
        }
        self.popups.commit(surface);
        self.arrange_layer_if_any(surface);
        // Só camadas que exigem o teclado (launcher, buscador, menus) pegam o foco
        // sozinhas. As que só querem o teclado quando clicadas (área de trabalho) não:
        // senão, a cada redesenho, tirariam o foco da janela em que se está digitando.
        if self.layer_for(surface).is_some_and(|l| {
            l.cached_state().keyboard_interactivity == smithay::wayland::shell::wlr_layer::KeyboardInteractivity::Exclusive
        }) {
            self.focus_layer_if_allowed(surface);
        }
        // Primeiro quadro do launcher: começa a animação de abrir.
        if !self.launcher_seen && self.layer_for(surface).is_some_and(|l| l.namespace() == "mide-launcher") {
            let has_buffer = smithay::backend::renderer::utils::with_renderer_surface_state(surface, |s| s.buffer().is_some()).unwrap_or(false);
            if has_buffer {
                self.launcher_seen = true;
                self.launcher_gone = false;
                if self.config.effects.animations {
                    self.launcher_anim = Some((Instant::now(), true));
                }
            }
        }
        self.follow_toplevel_mapping(surface);
        if let Some(window) = self.window_for(surface) {
            window.on_commit();
            self.open_animation_if_launched(&window);
            self.offer_clipboard_on_first_frame(surface);
        }
    }
}

impl State {
    /// Janela escondida e mostrada de novo pelo app. Pelo xdg-shell, um buffer nulo
    /// esconde a janela sem fechá-la, e para voltar o app refaz o começo (commit sem
    /// buffer, configure do compositor, quadro). Sem o configure, ele espera para sempre.
    fn follow_toplevel_mapping(&mut self, surface: &WlSurface) {
        let has_buffer = smithay::backend::renderer::utils::with_renderer_surface_state(surface, |s| s.buffer().is_some()).unwrap_or(false);
        if let Some(window) = self.window_for(surface) {
            let id = window_id(&window);
            if has_buffer {
                self.shown.insert(id);
            } else if self.shown.remove(&id) {
                tracing::info!("janela escondida pelo app (continua aberta)");
                let loc = self.space.element_location(&window).unwrap_or_default();
                self.space.unmap_elem(&window);
                self.hidden.push((window.clone(), loc));
                // O teclado vai para a janela de cima.
                let top = self.space.elements().last().cloned();
                let serial = smithay::utils::SERIAL_COUNTER.next_serial();
                match top {
                    Some(top) => crate::input::focus_window(self, &top, serial),
                    None => {
                        if let Some(keyboard) = self.seat.get_keyboard() {
                            keyboard.set_focus(self, None, serial);
                        }
                    }
                }
            }
            if let Some(toplevel) = window.toplevel() {
                if !toplevel.is_initial_configure_sent() {
                    toplevel.send_configure();
                }
            }
            return;
        }
        let Some(i) = self.hidden.iter().position(|(w, _)| w.toplevel().is_some_and(|t| t.wl_surface() == surface)) else {
            return;
        };
        let window = self.hidden[i].0.clone();
        if let Some(toplevel) = window.toplevel() {
            if !toplevel.is_initial_configure_sent() {
                toplevel.send_configure();
            }
        }
        if has_buffer {
            let (window, loc) = self.hidden.remove(i);
            tracing::info!("janela mostrada de novo pelo app");
            self.shown.insert(window_id(&window));
            self.space.map_element(window.clone(), loc, true);
            let serial = smithay::utils::SERIAL_COUNTER.next_serial();
            crate::input::focus_window(self, &window, serial);
            self.wake();
        }
    }
}

/// Teclas de verdade no estilo Mac (veja `input::mac_map`).
#[derive(Default)]
pub struct MacKeys {
    pub ctrl: bool,
    pub command: bool,
    pub pressed: std::collections::HashMap<u32, u32>,
}

/// Marca da janela que já mostrou o primeiro quadro (a área de transferência já foi
/// oferecida a ela).
struct ClipboardOffered;

impl State {
    /// A janela nova ganha o foco antes de desenhar, e muitos apps só criam o canal da
    /// área de transferência depois: a seleção oferecida naquela hora se perde e colar
    /// não funcionaria até trocar de janela. No primeiro quadro, se ela ainda está em
    /// foco, a seleção é oferecida de novo.
    fn offer_clipboard_on_first_frame(&mut self, surface: &WlSurface) {
        let has_buffer = smithay::backend::renderer::utils::with_renderer_surface_state(surface, |s| s.buffer().is_some()).unwrap_or(false);
        if !has_buffer {
            return;
        }
        let first = with_states(surface, |states| states.data_map.insert_if_missing_threadsafe(|| ClipboardOffered));
        if !first {
            return;
        }
        let focused = self.seat.get_keyboard().and_then(|k| k.current_focus()).is_some_and(|f| &f == surface);
        if let (true, Ok(client)) = (focused, self.display_handle.get_client(surface.id())) {
            set_data_device_focus(&self.display_handle, &self.seat, None);
            set_data_device_focus(&self.display_handle, &self.seat, Some(client));
        }
    }
}

delegate_compositor!(State);

impl BufferHandler for State {
    fn buffer_destroyed(&mut self, _buffer: &wl_buffer::WlBuffer) {}
}

impl ShmHandler for State {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

delegate_shm!(State);

impl XdgShellHandler for State {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        self.map_new_window(surface);
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        self.window_closed(&surface);
    }

    fn new_popup(&mut self, surface: PopupSurface, positioner: PositionerState) {
        // O popup é posicionado pela geometria que o cliente pediu, relativa ao pai,
        // ajustada para caber na tela (submenus que virariam para fora viram para o
        // outro lado ou deslizam, como o protocolo pede).
        surface.with_pending_state(|s| s.geometry = positioner.get_geometry());
        // Popup de camada (o menu de um ícone da Waybar, por exemplo): nasce sem pai e
        // só depois a camada o adota (`WlrLayerShellHandler::new_popup`), que o termina.
        if surface.get_parent_surface().is_none() {
            return;
        }
        self.unconstrain_popup(&surface, &positioner);
        if self.config.effects.menu_animation {
            self.appearing.push((surface.wl_surface().clone(), Instant::now()));
        }
        if let Err(err) = self.popups.track_popup(PopupKind::Xdg(surface.clone())) {
            tracing::warn!("popup sem pai válido: {err}");
        }
        let _ = surface.send_configure();
    }

    fn grab(&mut self, _surface: PopupSurface, _seat: wl_seat::WlSeat, _serial: Serial) {}

    fn move_request(&mut self, surface: ToplevelSurface, _seat: wl_seat::WlSeat, _serial: Serial) {
        self.begin_move(&surface);
    }

    fn maximize_request(&mut self, surface: ToplevelSurface) {
        self.maximize(&surface);
    }

    fn unmaximize_request(&mut self, surface: ToplevelSurface) {
        self.unmaximize(&surface);
    }

    fn minimize_request(&mut self, surface: ToplevelSurface) {
        self.minimize(surface.wl_surface());
    }

    fn reposition_request(
        &mut self,
        surface: PopupSurface,
        positioner: PositionerState,
        token: u32,
    ) {
        surface.with_pending_state(|s| s.geometry = positioner.get_geometry());
        self.unconstrain_popup(&surface, &positioner);
        surface.send_repositioned(token);
        let _ = surface.send_configure();
    }
}

delegate_xdg_shell!(State);

impl State {
    /// Ajusta a geometria do popup para caber na tela, pelas regras do posicionador
    /// (`constraint_adjustment`: virar, deslizar, redimensionar). O alvo vai nas
    /// coordenadas do pai: tela menos a posição da janela e dos popups acima dele.
    fn unconstrain_popup(&self, surface: &PopupSurface, positioner: &PositionerState) {
        use smithay::desktop::{find_popup_root_surface, get_popup_toplevel_coords};
        let kind = PopupKind::Xdg(surface.clone());
        let Ok(root) = find_popup_root_surface(&kind) else {
            return;
        };
        let Some(window) = self.space.elements().find(|w| w.toplevel().is_some_and(|t| t.wl_surface() == &root)) else {
            return;
        };
        let Some(window_loc) = self.space.element_location(window) else {
            return;
        };
        // Limite: a tela onde a janela está.
        let Some(screen) = self.output_at(window_loc.to_f64()).and_then(|o| self.space.output_geometry(&o)) else {
            return;
        };
        let mut target = screen;
        target.loc -= get_popup_toplevel_coords(&kind);
        target.loc -= window_loc;
        let geometry = positioner.get_unconstrained_geometry(target);
        if geometry != positioner.get_geometry() {
            tracing::debug!("popup ajustado para caber na tela: {:?} → {:?}", positioner.get_geometry(), geometry);
        }
        surface.with_pending_state(|s| s.geometry = geometry);
    }
}

impl SeatHandler for State {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<Self> {
        &mut self.seat_state
    }

    fn focus_changed(&mut self, seat: &Seat<Self>, focused: Option<&WlSurface>) {
        let client = focused.and_then(|s| self.display_handle.get_client(s.id()).ok());
        set_data_device_focus(&self.display_handle, seat, client);
    }

    fn cursor_image(
        &mut self,
        _seat: &Seat<Self>,
        image: smithay::input::pointer::CursorImageStatus,
    ) {
        self.cursor_status = image;
        self.wake();
    }
}

delegate_seat!(State);

/// Uma tela conectada, como as Configurações a mostram.
#[derive(Debug, Clone, PartialEq)]
pub struct OutputInfo {
    /// Conector (`DP-1`, `HDMI-A-1`, `eDP-1`).
    pub name: String,
    /// Fabricante e modelo, quando a tela informa.
    pub description: String,
    /// Modos que a tela aceita: largura, altura e frequência em mHz.
    pub modes: Vec<(i32, i32, i32)>,
    /// Modo em uso (ou o preferido, se desligada).
    pub current: (i32, i32, i32),
    pub position: (i32, i32),
    pub scale: f64,
    pub enabled: bool,
    pub primary: bool,
}

impl OutputInfo {
    /// Linha do comando `outputs`: campos separados por tabulação.
    pub fn line(&self) -> String {
        let modes: Vec<String> = self.modes.iter().map(|(w, h, r)| format!("{w}x{h}@{}", fmt_hz(*r))).collect();
        format!(
            "{}\t{}\t{}x{}@{}\t{},{}\t{}\t{}\t{}\t{}",
            self.name,
            if self.description.is_empty() { "-" } else { &self.description },
            self.current.0,
            self.current.1,
            fmt_hz(self.current.2),
            self.position.0,
            self.position.1,
            self.scale,
            if self.enabled { "ligada" } else { "desligada" },
            if self.primary { "principal" } else { "-" },
            modes.join(" "),
        )
    }
}

/// Frequência em Hz a partir de mHz, sem casas decimais quando é inteira.
pub fn fmt_hz(mhz: i32) -> String {
    if mhz % 1000 == 0 { format!("{}", mhz / 1000) } else { format!("{:.2}", mhz as f64 / 1000.0) }
}

impl SelectionHandler for State {
    /// Texto que o próprio mide pôs na área de transferência (`copy` no canal de
    /// controle): "Copiar caminho" da dock, do Arquivos e da área de trabalho, e o
    /// resultado da busca. Fica com o compositor, então continua lá mesmo que o app
    /// que copiou feche.
    type SelectionUserData = std::sync::Arc<Vec<u8>>;

    fn send_selection(
        &mut self,
        _ty: smithay::wayland::selection::SelectionTarget,
        _mime_type: String,
        fd: std::os::fd::OwnedFd,
        _seat: Seat<Self>,
        bytes: &Self::SelectionUserData,
    ) {
        // Numa thread: quem cola pode demorar a ler, e o compositor não espera.
        let bytes = bytes.clone();
        std::thread::spawn(move || {
            use std::io::Write;
            let _ = std::fs::File::from(fd).write_all(&bytes);
        });
    }
}

impl State {
    /// Põe `text` na área de transferência, oferecido pelo próprio compositor.
    pub fn copy_text(&mut self, text: &str) {
        let mimes = ["text/plain;charset=utf-8", "UTF8_STRING", "text/plain", "TEXT", "STRING"]
            .iter()
            .map(|m| m.to_string())
            .collect();
        smithay::wayland::selection::data_device::set_data_device_selection(
            &self.display_handle,
            &self.seat,
            mimes,
            std::sync::Arc::new(text.as_bytes().to_vec()),
        );
        tracing::info!("área de transferência: {} caractere(s) copiado(s)", text.chars().count());
    }

    /// Põe na área de transferência o conteúdo de um arquivo com o tipo `mime` (uma
    /// imagem da captura de tela, `image/png`).
    pub fn copy_bytes(&mut self, mime: &str, bytes: Vec<u8>) {
        let n = bytes.len();
        smithay::wayland::selection::data_device::set_data_device_selection(
            &self.display_handle,
            &self.seat,
            vec![mime.to_string()],
            std::sync::Arc::new(bytes),
        );
        tracing::info!("área de transferência: {n} bytes de {mime} copiados");
    }
}

impl DataDeviceHandler for State {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.data_device_state
    }
}

impl ClientDndGrabHandler for State {}
impl ServerDndGrabHandler for State {}

delegate_data_device!(State);

delegate_virtual_keyboard_manager!(State);

impl OutputHandler for State {}

impl WlrLayerShellHandler for State {
    fn shell_state(&mut self) -> &mut WlrLayerShellState {
        &mut self.layer_shell_state
    }

    fn new_layer_surface(
        &mut self,
        surface: LayerSurface,
        output: Option<WlOutput>,
        layer: Layer,
        namespace: String,
    ) {
        let output = output
            .as_ref()
            .and_then(Output::from_resource)
            .or_else(|| self.primary_output());
        let Some(output) = output else {
            tracing::warn!("superfície de camada sem saída, ignorada");
            return;
        };

        if self.config.effects.menu_animation && matches!(namespace.as_str(), "mide-bar-menu" | "mide-dock-menu") {
            self.appearing.push((surface.wl_surface().clone(), Instant::now()));
        }
        let desktop_surface = DesktopLayerSurface::new(surface, namespace.clone());
        let mut map = layer_map_for_output(&output);
        if let Err(err) = map.map_layer(&desktop_surface) {
            tracing::warn!("falha ao mapear camada '{namespace}': {err}");
            return;
        }
        map.arrange();
        drop(map);

        tracing::info!("camada '{namespace}' criada ({layer:?})");
    }

    /// Uma camada adotou um popup (menu de um painel de terceiros): posição dentro da
    /// tela da camada, registro e a primeira configuração.
    fn new_popup(&mut self, parent: LayerSurface, popup: PopupSurface) {
        let parent_surface = parent.wl_surface().clone();
        let place = self.space.outputs().find_map(|output| {
            let map = layer_map_for_output(output);
            let layer = map.layers().find(|l| l.wl_surface() == &parent_surface)?.clone();
            let geo = map.layer_geometry(&layer)?;
            Some((self.space.output_geometry(output)?, geo))
        });
        if let Some((screen, layer_geo)) = place {
            // Limite: a tela, relativo à camada (o popup é relativo a ela).
            let mut target = smithay::utils::Rectangle::from_size(screen.size);
            target.loc -= layer_geo.loc;
            let geometry = popup.with_pending_state(|s| s.positioner.get_unconstrained_geometry(target));
            popup.with_pending_state(|s| s.geometry = geometry);
        }
        if self.config.effects.menu_animation {
            self.appearing.push((popup.wl_surface().clone(), Instant::now()));
        }
        if let Err(err) = self.popups.track_popup(PopupKind::Xdg(popup.clone())) {
            tracing::warn!("popup de camada não registrado: {err}");
        }
        let _ = popup.send_configure();
        tracing::info!("popup de camada aberto");
    }

    fn layer_destroyed(&mut self, surface: LayerSurface) {
        let wl = surface.wl_surface().clone();
        for output in self.space.outputs().cloned().collect::<Vec<_>>() {
            let mut map = layer_map_for_output(&output);
            let layer = map.layers().find(|l| l.wl_surface() == &wl).cloned();
            if layer.as_ref().is_some_and(|l| l.namespace() == "mide-launcher") {
                self.launcher_seen = false;
                self.launcher_anim = None;
                self.launcher_gone = false;
            }
            if let Some(layer) = layer {
                map.unmap_layer(&layer);
                map.arrange();
            }
        }
        self.fit_windows_to_area();

        // Se a camada tinha o foco, o foco volta para a janela do topo.
        let Some(keyboard) = self.seat.get_keyboard() else {
            return;
        };
        if keyboard.current_focus().as_ref() != Some(&wl) {
            return;
        }
        let top = self
            .space
            .elements()
            .last()
            .and_then(|w| w.toplevel().map(|t| t.wl_surface().clone()));
        let serial = smithay::utils::SERIAL_COUNTER.next_serial();
        keyboard.set_focus(self, top, serial);
    }
}

delegate_layer_shell!(State);

delegate_output!(State);
smithay::delegate_viewporter!(State);

impl XdgDecorationHandler for State {
    fn new_decoration(&mut self, toplevel: ToplevelSurface) {
        set_server_side(&toplevel);
    }

    fn request_mode(&mut self, toplevel: ToplevelSurface, _mode: Mode) {
        // O mide decide: sempre decoração de servidor, para ter a barra estilo mac.
        set_server_side(&toplevel);
    }

    fn unset_mode(&mut self, toplevel: ToplevelSurface) {
        set_server_side(&toplevel);
    }
}

fn set_server_side(toplevel: &ToplevelSurface) {
    // Marca a superfície: só ela recebe a barra de título do mide.
    with_states(toplevel.wl_surface(), |states| {
        states.data_map.insert_if_missing_threadsafe(|| SsdMarker);
    });
    toplevel.with_pending_state(|s| s.decoration_mode = Some(Mode::ServerSide));
    toplevel.send_configure();
}

/// Marca de superfície que pediu decoração de servidor (o mide desenha a barra).
pub struct SsdMarker;

/// Duração do esmaecer ao fechar uma janela.
/// Duração da entrada dos menus.
pub const MENU_APPEAR: std::time::Duration = mide::motion::MENU_APPEAR;

pub const CLOSE_FADE: std::time::Duration = mide::motion::CLOSE_FADE;

/// Número da janela, dado quando ela abre e único na sessão.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowId(pub u64);

/// Número da janela (0 se não tiver).
pub fn window_id(window: &Window) -> u64 {
    window
        .toplevel()
        .and_then(|t| with_states(t.wl_surface(), |s| s.data_map.get::<WindowId>().map(|id| id.0)))
        .unwrap_or(0)
}

/// Altura da barra de título desta janela: 30 px se a janela usa decoração de
/// servidor, 0 se o app desenha a própria barra (GTK4, libadwaita).
pub fn bar_height_for(window: &Window) -> i32 {
    let Some(toplevel) = window.toplevel() else {
        return 0;
    };
    let ssd = with_states(toplevel.wl_surface(), |states| {
        states.data_map.get::<SsdMarker>().is_some()
    });
    if ssd { BAR_HEIGHT } else { 0 }
}

delegate_xdg_decoration!(State);

/// Camadas que ficam por cima do launcher enquanto ele está aberto, como no macOS:
/// a dock e a barra continuam visíveis sobre o fundo escurecido, com seus menus e
/// as notificações.
pub const ABOVE_LAUNCHER: &[&str] = &[
    "mide-bar",
    "mide-dock",
    "mide-bar-menu",
    "mide-dock-menu",
    "mide-notifications",
];

/// Camadas de topo e sobrepostas da saída, da frente para trás (ordem de desenho
/// invertida e de clique). Com o launcher aberto, as de `ABOVE_LAUNCHER` vêm antes.
pub fn front_layers(map: &smithay::desktop::LayerMap) -> Vec<smithay::desktop::LayerSurface> {
    use smithay::wayland::shell::wlr_layer::Layer;
    let mut layers: Vec<smithay::desktop::LayerSurface> = map
        .layers()
        .rev()
        .filter(|l| matches!(l.layer(), Layer::Top | Layer::Overlay))
        .cloned()
        .collect();
    if layers.iter().any(|l| l.namespace() == "mide-launcher") {
        // Ordenação estável: mantém a ordem original dentro de cada grupo.
        layers.sort_by_key(|l| !ABOVE_LAUNCHER.contains(&l.namespace()));
    }
    layers
}

/// Camadas de fundo (área de trabalho com ícones), da frente para trás: `Bottom` e
/// depois `Background`. Ficam atrás das janelas e na frente do papel de parede.
pub fn back_layers(map: &smithay::desktop::LayerMap) -> Vec<smithay::desktop::LayerSurface> {
    use smithay::wayland::shell::wlr_layer::Layer;
    let mut layers: Vec<smithay::desktop::LayerSurface> = map.layers().rev().filter(|l| l.layer() == Layer::Bottom).cloned().collect();
    layers.extend(map.layers().rev().filter(|l| l.layer() == Layer::Background).cloned());
    layers
}

impl State {
    /// O launcher está aberto?
    pub fn launcher_open(&self) -> bool {
        self.space
            .outputs()
            .any(|o| layer_map_for_output(o).layers().any(|l| l.namespace() == "mide-launcher"))
    }

    /// Abre o buscador (`mide-search`), ou fecha se já estiver aberto.
    pub fn toggle_search(&mut self) {
        let open = self.primary_output().and_then(|output| {
            layer_map_for_output(&output).layers().find(|l| l.namespace() == "mide-search").cloned()
        });
        match open {
            Some(layer) => {
                tracing::info!("buscador: fechar");
                layer.layer_surface().send_close();
            }
            None => {
                tracing::info!("buscador: abrir");
                crate::spawn_client("mide-search");
            }
        }
    }

    /// Abre o launcher (`mide-launcher`) ou, se já estiver aberto, pede para fechar.
    pub fn toggle_launcher(&mut self) {
        let open = self.primary_output().and_then(|output| {
            layer_map_for_output(&output)
                .layers()
                .find(|l| l.namespace() == "mide-launcher")
                .cloned()
        });
        match open {
            Some(layer) => {
                // Fecha esmaecendo: a camada só é fechada quando a animação acaba
                // (em `finish_animations`).
                if self.launcher_anim.is_some_and(|(_, opening)| !opening) {
                    return;
                }
                tracing::info!("launcher: fechar");
                if self.config.effects.animations {
                    self.launcher_anim = Some((Instant::now(), false));
                } else {
                    layer.layer_surface().send_close();
                }
            }
            None => {
                tracing::info!("launcher: abrir");
                crate::spawn_client("mide-launcher");
            }
        }
    }

    /// Retângulo do ícone do app desta janela na dock, em coordenadas da saída.
    pub fn dock_icon_for(&self, window: &Window) -> Option<smithay::utils::Rectangle<i32, Logical>> {
        let app_id = window
            .toplevel()
            .and_then(|t| {
                with_states(t.wl_surface(), |s| {
                    s.data_map
                        .get::<smithay::wayland::shell::xdg::XdgToplevelSurfaceData>()
                        .and_then(|d| d.lock().ok().and_then(|a| a.app_id.clone()))
                })
            })?
            .to_lowercase();
        let (_, icon) = self.dock_icons.iter().find(|(ids, _)| ids.contains(&app_id))?;
        let output = self.primary_output()?;
        let map = layer_map_for_output(&output);
        let dock = map.layers().find(|l| l.namespace() == "mide-dock")?;
        let geo = map.layer_geometry(dock)?;
        // Em coordenadas globais: a dock fica na tela principal, que pode não ser a
        // primeira (com a principal à direita, a lâmpada sairia da tela errada).
        let screen = self.space.output_geometry(&output)?;
        Some(smithay::utils::Rectangle::new(screen.loc + geo.loc + icon.loc, icon.size))
    }

    /// Depois de a saída mudar de tamanho: reorganiza as camadas (a dock volta ao
    /// lugar) e reajusta as janelas maximizadas à nova área.
    pub fn output_resized(&mut self) {
        let Some(output) = self.primary_output() else {
            return;
        };
        layer_map_for_output(&output).arrange();
        self.fit_windows_to_area();
    }

    /// Opacidade e zoom do launcher agora (animação de abrir ou fechar).
    pub fn launcher_look(&self) -> (f32, f64) {
        let ease = |t: f32| 1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3);
        match self.launcher_anim {
            Some((start, true)) => {
                let k = ease(start.elapsed().as_secs_f32() / LAUNCHER_OPEN.as_secs_f32());
                (k, 1.06 - 0.06 * k as f64)
            }
            Some((start, false)) => {
                let k = ease(start.elapsed().as_secs_f32() / LAUNCHER_CLOSE.as_secs_f32());
                (1.0 - k, 1.0 + 0.04 * k as f64)
            }
            None if self.launcher_gone => (0.0, 1.04),
            None => (1.0, 1.0),
        }
    }

    /// Quanto do menu já apareceu (0 a 1, suavizado) e o deslocamento vertical dele
    /// (desce 6 px até o lugar). Menus fora da lista já estão inteiros.
    pub fn appear(&self, surface: &WlSurface) -> (f32, i32) {
        let Some((_, at)) = self.appearing.iter().find(|(s, _)| s == surface) else {
            return (1.0, 0);
        };
        let t = (at.elapsed().as_secs_f32() / MENU_APPEAR.as_secs_f32()).clamp(0.0, 1.0);
        let e = 1.0 - (1.0 - t) * (1.0 - t);
        (e, (-6.0 * (1.0 - e)).round() as i32)
    }

    /// Tela principal: a escolhida nas Configurações (`[monitors] primary`), ou a
    /// primeira. É onde ficam a barra, a dock, o launcher e as janelas novas.
    pub fn primary_output(&self) -> Option<Output> {
        let wanted = self.config.monitors.primary.as_deref();
        self.space
            .outputs()
            .find(|o| wanted.is_some_and(|name| o.name() == name))
            .or_else(|| self.space.outputs().next())
            .cloned()
    }

    /// Tela sob o ponto (coordenadas globais), ou a principal. Espelhando, a principal.
    pub fn output_at(&self, pos: Point<f64, Logical>) -> Option<Output> {
        if self.config.monitors.mirror {
            return self.primary_output();
        }
        self.space.output_under(pos).next().cloned().or_else(|| self.primary_output())
    }

    /// Tela de onde vêm as camadas (barra, dock, área de trabalho) desenhadas em
    /// `output`: ela mesma, ou a principal quando as telas estão espelhadas.
    pub fn layer_output(&self, output: &Output) -> Output {
        if self.config.monitors.mirror {
            self.primary_output().unwrap_or_else(|| output.clone())
        } else {
            output.clone()
        }
    }

    /// Algo mudou na tela: pede um quadro ao anfitrião já.
    pub fn wake(&mut self) {
        if !self.redraw {
            self.redraw = true;
            if let Some(waker) = &self.waker {
                waker.ping();
            }
        }
    }

    /// Encerra as animações que terminaram. As de minimizar tiram a janela do espaço.
    pub fn finish_animations(&mut self, now: Instant) {
        // Pedido de fechar sem resposta em 2 s (o app perguntou "salvar?"): esquece.
        self.close_requests.retain(|(_, at)| now.duration_since(*at) < std::time::Duration::from_secs(2));
        self.ghosts.retain(|(_, _, at)| now.duration_since(*at) < CLOSE_FADE);
        self.appearing.retain(|(s, at)| s.is_alive() && now.duration_since(*at) < MENU_APPEAR);
        match self.launcher_anim {
            Some((start, true)) if now.duration_since(start) >= LAUNCHER_OPEN => self.launcher_anim = None,
            Some((start, false)) if now.duration_since(start) >= LAUNCHER_CLOSE => {
                self.launcher_anim = None;
                self.launcher_gone = true;
                let layer = self.primary_output().and_then(|o| {
                    layer_map_for_output(&o).layers().find(|l| l.namespace() == "mide-launcher").cloned()
                });
                if let Some(layer) = layer {
                    layer.layer_surface().send_close();
                }
            }
            _ => {}
        }
        let done: Vec<WindowAnimation> = self
            .animations
            .iter()
            .filter(|a| a.finished(now))
            .cloned()
            .collect();
        for anim in done {
            if anim.hide_at_end {
                self.space.unmap_elem(&anim.window);
            }
        }
        self.animations.retain(|a| !a.finished(now));
    }
}

