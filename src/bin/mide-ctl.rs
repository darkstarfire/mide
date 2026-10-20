//! `mide-ctl`: manda um comando ao compositor pelo canal de controle e mostra a
//! resposta. O mide é o do `WAYLAND_DISPLAY` (dentro da sessão, o atual).
//!
//! Exemplos: `mide-ctl list`, `mide-ctl restart shell`, `mide-ctl restart all`.

fn main() {
    let command = std::env::args().skip(1).collect::<Vec<_>>().join(" ");
    if command.trim().is_empty() {
        eprintln!("uso: mide-ctl <comando> (por exemplo: list, restart shell, restart all)");
        std::process::exit(2);
    }
    match mide::control_client::ask_compositor(&command) {
        Some(lines) => {
            for line in lines {
                println!("{line}");
            }
        }
        None => {
            eprintln!("mide-ctl: o mide não respondeu (WAYLAND_DISPLAY={})", std::env::var("WAYLAND_DISPLAY").unwrap_or_default());
            std::process::exit(1);
        }
    }
}
