//! Estado global do compositor e os handlers dos protocolos Wayland.
//!
//! Cada protocolo mínimo que o mide anuncia precisa de um estado (`*State`)
//! e de um handler que o implementa. As macros `delegate_*!` ligam os dois.

use std::{error::Error, time::{Duration, Instant}};

use smithay::reexports::wayland_protocols::xdg::{
    decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode,
    shell::server::xdg_toplevel::State as XdgState,
};

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

/// Duração da animação de maximizar.
pub const ANIMATION_DURATION: Duration = Duration::from_millis(220);

/// Duração do efeito lâmpada (minimizar e restaurar): mais longo para ser legível.
pub const GENIE_DURATION: Duration = Duration::from_millis(650);
/// Abrir e fechar o launcher.
pub const LAUNCHER_OPEN: Duration = Duration::from_millis(260);
pub const LAUNCHER_CLOSE: Duration = Duration::from_millis(170);

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
            if t < 0.5 { 4.0 * t * t * t } else { 1.0 - (-2.0 * t + 2.0).powi(3) / 2.0 }
        } else {
            1.0 - (1.0 - t).powi(3)
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
    pub glass: std::collections::HashMap<String, Vec<(smithay::utils::Rectangle<i32, smithay::utils::Logical>, f32)>>,
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
    /// Forma do ponteiro sobre a janela do mide (redimensionar, padrão).
    pub cursor_icon: smithay::reexports::winit::window::CursorIcon,
    /// Redimensionamento pela borda em andamento.
    pub resize_grab: Option<ResizeGrab>,
    /// Animações em andamento (ex.: maximizar).
    pub animations: Vec<WindowAnimation>,
    /// Janelas minimizadas, da mais antiga para a mais recente.
    pub minimized: Vec<MinimizedWindow>,
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
    /// Autoteste: finge o Ctrl pressionado (o teste não tem teclado real).
    pub test_ctrl: bool,
    /// Popups (menus, listas) rastreados até serem mapeados e destruídos.
    pub popups: PopupManager,
    /// Superfícies de camada (barra, dock, buscador) do protocolo wlr-layer-shell.
    pub layer_shell_state: WlrLayerShellState,
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
    pub seat: Seat<Self>,
}

/// Posição inicial da n-ésima janela em cascata: 48 px de margem e 32 px de passo.
pub fn cascade_position(index: usize) -> Point<i32, Logical> {
    let offset = (index as i32) * 32;
    (48 + offset, 48 + offset).into()
}

/// Arrasto de janela: guarda a janela e a distância entre o cursor e a origem dela.
pub struct MoveGrab {
    pub window: Window,
    pub offset: Point<f64, Logical>,
}

