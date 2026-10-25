//! Núcleo do player de vídeo do mide (decisão do usuário em 2026-10-25: o `ffmpeg`,
//! que já é dependência do mide, decodifica como programa à parte; o resto é do mide).
//!
//! - `probe` lê duração, tamanho, quadros por segundo e se há som (`ffprobe`);
//! - `Player` toca: um `ffmpeg` entrega os quadros já no tamanho da tela, em RGBA, por
//!   um cano, lidos numa thread com fila curta (a decodificação não corre na frente);
//!   outro `ffmpeg` toca o som direto no servidor de som (PipeWire/PulseAudio).
//! - O relógio é do player: pausar e pular param os dois programas e recomeçam na
//!   posição nova (`-ss`), o que é simples e não deixa som e imagem se separarem.
//!
//! O mesmo `Player` serve à janela do player e à miniatura que toca no Arquivos.

use std::{
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::mpsc::{sync_channel, Receiver},
    time::Instant,
};

/// Dados do vídeo.
#[derive(Debug, Clone, PartialEq)]
pub struct Info {
    /// Duração em segundos (0 se o arquivo não diz).
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    /// Quadros por segundo (30 se o arquivo não diz).
    pub fps: f64,
    pub has_audio: bool,
}

/// Lê os dados do vídeo pelo `ffprobe`.
pub fn probe(path: &Path) -> Option<Info> {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-show_entries", "format=duration:stream=codec_type,width,height,avg_frame_rate", "-of", "compact"])
        .arg(path)
        .output()
        .ok()?;
    parse_probe(&String::from_utf8_lossy(&out.stdout))
}

/// Lê a saída `compact` do `ffprobe` (uma linha por fluxo e uma do formato).
pub fn parse_probe(text: &str) -> Option<Info> {
    let mut info = Info { duration: 0.0, width: 0, height: 0, fps: 30.0, has_audio: false };
    let mut video = false;
    for line in text.lines() {
        let fields: Vec<(&str, &str)> = line.split('|').filter_map(|f| f.split_once('=')).collect();
        let get = |k: &str| fields.iter().find(|(key, _)| *key == k).map(|(_, v)| *v);
        match (line.split('|').next(), get("codec_type")) {
            (Some("stream"), Some("video")) if !video => {
                video = true;
                info.width = get("width").and_then(|v| v.parse().ok()).unwrap_or(0);
                info.height = get("height").and_then(|v| v.parse().ok()).unwrap_or(0);
                if let Some((n, d)) = get("avg_frame_rate").and_then(|r| r.split_once('/')) {
                    if let (Ok(n), Ok(d)) = (n.parse::<f64>(), d.parse::<f64>()) {
                        if n > 0.0 && d > 0.0 {
                            info.fps = (n / d).clamp(1.0, 120.0);
                        }
                    }
                }
            }
            (Some("stream"), Some("audio")) => info.has_audio = true,
            (Some("format"), _) => info.duration = get("duration").and_then(|v| v.parse().ok()).unwrap_or(0.0),
            _ => {}
        }
    }
    (video && info.width > 0 && info.height > 0).then_some(info)
}

/// Maior tamanho (em pixels, pares) com a proporção do vídeo que cabe em `max`.
pub fn fit(info: &Info, max: (u32, u32)) -> (u32, u32) {
    let (w, h) = (info.width.max(1) as f64, info.height.max(1) as f64);
    let k = (max.0.max(2) as f64 / w).min(max.1.max(2) as f64 / h);
    let even = |v: f64| ((v.floor() as u32) & !1).max(2);
    (even(w * k), even(h * k))
}

/// Um quadro: pixels RGBA opacos, tamanho e o instante do vídeo (segundos).
#[derive(Debug, Clone)]
pub struct Frame {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub time: f64,
}

/// Player de um arquivo de vídeo.
pub struct Player {
    pub path: PathBuf,
    pub info: Info,
    /// Tamanho dos quadros pedidos ao `ffmpeg`, em pixels.
    size: (u32, u32),
    frames: Option<Receiver<Frame>>,
    video: Option<Child>,
    audio: Option<Child>,
    /// Tocando desde (relógio) e a posição do vídeo nesse instante.
    started: Option<Instant>,
    base: f64,
    pub volume: f32,
    pub muted: bool,
    /// Volta ao começo no fim (miniatura no Arquivos).
    pub looped: bool,
    /// Toca o som (a miniatura do Arquivos também toca, como no Finder).
    with_audio: bool,
    current: Option<Frame>,
    /// O próximo quadro já lido, esperando a hora dele.
    next: Option<Frame>,
    ended: bool,
}

impl Player {
    /// Abre o vídeo, parado no começo, com quadros de até `max` pixels.
    pub fn open(path: &Path, max: (u32, u32), with_audio: bool) -> Option<Self> {
        let info = probe(path)?;
        let size = fit(&info, max);
        Some(Self {
            path: path.to_path_buf(),
            info,
            size,
            frames: None,
            video: None,
            audio: None,
            started: None,
            base: 0.0,
            volume: 1.0,
            muted: false,
            looped: false,
            with_audio,
            current: None,
            next: None,
            ended: false,
        })
    }

