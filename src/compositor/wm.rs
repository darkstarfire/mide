//! Política de janelas do mide: onde janelas novas aparecem, maximizar e restaurar,
//! minimizar, fechar, arrastar e ajustar à área útil.
//!
//! Os handlers dos protocolos (em `state.rs`) só traduzem o pedido do cliente e
//! chamam estas funções. Assim as regras do mide ficam num lugar só, separadas do
//! Smithay, e valem igual para qualquer backend.

use std::time::{Duration, Instant};

use smithay::{
    desktop::{layer_map_for_output, Window},
    output::Output,
    reexports::{
        wayland_protocols::xdg::shell::server::xdg_toplevel::State as XdgState,
        wayland_server::protocol::wl_surface::WlSurface,
    },
    utils::{Logical, Point, Size},
    wayland::{compositor::with_states, shell::xdg::ToplevelSurface},
};

use crate::state::{
    bar_height_for, window_id, MinimizedWindow, MoveGrab, State, WindowAnimation, WindowId, ANIMATION_DURATION,
    GENIE_DURATION,
};

/// Posição inicial da n-ésima janela em cascata: 48 px de margem e 32 px de passo.
pub fn cascade_position(index: usize) -> Point<i32, Logical> {
    let offset = (index as i32) * 32;
    (48 + offset, 48 + offset).into()
}

/// Marca de janela já conferida para o efeito lâmpada de abrir.
struct OpenChecked;

/// Marca do seletor de arquivos já posto no meio da tela.
struct CenterChecked;

impl State {
    /// Janela nova (pedido `xdg_toplevel`): configure inicial, número fixo, posição em
    /// cascata e foco.
    pub fn map_new_window(&mut self, surface: ToplevelSurface) {
        tracing::info!("nova janela criada");
        // O configure inicial sai no primeiro commit do app (`commit` em `state.rs`), como
        // pede o protocolo, já com o foco e o resto decididos aqui. Mandado antes, o app
        // podia perder o configure seguinte (ficava sem saber que é a janela ativa).

        // Janelas novas entram em cascata, para não nascerem uma sobre a outra, a partir
        // do canto da área útil (abaixo da barra superior).
        // Número fixo da janela, para a dock e a barra se referirem a ela.
        static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let id = NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        with_states(surface.wl_surface(), |states| {
            states.data_map.insert_if_missing_threadsafe(|| WindowId(id));
        });
        let window = Window::new_wayland_window(surface);
        let origin = self.usable_area().map(|a| a.loc).unwrap_or_default();
        let at = origin + cascade_position(self.space.elements().count());
        self.space.map_element(window.clone(), at, true);

        // Como no macOS, a janela recém-aberta recebe o foco.
        // O foco vem no primeiro commit, logo depois do configure inicial (`commit`):
        // teclado antes do configure derruba alguns apps (o foot cai). O estado "ativa"
        // já vai no configure inicial.
        if let Some(toplevel) = window.toplevel() {
            toplevel.with_pending_state(|s| s.states.set(XdgState::Activated));
            self.focus_on_map = Some(toplevel.wl_surface().clone());
        }
    }

    /// A janela do app foi destruída: sai do espaço e deixa a foto dela esmaecendo
    /// (a tirada no fechamento pedido pelo mide, ou a última foto, se o app fechou
    /// sozinho).
    pub fn window_closed(&mut self, surface: &ToplevelSurface) {
        if let Some(window) = self.window_for(surface.wl_surface()) {
            // Fechamento pedido pelo mide: a foto da janela esmaece (transição de fechar).
            // Guarda só a posição: o tamanho vem da foto (a geometria aqui já zerou).
            let id = window_id(&window);
            if self.config.effects.animations {
                if let Some(loc) = self.space.element_location(&window) {
                    let bar_h = bar_height_for(&window);
                    let size = window.geometry().size;
                    let frame = smithay::utils::Rectangle::new((loc.x, loc.y - bar_h).into(), (size.w, size.h + bar_h).into());
                    // O raio some com o app_id quando a janela é destruída: guarda agora.
                    let radius = crate::render::corner_radius(self, &window);
                    self.ghosts.push((id, frame, Instant::now(), radius));
                    tracing::info!("janela fechando com transição (raio {radius})");
                }
            }
            self.close_requests.retain(|(w, _)| *w != id);
            self.forget_fullscreen(&window);
            self.commits.remove(&id);
            self.shown.remove(&id);
            self.space.unmap_elem(&window);
            tracing::info!("janela fechada");
        } else if let Some(i) = self.hidden.iter().position(|(w, _)| w.toplevel().is_some_and(|t| t == surface)) {
            // Escondida pelo app e agora fechada de vez.
            let (window, _) = self.hidden.remove(i);
            self.shown.remove(&window_id(&window));
            tracing::info!("janela fechada");
        }
    }

    /// O app pediu para ser arrastado (barra desenhada por ele: apps Wayland com
    /// decoração própria e apps X11 pelo `xwayland-satellite`, que repassa o
    /// `_NET_WM_MOVERESIZE` como `move`). Daqui em diante o arrasto é igual ao da barra
    /// do mide: o mesmo `MoveGrab`, o mesmo desencaixe e o mesmo encaixe pelo topo.
    pub fn begin_move(&mut self, surface: &ToplevelSurface) {
        let Some(window) = self.window_for(surface.wl_surface()) else {
            return;
        };
        let Some(loc) = self.space.element_location(&window) else {
            return;
        };
        // Pedido atrasado (o botão já foi solto): sem arrasto, senão a janela seguiria
        // o ponteiro sem botão nenhum apertado. Em tela cheia a janela não sai do lugar.
        if !self.left_button_held() || self.fullscreen_entry(&window).is_some() {
            return;
        }
        // O app só pede para mover depois de o ponteiro andar alguns pixels. O arrasto
        // é medido a partir do ponto onde o botão foi pressionado, para não perder esse trecho.
        let anchor = self.press_location.unwrap_or(self.pointer_location);
        let offset = anchor - loc.to_f64();
        self.unsnap_fix = None;
        self.move_grab = Some(MoveGrab { window, offset, fraction: None });
    }

    /// O app pediu para ser redimensionado por uma borda (apps com borda própria, e
    /// apps X11 pelo satellite): o mesmo redimensionamento das bordas do mide.
    pub fn begin_resize(&mut self, surface: &ToplevelSurface, edges: smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::ResizeEdge) {
        use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::ResizeEdge;
        let Some(window) = self.window_for(surface.wl_surface()) else {
            return;
        };
        let Some(loc) = self.space.element_location(&window) else {
            return;
        };
        if !self.left_button_held() || self.is_snapped(surface) || crate::state::fixed_size(&window) || self.fullscreen_entry(&window).is_some() {
            return;
        }
        let edges = crate::decorations::Edges {
            left: matches!(edges, ResizeEdge::Left | ResizeEdge::TopLeft | ResizeEdge::BottomLeft),
            right: matches!(edges, ResizeEdge::Right | ResizeEdge::TopRight | ResizeEdge::BottomRight),
            top: matches!(edges, ResizeEdge::Top | ResizeEdge::TopLeft | ResizeEdge::TopRight),
            bottom: matches!(edges, ResizeEdge::Bottom | ResizeEdge::BottomLeft | ResizeEdge::BottomRight),
        };
        if !(edges.left || edges.right || edges.top || edges.bottom) {
            return;
        }
        tracing::info!("redimensionar pedido pelo app");
        let start_size = window.geometry().size;
        self.resize_grab = Some(crate::state::ResizeGrab {
            window,
            edges,
            start_pointer: self.press_location.unwrap_or(self.pointer_location),
            start_loc: loc,
            start_size,
        });
    }

