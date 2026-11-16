//! Atualização de Software (`mide-updates`): procura e instala as atualizações do
//! sistema, tudo pela interface, como a Atualização de Software do Mac. Por baixo, o
//! PackageKit (`mide::packages`) roda o apt e pede a senha de administrador pelo polkit
//! (a janela do mide); nada de terminal.
//!
//! `mide-updates --background` não abre janela: procura atualizações a cada 6 horas e
//! avisa nas notificações quando há (com "Verificar automaticamente" ligado).

use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::{Duration, SystemTime};

use mide::kit::{run, window_menu, App, MenuGroup, MenuItem, Ui, Window};
use mide::packages::{self, Info, Package, Progress, Stage, UpdateDetail};
use mide::settings_file::{self, Value};
use mide::tr;

/// Uma atualização na lista: o pacote novo e a versão instalada.
#[derive(Clone)]
struct Update {
    package: Package,
    installed: String,
}

/// O que a thread de trabalho manda para a janela.
enum Msg {
    Checked(Result<Vec<Update>, packages::Error>),
    Details(Vec<UpdateDetail>),
    Progress(Progress),
    Installed(Result<bool, packages::Error>),
}

/// Estado da tela.
enum State {
    Checking,
    UpToDate,
    Available,
    Installing(Option<Progress>),
    Done { restart: bool },
    Failed(String),
}

struct Updates {
    state: State,
    updates: Vec<Update>,
    details: Vec<UpdateDetail>,
    show_details: bool,
    checked_at: Option<SystemTime>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    waker: Option<mide::kit::Waker>,
    started: bool,
}

impl Updates {
    fn new() -> Self {
        let (tx, rx) = channel();
        Self { state: State::Checking, updates: Vec::new(), details: Vec::new(), show_details: false, checked_at: None, tx, rx, waker: None, started: false }
    }

    /// Procura atualizações numa thread (atualiza a lista do apt antes).
    fn check(&mut self) {
        self.state = State::Checking;
        self.details.clear();
        let (tx, waker) = (self.tx.clone(), self.waker.clone());
        std::thread::spawn(move || {
            let result = find_updates();
            let _ = tx.send(Msg::Checked(result));
            if let Some(w) = waker {
                w.wake();
            }
        });
    }

    /// Notas e changelog das atualizações (para "Mais informações").
    fn load_details(&self) {
        let ids: Vec<String> = self.updates.iter().map(|u| u.package.id.clone()).collect();
        let (tx, waker) = (self.tx.clone(), self.waker.clone());
        std::thread::spawn(move || {
            let details = packages::update_details(&ids).unwrap_or_default();
            let _ = tx.send(Msg::Details(details));
            if let Some(w) = waker {
                w.wake();
            }
        });
    }

    /// Instala todas as atualizações (o polkit pede a senha).
    fn install(&mut self) {
        self.state = State::Installing(None);
        let ids: Vec<String> = self.updates.iter().map(|u| u.package.id.clone()).collect();
        // Reiniciar depois: núcleo, firmware, systemd ou a biblioteca do C.
        let restart = self.updates.iter().any(|u| needs_restart(u.package.name()));
        let (tx, waker) = (self.tx.clone(), self.waker.clone());
        std::thread::spawn(move || {
            let progress_tx = tx.clone();
            let progress_waker = waker.clone();
            let result = packages::update(&ids, move |p| {
                let _ = progress_tx.send(Msg::Progress(p));
                if let Some(w) = &progress_waker {
                    w.wake();
                }
            });
            let _ = tx.send(Msg::Installed(result.map(|_| restart)));
            if let Some(w) = waker {
                w.wake();
            }
        });
    }

