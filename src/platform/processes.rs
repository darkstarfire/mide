//! Processos e uso do sistema, lidos do `/proc` (para o Monitor de Recursos).
//!
//! `Sampler::sample` lê tudo de uma vez e devolve uma `Snapshot` com as taxas (CPU em
//! %, bytes por segundo) calculadas contra a leitura anterior. Nada aqui precisa de
//! privilégio: processos de outros usuários aparecem, mas a leitura de disco por
//! processo (`/proc/PID/io`) só vem dos processos do próprio usuário (como no `top`).

use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

/// Um processo na hora da leitura.
#[derive(Clone, Debug, PartialEq)]
pub struct Process {
    pub pid: u32,
    pub ppid: u32,
    /// Nome curto (`comm`, até 15 letras no kernel) ou o nome do programa do
    /// `cmdline`, quando o `comm` foi cortado.
    pub name: String,
    /// Linha de comando inteira (vazia para os do kernel).
    pub command: String,
    pub uid: u32,
    /// Estado de uma letra (`R` rodando, `S` dormindo, `Z` zumbi...).
    pub state: char,
    pub threads: u32,
    /// Memória residente (bytes).
    pub memory: u64,
    /// Tempo de processador gasto desde o começo (segundos).
    pub cpu_time: f64,
    /// Uso de processador desde a leitura anterior, em % de um núcleo (pode passar de
    /// 100 com vários núcleos, como no Monitor de Atividade).
    pub cpu: f32,
    /// Bytes lidos e escritos no disco desde o começo (só os do usuário).
    pub disk_read: Option<u64>,
    pub disk_written: Option<u64>,
    /// Bytes por segundo lidos e escritos desde a leitura anterior.
    pub disk_read_rate: f64,
    pub disk_write_rate: f64,
    /// Processo do kernel (sem programa no disco).
    pub kernel: bool,
}

/// Tempos de processador do sistema (em tiques), da linha `cpu` do `/proc/stat`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CpuTimes {
    pub user: u64,
    pub system: u64,
    pub idle: u64,
    pub total: u64,
}

/// Uso do processador entre duas leituras, em % do total (0 a 100).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CpuUse {
    pub user: f32,
    pub system: f32,
    pub idle: f32,
}

/// Memória do sistema (bytes), do `/proc/meminfo`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Memory {
    pub total: u64,
    pub available: u64,
    pub cached: u64,
    pub buffers: u64,
    pub shared: u64,
    pub swap_total: u64,
    pub swap_free: u64,
}

impl Memory {
    /// Em uso pelos apps e pelo sistema (o que não dá para devolver na hora).
    pub fn used(&self) -> u64 {
        self.total.saturating_sub(self.available)
    }

    pub fn swap_used(&self) -> u64 {
        self.swap_total.saturating_sub(self.swap_free)
    }
}

/// Uma placa de rede (sem a de loopback).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Interface {
    pub name: String,
    pub received: u64,
    pub sent: u64,
    pub packets_in: u64,
    pub packets_out: u64,
    /// Bytes por segundo desde a leitura anterior.
    pub receive_rate: f64,
    pub send_rate: f64,
}

/// Tudo de uma leitura.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub processes: Vec<Process>,
    pub cpu: CpuUse,
    /// Núcleos do processador.
    pub cores: usize,
    pub memory: Memory,
    /// Pressão da memória do kernel (PSI, `some avg10`, de 0 a 100), quando existe.
    pub memory_pressure: Option<f32>,
    /// Bytes lidos e escritos nos discos desde o começo e por segundo agora.
    pub disk_read: u64,
    pub disk_written: u64,
    pub disk_read_rate: f64,
    pub disk_write_rate: f64,
    pub interfaces: Vec<Interface>,
    /// Tempo ligado (segundos).
    pub uptime: f64,
}

impl Snapshot {
    /// Soma dos bytes por segundo recebidos e enviados em todas as placas.
    pub fn network_rates(&self) -> (f64, f64) {
        self.interfaces.iter().fold((0.0, 0.0), |(r, s), i| (r + i.receive_rate, s + i.send_rate))
    }

