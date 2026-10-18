//! Ícones de apps do tema do sistema, prontos para desenhar com tiny-skia.

use std::{fs, path::Path};

use tiny_skia::Pixmap;

/// Ícone de app pelo nome: no tema padrão (`hicolor`, onde os apps instalam os
/// ícones) e, se não houver, no tema do usuário (ícones de sistema, como o das
/// Configurações). Nomes de arquivo (caminho absoluto) também valem.
pub fn load_icon(name: &str, size: u32) -> Option<Pixmap> {
    if name.starts_with('/') {
        return load_icon_file(name, size);
    }
    load_icon_hicolor(name, size).or_else(|| themed(&[name], size))
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

/// Temas de ícones do usuário, em ordem: `MIDE_ICON_THEME`, o tema do GNOME
/// (gsettings) e o Adwaita.
fn user_themes() -> &'static [String] {
    static THEMES: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    THEMES.get_or_init(|| {
        let mut themes = Vec::new();
        if let Ok(theme) = std::env::var("MIDE_ICON_THEME") {
            themes.push(theme);
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
    for theme in user_themes() {
        for name in names {
            let path = freedesktop_icons::lookup(name)
                .with_size(size.min(u16::MAX as u32) as u16)
                .with_theme(theme)
                .with_cache()
                .find();
            if let Some(path) = path {
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
