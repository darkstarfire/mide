//! Tela de bloqueio do mide: relógio, data, quem está usando e a senha, sobre o papel
//! de parede desfocado. Usa o protocolo de bloqueio do Wayland (`ext-session-lock-v1`):
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

use chrono::{Datelike, Local, Timelike};
use mide::{
    kit::{run, App, Ui, Waker, Window},
    ui::{
        draw::{fill_rounded, rgba},
        text::FontWeight,
    },
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

const WEEKDAYS: [&str; 7] = ["segunda-feira", "terça-feira", "quarta-feira", "quinta-feira", "sexta-feira", "sábado", "domingo"];
const MONTHS: [&str; 12] = ["janeiro", "fevereiro", "março", "abril", "maio", "junho", "julho", "agosto", "setembro", "outubro", "novembro", "dezembro"];

struct Lock {
    login: String,
    name: String,
    password: String,
    /// Conferindo a senha (o PAM pode levar uns segundos quando ela está errada).
    checking: Option<mpsc::Receiver<bool>>,
    message: Option<String>,
    /// Quando a senha foi recusada (o campo balança).
    shake: Option<Instant>,
}

impl Lock {
    fn new() -> Self {
        let (login, name) = user();
        Self { login, name, password: String::new(), checking: None, message: None, shake: None }
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
        if let Some(rx) = self.checking.as_ref().filter(|_| !ui.mirror()) {
            if let Ok(ok) = rx.try_recv() {
                self.checking = None;
                if ok {
                    tracing::info!("senha aceita: desbloqueando");
                    ui.unlock();
                    return;
                }
                tracing::info!("senha recusada");
                self.message = Some("Senha incorreta".into());
                self.shake = Some(Instant::now());
            }
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

        let now = Local::now();
        let time = format!("{:02}:{:02}", now.hour(), now.minute());
        let date = format!("{}, {} de {}", WEEKDAYS[now.weekday().num_days_from_monday() as usize], now.day(), MONTHS[now.month0() as usize]);
        let initial = self.name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
        let name = self.name.clone();
        let (cx, top) = (w / 2.0, h * 0.18);
        // O fundo (papel de parede desfocado e escurecido) é do compositor, na GPU:
        // aqui só o relógio, o avatar e o campo, e cada tecla custa pouco.
        ui.paint_text(|pixmap, text, (ox, oy)| {
            let white = [255, 255, 255, 255];
            let tw = text.measure(&time, 84.0, FontWeight::Semibold);
            text.draw(pixmap, &time, ox + cx - tw / 2.0, oy + top, 84.0, FontWeight::Semibold, white);
            let dw = text.measure(&date, 20.0, FontWeight::Regular);
            text.draw(pixmap, &date, ox + cx - dw / 2.0, oy + top + 110.0, 20.0, FontWeight::Regular, [255, 255, 255, 220]);
            // Avatar com a inicial e o nome, acima do campo de senha.
            let ay = h * 0.58;
            fill_rounded(pixmap, ox + cx - 36.0, oy + ay - 110.0, 72.0, 72.0, 36.0, rgba(255, 255, 255, 70));
            let iw = text.measure(&initial, 32.0, FontWeight::Semibold);
            text.draw(pixmap, &initial, ox + cx - iw / 2.0, oy + ay - 110.0 + 16.0, 32.0, FontWeight::Semibold, white);
            let nw = text.measure(&name, 16.0, FontWeight::Semibold);
            text.draw(pixmap, &name, ox + cx - nw / 2.0, oy + ay - 28.0, 16.0, FontWeight::Semibold, white);
        });
        let field_w = 260.0;
        let mut submitted = false;
        let checking = self.checking.is_some();
        ui.at(cx - field_w / 2.0 + dx, h * 0.58, field_w, |ui| {
            let placeholder = if checking { "Conferindo…" } else { "Digite a senha" };
            if !checking {
                submitted = ui.password_input(&mut self.password, placeholder).submitted();
            } else {
                let mut empty = String::new();
                let _ = ui.password_input(&mut empty, placeholder);
            }
        });
        if let Some(message) = self.message.clone() {
            ui.paint_text(|pixmap, text, (ox, oy)| {
                let mw = text.measure(&message, 14.0, FontWeight::Regular);
                text.draw(pixmap, &message, ox + cx - mw / 2.0, oy + h * 0.58 + 52.0, 14.0, FontWeight::Regular, [255, 220, 220, 255]);
            });
        }
        if submitted && !self.password.is_empty() {
            self.check(ui.waker());
        }
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "mide_lock=info".into()))
        .init();
    run(Window::new("Bloqueio").app_id("mide-lock").lock(), Lock::new())
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
