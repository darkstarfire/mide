//! Barra de título desenhada pelo mide (decoração de servidor).
//!
//! A barra fica logo acima do conteúdo da janela e tem três botões, como no
//! macOS: fechar, minimizar e maximizar. Arrastar a barra move a janela e o
//! duplo clique alterna a maximização. Os cantos são arredondados com shaders
//! de pixel. O título em texto ainda não é desenhado.

use smithay::{
    backend::renderer::{
        gles::{
            element::PixelShaderElement, GlesError, GlesPixelProgram, GlesRenderer, GlesTexProgram,
            Uniform, UniformName, UniformType,
        },
    },
    utils::{Logical, Point, Rectangle},
};

pub type Rect = Rectangle<i32, Logical>;

/// Elementos de shader com identidade estável entre quadros. O rastreador de dano
/// compara cada elemento pelo `Id` e pela contagem de mudanças: criar um elemento
/// novo a cada quadro (um `Id` novo) faz a área inteira dele contar como mudada.
/// Aqui o n-ésimo elemento de cada programa no quadro de cada tela reaproveita o do
/// quadro anterior; se a área ou os uniformes mudaram, ele é atualizado (e conta
/// como mudado só ele).
mod stable {
    use std::{cell::RefCell, collections::HashMap};

    use smithay::backend::renderer::{
        element::Kind,
        gles::{element::PixelShaderElement, GlesPixelProgram, Uniform},
    };

    use super::Rect;

    #[derive(Default)]
    struct Cache {
        output: String,
        counts: HashMap<u8, usize>,
        /// (tela, programa, ordem) → elemento.
        elements: HashMap<(String, u8, usize), Entry>,
        frame: u64,
    }

