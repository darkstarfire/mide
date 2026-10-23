//! Ícones de apps do tema do sistema, prontos para desenhar com tiny-skia.

use std::{fs, path::Path};

use tiny_skia::Pixmap;

/// Nome do tema de ícones do mide (o fork do WhiteSur, `mide-icons`, instalado pelo
/// `mide-atualizar`).
pub const MIDE_THEME: &str = "mide";

/// Ícone de app pelo nome: primeiro no tema do mide (quando instalado: os apps de
/// terceiros ganham a versão no estilo dele, como no GTK), depois no tema padrão
/// (`hicolor`, onde os apps instalam os ícones) e nos temas do usuário. Os apps do
/// mide (`mide-terminal`...) caem num ícone genérico sem o tema do mide. Nomes de
/// arquivo (caminho absoluto) também valem.
pub fn load_icon(name: &str, size: u32) -> Option<Pixmap> {
    if name.starts_with('/') {
        return load_icon_file(name, size);
    }
    let in_mide = || {
        let icon = mide_theme_installed().then(|| in_theme(MIDE_THEME, &[name], size)).flatten();
        if icon.is_some() {
            tracing::debug!("ícone {name}: tema do mide");
        }
        icon
    };
    in_mide()
        .or_else(|| load_icon_hicolor(name, size))
        .or_else(|| themed(&[name], size))
        .or_else(|| generic_name(name).and_then(|g| load_icon_hicolor(g, size).or_else(|| themed(&[g], size))))
}

/// Ícone genérico de um app do mide, para quando o tema do mide não está instalado.
pub fn generic_name(name: &str) -> Option<&'static str> {
    Some(match name {
        "mide-terminal" => "utilities-terminal",
        "mide-text" => "accessories-text-editor",
        "mide-files" => "system-file-manager",
        "mide-settings" => "preferences-system",
        "mide-screenshot" | "mide-screenshot-editor" => "applets-screenshooter",
        "mide-calculator" => "accessories-calculator",
        "mide-about" => "computer",
        _ => return None,
    })
}

/// O tema do mide está instalado (em `~/.local/share/icons` ou nas pastas de dados
/// do sistema)?
pub fn mide_theme_installed() -> bool {
    static INSTALLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *INSTALLED.get_or_init(|| {
        let home = std::env::var_os("XDG_DATA_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/share")));
        let system = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
        home.into_iter()
            .chain(system.split(':').filter(|d| !d.is_empty()).map(std::path::PathBuf::from))
            .any(|dir| dir.join("icons").join(MIDE_THEME).join("index.theme").is_file())
    })
}

/// Ícone do tema padrão: PNG direto, SVG renderizado.
fn load_icon_hicolor(name: &str, size: u32) -> Option<Pixmap> {
    let path = freedesktop_icons::lookup(name)
        .with_size(size.min(u16::MAX as u32) as u16)
        .with_cache()
        .find()?;
    match path.extension().and_then(|e| e.to_str()) {
        Some("png") => icon_from_png(&path, size),
        Some("svg") | Some("svgz") => icon_from_svg(&path, size),
        _ => None,
    }
}

/// Ícone de um arquivo (PNG, JPEG ou SVG), ajustado a `size`. Aceita `file://`.
pub fn load_icon_file(path: &str, size: u32) -> Option<Pixmap> {
    let path = Path::new(path.strip_prefix("file://").unwrap_or(path));
    match path.extension().and_then(|e| e.to_str()).map(str::to_lowercase).as_deref() {
        Some("svg") | Some("svgz") => icon_from_svg(path, size),
        Some(_) => icon_from_png(path, size),
        None => None,
    }
}

/// Temas de ícones do usuário, em ordem: `MIDE_ICON_THEME`, o tema do mide (se
/// instalado), o tema do GNOME (gsettings) e o Adwaita.
fn user_themes() -> &'static [String] {
    static THEMES: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    THEMES.get_or_init(|| {
        let mut themes = Vec::new();
        if let Ok(theme) = std::env::var("MIDE_ICON_THEME") {
            themes.push(theme);
        }
        if mide_theme_installed() {
            themes.push(MIDE_THEME.into());
        }
        let gnome = std::process::Command::new("gsettings")
            .args(["get", "org.gnome.desktop.interface", "icon-theme"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().trim_matches('\'').to_string())
            .filter(|t| !t.is_empty());
        themes.extend(gnome);
        themes.push("Adwaita".into());
        themes.dedup();
        themes
    })
}

/// Ícone de lugar ou de tipo de arquivo (pasta, lixeira, imagem...), procurado no
/// tema do usuário: esses ícones não ficam no tema padrão `hicolor`. Tenta cada
/// nome em ordem e devolve o primeiro que existir.
pub fn load_themed_icon(names: &[&str], size: u32) -> Option<Pixmap> {
    themed(names, size).or_else(|| names.iter().find_map(|n| load_icon_hicolor(n, size)))
}

/// Primeiro dos nomes encontrado nos temas do usuário.
fn themed(names: &[&str], size: u32) -> Option<Pixmap> {
    user_themes().iter().find_map(|theme| in_theme(theme, names, size))
}

