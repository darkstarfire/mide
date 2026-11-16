//! Monitor de Recursos (`mide-monitor`), como o Monitor de Atividade do Mac: os
//! processos com CPU, memória, energia e disco, as placas de rede, e embaixo os
//! gráficos e os números do sistema. Encerra e força o encerramento de processos.
//!
//! A leitura do `/proc` (`mide::processes`) roda numa thread, a cada intervalo
//! escolhido em Visualizar → Frequência de atualização, e acorda a janela.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use std::time::Duration;

use mide::kit::{run, window_menu, App, MenuGroup, MenuItem, Ui, Window};
use mide::processes::{bytes_text, quit, user_names, Process, Sampler, Snapshot};
use mide::tr;

/// Números do painel de baixo: (título, valor).
type Pairs = Vec<(String, String)>;

/// Quantas leituras os gráficos guardam.
const HISTORY: usize = 120;

/// Abas (como as do Monitor de Atividade).
const CPU: usize = 0;
const MEMORY: usize = 1;
const ENERGY: usize = 2;
const DISK: usize = 3;
const NETWORK: usize = 4;

/// Altura do painel de baixo (gráfico e números).
const PANEL_H: f32 = 150.0;

#[derive(Default)]
struct History {
    cpu_user: VecDeque<f32>,
    cpu_system: VecDeque<f32>,
    memory: VecDeque<f32>,
    pressure: VecDeque<f32>,
    disk_read: VecDeque<f32>,
    disk_write: VecDeque<f32>,
    net_in: VecDeque<f32>,
    net_out: VecDeque<f32>,
}

fn push(q: &mut VecDeque<f32>, v: f32) {
    if q.len() >= HISTORY {
        q.pop_front();
    }
    q.push_back(v);
}

impl History {
    fn add(&mut self, s: &Snapshot) {
        push(&mut self.cpu_user, s.cpu.user);
        push(&mut self.cpu_system, s.cpu.system);
        let total = s.memory.total.max(1) as f32;
        push(&mut self.memory, s.memory.used() as f32 * 100.0 / total);
        push(&mut self.pressure, s.memory_pressure.unwrap_or(0.0));
        push(&mut self.disk_read, s.disk_read_rate as f32);
        push(&mut self.disk_write, s.disk_write_rate as f32);
        let (rx, tx) = s.network_rates();
        push(&mut self.net_in, rx as f32);
        push(&mut self.net_out, tx as f32);
    }
}

/// O que mostrar na lista (Visualizar).
#[derive(Clone, Copy, PartialEq)]
enum Filter {
    All,
    Mine,
    Active,
}

struct Monitor {
    tab: usize,
    /// Coluna ordenada e se é crescente, por aba (0 = nome).
    sort: [(usize, bool); 5],
    search: String,
    selected: Option<u32>,
    snap: Option<Snapshot>,
    rx: Option<Receiver<Snapshot>>,
    interval: Arc<AtomicU64>,
    history: History,
    users: HashMap<u32, String>,
    my_uid: u32,
    filter: Filter,
    /// Nome e ícone dos apps instalados, pelo nome do programa.
    apps: HashMap<String, (String, String)>,
    confirm: Option<(u32, String)>,
    info: Option<u32>,
    message: Option<String>,
    /// Bateria (aba Energia), lida a cada leitura do sistema.
    battery: Option<mide::battery::Battery>,
}

impl Monitor {
    fn new() -> Self {
        let mut apps = HashMap::new();
        for app in mide::apps::load() {
            let program = app.command.split_whitespace().next().unwrap_or_default();
            let base = program.rsplit('/').next().unwrap_or_default().to_string();
            if !base.is_empty() {
                apps.entry(base).or_insert((app.name.clone(), app.icon.clone().unwrap_or_default()));
            }
        }
        Self {
            tab: CPU,
            sort: [(1, false), (1, false), (1, false), (1, false), (1, false)],
            search: String::new(),
            selected: None,
            snap: None,
            rx: None,
            interval: Arc::new(AtomicU64::new(2000)),
            history: History::default(),
            users: user_names(),
            // SAFETY: `getuid` não falha.
            my_uid: unsafe { libc::getuid() },
            filter: Filter::All,
            apps,
            confirm: None,
            info: None,
            message: None,
            battery: None,
        }
    }

