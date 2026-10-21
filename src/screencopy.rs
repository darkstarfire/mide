//! Captura de tela pelos apps (A19): o protocolo `wlr-screencopy` (v3).
//!
//! O OBS, o navegador numa videochamada e outros pedem a tela ao portal
//! (`xdg-desktop-portal-wlr`), e o portal pede a imagem ao mide por este protocolo e a
//! entrega pelo PipeWire. Programas como `grim` e `wf-recorder` usam o protocolo direto.
//!
//! A imagem sai do próprio quadro que o mide acabou de desenhar (sem desenho extra):
//! `copy` atende no próximo quadro; `copy_with_damage` espera a tela mudar (o primeiro
//! pedido de cada cliente sai logo, para a gravação não começar preta).

use std::{collections::HashMap, sync::Mutex, time::Instant};

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            gles::{GlesRenderer, GlesTarget},
            ExportMem,
        },
    },
    output::Output,
    reexports::{
        wayland_protocols_wlr::screencopy::v1::server::{
            zwlr_screencopy_frame_v1::{self, ZwlrScreencopyFrameV1},
            zwlr_screencopy_manager_v1::{self, ZwlrScreencopyManagerV1},
        },
        wayland_server::{
            backend::ClientId,
            protocol::{wl_buffer::WlBuffer, wl_shm},
            Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
        },
    },
    utils::{Physical, Rectangle},
};

use crate::state::State;

/// Pedidos de captura esperando um quadro, e os clientes que já receberam a primeira
/// imagem de cada tela.
#[derive(Default)]
pub struct Screencopy {
    pending: Vec<Pending>,
    served: HashMap<(ClientId, String), Instant>,
    /// Até quando o cursor precisa sair na imagem (o cursor de hardware não está no
    /// quadro: durante uma gravação, a cena desenha o cursor).
    pub cursor_until: Option<Instant>,
}

impl Screencopy {
    /// Alguma captura em andamento quer o cursor na imagem.
    pub fn wants_cursor(&self) -> bool {
        self.cursor_until.is_some_and(|t| t > Instant::now())
    }
}

struct Pending {
    frame: ZwlrScreencopyFrameV1,
    buffer: WlBuffer,
    with_damage: bool,
    output: Output,
    region: Rectangle<i32, Physical>,
}

/// Dados de um pedido (`zwlr_screencopy_frame_v1`): a tela, a região em pixels dela,
/// se o cursor entra e se a cópia já foi pedida (cada pedido copia uma vez).
pub struct FrameData {
    output: Option<Output>,
    region: Rectangle<i32, Physical>,
    cursor: bool,
    used: Mutex<bool>,
}

pub fn init(display: &DisplayHandle) {
    display.create_global::<State, ZwlrScreencopyManagerV1, ()>(3, ());
}

impl GlobalDispatch<ZwlrScreencopyManagerV1, ()> for State {
    fn bind(
        _state: &mut State,
        _display: &DisplayHandle,
        _client: &Client,
        resource: New<ZwlrScreencopyManagerV1>,
        _data: &(),
        data_init: &mut DataInit<'_, State>,
    ) {
        data_init.init(resource, ());
    }
}

