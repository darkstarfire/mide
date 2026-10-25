//! Texto com as fontes do sistema (cosmic-text), desenhado num pixmap do tiny-skia.

use std::collections::HashMap;

use cosmic_text::{Attrs, Buffer, CacheKeyFlags, Color, Family, FontSystem, Metrics, Shaping, Stretch, Style, SwashCache, SwashContent, SwashImage, Weight};
use tiny_skia::Pixmap;

/// Peso da fonte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FontWeight {
    Regular,
    Semibold,
    Bold,
}

impl FontWeight {
    fn to_cosmic(self) -> Weight {
        match self {
            FontWeight::Regular => Weight::NORMAL,
            FontWeight::Semibold => Weight::SEMIBOLD,
            FontWeight::Bold => Weight::BOLD,
        }
    }
}

/// Família e estilo da fonte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Face {
    Sans,
    Mono,
    MonoItalic,
}

/// Rosto (arquivo) escolhido da fonte de largura fixa para um estilo. Quando a família
/// não tem negrito ou itálico, o rosto normal é usado e o estilo é imitado (negrito
/// desenhado duas vezes, itálico inclinado): pedir um peso que a família não tem faz
/// o cosmic-text buscar outra fonte, quase sempre proporcional (letras finas em
/// células largas: "Proj eto s").
#[derive(Debug, Clone, PartialEq)]
struct MonoFace {
    family: String,
    weight: Weight,
    style: Style,
    stretch: Stretch,
    fake_bold: bool,
    fake_italic: bool,
}

/// Imagem de uma célula do terminal já desenhada: imagem e onde ela fica em relação
/// ao canto da célula.
type CellImage = (Pixmap, i32, i32);

/// Chave das células já desenhadas: texto, tamanho, célula, espaço disponível,
/// negrito, itálico e cor.
type CellKey = (String, u32, (u32, u32), u32, bool, bool, [u8; 4]);

/// Atributos do cosmic-text que escolhem exatamente o rosto `face`.
fn mono_attrs(face: &MonoFace) -> Attrs<'_> {
    let flags = if face.fake_italic { CacheKeyFlags::FAKE_ITALIC } else { CacheKeyFlags::empty() };
    Attrs::new().family(Family::Name(&face.family)).weight(face.weight).style(face.style).stretch(face.stretch).cache_key_flags(flags)
}

/// Ícone da Nerd Font (área de uso privado do Unicode): desenhado para caber na
/// célula (ou nas duas, quando a seguinte está livre), e não na medida da fonte.
pub fn is_icon(c: char) -> bool {
    matches!(c as u32, 0xE000..=0xF8FF | 0xF0000..=0xFFFFD | 0x100000..=0x10FFFD)
}

/// Um glifo pronto para colar: posição do canto da imagem em relação à origem do
/// texto (x a partir da caneta, y a partir da linha de base) e a imagem.
struct PlacedGlyph {
    x: i32,
    y: i32,
    image: SwashImage,
}

/// Retângulo ocupado pela tinta dos glifos (x0, y0, x1, y1), relativo à origem.
fn ink(glyphs: &[PlacedGlyph]) -> Option<(i32, i32, i32, i32)> {
    glyphs.iter().filter(|g| g.image.placement.width > 0 && g.image.placement.height > 0).fold(None, |acc, g| {
        let (x0, y0) = (g.x, g.y);
        let (x1, y1) = (g.x + g.image.placement.width as i32, g.y + g.image.placement.height as i32);
        Some(match acc {
            None => (x0, y0, x1, y1),
            Some((a, b, c, d)) => (a.min(x0), b.min(y0), c.max(x1), d.max(y1)),
        })
    })
}

/// Cola a imagem de um glifo (máscara ou colorida) no pixmap, na cor `rgba`.
fn composite(pixmap: &mut Pixmap, image: &SwashImage, (ox, oy): (i32, i32), rgba: [u8; 4]) {
    let (pw, ph) = (pixmap.width() as i32, pixmap.height() as i32);
    let (w, h) = (image.placement.width as i32, image.placement.height as i32);
    let data = pixmap.data_mut();
    for py in 0..h {
        for px in 0..w {
            let (tx, ty) = (ox + px, oy + py);
            if tx < 0 || ty < 0 || tx >= pw || ty >= ph {
                continue;
            }
            let i = (py * w + px) as usize;
            let (color, a) = match image.content {
                SwashContent::Mask => ([rgba[0], rgba[1], rgba[2]], image.data[i] as u32 * rgba[3] as u32 / 255),
                SwashContent::Color => {
                    let c = &image.data[i * 4..i * 4 + 4];
                    ([c[0], c[1], c[2]], c[3] as u32)
                }
                SwashContent::SubpixelMask => {
                    let c = &image.data[i * 4..i * 4 + 3];
                    ([rgba[0], rgba[1], rgba[2]], (c[0] as u32 + c[1] as u32 + c[2] as u32) / 3 * rgba[3] as u32 / 255)
                }
            };
            if a == 0 {
                continue;
            }
            let o = ((ty * pw + tx) * 4) as usize;
            for k in 0..3 {
                let src = color[k] as u32 * a / 255;
                data[o + k] = (src + data[o + k] as u32 * (255 - a) / 255).min(255) as u8;
            }
            data[o + 3] = (a + data[o + 3] as u32 * (255 - a) / 255).min(255) as u8;
        }
    }
}

