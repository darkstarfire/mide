//! Backend X11: roda o mide como servidor de exibicao X11.
//!
//! Nao existe suporte nativo no Smithay para atuar como servidor X11, entao este
//! backend cria a propria conexao X11 (via x11rb), abre uma janela raiz de
//! composicao, renderiza sobre X11 via XPutImage e integra com o event loop
//! calloop ja usado pelo mide.

use smithay::reexports::calloop::{
    generic::Generic, timer::Timer, Interest, LoopHandle, Mode, PostAction,
};
use std::{
    collections::HashMap,
    error::Error,
    os::fd::{AsFd, AsRawFd, BorrowedFd, RawFd},
    sync::{Arc, Mutex},
    time::Duration,
};
use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{self, ConnectionExt as _};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

use crate::config::{Action, Modifiers};
use super::input::run_action;
use crate::state::State;
use crate::x11_render::X11Render;
use crate::x11_scene::{DecorationHit, X11Scene};

/// Wrapper sem assumir propriedade do fd, para uso com `calloop::Generic`.
struct FdWrapper(RawFd);

impl AsFd for FdWrapper {
    fn as_fd(&self) -> BorrowedFd<'_> {
        // SAFETY: o fd e valido enquanto a conexao X11 estiver viva.
        unsafe { BorrowedFd::borrow_raw(self.0) }
    }
}

/// Atomos X11/EWMH usados pelo backend.
pub struct Atoms {
    pub wm_protocols: xproto::Atom,
    pub wm_delete_window: xproto::Atom,
    pub net_wm_pid: xproto::Atom,
    pub net_wm_name: xproto::Atom,
    pub utf8_string: xproto::Atom,
    pub net_wm_state: xproto::Atom,
    pub state_maximized_vert: xproto::Atom,
    pub state_maximized_horz: xproto::Atom,
    pub state_fullscreen: xproto::Atom,
    pub net_active_window: xproto::Atom,
    pub net_client_list: xproto::Atom,
    pub net_supported: xproto::Atom,
    pub net_wm_strut: xproto::Atom,
    pub net_wm_strut_partial: xproto::Atom,
    pub net_workarea: xproto::Atom,
    pub net_close_window: xproto::Atom,
    pub net_supporting_wm_check: xproto::Atom,
    pub net_wm_check: xproto::Atom,
}

impl Atoms {
    fn intern(conn: &RustConnection) -> Result<Self, Box<dyn Error>> {
        let atom = |name: &[u8]| -> Result<xproto::Atom, Box<dyn Error>> {
            Ok(conn.intern_atom(false, name)?.reply()?.atom)
        };
        Ok(Self {
            wm_protocols: atom(b"WM_PROTOCOLS")?,
            wm_delete_window: atom(b"WM_DELETE_WINDOW")?,
            net_wm_pid: atom(b"_NET_WM_PID")?,
            net_wm_name: atom(b"_NET_WM_NAME")?,
            utf8_string: atom(b"UTF8_STRING")?,
            net_wm_state: atom(b"_NET_WM_STATE")?,
            state_maximized_vert: atom(b"_NET_WM_STATE_MAXIMIZED_VERT")?,
            state_maximized_horz: atom(b"_NET_WM_STATE_MAXIMIZED_HORZ")?,
            state_fullscreen: atom(b"_NET_WM_STATE_FULLSCREEN")?,
            net_active_window: atom(b"_NET_ACTIVE_WINDOW")?,
            net_client_list: atom(b"_NET_CLIENT_LIST")?,
            net_supported: atom(b"_NET_SUPPORTED")?,
            net_wm_strut: atom(b"_NET_WM_STRUT")?,
            net_wm_strut_partial: atom(b"_NET_WM_STRUT_PARTIAL")?,
            net_workarea: atom(b"_NET_WORKAREA")?,
            net_close_window: atom(b"_NET_CLOSE_WINDOW")?,
            net_supporting_wm_check: atom(b"_NET_SUPPORTING_WM_CHECK")?,
            net_wm_check: atom(b"_NET_WM_CHECK")?,
        })
    }
}

/// Mapeia keysym <-> keycodes do servidor X, usado para XGrabKey e para traduzir
/// eventos de tecla em keysyms.
struct KeyMap {
    min_keycode: u8,
    keysym_to_keycodes: HashMap<u32, Vec<u8>>,
    keycode_to_syms: Vec<(u32, u32)>, // nível 0 (sem shift) e nível 1 (com shift)
}

impl KeyMap {
    fn build(conn: &RustConnection) -> Result<Self, Box<dyn Error>> {
        let setup = conn.setup();
        let min = setup.min_keycode;
        let max = setup.max_keycode;
        drop(setup);
        let map = conn
            .get_keyboard_mapping(min, max - min + 1)?
            .reply()?;
        let per = map.keysyms_per_keycode as usize;
        let mut keysym_to_keycodes: HashMap<u32, Vec<u8>> = HashMap::new();
        let mut keycode_to_syms: Vec<(u32, u32)> = Vec::new();
        for (i, code) in (min..=max).enumerate() {
            let base = i * per;
            let s0 = *map.keysyms.get(base).unwrap_or(&0);
            let s1 = if per > 1 {
                *map.keysyms.get(base + 1).unwrap_or(&0)
            } else {
                0
            };
            keycode_to_syms.push((s0, s1));
            if s0 != 0 {
                keysym_to_keycodes.entry(s0).or_default().push(code);
            }
            if s1 != 0 && s1 != s0 {
                keysym_to_keycodes.entry(s1).or_default().push(code);
            }
        }
        Ok(Self {
            min_keycode: min,
            keysym_to_keycodes,
            keycode_to_syms,
        })
    }