    fn receive(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Checked(Ok(list)) => {
                    self.checked_at = Some(SystemTime::now());
                    tracing::info!("atualizações: {} disponível(is)", list.len());
                    self.state = if list.is_empty() { State::UpToDate } else { State::Available };
                    self.updates = list;
                }
                Msg::Checked(Err(err)) => {
                    tracing::warn!("atualizações: não foi possível verificar: {err}");
                    self.state = State::Failed(error_text(&err));
                }
                Msg::Details(d) => self.details = d,
                Msg::Progress(p) => {
                    if let State::Installing(cur) = &mut self.state {
                        *cur = Some(p);
                    }
                }
                Msg::Installed(Ok(restart)) => {
                    tracing::info!("atualizações instaladas (reiniciar: {restart})");
                    self.updates.clear();
                    self.state = State::Done { restart };
                }
                Msg::Installed(Err(packages::Error::NotAuthorized | packages::Error::Cancelled)) => {
                    // Senha não dada: volta à lista, sem erro.
                    self.state = State::Available;
                }
                Msg::Installed(Err(err)) => {
                    tracing::warn!("atualizações: falhou: {err}");
                    self.state = State::Failed(error_text(&err));
                }
            }
        }
    }

    fn header(&self, ui: &mut Ui) {
        let (title, detail): (String, String) = match &self.state {
            State::Checking => (tr!("Checking for updates…").into_owned(), String::new()),
            State::UpToDate => (tr!("Your system is up to date").into_owned(), last_check(self.checked_at)),
            State::Available => (
                mide::kit::i18n::trn("{n} update available", "{n} updates available", self.updates.len() as u64).replace("{n}", &self.updates.len().to_string()),
                last_check(self.checked_at),
            ),
            State::Installing(_) => (tr!("Installing updates…").into_owned(), tr!("Keep the computer on until it finishes.").into_owned()),
            State::Done { restart: true } => (tr!("Updates installed").into_owned(), tr!("Restart the computer to finish.").into_owned()),
            State::Done { restart: false } => (tr!("Updates installed").into_owned(), tr!("Everything is up to date.").into_owned()),
            State::Failed(msg) => (tr!("Couldn’t update").into_owned(), msg.clone()),
        };
        ui.card(|ui| {
            ui.columns(
                64.0,
                12.0,
                |ui| ui.icon("system-software-update", 56),
                |ui| {
                    ui.strong(&title);
                    if !detail.is_empty() {
                        ui.note(&detail);
                    }
                    ui.note(&system_name());
                },
            );
        });
    }
}

/// Nome do sistema (o do `os-release`: "mios 1.0").
fn system_name() -> String {
    let text = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
    text.lines()
        .find_map(|l| l.strip_prefix("PRETTY_NAME="))
        .map(|v| v.trim_matches('"').to_string())
        .unwrap_or_else(|| "Linux".into())
}

fn last_check(at: Option<SystemTime>) -> String {
    match at {
        Some(t) => tr!("Last checked: {when}", when = mide::files::date_text(t, SystemTime::now())),
        None => String::new(),
    }
}

fn error_text(err: &packages::Error) -> String {
    match err {
        packages::Error::Unavailable(_) => tr!("The update service (PackageKit) isn’t available.").into_owned(),
        packages::Error::NotAuthorized => tr!("The administrator password is needed to update.").into_owned(),
        packages::Error::Cancelled => tr!("Cancelled.").into_owned(),
        packages::Error::Failed(msg) => msg.clone(),
    }
}

/// Pacotes que pedem reiniciar o computador para valer.
fn needs_restart(name: &str) -> bool {
    name.starts_with("linux-image") || name.starts_with("firmware-") || name.starts_with("intel-microcode") || name.starts_with("amd64-microcode") || name == "systemd" || name == "libc6" || name == "mide" || name == "dbus"
}

/// Atualiza a lista do apt e devolve as atualizações com a versão instalada.
fn find_updates() -> Result<Vec<Update>, packages::Error> {
    packages::refresh(false, |_| {})?;
    let list = packages::updates()?;
    let names: Vec<String> = list.iter().map(|p| p.name().to_string()).collect();
    let installed = packages::installed_versions(&names).unwrap_or_default();
    let mut updates: Vec<Update> = list.into_iter().map(|p| Update { installed: installed.get(p.name()).cloned().unwrap_or_default(), package: p }).collect();
    // Segurança primeiro, depois por nome.
    updates.sort_by(|a, b| (a.package.info != Info::Security, a.package.name()).cmp(&(b.package.info != Info::Security, b.package.name())));
    Ok(updates)
}

fn stage_text(stage: Stage) -> std::borrow::Cow<'static, str> {
    match stage {
        Stage::Waiting => tr!("Waiting…"),
        Stage::Authenticating => tr!("Waiting for the password…"),
        Stage::Refreshing => tr!("Checking…"),
        Stage::Downloading => tr!("Downloading…"),
        Stage::Installing | Stage::Updating => tr!("Installing…"),
        Stage::Removing => tr!("Removing…"),
        Stage::Finishing => tr!("Finishing…"),
        Stage::Working => tr!("Working…"),
    }
}

/// Liga e desliga a verificação automática (`[updates] auto`, ligada por padrão).
fn auto_check() -> bool {
    settings_file::flag("updates", "auto", true)
}

