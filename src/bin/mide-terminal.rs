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
    kit::{run, window_menu, App, Key, KeyPress, MenuGroup, MenuItem, Ui, Waker, Window},
    terminal::{
        graphics::PLACEHOLDER,
        keys::{key_bytes, paste_bytes},
        layout::{Direction, Divider, Layout, PaneId, Rect, Split},
        palette::{self, THEMES},
        profile::{self, parse_hex, Cursor, Profile, Profiles, StartDir, Trigger, TriggerAction, COLOR_SLOTS},
        pty::{Pty, Size, Spawn},
        session::{Saved, SavedLayout, SavedTab},
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
    /// Terminal de atalho: desce do topo e some com o mesmo atalho.
    hotkey: bool,
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
                "--hotkey" => options.hotkey = true,
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

/// De onde vem a saída e para onde vai o que se digita.
enum Backend {
    /// Um programa num PTY.
    Local(Pty),
    /// Um painel de uma sessão do tmux (`tmux -CC`): a saída chega pelo `%output` e o
    /// que se digita vai como `send-keys` pelo painel que roda o tmux.
    Tmux { gateway: mpsc::Sender<Vec<u8>>, pane: u32 },
}

struct Pane {
    term: Terminal,
    backend: Backend,
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
    /// Do perfil: fundo translúcido, vidro atrás dele e o aviso visual do sino.
    opacity: f32,
    blur: bool,
    visual_bell: bool,
    /// Gatilhos do perfil, a próxima linha a conferir e os trechos destacados por eles.
    triggers: Vec<Trigger>,
    scanned: u64,
    highlights: Vec<((u64, usize), (u64, usize), Rgb)>,
    /// Imagens já convertidas para desenhar (pelo número da imagem).
    image_cache: std::cell::RefCell<HashMap<u64, Pixmap>>,
    /// Imagens dos placeholders do kitty já no tamanho da grade (número, colunas,
    /// linhas e tamanho da célula).
    placeholder_cache: std::cell::RefCell<HashMap<(u32, u32, u32, u32, u32), Pixmap>>,
}

impl Pane {
    fn new(term: Terminal, backend: Backend, output: mpsc::Receiver<Vec<u8>>) -> Self {
        Self {
            term,
            backend,
            output,
            profile: String::new(),
            scroll: 0,
            scroll_rest: 0.0,
            selection: None,
            mouse_held: None,
            mouse_cell: None,
            bell_at: None,
            opacity: 1.0,
            blur: false,
            visual_bell: true,
            triggers: Vec::new(),
            scanned: 0,
            highlights: Vec::new(),
            image_cache: Default::default(),
            placeholder_cache: Default::default(),
        }
    }

    fn write(&self, bytes: Vec<u8>) {
        if bytes.is_empty() {
            return;
        }
        match &self.backend {
            Backend::Local(pty) => pty.write(bytes),
            Backend::Tmux { gateway, pane } => {
                for command in mide::terminal::tmux::send_keys(*pane, &bytes) {
                    let _ = gateway.send(format!("{command}\n").into_bytes());
                }
            }
        }
    }

    /// Avisa o programa do tamanho novo (no tmux, quem decide é o tmux).
    fn resize(&self, size: Size) {
        if let Backend::Local(pty) = &self.backend {
            pty.resize(size);
        }
    }

    fn cwd(&self) -> Option<PathBuf> {
        match &self.backend {
            Backend::Local(pty) => pty.cwd(),
            Backend::Tmux { .. } => None,
        }
    }

    fn foreground(&self) -> Option<(String, bool)> {
        match &self.backend {
            Backend::Local(pty) => pty.foreground(),
            Backend::Tmux { .. } => None,
        }
    }

    /// Número do painel no tmux, se for um.
    fn tmux_pane(&self) -> Option<u32> {
        match &self.backend {
            Backend::Tmux { pane, .. } => Some(*pane),
            Backend::Local(_) => None,
        }
    }

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
        // Respostas ao programa (no tmux, o próprio tmux responde por ele).
        let responses = self.term.take_responses();
        if self.tmux_pane().is_none() {
            self.write(responses);
        }
        if let Some(text) = self.term.take_clipboard() {
            *copy = Some(text);
        }
        if std::mem::take(&mut self.term.bell) && self.visual_bell {
            self.bell_at = Some(Instant::now());
        }
        if self.scroll > 0 {
            self.scroll = (self.scroll + self.term.history_len().saturating_sub(history_before)).min(self.term.history_len());
        }
        if read > 0 {
            self.run_triggers();
        }
        if ended {
            if let Backend::Local(pty) = &mut self.backend {
                pty.reap();
            }
        }
        (read > 0, ended)
    }

    /// Gatilhos nas linhas que terminaram desde a última vez (acima do cursor).
    fn run_triggers(&mut self) {
        let (first, _) = self.term.abs_range();
        let cursor = self.term.visible_abs(self.term.cursor().0, 0);
        self.scanned = self.scanned.max(first);
        self.highlights.retain(|(a, _, _)| a.0 >= first);
        if self.triggers.is_empty() || cursor <= self.scanned {
            self.scanned = self.scanned.max(cursor);
            return;
        }
        let mut notices = 0;
        for trigger in &self.triggers {
            for (a, b) in self.term.search_lines(&trigger.text, self.scanned, cursor - 1) {
                match trigger.action {
                    TriggerAction::Highlight => {
                        let color = parse_hex(&trigger.color).unwrap_or([255, 216, 77]);
                        if self.highlights.len() < 5_000 {
                            self.highlights.push((a, b, color));
                        }
                    }
                    TriggerAction::Notify if notices < 3 => {
                        notices += 1;
                        let cols = self.term.cols();
                        let line = self.term.text_between((a.0, 0), (a.0, cols - 1));
                        mide::notify::send("Terminal", &format!("Gatilho: {}", trigger.text), line.trim());
                    }
                    TriggerAction::Notify => {}
                    TriggerAction::Bell => self.bell_at = Some(Instant::now()),
                }
            }
        }
        self.scanned = cursor;
    }

    /// Título da aba: o que o programa pediu, ou o nome do programa em primeiro plano.
    fn title(&self) -> String {
        if let Some(t) = self.term.title.as_ref().filter(|t| !t.trim().is_empty()) {
            return t.clone();
        }
        self.foreground().map(|(name, _)| name).unwrap_or_else(|| TITLE.into())
    }

    fn selected_text(&self) -> Option<String> {
        let (a, b) = self.selection?.range(&self.term);
        Some(self.term.text_between(a, b)).filter(|t| !t.is_empty())
    }

    fn apply(&mut self, profile: &Profile) {
        self.term.theme = profile.theme();
        self.opacity = profile.opacity;
        self.blur = profile.blur;
        self.visual_bell = profile.visual_bell;
        self.triggers = profile.triggers.iter().filter(|t| !t.text.trim().is_empty()).cloned().collect();
        self.term.bold_bright = profile.bold_bright;
        self.term.set_history_limit(profile.scrollback);
        self.term.cursor_shape = match profile.cursor {
            Cursor::Block => CursorShape::Block,
            Cursor::Bar => CursorShape::Bar,
            Cursor::Underline => CursorShape::Underline,
        };
    }
}

struct Tab {
    layout: Layout,
    focus: PaneId,
    /// Painel em foco ampliado na aba inteira.
    zoom: bool,
    /// Saída nova enquanto a aba estava em segundo plano.
    activity: bool,
    /// Janela de uma sessão do tmux: o painel que roda o tmux e o número da janela.
    tmux: Option<(PaneId, u32)>,
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
    /// Família da fonte da janela (vazio: a padrão do sistema).
    font_family: String,
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
    /// Aba sendo arrastada (posição atual e onde o arrasto começou).
    tab_drag: Option<(usize, f32)>,
    /// Endereço sob o ponteiro (com Ctrl ou ⌘): painel, começo e fim (sublinhado).
    hover_url: Option<(PaneId, (u64, usize), (u64, usize))>,
    /// Endereço sob o clique direito (para "Abrir endereço" no menu).
    menu_url: Option<String>,
    /// Fechar sem perguntar (já confirmado).
    confirmed_close: bool,
    /// O que abrir na primeira aba (da linha de comando).
    first_spawn: Option<Spawn>,
    /// Teclado no estilo Mac (os menus mostram os atalhos com ⌘).
    mac: bool,
    /// Terminal de atalho: pedidos de mostrar/esconder e se está aparecendo.
    hotkey: Option<Hotkey>,
    /// Sessões do tmux em modo de controle (`tmux -CC`) abertas nos painéis.
    tmux: Vec<TmuxSession>,
}