/// Composição e desenho de texto. Carregar as fontes é caro, então crie um só.
pub struct Text {
    fonts: FontSystem,
    swash: SwashCache,
    /// Larguras já medidas: medir é caro, e as interfaces medem os mesmos textos a
    /// cada quadro (por exemplo, ao redimensionar uma janela).
    widths: HashMap<(String, u32, FontWeight, bool), f32>,
    /// Textos já desenhados (imagem, deslocamento e largura): as interfaces desenham os
    /// mesmos rótulos a cada quadro, e colar a imagem pronta é muito mais rápido que
    /// compor as letras de novo.
    images: HashMap<(String, u32, FontWeight, [u8; 4], Face), (Pixmap, i32, i32, f32)>,
    /// Família da fonte de largura fixa (`None`: a monoespaçada padrão do sistema).
    mono_family: Option<String>,
    /// A monoespaçada padrão do sistema (a do fontconfig), procurada uma vez.
    system_mono: Option<String>,
    /// Nerd Font instalada para os ícones que a fonte escolhida não tem (procurada uma
    /// vez; `Some(None)`: nenhuma).
    nerd_family: Option<Option<String>>,
    /// Rostos escolhidos por (negrito, itálico) para a família atual.
    mono_faces: HashMap<(bool, bool), MonoFace>,
    /// Células do terminal já desenhadas.
    cells: HashMap<CellKey, CellImage>,
}

impl Default for Text {
    fn default() -> Self {
        Self::new()
    }
}

impl Text {
    /// As fontes do sistema (para quem dá forma ao próprio texto, como o Editor de Texto).
    pub fn font_system(&mut self) -> &mut FontSystem {
        &mut self.fonts
    }

    /// Desenha uma letra já posicionada (`key` com a posição em subpixel; `(x, y)` em
    /// pixels inteiros, na linha de base) na cor `rgba`.
    pub fn draw_glyph(&mut self, pixmap: &mut Pixmap, key: cosmic_text::CacheKey, (x, y): (i32, i32), rgba: [u8; 4]) {
        if let Some(image) = self.swash.get_image(&mut self.fonts, key).as_ref() {
            composite(pixmap, image, (x + image.placement.left, y - image.placement.top), rgba);
        }
    }

    /// Famílias de fonte instaladas (os nomes, em ordem alfabética, sem repetir).
    pub fn families(&self) -> Vec<String> {
        let mut names: Vec<String> = self.fonts.db().faces().filter_map(|f| f.families.first().map(|(n, _)| n.clone())).collect();
        names.sort_by_key(|n| n.to_lowercase());
        names.dedup();
        names
    }

    /// A família existe no sistema?
    pub fn has_font_family(&self, name: &str) -> bool {
        self.has_family(name)
    }

    pub fn new() -> Self {
        Self {
            fonts: FontSystem::new(),
            swash: SwashCache::new(),
            widths: HashMap::new(),
            images: HashMap::new(),
            mono_family: None,
            system_mono: None,
            nerd_family: None,
            mono_faces: HashMap::new(),
            cells: HashMap::new(),
        }
    }

    fn buffer(&mut self, text: &str, size: f32, weight: FontWeight, face: Face) -> Buffer {
        let mut buffer = Buffer::new(&mut self.fonts, Metrics::new(size, size * 1.25));
        buffer.set_size(Some(100_000.0), Some(size * 1.5));
        let mono = match face {
            Face::Sans => None,
            _ => Some(self.mono_face(weight != FontWeight::Regular, face == Face::MonoItalic)),
        };
        let attrs = match &mono {
            None => Attrs::new().family(Family::SansSerif).weight(weight.to_cosmic()),
            Some(f) => mono_attrs(f),
        };
        buffer.set_text(text, &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.fonts, false);
        buffer
    }

    /// Troca a família da fonte de largura fixa (terminal, editor). Uma família que
    /// não existe volta à padrão do sistema.
    pub fn set_mono_family(&mut self, name: Option<&str>) {
        let name = name.map(str::trim).filter(|n| !n.is_empty()).filter(|n| self.has_family(n)).map(str::to_string);
        if name != self.mono_family {
            self.mono_family = name;
            self.widths.clear();
            self.images.clear();
            self.mono_faces.clear();
            self.cells.clear();
        }
    }

