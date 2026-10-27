//! App de teste do layer-shell (testes de interface): uma camada no topo da tela (como
//! a Waybar) com um popup ligado a ela (`get_popup`, o menu de um ícone do painel). A
//! camada é azul e o popup é laranja; cada um escreve no stdout quando é configurado.
//!
//! `mide-layer-popup-teste fundo`: uma faixa clara de 80 px no topo, na camada Bottom
//! (atrás da barra e das janelas, como um widget da mesa) e por cima da zona da barra;
//! depois de 4 s ela fica escura (tom atrás da barra superior, seção `barra-tom`).

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
    /// Modo `fundo`: faixa sem popup; largura dada pelo compositor.
    band: Option<u32>,
}

/// Faixa do modo `fundo`: altura e cores (clara e, depois, escura).
const BAND_H: u32 = 80;
const BAND_LIGHT: u32 = 0xfff4f4f6;
const BAND_DARK: u32 = 0xff18181c;

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

    fn configure(&mut self, _: &Connection, qh: &QueueHandle<Self>, layer: &LayerSurface, config: LayerSurfaceConfigure, _: u32) {
        println!("camada configurada");
        if self.band.is_some() {
            let w = config.new_size.0.max(1);
            self.band = Some(w);
            paint(&mut self.pool, layer.wl_surface(), w, BAND_H, BAND_LIGHT);
            return;
        }
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
    let band = std::env::args().nth(1).is_some_and(|a| a == "fundo");
    let layer = if band {
        let layer = layer_shell.create_layer_surface(&qh, surface, Layer::Bottom, Some("teste-fundo"), None);
        layer.set_anchor(Anchor::TOP | Anchor::LEFT | Anchor::RIGHT);
        layer.set_size(0, BAND_H);
        // Ignora a zona da barra: fica atrás dela, desde o y=0.
        layer.set_exclusive_zone(-1);
        layer
    } else {
        let layer = layer_shell.create_layer_surface(&qh, surface, Layer::Top, Some("teste-painel"), None);
        layer.set_anchor(Anchor::TOP | Anchor::LEFT);
        layer.set_margin(200, 0, 0, 200);
        layer.set_size(300, 40);
        layer
    };
    layer.commit();
    let pool = SlotPool::new(4096 * BAND_H as usize * 4 * 2, &shm).expect("memória");
    let mut app = Teste { registry: RegistryState::new(&globals), output: OutputState::new(&globals, &qh), shm, pool, _layer: layer, popup: None, compositor, xdg, done: false, band: band.then_some(0) };
    let start = std::time::Instant::now();
    let deadline = start + std::time::Duration::from_secs(20);
    let mut darkened = false;
    while !app.done && std::time::Instant::now() < deadline {
        if !band {
            let _ = queue.blocking_dispatch(&mut app);
            continue;
        }
        let _ = queue.roundtrip(&mut app);
        if !darkened && start.elapsed() >= std::time::Duration::from_secs(4) {
            if let Some(w) = app.band.filter(|w| *w > 0) {
                paint(&mut app.pool, app._layer.wl_surface(), w, BAND_H, BAND_DARK);
                println!("camada escura");
                darkened = true;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}
