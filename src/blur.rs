//! Desfoque (blur) do compositor, pela técnica "dual Kawase" (a mesma do KDE e do
//! Hyprland): a imagem é reduzida à metade algumas vezes e depois ampliada de volta,
//! misturando os vizinhos em cada passada. Fica barato na GPU e bem suave.
//!
//! Usos:
//! - Fundo do launcher: o papel de parede desfocado (calculado uma vez por papel de
//!   parede e tamanho de tela).
//! - Dock: o que estiver atrás dela (janelas e papel de parede), com os cantos
//!   arredondados da dock e o "vidro líquido": refração nas bordas, reflexo e tom.
//!   O desfoque só é refeito quando o que está atrás muda (assinatura dos elementos);
//!   as texturas intermediárias são reaproveitadas entre quadros.

use std::{cell::RefCell, collections::HashMap};

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            element::{texture::TextureRenderElement, Element, Id, Kind, RenderElement},
            gles::{
                element::TextureShaderElement, GlesError, GlesRenderer, GlesTexProgram, GlesTexture, Uniform,
                UniformName, UniformType,
            },
            Bind, Color32F, Frame, Offscreen, Renderer, Texture,
        },
    },
    utils::{Logical, Physical, Rectangle, Scale, Size, Transform},
};

const HEADER: &str = r#"#version 100

//_DEFINES_

#if defined(EXTERNAL)
#extension GL_OES_EGL_image_external : require
#endif

precision highp float;
#if defined(EXTERNAL)
uniform samplerExternalOES tex;
#else
uniform sampler2D tex;
#endif

uniform float alpha;
varying vec2 v_coords;

#if defined(DEBUG_FLAGS)
uniform float tint;
#endif
"#;

/// Redução: o ponto central pesa 4 e os quatro cantos, 1 cada.
const DOWN_SRC: &str = r#"
uniform vec2 halfpixel;
uniform float offset;

void main() {
    vec2 uv = v_coords;
    vec4 sum = texture2D(tex, uv) * 4.0;
    sum += texture2D(tex, uv - halfpixel * offset);
    sum += texture2D(tex, uv + halfpixel * offset);
    sum += texture2D(tex, uv + vec2(halfpixel.x, -halfpixel.y) * offset);
    sum += texture2D(tex, uv - vec2(halfpixel.x, -halfpixel.y) * offset);
    gl_FragColor = sum / 8.0 * alpha;
}
"#;

/// Ampliação: oito vizinhos em volta, os diagonais com peso dobrado.
const UP_SRC: &str = r#"
uniform vec2 halfpixel;
uniform float offset;

void main() {
    vec2 uv = v_coords;
    vec4 sum = texture2D(tex, uv + vec2(-halfpixel.x * 2.0, 0.0) * offset);
    sum += texture2D(tex, uv + vec2(-halfpixel.x, halfpixel.y) * offset) * 2.0;
    sum += texture2D(tex, uv + vec2(0.0, halfpixel.y * 2.0) * offset);
    sum += texture2D(tex, uv + vec2(halfpixel.x, halfpixel.y) * offset) * 2.0;
    sum += texture2D(tex, uv + vec2(halfpixel.x * 2.0, 0.0) * offset);
    sum += texture2D(tex, uv + vec2(halfpixel.x, -halfpixel.y) * offset) * 2.0;
    sum += texture2D(tex, uv + vec2(0.0, -halfpixel.y * 2.0) * offset);
    sum += texture2D(tex, uv + vec2(-halfpixel.x, -halfpixel.y) * offset) * 2.0;
    gl_FragColor = sum / 12.0 * alpha;
}
"#;

/// Resultado desfocado na tela, recortado num retângulo de cantos arredondados, com o
/// "vidro líquido" quando `glass` > 0:
/// - refração: perto da borda, a imagem de trás é puxada para dentro, como numa lente
///   (a borda parece curva e grossa);
/// - tom: um pouco mais de saturação e de luz, mais forte sobre fundos escuros, para o
///   conteúdo da dock continuar legível;
/// - reflexo: linha fina e clara na borda, mais forte do lado da luz (em cima, à
///   esquerda), e um brilho suave na faixa curva.
/// Tudo vem da distância até a borda (calculada, sem texturas extras): uma leitura de
/// textura por pixel, como antes.
const COMPOSITE_SRC: &str = r#"
uniform vec2 area_size;
uniform float corner;
uniform float glass;
// 1 quando a cena atrás foi desenhada invertida no eixo y (winit), 0 no DRM.
uniform float flip_y;