    /// keysym do evento de tecla (considerando o Shift).
    fn keysym_for_event(&self, keycode: u8, shift: bool) -> u32 {
        if keycode < self.min_keycode {
            return 0;
        }
        let idx = (keycode - self.min_keycode) as usize;
        let (s0, s1) = self
            .keycode_to_syms
            .get(idx)
            .copied()
            .unwrap_or((0, 0));
        if shift && s1 != 0 {
            s1
        } else {
            s0
        }
    }

    /// keycodes que geram esse keysym (para XGrabKey).
    fn keycodes_for_keysym(&self, keysym: u32) -> &[u8] {
        self.keysym_to_keycodes
            .get(&keysym)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }
}

/// Máscara X11 (bits brutos) de modificadores a partir do `Modifiers` do mide.
fn modifiers_mask(mods: &Modifiers) -> u16 {
    let mut m: u16 = 0;
    if mods.shift {
        m |= 1 << 0;
    }
    if mods.ctrl {
        m |= 1 << 2;
    }
    if mods.alt {
        m |= 1 << 3;
    }
    if mods.logo {
        m |= 1 << 6;
    }
    m
}

/// Area reservada por uma janela de camada (barra, dock) numa das bordas.
#[derive(Clone, Copy, Debug, Default)]
struct Strut {
    left: u32,
    right: u32,
    top: u32,
    bottom: u32,
}

