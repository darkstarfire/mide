//! App de teste do layer-shell (testes de interface): uma camada no topo da tela (como
//! a Waybar) com um popup ligado a ela (`get_popup`, o menu de um ícone do painel). A
//! camada é azul e o popup é laranja; cada um escreve no stdout quando é configurado.

use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_registry,
    output::{OutputHandler, OutputState},
    reexports::{
        client::{
            globals::registry_queue_init,
            protocol::{wl_output, wl_shm, wl_surface},
            Connection, QueueHandle,
        },
        protocols::xdg::shell::client::xdg_positioner::{Anchor as PAnchor, Gravity},
    },
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    shell::{
        wlr_layer::{Anchor, Layer, LayerShell, LayerShellHandler, LayerSurface, LayerSurfaceConfigure},
        xdg::{
            popup::{Popup, PopupConfigure, PopupHandler},
            window::{Window, WindowConfigure, WindowHandler},
            XdgPositioner, XdgShell,
        },
        WaylandSurface,
    },
    shm::{slot::SlotPool, Shm, ShmHandler},
};

struct Teste {
    registry: RegistryState,
    output: OutputState,
    shm: Shm,
    pool: SlotPool,
    /// Guardada para a camada continuar existindo.
    _layer: LayerSurface,
    popup: Option<Popup>,
    compositor: CompositorState,
    xdg: XdgShell,
    done: bool,
}

/// Pinta a superfície inteira de uma cor (ARGB) e manda.
fn paint(pool: &mut SlotPool, surface: &wl_surface::WlSurface, w: u32, h: u32, argb: u32) {
    let Ok((buffer, canvas)) = pool.create_buffer(w as i32, h as i32, w as i32 * 4, wl_shm::Format::Argb8888) else {
        return;
    };
    for px in canvas.chunks_exact_mut(4) {
        px.copy_from_slice(&argb.to_le_bytes());
    }
    surface.damage_buffer(0, 0, w as i32, h as i32);
    let _ = buffer.attach_to(surface);
    surface.commit();
}

impl LayerShellHandler for Teste {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        self.done = true;
    }

    fn configure(&mut self, _: &Connection, qh: &QueueHandle<Self>, layer: &LayerSurface, _: LayerSurfaceConfigure, _: u32) {
        println!("camada configurada");
        paint(&mut self.pool, layer.wl_surface(), 300, 40, 0xff2060e0);
        if self.popup.is_none() {
            // Popup sem pai, adotado pela camada (como os menus dos painéis).
            let positioner = XdgPositioner::new(&self.xdg).expect("posicionador");
            positioner.set_size(160, 120);
            positioner.set_anchor_rect(20, 0, 40, 40);
            positioner.set_anchor(PAnchor::Bottom);
            positioner.set_gravity(Gravity::Bottom);
            let surface = self.compositor.create_surface(qh);
            let popup = Popup::from_surface(None, &positioner, qh, surface, &self.xdg).expect("popup");
            layer.get_popup(popup.xdg_popup());
            popup.wl_surface().commit();
            self.popup = Some(popup);
        }
    }
}

impl PopupHandler for Teste {
    fn configure(&mut self, _: &Connection, _: &QueueHandle<Self>, popup: &Popup, config: PopupConfigure) {
        println!("popup configurado em {:?} {:?}", config.position, (config.width, config.height));
        paint(&mut self.pool, popup.wl_surface(), config.width as u32, config.height as u32, 0xffff8a1f);
    }

    fn done(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &Popup) {
        println!("popup fechado");
        self.popup = None;
    }
}

// O xdg-shell do sctk pede um tratador de janelas, mesmo sem janelas aqui.
impl WindowHandler for Teste {
    fn request_close(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &Window) {}
    fn configure(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &Window, _: WindowConfigure, _: u32) {}
}

impl CompositorHandler for Teste {
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: i32) {}
    fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: wl_output::Transform) {}
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {}
    fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
    fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
}

impl OutputHandler for Teste {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl ShmHandler for Teste {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for Teste {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
    registry_handlers![OutputState];
}

delegate_registry!(Teste);
smithay_client_toolkit::delegate_dispatch2!(Teste);

fn main() {
    let conn = Connection::connect_to_env().expect("sem compositor");
    let (globals, mut queue) = registry_queue_init::<Teste>(&conn).expect("registro");
    let qh = queue.handle();
    let compositor = CompositorState::bind(&globals, &qh).expect("wl_compositor");
    let layer_shell = LayerShell::bind(&globals, &qh).expect("layer-shell");
    let xdg = XdgShell::bind(&globals, &qh).expect("xdg-shell");
    let shm = Shm::bind(&globals, &qh).expect("wl_shm");
    let surface = compositor.create_surface(&qh);
    let layer = layer_shell.create_layer_surface(&qh, surface, Layer::Top, Some("teste-painel"), None);
    layer.set_anchor(Anchor::TOP | Anchor::LEFT);
    layer.set_margin(200, 0, 0, 200);
    layer.set_size(300, 40);
    layer.commit();
    let pool = SlotPool::new(300 * 40 * 4 * 4, &shm).expect("memória");
    let mut app = Teste { registry: RegistryState::new(&globals), output: OutputState::new(&globals, &qh), shm, pool, _layer: layer, popup: None, compositor, xdg, done: false };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while !app.done && std::time::Instant::now() < deadline {
        let _ = queue.blocking_dispatch(&mut app);
    }
}
