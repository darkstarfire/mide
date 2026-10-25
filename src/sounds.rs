//! Sons do sistema (A22): notificação, captura de tela, Lixeira, alerta, volume,
//! bateria fraca, gravação salva e entrada na sessão.
//!
//! Os sons são do mide, sintetizados aqui (senos, ruído filtrado e envelopes), sem
//! arquivo de som de terceiros. Na primeira vez, viram um tema de sons do freedesktop
//! (`~/.local/share/sounds/mide`, WAV estéreo de 48 kHz), com os nomes da
//! especificação (`message-new-instant`, `screen-capture`, `trash-empty`, `bell`...):
//! assim, os apps GTK que tocam sons de evento (libcanberra) usam os do mide na
//! sessão do mide (o tema vai no banco do dconf da sessão, `appearance`).
//!
//! Tocar: `pw-play` (PipeWire), ou `paplay`. `[sounds]` no `config.toml`: `enabled`
//! (padrão ligado), `volume` (0 a 100, padrão 70) e `alert` (som de alerta: vidro,
//! gota, sino, pop). Nos testes (`MIDE_SYSTEM_DRY_RUN`), só registra no log.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

const RATE: u32 = 48_000;
/// Muda quando um som muda: o tema é refeito.
const VERSION: &str = "3";

/// Sons de alerta oferecidos: chave, nome na tela.
pub const ALERTS: &[(&str, &str)] = &[("vidro", "Vidro"), ("gota", "Gota"), ("sino", "Sino"), ("pop", "Pop")];

/// Os eventos do tema: nome do freedesktop e o desenho.
const EVENTS: &[&str] = &[
    "message-new-instant",
    "screen-capture",
    "trash-empty",
    "trash-move",
    "bell",
    "dialog-warning",
    "dialog-information",
    "audio-volume-change",
    "battery-low",
    "complete",
    "desktop-login",
    "device-added",
    "device-removed",
];

/// Toca um som do tema (pelo nome do freedesktop), se os efeitos estão ligados.
pub fn play(name: &str) {
    if cfg!(test) || !enabled() {
        return;
    }
    if std::env::var_os("MIDE_SYSTEM_DRY_RUN").is_some() {
        tracing::info!("som (teste): {name}");
        return;
    }
    // O mesmo som várias vezes seguidas (vários itens para a Lixeira de uma vez)
    // toca uma vez só.
    {
        use std::sync::Mutex;
        static LAST: Mutex<Option<(String, std::time::Instant)>> = Mutex::new(None);
        let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
        if last.as_ref().is_some_and(|(n, t)| n == name && t.elapsed().as_millis() < 300) {
            return;
        }
        *last = Some((name.to_string(), std::time::Instant::now()));
    }
    let name = name.to_string();
    std::thread::spawn(move || {
        let Some(path) = ensure_theme().map(|dir| dir.join("stereo").join(format!("{name}.wav"))) else {
            return;
        };
        play_file(&path, volume());
    });
}

/// O som de alerta escolhido (sino do terminal, avisos).
pub fn play_alert() {
    play("bell");
}

/// Toca um som de alerta pela chave (prévia nas Configurações), mesmo antes de gravar.
pub fn preview_alert(key: &str) {
    if std::env::var_os("MIDE_SYSTEM_DRY_RUN").is_some() {
        tracing::info!("som (teste): alerta {key}");
        return;
    }
    let key = key.to_string();
    std::thread::spawn(move || {
        let path = std::env::temp_dir().join(format!("mide-alerta-{key}-{}.wav", std::process::id()));
        if write_wav(&path, &alert(&key)).is_ok() {
            play_file(&path, volume());
            let _ = fs::remove_file(&path);
        }
    });
}

pub fn enabled() -> bool {
    crate::settings_file::flag("sounds", "enabled", true)
}

/// Volume dos efeitos, de 0 a 1.
pub fn volume() -> f32 {
    match crate::settings_file::cached("sounds", "volume") {
        Some(crate::settings_file::Value::Int(n)) => n as f32 / 100.0,
        Some(crate::settings_file::Value::Float(x)) => x as f32 / 100.0,
        _ => 0.7,
    }
    .clamp(0.0, 1.0)
}