/// Area util da tela, depois de descontar as reservas das camadas.
#[derive(Clone, Copy, Debug, Default)]
struct WorkArea {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

/// Estado de gerenciamento de janelas compartilhado com o event loop.
#[derive(Default)]
struct LayoutState {
    /// Reservas por janela nao gerenciada (override-redirect).
    struts: HashMap<xproto::Window, Strut>,
    /// Janelas gerenciadas, na ordem de adocao.
    clients: Vec<xproto::Window>,
    /// Janela com foco no momento.
    active: Option<xproto::Window>,
    /// Tela bloqueada? (equivalente X11 a ext-session-lock-v1).
    locked: bool,
    work_area: WorkArea,
}

/// Conexao X11 encapsulada, compartilhada entre o backend e outros modulos.
pub struct X11Backend {
    pub conn: Arc<RustConnection>,
    pub screen_num: i32,
    pub window: xproto::Window,
    pub atoms: Arc<Atoms>,
    pub render: Arc<Mutex<X11Render>>,
    pub scene: Arc<Mutex<X11Scene>>,
    pub width: u32,
    pub height: u32,
    pub frame_count: u64,
}

/// Dados que o event loop precisa para tratar os eventos e gerenciar janelas.
struct Shared {
    conn: Arc<RustConnection>,
    atoms: Arc<Atoms>,
    scene: Arc<Mutex<X11Scene>>,
    layout: Arc<Mutex<LayoutState>>,
    keymap: KeyMap,
    /// Janela de composicao (mae das janelas clientes).
    comp_window: xproto::Window,
    /// Janela raiz do servidor X.
    root: xproto::Window,
    /// Janela de bloqueio (override-redirect, tela cheia) quando bloqueada.
    lock_window: Option<xproto::Window>,
    /// Arrastre em andamento: (janela, offset_x, offset_y) do ponteiro em
    /// relacao ao canto superior-esquerdo da janela, em pixels de saida.
    drag_window: Mutex<Option<(xproto::Window, i16, i16)>>,
}

impl X11Backend {
    /// Conecta ao display X11, cria a janela de composicao e registra no event loop.
    pub fn init(
        loop_handle: &LoopHandle<'static, State>,
        state: &mut State,
    ) -> Result<Self, Box<dyn Error>> {
        let (conn, screen_num) = RustConnection::connect(None)?;
        let conn = Arc::new(conn);

        let setup = conn.setup();
        let screen = &setup.roots[screen_num as usize];
        let width = screen.width_in_pixels as u32;
        let height = screen.height_in_pixels as u32;
        let root = screen.root;

        let window = conn.generate_id()?;
        let aux = xproto::CreateWindowAux {
            background_pixmap: None,
            background_pixel: None,
            border_pixmap: None,
            border_pixel: None,
            bit_gravity: None,
            win_gravity: None,
            backing_store: None,
            backing_planes: None,
            backing_pixel: None,
            override_redirect: None,
            save_under: None,
            event_mask: Some(
                xproto::EventMask::EXPOSURE
                    | xproto::EventMask::KEY_PRESS
                    | xproto::EventMask::KEY_RELEASE
                    | xproto::EventMask::BUTTON_PRESS
                    | xproto::EventMask::BUTTON_RELEASE
                    | xproto::EventMask::POINTER_MOTION
                    | xproto::EventMask::ENTER_WINDOW
                    | xproto::EventMask::LEAVE_WINDOW
                    | xproto::EventMask::STRUCTURE_NOTIFY
                    | xproto::EventMask::SUBSTRUCTURE_NOTIFY,
            ),
            do_not_propogate_mask: None,
            colormap: None,
            cursor: None,
        };
        conn.create_window(
            screen.root_depth,
            window,
            screen.root,
            0,
            0,
            screen.width_in_pixels,
            screen.height_in_pixels,
            0,
            xproto::WindowClass::INPUT_OUTPUT,
            screen.root_visual,
            &aux,
        )?;

        conn.map_window(window)?;

        // Assume o papel de window manager: intercepta requisicoes de mapeamento
        // e configuracao de janelas clientes na janela raiz.
        let root_attr = xproto::ChangeWindowAttributesAux::new().event_mask(
            xproto::EventMask::SUBSTRUCTURE_REDIRECT
                | xproto::EventMask::SUBSTRUCTURE_NOTIFY
                | xproto::EventMask::PROPERTY_CHANGE
                | xproto::EventMask::KEY_PRESS
                | xproto::EventMask::KEY_RELEASE,
        );
        if let Err(err) = conn.change_window_attributes(screen.root, &root_attr) {
            tracing::warn!("nao foi possivel assumir SubstructureRedirect na raiz: {err}");
        }

        let atoms = Arc::new(Atoms::intern(&conn)?);

        // Janela auxiliar de "WM em execucao" (algumas ferramentas EWMH exigem).
        let wm_check = conn.generate_id()?;
        let check_aux = xproto::CreateWindowAux::new().override_redirect(1);
        conn.create_window(
            0,
            wm_check,
            screen.root,
            -1,
            -1,
            1,
            1,
            0,
            xproto::WindowClass::INPUT_ONLY,
            0,
            &check_aux,
        )?;
        conn.change_property32(
            xproto::PropMode::REPLACE,
            wm_check,
            atoms.net_supporting_wm_check,
            xproto::AtomEnum::WINDOW,
            &[wm_check],
        )?;
        conn.change_property8(
            xproto::PropMode::REPLACE,
            wm_check,
            atoms.net_wm_check,
            atoms.utf8_string,
            b"mide",
        )?;
        conn.change_property32(
            xproto::PropMode::REPLACE,
            screen.root,
            atoms.net_supporting_wm_check,
            xproto::AtomEnum::WINDOW,
            &[wm_check],
        )?;

        // Lista de recursos EWMH anunciados.
        let supported = [
            atoms.net_wm_state,
            atoms.state_maximized_vert,
            atoms.state_maximized_horz,
            atoms.state_fullscreen,
            atoms.net_active_window,
            atoms.net_client_list,
            atoms.net_wm_strut,
            atoms.net_wm_strut_partial,
            atoms.net_workarea,
            atoms.net_close_window,
            atoms.net_wm_name,
            atoms.utf8_string,
        ];
        conn.change_property32(
            xproto::PropMode::REPLACE,
            screen.root,
            atoms.net_supported,
            xproto::AtomEnum::ATOM,
            &supported,
        )?;
        conn.change_property32(
            xproto::PropMode::REPLACE,
            screen.root,
            atoms.net_client_list,
            xproto::AtomEnum::WINDOW,
            &[],
        )?;
        conn.change_property32(
            xproto::PropMode::REPLACE,
            screen.root,
            atoms.net_workarea,
            xproto::AtomEnum::CARDINAL,
            &[0, 0, width, height],
        )?;

        conn.flush()?;

        let render = X11Render::new(conn.clone(), window, width, height)?;
        let render = Arc::new(Mutex::new(render));
        let scene = Arc::new(Mutex::new(X11Scene::new()));
        if let Ok(mut s) = scene.lock() {
            s.set_output_size(width as u16, height as u16);
            let scale: f32 = match std::env::var("MIDE_OUTPUT_SCALE")
                .ok()
                .and_then(|v| v.parse::<f32>().ok())
            {
                Some(s) if s.is_finite() && s > 0.0 => s,
                _ => 1.0,
            };
            s.set_output_scale(scale);
            tracing::info!("escala de saida X11: {scale}");
        }
        let layout = Arc::new(Mutex::new(LayoutState {
            work_area: WorkArea {
                x: 0,
                y: 0,
                width,
                height,
            },
            locked: false,
            ..Default::default()
        }));

        // Atalhos globais: XGrabKey na raiz para cada binding configurado.
        let keymap = KeyMap::build(&conn)?;
        for binding in &state.config.bindings {
            let mods = Modifiers {
                ctrl: binding.ctrl,
                shift: binding.shift,
                alt: binding.alt,
                logo: binding.logo,
                ..Default::default()
            };
            let mask = modifiers_mask(&mods);
            for &keycode in keymap.keycodes_for_keysym(u32::from(binding.key)) {
                let _ = conn.grab_key(
                    true,
                    root,
                    xproto::ModMask::from(mask),
                    keycode,
                    xproto::GrabMode::ASYNC,
                    xproto::GrabMode::ASYNC,
                );
            }
        }
        let _ = conn.flush();

        // Captura cliques do botao esquerdo em qualquer janela (incluindo clientes)
        // com owner_events=True: o cliente tambem recebe o clique normalmente, e o
        // compositor recebe copia para fazer hit-test de decocoes (barra de titulo,
        // botoes fechar/maximizar) e arrastre.
        let _ = conn.grab_button(
            true,
            window,
            xproto::EventMask::BUTTON_PRESS
                | xproto::EventMask::BUTTON_RELEASE
                | xproto::EventMask::POINTER_MOTION,
            xproto::GrabMode::ASYNC,
            xproto::GrabMode::ASYNC,
            0u32,
            0u32,
            xproto::ButtonIndex::from(1),
            xproto::ModMask::from(0u8),
        );
        let _ = conn.flush();
        let backend = X11Backend {
            conn: conn.clone(),
            screen_num: screen_num.try_into().unwrap_or(0),
            window,
            atoms: atoms.clone(),
            render: render.clone(),
            scene: scene.clone(),
            width,
            height,
            frame_count: 0,
        };

        let shared = Arc::new(Shared {
            conn: conn.clone(),
            atoms,
            scene: scene.clone(),
            layout,
            keymap,
            comp_window: window,
            root,
            lock_window: None,
            drag_window: Mutex::new(None),
        });

        let raw_fd = conn.stream().as_raw_fd();
        let shared_dispatch = shared.clone();
        loop_handle.insert_source(
            Generic::new(FdWrapper(raw_fd), Interest::READ, Mode::Level),
            move |_, _, state: &mut State| {
                shared_dispatch.dispatch(state);
                Ok(PostAction::Continue)
            },
        )?;

        // Timer de renderizacao: dispara a cada 16 ms (~60 FPS) para manter a
        // tela atualizada.
        let render_conn = conn.clone();
        let render_clone = render.clone();
        let scene_clone = scene.clone();
        let mut frame_count: u64 = 0;
        loop_handle.insert_source(
            Timer::from_duration(Duration::from_millis(16)),
            move |_, _, state: &mut State| {
                let _ = state;
                // Desenha o quadro completo usando a cena X11.
                if let Ok(mut render) = render_clone.lock() {
                    if let Err(err) = scene_clone.lock().unwrap().draw_frame(&render_conn, &mut render) {
                        tracing::warn!("erro ao desenhar cena X11: {err}");
                    }
                }
                frame_count = frame_count.wrapping_add(1);
                smithay::reexports::calloop::timer::TimeoutAction::ToDuration(Duration::from_millis(16))
            },
        )?;

        tracing::info!(
            "backend X11 inicializado (tela {}, janela {}, {}x{})",
            screen_num,
            window,
            width,
            height
        );
        Ok(backend)
    }
}

impl Shared {
    /// Despacha todos os eventos X11 pendentes.
    fn dispatch(&self, state: &mut State) {
        loop {
            match self.conn.poll_for_event() {
                Ok(Some(event)) => self.handle_event(state, event),
                Ok(None) => break,
                Err(err) => {
                    tracing::warn!("erro ao ler evento X11: {err}");
                    break;
                }
            }
        }
    }