/// Primeiro dos nomes encontrado em `theme` (sem cair no `hicolor`).
fn in_theme(theme: &str, names: &[&str], size: u32) -> Option<Pixmap> {
    for name in names {
        // Nos tamanhos pequenos (16 a 24), temas como o do mide (do WhiteSur) trazem
        // desenhos só de contorno na cor do texto: a pasta da lista da dock parecia um
        // símbolo. Um ícone colorido vem do desenho grande, reduzido para `size`; só os
        // "-symbolic" são de contorno de propósito.
        let lookup = if name.ends_with("-symbolic") { size } else { size.max(48) };
        let path = freedesktop_icons::lookup(name)
            .with_size(lookup.min(u16::MAX as u32) as u16)
            .with_theme(theme)
            .with_cache()
            .find();
        if let Some(path) = path {
            // O `hicolor` entra na busca de qualquer tema como herança: aqui só vale o
            // do próprio tema.
            if path.components().any(|c| c.as_os_str() == "hicolor") && theme != "hicolor" {
                continue;
            }
            let icon = match path.extension().and_then(|e| e.to_str()) {
                Some("png") => icon_from_png(&path, size),
                Some("svg") | Some("svgz") => icon_from_svg(&path, size),
                _ => None,
            };
            if icon.is_some() {
                return icon;
            }
        }
    }
    None
}

/// Retângulo (x, y, largura, altura) dos pixels visíveis de uma imagem RGBA.
/// Ícones de tema costumam ter margem transparente de tamanhos diferentes; sem
/// cortá-la, cada ícone parece de um tamanho e os espaços em volta ficam desiguais.
fn visible_bounds(rgba: &[u8], width: u32, height: u32) -> Option<(u32, u32, u32, u32)> {
    let (mut x0, mut y0, mut x1, mut y1) = (width, height, 0u32, 0u32);
    for y in 0..height {
        for x in 0..width {
            if rgba[((y * width + x) * 4 + 3) as usize] > 8 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    (x1 > x0 && y1 > y0).then(|| (x0, y0, x1 - x0, y1 - y0))
}

/// PNG com a parte visível ajustada ao tamanho do ícone, centralizada.
fn icon_from_png(path: &Path, size: u32) -> Option<Pixmap> {
    let img = image::open(path).ok()?.to_rgba8();
    let (bx, by, bw, bh) = visible_bounds(img.as_raw(), img.width(), img.height())?;
    let visible = image::imageops::crop_imm(&img, bx, by, bw, bh).to_image();
    let k = size as f32 / bw.max(bh) as f32;
    let (w, h) = (((bw as f32 * k).round() as u32).max(1), ((bh as f32 * k).round() as u32).max(1));
    let resized = image::imageops::resize(&visible, w, h, image::imageops::FilterType::Lanczos3);
    let (ox, oy) = ((size - w) / 2, (size - h) / 2);
    let mut pixmap = Pixmap::new(size, size)?;
    let data = pixmap.data_mut();
    for (x, y, px) in resized.enumerate_pixels() {
        let i = (((y + oy) * size + x + ox) * 4) as usize;
        // tiny-skia guarda pixels premultiplicados.
        let a = px[3] as u32;
        data[i] = (px[0] as u32 * a / 255) as u8;
        data[i + 1] = (px[1] as u32 * a / 255) as u8;
        data[i + 2] = (px[2] as u32 * a / 255) as u8;
        data[i + 3] = px[3];
    }
    Some(pixmap)
}

/// Renderiza o SVG com a parte visível ajustada ao tamanho do ícone. Primeiro
/// renderiza grande para achar a parte visível, depois renderiza de novo direto no
/// tamanho final (fica nítido). O resvg usa a própria cópia do tiny-skia; só os
/// bytes premultiplicados são copiados, então as versões não precisam bater.
fn icon_from_svg(path: &Path, size: u32) -> Option<Pixmap> {
    // Grande o bastante para medir a margem com precisão, sem pesar.
    let probe_size = size * 2;
    let data = fs::read(path).ok()?;
    let tree = resvg::usvg::Tree::from_data(&data, &resvg::usvg::Options::default()).ok()?;
    let svg_size = tree.size();
    let s0 = probe_size as f32 / svg_size.width().max(svg_size.height());
    let mut probe = resvg::tiny_skia::Pixmap::new(probe_size, probe_size)?;
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(s0, s0), &mut probe.as_mut());
    let (bx, by, bw, bh) = visible_bounds(probe.data(), probe_size, probe_size)?;

    let k = size as f32 / bw.max(bh) as f32;
    let ox = (size as f32 - bw as f32 * k) / 2.0 - bx as f32 * k;
    let oy = (size as f32 - bh as f32 * k) / 2.0 - by as f32 * k;
    let mut rendered = resvg::tiny_skia::Pixmap::new(size, size)?;
    let transform = resvg::tiny_skia::Transform::from_translate(ox, oy).pre_scale(s0 * k, s0 * k);
    resvg::render(&tree, transform, &mut rendered.as_mut());
    let mut pixmap = Pixmap::new(size, size)?;
    pixmap.data_mut().copy_from_slice(rendered.data());
    Some(pixmap)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bancada: `cargo test --lib bancada_icones -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn bancada_icones() {
        for (names, size) in [(&["user-home-symbolic"][..], 18), (&["folder", "text-x-generic"][..], 18), (&["folder", "text-x-generic"][..], 64), (&["text-x-generic"][..], 18), (&["application-pdf", "text-x-generic"][..], 18), (&["folder-download-symbolic"][..], 18)] {
            let t = std::time::Instant::now();
            let found = load_themed_icon(names, size).is_some();
            let first = t.elapsed();
            let t = std::time::Instant::now();
            load_themed_icon(names, size);
            println!("{:?}@{size}: {found} primeira {:.1} ms, de novo {:.1} ms", names[0], first.as_secs_f32() * 1e3, t.elapsed().as_secs_f32() * 1e3);
        }
    }
}
