//! Especificações da máquina, para a janela "Sobre" e as Configurações.

use std::fs;

/// Rótulo da tela no idioma da sessão. Os textos dos catálogos vivem até o fim do
/// programa, então o rótulo continua `&'static str`.
fn label(text: &'static str) -> &'static str {
    match crate::i18n::tr(text) {
        std::borrow::Cow::Borrowed(t) => t,
        std::borrow::Cow::Owned(t) => t.leak(),
    }
}

/// Pares (rótulo, valor) com as especificações da máquina, com os rótulos no idioma
/// da sessão.
pub fn system_info() -> Vec<(&'static str, String)> {
    raw_info().into_iter().map(|(l, v)| (label(l), v)).collect()
}

/// Pares (rótulo em inglês, valor): os rótulos servem de chave em [`machine`].
fn raw_info() -> Vec<(&'static str, String)> {
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
        .unwrap_or_else(|| crate::tr!("unknown").into_owned());
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
        // A memória que o sistema enxerga é um pouco menor que a instalada (parte fica
        // com o vídeo e o firmware): arredonda para cima, como a etiqueta do pente.
        .map(|kb| format!("{} GB", (kb / 1024.0 / 1024.0).ceil() as u64))
        .unwrap_or_default();
    let gpu = gpu_driver().unwrap_or_else(|| crate::tr!("unknown").into_owned());
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
        ("System", os),
        ("Kernel", kernel),
        ("Processor", if cores > 0 { crate::i18n::fill(&crate::i18n::trn("{cpu} ({n} core)", "{cpu} ({n} cores)", cores as u64), &[("cpu", &cpu)]) } else { cpu }),
        ("Memory", memory),
        ("Graphics driver", gpu),
        ("Computer", host),
        ("Uptime", uptime),
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

/// Dados da janela "Sobre este computador" (no estilo do Mac): o modelo, o tipo e as
/// especificações principais.
#[derive(Debug, Clone, Default)]
pub struct Machine {
    /// Nome do modelo (ex.: "OptiPlex 3090"); o nome do computador se não houver.
    pub name: String,
    /// Fabricante, tipo e ano (ex.: "Dell · Computador de mesa, 2026").
    pub subtitle: String,
    /// Notebook (pelo tipo de gabinete do firmware).
    pub laptop: bool,
    /// (rótulo, valor) na ordem da janela.
    pub specs: Vec<(&'static str, String)>,
}

pub fn machine() -> Machine {
    let dmi = |f: &str| read_trim(&format!("/sys/class/dmi/id/{f}")).filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("to be filled by o.e.m."));
    let chassis = dmi("chassis_type").and_then(|c| c.parse::<u32>().ok()).unwrap_or(0);
    // Tipos de gabinete do SMBIOS: 8–10 e 14 são portáteis; 30–32, tablets.
    let laptop = matches!(chassis, 8..=10 | 14 | 30..=32) || std::path::Path::new("/sys/class/power_supply/BAT0").exists();
    let host = read_trim("/proc/sys/kernel/hostname").unwrap_or_default();
    let name = dmi("product_name").unwrap_or_else(|| host.clone());
    let vendor = dmi("sys_vendor").unwrap_or_default();
    let kind = if laptop { crate::tr!("Laptop") } else { crate::tr!("Desktop computer") };
    let year = dmi("bios_date").and_then(|d| d.rsplit('/').next().map(str::to_string)).filter(|y| y.len() == 4);
    let subtitle = match (vendor.is_empty(), year) {
        (false, Some(y)) => format!("{vendor} · {kind}, {y}"),
        (false, None) => format!("{vendor} · {kind}"),
        (true, Some(y)) => format!("{kind}, {y}"),
        (true, None) => kind.into_owned(),
    };
    let info = raw_info();
    let get = |k: &str| info.iter().find(|(l, _)| *l == k).map(|(_, v)| v.clone()).unwrap_or_default();
    let gpu = gpu_name().unwrap_or_else(|| get("Graphics driver"));
    let os = get("System");
    let mut specs = vec![
        (label("Processor"), get("Processor")),
        (label("Graphics"), gpu),
        (label("Memory"), get("Memory")),
    ];
    if let Some(disk) = startup_disk() {
        specs.push((label("Startup disk"), disk));
    }
    specs.push((label("Computer name"), host));
    specs.push((label("System"), crate::tr!("mide {version} on {os}", version = env!("CARGO_PKG_VERSION"), os = os)));
    Machine { name, subtitle, laptop, specs }
}

/// Nome da placa de vídeo pelo `lspci -mm` (ex.: "Intel UHD Graphics 630").
fn gpu_name() -> Option<String> {
    let out = std::process::Command::new("lspci").arg("-mm").output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().find(|l| l.contains("\"VGA compatible controller\"") || l.contains("\"3D controller\""))?;
    let fields: Vec<&str> = line.split('"').filter(|f| !f.trim().is_empty()).collect();
    // dispositivo, classe, fabricante, modelo...
    let vendor = fields.get(2)?.replace(" Corporation", "").replace(" Corp.", "");
    let model = fields.get(3)?;
    // "CometLake-S GT2 [UHD Graphics 630]": o nome de mercado entre colchetes.
    let model = model.split_once('[').and_then(|(_, r)| r.strip_suffix(']')).unwrap_or(model);
    Some(format!("{vendor} {model}"))
}

/// Modelo do disco onde está a raiz do sistema (ex.: "MCR-RPRS-480").
fn startup_disk() -> Option<String> {
    let mounts = fs::read_to_string("/proc/mounts").ok()?;
    let dev = mounts.lines().find_map(|l| {
        let mut parts = l.split_whitespace();
        let (dev, point) = (parts.next()?, parts.next()?);
        (point == "/").then(|| dev.to_string())
    })?;
    let name = dev.strip_prefix("/dev/")?;
    // Partição (sda2, nvme0n1p2) → disco (sda, nvme0n1).
    let disk = fs::read_dir("/sys/block").ok()?.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).find(|d| name.starts_with(d.as_str()) && !d.starts_with("loop"))?;
    let model = read_trim(&format!("/sys/block/{disk}/device/model")).filter(|m| !m.is_empty());
    let size = read_trim(&format!("/sys/block/{disk}/size")).and_then(|s| s.parse::<u64>().ok()).map(|sectors| sectors * 512);
    match (model, size) {
        (Some(m), Some(s)) => Some(format!("{m} ({})", crate::files::size_text(s))),
        (Some(m), None) => Some(m),
        (None, Some(s)) => Some(format!("{disk} ({})", crate::files::size_text(s))),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    /// Os rótulos passam por `label` (longe de um `tr!("…")` que o teste de
    /// `mide::i18n` enxergue): todos precisam de tradução.
    #[test]
    fn rotulos_tem_traducao() {
        crate::i18n::init();
        crate::i18n::set_language("pt_BR");
        for text in ["System", "Kernel", "Processor", "Memory", "Graphics driver", "Computer", "Uptime", "Graphics", "Startup disk", "Computer name"] {
            assert!(mide_ui::kit::i18n::is_translated(None, text), "sem tradução: {text}");
        }
        assert_eq!(super::label("Memory"), "Memória");
    }
}
