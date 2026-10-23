//! Cliente de teste da captura por dmabuf (A25): faz o que o `xdg-desktop-portal-wlr`
//! faz com a tela quando o compositor oferece o `linux-dmabuf`, sem o portal nem o
//! PipeWire.
//!
//! 1. Pega a placa principal no feedback do `zwp_linux_dmabuf_v1` (`main_device`) e
//!    abre o render node dela com o GBM.
//! 2. Pede a tela (ou uma região) ao `wlr-screencopy` e espera o formato por dmabuf.
//! 3. Cria um buffer na GPU desse tamanho (linear ou com os modificadores da placa,
//!    "tiled"), entrega ao mide e pede a cópia.
//! 4. Com `ready`, lê o buffer pelo GBM (o Mesa desfaz o tiling ao mapear) e salva um
//!    PNG; com `failed`, sai com erro.
//!
//! Uso: `mide-screencopy-dmabuf SAIDA.png [--linear | --modifier HEX] [--region X,Y,L,A]`
//! (região em pontos da tela; sem `--linear` nem `--modifier`, o GBM escolhe entre os
//! modificadores da tabela). `--modifiers` só lista os modificadores do XRGB8888.
//! Imprime `ok` ou `failed` e o motivo.

use std::{
    fs::File,
    os::{fd::AsFd, unix::fs::FileExt},
};

use smithay::{
    backend::{
        allocator::{
            dmabuf::{Dmabuf, DmabufFlags},
            Fourcc,
        },
        drm::DrmDeviceFd,
        egl::{EGLContext, EGLDisplay},
        renderer::{gles::GlesRenderer, ExportMem, ImportDma},
    },
    reexports::gbm::{BufferObject, BufferObjectFlags, Device as GbmDevice, Format as GbmFormat, Modifier},
    utils::{DeviceFd, Rectangle},
};
use smithay_client_toolkit::reexports::{
    client::{
        globals::{registry_queue_init, GlobalListContents},
        protocol::{wl_buffer, wl_output, wl_registry},
        Connection, Dispatch, Proxy, QueueHandle,
    },
    protocols::wp::linux_dmabuf::zv1::client::{
        zwp_linux_buffer_params_v1::{self, ZwpLinuxBufferParamsV1},
        zwp_linux_dmabuf_feedback_v1::{self, ZwpLinuxDmabufFeedbackV1},
        zwp_linux_dmabuf_v1::{self, ZwpLinuxDmabufV1},
    },
    protocols_wlr::screencopy::v1::client::{
        zwlr_screencopy_frame_v1::{self, ZwlrScreencopyFrameV1},
        zwlr_screencopy_manager_v1::{self, ZwlrScreencopyManagerV1},
    },
};

/// XRGB8888 em fourcc ("XR24").
const XRGB8888: u32 = 0x3432_5258;

#[derive(Default)]
struct Teste {
    main_device: Option<u64>,
    table: Option<(std::os::fd::OwnedFd, u32)>,
    feedback_done: bool,
    dmabuf_format: Option<(u32, u32, u32)>,
    buffer_done: bool,
    result: Option<Result<(), String>>,
}