float sd_round_box(vec2 p, vec2 b, float r) {
    vec2 q = abs(p) - b + vec2(r);
    return min(max(q.x, q.y), 0.0) + length(max(q, vec2(0.0))) - r;
}

void main() {
    vec2 half_size = area_size * 0.5;
    vec2 p = v_coords * area_size - half_size;
    float d = sd_round_box(p, half_size, corner);
    // Direção para fora da borda mais próxima (gradiente da distância).
    vec2 e = vec2(0.75, 0.0);
    vec2 n = vec2(sd_round_box(p + e.xy, half_size, corner) - sd_round_box(p - e.xy, half_size, corner),
                  sd_round_box(p + e.yx, half_size, corner) - sd_round_box(p - e.yx, half_size, corner));
    n = n / max(length(n), 0.0001);
    // Faixa curva da borda: 0 no miolo, 1 na borda.
    float bevel = max(min(corner * 1.1, min(half_size.x, half_size.y)), 1.0);
    float t = clamp(1.0 + d / bevel, 0.0, 1.0);
    float bend = t * t;
    vec2 uv = v_coords - n * bend * glass * bevel * 1.3 / area_size;
    // No winit a cena atrás foi desenhada de baixo para cima (como a saída): desvira y.
    vec3 c = texture2D(tex, vec2(uv.x, mix(uv.y, 1.0 - uv.y, flip_y))).rgb;
    float lum = dot(c, vec3(0.299, 0.587, 0.114));
    c = mix(vec3(lum), c, 1.0 + 0.3 * glass);
    c = mix(c, vec3(1.0), glass * mix(0.20, 0.07, lum));
    // Reflexo: luz vindo de cima e um pouco da esquerda.
    vec2 light = normalize(vec2(-0.45, -1.0));
    float facing = dot(n, light);
    // Borda suave: faixa um pouco mais larga e bem mais fraca que uma linha.
    float rim = 1.0 - smoothstep(0.0, 2.4, -d);
    float spec = rim * (0.06 + 0.20 * max(facing, 0.0) + 0.06 * max(-facing, 0.0));
    float sheen = bend * 0.08 * max(facing, 0.0);
    c = clamp(c + glass * (spec + sheen), 0.0, 1.0);
    float cover = 1.0 - smoothstep(-0.5, 0.5, d);
    gl_FragColor = vec4(c, 1.0) * cover * alpha;
}
"#;

fn uniforms_pass() -> Vec<UniformName<'static>> {
    vec![UniformName::new("halfpixel", UniformType::_2f), UniformName::new("offset", UniformType::_1f)]
}

pub struct Blur {
    down: GlesTexProgram,
    up: GlesTexProgram,
    composite: GlesTexProgram,
    /// Papel de parede desfocado: chave (textura de origem, tamanho) e o resultado.
    wallpaper: RefCell<Option<((usize, Size<i32, Logical>), GlesTexture)>>,
    /// Texturas reaproveitadas entre quadros (fundo da dock e passadas dela), pelo
    /// tamanho e pelo papel (0: fundo; 16 + i: redução i; 32 + i: ampliação i). Cada
    /// passada tem a sua: em áreas pequenas, níveis vizinhos têm o mesmo tamanho, e uma
    /// textura só faria a GPU ler e escrever na mesma imagem (trava o driver).
    /// Também pela região de vidro dona delas: o desfoque guardado de uma região não
    /// pode ser sobrescrito por outra do mesmo tamanho.
    pool: RefCell<HashMap<(i32, i32, u8, String), GlesTexture>>,
    /// Último desfoque de cada região de vidro (dock, barra, menus...): assinatura do
    /// que estava atrás, textura e tamanho.
    regions: RefCell<HashMap<String, (u64, GlesTexture, Size<i32, Logical>)>>,
}

/// Passadas do desfoque (cada uma reduz à metade) e o deslocamento das amostras.
pub const LAUNCHER_PASSES: u32 = 5;
pub const DOCK_PASSES: u32 = 3;

