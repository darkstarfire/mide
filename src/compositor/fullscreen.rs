//! Tela cheia, como no macOS: a janela ocupa o monitor inteiro, sem a barra de título
//! do mide, e a barra superior e a dock somem dessa tela. Levar o ponteiro à borda de
//! cima faz descer a barra superior e, logo abaixo, uma faixa com a barra de título
//! da janela (luzes e título), que sobem de novo quando o ponteiro sai.
//!
//! Entra e sai pelo botão verde (⌥+clique continua maximizando), pelo ⌃⌘F, pelo menu
//! Janela e pelo próprio app (`xdg_toplevel.set_fullscreen`: F11 dos navegadores e do
//! VLC, jogos; o `xwayland-satellite` repassa o dos apps X11 pelo mesmo pedido).
//!
//! Decisões (como no Mac, dentro do que um compositor sem "espaços" faz):
//! - a tela fica "em tela cheia" enquanto a janela de cima nela é a janela em tela
//!   cheia. Alt+Tab (ou a dock) para outra janela a traz por cima, e a barra e a dock
//!   voltam; voltar à janela em tela cheia esconde as duas de novo. Diálogos (janelas
//!   com pai) por cima não contam;
//! - com vários monitores, só o monitor da janela muda;
//! - Esc não sai (é dos apps); minimizar não vale em tela cheia (como no Mac).

use std::time::{Duration, Instant};

use smithay::{
    desktop::{layer_map_for_output, LayerSurface, Window},
    output::Output,
    reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State as XdgState,
    utils::{Logical, Point, Rectangle, Size},
    wayland::shell::{wlr_layer::Layer, xdg::ToplevelSurface},
};

use crate::decorations::BAR_HEIGHT;
use crate::state::{State, WindowAnimation, ANIMATION_DURATION};

/// Janela em tela cheia e como ela estava antes (para voltar ao sair).
#[derive(Debug, Clone)]
pub struct Entry {
    pub window: Window,
    /// Tela onde está (pelo nome).
    pub output: String,
    /// Posição e tamanho de antes (tamanho 0: a janela entrou antes do primeiro quadro).
    pub loc: Option<Point<i32, Logical>>,
    pub size: Size<i32, Logical>,
    /// Estava maximizada, ou numa parte da tela (os estados de encaixe de antes).
    pub maximized: bool,
    pub tiled: Option<[bool; 4]>,
}

/// Altura da borda de cima que revela a barra (pontos).
pub const EDGE: f64 = 2.0;
/// Tempo parado na borda até a barra descer.
pub const HOVER_DELAY: Duration = Duration::from_millis(300);
/// Empurrar o ponteiro contra a borda (outro movimento lá) desce a barra mais cedo.
pub const PUSH_DELAY: Duration = Duration::from_millis(120);
/// Descida e subida da barra.
pub const SLIDE: Duration = mide::motion::DOCK_SLIDE;

/// Barra revelada no topo de uma tela em tela cheia.
#[derive(Debug, Default)]
pub struct Reveal {
    /// Ponteiro na borda de cima de uma tela em tela cheia: a tela e desde quando.
    hover: Option<(String, Instant)>,
    /// Tela da barra descida (ou subindo).
    output: Option<String>,
    shown: bool,
    /// Quando desceu ou começou a subir (sem animação: `None`).
    changed: Option<Instant>,
    /// Altura da barra superior da tela, medida ao descer. Guardada porque o desenho e
    /// o clique nas camadas perguntam o deslocamento com o mapa das camadas trancado
    /// (`layer_map_for_output` de novo ali travaria o compositor).
    bar_h: i32,
}

impl Reveal {
    /// Quanto a barra desceu na tela `output`, de 0 (escondida) a 1 (toda à mostra).
    pub fn progress(&self, output: &str, now: Instant) -> f32 {
        if self.output.as_deref() != Some(output) {
            return 0.0;
        }
        let t = self.changed.map_or(1.0, |at| (now.duration_since(at).as_secs_f32() / SLIDE.as_secs_f32()).clamp(0.0, 1.0));
        let e = mide::motion::ease_out(t);
        if self.shown { e } else { 1.0 - e }
    }

