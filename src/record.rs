//! Gravar a tela em vídeo (A21), como o ⌘⇧5 do Mac: a tela inteira ou uma área, com
//! o som do sistema e/ou o microfone.
//!
//! As imagens saem do quadro que o mide acabou de desenhar (como no `screencopy`), e
//! a codificação é do `ffmpeg` (programa externo, dependência obrigatória do mide):
//! os pixels vão pela entrada dele, com a hora de chegada de cada quadro; o som ele
//! mesmo lê do PipeWire (pelo PulseAudio dele): o do sistema pelo monitor da saída
//! padrão e o microfone pela entrada padrão. O vídeo fica em Vídeos/Gravações de
//! tela; no fim, um aviso diz onde.
//!
//! Com a tela parada não há quadro novo: uma linha de execução manda de novo o último
//! a cada 100 ms, para o vídeo acompanhar o tempo (e o som).

use std::{
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
    sync::mpsc::{sync_channel, RecvTimeoutError, SyncSender},
    time::{Duration, Instant},
};

use smithay::{
    backend::renderer::gles::{GlesRenderer, GlesTarget},
    output::Output,
    utils::{Physical, Rectangle},
};

use crate::state::State;

/// Gravação em andamento.
pub struct Recording {
    output: String,
    /// Área gravada, em pixels da tela (largura e altura pares, como o vídeo exige).
    region: Rectangle<i32, Physical>,
    frames: Option<SyncSender<Vec<u8>>>,
    pub started: Instant,
    pub path: PathBuf,
    /// Os primeiros quadros ficam de fora (a janela de escolha ainda sumindo).
    skip_until: Instant,
}

/// O que gravar: a tela (pelo nome; vazio, a principal), a área em pontos nela (ou a
/// tela toda) e os sons.
pub struct Request {
    pub output: Option<String>,
    pub area: Option<(i32, i32, i32, i32)>,
    pub system_audio: bool,
    pub microphone: bool,
}

/// Lê o pedido do canal de controle: `record start [full|x y w h] [sistema] [microfone] [tela NOME]`.
pub fn parse_request(words: &[&str]) -> Request {
    let mut request = Request { output: None, area: None, system_audio: false, microphone: false };
    let mut i = 0;
    while i < words.len() {
        match words[i] {
            "sistema" => request.system_audio = true,
            "microfone" => request.microphone = true,
            "tela" if i + 1 < words.len() => {
                request.output = Some(words[i + 1].to_string());
                i += 1;
            }
            "full" => {}
            _ => {
                let nums: Option<Vec<i32>> = words.get(i..i + 4).map(|w| w.iter().filter_map(|n| n.parse().ok()).collect());
                if let Some(n) = nums.filter(|n| n.len() == 4) {
                    request.area = Some((n[0], n[1], n[2], n[3]));
                    i += 3;
                }
            }
        }
        i += 1;
    }
    request
}

/// Argumentos do `ffmpeg` para gravar `w`×`h` em `path`, com os sons pedidos.
pub fn ffmpeg_args(w: i32, h: i32, system_audio: bool, microphone: bool, path: &str) -> Vec<String> {
    let mut args: Vec<String> = ["-hide_banner", "-loglevel", "error", "-y"].iter().map(|s| s.to_string()).collect();
    // Imagens pela entrada, com a hora de chegada de cada quadro.
    args.extend(
        ["-f", "rawvideo", "-pixel_format", "rgba", "-video_size", &format!("{w}x{h}"), "-use_wallclock_as_timestamps", "1", "-thread_queue_size", "64", "-i", "pipe:0"]
            .iter()
            .map(|s| s.to_string()),
    );
    let mut sounds = 0;
    if system_audio {
        args.extend(["-f", "pulse", "-thread_queue_size", "1024", "-i", "@DEFAULT_MONITOR@"].iter().map(|s| s.to_string()));
        sounds += 1;
    }
    if microphone {
        args.extend(["-f", "pulse", "-thread_queue_size", "1024", "-i", "@DEFAULT_SOURCE@"].iter().map(|s| s.to_string()));
        sounds += 1;
    }
    args.extend(["-map", "0:v"].iter().map(|s| s.to_string()));
    match sounds {
        2 => args.extend(["-filter_complex", "[1:a][2:a]amix=inputs=2:duration=longest[a]", "-map", "[a]"].iter().map(|s| s.to_string())),
        1 => args.extend(["-map", "1:a"].iter().map(|s| s.to_string())),
        _ => {}
    }
    // H.264 a 30 quadros por segundo (quadros repetidos quando a tela não muda) e AAC.
    args.extend(["-c:v", "libx264", "-preset", "veryfast", "-crf", "23", "-pix_fmt", "yuv420p", "-fps_mode", "cfr", "-r", "30"].iter().map(|s| s.to_string()));
    if sounds > 0 {
        // O som (do PipeWire) não acaba sozinho: o vídeo termina com as imagens.
        args.extend(["-c:a", "aac", "-b:a", "160k", "-shortest"].iter().map(|s| s.to_string()));
    }
    args.extend(["-movflags", "+faststart", path].iter().map(|s| s.to_string()));
    args
}

