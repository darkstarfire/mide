//! Ícones XPM (o formato de imagem em texto do X11), lidos pelo próprio mide.
//!
//! Vários programas instalados à mão apontam o `Icon=` do atalho `.desktop` para um
//! `.xpm` (o Eclipse usa `Icon=/opt/eclipse/.../icon.xpm`). O `image` não lê XPM, então
//! o ícone sumia na área de trabalho, no Arquivos e no arrasto. Aqui o XPM vira um PNG
//! guardado no cache (`$XDG_CACHE_HOME/mide/icons`), que o resto do mide já sabe mostrar.

use std::path::{Path, PathBuf};

/// Imagem lida de um XPM: largura, altura e pixels RGBA (não premultiplicados).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Maior lado aceito (um XPM é texto: 256x256 já tem ~140 KB).
const MAX_SIDE: u32 = 1024;

/// Lê um XPM (versão 3, o formato em código C). `None` se não for um XPM válido.
pub fn decode(text: &str) -> Option<Image> {
    let strings = c_strings(text);
    let mut it = strings.iter();
    let header: Vec<u32> = it.next()?.split_whitespace().take(4).map(|v| v.parse().ok()).collect::<Option<_>>()?;
    let [width, height, colors, cpp] = header[..] else { return None };
    if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE || cpp == 0 || cpp > 8 || colors == 0 {
        return None;
    }
    let cpp = cpp as usize;
    let mut table: std::collections::HashMap<&str, [u8; 4]> = std::collections::HashMap::new();
    for _ in 0..colors {
        let line = it.next()?;
        let key = line.get(..cpp)?;
        table.insert(key, color_of(line.get(cpp..)?)?);
    }
    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
    for _ in 0..height {
        let row = it.next()?;
        for x in 0..width as usize {
            let key = row.get(x * cpp..(x + 1) * cpp)?;
            rgba.extend_from_slice(table.get(key).unwrap_or(&[0, 0, 0, 0]));
        }
    }
    Some(Image { width, height, rgba })
}

/// Os textos entre aspas do arquivo, sem os comentários `/* */`.
fn c_strings(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut last = ' ';
                for c in chars.by_ref() {
                    if last == '*' && c == '/' {
                        break;
                    }
                    last = c;
                }
            }
            '"' => {
                let mut s = String::new();
                while let Some(c) = chars.next() {
                    match c {
                        '\\' => {
                            if let Some(n) = chars.next() {
                                s.push(n);
                            }
                        }
                        '"' => break,
                        _ => s.push(c),
                    }
                }
                out.push(s);
            }
            _ => {}
        }
    }
    out
}

/// Cor de uma linha da tabela (`c #F7941E`, `c None`, `s mask c none`...): a da tela
/// colorida (`c`) e, sem ela, a de tons de cinza ou a monocromática.
fn color_of(spec: &str) -> Option<[u8; 4]> {
    let words: Vec<&str> = spec.split_whitespace().collect();
    let mut found: Vec<(&str, String)> = Vec::new();
    let mut i = 0;
    while i < words.len() {
        let key = words[i];
        if matches!(key, "c" | "m" | "g" | "g4" | "s") {
            // O valor vai até a próxima chave (nomes de cor podem ter espaços).
            let mut j = i + 1;
            while j < words.len() && !matches!(words[j], "c" | "m" | "g" | "g4" | "s") {
                j += 1;
            }
            found.push((key, words[i + 1..j].join(" ")));
            i = j;
        } else {
            i += 1;
        }
    }
    let value = ["c", "g", "g4", "m"].iter().find_map(|k| found.iter().find(|(key, v)| key == k && !v.is_empty()).map(|(_, v)| v.clone()))?;
    parse_color(&value)
}

/// `#RGB`, `#RRGGBB`, `#RRRRGGGGBBBB`, `None` ou alguns nomes do X11.
fn parse_color(value: &str) -> Option<[u8; 4]> {
    if let Some(hex) = value.strip_prefix('#') {
        let n = hex.len();
        if n == 0 || n % 3 != 0 || n > 12 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let k = n / 3;
        let part = |i: usize| -> u8 {
            let v = u32::from_str_radix(&hex[i * k..(i + 1) * k], 16).unwrap_or(0);
            let max = (1u32 << (4 * k)) - 1;
            ((v * 255 + max / 2) / max) as u8
        };
        return Some([part(0), part(1), part(2), 255]);
    }
    let rgb = match value.to_ascii_lowercase().replace(' ', "").as_str() {
        "none" => return Some([0, 0, 0, 0]),
        "black" => [0, 0, 0],
        "white" => [255, 255, 255],
        "red" => [255, 0, 0],
        "green" => [0, 255, 0],
        "blue" => [0, 0, 255],
        "yellow" => [255, 255, 0],
        "cyan" => [0, 255, 255],
        "magenta" => [255, 0, 255],
        "orange" => [255, 165, 0],
        "gray" | "grey" => [190, 190, 190],
        "darkgray" | "darkgrey" => [169, 169, 169],
        "lightgray" | "lightgrey" => [211, 211, 211],
        _ => [128, 128, 128],
    };
    Some([rgb[0], rgb[1], rgb[2], 255])
}

