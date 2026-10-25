//! Captura de tela pelos apps (A19): o protocolo `wlr-screencopy` (v3).
//!
//! O OBS, o navegador numa videochamada e outros pedem a tela ao portal
//! (`xdg-desktop-portal-wlr`), e o portal pede a imagem ao mide por este protocolo e a
//! entrega pelo PipeWire. Programas como `grim` e `wf-recorder` usam o protocolo direto.
//!
//! A imagem sai do próprio quadro que o mide acabou de desenhar (sem desenho extra):
//! `copy` atende no próximo quadro; `copy_with_damage` espera a tela mudar (o primeiro
//! pedido de cada cliente sai logo, para a gravação não começar preta).
//!
//! Dois caminhos (A25): buffer em memória compartilhada (`wl_shm`, o `grim`), copiado
//! pelo processador; e buffer na GPU (dmabuf, o que o portal prefere quando o mide
//! oferece o `linux-dmabuf`), copiado na própria GPU (`glBlitFramebuffer`), sem passar
//! pela memória. O quadro anuncia os dois (o portal wlr 0.7 recusa a tela se não houver
//! o formato por dmabuf).

use std::{collections::HashMap, sync::Mutex, time::Instant};

use smithay::{
    backend::{
        allocator::{dmabuf::Dmabuf, Buffer as _, Fourcc},
        renderer::{
            gles::{GlesRenderer, GlesTarget},
            Bind, Blit, ExportMem, TextureFilter,
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
    /// O último quadro atendeu alguma captura (o winit precisa religar a janela).
    served_now: bool,
}

impl Screencopy {
    /// Recusa as capturas pendentes das telas que não vão mais desenhar (apagadas ou
    /// fora do arranjo): sem isso o app (o `grim`, o portal do OBS) esperava para
    /// sempre e o buffer dmabuf dele ficava preso.
    pub fn fail_outputs(&mut self, gone: impl Fn(&Output) -> bool) {
        self.pending.retain(|p| {
            if gone(&p.output) {
                p.frame.failed();
                false
            } else {
                true
            }
        });
    }

    /// O último quadro desenhado atendeu alguma captura (e mexeu no contexto do EGL).
    pub fn served_this_frame(&self) -> bool {
        self.served_now
    }

    /// Alguma captura em andamento quer o cursor na imagem.
    pub fn wants_cursor(&self) -> bool {
        self.cursor_until.is_some_and(|t| t > Instant::now())
    }
}

/// Onde a imagem vai: buffer em memória compartilhada ou na GPU.
enum Target {
    Shm(WlBuffer),
    Dma(Dmabuf),
}

struct Pending {
    frame: ZwlrScreencopyFrameV1,
    target: Target,
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
        state: &mut State,
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
            if state.dmabuf.capture {
                frame.linux_dmabuf(Fourcc::Xrgb8888 as u32, w, h);
            }
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
        let target = if let Ok(dmabuf) = smithay::wayland::dmabuf::get_dmabuf(&buffer) {
            let size = dmabuf.size();
            dma_fits(dmabuf.format().code, (size.w, size.h), (w, h)).then(|| Target::Dma(dmabuf.clone()))
        } else {
            smithay::wayland::shm::with_buffer_contents(&buffer, |_, _, b| {
                matches!(b.format, wl_shm::Format::Xrgb8888 | wl_shm::Format::Argb8888) && b.width == w && b.height == h && b.stride >= w * 4
            })
            .unwrap_or(false)
            .then(|| Target::Shm(buffer.clone()))
        };
        let Some(target) = target else {
            frame.post_error(zwlr_screencopy_frame_v1::Error::InvalidBuffer, "buffer de outro tamanho ou formato");
            return;
        };
        // O destino na GPU fica pronto agora, fora do quadro (o portal reaproveita os
        // mesmos buffers: só na primeira vez há trabalho).
        if let Target::Dma(dmabuf) = &target
            && !state.dmabuf.prepare_target(dmabuf)
        {
            frame.failed();
            return;
        }
        if data.cursor {
            state.screencopy.cursor_until = Some(Instant::now() + std::time::Duration::from_secs(1));
        }
        state.screencopy.pending.push(Pending { frame: frame.clone(), target, with_damage, output, region: data.region });
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
    framebuffer: &mut GlesTarget<'_>,
    output: &Output,
    damage: Option<&[Rectangle<i32, Physical>]>,
    state: &mut State,
) {
    state.screencopy.served_now = false;
    if state.screencopy.pending.is_empty() {
        return;
    }
    let size = output.current_mode().map(|m| m.size).unwrap_or_default();
    let full = Rectangle::<i32, Physical>::from_size(size);
    let all = std::mem::take(&mut state.screencopy.pending);
    let mut pixels: Option<Vec<u8>> = None;
    // Alguma cópia foi feita na GPU: espera ela terminar antes de avisar o app.
    let mut blitted = false;
    let mut ready = Vec::new();
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
        let flipped = crate::clip::flipped();
        state.screencopy.served_now = true;
        let copied = match &p.target {
            Target::Shm(buffer) => {
                if pixels.is_none() {
                    pixels = read_pixels(renderer, framebuffer, full);
                }
                pixels.as_deref().is_some_and(|rgba| write_shm(buffer, rgba, full, p.region, flipped))
            }
            Target::Dma(dmabuf) => match blit_dma(renderer, framebuffer, dmabuf.clone(), full, p.region, flipped) {
                Ok(()) => {
                    blitted = true;
                    true
                }
                Err(err) => {
                    tracing::debug!("captura por dmabuf falhou: {err}");
                    false
                }
            },
        };
        if !copied {
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
        ready.push(p.frame);
        state.screencopy.served.insert(key, Instant::now());
    }
    if blitted && !wait_gpu(renderer) {
        // Sem a certeza de que a cópia terminou, o app leria um quadro pela metade.
        for frame in ready {
            frame.failed();
        }
        return;
    }
    let now = rustix::time::clock_gettime(rustix::time::ClockId::Monotonic);
    let secs = now.tv_sec as u64;
    for frame in ready {
        frame.ready((secs >> 32) as u32, secs as u32, now.tv_nsec as u32);
    }
    // Clientes que não pedem há um tempo: esquecidos (a próxima captura sai logo).
    state.screencopy.served.retain(|_, at| at.elapsed() < std::time::Duration::from_secs(30));
}

/// Pixels RGBA da tela inteira, como estão no alvo (linhas de cima para baixo, ou de
/// baixo para cima se o alvo é invertido).
pub(crate) fn read_pixels(renderer: &mut GlesRenderer, framebuffer: &GlesTarget<'_>, full: Rectangle<i32, Physical>) -> Option<Vec<u8>> {
    let region = Rectangle::from_size((full.size.w, full.size.h).into());
    let mapping = renderer.copy_framebuffer(framebuffer, region, Fourcc::Abgr8888).ok()?;
    renderer.map_texture(&mapping).ok().map(|b| b.to_vec())
}

/// Copia os pixels RGBA da tela (`rgba`, a tela inteira `full`) da região para o
/// buffer em memória compartilhada (XRGB/ARGB, ordem BGRA na memória).
fn write_shm(buffer: &WlBuffer, rgba: &[u8], full: Rectangle<i32, Physical>, region: Rectangle<i32, Physical>, flipped: bool) -> bool {
    smithay::wayland::shm::with_buffer_contents_mut(buffer, |ptr, len, b| {
        // SAFETY: o buffer é do cliente e tem `len` bytes a partir de `ptr`
        // (conferido pelo Smithay); só escrevemos dentro dele.
        let dst = unsafe { std::slice::from_raw_parts_mut(ptr, len) };
        let (fw, fh) = (full.size.w as usize, full.size.h as usize);
        let stride = b.stride as usize;
        let offset = b.offset as usize;
        for row in 0..region.size.h as usize {
            let y = region.loc.y as usize + row;
            // Alvo com a origem embaixo (winit): as linhas vêm de baixo para cima.
            let src_y = if flipped { fh - 1 - y } else { y };
            for col in 0..region.size.w as usize {
                let x = region.loc.x as usize + col;
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
    })
    .is_ok()
}

/// O buffer dmabuf do app serve para a região `region` (largura, altura): mesmo
/// tamanho e um formato de 32 bits sem canal alfa útil, como o anunciado.
fn dma_fits(code: Fourcc, size: (i32, i32), region: (i32, i32)) -> bool {
    matches!(code, Fourcc::Xrgb8888 | Fourcc::Argb8888) && size == region
}

/// Retângulos do blit (origem no quadro, destino no buffer do app). O
/// `glBlitFramebuffer` conta as linhas de baixo para cima: no DRM e no backend sem
/// tela o quadro já está "de cabeça para baixo" em relação ao GL (a linha `y` do
/// quadro é a linha `y` da memória), igual ao buffer do app, então copia direto. No
/// winit o quadro está na orientação do GL (origem embaixo): a região vem de
/// `H - (y + h)` e o destino é invertido (altura negativa) para o app receber de pé.
/// `y_inverted`: o buffer do app foi criado com `Y_INVERT` (linhas de baixo para cima):
/// o destino troca de sentido mais uma vez.
fn blit_rects(full: (i32, i32), region: Rectangle<i32, Physical>, flipped: bool, y_inverted: bool) -> (Rectangle<i32, Physical>, Rectangle<i32, Physical>) {
    let (w, h) = (region.size.w, region.size.h);
    let src = if flipped { Rectangle::new((region.loc.x, full.1 - (region.loc.y + h)).into(), (w, h).into()) } else { region };
    if flipped != y_inverted {
        // Altura negativa: o `glBlitFramebuffer` aceita o destino de cabeça para baixo
        // (y de `h` até 0). A altura muda depois de criar o retângulo porque o Smithay
        // não aceita criar tamanho negativo (para o programa em modo de depuração).
        let mut dst = Rectangle::new((0, h).into(), (w, h).into());
        dst.size.h = -h;
        (src, dst)
    } else {
        (src, Rectangle::from_size((w, h).into()))
    }
}

/// Espera a GPU terminar as cópias. A cópia na GPU é assíncrona: sem esperar, o app
/// (o portal) pode ler o buffer antes de a GPU terminar, e o quadro sai rasgado ou
/// preto. Um blit de 1080p leva frações de milissegundo.
///
/// Por uma cerca do EGL, que não mexe no contexto: o `with_context` do Smithay deixa o
/// contexto sem a superfície da janela, e no winit a troca de buffers logo depois
/// falhava ("context lost"). Sem cerca (driver antigo), `glFinish`, exceto no winit.
/// `false`: não deu para saber se terminou (mais de 500 ms, ou winit sem cerca).
fn wait_gpu(renderer: &mut GlesRenderer) -> bool {
    let display = renderer.egl_context().display().clone();
    match smithay::backend::egl::fence::EGLFence::create(&display) {
        Ok(fence) => {
            let done = fence.client_wait(Some(std::time::Duration::from_millis(500)), true).unwrap_or(false);
            if !done {
                tracing::debug!("captura por dmabuf: a GPU demorou mais de 500 ms");
            }
            done
        }
        Err(_) if !crate::clip::flipped() => renderer.with_context(|gl| unsafe { gl.Finish() }).is_ok(),
        Err(_) => false,
    }
}

/// Winit: deixa a janela do anfitrião ligada ao contexto do EGL e lendo do buffer de
/// trás. A leitura dos pixels do Smithay (`copy_framebuffer`, usada pelo `grim`, pela
/// gravação e pelas capturas do mide) termina com `glReadBuffer(GL_NONE)` e com o
/// contexto sem a janela: depois dela, a cópia na GPU saía preta e a troca de buffers
/// da janela falhava (EGL_BAD_SURFACE, "context lost", e o mide parava de desenhar).
/// Um quadro vazio no alvo da janela liga a janela de novo, sem desenhar nada.
pub fn reattach_window(renderer: &mut GlesRenderer, framebuffer: &mut GlesTarget<'_>, size: smithay::utils::Size<i32, Physical>) {
    use smithay::backend::renderer::{Frame, Renderer};
    match renderer.render(framebuffer, size, smithay::utils::Transform::Normal) {
        Ok(mut frame) => {
            let _ = frame.with_context(|gl| unsafe { gl.ReadBuffer(smithay::backend::renderer::gles::ffi::BACK) });
            let _ = frame.finish();
        }
        Err(err) => tracing::debug!("janela do winit: não religou o contexto: {err}"),
    }
}

/// Copia a região do quadro para o buffer dmabuf do app, na GPU.
fn blit_dma(
    renderer: &mut GlesRenderer,
    framebuffer: &mut GlesTarget<'_>,
    mut dmabuf: Dmabuf,
    full: Rectangle<i32, Physical>,
    region: Rectangle<i32, Physical>,
    flipped: bool,
) -> Result<(), smithay::backend::renderer::gles::GlesError> {
    if flipped {
        reattach_window(renderer, framebuffer, full.size);
    }
    // O Smithay guarda o framebuffer de cada dmabuf (o portal reaproveita poucos
    // buffers do PipeWire): o custo de preparar o alvo não se repete a cada quadro.
    let y_inverted = dmabuf.y_inverted();
    let mut target = renderer.bind(&mut dmabuf)?;
    let (src, dst) = blit_rects((full.size.w, full.size.h), region, flipped, y_inverted);
    renderer.blit(framebuffer, &mut target, src, dst, TextureFilter::Nearest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dmabuf_do_tamanho_e_formato_certos() {
        assert!(dma_fits(Fourcc::Xrgb8888, (300, 200), (300, 200)));
        assert!(dma_fits(Fourcc::Argb8888, (300, 200), (300, 200)));
        assert!(!dma_fits(Fourcc::Xrgb8888, (300, 201), (300, 200)));
        assert!(!dma_fits(Fourcc::Nv12, (300, 200), (300, 200)));
        assert!(!dma_fits(Fourcc::Abgr2101010, (300, 200), (300, 200)));
    }

    #[test]
    fn blit_sem_inversao_copia_a_regiao() {
        let region = Rectangle::new((40, 50).into(), (300, 200).into());
        let (src, dst) = blit_rects((1366, 768), region, false, false);
        assert_eq!(src, region);
        assert_eq!(dst, Rectangle::from_size((300, 200).into()));
    }

    #[test]
    fn blit_invertido_le_de_baixo_e_desenha_de_pe() {
        let region = Rectangle::new((40, 50).into(), (300, 200).into());
        let (src, dst) = blit_rects((1366, 768), region, true, false);
        // Linhas 50..250 de cima são 518..718 contadas de baixo.
        assert_eq!(src, Rectangle::new((40, 518).into(), (300, 200).into()));
        assert_eq!((dst.loc.y, dst.loc.y + dst.size.h), (200, 0));
        // Tela inteira: a origem é a tela toda.
        let (src, _) = blit_rects((1366, 768), Rectangle::from_size((1366, 768).into()), true, false);
        assert_eq!(src, Rectangle::from_size((1366, 768).into()));
    }

    #[test]
    fn buffer_com_y_invert_recebe_o_destino_invertido() {
        let region = Rectangle::new((40, 50).into(), (300, 200).into());
        let (src, dst) = blit_rects((1366, 768), region, false, true);
        assert_eq!(src, region);
        assert_eq!((dst.loc.y, dst.loc.y + dst.size.h), (200, 0));
        // No winit as duas inversões se anulam.
        let (_, dst) = blit_rects((1366, 768), region, true, true);
        assert_eq!(dst, Rectangle::from_size((300, 200).into()));
    }
}