    /// O botão esquerdo está apertado (a pega implícita do Smithay está ativa).
    fn left_button_held(&self) -> bool {
        self.seat.get_pointer().is_some_and(|p| p.is_grabbed())
    }

    /// Área útil da tela onde a janela está (sem a barra e a dock, se estiverem nela).
    fn area_for(&self, window: &Window) -> Option<smithay::utils::Rectangle<i32, Logical>> {
        let output = self
            .space
            .element_location(window)
            .and_then(|loc| {
                let center = loc + Point::from((window.geometry().size.w / 2, window.geometry().size.h / 2));
                self.space.output_under(center.to_f64()).next().cloned()
            })
            .or_else(|| self.primary_output())?;
        self.usable_area_of(&output)
    }

    /// Maximiza a janela na área útil da tela dela, com animação.
    pub fn maximize(&mut self, surface: &ToplevelSurface) {
        let Some(window) = self.window_for(surface.wl_surface()) else {
            surface.send_configure();
            return;
        };
        // Janela de tamanho fixo (Sobre este computador): não maximiza. Em tela cheia,
        // o pedido fica para depois (o app sai da tela cheia antes).
        if crate::state::fixed_size(&window) || self.fullscreen_entry(&window).is_some() {
            surface.send_configure();
            return;
        }
        let Some(area) = self.area_for(&window) else {
            surface.send_configure();
            return;
        };
        self.fill_area(surface, &window, area, None);
    }

    /// Maximiza na área útil dada (encaixe pelo topo: a tela sob o ponteiro).
    pub fn maximize_in(&mut self, surface: &ToplevelSurface, area: smithay::utils::Rectangle<i32, Logical>) {
        let Some(window) = self.window_for(surface.wl_surface()).filter(|w| !crate::state::fixed_size(w) && self.fullscreen_entry(w).is_none()) else {
            return;
        };
        self.fill_area(surface, &window, area, None);
    }

    /// Põe a janela em `rect` (metade, terço ou quarto escolhido no encaixe pelo topo);
    /// `left` diz se ela encosta na borda esquerda.
    pub fn snap_to(&mut self, surface: &ToplevelSurface, rect: smithay::utils::Rectangle<i32, Logical>, left: bool) {
        let Some(window) = self.window_for(surface.wl_surface()).filter(|w| !crate::state::fixed_size(w) && self.fullscreen_entry(w).is_none()) else {
            return;
        };
        self.fill_area(surface, &window, rect, Some(left));
    }

    /// Metade esquerda (`left`) ou direita da área útil da tela da janela (Super+← e
    /// Super+→, como no GNOME). Super+↓ volta ao tamanho de antes.
    pub fn tile(&mut self, surface: &ToplevelSurface, left: bool) {
        let Some(window) = self.window_for(surface.wl_surface()).filter(|w| self.fullscreen_entry(w).is_none()) else {
            return;
        };
        let Some(area) = self.area_for(&window) else {
            return;
        };
        let half = area.size.w / 2;
        let rect = if left {
            smithay::utils::Rectangle::new(area.loc, (half, area.size.h).into())
        } else {
            smithay::utils::Rectangle::new((area.loc.x + half, area.loc.y).into(), (area.size.w - half, area.size.h).into())
        };
        tracing::info!("janela na metade {} da tela", if left { "esquerda" } else { "direita" });
        self.fill_area(surface, &window, rect, Some(left));
    }

    /// Põe a janela numa parte da área útil da tela dela: `part` em frações (x, y,
    /// largura, altura), como as metades e os quartos do menu Janela → Mover e
    /// redimensionar.
    pub fn place_part(&mut self, surface: &ToplevelSurface, part: (f32, f32, f32, f32)) {
        let Some(window) = self.window_for(surface.wl_surface()).filter(|w| !crate::state::fixed_size(w) && self.fullscreen_entry(w).is_none()) else {
            return;
        };
        let Some(area) = self.area_for(&window) else {
            return;
        };
        let (w, h) = (area.size.w as f32, area.size.h as f32);
        let x = area.loc.x + (part.0 * w).round() as i32;
        let y = area.loc.y + (part.1 * h).round() as i32;
        let rect = smithay::utils::Rectangle::new((x, y).into(), (((part.2 * w).round() as i32).max(80), ((part.3 * h).round() as i32).max(60)).into());
        tracing::info!("janela em {:?} da tela", part);
        self.fill_area(surface, &window, rect, Some(part.0 == 0.0));
    }

    /// Área útil da tela de uma janela (pela superfície).
    pub fn usable_area_for_surface(&self, surface: &WlSurface) -> Option<smithay::utils::Rectangle<i32, Logical>> {
        let window = self.window_for(surface)?;
        self.area_for(&window)
    }

    /// Centraliza a janela na tela dela, sem mudar o tamanho (como o "Centralizar" do
    /// menu Janela do Mac).
    pub fn center_window(&mut self, surface: &ToplevelSurface) {
        let Some(window) = self.window_for(surface.wl_surface()).filter(|w| self.fullscreen_entry(w).is_none()) else {
            return;
        };
        let Some(area) = self.area_for(&window) else {
            return;
        };
        let size = window.geometry().size;
        let loc = (area.loc.x + (area.size.w - size.w).max(0) / 2, area.loc.y + (area.size.h - size.h).max(0) / 2);
        self.space.map_element(window, loc, true);
        tracing::info!("janela centralizada");
    }

