//! Tela de bloqueio do mide, com o visual da tela de login (no estilo do macOS
//! Sonoma/Tahoe): a data e a hora grandes em cima, a foto e o nome de quem está usando,
//! o campo de senha de vidro (aviso de Caps Lock, botão →, a roda enquanto confere), a
//! mensagem da tela de bloqueio (`[login] message`), o Wi-Fi e a bateria no canto, sobre
//! o papel de parede desfocado. Usa o protocolo de bloqueio do Wayland (`ext-session-lock-v1`):
//! enquanto ela está aberta, o compositor não mostra nem deixa usar mais nada, e se
//! ela fechar sem a senha, a tela continua bloqueada.
//!
//! A senha é conferida pelo PAM do sistema (o mesmo do login), com o serviço
//! `mide-lock` se existir em `/etc/pam.d` (o instalador cria), ou o `login`.

use std::{
    error::Error,
    sync::mpsc,
    time::{Duration, Instant},
};

use mide::tr;
use mide::{
    kit::{run, App, LockField, Ui, Waker, Window},
    lockscreen::{self, Avatars, LoginOptions},
    ui::text::FontWeight,
};

/// Conferência da senha pelo PAM (interface C da libpam, sem crate).
mod pam {
    use std::ffi::{c_char, c_int, c_void, CString};

    const PAM_SUCCESS: c_int = 0;
    const PAM_PROMPT_ECHO_OFF: c_int = 1;
    const PAM_PROMPT_ECHO_ON: c_int = 2;
    const PAM_REFRESH_CRED: c_int = 0x0010;

    #[repr(C)]
    struct Message {
        style: c_int,
        msg: *const c_char,
    }

    #[repr(C)]
    struct Response {
        resp: *mut c_char,
        retcode: c_int,
    }

    #[repr(C)]
    struct Conversation {
        conv: extern "C" fn(c_int, *mut *const Message, *mut *mut Response, *mut c_void) -> c_int,
        appdata: *mut c_void,
    }

    // A biblioteca de execução (`libpam.so.0`): o pacote de desenvolvimento, com o
    // `libpam.so`, não precisa estar instalado.
    #[link(name = "libpam.so.0", kind = "dylib", modifiers = "+verbatim")]
    unsafe extern "C" {
        fn pam_start(service: *const c_char, user: *const c_char, conv: *const Conversation, handle: *mut *mut c_void) -> c_int;
        fn pam_authenticate(handle: *mut c_void, flags: c_int) -> c_int;
        fn pam_setcred(handle: *mut c_void, flags: c_int) -> c_int;
        fn pam_end(handle: *mut c_void, status: c_int) -> c_int;
    }

    unsafe extern "C" {
        fn calloc(count: usize, size: usize) -> *mut c_void;
        fn strdup(s: *const c_char) -> *mut c_char;
    }

    /// Responde às perguntas do PAM: a senha para as que não mostram o que se digita
    /// (e para as que mostram, como o usuário); avisos ficam sem resposta.
    extern "C" fn converse(count: c_int, messages: *mut *const Message, responses: *mut *mut Response, data: *mut c_void) -> c_int {
        if count <= 0 || messages.is_null() || responses.is_null() || data.is_null() {
            return 19; // PAM_CONV_ERR
        }
        // SAFETY: o PAM passa `count` mensagens válidas e espera `count` respostas
        // alocadas com malloc (ele libera); `data` é o CString da senha, vivo durante a
        // conversa.
        unsafe {
            let password = &*(data as *const CString);
            let list = calloc(count as usize, std::mem::size_of::<Response>()) as *mut Response;
            if list.is_null() {
                return 5; // PAM_BUF_ERR
            }
            for i in 0..count as usize {
                let message = &**messages.add(i);
                if message.style == PAM_PROMPT_ECHO_OFF || message.style == PAM_PROMPT_ECHO_ON {
                    (*list.add(i)).resp = strdup(password.as_ptr());
                }
            }
            *responses = list;
        }
        PAM_SUCCESS
    }