    fn handle_event(&self, state: &mut State, event: Event) {
        match event {
            Event::MapRequest(ev) => {
                // Janela cliente pediu para ser mapeada: reparenta sob a
                // janela de composicao e mapeia.
                if let Err(err) = self.adopt_window(ev.window) {
                    tracing::warn!("falha ao adotar janela X11 {}: {err}", ev.window);
                }
            }
            Event::ConfigureRequest(ev) => {
                // Nao deixa o cliente sair do estado maximizado/tela cheia.
                let special = self
                    .scene
                    .lock()
                    .ok()
                    .and_then(|s| s.state_of(ev.window))
                    .map(|(v, h, f, _)| v || h || f)
                    .unwrap_or(false);
                if !special {
                    let aux = xproto::ConfigureWindowAux::new()
                        .x(ev.x as i32)
                        .y(ev.y as i32)
                        .width(ev.width as u32)
                        .height(ev.height as u32);
                    let _ = self.conn.configure_window(ev.window, &aux);
                    let _ = self.conn.flush();
                }
            }
            Event::MapNotify(ev) => {
                if ev.window == self.comp_window {
                    return;
                }
                let managed = self
                    .scene
                    .lock()
                    .map(|s| s.windows.contains_key(&ev.window))
                    .unwrap_or(false);
                if managed {
                    if let Ok(mut scene) = self.scene.lock() {
                        scene.set_mapped(ev.window, true);
                    }
                } else {
                    // Janela de camada (override-redirect): observa as reservas
                    // de borda para calcular a area util.
                    let _ = self.conn.change_window_attributes(
                        ev.window,
                        &xproto::ChangeWindowAttributesAux::new()
                            .event_mask(xproto::EventMask::PROPERTY_CHANGE),
                    );
                    self.update_strut(ev.window);
                }
            }
            Event::UnmapNotify(ev) => {
                let managed = self
                    .scene
                    .lock()
                    .map(|s| s.windows.contains_key(&ev.window))
                    .unwrap_or(false);
                if managed {
                    if let Ok(mut scene) = self.scene.lock() {
                        scene.set_mapped(ev.window, false);
                    }
                } else {
                    self.remove_strut(ev.window);
                }
            }
            Event::DestroyNotify(ev) => {
                let managed = self
                    .scene
                    .lock()
                    .map(|s| s.windows.contains_key(&ev.window))
                    .unwrap_or(false);
                if managed {
                    if let Ok(mut scene) = self.scene.lock() {
                        scene.remove_window(ev.window);
                    }
                    if let Ok(mut layout) = self.layout.lock() {
                        layout.clients.retain(|w| *w != ev.window);
                        if layout.active == Some(ev.window) {
                            layout.active = None;
                        }
                    }
                    self.update_client_list();
                    let _ = self.conn.change_property32(
                        xproto::PropMode::REPLACE,
                        self.root,
                        self.atoms.net_active_window,
                        xproto::AtomEnum::WINDOW,
                        &[],
                    );
                }
                self.remove_strut(ev.window);
                tracing::debug!("janela X11 {} destruida", ev.window);
            }
            Event::PropertyNotify(ev) => {
                if ev.atom == self.atoms.net_wm_strut
                    || ev.atom == self.atoms.net_wm_strut_partial
                {
                    self.update_strut(ev.window);
                }
            }
            Event::KeyPress(ev) => {
                // Janela de bloqueio: qualquer tecla desbloqueia (MVP).
                if self.lock_window.is_some() {
                    if let Err(err) = self.unlock() {
                        tracing::warn!("falha ao desbloquear: {err}");
                    }
                    return;
                }
                // Atalho global pressionado na raiz? Executa a ação configurada.
                if ev.event == self.root && !self.is_locked() {
                    if let Some(action) = self.global_binding(state, ev.detail, u16::from(ev.state)) {
                        if action == crate::config::Action::LockScreen {
                            if let Err(err) = self.lock() {
                                tracing::warn!("falha ao bloquear tela: {err}");
                            }
                        } else {
                            run_action(state, action);
                        }
                    }
                } else if ev.event != self.root {
                    tracing::trace!("X11 tecla pressionada: code={}", ev.detail);
                }
            }
            Event::KeyRelease(ev) => {
                if let Some(lock_window) = self.lock_window {
                    if ev.event == lock_window {
                        return;
                    }
                }
                if ev.event == self.root && !self.is_locked() {
                    let _ = self.global_binding(state, ev.detail, u16::from(ev.state));
                } else if ev.event != self.root {
                    tracing::trace!("X11 tecla solta: code={}", ev.detail);
                }
            }
            Event::ButtonPress(ev) => {
                tracing::trace!(
                    "X11 botao pressionado: button={} em ({}, {})",
                    ev.detail,
                    ev.event_x,
                    ev.event_y
                );
                // Janela de bloqueio: ignora cliques (qualquer tecla desbloqueia).
                if self.lock_window.is_some() {
                    return;
                }
                // Clique em janela cliente: hit-test de decocoes do servidor.
                if ev.child != 0 && ev.detail == 1 {
                    if let Ok(scene) = self.scene.lock() {
                        if let Some(win) = scene.windows.get(&ev.child) {
                            if win.mapped {
                                let k = scene.output_scale;
                                // ev.event_x/y sao relativos à janela de
                                // composicao (0,0) => coordenadas de saida.
                                // Converte para pixels brutos e depois para
                                // coordenadas relativas à janela cliente.
                                let wx = (ev.event_x as f32 / k) as i16 - win.x;
                                let wy = (ev.event_y as f32 / k) as i16 - win.y;
                                match win.hit_test(wx, wy) {
                                    DecorationHit::Close => {
                                        drop(scene);
                                        self.send_close(ev.child);
                                        return;
                                    }
                                    DecorationHit::Maximize => {
                                        let (vert, horz, full, normal) =
                                            scene.state_of(ev.child).unwrap_or((false, false, false, None));
                                        drop(scene);
                                        let new_vert = !vert || !horz;
                                        let new_horz = new_vert;
                                        if let Err(err) =
                                            self.apply_state(ev.child, new_vert, new_horz, false)
                                        {
                                            tracing::warn!(
                                                "falha ao alternar maximizar janela {}: {err}",
                                                ev.child
                                            );
                                        }
                                        return;
                                    }
                                    DecorationHit::Title => {
                                        // Inicia arrastre: offset do ponteiro em
                                        // relacao ao canto superior-esquerdo da
                                        // janela (pixels de saida).
                                        let drag_off_x = (ev.event_x as f32 / k) as i16 - win.x;
                                        let drag_off_y = (ev.event_y as f32 / k) as i16 - win.y;
                                        drop(scene);
                                        *self.drag_window.lock().unwrap() =
                                            Some((ev.child, drag_off_x, drag_off_y));
                                        self.focus(ev.child);
                                        return;
                                    }
                                    DecorationHit::Client => {}
                                }
                            }
                        }
                    }
                }
                // Foca a janela cliente sob o ponteiro, se houver.
                if ev.child != 0 {
                    self.focus(ev.child);
                }
            }
            Event::EnterNotify(ev) => {
                // Foco segue o ponteiro ao entrar numa janela adotada.
                let known = self
                    .scene
                    .lock()
                    .map(|s| s.windows.contains_key(&ev.event))
                    .unwrap_or(false);
                if known {
                    self.focus(ev.event);
                }
            }
            Event::ButtonRelease(ev) => {
                tracing::trace!(
                    "X11 botao solto: button={} em ({}, {})",
                    ev.detail,
                    ev.event_x,
                    ev.event_y
                );
                // Cancela arrastre ao soltar o botao esquerdo.
                if ev.detail == 1 {
                    *self.drag_window.lock().unwrap() = None;
                }
            }
            Event::MotionNotify(ev) => {
                // Arrastre em andamento: move a janela acompanhando o ponteiro.
                let drag = self.drag_window.lock().unwrap();
                if let Some((win, off_x, off_y)) = drag.as_ref() {
                    let win = *win;
                    let off_x = *off_x;
                    let off_y = *off_y;
                    let nx: i32 = ev.event_x as i32 - off_x as i32;
                    let ny: i32 = ev.event_y as i32 - off_y as i32;
                    let aux = xproto::ConfigureWindowAux::new()
                        .x(nx)
                        .y(ny);
                    let _ = self.conn.configure_window(win, &aux);
                    drop(drag);
                    if let Ok(mut scene) = self.scene.lock() {
                        scene.update_window(win, nx as i16, ny as i16, 0u16, 0u16);
                    }
                    let _ = self.conn.flush();
                    return;
                }
                tracing::trace!(
                    "X11 movimento do ponteiro em ({}, {})",
                    ev.event_x,
                    ev.event_y
                );
            }
            Event::Expose(ev) => {
                tracing::trace!(
                    "X11 exposicao: janela {}, ({}, {}), {}x{}",
                    ev.window,
                    ev.x,
                    ev.y,
                    ev.width,
                    ev.height
                );
            }
            Event::ConfigureNotify(ev) => {
                tracing::trace!(
                    "X11 configuracao: janela {}, ({}, {}), {}x{}",
                    ev.window,
                    ev.x,
                    ev.y,
                    ev.width,
                    ev.height
                );
                if ev.window == self.comp_window {
                    if let Ok(mut scene) = self.scene.lock() {
                        scene.set_output_size(ev.width, ev.height);
                    }
                    self.recompute_work_area();
                } else {
                    if let Ok(mut scene) = self.scene.lock() {
                        scene.update_window(ev.event, ev.x, ev.y, ev.width, ev.height);
                    }
                }
            }
            Event::ClientMessage(ev) => self.client_message(ev),
            _ => {}
        }
    }