    /// A barra está descendo ou subindo (o backend continua pedindo quadros).
    pub fn animating(&self, now: Instant) -> bool {
        self.output.is_some() && self.changed.is_some_and(|at| now.duration_since(at) < SLIDE)
    }

    /// Barra descida (ou descendo) na tela `output`: os botões da faixa funcionam.
    pub fn shown_on(&self, output: &str) -> bool {
        self.shown && self.output.as_deref() == Some(output)
    }
}

/// Modo de jogo em tela cheia (A18): com uma janela em tela cheia de verdade (não só
/// maximizada) por cima, a tela fica só para ela. O mide cede o que é dele: a cena
/// dessa tela leva só a janela (sem papel de parede, ícones da mesa, janelas de baixo
/// nem vidro), os caches de desfoque da tela são soltos, as janelas e camadas escondidas
/// recebem o aviso de quadro no máximo uma vez por segundo, a barra e a dock param as
/// tarefas periódicas (perguntam `game_mode`/`launcher state` pelo canal de controle) e
/// as notificações vão só para a central. Rede, Bluetooth, som e o resto do sistema não
/// mudam. Desligável em Configurações → Efeitos (`[effects] game_mode`).
#[derive(Debug, Default)]
pub struct GameMode {
    /// Telas no modo agora (para registrar a entrada e a saída uma vez cada).
    outputs: Vec<String>,
    /// Por que o scanout direto (o buffer do app direto ao plano da placa) não vale
    /// neste backend; o backend diz ao começar. `None`: o backend não disse.
    pub scanout_note: Option<&'static str>,
}

/// Aviso de quadro das janelas e camadas escondidas pelo modo de jogo: no máximo um por
/// este intervalo (não param de vez: um app que espera o aviso não fica travado).
pub const HIDDEN_FRAME_THROTTLE: Duration = Duration::from_secs(1);

/// A camada some numa tela em tela cheia? A barra superior, a dock (e as camadas dela)
/// e as camadas de cima que reservam espaço (barras e docks de terceiros). As
/// notificações e as camadas Overlay (aviso de volume, menus, captura) continuam.
pub fn hidden_in_fullscreen(layer: &LayerSurface) -> bool {
    let ns = layer.namespace();
    if ns == "mide-notifications" {
        return false;
    }
    if ns == "mide-bar" || ns.starts_with("mide-dock") {
        return true;
    }
    use smithay::wayland::shell::wlr_layer::ExclusiveZone;
    layer.layer() == Layer::Top && matches!(layer.cached_state().exclusive_zone, ExclusiveZone::Exclusive(n) if n > 0)
}

/// A janela está (ou acabou de ser pedida) em tela cheia? Vale o estado pendente: a
/// barra do mide some na hora do pedido, sem esperar o app responder.
pub fn is_fullscreen(window: &Window) -> bool {
    window.toplevel().is_some_and(|t| t.with_pending_state(|s| s.states.contains(XdgState::Fullscreen)))
}

impl State {
    /// Registro de tela cheia da janela.
    pub fn fullscreen_entry(&self, window: &Window) -> Option<&Entry> {
        self.fullscreen.iter().find(|e| &e.window == window)
    }

    /// Janela em tela cheia que manda na tela `output`: a de cima nela (sem contar
    /// diálogos, janelas com pai) está em tela cheia nessa tela.
    pub fn fullscreen_on(&self, output: &Output) -> Option<Window> {
        if self.fullscreen.is_empty() {
            return None;
        }
        let name = output.name();
        let geo = self.space.output_geometry(output)?;
        let top = self
            .space
            .elements()
            .rev()
            .filter(|w| !self.minimized.iter().any(|m| &m.window == *w))
            .filter(|w| w.toplevel().is_none_or(|t| t.parent().is_none()))
            .find(|w| {
                self.space.element_location(w).is_some_and(|loc| {
                    let size = w.geometry().size;
                    geo.contains(loc + Point::from((size.w / 2, size.h.max(1) / 2)))
                })
            })?;
        self.fullscreen.iter().any(|e| &e.window == top && e.output == name).then(|| top.clone())
    }