/// O que se espera da resposta de cada comando mandado ao tmux.
#[derive(Debug, Clone, Copy)]
enum Expect {
    Windows,
    Capture(u32),
    Cursor(u32),
    Ignore,
}

/// Sessão do tmux rodando num painel (o "portal"): as janelas dela viram abas e os
/// painéis, painéis do mide-terminal.
struct TmuxSession {
    gateway: PaneId,
    client: mide::terminal::tmux::Client<Expect>,
    /// Escreve no PTY do portal (os comandos para o tmux).
    writer: mpsc::Sender<Vec<u8>>,
    /// Painel do tmux → painel do mide e o canal da saída dele.
    panes: HashMap<u32, (PaneId, mpsc::Sender<Vec<u8>>)>,
    /// Tamanho pedido ao tmux (colunas e linhas).
    size: (usize, usize),
}

impl TmuxSession {
    fn command(&mut self, command: &str, expect: Expect) {
        let bytes = self.client.send(command, expect);
        let _ = self.writer.send(bytes);
    }
}

struct Hotkey {
    toggles: mpsc::Receiver<()>,
    /// Quem acorda a janela quando chega um pedido (preenchido no primeiro quadro).
    waker: std::sync::Arc<std::sync::Mutex<Option<Waker>>>,
    visible: bool,
}