    /// Leva a janela para a tela `name`, no mesmo lugar relativo (maximizada ou numa
    /// parte, continua ocupando a mesma parte da tela nova).
    pub fn move_to_output(&mut self, surface: &ToplevelSurface, name: &str) -> bool {
        let Some(window) = self.window_for(surface.wl_surface()) else {
            return false;
        };
        let Some(target) = self.space.outputs().find(|o| o.name() == name).cloned() else {
            return false;
        };
        // Levada de propósito: não volta sozinha à tela de onde saiu.
        self.displaced.retain(|d| d.window != window);
        // Em tela cheia: continua em tela cheia, na outra tela.
        if let Some(entry) = self.fullscreen.iter_mut().find(|e| e.window == window) {
            entry.output = name.to_string();
            self.fit_fullscreen(&window);
            tracing::info!("janela em tela cheia levada para a tela {name}");
            return true;
        }
        let (Some(from), Some(to)) = (self.area_for(&window), self.usable_area_of(&target)) else {
            return false;
        };
        let maximized = surface.current_state().states.contains(XdgState::Maximized);
        if maximized {
            self.maximize_in(surface, to);
        } else {
            let loc = self.space.element_location(&window).unwrap_or(from.loc);
            let size = window.geometry().size;
            let fx = (loc.x - from.loc.x) as f32 / from.size.w.max(1) as f32;
            let fy = (loc.y - from.loc.y) as f32 / from.size.h.max(1) as f32;
            let x = to.loc.x + ((fx * to.size.w as f32) as i32).clamp(0, (to.size.w - size.w).max(0));
            let y = to.loc.y + ((fy * to.size.h as f32) as i32).clamp(0, (to.size.h - size.h).max(0));
            self.space.map_element(window, (x, y), true);
        }
        tracing::info!("janela levada para a tela {name}");
        true
    }

    /// Ocupa `area` (maximizada, ou numa metade com `tiled`), guardando a posição e o
    /// tamanho de antes para restaurar.
    fn fill_area(&mut self, surface: &ToplevelSurface, window: &Window, area: smithay::utils::Rectangle<i32, Logical>, tiled: Option<bool>) {
        let window = window.clone();
        let already = surface.current_state().states.contains(XdgState::Maximized) || self.tiled.contains(surface.wl_surface());

        let from_loc = self.space.element_location(&window);
        let from_size = window.geometry().size;
        // Já maximizada ou numa metade: o "antes" continua o de quando estava solta.
        // Maximizada antes do primeiro quadro (app que lembra que estava maximizado, como
        // o VS Code, o Zed e o Eclipse): ainda não há tamanho de antes; um 0x0 aqui fazia
        // o desencaixe pedir 0x0 e grudar o canto da janela no ponteiro (A27).
        self.unsnap_fix = None;
        if !already && from_size.w > 0 && from_size.h > 0 {
            if let Some(loc) = from_loc {
                self.restore_positions.retain(|(s, _)| s != surface.wl_surface());
                self.restore_positions.push((surface.wl_surface().clone(), loc));
            }
            self.restore_sizes.retain(|(s, _)| s != surface.wl_surface());
            self.restore_sizes.push((surface.wl_surface().clone(), from_size));
        }
        let to_loc: Point<i32, Logical> = (area.loc.x, area.loc.y + bar_height_for(&window)).into();
        let bar_h = bar_height_for(&window);
        let to_size: Size<i32, Logical> = (area.size.w, (area.size.h - bar_h).max(1)).into();
        if let (Some(from_loc), true) = (from_loc, self.config.effects.animations) {
            self.animations.retain(|a| &a.surface != surface.wl_surface());
            self.animations.push(WindowAnimation {
                window: window.clone(),
                surface: surface.wl_surface().clone(),
                from_loc,
                from_size,
                to_loc,
                to_size,
                zoom_from: from_size.w as f32 / to_size.w.max(1) as f32,
                zoom_to: 1.0,
                alpha_from: 1.0,
                alpha_to: 1.0,
                hide_at_end: false,
                genie: false,
                follow_icon: false,
                duration: ANIMATION_DURATION,
                start: Instant::now(),
            });
        }

        surface.with_pending_state(|s| {
            match tiled {
                None => {
                    s.states.set(XdgState::Maximized);
                    s.states.unset(XdgState::TiledLeft);
                    s.states.unset(XdgState::TiledRight);
                }
                Some(left) => {
                    s.states.unset(XdgState::Maximized);
                    s.states.set(XdgState::TiledTop);
                    s.states.set(XdgState::TiledBottom);
                    s.states.set(if left { XdgState::TiledLeft } else { XdgState::TiledRight });
                    s.states.unset(if left { XdgState::TiledRight } else { XdgState::TiledLeft });
                }
            }
            s.size = Some(to_size);
        });
        surface.send_configure();
        self.tiled.retain(|s| s != surface.wl_surface());
        if tiled.is_some() {
            self.tiled.push(surface.wl_surface().clone());
        }
        // O conteúdo começa abaixo da barra de título, que fica no topo da área.
        self.space.map_element(window, to_loc, true);
    }

    /// A janela está encaixada (maximizada, numa metade, terço ou quarto)?
    pub fn is_snapped(&self, surface: &ToplevelSurface) -> bool {
        self.tiled.contains(surface.wl_surface()) || surface.with_pending_state(|s| s.states.contains(XdgState::Maximized))
    }

    /// Tamanho para restaurar uma janela que nunca esteve solta (nasceu maximizada):
    /// dois terços do tamanho de agora, como no GNOME.
    fn fallback_restore_size(size: Size<i32, Logical>) -> Size<i32, Logical> {
        ((size.w * 2 / 3).max(400), (size.h * 2 / 3).max(300)).into()
    }

    /// Arrastando uma janela encaixada: ela volta ao tamanho de antes e fica sob o
    /// ponteiro, no mesmo ponto proporcional da barra (como no Mac e no Windows).
    /// Devolve o novo deslocamento do ponteiro em relação à janela.
    ///
    /// Vale igual para a barra do mide, para apps Wayland com barra própria e para apps
    /// X11: o app só muda de tamanho quando responde ao configure, então até lá a janela
    /// segue o ponteiro no tamanho que tem, com o ponteiro na mesma fração da largura, e
    /// no quadro com o tamanho novo é reposta sob o ponteiro (`fix_unsnapped_drag`).
    /// Assim nenhum quadro mostra a janela fora do lugar, e um app que escolhe outro
    /// tamanho (ou demora a responder) também fica certo.
    pub fn unsnap_for_drag(&mut self, surface: &ToplevelSurface, window: &Window, pointer: Point<f64, Logical>, offset: Point<f64, Logical>) -> Point<f64, Logical> {
        let size = window.geometry().size;
        let restore = self
            .restore_sizes
            .iter()
            .position(|(s, _)| s == surface.wl_surface())
            .map(|i| self.restore_sizes.remove(i).1)
            .filter(|s| s.w > 0 && s.h > 0)
            .unwrap_or_else(|| Self::fallback_restore_size(size));
        self.restore_positions.retain(|(s, _)| s != surface.wl_surface());
        self.tiled.retain(|s| s != surface.wl_surface());
        self.animations.retain(|a| &a.surface != surface.wl_surface());
        surface.with_pending_state(|s| {
            s.states.unset(XdgState::Maximized);
            for tiled in [XdgState::TiledLeft, XdgState::TiledRight, XdgState::TiledTop, XdgState::TiledBottom] {
                s.states.unset(tiled);
            }
            s.size = Some(restore);
        });
        surface.send_configure();
        let fraction = (offset.x / size.w.max(1) as f64).clamp(0.0, 1.0);
        // Altura do ponteiro a partir do topo: a mesma, sem passar da metade da janela
        // restaurada (na barra do mide é negativa, dentro da barra).
        let offset_y = offset.y.min(restore.h as f64 / 2.0);
        self.unsnap_fix = Some(crate::state::UnsnapFix {
            window: window.clone(),
            from_size: size,
            fraction,
            offset_y,
            pointer,
            since: Instant::now(),
        });
        tracing::info!("janela desencaixada pelo arrasto: volta a {}x{} (ponteiro a {:.0}% da largura)", restore.w, restore.h, fraction * 100.0);
        Point::<f64, Logical>::from((fraction * size.w as f64, offset_y))
    }