    /// Elemento guardado, os uniformes dele e o quadro em que foi usado.
    type Entry = (PixelShaderElement, Vec<Uniform<'static>>, u64);

    thread_local! {
        static CACHE: RefCell<Cache> = RefCell::default();
    }

    /// Começa o quadro de uma tela: a contagem por programa volta a zero e o que não
    /// foi usado nos últimos quadros é esquecido.
    pub fn begin_frame(output: &str) {
        CACHE.with_borrow_mut(|c| {
            c.frame += 1;
            c.output = output.to_string();
            c.counts.clear();
            let frame = c.frame;
            c.elements.retain(|_, (_, _, used)| frame - *used < 8);
        });
    }

    pub fn pixel(program: &GlesPixelProgram, tag: u8, area: Rect, uniforms: Vec<Uniform<'static>>) -> PixelShaderElement {
        CACHE.with_borrow_mut(|c| {
            let n = c.counts.entry(tag).or_default();
            let key = (c.output.clone(), tag, *n);
            *n += 1;
            let frame = c.frame;
            match c.elements.get_mut(&key) {
                Some((element, old, used)) => {
                    element.resize(area, None);
                    if *old != uniforms {
                        element.update_uniforms(uniforms.clone());
                        *old = uniforms;
                    }
                    *used = frame;
                    element.clone()
                }
                None => {
                    let element = PixelShaderElement::new(program.clone(), area, None, 1.0, uniforms.clone(), Kind::Unspecified);
                    c.elements.insert(key, (element.clone(), uniforms, frame));
                    element
                }
            }
        })
    }
}

pub use stable::begin_frame;
use stable::pixel;

/// Altura da barra de título em pixels.
pub const BAR_HEIGHT: i32 = 30;

/// Raio dos cantos da janela, em pixels.
pub const CORNER_RADIUS: i32 = 10;

const BUTTON_SIZE: i32 = 12;
const BUTTON_GAP: i32 = 8;
const BUTTON_MARGIN: i32 = 9;

// Paleta do macOS: barra clara, semáforo colorido quando a janela tem foco
// e botões cinza quando não tem.
const BAR_FOCUSED: [f32; 4] = [0.90, 0.90, 0.90, 1.0];
const BAR_UNFOCUSED: [f32; 4] = [0.96, 0.96, 0.96, 1.0];
const SEPARATOR: [f32; 4] = [0.74, 0.74, 0.74, 1.0];
const CLOSE_COLOR: [f32; 4] = [1.00, 0.37, 0.34, 1.0];
const MINIMIZE_COLOR: [f32; 4] = [1.00, 0.74, 0.18, 1.0];
const MAXIMIZE_COLOR: [f32; 4] = [0.16, 0.78, 0.25, 1.0];
const INACTIVE_BUTTON: [f32; 4] = [0.84, 0.84, 0.84, 1.0];

/// Retângulo arredondado preenchido. Os uniformes são `fill` (RGBA) e `radius`.
const ROUNDED_RECT_SRC: &str = r#"
precision highp float;
uniform float alpha;
uniform vec2 size;
uniform float radius;
uniform vec4 fill;
// Recorte pela moldura da janela (x, y, w, h na saída); largura 0 = sem recorte.
uniform vec4 clip;
uniform float clip_radius;
uniform float output_height;
varying vec2 v_coords;

float sd_round_box(vec2 p, vec2 b, float r) {
    vec2 q = abs(p) - b + vec2(r);
    return min(max(q.x, q.y), 0.0) + length(max(q, vec2(0.0))) - r;
}

void main() {
    vec2 p = v_coords * size - size * 0.5;
    float d = sd_round_box(p, size * 0.5, radius);
    float cover = 1.0 - smoothstep(-0.5, 0.5, d);
    if (clip.z > 0.0) {
        vec2 q = vec2(gl_FragCoord.x, output_height > 0.0 ? output_height - gl_FragCoord.y : gl_FragCoord.y);
        vec2 half_clip = clip.zw * 0.5;
        float dc = sd_round_box(q - (clip.xy + half_clip), half_clip, clip_radius);
        cover *= 1.0 - smoothstep(-0.5, 0.5, dc);
    }
    float a = fill.a * cover * alpha;
    gl_FragColor = vec4(fill.rgb * a, a);
}
"#;

/// Fundo do desktop com a sombra de todas as janelas (até 8), desenhado atrás de
/// tudo. Cor opaca: o fundo escurecido. Como fica atrás, nenhuma sombra apaga outra.
/// Uniformes e funções GLSL da sombra combinada das janelas, compartilhados pelo
/// fundo liso e pelo papel de parede.
pub const SHADOW_GLSL: &str = r#"
uniform vec4 w0;
uniform vec4 w1;
uniform vec4 w2;
uniform vec4 w3;
uniform vec4 w4;
uniform vec4 w5;
uniform vec4 w6;
uniform vec4 w7;
uniform vec4 sa;
uniform vec4 sb;
uniform float radius;
uniform float offset_y;
uniform float margin;

float sd_round_box(vec2 p, vec2 b, float r) {
    vec2 q = abs(p) - b + vec2(r);
    return min(max(q.x, q.y), 0.0) + length(max(q, vec2(0.0))) - r;
}

// Sombra de uma janela (moldura x, y, w, h) no ponto p, em coordenadas da saída.
float shadow_of(vec2 p, vec4 r, float s) {
    if (r.z <= 0.0 || s <= 0.0) {
        return 0.0;
    }
    vec2 half_size = r.zw * 0.5;
    vec2 center = r.xy + half_size + vec2(0.0, offset_y);
    float d = sd_round_box(p - center, half_size, radius);
    float f = 1.0 - smoothstep(0.0, margin, d);
    return s * f * f;
}

// Escurecimento combinado de todas as janelas: as sombras se somam sem se apagar.
float combined_shadow(vec2 p) {
    float keep = 1.0;
    keep *= 1.0 - shadow_of(p, w0, sa.x);
    keep *= 1.0 - shadow_of(p, w1, sa.y);
    keep *= 1.0 - shadow_of(p, w2, sa.z);
    keep *= 1.0 - shadow_of(p, w3, sa.w);
    keep *= 1.0 - shadow_of(p, w4, sb.x);
    keep *= 1.0 - shadow_of(p, w5, sb.y);
    keep *= 1.0 - shadow_of(p, w6, sb.z);
    keep *= 1.0 - shadow_of(p, w7, sb.w);
    return 1.0 - keep;
}

"#;

const DESKTOP_HEAD: &str = r#"
precision highp float;
uniform float alpha;
uniform vec2 size;
uniform vec4 fill;
varying vec2 v_coords;
"#;

const DESKTOP_MAIN: &str = r#"
void main() {
    vec2 p = v_coords * size;
    float shade = combined_shadow(p);
    gl_FragColor = vec4(fill.rgb * (1.0 - shade), 1.0) * alpha;
}
"#;


/// Traço fino de 1 px seguindo a borda arredondada, por dentro da forma.
/// Traço fino de 1 px seguindo a borda arredondada, por dentro da forma.
const OUTLINE_SRC: &str = r#"
precision highp float;
uniform float alpha;
uniform vec2 size;
uniform vec2 half_size;
uniform float radius;
uniform float width;
uniform vec4 fill;
varying vec2 v_coords;

float sd_round_box(vec2 p, vec2 b, float r) {
    vec2 q = abs(p) - b + vec2(r);
    return min(max(q.x, q.y), 0.0) + length(max(q, vec2(0.0))) - r;
}

void main() {
    vec2 p = v_coords * size - size * 0.5;
    float inside = -sd_round_box(p, half_size, radius);
    float stroke = smoothstep(-0.5, 0.5, inside) * (1.0 - smoothstep(width - 0.5, width + 0.5, inside));
    float a = fill.a * stroke * alpha;
    gl_FragColor = vec4(fill.rgb * a, a);
}
"#;


/// Símbolo dos botões quando o ponteiro está sobre eles: × (fechar), − (minimizar)
/// e + (maximizar). `kind` é 0, 1 ou 2.
const GLYPH_SRC: &str = r#"
precision highp float;
uniform float alpha;
uniform float kind;
uniform vec4 fill;
varying vec2 v_coords;

void main() {
    vec2 p = (v_coords - 0.5) * 2.0;
    float box = max(abs(p.x), abs(p.y));
    float dist;
    if (kind < 0.5) {
        dist = min(abs(p.x - p.y), abs(p.x + p.y)) / 1.41421;
    } else if (kind < 1.5) {
        dist = abs(p.y);
    } else {
        dist = min(abs(p.x), abs(p.y));
    }
    float line = 1.0 - smoothstep(0.10, 0.22, dist);
    float inside = 1.0 - smoothstep(0.55, 0.62, box);
    float a = fill.a * line * inside * alpha;
    gl_FragColor = vec4(fill.rgb * a, a);
}
"#;

/// Vidro claro (o mesmo da seleção do launcher e da dock, em `mide::ui::draw::glass`):
/// preenchimento branco translúcido em degradê, borda mais escura em cima e mais clara
/// embaixo, e um brilho fino por dentro, perto do topo.
const GLASS_SRC: &str = r#"
precision highp float;
uniform float alpha;
uniform vec2 size;
uniform float radius;
varying vec2 v_coords;

float sd_round_box(vec2 p, vec2 b, float r) {
    vec2 q = abs(p) - b + vec2(r);
    return min(max(q.x, q.y), 0.0) + length(max(q, vec2(0.0))) - r;
}

void main() {
    vec2 p = v_coords * size - size * 0.5;
    float d = sd_round_box(p, size * 0.5, radius);
    float cover = 1.0 - smoothstep(-0.5, 0.5, d);
    float t = v_coords.y;
    vec4 c = vec4(1.0, 1.0, 1.0, mix(0.62, 0.42, t));
    // Borda de 1 px: escura em cima, clara embaixo.
    float edge = 1.0 - smoothstep(0.0, 1.2, -d);
    c = mix(c, mix(vec4(0.0, 0.0, 0.0, 0.24), vec4(1.0, 1.0, 1.0, 0.7), t), edge);
    // Brilho fino logo por dentro da borda, mais forte em cima.
    float shine = (1.0 - smoothstep(0.0, 1.0, abs(-d - 1.8))) * (1.0 - t) * 0.5;
    c = vec4(mix(c.rgb, vec3(1.0), shine), max(c.a, shine));
    float a = c.a * cover * alpha;
    gl_FragColor = vec4(c.rgb * a, a);
}
"#;

/// Programas de shader usados pela decoração. Compilados uma vez na inicialização.
pub struct Shaders {
    rounded: GlesPixelProgram,
    desktop: GlesPixelProgram,
    clip_tex: GlesTexProgram,
    outline: GlesPixelProgram,
    glyph: GlesPixelProgram,
    glass: GlesPixelProgram,
}

impl Shaders {
    pub fn compile(renderer: &mut GlesRenderer) -> Result<Self, GlesError> {
        let rounded = renderer.compile_custom_pixel_shader(
            ROUNDED_RECT_SRC,
            &[
                UniformName::new("radius", UniformType::_1f),
                UniformName::new("fill", UniformType::_4f),
                UniformName::new("clip", UniformType::_4f),
                UniformName::new("clip_radius", UniformType::_1f),
                UniformName::new("output_height", UniformType::_1f),
            ],
        )?;
        let clip_tex = renderer
            .compile_custom_texture_shader(crate::clip::CLIP_TEX_SRC, &crate::clip::uniform_names())?;
        let mut desktop_uniforms = vec![UniformName::new("fill", UniformType::_4f)];
        desktop_uniforms.extend(shadow_uniform_names());
        let desktop = renderer.compile_custom_pixel_shader(
            format!("{DESKTOP_HEAD}{SHADOW_GLSL}{DESKTOP_MAIN}"),
            &desktop_uniforms,
        )?;
        let outline = renderer.compile_custom_pixel_shader(
            OUTLINE_SRC,
            &[
                UniformName::new("half_size", UniformType::_2f),
                UniformName::new("radius", UniformType::_1f),
                UniformName::new("width", UniformType::_1f),
                UniformName::new("fill", UniformType::_4f),
            ],
        )?;
        let glyph = renderer.compile_custom_pixel_shader(
            GLYPH_SRC,
            &[
                UniformName::new("kind", UniformType::_1f),
                UniformName::new("fill", UniformType::_4f),
            ],
        )?;
        let glass = renderer.compile_custom_pixel_shader(GLASS_SRC, &[UniformName::new("radius", UniformType::_1f)])?;
        Ok(Self {
            rounded,
            desktop,
            clip_tex,
            outline,
            glyph,
            glass,
        })
    }
}

/// Botões da barra de título, da esquerda para a direita.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Close,
    Minimize,
    Maximize,
}