    fn has_family(&self, name: &str) -> bool {
        self.fonts.db().faces().any(|f| f.families.iter().any(|(n, _)| n == name))
    }

    /// Família de largura fixa em uso: a escolhida ou a padrão do sistema.
    pub fn mono_family_name(&mut self) -> String {
        if let Some(name) = &self.mono_family {
            return name.clone();
        }
        if self.system_mono.is_none() {
            self.system_mono = Some(self.find_system_mono());
        }
        self.system_mono.clone().unwrap_or_default()
    }

    /// A monoespaçada padrão do sistema: a que o fontconfig indica (`fc-match
    /// monospace`, a mesma dos outros apps), ou uma das conhecidas, ou a primeira de
    /// largura fixa instalada. O cosmic-text, sozinho, procura sempre a "Noto Sans
    /// Mono", que pode não existir.
    fn find_system_mono(&mut self) -> String {
        let fc = std::process::Command::new("fc-match")
            .args(["-f", "%{family[0]}", "monospace"])
            .stderr(std::process::Stdio::null())
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
        let known = ["DejaVu Sans Mono", "Noto Sans Mono", "Liberation Mono", "Ubuntu Mono", "Hack", "Fira Mono", "Source Code Pro", "Noto Mono"];
        let candidates = fc.into_iter().chain(known.iter().map(|s| s.to_string()));
        for name in candidates {
            if self.fonts.db().faces().any(|f| f.monospaced && f.families.iter().any(|(n, _)| *n == name)) {
                return name;
            }
        }
        self.monospace_families().into_iter().next().unwrap_or_else(|| "monospace".into())
    }

    /// Rosto da família de largura fixa para o estilo pedido (ver `MonoFace`).
    fn mono_face(&mut self, bold: bool, italic: bool) -> MonoFace {
        if let Some(face) = self.mono_faces.get(&(bold, italic)) {
            return face.clone();
        }
        let family = self.mono_family_name();
        let want = if bold { 700 } else { 400 };
        let best = self
            .fonts
            .db()
            .faces()
            .filter(|f| f.families.iter().any(|(n, _)| *n == family))
            .map(|f| (f.weight, f.style, f.stretch))
            .min_by_key(|(w, style, stretch)| {
                let slanted = *style != Style::Normal;
                (*stretch != Stretch::Normal) as i32 * 100_000 + (slanted != italic) as i32 * 10_000 + (w.0 as i32 - want).abs()
            });
        let face = match best {
            Some((weight, style, stretch)) => MonoFace {
                family,
                weight,
                style,
                stretch,
                fake_bold: bold && weight.0 < 600,
                fake_italic: italic && style == Style::Normal,
            },
            None => MonoFace { family, weight: Weight(want as u16), style: Style::Normal, stretch: Stretch::Normal, fake_bold: false, fake_italic: italic },
        };
        self.mono_faces.insert((bold, italic), face.clone());
        face
    }

    /// Famílias de fonte de largura fixa instaladas, em ordem alfabética. Inclui as
    /// que não se declaram de largura fixa mas têm todas as letras da mesma largura
    /// (as Nerd Fonts sem "Mono" no nome, como a "Cousine Nerd Font": os ícones delas
    /// são mais largos, e o arquivo não se diz monoespaçado).
    pub fn monospace_families(&mut self) -> Vec<String> {
        let mut names: Vec<String> = self
            .fonts
            .db()
            .faces()
            .filter(|f| f.monospaced)
            .filter_map(|f| f.families.first().map(|(name, _)| name.clone()))
            .collect();
        let mut maybe: Vec<String> = self
            .fonts
            .db()
            .faces()
            .filter(|f| !f.monospaced)
            .filter_map(|f| f.families.first().map(|(name, _)| name.clone()))
            .filter(|n| n.contains("Nerd Font") || n.contains("Mono") || n.contains("Code"))
            .collect();
        maybe.sort();
        maybe.dedup();
        for name in maybe {
            if !names.contains(&name) && self.letters_same_width(&name) {
                names.push(name);
            }
        }
        names.sort_by_key(|n| n.to_lowercase());
        names.dedup();
        names
    }