/// O som de alerta escolhido.
pub fn alert_key() -> String {
    match crate::settings_file::cached("sounds", "alert") {
        Some(crate::settings_file::Value::Text(t)) if ALERTS.iter().any(|(k, _)| *k == t) => t,
        _ => "vidro".into(),
    }
}

fn play_file(path: &Path, volume: f32) {
    let ok = Command::new("pw-play").arg("--volume").arg(format!("{volume:.2}")).arg(path).status().is_ok_and(|s| s.success());
    if !ok {
        let _ = Command::new("paplay").arg(format!("--volume={}", (volume * 65536.0) as u32)).arg(path).status();
    }
}

/// Pasta do tema (`~/.local/share/sounds/mide`), criada (ou refeita, se os sons
/// mudaram ou o alerta escolhido mudou) na hora.
pub fn ensure_theme() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))?;
    let dir = base.join("sounds").join("mide");
    let stamp = format!("{VERSION} {}", alert_key());
    if fs::read_to_string(dir.join(".versao")).is_ok_and(|v| v == stamp) {
        return Some(dir);
    }
    let stereo = dir.join("stereo");
    fs::create_dir_all(&stereo).ok()?;
    fs::write(dir.join("index.theme"), "[Sound Theme]\nName=mide\nComment=Sons do mide (feitos para o mide)\nDirectories=stereo\n\n[stereo]\nOutputProfile=stereo\n").ok()?;
    for name in EVENTS {
        write_wav(&stereo.join(format!("{name}.wav")), &synth(name)).ok()?;
    }
    fs::write(dir.join(".versao"), stamp).ok()?;
    tracing::info!("sons: tema mide criado em {}", dir.display());
    Some(dir)
}

// ------------------------------------------------------------------ síntese

/// Um som: amostras de -1 a 1, mono.
type Wave = Vec<f32>;

fn silence(secs: f32) -> Wave {
    vec![0.0; (secs * RATE as f32) as usize]
}

/// Nota: seno com um harmônico leve, ataque curto e queda exponencial.
fn note(freq: f32, secs: f32, decay: f32, amp: f32) -> Wave {
    let n = (secs * RATE as f32) as usize;
    (0..n)
        .map(|i| {
            let t = i as f32 / RATE as f32;
            let attack = (t / 0.004).min(1.0);
            let env = attack * (-t * decay).exp();
            let s = (std::f32::consts::TAU * freq * t).sin() + 0.25 * (std::f32::consts::TAU * freq * 2.0 * t).sin() + 0.08 * (std::f32::consts::TAU * freq * 3.01 * t).sin();
            amp * env * s / 1.33
        })
        .collect()
}

/// Ruído filtrado (passa-baixa simples) com queda: estalos e papel.
fn noise(secs: f32, decay: f32, smooth: f32, amp: f32, seed: u32) -> Wave {
    let n = (secs * RATE as f32) as usize;
    let mut state = seed.max(1);
    let mut last = 0.0f32;
    (0..n)
        .map(|i| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            let white = (state as f32 / u32::MAX as f32) * 2.0 - 1.0;
            last += (white - last) * smooth;
            let t = i as f32 / RATE as f32;
            amp * last * (-t * decay).exp() * (t / 0.002).min(1.0)
        })
        .collect()
}