    /// Janela que tem a tela `output` só para ela (modo de jogo, A18): a opção ligada, a
    /// sessão desbloqueada e a janela de cima nessa tela em tela cheia.
    pub fn game_window(&self, output: &Output) -> Option<Window> {
        if !self.config.effects.game_mode || self.lock.locked {
            return None;
        }
        self.fullscreen_on(output)
    }

    /// A tela principal (a da barra, da dock e dos banners) está no modo de jogo.
    pub fn game_on_primary(&self) -> bool {
        self.primary_output().is_some_and(|o| self.game_window(&o).is_some())
    }

    /// A barra superior está escondida pelo modo de jogo (não revelada pelo ponteiro no
    /// topo): ela pode pausar o relógio, a bateria, a rede e a mídia.
    pub fn bar_paused(&self) -> bool {
        self.primary_output()
            .is_some_and(|o| self.game_window(&o).is_some() && self.fs_reveal.progress(&o.name(), Instant::now()) <= 0.0 && !self.fs_reveal.shown_on(&o.name()))
    }

    /// Confere quais telas estão no modo de jogo, registra quem entrou e quem saiu e
    /// devolve as telas que acabaram de entrar (o desenho solta os caches delas).
    pub fn refresh_game_mode(&mut self) -> Vec<String> {
        let now: Vec<(String, Window)> = self.space.outputs().filter_map(|o| Some((o.name(), self.game_window(o)?))).collect();
        let mut entered = Vec::new();
        for (name, window) in &now {
            if !self.game.outputs.contains(name) {
                let title = crate::render::window_title(window);
                let scanout = self.game.scanout_note.unwrap_or("não: backend sem plano da placa");
                tracing::info!("modo tela cheia: ligado na tela {name} ({title:?}); scanout direto {scanout}");
                entered.push(name.clone());
            }
        }
        let left: Vec<String> = self.game.outputs.iter().filter(|n| !now.iter().any(|(m, _)| m == *n)).cloned().collect();
        for name in &left {
            tracing::info!("modo tela cheia: desligado na tela {name}");
        }
        if !entered.is_empty() || !left.is_empty() {
            self.game.outputs = now.into_iter().map(|(n, _)| n).collect();
            // A barra, a dock e a mesa voltam a receber os avisos de quadro: um quadro já.
            self.wake();
        }
        entered
    }

    /// Janelas escondidas pelo modo de jogo: abaixo da janela em tela cheia, na tela
    /// dela (a tela principal de cada janela, como no aviso de quadro).
    pub fn game_hidden_windows(&self) -> Vec<Window> {
        if self.game.outputs.is_empty() {
            return Vec::new();
        }
        let mut hidden = Vec::new();
        for output in self.space.outputs() {
            let Some(game) = self.game_window(output) else {
                continue;
            };
            for window in self.space.elements().take_while(|w| **w != game) {
                if crate::presentation::primary_output(self, window).as_ref() == Some(output) {
                    hidden.push(window.clone());
                }
            }
        }
        hidden
    }

    /// Deslocamento vertical de uma camada na tela `output`: `None` se ela está
    /// escondida pela tela cheia; a barra superior revelada desce de cima.
    pub fn layer_offset(&self, output: &Output, layer: &LayerSurface) -> Option<i32> {
        if !hidden_in_fullscreen(layer) || self.fullscreen_on(output).is_none() {
            return Some(0);
        }
        if layer.namespace() != "mide-bar" {
            return None;
        }
        let p = self.fs_reveal.progress(&output.name(), Instant::now());
        (p > 0.0).then(|| self.reveal_shift(p))
    }