    /// "i", "M", "W" e "." têm a mesma largura na família (e vêm dela, não de outra)?
    fn letters_same_width(&mut self, family: &str) -> bool {
        let mut buffer = Buffer::new(&mut self.fonts, Metrics::new(20.0, 25.0));
        buffer.set_size(Some(10_000.0), Some(40.0));
        buffer.set_text("iMW.", &Attrs::new().family(Family::Name(family)), Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.fonts, false);
        let glyphs: Vec<(f32, cosmic_text::fontdb::ID)> = buffer.layout_runs().flat_map(|r| r.glyphs.iter().map(|g| (g.w, g.font_id))).collect();
        glyphs.len() == 4
            && glyphs.iter().all(|(w, _)| (w - glyphs[0].0).abs() < 0.01)
            && glyphs.iter().all(|(_, id)| self.fonts.db().face(*id).is_some_and(|f| f.families.iter().any(|(n, _)| n == family)))
    }

    /// Medidas da fonte de largura fixa em pixels: avanço de uma letra, subida e
    /// descida (a partir da linha de base).
    pub fn mono_metrics(&mut self, size: f32) -> (f32, f32, f32) {
        let face = self.mono_face(false, false);
        let mut buffer = Buffer::new(&mut self.fonts, Metrics::new(size, size * 1.25));
        buffer.set_size(Some(10_000.0), Some(size * 2.0));
        buffer.set_text("M", &mono_attrs(&face), Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.fonts, false);
        let glyph = buffer.layout_runs().flat_map(|r| r.glyphs.iter().map(|g| (g.w, g.font_id, g.font_weight))).next();
        let Some((advance, id, weight)) = glyph else {
            return (size * 0.6, size * 0.9, size * 0.25);
        };
        let Some(font) = self.fonts.get_font(id, weight) else {
            return (advance, size * 0.9, size * 0.25);
        };
        let m = font.metrics();
        let k = size / m.units_per_em.max(1) as f32;
        (advance, m.ascent * k, -m.descent * k)
    }

    /// Tamanho da célula da grade do terminal (pixels inteiros): o avanço da fonte e a
    /// altura da linha (1,3 vez o tamanho, ou a altura da fonte, se maior).
    pub fn mono_cell(&mut self, size: f32) -> (f32, f32) {
        let (advance, ascent, descent) = self.mono_metrics(size);
        (advance.round().max(1.0), (size * 1.3).round().max((ascent + descent).round()).max(1.0))
    }

