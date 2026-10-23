//! Buffers na GPU dos apps (A25): o protocolo `zwp_linux_dmabuf_v1`.
//!
//! Sem ele, todo app Wayland com OpenGL/Vulkan (OBS, Chrome, Electron, jogos, o
//! XWayland pelo satellite) cai para desenho no processador (llvmpipe): o Mesa só usa a
//! placa de vídeo quando o compositor aceita buffers dela (dmabuf). Com ele, o app
//! desenha na GPU e entrega o buffer pronto; o mide importa e desenha como textura,
//! sem cópia.
//!
//! O global sai na versão 4/5, com o "feedback" (`main_device`: o render node da placa
//! e os formatos/modificadores que o renderizador sabe importar). O Mesa e o
//! `xdg-desktop-portal-wlr` usam o `main_device` para achar a placa; sem ele caem no
//! llvmpipe mesmo com o global.
//!
//! O renderizador mora no backend (DRM, sem tela, winit), não no `State`. Cada backend
//! entrega um acesso a ele (um fecho com um `Rc` do renderizador, como uma função
//! anônima de Go que guarda um ponteiro), usado fora do desenho: quando um app cria um
//! buffer (se a importação falha, o app recebe `failed` na hora e pode tentar outro
//! formato) e quando um buffer vira destino de uma captura (`prepare_target`).
//!
//! `MIDE_NO_DMABUF=1` (só para diagnóstico) não anuncia o protocolo.

use smithay::{
    backend::{
        allocator::{dmabuf::Dmabuf, Buffer as _, Format},
        drm::DrmNode,
        renderer::{gles::GlesRenderer, ImportDma},
    },
    delegate_dmabuf,
    wayland::dmabuf::{DmabufFeedbackBuilder, DmabufGlobal, DmabufHandler, DmabufState, ImportNotifier},
};

use crate::state::State;

/// Acesso ao renderizador do backend, fora do desenho: roda a função com ele e diz se
/// conseguiu (o renderizador pode estar ocupado, o que não deve acontecer no despacho
/// dos clientes).
pub type RendererAccess = Box<dyn FnMut(&mut dyn FnMut(&mut GlesRenderer)) -> bool>;

/// Estado do `linux-dmabuf`: o global (se a placa permite) e o acesso ao renderizador.
pub struct Dmabufs {
    state: DmabufState,
    global: Option<DmabufGlobal>,
    renderer: Option<RendererAccess>,
    /// A captura de tela pode copiar para buffers dmabuf: o renderizador faz blit
    /// (GLES 3) e desenha em XRGB8888. Sem isso a captura oferece só memória
    /// compartilhada (antes do A25 era assim, e funcionava nessas placas).
    pub capture: bool,
}

impl Default for Dmabufs {
    fn default() -> Self {
        Self { state: DmabufState::new(), global: None, renderer: None, capture: false }
    }
}

impl Dmabufs {
    /// Prepara um buffer dmabuf como destino de desenho (a captura de tela copia nele),
    /// fora do quadro. O Smithay guarda o destino de cada buffer (a imagem do EGL e o
    /// framebuffer): no quadro, a cópia acha o destino pronto, sem esse trabalho nem a
    /// troca de contexto que ele faz no meio do desenho.
    /// `false` quando o renderizador recusa o buffer como destino (a captura falha logo).
    pub fn prepare_target(&mut self, dmabuf: &Dmabuf) -> bool {
        let mut ok = true;
        if let Some(access) = self.renderer.as_mut() {
            let mut target = dmabuf.clone();
            access(&mut |renderer| {
                use smithay::backend::renderer::Bind;
                if let Err(err) = renderer.bind(&mut target) {
                    tracing::debug!("captura por dmabuf: destino recusado: {err}");
                    ok = false;
                }
            });
        }
        ok
    }
}

/// Monta o acesso ao renderizador guardado em `shared` (o `Rc` do backend) por `get`.
pub fn access<T: 'static>(shared: std::rc::Rc<std::cell::RefCell<T>>, get: fn(&mut T) -> &mut GlesRenderer) -> RendererAccess {
    Box::new(move |f| match shared.try_borrow_mut() {
        Ok(mut inner) => {
            f(get(&mut inner));
            true
        }
        Err(_) => false,
    })
}