    /// Total de threads de todos os processos.
    pub fn threads(&self) -> u32 {
        self.processes.iter().map(|p| p.threads).sum()
    }
}

/// Lê o sistema e calcula as taxas contra a leitura anterior.
pub struct Sampler {
    root: std::path::PathBuf,
    last: Option<Instant>,
    cpu: CpuTimes,
    procs: HashMap<u32, (f64, Option<u64>, Option<u64>)>,
    disk: (u64, u64),
    net: HashMap<String, (u64, u64)>,
    tick: f64,
    page: u64,
}

impl Default for Sampler {
    fn default() -> Self {
        Self::new()
    }
}

impl Sampler {
    pub fn new() -> Self {
        Self::with_root("/proc")
    }

    /// Lê de outra pasta no lugar do `/proc` (testes).
    pub fn with_root(root: impl Into<std::path::PathBuf>) -> Self {
        // SAFETY: `sysconf` só lê constantes do sistema.
        let (tick, page) = unsafe { (libc::sysconf(libc::_SC_CLK_TCK), libc::sysconf(libc::_SC_PAGESIZE)) };
        Self {
            root: root.into(),
            last: None,
            cpu: CpuTimes::default(),
            procs: HashMap::new(),
            disk: (0, 0),
            net: HashMap::new(),
            tick: if tick > 0 { tick as f64 } else { 100.0 },
            page: if page > 0 { page as u64 } else { 4096 },
        }
    }

    pub fn sample(&mut self) -> Snapshot {
        let now = Instant::now();
        let elapsed = self.last.map(|t| now.duration_since(t).as_secs_f64()).filter(|s| *s > 0.0);
        self.last = Some(now);
        let read = |name: &str| std::fs::read_to_string(self.root.join(name)).unwrap_or_default();

        let stat = read("stat");
        let (cpu_now, cores) = parse_stat(&stat);
        let cpu = cpu_use(&self.cpu, &cpu_now);
        self.cpu = cpu_now;

        let mut processes = Vec::new();
        let mut seen = HashMap::new();
        if let Ok(dir) = std::fs::read_dir(&self.root) {
            for entry in dir.flatten() {
                let Some(pid) = entry.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else {
                    continue;
                };
                let Some(mut p) = read_process(&entry.path(), pid, self.tick, self.page) else {
                    continue;
                };
                if let (Some(secs), Some((before, read_before, write_before))) = (elapsed, self.procs.get(&pid)) {
                    p.cpu = (((p.cpu_time - before) / secs) * 100.0).max(0.0) as f32;
                    p.disk_read_rate = rate(*read_before, p.disk_read, secs);
                    p.disk_write_rate = rate(*write_before, p.disk_written, secs);
                }
                seen.insert(pid, (p.cpu_time, p.disk_read, p.disk_written));
                processes.push(p);
            }
        }
        self.procs = seen;

        let memory = parse_meminfo(&read("meminfo"));
        let memory_pressure = parse_pressure(&read("pressure/memory"));

        let (disk_read, disk_written) = parse_diskstats(&read("diskstats"), is_whole_disk);
        let (disk_read_rate, disk_write_rate) = match elapsed {
            Some(secs) => ((disk_read.saturating_sub(self.disk.0)) as f64 / secs, (disk_written.saturating_sub(self.disk.1)) as f64 / secs),
            None => (0.0, 0.0),
        };
        self.disk = (disk_read, disk_written);

        let mut interfaces = parse_net_dev(&read("net/dev"));
        for i in &mut interfaces {
            if let (Some(secs), Some((r, s))) = (elapsed, self.net.get(&i.name)) {
                i.receive_rate = i.received.saturating_sub(*r) as f64 / secs;
                i.send_rate = i.sent.saturating_sub(*s) as f64 / secs;
            }
        }
        self.net = interfaces.iter().map(|i| (i.name.clone(), (i.received, i.sent))).collect();

        let uptime = read("uptime").split_whitespace().next().and_then(|s| s.parse().ok()).unwrap_or(0.0);
        Snapshot { processes, cpu, cores, memory, memory_pressure, disk_read, disk_written, disk_read_rate, disk_write_rate, interfaces, uptime }
    }
}

