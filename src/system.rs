//! Estado e ações do sistema para os ícones da barra: som (`wpctl`), Wi-Fi
//! (`nmcli`) e Bluetooth (`bluetoothctl`).
//!
//! Com `MIDE_SYSTEM_DRY_RUN` definido (testes), o estado é simulado e as ações só
//! vão para o log: nada muda no computador de verdade.

use std::process::Command;

/// Rede Wi-Fi vista pelo NetworkManager.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Network {
    pub ssid: String,
    /// Sinal de 0 a 100.
    pub signal: u8,
    pub secure: bool,
    pub active: bool,
}

/// Aparelho Bluetooth pareado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub address: String,
    pub name: String,
    pub connected: bool,
}

/// Retrato do sistema para a barra.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Status {
    /// Volume de 0.0 a 1.0 (pode passar de 1.0) e se está mudo. `None` sem áudio.
    pub volume: Option<(f32, bool)>,
    /// Wi-Fi ligado e as redes vistas. `None` sem NetworkManager.
    pub wifi: Option<(bool, Vec<Network>)>,
    /// Bluetooth ligado e os aparelhos pareados. `None` sem adaptador.
    pub bluetooth: Option<(bool, Vec<Device>)>,
    /// Brilho da tela de 0.0 a 1.0. `None` sem controle de brilho (`brightnessctl`)
    /// ou sem tela com luz de fundo (monitores de mesa).
    pub brightness: Option<f32>,
}

impl Status {
    /// Rede conectada agora, se houver.
    pub fn active_network(&self) -> Option<&Network> {
        self.wifi.as_ref().and_then(|(_, list)| list.iter().find(|n| n.active))
    }
}

fn dry_run() -> bool {
    std::env::var_os("MIDE_SYSTEM_DRY_RUN").is_some()
}

/// Roda um comando com mensagens em inglês (saída estável para ler) e devolve a saída.
fn output(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program).args(args).env("LC_ALL", "C").output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Lê "Volume: 0.68 [MUTED]" do `wpctl get-volume`.
pub fn parse_volume(text: &str) -> Option<(f32, bool)> {
    let rest = text.trim().strip_prefix("Volume:")?.trim();
    let value: f32 = rest.split_whitespace().next()?.parse().ok()?;
    Some((value, rest.contains("[MUTED]")))
}

/// Separa uma linha do `nmcli -t` (campos com `:`, e `\:` dentro de um campo).
fn split_terse(line: &str) -> Vec<String> {
    let mut fields = vec![String::new()];
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(next) = chars.next() {
                    fields.last_mut().unwrap().push(next);
                }
            }
            ':' => fields.push(String::new()),
            _ => fields.last_mut().unwrap().push(c),
        }
    }
    fields
}

/// Lê `nmcli -t -f ACTIVE,SSID,SIGNAL,SECURITY dev wifi list`. Redes sem nome saem;
/// cada nome aparece uma vez (a conectada, ou a de sinal mais forte), do melhor
/// sinal para o pior, com a conectada primeiro.
pub fn parse_networks(text: &str) -> Vec<Network> {
    let mut list: Vec<Network> = Vec::new();
    for line in text.lines() {
        let f = split_terse(line);
        if f.len() < 4 || f[1].is_empty() {
            continue;
        }
        let net = Network {
            ssid: f[1].clone(),
            signal: f[2].parse().unwrap_or(0),
            secure: !f[3].trim().is_empty() && f[3].trim() != "--",
            active: f[0] == "yes",
        };
        match list.iter_mut().find(|n| n.ssid == net.ssid) {
            Some(old) => {
                if net.active || (!old.active && net.signal > old.signal) {
                    *old = net;
                }
            }
            None => list.push(net),
        }
    }
    list.sort_by(|a, b| b.active.cmp(&a.active).then(b.signal.cmp(&a.signal)));
    list
}