    /// Mensagens EWMH enviadas pelos clientes (para a janela raiz).
    fn client_message(&self, ev: xproto::ClientMessageEvent) {
        let data = ev.data.as_data32();
        if ev.type_ == self.atoms.net_wm_state {
            // Acao (0 remover, 1 adicionar, 2 alternar) sobre uma ou duas
            // propriedades de estado.
            let action = data[0];
            let (mut vert, mut horz, mut full) = self
                .scene
                .lock()
                .ok()
                .and_then(|s| s.state_of(ev.window))
                .map(|(v, h, f, _)| (v, h, f))
                .unwrap_or((false, false, false));
            for prop in [data[1], data[2]] {
                let target = if prop == self.atoms.state_maximized_vert {
                    Some(&mut vert)
                } else if prop == self.atoms.state_maximized_horz {
                    Some(&mut horz)
                } else if prop == self.atoms.state_fullscreen {
                    Some(&mut full)
                } else {
                    None
                };
                if let Some(flag) = target {
                    match action {
                        1 => *flag = true,
                        2 => *flag = !*flag,
                        _ => *flag = false,
                    }
                }
            }
            if let Err(err) = self.apply_state(ev.window, vert, horz, full) {
                tracing::warn!("falha ao aplicar estado da janela {}: {err}", ev.window);
            }
        } else if ev.type_ == self.atoms.net_active_window {
            self.focus(ev.window);
        } else if ev.type_ == self.atoms.net_close_window {
            self.send_close(ev.window);
        } else {
            tracing::trace!(
                "X11 mensagem do cliente: janela {}, tipo {}",
                ev.window,
                ev.type_
            );
        }
    }