    /// Altura da barra superior na tela (0 se ela não estiver lá).
    fn top_bar_height(&self, output: &Output) -> i32 {
        let source = self.layer_output(output);
        let map = layer_map_for_output(&source);
        map.layers()
            .find(|l| l.namespace() == "mide-bar")
            .and_then(|l| map.layer_geometry(l))
            .map_or(0, |g| g.size.h)
    }

    /// Deslocamento da barra e da faixa reveladas com o progresso `p`: as duas descem
    /// juntas de cima, como um bloco só.
    fn reveal_shift(&self, p: f32) -> i32 {
        let total = self.fs_reveal.bar_h + BAR_HEIGHT;
        (-(total as f32) * (1.0 - p)).round() as i32
    }

    /// Faixa com a barra de título da janela em tela cheia revelada na tela `output`,
    /// em coordenadas globais, e a janela. `None` se a barra não está à mostra.
    pub fn reveal_strip(&self, output: &Output) -> Option<(Window, Rectangle<i32, Logical>)> {
        let window = self.fullscreen_on(output)?;
        let p = self.fs_reveal.progress(&output.name(), Instant::now());
        if p <= 0.0 {
            return None;
        }
        let screen = self.space.output_geometry(output)?;
        let loc = self.space.element_location(&window)?;
        let y = screen.loc.y + self.fs_reveal.bar_h + self.reveal_shift(p);
        Some((window.clone(), Rectangle::new((loc.x, y).into(), (window.geometry().size.w, BAR_HEIGHT).into())))
    }

    /// Altura da barra superior revelada (medida quando ela desceu).
    pub fn fs_reveal_bar_height(&self) -> i32 {
        self.fs_reveal.bar_h
    }

    /// Faixa revelada sob o ponteiro (só com a barra toda à mostra ou descendo).
    pub fn reveal_strip_at(&self, pos: Point<f64, Logical>) -> Option<(Window, Rectangle<i32, Logical>)> {
        let output = self.output_at(pos)?;
        if !self.fs_reveal.shown_on(&output.name()) {
            return None;
        }
        self.reveal_strip(&output).filter(|(_, r)| r.to_f64().contains(pos))
    }

