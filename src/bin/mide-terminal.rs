//! Terminal do mide: shells numa janela do mide-ui, com o emulador próprio do mide
//! (`mide::terminal`). Inspirado no iTerm2 (sem ser cópia dele): abas, painéis
//! divididos, busca no histórico, perfis com Preferências, seleção por palavra e
//! por linha, saltos entre prompts e o texto que se reorganiza ao mudar a largura.
//!
//! Uso: `mide-terminal [-D pasta] [--title título] [--profile nome] [-e programa
//! argumentos...]`, ou `mide-terminal --preferences` para a janela de Preferências.
//!
//! Atalhos: os do Mac com ⌘ (a tecla Super) ou os do Linux com Ctrl+Shift:
//! ⌘T nova aba, ⌘W fechar painel, ⌘D dividir lado a lado, ⌘⇧D (Ctrl+Shift+E) em
//! cima e embaixo, ⌘⌥ setas trocar de painel, ⌘⇧Enter ampliar o painel, ⌘1…9
//! (Alt+1…9) ir à aba, Ctrl+PgUp/PgDn aba anterior/seguinte, ⌘F buscar, ⌘G/⌘⇧G
//! próxima/anterior, ⌘K limpar, ⌘C/⌘V copiar e colar, ⌘+/⌘−/⌘0 fonte, ⌘, Preferências,
//! ⌘⇧↑/↓ prompt anterior/seguinte, Shift+PgUp/PgDn/Home/End no histórico.

use std::{
    collections::HashMap,
    error::Error,
    path::PathBuf,
    sync::mpsc,
    time::{Duration, Instant, SystemTime},
};

use mide::{
    kit::{run, App, Key, KeyPress, Ui, Waker, Window},
    terminal::{
        keys::{key_bytes, paste_bytes},
        layout::{Direction, Divider, Layout, PaneId, Rect, Split},
        palette::{self, THEMES},
        profile::{self, Cursor, Profile, Profiles, StartDir},
        pty::{Pty, Size, Spawn},
        screen::{attr, CursorShape, MouseButton, MouseEvent, MouseMode, Rgb, Wide},
        Terminal,
    },
    ui::text::{FontWeight, Text},
};
use tiny_skia::{Paint, Pixmap, Rect as SkRect, Transform};

/// Margem em volta do texto de cada painel.
const PAD: f32 = 6.0;
const TITLE: &str = "Terminal";
/// Saída processada por quadro em cada painel (o resto fica para o próximo).
const CHUNK: usize = 4 << 20;
const TAB_BAR: f32 = 30.0;
const FIND_BAR: f32 = 40.0;

/// Linha de comando.
#[derive(Debug, Default)]
struct Options {
    spawn: Spawn,
    title: Option<String>,
    profile: Option<String>,
    preferences: bool,
}

impl Options {
    fn parse(args: impl IntoIterator<Item = String>) -> Self {
        let mut options = Options::default();
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "-e" => {
                    let rest = args.by_ref();
                    if let Some(program) = rest.next() {
                        options.spawn.command = Some((program, rest.collect()));
                    }
                }
                "-D" | "--working-directory" => options.spawn.directory = args.next().map(PathBuf::from),
                "--title" | "-T" => options.title = args.next(),
                "--profile" => options.profile = args.next(),
                "--preferences" => options.preferences = true,
                _ => {
                    if let Some(dir) = arg.strip_prefix("--working-directory=") {
                        options.spawn.directory = Some(PathBuf::from(dir));
                    }
                }
            }
        }
        options
    }
}

// ---- Painel: um shell ----

/// Como a seleção cresce ao arrastar: por caractere, palavra ou linha.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Unit {
    Char,
    Word,
    Line,
}

/// Seleção em números absolutos de linha (não muda quando chega saída nova).
#[derive(Debug, Clone, Copy)]
struct Selection {
    anchor: (u64, usize),
    head: (u64, usize),
    unit: Unit,
}

impl Selection {
    fn range(&self, term: &Terminal) -> ((u64, usize), (u64, usize)) {
        let (a, b) = if self.anchor <= self.head { (self.anchor, self.head) } else { (self.head, self.anchor) };
        match self.unit {
            Unit::Char => (a, b),
            Unit::Word => (term.word_at(a.0, a.1).map_or(a, |w| w.0), term.word_at(b.0, b.1).map_or(b, |w| w.1)),
            Unit::Line => (term.line_bounds(a.0).map_or(a, |l| l.0), term.line_bounds(b.0).map_or(b, |l| l.1)),
        }
    }
}

struct Pane {
    term: Terminal,
    pty: Pty,
    output: mpsc::Receiver<Vec<u8>>,
    profile: String,
    /// Linhas roladas para cima no histórico (0 = no fim).
    scroll: usize,
    scroll_rest: f32,
    selection: Option<Selection>,
    /// Botão do mouse informado ao programa e a última célula informada.
    mouse_held: Option<MouseButton>,
    mouse_cell: Option<(usize, usize)>,
    /// Quando o sino tocou (para o aviso visual).
    bell_at: Option<Instant>,
}

impl Pane {
    /// Saída nova. Devolve (chegou algo, o programa terminou).
    fn read(&mut self, ui: &mut Ui, copy: &mut Option<String>) -> (bool, bool) {
        let history_before = self.term.history_len();
        let (mut read, mut ended) = (0, false);
        while read < CHUNK {
            match self.output.try_recv() {
                Ok(chunk) if chunk.is_empty() => {
                    ended = true;
                    break;
                }
                Ok(chunk) => {
                    read += chunk.len();
                    self.term.feed(&chunk);
                }
                Err(_) => break,
            }
        }
        if read >= CHUNK {
            ui.request_tick();
        }
        self.pty.write(self.term.take_responses());
        if let Some(text) = self.term.take_clipboard() {
            *copy = Some(text);
        }
        if std::mem::take(&mut self.term.bell) {
            self.bell_at = Some(Instant::now());
        }
        if self.scroll > 0 {
            self.scroll = (self.scroll + self.term.history_len().saturating_sub(history_before)).min(self.term.history_len());
        }
        if ended {
            self.pty.reap();
        }
        (read > 0, ended)
    }

    /// Título da aba: o que o programa pediu, ou o nome do programa em primeiro plano.
    fn title(&self) -> String {
        if let Some(t) = self.term.title.as_ref().filter(|t| !t.trim().is_empty()) {
            return t.clone();
        }
        self.pty.foreground().map(|(name, _)| name).unwrap_or_else(|| TITLE.into())
    }

    fn selected_text(&self) -> Option<String> {
        let (a, b) = self.selection?.range(&self.term);
        Some(self.term.text_between(a, b)).filter(|t| !t.is_empty())
    }

    fn apply(&mut self, profile: &Profile) {
        self.term.theme = palette::theme(&profile.theme);
        self.term.bold_bright = profile.bold_bright;
        self.term.set_history_limit(profile.scrollback);
        self.term.cursor_shape = match profile.cursor {
            Cursor::Block => CursorShape::Block,
            Cursor::Bar => CursorShape::Bar,
            Cursor::Underline => CursorShape::Underline,
        };
        self.profile = profile.name.clone();
    }
}

struct Tab {
    layout: Layout,
    focus: PaneId,
    /// Painel em foco ampliado na aba inteira.
    zoom: bool,
    /// Saída nova enquanto a aba estava em segundo plano.
    activity: bool,
}

struct Find {
    query: String,
    matches: Vec<((u64, usize), (u64, usize))>,
    current: usize,
    /// Consulta da última busca (para refazer só quando muda).
    searched: Option<(String, PaneId)>,
    focus: bool,
}