fn rate(before: Option<u64>, now: Option<u64>, secs: f64) -> f64 {
    match (before, now) {
        (Some(b), Some(n)) => n.saturating_sub(b) as f64 / secs,
        _ => 0.0,
    }
}

/// Lê um processo da pasta dele (`/proc/PID`).
fn read_process(dir: &Path, pid: u32, tick: f64, page: u64) -> Option<Process> {
    let stat = std::fs::read_to_string(dir.join("stat")).ok()?;
    let fields = parse_pid_stat(&stat)?;
    let status = std::fs::read_to_string(dir.join("status")).unwrap_or_default();
    let uid = status.lines().find_map(|l| l.strip_prefix("Uid:")).and_then(|v| v.split_whitespace().next()).and_then(|v| v.parse().ok()).unwrap_or(0);
    let statm = std::fs::read_to_string(dir.join("statm")).unwrap_or_default();
    let resident = statm.split_whitespace().nth(1).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
    let cmdline = std::fs::read(dir.join("cmdline")).unwrap_or_default();
    let command = cmdline.split(|b| *b == 0).filter(|a| !a.is_empty()).map(|a| String::from_utf8_lossy(a).into_owned()).collect::<Vec<_>>().join(" ");
    let io = std::fs::read_to_string(dir.join("io")).ok();
    let io_field = |key: &str| io.as_deref().and_then(|t| t.lines().find_map(|l| l.strip_prefix(key))).and_then(|v| v.trim().parse::<u64>().ok());
    let name = display_name(&fields.comm, &cmdline);
    Some(Process {
        pid,
        ppid: fields.ppid,
        name,
        kernel: cmdline.is_empty(),
        command,
        uid,
        state: fields.state,
        threads: fields.threads,
        memory: resident * page,
        cpu_time: (fields.utime + fields.stime) as f64 / tick,
        cpu: 0.0,
        disk_read: io_field("read_bytes:"),
        disk_written: io_field("write_bytes:"),
        disk_read_rate: 0.0,
        disk_write_rate: 0.0,
    })
}

/// Campos do `/proc/PID/stat` que interessam.
#[derive(Debug, PartialEq)]
struct PidStat {
    comm: String,
    state: char,
    ppid: u32,
    utime: u64,
    stime: u64,
    threads: u32,
}

/// O nome (`comm`) vem entre parênteses e pode ter espaços e parênteses dentro: o
/// resto começa depois do último `)`.
fn parse_pid_stat(text: &str) -> Option<PidStat> {
    let open = text.find('(')?;
    let close = text.rfind(')')?;
    let comm = text.get(open + 1..close)?.to_string();
    let rest: Vec<&str> = text.get(close + 1..)?.split_whitespace().collect();
    // Depois do nome: estado (3), ppid (4) ... utime (14), stime (15) ... threads (20).
    let field = |n: usize| rest.get(n - 3).copied();
    Some(PidStat {
        comm,
        state: field(3)?.chars().next()?,
        ppid: field(4)?.parse().ok()?,
        utime: field(14)?.parse().ok()?,
        stime: field(15)?.parse().ok()?,
        threads: field(20)?.parse().ok()?,
    })
}

/// Nome para mostrar: o `comm`, ou o nome do programa do `cmdline` quando o `comm`
/// foi cortado nos 15 caracteres do kernel.
fn display_name(comm: &str, cmdline: &[u8]) -> String {
    if comm.len() >= 15 {
        let first = cmdline.split(|b| *b == 0).next().unwrap_or_default();
        let program = String::from_utf8_lossy(first);
        if let Some(base) = program.rsplit('/').next().filter(|b| b.starts_with(comm)) {
            return base.to_string();
        }
    }
    comm.to_string()
}

/// Linha `cpu` do `/proc/stat` e quantos núcleos (`cpuN`).
fn parse_stat(text: &str) -> (CpuTimes, usize) {
    let mut times = CpuTimes::default();
    let mut cores = 0;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("cpu ") {
            let v: Vec<u64> = rest.split_whitespace().filter_map(|s| s.parse().ok()).collect();
            let get = |i: usize| v.get(i).copied().unwrap_or(0);
            // user nice system idle iowait irq softirq steal
            times.user = get(0) + get(1);
            times.system = get(2) + get(5) + get(6);
            times.idle = get(3) + get(4);
            times.total = v.iter().take(8).sum();
        } else if line.starts_with("cpu") {
            cores += 1;
        }
    }
    (times, cores.max(1))
}

