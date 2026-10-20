//! Telas para as Configurações → Monitores: lê a lista do compositor (`outputs`),
//! monta as opções (resolução, frequência, escala) e grava a seção `[monitors]` do
//! `config.toml`. Lógica pura, testável sem interface.

use toml_edit::{value, DocumentMut, Item, Table};

/// Uma tela como o compositor informa.
#[derive(Debug, Clone, PartialEq)]
pub struct Display {
    pub name: String,
    pub description: String,
    /// Modos aceitos: largura, altura e frequência em Hz.
    pub modes: Vec<(i32, i32, f64)>,
    pub current: (i32, i32, f64),
    pub position: (i32, i32),
    pub scale: f64,
    pub enabled: bool,
    pub primary: bool,
}

/// Escalas oferecidas.
pub const SCALES: [f64; 5] = [1.0, 1.25, 1.5, 1.75, 2.0];

/// Lê uma linha do comando `outputs` do compositor.
pub fn parse_line(line: &str) -> Option<Display> {
    let f: Vec<&str> = line.split('\t').collect();
    if f.len() < 8 {
        return None;
    }
    let (x, y) = f[3].split_once(',')?;
    Some(Display {
        name: f[0].to_string(),
        description: if f[1] == "-" { String::new() } else { f[1].to_string() },
        current: parse_mode(f[2])?,
        position: (x.parse().ok()?, y.parse().ok()?),
        scale: f[4].parse().ok()?,
        enabled: f[5] == "ligada",
        primary: f[6] == "principal",
        modes: f[7].split_whitespace().filter_map(parse_mode).collect(),
    })
}

/// `1920x1080@60` → (1920, 1080, 60.0).
fn parse_mode(text: &str) -> Option<(i32, i32, f64)> {
    let (size, hz) = text.split_once('@')?;
    let (w, h) = size.split_once('x')?;
    Some((w.parse().ok()?, h.parse().ok()?, hz.parse().ok()?))
}

impl Display {
    /// Resoluções distintas, da maior para a menor.
    pub fn sizes(&self) -> Vec<(i32, i32)> {
        let mut sizes: Vec<(i32, i32)> = Vec::new();
        for (w, h, _) in &self.modes {
            if !sizes.contains(&(*w, *h)) {
                sizes.push((*w, *h));
            }
        }
        sizes.sort_by_key(|(w, h)| std::cmp::Reverse((*w as i64) * (*h as i64)));
        sizes
    }

    /// Frequências de uma resolução, da maior para a menor (sem repetir).
    pub fn rates(&self, size: (i32, i32)) -> Vec<f64> {
        let mut rates: Vec<f64> = self.modes.iter().filter(|(w, h, _)| (*w, *h) == size).map(|m| m.2).collect();
        rates.sort_by(|a, b| b.total_cmp(a));
        rates.dedup_by(|a, b| (*a - *b).abs() < 0.5);
        rates
    }

    /// Tamanho no arranjo (pixels lógicos: o modo dividido pela escala).
    pub fn logical_size(&self) -> (i32, i32) {
        let s = if self.scale >= 0.5 { self.scale } else { 1.0 };
        ((self.current.0 as f64 / s).round() as i32, (self.current.1 as f64 / s).round() as i32)
    }
}

/// Rótulos das opções.
pub fn size_label((w, h): (i32, i32)) -> String {
    format!("{w} × {h}")
}

pub fn rate_label(hz: f64) -> String {
    if (hz - hz.round()).abs() < 0.01 { format!("{} Hz", hz.round() as i32) } else { format!("{hz:.2} Hz") }
}

pub fn scale_label(scale: f64) -> String {
    format!("{}%", (scale * 100.0).round() as i32)
}

