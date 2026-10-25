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
    /// Saídas de som (alto-falantes, fones, HDMI) e entradas (microfones), com a
    /// padrão marcada. Vazias sem o `wpctl`.
    pub outputs: Vec<AudioDevice>,
    pub inputs: Vec<AudioDevice>,
}

/// Uma saída ou entrada de som do PipeWire.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AudioDevice {
    /// Número do nó no PipeWire (para o `wpctl`).
    pub id: u32,
    /// Nome para mostrar (o do aparelho, sem o perfil quando não há ambiguidade).
    pub name: String,
    pub default: bool,
    pub volume: f32,
}

/// Lê as saídas (`Sinks`) e entradas (`Sources`) da parte "Audio" do `wpctl status`.
pub fn parse_audio_devices(text: &str) -> (Vec<AudioDevice>, Vec<AudioDevice>) {
    let mut section = "";
    let mut in_audio = false;
    let mut devices: Vec<String> = Vec::new();
    let (mut sinks, mut sources) = (Vec::new(), Vec::new());
    for line in text.lines() {
        if !line.starts_with(' ') && !line.trim().is_empty() {
            in_audio = line.trim() == "Audio";
            continue;
        }
        if !in_audio {
            continue;
        }
        // Tira os desenhos da árvore ("│", "├─", "└─").
        let clean = line.trim_start_matches(|c: char| c == ' ' || c == '│' || c == '├' || c == '└' || c == '─');
        if let Some(name) = clean.strip_suffix(':') {
            section = match name.trim() {
                "Devices" => "devices",
                "Sinks" => "sinks",
                "Sources" => "sources",
                _ => "",
            };
            continue;
        }
        let default = clean.trim_start().starts_with('*');
        let rest = clean.trim_start().trim_start_matches('*').trim_start();
        let Some((num, after)) = rest.split_once(". ") else {
            continue;
        };
        let Ok(id) = num.trim().parse::<u32>() else {
            continue;
        };
        let (name, tail) = match after.rfind('[') {
            Some(i) => (after[..i].trim().to_string(), &after[i..]),
            None => (after.trim().to_string(), ""),
        };
        let volume = tail.strip_prefix("[vol:").and_then(|v| v.trim().trim_end_matches(']').split_whitespace().next()?.parse().ok()).unwrap_or(1.0);
        let device = AudioDevice { id, name, default, volume };
        match section {
            "devices" => devices.push(device.name),
            "sinks" => sinks.push(device),
            "sources" => sources.push(device),
            _ => {}
        }
    }
    // Nome curto: o do aparelho, quando ele tem só um nó na lista.
    for list in [&mut sinks, &mut sources] {
        let full: Vec<String> = list.iter().map(|d| d.name.clone()).collect();
        for d in list.iter_mut() {
            if let Some(dev) = devices.iter().filter(|dev| d.name.starts_with(dev.as_str())).max_by_key(|dev| dev.len()) {
                if full.iter().filter(|n| n.starts_with(dev.as_str())).count() == 1 {
                    d.name = dev.clone();
                }
            }
        }
    }
    (sinks, sources)
}

/// Escolhe a saída ou entrada de som padrão (os apps passam a tocar nela).
pub fn set_default_audio(id: u32) {
    if dry_run() {
        tracing::info!("sistema (teste): som padrão = {id}");
        return;
    }
    run("wpctl", &["set-default", &id.to_string()]);
}