enum Dialog {
    /// Fechar com programas rodando: o que fechar e os nomes dos programas.
    ConfirmClose(Closing, Vec<String>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Closing {
    Window,
    Tab(usize),
    Pane(PaneId),
}

struct TerminalApp {
    options: Options,
    profiles: Profiles,
    profiles_mtime: Option<SystemTime>,
    profiles_checked: Instant,
    panes: HashMap<PaneId, Pane>,
    next_id: PaneId,
    tabs: Vec<Tab>,
    active: usize,
    font_size: f32,
    cell: (f32, f32),
    find: Option<Find>,
    dialog: Option<Dialog>,
    /// Arrastando para selecionar neste painel.
    selecting: Option<PaneId>,
    last_click: Option<(Instant, (u64, usize), u8)>,
    /// Divisor sendo arrastado.
    dragging: Option<Divider>,
    title: String,
    error: Option<String>,
    waker: Option<Waker>,
    /// Fechar sem perguntar (já confirmado).
    confirmed_close: bool,
    /// Primeiro arquivo de ajustes a abrir na primeira aba.
    first_spawn: Option<Spawn>,
}

impl TerminalApp {
    fn new(options: Options) -> Self {
        let profiles = Profiles::load();
        let font_size = options.profile.as_deref().map_or_else(|| profiles.default_profile().font_size, |p| profiles.get(p).font_size);
        let first_spawn = Some(options.spawn.clone());
        Self {
            options,
            profiles,
            profiles_mtime: profile::path().and_then(|p| std::fs::metadata(p).ok()).and_then(|m| m.modified().ok()),
            profiles_checked: Instant::now(),
            panes: HashMap::new(),
            next_id: 1,
            tabs: Vec::new(),
            active: 0,
            font_size,
            cell: cell_size(&mut Text::new(), font_size),
            find: None,
            dialog: None,
            selecting: None,
            last_click: None,
            dragging: None,
            title: TITLE.into(),
            error: None,
            waker: None,
            confirmed_close: false,
            first_spawn,
        }
    }

    fn profile_name(&self) -> String {
        self.options.profile.clone().filter(|p| self.profiles.profiles.iter().any(|x| &x.name == p)).unwrap_or_else(|| self.profiles.default.clone())
    }

    /// Abre um shell num painel novo (`near`: painel cuja pasta a nova herda).
    fn spawn_pane(&mut self, near: Option<PaneId>) -> Option<PaneId> {
        let profile = self.profiles.get(&self.profile_name()).clone();
        let mut spawn = self.first_spawn.take().unwrap_or_default();
        if spawn.command.is_none() && !profile.command.trim().is_empty() {
            spawn.command = Some(("sh".into(), vec!["-c".into(), profile.command.clone()]));
        }
        if spawn.directory.is_none() && profile.start_dir == StartDir::Same {
            spawn.directory = near.and_then(|id| self.panes.get(&id)).and_then(|p| p.pty.cwd());
        }
        let (tx, output) = mpsc::channel();
        let waker = self.waker.clone();
        let wake = move || {
            if let Some(w) = &waker {
                w.wake();
            }
        };
        let (cols, rows) = (80, 24);
        let size = Size { cols, rows, width: (cols as f32 * self.cell.0) as u16, height: (rows as f32 * self.cell.1) as u16 };
        match Pty::spawn(&spawn, size, tx, wake) {
            Ok(pty) => {
                let mut term = Terminal::new(cols as usize, rows as usize);
                term.cell_pixels = (self.cell.0 as u16, self.cell.1 as u16);
                let mut pane = Pane { term, pty, output, profile: profile.name.clone(), scroll: 0, scroll_rest: 0.0, selection: None, mouse_held: None, mouse_cell: None, bell_at: None };
                pane.apply(&profile);
                let id = self.next_id;
                self.next_id += 1;
                self.panes.insert(id, pane);
                Some(id)
            }
            Err(err) => {
                self.error = Some(format!("Não foi possível abrir o shell: {err}"));
                None
            }
        }
    }

    fn focused(&self) -> Option<PaneId> {
        self.tabs.get(self.active).map(|t| t.focus)
    }

    fn new_tab(&mut self) {
        let near = self.focused();
        if let Some(id) = self.spawn_pane(near) {
            self.tabs.push(Tab { layout: Layout::Pane(id), focus: id, zoom: false, activity: false });
            self.active = self.tabs.len() - 1;
        }
    }

    fn split(&mut self, dir: Split) {
        let Some(focus) = self.focused() else {
            return;
        };
        if let Some(id) = self.spawn_pane(Some(focus)) {
            let tab = &mut self.tabs[self.active];
            tab.layout.split(focus, id, dir);
            tab.focus = id;
            tab.zoom = false;
        }
    }

    /// Tira o painel (o programa dele já terminou ou foi encerrado).
    fn remove_pane(&mut self, id: PaneId) {
        self.panes.remove(&id);
        let Some(t) = self.tabs.iter().position(|t| t.layout.panes().contains(&id)) else {
            return;
        };
        let tab = &mut self.tabs[t];
        if tab.layout == Layout::Pane(id) {
            self.tabs.remove(t);
            if self.active > t || self.active >= self.tabs.len() {
                self.active = self.active.saturating_sub(1);
            }
        } else {
            tab.layout.remove(id);
            if tab.focus == id {
                tab.focus = tab.layout.panes()[0];
            }
            tab.zoom = false;
        }
        if self.selecting == Some(id) {
            self.selecting = None;
        }
    }

    /// Programas rodando (além do shell) nos painéis.
    fn running(&self, panes: &[PaneId]) -> Vec<String> {
        panes.iter().filter_map(|id| self.panes.get(id)).filter_map(|p| p.pty.foreground()).filter(|(_, other)| *other).map(|(name, _)| name).collect()
    }

    /// Fecha (ou pergunta antes, se há programas rodando).
    fn close(&mut self, what: Closing, ui: &mut Ui) {
        let panes = match what {
            Closing::Window => self.panes.keys().copied().collect(),
            Closing::Tab(i) => self.tabs.get(i).map(|t| t.layout.panes()).unwrap_or_default(),
            Closing::Pane(id) => vec![id],
        };
        let running = self.running(&panes);
        if !running.is_empty() {
            self.dialog = Some(Dialog::ConfirmClose(what, running));
            return;
        }
        self.close_now(what, ui);
    }

    fn close_now(&mut self, what: Closing, ui: &mut Ui) {
        match what {
            Closing::Window => {
                self.confirmed_close = true;
                ui.close();
            }
            Closing::Tab(i) => {
                for id in self.tabs.get(i).map(|t| t.layout.panes()).unwrap_or_default() {
                    self.remove_pane(id);
                }
            }
            Closing::Pane(id) => self.remove_pane(id),
        }
        if self.tabs.is_empty() {
            self.confirmed_close = true;
            ui.close();
        }
    }

    fn set_font(&mut self, size: f32) {
        self.font_size = size.clamp(8.0, 32.0);
        self.cell = cell_size(&mut Text::new(), self.font_size);
        for pane in self.panes.values_mut() {
            pane.term.cell_pixels = (self.cell.0 as u16, self.cell.1 as u16);
        }
    }

    /// Releu os perfis se as Preferências gravaram (uma consulta por segundo).
    fn reload_profiles(&mut self, ui: &mut Ui) {
        ui.wake_after(Duration::from_secs(1));
        if self.profiles_checked.elapsed() < Duration::from_millis(900) {
            return;
        }
        self.profiles_checked = Instant::now();
        let mtime = profile::path().and_then(|p| std::fs::metadata(p).ok()).and_then(|m| m.modified().ok());
        if mtime == self.profiles_mtime {
            return;
        }
        self.profiles_mtime = mtime;
        let old_size = self.profiles.get(&self.profile_name()).font_size;
        self.profiles = Profiles::load();
        for pane in self.panes.values_mut() {
            let profile = self.profiles.get(&pane.profile).clone();
            pane.apply(&profile);
        }
        let new_size = self.profiles.get(&self.profile_name()).font_size;
        if new_size != old_size {
            self.set_font(new_size);
        }
    }

    // ---- Áreas ----

    fn areas(&self, (w, h): (f32, f32)) -> (Option<Rect>, Option<Rect>, Rect) {
        let mut y = 0.0;
        let tab_bar = (self.tabs.len() > 1).then(|| {
            y += TAB_BAR;
            (0.0, 0.0, w, TAB_BAR)
        });
        let find_bar = self.find.is_some().then(|| {
            let r = (0.0, y, w, FIND_BAR);
            y += FIND_BAR;
            r
        });
        (tab_bar, find_bar, (0.0, y, w, (h - y).max(1.0)))
    }

    /// Painéis da aba ativa com as áreas.
    fn pane_rects(&self, area: Rect) -> Vec<(PaneId, Rect)> {
        let Some(tab) = self.tabs.get(self.active) else {
            return Vec::new();
        };
        if tab.zoom {
            return vec![(tab.focus, area)];
        }
        tab.layout.rects(area)
    }

    fn grid_for(&self, rect: Rect) -> (usize, usize) {
        ((((rect.2 - 2.0 * PAD) / self.cell.0) as usize).max(2), (((rect.3 - 2.0 * PAD) / self.cell.1) as usize).max(1))
    }

    fn cell_at(&self, rect: Rect, pane: &Pane, (x, y): (f32, f32)) -> ((usize, usize), (u64, usize)) {
        let col = (((x - rect.0 - PAD) / self.cell.0).max(0.0) as usize).min(pane.term.cols() - 1);
        let row = (((y - rect.1 - PAD) / self.cell.1).max(0.0) as usize).min(pane.term.rows() - 1);
        ((col, row), (pane.term.visible_abs(row, pane.scroll), col))
    }

    // ---- Ações ----

    fn action(&mut self, ui: &mut Ui, action: Action, area: Rect) {
        let focus = self.focused();
        match action {
            Action::Copy => {
                if let Some(text) = focus.and_then(|id| self.panes.get(&id)).and_then(Pane::selected_text) {
                    ui.copy(&text);
                }
            }
            Action::Paste => ui.request_paste(),
            Action::NewTab => self.new_tab(),
            Action::ClosePane => {
                if let Some(id) = focus {
                    self.close(Closing::Pane(id), ui);
                }
            }
            Action::SplitSide => self.split(Split::Side),
            Action::SplitStack => self.split(Split::Stack),
            Action::Find => {
                let find = self.find.get_or_insert(Find { query: String::new(), matches: Vec::new(), current: 0, searched: None, focus: true });
                find.focus = true;
            }
            Action::FindNext | Action::FindPrevious => self.find_step(action == Action::FindNext),
            Action::Clear => {
                if let Some(pane) = focus.and_then(|id| self.panes.get_mut(&id)) {
                    // Como o "Limpar" do iTerm2: apaga o histórico e o shell redesenha o prompt.
                    pane.term.feed(b"\x1b[3J");
                    pane.scroll = 0;
                    pane.selection = None;
                    pane.pty.write(b"\x0c".to_vec());
                }
            }
            Action::Preferences => {
                if let Ok(exe) = std::env::current_exe() {
                    let _ = std::process::Command::new(exe).arg("--preferences").spawn();
                }
            }
            Action::FontBigger => self.set_font(self.font_size + 1.0),
            Action::FontSmaller => self.set_font(self.font_size - 1.0),
            Action::FontReset => {
                let size = self.profiles.get(&self.profile_name()).font_size;
                self.set_font(size);
            }
            Action::Zoom => {
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    tab.zoom = !tab.zoom && tab.layout.panes().len() > 1;
                }
            }
            Action::Tab(n) => {
                if n < self.tabs.len() {
                    self.active = n;
                }
            }
            Action::NextTab => self.active = (self.active + 1) % self.tabs.len().max(1),
            Action::PreviousTab => self.active = (self.active + self.tabs.len().max(1) - 1) % self.tabs.len().max(1),
            Action::PaneDirection(direction) => {
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    if let Some(next) = tab.layout.neighbor(area, tab.focus, direction) {
                        tab.focus = next;
                        tab.zoom = false;
                    }
                }
            }
            Action::NextPane | Action::PreviousPane => {
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    let panes = tab.layout.panes();
                    let i = panes.iter().position(|&p| p == tab.focus).unwrap_or(0);
                    let n = panes.len();
                    tab.focus = panes[if action == Action::NextPane { (i + 1) % n } else { (i + n - 1) % n }];
                    tab.zoom = false;
                }
            }
            Action::PageUp | Action::PageDown | Action::Top | Action::Bottom | Action::PreviousPrompt | Action::NextPrompt => {
                if let Some(pane) = focus.and_then(|id| self.panes.get_mut(&id)) {
                    scroll_action(pane, action);
                }
            }
        }
    }

    // ---- Busca ----

    fn find_refresh(&mut self) {
        let Some(focus) = self.focused() else {
            return;
        };
        let Some(find) = &mut self.find else {
            return;
        };
        let key = (find.query.clone(), focus);
        if find.searched.as_ref() == Some(&key) {
            return;
        }
        let Some(pane) = self.panes.get_mut(&focus) else {
            return;
        };
        find.matches = pane.term.search(&find.query);
        // Começa pela ocorrência mais recente (a última), como o iTerm2.
        find.current = find.matches.len().saturating_sub(1);
        find.searched = Some(key);
        if let Some(&(start, _)) = find.matches.get(find.current) {
            reveal(pane, start.0);
        }
    }

    fn find_step(&mut self, forward: bool) {
        self.find_refresh();
        let Some(focus) = self.focused() else {
            return;
        };
        let (Some(find), Some(pane)) = (&mut self.find, self.panes.get_mut(&focus)) else {
            return;
        };
        let n = find.matches.len();
        if n == 0 {
            return;
        }
        find.current = if forward { (find.current + 1) % n } else { (find.current + n - 1) % n };
        reveal(pane, find.matches[find.current].0 .0);
    }

    // ---- Quadro ----

    fn keyboard(&mut self, ui: &mut Ui, area: Rect) {
        let input = ui.input().clone();
        let typing = ui.typing();
        for press in &input.presses {
            if let Some(action) = shortcut(press) {
                self.action(ui, action, area);
                continue;
            }
            if self.dialog.is_some() {
                continue;
            }
            if self.find.is_some() && typing {
                // Na busca: Enter vai à próxima (Shift, à anterior), Esc fecha.
                match press.keysym {
                    0xff0d | 0xff8d => self.find_step(!press.shift),
                    0xff1b => self.find = None,
                    _ => {}
                }
                continue;
            }
            if press.keysym == 0xff1b && self.find.is_some() && !self.panes.is_empty() && press.text.is_none() {
                self.find = None;
            }
            let Some(pane) = self.focused().and_then(|id| self.panes.get_mut(&id)) else {
                return;
            };
            if let Some(bytes) = key_bytes(press, &pane.term.modes) {
                pane.scroll = 0;
                pane.selection = None;
                pane.pty.write(bytes);
            }
        }
        if let Some(text) = &input.paste {
            if !typing && self.dialog.is_none() {
                if let Some(pane) = self.focused().and_then(|id| self.panes.get_mut(&id)) {
                    pane.scroll = 0;
                    pane.pty.write(paste_bytes(text, pane.term.modes.bracketed_paste));
                }
            }
        }
    }

    fn mouse(&mut self, ui: &mut Ui, area: Rect) {
        if self.dialog.is_some() {
            return;
        }
        let input = ui.input().clone();
        let rects = self.pane_rects(area);
        // Divisores: arrastar muda a proporção.
        if let Some(tab) = self.tabs.get(self.active).filter(|t| !t.zoom) {
            let dividers = tab.layout.dividers(area);
            if input.pressed && self.dragging.is_none() {
                if let Some(pos) = input.pointer {
                    self.dragging = dividers.into_iter().find(|d| {
                        let r = d.rect;
                        pos.0 >= r.0 - 3.0 && pos.0 <= r.0 + r.2 + 3.0 && pos.1 >= r.1 - 3.0 && pos.1 <= r.1 + r.3 + 3.0
                    });
                }
            }
        }
        if let Some(divider) = self.dragging.clone() {
            if let (Some(pos), true) = (input.pointer, input.down) {
                let a = divider.area;
                let ratio = match divider.dir {
                    Split::Side => (pos.0 - a.0) / a.2.max(1.0),
                    Split::Stack => (pos.1 - a.1) / a.3.max(1.0),
                };
                self.tabs[self.active].layout.set_ratio(&divider.path, ratio);
            }
            if input.released || !input.down {
                self.dragging = None;
            }
            return;
        }

        let mut menu_action = None;
        for (id, rect) in rects {
            let response = ui.background(rect);
            let Some(pane) = self.panes.get(&id) else {
                continue;
            };
            if response.clicked || response.secondary_clicked {
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    tab.focus = id;
                }
            }
            let mods = (input.shift, input.alt, input.ctrl);
            let captured = pane.term.modes.mouse != MouseMode::Off && !input.shift;
            if captured {
                if let Some(pos) = input.pointer.filter(|_| response.hovered || pane.mouse_held.is_some()) {
                    let ((col, row), _) = self.cell_at(rect, pane, pos);
                    let pane = self.panes.get_mut(&id).unwrap();
                    let mut reports = Vec::new();
                    if response.hovered && input.scroll != 0.0 {
                        let button = if input.scroll < 0.0 { MouseButton::WheelUp } else { MouseButton::WheelDown };
                        reports.push((button, MouseEvent::Press));
                    }
                    if response.clicked {
                        pane.mouse_held = Some(MouseButton::Left);
                        reports.push((MouseButton::Left, MouseEvent::Press));
                    }
                    if response.secondary_clicked {
                        reports.push((MouseButton::Right, MouseEvent::Press));
                        reports.push((MouseButton::Right, MouseEvent::Release));
                    }
                    if pane.mouse_cell != Some((col, row)) {
                        reports.push((pane.mouse_held.unwrap_or(MouseButton::None), MouseEvent::Motion));
                    }
                    if input.released {
                        if let Some(button) = pane.mouse_held.take() {
                            reports.push((button, MouseEvent::Release));
                        }
                    }
                    pane.mouse_cell = Some((col, row));
                    let bytes: Vec<u8> = reports.into_iter().filter_map(|(b, e)| pane.term.mouse_report(b, e, col, row, mods)).flatten().collect();
                    pane.pty.write(bytes);
                }
                continue;
            }

            // Rolagem: no histórico, ou setas para programas de tela cheia (less, man).
            if response.hovered && input.scroll != 0.0 {
                let cell_h = self.cell.1;
                let pane = self.panes.get_mut(&id).unwrap();
                pane.scroll_rest += input.scroll / cell_h;
                let lines = pane.scroll_rest.trunc() as i32;
                pane.scroll_rest -= lines as f32;
                if lines != 0 {
                    if pane.term.alt_screen() {
                        let arrow: &[u8] = if lines > 0 { b"\x1bOB" } else { b"\x1bOA" };
                        pane.pty.write(arrow.repeat(lines.unsigned_abs() as usize));
                    } else {
                        let history = pane.term.history_len() as i64;
                        pane.scroll = (pane.scroll as i64 - lines as i64).clamp(0, history) as usize;
                    }
                }
            }

            // Seleção: arrastar; duplo clique pega a palavra e triplo, a linha.
            if let Some(pos) = input.pointer {
                let (_, at) = self.cell_at(rect, &self.panes[&id], pos);
                if response.clicked {
                    let now = Instant::now();
                    let count = match self.last_click {
                        Some((when, (line, col), n)) if now - when < Duration::from_millis(450) && line == at.0 && col.abs_diff(at.1) <= 1 => n % 3 + 1,
                        _ => 1,
                    };
                    self.last_click = Some((now, at, count));
                    let unit = [Unit::Char, Unit::Word, Unit::Line][count as usize - 1];
                    for p in self.panes.values_mut() {
                        p.selection = None;
                    }
                    self.panes.get_mut(&id).unwrap().selection = Some(Selection { anchor: at, head: at, unit });
                    self.selecting = Some(id);
                } else if self.selecting == Some(id) && input.down {
                    if let Some(selection) = &mut self.panes.get_mut(&id).unwrap().selection {
                        selection.head = at;
                    }
                }
            }
            if input.released && self.selecting == Some(id) {
                self.selecting = None;
                let pane = self.panes.get_mut(&id).unwrap();
                if pane.selection.is_some_and(|s| s.unit == Unit::Char && s.anchor == s.head) {
                    pane.selection = None;
                }
            }

            let has_selection = self.panes.get(&id).is_some_and(|p| p.selection.is_some());
            let several = self.tabs.get(self.active).is_some_and(|t| t.layout.panes().len() > 1);
            ui.context_menu(&response, |menu| {
                if has_selection && menu.item("Copiar") {
                    menu_action = Some(Action::Copy);
                }
                if menu.item("Colar") {
                    menu_action = Some(Action::Paste);
                }
                menu.separator();
                if menu.item("Dividir lado a lado") {
                    menu_action = Some(Action::SplitSide);
                }
                if menu.item("Dividir em cima e embaixo") {
                    menu_action = Some(Action::SplitStack);
                }
                if several && menu.item("Ampliar painel") {
                    menu_action = Some(Action::Zoom);
                }
                if menu.item("Nova aba") {
                    menu_action = Some(Action::NewTab);
                }
                menu.separator();
                if menu.item("Buscar…") {
                    menu_action = Some(Action::Find);
                }
                if menu.item("Limpar") {
                    menu_action = Some(Action::Clear);
                }
                if menu.item("Preferências…") {
                    menu_action = Some(Action::Preferences);
                }
                menu.separator();
                if menu.danger("Fechar painel") {
                    menu_action = Some(Action::ClosePane);
                }
            });
        }
        if let Some(action) = menu_action {
            self.action(ui, action, area);
        }
    }

    fn tab_bar(&mut self, ui: &mut Ui, rect: Rect) {
        let n = self.tabs.len();
        let plus = 34.0;
        let width = ((rect.2 - plus) / n as f32).min(240.0);
        let titles: Vec<String> = self.tabs.iter().map(|t| self.panes.get(&t.focus).map(Pane::title).unwrap_or_default()).collect();
        let mut clicked = None;
        let mut close = None;
        let mut hovered = None;
        for i in 0..n {
            let r = (rect.0 + i as f32 * width, rect.1, width, rect.3);
            let response = ui.background(r);
            if response.hovered {
                hovered = Some(i);
            }
            let over_x = ui.input().pointer.is_some_and(|(px, _)| px < r.0 + 26.0);
            if response.clicked {
                if over_x && n > 1 {
                    close = Some(i);
                } else {
                    clicked = Some(i);
                }
            }
        }
        let new_tab = ui.background((rect.0 + n as f32 * width, rect.1, plus, rect.3)).clicked;
        let (active, activity): (usize, Vec<bool>) = (self.active, self.tabs.iter().map(|t| t.activity).collect());
        ui.paint_text(|pixmap, text, (ox, oy)| {
            let (x0, y0) = (ox + rect.0, oy + rect.1);
            fill(pixmap, x0, y0, rect.2, rect.3, [222, 222, 227]);
            fill(pixmap, x0, y0 + rect.3 - 1.0, rect.2, 1.0, [200, 200, 206]);
            for (i, title) in titles.iter().enumerate() {
                let x = x0 + i as f32 * width;
                if i == active {
                    fill(pixmap, x, y0, width, rect.3, [246, 246, 248]);
                } else if hovered == Some(i) {
                    fill(pixmap, x, y0, width, rect.3 - 1.0, [212, 212, 218]);
                }
                if i > 0 {
                    fill(pixmap, x, y0 + 6.0, 1.0, rect.3 - 12.0, [196, 196, 204]);
                }
                if hovered == Some(i) && n > 1 {
                    text.draw(pixmap, "×", x + 9.0, y0 + 5.0, 15.0, FontWeight::Regular, [90, 90, 98, 255]);
                }
                let size = 12.5;
                let label = fit(text, title, size, width - 52.0);
                let tw = text.measure(&label, size, FontWeight::Regular);
                let color = if i == active { [24, 24, 28, 255] } else { [90, 90, 98, 255] };
                text.draw(pixmap, &label, x + (width - tw) / 2.0, y0 + (rect.3 - size * 1.25) / 2.0, size, FontWeight::Regular, color);
                if activity[i] && i != active {
                    fill_round(pixmap, x + width - 16.0, y0 + rect.3 / 2.0 - 3.0, 6.0, [10, 110, 235]);
                }
            }
            let px = x0 + n as f32 * width;
            text.draw(pixmap, "+", px + 11.0, y0 + 4.0, 17.0, FontWeight::Regular, [90, 90, 98, 255]);
        });
        if let Some(i) = close {
            self.close(Closing::Tab(i), ui);
        } else if let Some(i) = clicked {
            self.active = i;
        }
        if new_tab {
            self.new_tab();
        }
    }

    fn find_bar(&mut self, ui: &mut Ui, rect: Rect) {
        let Some(find) = &mut self.find else {
            return;
        };
        let count = if find.query.is_empty() {
            String::new()
        } else if find.matches.is_empty() {
            "Nada encontrado".into()
        } else {
            format!("{} de {}", find.current + 1, find.matches.len())
        };
        ui.paint(|pixmap, (ox, oy)| {
            fill(pixmap, ox + rect.0, oy + rect.1, rect.2, rect.3, [236, 236, 240]);
            fill(pixmap, ox + rect.0, oy + rect.1 + rect.3 - 1.0, rect.2, 1.0, [205, 205, 212]);
        });
        let (mut previous, mut next, mut done) = (false, false, false);
        let focus = std::mem::take(&mut find.focus);
        ui.at(rect.0 + 10.0, rect.1 + 4.0, rect.2 - 20.0, |ui| {
            ui.row(|ui| {
                if focus {
                    ui.focus_next_input();
                }
                ui.sized(260.0, |ui| {
                    let r = ui.text_input(&mut find.query, "Buscar no terminal");
                    if r.submitted() {
                        next = true;
                    }
                });
                ui.sized(120.0, |ui| ui.note(&count));
                previous = ui.icon_button("go-up-symbolic", "‹", true).clicked();
                next |= ui.icon_button("go-down-symbolic", "›", true).clicked();
                ui.push_right(100.0);
                done = ui.button("Concluído").clicked();
            });
        });
        if done {
            self.find = None;
            return;
        }
        self.find_refresh();
        if previous || next {
            self.find_step(next && !previous);
        }
    }

    fn draw(&self, ui: &mut Ui, area: Rect) {
        let rects = self.pane_rects(area);
        let several = rects.len() > 1;
        let focus = self.focused();
        let find = self.find.as_ref();
        let cell = self.cell;
        let font = self.font_size;
        let blink_off = self.profiles.get(&self.profile_name()).cursor_blink
            && (SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_millis()) / 530) % 2 == 1;
        if self.profiles.get(&self.profile_name()).cursor_blink {
            ui.wake_after(Duration::from_millis(530));
        }
        let mut flashing = false;
        let dividers = self.tabs.get(self.active).filter(|t| !t.zoom).map(|t| t.layout.dividers(area)).unwrap_or_default();
        ui.paint_text(|pixmap, text, (ox, oy)| {
            for (id, rect) in &rects {
                let Some(pane) = self.panes.get(id) else {
                    continue;
                };
                let matches = find.filter(|_| Some(*id) == focus).map(|f| (f.matches.as_slice(), f.current));
                let flash = pane.bell_at.is_some_and(|t| t.elapsed() < Duration::from_millis(160));
                flashing |= flash;
                draw_pane(pixmap, text, pane, (ox + rect.0, oy + rect.1, rect.2, rect.3), cell, font, matches, Some(*id) == focus && !blink_off);
                if several && Some(*id) != focus {
                    // Painéis sem foco ficam um pouco apagados (como no iTerm2).
                    fill_alpha(pixmap, ox + rect.0, oy + rect.1, rect.2, rect.3, [128, 128, 136], 0.10);
                }
                if flash {
                    fill_alpha(pixmap, ox + rect.0, oy + rect.1, rect.2, rect.3, [128, 128, 136], 0.25);
                }
            }
            for d in &dividers {
                fill(pixmap, ox + d.rect.0, oy + d.rect.1, d.rect.2, d.rect.3, [190, 190, 198]);
            }
        });
        if flashing {
            ui.request_tick();
        }
    }

    fn dialog(&mut self, ui: &mut Ui) {
        let Some(Dialog::ConfirmClose(what, programs)) = &self.dialog else {
            return;
        };
        let (what, programs) = (*what, programs.clone());
        let (mut yes, mut no) = (false, false);
        ui.modal(380.0, |ui| {
            ui.title(match what {
                Closing::Window => "Fechar esta janela?",
                Closing::Tab(_) => "Fechar esta aba?",
                Closing::Pane(_) => "Fechar este painel?",
            });
            ui.label(&format!("Ainda há programas rodando: {}. Fechar encerra esses programas.", programs.join(", ")));
            ui.space(8.0);
            ui.row(|ui| {
                ui.push_right(200.0);
                no = ui.button("Cancelar").clicked();
                yes = ui.primary_button("Fechar").clicked();
            });
        });
        let keys = ui.input().keys.clone();
        no |= keys.contains(&Key::Escape);
        yes |= keys.contains(&Key::Enter);
        if yes {
            self.dialog = None;
            self.close_now(what, ui);
        } else if no {
            self.dialog = None;
        }
    }
}