/// Seção `[monitors]` inteira trocada pelas telas dadas, sem mexer no resto do arquivo.
/// As posições são deslocadas para a tela mais à esquerda e mais acima ficar em 0.
pub fn write_monitors(text: &str, displays: &[Display]) -> Result<String, String> {
    let mut doc: DocumentMut = text.parse().map_err(|e| format!("config.toml inválido: {e}"))?;
    let enabled: Vec<&Display> = displays.iter().filter(|d| d.enabled).collect();
    let min_x = enabled.iter().map(|d| d.position.0).min().unwrap_or(0);
    let min_y = enabled.iter().map(|d| d.position.1).min().unwrap_or(0);
    let mut monitors = Table::new();
    if let Some(primary) = displays.iter().find(|d| d.primary) {
        monitors["primary"] = value(primary.name.clone());
    }
    let mut outputs = Table::new();
    outputs.set_implicit(true);
    for d in displays {
        let mut t = Table::new();
        if !d.enabled {
            t["enabled"] = value(false);
        }
        t["mode"] = value(format!("{}x{}@{}", d.current.0, d.current.1, trim_hz(d.current.2)));
        t["scale"] = value(d.scale);
        if d.enabled {
            t["x"] = value((d.position.0 - min_x) as i64);
            t["y"] = value((d.position.1 - min_y) as i64);
        }
        outputs[&d.name] = Item::Table(t);
    }
    monitors["outputs"] = Item::Table(outputs);
    doc["monitors"] = Item::Table(monitors);
    Ok(doc.to_string())
}

/// Grava as telas no `config.toml` do usuário.
pub fn save(displays: &[Display]) -> Result<(), String> {
    let path = crate::settings_file::config_path().ok_or("sem pasta de configuração")?;
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let updated = write_monitors(&text, displays)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, updated).map_err(|e| e.to_string())?;
    tracing::info!("config.toml: [monitors] atualizado");
    Ok(())
}

/// Telas que o compositor informa agora.
pub fn load() -> Vec<Display> {
    crate::control_client::ask_compositor("outputs").unwrap_or_default().iter().filter_map(|l| parse_line(l)).collect()
}

fn trim_hz(hz: f64) -> String {
    if (hz - hz.round()).abs() < 0.01 { format!("{}", hz.round() as i32) } else { format!("{hz:.2}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINE: &str = "DP-1\t530x300 mm\t1920x1080@60\t0,0\t1\tligada\tprincipal\t1920x1080@60 1920x1080@59.94 1280x720@60 2560x1440@144";

    #[test]
    fn le_a_linha_do_compositor() {
        let d = parse_line(LINE).unwrap();
        assert_eq!(d.name, "DP-1");
        assert_eq!(d.current, (1920, 1080, 60.0));
        assert!(d.enabled && d.primary);
        assert_eq!(d.sizes(), vec![(2560, 1440), (1920, 1080), (1280, 720)]);
        assert_eq!(d.rates((1920, 1080)), vec![60.0]);
        assert_eq!(rate_label(59.94), "59.94 Hz");
        assert_eq!(scale_label(1.25), "125%");
        assert!(parse_line("lixo").is_none());
    }

    #[test]
    fn grava_a_secao_de_telas_sem_mexer_no_resto() {
        let mut a = parse_line(LINE).unwrap();
        a.position = (1366, 0);
        let mut b = a.clone();
        b.name = "eDP-1".into();
        b.primary = false;
        b.position = (0, 312);
        b.scale = 1.25;
        let text = "# meus atalhos\n[effects]\nlamp = false\n";
        let out = write_monitors(text, &[a, b]).unwrap();
        assert!(out.contains("# meus atalhos") && out.contains("lamp = false"));
        let doc: DocumentMut = out.parse().unwrap();
        assert_eq!(doc["monitors"]["primary"].as_str(), Some("DP-1"));
        // Deslocadas para começar em 0.
        assert_eq!(doc["monitors"]["outputs"]["DP-1"]["x"].as_integer(), Some(1366));
        assert_eq!(doc["monitors"]["outputs"]["eDP-1"]["y"].as_integer(), Some(312));
        assert_eq!(doc["monitors"]["outputs"]["eDP-1"]["scale"].as_float(), Some(1.25));
        assert_eq!(doc["monitors"]["outputs"]["DP-1"]["mode"].as_str(), Some("1920x1080@60"));
    }
}
