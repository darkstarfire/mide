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
//!
//! Scanout direto (A18): a janela em tela cheia no modo de jogo recebe um feedback só
//! dela (`get_surface_feedback`), com uma tranche preferida marcada como scanout
//! (`TrancheFlags::Scanout`, placa = o nó primário da tela) com os formatos e
//! modificadores que o plano principal daquela tela aceita e que o renderizador também
//! importa (para a volta à composição), seguida da tranche normal. O Mesa realoca os
//! buffers do app nesses formatos (sem compressão que a tela não lê, por exemplo), e o
//! buffer pode ir direto à placa. Fora do modo, a superfície volta ao feedback padrão.

use smithay::{
    backend::{
        allocator::{dmabuf::Dmabuf, format::FormatSet, Buffer as _, Format},
        drm::DrmNode,
        renderer::{gles::GlesRenderer, ImportDma},
    },
    delegate_dmabuf,
    reexports::wayland_protocols::wp::linux_dmabuf::zv1::server::zwp_linux_dmabuf_feedback_v1::TrancheFlags,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    wayland::dmabuf::{DmabufFeedback, DmabufFeedbackBuilder, DmabufGlobal, DmabufHandler, DmabufState, ImportNotifier, SurfaceDmabufFeedbackState},
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
    /// Feedback padrão (a placa principal e os formatos do renderizador), mandado de
    /// volta às superfícies que saem do scanout direto.
    default_feedback: Option<DmabufFeedback>,
    /// Placa principal (render node) e formatos que o renderizador importa.
    main_device: Option<libc::dev_t>,
    render_formats: Vec<Format>,
}

impl Default for Dmabufs {
    fn default() -> Self {
        Self { state: DmabufState::new(), global: None, renderer: None, capture: false, default_feedback: None, main_device: None, render_formats: Vec::new() }
    }
}

/// Uma tranche do feedback: a placa, se é de scanout e os formatos (com modificador).
#[derive(Debug, Clone, PartialEq)]
pub struct Tranche {
    pub device: libc::dev_t,
    pub scanout: bool,
    pub formats: Vec<Format>,
}

/// As tranches do feedback de scanout, na ordem em que vão ao app: primeiro a de
/// scanout (o que o plano `plane` aceita e o renderizador também importa, na ordem do
/// renderizador), depois a normal (tudo o que o renderizador importa). Sem formato em
/// comum, só a normal (não há o que preferir).
pub fn scanout_tranches(main: libc::dev_t, render: &[Format], target: libc::dev_t, plane: &FormatSet) -> Vec<Tranche> {
    let both: Vec<Format> = render.iter().copied().filter(|f| plane.contains(f)).collect();
    let mut tranches = Vec::new();
    if !both.is_empty() {
        tranches.push(Tranche { device: target, scanout: true, formats: both });
    }
    tranches.push(Tranche { device: main, scanout: false, formats: render.to_vec() });
    tranches
}

/// Monta o feedback das tranches de `scanout_tranches` (a última é a principal).
fn build_feedback(main: libc::dev_t, tranches: &[Tranche]) -> std::io::Result<DmabufFeedback> {
    let main_formats = tranches.iter().find(|t| !t.scanout).map(|t| t.formats.clone()).unwrap_or_default();
    let mut builder = DmabufFeedbackBuilder::new(main, main_formats);
    for t in tranches.iter().filter(|t| t.scanout) {
        builder = builder.add_preference_tranche(t.device, Some(TrancheFlags::Scanout), t.formats.iter().copied());
    }
    builder.build()
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

    /// Feedback de scanout para uma tela cujo plano principal aceita `plane`, com o nó
    /// primário `target` da placa. `None` sem linux-dmabuf ou sem formato em comum
    /// entre o plano e o renderizador. Devolve também quantos formatos vão na tranche
    /// de scanout (para o log).
    pub fn scanout_feedback(&self, target: libc::dev_t, plane: &FormatSet) -> Option<(DmabufFeedback, usize)> {
        let main = self.main_device?;
        self.global.as_ref()?;
        let tranches = scanout_tranches(main, &self.render_formats, target, plane);
        let count = tranches.iter().find(|t| t.scanout)?.formats.len();
        match build_feedback(main, &tranches) {
            Ok(feedback) => Some((feedback, count)),
            Err(err) => {
                tracing::warn!("linux-dmabuf: feedback de scanout: {err}");
                None
            }
        }
    }

    /// Manda à superfície (se ela pediu feedback próprio) o feedback `feedback`, ou o
    /// padrão com `None`. Só manda quando muda (o Smithay compara).
    pub fn set_surface_feedback(&self, surface: &WlSurface, feedback: Option<&DmabufFeedback>) {
        let Some(feedback) = feedback.or(self.default_feedback.as_ref()) else {
            return;
        };
        smithay::wayland::compositor::with_states(surface, |states| {
            if let Some(state) = SurfaceDmabufFeedbackState::from_states(states) {
                state.set_feedback(feedback);
            }
        });
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
    state.dmabuf.default_feedback = Some(feedback);
    state.dmabuf.main_device = Some(node.dev_id());
    state.dmabuf.render_formats = formats.clone();
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

#[cfg(test)]
mod tests {
    use super::*;
    use smithay::backend::allocator::{Fourcc, Modifier};

    fn f(code: Fourcc, modifier: u64) -> Format {
        Format { code, modifier: Modifier::from(modifier) }
    }

    #[test]
    fn tranche_de_scanout_vem_antes_e_so_com_o_que_o_plano_aceita() {
        // Renderizador: XRGB linear, XRGB com compressão (CCS) e ARGB com tiling Y.
        let render = [f(Fourcc::Xrgb8888, 0), f(Fourcc::Xrgb8888, 0x0100000000000006), f(Fourcc::Argb8888, 0x0100000000000002)];
        // Plano principal: XRGB linear e ARGB com tiling Y (sem compressão), e um que o
        // renderizador não importa.
        let plane: FormatSet = [f(Fourcc::Xrgb8888, 0), f(Fourcc::Argb8888, 0x0100000000000002), f(Fourcc::Nv12, 0)].into_iter().collect();
        let tranches = scanout_tranches(10, &render, 20, &plane);
        assert_eq!(tranches.len(), 2);
        assert!(tranches[0].scanout);
        assert_eq!(tranches[0].device, 20);
        assert_eq!(tranches[0].formats, vec![f(Fourcc::Xrgb8888, 0), f(Fourcc::Argb8888, 0x0100000000000002)]);
        assert!(!tranches[1].scanout);
        assert_eq!(tranches[1].device, 10);
        assert_eq!(tranches[1].formats, render.to_vec());
        // E o feedback monta (tabela de formatos em memória selada).
        assert!(build_feedback(10, &tranches).is_ok());
    }

    #[test]
    fn sem_formato_em_comum_fica_so_a_tranche_normal() {
        let render = [f(Fourcc::Xrgb8888, 0x0100000000000006)];
        let plane: FormatSet = [f(Fourcc::Xrgb8888, 0)].into_iter().collect();
        let tranches = scanout_tranches(10, &render, 20, &plane);
        assert_eq!(tranches.len(), 1);
        assert!(!tranches[0].scanout);
    }
}