/// Papel amassado: chiado agudo (ruído sem os graves) que sobe e desce em rajadas
/// irregulares, cheio de estalinhos curtíssimos (1 a 3 ms) e claros, agrupados nas
/// rajadas, como o papel se fechando na mão. Sem notas nem graves: nada de "clique".
fn crumple(secs: f32, crackles: u32, amp: f32, seed: u32) -> Wave {
    let n = (secs * RATE as f32) as usize;
    let mut state = seed.wrapping_mul(2_654_435_761).max(1);
    let mut rand = move || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        state as f32 / u32::MAX as f32
    };
    // Rajadas: centros e larguras aleatórios, mais fortes no começo.
    let bursts: Vec<(f32, f32, f32)> = (0..7)
        .map(|i| {
            let at = secs * (0.05 + 0.85 * (i as f32 + rand()) / 7.0);
            let width = 0.025 + 0.05 * rand();
            let gain = (0.45 + 0.55 * rand()) * (1.0 - 0.5 * at / secs);
            (at, width, gain)
        })
        .collect();
    let swell = |t: f32| -> f32 { bursts.iter().map(|(c, w, g)| g * (-((t - c) / w).powi(2)).exp()).sum::<f32>().min(1.0) };
    // Chiado de fundo: ruído branco menos a parte grave (passa-alta simples).
    let mut out = vec![0.0f32; n];
    let mut low = 0.0f32;
    for (i, s) in out.iter_mut().enumerate() {
        let t = i as f32 / RATE as f32;
        let white = rand() * 2.0 - 1.0;
        low += (white - low) * 0.08;
        let fade = (t / 0.01).min(1.0) * ((secs - t) / 0.06).clamp(0.0, 1.0);
        *s = (white - low) * amp * 0.22 * (0.15 + swell(t)) * fade;
    }
    // Estalinhos dentro das rajadas.
    for _ in 0..crackles {
        let (c, w, g) = bursts[(rand() * bursts.len() as f32) as usize % bursts.len()];
        let at = (c + (rand() - 0.5) * 2.5 * w).clamp(0.0, secs - 0.004);
        let len = ((0.001 + 0.002 * rand()) * RATE as f32) as usize;
        let a = amp * g * (0.3 + 0.7 * rand());
        let start = (at * RATE as f32) as usize;
        let mut prev = 0.0f32;
        for k in 0..len {
            let Some(s) = out.get_mut(start + k) else { break };
            let white = rand() * 2.0 - 1.0;
            // Só os agudos do estalo (diferença entre amostras vizinhas).
            let bright = white - prev;
            prev = white;
            let env = (-(k as f32) / (len as f32 * 0.35)).exp();
            *s += 0.5 * a * bright * env;
        }
    }
    for s in &mut out {
        *s = s.clamp(-0.95, 0.95);
    }
    out
}

/// Soma `b` em `a` a partir de `at` segundos.
fn mix(mut a: Wave, b: &Wave, at: f32) -> Wave {
    let start = (at * RATE as f32) as usize;
    if a.len() < start + b.len() {
        a.resize(start + b.len(), 0.0);
    }
    for (i, s) in b.iter().enumerate() {
        a[start + i] += s;
    }
    a
}

/// Um som de alerta pela chave.
fn alert(key: &str) -> Wave {
    match key {
        // Gota: uma nota que escorrega para cima.
        "gota" => {
            let n = (0.22 * RATE as f32) as usize;
            let mut phase = 0.0f32;
            (0..n)
                .map(|i| {
                    let t = i as f32 / RATE as f32;
                    let f = 700.0 + 900.0 * (t / 0.22);
                    phase += std::f32::consts::TAU * f / RATE as f32;
                    0.55 * phase.sin() * (-t * 18.0).exp() * (t / 0.003).min(1.0)
                })
                .collect()
        }
        // Sino: duas notas graves com cauda longa.
        "sino" => mix(note(659.0, 1.0, 4.0, 0.45), &note(988.0, 0.9, 5.0, 0.25), 0.0),
        // Pop: estalo curto e redondo.
        "pop" => mix(note(420.0, 0.12, 40.0, 0.6), &noise(0.03, 120.0, 0.3, 0.25, 7), 0.0),
        // Vidro (padrão): toque agudo e limpo.
        _ => mix(note(1568.0, 0.5, 9.0, 0.4), &note(2349.0, 0.4, 12.0, 0.18), 0.0),
    }
}