    /// Quadro novo de uma janela desencaixada pelo arrasto: quando chega o tamanho
    /// novo (a resposta do app ao configure), ela é posta sob o ponteiro com ele na
    /// mesma fração da largura. Também acerta o arrasto em andamento.
    pub fn fix_unsnapped_drag(&mut self, window: &Window) {
        let Some(fix) = self.unsnap_fix.as_ref().filter(|f| &f.window == window) else {
            return;
        };
        // O app não respondeu (ou manteve o tamanho): desiste depois de um tempo, para um
        // quadro qualquer bem depois não mexer na janela.
        if fix.since.elapsed() > Duration::from_secs(5) {
            self.unsnap_fix = None;
            return;
        }
        let size = window.geometry().size;
        if size == fix.from_size || size.w <= 0 || size.h <= 0 {
            return;
        }
        let Some(fix) = self.unsnap_fix.take() else {
            return;
        };
        let offset = Point::<f64, Logical>::from((fix.fraction * size.w as f64, fix.offset_y.min(size.h as f64 / 2.0)));
        let mut loc: Point<i32, Logical> = (fix.pointer - offset).to_i32_round();
        if let Some(area) = self.output_at(fix.pointer).and_then(|o| self.usable_area_of(&o)) {
            loc.y = loc.y.max(area.loc.y + bar_height_for(window));
        }
        let dragging = self.move_grab.as_ref().is_some_and(|g| &g.window == window);
        if dragging && let Some(grab) = self.move_grab.as_mut() {
            grab.offset = offset;
            grab.fraction = None;
        }
        // Sobre uma zona de encaixe a janela fica parada e vai para lá ao soltar.
        if !(dragging && self.snap.is_some()) {
            self.space.map_element(window.clone(), loc, false);
        }
        tracing::info!("janela desencaixada chegou a {}x{}: posta em ({}, {}) sob o ponteiro", size.w, size.h, loc.x, loc.y);
        self.redraw = true;
    }

    /// Restaura a janela maximizada ao tamanho e posição anteriores, com animação.
    pub fn unmaximize(&mut self, surface: &ToplevelSurface) {
        let Some(window) = self.window_for(surface.wl_surface()) else {
            surface.send_configure();
            return;
        };
        if self.fullscreen_entry(&window).is_some() {
            surface.send_configure();
            return;
        }
        // Apps GTK pedem "desmaximizar" ao abrir, sem estarem maximizados. Nesse caso
        // não há o que restaurar, e a janela não pode sair do lugar.
        let tiled = self.tiled.contains(surface.wl_surface());
        if !surface.current_state().states.contains(XdgState::Maximized) && !tiled {
            surface.send_configure();
            return;
        }
        self.tiled.retain(|s| s != surface.wl_surface());
        // Restaurada pelo usuário: não volta sozinha à tela de onde saiu.
        self.displaced.retain(|d| d.window != window);
        // Pede de volta o tamanho que a janela tinha antes de maximizar, para a
        // animação saber o destino. Sem registro, o app escolhe o tamanho.
        // Janela que nasceu maximizada (sem tamanho de antes): dois terços do tamanho de
        // agora, no meio da área, como no desencaixe pelo arrasto (A27). Sem pedir um
        // tamanho, apps X11 pelo satellite ficavam do tamanho da tela.
        self.unsnap_fix = None;
        let saved = self
            .restore_sizes
            .iter()
            .position(|(s, _)| s == surface.wl_surface())
            .map(|i| self.restore_sizes.remove(i).1)
            .filter(|s| s.w > 0 && s.h > 0);
        let fallback = saved.is_none();
        let restore_size = Some(saved.unwrap_or_else(|| Self::fallback_restore_size(window.geometry().size)));
        surface.with_pending_state(|s| {
            s.states.unset(XdgState::Maximized);
            for tiled in [XdgState::TiledLeft, XdgState::TiledRight, XdgState::TiledTop, XdgState::TiledBottom] {
                s.states.unset(tiled);
            }
            s.size = restore_size;
        });
        surface.send_configure();

        let restore = self
            .restore_positions
            .iter()
            .position(|(s, _)| s == surface.wl_surface())
            .map(|i| self.restore_positions.remove(i).1)
            .filter(|_| !fallback)
            .or_else(|| {
                let area = self.area_for(&window)?;
                let size = restore_size?;
                let bar_h = bar_height_for(&window);
                Some((area.loc.x + (area.size.w - size.w) / 2, area.loc.y + bar_h + (area.size.h - bar_h - size.h).max(0) / 2).into())
            })
            .unwrap_or((48, 48).into());

        if let (Some(from_loc), Some(to_size), true) = (
            self.space.element_location(&window),
            restore_size,
            self.config.effects.animations,
        ) {
            self.animations.retain(|a| &a.surface != surface.wl_surface());
            let from_size = window.geometry().size;
            self.animations.push(WindowAnimation {
                window: window.clone(),
                surface: surface.wl_surface().clone(),
                from_loc,
                from_size,
                to_loc: restore,
                to_size,
                zoom_from: from_size.w as f32 / to_size.w.max(1) as f32,
                zoom_to: 1.0,
                alpha_from: 1.0,
                alpha_to: 1.0,
                hide_at_end: false,
                genie: false,
                follow_icon: false,
                duration: ANIMATION_DURATION,
                start: Instant::now(),
            });
        }
        self.space.map_element(window, restore, true);
    }

    /// Pede para a janela fechar (botão, atalho, menus) e marca o pedido: enquanto o
    /// app não fecha, ela é fotografada para esmaecer quando sumir.
    pub fn request_close(&mut self, window: &Window) {
        let Some(toplevel) = window.toplevel() else {
            return;
        };
        let id = window_id(window);
        self.close_requests.retain(|(w, _)| *w != id);
        self.close_requests.push((id, Instant::now()));
        toplevel.send_close();
    }