    /// Começa a thread que lê o sistema (com o despertador da janela).
    fn start(&mut self, ui: &Ui) {
        if self.rx.is_some() {
            return;
        }
        let (tx, rx) = channel();
        let waker = ui.waker();
        let interval = Arc::clone(&self.interval);
        std::thread::spawn(move || {
            let mut sampler = Sampler::new();
            // Primeira leitura logo (as taxas aparecem na segunda).
            let _ = sampler.sample();
            std::thread::sleep(Duration::from_millis(400));
            loop {
                if tx.send(sampler.sample()).is_err() {
                    return;
                }
                if let Some(w) = &waker {
                    w.wake();
                }
                std::thread::sleep(Duration::from_millis(interval.load(Ordering::Relaxed)));
            }
        });
        self.rx = Some(rx);
    }

    /// Nome e ícone de um processo: o do app instalado, quando é um.
    fn label(&self, p: &Process) -> (String, String) {
        match self.apps.get(&p.name) {
            Some((name, icon)) if !icon.is_empty() => (name.clone(), icon.clone()),
            Some((name, _)) => (name.clone(), "application-x-executable".into()),
            None => (p.name.clone(), if p.kernel { "applications-system".into() } else { "application-x-executable".into() }),
        }
    }

    fn user(&self, uid: u32) -> String {
        self.users.get(&uid).cloned().unwrap_or_else(|| uid.to_string())
    }

    /// Colunas da aba (título e largura), sem o nome.
    fn columns(&self) -> Vec<(String, f32)> {
        let c = |s: std::borrow::Cow<'static, str>, w: f32| (s.into_owned(), w);
        match self.tab {
            CPU => vec![c(tr!("% CPU"), 72.0), c(tr!("CPU Time"), 96.0), c(tr!("Threads"), 70.0), c(tr!("PID"), 70.0), c(tr!("User"), 96.0)],
            MEMORY => vec![c(tr!("Memory"), 100.0), c(tr!("Threads"), 70.0), c(tr!("PID"), 70.0), c(tr!("User"), 96.0)],
            ENERGY => vec![c(tr!("Energy Impact"), 150.0), c(tr!("% CPU"), 72.0), c(tr!("PID"), 70.0), c(tr!("User"), 96.0)],
            DISK => vec![c(tr!("Bytes Written"), 110.0), c(tr!("Bytes Read"), 110.0), c(tr!("PID"), 70.0), c(tr!("User"), 96.0)],
            _ => vec![c(tr!("Received"), 100.0), c(tr!("Sent"), 100.0), c(tr!("Packets In"), 140.0), c(tr!("Packets Out"), 140.0)],
        }
    }

    /// Valores da linha de um processo, na ordem das colunas.
    fn cells(&self, p: &Process) -> Vec<String> {
        let user = self.user(p.uid);
        match self.tab {
            CPU => vec![format!("{:.1}", p.cpu).replace('.', ","), cpu_time(p.cpu_time), p.threads.to_string(), p.pid.to_string(), user],
            MEMORY => vec![bytes_text(p.memory as f64), p.threads.to_string(), p.pid.to_string(), user],
            ENERGY => vec![format!("{:.1}", energy(p)).replace('.', ","), format!("{:.1}", p.cpu).replace('.', ","), p.pid.to_string(), user],
            _ => vec![p.disk_written.map(|b| bytes_text(b as f64)).unwrap_or_else(|| "—".into()), p.disk_read.map(|b| bytes_text(b as f64)).unwrap_or_else(|| "—".into()), p.pid.to_string(), user],
        }
    }

