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
    /// Diagonal em polegadas escolhida pelo usuário (`diagonal` no config), para telas
    /// que não informam o tamanho de verdade.
    pub diagonal: Option<f64>,
}

/// Diagonais oferecidas em "Tamanho da tela" (polegadas).
pub const DIAGONALS: [f64; 12] = [13.3, 14.0, 15.6, 17.0, 19.0, 21.5, 23.8, 24.0, 27.0, 32.0, 34.0, 43.0];

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
        diagonal: None,
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

    /// Tamanho que o monitor informa (`530x300 mm`), se for de verdade: alguns
    /// monitores e TVs informam só a proporção (`160x90`, `16x9`) ou nada.
    pub fn reported_mm(&self) -> Option<(f32, f32)> {
        let text = self.description.trim().strip_suffix("mm")?.trim();
        let (w, h) = text.split_once('x')?;
        let (w, h): (f32, f32) = (w.trim().parse().ok()?, h.trim().parse().ok()?);
        let aspect_only = [(160.0, 90.0), (160.0, 100.0), (16.0, 9.0), (16.0, 10.0), (4.0, 3.0), (5.0, 4.0)].contains(&(w, h));
        // Plausível: maior que 10 cm de largura e com a proporção da resolução.
        let (pw, ph) = (self.current.0.max(1) as f32, self.current.1.max(1) as f32);
        let same_shape = ((w / h) / (pw / ph) - 1.0).abs() < 0.2;
        (!aspect_only && w >= 100.0 && h >= 60.0 && same_shape).then_some((w, h))
    }

    /// Tamanho físico para o arranjo, em milímetros: pela diagonal escolhida, pelo que
    /// o monitor informa ou, sem nada confiável, estimado pela resolução (96 pontos por
    /// polegada).
    pub fn physical_mm(&self) -> (f32, f32) {
        let (pw, ph) = (self.current.0.max(1) as f32, self.current.1.max(1) as f32);
        if let Some(d) = self.diagonal.filter(|d| *d > 1.0) {
            let diag_px = (pw * pw + ph * ph).sqrt();
            let mm_per_px = d as f32 * 25.4 / diag_px;
            return (pw * mm_per_px, ph * mm_per_px);
        }
        if let Some(mm) = self.reported_mm() {
            return mm;
        }
        let (lw, lh) = self.logical_size();
        (lw as f32 * 25.4 / 96.0, lh as f32 * 25.4 / 96.0)
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
        if let Some(diagonal) = d.diagonal {
            t["diagonal"] = value(diagonal);
        }
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
    let mut list: Vec<Display> =
        crate::control_client::ask_compositor("outputs").unwrap_or_default().iter().filter_map(|l| parse_line(l)).collect();
    // A diagonal escolhida fica só no config (o compositor não precisa dela).
    let text = crate::settings_file::config_path().and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
    if let Ok(doc) = text.parse::<DocumentMut>() {
        for d in &mut list {
            d.diagonal = doc
                .get("monitors")
                .and_then(|m| m.get("outputs"))
                .and_then(|o| o.get(&d.name))
                .and_then(|t| t.get("diagonal"))
                .and_then(|v| v.as_float().or_else(|| v.as_integer().map(|n| n as f64)));
        }
    }
    list
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

    #[test]
    fn tamanho_fisico_da_tela() {
        let mut d = parse_line(LINE).unwrap();
        // 530x300 mm informados, com a proporção de 1920x1080: valem.
        assert_eq!(d.physical_mm(), (530.0, 300.0));
        // 160x90 é só a proporção: estima pela resolução (96 pontos por polegada).
        d.description = "160x90 mm".into();
        let (w, _) = d.physical_mm();
        assert!((w - 508.0).abs() < 1.0, "w = {w}");
        // Com a diagonal escolhida (27"): 597,7 mm de largura.
        d.diagonal = Some(27.0);
        let (w, h) = d.physical_mm();
        assert!((w - 597.7).abs() < 1.0 && (h - 336.2).abs() < 1.0, "{w}x{h}");
    }
}
