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
//!
//! Além da carga, cada caminho dá o que souber: estado, energia, capacidade cheia e de
//! projeto (saúde), ciclos e potência. O tempo até acabar ou até carregar sai da
//! potência suavizada (`Estimate`), para não pular a cada leitura.

use std::path::{Path, PathBuf};
use std::time::Instant;

/// De onde veio a leitura.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Kernel,
    /// `/run/cros-ec-battery.json` do serviço do Chromebook.
    EcService,
    /// `/dev/cros_ec` lido pelo próprio mide.
    Ec,
}

/// O que a bateria está fazendo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Charging,
    Discharging,
    /// Carregada, na tomada.
    Full,
    /// Na tomada sem carregar (limite de carga, carregador fraco).
    NotCharging,
    Unknown,
}

/// Leitura da bateria: carga (0 a 100) e o que mais o caminho souber.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Battery {
    pub percent: u8,
    pub charging: bool,
    pub source: Source,
    pub state: State,
    /// Energia agora, cheia hoje e de projeto (Wh).
    pub energy_wh: Option<f64>,
    pub full_wh: Option<f64>,
    pub design_wh: Option<f64>,
    /// Potência entrando ou saindo (W, sempre positiva).
    pub power_w: Option<f64>,
    /// Saúde: capacidade cheia de hoje sobre a de projeto (%).
    pub health_percent: Option<f64>,
    pub cycles: Option<u32>,
    /// Tempos já calculados pela fonte (o serviço do Chromebook, ou o kernel), em
    /// segundos. Sem eles, `Estimate` calcula pela potência.
    pub time_to_empty_s: Option<u64>,
    pub time_to_full_s: Option<u64>,
}

impl Battery {
    fn new(percent: u8, state: State, source: Source) -> Self {
        Self {
            percent: percent.min(100),
            charging: state == State::Charging,
            source,
            state,
            energy_wh: None,
            full_wh: None,
            design_wh: None,
            power_w: None,
            health_percent: None,
            cycles: None,
            time_to_empty_s: None,
            time_to_full_s: None,
        }
    }

