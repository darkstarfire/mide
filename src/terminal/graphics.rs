//! Imagens nos formatos Sixel (DEC, `DCS q`) e do kitty (`APC G`), além do
//! protocolo do iTerm2 que fica na tela ([`super::screen`]). Aqui só a decodificação
//! e os comandos; quem mostra a imagem é a tela.

use std::collections::HashMap;

/// Imagem decodificada: largura, altura e pixels RGBA (sem pré-multiplicar).
#[derive(Debug, Clone, PartialEq)]
pub struct Rgba {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// Maior lado aceito (protege a memória de imagens absurdas).
const MAX_SIDE: usize = 4096;

/// As 16 cores iniciais do Sixel (as do VT340), em porcentagem como no protocolo.
const VT340: [[u8; 3]; 16] = [
    [0, 0, 0],
    [20, 20, 80],
    [80, 13, 13],
    [20, 80, 20],
    [80, 20, 80],
    [20, 80, 80],
    [80, 80, 20],
    [53, 53, 53],
    [26, 26, 26],
    [33, 33, 60],
    [60, 26, 26],
    [33, 60, 33],
    [60, 33, 60],
    [33, 60, 60],
    [60, 60, 33],
    [80, 80, 80],
];

fn percent(v: u8) -> u8 {
    (v.min(100) as u32 * 255 / 100) as u8
}

/// Cor HLS do Sixel (matiz 0–360, luz e saturação em %) em RGB.
fn hls(h: u16, l: u16, s: u16) -> [u8; 3] {
    // No Sixel o azul é 0°; no HLS comum, o vermelho.
    let h = ((h as f32 + 240.0) % 360.0) / 360.0;
    let (l, s) = (l.min(100) as f32 / 100.0, s.min(100) as f32 / 100.0);
    if s == 0.0 {
        let v = (l * 255.0) as u8;
        return [v, v, v];
    }
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let channel = |mut t: f32| {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        let v = if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        };
        (v * 255.0).round() as u8
    };
    [channel(h + 1.0 / 3.0), channel(h), channel(h - 1.0 / 3.0)]
}