    /// Processos da lista: filtro, busca e ordem.
    fn rows(&self) -> Vec<&Process> {
        let Some(snap) = &self.snap else {
            return Vec::new();
        };
        let query = mide::apps::fold(self.search.trim());
        let mut rows: Vec<&Process> = snap
            .processes
            .iter()
            .filter(|p| match self.filter {
                Filter::All => true,
                Filter::Mine => p.uid == self.my_uid,
                Filter::Active => p.cpu > 0.0 || p.state == 'R',
            })
            .filter(|p| query.is_empty() || mide::apps::fold(&self.label(p).0).contains(&query) || p.pid.to_string() == query || mide::apps::fold(&p.command).contains(&query))
            .collect();
        let (col, asc) = self.sort[self.tab];
        let key = |p: &Process| -> f64 {
            match (self.tab, col) {
                (_, 0) => 0.0,
                (CPU, 1) | (ENERGY, 2) => p.cpu as f64,
                (CPU, 2) => p.cpu_time,
                (CPU, 3) | (MEMORY, 2) => p.threads as f64,
                (ENERGY, 1) => energy(p) as f64,
                (MEMORY, 1) => p.memory as f64,
                (DISK, 1) => p.disk_written.unwrap_or(0) as f64,
                (DISK, 2) => p.disk_read.unwrap_or(0) as f64,
                _ => p.pid as f64,
            }
        };
        let user_col = matches!((self.tab, col), (CPU, 5) | (MEMORY, 4) | (ENERGY, 4) | (DISK, 4));
        rows.sort_by(|a, b| {
            let o = if col == 0 {
                mide::apps::fold(&self.label(a).0).cmp(&mide::apps::fold(&self.label(b).0))
            } else if user_col {
                self.user(a.uid).cmp(&self.user(b.uid))
            } else {
                key(a).partial_cmp(&key(b)).unwrap_or(std::cmp::Ordering::Equal)
            };
            if asc { o } else { o.reverse() }
        });
        rows
    }

    fn quit_selected(&mut self, force: bool) {
        let Some((pid, name)) = self.confirm.take() else {
            return;
        };
        match quit(pid, force) {
            Ok(()) => {
                tracing::info!("processo {pid} ({name}) encerrado{}", if force { " à força" } else { "" });
                self.selected = None;
            }
            Err(err) => self.message = Some(tr!("Couldn’t quit “{name}”: {err}", name = name, err = err)),
        }
    }

