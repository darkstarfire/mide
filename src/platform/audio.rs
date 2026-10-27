//! Saídas e entradas de som como o usuário as vê (como no GNOME e no Mac): uma linha
//! por **porta de verdade** das placas (alto-falantes, fones, cada HDMI com o nome do
//! monitor, microfones), em todos os perfis da placa, e não um nó do PipeWire por
//! perfil ativo. Lido do `pactl -f json` (o servidor PulseAudio do PipeWire):
//!
//! - `pactl list cards`: placas, perfis e portas (com a disponibilidade: nada ligado
//!   no conector = "not available", que some da lista);
//! - `pactl list sinks/sources`: os nós que existem no perfil ativo, com volume e porta.
//!
//! Escolher uma porta que só existe em outro perfil (em placas SOF/UCM, como as dos
//! notebooks Intel, "Speaker" e "Headphones" ficam em perfis diferentes) troca o perfil
//! da placa antes (`pactl set-card-profile`) e depois define o padrão.
//!
//! Em Go, pense neste arquivo como um pacote que só transforma JSON em structs
//! (`parse_*`) e monta a lista (`build`), sem efeitos colaterais: por isso dá para
//! testar com as saídas reais guardadas em `testdata/`.

use serde_json::Value;

/// Tipo da saída/entrada, para o ícone e a ordem na lista.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AudioKind {
    Speaker,
    Headphones,
    Headset,
    Hdmi,
    Usb,
    Bluetooth,
    Mic,
    #[default]
    Other,
}

impl AudioKind {
    /// Ordem na lista: fones primeiro (ligados, são o que se quer ouvir), depois os
    /// alto-falantes do computador, aparelhos externos e monitores.
    fn rank(self) -> u8 {
        match self {
            AudioKind::Headphones | AudioKind::Headset => 0,
            AudioKind::Speaker | AudioKind::Mic => 1,
            AudioKind::Usb | AudioKind::Bluetooth => 2,
            AudioKind::Hdmi => 3,
            AudioKind::Other => 4,
        }
    }
}

/// Uma saída ou entrada de som, pronta para a lista.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AudioDevice {
    /// Identidade estável ("placa|porta" ou o nome do nó), para comparar leituras.
    pub key: String,
    /// Nome para mostrar ("Alto-falantes", "Fones de ouvido", "DELL U2717D").
    pub name: String,
    /// Linha de baixo (o conector do monitor, a placa quando há mais de uma). Pode ser vazia.
    pub detail: String,
    pub kind: AudioKind,
    pub default: bool,
    pub volume: f32,
    /// Placa e porta (vazias para nós sem placa, como saídas virtuais).
    pub card: String,
    pub port: String,
    /// Perfil da placa a ativar antes (a porta não existe no perfil de agora).
    pub profile: Option<String>,
    /// Nó do PipeWire (nome do sink/source no `pactl`), se já existe. Na leitura pelo
    /// `wpctl` (sem `pactl`), o número do nó.
    pub node: String,
    /// Lido pelo `wpctl` (reserva sem `pactl`): as ações usam o número do nó.
    pub via_wpctl: bool,
    /// O conector diz que há algo ligado (porta "available", não só "unknown").
    pub plugged: bool,
}

/// Disponibilidade de uma porta (conector).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Availability {
    Yes,
    No,
    Unknown,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Port {
    pub name: String,
    pub description: String,
    /// "speaker", "headphones", "hdmi", "mic"... (`port.type`, em minúsculas).
    pub kind: String,
    pub output: bool,
    pub priority: u32,
    pub availability: Availability,
    /// Perfis da placa que têm esta porta.
    pub profiles: Vec<String>,
    /// Nome do aparelho ligado (no HDMI, o monitor lido do ELD).
    pub product: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    pub name: String,
    pub priority: u32,
    pub available: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Card {
    pub index: u32,
    pub name: String,
    pub description: String,
    /// "pci", "usb", "bluetooth"...
    pub bus: String,
    pub form_factor: String,
    pub active_profile: String,
    pub profiles: Vec<Profile>,
    pub ports: Vec<Port>,
}

/// Um sink ou source.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub index: u32,
    pub name: String,
    pub description: String,
    /// Placa dona (`device.id`), se houver.
    pub card: Option<u32>,
    pub active_port: Option<String>,
    pub ports: Vec<String>,
    pub volume: f32,
    pub monitor: bool,
    pub bus: String,
    pub form_factor: String,
}

fn s(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_string()
}

