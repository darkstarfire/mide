//! Instalador do mios (`mios-installer`), feito do zero com o mide-ui, no jeito do
//! instalador do Mac: idioma, teclado e fuso, o disco, o usuário, o resumo e a cópia.
//! A instalação em si roda como root na parte `--helper` (`helper.rs`), chamada pelo
//! `sudo` do usuário do modo live (ou pelo `pkexec`), que manda o andamento por linhas.

mod helper;

use std::io::{BufRead, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc::{channel, Receiver, Sender};

use helper::Plan;
use mide::disks::Disk;
use mide::kit::{run, App, Ui, Window};
use mide::processes::bytes_text;
use mide::tr;

const LANGUAGES: &[(&str, &str)] = &[("en_US.UTF-8", "English"), ("pt_BR.UTF-8", "Português (Brasil)")];
const KEYBOARDS: &[(&str, &str)] = &[("us", "English (US)"), ("us:intl", "English (US, international)"), ("br", "Português (Brasil, ABNT2)"), ("pt", "Português (Portugal)"), ("es", "Español"), ("gb", "English (UK)"), ("de", "Deutsch"), ("fr", "Français")];

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd)]
enum Step {
    Welcome,
    Region,
    Disk,
    User,
    Summary,
    Installing,
    Finished,
}

enum Msg {
    Disks(Vec<Disk>),
    Progress(u32, String),
    Done(Result<(), String>),
}

struct Installer {
    step: Step,
    language: usize,
    keyboard: usize,
    timezones: Vec<String>,
    timezone: usize,
    disks: Vec<Disk>,
    disk: Option<usize>,
    fullname: String,
    username: String,
    password: String,
    confirm: String,
    hostname: String,
    autologin: bool,
    progress: (u32, String),
    error: Option<String>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    waker: Option<mide::kit::Waker>,
    started: bool,
}

/// Fusos horários do sistema (`zone1970.tab`), em ordem.
fn timezones() -> Vec<String> {
    let text = std::fs::read_to_string("/usr/share/zoneinfo/zone1970.tab").unwrap_or_default();
    let mut zones: Vec<String> = text.lines().filter(|l| !l.starts_with('#')).filter_map(|l| l.split('\t').nth(2).map(str::to_string)).collect();
    zones.sort();
    zones.dedup();
    if zones.is_empty() {
        zones = vec!["UTC".into(), "America/Sao_Paulo".into()];
    }
    zones
}

/// Usuário sugerido pelo nome completo: o primeiro nome, sem acentos, em minúsculas.
fn suggest_username(fullname: &str) -> String {
    let first = fullname.split_whitespace().next().unwrap_or_default();
    mide::apps::fold(first).chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_lowercase()
}

impl Installer {
    fn new() -> Self {
        let (tx, rx) = channel();
        let zones = timezones();
        let lang = std::env::var("LANG").unwrap_or_default();
        let language = if lang.starts_with("pt") { 1 } else { 0 };
        let tz_now = std::fs::read_to_string("/etc/timezone").unwrap_or_default().trim().to_string();
        let default_tz = if tz_now.is_empty() || tz_now == "Etc/UTC" { if language == 1 { "America/Sao_Paulo".to_string() } else { "UTC".to_string() } } else { tz_now };
        let timezone = zones.iter().position(|z| *z == default_tz).unwrap_or(0);
        Self {
            step: Step::Welcome,
            language,
            keyboard: if language == 1 { 2 } else { 0 },
            timezones: zones,
            timezone,
            disks: Vec::new(),
            disk: None,
            fullname: String::new(),
            username: String::new(),
            password: String::new(),
            confirm: String::new(),
            hostname: "mios".into(),
            autologin: false,
            progress: (0, String::new()),
            error: None,
            tx,
            rx,
            waker: None,
            started: false,
        }
    }