/// Retângulo da barra de uma janela, acima do conteúdo que começa em `content_loc`.
pub fn bar_rect(content_loc: Point<i32, Logical>, content_width: i32) -> Rect {
    Rectangle::new(
        (content_loc.x, content_loc.y - BAR_HEIGHT).into(),
        (content_width, BAR_HEIGHT).into(),
    )
}

/// Retângulo de cada botão, dentro da barra.
fn button_rects(bar: Rectangle<i32, Logical>) -> [(Button, Rectangle<i32, Logical>); 3] {
    let y = bar.loc.y + (BAR_HEIGHT - BUTTON_SIZE) / 2;
    let at = |i: i32| {
        Rectangle::new(
            (bar.loc.x + BUTTON_MARGIN + i * (BUTTON_SIZE + BUTTON_GAP), y).into(),
            (BUTTON_SIZE, BUTTON_SIZE).into(),
        )
    };
    [
        (Button::Close, at(0)),
        (Button::Minimize, at(1)),
        (Button::Maximize, at(2)),
    ]
}

/// Botão da barra que está sob `pos`, se houver.
pub fn button_at(bar: Rectangle<i32, Logical>, pos: Point<f64, Logical>) -> Option<Button> {
    button_rects(bar)
        .into_iter()
        .find(|(_, rect)| rect.to_f64().contains(pos))
        .map(|(button, _)| button)
}