    /// Ajusta as janelas à área útil: maximizadas ocupam a área inteira, e as outras
    /// não ficam com a barra de título sob a barra superior (como no macOS).
    /// Todas são remapeadas de baixo para cima, para manter a ordem de empilhamento.
    /// Cada janela usa a área útil da tela onde está (o centro dela): com vários
    /// monitores, uma janela maximizada no externo ficava presa à área da principal e
    /// pulava para lá sempre que uma camada mudava (abrir um app pela dock).
    pub fn fit_windows_to_area(&mut self) {
        let windows: Vec<Window> = self.space.elements().cloned().collect();
        for window in windows {
            // Tela cheia: a tela dela inteira, sem a área útil.
            if self.fit_fullscreen(&window) {
                continue;
            }
            let Some(loc) = self.space.element_location(&window) else {
                continue;
            };
            let size = window.geometry().size;
            let center = Point::<f64, Logical>::from((loc.x as f64 + size.w as f64 / 2.0, loc.y as f64 + size.h.max(1) as f64 / 2.0));
            let Some(area) = self.output_at(center).and_then(|o| self.usable_area_of(&o)) else {
                continue;
            };
            let bar_h = bar_height_for(&window);
            let maximized = window
                .toplevel()
                .is_some_and(|t| t.current_state().states.contains(XdgState::Maximized));
            let target = if maximized {
                if let Some(toplevel) = window.toplevel() {
                    toplevel.with_pending_state(|s| s.size = Some((area.size.w, (area.size.h - bar_h).max(1)).into()));
                    toplevel.send_configure();
                }
                Point::from((area.loc.x, area.loc.y + bar_h))
            } else {
                Point::from((loc.x, loc.y.max(area.loc.y + bar_h)))
            };
            if target != loc {
                tracing::info!("janela ajustada à área útil: {:?} -> {:?}", (loc.x, loc.y), (target.x, target.y));
            }
            self.space.map_element(window, target, false);
        }
    }

}

impl State {
    /// Minimiza a janela da superfície: ela encolhe em direção ao centro inferior da
    /// tela, perde opacidade e sai do espaço. Volta com `restore_minimized`.
    pub fn minimize(&mut self, surface: &WlSurface) {
        let Some(window) = self.window_for(surface) else {
            return;
        };
        // Já minimizada (ainda na animação de saída: "minimizar tudo" de novo, dois
        // cliques no botão): uma entrada só. Duas deixavam a janela restaurada ainda na
        // lista, e o mide ignorava os cliques e as teclas dela (parecia congelada).
        if self.minimized.iter().any(|m| m.window == window) {
            return;
        }
        // Como no Mac, minimizar não vale em tela cheia (o amarelo da faixa fica cinza).
        if self.fullscreen_entry(&window).is_some() {
            tracing::info!("minimizar ignorado: janela em tela cheia");
            return;
        }
        let Some(loc) = self.space.element_location(&window) else {
            return;
        };
        let size = window.geometry().size;
        let target = self.dock_target(&window, size);
        let follow = self.dock_icon_for(&window).is_some();
        // Restaurando ainda (clique rápido no ícone): volta de onde a lâmpada está, sem
        // recomeçar do tamanho cheio.
        let start = self.reverse_start(surface, false);
        self.minimized.push(MinimizedWindow {
            window: window.clone(),
            loc,
            size,
        });
        self.animations.retain(|a| &a.surface != surface);
        if !self.config.effects.animations {
            // Sem animação, a janela sai do espaço na hora.
            self.space.unmap_elem(&window);
        } else {
        self.animations.push(WindowAnimation {
            window: window.clone(),
            surface: surface.clone(),
            from_loc: loc,
            from_size: size,
            to_loc: target.0,
            to_size: target.1,
            zoom_from: 1.0,
            zoom_to: MINIMIZE_SCALE,
            alpha_from: 1.0,
            alpha_to: 0.0,
            hide_at_end: true,
            genie: self.config.effects.lamp,
            follow_icon: follow,
            duration: if self.config.effects.lamp { GENIE_DURATION } else { ANIMATION_DURATION },
            start,
        });
        }
        // Se a janela tinha o foco, o foco vai para a janela de cima que sobrou.
        if let Some(keyboard) = self.seat.get_keyboard() {
            if keyboard.current_focus().as_ref() == Some(surface) {
                let next = self
                    .space
                    .elements()
                    .rev()
                    .find(|w| w.toplevel().map(|t| t.wl_surface()) != Some(surface))
                    .and_then(|w| w.toplevel().map(|t| t.wl_surface().clone()));
                let serial = smithay::utils::SERIAL_COUNTER.next_serial();
                keyboard.set_focus(self, next, serial);
            }
        }
        tracing::info!("janela minimizada");
    }

    /// Traz de volta a janela minimizada mais recente, com a animação de volta.
    pub fn restore_minimized(&mut self) {
        let Some(MinimizedWindow { window, loc, size }) = self.minimized.pop() else {
            return;
        };
        // Nenhuma outra entrada da mesma janela fica para trás (ela receberia o input).
        self.minimized.retain(|m| m.window != window);
        let Some(surface) = window.toplevel().map(|t| t.wl_surface().clone()) else {
            return;
        };
        let target = self.dock_target(&window, size);
        let follow = self.dock_icon_for(&window).is_some();
        // Minimizando ainda (clique rápido no ícone): a lâmpada inverte de onde está.
        let start = self.reverse_start(&surface, true);
        self.space.map_element(window.clone(), loc, true);
        self.animations.retain(|a| a.surface != surface);
        if self.config.effects.animations {
        self.animations.push(WindowAnimation {
            window: window.clone(),
            surface: surface.clone(),
            from_loc: target.0,
            from_size: target.1,
            to_loc: loc,
            to_size: size,
            zoom_from: MINIMIZE_SCALE,
            zoom_to: 1.0,
            alpha_from: 0.0,
            alpha_to: 1.0,
            hide_at_end: false,
            genie: self.config.effects.lamp,
            follow_icon: follow,
            duration: if self.config.effects.lamp { GENIE_DURATION } else { ANIMATION_DURATION },
            start,
        });
        }
        let serial = smithay::utils::SERIAL_COUNTER.next_serial();
        crate::input::focus_window(self, &window, serial);
        tracing::info!("janela restaurada");
    }