    /// Pede ao cliente para fechar (WM_DELETE_WINDOW).
    fn send_close(&self, window: xproto::Window) {
        let event = xproto::ClientMessageEvent::new(
            32,
            window,
            self.atoms.wm_protocols,
            [self.atoms.wm_delete_window, 0u32, 0, 0, 0],
        );
        let _ = self
            .conn
            .send_event(false, window, xproto::EventMask::NO_EVENT, event);
        let _ = self.conn.flush();
    }

    /// Foca e traz para o topo uma janela cliente.
    fn focus(&self, window: xproto::Window) {
        let _ = self.conn.set_input_focus(
            xproto::InputFocus::POINTER_ROOT,
            window,
            xproto::Time::CURRENT_TIME,
        );
        let aux = xproto::ConfigureWindowAux::new().stack_mode(xproto::StackMode::ABOVE);
        let _ = self.conn.configure_window(window, &aux);
        let _ = self.conn.change_property32(
            xproto::PropMode::REPLACE,
            self.root,
            self.atoms.net_active_window,
            xproto::AtomEnum::WINDOW,
            &[window],
        );
        if let Ok(mut layout) = self.layout.lock() {
            layout.active = Some(window);
        }
        let _ = self.conn.flush();
    }

    /// Bloqueia a sessao: cria janela override-redirect em tela cheia sobre tudo
    /// e foca o teclado nela (equivalente X11 a ext-session-lock-v1).
    /// Sem app de senha ainda: qualquer tecla desbloqueia (MVP).
    fn lock(&self) -> Result<(), Box<dyn Error>> {
        if self.lock_window.is_some() {
            return Ok(()); // ja bloqueada
        }
        let (out_w, out_h) = self.output_size();
        let id = self.conn.generate_id()?;
        let aux = xproto::CreateWindowAux::new()
            .override_redirect(1)
            .background_pixel(0)
            .event_mask(xproto::EventMask::KEY_PRESS | xproto::EventMask::KEY_RELEASE);
        self.conn.create_window(
            0,
            id,
            self.root,
            0,
            0,
            out_w as u16,
            out_h as u16,
            0,
            xproto::WindowClass::INPUT_OUTPUT,
            0,
            &aux,
        )?;
        self.conn.map_window(id)?;
        // Foca o teclado na janela de bloqueio e a coloca no topo.
        let _ = self.conn.set_input_focus(
            xproto::InputFocus::NONE,
            id,
            0u32,
        );
        let aux = xproto::ConfigureWindowAux::new().stack_mode(xproto::StackMode::ABOVE);
        let _ = self.conn.configure_window(id, &aux);
        self.conn.change_property32(
            xproto::PropMode::REPLACE,
            self.root,
            self.atoms.net_active_window,
            xproto::AtomEnum::WINDOW,
            &[id],
        )?;
        self.conn.flush()?;
        if let Ok(mut layout) = self.layout.lock() {
            layout.locked = true;
        }
        tracing::info!("tela de bloqueio X11 ativada (janela {})", id);
        Ok(())
    }

    /// Desbloqueia a sessao: remove a janela de bloqueio e devolve o foco.
    fn unlock(&self) -> Result<(), Box<dyn Error>> {
        let Some(id) = self.lock_window else {
            return Ok(());
        };
        let _ = self.conn.destroy_window(id);
        self.conn.flush()?;
        if let Ok(mut layout) = self.layout.lock() {
            layout.locked = false;
        }
        // Devolve o foco ao ponteiro (padrao do mide).
        let _ = self.conn.set_input_focus(
            xproto::InputFocus::POINTER_ROOT,
            0u32,
            0u32,
        );
        tracing::info!("tela de bloqueio X11 desativada");
        Ok(())
    }

    /// Traduz um evento de tecla na raiz para a acao de atalho global, se houver.
    fn global_binding(
        &self,
        state: &mut State,
        keycode: u8,
        state_mask: u16,
    ) -> Option<Action> {
        let mods = Modifiers {
            shift: state_mask & 0x01 != 0,
            ctrl: state_mask & 0x04 != 0,
            alt: state_mask & 0x08 != 0,
            logo: state_mask & 0x40 != 0,
            ..Default::default()
        };
        let keysym = self.keymap.keysym_for_event(keycode, mods.shift);
        if keysym == 0 {
            return None;
        }
        let keysym = xkbcommon::xkb::utf32_to_keysym(keysym);
        if let Some(action) = state.config.match_binding(mods, keysym) {
            tracing::info!("atalho global X11: {}", action);
            return Some(action);
        }
        None
    }