    /// Painel de baixo: números à esquerda e à direita, gráfico no meio.
    fn panel(&self, ui: &mut Ui, rect: (f32, f32, f32, f32)) {
        let Some(s) = &self.snap else {
            return;
        };
        let (x, y, w, h) = rect;
        let side = (w * 0.3).clamp(180.0, 260.0);
        let graph_w = (w - 2.0 * side - 32.0).max(120.0);
        let h_ = &self.history;
        let v = |q: &VecDeque<f32>| q.iter().copied().collect::<Vec<f32>>();
        let accent = ui.theme().accent;
        let (left, right, title): (Pairs, Pairs, String) = match self.tab {
            CPU => (
                vec![
                    (tr!("System:").into_owned(), pct(s.cpu.system)),
                    (tr!("User:").into_owned(), pct(s.cpu.user)),
                    (tr!("Idle:").into_owned(), pct(s.cpu.idle)),
                ],
                vec![
                    (tr!("Threads:").into_owned(), s.threads().to_string()),
                    (tr!("Processes:").into_owned(), s.processes.len().to_string()),
                    (tr!("Cores:").into_owned(), s.cores.to_string()),
                ],
                tr!("CPU Load").into_owned(),
            ),
            ENERGY => {
                let (charge, state) = match &self.battery {
                    Some(b) => (
                        format!("{} %", b.percent),
                        if b.charging { tr!("Charging").into_owned() } else { tr!("On Battery").into_owned() },
                    ),
                    None => ("—".to_string(), tr!("Power Adapter").into_owned()),
                };
                let remaining = self.battery.as_ref().and_then(|b| if b.charging { b.time_to_full_s } else { b.time_to_empty_s }).map(mide::battery::format_duration).unwrap_or_else(|| "—".to_string());
                let power = self.battery.as_ref().and_then(|b| b.power_w).map(|w| format!("{:.1} W", w).replace('.', ",")).unwrap_or_else(|| "—".to_string());
                (
                    vec![
                        (tr!("Battery:").into_owned(), charge),
                        (tr!("Power Source:").into_owned(), state),
                        (tr!("Time Remaining:").into_owned(), remaining),
                    ],
                    vec![(tr!("Power Use:").into_owned(), power), (tr!("Uptime:").into_owned(), uptime(s.uptime))],
                    tr!("Energy Impact").into_owned(),
                )
            }
            MEMORY => (
                vec![
                    (tr!("Physical Memory:").into_owned(), bytes_text(s.memory.total as f64)),
                    (tr!("Memory Used:").into_owned(), bytes_text(s.memory.used() as f64)),
                    (tr!("Cached Files:").into_owned(), bytes_text(s.memory.cached as f64)),
                    (tr!("Swap Used:").into_owned(), bytes_text(s.memory.swap_used() as f64)),
                ],
                vec![
                    (tr!("Available:").into_owned(), bytes_text(s.memory.available as f64)),
                    (tr!("Shared:").into_owned(), bytes_text(s.memory.shared as f64)),
                    (tr!("Uptime:").into_owned(), uptime(s.uptime)),
                ],
                if s.memory_pressure.is_some() { tr!("Memory Pressure").into_owned() } else { tr!("Memory Used").into_owned() },
            ),
            DISK => (
                vec![
                    (tr!("Data read:").into_owned(), bytes_text(s.disk_read as f64)),
                    (tr!("Data written:").into_owned(), bytes_text(s.disk_written as f64)),
                ],
                vec![
                    (tr!("Read/sec:").into_owned(), bytes_text(s.disk_read_rate)),
                    (tr!("Written/sec:").into_owned(), bytes_text(s.disk_write_rate)),
                ],
                tr!("Data").into_owned(),
            ),
            _ => {
                let (rx, tx) = s.network_rates();
                let total = |f: fn(&mide::processes::Interface) -> u64| s.interfaces.iter().map(f).sum::<u64>();
                (
                    vec![
                        (tr!("Data received:").into_owned(), bytes_text(total(|i| i.received) as f64)),
                        (tr!("Data sent:").into_owned(), bytes_text(total(|i| i.sent) as f64)),
                    ],
                    vec![
                        (tr!("Received/sec:").into_owned(), bytes_text(rx)),
                        (tr!("Sent/sec:").into_owned(), bytes_text(tx)),
                    ],
                    tr!("Data").into_owned(),
                )
            }
        };
        ui.at(x + 16.0, y + 16.0, side - 16.0, |ui| {
            for (k, val) in &left {
                ui.key_value(k, val, 130.0);
            }
        });
        ui.at(x + side + 16.0, y + 10.0, graph_w, |ui| {
            ui.heading(&title);
            let gh = (h - 54.0).max(40.0);
            let red = [255, 69, 58, 255];
            let blue = accent;
            let green = [52, 199, 89, 255];
            match self.tab {
                CPU => {
                    let (sys, user) = (v(&h_.cpu_system), v(&h_.cpu_user));
                    ui.history_graph(&[(&sys, red), (&user, blue)], 100.0, true, gh);
                }
                MEMORY => {
                    let data = if s.memory_pressure.is_some() { v(&h_.pressure) } else { v(&h_.memory) };
                    ui.history_graph(&[(&data, green)], 100.0, false, gh);
                }
                ENERGY => {
                    // Impacto energético do sistema: o uso total do processador.
                    let total: Vec<f32> = h_.cpu_user.iter().zip(&h_.cpu_system).map(|(u, s)| u + s).collect();
                    ui.history_graph(&[(&total, green)], 100.0, false, gh);
                }
                DISK => {
                    let (r, w_) = (v(&h_.disk_read), v(&h_.disk_write));
                    let max = r.iter().chain(&w_).fold(1024.0f32, |m, x| m.max(*x)) * 1.2;
                    ui.history_graph(&[(&r, blue), (&w_, red)], max, false, gh);
                }
                _ => {
                    let (i, o) = (v(&h_.net_in), v(&h_.net_out));
                    let max = i.iter().chain(&o).fold(1024.0f32, |m, x| m.max(*x)) * 1.2;
                    ui.history_graph(&[(&i, blue), (&o, red)], max, false, gh);
                }
            }
        });
        ui.at(x + side + graph_w + 32.0, y + 16.0, side - 16.0, |ui| {
            for (k, val) in &right {
                ui.key_value(k, val, 110.0);
            }
        });
    }

