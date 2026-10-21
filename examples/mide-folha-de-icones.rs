//! Folha de contato de ícones (revisão do tema de ícones, B15): desenha os SVG/PNG
//! pedidos numa grade com o mesmo carregador do mide (resvg) e escreve a lista com o
//! número de cada um. O ImageMagick falha em muitos SVGs do tema; este não.
//!
//! `cargo run --example mide-folha-de-icones -- folha.png 48 arquivo1.svg arquivo2.svg ...`

use tiny_skia::{Color, Pixmap};

fn main() {
    let mut args = std::env::args().skip(1);
    let out = args.next().expect("uso: mide-folha-de-icones <saída.png> <tamanho> <ícones...>");
    let size: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(48);
    let files: Vec<String> = args.collect();
    let (cols, cell) = (16usize, size + 12);
    let rows = files.len().div_ceil(cols).max(1);
    let mut sheet = Pixmap::new(cols as u32 * cell, rows as u32 * cell).expect("folha");
    sheet.fill(Color::from_rgba8(236, 236, 240, 255));
    for (i, file) in files.iter().enumerate() {
        let (x, y) = ((i % cols) as u32 * cell + 6, (i / cols) as u32 * cell + 6);
        match mide::icons::load_icon_file(file, size) {
            Some(icon) => mide::ui::draw::blit(&mut sheet, &icon, x as i32, y as i32, 1.0),
            None => eprintln!("não abriu: {file}"),
        }
        println!("{i}\t{file}");
    }
    sheet.save_png(&out).expect("gravar a folha");
}