impl Blur {
    pub fn compile(renderer: &mut GlesRenderer) -> Result<Self, GlesError> {
        let down = renderer.compile_custom_texture_shader(format!("{HEADER}{DOWN_SRC}"), &uniforms_pass())?;
        let up = renderer.compile_custom_texture_shader(format!("{HEADER}{UP_SRC}"), &uniforms_pass())?;
        let composite = renderer.compile_custom_texture_shader(
            format!("{HEADER}{COMPOSITE_SRC}"),
            // "size" e "radius" não chegam ao shader de textura (nomes já usados pelo
            // Smithay); os nomes daqui são próprios.
            &[
                UniformName::new("area_size", UniformType::_2f),
                UniformName::new("corner", UniformType::_1f),
                UniformName::new("glass", UniformType::_1f),
                UniformName::new("flip_y", UniformType::_1f),
            ],
        )?;
        Ok(Self {
            down,
            up,
            composite,
            wallpaper: RefCell::new(None),
            pool: RefCell::new(HashMap::new()),
            regions: RefCell::new(HashMap::new()),
        })
    }

    /// Textura do tamanho pedido: a guardada para esse papel e essa região (se `role`
    /// vier) ou nova.
    fn target(&self, renderer: &mut GlesRenderer, size: Size<i32, Logical>, role: Option<(u8, &str)>) -> Option<GlesTexture> {
        let Some((role, owner)) = role else {
            return renderer.create_buffer(Fourcc::Abgr8888, (size.w, size.h).into()).ok();
        };
        let key = (size.w, size.h, role, owner.to_string());
        if let Some(texture) = self.pool.borrow().get(&key) {
            return Some(texture.clone());
        }
        let texture: GlesTexture = renderer.create_buffer(Fourcc::Abgr8888, (size.w, size.h).into()).ok()?;
        let mut pool = self.pool.borrow_mut();
        // Tamanhos antigos (dock que mudou de largura) não ficam acumulando.
        if pool.len() > 64 {
            pool.clear();
        }
        pool.insert(key, texture.clone());
        Some(texture)
    }

    /// Desfoque de uma região de vidro (`name`: "mide-dock", "mide-bar#0"...), refeito
    /// só quando a assinatura `key` do que está atrás muda. `backdrop` desenha o que
    /// está atrás (numa textura do tamanho da saída).
    #[allow(clippy::too_many_arguments)]
    pub fn region_blur(
        &self,
        renderer: &mut GlesRenderer,
        name: &str,
        key: u64,
        out_size: Size<i32, Logical>,
        src: Rectangle<f64, Logical>,
        glass: f32,
        backdrop: impl FnOnce(&mut GlesRenderer, GlesTexture) -> Option<GlesTexture>,
    ) -> Option<(GlesTexture, Size<i32, Logical>)> {
        // Área pequena demais (a dock no primeiro quadro, antes de ter tamanho).
        if src.size.w < 16.0 || src.size.h < 16.0 {
            return None;
        }
        let key = key ^ glass.to_bits() as u64;
        if let Some((k, texture, size)) = self.regions.borrow().get(name) {
            if *k == key {
                return Some((texture.clone(), *size));
            }
        }
        // O fundo desenhado é só a entrada das passadas: pode ser o mesmo para todas.
        let target = self.target(renderer, out_size, Some((0, "")))?;
        let texture = backdrop(renderer, target)?;
        let (passes, offset) = if glass > 0.0 { (DOCK_PASSES - 1, 1.6) } else { (DOCK_PASSES, 2.0) };
        let (blurred, size) = self.blur_with(renderer, &texture, out_size, src, passes, offset, Some(name))?;
        tracing::debug!("vidro: desfoque refeito ({name})");
        let mut regions = self.regions.borrow_mut();
        if regions.len() > 32 {
            regions.clear();
        }
        regions.insert(name.to_string(), (key, blurred.clone(), size));
        Some((blurred, size))
    }