fn availability(text: &str) -> Availability {
    match text {
        "available" => Availability::Yes,
        "not available" => Availability::No,
        _ => Availability::Unknown,
    }
}

/// Lê `pactl -f json list cards`.
pub fn parse_cards(text: &str) -> Vec<Card> {
    let Ok(Value::Array(list)) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    list.iter()
        .map(|c| {
            let props = &c["properties"];
            let mut profiles: Vec<Profile> = c["profiles"]
                .as_object()
                .map(|m| {
                    m.iter()
                        .map(|(name, p)| Profile { name: name.clone(), priority: p["priority"].as_u64().unwrap_or(0) as u32, available: p["available"].as_bool().unwrap_or(true) })
                        .collect()
                })
                .unwrap_or_default();
            profiles.sort_by_key(|p| std::cmp::Reverse(p.priority));
            let ports = c["ports"]
                .as_object()
                .map(|m| {
                    m.iter()
                        .map(|(name, p)| {
                            let pp = &p["properties"];
                            let kind = pp["port.type"].as_str().map(str::to_string).unwrap_or_else(|| s(&p["type"])).to_lowercase();
                            Port {
                                name: name.clone(),
                                description: s(&p["description"]),
                                kind,
                                // "[Out] HDMI1"/"[In] Mic1" (UCM) ou "analog-output-speaker".
                                output: !(name.starts_with("[In]") || name.contains("input")),
                                priority: p["priority"].as_u64().unwrap_or(0) as u32,
                                availability: availability(p["availability"].as_str().unwrap_or_default()),
                                profiles: p["profiles"].as_array().map(|a| a.iter().map(s).collect()).unwrap_or_default(),
                                product: pp["device.product.name"].as_str().filter(|t| !t.trim().is_empty()).map(|t| t.trim().to_string()),
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();
            Card {
                index: c["index"].as_u64().unwrap_or(0) as u32,
                name: s(&c["name"]),
                description: props["device.description"].as_str().map(str::to_string).unwrap_or_else(|| s(&c["name"])),
                bus: s(&props["device.bus"]),
                form_factor: s(&props["device.form_factor"]),
                active_profile: s(&c["active_profile"]),
                profiles,
                ports,
            }
        })
        .collect()
}

/// Lê `pactl -f json list sinks` (ou `sources`).
pub fn parse_nodes(text: &str) -> Vec<Node> {
    let Ok(Value::Array(list)) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    list.iter()
        .map(|n| {
            let props = &n["properties"];
            let name = s(&n["name"]);
            // Volume: a média dos canais (65536 = 100%).
            let volume = n["volume"]
                .as_object()
                .map(|m| {
                    let values: Vec<f64> = m.values().filter_map(|c| c["value"].as_f64()).collect();
                    if values.is_empty() {
                        1.0
                    } else {
                        values.iter().sum::<f64>() / values.len() as f64 / 65536.0
                    }
                })
                .unwrap_or(1.0) as f32;
            Node {
                index: n["index"].as_u64().unwrap_or(0) as u32,
                monitor: props["device.class"].as_str() == Some("monitor") || name.ends_with(".monitor"),
                name,
                description: s(&n["description"]),
                card: props["device.id"].as_str().and_then(|t| t.parse().ok()),
                active_port: n["active_port"].as_str().filter(|t| !t.is_empty()).map(str::to_string),
                ports: n["ports"].as_array().map(|a| a.iter().map(|p| s(&p["name"])).collect()).unwrap_or_default(),
                volume,
                bus: s(&props["device.bus"]),
                form_factor: s(&props["device.form_factor"]),
            }
        })
        .collect()
}

/// Nome em português de uma porta, pelo tipo e pela descrição (as descrições vêm do
/// ALSA/UCM em inglês).
fn port_label(port: &Port) -> String {
    let desc = port.description.trim();
    let clean = desc.trim_end_matches(" Output").trim_end_matches(" Input").trim();
    let known = match clean {
        "Speaker" | "Speakers" | "Internal Speaker" | "Internal Speakers" => Some("Alto-falantes"),
        "Headphones" | "Headphone" => Some("Fones de ouvido"),
        "Headset" => Some("Headset"),
        "Line Out" => Some("Saída de linha"),
        "Line In" => Some("Entrada de linha"),
        "Digital Microphone" => Some("Microfone digital"),
        "Internal Microphone" => Some("Microfone interno"),
        "Stereo Microphone" => Some("Microfone estéreo"),
        "Headset Microphone" => Some("Microfone do headset"),
        "Headphones Stereo Microphone" => Some("Microfone dos fones"),
        "Microphone" => Some("Microfone"),
        "Front Microphone" => Some("Microfone frontal"),
        "Rear Microphone" => Some("Microfone traseiro"),
        "Analog Output" => Some("Saída analógica"),
        "Analog Input" => Some("Entrada analógica"),
        "Digital Output (S/PDIF)" => Some("Saída digital (S/PDIF)"),
        _ => None,
    };
    if let Some(k) = known {
        return k.to_string();
    }
    match port.kind.as_str() {
        "speaker" if clean.is_empty() => "Alto-falantes".into(),
        "headphones" if clean.is_empty() => "Fones de ouvido".into(),
        "mic" if clean.is_empty() => "Microfone".into(),
        _ if clean.is_empty() => port.name.clone(),
        _ => clean.to_string(),
    }
}

fn kind_of(port_kind: &str, bus: &str, form_factor: &str, output: bool) -> AudioKind {
    match bus {
        "bluetooth" => return AudioKind::Bluetooth,
        "usb" => {
            return match form_factor {
                "headset" => AudioKind::Headset,
                "headphone" => AudioKind::Headphones,
                _ => AudioKind::Usb,
            };
        }
        _ => {}
    }
    match port_kind {
        "speaker" => AudioKind::Speaker,
        "headphones" => AudioKind::Headphones,
        "headset" => AudioKind::Headset,
        "hdmi" | "spdif" | "displayport" => AudioKind::Hdmi,
        "mic" => AudioKind::Mic,
        _ if !output => AudioKind::Mic,
        _ => AudioKind::Other,
    }
}

/// Melhor perfil da placa com a porta: o ativo, se já tem; senão o disponível de maior
/// prioridade (é o que o wireplumber escolheria).
fn profile_for(card: &Card, port: &Port) -> Option<String> {
    if port.profiles.contains(&card.active_profile) {
        return None;
    }
    card.profiles.iter().filter(|p| p.available && port.profiles.contains(&p.name)).max_by_key(|p| p.priority).map(|p| p.name.clone())
}

/// Monta a lista de saídas (`output`) ou entradas: uma linha por porta disponível das
/// placas, mais os nós sem placa (saídas virtuais). `default` é o nome do sink/source
/// padrão (`pactl get-default-sink`).
pub fn build(cards: &[Card], nodes: &[Node], default: &str, output: bool) -> Vec<AudioDevice> {
    let mut list = Vec::new();
    // Placas com alguma porta nesta direção (para saber se é preciso dizer a placa).
    let used: Vec<&Card> = cards.iter().filter(|c| c.ports.iter().any(|p| p.output == output && p.availability != Availability::No)).collect();
    let many_cards = used.len() > 1;
    for card in &used {
        let ports: Vec<&Port> = card.ports.iter().filter(|p| p.output == output && p.availability != Availability::No && !p.profiles.is_empty()).collect();
        let external = matches!(card.bus.as_str(), "usb" | "bluetooth");
        for port in &ports {
            let node = nodes.iter().find(|n| !n.monitor && n.card == Some(card.index) && (n.active_port.as_deref() == Some(port.name.as_str()) || (n.active_port.is_none() && n.ports.contains(&port.name))));
            // O nó pode existir com outra porta ativa (placas HDA comuns: um sink com
            // "alto-falante" e "fones" como portas).
            let node = node.or_else(|| nodes.iter().find(|n| !n.monitor && n.card == Some(card.index) && n.ports.contains(&port.name)));
            let label = port_label(port);
            let (name, mut detail) = if external {
                // USB e Bluetooth: o nome do aparelho; a porta só se houver mais de uma.
                (card.description.clone(), if ports.len() > 1 { label } else { String::new() })
            } else if let Some(product) = &port.product {
                // HDMI com o monitor conhecido: o nome dele, e o conector embaixo.
                (product.clone(), label)
            } else {
                (label, String::new())
            };
            if many_cards && !external {
                detail = if detail.is_empty() { card.description.clone() } else { format!("{detail} · {}", card.description) };
            }
            let on_port = node.is_some_and(|n| n.active_port.as_deref().is_none_or(|p| p == port.name));
            list.push(AudioDevice {
                key: format!("{}|{}", card.name, port.name),
                name,
                detail,
                kind: kind_of(&port.kind, &card.bus, &card.form_factor, output),
                default: node.is_some_and(|n| n.name == default) && on_port,
                volume: node.map(|n| n.volume).unwrap_or(1.0),
                card: card.name.clone(),
                port: port.name.clone(),
                profile: profile_for(card, port),
                node: node.map(|n| n.name.clone()).unwrap_or_default(),
                via_wpctl: false,
                plugged: port.availability == Availability::Yes,
            });
        }
    }
    // Nós que não vêm de porta de placa (saídas virtuais, rede, placas sem portas).
    for n in nodes.iter().filter(|n| !n.monitor) {
        let from_card = n.card.and_then(|i| used.iter().find(|c| c.index == i)).is_some_and(|c| c.ports.iter().any(|p| p.output == output && (n.ports.contains(&p.name) || n.active_port.as_deref() == Some(p.name.as_str()))));
        if from_card {
            continue;
        }
        list.push(AudioDevice {
            key: n.name.clone(),
            name: n.description.clone(),
            detail: String::new(),
            kind: kind_of("", &n.bus, &n.form_factor, output),
            default: n.name == default,
            volume: n.volume,
            node: n.name.clone(),
            ..Default::default()
        });
    }
    list.sort_by(|a, b| a.kind.rank().cmp(&b.kind.rank()).then_with(|| a.name.cmp(&b.name)));
    list
}

/// Para onde ir quando os conectores mudam (como o Mac): fones ligados agora passam a
/// ser o padrão; se o padrão sumiu (fones tirados), volta ao melhor que sobrou (os
/// alto-falantes). `before` é a leitura anterior (vazia na primeira: aí só corrige um
/// padrão que aponta para um conector sem nada ligado). `default_gone`: o padrão de
/// agora não está na lista (porta indisponível).
pub fn follow(before: &[AudioDevice], now: &[AudioDevice], default_gone: bool) -> Option<AudioDevice> {
    if now.is_empty() {
        return None;
    }
    if !before.is_empty() {
        let plugged = now.iter().find(|d| matches!(d.kind, AudioKind::Headphones | AudioKind::Headset) && d.plugged && !d.default && !before.iter().any(|b| b.key == d.key));
        if let Some(d) = plugged {
            return Some(d.clone());
        }
    }
    if default_gone && !now.iter().any(|d| d.default) {
        // Fones com algo ligado primeiro (o conector diz que há), depois o que é do
        // próprio computador (alto-falantes, microfone interno).
        return now.iter().min_by_key(|d| match d.kind {
            AudioKind::Headphones | AudioKind::Headset if d.plugged => 0,
            AudioKind::Speaker | AudioKind::Mic => 1,
            AudioKind::Headphones | AudioKind::Headset | AudioKind::Usb | AudioKind::Bluetooth => 2,
            AudioKind::Hdmi => 3,
            AudioKind::Other => 4,
        }).cloned();
    }
    None
}

/// O padrão de agora aponta para uma porta indisponível (ou para nada)? Ex.: o perfil
/// "Headphones" ativo sem fone ligado (o caso do Alder Lake com SOF).
pub fn default_gone(cards: &[Card], nodes: &[Node], default: &str) -> bool {
    let Some(node) = nodes.iter().find(|n| n.name == default) else {
        return !default.is_empty() || nodes.iter().any(|n| !n.monitor);
    };
    let Some(card) = node.card.and_then(|i| cards.iter().find(|c| c.index == i)) else {
        return false;
    };
    node.active_port.as_ref().and_then(|p| card.ports.iter().find(|cp| cp.name == *p)).is_some_and(|p| p.availability == Availability::No)
}

/// Comandos para escolher um aparelho, em ordem. O nó do perfil novo só existe depois
/// da troca: `NODE` é trocado pelo nome achado em seguida (ver `system::select_audio`).
pub fn select_commands(d: &AudioDevice, output: bool) -> Vec<Vec<String>> {
    let mut cmds = Vec::new();
    if let Some(profile) = &d.profile {
        cmds.push(vec!["set-card-profile".into(), d.card.clone(), profile.clone()]);
    }
    let node = if d.node.is_empty() || d.profile.is_some() { "NODE".to_string() } else { d.node.clone() };
    if !d.port.is_empty() {
        cmds.push(vec![if output { "set-sink-port" } else { "set-source-port" }.into(), node.clone(), d.port.clone()]);
    }
    cmds.push(vec![if output { "set-default-sink" } else { "set-default-source" }.into(), node]);
    cmds
}

/// Nó (sink/source) de uma placa com a porta, numa leitura nova.
pub fn node_for(nodes: &[Node], cards: &[Card], card: &str, port: &str) -> Option<String> {
    let index = cards.iter().find(|c| c.name == card)?.index;
    nodes
        .iter()
        .filter(|n| !n.monitor && n.card == Some(index))
        .find(|n| n.active_port.as_deref() == Some(port))
        .or_else(|| nodes.iter().find(|n| !n.monitor && n.card == Some(index) && n.ports.iter().any(|p| p == port)))
        .map(|n| n.name.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    const CARDS: &str = include_str!("testdata/pactl-cards.json");
    const SINKS: &str = include_str!("testdata/pactl-sinks.json");
    const SINKS_HEADPHONES: &str = include_str!("testdata/pactl-sinks-headphones.json");
    const SOURCES: &str = include_str!("testdata/pactl-sources.json");
    const SPEAKER_SINK: &str = "alsa_output.pci-0000_00_1f.3-platform-skl_hda_dsp_generic.HiFi__Speaker__sink";
    const HEADPHONES_SINK: &str = "alsa_output.pci-0000_00_1f.3-platform-skl_hda_dsp_generic.HiFi__Headphones__sink";
    const PROFILE_SPEAKER: &str = "HiFi (HDMI1, HDMI2, HDMI3, Mic1, Mic2, Speaker)";
    const PROFILE_HEADPHONES: &str = "HiFi (HDMI1, HDMI2, HDMI3, Headphones, Mic1, Mic2)";

    #[test]
    fn le_placa_sof_com_dois_perfis() {
        let cards = parse_cards(CARDS);
        assert_eq!(cards.len(), 1);
        let c = &cards[0];
        assert_eq!(c.bus, "pci");
        assert_eq!(c.active_profile, PROFILE_SPEAKER);
        let speaker = c.ports.iter().find(|p| p.name == "[Out] Speaker").unwrap();
        assert_eq!(speaker.profiles, vec![PROFILE_SPEAKER.to_string()]);
        assert_eq!(speaker.kind, "speaker");
        let hdmi1 = c.ports.iter().find(|p| p.name == "[Out] HDMI1").unwrap();
        assert_eq!(hdmi1.product.as_deref(), Some("DELL U2717D"));
        assert_eq!(hdmi1.availability, Availability::Yes);
        assert!(!c.ports.iter().find(|p| p.name == "[In] Mic1").unwrap().output);
    }

    #[test]
    fn saidas_de_verdade_sem_o_nome_da_placa() {
        let cards = parse_cards(CARDS);
        let sinks = parse_nodes(SINKS);
        let outs = build(&cards, &sinks, SPEAKER_SINK, true);
        // HDMI2, HDMI3 e Headphones sem nada ligado: fora.
        let names: Vec<(&str, &str)> = outs.iter().map(|d| (d.name.as_str(), d.detail.as_str())).collect();
        assert_eq!(names, vec![("Alto-falantes", ""), ("DELL U2717D", "HDMI / DisplayPort 1")]);
        assert!(outs[0].default);
        assert_eq!(outs[0].kind, AudioKind::Speaker);
        assert_eq!(outs[0].profile, None);
        assert_eq!(outs[1].kind, AudioKind::Hdmi);
        assert!(!outs[1].default);
        assert!(outs.iter().all(|d| !d.name.contains("Alder Lake")));
    }

    #[test]
    fn perfil_dos_fones_ativo_sem_fone_mostra_os_alto_falantes() {
        // O caso do usuário: o perfil "Headphones" ativo (prioridade maior) e nada no
        // conector. Os alto-falantes aparecem (pedindo a troca de perfil) e o padrão
        // está numa porta sem nada ligado.
        let mut cards = parse_cards(CARDS);
        cards[0].active_profile = PROFILE_HEADPHONES.into();
        let sinks = parse_nodes(SINKS_HEADPHONES);
        let outs = build(&cards, &sinks, HEADPHONES_SINK, true);
        let speaker = outs.iter().find(|d| d.kind == AudioKind::Speaker).unwrap();
        assert_eq!(speaker.profile.as_deref(), Some(PROFILE_SPEAKER));
        assert!(speaker.node.is_empty());
        assert!(!outs.iter().any(|d| d.kind == AudioKind::Headphones));
        assert!(default_gone(&cards, &sinks, HEADPHONES_SINK));
        let to = follow(&[], &outs, true).unwrap();
        assert_eq!(to.kind, AudioKind::Speaker);
        let cmds = select_commands(&to, true);
        assert_eq!(cmds[0], vec!["set-card-profile".to_string(), cards[0].name.clone(), PROFILE_SPEAKER.into()]);
        assert_eq!(cmds[1][0], "set-sink-port");
        assert_eq!(cmds[2], vec!["set-default-sink".to_string(), "NODE".into()]);
        // Depois da troca, o nó dos alto-falantes é achado pela porta.
        assert_eq!(node_for(&parse_nodes(SINKS), &cards, &cards[0].name, "[Out] Speaker").as_deref(), Some(SPEAKER_SINK));
    }

    #[test]
    fn ligar_e_tirar_o_fone_acompanha() {
        let cards = parse_cards(CARDS);
        let sinks = parse_nodes(SINKS);
        let before = build(&cards, &sinks, SPEAKER_SINK, true);
        assert!(!default_gone(&cards, &sinks, SPEAKER_SINK));
        assert_eq!(follow(&before, &before, false), None);
        // Fone ligado: a porta passa a "available" e entra na lista.
        let mut plugged = cards.clone();
        plugged[0].ports.iter_mut().find(|p| p.name == "[Out] Headphones").unwrap().availability = Availability::Yes;
        let now = build(&plugged, &sinks, SPEAKER_SINK, true);
        assert_eq!(now[0].name, "Fones de ouvido");
        let to = follow(&before, &now, false).unwrap();
        assert_eq!(to.kind, AudioKind::Headphones);
        assert_eq!(to.profile.as_deref(), Some(PROFILE_HEADPHONES));
        // Fone tirado com o perfil dele ativo: volta aos alto-falantes.
        let mut on_hp = cards.clone();
        on_hp[0].active_profile = PROFILE_HEADPHONES.into();
        let hp_sinks = parse_nodes(SINKS_HEADPHONES);
        let after = build(&on_hp, &hp_sinks, HEADPHONES_SINK, true);
        let back = follow(&now, &after, default_gone(&on_hp, &hp_sinks, HEADPHONES_SINK)).unwrap();
        assert_eq!(back.kind, AudioKind::Speaker);
    }

    #[test]
    fn entradas_sem_os_monitores() {
        let cards = parse_cards(CARDS);
        let sources = parse_nodes(SOURCES);
        let ins = build(&cards, &sources, "alsa_input.pci-0000_00_1f.3-platform-skl_hda_dsp_generic.HiFi__Mic2__source", false);
        let names: Vec<&str> = ins.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, vec!["Microfone digital", "Microfone estéreo"]);
        assert!(ins[1].default);
        assert!(ins.iter().all(|d| d.kind == AudioKind::Mic));
        assert_eq!(select_commands(&ins[0], false)[0][0], "set-source-port");
    }

    #[test]
    fn duas_placas_e_bluetooth() {
        let mut cards = parse_cards(CARDS);
        let mut bt = cards[0].clone();
        bt.index = 90;
        bt.name = "bluez_card.00_11_22_33_44_55".into();
        bt.description = "Fone JBL".into();
        bt.bus = "bluetooth".into();
        bt.ports = vec![Port { name: "headset-output".into(), description: "Headset".into(), kind: "headset".into(), output: true, priority: 0, availability: Availability::Unknown, profiles: vec![bt.active_profile.clone()], product: None }];
        cards.push(bt);
        let outs = build(&cards, &parse_nodes(SINKS), SPEAKER_SINK, true);
        let fone = outs.iter().find(|d| d.kind == AudioKind::Bluetooth).unwrap();
        assert_eq!((fone.name.as_str(), fone.detail.as_str()), ("Fone JBL", ""));
        let speaker = outs.iter().find(|d| d.kind == AudioKind::Speaker).unwrap();
        assert_eq!(speaker.detail, "Alder Lake PCH-P High Definition Audio Controller");
    }

    #[test]
    fn saida_virtual_sem_placa() {
        let text = r#"[{"index":7,"name":"easyeffects_sink","description":"Easy Effects Sink","mute":false,"volume":{"mono":{"value":32768}},"ports":[],"properties":{"media.class":"Audio/Sink"}}]"#;
        let outs = build(&[], &parse_nodes(text), "easyeffects_sink", true);
        assert_eq!(outs.len(), 1);
        assert_eq!(outs[0].name, "Easy Effects Sink");
        assert!(outs[0].default);
        assert!((outs[0].volume - 0.5).abs() < 1e-6);
    }
}