/// O desenho de cada evento.
pub fn synth(name: &str) -> Wave {
    match name {
        // Notificação: duas notas suaves subindo.
        "message-new-instant" | "dialog-information" => mix(note(988.0, 0.35, 10.0, 0.35), &note(1319.0, 0.45, 8.0, 0.35), 0.09),
        // Obturador: dois estalos de ruído.
        "screen-capture" => mix(noise(0.05, 70.0, 0.55, 0.7, 11), &noise(0.07, 55.0, 0.4, 0.6, 23), 0.085),
        // Papel amassado caindo na Lixeira.
        // Esvaziar: papel amassado mais longo e a queda no fundo da lixeira.
        "trash-empty" => mix(crumple(0.6, 160, 0.7, 5), &noise(0.18, 22.0, 0.04, 0.35, 41), 0.55),
        // Mover para a Lixeira: papel amassado curto.
        "trash-move" => crumple(0.38, 90, 0.7, 17),
        "bell" | "dialog-warning" => alert(&alert_key()),
        // Volume: um toque curto.
        "audio-volume-change" => note(880.0, 0.08, 45.0, 0.45),
        // Bateria fraca: duas notas descendo, repetidas.
        "battery-low" => mix(mix(note(784.0, 0.18, 14.0, 0.4), &note(587.0, 0.25, 12.0, 0.4), 0.16), &note(587.0, 0.25, 12.0, 0.3), 0.42),
        // Pronto (gravação, cópia): três notas subindo.
        "complete" => mix(mix(note(784.0, 0.3, 12.0, 0.3), &note(988.0, 0.3, 12.0, 0.3), 0.08), &note(1175.0, 0.5, 8.0, 0.3), 0.16),
        // Entrada na sessão: acorde aberto, um pouco mais longo.
        "desktop-login" => mix(mix(mix(note(523.0, 1.2, 3.0, 0.22), &note(659.0, 1.1, 3.0, 0.2), 0.12), &note(784.0, 1.0, 3.2, 0.2), 0.24), &note(1047.0, 0.9, 3.5, 0.16), 0.36),
        "device-added" => mix(note(660.0, 0.15, 20.0, 0.35), &note(990.0, 0.2, 16.0, 0.35), 0.08),
        "device-removed" => mix(note(990.0, 0.15, 20.0, 0.35), &note(660.0, 0.2, 16.0, 0.35), 0.08),
        _ => silence(0.05),
    }
}

/// Grava como WAV estéreo de 16 bits.
fn write_wav(path: &Path, wave: &Wave) -> std::io::Result<()> {
    let samples: Vec<i16> = wave.iter().map(|s| (s.clamp(-1.0, 1.0) * 30000.0) as i16).collect();
    let data_len = (samples.len() * 4) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&2u16.to_le_bytes()); // estéreo
    out.extend_from_slice(&RATE.to_le_bytes());
    out.extend_from_slice(&(RATE * 4).to_le_bytes());
    out.extend_from_slice(&4u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
        out.extend_from_slice(&s.to_le_bytes());
    }
    let tmp = path.with_extension("wav.mide");
    fs::write(&tmp, out)?;
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sons_validos() {
        for name in EVENTS {
            let wave = synth(name);
            assert!(wave.len() > RATE as usize / 50, "{name} curto demais");
            assert!(wave.len() < RATE as usize * 2, "{name} longo demais");
            let peak = wave.iter().fold(0.0f32, |m, s| m.max(s.abs()));
            assert!(peak > 0.05 && peak <= 1.0, "{name}: pico {peak}");
        }
        for (key, _) in ALERTS {
            assert!(!alert(key).is_empty());
        }
        let dir = std::env::temp_dir().join(format!("mide-sons-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.wav");
        write_wav(&path, &synth("bell")).unwrap();
        let bytes = fs::read(&path).unwrap();
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
        assert_eq!(bytes.len() as u32 - 8, u32::from_le_bytes(bytes[4..8].try_into().unwrap()));
        let _ = fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod crumple_tests {
    use super::*;

    /// Papel amassado tem a energia nos agudos (centro do espectro acima de 3 kHz),
    /// não num estalo grave. Com `MIDE_SOUND_DUMP=pasta`, grava os WAV para ouvir.
    #[test]
    fn papel_amassado_e_agudo() {
        for name in ["trash-move", "trash-empty"] {
            let w = synth(name);
            if let Some(dir) = std::env::var_os("MIDE_SOUND_DUMP") {
                write_wav(&Path::new(&dir).join(format!("{name}.wav")), &w).unwrap();
            }
            // Frequência média pelos cruzamentos de zero: aproxima o centro do espectro.
            let crossings = w.windows(2).filter(|p| (p[0] >= 0.0) != (p[1] >= 0.0)).count();
            let freq = crossings as f32 / 2.0 / (w.len() as f32 / RATE as f32);
            assert!(freq > 3000.0, "{name}: {freq} Hz");
            assert!(w.iter().all(|s| s.abs() <= 0.95));
        }
    }
}