/// Elementos da barra: fundo quadrado e três botões circulares.
/// Cada elemento é um retângulo arredondado. A lista vai da frente para trás,
/// então os botões vêm antes do fundo.
pub fn bar_elements(
    shaders: &Shaders,
    bar: Rect,
    focused: bool,
    hovered: Option<Button>,
    clip: &[Uniform<'static>],
) -> Vec<PixelShaderElement> {
    let mut elements = Vec::with_capacity(12);
    for (button, rect) in button_rects(bar) {
        let color = match (focused, button) {
            (false, _) => INACTIVE_BUTTON,
            (true, Button::Close) => CLOSE_COLOR,
            (true, Button::Minimize) => MINIMIZE_COLOR,
            (true, Button::Maximize) => MAXIMIZE_COLOR,
        };
        // Ordem de frente para trás: símbolo, anel e depois o círculo.
        if focused && hovered == Some(button) {
            elements.push(glyph(shaders, rect, button));
        }
        elements.push(ring(shaders, rect));
        elements.push(rounded(shaders, rect, color, BUTTON_SIZE / 2, clip));
    }
    // Brilho na metade superior e linha clara no topo: dá volume à barra.
    let top_half = Rectangle::new(bar.loc, (bar.size.w, bar.size.h / 2).into());
    elements.push(rounded(shaders, top_half, [1.0, 1.0, 1.0, 0.16], 0, clip));
    let top_line = Rectangle::new(bar.loc, (bar.size.w, 1).into());
    elements.push(rounded(shaders, top_line, [1.0, 1.0, 1.0, 0.7], 0, clip));
    let background = if focused { BAR_FOCUSED } else { BAR_UNFOCUSED };
    elements.push(rounded(shaders, bar, background, 0, clip));
    // Linha fina separando a barra do conteúdo.
    let separator = Rectangle::new((bar.loc.x, bar.loc.y + bar.size.h - 1).into(), (bar.size.w, 1).into());
    elements.push(rounded(shaders, separator, SEPARATOR, 0, clip));
    elements
}

/// Símbolo de um botão, desenhado quando o ponteiro está sobre o grupo de botões.
fn glyph(shaders: &Shaders, rect: Rect, button: Button) -> PixelShaderElement {
    let kind = match button {
        Button::Close => 0.0f32,
        Button::Minimize => 1.0,
        Button::Maximize => 2.0,
    };
    pixel(&shaders.glyph, 0, rect, vec![
            Uniform::new("kind", kind),
            Uniform::new("fill", [0.0, 0.0, 0.0, 0.55]),
        ])
}

/// Anel fino escuro em volta de um botão, para dar contorno ao círculo.
fn ring(shaders: &Shaders, rect: Rect) -> PixelShaderElement {
    pixel(&shaders.outline, 1, rect, vec![
            Uniform::new("half_size", (rect.size.w as f32 / 2.0, rect.size.h as f32 / 2.0)),
            Uniform::new("radius", (rect.size.w / 2) as f32 * crate::clip::scale() as f32),
            Uniform::new("width", 1.0f32),
            Uniform::new("fill", [0.0, 0.0, 0.0, 0.18]),
        ])
}

/// Janela que projeta sombra: moldura e intensidade (0 = sem sombra).
#[derive(Debug, Clone, Copy)]
pub struct Caster {
    pub frame: Rect,
    pub strength: f32,
}

/// Nomes dos uniformes usados por `SHADOW_GLSL`.
pub fn shadow_uniform_names() -> Vec<UniformName<'static>> {
    let mut names: Vec<UniformName<'static>> =
        (0..8).map(|i| UniformName::new(format!("w{i}"), UniformType::_4f)).collect();
    names.push(UniformName::new("sa", UniformType::_4f));
    names.push(UniformName::new("sb", UniformType::_4f));
    names.push(UniformName::new("radius", UniformType::_1f));
    names.push(UniformName::new("offset_y", UniformType::_1f));
    names.push(UniformName::new("margin", UniformType::_1f));
    names
}