/// PNG no cache feito a partir do XPM `path` (criado na primeira vez e refeito quando
/// o XPM muda). `None` se o arquivo não é um XPM que dá para ler.
pub fn png_for(path: &Path) -> Option<PathBuf> {
    let meta = std::fs::metadata(path).ok().filter(|m| m.is_file() && m.len() <= 8 << 20)?;
    png_in(path, &meta, &cache_dir()?)
}

/// O mesmo que [`png_for`], com o cache em `dir`.
fn png_in(path: &Path, meta: &std::fs::Metadata, dir: &Path) -> Option<PathBuf> {
    let modified = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (path, modified, meta.len()).hash(&mut hasher);
    let out = dir.join(format!("xpm-{:016x}.png", hasher.finish()));
    if out.is_file() {
        return Some(out);
    }
    let image = decode(&std::fs::read_to_string(path).ok()?)?;
    std::fs::create_dir_all(dir).ok()?;
    // Grava num temporário e troca: duas janelas lendo o mesmo ícone não veem meio PNG.
    let tmp = dir.join(format!(".tmp-{}-{}", std::process::id(), out.file_name()?.to_string_lossy()));
    image::save_buffer(&tmp, &image.rgba, image.width, image.height, image::ExtendedColorType::Rgba8).ok()?;
    std::fs::rename(&tmp, &out).ok()?;
    Some(out)
}

/// `$XDG_CACHE_HOME/mide/icons` (ou `~/.cache/mide/icons`).
fn cache_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
    Some(base.join("mide").join("icons"))
}

/// Ícone de um atalho pronto para mostrar: um `.xpm` vira o PNG do cache; os outros
/// voltam como estão.
pub fn displayable_icon(icon: String) -> String {
    if icon.starts_with('/') && icon.to_ascii_lowercase().ends_with(".xpm") {
        if let Some(png) = png_for(Path::new(&icon)) {
            return png.to_string_lossy().into_owned();
        }
        tracing::info!("ícone {icon}: XPM que não deu para ler");
        return "application-x-executable".into();
    }
    icon
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Como o `icon.xpm` do Eclipse: 2 caracteres por pixel, comentários e `s mask`.
    const SAMPLE: &str = r#"/* XPM */
static char * icon_xpm[] = {
"3 2 3 2",
"  	c None",
". 	c #F7941E",
"X 	s mask c #000",
/* pixels */
"  . X ",
". X   ",
};"#;

    #[test]
    fn le_xpm_com_transparencia_e_cores() {
        let img = decode(SAMPLE).expect("xpm válido");
        assert_eq!((img.width, img.height), (3, 2));
        let px = |x: usize, y: usize| &img.rgba[(y * 3 + x) * 4..(y * 3 + x) * 4 + 4];
        assert_eq!(px(0, 0), &[0, 0, 0, 0]);
        assert_eq!(px(1, 0), &[0xF7, 0x94, 0x1E, 255]);
        assert_eq!(px(2, 0), &[0, 0, 0, 255]);
        assert_eq!(px(0, 1), &[0xF7, 0x94, 0x1E, 255]);
        assert_eq!(px(2, 1), &[0, 0, 0, 0]);
        // Linha curta: não é um XPM válido.
        assert!(decode(&SAMPLE.replace("\". X   \"", "\". X\"")).is_none());
    }

    #[test]
    fn cores_de_16_bits_e_nomes() {
        assert_eq!(parse_color("#FFFF00008080"), Some([255, 0, 128, 255]));
        assert_eq!(parse_color("None"), Some([0, 0, 0, 0]));
        assert_eq!(parse_color("white"), Some([255, 255, 255, 255]));
        assert_eq!(color_of("m white c #102030"), Some([0x10, 0x20, 0x30, 255]));
        assert_eq!(parse_color("#12"), None);
    }

    #[test]
    fn rejeita_o_que_nao_e_xpm() {
        assert!(decode("não é um xpm").is_none());
        assert!(decode("\"99999 99999 1 1\"").is_none());
        assert_eq!(displayable_icon("eclipse".into()), "eclipse");
    }

    #[test]
    fn xpm_vira_png_no_cache() {
        let dir = std::env::temp_dir().join(format!("mide-xpm-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let xpm = dir.join("icon.xpm");
        std::fs::write(&xpm, "\"2 1 2 1\",\"a c #FF0000\",\"b c None\",\"ab\"").unwrap();
        let meta = std::fs::metadata(&xpm).unwrap();
        let png = png_in(&xpm, &meta, &dir.join("cache")).expect("png");
        assert_eq!(png_in(&xpm, &meta, &dir.join("cache")), Some(png.clone()), "reaproveita o cache");
        let img = image::open(&png).unwrap().to_rgba8();
        assert_eq!(img.dimensions(), (2, 1));
        assert_eq!(img.get_pixel(0, 0).0, [255, 0, 0, 255]);
        assert_eq!(img.get_pixel(1, 0).0[3], 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