    fn info_view(&mut self, ui: &mut Ui) {
        let Some(pid) = self.info else {
            return;
        };
        let Some(p) = self.snap.as_ref().and_then(|s| s.processes.iter().find(|p| p.pid == pid)).cloned() else {
            self.info = None;
            return;
        };
        let (name, _) = self.label(&p);
        let parent = self.snap.as_ref().and_then(|s| s.processes.iter().find(|q| q.pid == p.ppid)).map(|q| format!("{} ({})", self.label(q).0, q.pid)).unwrap_or_else(|| p.ppid.to_string());
        let mut close = ui.input().key(mide::kit::Key::Escape);
        let user = self.user(p.uid);
        ui.modal(460.0, |ui| {
            ui.title(&name);
            ui.key_value(&tr!("Parent Process:"), &parent, 150.0);
            ui.key_value(&tr!("User:"), &user, 150.0);
            ui.key_value(&tr!("Process ID:"), &p.pid.to_string(), 150.0);
            ui.key_value(&tr!("% CPU:"), &pct(p.cpu), 150.0);
            ui.key_value(&tr!("CPU Time:"), &cpu_time(p.cpu_time), 150.0);
            ui.key_value(&tr!("Memory:"), &bytes_text(p.memory as f64), 150.0);
            ui.key_value(&tr!("Threads:"), &p.threads.to_string(), 150.0);
            if !p.command.is_empty() {
                ui.note(&p.command);
            }
            ui.row(|ui| {
                if ui.button(&tr!("Quit Process")).clicked() {
                    self.confirm = Some((p.pid, name.clone()));
                    close = true;
                }
                if ui.primary_button(&tr!("Close")).clicked() {
                    close = true;
                }
            });
        });
        if close {
            self.info = None;
        }
    }
}

/// Impacto energético aproximado pelo uso do processador (sem medidor de energia por
/// processo no Linux): o mesmo número que o % de CPU, como base de comparação.
fn energy(p: &Process) -> f32 {
    p.cpu
}

fn pct(v: f32) -> String {
    format!("{:.2} %", v).replace('.', ",")
}

/// Tempo de CPU como "1:02:03,45" (horas só quando há).
fn cpu_time(secs: f64) -> String {
    let total = secs.max(0.0);
    let h = (total / 3600.0) as u64;
    let m = ((total % 3600.0) / 60.0) as u64;
    let s = total % 60.0;
    let text = if h > 0 { format!("{h}:{m:02}:{s:05.2}") } else { format!("{m}:{s:05.2}") };
    text.replace('.', ",")
}

fn uptime(secs: f64) -> String {
    let s = secs as u64;
    let (d, h, m) = (s / 86400, (s % 86400) / 3600, (s % 3600) / 60);
    if d > 0 { tr!("{d} d {h} h {m} min", d = d, h = h, m = m) } else { tr!("{h} h {m} min", h = h, m = m) }
}

