//! Efeito lâmpada (genie) ao minimizar e restaurar janelas, como no macOS.
//!
//! A janela inteira é renderizada numa textura fora da tela e desenhada com um
//! shader em duas fases: primeiro a base estica até o ícone na dock e forma o funil
//! (cada linha estreita conforme se aproxima da dock); depois o topo desce e a janela
//! escorre pelo funil para dentro do ícone.

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            element::{
                surface::WaylandSurfaceRenderElement, texture::TextureRenderElement,
                AsRenderElements, Element, Id, Kind, RenderElement,
            },
            gles::{
                element::TextureShaderElement, GlesError, GlesRenderer, GlesTexProgram, Uniform,
                UniformName, UniformType,
            },
            Bind, Color32F, Frame, Offscreen, Renderer,
        },
    },
    desktop::Window,
    utils::{Logical, Point, Rectangle, Scale, Transform},
};

const GENIE_SRC: &str = r#"#version 100

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

// Tamanho da área desenhada e, dentro dela, a janela e o alvo (x, y, w, h).
uniform vec2 area_size;
uniform vec4 win;
uniform vec4 target;
// 0 = janela normal, 1 = recolhida no alvo.
uniform float progress;
// 1 quando a textura está de cabeça para baixo (origem do GL embaixo).
uniform float flip_y;
// Tamanho da textura (a janela inteira) e raio dos cantos, para arredondar no funil.
uniform vec2 tex_size;
uniform float corner;

float sd_round_box(vec2 p, vec2 b, float r) {
    vec2 q = abs(p) - b + vec2(r);
    return min(max(q.x, q.y), 0.0) + length(max(q, vec2(0.0))) - r;
}

#if defined(DEBUG_FLAGS)
uniform float tint;
#endif

void main() {
    vec2 p = v_coords * area_size;
    float t = clamp(progress, 0.0, 1.0);

    float win_top = win.y;
    float win_bot = win.y + win.w;
    float tg_top = target.y;
    float tg_bot = target.y + target.w;

    // Fase 1: a base estica até a dock e o funil se forma.
    float a = smoothstep(0.0, 1.0, clamp(t / 0.45, 0.0, 1.0));
    // Fase 2: o topo desce e a janela escorre pelo funil para dentro do ícone.
    float b = smoothstep(0.0, 1.0, clamp((t - 0.35) / 0.65, 0.0, 1.0));

    float y_bot = mix(win_bot, tg_bot, a);
    float y_top = mix(win_top, tg_top, b);
    float v = (p.y - y_top) / max(y_bot - y_top, 0.001);

    // Largura de cada linha pela altura absoluta: largura da janela no topo dela,
    // largura do ícone no nível da dock, com curva suave entre as duas.
    float span = max(tg_bot - win_top, 1.0);
    float k = clamp((p.y - win_top) / span, 0.0, 1.0);
    float squeeze = smoothstep(0.0, 1.0, k) * a;
    // No fim, tudo converge para a largura do ícone.
    squeeze = mix(squeeze, 1.0, b * b);
    float left = mix(win.x, target.x, squeeze);
    float right = mix(win.x + win.z, target.x + target.z, squeeze);
    float u = (p.x - left) / max(right - left, 0.001);

    if (u < 0.0 || u > 1.0 || v < 0.0 || v > 1.0) {
        gl_FragColor = vec4(0.0);
        return;
    }
    vec2 uv = vec2(u, mix(v, 1.0 - v, flip_y));
    vec4 color = texture2D(tex, uv);
    // Cantos arredondados da janela, no espaço da própria textura.
    vec2 tp = vec2(u, v) * tex_size;
    float dc = sd_round_box(tp - tex_size * 0.5, tex_size * 0.5, corner);
    color *= 1.0 - smoothstep(-0.5, 0.5, dc);

#if defined(NO_ALPHA)
    color = vec4(color.rgb, 1.0) * alpha;
#else
    color = color * alpha;
#endif

#if defined(DEBUG_FLAGS)
    if (tint == 1.0)
        color = vec4(0.0, 0.2, 0.0, 0.2) + color * 0.8;
#endif

    gl_FragColor = color;
}
"#;

/// Programa de shader do efeito lâmpada.
pub struct Genie {
    program: GlesTexProgram,
    /// Fotos das janelas com fechamento pedido (número da janela, textura e tamanho),
    /// usadas na transição de fechar.
    pub snapshots: std::cell::RefCell<std::collections::HashMap<u64, (smithay::backend::renderer::gles::GlesTexture, smithay::utils::Size<i32, Logical>)>>,
    /// Quando foi tirada a última foto de cada janela (para fechar com transição).
    pub photo_taken: std::cell::RefCell<std::collections::HashMap<u64, std::time::Instant>>,
    /// Desde quando cada janela tem o vidro dela (depois da lâmpada ou de outra
    /// animação): o vidro entra esmaecendo, sem aparecer de uma vez.
    pub glass_since: std::cell::RefCell<std::collections::HashMap<u64, std::time::Instant>>,
}

/// Duração da entrada do vidro de uma janela.
pub const GLASS_FADE: std::time::Duration = mide::motion::GLASS_FADE;

