//! Papel de parede do desktop, com a sombra combinada das janelas por cima.
//!
//! A imagem (ou o gradiente padrão) é ajustada uma vez ao tamanho da saída,
//! cobrindo a tela inteira e cortando as sobras, e vira uma textura. Um shader de
//! textura desenha essa textura e escurece onde há sombra de janela. Assim o
//! fundo continua sendo um único elemento opaco, atrás de tudo.

use std::{cell::RefCell, path::PathBuf};

use image::RgbaImage;
use mide::backdrop::{cover, default_gradient};
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            element::{texture::TextureRenderElement, Id, Kind},
            gles::{element::TextureShaderElement, GlesError, GlesRenderer, GlesTexProgram, GlesTexture},
            ImportMem, Renderer,
        },
    },
    utils::{Logical, Rectangle, Size, Transform},
};

use crate::decorations::{self, Caster};

const WALLPAPER_HEAD: &str = r#"#version 100

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
uniform vec2 area_size;

#if defined(DEBUG_FLAGS)
uniform float tint;
#endif
"#;

const WALLPAPER_MAIN: &str = r#"
void main() {
    vec4 color = texture2D(tex, v_coords);
    float shade = combined_shadow(v_coords * area_size);
    gl_FragColor = vec4(color.rgb * (1.0 - shade), 1.0) * alpha;
}
"#;

/// Papel de parede e a textura pronta para o tamanho atual da saída.
pub struct Wallpaper {
    program: GlesTexProgram,
    /// Imagem do usuário já decodificada; `None` usa o gradiente padrão.
    image: RefCell<Option<RgbaImage>>,
    /// Caminho da imagem atual (para trocar quando a configuração mudar).
    path: RefCell<Option<PathBuf>>,
    cache: RefCell<Option<(Size<i32, Logical>, GlesTexture, Id)>>,
    /// Último tamanho pedido e quando mudou: enquanto a saída muda de tamanho
    /// (janela do mide sendo redimensionada), a textura antiga é esticada.
    resizing: RefCell<Option<(Size<i32, Logical>, std::time::Instant)>>,
}

impl Wallpaper {
    /// Compila o shader e carrega a imagem. Se a imagem não abrir, usa o gradiente.
    pub fn new(renderer: &mut GlesRenderer, path: Option<PathBuf>) -> Result<Self, GlesError> {
        let mut names = vec![smithay::backend::renderer::gles::UniformName::new(
            "area_size",
            smithay::backend::renderer::gles::UniformType::_2f,
        )];
        names.extend(decorations::shadow_uniform_names());
        let program = renderer.compile_custom_texture_shader(
            format!("{WALLPAPER_HEAD}{}{WALLPAPER_MAIN}", decorations::SHADOW_GLSL),
            &names,
        )?;
        let wallpaper = Self {
            program,
            image: RefCell::new(None),
            path: RefCell::new(None),
            cache: RefCell::new(None),
            resizing: RefCell::new(None),
        };
        wallpaper.load_image(path);
        Ok(wallpaper)
    }

    fn load_image(&self, path: Option<PathBuf>) {
        let image = path.as_ref().and_then(|path| match image::open(path) {
            Ok(img) => {
                tracing::info!("papel de parede: {}", path.display());
                Some(img.to_rgba8())
            }
            Err(err) => {
                tracing::warn!("papel de parede {} não abriu ({err}); usando o padrão", path.display());
                None
            }
        });
        if image.is_none() {
            tracing::info!("papel de parede: gradiente padrão");
        }
        *self.image.borrow_mut() = image;
        *self.path.borrow_mut() = path;
        *self.cache.borrow_mut() = None;
    }

    /// Troca a imagem se a configuração apontar para outra.
    pub fn follow(&self, path: Option<PathBuf>) {
        if *self.path.borrow() != path {
            self.load_image(path);
        }
    }

    /// Textura atual do papel de parede e o tamanho dela (para o desfoque do launcher).
    pub fn texture(&self) -> Option<(GlesTexture, Size<i32, Logical>)> {
        self.cache.borrow().as_ref().map(|(size, tex, _)| (tex.clone(), *size))
    }

    /// Elemento do fundo cobrindo `output`, com as sombras de `casters`.
    pub fn element(
        &self,
        renderer: &mut GlesRenderer,
        output: Rectangle<i32, Logical>,
        casters: &[Caster],
    ) -> Option<TextureShaderElement> {
        let size = output.size;
        if size.w <= 0 || size.h <= 0 {
            return None;
        }
        let mut cache = self.cache.borrow_mut();
        // Tamanho mudando: só refaz a textura quando parar de mudar por 150 ms.
        let settled = {
            let mut resizing = self.resizing.borrow_mut();
            let now = std::time::Instant::now();
            match *resizing {
                Some((last, since)) if last == size => now.duration_since(since) >= std::time::Duration::from_millis(150),
                _ => {
                    *resizing = Some((size, now));
                    cache.is_none()
                }
            }
        };
        if cache.as_ref().map(|(s, _, _)| *s) != Some(size) && settled {
            let pixels = match self.image.borrow().as_ref() {
                Some(img) => cover(img, size.w as u32, size.h as u32),
                None => default_gradient(size.w as u32, size.h as u32),
            };
            let texture = renderer
                .import_memory(pixels.as_raw(), Fourcc::Abgr8888, (size.w, size.h).into(), false)
                .map_err(|err| tracing::warn!("falha ao enviar o papel de parede: {err}"))
                .ok()?;
            *cache = Some((size, texture, Id::new()));
        }
        // A textura pode ser de outro tamanho (esticada durante o redimensionamento).
        let (tex_size, texture, id) = cache.as_ref()?;
        let tex_size = *tex_size;
        let full = Rectangle::<i32, smithay::utils::Buffer>::from_size((size.w, size.h).into());
        let inner = TextureRenderElement::from_static_texture(
            id.clone(),
            renderer.context_id(),
            (output.loc.x as f64, output.loc.y as f64),
            texture.clone(),
            1,
            Transform::Normal,
            None,
            Some(Rectangle::from_size((tex_size.w as f64, tex_size.h as f64).into())),
            Some(size),
            Some(vec![full]),
            Kind::Unspecified,
        );
        let mut uniforms = vec![smithay::backend::renderer::gles::Uniform::new(
            "area_size",
            (size.w as f32, size.h as f32),
        )];
        uniforms.extend(decorations::shadow_uniforms(casters));
        Some(TextureShaderElement::new(inner, self.program.clone(), uniforms))
    }
}
