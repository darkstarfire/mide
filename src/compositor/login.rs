//! Modo de login (C5): o compositor roda só a tela de login (`mide-login`, greeter do
//! `greetd`), sem barra, dock, mesa, notificações, apps de início, XWayland nem
//! portais. É o comando que o `greetd` inicia (`mide --login`, ou `MIDE_LOGIN=1`).
//!
//! A tela de login usa o protocolo de bloqueio (`ext-session-lock-v1`), como o
//! `mide-lock`: o compositor desenha o papel de parede desfocado atrás dela, cobre
//! todas as telas, só ela recebe teclado e mouse, e nenhum atalho abre nada. Se o
//! greeter cair, abre outro. Quando ele "desbloqueia" (a sessão escolhida foi pedida
//! ao greetd), o compositor sai, e o greetd inicia a sessão.

use std::sync::OnceLock;

static ACTIVE: OnceLock<bool> = OnceLock::new();

/// Lê o modo uma vez (argumento `--login` ou `MIDE_LOGIN=1`). Tira a variável do
/// ambiente, para a sessão que o greetd iniciar depois não herdar o modo.
pub fn init() -> bool {
    *ACTIVE.get_or_init(|| {
        let by_arg = std::env::args().skip(1).any(|a| a == "--login");
        let by_env = std::env::var("MIDE_LOGIN").is_ok_and(|v| v == "1");
        // SAFETY: chamado no começo do `main`, antes de qualquer outra thread.
        unsafe { std::env::remove_var("MIDE_LOGIN") };
        by_arg || by_env
    })
}

/// O compositor está no modo de login.
pub fn active() -> bool {
    ACTIVE.get().copied().unwrap_or(false)
}

/// O programa da tela de bloqueio: o greeter no modo de login, senão o `mide-lock`.
pub fn lock_program() -> &'static str {
    if active() { "mide-login" } else { "mide-lock" }
}