impl App for TerminalApp {
    fn view(&mut self, ui: &mut Ui) {
        if self.waker.is_none() {
            self.waker = ui.waker();
        }
        if self.tabs.is_empty() && self.error.is_none() {
            if let Some(title) = &self.options.title {
                ui.set_title(title);
            }
            self.new_tab();
        }
        if let Some(error) = &self.error {
            if self.tabs.is_empty() {
                ui.page(|ui| ui.label(error));
                return;
            }
        }
        self.reload_profiles(ui);

        // Saída de todos os painéis (abas em segundo plano também).
        let mut copy = None;
        let mut ended = Vec::new();
        let active_panes = self.tabs.get(self.active).map(|t| t.layout.panes()).unwrap_or_default();
        for (id, pane) in &mut self.panes {
            let (got, end) = pane.read(ui, &mut copy);
            if end {
                ended.push(*id);
            } else if got && !active_panes.contains(id) {
                if let Some(tab) = self.tabs.iter_mut().find(|t| t.layout.panes().contains(id)) {
                    tab.activity = true;
                }
            }
        }
        if let Some(text) = copy {
            ui.copy(&text);
        }
        for id in ended {
            self.remove_pane(id);
        }
        if self.tabs.is_empty() {
            self.confirmed_close = true;
            ui.close();
            return;
        }
        if let Some(tab) = self.tabs.get_mut(self.active) {
            tab.activity = false;
        }

        let (_, _, w, h) = ui.region();
        let (_, _, area) = self.areas((w, h));
        // Painéis da aba ativa com o tamanho da área deles.
        for (id, rect) in self.pane_rects(area) {
            let grid = self.grid_for(rect);
            let cell = self.cell;
            if let Some(pane) = self.panes.get_mut(&id) {
                if (pane.term.cols(), pane.term.rows()) != grid {
                    pane.term.resize(grid.0, grid.1);
                    pane.pty.resize(Size { cols: grid.0 as u16, rows: grid.1 as u16, width: (grid.0 as f32 * cell.0) as u16, height: (grid.1 as f32 * cell.1) as u16 });
                    pane.selection = None;
                    pane.scroll = pane.scroll.min(pane.term.history_len());
                }
            }
        }

        // Título: o da aba ativa (ou o pedido na linha de comando).
        if self.options.title.is_none() {
            let title = self.focused().and_then(|id| self.panes.get(&id)).map(Pane::title).unwrap_or_else(|| TITLE.into());
            if title != self.title {
                ui.set_title(&title);
                self.title = title;
            }
        }

        self.keyboard(ui, area);
        if self.tabs.is_empty() {
            return;
        }
        // Um atalho pode ter aberto a busca ou uma aba: as áreas mudam.
        let (tab_bar, find_bar, area) = self.areas((w, h));
        self.draw(ui, area);
        self.mouse(ui, area);
        if let Some(rect) = tab_bar.filter(|_| self.tabs.len() > 1) {
            self.tab_bar(ui, rect);
        }
        if let Some(rect) = find_bar.filter(|_| self.find.is_some()) {
            self.find_bar(ui, rect);
        }
        self.dialog(ui);
    }

