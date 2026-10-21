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
    /// Sombras do último quadro (posição e força de cada janela), a identidade da
    /// textura e a do elemento: muda quando as sombras ou a textura mudam.
    shadows: RefCell<Option<(Vec<(i32, i32, i32, i32, u32)>, Id, Id)>>,
    /// Último tamanho pedido e quando mudou: enquanto a saída muda de tamanho
    /// (janela do mide sendo redimensionada), a textura antiga é esticada.
    resizing: RefCell<Option<(Size<i32, Logical>, std::time::Instant)>>,
    /// Muda a cada imagem nova na tela (entra na assinatura do vidro: a dock refaz o
    /// desfoque com o papel de parede novo).
    generation: std::cell::Cell<u64>,
    /// Troca em andamento: a imagem nova sendo lida e ajustada à tela numa thread.
    /// (caminho, imagem original, imagem já ajustada à tela, tamanho da tela).
    pending: RefCell<Option<std::sync::mpsc::Receiver<(Option<PathBuf>, Option<RgbaImage>, RgbaImage, Size<i32, Logical>)>>>,
    /// Esmaecer da imagem antiga para a nova: textura antiga, tamanho e início.
    fading: RefCell<Option<(GlesTexture, Size<i32, Logical>, std::time::Instant)>>,
}