impl Genie {
    pub fn compile(renderer: &mut GlesRenderer) -> Result<Self, GlesError> {
        let program = renderer.compile_custom_texture_shader(
            GENIE_SRC,
            &[
                UniformName::new("area_size", UniformType::_2f),
                UniformName::new("win", UniformType::_4f),
                UniformName::new("target", UniformType::_4f),
                UniformName::new("progress", UniformType::_1f),
                UniformName::new("flip_y", UniformType::_1f),
                UniformName::new("tex_size", UniformType::_2f),
                UniformName::new("corner", UniformType::_1f),
            ],
        )?;
        Ok(Self { program, snapshots: Default::default(), photo_taken: Default::default(), glass_since: Default::default() })
    }

    /// Elemento da janela deformada. `window_rect` é a área visível da janela e
    /// `target` o retângulo pequeno de destino, ambos em coordenadas da saída.
    /// `progress` vai de 0 (normal) a 1 (recolhida).
    #[allow(clippy::too_many_arguments)]
    pub fn element(
        &self,
        renderer: &mut GlesRenderer,
        window: &Window,
        window_rect: Rectangle<i32, Logical>,
        bar_height: i32,
        overlays: &[&dyn RenderElement<GlesRenderer>],
        corner: f32,
        target: Rectangle<i32, Logical>,
        progress: f32,
        alpha: f32,
    ) -> Option<TextureShaderElement> {
        let size = window_rect.size;
        if size.w <= 0 || size.h <= 0 {
            return None;
        }

        let texture = snapshot(renderer, window, size, bar_height, overlays)?;

        // Área que contém a janela e o alvo: é onde o funil é desenhado.
        let area = window_rect.merge(target);
        let local = |r: Rectangle<i32, Logical>| -> [f32; 4] {
            [
                (r.loc.x - area.loc.x) as f32,
                (r.loc.y - area.loc.y) as f32,
                r.size.w as f32,
                r.size.h as f32,
            ]
        };
        let inner = TextureRenderElement::from_static_texture(
            Id::new(),
            renderer.context_id(),
            crate::clip::phys(area.loc).to_f64(),
            texture,
            1,
            Transform::Normal,
            Some(alpha),
            // A textura inteira é a origem; o destino é a área do funil. Sem `src`
            // explícito, o Smithay usaria o tamanho do destino como origem e distorceria.
            Some(Rectangle::from_size((size.w as f64, size.h as f64).into())),
            Some(area.size),
            None,
            Kind::Unspecified,
        );
        Some(TextureShaderElement::new(
            inner,
            self.program.clone(),
            vec![
                Uniform::new("area_size", (area.size.w as f32, area.size.h as f32)),
                Uniform::new("win", local(window_rect)),
                Uniform::new("target", local(target)),
                Uniform::new("progress", progress),
                Uniform::new("flip_y", genie_flip()),
                Uniform::new("tex_size", (size.w as f32, size.h as f32)),
                Uniform::new("corner", corner * crate::clip::scale() as f32),
            ],
        ))
    }
}

/// Orientação da textura fora da tela. Pode ser ajustada por `MIDE_GENIE_FLIP` (0 ou 1)
/// para depuração; o padrão foi conferido no quadro salvo.
fn genie_flip() -> f32 {
    std::env::var("MIDE_GENIE_FLIP")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.0)
}

/// A janela inteira numa textura do tamanho `size` (moldura): barra e título por cima
/// (`overlays`, em coordenadas da textura) e o conteúdo logo abaixo da barra. Usada
/// pelo efeito lâmpada e pela foto que esmaece ao fechar a janela.
pub fn snapshot(
    renderer: &mut GlesRenderer,
    window: &Window,
    size: smithay::utils::Size<i32, Logical>,
    bar_height: i32,
    overlays: &[&dyn RenderElement<GlesRenderer>],
) -> Option<smithay::backend::renderer::gles::GlesTexture> {
        // A janela inteira numa textura: barra em cima (se houver) e o conteúdo logo
        // abaixo. A superfície começa antes da área visível pelo deslocamento da geometria.
        let origin: Point<i32, Logical> = (
            -window.geometry().loc.x,
            bar_height - window.geometry().loc.y,
        )
            .into();
        let content: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
            window.render_elements(renderer, origin.to_physical(1), Scale::from(1.0), 1.0);
        let mut texture: smithay::backend::renderer::gles::GlesTexture = renderer
            .create_buffer(Fourcc::Abgr8888, (size.w, size.h).into())
            .ok()?;
        {
            let mut target_fb = renderer.bind(&mut texture).ok()?;
            let mut frame = renderer
                .render(&mut target_fb, (size.w, size.h).into(), Transform::Normal)
                .ok()?;
            let full = Rectangle::from_size((size.w, size.h).into());
            frame.clear(Color32F::new(0.0, 0.0, 0.0, 0.0), &[full]).ok()?;
            // A lista vai da frente para trás; o desenho vai de trás para a frente.
            for element in content.iter().rev() {
                let src = element.src();
                let dst = element.geometry(Scale::from(1.0));
                let damage = [Rectangle::from_size(dst.size)];
                let _ = element.draw(&mut frame, src, dst, &damage, &[]);
            }
            // Barra e título por cima, também de trás para a frente.
            for element in overlays.iter().rev() {
                let src = element.src();
                let dst = element.geometry(Scale::from(1.0));
                let damage = [Rectangle::from_size(dst.size)];
                let _ = element.draw(&mut frame, src, dst, &damage, &[]);
            }
            let _ = frame.finish();
        }
    Some(texture)
}