impl State {
    pub fn new(display_handle: DisplayHandle, config: Config) -> Result<Self, Box<dyn Error>> {
        // Teclado virtual: deixa ferramentas como o wtype digitarem nos testes automáticos.
        VirtualKeyboardManagerState::new::<Self, _>(&display_handle, |_| true);
        let compositor_state = CompositorState::new::<Self>(&display_handle);
        let xdg_shell_state = XdgShellState::new::<Self>(&display_handle);
        let shm_state = ShmState::new::<Self>(&display_handle, vec![]);
        let data_device_state = DataDeviceState::new::<Self>(&display_handle);
        let layer_shell_state = WlrLayerShellState::new::<Self>(&display_handle);
        let decoration_state = XdgDecorationState::new::<Self>(&display_handle);

        let mut seat_state = SeatState::new();
        let mut seat = seat_state.new_wl_seat(&display_handle, "seat0");
        seat.add_keyboard(XkbConfig::default(), 200, 25)?;
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
            cursor_icon: Default::default(),
            animations: Vec::new(),
            minimized: Vec::new(),
            dock_icons: Vec::new(),
            preview: None,
            pending_open: Vec::new(),
            launcher_anim: None,
            launcher_seen: false,
            test_ctrl: false,
            resize_grab: None,
            popups: PopupManager::default(),
            layer_shell_state,
            decoration_state,
            title_press: false,
            last_title_click: None,
            config,
            quit_requested: false,
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
        use std::sync::atomic::{AtomicBool, Ordering};
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
            // O arrange só envia configurações depois dessa.
            layer.user_data().insert_if_missing_threadsafe(|| AtomicBool::new(false));
            let configured = layer
                .user_data()
                .get::<AtomicBool>()
                .map(|flag| flag.swap(true, Ordering::Relaxed))
                .unwrap_or(true);
            if !configured {
                layer.layer_surface().send_configure();
            }
            return;
        }
    }

    /// Dá o foco do teclado a uma camada que aceita foco. Usado no primeiro commit
    /// e no clique, porque antes disso o cliente ainda não está pronto para receber teclas.
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
    pub fn mark_input(&mut self, kind: &'static str, surface: Option<WlSurface>) {
        use smithay::reexports::wayland_server::Resource;
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
        self.focus_layer_if_allowed(surface);
        // Primeiro quadro do launcher: começa a animação de abrir.
        if !self.launcher_seen && self.layer_for(surface).is_some_and(|l| l.namespace() == "mide-launcher") {
            let has_buffer = smithay::backend::renderer::utils::with_renderer_surface_state(surface, |s| s.buffer().is_some()).unwrap_or(false);
            if has_buffer {
                self.launcher_seen = true;
                if self.config.effects.animations {
                    self.launcher_anim = Some((Instant::now(), true));
                }
            }
        }
        if let Some(window) = self.window_for(surface) {
            window.on_commit();
            self.open_animation_if_launched(&window);
        }
    }
}