    fn close_requested(&mut self) -> bool {
        if self.confirmed_close {
            return true;
        }
        let all: Vec<PaneId> = self.panes.keys().copied().collect();
        let running = self.running(&all);
        if running.is_empty() {
            return true;
        }
        self.dialog = Some(Dialog::ConfirmClose(Closing::Window, running));
        false
    }
}

/// Rola o painel até a linha absoluta aparecer (no meio, se possível).
fn reveal(pane: &mut Pane, abs: u64) {
    let history = pane.term.history_len();
    let first = pane.term.visible_abs(0, history);
    let line = (abs.saturating_sub(first)) as usize;
    let rows = pane.term.rows();
    let top = line.saturating_sub(rows / 2);
    pane.scroll = history.saturating_sub(top).min(history);
}

fn scroll_action(pane: &mut Pane, action: Action) {
    let (rows, history) = (pane.term.rows(), pane.term.history_len());
    match action {
        Action::PageUp => pane.scroll = (pane.scroll + rows.saturating_sub(1).max(1)).min(history),
        Action::PageDown => pane.scroll = pane.scroll.saturating_sub(rows.saturating_sub(1).max(1)),
        Action::Top => pane.scroll = history,
        Action::Bottom => pane.scroll = 0,
        Action::PreviousPrompt | Action::NextPrompt => {
            let top = pane.term.visible_abs(0, pane.scroll);
            if let Some(mark) = pane.term.prompt_near(top, action == Action::NextPrompt) {
                let first = pane.term.visible_abs(0, history);
                pane.scroll = history.saturating_sub((mark - first) as usize).min(history);
            }
        }
        _ => {}
    }
}