    /// Posição no primeiro quadro com conteúdo (cada janela é conferida uma vez):
    /// - diálogo com janela-pai (`xdg_toplevel.set_parent`; no X11, o `WM_TRANSIENT_FOR`
    ///   que o `xwayland-satellite` repassa): centrado sobre a pai (A26);
    /// - janela de abertura (splash, A26): no meio da tela onde está o ponteiro;
    /// - seletor de arquivos dos apps (`mide-chooser`, B25) e pedido de senha de
    ///   administrador (`mide-auth`, C7): no meio da área útil da tela da janela (ainda
    ///   não se ligam à janela de quem pediu).
    ///
    /// As outras janelas ficam onde a cascata as pôs (`map_new_window`).
    pub(crate) fn center_dialog_once(&mut self, window: &Window) {
        if window.geometry().size.w <= 0 {
            return;
        }
        let Some(toplevel) = window.toplevel().cloned() else {
            return;
        };
        if !with_states(toplevel.wl_surface(), |s| s.data_map.insert_if_missing_threadsafe(|| CenterChecked)) {
            return;
        }
        let app_id = with_states(toplevel.wl_surface(), |s| {
            s.data_map
                .get::<smithay::wayland::shell::xdg::XdgToplevelSurfaceData>()
                .and_then(|d| d.lock().ok().and_then(|a| a.app_id.clone()))
        })
        .unwrap_or_default();
        let size = window.geometry().size;
        let bar_h = bar_height_for(window);

        // Diálogo com janela-pai: centrado sobre ela, sem sair da área útil da tela dela.
        if let Some(parent) = toplevel.parent().and_then(|p| self.window_for(&p))
            && let (Some(ploc), Some(area)) = (self.space.element_location(&parent), self.area_for(&parent))
        {
            let pbar = bar_height_for(&parent);
            let psize = parent.geometry().size;
            let parent_frame = smithay::utils::Rectangle::new((ploc.x, ploc.y - pbar).into(), (psize.w, psize.h + pbar).into());
            let at = centered_over(parent_frame, area, size, bar_h);
            self.space.map_element(window.clone(), at, true);
            tracing::info!("diálogo centrado sobre a janela-pai em ({}, {})", at.x, at.y);
            return;
        }

        let splash = toplevel.parent().is_none() && {
            let screen = self.output_at(self.pointer_location).and_then(|o| self.space.output_geometry(&o));
            looks_like_splash(window, screen.map(|g| g.size))
        };
        if splash {
            // Janela de abertura: no meio da tela onde está o ponteiro.
            let Some(area) = self.output_at(self.pointer_location).and_then(|o| self.usable_area_of(&o)) else {
                return;
            };
            let at = centered_in(area, size, bar_h);
            self.space.map_element(window.clone(), at, true);
            tracing::info!("janela de abertura no meio da tela em ({}, {})", at.x, at.y);
            return;
        }

        if app_id != "mide-chooser" && app_id != "mide-auth" {
            return;
        }
        let Some(area) = self.area_for(window) else {
            return;
        };
        let at = centered_in(area, size, bar_h);
        self.space.map_element(window.clone(), at, true);
        tracing::info!("diálogo {app_id} no meio da tela em ({}, {})", at.x, at.y);
    }

    /// Primeira janela com conteúdo de um app aberto pela dock: sai do ícone dele com o
    /// efeito lâmpada (o mesmo de restaurar). Cada janela é conferida uma vez.
    pub(crate) fn open_animation_if_launched(&mut self, window: &Window) {
        self.pending_open.retain(|(_, at)| at.elapsed() < Duration::from_secs(15));
        if self.pending_open.is_empty() || window.geometry().size.w <= 0 {
            return;
        }
        let Some(toplevel) = window.toplevel().cloned() else {
            return;
        };
        let app_id = with_states(toplevel.wl_surface(), |s| {
            s.data_map
                .get::<smithay::wayland::shell::xdg::XdgToplevelSurfaceData>()
                .and_then(|d| d.lock().ok().and_then(|a| a.app_id.clone()))
        })
        .unwrap_or_default()
        .to_lowercase();
        if app_id.is_empty() {
            return;
        }
        let first = with_states(toplevel.wl_surface(), |s| s.data_map.insert_if_missing_threadsafe(|| OpenChecked));
        if !first {
            return;
        }
        let Some(index) = self.pending_open.iter().position(|(ids, _)| ids.contains(&app_id)) else {
            return;
        };
        self.pending_open.remove(index);
        let (Some(loc), Some(icon)) = (self.space.element_location(window), self.dock_icon_for(window)) else {
            return;
        };
        if !self.config.effects.animations {
            return;
        }
        tracing::info!("abrindo pela dock: {app_id} sai do ícone");
        let size = window.geometry().size;
        self.animations.retain(|a| &a.surface != toplevel.wl_surface());
        self.animations.push(WindowAnimation {
            window: window.clone(),
            surface: toplevel.wl_surface().clone(),
            from_loc: icon.loc,
            from_size: icon.size,
            to_loc: loc,
            to_size: size,
            zoom_from: MINIMIZE_SCALE,
            zoom_to: 1.0,
            alpha_from: 0.0,
            alpha_to: 1.0,
            hide_at_end: false,
            genie: self.config.effects.lamp,
            follow_icon: true,
            duration: if self.config.effects.lamp { GENIE_DURATION } else { ANIMATION_DURATION },
            start: Instant::now(),
        });
    }

    /// Início da animação de minimizar (`hiding` falso) ou restaurar (`hiding`
    /// verdadeiro: a que está em andamento minimiza) que inverte a contrária no ponto em
    /// que ela está: a nova começa "no passado", no instante em que o desenho é o mesmo.
    /// Sem a contrária em andamento, agora.
    fn reverse_start(&self, surface: &WlSurface, hiding: bool) -> Instant {
        let now = Instant::now();
        // Só a de minimizar ou restaurar (a janela encolhida numa ponta), não a de
        // maximizar ou encaixar.
        let dock_like = |a: &WindowAnimation| a.zoom_to == MINIMIZE_SCALE || a.zoom_from == MINIMIZE_SCALE;
        let Some(running) = self.animations.iter().find(|a| &a.surface == surface && a.hide_at_end == hiding && dock_like(a) && !a.finished(now)) else {
            return now;
        };
        // Quanto da ida já aconteceu (0 a 1, na curva) e o ponto da volta com o mesmo
        // desenho: a volta anda de 1 para 0 na mesma curva.
        let shown = running.eased(now);
        let duration = if self.config.effects.lamp { GENIE_DURATION } else { ANIMATION_DURATION };
        let ease = |t: f32| if self.config.effects.lamp { mide::motion::ease_in_out(t) } else { mide::motion::ease_out(t) };
        let t = inverse_ease(ease, 1.0 - shown);
        tracing::info!("lâmpada invertida em {:.0}%", shown * 100.0);
        now.checked_sub(duration.mul_f32(t)).unwrap_or(now)
    }

    /// A cada quadro: as animações que seguem o ícone da dock vão até onde ele está
    /// agora (a ampliação e a dock mudando mexem nele).
    pub(crate) fn follow_dock_icons(&mut self) {
        let targets: Vec<(usize, smithay::utils::Rectangle<i32, Logical>)> = self
            .animations
            .iter()
            .enumerate()
            .filter(|(_, a)| a.follow_icon)
            .filter_map(|(i, a)| Some((i, self.dock_icon_for(&a.window)?)))
            .collect();
        for (i, icon) in targets {
            let anim = &mut self.animations[i];
            if anim.hide_at_end {
                anim.to_loc = icon.loc;
                anim.to_size = icon.size;
            } else {
                anim.from_loc = icon.loc;
                anim.from_size = icon.size;
            }
        }
    }