/// Começa a gravar. Devolve o arquivo do vídeo.
pub fn start(state: &mut State, request: Request) -> Result<PathBuf, String> {
    if state.recording.is_some() {
        return Err("já está gravando".into());
    }
    let output = match &request.output {
        Some(name) => state.space.outputs().find(|o| o.name() == *name).cloned(),
        None => state.primary_output(),
    }
    .ok_or("tela não encontrada")?;
    let mode = output.current_mode().ok_or("tela sem modo")?;
    let scale = output.current_scale().fractional_scale();
    let screen = Rectangle::<i32, Physical>::from_size(mode.size);
    // Área em pontos → pixels, dentro da tela, com lados pares.
    let region = match request.area {
        Some((x, y, w, h)) => {
            let r = Rectangle::<i32, Physical>::new(
                ((x as f64 * scale).round() as i32, (y as f64 * scale).round() as i32).into(),
                ((w as f64 * scale).round() as i32, (h as f64 * scale).round() as i32).into(),
            );
            r.intersection(screen).ok_or("área fora da tela")?
        }
        None => screen,
    };
    let region = Rectangle::new(region.loc, ((region.size.w / 2 * 2).max(2), (region.size.h / 2 * 2).max(2)).into());
    let dir = mide::places::videos_dir().join("Gravações de tela");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let stamp = chrono::Local::now().format("%Y-%m-%d %H-%M-%S");
    let path = dir.join(format!("Gravação de tela {stamp}.mp4"));
    let args = ffmpeg_args(region.size.w, region.size.h, request.system_audio, request.microphone, &path.to_string_lossy());
    let mut child = Command::new(std::env::var("MIDE_FFMPEG").unwrap_or_else(|_| "ffmpeg".into()))
        .args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| if e.kind() == std::io::ErrorKind::NotFound { "o ffmpeg não está instalado (sudo apt install ffmpeg)".to_string() } else { format!("ffmpeg: {e}") })?;
    let mut stdin = child.stdin.take().ok_or("ffmpeg sem entrada")?;
    // Poucos quadros na fila: se o ffmpeg atrasar, os novos esperam (são descartados no
    // desenho, que não pode travar).
    let (tx, rx) = sync_channel::<Vec<u8>>(4);
    let video = path.clone();
    std::thread::Builder::new()
        .name("gravacao".into())
        .spawn(move || {
            let mut last: Option<Vec<u8>> = None;
            let mut broken = false;
            loop {
                let frame = match rx.recv_timeout(Duration::from_millis(100)) {
                    Ok(frame) => {
                        last = Some(frame);
                        last.as_ref()
                    }
                    // Tela parada: o último quadro de novo (o vídeo acompanha o tempo).
                    Err(RecvTimeoutError::Timeout) => last.as_ref(),
                    Err(RecvTimeoutError::Disconnected) => break,
                };
                if let Some(frame) = frame {
                    if stdin.write_all(frame).is_err() {
                        broken = true;
                        break;
                    }
                }
            }
            // Fim: o último quadro mais uma vez (até a hora de parar) e fecha a entrada.
            if let (Some(frame), false) = (last.as_ref(), broken) {
                let _ = stdin.write_all(frame);
            }
            drop(stdin);
            // Segurança: se o ffmpeg não terminar em 15 s depois de parar, encerra.
            let deadline = Instant::now() + Duration::from_secs(15);
            while matches!(child.try_wait(), Ok(None)) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(100));
            }
            if matches!(child.try_wait(), Ok(None)) {
                tracing::warn!("gravação: o ffmpeg não terminou; encerrando");
                let _ = child.kill();
            }
            let output = child.wait_with_output();
            match output {
                Ok(out) if out.status.success() => {
                    tracing::info!("gravação salva: {}", video.display());
                    mide::sounds::play("complete");
                    mide::notify::send("Gravação de tela", "Gravação salva", &video.display().to_string());
                }
                Ok(out) => {
                    let err = String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("").to_string();
                    tracing::warn!("gravação: o ffmpeg saiu com {} ({err})", out.status);
                    mide::notify::send("Gravação de tela", "A gravação não foi salva", &err);
                }
                Err(err) => tracing::warn!("gravação: {err}"),
            }
        })
        .map_err(|e| e.to_string())?;
    tracing::info!(
        "gravando {} ({}x{} em {},{}){}{} em {}",
        output.name(),
        region.size.w,
        region.size.h,
        region.loc.x,
        region.loc.y,
        if request.system_audio { ", som do sistema" } else { "" },
        if request.microphone { ", microfone" } else { "" },
        path.display()
    );
    let now = Instant::now();
    state.recording = Some(Recording { output: output.name(), region, frames: Some(tx), started: now, path: path.clone(), skip_until: now + Duration::from_millis(250) });
    // Primeiro quadro já (a tela pode estar parada).
    state.redraw = true;
    state.wake();
    Ok(path)
}