impl Dispatch<ZwlrScreencopyManagerV1, ()> for State {
    fn request(
        _state: &mut State,
        _client: &Client,
        _manager: &ZwlrScreencopyManagerV1,
        request: zwlr_screencopy_manager_v1::Request,
        _data: &(),
        _display: &DisplayHandle,
        data_init: &mut DataInit<'_, State>,
    ) {
        let (frame, cursor, output, logical) = match request {
            zwlr_screencopy_manager_v1::Request::CaptureOutput { frame, overlay_cursor, output } => (frame, overlay_cursor != 0, output, None),
            zwlr_screencopy_manager_v1::Request::CaptureOutputRegion { frame, overlay_cursor, output, x, y, width, height } => {
                (frame, overlay_cursor != 0, output, Some((x, y, width, height)))
            }
            _ => return,
        };
        let output = Output::from_resource(&output);
        let size = output.as_ref().and_then(|o| o.current_mode()).map(|m| m.size).unwrap_or_default();
        let full = Rectangle::<i32, Physical>::from_size(size);
        // Região pedida em coordenadas lógicas da tela: em pixels, dentro dela.
        let region = match (logical, &output) {
            (Some((x, y, w, h)), Some(output)) => {
                let k = output.current_scale().fractional_scale();
                let r = Rectangle::<i32, Physical>::new(
                    ((x as f64 * k).round() as i32, (y as f64 * k).round() as i32).into(),
                    ((w as f64 * k).round() as i32, (h as f64 * k).round() as i32).into(),
                );
                r.intersection(full).unwrap_or_default()
            }
            _ => full,
        };
        let frame = data_init.init(frame, FrameData { output: output.clone(), region, cursor, used: Mutex::new(false) });
        if output.is_none() || region.size.w <= 0 || region.size.h <= 0 {
            frame.failed();
            return;
        }
        let (w, h) = (region.size.w as u32, region.size.h as u32);
        frame.buffer(wl_shm::Format::Xrgb8888, w, h, w * 4);
        if frame.version() >= 3 {
            frame.buffer_done();
        }
    }
}

impl Dispatch<ZwlrScreencopyFrameV1, FrameData> for State {
    fn request(
        state: &mut State,
        _client: &Client,
        frame: &ZwlrScreencopyFrameV1,
        request: zwlr_screencopy_frame_v1::Request,
        data: &FrameData,
        _display: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        let (buffer, with_damage) = match request {
            zwlr_screencopy_frame_v1::Request::Copy { buffer } => (buffer, false),
            zwlr_screencopy_frame_v1::Request::CopyWithDamage { buffer } => (buffer, true),
            zwlr_screencopy_frame_v1::Request::Destroy => {
                state.screencopy.pending.retain(|p| &p.frame != frame);
                return;
            }
            _ => return,
        };
        let Some(output) = data.output.clone() else {
            frame.failed();
            return;
        };
        if std::mem::replace(&mut *data.used.lock().unwrap_or_else(|e| e.into_inner()), true) {
            frame.post_error(zwlr_screencopy_frame_v1::Error::AlreadyUsed, "a captura já foi pedida");
            return;
        }
        let (w, h) = (data.region.size.w, data.region.size.h);
        let fits = smithay::wayland::shm::with_buffer_contents(&buffer, |_, _, b| {
            matches!(b.format, wl_shm::Format::Xrgb8888 | wl_shm::Format::Argb8888) && b.width == w && b.height == h && b.stride >= w * 4
        })
        .unwrap_or(false);
        if !fits {
            frame.post_error(zwlr_screencopy_frame_v1::Error::InvalidBuffer, "buffer de outro tamanho ou formato");
            return;
        }
        if data.cursor {
            state.screencopy.cursor_until = Some(Instant::now() + std::time::Duration::from_secs(1));
        }
        state.screencopy.pending.push(Pending { frame: frame.clone(), buffer, with_damage, output, region: data.region });
        // Um quadro novo atende o pedido (a cópia sai dele).
        state.redraw = true;
        if let Some(waker) = &state.waker {
            waker.ping();
        }
    }
}