/// Uniformes das sombras: até 8 janelas (`w0`..`w7`, `sa`, `sb`) e os parâmetros comuns.
pub fn shadow_uniforms(casters: &[Caster]) -> Vec<Uniform<'static>> {
    let mut out = Vec::with_capacity(13);
    let mut strengths = [0.0f32; 8];
    for i in 0..8 {
        let rect = casters.get(i).map(|c| {
            strengths[i] = c.strength;
            [c.frame.loc.x as f32, c.frame.loc.y as f32, c.frame.size.w as f32, c.frame.size.h as f32]
        });
        out.push(Uniform::new(format!("w{i}"), rect.unwrap_or([0.0; 4])));
    }
    out.push(Uniform::new("sa", [strengths[0], strengths[1], strengths[2], strengths[3]]));
    out.push(Uniform::new("sb", [strengths[4], strengths[5], strengths[6], strengths[7]]));
    out.push(Uniform::new("radius", CORNER_RADIUS as f32 * crate::clip::scale() as f32));
    out.push(Uniform::new("offset_y", SHADOW_OFFSET as f32));
    out.push(Uniform::new("margin", SHADOW_MARGIN as f32));
    out
}

/// Fundo do desktop em tela cheia, com as sombras de todas as janelas.
pub fn desktop_element(shaders: &Shaders, output: Rect, bg: [f32; 4], casters: &[Caster]) -> PixelShaderElement {
    let mut uniforms = vec![Uniform::new("fill", bg)];
    uniforms.extend(shadow_uniforms(casters));
    pixel(&shaders.desktop, 3, output, uniforms)
}

