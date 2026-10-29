//! Carga da bateria, pelo caminho que funcionar:
//!
//! 1. o kernel (`/sys/class/power_supply/BAT*`), o caminho de sempre;
//! 2. sem bateria presente no kernel, o arquivo `/run/cros-ec-battery.json` do serviço
//!    `cros-ec-battery` (github.com/fabioaacarneiro/chromebook-linux-fixes), que lê a
//!    bateria direto do EC como root;
//! 3. o próprio EC do Chromebook (`/dev/cros_ec`), quando o usuário pode lê-lo.
//!
//! Por que existe: em Chromebooks com firmware alternativo (MrChromebox), o firmware
//! diz ao Linux "battery absent" quando o EC não reconhece a bateria (comum com
//! baterias paralelas), embora o EC converse com ela normalmente. O kernel então não
//! mostra bateria nenhuma, mas a memória compartilhada do EC tem a carga, a capacidade
//! e se está carregando.

use std::path::{Path, PathBuf};

/// De onde veio a leitura.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Kernel,
    /// `/run/cros-ec-battery.json` do serviço do Chromebook.
    EcService,
    /// `/dev/cros_ec` lido pelo próprio mide.
    Ec,
}

/// Carga (0 a 100) e se está carregando.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Battery {
    pub percent: u8,
    pub charging: bool,
    pub source: Source,
}

/// O arquivo do serviço vale se foi gravado há pouco (ele grava a cada 5 s).
const SERVICE_MAX_AGE_S: f64 = 60.0;

/// Bateria do computador, se houver uma (por qualquer caminho).
/// `MIDE_POWER_SUPPLY_DIR` troca a pasta do kernel e desliga os outros caminhos (os
/// testes usam uma pasta vazia: a barra fica igual com ou sem bateria na máquina).
pub fn read() -> Option<Battery> {
    if let Some(dir) = std::env::var_os("MIDE_POWER_SUPPLY_DIR") {
        return from_kernel(Path::new(&dir));
    }
    from_kernel(Path::new("/sys/class/power_supply"))
        .or_else(|| from_service(Path::new("/run/cros-ec-battery.json"), now()))
        .or_else(|| from_ec(Path::new("/dev/cros_ec")))
}

fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or_default()
}

/// Primeira bateria presente do kernel.
fn from_kernel(root: &Path) -> Option<Battery> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(root).ok()?.flatten().map(|e| e.path()).collect();
    entries.sort();
    entries.into_iter().find_map(|path| {
        let read = |name: &str| std::fs::read_to_string(path.join(name)).map(|s| s.trim().to_string()).ok();
        let is_battery = read("type").map_or_else(
            || path.file_name().is_some_and(|n| n.to_string_lossy().starts_with("BAT")),
            |t| t == "Battery",
        );
        // "present" 0: o firmware diz que não há bateria (o caso do Chromebook).
        if !is_battery || read("present").is_some_and(|p| p == "0") || read("scope").is_some_and(|s| s == "Device") {
            return None;
        }
        let percent: u8 = read("capacity")?.parse().ok()?;
        Some(Battery { percent: percent.min(100), charging: read("status").is_some_and(|s| s == "Charging"), source: Source::Kernel })
    })
}

