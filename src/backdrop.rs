//! Fundo da área de trabalho: imagem ajustada à tela e o gradiente padrão do mide.
//! Usado pelo compositor (papel de parede) e pelas Configurações (miniaturas).

use image::{imageops::FilterType, RgbaImage};

/// Ajusta a imagem para cobrir `w`x`h` sem distorcer, cortando o que sobra no centro.
pub fn cover(img: &RgbaImage, w: u32, h: u32) -> RgbaImage {
    let (iw, ih) = (img.width().max(1) as f64, img.height().max(1) as f64);
    let scale = (w as f64 / iw).max(h as f64 / ih);
    let sw = ((iw * scale).round() as u32).max(w);
    let sh = ((ih * scale).round() as u32).max(h);
    let resized = image::imageops::resize(img, sw, sh, FilterType::Triangle);
    let x = (sw - w) / 2;
    let y = (sh - h) / 2;
    image::imageops::crop_imm(&resized, x, y, w, h).to_image()
}

/// Uma faixa do papel de parede padrão: a borda de cima é uma curva
/// `y = base + tilt·x + amp·sen(freq·x + phase) + amp/2·sen(2,3·freq·x + phase2)`
/// (em unidades da altura da tela), e tudo abaixo dela é da faixa.
struct Ribbon {
    base: f32,
    tilt: f32,
    amp: f32,
    freq: f32,
    phase: f32,
    phase2: f32,
    color: [f32; 3],
}

impl Ribbon {
    /// Altura da borda em `x` e a inclinação dela.
    fn edge(&self, x: f32) -> (f32, f32) {
        let (a, b) = (self.freq * x + self.phase, 2.3 * self.freq * x + self.phase2);
        let y = self.base + self.tilt * x + self.amp * a.sin() + 0.5 * self.amp * b.sin();
        let slope = self.tilt + self.amp * self.freq * a.cos() + 0.5 * self.amp * 2.3 * self.freq * b.cos();
        (y, slope)
    }
}

/// Faixas de trás para a frente: do índigo ao pêssego, como seda dobrada.
const RIBBONS: [Ribbon; 5] = [
    Ribbon { base: 0.20, tilt: 0.10, amp: 0.10, freq: 2.1, phase: 0.6, phase2: 2.0, color: [58.0, 44.0, 140.0] },
    Ribbon { base: 0.36, tilt: 0.08, amp: 0.11, freq: 1.8, phase: 2.4, phase2: 0.3, color: [104.0, 54.0, 172.0] },
    Ribbon { base: 0.50, tilt: 0.06, amp: 0.10, freq: 2.0, phase: 4.1, phase2: 1.2, color: [176.0, 64.0, 160.0] },
    Ribbon { base: 0.64, tilt: 0.04, amp: 0.09, freq: 2.3, phase: 5.3, phase2: 3.1, color: [236.0, 104.0, 118.0] },
    Ribbon { base: 0.79, tilt: 0.02, amp: 0.07, freq: 2.6, phase: 1.1, phase2: 4.4, color: [252.0, 170.0, 122.0] },
];

/// Papel de parede padrão do mide: faixas curvas sobrepostas sobre um céu azul-noite
/// (inspirado nos fundos do macOS). Cada faixa da frente faz uma sombra suave na de
/// trás, tem um brilho fino na borda e clareia perto dela (dá volume). As formas usam a
/// altura como medida, então não esticam em telas largas. Gerado uma vez por tamanho de
/// tela, em todos os núcleos.
pub fn default_gradient(w: u32, h: u32) -> RgbaImage {
    let (w, h) = (w.max(1), h.max(1));
    let mut out = RgbaImage::new(w, h);
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(16);
    let rows_per = (h as usize).div_ceil(threads).max(1);
    let stride = w as usize * 4;
    std::thread::scope(|scope| {
        for (chunk, rows) in out.chunks_mut(rows_per * stride).enumerate() {
            scope.spawn(move || {
                let y0 = chunk * rows_per;
                for (r, row) in rows.chunks_mut(stride).enumerate() {
                    let y = (y0 + r) as u32;
                    for (x, px) in row.chunks_exact_mut(4).enumerate() {
                        let c = wallpaper_pixel(x as u32, y, w, h);
                        px.copy_from_slice(&c);
                    }
                }
            });
        }
    });
    out
}