impl App for Monitor {
    fn view(&mut self, ui: &mut Ui) {
        self.start(ui);
        if let Some(rx) = &self.rx {
            let mut fresh = false;
            while let Ok(s) = rx.try_recv() {
                if self.snap.is_none() {
                    tracing::info!("monitor: {} processos, {} núcleos, {} placas de rede", s.processes.len(), s.cores, s.interfaces.len());
                }
                self.history.add(&s);
                self.snap = Some(s);
                fresh = true;
            }
            if fresh && self.tab == ENERGY {
                self.battery = mide::battery::read();
            }
        }
        for action in ui.input().menu.clone() {
            match action.as_str() {
                "quit" => ui.close(),
                "tab-0" | "tab-1" | "tab-2" | "tab-3" | "tab-4" => self.tab = action[4..].parse().unwrap_or(0),
                "all" => self.filter = Filter::All,
                "mine" => self.filter = Filter::Mine,
                "active" => self.filter = Filter::Active,
                "every-1" => self.interval.store(1000, Ordering::Relaxed),
                "every-2" => self.interval.store(2000, Ordering::Relaxed),
                "every-5" => self.interval.store(5000, Ordering::Relaxed),
                "quit-process" => {
                    if let Some(p) = self.selected.and_then(|pid| self.snap.as_ref()?.processes.iter().find(|p| p.pid == pid).cloned()) {
                        self.confirm = Some((p.pid, self.label(&p).0));
                    }
                }
                "info" => self.info = self.selected,
                _ => {}
            }
        }
        // ⌘1…⌘5 trocam de aba (como no Mac).
        for (n, key) in ["1", "2", "3", "4", "5"].iter().enumerate() {
            if ui.input().shortcut(key) && !ui.typing() {
                self.tab = n;
            }
        }
        let tabs = [tr!("CPU"), tr!("Memory"), tr!("Energy"), tr!("Disk"), tr!("Network")];
        let tab_refs: Vec<&str> = tabs.iter().map(|t| t.as_ref()).collect();
        let has_selection = self.selected.is_some() && self.tab != NETWORK;
        let mut clicked = None;
        let subtitle = match self.filter {
            Filter::All => tr!("All Processes"),
            Filter::Mine => tr!("My Processes"),
            Filter::Active => tr!("Active Processes"),
        };
        ui.toolbar(|ui| {
            // Como no Monitor de Atividade: o nome do app, ou o filtro quando não são
            // todos os processos (o título não cabe inteiro com os dois).
            let title = if self.filter == Filter::All { tr!("Resource Monitor") } else { subtitle.clone() };
            ui.toolbar_title(&title, 220.0);
            clicked = ui.toolbar_buttons(&[("process-stop-symbolic", "×", false, has_selection), ("dialog-information-symbolic", "i", false, has_selection)]);
            ui.push_right(200.0 + 12.0 + 380.0);
            let mut tab = self.tab;
            if ui.segmented(&tab_refs, &mut tab, Some(380.0)).clicked() {
                self.tab = tab;
            }
            ui.search_field(&mut self.search, &tr!("Search"));
        });
        match clicked {
            Some(0) => {
                if let Some(p) = self.selected.and_then(|pid| self.snap.as_ref()?.processes.iter().find(|p| p.pid == pid).cloned()) {
                    self.confirm = Some((p.pid, self.label(&p).0));
                }
            }
            Some(1) => self.info = self.selected,
            _ => {}
        }
        let region = ui.region();
        let top = region.1 + region.3 - ui.remaining_height();
        let (x, w) = (region.0 + 12.0, region.2 - 24.0);
        let table_h = (region.1 + region.3 - top - PANEL_H - 12.0).max(80.0);
        let columns = self.columns();
        let col_refs: Vec<(&str, f32)> = columns.iter().map(|(t, w)| (t.as_str(), *w)).collect();
        if self.snap.is_none() {
            ui.at(x, top + 20.0, w, |ui| ui.note(&tr!("Reading the system…")));
        } else if self.tab == NETWORK {
            let snap = self.snap.clone().unwrap_or_default();
            ui.at(x, top, w, |ui| {
                ui.list_header(&tr!("Interface"), &col_refs, (0, true));
                ui.scroll_area(table_h - 26.0, |ui| {
                    for i in &snap.interfaces {
                        let cells = [bytes_text(i.received as f64), bytes_text(i.sent as f64), i.packets_in.to_string(), i.packets_out.to_string()];
                        let cols: Vec<(&str, f32)> = cells.iter().zip(&col_refs).map(|(c, (_, w))| (c.as_str(), *w)).collect();
                        ui.list_row("network-wired-symbolic", &i.name, &cols, false);
                    }
                });
            });
        } else {
            let rows: Vec<(u32, String, String, Vec<String>)> = self
                .rows()
                .into_iter()
                .map(|p| {
                    let (name, icon) = self.label(p);
                    (p.pid, name, icon, self.cells(p))
                })
                .collect();
            let mut sort = self.sort[self.tab];
            let mut select = None;
            let mut open_info = None;
            ui.at(x, top, w, |ui| {
                if let Some(c) = ui.list_header(&tr!("Process Name"), &col_refs, sort) {
                    sort = if sort.0 == c { (c, !sort.1) } else { (c, c == 0) };
                }
                ui.scroll_area(table_h - 26.0, |ui| {
                    for (pid, name, icon, cells) in &rows {
                        let cols: Vec<(&str, f32)> = cells.iter().zip(&col_refs).map(|(c, (_, w))| (c.as_str(), *w)).collect();
                        let r = ui.list_row(icon, name, &cols, self.selected == Some(*pid));
                        if r.clicked() {
                            select = Some(*pid);
                        }
                        if r.double_clicked() {
                            open_info = Some(*pid);
                        }
                    }
                });
            });
            self.sort[self.tab] = sort;
            if let Some(pid) = select {
                self.selected = Some(pid);
            }
            if open_info.is_some() {
                self.info = open_info;
            }
        }
        // Painel de baixo.
        let panel = (x, top + table_h + 4.0, w, PANEL_H);
        let t = ui.theme().clone();
        ui.paint(|canvas, _| {
            mide::ui::draw::fill_rounded(canvas, panel.0, panel.1, panel.2, panel.3, 12.0, tiny_skia::Color::from_rgba8(t.control[0], t.control[1], t.control[2], t.control[3]));
        });
        self.panel(ui, panel);
        self.info_view(ui);
        if let Some((_, name)) = self.confirm.clone() {
            let title = tr!("Are you sure you want to quit this process?").into_owned();
            let message = tr!("“{name}” will be closed. Unsaved changes may be lost.", name = name);
            match ui.alert(&title, &message, &[&tr!("Force Quit"), &tr!("Cancel"), &mide::i18n::trc("process", "Quit")]) {
                Some(0) => self.quit_selected(true),
                Some(2) => self.quit_selected(false),
                Some(_) => self.confirm = None,
                None => {}
            }
        }
        if let Some(message) = self.message.clone()
            && ui.alert(&message, "", &[&tr!("OK")]).is_some()
        {
            self.message = None;
        }
    }