/// Marca de janela já conferida para o efeito lâmpada de abrir.
struct OpenChecked;

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
        tracing::info!("nova janela criada");
        // Sem o configure inicial o cliente nunca desenha a janela.
        surface.send_configure();

        // Janelas novas entram em cascata, para não nascerem uma sobre a outra, a partir
        // do canto da área útil (abaixo da barra superior).
        // Número fixo da janela, para a dock e a barra se referirem a ela.
        static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let id = NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        with_states(surface.wl_surface(), |states| {
            states.data_map.insert_if_missing_threadsafe(|| WindowId(id));
        });
        let window = Window::new_wayland_window(surface);
        let origin = self.usable_area().map(|a| a.loc).unwrap_or_default();
        let at = origin + cascade_position(self.space.elements().count());
        self.space.map_element(window.clone(), at, true);

        // Como no macOS, a janela recém-aberta recebe o foco.
        let serial = smithay::utils::SERIAL_COUNTER.next_serial();
        crate::input::focus_window(self, &window, serial);
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        if let Some(window) = self.window_for(surface.wl_surface()) {
            self.space.unmap_elem(&window);
            tracing::info!("janela fechada");
        }
    }

    fn new_popup(&mut self, surface: PopupSurface, positioner: PositionerState) {
        // O popup é posicionado pela geometria que o cliente pediu, relativa ao pai.
        surface.with_pending_state(|s| s.geometry = positioner.get_geometry());
        if let Err(err) = self.popups.track_popup(PopupKind::Xdg(surface.clone())) {
            tracing::warn!("popup sem pai válido: {err}");
        }
        let _ = surface.send_configure();
    }

    fn grab(&mut self, _surface: PopupSurface, _seat: wl_seat::WlSeat, _serial: Serial) {}

    fn move_request(&mut self, surface: ToplevelSurface, _seat: wl_seat::WlSeat, _serial: Serial) {
        let Some(window) = self.window_for(surface.wl_surface()) else {
            return;
        };
        let Some(loc) = self.space.element_location(&window) else {
            return;
        };
        // O app só pede para mover depois de o ponteiro andar alguns pixels. O arrasto
        // é medido a partir do ponto onde o botão foi pressionado, para não perder esse trecho.
        let anchor = self.press_location.unwrap_or(self.pointer_location);
        let offset = anchor - loc.to_f64();
        self.move_grab = Some(MoveGrab { window, offset });
    }

    fn maximize_request(&mut self, surface: ToplevelSurface) {
        let Some(window) = self.window_for(surface.wl_surface()) else {
            surface.send_configure();
            return;
        };
        // Área útil: a saída sem as zonas exclusivas das camadas (barra superior).
        let Some(area) = self.usable_area() else {
            surface.send_configure();
            return;
        };

        let from_loc = self.space.element_location(&window);
        if let Some(loc) = from_loc {
            self.restore_positions.retain(|(s, _)| s != surface.wl_surface());
            self.restore_positions.push((surface.wl_surface().clone(), loc));
        }
        let from_size = window.geometry().size;
        self.restore_sizes.retain(|(s, _)| s != surface.wl_surface());
        self.restore_sizes.push((surface.wl_surface().clone(), from_size));
        let to_loc: Point<i32, Logical> = (area.loc.x, area.loc.y + bar_height_for(&window)).into();
        let bar_h = bar_height_for(&window);
        let to_size: Size<i32, Logical> = (area.size.w, (area.size.h - bar_h).max(1)).into();
        if let (Some(from_loc), true) = (from_loc, self.config.effects.animations) {
            self.animations.retain(|a| &a.surface != surface.wl_surface());
            self.animations.push(WindowAnimation {
                window: window.clone(),
                surface: surface.wl_surface().clone(),
                from_loc,
                from_size,
                to_loc,
                to_size,
                zoom_from: from_size.w as f32 / to_size.w.max(1) as f32,
                zoom_to: 1.0,
                alpha_from: 1.0,
                alpha_to: 1.0,
                hide_at_end: false,
                genie: false,
                duration: ANIMATION_DURATION,
                start: Instant::now(),
            });
        }

        surface.with_pending_state(|s| {
            s.states.set(XdgState::Maximized);
            s.size = Some(to_size);
        });
        surface.send_configure();
        // O conteúdo começa abaixo da barra de título, que fica no topo da área.
        self.space.map_element(window, to_loc, true);
    }

    fn unmaximize_request(&mut self, surface: ToplevelSurface) {
        let Some(window) = self.window_for(surface.wl_surface()) else {
            surface.send_configure();
            return;
        };
        // Apps GTK pedem "desmaximizar" ao abrir, sem estarem maximizados. Nesse caso
        // não há o que restaurar, e a janela não pode sair do lugar.
        if !surface.current_state().states.contains(XdgState::Maximized) {
            surface.send_configure();
            return;
        }
        // Pede de volta o tamanho que a janela tinha antes de maximizar, para a
        // animação saber o destino. Sem registro, o app escolhe o tamanho.
        let restore_size = self
            .restore_sizes
            .iter()
            .position(|(s, _)| s == surface.wl_surface())
            .map(|i| self.restore_sizes.remove(i).1);
        surface.with_pending_state(|s| {
            s.states.unset(XdgState::Maximized);
            s.size = restore_size;
        });
        surface.send_configure();

        let restore = self
            .restore_positions
            .iter()
            .position(|(s, _)| s == surface.wl_surface())
            .map(|i| self.restore_positions.remove(i).1)
            .unwrap_or((48, 48).into());

        if let (Some(from_loc), Some(to_size), true) = (
            self.space.element_location(&window),
            restore_size,
            self.config.effects.animations,
        ) {
            self.animations.retain(|a| &a.surface != surface.wl_surface());
            let from_size = window.geometry().size;
            self.animations.push(WindowAnimation {
                window: window.clone(),
                surface: surface.wl_surface().clone(),
                from_loc,
                from_size,
                to_loc: restore,
                to_size,
                zoom_from: from_size.w as f32 / to_size.w.max(1) as f32,
                zoom_to: 1.0,
                alpha_from: 1.0,
                alpha_to: 1.0,
                hide_at_end: false,
                genie: false,
                duration: ANIMATION_DURATION,
                start: Instant::now(),
            });
        }
        self.space.map_element(window, restore, true);
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
        surface.send_repositioned(token);
        let _ = surface.send_configure();
    }
}

