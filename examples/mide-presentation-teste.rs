//! Cliente de teste do `wp_presentation` (presentation-time), como o Chrome usa: uma
//! janela que redesenha a cada aviso de quadro (`frame`) e pede, em cada commit, o
//! aviso de apresentação. Ao juntar `N` avisos (padrão 60) ou em 15 s, imprime uma
//! linha com o resumo e sai:
//!
//! `presentation: clock=1 presented=60 discarded=0 refresh_ns=16666666 seq=crescente
//!  intervalo_ms=16.7 flags=0x0`
//!
//! (`clock` é o relógio anunciado, 1 = monotônico; `intervalo_ms`, a média entre as
//! horas de apresentação; `flags`, as marcas do último aviso: 1 vsync, 2 hw_clock,
//! 4 hw_completion, 8 zero_copy). Sem o global, imprime `presentation: ausente`.
//!
//! Uso: `mide-presentation-teste [N]`.

use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_registry,
    output::{OutputHandler, OutputState},
    reexports::{
        client::{
            globals::registry_queue_init,
            protocol::{wl_output, wl_shm, wl_surface},
            Connection, Dispatch, QueueHandle,
        },
        protocols::wp::presentation_time::client::{wp_presentation, wp_presentation_feedback},
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

const W: u32 = 240;
const H: u32 = 160;

#[derive(Default)]
struct Resumo {
    clock: Option<u32>,
    presented: u32,
    discarded: u32,
    /// Horas de apresentação (ns no relógio anunciado) e números dos vblanks.
    times: Vec<u128>,
    seqs: Vec<u64>,
    refresh: u32,
    flags: u32,
}

struct Teste {
    registry: RegistryState,
    output: OutputState,
    shm: Shm,
    pool: SlotPool,
    window: Window,
    presentation: Option<wp_presentation::WpPresentation>,
    configured: bool,
    frame: u32,
    resumo: Resumo,
}

impl Teste {
    /// Pinta um quadro (a cor muda a cada um), pede o próximo `frame` e o aviso de
    /// apresentação, e manda.
    fn draw(&mut self, qh: &QueueHandle<Self>) {
        let Ok((buffer, canvas)) = self.pool.create_buffer(W as i32, H as i32, W as i32 * 4, wl_shm::Format::Argb8888) else {
            return;
        };
        self.frame += 1;
        let shade = (self.frame * 7 % 255) as u8;
        let argb = u32::from_be_bytes([0xff, shade, 0x60, 0xe0]);
        for px in canvas.as_chunks_mut::<4>().0 {
            px.copy_from_slice(&argb.to_le_bytes());
        }
        let surface = self.window.wl_surface().clone();
        surface.damage_buffer(0, 0, W as i32, H as i32);
        surface.frame(qh, smithay_client_toolkit::compositor::FrameCallbackData(surface.clone()));
        if let Some(p) = &self.presentation {
            p.feedback(&surface, qh, ());
        }
        let _ = buffer.attach_to(&surface);
        surface.commit();
    }

    fn summary(&self) -> String {
        let r = &self.resumo;
        let Some(clock) = r.clock else {
            return "presentation: ausente".into();
        };
        let increasing = r.seqs.windows(2).all(|w| w[1] > w[0]);
        let interval = if r.times.len() > 1 {
            (r.times[r.times.len() - 1] - r.times[0]) as f64 / (r.times.len() - 1) as f64 / 1e6
        } else {
            0.0
        };
        format!(
            "presentation: clock={clock} presented={} discarded={} refresh_ns={} seq={} intervalo_ms={interval:.1} flags={:#x}",
            r.presented,
            r.discarded,
            r.refresh,
            if increasing { "crescente" } else { "fora_de_ordem" },
            r.flags
        )
    }
}

impl Dispatch<wp_presentation::WpPresentation, ()> for Teste {
    fn event(
        state: &mut Self,
        _: &wp_presentation::WpPresentation,
        event: wp_presentation::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wp_presentation::Event::ClockId { clk_id } = event {
            state.resumo.clock = Some(clk_id);
        }
    }
}

impl Dispatch<wp_presentation_feedback::WpPresentationFeedback, ()> for Teste {
    fn event(
        state: &mut Self,
        _: &wp_presentation_feedback::WpPresentationFeedback,
        event: wp_presentation_feedback::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let r = &mut state.resumo;
        match event {
            wp_presentation_feedback::Event::Presented { tv_sec_hi, tv_sec_lo, tv_nsec, refresh, seq_hi, seq_lo, flags } => {
                let secs = (u64::from(tv_sec_hi) << 32) | u64::from(tv_sec_lo);
                r.times.push(u128::from(secs) * 1_000_000_000 + u128::from(tv_nsec));
                r.seqs.push((u64::from(seq_hi) << 32) | u64::from(seq_lo));
                r.refresh = refresh;
                r.flags = flags.into();
                r.presented += 1;
            }
            wp_presentation_feedback::Event::Discarded => r.discarded += 1,
            _ => {}
        }
    }
}

impl WindowHandler for Teste {
    fn request_close(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &Window) {
        println!("{}", self.summary());
        std::process::exit(0);
    }

    fn configure(&mut self, _: &Connection, qh: &QueueHandle<Self>, _: &Window, _: WindowConfigure, _: u32) {
        if !self.configured {
            self.configured = true;
            self.draw(qh);
        }
    }
}

impl CompositorHandler for Teste {
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: i32) {}
    fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: wl_output::Transform) {}
    fn frame(&mut self, _: &Connection, qh: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {
        self.draw(qh);
    }
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
    let want: u32 = std::env::args().nth(1).and_then(|n| n.parse().ok()).unwrap_or(60);
    let conn = Connection::connect_to_env().expect("sem compositor");
    let (globals, mut queue) = registry_queue_init::<Teste>(&conn).expect("registro");
    let qh = queue.handle();
    let compositor = CompositorState::bind(&globals, &qh).expect("wl_compositor");
    let xdg = XdgShell::bind(&globals, &qh).expect("xdg-shell");
    let shm = Shm::bind(&globals, &qh).expect("wl_shm");
    let pool = SlotPool::new((W * H * 4 * 4) as usize, &shm).expect("memória");
    let presentation = globals.bind::<wp_presentation::WpPresentation, _, _>(&qh, 1..=2, ()).ok();
    let surface = compositor.create_surface(&qh);
    let window = xdg.create_window(surface, WindowDecorations::RequestServer, &qh);
    window.set_title("Apresentação");
    window.set_app_id("mide-presentation-teste");
    window.commit();
    let mut app = Teste {
        registry: RegistryState::new(&globals),
        output: OutputState::new(&globals, &qh),
        shm,
        pool,
        window,
        presentation,
        configured: false,
        frame: 0,
        resumo: Resumo::default(),
    };
    if app.presentation.is_none() {
        println!("presentation: ausente");
        return;
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while app.resumo.presented < want && std::time::Instant::now() < deadline {
        let _ = queue.blocking_dispatch(&mut app);
    }
    println!("{}", app.summary());
}
