//! Janela "Sobre este computador": versão do mide e especificações da máquina.
//!
//! Janela comum (xdg-shell) que pede decoração do servidor, então ganha a barra
//! de título do mide. Os dados vêm de `/etc/os-release`, `/proc` e `/sys`.

use std::error::Error;

use mide::ui::{
    draw::{rgba, to_argb8888},
    text::{FontWeight, Text},
};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_registry,
    output::{OutputHandler, OutputState},
    reexports::{
        calloop::EventLoop,
        calloop_wayland_source::WaylandSource,
        client::{
            globals::registry_queue_init,
            protocol::{wl_output, wl_shm, wl_surface},
            Connection, QueueHandle,
        },
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
use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Stroke, Transform};

/// Tamanho inicial; a janela pode ser redimensionada à vontade a partir do mínimo.
const WIDTH: u32 = 540;
const HEIGHT: u32 = 360;
const MIN_WIDTH: u32 = 360;
const MIN_HEIGHT: u32 = 300;

fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "mide_about=info".into()),
        )
        .init();

    let conn = Connection::connect_to_env()?;
    let (globals, event_queue) = registry_queue_init(&conn)?;
    let qh = event_queue.handle();

    let compositor = CompositorState::bind(&globals, &qh)?;
    let xdg_shell = XdgShell::bind(&globals, &qh)?;
    let shm = Shm::bind(&globals, &qh)?;

    let surface = compositor.create_surface(&qh);
    let window = xdg_shell.create_window(surface, WindowDecorations::RequestServer, &qh);
    window.set_title("Sobre este computador");
    window.set_app_id("mide-about");
    window.set_min_size(Some((MIN_WIDTH, MIN_HEIGHT)));
    window.commit();

    let pool = SlotPool::new((WIDTH * HEIGHT * 4) as usize, &shm)?;
    let mut about = About {
        registry_state: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &qh),
        shm,
        pool,
        window,
        text: Text::new(),
        info: mide::sysinfo::system_info(),
        exit: false,
        dirty: false,
        width: WIDTH,
        height: HEIGHT,
    };

    let mut event_loop: EventLoop<About> = EventLoop::try_new()?;
    WaylandSource::new(conn.clone(), event_queue)
        .insert(event_loop.handle())
        .map_err(|e| e.to_string())?;
    while !about.exit {
        event_loop.dispatch(None, &mut about)?;
        if about.dirty {
            about.dirty = false;
            about.draw();
        }
    }
    Ok(())
}

/// Corta o texto com reticências para caber em `max` pixels.
fn fit(text: &mut Text, value: &str, max: f32) -> String {
    if text.measure(value, 12.5, FontWeight::Regular) <= max {
        return value.to_string();
    }
    let mut chars: Vec<char> = value.chars().collect();
    while !chars.is_empty() {
        chars.pop();
        let candidate = format!("{}…", chars.iter().collect::<String>().trim_end());
        if text.measure(&candidate, 12.5, FontWeight::Regular) <= max {
            return candidate;
        }
    }
    "…".into()
}