/// Anuncia o `linux-dmabuf` com os formatos que `renderer` importa e o render node
/// `node` como placa principal. Sem render node ou sem formatos (renderizador sem
/// `EGL_EXT_image_dma_buf_import`, como o llvmpipe), não anuncia: os apps seguem por
/// memória compartilhada.
pub fn setup(state: &mut State, renderer: &GlesRenderer, node: Option<DrmNode>, access: RendererAccess) {
    if std::env::var_os("MIDE_NO_DMABUF").is_some_and(|v| v == "1") {
        tracing::info!("linux-dmabuf desligado (MIDE_NO_DMABUF=1): os apps desenham sem a GPU");
        return;
    }
    if state.dmabuf.global.is_some() {
        return;
    }
    let formats: Vec<Format> = renderer.dmabuf_formats().iter().copied().collect();
    let Some(node) = node else {
        tracing::warn!("linux-dmabuf não anunciado: sem render node da placa de vídeo (apps sem GPU)");
        return;
    };
    if formats.is_empty() {
        tracing::warn!("linux-dmabuf não anunciado: o renderizador não importa dmabuf (apps sem GPU)");
        return;
    }
    let feedback = match DmabufFeedbackBuilder::new(node.dev_id(), formats.iter().copied()).build() {
        Ok(feedback) => feedback,
        Err(err) => {
            tracing::warn!("linux-dmabuf não anunciado: feedback: {err}");
            return;
        }
    };
    let global = state.dmabuf.state.create_global_with_default_feedback::<State>(&state.display_handle, &feedback);
    state.dmabuf.global = Some(global);
    state.dmabuf.renderer = Some(access);
    use smithay::backend::renderer::gles::Capability;
    let xrgb = renderer.egl_context().dmabuf_render_formats().iter().any(|f| f.code == smithay::backend::allocator::Fourcc::Xrgb8888);
    state.dmabuf.capture = renderer.capabilities().contains(&Capability::Blit) && xrgb;
    if !state.dmabuf.capture {
        tracing::warn!("captura de tela só por memória compartilhada: o renderizador não copia para dmabuf (sem GLES 3 ou sem XRGB8888)");
    }
    let codes: std::collections::BTreeSet<String> = formats.iter().map(|f| format!("{:?}", f.code)).collect();
    tracing::info!(
        "linux-dmabuf anunciado: placa {node} ({} formato(s) com modificadores: {})",
        formats.len(),
        codes.into_iter().collect::<Vec<_>>().join(", ")
    );
}

/// Render node de um dispositivo DRM aberto (o próprio, se já é um render node).
pub fn render_node(fd: &impl std::os::fd::AsFd) -> Option<DrmNode> {
    use smithay::backend::drm::NodeType;
    let node = DrmNode::from_file(fd).ok()?;
    if node.ty() == NodeType::Render {
        return Some(node);
    }
    node.node_with_type(NodeType::Render).and_then(Result::ok)
}

/// Render node da placa por trás de um display EGL (winit, onde não há fd da placa).
pub fn render_node_of_egl(display: &smithay::backend::egl::EGLDisplay) -> Option<DrmNode> {
    let device = smithay::backend::egl::EGLDevice::device_for_display(display).ok()?;
    device.try_get_render_node().ok().flatten()
}

impl DmabufHandler for State {
    fn dmabuf_state(&mut self) -> &mut DmabufState {
        &mut self.dmabuf.state
    }

    fn dmabuf_imported(&mut self, _global: &DmabufGlobal, dmabuf: Dmabuf, notifier: ImportNotifier) {
        // Renderizador ocupado (não deveria acontecer): aceita, e o desenho tenta importar.
        let mut ok = true;
        if let Some(access) = self.dmabuf.renderer.as_mut() {
            access(&mut |renderer| ok = renderer.import_dmabuf(&dmabuf, None).is_ok());
        }
        if ok {
            if notifier.successful::<State>().is_err() {
                tracing::debug!("linux-dmabuf: o app sumiu antes do buffer ficar pronto");
            }
        } else {
            tracing::debug!("linux-dmabuf: buffer recusado ({:?} {:?})", dmabuf.format(), dmabuf.size());
            notifier.failed();
        }
    }
}

delegate_dmabuf!(State);