    /// Põe a janela em tela cheia na tela `output` (ou na dela), guardando como estava.
    pub fn enter_fullscreen(&mut self, surface: &ToplevelSurface, output: Option<Output>) {
        let Some(window) = self.window_for(surface.wl_surface()) else {
            return;
        };
        if self.fullscreen_entry(&window).is_some() {
            if surface.is_initial_configure_sent() {
                surface.send_configure();
            }
            return;
        }
        // Janela de tamanho fixo (Sobre este computador): como no Mac, não vai à tela cheia.
        if crate::state::fixed_size(&window) {
            if surface.is_initial_configure_sent() {
                surface.send_configure();
            }
            return;
        }
        let from_loc = self.space.element_location(&window);
        let from_size = window.geometry().size;
        let output = output.or_else(|| {
            let loc = from_loc?;
            self.space.output_under((loc + Point::from((from_size.w / 2, from_size.h.max(1) / 2))).to_f64()).next().cloned()
        });
        let Some(output) = output.or_else(|| self.primary_output()) else {
            return;
        };
        let Some(screen) = self.space.output_geometry(&output) else {
            return;
        };
        let (maximized, tiled) = surface.with_pending_state(|s| {
            let st = &s.states;
            let tiled = self.tiled.contains(surface.wl_surface()).then(|| {
                [st.contains(XdgState::TiledLeft), st.contains(XdgState::TiledRight), st.contains(XdgState::TiledTop), st.contains(XdgState::TiledBottom)]
            });
            (st.contains(XdgState::Maximized), tiled)
        });
        self.fullscreen.push(Entry {
            window: window.clone(),
            output: output.name(),
            loc: from_loc.filter(|_| from_size.w > 0 && from_size.h > 0),
            size: from_size,
            maximized,
            tiled,
        });
        self.tiled.retain(|s| s != surface.wl_surface());
        self.unsnap_fix = None;
        self.displaced.retain(|d| d.window != window);
        if let (Some(from_loc), true, true) = (from_loc, from_size.w > 0, self.config.effects.animations) {
            self.animations.retain(|a| &a.surface != surface.wl_surface());
            self.animations.push(WindowAnimation {
                window: window.clone(),
                surface: surface.wl_surface().clone(),
                from_loc,
                from_size,
                to_loc: screen.loc,
                to_size: screen.size,
                zoom_from: from_size.w as f32 / screen.size.w.max(1) as f32,
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
        let client_output = {
            use smithay::reexports::wayland_server::Resource;
            surface.wl_surface().client().and_then(|c| output.client_outputs(&c).into_iter().next())
        };
        surface.with_pending_state(|s| {
            s.states.set(XdgState::Fullscreen);
            for state in [XdgState::Maximized, XdgState::TiledLeft, XdgState::TiledRight, XdgState::TiledTop, XdgState::TiledBottom] {
                s.states.unset(state);
            }
            s.size = Some(screen.size);
            s.fullscreen_output = client_output;
        });
        if surface.is_initial_configure_sent() {
            surface.send_configure();
        }
        self.space.map_element(window.clone(), screen.loc, true);
        let serial = smithay::utils::SERIAL_COUNTER.next_serial();
        crate::input::focus_window(self, &window, serial);
        self.fs_reveal = Reveal::default();
        tracing::info!("janela em tela cheia na tela {} ({}x{})", output.name(), screen.size.w, screen.size.h);
        // A janela mudou de lugar sob o ponteiro parado: o foco dele acompanha.
        crate::input::refresh_pointer_focus(self);
        self.wake();
    }

    /// Sai da tela cheia: a janela volta ao lugar e ao tamanho de antes (ou maximizada,
    /// ou à parte da tela em que estava).
    pub fn exit_fullscreen(&mut self, surface: &ToplevelSurface) {
        let Some(window) = self.window_for(surface.wl_surface()) else {
            return;
        };
        let Some(i) = self.fullscreen.iter().position(|e| e.window == window) else {
            // Pedido sem estar em tela cheia (alguns apps mandam ao abrir): só confirma.
            surface.with_pending_state(|s| s.states.unset(XdgState::Fullscreen));
            if surface.is_initial_configure_sent() {
                surface.send_configure();
            }
            return;
        };
        let entry = self.fullscreen.remove(i);
        let from_loc = self.space.element_location(&window);
        let from_size = window.geometry().size;
        let output = self.space.outputs().find(|o| o.name() == entry.output).cloned().or_else(|| self.primary_output());
        let area = output.as_ref().and_then(|o| self.usable_area_of(o));
        surface.with_pending_state(|s| {
            s.states.unset(XdgState::Fullscreen);
            s.fullscreen_output = None;
        });
        // Daqui em diante a barra do mide volta (o estado pendente já saiu da tela cheia).
        let bar_h = crate::state::bar_height_for(&window);
        let (to_loc, to_size) = match (entry.maximized, area) {
            (true, Some(area)) => {
                surface.with_pending_state(|s| s.states.set(XdgState::Maximized));
                ((area.loc.x, area.loc.y + bar_h).into(), Size::from((area.size.w, (area.size.h - bar_h).max(1))))
            }
            _ => {
                let size = if entry.size.w > 0 && entry.size.h > 0 {
                    entry.size
                } else {
                    let base = area.map_or(from_size, |a| a.size);
                    ((base.w * 2 / 3).max(400), (base.h * 2 / 3).max(300)).into()
                };
                let loc = entry.loc.unwrap_or_else(|| match area {
                    Some(a) => (a.loc.x + (a.size.w - size.w).max(0) / 2, a.loc.y + bar_h + (a.size.h - bar_h - size.h).max(0) / 2).into(),
                    None => (48, 76).into(),
                });
                if let Some(flags) = entry.tiled {
                    surface.with_pending_state(|s| {
                        for (on, state) in flags.into_iter().zip([XdgState::TiledLeft, XdgState::TiledRight, XdgState::TiledTop, XdgState::TiledBottom]) {
                            if on {
                                s.states.set(state);
                            }
                        }
                    });
                    self.tiled.push(surface.wl_surface().clone());
                }
                (loc, size)
            }
        };
        surface.with_pending_state(|s| s.size = Some(to_size));
        surface.send_configure();
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
        self.space.map_element(window, to_loc, true);
        self.fs_reveal = Reveal::default();
        tracing::info!("janela saiu da tela cheia: volta a ({}, {}) {}x{}", to_loc.x, to_loc.y, to_size.w, to_size.h);
        // A janela mudou de lugar sob o ponteiro parado: o foco dele acompanha.
        crate::input::refresh_pointer_focus(self);
        self.wake();
    }

    /// Entra ou sai da tela cheia (botão verde, ⌃⌘F, menu Janela).
    pub fn toggle_fullscreen(&mut self, surface: &ToplevelSurface) {
        let in_fullscreen = self.window_for(surface.wl_surface()).is_some_and(|w| self.fullscreen_entry(&w).is_some());
        if in_fullscreen {
            self.exit_fullscreen(surface);
        } else {
            self.enter_fullscreen(surface, None);
        }
    }

    /// A janela fechou: esquece a tela cheia dela.
    pub fn forget_fullscreen(&mut self, window: &Window) {
        if self.fullscreen.iter().any(|e| &e.window == window) {
            self.fullscreen.retain(|e| &e.window != window);
            self.fs_reveal = Reveal::default();
        }
    }

    /// As telas mudaram: cada janela em tela cheia ocupa a tela dela inteira (ou a
    /// tela onde ficou, se a dela saiu). Devolve se a janela era de tela cheia.
    pub fn fit_fullscreen(&mut self, window: &Window) -> bool {
        let Some(i) = self.fullscreen.iter().position(|e| &e.window == window) else {
            return false;
        };
        let output = self.space.outputs().find(|o| o.name() == self.fullscreen[i].output).cloned().or_else(|| {
            let loc = self.space.element_location(window)?;
            self.output_at(loc.to_f64())
        });
        let Some((output, screen)) = output.and_then(|o| Some((o.clone(), self.space.output_geometry(&o)?))) else {
            return true;
        };
        self.fullscreen[i].output = output.name();
        if let Some(toplevel) = window.toplevel() {
            let changed = toplevel.with_pending_state(|s| std::mem::replace(&mut s.size, Some(screen.size)) != Some(screen.size));
            if changed {
                toplevel.send_configure();
            }
        }
        if self.space.element_location(window) != Some(screen.loc) {
            self.space.map_element(window.clone(), screen.loc, false);
        }
        true
    }

    /// Ponteiro andou: com uma janela em tela cheia, parar na borda de cima (ou
    /// empurrar contra ela) desce a barra; sair da barra e da faixa sobe.
    pub fn fullscreen_pointer(&mut self, pos: Point<f64, Logical>) {
        if self.fullscreen.is_empty() {
            if self.fs_reveal.output.is_some() || self.fs_reveal.hover.is_some() {
                self.fs_reveal = Reveal::default();
            }
            return;
        }
        let now = Instant::now();
        let Some(output) = self.output_at(pos) else {
            return;
        };
        let name = output.name();
        let full = self.fullscreen_on(&output).is_some();
        let Some(screen) = self.space.output_geometry(&output) else {
            return;
        };
        let y = pos.y - screen.loc.y as f64;
        if self.fs_reveal.shown_on(&name) {
            // Fica enquanto o ponteiro está na barra ou na faixa, ou com um menu da barra aberto.
            let bottom = (self.fs_reveal.bar_h + BAR_HEIGHT) as f64 + 6.0;
            if !(full && (y < bottom || self.bar_menu_open(&output))) {
                self.hide_reveal(now);
            }
            return;
        }
        if self.fs_reveal.shown {
            self.hide_reveal(now);
        }
        if !full || y >= EDGE {
            self.fs_reveal.hover = None;
            return;
        }
        match &self.fs_reveal.hover {
            Some((n, since)) if *n == name => {
                if now.duration_since(*since) >= PUSH_DELAY {
                    self.show_reveal(&name, now);
                }
            }
            _ => {
                self.fs_reveal.hover = Some((name, now));
                // Parado, nenhum evento chega: o relógio confere de novo depois do tempo.
                let timer = smithay::reexports::calloop::timer::Timer::from_duration(HOVER_DELAY + Duration::from_millis(5));
                let _ = self.loop_handle.insert_source(timer, |_, _, state| {
                    state.fullscreen_hover_tick();
                    smithay::reexports::calloop::timer::TimeoutAction::Drop
                });
            }
        }
    }

    /// Relógio do ponteiro parado na borda: desce a barra se ele continua lá.
    fn fullscreen_hover_tick(&mut self) {
        let Some((name, since)) = self.fs_reveal.hover.clone() else {
            return;
        };
        let pos = self.pointer_location;
        let still = self.output_at(pos).filter(|o| o.name() == name).and_then(|o| {
            let screen = self.space.output_geometry(&o)?;
            Some(pos.y - (screen.loc.y as f64) < EDGE && self.fullscreen_on(&o).is_some())
        });
        if still == Some(true) && since.elapsed() + Duration::from_millis(10) >= HOVER_DELAY {
            self.show_reveal(&name, Instant::now());
        }
    }

    fn show_reveal(&mut self, name: &str, now: Instant) {
        if let Some(output) = self.space.outputs().find(|o| o.name() == name).cloned() {
            self.fs_reveal.bar_h = self.top_bar_height(&output);
        }
        self.fs_reveal.hover = None;
        self.fs_reveal.output = Some(name.to_string());
        self.fs_reveal.shown = true;
        self.fs_reveal.changed = self.config.effects.animations.then_some(now);
        tracing::info!("tela cheia: barra revelada na tela {name}");
        self.wake();
    }

    fn hide_reveal(&mut self, now: Instant) {
        self.fs_reveal.hover = None;
        self.fs_reveal.shown = false;
        // Sobe a partir de onde está (se ainda descia, sem pular).
        let started = self.fs_reveal.changed.map(|at| now.duration_since(at).min(SLIDE));
        self.fs_reveal.changed = match (self.config.effects.animations, started) {
            (true, Some(gone)) => Some(now - (SLIDE - gone)),
            (true, None) => Some(now),
            (false, _) => None,
        };
        if !self.config.effects.animations {
            self.fs_reveal.output = None;
        }
        tracing::info!("tela cheia: barra escondida");
        self.wake();
    }

    /// Um menu da barra superior está aberto na tela (a barra continua revelada).
    fn bar_menu_open(&self, output: &Output) -> bool {
        let source = self.layer_output(output);
        layer_map_for_output(&source).layers().any(|l| l.namespace() == "mide-bar-menu")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn barra_desce_e_sobe() {
        let now = Instant::now();
        let mut r = Reveal::default();
        assert_eq!(r.progress("A", now), 0.0);
        r.output = Some("A".into());
        r.shown = true;
        r.changed = None;
        assert_eq!(r.progress("A", now), 1.0);
        assert_eq!(r.progress("B", now), 0.0);
        r.changed = Some(now);
        assert!(r.progress("A", now) < 0.01);
        assert!(r.animating(now));
        assert!(r.progress("A", now + SLIDE) > 0.99);
        r.shown = false;
        r.changed = Some(now);
        assert!(r.progress("A", now + SLIDE) < 0.01);
        assert!(!r.animating(now + SLIDE));
    }
}