/// Desenha um painel: fundo, células, seleção, ocorrências da busca e cursor.
#[allow(clippy::too_many_arguments)]
fn draw_pane(
    pixmap: &mut Pixmap,
    text: &mut Text,
    pane: &Pane,
    (x0, y0, w, h): Rect,
    cell: (f32, f32),
    font: f32,
    matches: Option<(&[((u64, usize), (u64, usize))], usize)>,
    show_cursor_here: bool,
) {
    let term = &pane.term;
    let scroll = pane.scroll.min(term.history_len());
    let background = term.default_bg();
    fill(pixmap, x0, y0, w, h, background);
    let selection = pane.selection.map(|s| s.range(term));
    let (cursor_row, cursor_col) = term.cursor();
    let show_cursor = term.modes.show_cursor && scroll == 0 && show_cursor_here;
    let top_abs = term.visible_abs(0, scroll);
    let bottom_abs = term.visible_abs(term.rows() - 1, scroll);
    let visible: Vec<(usize, ((u64, usize), (u64, usize)))> = matches
        .map(|(m, _)| m.iter().enumerate().filter(|(_, (a, b))| b.0 >= top_abs && a.0 <= bottom_abs).map(|(i, r)| (i, *r)).collect())
        .unwrap_or_default();
    let current = matches.map(|(_, c)| c);
    let (px0, py0) = (x0 + PAD, y0 + PAD);
    for y in 0..term.rows() {
        let row = term.visible_row(y, scroll);
        let abs = term.visible_abs(y, scroll);
        let py = py0 + y as f32 * cell.1;
        for (x, c) in row.cells.iter().enumerate() {
            if c.wide == Wide::Spacer {
                continue;
            }
            let px = px0 + x as f32 * cell.0;
            let width = if c.wide == Wide::Lead { 2.0 } else { 1.0 } * cell.0;
            let (mut fg, mut bg) = term.cell_colors(&c.pen);
            let at = (abs, x);
            if let Some((i, _)) = visible.iter().find(|(_, (a, b))| at >= *a && at <= *b) {
                bg = if Some(*i) == current { [255, 170, 60] } else { [255, 230, 140] };
                fg = [24, 24, 28];
            }
            if selection.is_some_and(|(a, b)| at >= a && at <= b) {
                bg = term.theme.selection;
            }
            let at_cursor = show_cursor && y == cursor_row && x == cursor_col;
            if at_cursor && term.cursor_shape == CursorShape::Block {
                bg = term.cursor_color();
                fg = background;
            }
            if bg != background {
                fill(pixmap, px, py, width, cell.1, bg);
            }
            if at_cursor {
                let color = term.cursor_color();
                match term.cursor_shape {
                    CursorShape::Bar => fill(pixmap, px, py, 2.0, cell.1, color),
                    CursorShape::Underline => fill(pixmap, px, py + cell.1 - 2.0, width, 2.0, color),
                    CursorShape::Block => {}
                }
            }
            if c.c != ' ' && c.pen.attr & attr::HIDDEN == 0 {
                let rgba = [fg[0], fg[1], fg[2], 255];
                let (bold, italic) = (c.pen.attr & attr::BOLD != 0, c.pen.attr & attr::ITALIC != 0);
                let mut buf = [0u8; 4];
                let s = if c.combining.is_some() { c.text() } else { c.c.encode_utf8(&mut buf).to_string() };
                text.draw_mono_styled(pixmap, &s, px, py + 1.0, font, rgba, bold, italic);
            }
            if c.pen.attr & (attr::UNDERLINE | attr::DOUBLE_UNDERLINE) != 0 {
                fill(pixmap, px, py + cell.1 - 2.0, width, 1.0, fg);
                if c.pen.attr & attr::DOUBLE_UNDERLINE != 0 {
                    fill(pixmap, px, py + cell.1 - 4.0, width, 1.0, fg);
                }
            }
            if c.pen.attr & attr::STRIKE != 0 {
                fill(pixmap, px, py + cell.1 / 2.0, width, 1.0, fg);
            }
        }
    }
    // Lendo o histórico: barra fina à direita mostrando onde está.
    let history = term.history_len();
    if scroll > 0 && history > 0 {
        let total = (history + term.rows()) as f32;
        let track = h - 8.0;
        let bar = (term.rows() as f32 / total * track).max(24.0);
        let top = (history - scroll) as f32 / (total - term.rows() as f32) * (track - bar);
        fill(pixmap, x0 + w - 7.0, y0 + 4.0 + top, 4.0, bar, [150, 150, 158]);
    }
}