/// Para a gravação (o `ffmpeg` termina o arquivo numa linha de execução).
pub fn stop(state: &mut State) -> Option<PathBuf> {
    let mut recording = state.recording.take()?;
    recording.frames = None;
    tracing::info!("gravação parada ({:.0} s)", recording.started.elapsed().as_secs_f32());
    Some(recording.path)
}

/// Depois de desenhar `output`: manda a área gravada ao `ffmpeg`.
pub fn feed(renderer: &mut GlesRenderer, framebuffer: &GlesTarget<'_>, output: &Output, state: &mut State) {
    let Some(recording) = state.recording.as_ref() else {
        return;
    };
    if recording.output != output.name() {
        return;
    }
    if Instant::now() < recording.skip_until {
        state.redraw = true;
        return;
    }
    // O cursor entra na imagem (no DRM, a cena o desenha em vez do plano de hardware).
    state.screencopy.cursor_until = Some(Instant::now() + Duration::from_millis(500));
    let Some(mode) = output.current_mode() else {
        return;
    };
    let full = Rectangle::<i32, Physical>::from_size(mode.size);
    let Some(rgba) = crate::screencopy::read_pixels(renderer, framebuffer, full) else {
        return;
    };
    let r = recording.region;
    let (fw, fh) = (full.size.w as usize, full.size.h as usize);
    let flipped = crate::clip::flipped();
    let mut frame = Vec::with_capacity((r.size.w * r.size.h * 4) as usize);
    for row in 0..r.size.h as usize {
        let y = r.loc.y as usize + row;
        let src_y = if flipped { fh.saturating_sub(1 + y) } else { y };
        let start = (src_y * fw + r.loc.x as usize) * 4;
        let end = start + r.size.w as usize * 4;
        if end <= rgba.len() {
            frame.extend_from_slice(&rgba[start..end]);
        }
    }
    if frame.len() != (r.size.w * r.size.h * 4) as usize {
        return;
    }
    if let Some(tx) = &recording.frames {
        // Fila cheia (o ffmpeg atrasou): este quadro fica de fora.
        let _ = tx.try_send(frame);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pedido_e_argumentos() {
        let r = parse_request(&["10", "20", "300", "200", "sistema", "microfone"]);
        assert_eq!(r.area, Some((10, 20, 300, 200)));
        assert!(r.system_audio && r.microphone);
        let full = parse_request(&["full"]);
        assert!(full.area.is_none() && !full.system_audio);
        let args = ffmpeg_args(1366, 768, true, true, "/tmp/x.mp4");
        assert!(args.windows(2).any(|w| w[0] == "-video_size" && w[1] == "1366x768"));
        assert!(args.iter().any(|a| a.contains("amix=inputs=2")));
        assert_eq!(args.last().map(String::as_str), Some("/tmp/x.mp4"));
        let silent = ffmpeg_args(100, 100, false, false, "/tmp/y.mp4");
        assert!(!silent.iter().any(|a| a == "aac"));
        assert!(args.iter().any(|a| a == "-shortest"));
    }
}