/// Decodifica o conteúdo de uma `DCS ... q` (Sixel). Pixels não pintados ficam
/// transparentes.
pub fn decode_sixel(data: &[u8]) -> Option<Rgba> {
    let mut palette: Vec<[u8; 3]> = VT340.iter().map(|c| [percent(c[0]), percent(c[1]), percent(c[2])]).collect();
    palette.resize(256, [0, 0, 0]);
    let mut rows: Vec<Vec<u32>> = Vec::new();
    let (mut x, mut y, mut color, mut width) = (0usize, 0usize, 0usize, 0usize);
    let mut i = 0;
    let number = |i: &mut usize| -> Option<u32> {
        let start = *i;
        while *i < data.len() && data[*i].is_ascii_digit() {
            *i += 1;
        }
        std::str::from_utf8(&data[start..*i]).ok()?.parse().ok()
    };
    let put = |rows: &mut Vec<Vec<u32>>, x: usize, y: usize, bits: u8, count: usize, rgb: [u8; 3]| {
        let pixel = 0xff00_0000 | (rgb[0] as u32) << 16 | (rgb[1] as u32) << 8 | rgb[2] as u32;
        for bit in 0..6 {
            if bits & (1 << bit) == 0 || y + bit >= MAX_SIDE {
                continue;
            }
            let row_index = y + bit;
            if rows.len() <= row_index {
                rows.resize(row_index + 1, Vec::new());
            }
            let row = &mut rows[row_index];
            let end = (x + count).min(MAX_SIDE);
            if row.len() < end {
                row.resize(end, 0);
            }
            for px in row.iter_mut().take(end).skip(x) {
                *px = pixel;
            }
        }
    };
    while i < data.len() {
        let c = data[i];
        match c {
            b'"' => {
                // Proporção e tamanho sugerido: só para reservar espaço.
                i += 1;
                for _ in 0..4 {
                    let _ = number(&mut i);
                    if i < data.len() && data[i] == b';' {
                        i += 1;
                    }
                }
            }
            b'#' => {
                i += 1;
                let index = number(&mut i).unwrap_or(0) as usize % 256;
                if i < data.len() && data[i] == b';' {
                    let mut values = [0u32; 4];
                    for v in values.iter_mut() {
                        if i < data.len() && data[i] == b';' {
                            i += 1;
                            *v = number(&mut i).unwrap_or(0);
                        }
                    }
                    let [kind, a, b, c] = values;
                    palette[index] = if kind == 1 { hls(a as u16, b as u16, c as u16) } else { [percent(a as u8), percent(b as u8), percent(c as u8)] };
                }
                color = index;
            }
            b'!' => {
                i += 1;
                let count = number(&mut i).unwrap_or(1).max(1) as usize;
                if i < data.len() && (b'?'..=b'~').contains(&data[i]) {
                    put(&mut rows, x, y, data[i] - b'?', count, palette[color]);
                    x = (x + count).min(MAX_SIDE);
                    width = width.max(x);
                    i += 1;
                }
            }
            b'$' => {
                x = 0;
                i += 1;
            }
            b'-' => {
                x = 0;
                y += 6;
                i += 1;
            }
            b'?'..=b'~' => {
                put(&mut rows, x, y, c - b'?', 1, palette[color]);
                x = (x + 1).min(MAX_SIDE);
                width = width.max(x);
                i += 1;
            }
            _ => i += 1,
        }
    }
    // A altura vai até a última linha pintada.
    while rows.last().is_some_and(|r| r.iter().all(|p| *p == 0)) {
        rows.pop();
    }
    let height = rows.len();
    if width == 0 || height == 0 {
        return None;
    }
    let mut pixels = vec![0u8; width * height * 4];
    for (y, row) in rows.iter().enumerate() {
        for (x, p) in row.iter().enumerate() {
            let k = (y * width + x) * 4;
            pixels[k] = (p >> 16) as u8;
            pixels[k + 1] = (p >> 8) as u8;
            pixels[k + 2] = *p as u8;
            pixels[k + 3] = (p >> 24) as u8;
        }
    }
    Some(Rgba { width: width as u32, height: height as u32, pixels })
}

/// Caractere que marca o lugar de uma imagem do kitty na grade (placeholder Unicode):
/// os programas (Neovim, tmux) escrevem este caractere em cada célula da imagem.
pub const PLACEHOLDER: char = '\u{10EEEE}';