    /// Saúde pela capacidade cheia sobre a de projeto, quando a fonte não a deu.
    fn with_health(mut self) -> Self {
        if self.health_percent.is_none()
            && let (Some(full), Some(design)) = (self.full_wh, self.design_wh)
            && design > 0.0
            && full > 0.0
        {
            self.health_percent = Some((full * 100.0 / design).min(100.0));
        }
        self
    }
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
        // Os valores do kernel vêm em micro (µWh, µAh, µW, µA, µV).
        let micro = |name: &str| read(name).and_then(|v| v.parse::<f64>().ok()).filter(|v| *v >= 0.0).map(|v| v / 1e6);
        let is_battery = read("type").map_or_else(
            || path.file_name().is_some_and(|n| n.to_string_lossy().starts_with("BAT")),
            |t| t == "Battery",
        );
        // "present" 0: o firmware diz que não há bateria (o caso do Chromebook).
        if !is_battery || read("present").is_some_and(|p| p == "0") || read("scope").is_some_and(|s| s == "Device") {
            return None;
        }
        let percent: u8 = read("capacity")?.parse().ok()?;
        let state = match read("status").as_deref() {
            Some("Charging") => State::Charging,
            Some("Discharging") => State::Discharging,
            Some("Full") => State::Full,
            Some("Not charging") => State::NotCharging,
            _ => State::Unknown,
        };
        let mut b = Battery::new(percent, state, Source::Kernel);
        if micro("energy_now").is_some() {
            // Em energia (µWh) e potência (µW).
            b.energy_wh = micro("energy_now");
            b.full_wh = micro("energy_full");
            b.design_wh = micro("energy_full_design");
            b.power_w = micro("power_now");
        } else {
            // Em carga (µAh) e corrente (µA): vezes a tensão viram energia e potência.
            let volts = micro("voltage_now").filter(|v| *v > 0.0).or_else(|| micro("voltage_min_design"));
            let design_volts = micro("voltage_min_design").or(volts);
            let wh = |charge: Option<f64>, v: Option<f64>| charge.zip(v).map(|(c, v)| c * v);
            b.energy_wh = wh(micro("charge_now"), volts);
            b.full_wh = wh(micro("charge_full"), volts);
            b.design_wh = wh(micro("charge_full_design"), design_volts);
            b.power_w = micro("current_now").zip(volts).map(|(a, v)| a * v);
        }
        b.cycles = read("cycle_count").and_then(|c| c.parse().ok()).filter(|c| *c > 0);
        b.time_to_empty_s = read("time_to_empty_now").and_then(|t| t.parse().ok()).filter(|t| *t > 0);
        b.time_to_full_s = read("time_to_full_now").and_then(|t| t.parse().ok()).filter(|t| *t > 0);
        Some(b.with_health())
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
    let f = |key: &str| json.get(key).and_then(|v| v.as_f64());
    let state = match json.get("state").and_then(|v| v.as_str()) {
        Some("charging") => State::Charging,
        Some("discharging") => State::Discharging,
        Some("full") => State::Full,
        Some("not-charging") => State::NotCharging,
        _ => State::Unknown,
    };
    let mut b = Battery::new(percent.round().clamp(0.0, 100.0) as u8, state, Source::EcService);
    b.power_w = f("power_w").map(f64::abs);
    b.health_percent = f("health_percent").map(|h| h.min(100.0));
    b.cycles = f("cycles").map(|c| c as u32).filter(|c| *c > 0);
    b.time_to_empty_s = f("time_to_empty_s").map(|t| t as u64).filter(|t| *t > 0);
    b.time_to_full_s = f("time_to_full_s").map(|t| t as u64).filter(|t| *t > 0);
    // Energia em Wh pela carga (mAh) e a tensão de projeto (mV), quando vierem.
    let volts = f("design_mv").or(f("voltage_mv")).map(|mv| mv / 1000.0);
    let wh = |key: &str| f(key).zip(volts).map(|(mah, v)| mah / 1000.0 * v);
    b.energy_wh = wh("remaining_mah");
    b.full_wh = wh("full_mah");
    b.design_wh = wh("design_mah");
    Some(b.with_health())
}

// Memória compartilhada do EC (ec_commands.h): a bateria fica em 0x40..0x80.
const MEMMAP_BATT_BASE: u32 = 0x40;
const MEMMAP_BATT_LEN: u32 = 0x40;
const FLAG_AC: u8 = 0x01;
const FLAG_DISCHARGING: u8 = 0x04;
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
/// projeto), estado pelas marcas, e energia, potência e ciclos.
fn parse_ec(d: &[u8]) -> Option<Battery> {
    let u32_at = |at: usize| -> Option<u32> {
        let v = u32::from_le_bytes(d.get(at..at + 4)?.try_into().ok()?);
        // O EC usa 0xFFFFFFFF (e afins) para "desconhecido".
        (v < 0x7FFF_FFFF).then_some(v)
    };
    let remaining = u32_at(0x08)?;
    let design = u32_at(0x10).filter(|v| *v > 0);
    let last_full = u32_at(0x18).filter(|v| *v > 0);
    let full = last_full.or(design)?;
    let flags = *d.get(0x0C)?;
    let percent = (remaining as f64 * 100.0 / full as f64).round().clamp(0.0, 100.0) as u8;
    let state = if flags & FLAG_CHARGING != 0 {
        State::Charging
    } else if flags & FLAG_DISCHARGING != 0 {
        State::Discharging
    } else if flags & FLAG_AC != 0 {
        if percent >= 95 { State::Full } else { State::NotCharging }
    } else {
        State::Unknown
    };
    let mut b = Battery::new(percent, state, Source::Ec);
    // Carga em mAh e tensão em mV: Wh = mAh × V / 1000.
    let volts = u32_at(0x00).filter(|v| *v > 0).map(|mv| mv as f64 / 1000.0);
    let design_volts = u32_at(0x14).filter(|v| *v > 0).map(|mv| mv as f64 / 1000.0).or(volts);
    let wh = |mah: Option<u32>, v: Option<f64>| mah.zip(v).map(|(c, v)| c as f64 / 1000.0 * v);
    b.energy_wh = wh(Some(remaining), design_volts);
    b.full_wh = wh(last_full, design_volts);
    b.design_wh = wh(design, design_volts);
    b.power_w = u32_at(0x04).zip(volts).map(|(ma, v)| ma as f64 / 1000.0 * v);
    b.cycles = u32_at(0x1C).filter(|c| *c > 0);
    Some(b.with_health())
}

