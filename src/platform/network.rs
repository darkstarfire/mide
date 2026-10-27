//! Rede cabeada (Ethernet) vista pelo NetworkManager, e qual conexão é a principal
//! (a da rota padrão de menor métrica), para a barra mostrar o ícone certo como no
//! Mac: cabo conectado e principal = ícone de rede cabeada; senão, o do Wi-Fi.

/// Estado de uma placa de rede cabeada.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WiredState {
    Connected,
    Connecting,
    /// Cabo ligado, sem conexão ativa.
    Disconnected,
    /// Sem cabo (o NetworkManager diz "unavailable").
    Unplugged,
}

/// Placa de rede cabeada.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wired {
    /// Nome da interface ("enp44s0").
    pub device: String,
    /// Nome da conexão do NetworkManager ("Wired connection 1"), se houver.
    pub connection: String,
    pub state: WiredState,
    /// Endereço IPv4 sem a máscara, quando conectada.
    pub address: Option<String>,
}

/// Interfaces que não são placas de verdade (Docker, pontes, máquinas virtuais, VPN).
pub fn is_virtual(device: &str) -> bool {
    const PREFIXES: &[&str] = &["veth", "docker", "br-", "virbr", "vnet", "tap", "tun", "lo", "p2p", "vboxnet", "vmnet", "zt", "tailscale", "wg", "podman", "cni", "flannel", "lxc", "lxd"];
    PREFIXES.iter().any(|p| device.starts_with(p))
}

/// Lê `nmcli -t -f DEVICE,TYPE,STATE,CONNECTION device`: só as placas cabeadas de
/// verdade (sem interfaces virtuais e sem as que o NetworkManager não gerencia).
pub fn parse_wired(text: &str) -> Vec<Wired> {
    text.lines()
        .filter_map(|line| {
            let f = super::system::split_terse(line);
            if f.len() < 4 || f[1] != "ethernet" || is_virtual(&f[0]) {
                return None;
            }
            let state = match f[2].as_str() {
                "connected" => WiredState::Connected,
                s if s.starts_with("connecting") => WiredState::Connecting,
                "disconnected" | "deactivating" => WiredState::Disconnected,
                "unavailable" => WiredState::Unplugged,
                // "unmanaged", "connected (externally)" (feito por outro programa) etc.
                _ => return None,
            };
            Some(Wired { device: f[0].clone(), connection: f[3].clone(), state, address: None })
        })
        .collect()
}

/// Lê "IP4.ADDRESS[1]:192.168.10.17/24" do `nmcli -t -f IP4.ADDRESS device show X`.
pub fn parse_address(text: &str) -> Option<String> {
    text.lines().find_map(|l| {
        let (_, value) = l.split_once(':')?;
        let ip = value.split('/').next()?.trim();
        (!ip.is_empty()).then(|| ip.to_string())
    })
}

/// Interface da rota padrão de menor métrica (`ip route show default`): a conexão
/// principal, por onde a internet sai.
pub fn parse_primary(text: &str) -> Option<String> {
    text.lines()
        .filter_map(|l| {
            let words: Vec<&str> = l.split_whitespace().collect();
            let field = |name: &str| words.iter().position(|w| *w == name).and_then(|i| words.get(i + 1)).copied();
            let dev = field("dev")?;
            let metric: u32 = field("metric").and_then(|m| m.parse().ok()).unwrap_or(0);
            Some((metric, dev.to_string()))
        })
        .min_by_key(|(m, _)| *m)
        .map(|(_, d)| d)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Saída real do notebook do usuário (cabo e Wi-Fi, mais Docker).
    const DEVICES: &str = "enp44s0:ethernet:connected:Wired connection 1\nwlp43s0:wifi:connected:Gavom\nbr-0688f299f3bc:bridge:connected (externally):br-0688f299f3bc\ndocker0:bridge:connected (externally):docker0\nlo:loopback:connected (externally):lo\np2p-dev-wlp43s0:wifi-p2p:disconnected:\nveth3ec711f:ethernet:unmanaged:\nveth48a1fce:ethernet:unmanaged:\n";
    const ROUTES: &str = "default via 192.168.10.1 dev enp44s0 proto dhcp src 192.168.10.17 metric 100 \ndefault via 192.168.30.1 dev wlp43s0 proto dhcp src 192.168.30.168 metric 600\n";

    #[test]
    fn so_o_cabo_de_verdade() {
        let list = parse_wired(DEVICES);
        assert_eq!(list, vec![Wired { device: "enp44s0".into(), connection: "Wired connection 1".into(), state: WiredState::Connected, address: None }]);
        assert_eq!(parse_wired("eth0:ethernet:unavailable:\n")[0].state, WiredState::Unplugged);
        assert_eq!(parse_wired("eth0:ethernet:connecting (getting IP configuration):Casa\n")[0].state, WiredState::Connecting);
    }

    #[test]
    fn rota_principal_pela_metrica() {
        assert_eq!(parse_primary(ROUTES).as_deref(), Some("enp44s0"));
        assert_eq!(parse_primary("default via 10.0.0.1 dev wlan0 metric 600\ndefault via 10.0.0.2 dev eth0 metric 20100\n").as_deref(), Some("wlan0"));
        assert_eq!(parse_primary(""), None);
    }

    #[test]
    fn endereco_sem_mascara() {
        assert_eq!(parse_address("IP4.ADDRESS[1]:192.168.10.17/24\n").as_deref(), Some("192.168.10.17"));
        assert_eq!(parse_address(""), None);
    }
}
