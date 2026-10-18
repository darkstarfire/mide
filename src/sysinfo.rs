//! Especificações da máquina, para a janela "Sobre" e as Configurações.

use std::fs;

/// Pares (rótulo, valor) com as especificações da máquina.
pub fn system_info() -> Vec<(&'static str, String)> {
    let os = fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|text| {
            text.lines()
                .find_map(|l| l.strip_prefix("PRETTY_NAME=").map(|v| v.trim_matches('"').to_string()))
        })
        .unwrap_or_else(|| "Linux".into());
    let kernel = read_trim("/proc/sys/kernel/osrelease").unwrap_or_default();
    let cpu = fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|text| {
            text.lines()
                .find_map(|l| l.strip_prefix("model name").map(|v| v.trim_start_matches([' ', '\t', ':']).to_string()))
        })
        .unwrap_or_else(|| "desconhecido".into());
    let cores = fs::read_to_string("/proc/cpuinfo")
        .map(|t| t.lines().filter(|l| l.starts_with("processor")).count())
        .unwrap_or(0);
    let memory = fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|text| {
            text.lines().find_map(|l| {
                l.strip_prefix("MemTotal:")
                    .and_then(|v| v.split_whitespace().next()?.parse::<f64>().ok())
            })
        })
        .map(|kb| format!("{:.1} GB", kb / 1024.0 / 1024.0))
        .unwrap_or_default();
    let gpu = gpu_driver().unwrap_or_else(|| "desconhecido".into());
    let host = read_trim("/proc/sys/kernel/hostname").unwrap_or_default();
    let uptime = read_trim("/proc/uptime")
        .and_then(|t| t.split_whitespace().next()?.parse::<f64>().ok())
        .map(|secs| {
            let mins = (secs / 60.0) as u64;
            if mins >= 60 {
                format!("{} h {} min", mins / 60, mins % 60)
            } else {
                format!("{mins} min")
            }
        })
        .unwrap_or_default();

    vec![
        ("Sistema", os),
        ("Kernel", kernel),
        ("Processador", if cores > 0 { format!("{cpu} ({cores} núcleos)") } else { cpu }),
        ("Memória", memory),
        ("Vídeo", gpu),
        ("Computador", host),
        ("Ligado há", uptime),
    ]
}

fn read_trim(path: &str) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

/// Driver da primeira placa de vídeo (ex.: i915, amdgpu, nouveau).
fn gpu_driver() -> Option<String> {
    let mut cards: Vec<_> = fs::read_dir("/sys/class/drm").ok()?.flatten().collect();
    cards.sort_by_key(|e| e.file_name());
    cards.into_iter().find_map(|entry| {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with("card") || name.contains('-') {
            return None;
        }
        let uevent = fs::read_to_string(entry.path().join("device/uevent")).ok()?;
        uevent.lines().find_map(|l| l.strip_prefix("DRIVER=").map(str::to_string))
    })
}