/// Abaixo disto (W), a potência não dá um tempo confiável (bateria parada, leitura
/// ruidosa): sem estimativa.
const MIN_POWER_W: f64 = 0.5;
/// Constante de tempo da média da potência: a estimativa segue o uso em ~1 min, sem
/// pular a cada leitura.
const SMOOTHING_S: f64 = 60.0;

/// Tempo até acabar e até carregar, pela potência suavizada (média móvel exponencial
/// no tempo, recomeçada quando o estado muda).
#[derive(Debug, Default)]
pub struct Estimate {
    power: Option<f64>,
    state: Option<State>,
    at: Option<Instant>,
}

impl Estimate {
    /// Junta a leitura e devolve (até acabar, até carregar), em segundos. Os tempos dados
    /// pela própria fonte valem como estão (o serviço do Chromebook já suaviza).
    pub fn update(&mut self, b: &Battery, now: Instant) -> (Option<u64>, Option<u64>) {
        if self.state != Some(b.state) {
            self.power = None;
        }
        self.state = Some(b.state);
        if let Some(p) = b.power_w.filter(|p| *p > 0.0) {
            let dt = self.at.map_or(0.0, |at| now.saturating_duration_since(at).as_secs_f64());
            self.power = Some(match self.power {
                Some(old) => {
                    let k = 1.0 - (-dt / SMOOTHING_S).exp();
                    old + (p - old) * k
                }
                None => p,
            });
        }
        self.at = Some(now);
        let power = self.power.filter(|p| *p >= MIN_POWER_W);
        let empty = match b.state {
            State::Discharging => b.time_to_empty_s.or_else(|| Some((b.energy_wh? / power? * 3600.0) as u64)),
            _ => None,
        };
        let full = match b.state {
            State::Charging => b
                .time_to_full_s
                .or_else(|| Some(((b.full_wh? - b.energy_wh?).max(0.0) / power? * 3600.0) as u64)),
            _ => None,
        };
        // Mais de 2 dias: leitura sem sentido (potência quase zero).
        let sane = |t: Option<u64>| t.filter(|t| *t > 0 && *t < 48 * 3600);
        (sane(empty), sane(full))
    }
}