    pub fn playing(&self) -> bool {
        self.started.is_some()
    }

    pub fn ended(&self) -> bool {
        self.ended
    }

    /// Posição atual em segundos.
    pub fn position(&self) -> f64 {
        let t = match self.started {
            Some(at) => self.base + at.elapsed().as_secs_f64(),
            None => self.base,
        };
        if self.info.duration > 0.0 { t.min(self.info.duration) } else { t }
    }

    /// Quadro para mostrar agora.
    pub fn frame(&self) -> Option<&Frame> {
        self.current.as_ref()
    }

    /// Tamanho dos quadros, em pixels.
    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    pub fn play(&mut self) {
        if self.playing() {
            return;
        }
        if self.ended || (self.info.duration > 0.0 && self.base >= self.info.duration - 0.05) {
            self.base = 0.0;
        }
        self.ended = false;
        self.start(self.base, true);
    }

    pub fn pause(&mut self) {
        if !self.playing() {
            return;
        }
        self.base = self.position();
        self.stop_processes();
        self.started = None;
    }

    pub fn toggle(&mut self) {
        if self.playing() { self.pause() } else { self.play() }
    }

    /// Vai para `t` segundos (tocando continua tocando; parado mostra o quadro de lá).
    pub fn seek(&mut self, t: f64) {
        let t = if self.info.duration > 0.0 { t.clamp(0.0, (self.info.duration - 0.05).max(0.0)) } else { t.max(0.0) };
        let playing = self.playing();
        self.stop_processes();
        self.base = t;
        self.ended = false;
        self.next = None;
        self.start(t, playing);
    }

    /// Novo tamanho máximo dos quadros (janela redimensionada): recomeça a decodificação
    /// na posição atual.
    pub fn resize(&mut self, max: (u32, u32)) {
        let size = fit(&self.info, max);
        if size == self.size {
            return;
        }
        self.size = size;
        let t = self.position();
        let playing = self.playing();
        self.stop_processes();
        self.base = t;
        self.next = None;
        self.start(t, playing);
    }

    /// Volume (0 a 1) e mudo: o som recomeça na posição atual com o volume novo.
    pub fn set_volume(&mut self, volume: f32, muted: bool) {
        if (volume - self.volume).abs() < 0.001 && muted == self.muted {
            return;
        }
        self.volume = volume.clamp(0.0, 1.0);
        self.muted = muted;
        if self.playing() {
            if let Some(mut child) = self.audio.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
            self.audio = self.spawn_audio(self.position());
        }
    }