/// Intensidade da sombra de uma janela.
pub fn shadow_strength(focused: bool) -> f32 {
    if focused { 0.45 } else { 0.25 }
}

fn rounded(
    shaders: &Shaders,
    area: Rect,
    color: [f32; 4],
    radius: i32,
    clip: &[Uniform<'static>],
) -> PixelShaderElement {
    let mut uniforms = vec![
        Uniform::new("radius", radius as f32 * crate::clip::scale() as f32),
        Uniform::new("fill", color),
    ];
    uniforms.extend(clip.iter().cloned());
    pixel(&shaders.rounded, 2, area, uniforms)
}

/// Retângulo arredondado sem recorte (painéis do compositor, como as prévias da
/// dock). Os uniformes de recorte vão zerados: o programa guarda o último valor.
pub fn rounded_rect_element(shaders: &Shaders, area: Rect, color: [f32; 4], radius: i32) -> PixelShaderElement {
    let no_clip = [
        Uniform::new("clip", [0.0f32, 0.0, 0.0, 0.0]),
        Uniform::new("clip_radius", 0.0f32),
        Uniform::new("output_height", 0.0f32),
    ];
    rounded(shaders, area, color, radius, &no_clip)
}

/// Vidro claro arredondado (destaque da prévia sob o ponteiro).
pub fn glass_element(shaders: &Shaders, area: Rect, radius: i32) -> PixelShaderElement {
    pixel(&shaders.glass, 4, area, vec![Uniform::new("radius", radius as f32 * crate::clip::scale() as f32)])
}

/// × na cor pedida (botão de fechar das prévias, branco sobre o círculo escuro).
pub fn close_glyph_colored(shaders: &Shaders, rect: Rect, color: [f32; 4]) -> PixelShaderElement {
    pixel(&shaders.glyph, 0, rect, vec![Uniform::new("kind", 0.0f32), Uniform::new("fill", color)])
}

/// Contorno fino arredondado.

impl Shaders {
    /// Programa de textura com recorte arredondado, para o conteúdo das janelas.
    pub fn clip_program(&self) -> GlesTexProgram {
        self.clip_tex.clone()
    }
}

/// Margem em pixels em volta da janela onde a sombra aparece.
pub const SHADOW_MARGIN: i32 = 24;
/// Deslocamento da sombra para baixo, em pixels.
pub const SHADOW_OFFSET: i32 = 8;

/// Traço de 1 px em volta da janela inteira, seguindo os cantos arredondados.
pub fn outline_element(shaders: &Shaders, frame: Rect, focused: bool) -> PixelShaderElement {
    let alpha = if focused { 0.22 } else { 0.12 };
    pixel(&shaders.outline, 1, frame, vec![
            Uniform::new(
                "half_size",
                (frame.size.w as f32 / 2.0, frame.size.h as f32 / 2.0),
            ),
            Uniform::new("radius", CORNER_RADIUS as f32 * crate::clip::scale() as f32),
            Uniform::new("width", 1.0f32),
            Uniform::new("fill", [0.0, 0.0, 0.0, alpha]),
        ])
}

/// Largura, em pixels, da faixa da borda que aciona o redimensionamento.
pub const RESIZE_MARGIN: i32 = 6;

/// Bordas e cantos da janela que estão sendo arrastados.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Edges {
    pub left: bool,
    pub right: bool,
    pub top: bool,
    pub bottom: bool,
}