    /// A senha de `user` está certa?
    pub fn authenticate(service: &str, user: &str, password: &str) -> bool {
        let (Ok(service), Ok(user), Ok(password)) = (CString::new(service), CString::new(user), CString::new(password)) else {
            return false;
        };
        let conv = Conversation { conv: converse, appdata: &password as *const CString as *mut c_void };
        let mut handle: *mut c_void = std::ptr::null_mut();
        // SAFETY: as strings e a conversa vivem até o `pam_end`; o handle só é usado
        // depois de um `pam_start` que deu certo.
        unsafe {
            if pam_start(service.as_ptr(), user.as_ptr(), &conv, &mut handle) != PAM_SUCCESS {
                return false;
            }
            let status = pam_authenticate(handle, 0);
            if status == PAM_SUCCESS {
                // Renova as credenciais (tíquetes do Kerberos, chaveiros).
                pam_setcred(handle, PAM_REFRESH_CRED);
            }
            pam_end(handle, status);
            status == PAM_SUCCESS
        }
    }
}

/// Nome de login e nome completo (do `/etc/passwd`) de quem está usando.
fn user() -> (String, String) {
    let uid = rustix::process::getuid().as_raw();
    let entry = std::fs::read_to_string("/etc/passwd").ok().and_then(|passwd| {
        passwd.lines().find_map(|line| {
            let f: Vec<&str> = line.split(':').collect();
            (f.len() >= 7 && f[2].parse::<u32>().ok() == Some(uid)).then(|| (f[0].to_string(), f[4].split(',').next().unwrap_or("").to_string()))
        })
    });
    let login = entry.as_ref().map(|e| e.0.clone()).or_else(|| std::env::var("USER").ok()).unwrap_or_default();
    let full = entry.map(|e| e.1).filter(|n| !n.trim().is_empty()).unwrap_or_else(|| login.clone());
    (login, full)
}

fn pam_service() -> &'static str {
    if std::path::Path::new("/etc/pam.d/mide-lock").exists() { "mide-lock" } else { "login" }
}

/// Senha dos testes automáticos (`MIDE_LOCK_TEST_PASSWORD`): só nas compilações de
/// desenvolvimento; a versão de distribuição sempre usa o PAM.
fn test_password() -> Option<String> {
    if cfg!(debug_assertions) { std::env::var("MIDE_LOCK_TEST_PASSWORD").ok() } else { None }
}



struct Lock {
    login: String,
    name: String,
    password: String,
    /// Conferindo a senha (o PAM pode levar uns segundos quando ela está errada).
    checking: Option<mpsc::Receiver<bool>>,
    message: Option<String>,
    /// Quando a senha foi recusada (o campo balança).
    shake: Option<Instant>,
    /// Senhas recusadas seguidas (a dica aparece na terceira, como no Mac).
    failures: u32,
    account: lockscreen::Account,
    options: LoginOptions,
    avatars: Avatars,
    status: lockscreen::Status,
}

impl Lock {
    fn new() -> Self {
        let (login, name) = user();
        let account = lockscreen::account(&login).unwrap_or_default();
        let name = if account.name.trim().is_empty() { name } else { account.name.clone() };
        Self {
            login,
            name,
            password: String::new(),
            checking: None,
            message: None,
            shake: None,
            failures: 0,
            account,
            options: LoginOptions::load(),
            avatars: Avatars::default(),
            status: lockscreen::Status::start(),
        }
    }

    fn check(&mut self, waker: Option<Waker>) {
        let (tx, rx) = mpsc::channel();
        let (login, password) = (self.login.clone(), std::mem::take(&mut self.password));
        std::thread::spawn(move || {
            let ok = match test_password() {
                Some(expected) => {
                    std::thread::sleep(Duration::from_millis(200));
                    password == expected
                }
                None => pam::authenticate(pam_service(), &login, &password),
            };
            let _ = tx.send(ok);
            if let Some(w) = waker {
                w.wake();
            }
        });
        self.checking = Some(rx);
        self.message = None;
    }
}