/// Cor de um pixel do papel de parede padrão.
fn wallpaper_pixel(x: u32, y: u32, w: u32, h: u32) -> [u8; 4] {
    let fh = h as f32;
    let px = 1.0 / fh; // um pixel, em unidades da altura
    // Coordenadas com a altura como medida, centradas na horizontal.
    let u = (x as f32 - w as f32 / 2.0) / fh;
    let v = y as f32 / fh;
    // Céu: azul-noite em cima, índigo embaixo, com um brilho frio no alto à esquerda.
    let mut color = mix([14.0, 18.0, 52.0], [38.0, 30.0, 98.0], smooth(v * 1.4));
    color = mix(color, [70.0, 90.0, 190.0], blob(u, v, -0.45, 0.05, 0.6) * 0.35);
    for ribbon in &RIBBONS {
        let (edge, slope) = ribbon.edge(u);
        // Distância até a borda, na perpendicular (positiva dentro da faixa).
        let d = (v - edge) / (1.0 + slope * slope).sqrt();
        // Sombra da faixa sobre o que está atrás, logo acima da borda.
        if d < 0.0 {
            let shadow = (d * 22.0).exp() * 0.42;
            color = mix(color, [8.0, 6.0, 24.0], shadow);
            if d < -1.5 * px {
                continue;
            }
        }
        // Volume: clara perto da borda, mais escura no fundo da faixa.
        let depth = d.max(0.0);
        let lit = mix(ribbon.color, [255.0, 236.0, 220.0], (-depth * 9.0).exp() * 0.28);
        let shaded = mix(lit, mul(ribbon.color, 0.62), smooth(depth * 2.2) * 0.55);
        // Brilho fino na borda.
        let rim = (-depth / (1.6 * px)).exp() * 0.30;
        let fabric = mix(shaded, [255.0, 245.0, 240.0], rim);
        // Borda sem serrilhado (cobre de -0,75 a +0,75 pixel).
        let cover = smooth((d + 0.75 * px) / (1.5 * px));
        color = mix(color, fabric, cover);
    }
    // Brilho quente vindo de baixo à direita.
    color = mix(color, [255.0, 214.0, 170.0], blob(u, v, 0.55, 1.05, 0.7) * 0.22);
    // Pontilhado leve para evitar faixas nos degradês.
    let dither = (((x * 7 + y * 13) % 5) as f32 - 2.0) * 0.4;
    [
        (color[0] + dither).clamp(0.0, 255.0) as u8,
        (color[1] + dither).clamp(0.0, 255.0) as u8,
        (color[2] + dither).clamp(0.0, 255.0) as u8,
        255,
    ]
}

fn mul(c: [f32; 3], k: f32) -> [f32; 3] {
    [c[0] * k, c[1] * k, c[2] * k]
}

fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// Intensidade de uma mancha circular suave centrada em (cx, cy) com raio `r`.
fn blob(u: f32, v: f32, cx: f32, cy: f32, r: f32) -> f32 {
    let d = ((u - cx).powi(2) + (v - cy).powi(2)).sqrt() / r;
    let f = (1.0 - d).clamp(0.0, 1.0);
    f * f
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cobrir_mantem_tamanho_pedido() {
        let wide = RgbaImage::from_pixel(400, 100, image::Rgba([10, 20, 30, 255]));
        let out = cover(&wide, 200, 200);
        assert_eq!(out.dimensions(), (200, 200));
        let tall = RgbaImage::from_pixel(50, 300, image::Rgba([1, 2, 3, 255]));
        assert_eq!(cover(&tall, 320, 180).dimensions(), (320, 180));
    }

    #[test]
    fn cobrir_corta_o_centro() {
        // Metade esquerda preta e direita branca; cobrir um quadrado mantém o centro.
        let mut img = RgbaImage::from_pixel(300, 100, image::Rgba([0, 0, 0, 255]));
        for x in 150..300 {
            for y in 0..100 {
                img.put_pixel(x, y, image::Rgba([255, 255, 255, 255]));
            }
        }
        let out = cover(&img, 100, 100);
        assert!(out.get_pixel(10, 50)[0] < 30);
        assert!(out.get_pixel(90, 50)[0] > 225);
    }

    #[test]
    fn gradiente_padrao_e_opaco_e_varia() {
        let g = default_gradient(64, 48);
        assert!(g.pixels().all(|p| p[3] == 255));
        assert_ne!(g.get_pixel(0, 0), g.get_pixel(63, 47));
    }

    /// Gera uma prévia em PNG e mede o tempo em 1920x1080:
    /// `MIDE_PREVIA=/tmp/fundo.png cargo test --lib previa_papel -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn previa_papel_de_parede() {
        let t = std::time::Instant::now();
        let g = default_gradient(1920, 1080);
        println!("papel de parede 1920x1080: {:.1} ms", t.elapsed().as_secs_f32() * 1e3);
        if let Ok(path) = std::env::var("MIDE_PREVIA") {
            g.save(path).unwrap();
        }
    }
}