fn cpu_use(before: &CpuTimes, now: &CpuTimes) -> CpuUse {
    let total = now.total.saturating_sub(before.total);
    if before.total == 0 || total == 0 {
        return CpuUse { user: 0.0, system: 0.0, idle: 100.0 };
    }
    let pct = |a: u64, b: u64| a.saturating_sub(b) as f32 * 100.0 / total as f32;
    CpuUse { user: pct(now.user, before.user), system: pct(now.system, before.system), idle: pct(now.idle, before.idle) }
}

fn parse_meminfo(text: &str) -> Memory {
    let get = |key: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(key).and_then(|r| r.strip_prefix(':')))
            .and_then(|v| v.split_whitespace().next())
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0)
            * 1024
    };
    Memory {
        total: get("MemTotal"),
        available: get("MemAvailable"),
        cached: get("Cached"),
        buffers: get("Buffers"),
        shared: get("Shmem"),
        swap_total: get("SwapTotal"),
        swap_free: get("SwapFree"),
    }
}

/// `some avg10=1.23 ...` do `/proc/pressure/memory`.
fn parse_pressure(text: &str) -> Option<f32> {
    let line = text.lines().find(|l| l.starts_with("some"))?;
    line.split_whitespace().find_map(|f| f.strip_prefix("avg10=")).and_then(|v| v.parse().ok())
}

/// Soma dos setores lidos e escritos (em bytes) dos discos inteiros.
fn parse_diskstats(text: &str, whole: impl Fn(&str) -> bool) -> (u64, u64) {
    let mut read = 0;
    let mut written = 0;
    for line in text.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 10 || !whole(f[2]) {
            continue;
        }
        read += f[5].parse::<u64>().unwrap_or(0) * 512;
        written += f[9].parse::<u64>().unwrap_or(0) * 512;
    }
    (read, written)
}

/// Disco inteiro (não partição, nem loop/ram/zram): tem pasta em `/sys/block`.
fn is_whole_disk(name: &str) -> bool {
    if name.starts_with("loop") || name.starts_with("ram") || name.starts_with("zram") || name.starts_with("dm-") {
        return false;
    }
    Path::new("/sys/block").join(name).exists()
}

fn parse_net_dev(text: &str) -> Vec<Interface> {
    text.lines()
        .skip(2)
        .filter_map(|line| {
            let (name, rest) = line.split_once(':')?;
            let name = name.trim();
            if name == "lo" {
                return None;
            }
            let v: Vec<u64> = rest.split_whitespace().filter_map(|s| s.parse().ok()).collect();
            Some(Interface {
                name: name.to_string(),
                received: *v.first()?,
                packets_in: *v.get(1)?,
                sent: *v.get(8)?,
                packets_out: *v.get(9)?,
                receive_rate: 0.0,
                send_rate: 0.0,
            })
        })
        .collect()
}

/// Nomes dos usuários pelo `uid` (`/etc/passwd`).
pub fn user_names() -> HashMap<u32, String> {
    let text = std::fs::read_to_string("/etc/passwd").unwrap_or_default();
    text.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split(':').collect();
            Some((f.get(2)?.parse().ok()?, f.first()?.to_string()))
        })
        .collect()
}

/// Pede ao processo para encerrar (`SIGTERM`) ou o força (`SIGKILL`).
pub fn quit(pid: u32, force: bool) -> std::io::Result<()> {
    let signal = if force { libc::SIGKILL } else { libc::SIGTERM };
    // SAFETY: `kill` só manda um sinal ao processo; o erro vem em `errno`.
    let r = unsafe { libc::kill(pid as libc::pid_t, signal) };
    if r == 0 { Ok(()) } else { Err(std::io::Error::last_os_error()) }
}