/// Lê "Device AA:BB:... Nome" do `bluetoothctl devices`.
pub fn parse_devices(text: &str) -> Vec<Device> {
    text.lines()
        .filter_map(|l| {
            let rest = l.trim().strip_prefix("Device ")?;
            let (address, name) = rest.split_once(' ')?;
            Some(Device { address: address.into(), name: name.trim().into(), connected: false })
        })
        .collect()
}

/// Estado simulado dos testes.
fn fake_status() -> Status {
    Status {
        volume: Some((0.5, false)),
        wifi: Some((
            true,
            vec![
                Network { ssid: "Casa".into(), signal: 82, secure: true, active: true },
                Network { ssid: "Café".into(), signal: 60, secure: false, active: false },
                Network { ssid: "Vizinho".into(), signal: 35, secure: true, active: false },
            ],
        )),
        bluetooth: Some((
            true,
            vec![
                Device { address: "00:11:22:33:44:55".into(), name: "Fone".into(), connected: true },
                Device { address: "66:77:88:99:AA:BB".into(), name: "Teclado".into(), connected: false },
            ],
        )),
        brightness: Some(0.7),
    }
}

/// Brilho pela saída de `brightnessctl -m` (`aparelho,classe,atual,70%,máximo`).
pub fn parse_brightness(text: &str) -> Option<f32> {
    let line = text.lines().next()?;
    let percent = line.split(',').nth(3)?.trim().trim_end_matches('%');
    percent.parse::<f32>().ok().map(|p| (p / 100.0).clamp(0.0, 1.0))
}

/// Lê o estado atual (alguns comandos externos; rode fora da thread da interface).
pub fn read_status() -> Status {
    if dry_run() {
        return fake_status();
    }
    let volume = output("wpctl", &["get-volume", "@DEFAULT_AUDIO_SINK@"]).and_then(|t| parse_volume(&t));
    let wifi = output("nmcli", &["-t", "-f", "WIFI", "general"]).map(|t| {
        let on = t.trim() == "enabled";
        let list = if on {
            output("nmcli", &["-t", "-f", "ACTIVE,SSID,SIGNAL,SECURITY", "device", "wifi", "list", "--rescan", "no"])
                .map(|t| parse_networks(&t))
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        (on, list)
    });
    let bluetooth = output("bluetoothctl", &["show"]).filter(|t| t.contains("Controller")).map(|t| {
        let on = t.lines().any(|l| l.trim() == "Powered: yes");
        let mut devices = output("bluetoothctl", &["devices", "Paired"]).map(|t| parse_devices(&t)).unwrap_or_default();
        let connected = output("bluetoothctl", &["devices", "Connected"]).map(|t| parse_devices(&t)).unwrap_or_default();
        for d in &mut devices {
            d.connected = connected.iter().any(|c| c.address == d.address);
        }
        (on, devices)
    });
    let brightness = output("brightnessctl", &["-m", "-c", "backlight"]).and_then(|t| parse_brightness(&t));
    Status { volume, wifi, bluetooth, brightness }
}

/// Executa uma ação em segundo plano (ou só registra, nos testes).
fn run(program: &str, args: &[&str]) {
    let shown = format!("{program} {}", args.join(" "));
    if dry_run() {
        tracing::info!("sistema (teste): {shown}");
        return;
    }
    tracing::info!("sistema: {shown}");
    let mut command = Command::new(program);
    command.args(args);
    match command.spawn() {
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(err) => tracing::warn!("falha ao executar {shown}: {err}"),
    }
}

pub fn set_volume(value: f32) {
    let value = format!("{:.2}", value.clamp(0.0, 1.0));
    run("wpctl", &["set-volume", "@DEFAULT_AUDIO_SINK@", &value]);
}

pub fn set_brightness(value: f32) {
    let value = format!("{}%", (value.clamp(0.01, 1.0) * 100.0).round() as i32);
    run("brightnessctl", &["-c", "backlight", "set", &value]);
}

/// Ações de energia e sessão do menu do mide (com confirmação na barra). Nos testes
/// só são registradas, nunca executadas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Power {
    Sleep,
    Restart,
    Shutdown,
    Lock,
}