    /// Avança o relógio: troca o quadro mostrado quando chega a hora do próximo.
    /// Devolve se o quadro mudou.
    pub fn tick(&mut self) -> bool {
        let now = self.position();
        let mut changed = false;
        loop {
            if self.next.is_none() {
                let Some(rx) = self.frames.as_ref() else {
                    break;
                };
                match rx.try_recv() {
                    Ok(frame) => self.next = Some(frame),
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        // Acabaram os quadros: fim do vídeo (tocando).
                        self.frames = None;
                        if self.playing() {
                            self.finish();
                        }
                        break;
                    }
                }
            }
            // Parado (depois de pular), o primeiro quadro que chega é o mostrado.
            let due = self.next.as_ref().is_some_and(|f| f.time <= now + 0.005 || !self.playing() && self.current.is_none() || !self.playing());
            if !due {
                break;
            }
            self.current = self.next.take();
            changed = true;
            if !self.playing() {
                // Parado: só um quadro; o resto da decodificação não é necessário.
                self.stop_processes();
                break;
            }
        }
        if self.playing() && self.info.duration > 0.0 && now >= self.info.duration {
            self.finish();
        }
        changed
    }

    /// Tempo até o próximo quadro (para o app pedir o desenho na hora certa).
    pub fn next_due(&self) -> Option<std::time::Duration> {
        if !self.playing() {
            return None;
        }
        let wait = self.next.as_ref().map(|f| f.time - self.position()).unwrap_or(1.0 / self.info.fps);
        Some(std::time::Duration::from_secs_f64(wait.clamp(0.002, 0.1)))
    }

    fn finish(&mut self) {
        self.stop_processes();
        self.started = None;
        if self.looped {
            self.base = 0.0;
            self.start(0.0, true);
        } else {
            self.base = self.info.duration;
            self.ended = true;
        }
    }

    /// Começa os programas na posição `t`; `playing` liga o relógio (senão, só busca o
    /// quadro de lá).
    fn start(&mut self, t: f64, playing: bool) {
        let (w, h) = self.size;
        let fps = self.info.fps;
        let mut cmd = Command::new("ffmpeg");
        cmd.args(["-nostdin", "-loglevel", "error", "-ss", &format!("{t:.3}"), "-i"])
            .arg(&self.path)
            .args(["-an", "-sn", "-vf", &format!("fps={fps:.3},scale={w}:{h}:flags=bilinear,format=rgba"), "-f", "rawvideo", "-pix_fmt", "rgba", "-"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let Ok(mut child) = cmd.spawn() else {
            tracing::warn!("vídeo: o ffmpeg não abriu");
            return;
        };
        let Some(mut out) = child.stdout.take() else {
            return;
        };
        let (tx, rx) = sync_channel::<Frame>(3);
        let bytes = (w * h * 4) as usize;
        std::thread::spawn(move || {
            let mut n = 0u64;
            loop {
                let mut pixels = vec![0u8; bytes];
                if out.read_exact(&mut pixels).is_err() {
                    break;
                }
                let frame = Frame { pixels, width: w, height: h, time: t + n as f64 / fps };
                n += 1;
                if tx.send(frame).is_err() {
                    break;
                }
            }
        });
        self.video = Some(child);
        self.frames = Some(rx);
        if playing {
            self.audio = self.spawn_audio(t);
            self.started = Some(Instant::now());
            self.base = t;
        }
    }

    /// Som tocando direto no servidor de som, do instante `t`.
    fn spawn_audio(&self, t: f64) -> Option<Child> {
        // Nos testes, sem som (não toca nada na sessão de quem testa).
        if !self.with_audio || self.muted || !self.info.has_audio || std::env::var_os("MIDE_SYSTEM_DRY_RUN").is_some() {
            return None;
        }
        Command::new("ffmpeg")
            .args(["-nostdin", "-loglevel", "error", "-ss", &format!("{t:.3}"), "-i"])
            .arg(&self.path)
            .args(["-vn", "-sn", "-af", &format!("volume={:.2}", self.volume), "-f", "pulse", "Vídeo"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .ok()
    }

    fn stop_processes(&mut self) {
        self.frames = None;
        for child in [self.video.take(), self.audio.take()].into_iter().flatten() {
            let mut child = child;
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.stop_processes();
    }
}

/// Tempo como no QuickTime: "1:05" ou "1:02:03".
pub fn time_text(seconds: f64) -> String {
    let s = seconds.max(0.0).floor() as u64;
    let (h, m, s) = (s / 3600, (s / 60) % 60, s % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

/// O arquivo é um vídeo (pela extensão)?
pub fn is_video(path: &Path) -> bool {
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    matches!(ext.as_str(), "mp4" | "m4v" | "mov" | "mkv" | "webm" | "avi" | "wmv" | "flv" | "ogv" | "mpg" | "mpeg" | "3gp" | "ts")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_dados_do_ffprobe() {
        let text = "stream|codec_type=video|width=1920|height=1080|avg_frame_rate=30000/1001\nstream|codec_type=audio|avg_frame_rate=0/0\nformat|duration=12.500000\n";
        let info = parse_probe(text).unwrap();
        assert_eq!((info.width, info.height, info.has_audio), (1920, 1080, true));
        assert!((info.fps - 29.97).abs() < 0.01);
        assert!((info.duration - 12.5).abs() < 1e-9);
        assert_eq!(parse_probe("stream|codec_type=audio\nformat|duration=3\n"), None);
    }

    #[test]
    fn cabe_mantendo_a_proporcao() {
        let info = Info { duration: 1.0, width: 1920, height: 1080, fps: 30.0, has_audio: false };
        assert_eq!(fit(&info, (800, 800)), (800, 450));
        assert_eq!(fit(&info, (1000, 300)), (532, 300));
        assert_eq!(time_text(65.4), "1:05");
        assert_eq!(time_text(3723.0), "1:02:03");
    }

    /// Toca de verdade um vídeo gerado pelo ffmpeg (sem som), se o ffmpeg existir.
    #[test]
    fn toca_quadros_de_um_video() {
        let dir = std::env::temp_dir().join(format!("mide-video-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("t.mp4");
        let made = Command::new("ffmpeg")
            .args(["-loglevel", "error", "-f", "lavfi", "-i", "testsrc2=size=320x180:rate=20", "-t", "1", "-pix_fmt", "yuv420p", "-y"])
            .arg(&file)
            .status()
            .is_ok_and(|s| s.success());
        if !made {
            return;
        }
        let mut p = Player::open(&file, (160, 160), false).unwrap();
        assert_eq!(p.size(), (160, 90));
        p.play();
        let start = Instant::now();
        let mut frames = 0;
        while !p.ended() && start.elapsed().as_secs() < 5 {
            if p.tick() {
                frames += 1;
                let f = p.frame().unwrap();
                assert_eq!(f.pixels.len(), 160 * 90 * 4);
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(p.ended(), "não terminou");
        assert!(frames >= 15, "poucos quadros: {frames}");
        // Pular parado mostra o quadro de lá.
        p.seek(0.5);
        let start = Instant::now();
        while p.frame().is_none_or(|f| (f.time - 0.5).abs() > 0.01) && start.elapsed().as_secs() < 3 {
            p.tick();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!((p.frame().unwrap().time - 0.5).abs() < 0.01);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