fn fail(msg: &str) -> ! {
    println!("failed: {msg}");
    std::process::exit(1);
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mut out = None;
    let mut linear = false;
    let mut region: Option<(i32, i32, i32, i32)> = None;
    let mut explicit: Option<u64> = None;
    let mut list = false;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--linear" => linear = true,
            "--modifiers" => list = true,
            "--modifier" => {
                let v = args.next().unwrap_or_default();
                explicit = Some(u64::from_str_radix(v.trim_start_matches("0x"), 16).unwrap_or_else(|_| fail("--modifier HEX")));
            }
            "--region" => {
                let v: Vec<i32> = args.next().unwrap_or_default().split(',').filter_map(|n| n.trim().parse().ok()).collect();
                if v.len() != 4 {
                    fail("--region X,Y,L,A");
                }
                region = Some((v[0], v[1], v[2], v[3]));
            }
            _ => out = Some(a),
        }
    }
    let out = out.unwrap_or_else(|| fail("uso: mide-screencopy-dmabuf SAIDA.png [--linear] [--region X,Y,L,A]"));

    let conn = Connection::connect_to_env().unwrap_or_else(|e| fail(&format!("sem compositor: {e}")));
    let (globals, mut queue) = registry_queue_init::<Teste>(&conn).unwrap_or_else(|e| fail(&format!("registro: {e}")));
    let qh = queue.handle();
    let dmabuf: ZwpLinuxDmabufV1 = globals.bind(&qh, 4..=5, ()).unwrap_or_else(|_| fail("o compositor não oferece linux-dmabuf v4"));
    let manager: ZwlrScreencopyManagerV1 = globals.bind(&qh, 3..=3, ()).unwrap_or_else(|_| fail("sem wlr-screencopy v3"));
    let output: wl_output::WlOutput = globals.bind(&qh, 1..=4, ()).unwrap_or_else(|_| fail("sem tela"));
    let mut t = Teste::default();

    // 1. A placa principal.
    let _feedback = dmabuf.get_default_feedback(&qh, ());
    while !t.feedback_done {
        queue.blocking_dispatch(&mut t).unwrap_or_else(|e| fail(&format!("despacho: {e}")));
    }
    let dev = t.main_device.unwrap_or_else(|| fail("feedback sem main_device"));
    let node = std::fs::read_dir("/dev/dri")
        .unwrap_or_else(|e| fail(&format!("/dev/dri: {e}")))
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("renderD")))
        .find(|p| rustix::fs::stat(p).is_ok_and(|s| s.st_rdev == dev))
        .unwrap_or_else(|| fail("render node do main_device não encontrado"));
    let file = std::fs::OpenOptions::new().read(true).write(true).open(&node).unwrap_or_else(|e| fail(&format!("{}: {e}", node.display())));
    let gbm = GbmDevice::new(DrmDeviceFd::new(DeviceFd::from(std::os::fd::OwnedFd::from(file)))).unwrap_or_else(|e| fail(&format!("gbm: {e}")));

    // Modificadores do XRGB8888 que o compositor importa (tabela do feedback).
    let mut modifiers = Vec::new();
    if let Some((fd, size)) = &t.table {
        let mut bytes = vec![0u8; *size as usize];
        let file = File::from(fd.try_clone().unwrap_or_else(|e| fail(&format!("tabela: {e}"))));
        if file.read_exact_at(&mut bytes, 0).is_ok() {
            for entry in bytes.as_chunks::<16>().0 {
                let code = u32::from_ne_bytes(entry[0..4].try_into().unwrap_or_default());
                let modifier = u64::from_ne_bytes(entry[8..16].try_into().unwrap_or_default());
                if code == XRGB8888 {
                    modifiers.push(Modifier::from(modifier));
                }
            }
        }
    }

    if list {
        for m in &modifiers {
            println!("{:#018x} {m:?}", u64::from(*m));
        }
        return;
    }

    // 2. A captura.
    let frame = match region {
        Some((x, y, w, h)) => manager.capture_output_region(0, &output, x, y, w, h, &qh, ()),
        None => manager.capture_output(0, &output, &qh, ()),
    };
    while !t.buffer_done && t.result.is_none() {
        queue.blocking_dispatch(&mut t).unwrap_or_else(|e| fail(&format!("despacho: {e}")));
    }
    if let Some(Err(e)) = &t.result {
        fail(e);
    }
    let (fourcc, w, h) = t.dmabuf_format.unwrap_or_else(|| fail("o quadro não anunciou linux_dmabuf"));
    if fourcc != XRGB8888 {
        fail(&format!("formato inesperado {fourcc:#x}"));
    }

    // 3. O buffer na GPU.
    let chosen: Vec<Modifier> = if linear {
        vec![Modifier::Linear]
    } else if let Some(m) = explicit {
        if !modifiers.contains(&Modifier::from(m)) {
            fail(&format!("modificador {m:#x} fora da tabela do compositor"));
        }
        vec![Modifier::from(m)]
    } else {
        // "Tiled": o que a placa prefere entre os da tabela, menos o linear.
        modifiers.iter().copied().filter(|m| !matches!(m, Modifier::Linear | Modifier::Invalid)).collect()
    };
    if chosen.is_empty() {
        fail("nenhum modificador para o buffer");
    }
    let bo = gbm
        .create_buffer_object_with_modifiers2::<()>(w, h, GbmFormat::Xrgb8888, chosen.iter().copied(), BufferObjectFlags::RENDERING)
        .unwrap_or_else(|e| fail(&format!("buffer na GPU: {e}")));
    let modifier: u64 = bo.modifier().into();
    let params = dmabuf.create_params(&qh, ());
    for plane in 0..bo.plane_count() as i32 {
        let fd = bo.fd_for_plane(plane).unwrap_or_else(|_| fail("fd do plano"));
        params.add(fd.as_fd(), plane as u32, bo.offset(plane), bo.stride_for_plane(plane), (modifier >> 32) as u32, modifier as u32);
    }
    let buffer = params.create_immed(w as i32, h as i32, XRGB8888, zwp_linux_buffer_params_v1::Flags::empty(), &qh, ());
    frame.copy(&buffer);
    while t.result.is_none() {
        queue.blocking_dispatch(&mut t).unwrap_or_else(|e| fail(&format!("despacho: {e}")));
    }
    if let Some(Err(e)) = &t.result {
        fail(e);
    }

    // 4. A imagem, lida como um app que recebe o buffer (o OBS pelo PipeWire) lê:
    // importada no EGL da placa (que entende os modificadores, inclusive os de
    // compressão) e copiada da textura. Mapear direto pelo GBM não serve para buffers
    // comprimidos (CCS da Intel): o driver do teste não sabe que outro processo
    // escreveu neles e devolve os dados sem descomprimir.
    let rgb = read_by_egl(&gbm, &bo, w, h, modifier).unwrap_or_else(|e| fail(&format!("ler o buffer pelo EGL: {e}")));
    image::RgbImage::from_raw(w, h, rgb)
        .unwrap_or_else(|| fail("imagem"))
        .save(&out)
        .unwrap_or_else(|e| fail(&format!("salvar {out}: {e}")));
    println!("ok {w}x{h} modificador {:?} ({} plano(s)) placa {}", Modifier::from(modifier), bo.plane_count(), node.display());
    buffer.destroy();
}