    /// Publica a lista de janelas gerenciadas na raiz.
    fn update_client_list(&self) {
        let list = self
            .layout
            .lock()
            .map(|l| l.clients.clone())
            .unwrap_or_default();
        let _ = self.conn.change_property32(
            xproto::PropMode::REPLACE,
            self.root,
            self.atoms.net_client_list,
            xproto::AtomEnum::WINDOW,
            &list,
        );
        let _ = self.conn.flush();
    }

    /// Tamanho atual da saida.
    fn output_size(&self) -> (u16, u16) {
        self.scene
            .lock()
            .map(|s| (s.output_width, s.output_height))
            .unwrap_or((0, 0))
    }

    /// Esta com a tela bloqueada?
    fn is_locked(&self) -> bool {
        self.layout.lock().map(|l| l.locked).unwrap_or(false)
    }

    /// Le `_NET_WM_STRUT`/`_NET_WM_STRUT_PARTIAL` de uma janela.
    fn read_strut(&self, window: xproto::Window) -> Option<Strut> {
        let read = |atom: xproto::Atom| -> Option<Vec<u32>> {
            let prop = self
                .conn
                .get_property(false, window, atom, xproto::AtomEnum::CARDINAL, 0, 16)
                .ok()?
                .reply()
                .ok()?;
            prop.value32().map(|v| v.collect())
        };
        if let Some(v) = read(self.atoms.net_wm_strut_partial) {
            if v.len() >= 4 {
                return Some(Strut {
                    left: v[0],
                    right: v[1],
                    top: v[2],
                    bottom: v[3],
                });
            }
        }
        if let Some(v) = read(self.atoms.net_wm_strut) {
            if v.len() >= 4 {
                return Some(Strut {
                    left: v[0],
                    right: v[1],
                    top: v[2],
                    bottom: v[3],
                });
            }
        }
        None
    }

    /// Atualiza a reserva de borda de uma janela de camada.
    fn update_strut(&self, window: xproto::Window) {
        if window == self.comp_window || window < 2 {
            return;
        }
        let managed = self
            .scene
            .lock()
            .map(|s| s.windows.contains_key(&window))
            .unwrap_or(false);
        if managed {
            return;
        }
        let strut = self.read_strut(window);
        if let Ok(mut layout) = self.layout.lock() {
            match strut {
                Some(s) => {
                    layout.struts.insert(window, s);
                }
                None => {
                    layout.struts.remove(&window);
                }
            }
        }
        self.recompute_work_area();
    }

    /// Remove a reserva de uma janela destruida/desmapeada.
    fn remove_strut(&self, window: xproto::Window) {
        let removed = if let Ok(mut layout) = self.layout.lock() {
            layout.struts.remove(&window).is_some()
        } else {
            false
        };
        if removed {
            self.recompute_work_area();
        }
    }

    /// Recalcula a area util a partir das reservas e publica `_NET_WORKAREA`.
    fn recompute_work_area(&self) {
        let (width, height) = self.output_size();
        let area = {
            let Ok(layout) = self.layout.lock() else {
                return;
            };
            let mut left = 0u32;
            let mut right = 0u32;
            let mut top = 0u32;
            let mut bottom = 0u32;
            for s in layout.struts.values() {
                left = left.max(s.left);
                right = right.max(s.right);
                top = top.max(s.top);
                bottom = bottom.max(s.bottom);
            }
            let x = left.min(width as u32);
            let y = top.min(height as u32);
            WorkArea {
                x,
                y,
                width: (width as u32).saturating_sub(left).saturating_sub(right),
                height: (height as u32).saturating_sub(top).saturating_sub(bottom),
            }
        };
        if let Ok(mut layout) = self.layout.lock() {
            layout.work_area = area;
        }
        let _ = self.conn.change_property32(
            xproto::PropMode::REPLACE,
            self.root,
            self.atoms.net_workarea,
            xproto::AtomEnum::CARDINAL,
            &[area.x, area.y, area.width, area.height],
        );
        let _ = self.conn.flush();
        tracing::info!(
            "area util X11: {}x{}+{},{}",
            area.width,
            area.height,
            area.x,
            area.y
        );
    }

    /// Le o `_NET_WM_STATE` pedido pelo cliente antes de mapear.
    fn read_wm_state(&self, window: xproto::Window) -> Vec<u32> {
        self.conn
            .get_property(false, window, self.atoms.net_wm_state, xproto::AtomEnum::ATOM, 0, 32)
            .ok()
            .and_then(|c| c.reply().ok())
            .and_then(|p| p.value32().map(|v| v.collect()))
            .unwrap_or_default()
    }

    /// Le o título da janela (`_NET_WM_NAME` UTF8_STRING ou `WM_NAME` STRING).
    fn read_window_title(&self, window: xproto::Window) -> Option<String> {
        let prop = self
            .conn
            .get_property(
                false,
                window,
                self.atoms.net_wm_name,
                self.atoms.utf8_string,
                0,
                1024,
            )
            .ok()
            .and_then(|c| c.reply().ok())?;
        let bytes: Vec<u8> = prop.value8()?.collect();
        let title = String::from_utf8_lossy(&bytes).into_owned();
        if !title.is_empty() {
            return Some(title);
        }
        // Fallback WM_NAME.
        let prop = self
            .conn
            .get_property(
                false,
                window,
                xproto::AtomEnum::WM_NAME,
                xproto::AtomEnum::STRING,
                0,
                1024,
            )
            .ok()
            .and_then(|c| c.reply().ok())?;
        let bytes: Vec<u8> = prop.value8()?.collect();
        let title = String::from_utf8_lossy(&bytes).into_owned();
        if title.is_empty() {
            None
        } else {
            Some(title)
        }
    }