/// Atende as capturas desta tela com o quadro que acabou de ser desenhado em
/// `framebuffer`. `damage`: o que mudou neste quadro (`None`: nada).
pub fn serve(
    renderer: &mut GlesRenderer,
    framebuffer: &GlesTarget<'_>,
    output: &Output,
    damage: Option<&[Rectangle<i32, Physical>]>,
    state: &mut State,
) {
    if state.screencopy.pending.is_empty() {
        return;
    }
    let size = output.current_mode().map(|m| m.size).unwrap_or_default();
    let full = Rectangle::<i32, Physical>::from_size(size);
    let all = std::mem::take(&mut state.screencopy.pending);
    let mut pixels: Option<Vec<u8>> = None;
    for p in all {
        if !p.frame.is_alive() {
            continue;
        }
        if &p.output != output {
            state.screencopy.pending.push(p);
            continue;
        }
        let Some(client) = p.frame.client().map(|c| c.id()) else {
            continue;
        };
        let key = (client, output.name());
        let first = !state.screencopy.served.contains_key(&key);
        let changed: Vec<Rectangle<i32, Physical>> = damage.unwrap_or_default().iter().filter_map(|d| d.intersection(p.region)).collect();
        if p.with_damage && changed.is_empty() && !first {
            state.screencopy.pending.push(p);
            continue;
        }
        if pixels.is_none() {
            pixels = read_pixels(renderer, framebuffer, full);
        }
        let Some(rgba) = pixels.as_deref() else {
            p.frame.failed();
            continue;
        };
        let flipped = crate::clip::flipped();
        let written = smithay::wayland::shm::with_buffer_contents_mut(&p.buffer, |ptr, len, b| {
            // SAFETY: o buffer é do cliente e tem `len` bytes a partir de `ptr`
            // (conferido pelo Smithay); só escrevemos dentro dele.
            let dst = unsafe { std::slice::from_raw_parts_mut(ptr, len) };
            let (fw, fh) = (full.size.w as usize, full.size.h as usize);
            let stride = b.stride as usize;
            let offset = b.offset as usize;
            for row in 0..p.region.size.h as usize {
                let y = p.region.loc.y as usize + row;
                // Alvo com a origem embaixo (winit): as linhas vêm de baixo para cima.
                let src_y = if flipped { fh - 1 - y } else { y };
                for col in 0..p.region.size.w as usize {
                    let x = p.region.loc.x as usize + col;
                    let s = (src_y * fw + x) * 4;
                    let d = offset + row * stride + col * 4;
                    if s + 4 <= rgba.len() && d + 4 <= dst.len() {
                        dst[d] = rgba[s + 2];
                        dst[d + 1] = rgba[s + 1];
                        dst[d + 2] = rgba[s];
                        dst[d + 3] = 255;
                    }
                }
            }
        });
        if written.is_err() {
            p.frame.failed();
            continue;
        }
        if p.with_damage && p.frame.version() >= 2 {
            // A caixa de tudo o que mudou, relativa à região (na primeira, ela inteira).
            let area = if first || changed.is_empty() {
                p.region
            } else {
                changed.iter().skip(1).fold(changed[0], |a, b| a.merge(*b))
            };
            p.frame.damage(
                (area.loc.x - p.region.loc.x).max(0) as u32,
                (area.loc.y - p.region.loc.y).max(0) as u32,
                area.size.w as u32,
                area.size.h as u32,
            );
        }
        p.frame.flags(zwlr_screencopy_frame_v1::Flags::empty());
        let now = rustix::time::clock_gettime(rustix::time::ClockId::Monotonic);
        let secs = now.tv_sec as u64;
        p.frame.ready((secs >> 32) as u32, secs as u32, now.tv_nsec as u32);
        state.screencopy.served.insert(key, Instant::now());
    }
    // Clientes que não pedem há um tempo: esquecidos (a próxima captura sai logo).
    state.screencopy.served.retain(|_, at| at.elapsed() < std::time::Duration::from_secs(30));
}

/// Pixels RGBA da tela inteira, como estão no alvo (linhas de cima para baixo, ou de
/// baixo para cima se o alvo é invertido).
fn read_pixels(renderer: &mut GlesRenderer, framebuffer: &GlesTarget<'_>, full: Rectangle<i32, Physical>) -> Option<Vec<u8>> {
    let region = Rectangle::from_size((full.size.w, full.size.h).into());
    let mapping = renderer.copy_framebuffer(framebuffer, region, Fourcc::Abgr8888).ok()?;
    renderer.map_texture(&mapping).ok().map(|b| b.to_vec())
}