impl App for Lock {
    fn view(&mut self, ui: &mut Ui) {
        let (_, _, w, h) = ui.region();
        // Resposta do PAM: só na tela principal (as dos outros monitores são cópias e
        // não podem consumir a resposta).
        if let Some(Ok(ok)) = self.checking.as_ref().filter(|_| !ui.mirror()).map(|rx| rx.try_recv()) {
            self.checking = None;
            if ok {
                tracing::info!("senha aceita: desbloqueando");
                ui.unlock();
                return;
            }
            tracing::info!("senha recusada");
            // Como no Mac, a senha errada só balança o campo (e a dica vem na terceira).
            self.failures += 1;
            self.shake = Some(Instant::now());
        }
        // O relógio anda; o campo balança por meio segundo.
        ui.wake_after(Duration::from_secs(1));
        let shaking = self.shake.is_some_and(|t| t.elapsed() < Duration::from_millis(450));
        if shaking {
            ui.request_tick();
        }
        let dx = self.shake.filter(|_| shaking).map_or(0.0, |t| {
            let s = t.elapsed().as_secs_f32();
            (s * 40.0).sin() * 12.0 * (1.0 - s / 0.45)
        });

        let cx = w / 2.0;
        lockscreen::clock(ui, cx, (h * 0.075).round());
        let status = self.status.get();
        let mut right = w - 10.0;
        if let Some((percent, charging)) = status.battery {
            right = lockscreen::status_battery(ui, percent, charging, right);
        }
        if let Some(signal) = status.wifi {
            lockscreen::status_wifi(ui, signal, right);
        }
        // Foto, nome e a senha na metade de baixo, como na tela de login.
        const AVATAR: f32 = 100.0;
        let y = (h * 0.50).round();
        let (icon, name) = (self.account.icon.clone(), self.name.clone());
        self.avatars.paint(ui, icon.as_deref(), &name, cx - AVATAR / 2.0, y, AVATAR, false);
        lockscreen::centered(ui, &name, cx, y + AVATAR + 12.0, 20.0, FontWeight::Semibold, lockscreen::WHITE);
        let field_w = 240.0;
        let field_y = y + AVATAR + 50.0;
        let mut submitted = false;
        let checking = self.checking.is_some();
        ui.at(cx - field_w / 2.0 + dx, field_y, field_w, |ui| {
            let look = LockField { secret: true, focused: !ui.mirror(), busy: checking, submit: true };
            submitted = ui.lock_field(&mut self.password, &tr!("Enter Password"), look).submitted;
        });
        let hint = self.account.hint.trim().to_string();
        if let Some(message) = self.message.clone() {
            lockscreen::centered(ui, &message, cx, field_y + 46.0, 13.0, FontWeight::Semibold, [255, 214, 214, 255]);
        } else if self.failures >= 3 && self.options.password_hints && !hint.is_empty() {
            lockscreen::centered(ui, &tr!("Hint: {hint}", hint = hint), cx, field_y + 46.0, 13.0, FontWeight::Semibold, [255, 255, 255, 235]);
        }
        if !self.options.message.is_empty() {
            let message = self.options.message.clone();
            lockscreen::centered(ui, &message, cx, h - 48.0, 13.0, FontWeight::Semibold, [255, 255, 255, 230]);
        }
        if submitted && !self.password.is_empty() {
            self.check(ui.waker());
        }
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let _logging = mide::logging::init("mide-lock", "mide_lock=info");
    run(Window::new(&mide::i18n::trc("window title", "Lock Screen")).app_id("mide-lock").lock(), Lock::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usuario_e_servico() {
        let (login, name) = user();
        assert!(!login.is_empty());
        assert!(!name.is_empty());
        assert!(["mide-lock", "login"].contains(&pam_service()));
    }

    /// O PAM de verdade recusa uma senha errada (e não trava).
    #[test]
    fn pam_recusa_senha_errada() {
        let (login, _) = user();
        assert!(!pam::authenticate(pam_service(), &login, "senha-errada-de-teste-do-mide"));
    }
}