/// Tamanho em bytes para ler: "512 bytes", "1,2 MB", "3,40 GB" (base 1000, como no Mac).
pub fn bytes_text(bytes: f64) -> String {
    const UNITS: [&str; 5] = ["KB", "MB", "GB", "TB", "PB"];
    if bytes < 1000.0 {
        return format!("{} bytes", bytes.round() as u64);
    }
    let mut v = bytes / 1000.0;
    let mut i = 0;
    while v >= 1000.0 && i + 1 < UNITS.len() {
        v /= 1000.0;
        i += 1;
    }
    let text = if v >= 100.0 { format!("{v:.0}") } else if v >= 10.0 { format!("{v:.1}") } else { format!("{v:.2}") };
    format!("{} {}", text.replace('.', ","), UNITS[i])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stat_do_processo_com_parenteses_e_espacos_no_nome() {
        let line = "1234 (Web Content (x)) S 1 1234 1234 0 -1 4194560 100 0 0 0 250 50 0 0 20 0 27 0 1000 0 0";
        let s = parse_pid_stat(line).unwrap();
        assert_eq!(s.comm, "Web Content (x)");
        assert_eq!((s.state, s.ppid, s.utime, s.stime, s.threads), ('S', 1, 250, 50, 27));
    }

    #[test]
    fn nome_cortado_pelo_kernel_vem_do_cmdline() {
        assert_eq!(display_name("xdg-desktop-por", b"/usr/libexec/xdg-desktop-portal-gtk\0--replace"), "xdg-desktop-portal-gtk");
        assert_eq!(display_name("bash", b"/bin/bash\0"), "bash");
    }

    #[test]
    fn cpu_do_sistema_entre_duas_leituras() {
        let (a, cores) = parse_stat("cpu  100 0 50 800 50 0 0 0 0 0\ncpu0 1\ncpu1 1\nintr 1");
        assert_eq!(cores, 2);
        let (b, _) = parse_stat("cpu  200 0 100 1050 50 0 0 0 0 0\n");
        let u = cpu_use(&a, &b);
        assert!((u.user - 25.0).abs() < 0.01 && (u.system - 12.5).abs() < 0.01 && (u.idle - 62.5).abs() < 0.01);
    }

    #[test]
    fn memoria_disco_rede_e_pressao() {
        let m = parse_meminfo("MemTotal:  8000 kB\nMemAvailable: 6000 kB\nCached: 1000 kB\nSwapTotal: 2000 kB\nSwapFree: 1500 kB\n");
        assert_eq!((m.used(), m.swap_used()), (2000 * 1024, 500 * 1024));
        let disks = "   8       0 sda 10 0 100 0 20 0 200 0 0 0 0\n   8       1 sda1 5 0 40 0 5 0 50 0 0 0 0\n";
        assert_eq!(parse_diskstats(disks, |n| n == "sda"), (100 * 512, 200 * 512));
        let net = "Inter-|   Receive\n face |bytes\n    lo: 9 1 0 0 0 0 0 0 9 1 0 0 0 0 0 0\n  eth0: 1000 10 0 0 0 0 0 0 500 5 0 0 0 0 0 0\n";
        let i = parse_net_dev(net);
        assert_eq!(i.len(), 1);
        assert_eq!((i[0].name.as_str(), i[0].received, i[0].sent, i[0].packets_in, i[0].packets_out), ("eth0", 1000, 500, 10, 5));
        assert_eq!(parse_pressure("some avg10=1.50 avg60=0.10 avg300=0.00 total=1\nfull avg10=0.00"), Some(1.5));
    }

    #[test]
    fn tamanhos_para_ler() {
        assert_eq!(bytes_text(512.0), "512 bytes");
        assert_eq!(bytes_text(1_234_000.0), "1,23 MB");
        assert_eq!(bytes_text(45_600_000_000.0), "45,6 GB");
    }

    #[test]
    fn le_o_proc_de_verdade() {
        let mut s = Sampler::new();
        let first = s.sample();
        assert!(first.processes.iter().any(|p| p.pid == std::process::id()));
        assert!(first.memory.total > 0 && first.cores >= 1);
        std::thread::sleep(std::time::Duration::from_millis(50));
        let second = s.sample();
        assert!(second.cpu.idle <= 100.0);
    }
}