    fn send(&self) -> impl Fn(Msg) + Send + Clone + 'static {
        let (tx, waker) = (self.tx.clone(), self.waker.clone());
        move |m| {
            let _ = tx.send(m);
            if let Some(w) = &waker {
                w.wake();
            }
        }
    }

    fn load_disks(&self) {
        let send = self.send();
        std::thread::spawn(move || {
            let live = helper::live_medium_disk().map(|p| p.to_string_lossy().into_owned());
            let list: Vec<Disk> = mide::disks::disks().unwrap_or_default().into_iter().filter(|d| Some(&d.device) != live.as_ref() && d.size >= 16 * 1024 * 1024 * 1024 && !d.optical).collect();
            send(Msg::Disks(list));
        });
    }

    fn plan(&self) -> Plan {
        Plan {
            disk: self.disk.and_then(|i| self.disks.get(i)).map(|d| d.device.clone()).unwrap_or_default(),
            locale: LANGUAGES[self.language].0.into(),
            keyboard: KEYBOARDS[self.keyboard].0.split(':').next().unwrap_or("us").into(),
            timezone: self.timezones.get(self.timezone).cloned().unwrap_or_default(),
            fullname: self.fullname.trim().into(),
            username: self.username.trim().into(),
            password: self.password.clone(),
            hostname: self.hostname.trim().into(),
            autologin: self.autologin,
        }
    }

    /// Começa a instalação: a parte `--helper` como root, com o plano na entrada.
    fn install(&mut self) {
        self.step = Step::Installing;
        self.error = None;
        let plan = serde_json::to_string(&self.plan()).unwrap_or_default();
        let send = self.send();
        std::thread::spawn(move || {
            let exe = std::env::current_exe().unwrap_or_else(|_| "mios-installer".into());
            // O usuário do modo live usa o sudo sem senha; fora dele, o pkexec pede a senha.
            let sudo_ok = Command::new("sudo").args(["-n", "true"]).status().is_ok_and(|s| s.success());
            let mut cmd = if sudo_ok {
                let mut c = Command::new("sudo");
                c.arg("-n").arg(&exe);
                c
            } else {
                let mut c = Command::new("pkexec");
                c.arg(&exe);
                c
            };
            let child = cmd.arg("--helper").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn();
            let mut child = match child {
                Ok(c) => c,
                Err(e) => {
                    send(Msg::Done(Err(e.to_string())));
                    return;
                }
            };
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(plan.as_bytes());
            }
            let mut result = Err(tr!("The installation stopped without an answer.").into_owned());
            if let Some(out) = child.stdout.take() {
                for line in std::io::BufReader::new(out).lines().map_while(Result::ok) {
                    if let Some(rest) = line.strip_prefix("PROGRESS ") {
                        let (n, stage) = rest.split_once(' ').unwrap_or((rest, ""));
                        send(Msg::Progress(n.parse().unwrap_or(0), stage.to_string()));
                    } else if let Some(err) = line.strip_prefix("ERROR ") {
                        result = Err(err.to_string());
                    } else if line == "DONE" {
                        result = Ok(());
                    }
                }
            }
            let _ = child.wait();
            send(Msg::Done(result));
        });
    }

    fn receive(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Disks(d) => {
                    if d.len() == 1 {
                        self.disk = Some(0);
                    }
                    self.disks = d;
                }
                Msg::Progress(n, stage) => self.progress = (n, stage),
                Msg::Done(Ok(())) => {
                    tracing::info!("instalador: instalado");
                    self.step = Step::Finished;
                }
                Msg::Done(Err(e)) => {
                    tracing::warn!("instalador: falhou: {e}");
                    self.error = Some(e);
                    self.step = Step::Summary;
                }
            }
        }
    }

    fn stage_text(stage: &str) -> std::borrow::Cow<'static, str> {
        match stage {
            "partitioning" => tr!("Preparing the disk…"),
            "formatting" => tr!("Formatting…"),
            "mounting" | "copying" => tr!("Copying mios to the disk…"),
            "configuring" => tr!("Configuring…"),
            "user" => tr!("Creating your account…"),
            "login" | "cleaning" => tr!("Finishing the setup…"),
            "bootloader" | "initramfs" => tr!("Making the disk start the computer…"),
            _ => tr!("Finishing…"),
        }
    }

    fn page(&mut self, ui: &mut Ui) {
        match self.step {
            Step::Welcome => {
                ui.title(&tr!("Welcome to mios"));
                ui.note(&tr!("mios is a Mac inspired operating system, with the mide desktop. Choose your language."));
                let names: Vec<String> = LANGUAGES.iter().map(|l| l.1.to_string()).collect();
                let refs: Vec<&str> = names.iter().map(String::as_str).collect();
                let before = self.language;
                ui.segmented(&refs, &mut self.language, None);
                if self.language != before {
                    mide::kit::i18n::set_language(if self.language == 1 { "pt_BR" } else { "en" });
                    self.keyboard = if self.language == 1 { 2 } else { 0 };
                }
            }
            Step::Region => {
                ui.title(&tr!("Keyboard and Region"));
                let names: Vec<String> = KEYBOARDS.iter().map(|k| k.1.to_string()).collect();
                ui.card(|ui| {
                    ui.dropdown(&tr!("Keyboard"), &names, &mut self.keyboard);
                    ui.dropdown(&tr!("Time Zone"), &self.timezones, &mut self.timezone);
                });
            }
            Step::Disk => {
                ui.title(&tr!("Choose the Disk"));
                ui.note(&tr!("Everything on the chosen disk will be erased."));
                if self.disks.is_empty() {
                    ui.note(&tr!("No disk of 16 GB or more found."));
                }
                let disks = self.disks.clone();
                ui.card(|ui| {
                    for (i, d) in disks.iter().enumerate() {
                        let detail = format!("{} · {}", bytes_text(d.size as f64), d.device);
                        if ui.list_row("drive-harddisk", &d.name(), &[(detail.as_str(), 220.0)], self.disk == Some(i)).clicked() {
                            self.disk = Some(i);
                        }
                    }
                });
            }
            Step::User => {
                ui.title(&tr!("Create Your Account"));
                ui.card(|ui| {
                    let before = suggest_username(&self.fullname);
                    if ui.text_input(&mut self.fullname, &tr!("Full name")).changed() && (self.username.is_empty() || self.username == before) {
                        self.username = suggest_username(&self.fullname);
                    }
                    ui.text_input(&mut self.username, &tr!("User name"));
                    ui.password_input(&mut self.password, &tr!("Password"));
                    ui.password_input(&mut self.confirm, &tr!("Confirm password"));
                    ui.text_input(&mut self.hostname, &tr!("Computer name"));
                    ui.switch(&tr!("Log in automatically"), &mut self.autologin);
                });
                if !self.username.is_empty() && !helper::valid_username(&self.username) {
                    ui.note(&tr!("The user name uses lowercase letters, numbers, - and _, starting with a letter."));
                }
                if !self.confirm.is_empty() && self.password != self.confirm {
                    ui.note(&tr!("The passwords are different."));
                }
            }
            Step::Summary => {
                ui.title(&tr!("Ready to Install"));
                let plan = self.plan();
                let disk = self.disk.and_then(|i| self.disks.get(i)).map(|d| format!("{} ({})", d.name(), d.device)).unwrap_or_default();
                ui.card(|ui| {
                    ui.key_value(&tr!("Language"), LANGUAGES[self.language].1, 160.0);
                    ui.key_value(&tr!("Keyboard"), KEYBOARDS[self.keyboard].1, 160.0);
                    ui.key_value(&tr!("Time Zone"), &plan.timezone, 160.0);
                    ui.key_value(&tr!("Disk"), &disk, 160.0);
                    ui.key_value(&tr!("Account"), &format!("{} ({})", plan.fullname, plan.username), 160.0);
                    ui.key_value(&tr!("Computer name"), &plan.hostname, 160.0);
                });
                ui.note(&tr!("Everything on the disk will be erased. This can’t be undone."));
                if let Some(e) = &self.error {
                    ui.card(|ui| {
                        ui.strong(&tr!("The installation failed"));
                        ui.note(e);
                    });
                }
            }
            Step::Installing => {
                ui.title(&tr!("Installing mios"));
                ui.progress_bar(Some(self.progress.0 as f32 / 100.0));
                ui.note(&Self::stage_text(&self.progress.1));
                ui.note(&tr!("Keep the computer on until it finishes."));
            }
            Step::Finished => {
                ui.title(&tr!("mios is installed"));
                ui.note(&tr!("Restart the computer and remove the installation drive."));
            }
        }
    }

    fn can_continue(&self) -> bool {
        match self.step {
            Step::Disk => self.disk.is_some(),
            Step::User => !self.fullname.trim().is_empty() && helper::valid_username(self.username.trim()) && !self.password.is_empty() && self.password == self.confirm,
            _ => true,
        }
    }
}