    /// Posição e tamanho de origem da animação de minimizar: centro inferior, pequeno.
    fn dock_target(&self, window: &Window, size: Size<i32, Logical>) -> (Point<i32, Logical>, Size<i32, Logical>) {
        // Se a dock tem o ícone do app, o efeito vai até ele, onde ele estiver agora.
        if let Some(icon) = self.dock_icon_for(window) {
            tracing::info!("lâmpada: alvo no ícone da dock em {:?} {:?}", icon.loc, icon.size);
            return (icon.loc, icon.size);
        }
        tracing::debug!("lâmpada: ícone do app não encontrado na dock, alvo no centro inferior");
        let small: Size<i32, Logical> = (
            (size.w as f32 * MINIMIZE_SCALE).round() as i32,
            (size.h as f32 * MINIMIZE_SCALE).round() as i32,
        )
            .into();
        let area = self
            .primary_output()
            .and_then(|o| self.space.output_geometry(&o))
            .unwrap_or_else(|| smithay::utils::Rectangle::new((0, 0).into(), (1280, 800).into()));
        let x = area.loc.x + area.size.w / 2 - small.w / 2;
        let y = area.loc.y + area.size.h - small.h - 12;
        ((x, y).into(), small)
    }

    /// Alguma janela (fora as minimizadas) está onde a dock fica quando aparece?
    /// Usado pela dock que se esconde sozinha: só some quando há janela por cima dela.
    pub fn dock_covered(&self) -> bool {
        let Some(output) = self.primary_output() else {
            return false;
        };
        let Some(screen) = self.space.output_geometry(&output) else {
            return false;
        };
        let map = layer_map_for_output(&output);
        let Some(geo) = map.layers().find(|l| l.namespace() == "mide-dock").and_then(|l| map.layer_geometry(l)) else {
            return false;
        };
        // Onde a dock fica à mostra (margem de 8 px da borda de baixo), mesmo escondida,
        // em coordenadas globais.
        let shown = smithay::utils::Rectangle::<i32, Logical>::new(
            (screen.loc.x + geo.loc.x, screen.loc.y + screen.size.h - 8 - geo.size.h).into(),
            geo.size,
        );
        self.space.elements().any(|w| {
            self.space.element_location(w).is_some_and(|loc| {
                let rect = smithay::utils::Rectangle::new(loc, w.geometry().size);
                rect.overlaps(shown)
            })
        })
    }

    /// Área útil da saída: sem as zonas exclusivas das camadas (ex.: barra superior),
    /// em coordenadas globais. É onde janelas maximizadas ficam.
    pub fn usable_area(&self) -> Option<smithay::utils::Rectangle<i32, Logical>> {
        self.usable_area_of(&self.primary_output()?)
    }

    /// Área útil de uma tela (sem a barra e a dock, se estiverem nela), em
    /// coordenadas globais.
    pub fn usable_area_of(&self, output: &Output) -> Option<smithay::utils::Rectangle<i32, Logical>> {
        let geo = self.space.output_geometry(output)?;
        let zone = layer_map_for_output(output).non_exclusive_zone();
        Some(smithay::utils::Rectangle::new(geo.loc + zone.loc, zone.size))
    }

}

/// Janela de abertura (splash), pelo que o app pede (A26). Os apps X11 chegam pelo
/// `xwayland-satellite` como `xdg_toplevel` comuns: ele manda as janelas
/// `_NET_WM_WINDOW_TYPE_SPLASH` com o tamanho mínimo igual ao máximo, e as que pedem
/// "sem decoração" pelas dicas do Motif com o modo `client_side` do `xdg-decoration`.
/// A regra: tamanho fixo (mínimo == máximo), ou "sem decoração" e pequena (menos de
/// 2/3 da tela nas duas medidas, para não pegar janelas principais que desenham a
/// própria barra). Quem chama confere antes que a janela não tem pai.
fn looks_like_splash(window: &Window, screen: Option<Size<i32, Logical>>) -> bool {
    if crate::state::fixed_size(window) {
        return true;
    }
    let Some(toplevel) = window.toplevel() else {
        return false;
    };
    let no_decoration = with_states(toplevel.wl_surface(), |s| s.data_map.get::<crate::state::WantsNoDecoration>().is_some());
    let Some(screen) = screen else {
        return false;
    };
    no_decoration && is_small(window.geometry().size, screen)
}

/// Menor que 2/3 da tela nas duas medidas.
fn is_small(size: Size<i32, Logical>, screen: Size<i32, Logical>) -> bool {
    size.w * 3 < screen.w * 2 && size.h * 3 < screen.h * 2
}

/// Posição (do conteúdo) para a janela, com a barra do mide em cima (`bar_h`), ficar
/// no meio da área útil. Maior que a área, encosta no canto de cima à esquerda.
fn centered_in(
    area: smithay::utils::Rectangle<i32, Logical>,
    size: Size<i32, Logical>,
    bar_h: i32,
) -> Point<i32, Logical> {
    let x = area.loc.x + (area.size.w - size.w).max(0) / 2;
    let y = area.loc.y + bar_h + (area.size.h - size.h - bar_h).max(0) / 2;
    (x, y).into()
}

/// Posição (do conteúdo) para o diálogo ficar centrado sobre a moldura da janela-pai
/// (`parent_frame`, com a barra dela), sem sair da área útil (`area`).
fn centered_over(
    parent_frame: smithay::utils::Rectangle<i32, Logical>,
    area: smithay::utils::Rectangle<i32, Logical>,
    size: Size<i32, Logical>,
    bar_h: i32,
) -> Point<i32, Logical> {
    let frame_h = size.h + bar_h;
    let x = parent_frame.loc.x + (parent_frame.size.w - size.w) / 2;
    let y = parent_frame.loc.y + (parent_frame.size.h - frame_h) / 2;
    // Dentro da área útil: a barra do diálogo nunca fica embaixo da barra superior.
    let x = x.min(area.loc.x + area.size.w - size.w).max(area.loc.x);
    let y = y.min(area.loc.y + area.size.h - frame_h).max(area.loc.y);
    (x, y + bar_h).into()
}

/// Escala final da janela ao minimizar, em relação ao tamanho natural.
pub const MINIMIZE_SCALE: f32 = 0.12;

#[cfg(test)]
mod tests {
    use super::{cascade_position, centered_in, centered_over, is_small};
    use smithay::utils::Rectangle;

    #[test]
    fn abertura_no_meio_da_area_util() {
        // Tela 1366x768 com a barra (28) e a dock: área útil (0, 28) 1366x673.
        let area = Rectangle::new((0, 28).into(), (1366, 673).into());
        assert_eq!(centered_in(area, (300, 200).into(), 30), (533, 279).into());
        // Sem a barra do mide.
        assert_eq!(centered_in(area, (300, 200).into(), 0), (533, 264).into());
        // Maior que a área: no canto.
        assert_eq!(centered_in(area, (2000, 900).into(), 30), (0, 58).into());
    }

    #[test]
    fn dialogo_centrado_sobre_a_pai() {
        let area = Rectangle::new((0, 28).into(), (1366, 673).into());
        // Pai em (48, 76) 600x400 com barra: moldura (48, 46) 600x430.
        let pai = Rectangle::new((48, 46).into(), (600, 430).into());
        assert_eq!(centered_over(pai, area, (300, 200).into(), 30), (198, 176).into());
        // Diálogo maior que a pai, perto do canto: não passa da área útil.
        let pai = Rectangle::new((0, 28).into(), (200, 100).into());
        assert_eq!(centered_over(pai, area, (400, 300).into(), 30), (0, 58).into());
    }

