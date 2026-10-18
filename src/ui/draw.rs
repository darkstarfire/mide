//! Formas e cores para desenhar a interface dos clientes com tiny-skia.

use tiny_skia::{
    Color, FillRule, GradientStop, LinearGradient, Paint, PathBuilder, Pixmap, Point, Shader,
    SpreadMode, Transform,
};

/// Retângulo arredondado como caminho, com cantos em curvas cúbicas.
pub fn rounded_rect(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<tiny_skia::Path> {
    const K: f32 = 0.5523;
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.cubic_to(x + w - r + r * K, y, x + w, y + r - r * K, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.cubic_to(x + w, y + h - r + r * K, x + w - r + r * K, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.cubic_to(x + r - r * K, y + h, x, y + h - r + r * K, x, y + h - r);
    pb.line_to(x, y + r);
    pb.cubic_to(x, y + r - r * K, x + r - r * K, y, x + r, y);
    pb.close();
    pb.finish()
}

/// Preenche um retângulo arredondado com uma cor.
pub fn fill_rounded(pixmap: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, r: f32, color: Color) {
    if let Some(path) = rounded_rect(x, y, w, h, r) {
        let mut paint = Paint::default();
        paint.set_color(color);
        paint.anti_alias = true;
        pixmap.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
    }
}

/// Apaga (deixa transparente) um retângulo de cantos arredondados: tira a sombra de
/// baixo de um painel translúcido antes de pintá-lo (senão o vidro fica acinzentado).
pub fn clear_rounded(pixmap: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, r: f32) {
    if let Some(path) = rounded_rect(x, y, w, h, r) {
        let mut paint = Paint::default();
        paint.blend_mode = tiny_skia::BlendMode::Clear;
        paint.anti_alias = true;
        pixmap.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
    }
}

/// Pinta um retângulo de cantos arredondados com uma cor sólida, *substituindo* o que
/// havia por baixo (como `clear_rounded` seguido de `fill_rounded`, bem mais rápido em
/// áreas grandes): cada linha é escrita direto, com a extensão calculada pelo canto, e
/// só os pixels da borda misturam (antisserrilhado). `rgba` não premultiplicado.
pub fn paint_rounded_solid(pixmap: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, r: f32, rgba: [u8; 4]) {
    let (pw, ph) = (pixmap.width() as i32, pixmap.height() as i32);
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    let k = rgba[3] as f32 / 255.0;
    let solid = |cov: f32| {
        let a = k * cov;
        tiny_skia::PremultipliedColorU8::from_rgba(
            (rgba[0] as f32 * a).round() as u8,
            (rgba[1] as f32 * a).round() as u8,
            (rgba[2] as f32 * a).round() as u8,
            (255.0 * a).round() as u8,
        )
        .unwrap()
    };
    let full = solid(1.0);
    let pixels = pixmap.pixels_mut();
    let y0 = (y.floor() as i32).max(0);
    let y1 = ((y + h).ceil() as i32).min(ph);
    for row in y0..y1 {
        // Cobertura vertical da linha (bordas de cima e de baixo fracionárias).
        let (top, bottom) = (row as f32, row as f32 + 1.0);
        let vcov = (bottom.min(y + h) - top.max(y)).clamp(0.0, 1.0);
        if vcov <= 0.0 {
            continue;
        }
        // Meia-largura recolhida pelo canto no centro da linha.
        let cy = top + 0.5;
        let dy = if cy < y + r { y + r - cy } else if cy > y + h - r { cy - (y + h - r) } else { 0.0 };
        let inset = if dy > 0.0 { r - (r * r - dy.min(r) * dy.min(r)).sqrt() } else { 0.0 };
        let (left, right) = (x + inset, x + w - inset);
        let xa = (left.floor() as i32).max(0);
        let xb = (right.ceil() as i32).min(pw);
        let base = (row * pw) as usize;
        for col in xa..xb {
            let (cl, cr) = (col as f32, col as f32 + 1.0);
            let hcov = (cr.min(right) - cl.max(left)).clamp(0.0, 1.0);
            let cov = hcov * vcov;
            let p = &mut pixels[base + col as usize];
            if cov >= 0.999 {
                *p = full;
            } else if cov > 0.0 {
                // Borda: substitui na proporção da cobertura (o que havia fica com o
                // resto), como apagar e pintar com antisserrilhado.
                let c = solid(cov);
                let inv = ((1.0 - cov) * 255.0).round() as u32;
                let mix = |s: u8, d: u8| (s as u32 + (d as u32 * inv + 127) / 255).min(255) as u8;
                let a = mix(c.alpha(), p.alpha());
                if let Some(out) = tiny_skia::PremultipliedColorU8::from_rgba(mix(c.red(), p.red()).min(a), mix(c.green(), p.green()).min(a), mix(c.blue(), p.blue()).min(a), a) {
                    *p = out;
                }
            }
        }
    }
}

/// Sombra suave em volta de um retângulo arredondado, calculada direto nos pixels
/// pela distância até a borda: só a faixa em volta é percorrida (o miolo, que o painel
/// cobre, é pulado), então é barata mesmo em painéis grandes. `spread` é a largura da
/// sombra, `drop` desce a sombra e `alpha` é a força perto da borda.
pub fn soft_shadow(pixmap: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, r: f32, spread: f32, drop: f32, alpha: u8) {
    let (pw, ph) = (pixmap.width() as i32, pixmap.height() as i32);
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    let (hw, hh) = (w / 2.0, h / 2.0);
    let (cx, cy) = (x + hw, y + hh + drop);
    let sdf = |px: f32, py: f32| {
        let (qx, qy) = ((px - cx).abs() - hw + r, (py - cy).abs() - hh + r);
        // Fora dos cantos a distância é reta (sem raiz quadrada).
        if qx <= 0.0 || qy <= 0.0 {
            return qx.max(qy) - r;
        }
        qx.hypot(qy) - r
    };
    let x0 = ((x - spread).floor() as i32).max(0);
    let x1 = ((x + w + spread).ceil() as i32).min(pw);
    let y0 = ((y - spread + drop).floor() as i32).max(0);
    let y1 = ((y + h + spread + drop).ceil() as i32).min(ph);
    // O que o painel cobre (pulado): a faixa do meio na largura toda das linhas do
    // painel, e as colunas longe dos cantos nas linhas de cima e de baixo dele.
    let (in_x0, in_x1) = ((x + 1.0).ceil() as i32, (x + w - 1.0).floor() as i32);
    let (mid_y0, mid_y1) = ((y + r).ceil() as i32, (y + h - r).floor() as i32);
    let (cap_x0, cap_x1) = ((x + r).ceil() as i32, (x + w - r).floor() as i32);
    let (cap_y0, cap_y1) = ((y + 1.0).ceil() as i32, (y + h - 1.0).floor() as i32);
    let a = alpha as f32;
    let pixels = pixmap.pixels_mut();
    for row in y0..y1 {
        let mut col = x0;
        while col < x1 {
            if row >= mid_y0 && row < mid_y1 && col >= in_x0 && col < in_x1 {
                col = in_x1;
                continue;
            }
            if row >= cap_y0 && row < cap_y1 && col >= cap_x0 && col < cap_x1 {
                col = cap_x1;
                continue;
            }
            let d = sdf(col as f32 + 0.5, row as f32 + 0.5);
            let t = (1.0 - d.max(0.0) / spread).clamp(0.0, 1.0);
            let sa = (a * t * t).round() as u32;
            if sa > 0 {
                let p = &mut pixels[(row * pw + col) as usize];
                let inv = 255 - sa;
                let k = |c: u8| ((c as u32 * inv + 127) / 255) as u8;
                let out_a = (sa + (p.alpha() as u32 * inv + 127) / 255).min(255) as u8;
                if let Some(c) = tiny_skia::PremultipliedColorU8::from_rgba(k(p.red()).min(out_a), k(p.green()).min(out_a), k(p.blue()).min(out_a), out_a) {
                    *p = c;
                }
            }
            col += 1;
        }
    }
}

/// Gradiente vertical de `top` (em `y0`) até `bottom` (em `y1`).
pub fn vertical_gradient(y0: f32, y1: f32, top: Color, bottom: Color) -> Shader<'static> {
    LinearGradient::new(
        Point::from_xy(0.0, y0),
        Point::from_xy(0.0, y1),
        vec![GradientStop::new(0.0, top), GradientStop::new(1.0, bottom)],
        SpreadMode::Pad,
        Transform::identity(),
    )
    .unwrap_or(Shader::SolidColor(top))
}

/// Vidro claro da dock do mide (e da seleção do launcher): preenchimento translúcido
/// um pouco mais claro em cima, borda fina mais escura em cima e mais clara embaixo
/// (dá volume) e um brilho interno de 1 px logo abaixo da borda de cima.
pub fn glass(pixmap: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, radius: f32) {
    if let Some(path) = rounded_rect(x + 0.5, y + 0.5, w - 1.0, h - 1.0, radius) {
        let mut fill = Paint::default();
        fill.anti_alias = true;
        fill.shader = vertical_gradient(y, y + h, rgba(255, 255, 255, 92), rgba(235, 235, 245, 70));
        pixmap.fill_path(&path, &fill, FillRule::Winding, Transform::identity(), None);
        let mut border = Paint::default();
        border.anti_alias = true;
        border.shader = vertical_gradient(y, y + h, rgba(0, 0, 0, 70), rgba(255, 255, 255, 110));
        pixmap.stroke_path(&path, &border, &tiny_skia::Stroke { width: 1.0, ..Default::default() }, Transform::identity(), None);
    }
    if let Some(path) = rounded_rect(x + 1.5, y + 1.5, w - 3.0, h - 3.0, (radius - 1.0).max(0.0)) {
        let mut shine = Paint::default();
        shine.anti_alias = true;
        shine.shader = vertical_gradient(y, y + h, rgba(255, 255, 255, 60), rgba(255, 255, 255, 0));
        pixmap.stroke_path(&path, &shine, &tiny_skia::Stroke { width: 1.0, ..Default::default() }, Transform::identity(), None);
    }
}

/// Véu do vidro líquido: só um preenchimento claro e leve, sem bordas. A borda curva,
/// o reflexo e o tom são desenhados pelo compositor atrás da superfície.
pub fn glass_veil(pixmap: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, radius: f32) {
    if let Some(path) = rounded_rect(x, y, w, h, radius) {
        let mut fill = Paint::default();
        fill.anti_alias = true;
        fill.shader = vertical_gradient(y, y + h, rgba(255, 255, 255, 34), rgba(255, 255, 255, 18));
        pixmap.fill_path(&path, &fill, FillRule::Winding, Transform::identity(), None);
    }
}

/// Cola `src` em `dst` na posição inteira (`x`, `y`), sem escala, com mistura normal
/// (as duas imagens em RGBA premultiplicado) e `opacity` de 0 a 1. Faz o mesmo que
/// `draw_pixmap` nesse caso, bem mais rápido: pixel opaco é copiado, transparente é
/// pulado, e só as bordas misturam. Ícones e miniaturas passam por aqui.
pub fn blit(dst: &mut Pixmap, src: &Pixmap, x: i32, y: i32, opacity: f32) {
    let (dw, dh, sw, sh) = (dst.width() as i32, dst.height() as i32, src.width() as i32, src.height() as i32);
    let (x0, y0) = (x.max(0), y.max(0));
    let (x1, y1) = ((x + sw).min(dw), (y + sh).min(dh));
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    let k = (opacity.clamp(0.0, 1.0) * 256.0) as u32;
    let src_px = src.pixels();
    let dst_px = dst.pixels_mut();
    for row in y0..y1 {
        let s_row = ((row - y) * sw) as usize;
        let d_row = (row * dw) as usize;
        for col in x0..x1 {
            let sp = src_px[s_row + (col - x) as usize];
            let (mut r, mut g, mut b, mut a) = (sp.red() as u32, sp.green() as u32, sp.blue() as u32, sp.alpha() as u32);
            if k < 256 {
                r = r * k >> 8;
                g = g * k >> 8;
                b = b * k >> 8;
                a = a * k >> 8;
            }
            if a == 0 {
                continue;
            }
            let d = &mut dst_px[d_row + col as usize];
            if a == 255 {
                *d = tiny_skia::PremultipliedColorU8::from_rgba(r as u8, g as u8, b as u8, 255).unwrap();
                continue;
            }
            let inv = 255 - a;
            let mix = |s: u32, d: u8| -> u8 { (s + (d as u32 * inv + 127) / 255).min(255) as u8 };
            let out_a = mix(a, d.alpha());
            let (or, og, ob) = (mix(r, d.red()).min(out_a), mix(g, d.green()).min(out_a), mix(b, d.blue()).min(out_a));
            if let Some(c) = tiny_skia::PremultipliedColorU8::from_rgba(or, og, ob, out_a) {
                *d = c;
            }
        }
    }
}

/// Copia o pixmap (RGBA premultiplicado) para um buffer wl_shm Argb8888 (B, G, R, A).
/// Troca R e B de 4 em 4 bytes como um número só (bem mais rápido que byte a byte).
pub fn to_argb8888(pixmap: &Pixmap, canvas: &mut [u8]) {
    argb_row(pixmap.data(), canvas);
}

/// Converte um trecho de pixels RGBA premultiplicados para Argb8888 (B, G, R, A).
pub fn argb_row(src: &[u8], canvas: &mut [u8]) {
    for (dst, src) in canvas.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
        let v = u32::from_le_bytes([src[0], src[1], src[2], src[3]]);
        let swapped = (v & 0xFF00_FF00) | ((v & 0x0000_00FF) << 16) | ((v >> 16) & 0x0000_00FF);
        dst.copy_from_slice(&swapped.to_le_bytes());
    }
}

/// Preenche o pixmap inteiro com um gradiente vertical, linha a linha: uma cor por
/// linha, copiada ao longo dela. Para fundos de tela cheia é dezenas de vezes mais
/// rápido que o gradiente genérico do tiny-skia.
pub fn fill_vertical_fast(pixmap: &mut Pixmap, top: [u8; 4], bottom: [u8; 4]) {
    let (w, h) = (pixmap.width() as usize, pixmap.height() as usize);
    let pixels = pixmap.pixels_mut();
    for y in 0..h {
        let t = if h > 1 { y as f32 / (h - 1) as f32 } else { 0.0 };
        let mix = |a: u8, b: u8| a as f32 + (b as f32 - a as f32) * t;
        let alpha = mix(top[3], bottom[3]);
        let k = alpha / 255.0;
        let color = tiny_skia::PremultipliedColorU8::from_rgba(
            (mix(top[0], bottom[0]) * k).round() as u8,
            (mix(top[1], bottom[1]) * k).round() as u8,
            (mix(top[2], bottom[2]) * k).round() as u8,
            alpha.round() as u8,
        );
        if let Some(color) = color {
            pixels[y * w..(y + 1) * w].fill(color);
        }
    }
}

/// Cor a partir de RGBA em bytes.
pub fn rgba(r: u8, g: u8, b: u8, a: u8) -> Color {
    Color::from_rgba8(r, g, b, a)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pintura_solida_igual_a_apagar_e_preencher() {
        let mut a = Pixmap::new(60, 40).unwrap();
        a.fill(tiny_skia::Color::from_rgba8(30, 30, 30, 120));
        let mut b = a.clone();
        clear_rounded(&mut a, 5.0, 4.0, 48.0, 30.0, 9.0);
        fill_rounded(&mut a, 5.0, 4.0, 48.0, 30.0, 9.0, tiny_skia::Color::from_rgba8(248, 248, 250, 196));
        paint_rounded_solid(&mut b, 5.0, 4.0, 48.0, 30.0, 9.0, [248, 248, 250, 196]);
        // Miolo idêntico. Nas bordas antisserrilhadas a pintura direta interpola pela
        // cobertura; apagar e depois pintar (tiny-skia) reduz o fundo duas vezes, então
        // ali a diferença pode chegar a ~25%.
        let px = |p: &Pixmap, x: u32, y: u32| p.pixel(x, y).unwrap();
        assert_eq!(px(&a, 30, 20), px(&b, 30, 20));
        assert_eq!(px(&a, 2, 2), px(&b, 2, 2), "fora fica como estava");
        for (p, q) in a.pixels().iter().zip(b.pixels()) {
            assert!((p.alpha() as i32 - q.alpha() as i32).abs() <= 64, "{} vs {}", p.alpha(), q.alpha());
        }
    }

    #[test]
    fn blit_igual_ao_draw_pixmap() {
        // Ícone com miolo opaco, borda semitransparente e cantos vazios.
        let mut icon = Pixmap::new(8, 8).unwrap();
        icon.fill(tiny_skia::Color::from_rgba8(200, 40, 40, 120));
        if let Some(path) = rounded_rect(1.0, 1.0, 6.0, 6.0, 2.0) {
            let mut paint = Paint::default();
            paint.set_color(tiny_skia::Color::from_rgba8(20, 120, 220, 255));
            icon.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
        }
        for opacity in [1.0, 0.35] {
            let mut a = Pixmap::new(12, 12).unwrap();
            a.fill(tiny_skia::Color::from_rgba8(240, 240, 245, 255));
            let mut b = a.clone();
            let paint = tiny_skia::PixmapPaint { opacity, ..Default::default() };
            a.draw_pixmap(-2, 6, icon.as_ref(), &paint, Transform::identity(), None);
            blit(&mut b, &icon, -2, 6, opacity);
            for (p, q) in a.pixels().iter().zip(b.pixels()) {
                for (u, v) in [(p.red(), q.red()), (p.green(), q.green()), (p.blue(), q.blue()), (p.alpha(), q.alpha())] {
                    assert!((u as i32 - v as i32).abs() <= 2, "{u} vs {v} (opacidade {opacity})");
                }
            }
        }
    }

    #[test]
    fn conversao_troca_vermelho_e_azul() {

        let mut p = Pixmap::new(1, 1).unwrap();
        p.fill(Color::from_rgba8(10, 20, 30, 255));
        let mut out = [0u8; 4];
        to_argb8888(&p, &mut out);
        assert_eq!(out, [30, 20, 10, 255]);
    }

    #[test]
    fn gradiente_rapido_igual_nas_pontas() {
        let mut p = Pixmap::new(4, 3).unwrap();
        fill_vertical_fast(&mut p, [255, 0, 0, 255], [0, 0, 255, 255]);
        assert_eq!(p.pixel(0, 0).unwrap().red(), 255);
        assert_eq!(p.pixel(3, 2).unwrap().blue(), 255);
        assert_eq!(p.pixel(2, 1).unwrap().red(), p.pixel(0, 1).unwrap().red());
    }
}
