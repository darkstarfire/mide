//! Miniaturas dos arquivos (Arquivos e prévia), como no Finder:
//!
//! - Imagens (PNG, JPEG, SVG): a própria imagem reduzida.
//! - Texto (`files::is_text`): uma folha branca com as primeiras linhas do arquivo.
//! - O resto (PDF, vídeo, fontes, WebP, AppImage...): os geradores de miniatura do
//!   sistema (padrão freedesktop, `/usr/share/thumbnailers/*.thumbnailer`), com o
//!   resultado guardado em `~/.cache/mide/thumbnails` (chave: caminho, tamanho do
//!   arquivo e data de modificação; muda o arquivo, gera de novo).
//!
//! Tudo aqui é lento (abre arquivos, roda programas): chamar fora da linha da janela.

use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    process::Command,
    sync::OnceLock,
    time::{Duration, Instant},
};

use tiny_skia::{Color, Pixmap};

use crate::ui::{draw, text::Text};

/// Maior arquivo de texto lido para a folha (só o começo importa).
const TEXT_LIMIT: u64 = 4 << 20;

/// Miniatura de `path` cabendo num quadrado de `size` pixels, ou `None` (sem
/// miniatura: o ícone do tipo de arquivo fica no lugar).
pub fn make(path: &Path, size: u32) -> Option<Pixmap> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || size < 16 {
        return None;
    }
    if crate::places::is_image(path) || extension(path).is_some_and(|e| e == "svg") {
        return crate::icons::load_icon_file(&path.to_string_lossy(), size);
    }
    if meta.len() <= TEXT_LIMIT && crate::files::is_text(path) {
        return text_page(path, size);
    }
    let png = system_thumbnail(path, &meta, if size > 128 { 256 } else { 128 })?;
    crate::icons::load_icon_file(&png.to_string_lossy(), size)
}

fn extension(path: &Path) -> Option<String> {
    path.extension().and_then(|e| e.to_str()).map(str::to_lowercase)
}

// ------------------------------------------------------------------- texto