/// Acentos que dizem a linha e a coluna de cada placeholder (a posição na lista é o
/// número): a tabela fixa do protocolo do kitty, tirada dos caracteres combinantes do
/// Unicode.
const DIACRITICS: [u32; 297] = [
    0x0305, 0x030D, 0x030E, 0x0310, 0x0312, 0x033D, 0x033E, 0x033F, 0x0346, 0x034A,
    0x034B, 0x034C, 0x0350, 0x0351, 0x0352, 0x0357, 0x035B, 0x0363, 0x0364, 0x0365,
    0x0366, 0x0367, 0x0368, 0x0369, 0x036A, 0x036B, 0x036C, 0x036D, 0x036E, 0x036F,
    0x0483, 0x0484, 0x0485, 0x0486, 0x0487, 0x0592, 0x0593, 0x0594, 0x0595, 0x0597,
    0x0598, 0x0599, 0x059C, 0x059D, 0x059E, 0x059F, 0x05A0, 0x05A1, 0x05A8, 0x05A9,
    0x05AB, 0x05AC, 0x05AF, 0x05C4, 0x0610, 0x0611, 0x0612, 0x0613, 0x0614, 0x0615,
    0x0616, 0x0617, 0x0657, 0x0658, 0x0659, 0x065A, 0x065B, 0x065D, 0x065E, 0x06D6,
    0x06D7, 0x06D8, 0x06D9, 0x06DA, 0x06DB, 0x06DC, 0x06DF, 0x06E0, 0x06E1, 0x06E2,
    0x06E4, 0x06E7, 0x06E8, 0x06EB, 0x06EC, 0x0730, 0x0732, 0x0733, 0x0735, 0x0736,
    0x073A, 0x073D, 0x073F, 0x0740, 0x0741, 0x0743, 0x0745, 0x0747, 0x0749, 0x074A,
    0x07EB, 0x07EC, 0x07ED, 0x07EE, 0x07EF, 0x07F0, 0x07F1, 0x07F3, 0x0816, 0x0817,
    0x0818, 0x0819, 0x081B, 0x081C, 0x081D, 0x081E, 0x081F, 0x0820, 0x0821, 0x0822,
    0x0823, 0x0825, 0x0826, 0x0827, 0x0829, 0x082A, 0x082B, 0x082C, 0x082D, 0x0951,
    0x0953, 0x0954, 0x0F82, 0x0F83, 0x0F86, 0x0F87, 0x135D, 0x135E, 0x135F, 0x17DD,
    0x193A, 0x1A17, 0x1A75, 0x1A76, 0x1A77, 0x1A78, 0x1A79, 0x1A7A, 0x1A7B, 0x1A7C,
    0x1B6B, 0x1B6D, 0x1B6E, 0x1B6F, 0x1B70, 0x1B71, 0x1B72, 0x1B73, 0x1CD0, 0x1CD1,
    0x1CD2, 0x1CDA, 0x1CDB, 0x1CE0, 0x1DC0, 0x1DC1, 0x1DC3, 0x1DC4, 0x1DC5, 0x1DC6,
    0x1DC7, 0x1DC8, 0x1DC9, 0x1DCB, 0x1DCC, 0x1DD1, 0x1DD2, 0x1DD3, 0x1DD4, 0x1DD5,
    0x1DD6, 0x1DD7, 0x1DD8, 0x1DD9, 0x1DDA, 0x1DDB, 0x1DDC, 0x1DDD, 0x1DDE, 0x1DDF,
    0x1DE0, 0x1DE1, 0x1DE2, 0x1DE3, 0x1DE4, 0x1DE5, 0x1DE6, 0x1DFE, 0x20D0, 0x20D1,
    0x20D4, 0x20D5, 0x20D6, 0x20D7, 0x20DB, 0x20DC, 0x20E1, 0x20E7, 0x20E9, 0x20F0,
    0x2CEF, 0x2CF0, 0x2CF1, 0x2DE0, 0x2DE1, 0x2DE2, 0x2DE3, 0x2DE4, 0x2DE5, 0x2DE6,
    0x2DE7, 0x2DE8, 0x2DE9, 0x2DEA, 0x2DEB, 0x2DEC, 0x2DED, 0x2DEE, 0x2DEF, 0x2DF0,
    0x2DF1, 0x2DF2, 0x2DF3, 0x2DF4, 0x2DF5, 0x2DF6, 0x2DF7, 0x2DF8, 0x2DF9, 0x2DFA,
    0x2DFB, 0x2DFC, 0x2DFD, 0x2DFE, 0x2DFF, 0xA66F, 0xA67C, 0xA67D, 0xA6F0, 0xA6F1,
    0xA8E0, 0xA8E1, 0xA8E2, 0xA8E3, 0xA8E4, 0xA8E5, 0xA8E6, 0xA8E7, 0xA8E8, 0xA8E9,
    0xA8EA, 0xA8EB, 0xA8EC, 0xA8ED, 0xA8EE, 0xA8EF, 0xA8F0, 0xA8F1, 0xAAB0, 0xAAB2,
    0xAAB3, 0xAAB7, 0xAAB8, 0xAABE, 0xAABF, 0xAAC1, 0xFE20, 0xFE21, 0xFE22, 0xFE23,
    0xFE24, 0xFE25, 0xFE26, 0x10A0F, 0x10A38, 0x1D185, 0x1D186, 0x1D187, 0x1D188, 0x1D189,
    0x1D1AA, 0x1D1AB, 0x1D1AC, 0x1D1AD, 0x1D242, 0x1D243, 0x1D244,
];

/// Número que um acento de placeholder representa (linha, coluna ou o byte alto do
/// número da imagem).
pub fn diacritic_index(c: char) -> Option<usize> {
    DIACRITICS.binary_search(&(c as u32)).ok()
}