impl App for Installer {
    fn view(&mut self, ui: &mut Ui) {
        if !self.started {
            self.started = true;
            self.waker = ui.waker();
            self.load_disks();
        }
        self.receive();
        ui.toolbar(|ui| ui.toolbar_title(&tr!("Install mios"), 300.0));
        ui.page(|ui| {
            self.page(ui);
            ui.space(12.0);
            ui.row(|ui| {
                let back = match self.step {
                    Step::Region => Some(Step::Welcome),
                    Step::Disk => Some(Step::Region),
                    Step::User => Some(Step::Disk),
                    Step::Summary => Some(Step::User),
                    _ => None,
                };
                if let Some(b) = back
                    && ui.button(&tr!("Back")).clicked()
                {
                    self.step = b;
                }
                match self.step {
                    Step::Welcome => {
                        if ui.button(&tr!("Try mios")).clicked() {
                            ui.close();
                        }
                        if ui.primary_button(&tr!("Continue")).clicked() {
                            self.step = Step::Region;
                        }
                    }
                    Step::Summary => {
                        if ui.primary_button(&tr!("Install")).clicked() {
                            self.install();
                        }
                    }
                    Step::Installing => {}
                    Step::Finished => {
                        if ui.primary_button(&tr!("Restart Now")).clicked() {
                            mide::system::power(mide::system::Power::Restart);
                        }
                    }
                    _ => {
                        if self.can_continue() && ui.primary_button(&tr!("Continue")).clicked() {
                            self.step = match self.step {
                                Step::Region => Step::Disk,
                                Step::Disk => Step::User,
                                _ => Step::Summary,
                            };
                        }
                    }
                }
            });
        });
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--helper") {
        helper::main_helper();
    }
    if let Some(i) = args.iter().position(|a| a == "--auto") {
        helper::main_auto(args.get(i + 1).map(String::as_str).unwrap_or("/dev/vda"));
    }
    let _logging = mide::logging::init("mios-installer", "mios_installer=info,mide=info");
    mide::i18n::init();
    let window = Window::new(&tr!("Install mios")).app_id("mios-installer").size(760, 600).min_size(620, 520).full_size_content();
    run(window, Installer::new())
}