    /// Desenha a região `src` de `texture` (de tamanho total `tex_size`) inteira em
    /// `target` (de tamanho `size`), com um dos programas de passada.
    fn pass(
        &self,
        renderer: &mut GlesRenderer,
        program: &GlesTexProgram,
        texture: &GlesTexture,
        tex_size: Size<i32, Logical>,
        src: Rectangle<f64, Logical>,
        size: Size<i32, Logical>,
        offset: f32,
        role: Option<(u8, &str)>,
    ) -> Option<GlesTexture> {
        let mut target = self.target(renderer, size, role)?;
        let inner = TextureRenderElement::from_static_texture(
            Id::new(),
            renderer.context_id(),
            (0.0, 0.0),
            texture.clone(),
            1,
            Transform::Normal,
            None,
            Some(src),
            Some(size),
            None,
            Kind::Unspecified,
        );
        let halfpixel = (0.5 / tex_size.w.max(1) as f32, 0.5 / tex_size.h.max(1) as f32);
        let element = TextureShaderElement::new(
            inner,
            program.clone(),
            vec![Uniform::new("halfpixel", halfpixel), Uniform::new("offset", offset)],
        );
        {
            let mut fb = renderer.bind(&mut target).ok()?;
            let mut frame = renderer.render(&mut fb, (size.w, size.h).into(), Transform::Normal).ok()?;
            let full = Rectangle::from_size((size.w, size.h).into());
            frame.clear(Color32F::new(0.0, 0.0, 0.0, 0.0), &[full]).ok()?;
            let dst = element.geometry(Scale::from(1.0));
            let _ = element.draw(&mut frame, element.src(), dst, &[Rectangle::from_size(dst.size)], &[]);
            let _ = frame.finish();
        }
        Some(target)
    }

    /// Desfoca a região `src` de `texture`: `passes` reduções e as ampliações de volta.
    /// Devolve a textura (com metade do tamanho da região) e o tamanho dela.
    pub fn blur(
        &self,
        renderer: &mut GlesRenderer,
        texture: &GlesTexture,
        tex_size: Size<i32, Logical>,
        src: Rectangle<f64, Logical>,
        passes: u32,
        offset: f32,
    ) -> Option<(GlesTexture, Size<i32, Logical>)> {
        self.blur_with(renderer, texture, tex_size, src, passes, offset, None)
    }

    #[allow(clippy::too_many_arguments)]
    fn blur_with(
        &self,
        renderer: &mut GlesRenderer,
        texture: &GlesTexture,
        tex_size: Size<i32, Logical>,
        src: Rectangle<f64, Logical>,
        passes: u32,
        offset: f32,
        reuse: Option<&str>,
    ) -> Option<(GlesTexture, Size<i32, Logical>)> {
        let base = Size::<i32, Logical>::from((src.size.w.round() as i32, src.size.h.round() as i32));
        let level = |i: u32| -> Size<i32, Logical> { ((base.w >> i).max(1), (base.h >> i).max(1)).into() };
        // Reduções.
        let mut chain: Vec<(GlesTexture, Size<i32, Logical>)> = Vec::new();
        let mut current = (texture.clone(), tex_size, src);
        for i in 1..=passes {
            let size = level(i);
            let out = self.pass(renderer, &self.down, &current.0, current.1, current.2, size, offset, reuse.map(|o| (16 + i as u8, o)))?;
            chain.push((out.clone(), size));
            current = (out, size, Rectangle::from_size((size.w as f64, size.h as f64).into()));
        }
        // Ampliações, de volta até a metade do tamanho original.
        for i in (1..passes).rev() {
            let size = level(i);
            let out = self.pass(renderer, &self.up, &current.0, current.1, current.2, size, offset, reuse.map(|o| (32 + i as u8, o)))?;
            current = (out, size, Rectangle::from_size((size.w as f64, size.h as f64).into()));
        }
        Some((current.0, current.1))
    }