/// Volume de uma saída ou entrada (0.0 a 1.0).
pub fn set_device_volume(id: u32, value: f32) {
    if dry_run() {
        tracing::info!("sistema (teste): volume de {id} = {value:.2}");
        return;
    }
    run("wpctl", &["set-volume", &id.to_string(), &format!("{:.2}", value.clamp(0.0, 1.0))]);
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
        outputs: vec![
            AudioDevice { id: 50, name: "Alto-falantes".into(), default: true, volume: 0.5 },
            AudioDevice { id: 51, name: "Fone USB".into(), default: false, volume: 0.8 },
        ],
        inputs: vec![AudioDevice { id: 52, name: "Microfone".into(), default: true, volume: 0.9 }],
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
    let (outputs, inputs) = output("wpctl", &["status"]).map(|t| parse_audio_devices(&t)).unwrap_or_default();
    Status { volume, wifi, bluetooth, brightness, outputs, inputs }
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

/// Roda um comando e espera ele terminar (ou só registra, nos testes). Para quem
/// precisa ler o resultado logo depois (o aviso das teclas multimídia, numa thread).
fn run_wait(program: &str, args: &[&str]) {
    let shown = format!("{program} {}", args.join(" "));
    if dry_run() {
        tracing::info!("sistema (teste): {shown}");
        return;
    }
    tracing::info!("sistema: {shown}");
    if let Err(err) = Command::new(program).args(args).status() {
        tracing::warn!("falha ao executar {shown}: {err}");
    }
}

/// Níveis simulados dos testes (`MIDE_SYSTEM_DRY_RUN`): as teclas multimídia mudam
/// estes valores no lugar do som e do brilho de verdade.
struct FakeLevels {
    volume: f32,
    muted: bool,
    mic_muted: bool,
    brightness: f32,
}

static FAKE: std::sync::Mutex<FakeLevels> = std::sync::Mutex::new(FakeLevels { volume: 0.5, muted: false, mic_muted: false, brightness: 0.7 });

/// Demora simulada do `wpctl`/`brightnessctl` nos testes
/// (`MIDE_SYSTEM_DRY_RUN_DELAY_MS`): o primeiro comando espera o tempo inteiro, o
/// segundo a metade e assim por diante, para um aperto seguinte poder terminar antes
/// do anterior se os dois rodarem ao mesmo tempo.
fn fake_latency() {
    use std::sync::atomic::{AtomicU64, Ordering};
    static CALLS: AtomicU64 = AtomicU64::new(0);
    let Some(ms) = std::env::var("MIDE_SYSTEM_DRY_RUN_DELAY_MS").ok().and_then(|v| v.parse::<u64>().ok()) else {
        return;
    };
    let n = CALLS.fetch_add(1, Ordering::Relaxed);
    std::thread::sleep(std::time::Duration::from_millis(ms / (n + 1)));
}

fn fake<T>(f: impl FnOnce(&mut FakeLevels) -> T) -> T {
    f(&mut FAKE.lock().unwrap_or_else(|e| e.into_inner()))
}

/// Passo de 5% sem erro de arredondamento acumulado (0,55 e não 0,5499...).
fn step5(value: f32, up: bool, min: f32) -> f32 {
    let steps = (value * 20.0).round() + if up { 1.0 } else { -1.0 };
    (steps / 20.0).clamp(min, 1.0)
}

/// Teclas de volume do teclado: 5% a mais ou a menos (no máximo 100%). Espera o
/// comando terminar (chame fora da thread do compositor).
pub fn volume_step(up: bool) {
    if dry_run() {
        fake_latency();
        fake(|f| f.volume = step5(f.volume, up, 0.0));
    }
    run_wait("wpctl", &["set-volume", "-l", "1.0", "@DEFAULT_AUDIO_SINK@", if up { "5%+" } else { "5%-" }]);
}

/// Tecla de mudo: liga ou desliga o som (espera o comando).
pub fn toggle_mute() {
    if dry_run() {
        fake_latency();
        fake(|f| f.muted = !f.muted);
    }
    run_wait("wpctl", &["set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"]);
}

/// Tecla de microfone mudo (espera o comando).
pub fn toggle_mic_mute() {
    if dry_run() {
        fake_latency();
        fake(|f| f.mic_muted = !f.mic_muted);
    }
    run_wait("wpctl", &["set-mute", "@DEFAULT_AUDIO_SOURCE@", "toggle"]);
}

/// Teclas de brilho: 5% a mais ou a menos (sem chegar a apagar a tela; espera o comando).
pub fn brightness_step(up: bool) {
    if dry_run() {
        fake_latency();
        fake(|f| f.brightness = step5(f.brightness, up, 0.01));
    }
    run_wait("brightnessctl", &["-c", "backlight", "-n", "1%", "set", if up { "5%+" } else { "5%-" }]);
}

/// Volume (0 a 1) e mudo da saída de som padrão, agora. `None` sem áudio.
pub fn read_volume() -> Option<(f32, bool)> {
    if dry_run() {
        return Some(fake(|f| (f.volume, f.muted)));
    }
    output("wpctl", &["get-volume", "@DEFAULT_AUDIO_SINK@"]).and_then(|t| parse_volume(&t))
}

/// O microfone padrão está mudo? `None` sem microfone.
pub fn read_mic_muted() -> Option<bool> {
    if dry_run() {
        return Some(fake(|f| f.mic_muted));
    }
    output("wpctl", &["get-volume", "@DEFAULT_AUDIO_SOURCE@"]).and_then(|t| parse_volume(&t)).map(|(_, muted)| muted)
}

/// Brilho da tela (0 a 1), agora. `None` sem luz de fundo controlável.
pub fn read_brightness() -> Option<f32> {
    if dry_run() {
        return Some(fake(|f| f.brightness));
    }
    output("brightnessctl", &["-m", "-c", "backlight"]).and_then(|t| parse_brightness(&t))
}

/// Teclas de mídia (tocar/pausar, próxima, anterior, parar): pelo MPRIS no D-Bus da
/// sessão, no player que está tocando (ou no primeiro que houver). `method` é o do
/// `org.mpris.MediaPlayer2.Player` ("PlayPause", "Next", "Previous", "Stop").
pub fn media_player(method: &'static str) {
    if dry_run() {
        tracing::info!("sistema (teste): mpris {method}");
        return;
    }
    std::thread::spawn(move || {
        let called = (|| -> zbus::Result<Option<String>> {
            let conn = zbus::blocking::Connection::session()?;
            let dbus = zbus::blocking::fdo::DBusProxy::new(&conn)?;
            let players: Vec<String> = dbus.list_names()?.into_iter().map(|n| n.to_string()).filter(|n| n.starts_with("org.mpris.MediaPlayer2.")).collect();
            let proxy = |name: &str| zbus::blocking::Proxy::new(&conn, name.to_string(), "/org/mpris/MediaPlayer2", "org.mpris.MediaPlayer2.Player");
            let playing = players.iter().find(|n| proxy(n).and_then(|p| p.get_property::<String>("PlaybackStatus")).is_ok_and(|s| s == "Playing"));
            let Some(name) = playing.or(players.first()).cloned() else {
                return Ok(None);
            };
            proxy(&name)?.call::<_, _, ()>(method, &())?;
            Ok(Some(name))
        })();
        match called {
            Ok(Some(name)) => tracing::info!("sistema: mpris {method} em {name}"),
            Ok(None) => tracing::info!("sistema: mpris {method}: nenhum player aberto"),
            Err(err) => tracing::warn!("sistema: mpris {method} falhou: {err}"),
        }
    });
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

/// Pelo logind no D-Bus (`org.freedesktop.login1`): funciona com o systemd e com o
/// elogind (distribuições sem systemd, como o Slackware e o Void). Sem resposta do
/// logind, tenta o `systemctl`/`loginctl`.
pub fn power(action: Power) {
    let (method, fallback): (&str, (&str, &[&str])) = match action {
        Power::Sleep => ("Suspend", ("systemctl", &["suspend"])),
        Power::Restart => ("Reboot", ("systemctl", &["reboot"])),
        Power::Shutdown => ("PowerOff", ("systemctl", &["poweroff"])),
        Power::Lock => ("LockSessions", ("loginctl", &["lock-session"])),
    };
    if dry_run() {
        tracing::info!("sistema (teste): logind {method}");
        return;
    }
    std::thread::spawn(move || {
        let called = zbus::blocking::Connection::system().and_then(|conn| {
            let proxy = zbus::blocking::Proxy::new(&conn, "org.freedesktop.login1", "/org/freedesktop/login1", "org.freedesktop.login1.Manager")?;
            if method == "LockSessions" {
                proxy.call::<_, _, ()>(method, &())
            } else {
                // `true`: o logind pode pedir a senha (polkit) se for preciso.
                proxy.call::<_, _, ()>(method, &(true,))
            }
        });
        match called {
            Ok(()) => tracing::info!("sistema: logind {method}"),
            Err(err) => {
                tracing::warn!("sistema: logind {method} falhou ({err}); tentando {}", fallback.0);
                run(fallback.0, fallback.1);
            }
        }
    });
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
    fn passo_de_cinco_por_cento() {
        assert_eq!(step5(0.5, true, 0.0), 0.55);
        assert_eq!(step5(0.98, true, 0.0), 1.0);
        assert_eq!(step5(0.02, false, 0.0), 0.0);
        assert_eq!(step5(0.05, false, 0.01), 0.01);
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

#[cfg(test)]
mod audio_tests {
    use super::*;

    #[test]
    fn le_saidas_e_entradas_do_wpctl() {
        let text = "PipeWire 'pipewire-0' [1.0.5]\n └─ Clients:\n        31. pipewire\n\nAudio\n ├─ Devices:\n │      46. Built-in Audio                      [alsa]\n │      53. Audio Adapter (Unitek Y-247A)       [alsa]\n │  \n ├─ Sinks:\n │  *   60. Audio Adapter (Unitek Y-247A) Analog Stereo [vol: 0.80]\n │      61. Built-in Audio Analog Stereo  [vol: 0.55 MUTED]\n │      62. Built-in Audio Digital Stereo (HDMI) [vol: 1.00]\n │  \n ├─ Sources:\n │      58. Built-in Audio Analog Stereo  [vol: 1.00]\n │  *   65. Audio Adapter (Unitek Y-247A) Mono  [vol: 0.40]\n │  \n └─ Streams:\n\nVideo\n ├─ Sinks:\n │      90. Câmera [vol: 1.00]\n";
        let (sinks, sources) = parse_audio_devices(text);
        assert_eq!(sinks.len(), 3);
        assert_eq!(sinks[0], AudioDevice { id: 60, name: "Audio Adapter (Unitek Y-247A)".into(), default: true, volume: 0.8 });
        // Dois nós do mesmo aparelho: fica o nome completo.
        assert_eq!(sinks[1].name, "Built-in Audio Analog Stereo");
        assert!((sinks[1].volume - 0.55).abs() < 1e-6);
        assert_eq!(sources.iter().map(|d| (d.id, d.default)).collect::<Vec<_>>(), [(58, false), (65, true)]);
        assert_eq!(sources[0].name, "Built-in Audio");
    }
}