/// Importa o buffer no EGL da placa e copia os pixels (linhas de cima para baixo, RGB).
fn read_by_egl(gbm: &GbmDevice<DrmDeviceFd>, bo: &BufferObject<()>, w: u32, h: u32, modifier: u64) -> Result<Vec<u8>, String> {
    let mut builder = Dmabuf::builder((w as i32, h as i32), Fourcc::Xrgb8888, Modifier::from(modifier), DmabufFlags::empty());
    for plane in 0..bo.plane_count() as i32 {
        let fd = bo.fd_for_plane(plane).map_err(|_| "fd do plano".to_string())?;
        builder.add_plane(fd, plane as u32, bo.offset(plane), bo.stride_for_plane(plane));
    }
    let dmabuf = builder.build().ok_or("dmabuf sem planos")?;
    // SAFETY: o display guarda uma cópia do dispositivo GBM; o contexto é só deste
    // renderizador.
    let egl = unsafe { EGLDisplay::new(gbm.clone()) }.map_err(|e| e.to_string())?;
    let context = EGLContext::new(&egl).map_err(|e| e.to_string())?;
    let mut renderer = unsafe { GlesRenderer::new(context) }.map_err(|e| e.to_string())?;
    let texture = renderer.import_dmabuf(&dmabuf, None).map_err(|e| e.to_string())?;
    let mapping = renderer
        .copy_texture(&texture, Rectangle::from_size((w as i32, h as i32).into()), Fourcc::Abgr8888)
        .map_err(|e| e.to_string())?;
    let rgba = renderer.map_texture(&mapping).map_err(|e| e.to_string())?;
    Ok(rgba.as_chunks::<4>().0.iter().flat_map(|p| [p[0], p[1], p[2]]).collect())
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Teste {
    fn event(_: &mut Self, _: &wl_registry::WlRegistry, _: wl_registry::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<wl_output::WlOutput, ()> for Teste {
    fn event(_: &mut Self, _: &wl_output::WlOutput, _: wl_output::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<ZwpLinuxDmabufV1, ()> for Teste {
    fn event(_: &mut Self, _: &ZwpLinuxDmabufV1, _: zwp_linux_dmabuf_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<ZwpLinuxDmabufFeedbackV1, ()> for Teste {
    fn event(t: &mut Self, _: &ZwpLinuxDmabufFeedbackV1, event: zwp_linux_dmabuf_feedback_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match event {
            zwp_linux_dmabuf_feedback_v1::Event::MainDevice { device } => {
                if device.len() >= 8 {
                    t.main_device = Some(u64::from_ne_bytes(device[..8].try_into().unwrap_or_default()));
                }
            }
            zwp_linux_dmabuf_feedback_v1::Event::FormatTable { fd, size } => t.table = Some((fd, size)),
            zwp_linux_dmabuf_feedback_v1::Event::Done => t.feedback_done = true,
            _ => {}
        }
    }
}

impl Dispatch<ZwpLinuxBufferParamsV1, ()> for Teste {
    fn event(t: &mut Self, _: &ZwpLinuxBufferParamsV1, event: zwp_linux_buffer_params_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let zwp_linux_buffer_params_v1::Event::Failed = event {
            t.result = Some(Err("o compositor recusou o buffer".into()));
        }
    }
}

impl Dispatch<wl_buffer::WlBuffer, ()> for Teste {
    fn event(_: &mut Self, _: &wl_buffer::WlBuffer, _: wl_buffer::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<ZwlrScreencopyManagerV1, ()> for Teste {
    fn event(_: &mut Self, _: &ZwlrScreencopyManagerV1, _: zwlr_screencopy_manager_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<ZwlrScreencopyFrameV1, ()> for Teste {
    fn event(t: &mut Self, frame: &ZwlrScreencopyFrameV1, event: zwlr_screencopy_frame_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match event {
            zwlr_screencopy_frame_v1::Event::LinuxDmabuf { format, width, height } => t.dmabuf_format = Some((format, width, height)),
            zwlr_screencopy_frame_v1::Event::BufferDone => t.buffer_done = true,
            zwlr_screencopy_frame_v1::Event::Ready { .. } => t.result = Some(Ok(())),
            zwlr_screencopy_frame_v1::Event::Failed => t.result = Some(Err(format!("o compositor respondeu failed ({})", frame.id()))),
            _ => {}
        }
    }
}