    /// Papel de parede desfocado para o fundo do launcher (guardado entre quadros).
    pub fn wallpaper_element(
        &self,
        renderer: &mut GlesRenderer,
        wallpaper: Option<(GlesTexture, Size<i32, Logical>)>,
        output: Rectangle<i32, Logical>,
        alpha: f32,
    ) -> Option<TextureRenderElement<GlesTexture>> {
        let (texture, tex_size) = wallpaper?;
        let key = (texture.tex_id() as usize, output.size);
        let mut cache = self.wallpaper.borrow_mut();
        if cache.as_ref().map(|(k, _)| *k) != Some(key) {
            let src = Rectangle::from_size((tex_size.w as f64, tex_size.h as f64).into());
            let (blurred, _) = self.blur(renderer, &texture, tex_size, src, LAUNCHER_PASSES, 3.0)?;
            tracing::info!("fundo do launcher desfocado ({}x{})", output.size.w, output.size.h);
            *cache = Some((key, blurred));
        }
        let (_, blurred) = cache.as_ref()?;
        let size = blurred.size();
        // Opaco só quando aparece por inteiro (no esmaecer, o que está atrás aparece).
        let opaque = (alpha >= 1.0).then(|| vec![Rectangle::from_size((output.size.w, output.size.h).into())]);
        Some(TextureRenderElement::from_static_texture(
            Id::new(),
            renderer.context_id(),
            (output.loc.x as f64, output.loc.y as f64),
            blurred.clone(),
            1,
            Transform::Normal,
            Some(alpha),
            Some(Rectangle::from_size((size.w as f64, size.h as f64).into())),
            Some(output.size),
            opaque,
            Kind::Unspecified,
        ))
    }

    /// Elemento com o desfoque `blurred` em `area`, recortado com cantos de `radius`;
    /// `glass` (0 a 1) é a força do vidro líquido (0: só o desfoque).
    pub fn composite_element(
        &self,
        renderer: &mut GlesRenderer,
        blurred: GlesTexture,
        blurred_size: Size<i32, Logical>,
        area: Rectangle<i32, Logical>,
        radius: f32,
        glass: f32,
        alpha: f32,
    ) -> TextureShaderElement {
        // Com `Transform::Flipped180` aqui, o Smithay não varia `v_coords` (o recorte
        // dos cantos some); a orientação é acertada na amostragem do shader.
        let inner = TextureRenderElement::from_static_texture(
            Id::new(),
            renderer.context_id(),
            crate::clip::phys(area.loc).to_f64(),
            blurred,
            1,
            Transform::Normal,
            (alpha < 1.0).then_some(alpha),
            Some(Rectangle::from_size((blurred_size.w as f64, blurred_size.h as f64).into())),
            Some(area.size),
            None,
            Kind::Unspecified,
        );
        TextureShaderElement::new(
            inner,
            self.composite.clone(),
            vec![
                // Em pixels da tela: a forma (cantos, borda) acompanha a escala.
                Uniform::new("area_size", (area.size.w as f32 * crate::clip::scale() as f32, area.size.h as f32 * crate::clip::scale() as f32)),
                Uniform::new("corner", radius * crate::clip::scale() as f32),
                Uniform::new("glass", glass),
                Uniform::new("flip_y", if crate::clip::flipped() { 1.0f32 } else { 0.0 }),
            ],
        )
    }
}

/// Desenha os elementos (de trás para a frente) numa textura do tamanho de `area`,
/// como se a textura fosse aquele pedaço da tela.
pub fn render_region<E: RenderElement<GlesRenderer>>(
    renderer: &mut GlesRenderer,
    elements: &[E],
    area: Rectangle<i32, Logical>,
    output_size: Size<i32, Logical>,
    mut target: GlesTexture,
) -> Option<GlesTexture> {
    // A textura cobre a saída inteira (mesmo tamanho, para os recortes das janelas,
    // que usam a posição do pixel, continuarem certos); depois só a região é usada e
    // só ela é limpa e desenhada.
    {
        let mut fb = renderer.bind(&mut target).ok()?;
        // Mesma orientação da saída: os recortes das janelas usam a posição do pixel.
        let mut frame = renderer.render(&mut fb, (output_size.w, output_size.h).into(), crate::clip::output_transform()).ok()?;
        let region: Rectangle<i32, Physical> = Rectangle::new((area.loc.x, area.loc.y).into(), (area.size.w, area.size.h).into());
        frame.clear(Color32F::new(0.12, 0.12, 0.14, 1.0), &[region]).ok()?;
        for element in elements.iter().rev() {
            let dst = element.geometry(Scale::from(crate::clip::scale()));
            let Some(visible) = dst.intersection(region) else {
                continue;
            };
            let damage = Rectangle::new(visible.loc - dst.loc, visible.size);
            let _ = element.draw(&mut frame, element.src(), dst, &[damage], &[]);
        }
        let _ = frame.finish();
    }
    Some(target)
}