/// Ações do terminal (atalhos e menus).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Copy,
    Paste,
    NewTab,
    ClosePane,
    SplitSide,
    SplitStack,
    Find,
    FindNext,
    FindPrevious,
    Clear,
    Preferences,
    FontBigger,
    FontSmaller,
    FontReset,
    Zoom,
    Tab(usize),
    NextTab,
    PreviousTab,
    PaneDirection(Direction),
    NextPane,
    PreviousPane,
    PageUp,
    PageDown,
    Top,
    Bottom,
    PreviousPrompt,
    NextPrompt,
}

/// Atalho do terminal para a tecla, nas duas formas: ⌘ (Super), como no Mac, ou
/// Ctrl+Shift, como nos terminais do Linux.
fn shortcut(press: &KeyPress) -> Option<Action> {
    let k = press.keysym;
    let ch = char::from_u32(k).map(|c| c.to_ascii_lowercase());
    let arrow = match k {
        0xff51 => Some(Direction::Left),
        0xff52 => Some(Direction::Up),
        0xff53 => Some(Direction::Right),
        0xff54 => Some(Direction::Down),
        _ => None,
    };
    let enter = k == 0xff0d || k == 0xff8d;

    // Sem ⌘ nem Ctrl: Shift+PgUp/PgDn/Home/End no histórico.
    if press.shift && !press.ctrl && !press.alt && !press.logo {
        return match k {
            0xff55 => Some(Action::PageUp),
            0xff56 => Some(Action::PageDown),
            0xff50 => Some(Action::Top),
            0xff57 => Some(Action::Bottom),
            _ => None,
        };
    }
    // Alt+1…9 (sem outros modificadores): ir à aba, como no GNOME Terminal.
    if press.alt && !press.ctrl && !press.shift && !press.logo {
        return ch.and_then(|c| c.to_digit(10)).filter(|d| *d >= 1).map(|d| Action::Tab(d as usize - 1));
    }
    // Ctrl+PgUp/PgDn (e Ctrl+Tab): aba anterior e seguinte.
    if press.ctrl && !press.logo && !press.alt {
        match k {
            0xff55 if !press.shift => return Some(Action::PreviousTab),
            0xff56 if !press.shift => return Some(Action::NextTab),
            0xff09 => return Some(Action::NextTab),
            0xfe20 => return Some(Action::PreviousTab),
            _ => {}
        }
    }

    let command = press.logo && !press.ctrl;
    let linux = press.ctrl && press.shift && !press.logo;
    if !command && !linux {
        return None;
    }
    if let Some(direction) = arrow {
        if press.alt {
            return Some(Action::PaneDirection(direction));
        }
        if (command && press.shift) || linux {
            return match direction {
                Direction::Up => Some(Action::PreviousPrompt),
                Direction::Down => Some(Action::NextPrompt),
                _ => None,
            };
        }
        return None;
    }
    if enter && (press.shift || linux) {
        return Some(Action::Zoom);
    }
    if command && !press.alt {
        if let Some(d) = ch.and_then(|c| c.to_digit(10)).filter(|d| *d >= 1) {
            return Some(Action::Tab(d as usize - 1));
        }
    }
    let shifted = command && press.shift;
    Some(match ch? {
        'c' => Action::Copy,
        'v' => Action::Paste,
        't' => Action::NewTab,
        'w' => Action::ClosePane,
        'd' if shifted => Action::SplitStack,
        'd' => Action::SplitSide,
        'e' if linux => Action::SplitStack,
        'f' => Action::Find,
        'g' if shifted => Action::FindPrevious,
        'g' => Action::FindNext,
        'k' => Action::Clear,
        ',' | '<' => Action::Preferences,
        '=' | '+' => Action::FontBigger,
        '-' | '_' => Action::FontSmaller,
        '0' | ')' => Action::FontReset,
        '[' | '{' if shifted || linux => Action::PreviousTab,
        ']' | '}' if shifted || linux => Action::NextTab,
        '[' => Action::PreviousPane,
        ']' => Action::NextPane,
        _ => return None,
    })
}