/// Folha de papel com as primeiras linhas do arquivo, em letra pequena (como os
/// ícones de texto do Finder): proporção de página, borda fina e sombra leve.
pub fn text_page(path: &Path, size: u32) -> Option<Pixmap> {
    let bytes = read_start(path, 8 << 10)?;
    let text = String::from_utf8_lossy(&bytes);
    let mut pixmap = Pixmap::new(size, size)?;
    let s = size as f32;
    // Folha em pé, como os documentos do Finder: canto de cima à direita dobrado.
    let (pw, ph) = (s * 0.74, s * 0.94);
    let (px, py) = ((s - pw) / 2.0, (s - ph) / 2.0);
    let r = s * 0.035;
    let fold = pw * 0.24;
    let page = {
        let mut pb = tiny_skia::PathBuilder::new();
        pb.move_to(px + r, py);
        pb.line_to(px + pw - fold, py);
        pb.line_to(px + pw, py + fold);
        pb.line_to(px + pw, py + ph - r);
        pb.quad_to(px + pw, py + ph, px + pw - r, py + ph);
        pb.line_to(px + r, py + ph);
        pb.quad_to(px, py + ph, px, py + ph - r);
        pb.line_to(px, py + r);
        pb.quad_to(px, py, px + r, py);
        pb.close();
        pb.finish()?
    };
    // Sombra no formato da folha (sem o canto dobrado): camadas finas deslocadas
    // para baixo. Uma sombra retangular aparecia no canto como uma segunda orelha.
    for i in 1..=4 {
        let mut shade = tiny_skia::Paint::default();
        shade.anti_alias = true;
        shade.set_color(Color::from_rgba8(0, 0, 0, 14));
        let d = i as f32 * s * 0.004;
        pixmap.fill_path(&page, &shade, tiny_skia::FillRule::Winding, draw::tf().pre_translate(0.0, d), None);
    }
    let mut paint = tiny_skia::Paint::default();
    paint.anti_alias = true;
    // Branco com um cinza muito leve embaixo (a folha tem volume).
    paint.shader = tiny_skia::LinearGradient::new(
        tiny_skia::Point::from_xy(0.0, py),
        tiny_skia::Point::from_xy(0.0, py + ph),
        vec![
            tiny_skia::GradientStop::new(0.0, Color::from_rgba8(255, 255, 255, 255)),
            tiny_skia::GradientStop::new(1.0, Color::from_rgba8(242, 242, 244, 255)),
        ],
        tiny_skia::SpreadMode::Pad,
        tiny_skia::Transform::identity(),
    )?;
    pixmap.fill_path(&page, &paint, tiny_skia::FillRule::Winding, draw::tf(), None);
    let mut edge = tiny_skia::Paint::default();
    edge.anti_alias = true;
    edge.set_color(Color::from_rgba8(0, 0, 0, 34));
    pixmap.stroke_path(&page, &edge, &tiny_skia::Stroke { width: 1.0, ..Default::default() }, draw::tf(), None);
    // Letra proporcional ao tamanho da folha: em ícones pequenos vira só a "textura"
    // das linhas, como no Finder; nos grandes dá para ler. As linhas de cima param antes
    // da dobra.
    let font = (s / 18.0).clamp(2.2, 9.0);
    let line_h = font * 1.35;
    let margin = pw * 0.1;
    let max_lines = ((ph - 2.0 * margin) / line_h).floor() as usize;
    thread_local! {
        static TEXT: std::cell::RefCell<Option<Text>> = const { std::cell::RefCell::new(None) };
    }
    TEXT.with(|cell| {
        let mut cell = cell.borrow_mut();
        let text_engine = cell.get_or_insert_with(Text::new);
        for (i, line) in text.lines().take(max_lines).enumerate() {
            let line: String = line.replace('\t', "    ").chars().take(120).collect();
            if line.trim().is_empty() {
                continue;
            }
            let y = py + margin + i as f32 * line_h;
            let room = if y < py + fold + line_h * 0.2 { pw - 2.0 * margin - fold * 0.8 } else { pw - 2.0 * margin };
            // Só o que cabe na folha: o resto da linha é cortado pela própria largura.
            let mut row = Pixmap::new(room.ceil().max(1.0) as u32, line_h.ceil().max(1.0) as u32)?;
            text_engine.draw_mono(&mut row, &line, 0.0, 0.0, font, [46, 46, 52, 255]);
            draw::blit(&mut pixmap, &row, (px + margin) as i32, y as i32, 1.0);
        }
        // A extensão grande, em cinza, por cima do conteúdo (como o "TXT" do Finder).
        let ext = path.extension().and_then(|e| e.to_str()).map(str::to_uppercase).unwrap_or_default();
        if !ext.is_empty() && ext.chars().count() <= 5 && s >= 32.0 {
            let mut fs = pw * 0.24;
            let weight = crate::ui::text::FontWeight::Semibold;
            let mut w = text_engine.measure(&ext, fs, weight);
            if w > pw * 0.8 {
                fs *= pw * 0.8 / w;
                w = text_engine.measure(&ext, fs, weight);
            }
            let (ex, ey) = (px + (pw - w) / 2.0, py + ph * 0.66 - fs * 0.6);
            text_engine.draw(&mut pixmap, &ext, ex, ey, fs, weight, [140, 140, 146, 215]);
        }
        Some(())
    })?;
    // Dobra do canto: o verso da folha, cinza claro.
    let mut pb = tiny_skia::PathBuilder::new();
    pb.move_to(px + pw - fold, py);
    pb.line_to(px + pw - fold, py + fold - r);
    pb.quad_to(px + pw - fold, py + fold, px + pw - fold + r, py + fold);
    pb.line_to(px + pw, py + fold);
    pb.close();
    if let Some(flap) = pb.finish() {
        let mut back = tiny_skia::Paint::default();
        back.anti_alias = true;
        back.set_color(Color::from_rgba8(226, 226, 230, 255));
        pixmap.fill_path(&flap, &back, tiny_skia::FillRule::Winding, draw::tf(), None);
        pixmap.stroke_path(&flap, &edge, &tiny_skia::Stroke { width: 1.0, ..Default::default() }, draw::tf(), None);
    }
    Some(pixmap)
}

fn read_start(path: &Path, limit: usize) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).ok()?;
    let mut buffer = vec![0; limit];
    let n = file.read(&mut buffer).ok()?;
    buffer.truncate(n);
    Some(buffer)
}