pub fn power(action: Power) {
    match action {
        Power::Sleep => run("systemctl", &["suspend"]),
        Power::Restart => run("systemctl", &["reboot"]),
        Power::Shutdown => run("systemctl", &["poweroff"]),
        Power::Lock => run("loginctl", &["lock-session"]),
    }
}

pub fn set_muted(muted: bool) {
    run("wpctl", &["set-mute", "@DEFAULT_AUDIO_SINK@", if muted { "1" } else { "0" }]);
}

pub fn set_wifi(on: bool) {
    run("nmcli", &["radio", "wifi", if on { "on" } else { "off" }]);
}

/// Conecta a uma rede (redes salvas ou abertas; para senha nova, abre as configurações).
pub fn connect_network(ssid: &str) {
    run("nmcli", &["device", "wifi", "connect", ssid]);
}

pub fn set_bluetooth(on: bool) {
    run("bluetoothctl", &["power", if on { "on" } else { "off" }]);
}

pub fn toggle_device(device: &Device) {
    let action = if device.connected { "disconnect" } else { "connect" };
    run("bluetoothctl", &[action, &device.address]);
}

/// Abre um painel das configurações (`wifi`, `bluetooth`, `sound`, `power`...). As
/// seções que o mide tem abrem nas Configurações do mide (`mide-settings`, ao lado do
/// programa atual ou no PATH); as outras, nas do GNOME.
pub fn open_settings(panel: &str) {
    // Vazio: abre as Configurações na seção inicial.
    const OWN: &[&str] = &["", "wifi", "bluetooth", "sound", "wallpaper", "dock", "effects", "keyboard", "about"];
    if OWN.contains(&panel) {
        let sibling = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|d| d.join("mide-settings")))
            .filter(|p| p.exists());
        let program = sibling.map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|| "mide-settings".into());
        if dry_run() {
            tracing::info!("sistema (teste): mide-settings {panel}");
            return;
        }
        let mut command = Command::new(&program);
        if !panel.is_empty() {
            command.arg(panel);
        }
        match command.spawn() {
            Ok(mut child) => {
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
                tracing::info!("configurações: {panel}");
            }
            Err(err) => tracing::warn!("falha ao abrir as configurações: {err}"),
        }
        return;
    }
    run("gnome-control-center", &[panel]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brilho_do_brightnessctl() {
        assert_eq!(parse_brightness("intel_backlight,backlight,750,75%,1000\n"), Some(0.75));
        assert_eq!(parse_brightness(""), None);
        assert_eq!(parse_brightness("lixo"), None);
    }

    #[test]
    fn volume_e_mudo() {
        assert_eq!(parse_volume("Volume: 0.68 [MUTED]\n"), Some((0.68, true)));
        assert_eq!(parse_volume("Volume: 1.00"), Some((1.0, false)));
        assert_eq!(parse_volume("erro"), None);
    }

    #[test]
    fn redes_sem_repetir_e_conectada_primeiro() {
        let text = "no:CLARO:100:WPA2\nno:VIVO\\:5G:84:WPA2\nyes:VIVO\\:5G:82:WPA2\nno::50:WPA2\nno:Aberta:40:\n";
        let list = parse_networks(text);
        let names: Vec<&str> = list.iter().map(|n| n.ssid.as_str()).collect();
        assert_eq!(names, vec!["VIVO:5G", "CLARO", "Aberta"]);
        assert!(list[0].active);
        assert!(!list[2].secure);
    }

    #[test]
    fn aparelhos_bluetooth() {
        let list = parse_devices("Device 00:11:22:33:44:55 Fone JBL\nDevice AA:BB:CC:DD:EE:FF Teclado\nlixo\n");
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name, "Fone JBL");
        assert_eq!(list[1].address, "AA:BB:CC:DD:EE:FF");
    }
}