fn fill(pixmap: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, c: Rgb) {
    fill_alpha(pixmap, x, y, w, h, c, 1.0);
}

fn fill_alpha(pixmap: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, c: Rgb, alpha: f32) {
    if let Some(rect) = SkRect::from_xywh(x, y, w, h) {
        let mut paint = Paint::default();
        paint.set_color_rgba8(c[0], c[1], c[2], (alpha * 255.0) as u8);
        pixmap.fill_rect(rect, &paint, Transform::identity(), None);
    }
}

fn fill_round(pixmap: &mut Pixmap, x: f32, y: f32, d: f32, c: Rgb) {
    mide::ui::draw::fill_rounded(pixmap, x, y, d, d, d / 2.0, mide::ui::draw::rgba(c[0], c[1], c[2], 255));
}

/// Texto que cabe em `width` (com reticências).
fn fit(text: &mut Text, s: &str, size: f32, width: f32) -> String {
    if text.measure(s, size, FontWeight::Regular) <= width {
        return s.to_string();
    }
    let mut chars: Vec<char> = s.chars().collect();
    while !chars.is_empty() {
        chars.pop();
        let candidate: String = chars.iter().collect::<String>() + "…";
        if text.measure(&candidate, size, FontWeight::Regular) <= width {
            return candidate;
        }
    }
    String::new()
}

/// Tamanho de uma célula na fonte de largura fixa (largura média de 10 letras,
/// arredondada para as colunas caírem em pixels inteiros).
fn cell_size(text: &mut Text, font: f32) -> (f32, f32) {
    let width = text.measure_mono("MMMMMMMMMM", font) / 10.0;
    (width.max(1.0).round(), (font * 1.3).round())
}

// ---- Preferências ----

/// Janela de Preferências: perfis à esquerda, ajustes do perfil escolhido à direita.
/// Cada mudança é gravada na hora e os terminais abertos aplicam em até um segundo.
struct Preferences {
    profiles: Profiles,
    selected: usize,
    message: Option<String>,
}

const CURSORS: [(&str, Cursor); 3] = [("Bloco", Cursor::Block), ("Barra", Cursor::Bar), ("Sublinhado", Cursor::Underline)];
const SCROLLBACKS: [usize; 5] = [1_000, 5_000, 10_000, 50_000, 100_000];

impl Preferences {
    fn save(&mut self) {
        self.message = self.profiles.save().err().map(|e| format!("Não foi possível gravar: {e}"));
    }
}