/// O arquivo gravado pelo serviço `cros-ec-battery`, se for recente.
fn from_service(path: &Path, now: f64) -> Option<Battery> {
    let text = std::fs::read_to_string(path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    if json.get("ok").and_then(|v| v.as_bool()) != Some(true) {
        return None;
    }
    let ts = json.get("ts").and_then(|v| v.as_f64())?;
    if (now - ts).abs() > SERVICE_MAX_AGE_S {
        return None;
    }
    let percent = json.get("percent").and_then(|v| v.as_f64())?;
    let charging = json.get("state").and_then(|v| v.as_str()) == Some("charging");
    Some(Battery { percent: percent.round().clamp(0.0, 100.0) as u8, charging, source: Source::EcService })
}

// Memória compartilhada do EC (ec_commands.h): a bateria fica em 0x40..0x80.
const MEMMAP_BATT_BASE: u32 = 0x40;
const MEMMAP_BATT_LEN: u32 = 0x40;
const FLAG_CHARGING: u8 = 0x08;
/// `struct cros_ec_readmem { u32 offset; u32 bytes; u8 buffer[255]; }`, 264 bytes.
const READMEM_SIZE: usize = 264;
/// `CROS_EC_DEV_IOCRDMEM = _IOWR(0xEC, 1, struct cros_ec_readmem)`.
const IOC_RDMEM: u64 = (3 << 30) | ((READMEM_SIZE as u64) << 16) | (0xEC << 8) | 1;

/// O EC do Chromebook direto (precisa de permissão de leitura no `/dev/cros_ec`).
fn from_ec(dev: &Path) -> Option<Battery> {
    use std::os::fd::AsRawFd;
    let file = std::fs::File::open(dev).ok()?;
    let mut buf = [0u8; READMEM_SIZE];
    buf[0..4].copy_from_slice(&MEMMAP_BATT_BASE.to_le_bytes());
    buf[4..8].copy_from_slice(&MEMMAP_BATT_LEN.to_le_bytes());
    // SAFETY: `buf` tem o tamanho da estrutura que o ioctl lê e escreve, e vive até o
    // fim da chamada; o descritor é do arquivo aberto acima.
    let ret = unsafe { libc::ioctl(file.as_raw_fd(), IOC_RDMEM as _, buf.as_mut_ptr()) };
    if ret < 0 {
        return None;
    }
    parse_ec(&buf[8..8 + MEMMAP_BATT_LEN as usize])
}

/// Lê a bateria da memória do EC: carga restante sobre a capacidade cheia (ou a de
/// projeto) e a marca "carregando".
fn parse_ec(d: &[u8]) -> Option<Battery> {
    let u32_at = |at: usize| -> Option<u32> {
        let v = u32::from_le_bytes(d.get(at..at + 4)?.try_into().ok()?);
        // O EC usa 0xFFFFFFFF (e afins) para "desconhecido".
        (v < 0x7FFF_FFFF).then_some(v)
    };
    let remaining = u32_at(0x08)?;
    let full = u32_at(0x18).filter(|v| *v > 0).or_else(|| u32_at(0x10).filter(|v| *v > 0))?;
    let flags = *d.get(0x0C)?;
    let percent = (remaining as f64 * 100.0 / full as f64).round().clamp(0.0, 100.0) as u8;
    Some(Battery { percent, charging: flags & FLAG_CHARGING != 0, source: Source::Ec })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mide-bateria-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn kernel_ignora_bateria_ausente_e_de_aparelhos() {
        let root = tmp("kernel");
        let bat = root.join("BAT0");
        std::fs::create_dir_all(&bat).unwrap();
        std::fs::write(bat.join("type"), "Battery\n").unwrap();
        std::fs::write(bat.join("present"), "0\n").unwrap();
        std::fs::write(bat.join("capacity"), "0\n").unwrap();
        // O firmware diz "ausente": o kernel não serve, vale o próximo caminho.
        assert_eq!(from_kernel(&root), None);
        std::fs::write(bat.join("present"), "1\n").unwrap();
        std::fs::write(bat.join("capacity"), "73\n").unwrap();
        std::fs::write(bat.join("status"), "Charging\n").unwrap();
        assert_eq!(from_kernel(&root), Some(Battery { percent: 73, charging: true, source: Source::Kernel }));
        // Bateria de mouse sem fio (scope Device) não é a do computador.
        let mouse = root.join("hidpp_battery_0");
        std::fs::create_dir_all(&mouse).unwrap();
        std::fs::write(mouse.join("type"), "Battery\n").unwrap();
        std::fs::write(mouse.join("scope"), "Device\n").unwrap();
        std::fs::write(mouse.join("capacity"), "10\n").unwrap();
        std::fs::remove_dir_all(&bat).unwrap();
        assert_eq!(from_kernel(&root), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn arquivo_do_servico_do_chromebook() {
        let dir = tmp("servico");
        let file = dir.join("cros-ec-battery.json");
        std::fs::write(&file, r#"{"ok": true, "ts": 1000.0, "percent": 56.6, "state": "charging", "ac": true}"#).unwrap();
        assert_eq!(from_service(&file, 1005.0), Some(Battery { percent: 57, charging: true, source: Source::EcService }));
        // Velho (o serviço parou): não vale.
        assert_eq!(from_service(&file, 2000.0), None);
        std::fs::write(&file, r#"{"ok": true, "ts": 1000.0, "percent": 20.0, "state": "discharging"}"#).unwrap();
        assert_eq!(from_service(&file, 1001.0).map(|b| (b.percent, b.charging)), Some((20, false)));
        std::fs::write(&file, r#"{"ok": false, "error": "sem EC"}"#).unwrap();
        assert_eq!(from_service(&file, 1001.0), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn memoria_do_ec() {
        let mut d = [0u8; 0x40];
        d[0x00..0x04].copy_from_slice(&11_400u32.to_le_bytes()); // tensão
        d[0x08..0x0C].copy_from_slice(&2_400u32.to_le_bytes()); // restante (mAh)
        d[0x0C] = 0x01 | 0x02 | FLAG_CHARGING;
        d[0x10..0x14].copy_from_slice(&4_000u32.to_le_bytes()); // projeto
        d[0x18..0x1C].copy_from_slice(&3_200u32.to_le_bytes()); // cheia hoje
        assert_eq!(parse_ec(&d), Some(Battery { percent: 75, charging: true, source: Source::Ec }));
        // Capacidade cheia desconhecida: usa a de projeto.
        d[0x18..0x1C].copy_from_slice(&u32::MAX.to_le_bytes());
        d[0x0C] = 0x04;
        assert_eq!(parse_ec(&d).map(|b| (b.percent, b.charging)), Some((60, false)));
        // Carga desconhecida: nada.
        d[0x08..0x0C].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(parse_ec(&d), None);
    }
}