/// O que um comando do kitty pede à tela.
#[derive(Debug, Clone, PartialEq)]
pub enum KittyAction {
    /// Mostrar a imagem no cursor (`cols`/`rows`: tamanho pedido em células) e mover
    /// o cursor para depois dela (ou não, `C=1`).
    Display { image: Rgba, cols: Option<u32>, rows: Option<u32>, move_cursor: bool },
    /// Apagar as imagens mostradas (todas, ou as de um número).
    Delete { id: Option<u32> },
    /// Colocação virtual (`U=1`): a imagem aparece onde o programa escrever os
    /// placeholders dela, ocupando `cols` x `rows` células.
    Virtual { id: u32, cols: u32, rows: u32 },
}

/// Estado do protocolo do kitty: a transmissão em partes e as imagens guardadas.
#[derive(Debug, Default)]
pub struct Kitty {
    /// Chaves e conteúdo das partes recebidas (`m=1`).
    pending: Option<(HashMap<String, String>, String)>,
    stored: HashMap<u32, Rgba>,
}

impl Kitty {
    /// Um comando (`G` + chaves + `;` + conteúdo em base64). Devolve o que mostrar e a
    /// resposta para o programa (se ele quer uma).
    pub fn command(&mut self, data: &[u8]) -> (Option<KittyAction>, Option<String>) {
        let text = String::from_utf8_lossy(data);
        let Some(body) = text.strip_prefix('G') else {
            return (None, None);
        };
        let (keys, payload) = body.split_once(';').unwrap_or((body, ""));
        let mut keys: HashMap<String, String> = keys.split(',').filter_map(|kv| kv.split_once('=')).map(|(k, v)| (k.to_string(), v.to_string())).collect();
        let more = keys.get("m").map(String::as_str) == Some("1");
        // Partes seguintes de uma transmissão: só `m` (e o conteúdo).
        let mut payload = payload.to_string();
        if let Some((first, mut acc)) = self.pending.take() {
            acc.push_str(&payload);
            if more {
                if acc.len() < 64 << 20 {
                    self.pending = Some((first, acc));
                }
                return (None, None);
            }
            keys = first;
            payload = acc;
        } else if more {
            self.pending = Some((keys, payload));
            return (None, None);
        }
        let get = |k: &str| keys.get(k).map(String::as_str);
        let id = get("i").and_then(|v| v.parse::<u32>().ok());
        let quiet = get("q").and_then(|v| v.parse::<u8>().ok()).unwrap_or(0);
        let reply = |result: Result<(), String>| -> Option<String> {
            let id = id?;
            match result {
                Ok(()) if quiet == 0 => Some(format!("\x1b_Gi={id};OK\x1b\\")),
                Err(e) if quiet < 2 => Some(format!("\x1b_Gi={id};{e}\x1b\\")),
                _ => None,
            }
        };
        let action = get("a").unwrap_or("t");
        match action {
            "d" => {
                let what = get("d").unwrap_or("a");
                let target = matches!(what, "i" | "I").then_some(id).flatten();
                if let Some(id) = target {
                    self.stored.remove(&id);
                }
                (Some(KittyAction::Delete { id: target }), None)
            }
            "p" => match id.and_then(|i| self.stored.get(&i)) {
                Some(image) => (Some(self.display(image.clone(), &keys)), reply(Ok(()))),
                None => (None, reply(Err("ENOENT:imagem desconhecida".into()))),
            },
            "q" | "t" | "T" => match decode(&keys, &payload) {
                Ok(image) => {
                    let shown = (action == "T").then(|| self.display(image.clone(), &keys));
                    if action != "q" {
                        if let Some(id) = id {
                            self.stored.insert(id, image);
                            // Só as últimas imagens guardadas.
                            if self.stored.len() > 64 {
                                if let Some(&oldest) = self.stored.keys().min() {
                                    self.stored.remove(&oldest);
                                }
                            }
                        }
                    }
                    (shown, reply(Ok(())))
                }
                Err(e) => (None, reply(Err(e))),
            },
            _ => (None, reply(Err("EINVAL:ação desconhecida".into()))),
        }
    }