// -------------------------------------------------- geradores do sistema

/// Gerador de miniaturas (`.thumbnailer`): comando e tipos que ele atende.
#[derive(Debug, Clone)]
struct Thumbnailer {
    exec: String,
    mimes: Vec<String>,
}

fn thumbnailers() -> &'static [Thumbnailer] {
    static LIST: OnceLock<Vec<Thumbnailer>> = OnceLock::new();
    LIST.get_or_init(|| {
        let mut list = Vec::new();
        for dir in data_dirs() {
            let Ok(read) = std::fs::read_dir(dir.join("thumbnailers")) else {
                continue;
            };
            for file in read.flatten() {
                if let Some(t) = std::fs::read_to_string(file.path()).ok().and_then(|text| parse_thumbnailer(&text)) {
                    list.push(t);
                }
            }
        }
        tracing::debug!("geradores de miniatura: {}", list.len());
        list
    })
}

fn data_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share"))) {
        dirs.push(home);
    }
    let system = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    dirs.extend(system.split(':').filter(|d| !d.is_empty()).map(PathBuf::from));
    dirs
}

fn parse_thumbnailer(text: &str) -> Option<Thumbnailer> {
    let mut exec = None;
    let mut mimes = Vec::new();
    let mut try_exec = None;
    for line in text.lines() {
        let line = line.trim();
        if let Some(v) = line.strip_prefix("Exec=") {
            exec = Some(v.to_string());
        } else if let Some(v) = line.strip_prefix("TryExec=") {
            try_exec = Some(v.to_string());
        } else if let Some(v) = line.strip_prefix("MimeType=") {
            mimes = v.split(';').filter(|m| !m.is_empty()).map(str::to_string).collect();
        }
    }
    let exec = exec?;
    let program = try_exec.unwrap_or_else(|| exec.split_whitespace().next().unwrap_or_default().to_string());
    if !program_exists(&program) {
        return None;
    }
    Some(Thumbnailer { exec, mimes })
}

fn program_exists(program: &str) -> bool {
    if program.contains('/') {
        return Path::new(program).is_file();
    }
    std::env::var_os("PATH").is_some_and(|paths| std::env::split_paths(&paths).any(|d| d.join(program).is_file()))
}

/// Tipo (MIME) pelo nome do arquivo, pela base do sistema (`mime/globs2`).
pub fn mime_for(path: &Path) -> Option<String> {
    static GLOBS: OnceLock<HashMap<String, String>> = OnceLock::new();
    let globs = GLOBS.get_or_init(|| {
        let mut map: HashMap<String, (u32, String)> = HashMap::new();
        for dir in data_dirs() {
            let Ok(text) = std::fs::read_to_string(dir.join("mime/globs2")) else {
                continue;
            };
            for line in text.lines().filter(|l| !l.starts_with('#')) {
                let mut parts = line.splitn(3, ':');
                let (Some(weight), Some(mime), Some(glob)) = (parts.next(), parts.next(), parts.next()) else {
                    continue;
                };
                let Some(ext) = glob.strip_prefix("*.") else {
                    continue;
                };
                if ext.contains(['*', '?', '[']) {
                    continue;
                }
                let weight: u32 = weight.parse().unwrap_or(50);
                let ext = ext.to_lowercase();
                if map.get(&ext).is_none_or(|(w, _)| weight > *w) {
                    map.insert(ext, (weight, mime.to_string()));
                }
            }
        }
        map.into_iter().map(|(k, (_, v))| (k, v)).collect()
    });
    let name = path.file_name()?.to_string_lossy().to_lowercase();
    // Extensões compostas primeiro (.tar.gz).
    let mut rest = name.as_str();
    while let Some(i) = rest.find('.') {
        rest = &rest[i + 1..];
        if let Some(mime) = globs.get(rest) {
            return Some(mime.clone());
        }
    }
    None
}

fn cache_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
    Some(base.join("mide/thumbnails"))
}