impl App for Preferences {
    fn view(&mut self, ui: &mut Ui) {
        let names: Vec<String> = self.profiles.profiles.iter().map(|p| p.name.clone()).collect();
        let items: Vec<(&str, &str)> = names.iter().map(|n| (n.as_str(), "utilities-terminal")).collect();
        let default = self.profiles.default.clone();
        let mut changed = false;
        let mut add = None;
        let mut remove = false;
        let selected = self.selected.min(names.len() - 1);
        let count = names.len();
        let message = self.message.clone();
        let profile = &mut self.profiles.profiles[selected];
        let mut make_default = false;
        self.selected = ui.sidebar(&items, selected, |ui, _| {
            ui.title(&profile.name);
            if profile.name == default {
                ui.note("Perfil padrão das janelas e abas novas.");
            }
            if let Some(m) = &message {
                ui.note(m);
            }
            ui.card(|ui| {
                ui.label("Nome");
                let mut name = profile.name.clone();
                if ui.text_input(&mut name, "Nome do perfil").changed() && !name.trim().is_empty() && !names.iter().any(|n| n == &name) {
                    profile.name = name;
                    changed = true;
                }
            });
            ui.heading("Aparência");
            ui.card(|ui| {
                let themes: Vec<String> = THEMES.iter().map(|t| t.name.to_string()).collect();
                let mut theme = THEMES.iter().position(|t| t.name == profile.theme).unwrap_or(0);
                if ui.dropdown("Cores", &themes, &mut theme).changed() {
                    profile.theme = THEMES[theme].name.into();
                    changed = true;
                }
                // Amostra das cores do tema: fundo, texto e as 16 cores.
                let t = palette::theme(&profile.theme);
                ui.custom(26.0, |pixmap, (x, y, _, h)| {
                    fill(pixmap, x, y, 16.0 * 18.0 + 30.0, h, t.background);
                    fill(pixmap, x + 6.0, y + 5.0, 16.0, 16.0, t.foreground);
                    for (i, c) in t.ansi.iter().enumerate() {
                        fill(pixmap, x + 28.0 + i as f32 * 18.0, y + 5.0, 16.0, 16.0, *c);
                    }
                });
                let mut size = profile.font_size;
                if ui.slider("Tamanho da fonte", &mut size, 9.0, 24.0).changed() {
                    profile.font_size = size.round();
                    changed = true;
                }
                let names: Vec<String> = CURSORS.iter().map(|(n, _)| n.to_string()).collect();
                let mut cursor = CURSORS.iter().position(|(_, c)| *c == profile.cursor).unwrap_or(0);
                if ui.dropdown("Cursor", &names, &mut cursor).changed() {
                    profile.cursor = CURSORS[cursor].1;
                    changed = true;
                }
                changed |= ui.switch("Cursor piscando", &mut profile.cursor_blink).changed();
                changed |= ui.switch("Negrito em cor clara", &mut profile.bold_bright).changed();
                changed |= ui.switch("Aviso visual do sino", &mut profile.visual_bell).changed();
            });
            ui.heading("Sessão");
            ui.card(|ui| {
                let options: Vec<String> = SCROLLBACKS.iter().map(|n| format!("{} linhas", group(*n))).collect();
                let mut i = SCROLLBACKS.iter().position(|n| *n >= profile.scrollback).unwrap_or(2);
                if ui.dropdown("Histórico", &options, &mut i).changed() {
                    profile.scrollback = SCROLLBACKS[i];
                    changed = true;
                }
                let dirs = vec!["Na mesma pasta".to_string(), "Na pasta pessoal".to_string()];
                let mut d = if profile.start_dir == StartDir::Same { 0 } else { 1 };
                if ui.dropdown("Abas e painéis novos abrem", &dirs, &mut d).changed() {
                    profile.start_dir = if d == 0 { StartDir::Same } else { StartDir::Home };
                    changed = true;
                }
                ui.label("Comando ao abrir");
                changed |= ui.text_input(&mut profile.command, "Shell do usuário").changed();
            });
            ui.space(8.0);
            ui.row(|ui| {
                if ui.button("Novo perfil").clicked() {
                    add = Some(None);
                }
                if ui.button("Duplicar").clicked() {
                    add = Some(Some(profile.clone()));
                }
                if profile.name != default && ui.button("Usar como padrão").clicked() {
                    make_default = true;
                }
                if count > 1 && ui.button("Apagar").clicked() {
                    remove = true;
                }
            });
        });
        if make_default {
            self.profiles.default = self.profiles.profiles[selected].name.clone();
            changed = true;
        }
        if let Some(base) = add {
            let mut p = base.unwrap_or_default();
            p.name = self.profiles.free_name(if p.name == "Padrão" || p.name.is_empty() { "Perfil" } else { &p.name });
            self.profiles.profiles.push(p);
            self.selected = self.profiles.profiles.len() - 1;
            changed = true;
        }
        if remove {
            let name = self.profiles.profiles.remove(selected).name;
            if self.profiles.default == name {
                self.profiles.default = self.profiles.profiles[0].name.clone();
            }
            self.selected = 0;
            changed = true;
        }
        // Nome trocado: o padrão acompanha.
        if !self.profiles.profiles.iter().any(|p| p.name == self.profiles.default) {
            self.profiles.default = self.profiles.profiles[0].name.clone();
        }
        if changed {
            self.save();
        }
    }
}

/// "10000" vira "10.000".
fn group(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push('.');
        }
        out.push(c);
    }
    out
}

fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "mide_terminal=info".into()))
        .init();
    let options = Options::parse(std::env::args().skip(1));
    if options.preferences {
        let window = Window::new("Preferências do Terminal").app_id("mide-terminal-preferences").size(720, 560).min_size(560, 420);
        return run(window, Preferences { profiles: Profiles::load(), selected: 0, message: None });
    }
    let window = Window::new(TITLE).app_id("mide-terminal").size(700, 500).min_size(240, 120);
    run(window, TerminalApp::new(options))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(keysym: u32, ctrl: bool, shift: bool, alt: bool, logo: bool) -> KeyPress {
        KeyPress { keysym, text: None, ctrl, shift, alt, logo }
    }

    #[test]
    fn atalhos_do_mac_e_do_linux() {
        let cmd = |k: char| press(k as u32, false, false, false, true);
        let cmd_shift = |k: char| press(k as u32, false, true, false, true);
        let linux = |k: char| press(k as u32, true, true, false, false);
        assert_eq!(shortcut(&cmd('t')), Some(Action::NewTab));
        assert_eq!(shortcut(&linux('T')), Some(Action::NewTab));
        assert_eq!(shortcut(&cmd('d')), Some(Action::SplitSide));
        assert_eq!(shortcut(&cmd_shift('D')), Some(Action::SplitStack));
        assert_eq!(shortcut(&linux('E')), Some(Action::SplitStack));
        assert_eq!(shortcut(&cmd('f')), Some(Action::Find));
        assert_eq!(shortcut(&cmd_shift('G')), Some(Action::FindPrevious));
        assert_eq!(shortcut(&cmd('3')), Some(Action::Tab(2)));
        assert_eq!(shortcut(&press('2' as u32, false, false, true, false)), Some(Action::Tab(1)));
        assert_eq!(shortcut(&press(0xff53, false, false, true, true)), Some(Action::PaneDirection(Direction::Right)));
        assert_eq!(shortcut(&press(0xff52, true, true, false, false)), Some(Action::PreviousPrompt));
        assert_eq!(shortcut(&press(0xff0d, false, true, false, true)), Some(Action::Zoom));
        assert_eq!(shortcut(&press(0xff56, true, false, false, false)), Some(Action::NextTab));
        assert_eq!(shortcut(&press(0xff55, false, true, false, false)), Some(Action::PageUp));
        assert_eq!(shortcut(&cmd(',')), Some(Action::Preferences));
        // Ctrl+C e Ctrl+D vão para o programa.
        assert_eq!(shortcut(&press('c' as u32, true, false, false, false)), None);
        assert_eq!(shortcut(&press('d' as u32, true, false, false, false)), None);
    }

    #[test]
    fn opcoes_da_linha_de_comando() {
        let o = Options::parse(["-D", "/tmp", "--title", "T", "--profile", "Escuro", "-e", "ls", "-l"].map(String::from));
        assert_eq!(o.spawn.directory, Some(PathBuf::from("/tmp")));
        assert_eq!(o.title.as_deref(), Some("T"));
        assert_eq!(o.profile.as_deref(), Some("Escuro"));
        assert_eq!(o.spawn.command, Some(("ls".into(), vec!["-l".into()])));
        assert!(Options::parse(["--preferences".to_string()]).preferences);
    }

    #[test]
    fn milhar() {
        assert_eq!(group(1000), "1.000");
        assert_eq!(group(100000), "100.000");
    }
}