impl TerminalApp {
    fn new(options: Options) -> Self {
        let profiles = Profiles::load();
        let start_profile = profiles.get(options.profile.as_deref().unwrap_or("")).clone();
        let (font_size, font_family) = (start_profile.font_size, start_profile.font.clone());
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
            cell: cell_size(font_size, &font_family),
            font_family,
            find: None,
            dialog: None,
            selecting: None,
            last_click: None,
            dragging: None,
            title: TITLE.into(),
            error: None,
            waker: None,
            tab_drag: None,
            hover_url: None,
            menu_url: None,
            confirmed_close: false,
            first_spawn,
            mac: mac_style(),
            hotkey: None,
            tmux: Vec::new(),
        }
    }

    fn profile_name(&self) -> String {
        self.options.profile.clone().filter(|p| self.profiles.profiles.iter().any(|x| &x.name == p)).unwrap_or_else(|| self.profiles.default.clone())
    }

    /// Abre um shell num painel novo (`near`: painel cuja pasta a nova herda).
    fn spawn_pane(&mut self, near: Option<PaneId>, profile: Option<&str>) -> Option<PaneId> {
        self.spawn_pane_in(near, profile, None)
    }

    /// Como `spawn_pane`, numa pasta escolhida (sessão restaurada).
    fn spawn_pane_in(&mut self, near: Option<PaneId>, profile: Option<&str>, dir: Option<PathBuf>) -> Option<PaneId> {
        // Perfil escolhido (menu Perfis, --profile) fica preso ao painel; sem escolha,
        // o painel segue o padrão, mesmo que o padrão mude depois.
        let chosen = profile.filter(|p| !p.is_empty()).map(str::to_string).or_else(|| self.options.profile.clone()).unwrap_or_default();
        let profile = self.profiles.get(&chosen).clone();
        let mut spawn = self.first_spawn.take().unwrap_or_default();
        if dir.is_some() {
            spawn.directory = dir;
        }
        if spawn.command.is_none() && !profile.command.trim().is_empty() {
            spawn.command = Some(("sh".into(), vec!["-c".into(), profile.command.clone()]));
        }
        if profile.shell_integration {
            mide::terminal::integration::apply(&mut spawn);
        }
        if spawn.directory.is_none() && profile.start_dir == StartDir::Same {
            spawn.directory = near.and_then(|id| self.panes.get(&id)).and_then(|p| p.cwd());
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
                let mut pane = Pane::new(term, Backend::Local(pty), output);
                pane.profile = chosen;
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
        self.new_tab_with(None);
    }

    /// Aba nova com o perfil escolhido (ou o padrão).
    fn new_tab_with(&mut self, profile: Option<&str>) {
        let near = self.focused();
        if let Some(id) = self.spawn_pane(near, profile) {
            self.tabs.push(Tab { layout: Layout::Pane(id), focus: id, zoom: false, activity: false, tmux: None });
            self.active = self.tabs.len() - 1;
        }
    }

    fn split(&mut self, dir: Split) {
        let Some(focus) = self.focused() else {
            return;
        };
        let profile = self.panes.get(&focus).map(|p| p.profile.clone());
        if let Some(id) = self.spawn_pane(Some(focus), profile.as_deref()) {
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
        panes.iter().filter_map(|id| self.panes.get(id)).filter_map(|p| p.foreground()).filter(|(_, other)| *other).map(|(name, _)| name).collect()
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
                if self.hotkey.is_none() {
                    self.save_session();
                }
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

    // ---- tmux (modo de controle) ----

    /// Sessão do tmux e número do painel no tmux de um painel do mide.
    fn tmux_of(&self, id: PaneId) -> Option<(usize, u32)> {
        self.tmux.iter().enumerate().find_map(|(i, s)| s.panes.iter().find(|(_, (l, _))| *l == id).map(|(r, _)| (i, *r)))
    }

    /// Começo, conversa e fim das sessões do tmux nos painéis.
    fn tmux_step(&mut self) {
        let ids: Vec<PaneId> = self.panes.keys().copied().collect();
        for gid in ids {
            let Some(pane) = self.panes.get_mut(&gid) else {
                continue;
            };
            let active = pane.term.tmux_active();
            let (data, ended) = pane.term.take_tmux();
            let known = self.tmux.iter().position(|s| s.gateway == gid);
            if known.is_none() && (active || !data.is_empty()) {
                // tmux dentro de um painel do tmux: fica como está.
                let Backend::Local(pty) = &pane.backend else {
                    continue;
                };
                // Nada pode ser escrito no terminal deste painel enquanto o tmux fala
                // (estamos dentro da sequência dele); o aviso é desenhado por cima.
                let writer = pty.writer();
                let mut session = TmuxSession { gateway: gid, client: Default::default(), writer, panes: HashMap::new(), size: (0, 0) };
                session.command(mide::terminal::tmux::LIST_WINDOWS, Expect::Windows);
                self.tmux.push(session);
            }
            if let Some(i) = self.tmux.iter().position(|s| s.gateway == gid) {
                if !data.is_empty() {
                    let messages = self.tmux[i].client.feed(&data);
                    self.tmux_messages(i, messages);
                }
                if ended {
                    if let Some(i) = self.tmux.iter().position(|s| s.gateway == gid) {
                        self.tmux_end(i);
                    }
                }
            }
        }
        // O painel do tmux fechou: a sessão acaba junto.
        while let Some(i) = self.tmux.iter().position(|s| !self.panes.contains_key(&s.gateway)) {
            self.tmux_end(i);
        }
    }

    fn tmux_messages(&mut self, i: usize, messages: Vec<mide::terminal::tmux::Message<Expect>>) {
        use mide::terminal::tmux::{Event, Message};
        let mut relist = false;
        let mut exit = false;
        for message in messages {
            match message {
                Message::Event(Event::Output { pane, data }) => {
                    if let Some((_, tx)) = self.tmux[i].panes.get(&pane) {
                        let _ = tx.send(data);
                    }
                }
                Message::Event(Event::Exit(_)) => exit = true,
                Message::Event(Event::PaneActive { .. } | Event::WindowActive { .. }) => {}
                Message::Event(_) => relist = true,
                Message::Reply(Expect::Windows, reply) if reply.ok => self.tmux_windows(i, &reply.lines),
                Message::Reply(Expect::Capture(remote), reply) if reply.ok => {
                    if let Some(pane) = self.tmux[i].panes.get(&remote).and_then(|(l, _)| self.panes.get_mut(l)) {
                        pane.term.feed(reply.lines.join("\r\n").as_bytes());
                    }
                }
                Message::Reply(Expect::Cursor(remote), reply) if reply.ok => {
                    let pos: Vec<usize> = reply.lines.first().map(|l| l.split(' ').filter_map(|n| n.parse().ok()).collect()).unwrap_or_default();
                    if let (Some(&x), Some(&y), Some(pane)) = (pos.first(), pos.get(1), self.tmux[i].panes.get(&remote).and_then(|(l, _)| self.panes.get_mut(l))) {
                        pane.term.feed(format!("\x1b[{};{}H", y + 1, x + 1).as_bytes());
                    }
                }
                Message::Reply(..) => {}
            }
        }
        if relist {
            self.tmux[i].command(mide::terminal::tmux::LIST_WINDOWS, Expect::Windows);
        }
        if exit {
            self.tmux_end(i);
        }
        if let Some(w) = &self.waker {
            w.wake();
        }
    }

    /// Acerta abas e painéis com a lista de janelas do tmux.
    fn tmux_windows(&mut self, i: usize, lines: &[String]) {
        use mide::terminal::tmux::{capture, cursor, parse_window_line};
        let gid = self.tmux[i].gateway;
        let profile = self.panes.get(&gid).map(|p| self.profiles.get(&p.profile).clone()).unwrap_or_default();
        let mut windows = Vec::new();
        let mut remotes = std::collections::HashSet::new();
        for line in lines {
            let Some((window, layout, active, name)) = parse_window_line(line) else {
                continue;
            };
            windows.push(window);
            for remote in layout.panes() {
                remotes.insert(remote);
                if self.tmux[i].panes.contains_key(&remote) {
                    continue;
                }
                let (tx, rx) = mpsc::channel();
                let mut term = Terminal::new(80, 24);
                term.cell_pixels = (self.cell.0 as u16, self.cell.1 as u16);
                let mut pane = Pane::new(term, Backend::Tmux { gateway: self.tmux[i].writer.clone(), pane: remote }, rx);
                pane.apply(&profile);
                let id = self.next_id;
                self.next_id += 1;
                self.panes.insert(id, pane);
                self.tmux[i].panes.insert(remote, (id, tx));
                self.tmux[i].command(&capture(remote), Expect::Capture(remote));
                self.tmux[i].command(&cursor(remote), Expect::Cursor(remote));
            }
            let session = &self.tmux[i];
            let Some(tree) = layout.to_layout(&|r| session.panes.get(&r).map(|(l, _)| *l)) else {
                continue;
            };
            let panes = tree.panes();
            for id in &panes {
                if let Some(p) = self.panes.get_mut(id) {
                    if p.term.title.is_none() {
                        p.term.title = Some(name.clone());
                    }
                }
            }
            match self.tabs.iter_mut().find(|t| t.tmux == Some((gid, window))) {
                Some(tab) => {
                    if !panes.contains(&tab.focus) {
                        tab.focus = panes[0];
                    }
                    tab.layout = tree;
                    tab.zoom = false;
                }
                None => {
                    self.tabs.push(Tab { layout: tree, focus: panes[0], zoom: false, activity: false, tmux: Some((gid, window)) });
                    if active {
                        self.active = self.tabs.len() - 1;
                    }
                }
            }
        }
        // Janelas e painéis que não existem mais no tmux.
        let gone: Vec<u32> = self.tmux[i].panes.keys().filter(|r| !remotes.contains(r)).copied().collect();
        for remote in gone {
            if let Some((local, _)) = self.tmux[i].panes.remove(&remote) {
                self.panes.remove(&local);
            }
        }
        self.tabs.retain(|t| !matches!(t.tmux, Some((g, w)) if g == gid && !windows.contains(&w)));
        self.active = self.active.min(self.tabs.len().saturating_sub(1));
    }

    /// Fim de uma sessão do tmux: as abas dela fecham e o painel do tmux volta.
    fn tmux_end(&mut self, i: usize) {
        let session = self.tmux.remove(i);
        for (local, _) in session.panes.values() {
            self.panes.remove(local);
        }
        self.tabs.retain(|t| t.tmux.is_none_or(|(g, _)| g != session.gateway));
        if let Some(pane) = self.panes.get_mut(&session.gateway) {
            pane.term.feed("\r\n\x1b[2m[tmux desanexado]\x1b[0m\r\n".as_bytes());
            if let Some(t) = self.tabs.iter().position(|t| t.layout.panes().contains(&session.gateway)) {
                self.active = t;
            }
        }
        self.active = self.active.min(self.tabs.len().saturating_sub(1));
    }

    /// Tamanho da área do terminal: a janela toda, ou só a parte de cima da tela no
    /// terminal de atalho (o resto fica transparente e deixa os cliques passarem).
    fn view_size(&self, ui: &mut Ui) -> (f32, f32, f32, f32) {
        let (x, y, w, h) = ui.region();
        if self.hotkey.is_none() {
            return (x, y, w, h);
        }
        let panel = (h * self.profiles.hotkey_height).round().max(120.0).min(h);
        ui.input_area((0.0, 0.0, w, panel));
        (x, y, w, panel)
    }

    /// Comandos demorados que terminaram com a janela sem foco (ou numa aba de trás):
    /// avisa com uma notificação, como o iTerm2.
    fn command_alerts(&mut self, ui: &mut Ui, active_panes: &[PaneId]) {
        let focused = ui.input().focused;
        for (id, pane) in &mut self.panes {
            let after = self.profiles.get(&pane.profile).notify_after;
            for done in pane.term.take_finished() {
                let visible = focused && active_panes.contains(id);
                if after == 0 || visible || done.duration < Duration::from_secs(after as u64) {
                    continue;
                }
                let cols = pane.term.cols();
                let command = pane.term.text_between((done.line, 0), (done.line, cols - 1));
                let summary = if done.status == 0 { "Comando terminado".to_string() } else { format!("Comando falhou (código {})", done.status) };
                let body = format!("{} — em {}", command.trim(), duration_text(done.duration));
                mide::notify::send("Terminal", &summary, &body);
            }
        }
    }

    /// Guarda abas e painéis (com as pastas) para a próxima janela, se o usuário quer.
    fn save_session(&self) {
        if !self.profiles.restore || self.tabs.is_empty() {
            return;
        }
        let info = |id: PaneId| {
            let pane = self.panes.get(&id);
            (pane.and_then(|p| p.cwd()), pane.map(|p| p.profile.clone()).unwrap_or_default())
        };
        let tabs = self
            .tabs
            .iter()
            .filter(|t| t.tmux.is_none())
            .map(|t| SavedTab { layout: SavedLayout::from_layout(&t.layout, &info), focus: t.layout.panes().iter().position(|&p| p == t.focus).unwrap_or(0) })
            .collect();
        let saved = Saved { tabs, active: self.active, font_size: Some(self.font_size) };
        if let Err(err) = saved.save() {
            tracing::warn!("sessão do terminal não gravada: {err}");
        }
    }

    /// Reabre a sessão guardada. Devolve se abriu alguma aba.
    fn restore_session(&mut self) -> bool {
        let Some(saved) = Saved::load() else {
            return false;
        };
        if let Some(size) = saved.font_size {
            self.set_font(size);
        }
        for tab in &saved.tabs {
            let layout = tab.layout.to_layout(&mut |dir, profile| self.spawn_pane_in(None, Some(profile), dir.cloned()));
            if let Some(layout) = layout {
                let panes = layout.panes();
                let focus = panes.get(tab.focus).copied().unwrap_or(panes[0]);
                self.tabs.push(Tab { layout, focus, zoom: false, activity: false, tmux: None });
            }
        }
        self.active = saved.active.min(self.tabs.len().saturating_sub(1));
        !self.tabs.is_empty()
    }

    fn set_font(&mut self, size: f32) {
        self.font_size = size.clamp(8.0, 32.0);
        self.cell = cell_size(self.font_size, &self.font_family);
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
        self.mac = mac_style();
        let mtime = profile::path().and_then(|p| std::fs::metadata(p).ok()).and_then(|m| m.modified().ok());
        if mtime == self.profiles_mtime {
            return;
        }
        self.profiles_mtime = mtime;
        let old = self.profiles.get(&self.profile_name()).clone();
        self.profiles = Profiles::load();
        for pane in self.panes.values_mut() {
            let profile = self.profiles.get(&pane.profile).clone();
            pane.apply(&profile);
        }
        let new = self.profiles.get(&self.profile_name()).clone();
        if new.font_size != old.font_size || new.font != old.font {
            self.font_family = new.font.clone();
            self.set_font(new.font_size);
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
        // Num painel do tmux, abas e painéis novos (e fechar) são pedidos ao tmux.
        if let Some((i, remote)) = focus.and_then(|id| self.tmux_of(id)) {
            let command = match action {
                Action::NewTab => Some("new-window".to_string()),
                Action::SplitSide => Some(format!("split-window -h -t %{remote}")),
                Action::SplitStack => Some(format!("split-window -v -t %{remote}")),
                Action::ClosePane => Some(format!("kill-pane -t %{remote}")),
                Action::TmuxDetach => Some("detach-client".to_string()),
                _ => None,
            };
            if let Some(command) = command {
                self.tmux[i].command(&command, Expect::Ignore);
                return;
            }
        }
        if action == Action::TmuxDetach {
            if let Some(i) = focus.and_then(|id| self.tmux.iter().position(|s| s.gateway == id)) {
                self.tmux[i].command("detach-client", Expect::Ignore);
            }
            return;
        }
        match action {
            Action::Copy => {
                if let Some(text) = focus.and_then(|id| self.panes.get(&id)).and_then(Pane::selected_text) {
                    ui.copy(&text);
                }
            }
            Action::Paste => ui.request_paste(),
            Action::SelectAll => {
                if let Some(pane) = focus.and_then(|id| self.panes.get_mut(&id)) {
                    let (first, last) = pane.term.abs_range();
                    let cols = pane.term.cols();
                    pane.selection = Some(Selection { anchor: (first, 0), head: (last, cols - 1), unit: Unit::Char });
                }
            }
            Action::NewWindow => {
                let dir = focus.and_then(|id| self.panes.get(&id)).and_then(|p| p.cwd());
                if let Ok(exe) = std::env::current_exe() {
                    let mut command = std::process::Command::new(exe);
                    if let Some(dir) = dir {
                        command.arg("-D").arg(dir);
                    }
                    let _ = command.spawn();
                }
            }
            Action::CloseWindow => self.close(Closing::Window, ui),
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
                    pane.write(b"\x0c".to_vec());
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
            // Tratado no começo (só vale com o tmux).
            Action::TmuxDetach => {}
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
            // O painel que roda o tmux -CC não recebe teclas (seriam comandos para o
            // tmux); Esc nele desanexa.
            if let Some(i) = self.focused().and_then(|id| self.tmux.iter().position(|s| s.gateway == id)) {
                if press.keysym == 0xff1b {
                    self.tmux[i].command("detach-client", Expect::Ignore);
                }
                continue;
            }
            let Some(pane) = self.focused().and_then(|id| self.panes.get_mut(&id)) else {
                return;
            };
            if let Some(bytes) = key_bytes(press, &pane.term.modes) {
                pane.scroll = 0;
                pane.selection = None;
                pane.write(bytes);
            }
        }
        let gateway = self.focused().is_some_and(|id| self.tmux.iter().any(|s| s.gateway == id));
        if let Some(text) = &input.paste {
            if !typing && self.dialog.is_none() && !gateway {
                if let Some(pane) = self.focused().and_then(|id| self.panes.get_mut(&id)) {
                    pane.scroll = 0;
                    pane.write(paste_bytes(text, pane.term.modes.bracketed_paste));
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
        let (mut open_link, mut copy_link) = (None::<String>, None::<String>);
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
                    pane.write(bytes);
                }
                continue;
            }

            // Endereços: com Ctrl ou ⌘ ficam sublinhados e o clique abre (como no
            // iTerm2); o clique direito guarda o endereço para o menu.
            let url = input.pointer.filter(|_| response.hovered).and_then(|pos| {
                let (_, at) = self.cell_at(rect, pane, pos);
                pane.term.url_at(at.0, at.1)
            });
            if response.secondary_clicked {
                self.menu_url = url.as_ref().map(|u| u.2.clone());
            }
            if input.ctrl || input.logo {
                if let Some((a, b, link)) = &url {
                    self.hover_url = Some((id, *a, *b));
                    if response.clicked {
                        mide::places::open(link);
                        continue;
                    }
                } else if self.hover_url.is_some_and(|(p, _, _)| p == id) {
                    self.hover_url = None;
                }
            } else if self.hover_url.is_some_and(|(p, _, _)| p == id) {
                self.hover_url = None;
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
                        pane.write(arrow.repeat(lines.unsigned_abs() as usize));
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

            // Os mesmos itens (e nomes) dos menus da barra superior.
            let has_selection = self.panes.get(&id).is_some_and(|p| p.selection.is_some());
            let several = self.tabs.get(self.active).is_some_and(|t| t.layout.panes().len() > 1);
            let sections: [&[Action]; 4] = [
                &[Action::Copy, Action::Paste, Action::SelectAll],
                &[Action::SplitSide, Action::SplitStack, Action::Zoom, Action::NewTab],
                &[Action::Find, Action::Clear, Action::Preferences],
                &[Action::ClosePane],
            ];
            let link = self.menu_url.clone();
            ui.context_menu(&response, |menu| {
                if let Some(link) = &link {
                    if menu.item("Abrir endereço") {
                        open_link = Some(link.clone());
                    }
                    if menu.item("Copiar endereço") {
                        copy_link = Some(link.clone());
                    }
                    menu.separator();
                }
                for (i, section) in sections.iter().enumerate() {
                    if i > 0 {
                        menu.separator();
                    }
                    for &action in section.iter() {
                        let shown = match action {
                            Action::Copy => has_selection,
                            Action::Zoom => several,
                            _ => true,
                        };
                        let chosen = if action == Action::ClosePane { menu.danger(action.label()) } else { shown && menu.item(action.label()) };
                        if chosen {
                            menu_action = Some(action);
                        }
                    }
                }
            });
        }
        if let Some(action) = menu_action {
            self.action(ui, action, area);
        }
        if let Some(link) = open_link {
            mide::places::open(&link);
        }
        if let Some(link) = copy_link {
            ui.copy(&link);
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
        // Arrastar uma aba para os lados muda a ordem.
        let input = ui.input().clone();
        if let (Some(i), Some((px, _))) = (clicked, input.pointer) {
            self.tab_drag = Some((i, px));
        }
        if let Some((from, start)) = self.tab_drag {
            if !input.down || input.released {
                self.tab_drag = None;
            } else if let Some((px, _)) = input.pointer {
                if (px - start).abs() > 6.0 {
                    let to = (((px - rect.0) / width).floor().max(0.0) as usize).min(n - 1);
                    if to != from {
                        let tab = self.tabs.remove(from);
                        self.tabs.insert(to, tab);
                        self.active = to;
                        self.tab_drag = Some((to, px));
                        clicked = None;
                    }
                }
            }
        }
        let (active, activity): (usize, Vec<bool>) = (self.active, self.tabs.iter().map(|t| t.activity).collect());
        let c = bar_colors(self.focused().and_then(|id| self.panes.get(&id)).map(|p| p.term.default_bg()));
        ui.paint_text(|pixmap, text, (ox, oy)| {
            let (x0, y0) = (ox + rect.0, oy + rect.1);
            fill(pixmap, x0, y0, rect.2, rect.3, c.bar);
            fill(pixmap, x0, y0 + rect.3 - 1.0, rect.2, 1.0, c.line);
            for (i, title) in titles.iter().enumerate() {
                let x = x0 + i as f32 * width;
                if i == active {
                    fill(pixmap, x, y0, width, rect.3, c.active);
                } else if hovered == Some(i) {
                    fill(pixmap, x, y0, width, rect.3 - 1.0, c.hover);
                }
                if i > 0 {
                    fill(pixmap, x, y0 + 6.0, 1.0, rect.3 - 12.0, c.line);
                }
                if hovered == Some(i) && n > 1 {
                    text.draw(pixmap, "×", x + 9.0, y0 + 5.0, 15.0, FontWeight::Regular, c.dim);
                }
                let size = 12.5;
                let label = fit(text, title, size, width - 52.0);
                let tw = text.measure(&label, size, FontWeight::Regular);
                let color = if i == active { c.text } else { c.dim };
                text.draw(pixmap, &label, x + (width - tw) / 2.0, y0 + (rect.3 - size * 1.25) / 2.0, size, FontWeight::Regular, color);
                if activity[i] && i != active {
                    fill_round(pixmap, x + width - 16.0, y0 + rect.3 / 2.0 - 3.0, 6.0, [10, 110, 235]);
                }
            }
            let px = x0 + n as f32 * width;
            text.draw(pixmap, "+", px + 11.0, y0 + 4.0, 17.0, FontWeight::Regular, c.dim);
        });
        if let Some(i) = close {
            match self.tabs[i].tmux {
                // Aba do tmux: quem fecha a janela é o tmux.
                Some((gid, window)) => {
                    if let Some(s) = self.tmux.iter_mut().find(|s| s.gateway == gid) {
                        s.command(&format!("kill-window -t @{window}"), Expect::Ignore);
                    }
                }
                None => self.close(Closing::Tab(i), ui),
            }
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
        let gateways: Vec<PaneId> = self.tmux.iter().map(|s| s.gateway).collect();
        let family = self.font_family.clone();
        // Painéis translúcidos com vidro: o compositor desfoca o que está atrás. Um
        // vidro só para a área toda (vários, um por painel, deixam emendas).
        if rects.iter().any(|(id, _)| self.panes.get(id).is_some_and(|p| p.opacity < 1.0 && p.blur)) {
            ui.glass_behind(area, 0.0);
        }
        ui.paint_text(|pixmap, text, (ox, oy)| {
            text.set_mono_family(Some(&family));
            for (id, rect) in &rects {
                let Some(pane) = self.panes.get(id) else {
                    continue;
                };
                let matches = find.filter(|_| Some(*id) == focus).map(|f| (f.matches.as_slice(), f.current));
                let flash = pane.bell_at.is_some_and(|t| t.elapsed() < Duration::from_millis(160));
                flashing |= flash;
                let link = self.hover_url.filter(|(p, _, _)| p == id).map(|(_, a, b)| (a, b));
                draw_pane(pixmap, text, pane, (ox + rect.0, oy + rect.1, rect.2, rect.3), cell, font, matches, link, Some(*id) == focus && !blink_off);
                if gateways.contains(id) {
                    // Painel que roda o tmux -CC: um aviso no lugar do protocolo.
                    fill_alpha(pixmap, ox + rect.0, oy + rect.1, rect.2, rect.3, pane.term.default_bg(), 0.92);
                    let msg = "O tmux está em modo de controle: as janelas dele abriram em abas.";
                    let sub = "Esc aqui (ou Shell → Desanexar do tmux) desanexa.";
                    let fg = pane.term.default_fg();
                    let rgba = [fg[0], fg[1], fg[2], 255];
                    let (tx, ty) = (ox + rect.0 + 16.0, oy + rect.1 + 16.0);
                    text.draw(pixmap, msg, tx, ty, 13.5, FontWeight::Semibold, rgba);
                    text.draw(pixmap, sub, tx, ty + 22.0, 12.5, FontWeight::Regular, rgba);
                }
                if several && Some(*id) != focus {
                    // Painéis sem foco ficam um pouco apagados (como no iTerm2): um véu da
                    // cor do fundo aproxima o texto do fundo, no tema claro e no escuro.
                    fill_alpha(pixmap, ox + rect.0, oy + rect.1, rect.2, rect.3, pane.term.default_bg(), 0.35);
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
        // Terminal de atalho: o mesmo atalho mostra e esconde.
        if let Some(hotkey) = &mut self.hotkey {
            if let Ok(mut w) = hotkey.waker.lock() {
                if w.is_none() {
                    *w = ui.waker();
                }
            }
            let mut toggled = false;
            while hotkey.toggles.try_recv().is_ok() {
                toggled = !toggled;
            }
            if toggled {
                hotkey.visible = !hotkey.visible;
                ui.set_hidden(!hotkey.visible);
            }
        }
        if self.tabs.is_empty() && self.error.is_none() {
            if let Some(title) = &self.options.title {
                ui.set_title(title);
            }
            // Sem nada pedido na linha de comando: reabre a última sessão (se ligado).
            let plain = self.options.spawn.command.is_none() && self.options.spawn.directory.is_none() && self.hotkey.is_none();
            if !(plain && self.profiles.restore && self.restore_session()) {
                self.new_tab();
            }
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
        self.tmux_step();
        self.command_alerts(ui, &active_panes);
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

        let (_, _, w, h) = self.view_size(ui);
        let (_, _, area) = self.areas((w, h));
        // Painéis da aba ativa com o tamanho da área deles.
        for (id, rect) in self.pane_rects(area) {
            let grid = self.grid_for(rect);
            let cell = self.cell;
            if let Some(pane) = self.panes.get_mut(&id) {
                if (pane.term.cols(), pane.term.rows()) != grid {
                    pane.term.resize(grid.0, grid.1);
                    pane.resize(Size { cols: grid.0 as u16, rows: grid.1 as u16, width: (grid.0 as f32 * cell.0) as u16, height: (grid.1 as f32 * cell.1) as u16 });
                    pane.selection = None;
                    pane.scroll = pane.scroll.min(pane.term.history_len());
                    // As linhas mudaram de número com o reflow.
                    pane.highlights.clear();
                    pane.scanned = pane.term.visible_abs(pane.term.cursor().0, 0);
                }
            }
        }

        // Aba do tmux: o tmux precisa saber o tamanho da área (ele arruma os painéis).
        if let Some((gid, _)) = self.tabs.get(self.active).and_then(|t| t.tmux) {
            let grid = self.grid_for(area);
            if let Some(session) = self.tmux.iter_mut().find(|s| s.gateway == gid) {
                if session.size != grid {
                    session.size = grid;
                    session.command(&format!("refresh-client -C {}x{}", grid.0, grid.1), Expect::Ignore);
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

        // Itens escolhidos nos menus da barra superior.
        for id in ui.input().menu.clone() {
            if let Some(name) = id.strip_prefix("perfil:") {
                self.new_tab_with(Some(name));
            } else if let Some(action) = Action::from_id(&id) {
                self.action(ui, action, area);
            }
        }
        self.keyboard(ui, area);
        if self.tabs.is_empty() {
            return;
        }
        // Um atalho pode ter aberto a busca ou uma aba: as áreas mudam.
        let (tab_bar, find_bar, area) = self.areas((w, h));
        self.draw(ui, area);
        if self.hotkey.is_some() {
            // Borda de baixo do painel que desceu do topo.
            ui.paint(|pixmap, (ox, oy)| fill_alpha(pixmap, ox, oy + h - 1.0, w, 1.0, [0, 0, 0], 0.3));
        }
        self.mouse(ui, area);
        if let Some(rect) = tab_bar.filter(|_| self.tabs.len() > 1) {
            self.tab_bar(ui, rect);
        }
        if let Some(rect) = find_bar.filter(|_| self.find.is_some()) {
            self.find_bar(ui, rect);
        }
        self.dialog(ui);
    }

    fn menu_bar(&mut self) -> Vec<MenuGroup> {
        if self.hotkey.is_some() {
            return Vec::new();
        }
        let mac = self.mac;
        let item = |a: Action| a.item(mac);
        let pane = self.focused().and_then(|id| self.panes.get(&id));
        let has_selection = pane.is_some_and(|p| p.selection.is_some());
        let tab = self.tabs.get(self.active);
        let several_panes = tab.is_some_and(|t| t.layout.panes().len() > 1);
        let several_tabs = self.tabs.len() > 1;
        let zoom = tab.is_some_and(|t| t.zoom);
        let finding = self.find.is_some();
        let in_tmux = self.focused().is_some_and(|id| self.tmux_of(id).is_some() || self.tmux.iter().any(|s| s.gateway == id));
        let mut profiles: Vec<MenuItem> = self
            .profiles
            .profiles
            .iter()
            .map(|p| MenuItem::action(&format!("perfil:{}", p.name), &format!("Nova aba com “{}”", p.name)))
            .collect();
        profiles.push(MenuItem::Separator);
        profiles.push(item(Action::Preferences));
        let mut window = vec![
            item(Action::PreviousTab).enabled(several_tabs),
            item(Action::NextTab).enabled(several_tabs),
            item(Action::PreviousPane).enabled(several_panes),
            item(Action::NextPane).enabled(several_panes),
        ];
        if several_tabs {
            window.push(MenuItem::Separator);
            for (i, t) in self.tabs.iter().enumerate() {
                let title = self.panes.get(&t.focus).map(Pane::title).unwrap_or_default();
                let mut entry = MenuItem::action(&Action::Tab(i).id(), &title).checked(i == self.active);
                if i < 9 {
                    entry = entry.shortcut(&format!("{}+{}", if mac { "Super" } else { "Alt" }, i + 1));
                }
                window.push(entry);
            }
        }
        vec![
            MenuGroup {
                title: "Shell".into(),
                items: vec![
                    item(Action::NewWindow),
                    item(Action::NewTab),
                    MenuItem::Separator,
                    item(Action::SplitSide),
                    item(Action::SplitStack),
                    MenuItem::Separator,
                    item(Action::ClosePane),
                    item(Action::CloseWindow),
                    MenuItem::Separator,
                    item(Action::TmuxDetach).enabled(in_tmux),
                ],
            },
            MenuGroup {
                title: "Editar".into(),
                items: vec![
                    item(Action::Copy).enabled(has_selection),
                    item(Action::Paste),
                    item(Action::SelectAll),
                    MenuItem::Separator,
                    item(Action::Find),
                    item(Action::FindNext).enabled(finding),
                    item(Action::FindPrevious).enabled(finding),
                    MenuItem::Separator,
                    item(Action::Clear),
                ],
            },
            MenuGroup {
                title: "Visualizar".into(),
                items: vec![
                    item(Action::FontBigger),
                    item(Action::FontSmaller),
                    item(Action::FontReset),
                    MenuItem::Separator,
                    item(Action::Zoom).checked(zoom).enabled(several_panes),
                    MenuItem::Separator,
                    item(Action::PreviousPrompt),
                    item(Action::NextPrompt),
                    item(Action::Top),
                    item(Action::Bottom),
                ],
            },
            MenuGroup { title: "Perfis".into(), items: profiles },
            window_menu(window),
        ]
    }

    fn close_requested(&mut self) -> bool {
        if self.confirmed_close {
            return true;
        }
        let all: Vec<PaneId> = self.panes.keys().copied().collect();
        let running = self.running(&all);
        if running.is_empty() {
            if self.hotkey.is_none() {
                self.save_session();
            }
            return true;
        }
        self.dialog = Some(Dialog::ConfirmClose(Closing::Window, running));
        false
    }
}

/// "1 min 5 s", "42 s".
fn duration_text(d: Duration) -> String {
    let s = d.as_secs();
    match (s / 3600, s / 60 % 60, s % 60) {
        (0, 0, s) => format!("{s} s"),
        (0, m, s) => format!("{m} min {s} s"),
        (h, m, _) => format!("{h} h {m} min"),
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
    link: Option<((u64, usize), (u64, usize))>,
    show_cursor_here: bool,
) {
    let term = &pane.term;
    let scroll = pane.scroll.min(term.history_len());
    let background = term.default_bg();
    // Fundo translúcido: os pixels ficam com a transparência (não misturados).
    let alpha = (pane.opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
    mide::ui::draw::paint_rounded_solid(pixmap, x0, y0, w, h, 0.0, [background[0], background[1], background[2], alpha]);
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
    // Marcas dos prompts (integração do shell) na margem esquerda: azul se o comando
    // deu certo, vermelha se falhou, cinza enquanto não houve comando.
    for mark in term.marks().iter().filter(|m| m.line >= top_abs && m.line <= bottom_abs) {
        let y = (mark.line - top_abs) as f32;
        let color = match mark.status {
            Some(0) => [10, 110, 235],
            Some(_) => [230, 60, 50],
            None => [150, 150, 158],
        };
        fill(pixmap, x0 + 1.0, py0 + y * cell.1 + 2.0, 3.0, cell.1 - 4.0, color);
    }
    for y in 0..term.rows() {
        let row = term.visible_row(y, scroll);
        let abs = term.visible_abs(y, scroll);
        let py = py0 + y as f32 * cell.1;
        // Placeholder anterior desta linha (imagem, linha e coluna), para os que vêm
        // sem coluna (a seguinte) ou sem linha (a mesma).
        let mut previous: Option<(u32, usize, usize)> = None;
        for (x, c) in row.cells.iter().enumerate() {
            if c.c == PLACEHOLDER {
                let px = px0 + x as f32 * cell.0;
                let bg_rgb = term.cell_colors(&c.pen).1;
                if bg_rgb != background {
                    fill(pixmap, px, py, cell.0, cell.1, bg_rgb);
                }
                let place = placeholder_place(c, previous);
                previous = place;
                if let Some((id, prow, pcol)) = place {
                    draw_placeholder(pixmap, pane, id, prow, pcol, (px, py), cell);
                }
                continue;
            }
            previous = None;
            if c.wide == Wide::Spacer {
                continue;
            }
            let px = px0 + x as f32 * cell.0;
            let width = if c.wide == Wide::Lead { 2.0 } else { 1.0 } * cell.0;
            let (mut fg, mut bg) = term.cell_colors(&c.pen);
            let at = (abs, x);
            if let Some((_, _, color)) = pane.highlights.iter().find(|(a, b, _)| at >= *a && at <= *b) {
                bg = *color;
                fg = [24, 24, 28];
            }
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
            if link.is_some_and(|(a, b)| at >= a && at <= b) {
                fill(pixmap, px, py + cell.1 - 2.0, width, 1.0, fg);
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
    draw_images(pixmap, pane, (x0, y0, w, h), cell, top_abs, bottom_abs);
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

/// Imagem, linha e coluna de um placeholder do kitty: a linha e a coluna vêm dos
/// acentos (ou do placeholder anterior), o número da imagem vem da cor do texto e,
/// o byte alto dele, do terceiro acento.
fn placeholder_place(c: &mide::terminal::screen::Cell, previous: Option<(u32, usize, usize)>) -> Option<(u32, usize, usize)> {
    use mide::terminal::{graphics::diacritic_index, screen::Color};
    let marks: Vec<usize> = c.combining.as_deref().unwrap_or("").chars().filter_map(diacritic_index).collect();
    let mut id = match c.pen.fg {
        Color::Rgb(r, g, b) => (r as u32) << 16 | (g as u32) << 8 | b as u32,
        Color::Indexed(i) => i as u32,
        Color::Default => 0,
    };
    if let Some(&msb) = marks.get(2) {
        id |= (msb as u32) << 24;
    }
    let same = previous.filter(|(pid, _, _)| *pid == id);
    let row = marks.first().copied().or(same.map(|(_, r, _)| r))?;
    let col = match marks.get(1) {
        Some(&col) => col,
        None => same.filter(|(_, r, _)| *r == row).map(|(_, _, c)| c + 1).unwrap_or(0),
    };
    Some((id, row, col))
}

/// Desenha, na célula, o pedaço (linha, coluna) da imagem do placeholder: a imagem
/// cabe inteira em colunas x linhas células, centrada e na proporção dela.
fn draw_placeholder(pixmap: &mut Pixmap, pane: &Pane, id: u32, row: usize, col: usize, (x, y): (f32, f32), cell: (f32, f32)) {
    let Some((image, cols, rows)) = pane.term.placeholder_image(id) else {
        return;
    };
    let (cw, ch) = (cell.0.max(1.0) as u32, cell.1.max(1.0) as u32);
    if row as u32 >= rows || col as u32 >= cols {
        return;
    }
    let mut cache = pane.placeholder_cache.borrow_mut();
    let key = (id, cols, rows, cw, ch);
    if !cache.contains_key(&key) {
        if cache.len() > 16 {
            cache.clear();
        }
        let (tw, th) = (cols * cw, rows * ch);
        let Some(source) = to_pixmap(&image.pixels, (image.width, image.height)) else {
            return;
        };
        let Some(mut scaled) = Pixmap::new(tw, th) else {
            return;
        };
        let k = (tw as f32 / image.width as f32).min(th as f32 / image.height as f32);
        let (dw, dh) = (image.width as f32 * k, image.height as f32 * k);
        let paint = tiny_skia::PixmapPaint { quality: tiny_skia::FilterQuality::Bilinear, ..Default::default() };
        scaled.draw_pixmap(0, 0, source.as_ref(), &paint, Transform::from_row(k, 0.0, 0.0, k, (tw as f32 - dw) / 2.0, (th as f32 - dh) / 2.0), None);
        cache.insert(key, scaled);
    }
    let Some(scaled) = cache.get(&key) else {
        return;
    };
    let Some(rect) = tiny_skia::IntRect::from_xywh((col as u32 * cw) as i32, (row as u32 * ch) as i32, cw, ch) else {
        return;
    };
    if let Some(part) = scaled.clone_rect(rect) {
        pixmap.draw_pixmap(x.round() as i32, y.round() as i32, part.as_ref(), &tiny_skia::PixmapPaint::default(), Transform::identity(), None);
    }
}

/// Imagens do terminal visíveis no painel, recortadas na área dele.
fn draw_images(pixmap: &mut Pixmap, pane: &Pane, (x0, y0, w, h): Rect, cell: (f32, f32), top: u64, bottom: u64) {
    let visible: Vec<_> = pane.term.images().filter(|i| i.line <= bottom && i.line + i.rows as u64 > top).collect();
    let mut cache = pane.image_cache.borrow_mut();
    // Esquece as que já saíram do terminal.
    cache.retain(|id, _| pane.term.images().any(|i| i.id == *id));
    if visible.is_empty() {
        return;
    }
    let Some(mask) = clip_mask(pixmap.width(), pixmap.height(), (x0, y0, w, h)) else {
        return;
    };
    for image in visible {
        let source = cache.entry(image.id).or_insert_with(|| to_pixmap(&image.rgba, image.source).unwrap_or_else(|| Pixmap::new(1, 1).unwrap()));
        // O desenho usa o tamanho da célula do momento (a fonte pode ter mudado).
        let (cw, ch) = pane.term.cell_pixels;
        let k = (cell.0 / cw.max(1) as f32).min(cell.1 / ch.max(1) as f32);
        let (dw, dh) = (image.width as f32 * k, image.height as f32 * k);
        let x = x0 + PAD + image.col as f32 * cell.0;
        let y = y0 + PAD + (image.line as f32 - top as f32) * cell.1;
        let (sx, sy) = (dw / source.width() as f32, dh / source.height() as f32);
        let paint = tiny_skia::PixmapPaint { quality: tiny_skia::FilterQuality::Bilinear, ..Default::default() };
        pixmap.draw_pixmap(0, 0, source.as_ref(), &paint, Transform::from_row(sx, 0.0, 0.0, sy, x, y), Some(&mask));
    }
}

fn clip_mask(width: u32, height: u32, (x, y, w, h): Rect) -> Option<tiny_skia::Mask> {
    let mut mask = tiny_skia::Mask::new(width, height)?;
    let path = tiny_skia::PathBuilder::from_rect(SkRect::from_xywh(x, y, w, h)?);
    mask.fill_path(&path, tiny_skia::FillRule::Winding, false, Transform::identity());
    Some(mask)
}

/// RGBA comum (sem pré-multiplicar) para o formato do tiny-skia.
fn to_pixmap(rgba: &[u8], (w, h): (u32, u32)) -> Option<Pixmap> {
    let mut pixmap = Pixmap::new(w, h)?;
    for (dst, src) in pixmap.pixels_mut().iter_mut().zip(rgba.chunks_exact(4)) {
        let a = src[3] as u32;
        let p = |c: u8| (c as u32 * a / 255) as u8;
        *dst = tiny_skia::PremultipliedColorU8::from_rgba(p(src[0]), p(src[1]), p(src[2]), src[3])?;
    }
    Some(pixmap)
}

/// Ações do terminal (atalhos e menus).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Copy,
    Paste,
    SelectAll,
    NewWindow,
    CloseWindow,
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
    TmuxDetach,
}

/// Ações sem dados (as que aparecem nos menus por nome).
const SIMPLE: [Action; 27] = [
    Action::Copy,
    Action::Paste,
    Action::SelectAll,
    Action::NewWindow,
    Action::CloseWindow,
    Action::NewTab,
    Action::ClosePane,
    Action::SplitSide,
    Action::SplitStack,
    Action::Find,
    Action::FindNext,
    Action::FindPrevious,
    Action::Clear,
    Action::Preferences,
    Action::FontBigger,
    Action::FontSmaller,
    Action::FontReset,
    Action::Zoom,
    Action::NextTab,
    Action::PreviousTab,
    Action::NextPane,
    Action::PreviousPane,
    Action::Top,
    Action::Bottom,
    Action::PreviousPrompt,
    Action::NextPrompt,
    Action::TmuxDetach,
];

impl Action {
    /// Nome nos menus (o mesmo na barra superior e no botão direito).
    fn label(self) -> &'static str {
        match self {
            Action::Copy => "Copiar",
            Action::Paste => "Colar",
            Action::SelectAll => "Selecionar tudo",
            Action::NewWindow => "Nova janela",
            Action::CloseWindow => "Fechar janela",
            Action::NewTab => "Nova aba",
            Action::ClosePane => "Fechar painel",
            Action::SplitSide => "Dividir lado a lado",
            Action::SplitStack => "Dividir em cima e embaixo",
            Action::Find => "Buscar…",
            Action::FindNext => "Próxima ocorrência",
            Action::FindPrevious => "Ocorrência anterior",
            Action::Clear => "Limpar",
            Action::Preferences => "Preferências…",
            Action::FontBigger => "Aumentar fonte",
            Action::FontSmaller => "Diminuir fonte",
            Action::FontReset => "Tamanho normal",
            Action::Zoom => "Ampliar painel",
            Action::NextTab => "Próxima aba",
            Action::PreviousTab => "Aba anterior",
            Action::NextPane => "Próximo painel",
            Action::PreviousPane => "Painel anterior",
            Action::Top => "Início do histórico",
            Action::Bottom => "Fim do histórico",
            Action::PreviousPrompt => "Prompt anterior",
            Action::NextPrompt => "Próximo prompt",
            Action::TmuxDetach => "Desanexar do tmux",
            Action::Tab(_) | Action::PaneDirection(_) | Action::PageUp | Action::PageDown => "",
        }
    }

    /// Atalho mostrado nos menus, no estilo do teclado em uso (⌘ ou Ctrl+Shift).
    fn shortcut(self, mac: bool) -> &'static str {
        let (m, l) = match self {
            Action::Copy => ("Super+C", "Ctrl+Shift+C"),
            Action::Paste => ("Super+V", "Ctrl+Shift+V"),
            Action::SelectAll => ("Super+A", "Ctrl+Shift+A"),
            Action::NewWindow => ("Super+N", "Ctrl+Shift+N"),
            Action::CloseWindow => ("Super+Shift+W", ""),
            Action::NewTab => ("Super+T", "Ctrl+Shift+T"),
            Action::ClosePane => ("Super+W", "Ctrl+Shift+W"),
            Action::SplitSide => ("Super+D", "Ctrl+Shift+D"),
            Action::SplitStack => ("Super+Shift+D", "Ctrl+Shift+E"),
            Action::Find => ("Super+F", "Ctrl+Shift+F"),
            Action::FindNext => ("Super+G", "Ctrl+Shift+G"),
            Action::FindPrevious => ("Super+Shift+G", ""),
            Action::Clear => ("Super+K", "Ctrl+Shift+K"),
            Action::Preferences => ("Super+,", "Ctrl+Shift+,"),
            Action::FontBigger => ("Super+=", "Ctrl+Shift+="),
            Action::FontSmaller => ("Super+-", "Ctrl+Shift+-"),
            Action::FontReset => ("Super+0", "Ctrl+Shift+0"),
            Action::Zoom => ("Super+Shift+Return", "Ctrl+Shift+Return"),
            Action::NextTab => ("Super+Shift+]", "Ctrl+Page_Down"),
            Action::PreviousTab => ("Super+Shift+[", "Ctrl+Page_Up"),
            Action::NextPane => ("Super+]", ""),
            Action::PreviousPane => ("Super+[", ""),
            Action::Top => ("Shift+Home", "Shift+Home"),
            Action::Bottom => ("Shift+End", "Shift+End"),
            Action::PreviousPrompt => ("Super+Shift+Up", "Ctrl+Shift+Up"),
            Action::NextPrompt => ("Super+Shift+Down", "Ctrl+Shift+Down"),
            _ => ("", ""),
        };
        if mac { m } else { l }
    }

    fn id(self) -> String {
        match self {
            Action::Tab(n) => format!("aba:{n}"),
            other => format!("{other:?}"),
        }
    }

    fn from_id(id: &str) -> Option<Action> {
        if let Some(n) = id.strip_prefix("aba:") {
            return n.parse().ok().map(Action::Tab);
        }
        SIMPLE.into_iter().find(|a| a.id() == id)
    }

    fn item(self, mac: bool) -> MenuItem {
        MenuItem::action(&self.id(), self.label()).shortcut(self.shortcut(mac))
    }
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
        'a' => Action::SelectAll,
        'n' => Action::NewWindow,
        't' => Action::NewTab,
        'w' if shifted => Action::CloseWindow,
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

/// Cores da barra de abas: claras, ou escuras quando o tema do painel é escuro.
struct BarColors {
    bar: Rgb,
    active: Rgb,
    hover: Rgb,
    line: Rgb,
    text: [u8; 4],
    dim: [u8; 4],
}

fn bar_colors(background: Option<Rgb>) -> BarColors {
    let dark = background.is_some_and(|b| (b[0] as u32 * 299 + b[1] as u32 * 587 + b[2] as u32 * 114) / 1000 < 110);
    if dark {
        BarColors { bar: [36, 36, 42], active: [58, 58, 66], hover: [47, 47, 54], line: [70, 70, 80], text: [232, 232, 238, 255], dim: [160, 160, 170, 255] }
    } else {
        BarColors { bar: [222, 222, 227], active: [246, 246, 248], hover: [212, 212, 218], line: [196, 196, 204], text: [24, 24, 28, 255], dim: [90, 90, 98, 255] }
    }
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

/// Teclado no estilo Mac (Configurações → Teclado).
fn mac_style() -> bool {
    matches!(mide::settings_file::get("keyboard", "mac_style"), Some(mide::settings_file::Value::Bool(true)))
}

/// Tamanho de uma célula na fonte de largura fixa (largura média de 10 letras,
/// arredondada para as colunas caírem em pixels inteiros).
fn cell_size(font: f32, family: &str) -> (f32, f32) {
    let text = &mut Text::new();
    text.set_mono_family(Some(family));
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
    /// Cor sendo editada (posição em `COLOR_SLOTS`) e o texto do campo.
    color_slot: usize,
    color_text: String,
    /// Perfil mostrado no quadro anterior.
    shown: Option<usize>,
    /// Fontes de largura fixa instaladas.
    families: Vec<String>,
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
        // Outro perfil escolhido: o campo de cor mostra a cor dele.
        let profile_changed = self.shown != Some(selected);
        self.shown = Some(selected);
        let Profiles { profiles, restore, hotkey_height, .. } = &mut self.profiles;
        let profile = &mut profiles[selected];
        let (color_slot, color_text) = (&mut self.color_slot, &mut self.color_text);
        let families = &self.families;
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
                let mut fonts = vec!["Padrão do sistema".to_string()];
                fonts.extend(families.iter().cloned());
                let mut f = families.iter().position(|n| *n == profile.font).map_or(0, |i| i + 1);
                if ui.dropdown("Fonte", &fonts, &mut f).changed() {
                    profile.font = if f == 0 { String::new() } else { families[f - 1].clone() };
                    changed = true;
                }
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
            ui.heading("Transparência");
            ui.card(|ui| {
                let mut opacity = profile.opacity * 100.0;
                if ui.slider("Opacidade", &mut opacity, 30.0, 100.0).changed() {
                    profile.opacity = (opacity.round() / 100.0).clamp(0.3, 1.0);
                    changed = true;
                }
                changed |= ui.switch_detail("Vidro", "Desfoca o que está atrás do fundo translúcido", &mut profile.blur).changed();
            });
            ui.heading("Cores");
            ui.card(|ui| {
                let slots: Vec<String> = COLOR_SLOTS.iter().map(|(_, name)| name.to_string()).collect();
                let mut slot = *color_slot;
                if ui.dropdown("Cor", &slots, &mut slot).changed() || color_text.is_empty() || profile_changed {
                    *color_slot = slot;
                    *color_text = profile.color_hex(COLOR_SLOTS[slot].0);
                }
                let current = mide::terminal::profile::parse_hex(&profile.color_hex(COLOR_SLOTS[*color_slot].0)).unwrap_or([0, 0, 0]);
                ui.row(|ui| {
                    ui.sized(160.0, |ui| {
                        if ui.text_input(color_text, "#rrggbb").changed() {
                            if let Some(rgb) = mide::terminal::profile::parse_hex(color_text) {
                                profile.colors.insert(COLOR_SLOTS[*color_slot].0.into(), format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2]));
                                changed = true;
                            }
                        }
                    });
                    ui.custom(32.0, |pixmap, (x, y, _, h)| {
                        // Borda cinza: a amostra aparece mesmo branca no cartão branco.
                        mide::ui::draw::fill_rounded(pixmap, x, y, 48.0, h, 6.0, mide::ui::draw::rgba(170, 170, 178, 255));
                        mide::ui::draw::fill_rounded(pixmap, x + 1.0, y + 1.0, 46.0, h - 2.0, 5.0, mide::ui::draw::rgba(current[0], current[1], current[2], 255));
                    });
                });
                if !profile.colors.is_empty() && ui.button("Usar as cores do tema").clicked() {
                    profile.colors.clear();
                    *color_text = profile.color_hex(COLOR_SLOTS[*color_slot].0);
                    changed = true;
                }
            });
            ui.heading("Gatilhos");
            ui.card(|ui| {
                ui.note("Quando o texto aparece na saída (sem diferenciar maiúsculas nem acentos).");
                let actions = ["Destacar", "Notificar", "Piscar"].map(String::from);
                let mut remove = None;
                for (i, trigger) in profile.triggers.iter_mut().enumerate() {
                    ui.row(|ui| {
                        ui.sized(200.0, |ui| changed |= ui.text_input(&mut trigger.text, "Texto (ex.: erro)").changed());
                        let mut a = match trigger.action {
                            TriggerAction::Highlight => 0,
                            TriggerAction::Notify => 1,
                            TriggerAction::Bell => 2,
                        };
                        ui.sized(170.0, |ui| {
                            if ui.dropdown("", &actions, &mut a).changed() {
                                trigger.action = [TriggerAction::Highlight, TriggerAction::Notify, TriggerAction::Bell][a];
                                changed = true;
                            }
                        });
                        if ui.button("Remover").clicked() {
                            remove = Some(i);
                        }
                    });
                }
                if let Some(i) = remove {
                    profile.triggers.remove(i);
                    changed = true;
                }
                if ui.button("Novo gatilho").clicked() {
                    profile.triggers.push(Trigger::default());
                    changed = true;
                }
            });
            ui.heading("Sessão");
            ui.card(|ui| {
                let options: Vec<String> = SCROLLBACKS.iter().map(|n| format!("{} linhas", group(*n))).collect();
                let mut i = SCROLLBACKS.iter().position(|n| *n >= profile.scrollback).unwrap_or(2);
                if ui.dropdown("Histórico", &options, &mut i).changed() {
                    profile.scrollback = SCROLLBACKS[i];
                    changed = true;
                }
                changed |= ui.switch_detail("Reabrir abas e painéis", "Volta como estava a última janela fechada", restore).changed();
                let mut height = *hotkey_height * 100.0;
                if ui.slider("Altura do terminal de atalho (F12)", &mut height, 20.0, 95.0).changed() {
                    *hotkey_height = (height.round() / 100.0).clamp(0.2, 0.95);
                    changed = true;
                }
                let dirs = vec!["Na mesma pasta".to_string(), "Na pasta pessoal".to_string()];
                let mut d = if profile.start_dir == StartDir::Same { 0 } else { 1 };
                if ui.dropdown("Abas e painéis novos abrem", &dirs, &mut d).changed() {
                    profile.start_dir = if d == 0 { StartDir::Same } else { StartDir::Home };
                    changed = true;
                }
                changed |= ui
                    .switch_detail("Integração do shell", "Marca os prompts, avisa quando um comando termina e traz o imgcat (bash e zsh)", &mut profile.shell_integration)
                    .changed();
                let waits = ["Nunca", "5 segundos", "10 segundos", "30 segundos", "1 minuto"].map(String::from);
                const WAITS: [u32; 5] = [0, 5, 10, 30, 60];
                let mut w = WAITS.iter().position(|&s| s == profile.notify_after).unwrap_or(2);
                if ui.dropdown("Avisar comando demorado após", &waits, &mut w).changed() {
                    profile.notify_after = WAITS[w];
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
        return run(window, Preferences { profiles: Profiles::load(), selected: 0, message: None, color_slot: 1, color_text: String::new(), shown: None, families: Text::new().monospace_families() });
    }
    if options.hotkey {
        return run_hotkey(options);
    }
    let window = Window::new(TITLE).app_id("mide-terminal").size(700, 500).min_size(240, 120);
    run(window, TerminalApp::new(options))
}

/// Socket do terminal de atalho desta sessão (um por display Wayland).
fn hotkey_socket() -> PathBuf {
    let display = std::env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| "wayland-0".into());
    std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("mide").join(format!("terminal-atalho-{display}.sock"))
}

/// `mide-terminal --hotkey`: se o terminal de atalho já existe, pede para ele aparecer
/// ou sumir e sai; senão, vira o terminal de atalho (e atende os próximos pedidos).
fn run_hotkey(options: Options) -> Result<(), Box<dyn Error>> {
    use std::io::Write;
    use std::os::unix::net::{UnixListener, UnixStream};
    let path = hotkey_socket();
    if let Ok(mut stream) = UnixStream::connect(&path) {
        stream.write_all(b"alternar\n")?;
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path)?;
    let (tx, toggles) = mpsc::channel();
    let waker: std::sync::Arc<std::sync::Mutex<Option<Waker>>> = Default::default();
    let thread_waker = waker.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            drop(stream);
            if tx.send(()).is_err() {
                break;
            }
            if let Some(w) = thread_waker.lock().ok().and_then(|w| w.clone()) {
                w.wake();
            }
        }
    });
    let mut app = TerminalApp::new(options);
    app.hotkey = Some(Hotkey { toggles, waker, visible: true });
    let window = Window::new(TITLE).app_id("mide-terminal").dropdown("mide-terminal-atalho");
    let result = run(window, app);
    let _ = std::fs::remove_file(&path);
    result
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
        assert!(Options::parse(["--hotkey".to_string()]).hotkey);
    }

    #[test]
    fn acoes_dos_menus_vao_e_voltam() {
        for a in SIMPLE {
            assert_eq!(Action::from_id(&a.id()), Some(a), "{a:?}");
            assert!(!a.label().is_empty(), "{a:?}");
        }
        assert_eq!(Action::from_id("aba:2"), Some(Action::Tab(2)));
        assert_eq!(Action::NewTab.shortcut(true), "Super+T");
        assert_eq!(Action::NewTab.shortcut(false), "Ctrl+Shift+T");
    }

    #[test]
    fn duracao_por_extenso() {
        assert_eq!(duration_text(Duration::from_secs(42)), "42 s");
        assert_eq!(duration_text(Duration::from_secs(65)), "1 min 5 s");
        assert_eq!(duration_text(Duration::from_secs(3720)), "1 h 2 min");
    }

    #[test]
    fn milhar() {
        assert_eq!(group(1000), "1.000");
        assert_eq!(group(100000), "100.000");
    }
}
