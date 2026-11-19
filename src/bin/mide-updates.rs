//! Atualização de Software (`mide-updates`): procura e instala as atualizações do
//! sistema, tudo pela interface, como a Atualização de Software do Mac. Por baixo, o
//! PackageKit (`mide::packages`) roda o apt e pede a senha de administrador pelo polkit
//! (a janela do mide); nada de terminal.
//!
//! `mide-updates --background` não abre janela: procura atualizações a cada 6 horas e
//! avisa nas notificações quando há (com "Verificar automaticamente" ligado).
//!
//! Fora do apt (Fedora, Arch, Slackware, o PC de desenvolvimento), o próprio mide também
//! aparece aqui: a versão nova publicada, o que mudou e o botão de atualizar, que compila
//! a versão nova numa cópia própria do projeto e instala com a senha numa janela
//! (`mide::selfupdate`).

use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::{Duration, SystemTime};

use mide::kit::{run, window_menu, App, MenuGroup, MenuItem, Ui, Window};
use mide::packages::{self, Info, Package, Progress, Stage, UpdateDetail};
use mide::selfupdate::{self, Install, Step, Version};
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
    MideChecked(Result<Option<(Version, Vec<String>)>, String>),
    MideStep(Step),
    MideDone(Result<Version, String>),
}

/// O mide compilado nesta máquina (fora do apt): versão nova e atualização.
enum Mide {
    /// Pelo apt (o PackageKit cuida) ou rodando da pasta do projeto: nada aqui.
    NotApplicable,
    Checking,
    UpToDate,
    Available { latest: Version, changes: Vec<String> },
    Updating { to: Version, step: Step },
    Done(Version),
    Failed(String),
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
    install: Install,
    mide: Mide,
    show_changes: bool,
}

impl Updates {
    fn new() -> Self {
        let (tx, rx) = channel();
        let install = selfupdate::detect();
        // No log, para saber depois por que o mide aparece (ou não) aqui.
        tracing::info!("mide {}: instalação {install:?}", selfupdate::current());
        let mide = if install.from_source() { Mide::Checking } else { Mide::NotApplicable };
        Self { state: State::Checking, updates: Vec::new(), details: Vec::new(), show_details: false, checked_at: None, tx, rx, waker: None, started: false, install, mide, show_changes: false }
    }

    /// Procura a versão nova do mide (só fora do apt).
    fn check_mide(&mut self) {
        if !self.install.from_source() || matches!(self.mide, Mide::Updating { .. }) {
            return;
        }
        self.mide = Mide::Checking;
        let (tx, waker) = (self.tx.clone(), self.waker.clone());
        std::thread::spawn(move || {
            let _ = tx.send(Msg::MideChecked(find_mide_update()));
            if let Some(w) = waker {
                w.wake();
            }
        });
    }