    #[test]
    fn pequena_e_menos_de_dois_tercos() {
        let tela = (1366, 768).into();
        assert!(is_small((400, 300).into(), tela));
        assert!(!is_small((1000, 300).into(), tela));
        assert!(!is_small((400, 600).into(), tela));
    }

    #[test]
    fn primeira_janela_fica_na_margem() {
        assert_eq!(cascade_position(0), (48, 48).into());
    }

    #[test]
    fn janelas_seguintes_nao_se_sobrepoem_no_inicio() {
        let a = cascade_position(0);
        let b = cascade_position(1);
        let c = cascade_position(2);
        assert_ne!(a, b);
        assert_ne!(b, c);
        assert_eq!(b.x - a.x, 32);
        assert_eq!(c.y - b.y, 32);
    }
}


/// Quanto de cada janela fica à mostra na borda com a mesa à mostra.
const REVEAL_PEEK: i32 = 56;
/// Duração do afastar e voltar das janelas.
const REVEAL_DURATION: Duration = Duration::from_millis(380);

/// Mostrar a mesa (como clicar no papel de parede no macOS): as janelas deslizam para a
/// borda mais próxima, deixando uma faixa à mostra, e voltam com outro clique no papel
/// de parede, Esc, um clique numa janela ou ao abrir ou focar um app.
impl State {
    /// A mesa está à mostra?
    pub fn desktop_revealed(&self) -> bool {
        !self.revealed.is_empty()
    }

    /// Afasta as janelas, ou as traz de volta.
    pub fn toggle_reveal_desktop(&mut self) {
        if self.desktop_revealed() {
            self.restore_revealed_and_focus();
        } else {
            self.reveal_desktop();
        }
    }

    /// Afasta as janelas à mostra para as bordas da tela delas.
    pub fn reveal_desktop(&mut self) {
        if self.desktop_revealed() {
            return;
        }
        let windows: Vec<Window> = self
            .space
            .elements()
            .filter(|w| !self.minimized.iter().any(|m| &m.window == *w))
            .filter(|w| !self.animations.iter().any(|a| &a.window == *w && a.hide_at_end))
            .cloned()
            .collect();
        for window in windows {
            let Some(loc) = self.space.element_location(&window) else {
                continue;
            };
            let w = window.geometry().size.w;
            let h = window.geometry().size.h;
            // A tela da janela é a que tem o centro dela (ou a principal).
            let center = Point::<i32, Logical>::from((loc.x + w / 2, loc.y + h / 2));
            let output = self
                .space
                .outputs()
                .find(|o| self.space.output_geometry(o).is_some_and(|g| g.contains(center)))
                .cloned()
                .or_else(|| self.space.outputs_for_element(&window).first().cloned())
                .or_else(|| self.primary_output());
            let Some((screen, name)) = output.and_then(|o| self.space.output_geometry(&o).map(|g| (g, o.name()))) else {
                continue;
            };
            // Vai até a borda da própria tela; o que passa dela não aparece na vizinha.
            self.reveal_homes.retain(|(other, _)| other != &window);
            self.reveal_homes.push((window.clone(), name));
            // Para o lado mais perto: o centro da janela decide.
            let to_x = if loc.x + w / 2 < screen.loc.x + screen.size.w / 2 {
                screen.loc.x - w + REVEAL_PEEK
            } else {
                screen.loc.x + screen.size.w - REVEAL_PEEK
            };
            self.slide_window(&window, loc, (to_x, loc.y).into());
            self.revealed.push((window, loc));
        }
        tracing::info!("mesa à mostra: {} janela(s) afastada(s)", self.revealed.len());
    }

    /// Traz as janelas afastadas de volta para onde estavam.
    pub fn restore_revealed(&mut self) {
        let revealed = std::mem::take(&mut self.revealed);
        if revealed.is_empty() {
            return;
        }
        tracing::info!("mesa: {} janela(s) de volta", revealed.len());
        for (window, loc) in revealed {
            // Fechada ou minimizada enquanto isso: fica como está.
            if self.minimized.iter().any(|m| m.window == window) {
                continue;
            }
            let Some(now) = self.space.element_location(&window) else {
                continue;
            };
            self.slide_window(&window, now, loc);
        }
    }

    /// Traz as janelas de volta e devolve o foco à de cima (Esc, segundo clique no
    /// papel de parede, Super+D): a área de trabalho tinha ficado com o teclado.
    pub fn restore_revealed_and_focus(&mut self) {
        let top = self.revealed.iter().map(|(w, _)| w.clone()).collect::<Vec<_>>();
        self.restore_revealed();
        let window = self.space.elements().rev().find(|w| top.contains(w)).cloned();
        if let Some(window) = window {
            crate::input::focus_window(self, &window, smithay::utils::SERIAL_COUNTER.next_serial());
        }
    }

    /// Move a janela de `from` a `to`, deslizando se as animações estão ligadas.
    fn slide_window(&mut self, window: &Window, from: Point<i32, Logical>, to: Point<i32, Logical>) {
        if let (Some(toplevel), true) = (window.toplevel(), self.config.effects.animations) {
            let surface = toplevel.wl_surface().clone();
            let size = window.geometry().size;
            self.animations.retain(|a| a.surface != surface);
            self.animations.push(WindowAnimation {
                window: window.clone(),
                surface,
                from_loc: from,
                from_size: size,
                to_loc: to,
                to_size: size,
                zoom_from: 1.0,
                zoom_to: 1.0,
                alpha_from: 1.0,
                alpha_to: 1.0,
                hide_at_end: false,
                genie: false,
                follow_icon: false,
                duration: REVEAL_DURATION,
                start: Instant::now(),
            });
        }
        self.space.map_element(window.clone(), to, false);
        self.wake();
    }
}

/// O `t` (0 a 1) em que a curva `ease` vale `y` (as curvas do mide são crescentes).
fn inverse_ease(ease: impl Fn(f32) -> f32, y: f32) -> f32 {
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    for _ in 0..24 {
        let mid = (lo + hi) / 2.0;
        if ease(mid) < y {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    (lo + hi) / 2.0
}

#[cfg(test)]
mod reverse_tests {
    use super::inverse_ease;

    #[test]
    fn curva_invertida_volta_ao_mesmo_ponto() {
        for y in [0.0f32, 0.1, 0.5, 0.83, 1.0] {
            let t = inverse_ease(mide::motion::ease_out, y);
            assert!((mide::motion::ease_out(t) - y).abs() < 1e-3, "{y}");
            let t = inverse_ease(mide::motion::ease_in_out, y);
            assert!((mide::motion::ease_in_out(t) - y).abs() < 1e-3, "{y}");
        }
    }
}