    fn menu_bar(&mut self) -> Vec<MenuGroup> {
        let check = |on: bool, item: MenuItem| item.checked(on);
        vec![
            MenuGroup {
                title: tr!("File").into_owned(),
                items: vec![MenuItem::action("quit", &tr!("Quit Resource Monitor")).shortcut("Ctrl+Q")],
            },
            MenuGroup {
                title: tr!("View").into_owned(),
                items: vec![
                    check(self.filter == Filter::All, MenuItem::action("all", &tr!("All Processes"))),
                    check(self.filter == Filter::Mine, MenuItem::action("mine", &tr!("My Processes"))),
                    check(self.filter == Filter::Active, MenuItem::action("active", &tr!("Active Processes"))),
                    MenuItem::Separator,
                    MenuItem::action("tab-0", &tr!("CPU")).shortcut("Ctrl+1"),
                    MenuItem::action("tab-1", &tr!("Memory")).shortcut("Ctrl+2"),
                    MenuItem::action("tab-2", &tr!("Energy")).shortcut("Ctrl+3"),
                    MenuItem::action("tab-3", &tr!("Disk")).shortcut("Ctrl+4"),
                    MenuItem::action("tab-4", &tr!("Network")).shortcut("Ctrl+5"),
                    MenuItem::Separator,
                    check(self.interval.load(Ordering::Relaxed) == 1000, MenuItem::action("every-1", &tr!("Update Every Second"))),
                    check(self.interval.load(Ordering::Relaxed) == 2000, MenuItem::action("every-2", &tr!("Update Every 2 Seconds"))),
                    check(self.interval.load(Ordering::Relaxed) == 5000, MenuItem::action("every-5", &tr!("Update Every 5 Seconds"))),
                    MenuItem::Separator,
                    MenuItem::action("quit-process", &tr!("Quit Process")),
                    MenuItem::action("info", &tr!("Get Info")).shortcut("Ctrl+I"),
                ],
            },
            window_menu(Vec::new()),
        ]
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _logging = mide::logging::init("mide-monitor", "mide_monitor=info,mide=info");
    mide::i18n::init();
    let window = Window::new(&tr!("Resource Monitor")).app_id("mide-monitor").size(980, 640).min_size(720, 480).full_size_content();
    run(window, Monitor::new())
}