/// Duração para mostrar: "2 h 15 min", "45 min", "3 h".
pub fn format_duration(secs: u64) -> String {
    let minutes = (secs + 30) / 60;
    let (h, m) = (minutes / 60, minutes % 60);
    match (h, m) {
        (0, m) => format!("{} min", m.max(1)),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m} min"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mide-bateria-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(dir: &Path, files: &[(&str, &str)]) {
        std::fs::create_dir_all(dir).unwrap();
        for (name, value) in files {
            std::fs::write(dir.join(name), format!("{value}\n")).unwrap();
        }
    }

    fn close(a: Option<f64>, b: f64) -> bool {
        a.is_some_and(|a| (a - b).abs() < 0.01)
    }

    #[test]
    fn kernel_ignora_bateria_ausente_e_de_aparelhos() {
        let root = tmp("kernel");
        let bat = root.join("BAT0");
        write(&bat, &[("type", "Battery"), ("present", "0"), ("capacity", "0")]);
        // O firmware diz "ausente": o kernel não serve, vale o próximo caminho.
        assert_eq!(from_kernel(&root), None);
        write(&bat, &[("present", "1"), ("capacity", "73"), ("status", "Charging")]);
        let b = from_kernel(&root).unwrap();
        assert_eq!((b.percent, b.charging, b.state, b.source), (73, true, State::Charging, Source::Kernel));
        // Bateria de mouse sem fio (scope Device) não é a do computador.
        write(&root.join("hidpp_battery_0"), &[("type", "Battery"), ("scope", "Device"), ("capacity", "10")]);
        std::fs::remove_dir_all(&bat).unwrap();
        assert_eq!(from_kernel(&root), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn kernel_em_energia_e_em_carga() {
        let root = tmp("energia");
        // Em energia: 30 Wh de 40 (projeto 50), gastando 10 W, 120 ciclos.
        write(&root.join("BAT0"), &[
            ("type", "Battery"), ("capacity", "75"), ("status", "Discharging"),
            ("energy_now", "30000000"), ("energy_full", "40000000"), ("energy_full_design", "50000000"),
            ("power_now", "10000000"), ("cycle_count", "120"),
        ]);
        let b = from_kernel(&root).unwrap();
        assert_eq!(b.state, State::Discharging);
        assert!(close(b.energy_wh, 30.0) && close(b.full_wh, 40.0) && close(b.power_w, 10.0));
        assert!(close(b.health_percent, 80.0));
        assert_eq!(b.cycles, Some(120));
        // Em carga: 2 Ah de 4 (projeto 5) a 10 V, 0,5 A carregando.
        let _ = std::fs::remove_dir_all(&root);
        write(&root.join("BAT1"), &[
            ("type", "Battery"), ("capacity", "50"), ("status", "Charging"),
            ("charge_now", "2000000"), ("charge_full", "4000000"), ("charge_full_design", "5000000"),
            ("current_now", "500000"), ("voltage_now", "10000000"), ("voltage_min_design", "10000000"),
            ("cycle_count", "0"),
        ]);
        let b = from_kernel(&root).unwrap();
        assert!(close(b.energy_wh, 20.0) && close(b.full_wh, 40.0) && close(b.design_wh, 50.0) && close(b.power_w, 5.0));
        assert!(close(b.health_percent, 80.0));
        // Ciclos 0: o kernel não sabe.
        assert_eq!(b.cycles, None);
        // Carregada na tomada e na tomada sem carregar.
        write(&root.join("BAT1"), &[("status", "Full")]);
        assert_eq!(from_kernel(&root).unwrap().state, State::Full);
        write(&root.join("BAT1"), &[("status", "Not charging")]);
        assert_eq!(from_kernel(&root).unwrap().state, State::NotCharging);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn arquivo_do_servico_do_chromebook() {
        let dir = tmp("servico");
        let file = dir.join("cros-ec-battery.json");
        std::fs::write(&file, r#"{"ok": true, "ts": 1000.0, "percent": 56.6, "state": "charging", "ac": true,
            "time_to_full_s": 4500, "time_to_empty_s": null, "health_percent": 91.5, "power_w": 18.2, "cycles": 210,
            "remaining_mah": 2000, "full_mah": 3600, "design_mah": 4000, "design_mv": 11400}"#).unwrap();
        let b = from_service(&file, 1005.0).unwrap();
        assert_eq!((b.percent, b.charging, b.state, b.source), (57, true, State::Charging, Source::EcService));
        assert_eq!((b.time_to_full_s, b.time_to_empty_s, b.cycles), (Some(4500), None, Some(210)));
        assert!(close(b.health_percent, 91.5) && close(b.power_w, 18.2) && close(b.energy_wh, 22.8));
        // Velho (o serviço parou): não vale.
        assert_eq!(from_service(&file, 2000.0), None);
        std::fs::write(&file, r#"{"ok": true, "ts": 1000.0, "percent": 20.0, "state": "discharging"}"#).unwrap();
        let b = from_service(&file, 1001.0).unwrap();
        assert_eq!((b.percent, b.charging, b.state, b.health_percent), (20, false, State::Discharging, None));
        std::fs::write(&file, r#"{"ok": true, "ts": 1000.0, "percent": 99.0, "state": "full"}"#).unwrap();
        assert_eq!(from_service(&file, 1001.0).unwrap().state, State::Full);
        std::fs::write(&file, r#"{"ok": false, "error": "sem EC"}"#).unwrap();
        assert_eq!(from_service(&file, 1001.0), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn memoria_do_ec() {
        let mut d = [0u8; 0x40];
        d[0x00..0x04].copy_from_slice(&12_000u32.to_le_bytes()); // tensão (mV)
        d[0x04..0x08].copy_from_slice(&1_000u32.to_le_bytes()); // corrente (mA)
        d[0x08..0x0C].copy_from_slice(&2_400u32.to_le_bytes()); // restante (mAh)
        d[0x0C] = FLAG_AC | 0x02 | FLAG_CHARGING;
        d[0x10..0x14].copy_from_slice(&4_000u32.to_le_bytes()); // projeto (mAh)
        d[0x14..0x18].copy_from_slice(&10_000u32.to_le_bytes()); // tensão de projeto
        d[0x18..0x1C].copy_from_slice(&3_200u32.to_le_bytes()); // cheia hoje (mAh)
        d[0x1C..0x20].copy_from_slice(&150u32.to_le_bytes()); // ciclos
        let b = parse_ec(&d).unwrap();
        assert_eq!((b.percent, b.charging, b.state, b.source, b.cycles), (75, true, State::Charging, Source::Ec, Some(150)));
        assert!(close(b.power_w, 12.0) && close(b.energy_wh, 24.0) && close(b.full_wh, 32.0));
        assert!(close(b.health_percent, 80.0));
        // Capacidade cheia desconhecida: usa a de projeto (e a saúde fica sem dado).
        d[0x18..0x1C].copy_from_slice(&u32::MAX.to_le_bytes());
        d[0x0C] = FLAG_DISCHARGING;
        let b = parse_ec(&d).unwrap();
        assert_eq!((b.percent, b.charging, b.state, b.health_percent), (60, false, State::Discharging, None));
        // Na tomada, sem carregar nem descarregar.
        d[0x0C] = FLAG_AC;
        assert_eq!(parse_ec(&d).unwrap().state, State::NotCharging);
        // Carga desconhecida: nada.
        d[0x08..0x0C].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(parse_ec(&d), None);
    }

    #[test]
    fn estimativa_suave_e_sem_tempo_com_potencia_baixa() {
        let mut b = Battery::new(50, State::Discharging, Source::Kernel);
        b.energy_wh = Some(20.0);
        b.full_wh = Some(40.0);
        b.power_w = Some(10.0);
        let t0 = Instant::now();
        let mut e = Estimate::default();
        // 20 Wh a 10 W: 2 h.
        assert_eq!(e.update(&b, t0), (Some(7200), None));
        // Um pico de 40 W por um instante quase não mexe na estimativa.
        b.power_w = Some(40.0);
        let (empty, _) = e.update(&b, t0 + Duration::from_secs(1));
        assert!(empty.is_some_and(|t| t > 6800 && t < 7200), "{empty:?}");
        // Carregando: recomeça a média; faltam 20 Wh a 10 W = 2 h.
        b.state = State::Charging;
        b.power_w = Some(10.0);
        assert_eq!(e.update(&b, t0 + Duration::from_secs(2)), (None, Some(7200)));
        // Potência quase zero: sem tempo.
        b.power_w = Some(0.1);
        let mut e = Estimate::default();
        assert_eq!(e.update(&b, t0), (None, None));
        // Tempo dado pela fonte vale como está.
        b.state = State::Discharging;
        b.time_to_empty_s = Some(5400);
        assert_eq!(e.update(&b, t0).0, Some(5400));
    }

    #[test]
    fn duracao_formatada() {
        assert_eq!(format_duration(2 * 3600 + 15 * 60), "2 h 15 min");
        assert_eq!(format_duration(45 * 60), "45 min");
        assert_eq!(format_duration(3 * 3600 + 10), "3 h");
        assert_eq!(format_duration(20), "1 min");
    }
}