    /// Aplica maximizar/tela cheia a uma janela, guardando a geometria original.
    fn apply_state(
        &self,
        window: xproto::Window,
        maximized_vert: bool,
        maximized_horz: bool,
        fullscreen: bool,
    ) -> Result<(), Box<dyn Error>> {
        let (out_w, out_h) = self.output_size();
        let work = self
            .layout
            .lock()
            .map(|l| l.work_area)
            .unwrap_or(WorkArea {
                x: 0,
                y: 0,
                width: out_w as u32,
                height: out_h as u32,
            });
        let geom = self.conn.get_geometry(window)?.reply()?;
        let (old_vert, old_horz, old_full, old_normal) = self
            .scene
            .lock()
            .ok()
            .and_then(|s| s.state_of(window))
            .unwrap_or((false, false, false, None));
        let was_special = old_vert || old_horz || old_full;
        let will_special = maximized_vert || maximized_horz || fullscreen;
        let normal = if !was_special && will_special {
            Some((geom.x, geom.y, geom.width, geom.height))
        } else {
            old_normal
        };
        let base = normal.unwrap_or((geom.x, geom.y, geom.width, geom.height));
        let (mut x, mut y, mut w, mut h) = base;
        if fullscreen {
            x = 0;
            y = 0;
            w = out_w;
            h = out_h;
        } else if maximized_vert && maximized_horz {
            x = work.x as i16;
            y = work.y as i16;
            w = work.width as u16;
            h = work.height as u16;
        } else if maximized_horz {
            x = work.x as i16;
            w = work.width as u16;
        } else if maximized_vert {
            y = work.y as i16;
            h = work.height as u16;
        }

        if let Ok(mut scene) = self.scene.lock() {
            scene.set_state(window, maximized_vert, maximized_horz, fullscreen, normal);
            scene.update_window(window, x, y, w, h);
        }
        let aux = xproto::ConfigureWindowAux::new()
            .x(x as i32)
            .y(y as i32)
            .width(w as u32)
            .height(h as u32);
        self.conn.configure_window(window, &aux)?;
        self.update_wm_state_property(window, maximized_vert, maximized_horz, fullscreen)?;
        self.conn.flush()?;
        tracing::info!(
            "janela {}: estado vert={} horz={} full={} -> {}x{}+{},{}",
            window,
            maximized_vert,
            maximized_horz,
            fullscreen,
            w,
            h,
            x,
            y
        );
        Ok(())
    }

    /// Escreve `_NET_WM_STATE` na janela conforme o estado atual.
    fn update_wm_state_property(
        &self,
        window: xproto::Window,
        maximized_vert: bool,
        maximized_horz: bool,
        fullscreen: bool,
    ) -> Result<(), Box<dyn Error>> {
        let mut states = Vec::new();
        if maximized_vert {
            states.push(self.atoms.state_maximized_vert);
        }
        if maximized_horz {
            states.push(self.atoms.state_maximized_horz);
        }
        if fullscreen {
            states.push(self.atoms.state_fullscreen);
        }
        self.conn.change_property32(
            xproto::PropMode::REPLACE,
            window,
            self.atoms.net_wm_state,
            xproto::AtomEnum::ATOM,
            &states,
        )?;
        Ok(())
    }

    /// Adota uma janela cliente: reparenta sob a janela de composicao, registra
    /// na cena e a mapeia.
    fn adopt_window(&self, window: xproto::Window) -> Result<(), Box<dyn Error>> {
        if window == self.comp_window {
            return Ok(());
        }
        if self
            .scene
            .lock()
            .map(|s| s.windows.contains_key(&window))
            .unwrap_or(false)
        {
            return Ok(());
        }

        // Posiciona em cascata para nao empilhar todas as janelas em (0, 0).
        let slot = self.scene.lock().map(|s| s.windows.len()).unwrap_or(0);
        let offset = 40 + ((slot % 8) as i16) * 40;

        let geom = self.conn.get_geometry(window)?.reply()?;
        self.conn
            .reparent_window(window, self.comp_window, offset, offset)?;
        if let Ok(mut scene) = self.scene.lock() {
            scene.add_window(window, offset, offset, geom.width, geom.height);
        }
        if let Ok(mut layout) = self.layout.lock() {
            if !layout.clients.contains(&window) {
                layout.clients.push(window);
            }
        }
        self.update_client_list();

        // Estado inicial pedido pelo cliente (`_NET_WM_STATE` antes de mapear).
        let mut vert = false;
        let mut horz = false;
        let mut full = false;
        for atom in self.read_wm_state(window) {
            if atom == self.atoms.state_maximized_vert {
                vert = true;
            } else if atom == self.atoms.state_maximized_horz {
                horz = true;
            } else if atom == self.atoms.state_fullscreen {
                full = true;
            }
        }

        self.conn.map_window(window)?;
        // Seleciona eventos de ponteiro para hit-test de decocoes e arrasto.
        let _ = self.conn.change_window_attributes(
            window,
            &xproto::ChangeWindowAttributesAux::new()
                .event_mask(
                    xproto::EventMask::ENTER_WINDOW
                        | xproto::EventMask::PROPERTY_CHANGE
                        | xproto::EventMask::BUTTON_PRESS
                        | xproto::EventMask::BUTTON_RELEASE
                        | xproto::EventMask::POINTER_MOTION,
                ),
        );
        // Le o título da janela (_NET_WM_NAME / WM_NAME).
        let title = self.read_window_title(window);
        if let Ok(mut scene) = self.scene.lock() {
            scene.set_title(window, title.clone());
        }
        self.focus(window);
        if vert || horz || full {
            self.apply_state(window, vert, horz, full)?;
        }
        tracing::info!(
            "janela X11 {} adotada ({}x{})",
            window,
            geom.width,
            geom.height
        );
        Ok(())
    }
}