    /// Compila e instala a versão nova do mide (pede a senha no fim).
    fn update_mide(&mut self, to: Version) {
        self.mide = Mide::Updating { to, step: Step::Downloading };
        let install = self.install.clone();
        let (tx, waker) = (self.tx.clone(), self.waker.clone());
        std::thread::spawn(move || {
            let step_tx = tx.clone();
            let step_waker = waker.clone();
            let result = selfupdate::update(&install, to, move |step| {
                let _ = step_tx.send(Msg::MideStep(step));
                if let Some(w) = &step_waker {
                    w.wake();
                }
            });
            let _ = tx.send(Msg::MideDone(result.map(|_| to)));
            if let Some(w) = waker {
                w.wake();
            }
        });
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
                Msg::MideChecked(Ok(Some((latest, changes)))) => {
                    tracing::info!("mide: versão nova {latest} (instalada: {})", selfupdate::current());
                    self.mide = Mide::Available { latest, changes };
                }
                Msg::MideChecked(Ok(None)) => {
                    tracing::info!("mide: {} é a versão mais nova", selfupdate::current());
                    self.mide = Mide::UpToDate;
                }
                Msg::MideChecked(Err(err)) => {
                    tracing::warn!("mide: não foi possível ver a versão nova: {err}");
                    self.mide = Mide::Failed(tr!("Couldn’t check for a new mide version: {err}", err = err));
                }
                Msg::MideStep(step) => {
                    if let Mide::Updating { step: cur, .. } = &mut self.mide {
                        *cur = step;
                    }
                }
                Msg::MideDone(Ok(to)) => {
                    tracing::info!("mide: {to} instalado");
                    self.mide = Mide::Done(to);
                }
                Msg::MideDone(Err(err)) => {
                    tracing::warn!("mide: a atualização falhou: {err}");
                    self.mide = Mide::Failed(err);
                }
            }
        }
    }

    fn header(&self, ui: &mut Ui) {
        // A versão nova do mide (fora do apt) conta como mais uma atualização.
        let total = self.updates.len() + usize::from(matches!(self.mide, Mide::Available { .. }));
        let count = || mide::kit::i18n::trn("{n} update available", "{n} updates available", total as u64).replace("{n}", &total.to_string());
        let (title, detail): (String, String) = match &self.state {
            State::Checking => (tr!("Checking for updates…").into_owned(), String::new()),
            State::UpToDate if total > 0 => (count(), last_check(self.checked_at)),
            State::UpToDate => (tr!("Your system is up to date").into_owned(), last_check(self.checked_at)),
            State::Available => (count(), last_check(self.checked_at)),
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

impl Updates {
    /// Cartão do mide compilado nesta máquina: versão nova, mudanças e o botão.
    fn mide_card(&mut self, ui: &mut Ui) {
        let current = selfupdate::current();
        match &self.mide {
            Mide::NotApplicable => {}
            Mide::Checking | Mide::UpToDate => {
                // Como o "Seu Mac está atualizado": a versão do mide sempre à vista, no
                // mesmo lugar do cartão da versão nova (sem ele, parecia que o app nem
                // olhava o mide).
                let note = if matches!(self.mide, Mide::Checking) {
                    tr!("Checking for a new version of mide…")
                } else {
                    tr!("mide is up to date. New versions appear here when they’re published.")
                };
                ui.card(|ui| {
                    ui.strong(&tr!("mide {version}", version = current));
                    ui.note(&note);
                });
            }
            Mide::Available { latest, changes } => {
                let (latest, changes) = (*latest, changes.clone());
                let mut update = false;
                ui.card(|ui| {
                    ui.strong(&tr!("mide {version}", version = latest));
                    ui.note(&tr!("You have version {version}.", version = current));
                    let shown = if self.show_changes { changes.len() } else { changes.len().min(6) };
                    for c in &changes[..shown] {
                        ui.note(&format!("• {c}"));
                    }
                    let more = changes.len() - shown;
                    if more > 0 && ui.button_styled(&mide::kit::i18n::trn("+{n} other change", "+{n} other changes", more as u64).replace("{n}", &more.to_string()), mide::kit::ButtonStyle::Plain).clicked() {
                        self.show_changes = true;
                    }
                    ui.note(&tr!("The new version is built on this computer, in the background (it can take 15 to 40 minutes). The administrator password is asked in a window."));
                    if ui.primary_button(&tr!("Update mide")).clicked() {
                        update = true;
                    }
                });
                if update {
                    self.update_mide(latest);
                }
            }
            Mide::Updating { to, step } => {
                let (to, step) = (*to, step.clone());
                ui.card(|ui| {
                    ui.strong(&tr!("Updating mide to {version}…", version = to));
                    let (value, text) = match step {
                        Step::Downloading => (None, tr!("Downloading…").into_owned()),
                        Step::Building { done, total } => (Some(done as f32 / total.max(1) as f32), tr!("Building ({done} of about {total} parts)…", done = done, total = total)),
                        Step::Installing => (None, tr!("Installing… (enter the administrator password in the window)").into_owned()),
                    };
                    ui.progress_bar(value);
                    ui.note(&text);
                    ui.note(&tr!("You can keep using the computer; the build runs at low priority."));
                });
            }
            Mide::Done(to) => {
                let to = *to;
                let mut restart = false;
                ui.card(|ui| {
                    ui.strong(&tr!("mide {version} installed", version = to));
                    ui.note(&tr!("It takes effect the next time you log in, or now by restarting the session (open app windows close)."));
                    if ui.primary_button(&tr!("Restart Session Now")).clicked() {
                        restart = true;
                    }
                });
                if restart {
                    let _ = std::process::Command::new("mide-ctl").args(["restart", "all"]).spawn();
                }
            }
            Mide::Failed(msg) => {
                let msg = msg.clone();
                let mut again = false;
                ui.card(|ui| {
                    ui.strong(&tr!("Couldn’t update mide"));
                    // O fim da saída da compilação ou da instalação (o motivo costuma estar lá).
                    let tail: Vec<&str> = msg.lines().rev().take(6).collect::<Vec<_>>().into_iter().rev().collect();
                    ui.note(&tail.join("\n"));
                    if ui.button(&tr!("Try Again")).clicked() {
                        again = true;
                    }
                });
                if again {
                    self.check_mide();
                }
            }
        }
    }
}

/// Versão nova do mide publicada, com o que mudou desde a instalada (ou `None`).
fn find_mide_update() -> Result<Option<(Version, Vec<String>)>, String> {
    let current = selfupdate::current();
    let repo = selfupdate::repo();
    match selfupdate::latest(&repo)? {
        Some(latest) if latest > current => {
            // A cópia do projeto já fica pronta para a atualização (só o histórico).
            let dir = selfupdate::source_dir();
            let changes = match selfupdate::fetch(&dir, &repo) {
                Ok(()) => selfupdate::changes(&dir, current, latest),
                Err(err) => {
                    tracing::warn!("mide: não deu para baixar o histórico: {err}");
                    Vec::new()
                }
            };
            Ok(Some((latest, changes)))
        }
        _ => Ok(None),
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
            self.check_mide();
        }
        self.receive();
        for action in ui.input().menu.clone() {
            match action.as_str() {
                "check" => {
                    self.check();
                    self.check_mide();
                }
                "quit" => ui.close(),
                _ => {}
            }
        }
        ui.toolbar(|ui| ui.toolbar_title(&tr!("Software Update"), 300.0));
        let busy = matches!(self.state, State::Checking | State::Installing(_));
        ui.page(|ui| {
            self.header(ui);
            self.mide_card(ui);
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
                            self.check_mide();
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
    let from_source = selfupdate::detect().from_source();
    let mut told: Option<Version> = None;
    loop {
        // Fora do apt, a versão nova do mide (um aviso por versão).
        if auto_check() && from_source {
            match find_mide_update() {
                Ok(Some((latest, _))) if told != Some(latest) => {
                    told = Some(latest);
                    tracing::info!("mide: aviso da versão {latest}");
                    mide::notify::send_and_wait("mide-updates", &tr!("Software Update"), &tr!("mide {version} is available", version = latest));
                }
                Ok(_) => {}
                Err(err) => tracing::warn!("mide: verificação de fundo falhou: {err}"),
            }
        }
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