impl Edges {
    /// Só a borda superior (sem canto): fica para o arrasto da barra de título.
    pub fn only_top(self) -> bool {
        self.top && !self.left && !self.right && !self.bottom
    }
}

/// Bordas sob `pos`, se o ponteiro está na faixa de borda de `frame` (barra + conteúdo).
pub fn resize_edges_at(frame: Rect, pos: Point<f64, Logical>) -> Option<Edges> {
    let f = frame.to_f64();
    if !f.contains(pos) {
        return None;
    }
    let m = RESIZE_MARGIN as f64;
    let edges = Edges {
        left: pos.x < f.loc.x + m,
        right: pos.x > f.loc.x + f.size.w - m,
        top: pos.y < f.loc.y + m,
        bottom: pos.y > f.loc.y + f.size.h - m,
    };
    let any = edges.left || edges.right || edges.top || edges.bottom;
    any.then_some(edges)
}



#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn barra_fica_acima_do_conteudo() {
        let bar = bar_rect((100, 200).into(), 640);
        assert_eq!(bar.loc, (100, 200 - BAR_HEIGHT).into());
        assert_eq!(bar.size, (640, BAR_HEIGHT).into());
    }

    #[test]
    fn botao_fechar_e_o_primeiro() {
        let bar = bar_rect((0, BAR_HEIGHT).into(), 400);
        let close_center = (BUTTON_MARGIN + BUTTON_SIZE / 2, BAR_HEIGHT / 2);
        assert_eq!(
            button_at(bar, (close_center.0 as f64, close_center.1 as f64 ).into()),
            Some(Button::Close)
        );
    }

    #[test]
    fn hover_so_no_botao_sob_o_ponteiro() {
        let bar = bar_rect((0, BAR_HEIGHT).into(), 400);
        let centro_do_minimizar = (BUTTON_MARGIN + BUTTON_SIZE + BUTTON_GAP + BUTTON_SIZE / 2, BAR_HEIGHT / 2);
        assert_eq!(
            button_at(bar, (centro_do_minimizar.0 as f64, centro_do_minimizar.1 as f64).into()),
            Some(Button::Minimize)
        );
        assert_eq!(button_at(bar, (300.0, 15.0).into()), None);
    }

    #[test]
    fn borda_e_canto_detectados() {
        let frame: Rect = Rectangle::new((100, 100).into(), (400, 300).into());
        let canto = resize_edges_at(frame, (101.0, 101.0).into()).unwrap();
        assert!(canto.left && canto.top && !canto.right && !canto.bottom);
        let base = resize_edges_at(frame, (300.0, 399.0).into()).unwrap();
        assert!(base.bottom && !base.left && !base.right);
        assert_eq!(resize_edges_at(frame, (300.0, 250.0).into()), None);
    }

    #[test]
    fn borda_superior_sozinha_fica_para_a_barra() {
        let frame: Rect = Rectangle::new((100, 100).into(), (400, 300).into());
        let topo = resize_edges_at(frame, (300.0, 101.0).into()).unwrap();
        assert!(topo.only_top());
    }

    #[test]
    fn clique_fora_dos_botoes_nao_casa() {
        let bar = bar_rect((0, BAR_HEIGHT).into(), 400);
        assert_eq!(button_at(bar, (300.0, 15.0).into()), None);
    }

}