/// Duração do esmaecer ao trocar de papel de parede.
const FADE: std::time::Duration = std::time::Duration::from_millis(450);

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
            shadows: RefCell::new(None),
            resizing: RefCell::new(None),
            generation: std::cell::Cell::new(0),
            pending: RefCell::new(None),
            fading: RefCell::new(None),
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

    /// Troca a imagem se a configuração apontar para outra. A imagem nova é lida e
    /// ajustada à tela numa thread (sem travar o compositor); quando fica pronta, a
    /// antiga esmaece para a nova (`element`).
    pub fn follow(&self, path: Option<PathBuf>) {
        if *self.path.borrow() == path {
            return;
        }
        *self.path.borrow_mut() = path.clone();
        let Some(size) = self.cache.borrow().as_ref().map(|(s, _, _)| *s) else {
            // Ainda sem nada na tela: carrega direto.
            self.load_image(path);
            return;
        };
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let image = path.as_ref().and_then(|p| image::open(p).ok()).map(|i| i.to_rgba8());
            let pixels = match &image {
                Some(img) => cover(img, size.w as u32, size.h as u32),
                None => default_gradient(size.w as u32, size.h as u32),
            };
            // A original também volta (para refazer em outro tamanho de tela).
            let _ = tx.send((path, image, pixels, size));
        });
        *self.pending.borrow_mut() = Some(rx);
    }

    /// Geração da imagem na tela (muda a cada troca e a cada novo tamanho).
    pub fn generation(&self) -> u64 {
        self.generation.get()
    }

    /// O papel de parede está esmaecendo para outro (ou a imagem nova ainda está sendo
    /// preparada): quem desenha por cima refaz o vidro a cada quadro só nesse tempo.
    pub fn busy(&self) -> bool {
        self.pending.borrow().is_some() || self.fading.borrow().is_some()
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
    ) -> Vec<crate::clip::Keyed<TextureShaderElement>> {
        let size = output.size;
        if size.w <= 0 || size.h <= 0 {
            return Vec::new();
        }
        // Imagem nova pronta (thread): a atual vira a que esmaece, e a nova entra.
        let ready = self.pending.borrow().as_ref().and_then(|rx| rx.try_recv().ok());
        if let Some((path, image, prepared, prepared_for)) = ready {
            *self.pending.borrow_mut() = None;
            let source = image.clone();
            let pixels = if prepared_for == size {
                Some(prepared)
            } else {
                image.as_ref().map(|img| cover(img, size.w as u32, size.h as u32))
            };
            tracing::info!("papel de parede: {}", path.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "gradiente padrão".into()));
            let old = self.cache.borrow().as_ref().map(|(s, t, _)| (t.clone(), *s));
            *self.image.borrow_mut() = source;
            *self.cache.borrow_mut() = None;
            if let Some(px) = pixels.or_else(|| Some(default_gradient(size.w as u32, size.h as u32))) {
                if let Ok(texture) = renderer.import_memory(px.as_raw(), Fourcc::Abgr8888, (size.w, size.h).into(), false) {
                    *self.cache.borrow_mut() = Some((size, texture, Id::new()));
                }
            }
            if let Some((texture, old_size)) = old {
                *self.fading.borrow_mut() = Some((texture, old_size, std::time::Instant::now()));
            }
            self.generation.set(self.generation.get() + 1);
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
            let Ok(texture) = renderer
                .import_memory(pixels.as_raw(), Fourcc::Abgr8888, (size.w, size.h).into(), false)
                .map_err(|err| tracing::warn!("falha ao enviar o papel de parede: {err}"))
            else {
                return Vec::new();
            };
            *cache = Some((size, texture, Id::new()));
            self.generation.set(self.generation.get() + 1);
        }
        // A textura pode ser de outro tamanho (esticada durante o redimensionamento).
        let Some((tex_size, texture, id)) = cache.as_ref().map(|(s, t, i)| (*s, t.clone(), i.clone())) else {
            return Vec::new();
        };
        drop(cache);
        // Esmaecer: a nova por cima, ficando opaca; a antiga embaixo, até o fim.
        let fade = self.fading.borrow().as_ref().map(|(_, _, start)| start.elapsed().as_secs_f32() / FADE.as_secs_f32());
        let k = match fade {
            Some(t) if t < 1.0 => {
                let t = t.clamp(0.0, 1.0);
                t * t * (3.0 - 2.0 * t)
            }
            Some(_) => {
                *self.fading.borrow_mut() = None;
                1.0
            }
            None => 1.0,
        };
        // As sombras das janelas são desenhadas aqui (pelos uniformes, que o rastreador
        // de dano não vê): mudaram, o fundo é redesenhado (identidade nova).
        let signature: Vec<(i32, i32, i32, i32, u32)> = casters.iter().map(|c| (c.frame.loc.x, c.frame.loc.y, c.frame.size.w, c.frame.size.h, c.strength.to_bits())).collect();
        let id = {
            let mut last = self.shadows.borrow_mut();
            let same = last.as_ref().is_some_and(|(sig, base, _)| *sig == signature && *base == id);
            if !same {
                *last = Some((signature, id.clone(), Id::new()));
            }
            last.as_ref().map(|(_, _, element)| element.clone()).unwrap_or(id)
        };
        let mut out = vec![self.shader_element(renderer, output, texture, tex_size, id, k, casters)];
        if k < 1.0 {
            if let Some((old, old_size, _)) = self.fading.borrow().as_ref() {
                out.push(self.shader_element(renderer, output, old.clone(), *old_size, Id::new(), 1.0, casters));
            }
        }
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn shader_element(
        &self,
        renderer: &mut GlesRenderer,
        output: Rectangle<i32, Logical>,
        texture: GlesTexture,
        tex_size: Size<i32, Logical>,
        id: Id,
        alpha: f32,
        casters: &[Caster],
    ) -> crate::clip::Keyed<TextureShaderElement> {
        let size = output.size;
        let full = Rectangle::<i32, smithay::utils::Buffer>::from_size((size.w, size.h).into());
        let inner = TextureRenderElement::from_static_texture(
            id.clone(),
            renderer.context_id(),
            (output.loc.x as f64, output.loc.y as f64),
            texture,
            1,
            Transform::Normal,
            (alpha < 1.0).then_some(alpha),
            Some(Rectangle::from_size((tex_size.w as f64, tex_size.h as f64).into())),
            Some(size),
            (alpha >= 1.0).then(|| vec![full]),
            Kind::Unspecified,
        );
        let mut uniforms = vec![smithay::backend::renderer::gles::Uniform::new(
            "area_size",
            (size.w as f32, size.h as f32),
        )];
        uniforms.extend(decorations::shadow_uniforms(casters));
        crate::clip::Keyed::new(id, TextureShaderElement::new(inner, self.program.clone(), uniforms))
    }
}