impl App for Updates {
    fn view(&mut self, ui: &mut Ui) {
        if !self.started {
            self.started = true;
            self.waker = ui.waker();
            self.check();
        }
        self.receive();
        for action in ui.input().menu.clone() {
            match action.as_str() {
                "check" => self.check(),
                "quit" => ui.close(),
                _ => {}
            }
        }
        ui.toolbar(|ui| ui.toolbar_title(&tr!("Software Update"), 300.0));
        let busy = matches!(self.state, State::Checking | State::Installing(_));
        ui.page(|ui| {
            self.header(ui);
            match &self.state {
                State::Checking => ui.progress_bar(None),
                State::Installing(p) => {
                    let value = p.and_then(|p| p.percent).map(|v| v as f32 / 100.0);
                    ui.progress_bar(value);
                    ui.note(&p.map(|p| stage_text(p.stage)).unwrap_or_else(|| tr!("Waiting for the password…")));
                }
                _ => {}
            }
            if !self.updates.is_empty() {
                let updates = self.updates.clone();
                // Como no Mac: as primeiras na tela e o resto em "Mais informações".
                let shown = if self.show_details { updates.len() } else { updates.len().min(8) };
                ui.card(|ui| {
                    for u in &updates[..shown] {
                        let version = if u.installed.is_empty() { u.package.version().to_string() } else { format!("{} → {}", u.installed, u.package.version()) };
                        ui.key_value(u.package.name(), &version, 260.0);
                        if u.package.info == Info::Security {
                            ui.badge(&tr!("Security"));
                        }
                    }
                    if shown < updates.len() {
                        ui.note(&mide::kit::i18n::trn("+{n} other", "+{n} others", (updates.len() - shown) as u64).replace("{n}", &(updates.len() - shown).to_string()));
                    }
                });
            }
            if self.show_details && !self.details.is_empty() {
                let details = self.details.clone();
                ui.card(|ui| {
                    for d in &details {
                        let name = d.id.split(';').next().unwrap_or_default();
                        ui.strong(name);
                        let text = if !d.update_text.trim().is_empty() { &d.update_text } else { &d.changelog };
                        // Só o começo de cada changelog (os do Debian são longos).
                        let short: String = text.lines().filter(|l| !l.trim().is_empty()).take(6).collect::<Vec<_>>().join("\n");
                        ui.note(if short.is_empty() { "—" } else { &short });
                    }
                });
            }
            ui.row(|ui| {
                match self.state {
                    State::Available => {
                        if ui.button(&tr!("More Info…")).clicked() {
                            self.show_details = !self.show_details;
                            if self.show_details && self.details.is_empty() {
                                self.load_details();
                            }
                        }
                        if ui.primary_button(&tr!("Update Now")).clicked() {
                            self.install();
                        }
                    }
                    State::Done { restart: true } => {
                        if ui.primary_button(&tr!("Restart Now")).clicked() {
                            mide::system::power(mide::system::Power::Restart);
                        }
                    }
                    _ => {
                        if !busy && ui.button(&tr!("Check Now")).clicked() {
                            self.check();
                        }
                    }
                }
            });
            ui.card(|ui| {
                let mut on = auto_check();
                if ui.switch_detail(&tr!("Check for updates automatically"), &tr!("A notification appears when updates are available"), &mut on).changed() {
                    let _ = settings_file::set("updates", "auto", (!on).then_some(Value::Bool(false)));
                }
            });
        });
    }

    fn menu_bar(&mut self) -> Vec<MenuGroup> {
        vec![
            MenuGroup {
                title: tr!("File").into_owned(),
                items: vec![
                    MenuItem::action("check", &tr!("Check Now")).shortcut("Ctrl+R"),
                    MenuItem::Separator,
                    MenuItem::action("quit", &tr!("Quit Software Update")).shortcut("Ctrl+Q"),
                ],
            },
            window_menu(Vec::new()),
        ]
    }
}

/// Modo de fundo: procura atualizações a cada 6 horas e avisa nas notificações.
fn background() {
    let mut last_count = 0;
    loop {
        if auto_check() {
            match find_updates() {
                Ok(list) if !list.is_empty() && list.len() != last_count => {
                    last_count = list.len();
                    let title = tr!("Software Update");
                    let body = mide::kit::i18n::trn("{n} update available", "{n} updates available", list.len() as u64).replace("{n}", &list.len().to_string());
                    tracing::info!("atualizações: aviso de {} disponível(is)", list.len());
                    mide::notify::send_and_wait("mide-updates", &title, &body);
                }
                Ok(list) => last_count = list.len(),
                Err(err) => tracing::warn!("atualizações: verificação de fundo falhou: {err}"),
            }
        }
        std::thread::sleep(Duration::from_secs(6 * 3600));
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _logging = mide::logging::init("mide-updates", "mide_updates=info,mide=info");
    mide::i18n::init();
    if std::env::args().any(|a| a == "--background") {
        // Espera a sessão se acomodar antes da primeira busca.
        std::thread::sleep(Duration::from_secs(120));
        background();
        return Ok(());
    }
    let window = Window::new(&tr!("Software Update")).app_id("mide-updates").size(620, 560).min_size(480, 420).full_size_content();
    run(window, Updates::new())
}