struct About {
    registry_state: RegistryState,
    output_state: OutputState,
    shm: Shm,
    pool: SlotPool,
    window: Window,
    text: Text,
    info: Vec<(&'static str, String)>,
    exit: bool,
    /// Desenho pendente: vários `configure` seguidos (redimensionando) geram um só quadro.
    dirty: bool,
    width: u32,
    height: u32,
}

impl About {
    fn draw(&mut self) {
        let (width, height) = (self.width, self.height);
        let Some(mut pixmap) = Pixmap::new(width, height) else {
            return;
        };
        // Cores do tema do mide-ui (claro ou escuro, como nas Configurações).
        let theme = mide::kit::Theme::system();
        let [br, bg, bb, _] = theme.background;
        pixmap.fill(tiny_skia::Color::from_rgba8(br, bg, bb, 255));
        let [ir, ig, ib, _] = theme.text;
        let cx = width as f32 / 2.0;
        // Conteúdo centralizado na vertical quando a janela fica mais alta.
        let top = ((height as f32 - HEIGHT as f32) / 2.0).max(0.0);
        let mut canvas = Pixmap::new(width, HEIGHT.max(height)).unwrap_or_else(|| Pixmap::new(1, 1).unwrap());
        let pixmap_final = &mut pixmap;
        let pixmap = &mut canvas;

        // Marca grande do mide no topo.
        if let Some(ring) = PathBuilder::from_circle(cx, 52.0, 22.0) {
            let mut paint = Paint::default();
            paint.set_color(rgba(ir, ig, ib, 255));
            paint.anti_alias = true;
            let stroke = Stroke {
                width: 6.0,
                ..Default::default()
            };
            pixmap.stroke_path(&ring, &paint, &stroke, Transform::identity(), None);
        }
        if let Some(dot) = PathBuilder::from_circle(cx + 7.0, 45.0, 7.0) {
            let mut paint = Paint::default();
            paint.set_color(rgba(ir, ig, ib, 255));
            paint.anti_alias = true;
            pixmap.fill_path(&dot, &paint, FillRule::Winding, Transform::identity(), None);
        }

        let title = "mide";
        let tw = self.text.measure(title, 24.0, FontWeight::Bold);
        self.text.draw(pixmap, title, cx - tw / 2.0, 86.0, 24.0, FontWeight::Bold, theme.text);
        let version = format!("Versão {}", env!("CARGO_PKG_VERSION"));
        let vw = self.text.measure(&version, 12.5, FontWeight::Regular);
        self.text.draw(pixmap, &version, cx - vw / 2.0, 118.0, 12.5, FontWeight::Regular, theme.text_secondary);

        // Especificações em duas colunas, centralizadas: rótulos alinhados à direita
        // e valores à esquerda, cortados com reticências se a janela for estreita.
        let info = self.info.clone();
        let label_w = info
            .iter()
            .map(|(l, _)| self.text.measure(l, 12.5, FontWeight::Semibold))
            .fold(0.0, f32::max);
        let value_w = info
            .iter()
            .map(|(_, v)| self.text.measure(v, 12.5, FontWeight::Regular))
            .fold(0.0, f32::max);
        let block = (label_w + 12.0 + value_w).min(width as f32 - 40.0);
        let label_right = (cx - block / 2.0 + label_w).max(20.0 + label_w);
        let value_x = label_right + 12.0;
        let value_max = width as f32 - 20.0 - value_x;
        let mut y = 152.0;
        for (label, value) in &info {
            let lw = self.text.measure(label, 12.5, FontWeight::Semibold);
            self.text.draw(pixmap, label, label_right - lw, y, 12.5, FontWeight::Semibold, theme.text);
            let value = fit(&mut self.text, value, value_max);
            self.text.draw(pixmap, &value, value_x, y, 12.5, FontWeight::Regular, theme.text_secondary);
            y += 24.0;
        }
        pixmap_final.draw_pixmap(0, top as i32, pixmap.as_ref(), &Default::default(), Transform::identity(), None);
        let pixmap = pixmap_final;

        let Ok((buffer, canvas)) =
            self.pool.create_buffer(width as i32, height as i32, width as i32 * 4, wl_shm::Format::Argb8888)
        else {
            return;
        };
        to_argb8888(pixmap, canvas);
        let surface = self.window.wl_surface();
        surface.damage_buffer(0, 0, width as i32, height as i32);
        if buffer.attach_to(surface).is_ok() {
            self.window.commit();
        }
    }
}

impl CompositorHandler for About {
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: i32) {}
    fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: wl_output::Transform) {}
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {}
    fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
    fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
}

impl OutputHandler for About {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl WindowHandler for About {
    fn request_close(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &Window) {
        self.exit = true;
    }

    fn configure(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &Window, configure: WindowConfigure, _: u32) {
        // Sem tamanho sugerido, mantém o atual; com tamanho, respeita o mínimo.
        let (w, h) = configure.new_size;
        if let Some(w) = w {
            self.width = w.get().max(MIN_WIDTH);
        }
        if let Some(h) = h {
            self.height = h.get().max(MIN_HEIGHT);
        }
        self.dirty = true;
    }
}

impl ShmHandler for About {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

delegate_registry!(About);
smithay_client_toolkit::delegate_dispatch2!(About);

impl ProvidesRegistryState for About {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }

    registry_handlers![OutputState];
}