    /// Imagem guardada com este número.
    pub fn image(&self, id: u32) -> Option<&Rgba> {
        self.stored.get(&id)
    }

    fn display(&self, image: Rgba, keys: &HashMap<String, String>) -> KittyAction {
        let num = |k: &str| keys.get(k).and_then(|v| v.parse::<u32>().ok()).filter(|n| *n > 0);
        if keys.get("U").map(String::as_str) == Some("1") {
            if let Some(id) = keys.get("i").and_then(|v| v.parse::<u32>().ok()) {
                let (w, h) = (image.width.max(1), image.height.max(1));
                // Sem tamanho pedido: um tamanho razoável pela proporção.
                let cols = num("c").unwrap_or_else(|| (w / 8).clamp(1, 80));
                let rows = num("r").unwrap_or_else(|| ((cols * h * 8) / (w * 16)).max(1));
                return KittyAction::Virtual { id, cols, rows };
            }
        }
        KittyAction::Display { image, cols: num("c"), rows: num("r"), move_cursor: keys.get("C").map(String::as_str) != Some("1") }
    }
}

/// Pixels de uma transmissão do kitty: PNG (`f=100`), RGBA (`f=32`, padrão) ou RGB
/// (`f=24`), direto no comando (`t=d`) ou num arquivo (`t=f`/`t=t`).
fn decode(keys: &HashMap<String, String>, payload: &str) -> Result<Rgba, String> {
    if keys.get("o").is_some() {
        return Err("ENOTSUPPORTED:compressão".into());
    }
    let raw = super::screen::decode_base64_pub(payload).ok_or("EINVAL:base64")?;
    let bytes = match keys.get("t").map(String::as_str).unwrap_or("d") {
        "d" => raw,
        "f" | "t" => {
            let path = String::from_utf8(raw).map_err(|_| "EINVAL:caminho")?;
            let meta = std::fs::metadata(&path).map_err(|_| "ENOENT:arquivo")?;
            if !meta.is_file() || meta.len() > 64 << 20 {
                return Err("EINVAL:arquivo".into());
            }
            let bytes = std::fs::read(&path).map_err(|_| "ENOENT:arquivo")?;
            // Arquivo temporário: o terminal apaga depois de ler (como o kitty).
            if keys.get("t").map(String::as_str) == Some("t") && path.contains("tty-graphics-protocol") {
                let _ = std::fs::remove_file(&path);
            }
            bytes
        }
        _ => return Err("ENOTSUPPORTED:meio".into()),
    };
    let num = |k: &str| keys.get(k).and_then(|v| v.parse::<u32>().ok()).unwrap_or(0);
    match keys.get("f").map(String::as_str).unwrap_or("32") {
        "100" => {
            let image = image::load_from_memory(&bytes).map_err(|_| "EBADPNG:imagem")?.to_rgba8();
            if image.width() as usize > MAX_SIDE || image.height() as usize > MAX_SIDE {
                return Err("EINVAL:grande demais".into());
            }
            Ok(Rgba { width: image.width(), height: image.height(), pixels: image.into_raw() })
        }
        f @ ("32" | "24") => {
            let (w, h) = (num("s"), num("v"));
            let channels = if f == "32" { 4 } else { 3 };
            if w == 0 || h == 0 || w as usize > MAX_SIDE || h as usize > MAX_SIDE || bytes.len() < (w * h) as usize * channels {
                return Err("EINVAL:tamanho".into());
            }
            let pixels = if channels == 4 {
                bytes[..(w * h * 4) as usize].to_vec()
            } else {
                bytes.chunks_exact(3).take((w * h) as usize).flat_map(|p| [p[0], p[1], p[2], 255]).collect()
            };
            Ok(Rgba { width: w, height: h, pixels })
        }
        _ => Err("ENOTSUPPORTED:formato".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sixel_com_cores_repeticao_e_bandas() {
        // Cor 1 = vermelho; "~" pinta as 6 linhas; "!3~" repete; "-" desce 6 linhas.
        let img = decode_sixel(b"\"1;1;4;12#1;2;100;0;0#1~!3~-#2;2;0;0;100@").unwrap();
        assert_eq!((img.width, img.height), (4, 7));
        assert_eq!(&img.pixels[0..4], &[255, 0, 0, 255]);
        assert_eq!(&img.pixels[(5 * 4 + 3) * 4..(5 * 4 + 3) * 4 + 4], &[255, 0, 0, 255]);
        // Linha 6, coluna 0: o "@" (bit 0) da segunda banda, azul.
        assert_eq!(&img.pixels[(6 * 4) * 4..(6 * 4) * 4 + 4], &[0, 0, 255, 255]);
        // Coluna 1 da linha 6 não foi pintada: transparente.
        assert_eq!(img.pixels[(6 * 4 + 1) * 4 + 3], 0);
        assert!(decode_sixel(b"").is_none());
    }

    #[test]
    fn hls_do_sixel() {
        // No Sixel, 120° é vermelho e 0° é azul.
        assert_eq!(hls(120, 50, 100), [255, 0, 0]);
        assert_eq!(hls(0, 50, 100), [0, 0, 255]);
    }

    #[test]
    fn kitty_rgb_em_partes_mostrar_guardar_e_apagar() {
        let mut k = Kitty::default();
        // 1x2 RGB = 6 bytes ("/wAAAP8A" em base64), em duas partes.
        let (a, r) = k.command(b"Ga=T,f=24,s=1,v=2,i=7,m=1;/wAA");
        assert!(a.is_none() && r.is_none());
        let (a, r) = k.command(b"Gm=0;AP8A");
        let Some(KittyAction::Display { image, move_cursor, .. }) = a else { panic!("{a:?}") };
        assert_eq!((image.width, image.height, move_cursor), (1, 2, true));
        assert_eq!(image.pixels, vec![255, 0, 0, 255, 0, 255, 0, 255]);
        assert_eq!(r.as_deref(), Some("\x1b_Gi=7;OK\x1b\\"));
        // Mostrar de novo a guardada, com tamanho em células e sem mover o cursor.
        let (a, _) = k.command(b"Ga=p,i=7,c=4,r=2,C=1");
        assert!(matches!(a, Some(KittyAction::Display { cols: Some(4), rows: Some(2), move_cursor: false, .. })));
        assert_eq!(k.command(b"Ga=d,d=i,i=7").0, Some(KittyAction::Delete { id: Some(7) }));
        assert_eq!(k.command(b"Ga=p,i=7").1.as_deref(), Some("\x1b_Gi=7;ENOENT:imagem desconhecida\x1b\\"));
    }

    #[test]
    fn kitty_colocacao_virtual_para_placeholders() {
        let mut k = Kitty::default();
        let (a, _) = k.command(b"Ga=T,U=1,f=24,s=1,v=1,i=42,c=10,r=3,q=2;/wAA");
        assert_eq!(a, Some(KittyAction::Virtual { id: 42, cols: 10, rows: 3 }));
        assert!(k.image(42).is_some());
        assert_eq!(diacritic_index('\u{0305}'), Some(0));
        assert_eq!(diacritic_index('\u{030D}'), Some(1));
        assert_eq!(diacritic_index('a'), None);
    }

    #[test]
    fn kitty_consulta_e_silencio() {
        let mut k = Kitty::default();
        // Os programas perguntam com a=q para saber se o terminal entende o protocolo.
        let (a, r) = k.command(b"Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA");
        assert!(a.is_none());
        assert_eq!(r.as_deref(), Some("\x1b_Gi=31;OK\x1b\\"));
        assert_eq!(k.command(b"Gi=1,a=t,f=24,s=1,v=1,q=1;AAAA").1, None);
        assert_eq!(k.command(b"Gi=1,a=t,f=99,q=2;AAAA").1, None);
        assert!(k.command(b"Gi=2,a=t,f=99;AAAA").1.unwrap().contains("ENOTSUPPORTED"));
    }
}