    /// Desenha um caractere (com os acentos que o acompanham) numa célula do terminal
    /// com o canto em `(x, y)`. A letra fica na grade: centrada na célula pelo avanço
    /// e apoiada na linha de base da fonte principal, também quando vem de outra fonte
    /// (ícones, símbolos). Ícones mais largos ou altos que a célula são reduzidos para
    /// caber em `room` pixels de largura (uma ou duas células) e na altura da célula.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_cell(&mut self, pixmap: &mut Pixmap, text: &str, (x, y): (f32, f32), cell: (f32, f32), room: f32, size: f32, rgba: [u8; 4], bold: bool, italic: bool) {
        let k = super::draw::scale();
        if k != 1.0 {
            super::draw::set_scale(1.0);
            self.draw_cell(pixmap, text, (x * k, y * k), (cell.0 * k, cell.1 * k), room * k, size * k, rgba, bold, italic);
            super::draw::set_scale(k);
            return;
        }
        let key: CellKey = (text.to_string(), size.to_bits(), (cell.0.to_bits(), cell.1.to_bits()), room.to_bits(), bold, italic, rgba);
        if !self.cells.contains_key(&key) {
            let image = self.render_cell(text, cell, room, size, rgba, bold, italic);
            if self.cells.len() > 8192 {
                self.cells.clear();
            }
            self.cells.insert(key.clone(), image);
        }
        let (image, dx, dy) = &self.cells[&key];
        crate::ui::draw::blit_px(pixmap, image, x.round() as i32 + dx, y.round() as i32 + dy, 1.0);
    }

    /// Nerd Font para os ícones: a só de símbolos, se houver, ou uma Mono, ou outra.
    fn nerd_family(&mut self) -> Option<String> {
        if self.nerd_family.is_none() {
            let mut names: Vec<String> =
                self.fonts.db().faces().flat_map(|f| f.families.iter().map(|(n, _)| n.clone())).filter(|n| n.contains("Nerd Font")).collect();
            names.sort();
            names.dedup();
            let rank = |n: &String| match (n.starts_with("Symbols"), n.contains("Mono")) {
                (true, true) => 0,
                (true, false) => 1,
                (false, true) => 2,
                _ => 3,
            };
            names.sort_by_key(rank);
            self.nerd_family = Some(names.into_iter().next());
        }
        self.nerd_family.clone().flatten()
    }

    /// Glifos do texto no rosto `face`: posições em relação à caneta e à linha de base,
    /// o avanço total e se todos vieram da própria família (e não de outra fonte).
    fn cell_glyphs(&mut self, text: &str, size: f32, face: &MonoFace) -> (Vec<PlacedGlyph>, f32, bool) {
        let mut buffer = Buffer::new(&mut self.fonts, Metrics::new(size, size * 1.25));
        buffer.set_size(Some(10_000.0), Some(size * 3.0));
        buffer.set_text(text, &mono_attrs(face), Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.fonts, false);
        let mut out = Vec::new();
        let mut advance = 0.0;
        let mut own = true;
        let glyphs: Vec<cosmic_text::LayoutGlyph> = buffer.layout_runs().flat_map(|r| r.glyphs.iter().cloned()).collect();
        for g in glyphs {
            advance += g.w;
            own &= self.fonts.db().face(g.font_id).is_some_and(|f| f.families.iter().any(|(n, _)| *n == face.family));
            let physical = g.physical((0.0, 0.0), 1.0);
            if let Some(image) = self.swash.get_image(&mut self.fonts, physical.cache_key).clone() {
                out.push(PlacedGlyph { x: physical.x + image.placement.left, y: physical.y - image.placement.top, image });
            }
        }
        (out, advance, own)
    }

    #[allow(clippy::too_many_arguments)]
    fn render_cell(&mut self, text: &str, (cw, ch): (f32, f32), room: f32, size: f32, rgba: [u8; 4], bold: bool, italic: bool) -> CellImage {
        let face = self.mono_face(bold, italic);
        let (_, ascent, descent) = self.mono_metrics(size);
        // Linha de base: a altura da fonte centrada na célula.
        let baseline = ((ch - (ascent + descent)) / 2.0 + ascent).round() as i32;
        let icon = text.chars().next().is_some_and(is_icon);
        let (mut glyphs, mut advance, own) = self.cell_glyphs(text, size, &face);
        // Ícone que a fonte não tem: de uma Nerd Font instalada. A busca geral do
        // cosmic-text pegaria qualquer fonte com algo nesse ponto da área privada
        // (Material Icons, Tabler...), com outro desenho.
        let mut face = face;
        if let Some(nerd) = (icon && !own).then(|| self.nerd_family()).flatten() {
            let nerd_face = MonoFace { family: nerd, weight: Weight::NORMAL, style: Style::Normal, stretch: Stretch::Normal, fake_bold: false, fake_italic: false };
            let (g, a, from_nerd) = self.cell_glyphs(text, size, &nerd_face);
            if from_nerd {
                (glyphs, advance, face) = (g, a, nerd_face);
            }
        }
        let mut bbox = ink(&glyphs);
        // Largura máxima do desenho: a célula, ou as duas menos uma folga (o ícone
        // estendido sobre o espaço não encosta no texto que vem depois).
        let max_w = if room > cw + 0.5 { (room - (cw * 0.4).round().max(1.0)).max(cw) } else { cw };
        // Ícones e símbolos de outra fonte maiores que o espaço: reduzidos para caber.
        let fit = |b: (i32, i32, i32, i32)| ((max_w / (b.2 - b.0).max(1) as f32).min(ch / (b.3 - b.1).max(1) as f32)).min(1.0);
        if let Some(b) = bbox.filter(|b| icon || (!own && (b.2 - b.0) as f32 > cw + 1.0)) {
            let f = fit(b);
            if f < 1.0 {
                // Um pouco menor que a conta exata: o arredondamento dos pixels não corta.
                let (g, a, _) = self.cell_glyphs(text, size * f * 0.97, &face);
                glyphs = g;
                advance = a;
                bbox = ink(&glyphs);
            }
        }
        let scaled_icon = icon || (!own && bbox.is_some_and(|b| (b.2 - b.0) as f32 > cw + 1.0));
        let (ox, oy) = match bbox {
            Some(b) if scaled_icon || !own && b.1 + baseline < 0 => {
                let width = (b.2 - b.0) as f32;
                // Centrado na célula, se couber; senão, a partir do começo da célula,
                // sobre o espaço seguinte.
                let ox = if width <= cw + 0.5 { ((cw - width) / 2.0).floor() as i32 - b.0 } else { -b.0 };
                // Na linha de base, mas sem sair da célula por cima nem por baixo.
                let mut oy = baseline;
                if oy + b.3 > ch as i32 {
                    oy = ch as i32 - b.3;
                }
                if oy + b.1 < 0 {
                    oy = -b.1;
                }
                (ox, oy)
            }
            _ => {
                // Letras: centradas pelo avanço na célula (ou nas duas, se largas).
                let area = if advance > cw * 1.5 { room.max(cw) } else { cw };
                (((area - advance) / 2.0).round() as i32, baseline)
            }
        };
        // Margem para letras que passam da célula (itálicos, acentos).
        let margin = (size * 0.5).ceil() as i32;
        let (pw, ph) = (room.max(cw).ceil() as i32 + 2 * margin, ch.ceil() as i32 + 2 * margin);
        let mut pixmap = Pixmap::new(pw.max(1) as u32, ph.max(1) as u32).unwrap_or_else(|| Pixmap::new(1, 1).unwrap());
        for g in &glyphs {
            let at = (margin + ox + g.x, margin + oy + g.y);
            composite(&mut pixmap, &g.image, at, rgba);
            if face.fake_bold {
                // Negrito imitado: o mesmo glifo um pixel à direita.
                composite(&mut pixmap, &g.image, (at.0 + 1, at.1), rgba);
            }
        }
        (pixmap, -margin, -margin)
    }

    /// Largura do texto em pixels.
    pub fn measure(&mut self, text: &str, size: f32, weight: FontWeight) -> f32 {
        self.measure_with(text, size, weight, false)
    }

    /// Largura do texto em fonte de largura fixa (editor de texto).
    pub fn measure_mono(&mut self, text: &str, size: f32) -> f32 {
        self.measure_with(text, size, FontWeight::Regular, true)
    }

    fn measure_with(&mut self, text: &str, size: f32, weight: FontWeight, mono: bool) -> f32 {
        // Em telas com escala, mede no tamanho em pixels e devolve em pontos: a largura
        // bate com o texto desenhado (`draw_with`).
        let k = super::draw::scale();
        if k != 1.0 {
            super::draw::set_scale(1.0);
            let width = self.measure_with(text, size * k, weight, mono) / k;
            super::draw::set_scale(k);
            return width;
        }
        let key = (text.to_string(), size.to_bits(), weight, mono);
        if let Some(width) = self.widths.get(&key) {
            return *width;
        }
        let buffer = self.buffer(text, size, weight, if mono { Face::Mono } else { Face::Sans });
        let width = buffer.layout_runs().map(|r| r.line_w).fold(0.0, f32::max);
        if self.widths.len() > 8192 {
            self.widths.clear();
        }
        self.widths.insert(key, width);
        width
    }

    /// Desenha `text` com o topo da linha em `(x, y)`. Devolve a largura desenhada.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        pixmap: &mut Pixmap,
        text: &str,
        x: f32,
        y: f32,
        size: f32,
        weight: FontWeight,
        rgba: [u8; 4],
    ) -> f32 {
        self.draw_with(pixmap, text, (x, y), size, weight, rgba, Face::Sans)
    }

    /// Como `draw`, com posição e tamanho já em pixels do canvas (sem a escala da tela):
    /// texto desenhado sobre uma imagem (marcações do Editor de Capturas).
    #[allow(clippy::too_many_arguments)]
    pub fn draw_px(&mut self, pixmap: &mut Pixmap, text: &str, x: f32, y: f32, size: f32, weight: FontWeight, rgba: [u8; 4]) -> f32 {
        let k = super::draw::scale();
        super::draw::set_scale(1.0);
        let width = self.draw_with(pixmap, text, (x, y), size, weight, rgba, Face::Sans);
        super::draw::set_scale(k);
        width
    }

    /// Desenha em fonte de largura fixa (editor de texto).
    pub fn draw_mono(&mut self, pixmap: &mut Pixmap, text: &str, x: f32, y: f32, size: f32, rgba: [u8; 4]) -> f32 {
        self.draw_with(pixmap, text, (x, y), size, FontWeight::Regular, rgba, Face::Mono)
    }

    /// Fonte de largura fixa em negrito e/ou itálico (terminal).
    #[allow(clippy::too_many_arguments)]
    pub fn draw_mono_styled(&mut self, pixmap: &mut Pixmap, text: &str, x: f32, y: f32, size: f32, rgba: [u8; 4], bold: bool, italic: bool) -> f32 {
        let weight = if bold { FontWeight::Bold } else { FontWeight::Regular };
        let face = if italic { Face::MonoItalic } else { Face::Mono };
        self.draw_with(pixmap, text, (x, y), size, weight, rgba, face)
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_with(&mut self, pixmap: &mut Pixmap, text: &str, (x, y): (f32, f32), size: f32, weight: FontWeight, rgba: [u8; 4], face: Face) -> f32 {
        // Pontos para pixels (ver `draw::scale`): o texto é rasterizado no tamanho real.
        let k = super::draw::scale();
        if k != 1.0 {
            super::draw::set_scale(1.0);
            let width = self.draw_with(pixmap, text, (x * k, y * k), size * k, weight, rgba, face) / k;
            super::draw::set_scale(k);
            return width;
        }
        let key = (text.to_string(), size.to_bits(), weight, rgba, face);
        if !self.images.contains_key(&key) {
            let image = self.render(text, size, weight, rgba, face);
            if self.images.len() > 4096 {
                self.images.clear();
            }
            self.images.insert(key.clone(), image);
        }
        let (image, dx, dy, width) = &self.images[&key];
        crate::ui::draw::blit(pixmap, image, x.round() as i32 + dx, y.round() as i32 + dy, 1.0);
        *width
    }

    /// Desenha o texto numa imagem transparente só dele: a imagem, onde ela fica em
    /// relação ao ponto do texto e a largura do texto.
    fn render(&mut self, text: &str, size: f32, weight: FontWeight, rgba: [u8; 4], face: Face) -> (Pixmap, i32, i32, f32) {
        let mut buffer = self.buffer(text, size, weight, face);
        let width = buffer.layout_runs().map(|r| r.line_w).fold(0.0, f32::max);
        // Margem para letras que passam um pouco do retângulo (itálicos, acentos).
        let margin = (size * 0.5).ceil() as i32;
        let (pw, ph) = (width.ceil() as i32 + 2 * margin, (size * 1.5).ceil() as i32 + 2 * margin);
        let mut pixmap = Pixmap::new(pw.max(1) as u32, ph.max(1) as u32).unwrap_or_else(|| Pixmap::new(1, 1).unwrap());
        let color = Color::rgba(rgba[0], rgba[1], rgba[2], rgba[3]);
        let data = pixmap.data_mut();
        buffer.draw(&mut self.fonts, &mut self.swash, color, |gx, gy, w, h, c| {
            let a = c.a() as u32;
            if a == 0 {
                return;
            }
            for py in 0..h as i32 {
                for px in 0..w as i32 {
                    let (tx, ty) = (margin + gx + px, margin + gy + py);
                    if tx < 0 || ty < 0 || tx >= pw || ty >= ph {
                        continue;
                    }
                    let i = ((ty * pw + tx) * 4) as usize;
                    // Sobreposição premultiplicada.
                    for (k, ch) in [c.r(), c.g(), c.b()].into_iter().enumerate() {
                        let src = ch as u32 * a / 255;
                        let dst = data[i + k] as u32;
                        data[i + k] = (src + dst * (255 - a) / 255).min(255) as u8;
                    }
                    let dst_a = data[i + 3] as u32;
                    data[i + 3] = (a + dst_a * (255 - a) / 255).min(255) as u8;
                }
            }
        });
        (pixmap, -margin, -margin, width)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Retângulo da tinta (alfa > 0) de uma célula desenhada, relativo ao canto da
    /// célula: (x0, y0, x1, y1).
    fn cell_ink(text: &mut Text, s: &str, cell: (f32, f32), room: f32, size: f32, bold: bool) -> Option<(i32, i32, i32, i32)> {
        let (pix, dx, dy) = text.render_cell(s, cell, room, size, [0, 0, 0, 255], bold, false);
        let mut out: Option<(i32, i32, i32, i32)> = None;
        for y in 0..pix.height() as i32 {
            for x in 0..pix.width() as i32 {
                if pix.pixels()[(y * pix.width() as i32 + x) as usize].alpha() > 24 {
                    let (cx, cy) = (x + dx, y + dy);
                    out = Some(match out {
                        None => (cx, cy, cx + 1, cy + 1),
                        Some((a, b, c, d)) => (a.min(cx), b.min(cy), c.max(cx + 1), d.max(cy + 1)),
                    });
                }
            }
        }
        out
    }

    /// Famílias de teste que estiverem instaladas (a padrão e as Nerd Fonts daqui).
    fn familias(text: &mut Text) -> Vec<Option<&'static str>> {
        let mut out = vec![None];
        for f in ["Cousine Nerd Font", "CodeNewRoman Nerd Font Mono", "DejaVu Sans Mono", "Noto Sans Mono"] {
            if text.has_family(f) {
                out.push(Some(f));
            }
        }
        out
    }

    #[test]
    fn monoespacada_do_sistema() {
        let mut text = Text::new();
        let name = text.mono_family_name();
        assert!(text.has_family(&name), "{name}");
        // A escolhida na configuração vale; uma que não existe volta à do sistema.
        text.set_mono_family(Some("Fonte Que Não Existe"));
        assert_eq!(text.mono_family_name(), name);
    }

    #[test]
    fn negrito_e_italico_ficam_na_familia() {
        // Sem negrito na família, o normal é usado e o negrito é imitado (antes, o
        // cosmic-text trocava por uma fonte proporcional: "Proj eto s").
        let mut text = Text::new();
        for family in familias(&mut text).into_iter().flatten() {
            text.set_mono_family(Some(family));
            for (bold, italic) in [(false, false), (true, false), (false, true), (true, true)] {
                let face = text.mono_face(bold, italic);
                assert_eq!(face.family, family);
                assert_eq!(face.stretch, Stretch::Normal, "{family}: rosto condensado");
                for letter in ["j", "P", "o", "M"] {
                    let (_, advance, own) = text.cell_glyphs(letter, 15.0, &face);
                    assert!(own, "{family} {letter} (negrito {bold}, itálico {italic}) veio de outra fonte");
                    let (cw, _) = text.mono_cell(15.0);
                    assert!((advance - cw).abs() <= 1.0, "{family} {letter}: avanço {advance} numa célula de {cw}");
                }
            }
        }
    }

    #[test]
    fn nerd_font_sem_mono_no_nome_aparece_na_lista() {
        let mut text = Text::new();
        let list = text.monospace_families();
        if text.has_family("Cousine Nerd Font") {
            assert!(list.iter().any(|f| f == "Cousine Nerd Font"), "{list:?}");
        }
        // Fontes proporcionais ficam de fora.
        for f in ["DejaVu Sans", "Noto Sans"] {
            assert!(!list.iter().any(|n| n == f), "{f} na lista");
        }
    }

    #[test]
    fn letras_dentro_da_celula_na_linha_de_base() {
        let mut text = Text::new();
        for family in familias(&mut text) {
            text.set_mono_family(family);
            for size in [10.0, 13.0, 17.0, 24.0] {
                let cell = text.mono_cell(size);
                let (cw, ch) = (cell.0 as i32, cell.1 as i32);
                for bold in [false, true] {
                    for letter in ["M", "j", "g", "W", "_", "│"] {
                        let (x0, y0, x1, y1) = cell_ink(&mut text, letter, cell, cell.0, size, bold).unwrap();
                        assert!(x0 >= -1 && x1 <= cw + 2, "{family:?} {size} {letter}: x {x0}..{x1} em {cw}");
                        assert!(y0 >= -1 && y1 <= ch + 1, "{family:?} {size} {letter}: y {y0}..{y1} em {ch}");
                    }
                    // A base de "M" e de "x" na mesma linha (linha de base comum).
                    let m = cell_ink(&mut text, "M", cell, cell.0, size, bold).unwrap();
                    let x = cell_ink(&mut text, "x", cell, cell.0, size, bold).unwrap();
                    assert!((m.3 - x.3).abs() <= 1, "{family:?} {size}: base de M {} e de x {}", m.3, x.3);
                }
            }
        }
    }

    #[test]
    fn icones_em_uma_ou_duas_celulas() {
        let mut text = Text::new();
        if text.nerd_family().is_none() {
            return; // sem Nerd Font instalada
        }
        for family in familias(&mut text) {
            text.set_mono_family(family);
            for size in [10.0, 13.0, 17.0, 24.0] {
                let cell = text.mono_cell(size);
                let (cw, ch) = (cell.0 as i32, cell.1 as i32);
                // Pasta, Vim, pasta aberta, Git, Rust, Ubuntu (faixas diferentes da Nerd Font).
                for icon in ["\u{e5ff}", "\u{e62b}", "\u{f115}", "\u{f1d3}", "\u{e7a8}", "\u{f31b}", "\u{f0219}"] {
                    for (room, cells) in [(cell.0, 1), (2.0 * cell.0, 2)] {
                        let Some((x0, y0, x1, y1)) = cell_ink(&mut text, icon, cell, room, size, false) else {
                            panic!("{family:?} {icon:?} sem desenho");
                        };
                        assert!(x0 >= 0 && x1 <= cells * cw, "{family:?} {size} {icon:?}: x {x0}..{x1} em {cells} célula(s) de {cw}");
                        assert!(y0 >= 0 && y1 <= ch, "{family:?} {size} {icon:?}: y {y0}..{y1} em {ch}");
                        // Visível: pelo menos metade da célula de largura ou de altura.
                        assert!((x1 - x0) * 2 >= cw || (y1 - y0) * 2 >= ch, "{family:?} {size} {icon:?}: pequeno demais");
                    }
                }
            }
        }
    }

    /// Bancada: `cargo test --lib bancada_texto -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn bancada_texto() {
        let mut text = Text::new();
        let mut pix = Pixmap::new(400, 40).unwrap();
        text.draw(&mut pix, "aquecer", 0.0, 0.0, 14.0, FontWeight::Regular, [0, 0, 0, 255]);
        let t = std::time::Instant::now();
        for i in 0..40 {
            let s = format!("Resultado número {i} — Configurações");
            text.measure(&s, 14.0, FontWeight::Regular);
            text.draw(&mut pix, &s, 0.0, 0.0, 14.0, FontWeight::Regular, [0, 0, 0, 255]);
        }
        println!("40 textos novos (medir + desenhar): {:.2} ms", t.elapsed().as_secs_f32() * 1e3);
    }
}