delegate_xdg_shell!(State);

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
        _image: smithay::input::pointer::CursorImageStatus,
    ) {
    }
}

delegate_seat!(State);

impl SelectionHandler for State {
    type SelectionUserData = ();
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
            .or_else(|| self.space.outputs().next().cloned());
        let Some(output) = output else {
            tracing::warn!("superfície de camada sem saída, ignorada");
            return;
        };

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

    fn layer_destroyed(&mut self, surface: LayerSurface) {
        let wl = surface.wl_surface().clone();
        for output in self.space.outputs().cloned().collect::<Vec<_>>() {
            let mut map = layer_map_for_output(&output);
            let layer = map.layers().find(|l| l.wl_surface() == &wl).cloned();
            if layer.as_ref().is_some_and(|l| l.namespace() == "mide-launcher") {
                self.launcher_seen = false;
                self.launcher_anim = None;
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

#[cfg(test)]
mod tests {
    use super::cascade_position;

    #[test]
    fn primeira_janela_fica_na_margem() {
        assert_eq!(cascade_position(0), (48, 48).into());
    }

    #[test]
    fn janelas_seguintes_nao_se_sobrepoem_no_inicio() {
        let a = cascade_position(0);
        let b = cascade_position(1);
        let c = cascade_position(2);
        assert_ne!(a, b);
        assert_ne!(b, c);
        assert_eq!(b.x - a.x, 32);
        assert_eq!(c.y - b.y, 32);
    }
}

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

impl State {
    /// Minimiza a janela da superfície: ela encolhe em direção ao centro inferior da
    /// tela, perde opacidade e sai do espaço. Volta com `restore_minimized`.
    pub fn minimize(&mut self, surface: &WlSurface) {
        let Some(window) = self.window_for(surface) else {
            return;
        };
        let Some(loc) = self.space.element_location(&window) else {
            return;
        };
        let size = window.geometry().size;
        let target = self.dock_target(&window, size);
        self.minimized.push(MinimizedWindow {
            window: window.clone(),
            loc,
            size,
        });
        self.animations.retain(|a| &a.surface != surface);
        if !self.config.effects.animations {
            // Sem animação, a janela sai do espaço na hora.
            self.space.unmap_elem(&window);
        } else {
        self.animations.push(WindowAnimation {
            window: window.clone(),
            surface: surface.clone(),
            from_loc: loc,
            from_size: size,
            to_loc: target.0,
            to_size: target.1,
            zoom_from: 1.0,
            zoom_to: MINIMIZE_SCALE,
            alpha_from: 1.0,
            alpha_to: 0.0,
            hide_at_end: true,
            genie: self.config.effects.lamp,
            duration: if self.config.effects.lamp { GENIE_DURATION } else { ANIMATION_DURATION },
            start: Instant::now(),
        });
        }
        // Se a janela tinha o foco, o foco vai para a janela de cima que sobrou.
        if let Some(keyboard) = self.seat.get_keyboard() {
            if keyboard.current_focus().as_ref() == Some(surface) {
                let next = self
                    .space
                    .elements()
                    .rev()
                    .find(|w| w.toplevel().map(|t| t.wl_surface()) != Some(surface))
                    .and_then(|w| w.toplevel().map(|t| t.wl_surface().clone()));
                let serial = smithay::utils::SERIAL_COUNTER.next_serial();
                keyboard.set_focus(self, next, serial);
            }
        }
        tracing::info!("janela minimizada");
    }

    /// Traz de volta a janela minimizada mais recente, com a animação de volta.
    pub fn restore_minimized(&mut self) {
        let Some(MinimizedWindow { window, loc, size }) = self.minimized.pop() else {
            return;
        };
        let Some(surface) = window.toplevel().map(|t| t.wl_surface().clone()) else {
            return;
        };
        let target = self.dock_target(&window, size);
        self.space.map_element(window.clone(), loc, true);
        self.animations.retain(|a| a.surface != surface);
        if self.config.effects.animations {
        self.animations.push(WindowAnimation {
            window: window.clone(),
            surface: surface.clone(),
            from_loc: target.0,
            from_size: target.1,
            to_loc: loc,
            to_size: size,
            zoom_from: MINIMIZE_SCALE,
            zoom_to: 1.0,
            alpha_from: 0.0,
            alpha_to: 1.0,
            hide_at_end: false,
            genie: self.config.effects.lamp,
            duration: if self.config.effects.lamp { GENIE_DURATION } else { ANIMATION_DURATION },
            start: Instant::now(),
        });
        }
        let serial = smithay::utils::SERIAL_COUNTER.next_serial();
        crate::input::focus_window(self, &window, serial);
        tracing::info!("janela restaurada");
    }

    /// Primeira janela com conteúdo de um app aberto pela dock: sai do ícone dele com o
    /// efeito lâmpada (o mesmo de restaurar). Cada janela é conferida uma vez.
    fn open_animation_if_launched(&mut self, window: &Window) {
        self.pending_open.retain(|(_, at)| at.elapsed() < Duration::from_secs(15));
        if self.pending_open.is_empty() || window.geometry().size.w <= 0 {
            return;
        }
        let Some(toplevel) = window.toplevel().cloned() else {
            return;
        };
        let app_id = with_states(toplevel.wl_surface(), |s| {
            s.data_map
                .get::<smithay::wayland::shell::xdg::XdgToplevelSurfaceData>()
                .and_then(|d| d.lock().ok().and_then(|a| a.app_id.clone()))
        })
        .unwrap_or_default()
        .to_lowercase();
        if app_id.is_empty() {
            return;
        }
        let first = with_states(toplevel.wl_surface(), |s| s.data_map.insert_if_missing_threadsafe(|| OpenChecked));
        if !first {
            return;
        }
        let Some(index) = self.pending_open.iter().position(|(ids, _)| ids.contains(&app_id)) else {
            return;
        };
        self.pending_open.remove(index);
        let (Some(loc), Some(icon)) = (self.space.element_location(window), self.dock_icon_for(window)) else {
            return;
        };
        if !self.config.effects.animations {
            return;
        }
        tracing::info!("abrindo pela dock: {app_id} sai do ícone");
        let size = window.geometry().size;
        self.animations.retain(|a| &a.surface != toplevel.wl_surface());
        self.animations.push(WindowAnimation {
            window: window.clone(),
            surface: toplevel.wl_surface().clone(),
            from_loc: icon.loc,
            from_size: icon.size,
            to_loc: loc,
            to_size: size,
            zoom_from: MINIMIZE_SCALE,
            zoom_to: 1.0,
            alpha_from: 0.0,
            alpha_to: 1.0,
            hide_at_end: false,
            genie: self.config.effects.lamp,
            duration: if self.config.effects.lamp { GENIE_DURATION } else { ANIMATION_DURATION },
            start: Instant::now(),
        });
    }

    /// Posição e tamanho de origem da animação de minimizar: centro inferior, pequeno.
    fn dock_target(&self, window: &Window, size: Size<i32, Logical>) -> (Point<i32, Logical>, Size<i32, Logical>) {
        // Se a dock tem o ícone do app, o efeito vai até ele, onde ele estiver agora.
        if let Some(icon) = self.dock_icon_for(window) {
            tracing::debug!("lâmpada: alvo no ícone da dock em {:?} {:?}", icon.loc, icon.size);
            return (icon.loc, icon.size);
        }
        tracing::debug!("lâmpada: ícone do app não encontrado na dock, alvo no centro inferior");
        let small: Size<i32, Logical> = (
            (size.w as f32 * MINIMIZE_SCALE).round() as i32,
            (size.h as f32 * MINIMIZE_SCALE).round() as i32,
        )
            .into();
        let area = self
            .space
            .outputs()
            .next()
            .and_then(|o| self.space.output_geometry(o))
            .unwrap_or_else(|| smithay::utils::Rectangle::new((0, 0).into(), (1280, 800).into()));
        let x = area.loc.x + area.size.w / 2 - small.w / 2;
        let y = area.loc.y + area.size.h - small.h - 12;
        ((x, y).into(), small)
    }

    /// Área útil da saída: sem as zonas exclusivas das camadas (ex.: barra superior),
    /// em coordenadas globais. É onde janelas maximizadas ficam.
    pub fn usable_area(&self) -> Option<smithay::utils::Rectangle<i32, Logical>> {
        let output = self.space.outputs().next()?;
        let geo = self.space.output_geometry(output)?;
        let zone = layer_map_for_output(output).non_exclusive_zone();
        Some(smithay::utils::Rectangle::new(geo.loc + zone.loc, zone.size))
    }

}

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

impl State {
    /// O launcher está aberto?
    pub fn launcher_open(&self) -> bool {
        self.space
            .outputs()
            .next()
            .is_some_and(|o| layer_map_for_output(o).layers().any(|l| l.namespace() == "mide-launcher"))
    }

    /// Abre o launcher (`mide-launcher`) ou, se já estiver aberto, pede para fechar.
    pub fn toggle_launcher(&mut self) {
        let open = self.space.outputs().next().and_then(|output| {
            layer_map_for_output(output)
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
        let output = self.space.outputs().next()?;
        let map = layer_map_for_output(output);
        let dock = map.layers().find(|l| l.namespace() == "mide-dock")?;
        let geo = map.layer_geometry(dock)?;
        Some(smithay::utils::Rectangle::new(geo.loc + icon.loc, icon.size))
    }

    /// Depois de a saída mudar de tamanho: reorganiza as camadas (a dock volta ao
    /// lugar) e reajusta as janelas maximizadas à nova área.
    pub fn output_resized(&mut self) {
        let Some(output) = self.space.outputs().next().cloned() else {
            return;
        };
        layer_map_for_output(&output).arrange();
        self.fit_windows_to_area();
    }

    /// Ajusta as janelas à área útil: maximizadas ocupam a área inteira, e as outras
    /// não ficam com a barra de título sob a barra superior (como no macOS).
    /// Todas são remapeadas de baixo para cima, para manter a ordem de empilhamento.
    pub fn fit_windows_to_area(&mut self) {
        let Some(area) = self.usable_area() else {
            return;
        };
        let windows: Vec<Window> = self.space.elements().cloned().collect();
        for window in windows {
            let Some(loc) = self.space.element_location(&window) else {
                continue;
            };
            let bar_h = bar_height_for(&window);
            let maximized = window
                .toplevel()
                .is_some_and(|t| t.current_state().states.contains(XdgState::Maximized));
            let target = if maximized {
                if let Some(toplevel) = window.toplevel() {
                    toplevel.with_pending_state(|s| s.size = Some((area.size.w, (area.size.h - bar_h).max(1)).into()));
                    toplevel.send_configure();
                }
                Point::from((area.loc.x, area.loc.y + bar_h))
            } else {
                Point::from((loc.x, loc.y.max(area.loc.y + bar_h)))
            };
            if target != loc {
                tracing::info!("janela ajustada à área útil: {:?} -> {:?}", (loc.x, loc.y), (target.x, target.y));
            }
            self.space.map_element(window, target, false);
        }
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
            None => (1.0, 1.0),
        }
    }

    /// Encerra as animações que terminaram. As de minimizar tiram a janela do espaço.
    pub fn finish_animations(&mut self, now: Instant) {
        match self.launcher_anim {
            Some((start, true)) if now.duration_since(start) >= LAUNCHER_OPEN => self.launcher_anim = None,
            Some((start, false)) if now.duration_since(start) >= LAUNCHER_CLOSE => {
                self.launcher_anim = None;
                let layer = self.space.outputs().next().and_then(|o| {
                    layer_map_for_output(o).layers().find(|l| l.namespace() == "mide-launcher").cloned()
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

/// Escala final da janela ao minimizar, em relação ao tamanho natural.
pub const MINIMIZE_SCALE: f32 = 0.12;
