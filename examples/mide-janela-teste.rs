//! App de teste das posições de janelas novas (A26, seção `centro` da bateria). Uso:
//! - `mide-janela-teste fixa`: janela "Abertura" 300x200 de tamanho fixo (mínimo ==
//!   máximo), como o `xwayland-satellite` manda as janelas de abertura (splash) do X11;
//! - `mide-janela-teste sem-decoracao`: janela "Sem decoração" 400x300 que pede o modo
//!   `client_side` do `xdg-decoration` (as dicas do Motif dos apps X11);
//! - `mide-janela-teste filho`: janela "Pai" 600x400 e, depois de ela aparecer, o
//!   diálogo "Diálogo" 300x200 ligado a ela (`xdg_toplevel.set_parent`);
//! - `mide-janela-teste comum`: janela "Comum" 400x300 sem nada disso (fica na cascata).
//!
//! Cada janela é pintada de uma cor só; o programa sai sozinho em 20 s.

use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_registry,
    output::{OutputHandler, OutputState},
    reexports::client::{
        globals::registry_queue_init,
        protocol::{wl_output, wl_shm, wl_surface},
        Connection, QueueHandle,
    },
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    shell::{
        xdg::{
            window::{Window, WindowConfigure, WindowDecorations, WindowHandler},
            XdgShell,
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
    compositor: CompositorState,
    xdg: XdgShell,
    /// Janelas abertas, com o tamanho de cada uma.
    windows: Vec<(Window, u32, u32)>,
    /// No modo `filho`: o diálogo ainda vai ser aberto quando a pai aparecer.
    child_pending: bool,
    done: bool,
}

/// Pinta a superfície inteira de uma cor (ARGB) e manda.
fn paint(pool: &mut SlotPool, surface: &wl_surface::WlSurface, w: u32, h: u32, argb: u32) {
    let Ok((buffer, canvas)) = pool.create_buffer(w as i32, h as i32, w as i32 * 4, wl_shm::Format::Argb8888) else {
        return;
    };
    for px in canvas.as_chunks_mut::<4>().0 {
        px.copy_from_slice(&argb.to_le_bytes());
    }
    surface.damage_buffer(0, 0, w as i32, h as i32);
    let _ = buffer.attach_to(surface);
    surface.commit();
}

impl Teste {
    /// Cria uma janela; `fixed` pede tamanho fixo, `parent` liga a uma janela-pai.
    #[allow(clippy::too_many_arguments)]
    fn open(&mut self, qh: &QueueHandle<Self>, title: &str, w: u32, h: u32, deco: WindowDecorations, fixed: bool, parent: Option<usize>) {
        let surface = self.compositor.create_surface(qh);
        let window = self.xdg.create_window(surface, deco, qh);
        window.set_title(title);
        window.set_app_id("mide-janela-teste");
        if fixed {
            window.set_min_size(Some((w, h)));
            window.set_max_size(Some((w, h)));
        }
        if let Some(i) = parent {
            window.set_parent(Some(&self.windows[i].0));
        }
        window.commit();
        self.windows.push((window, w, h));
    }
}

impl WindowHandler for Teste {
    fn request_close(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &Window) {
        self.done = true;
    }

    fn configure(&mut self, _: &Connection, qh: &QueueHandle<Self>, window: &Window, _: WindowConfigure, _: u32) {
        let Some(i) = self.windows.iter().position(|(w, _, _)| w == window) else {
            return;
        };
        let (w, h) = (self.windows[i].1, self.windows[i].2);
        let color = if i == 0 { 0xff2060e0 } else { 0xffff8a1f };
        let surface = self.windows[i].0.wl_surface().clone();
        paint(&mut self.pool, &surface, w, h, color);
        println!("janela {i} configurada");
        if i == 0 && self.child_pending {
            self.child_pending = false;
            self.open(qh, "Diálogo", 300, 200, WindowDecorations::RequestServer, false, Some(0));
        }
    }
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
    let mode = std::env::args().nth(1).unwrap_or_else(|| "comum".into());
    let conn = Connection::connect_to_env().expect("sem compositor");
    let (globals, mut queue) = registry_queue_init::<Teste>(&conn).expect("registro");
    let qh = queue.handle();
    let compositor = CompositorState::bind(&globals, &qh).expect("wl_compositor");
    let xdg = XdgShell::bind(&globals, &qh).expect("xdg-shell");
    let shm = Shm::bind(&globals, &qh).expect("wl_shm");
    let pool = SlotPool::new(600 * 400 * 4 * 4, &shm).expect("memória");
    let mut app = Teste {
        registry: RegistryState::new(&globals),
        output: OutputState::new(&globals, &qh),
        shm,
        pool,
        compositor,
        xdg,
        windows: Vec::new(),
        child_pending: false,
        done: false,
    };
    match mode.as_str() {
        "fixa" => app.open(&qh, "Abertura", 300, 200, WindowDecorations::RequestServer, true, None),
        "sem-decoracao" => app.open(&qh, "Sem decoração", 400, 300, WindowDecorations::RequestClient, false, None),
        "filho" => {
            app.child_pending = true;
            app.open(&qh, "Pai", 600, 400, WindowDecorations::RequestServer, false, None);
        }
        _ => app.open(&qh, "Comum", 400, 300, WindowDecorations::RequestServer, false, None),
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while !app.done && std::time::Instant::now() < deadline {
        let _ = queue.blocking_dispatch(&mut app);
    }
}