/// Miniatura pelo gerador do sistema, do cache ou gerada agora (com limite de tempo).
fn system_thumbnail(path: &Path, meta: &std::fs::Metadata, size: u32) -> Option<PathBuf> {
    let mime = mime_for(path)?;
    let thumbnailer = thumbnailers().iter().find(|t| t.mimes.iter().any(|m| m == &mime))?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut hasher);
    meta.len().hash(&mut hasher);
    meta.modified().ok().hash(&mut hasher);
    size.hash(&mut hasher);
    let dir = cache_dir()?;
    let out = dir.join(format!("{:016x}.png", hasher.finish()));
    if out.is_file() {
        return Some(out);
    }
    // Falhou antes (arquivo vazio de marca): não tenta de novo a cada abertura.
    let failed = out.with_extension("falhou");
    if failed.exists() {
        return None;
    }
    std::fs::create_dir_all(&dir).ok()?;
    let tmp = out.with_extension("tmp.png");
    let uri = format!("file://{}", encode_uri(&path.to_string_lossy()));
    let args: Vec<String> = thumbnailer
        .exec
        .split_whitespace()
        .map(|a| a.replace("%i", &path.to_string_lossy()).replace("%u", &uri).replace("%o", &tmp.to_string_lossy()).replace("%s", &size.to_string()))
        .collect();
    let (program, rest) = args.split_first()?;
    let child = Command::new(program).args(rest).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn();
    let Ok(mut child) = child else {
        return None;
    };
    let start = Instant::now();
    let ok = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.success(),
            Ok(None) if start.elapsed() < Duration::from_secs(10) => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break false;
            }
        }
    };
    if ok && tmp.is_file() && std::fs::rename(&tmp, &out).is_ok() {
        tracing::debug!("miniatura gerada ({program}): {}", path.display());
        Some(out)
    } else {
        let _ = std::fs::remove_file(&tmp);
        let _ = std::fs::write(&failed, b"");
        tracing::debug!("miniatura não gerada ({program}): {}", path.display());
        None
    }
}

/// Endereço `file://` de um caminho (bytes fora do ASCII seguro em `%XX`).
pub fn file_uri(path: &std::path::Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    format!("file://{}", encode_bytes(path.as_os_str().as_bytes()))
}

fn encode_uri(path: &str) -> String {
    encode_bytes(path.as_bytes())
}

fn encode_bytes(path: &[u8]) -> String {
    let mut out = String::new();
    for &b in path {
        if b.is_ascii_alphanumeric() || b"/-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Grava a folha de um texto em `MIDE_THUMB_OUT` (para conferir o desenho a olho).
    #[test]
    #[ignore]
    fn folha_de_texto_para_conferir() {
        let Some(out) = std::env::var_os("MIDE_THUMB_OUT") else { return };
        let dir = std::env::temp_dir().join("mide-folha");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("leia-me.txt");
        std::fs::write(&file, "Aqui vão as primeiras linhas.\nO mide mostra o conteúdo\nde cada arquivo de texto,\ncom a orelha dobrada no canto.\n\nComo no Finder do Mac:\no tipo do arquivo em cinza\npor cima do texto.\n\nMais uma linha.\nE outra.\nE mais outra para encher.").unwrap();
        let pix = text_page(&file, 256).unwrap();
        pix.save_png(out).unwrap();
    }

    #[test]
    fn folha_de_texto_tem_linhas_escuras() {
        let dir = std::env::temp_dir().join(format!("mide-thumb-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("notas.txt");
        std::fs::write(&file, "fn main() {\n    println!(\"oi\");\n}\n").unwrap();
        let pix = make(&file, 128).expect("miniatura do texto");
        assert_eq!((pix.width(), pix.height()), (128, 128));
        let dark = pix.pixels().iter().filter(|p| p.alpha() > 200 && p.red() < 120).count();
        assert!(dark > 10, "há texto desenhado ({dark} pontos escuros)");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn gerador_e_tipo_do_arquivo() {
        let t = parse_thumbnailer("[Thumbnailer Entry]\nTryExec=/bin/sh\nExec=/bin/sh -c x %i %o %s\nMimeType=application/pdf;image/x-foo;\n").unwrap();
        assert_eq!(t.mimes, vec!["application/pdf", "image/x-foo"]);
        assert!(parse_thumbnailer("[Thumbnailer Entry]\nExec=programa-que-nao-existe %i\nMimeType=a/b;\n").is_none());
        assert_eq!(encode_uri("/a b/ç.txt"), "/a%20b/%C3%A7.txt");
    }
}
