//! Recorte arredondado do conteúdo das janelas.
//!
//! O conteúdo do app é desenhado com um shader de textura que zera os pixels fora
//! de um retângulo arredondado (a moldura da janela). Assim os cantos mostram o que
//! está de fato atrás (o desktop com sombra ou outra janela), sem máscaras pintadas
//! por cima.

use smithay::{
    backend::renderer::{
        element::{Element, Id, Kind, RenderElement, UnderlyingStorage},
        gles::{GlesError, GlesFrame, GlesRenderer, GlesTexProgram, Uniform, UniformName, UniformType},
        utils::{CommitCounter, DamageSet, OpaqueRegions},
    },
    utils::{Buffer, Logical, Physical, Point, Rectangle, Scale, Transform},
};

/// Shader de textura com recorte arredondado. Segue o modelo do shader padrão do
/// Smithay (variantes `EXTERNAL`, `NO_ALPHA` e `DEBUG_FLAGS`).
pub const CLIP_TEX_SRC: &str = r#"#version 100

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

// Moldura da janela em coordenadas da saída (x, y, w, h) e raio dos cantos.
uniform vec4 clip;
uniform float clip_radius;
// Altura da saída quando o alvo é invertido no eixo y (winit: gl_FragCoord com a
// origem embaixo); 0 quando não é (DRM: a origem já é o topo).
uniform float output_height;

#if defined(DEBUG_FLAGS)
uniform float tint;
#endif

float sd_round_box(vec2 p, vec2 b, float r) {
    vec2 q = abs(p) - b + vec2(r);
    return min(max(q.x, q.y), 0.0) + length(max(q, vec2(0.0))) - r;
}

void main() {
    vec4 color = texture2D(tex, v_coords);

#if defined(NO_ALPHA)
    color = vec4(color.rgb, 1.0) * alpha;
#else
    color = color * alpha;
#endif

    vec2 p = vec2(gl_FragCoord.x, output_height > 0.0 ? output_height - gl_FragCoord.y : gl_FragCoord.y);
    vec2 half_size = clip.zw * 0.5;
    float d = sd_round_box(p - (clip.xy + half_size), half_size, clip_radius);
    color *= 1.0 - smoothstep(-0.5, 0.5, d);

#if defined(DEBUG_FLAGS)
    if (tint == 1.0)
        color = vec4(0.0, 0.2, 0.0, 0.2) + color * 0.8;
#endif

    gl_FragColor = color;
}
"#;

/// Nomes dos uniformes do recorte, para compilar o programa.
pub fn uniform_names() -> [UniformName<'static>; 3] {
    [
        UniformName::new("clip", UniformType::_4f),
        UniformName::new("clip_radius", UniformType::_1f),
        UniformName::new("output_height", UniformType::_1f),
    ]
}

/// Orientação dos alvos de desenho: no winit a saída é invertida no eixo y
/// (`Transform::Flipped180`), no DRM não. Vale para a tela e para as texturas fora da
/// tela do vidro, que usam a mesma orientação da saída.
static FLIPPED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// O backend diz se a saída é invertida no eixo y (winit sim, DRM não).
pub fn set_flipped(flipped: bool) {
    FLIPPED.store(flipped, std::sync::atomic::Ordering::Relaxed);
}

pub fn flipped() -> bool {
    FLIPPED.load(std::sync::atomic::Ordering::Relaxed)
}

/// Transformação dos alvos de desenho (saída e texturas do vidro).
pub fn output_transform() -> smithay::utils::Transform {
    if flipped() {
        smithay::utils::Transform::Flipped180
    } else {
        smithay::utils::Transform::Normal
    }
}

/// Valores dos uniformes do recorte para uma moldura.
pub fn uniforms(frame: Rectangle<i32, Logical>, radius: f32, output_height: f32) -> Vec<Uniform<'static>> {
    // Sem inversão, a posição do pixel já tem a origem no topo (ver o shader).
    let output_height = if flipped() { output_height } else { 0.0 };
    vec![
        Uniform::new(
            "clip",
            [
                frame.loc.x as f32,
                frame.loc.y as f32,
                frame.size.w as f32,
                frame.size.h as f32,
            ],
        ),
        Uniform::new("clip_radius", radius),
        Uniform::new("output_height", output_height),
    ]
}

/// Elemento de conteúdo desenhado com o recorte arredondado.
pub struct Clipped<E> {
    inner: E,
    program: GlesTexProgram,
    uniforms: Vec<Uniform<'static>>,
}

impl<E> Clipped<E> {
    pub fn new(inner: E, program: GlesTexProgram, uniforms: Vec<Uniform<'static>>) -> Self {
        Self {
            inner,
            program,
            uniforms,
        }
    }
}

impl<E: Element> Element for Clipped<E> {
    fn id(&self) -> &Id {
        self.inner.id()
    }

    fn current_commit(&self) -> CommitCounter {
        self.inner.current_commit()
    }

    fn location(&self, scale: Scale<f64>) -> Point<i32, Physical> {
        self.inner.location(scale)
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
        self.inner.src()
    }

    fn transform(&self) -> Transform {
        self.inner.transform()
    }

    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.inner.geometry(scale)
    }

    fn damage_since(&self, scale: Scale<f64>, commit: Option<CommitCounter>) -> DamageSet<i32, Physical> {
        self.inner.damage_since(scale, commit)
    }

    // Os cantos ficam transparentes, então a região opaca do app não vale mais.
    fn opaque_regions(&self, _scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        OpaqueRegions::default()
    }

    fn alpha(&self) -> f32 {
        self.inner.alpha()
    }

    fn kind(&self) -> Kind {
        self.inner.kind()
    }
}

impl<E: RenderElement<GlesRenderer>> RenderElement<GlesRenderer> for Clipped<E> {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), GlesError> {
        frame.override_default_tex_program(self.program.clone(), self.uniforms.clone());
        let result = self.inner.draw(frame, src, dst, damage, opaque_regions);
        frame.clear_tex_program_override();
        result
    }

    // Recortado, o conteúdo não pode ir direto para a tela sem passar pelo shader.
    fn underlying_storage(&self, _renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        None
    }
}
