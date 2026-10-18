//! O `Ui`: onde o app descreve a tela a cada quadro, no estilo "imediato" (como o
//! egui). Cada componente desenha a si mesmo, ocupa um lugar no layout e responde
//! se foi clicado ou mudou:
//!
//! ```ignore
//! ui.title("Olá");
//! if ui.button("Contar").clicked() {
//!     self.contagem += 1;
//! }
//! ui.switch("Modo avião", &mut self.aviao);
//! ```
//!
//! Layout: os componentes se empilham de cima para baixo; `row` põe lado a lado;
//! `card` agrupa num cartão branco; `page` é uma área com margem que rola; e
//! `sidebar` faz a barra lateral com seções, como nas Configurações do mide.

use std::collections::HashMap;

use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, PixmapPaint, Stroke, Transform};

use super::{
    input::{Input, Key},
    theme::{color, Rgba, Theme},
};
use crate::ui::{
    draw::{fill_rounded, rounded_rect},
    text::{FontWeight, Text},
};

/// Resposta de um componente neste quadro.
#[derive(Debug, Clone, Copy, Default)]
pub struct Response {
    /// Clicado (apertado e solto em cima dele).
    pub clicked: bool,
    /// Ponteiro em cima.
    pub hovered: bool,
    /// O valor mudou (interruptor, trilho, campo de texto).
    pub changed: bool,
    /// Enter num campo de texto.
    pub submitted: bool,
    /// Duplo clique (dois cliques seguidos no mesmo componente).
    pub double_clicked: bool,
    /// Clique com o botão direito (abre o menu de contexto).
    pub secondary_clicked: bool,
    /// Lugar ocupado (x, y, largura, altura), nas coordenadas da área atual.
    pub rect: (f32, f32, f32, f32),
    /// Número do componente neste quadro (para o menu de contexto saber de quem é).
    pub id: u64,
}

impl Response {
    pub fn clicked(&self) -> bool {
        self.clicked
    }
    pub fn changed(&self) -> bool {
        self.changed
    }
    pub fn submitted(&self) -> bool {
        self.submitted
    }
    pub fn hovered(&self) -> bool {
        self.hovered
    }
    pub fn double_clicked(&self) -> bool {
        self.double_clicked
    }
    pub fn secondary_clicked(&self) -> bool {
        self.secondary_clicked
    }
}

/// Imagem de um ladrilho (`tile`): ícone do tema pelo nome ou uma imagem pronta.
pub enum Tile<'p> {
    Icon(&'p str),
    Image(&'p Pixmap),
}

/// Menu de contexto aberto: de qual componente e onde (na janela).
#[derive(Clone, Copy)]
struct MenuState {
    owner: u64,
    pos: (f32, f32),
    height: f32,
}

/// Itens de um menu de contexto, montados dentro de `Ui::context_menu`.
pub struct Menu<'c> {
    ctx: &'c mut Ctx,
    x: f32,
    y: f32,
    w: f32,
    rows: Vec<(String, u8, bool)>,
}

impl Menu<'_> {
    fn row(&mut self, label: &str, kind: u8) -> bool {
        let h = if kind == 2 { 9.0 } else { 26.0 };
        let rect = (self.x, self.y, self.w, h);
        self.y += h;
        let hovered = kind != 2 && self.ctx.input.over(rect);
        self.rows.push((label.to_string(), kind, hovered));
        if hovered && self.ctx.input.pressed {
            // Escolhido: fecha o menu, e o clique não passa para o que está atrás.
            self.ctx.memory.menu = None;
            self.ctx.memory.menu_rect = None;
            self.ctx.repaint = true;
            self.ctx.input.pressed = false;
            return true;
        }
        false
    }

    /// Item comum; devolve `true` quando escolhido.
    pub fn item(&mut self, label: &str) -> bool {
        self.row(label, 0)
    }

    /// Item que apaga coisas (texto em vermelho).
    pub fn danger(&mut self, label: &str) -> bool {
        self.row(label, 1)
    }

    pub fn separator(&mut self) {
        self.row("", 2);
    }
}

/// O que dura entre quadros: componente apertado, foco do teclado e rolagens.
#[derive(Default)]
pub struct Memory {
    /// Componente onde o botão foi apertado (o clique vale se soltar nele).
    pub active: Option<u64>,
    /// Componente com o foco do teclado (campo de texto).
    pub focus: Option<u64>,
    /// Rolagem de cada área.
    pub scroll: HashMap<u64, f32>,
    /// Arrasto da barra de rolagem: área e ponto do clique dentro da alça.
    pub dragging_scroll: Option<(u64, f32)>,
    /// O próximo `text_input` desenhado recebe o foco (`Ui::focus_next_input`).
    focus_next: bool,
    /// O texto do campo com foco está todo selecionado (digitar substitui).
    select_all: bool,
    /// Último clique (componente e quando), para reconhecer o duplo clique.
    last_click: Option<(u64, std::time::Instant)>,
    /// Menu de contexto aberto e o retângulo dele no último quadro (na janela).
    menu: Option<MenuState>,
    menu_rect: Option<(f32, f32, f32, f32)>,
    menu_drawn: bool,
}

/// Tudo que os componentes compartilham num quadro.
pub struct Ctx {
    pub text: Text,
    pub theme: Theme,
    pub input: Input,
    pub memory: Memory,
    /// Algo mudou: desenhar de novo logo em seguida.
    pub repaint: bool,
    next_id: u64,
    /// Ícones do tema já carregados (compartilhados: desenhar não copia a imagem).
    icons: HashMap<String, Option<std::rc::Rc<Pixmap>>>,
    /// Camada por cima de tudo (menus de contexto) e o tamanho da janela.
    overlay: Option<Pixmap>,
    window: (u32, u32),
    /// Algum componente está sob o ponteiro neste quadro.
    hover_claimed: bool,
    /// O app pediu outro quadro em breve (`Ui::request_tick`).
    pub ticking: bool,
    /// Título pedido pelo app (`Ui::set_title`), aplicado pela janela.
    pub title: Option<String>,
}

impl Ctx {
    pub fn new(theme: Theme) -> Self {
        Self {
            text: Text::new(),
            theme,
            input: Input::default(),
            memory: Memory::default(),
            repaint: false,
            next_id: 1,
            icons: HashMap::new(),
            overlay: None,
            window: (0, 0),
            hover_claimed: false,
            ticking: false,
            title: None,
        }
    }

    /// Camada de cima (menus) deste quadro, se houver.
    pub fn take_overlay(&mut self) -> Option<Pixmap> {
        self.overlay.take()
    }

    /// Começo de um quadro: os ids voltam a contar do início.
    pub fn begin_frame(&mut self) {
        self.next_id = 1;
        self.repaint = false;
        self.hover_claimed = false;
        self.ticking = false;
        self.memory.menu_drawn = false;
        self.overlay = None;
        if self.input.key(Key::Escape) && self.memory.menu.is_some() {
            self.memory.menu = None;
            self.memory.menu_rect = None;
            self.input.keys.retain(|k| *k != Key::Escape);
            self.repaint = true;
        }
    }

    /// Fim de um quadro: solta o componente apertado e limpa a entrada do quadro.
    pub fn end_frame(&mut self) {
        // Menu aberto: clique fora fecha; se o dono dele sumiu, fecha também.
        if self.memory.menu.is_some() {
            let outside = (self.input.pressed || self.input.secondary)
                && !self.memory.menu_rect.is_some_and(|r| self.input.over(r));
            if !self.memory.menu_drawn || (outside && !self.input.secondary) {
                self.memory.menu = None;
                self.memory.menu_rect = None;
                self.repaint = true;
            }
        }
        if self.input.released || !self.input.down {
            self.memory.active = None;
            if self.input.released {
                self.memory.dragging_scroll = None;
            }
        }
        self.input.end_frame();
    }

    fn id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Flow {
    Column,
    Row,
    /// Lado a lado, quebrando linha quando acaba a largura (grade de ícones).
    Wrap,
}

/// Uma área de desenho com layout. Os componentes são métodos dela.
pub struct Ui<'a> {
    canvas: &'a mut Pixmap,
    ctx: &'a mut Ctx,
    /// Deslocamento desta área em relação à janela (para o ponteiro).
    offset: (f32, f32),
    /// Região disponível: x, largura e onde o próximo item vai.
    x: f32,
    width: f32,
    cursor_y: f32,
    /// Altura visível (para pular o desenho do que está fora da tela).
    visible: (f32, f32),
    flow: Flow,
    row_x: f32,
    row_h: f32,
    /// Dentro de um cartão: o próximo item é o primeiro (sem linha em cima)?
    in_card: bool,
    card_first: bool,
    /// Próxima `sidebar` entrega a área sem página (usado por `sidebar_area`).
    sidebar_raw: bool,
    /// Largura forçada para os componentes dentro de `sized`.
    next_width: Option<f32>,
    /// O próximo `icon_button` aparece marcado (usado por `icon_toggle`).
    toggled: bool,
}

impl<'a> Ui<'a> {
    /// Área da janela inteira.
    pub fn root(canvas: &'a mut Pixmap, ctx: &'a mut Ctx) -> Self {
        let (w, h) = (canvas.width() as f32, canvas.height() as f32);
        ctx.window = (canvas.width(), canvas.height());
        Self {
            canvas,
            ctx,
            offset: (0.0, 0.0),
            x: 0.0,
            width: w,
            cursor_y: 0.0,
            visible: (0.0, h),
            flow: Flow::Column,
            row_x: 0.0,
            row_h: 0.0,
            in_card: false,
            card_first: true,
            sidebar_raw: false,
            next_width: None,
            toggled: false,
        }
    }

    /// Tema em uso.
    pub fn theme(&self) -> &Theme {
        &self.ctx.theme
    }

    /// Largura disponível nesta área.
    pub fn available_width(&self) -> f32 {
        self.width
    }

    /// Pede para desenhar de novo (por exemplo, depois de mudar o estado fora de um clique).
    pub fn request_repaint(&mut self) {
        self.ctx.repaint = true;
    }

    /// Ponteiro nas coordenadas desta área.
    fn pointer(&self) -> Option<(f32, f32)> {
        self.ctx.input.pointer.map(|(x, y)| (x - self.offset.0, y - self.offset.1))
    }

    fn over(&self, rect: (f32, f32, f32, f32)) -> bool {
        self.pointer().is_some_and(|(px, py)| px >= rect.0 && px <= rect.0 + rect.2 && py >= rect.1 && py <= rect.1 + rect.3)
    }

    /// Reserva um lugar para o próximo item: em coluna ocupa a largura toda (ou
    /// `width`); em linha, vai para a direita do anterior.
    fn alloc(&mut self, width: Option<f32>, height: f32) -> (f32, f32, f32, f32) {
        let spacing = self.ctx.theme.spacing;
        let width = width.or(self.next_width);
        match self.flow {
            Flow::Column => {
                let w = width.unwrap_or(self.width).min(self.width);
                let rect = (self.x, self.cursor_y, w, height);
                self.cursor_y += height + spacing;
                rect
            }
            Flow::Row => {
                let w = width.unwrap_or(self.width - (self.row_x - self.x)).max(0.0);
                let rect = (self.row_x, self.cursor_y, w, height);
                self.row_x += w + spacing;
                self.row_h = self.row_h.max(height);
                rect
            }
            Flow::Wrap => {
                let w = width.unwrap_or(self.width).min(self.width);
                if self.row_x > self.x && self.row_x + w > self.x + self.width {
                    self.cursor_y += self.row_h + spacing;
                    self.row_x = self.x;
                    self.row_h = 0.0;
                }
                let rect = (self.row_x, self.cursor_y, w, height);
                self.row_x += w + spacing;
                self.row_h = self.row_h.max(height);
                rect
            }
        }
    }

    /// Clique: apertou neste componente e soltou em cima dele.
    fn interact(&mut self, id: u64, rect: (f32, f32, f32, f32)) -> Response {
        // Com um menu aberto, o que está debaixo dele não reage.
        let under_menu = self.ctx.memory.menu_rect.is_some_and(|r| self.ctx.input.over(r));
        let hovered = !under_menu && self.over(rect) && self.ctx.memory.active.is_none_or(|a| a == id);
        if hovered {
            self.ctx.hover_claimed = true;
        }
        if hovered && self.ctx.input.pressed {
            self.ctx.memory.active = Some(id);
        }
        let clicked = hovered && self.ctx.input.released && self.ctx.memory.active == Some(id);
        let mut double_clicked = false;
        if clicked {
            self.ctx.repaint = true;
            let now = std::time::Instant::now();
            double_clicked = self
                .ctx
                .memory
                .last_click
                .is_some_and(|(last, at)| last == id && now.duration_since(at) < std::time::Duration::from_millis(450));
            self.ctx.memory.last_click = if double_clicked { None } else { Some((id, now)) };
        }
        let secondary_clicked = hovered && self.ctx.input.secondary;
        Response { clicked, hovered, double_clicked, secondary_clicked, rect, id, ..Default::default() }
    }

    /// Resposta para uma área livre (por exemplo, o fundo de uma pasta), só se nenhum
    /// componente estiver sob o ponteiro neste quadro: clique direito no vazio.
    pub fn background(&mut self, rect: (f32, f32, f32, f32)) -> Response {
        let id = self.ctx.id();
        if self.ctx.hover_claimed {
            return Response { rect, id, ..Default::default() };
        }
        let under_menu = self.ctx.memory.menu_rect.is_some_and(|r| self.ctx.input.over(r));
        let hovered = !under_menu && self.over(rect);
        Response {
            hovered,
            clicked: hovered && self.ctx.input.pressed,
            secondary_clicked: hovered && self.ctx.input.secondary,
            rect,
            id,
            ..Default::default()
        }
    }

    /// Troca o título da janela (por exemplo, o nome da pasta aberta).
    pub fn set_title(&mut self, title: &str) {
        self.ctx.title = Some(title.to_string());
    }

    /// Pede outro quadro daqui a pouco (40 ms), mesmo sem mouse nem teclado: para
    /// mostrar trabalho feito em segundo plano. Vale só para o quadro atual.
    pub fn request_tick(&mut self) {
        self.ctx.ticking = true;
    }

    /// O próximo campo de texto desenhado já começa com o foco (para digitar).
    pub fn focus_next_input(&mut self) {
        self.ctx.memory.focus_next = true;
    }

    /// Algum campo de texto está com o foco (as teclas são dele).
    pub fn typing(&self) -> bool {
        self.ctx.memory.focus.is_some()
    }

    /// Numa linha (`row`, `toolbar`), pula para que os próximos `width` pixels
    /// terminem na borda direita.
    pub fn push_right(&mut self, width: f32) {
        if self.flow != Flow::Column {
            let end = self.x + self.width - width;
            self.row_x = self.row_x.max(end);
        }
    }

    /// Faz o próximo componente ocupar `width` de largura (numa linha).
    pub fn sized(&mut self, width: f32, add: impl FnOnce(&mut Ui)) {
        let old = self.next_width.replace(width);
        add(self);
        self.next_width = old;
    }

    /// Texto numa linha só, do tamanho dele (por exemplo, o nome da pasta numa barra
    /// de ferramentas). Corta com "…" se não couber.
    pub fn strong(&mut self, s: &str) {
        let t = self.ctx.theme.clone();
        let size = t.font + 1.0;
        let room = if self.flow == Flow::Column { self.width } else { self.width - (self.row_x - self.x) };
        let shown = self.fit(s, size, FontWeight::Semibold, room.max(0.0));
        let w = self.measure(&shown, size, FontWeight::Semibold);
        let rect = self.alloc(Some(w), 28.0);
        self.draw_text(&shown, rect.0, rect.1 + (28.0 - size * 1.25) / 2.0, size, FontWeight::Semibold, t.text);
    }

    /// Parte visível desta região (x, y, largura, altura), por exemplo para o fundo de
    /// uma pasta reagir ao clique no vazio.
    pub fn region(&self) -> (f32, f32, f32, f32) {
        (self.x, self.visible.0, self.width, self.visible.1 - self.visible.0)
    }

    /// Entrada deste quadro (teclas, atalhos com Ctrl).
    pub fn input(&self) -> &Input {
        &self.ctx.input
    }

    /// Menu de contexto do componente `response`: abre com o clique direito nele e
    /// fica por cima de tudo. Os itens são montados no `build`:
    ///
    /// ```ignore
    /// let r = ui.list_item("foto.jpg", "");
    /// ui.context_menu(&r, |m| {
    ///     if m.item("Abrir") { /* ... */ }
    ///     m.separator();
    ///     if m.danger("Mover para a Lixeira") { /* ... */ }
    /// });
    /// ```
    pub fn context_menu(&mut self, response: &Response, build: impl FnOnce(&mut Menu)) {
        if response.secondary_clicked {
            if let Some(pos) = self.ctx.input.pointer {
                self.ctx.memory.menu = Some(MenuState { owner: response.id, pos, height: 0.0 });
                self.ctx.memory.menu_rect = None;
                self.ctx.repaint = true;
            }
        }
        let Some(state) = self.ctx.memory.menu.filter(|m| m.owner == response.id) else {
            return;
        };
        self.ctx.memory.menu_drawn = true;
        let t = self.ctx.theme.clone();
        let (ww, wh) = (self.ctx.window.0 as f32, self.ctx.window.1 as f32);
        let w = 220.0;
        // Dentro da janela: abre para o lado e para cima se não couber.
        let x = if state.pos.0 + w + 6.0 > ww { (state.pos.0 - w).max(4.0) } else { state.pos.0 + 2.0 };
        let y0 = if state.pos.1 + state.height + 6.0 > wh { (state.pos.1 - state.height).max(4.0) } else { state.pos.1 };
        let mut menu = Menu { ctx: &mut *self.ctx, x: x + 5.0, y: y0 + 5.0, w: w - 10.0, rows: Vec::new() };
        build(&mut menu);
        let (rows, end) = (std::mem::take(&mut menu.rows), menu.y);
        let still_open = self.ctx.memory.menu.is_some();
        let h = end - y0 + 5.0;
        if !still_open {
            return;
        }
        if let Some(m) = self.ctx.memory.menu.as_mut() {
            m.height = h;
        }
        self.ctx.memory.menu_rect = Some((x, y0, w, h));
        // Desenho na camada de cima.
        let overlay = self.ctx.overlay.get_or_insert_with(|| Pixmap::new(ww.max(1.0) as u32, wh.max(1.0) as u32).unwrap_or_else(|| Pixmap::new(1, 1).unwrap()));
        for (grow, alpha) in [(6.0, 12u8), (3.0, 20), (1.0, 32)] {
            fill_rounded(overlay, x - grow, y0 - grow + 4.0, w + 2.0 * grow, h + 2.0 * grow, 10.0 + grow, color([0, 0, 0, alpha]));
        }
        fill_rounded(overlay, x, y0, w, h, 10.0, color([246, 246, 248, 245]));
        let mut y = y0 + 5.0;
        let text = &mut self.ctx.text;
        for (label, kind, hovered) in rows {
            if kind == 2 {
                fill_rounded(overlay, x + 12.0, y + 4.0, w - 24.0, 1.0, 0.0, color([0, 0, 0, 30]));
                y += 9.0;
                continue;
            }
            if hovered {
                fill_rounded(overlay, x + 5.0, y, w - 10.0, 26.0, 6.0, color(t.accent));
            }
            let c = match (kind, hovered) {
                (_, true) => t.on_accent,
                (1, false) => [210, 40, 40, 255],
                _ => t.text,
            };
            text.draw(overlay, &label, x + 16.0, y + (26.0 - t.font * 1.25) / 2.0, t.font, FontWeight::Regular, c);
            y += 26.0;
        }
    }

    fn visible(&self, rect: (f32, f32, f32, f32)) -> bool {
        rect.1 + rect.3 >= self.visible.0 && rect.1 <= self.visible.1
    }

    /// Linha fina entre itens de um mesmo cartão.
    fn card_line(&mut self, y: f32) {
        if self.in_card {
            if !self.card_first {
                let c = self.ctx.theme.line;
                fill_rounded(self.canvas, self.x, y - self.ctx.theme.spacing / 2.0, self.width, 1.0, 0.0, color(c));
            }
            self.card_first = false;
        }
    }

    fn draw_text(&mut self, s: &str, x: f32, y: f32, size: f32, weight: FontWeight, c: Rgba) -> f32 {
        self.ctx.text.draw(self.canvas, s, x, y, size, weight, c)
    }

    fn measure(&mut self, s: &str, size: f32, weight: FontWeight) -> f32 {
        self.ctx.text.measure(s, size, weight)
    }

    /// Corta o texto com reticências para caber em `max`.
    fn fit(&mut self, s: &str, size: f32, weight: FontWeight, max: f32) -> String {
        if self.measure(s, size, weight) <= max {
            return s.to_string();
        }
        let mut chars: Vec<char> = s.chars().collect();
        while !chars.is_empty() {
            chars.pop();
            let c = format!("{}…", chars.iter().collect::<String>().trim_end());
            if self.measure(&c, size, weight) <= max {
                return c;
            }
        }
        "…".into()
    }

    // ---------------------------------------------------------------- textos

    /// Título grande, em negrito.
    pub fn title(&mut self, s: &str) {
        let t = self.ctx.theme.clone();
        let rect = self.alloc(None, t.font_title * 1.5);
        let s = self.fit(s, t.font_title, FontWeight::Bold, rect.2);
        self.draw_text(&s, rect.0, rect.1, t.font_title, FontWeight::Bold, t.text);
    }

    /// Nome de um grupo, pequeno e cinza, acima de um cartão.
    pub fn heading(&mut self, s: &str) {
        let t = self.ctx.theme.clone();
        let rect = self.alloc(None, t.font_small * 1.4);
        self.draw_text(s, rect.0 + 4.0, rect.1, t.font_small, FontWeight::Semibold, t.text_secondary);
    }

    /// Texto comum (quebrado em linhas se não couber).
    pub fn label(&mut self, s: &str) {
        let t = self.ctx.theme.clone();
        self.paragraph(s, t.font, t.text);
    }

    /// Texto menor e cinza, para explicações.
    pub fn note(&mut self, s: &str) {
        let t = self.ctx.theme.clone();
        self.paragraph(s, t.font_small, t.text_secondary);
    }

    fn paragraph(&mut self, s: &str, size: f32, c: Rgba) {
        let max = if self.flow == Flow::Row { f32::MAX } else { self.width };
        let mut lines: Vec<String> = Vec::new();
        for para in s.split('\n') {
            let mut current = String::new();
            for word in para.split_whitespace() {
                let candidate = if current.is_empty() { word.to_string() } else { format!("{current} {word}") };
                if self.measure(&candidate, size, FontWeight::Regular) > max && !current.is_empty() {
                    lines.push(std::mem::take(&mut current));
                    current = word.to_string();
                } else {
                    current = candidate;
                }
            }
            lines.push(current);
        }
        let line_h = size * 1.4;
        let natural = lines.iter().map(|l| self.measure(l, size, FontWeight::Regular)).fold(0.0, f32::max);
        let width = if self.flow == Flow::Row { Some(natural) } else { None };
        let rect = self.alloc(width, line_h * lines.len() as f32);
        self.card_line(rect.1);
        if self.visible(rect) {
            for (i, line) in lines.iter().enumerate() {
                self.draw_text(line, rect.0, rect.1 + i as f32 * line_h, size, FontWeight::Regular, c);
            }
        }
    }

    // ---------------------------------------------------------------- controles

    fn button_impl(&mut self, s: &str, accent: bool) -> Response {
        let t = self.ctx.theme.clone();
        let id = self.ctx.id();
        let w = self.measure(s, t.font, FontWeight::Semibold) + 32.0;
        let rect = self.alloc(Some(w), t.row);
        let r = self.interact(id, rect);
        let pressed = self.ctx.memory.active == Some(id) && r.hovered;
        let fill = match (accent, r.hovered, pressed) {
            (true, _, true) => [t.accent[0].saturating_sub(30), t.accent[1].saturating_sub(30), t.accent[2].saturating_sub(30), 255],
            (true, _, false) => t.accent,
            (false, _, true) => [0, 0, 0, 46],
            (false, true, false) => t.hover,
            (false, false, false) => t.control,
        };
        fill_rounded(self.canvas, rect.0, rect.1, rect.2, rect.3, t.control_radius, color(fill));
        let tc = if accent { t.on_accent } else { t.text };
        let tw = self.measure(s, t.font, FontWeight::Semibold);
        self.draw_text(s, rect.0 + (rect.2 - tw) / 2.0, rect.1 + (rect.3 - t.font * 1.25) / 2.0, t.font, FontWeight::Semibold, tc);
        r
    }

    /// Botão comum.
    pub fn button(&mut self, s: &str) -> Response {
        self.button_impl(s, false)
    }

    /// Botão de destaque (ação principal), na cor do tema.
    pub fn primary_button(&mut self, s: &str) -> Response {
        self.button_impl(s, true)
    }

    /// Linha com um texto à esquerda e um interruptor à direita.
    pub fn switch(&mut self, label: &str, value: &mut bool) -> Response {
        let t = self.ctx.theme.clone();
        let id = self.ctx.id();
        let rect = self.alloc(None, t.row);
        self.card_line(rect.1);
        let mut r = self.interact(id, rect);
        if r.clicked {
            *value = !*value;
            r.changed = true;
        }
        let label = self.fit(label, t.font, FontWeight::Regular, rect.2 - 60.0);
        self.draw_text(&label, rect.0, rect.1 + (rect.3 - t.font * 1.25) / 2.0, t.font, FontWeight::Regular, t.text);
        let (sx, sy) = (rect.0 + rect.2 - 40.0, rect.1 + (rect.3 - 22.0) / 2.0);
        let fill = if *value { t.accent } else { [0, 0, 0, 40] };
        fill_rounded(self.canvas, sx, sy, 40.0, 22.0, 11.0, color(fill));
        let kx = if *value { sx + 29.0 } else { sx + 11.0 };
        if let Some(knob) = PathBuilder::from_circle(kx, sy + 11.0, 9.0) {
            let mut paint = Paint::default();
            paint.anti_alias = true;
            paint.set_color(color([255, 255, 255, 255]));
            self.canvas.fill_path(&knob, &paint, FillRule::Winding, Transform::identity(), None);
        }
        r
    }

    /// Linha com um texto e um trilho para escolher um valor entre `min` e `max`.
    pub fn slider(&mut self, label: &str, value: &mut f32, min: f32, max: f32) -> Response {
        let t = self.ctx.theme.clone();
        let id = self.ctx.id();
        let rect = self.alloc(None, t.row);
        self.card_line(rect.1);
        let label_w = (rect.2 * 0.35).min(180.0);
        let (tx, tw) = (rect.0 + label_w, rect.2 - label_w - 50.0);
        let track = (tx - 8.0, rect.1, tw + 16.0, rect.3);
        let mut r = self.interact(id, track);
        if self.ctx.memory.active == Some(id) && self.ctx.input.down {
            if let Some((px, _)) = self.pointer() {
                let k = ((px - tx) / tw).clamp(0.0, 1.0);
                let new = min + k * (max - min);
                if (new - *value).abs() > f32::EPSILON {
                    *value = new;
                    r.changed = true;
                    self.ctx.repaint = true;
                }
            }
        }
        let label = self.fit(label, t.font, FontWeight::Regular, label_w - 12.0);
        self.draw_text(&label, rect.0, rect.1 + (rect.3 - t.font * 1.25) / 2.0, t.font, FontWeight::Regular, t.text);
        let k = ((*value - min) / (max - min).max(f32::EPSILON)).clamp(0.0, 1.0);
        let cy = rect.1 + rect.3 / 2.0;
        fill_rounded(self.canvas, tx, cy - 3.0, tw, 6.0, 3.0, color([0, 0, 0, 30]));
        fill_rounded(self.canvas, tx, cy - 3.0, tw * k, 6.0, 3.0, color(t.accent));
        if let Some(knob) = PathBuilder::from_circle(tx + tw * k, cy, 9.0) {
            let mut paint = Paint::default();
            paint.anti_alias = true;
            paint.set_color(color([255, 255, 255, 255]));
            self.canvas.fill_path(&knob, &paint, FillRule::Winding, Transform::identity(), None);
            paint.set_color(color([0, 0, 0, 60]));
            self.canvas.stroke_path(&knob, &paint, &Stroke::default(), Transform::identity(), None);
        }
        let shown = format!("{}", value.round() as i64);
        self.draw_text(&shown, tx + tw + 16.0, rect.1 + (rect.3 - t.font_small * 1.25) / 2.0, t.font_small, FontWeight::Regular, t.text_secondary);
        r
    }

    /// Campo de texto de uma linha. Clique dá o foco; Enter responde `submitted`.
    pub fn text_input(&mut self, value: &mut String, placeholder: &str) -> Response {
        let t = self.ctx.theme.clone();
        let id = self.ctx.id();
        let rect = self.alloc(None, t.row);
        let mut r = self.interact(id, rect);
        if std::mem::take(&mut self.ctx.memory.focus_next) {
            self.ctx.memory.focus = Some(id);
            self.ctx.memory.select_all = !value.is_empty();
            self.ctx.repaint = true;
        } else if r.clicked {
            self.ctx.memory.focus = Some(id);
            self.ctx.memory.select_all = false;
            self.ctx.repaint = true;
        } else if self.ctx.input.pressed && !r.hovered && self.ctx.memory.focus == Some(id) {
            self.ctx.memory.focus = None;
        }
        let focused = self.ctx.memory.focus == Some(id);
        if focused {
            // Tudo selecionado: digitar ou apagar troca o texto inteiro.
            let all = self.ctx.memory.select_all;
            if !self.ctx.input.text.is_empty() {
                if all {
                    value.clear();
                    self.ctx.memory.select_all = false;
                }
                value.push_str(&self.ctx.input.text);
                r.changed = true;
            }
            for key in self.ctx.input.keys.clone() {
                match key {
                    Key::Backspace | Key::Delete if self.ctx.memory.select_all => {
                        r.changed |= !value.is_empty();
                        value.clear();
                        self.ctx.memory.select_all = false;
                    }
                    Key::Left | Key::Right => self.ctx.memory.select_all = false,
                    Key::Backspace => {
                        r.changed |= value.pop().is_some();
                    }
                    Key::Enter => r.submitted = true,
                    Key::Escape => self.ctx.memory.focus = None,
                    _ => {}
                }
            }
            if r.changed || r.submitted {
                self.ctx.repaint = true;
            }
        }
        fill_rounded(self.canvas, rect.0, rect.1, rect.2, rect.3, t.control_radius, color(t.card));
        let border = if focused { t.accent } else { [0, 0, 0, 40] };
        if let Some(path) = rounded_rect(rect.0 + 0.5, rect.1 + 0.5, rect.2 - 1.0, rect.3 - 1.0, t.control_radius) {
            let mut paint = Paint::default();
            paint.anti_alias = true;
            paint.set_color(color(border));
            let width = if focused { 2.0 } else { 1.0 };
            self.canvas.stroke_path(&path, &paint, &Stroke { width, ..Default::default() }, Transform::identity(), None);
        }
        let ty = rect.1 + (rect.3 - t.font * 1.25) / 2.0;
        if value.is_empty() && !focused {
            self.draw_text(placeholder, rect.0 + 10.0, ty, t.font, FontWeight::Regular, t.text_secondary);
        } else {
            let shown = self.fit(value, t.font, FontWeight::Regular, rect.2 - 24.0);
            if focused && self.ctx.memory.select_all {
                let sw = self.measure(&shown, t.font, FontWeight::Regular);
                let [r_, g, b, _] = t.accent;
                fill_rounded(self.canvas, rect.0 + 8.0, rect.1 + 6.0, sw + 4.0, rect.3 - 12.0, 3.0, color([r_, g, b, 70]));
            }
            let w = self.draw_text(&shown, rect.0 + 10.0, ty, t.font, FontWeight::Regular, t.text);
            if focused && !self.ctx.memory.select_all {
                fill_rounded(self.canvas, rect.0 + 11.0 + w, rect.1 + 7.0, 1.5, rect.3 - 14.0, 0.5, color(t.text));
            }
        }
        r
    }

    /// Item de lista clicável: texto à esquerda, detalhe cinza à direita.
    pub fn list_item(&mut self, label: &str, detail: &str) -> Response {
        let t = self.ctx.theme.clone();
        let id = self.ctx.id();
        let rect = self.alloc(None, t.row);
        self.card_line(rect.1);
        let r = self.interact(id, (rect.0 - 8.0, rect.1, rect.2 + 16.0, rect.3));
        if r.hovered {
            fill_rounded(self.canvas, rect.0 - 8.0, rect.1, rect.2 + 16.0, rect.3, t.control_radius, color(t.hover));
        }
        let dw = if detail.is_empty() { 0.0 } else { self.measure(detail, t.font_small, FontWeight::Regular) + 12.0 };
        let label = self.fit(label, t.font, FontWeight::Regular, rect.2 - dw);
        self.draw_text(&label, rect.0, rect.1 + (rect.3 - t.font * 1.25) / 2.0, t.font, FontWeight::Regular, t.text);
        if !detail.is_empty() {
            self.draw_text(detail, rect.0 + rect.2 - dw + 12.0, rect.1 + (rect.3 - t.font_small * 1.25) / 2.0, t.font_small, FontWeight::Regular, t.text_secondary);
        }
        r
    }

    /// Ícone do tema do usuário pelo nome (por exemplo, "folder" ou "audio-volume-high").
    pub fn icon(&mut self, name: &str, size: u32) {
        let icon = self
            .ctx
            .icons
            .entry(format!("{name}@{size}"))
            .or_insert_with(|| crate::icons::load_themed_icon(&[name], size).map(std::rc::Rc::new))
            .clone();
        let rect = self.alloc(Some(size as f32), size as f32);
        if let Some(icon) = icon {
            crate::ui::draw::blit(self.canvas, &icon, rect.0 as i32, rect.1 as i32, 1.0);
        }
    }

    /// Ladrilho de uma grade de ícones (como no Finder): imagem com o nome embaixo.
    /// Selecionado, ganha o vidro do mide atrás e o nome numa pílula na cor de destaque.
    pub fn tile(&mut self, image: Tile, label: &str, selected: bool, size: u32) -> Response {
        let t = self.ctx.theme.clone();
        let id = self.ctx.id();
        let (w, h) = (size as f32 + 44.0, size as f32 + 46.0);
        let rect = self.alloc(Some(w), h);
        let r = self.interact(id, rect);
        if !self.visible(rect) {
            return r;
        }
        let (ix, iy) = (rect.0 + (w - size as f32) / 2.0, rect.1 + 6.0);
        if selected {
            crate::ui::draw::glass(self.canvas, ix - 8.0, iy - 6.0, size as f32 + 16.0, size as f32 + 12.0, 10.0);
        } else if r.hovered {
            fill_rounded(self.canvas, ix - 8.0, iy - 6.0, size as f32 + 16.0, size as f32 + 12.0, 10.0, color(t.control));
        }
        match image {
            Tile::Image(pix) => {
                let (pw, ph) = (pix.width() as f32, pix.height() as f32);
                let (dx, dy) = (ix + (size as f32 - pw) / 2.0, iy + (size as f32 - ph) / 2.0);
                crate::ui::draw::blit(self.canvas, pix, dx as i32, dy as i32, 1.0);
            }
            Tile::Icon(name) => {
                let key = format!("{name}@{size}");
                let icon = self.ctx.icons.entry(key).or_insert_with(|| crate::icons::load_themed_icon(&[name, "text-x-generic"], size).map(std::rc::Rc::new)).clone();
                if let Some(icon) = icon {
                    crate::ui::draw::blit(self.canvas, &icon, ix as i32, iy as i32, 1.0);
                }
            }
        }
        let max = w - 6.0;
        let shown = self.fit(label, t.font_small, FontWeight::Regular, max);
        let lw = self.measure(&shown, t.font_small, FontWeight::Regular);
        let (lx, ly) = (rect.0 + (w - lw) / 2.0, iy + size as f32 + 10.0);
        if selected {
            fill_rounded(self.canvas, lx - 5.0, ly - 1.0, lw + 10.0, t.font_small * 1.4, 5.0, color(t.accent));
        }
        let c = if selected { t.on_accent } else { t.text };
        self.draw_text(&shown, lx, ly, t.font_small, FontWeight::Regular, c);
        r
    }

    /// Linha de uma lista de arquivos: ícone, nome e colunas de detalhe à direita
    /// (cada uma com sua largura). Selecionada, fica na cor de destaque.
    pub fn list_row(&mut self, icon: &str, label: &str, columns: &[(&str, f32)], selected: bool) -> Response {
        let t = self.ctx.theme.clone();
        let id = self.ctx.id();
        let rect = self.alloc(None, 28.0);
        let r = self.interact(id, rect);
        if !self.visible(rect) {
            return r;
        }
        if selected {
            fill_rounded(self.canvas, rect.0, rect.1, rect.2, rect.3, 6.0, color(t.accent));
        } else if r.hovered {
            fill_rounded(self.canvas, rect.0, rect.1, rect.2, rect.3, 6.0, color(t.hover));
        }
        let key = format!("{icon}@20");
        let pix = self.ctx.icons.entry(key).or_insert_with(|| crate::icons::load_themed_icon(&[icon, "text-x-generic"], 20).map(std::rc::Rc::new)).clone();
        if let Some(pix) = pix {
            crate::ui::draw::blit(self.canvas, &pix, (rect.0 + 8.0) as i32, (rect.1 + 4.0) as i32, 1.0);
        }
        let cols: f32 = columns.iter().map(|c| c.1).sum();
        let ty = rect.1 + (28.0 - t.font * 1.25) / 2.0;
        let (main, side) = if selected { (t.on_accent, t.on_accent) } else { (t.text, t.text_secondary) };
        let shown = self.fit(label, t.font, FontWeight::Regular, (rect.2 - 40.0 - cols).max(0.0));
        self.draw_text(&shown, rect.0 + 36.0, ty, t.font, FontWeight::Regular, main);
        let mut cx = rect.0 + rect.2 - cols;
        for (text, w) in columns {
            let shown = self.fit(text, t.font_small, FontWeight::Regular, w - 8.0);
            self.draw_text(&shown, cx, rect.1 + (28.0 - t.font_small * 1.25) / 2.0, t.font_small, FontWeight::Regular, side);
            cx += w;
        }
        r
    }

    /// Botão pequeno com um ícone do tema (e um texto de reserva se o ícone faltar).
    pub fn icon_button(&mut self, icon: &str, fallback: &str, enabled: bool) -> Response {
        let t = self.ctx.theme.clone();
        let id = self.ctx.id();
        let rect = self.alloc(Some(32.0), 28.0);
        let mut r = self.interact(id, rect);
        if !enabled {
            r.clicked = false;
            r.double_clicked = false;
        }
        if std::mem::take(&mut self.toggled) {
            fill_rounded(self.canvas, rect.0, rect.1, rect.2, rect.3, t.control_radius, color(t.control));
        } else if enabled && r.hovered {
            fill_rounded(self.canvas, rect.0, rect.1, rect.2, rect.3, t.control_radius, color(t.hover));
        }
        let key = format!("{icon}@16");
        let pix = self.ctx.icons.entry(key).or_insert_with(|| crate::icons::load_themed_icon(&[icon], 16).map(std::rc::Rc::new)).clone();
        match pix {
            Some(pix) => {
                crate::ui::draw::blit(self.canvas, &pix, (rect.0 + 8.0) as i32, (rect.1 + 6.0) as i32, if enabled { 1.0 } else { 0.35 });
            }
            None => {
                let c = if enabled { t.text } else { t.text_secondary };
                let fw = self.measure(fallback, 16.0, FontWeight::Semibold);
                self.draw_text(fallback, rect.0 + (32.0 - fw) / 2.0, rect.1 + 3.0, 16.0, FontWeight::Semibold, c);
            }
        }
        r
    }

    /// Botão com ícone que fica marcado quando `on` (escolha de visão: ícones ou lista).
    pub fn icon_toggle(&mut self, icon: &str, fallback: &str, on: bool) -> Response {
        self.toggled = on;
        let r = self.icon_button(icon, fallback, true);
        self.toggled = false;
        r
    }

    /// Barra de ferramentas no topo da área: faixa com os itens lado a lado,
    /// centralizados na altura, e uma linha fina embaixo.
    pub fn toolbar(&mut self, add: impl FnOnce(&mut Ui)) {
        let t = self.ctx.theme.clone();
        let (x, y0, w, h) = (self.x, self.cursor_y, self.width, 52.0);
        fill_rounded(self.canvas, x, y0 + h - 1.0, w, 1.0, 0.0, color(t.line));
        let (old_flow, old_x, old_w, old_h) = (self.flow, self.row_x, self.x, self.row_h);
        self.flow = Flow::Row;
        self.x = x + 12.0;
        self.row_x = x + 12.0;
        self.row_h = 0.0;
        let old_y = self.cursor_y;
        self.cursor_y = y0 + (h - 28.0) / 2.0;
        let old_width = self.width;
        self.width = w - 24.0;
        add(self);
        self.flow = old_flow;
        self.row_x = old_x;
        self.x = old_w;
        self.row_h = old_h;
        self.width = old_width;
        self.cursor_y = old_y + h;
    }

    /// Grade que quebra linha: os itens vão lado a lado e descem quando a largura acaba.
    pub fn wrap(&mut self, add: impl FnOnce(&mut Ui)) {
        let (old_flow, old_x, old_h) = (self.flow, self.row_x, self.row_h);
        self.flow = Flow::Wrap;
        self.row_x = self.x;
        self.row_h = 0.0;
        add(self);
        let h = self.row_h;
        self.flow = old_flow;
        self.row_x = old_x;
        self.row_h = old_h;
        self.cursor_y += h + self.ctx.theme.spacing;
    }

    /// Linha fina horizontal.
    pub fn separator(&mut self) {
        let c = self.ctx.theme.line;
        let rect = self.alloc(None, 1.0);
        fill_rounded(self.canvas, rect.0, rect.1, rect.2, 1.0, 0.0, color(c));
    }

    /// Espaço vazio na vertical.
    pub fn space(&mut self, height: f32) {
        self.cursor_y += height;
    }

    // ---------------------------------------------------------------- layout

    /// Itens lado a lado, da esquerda para a direita.
    pub fn row(&mut self, add: impl FnOnce(&mut Ui)) {
        let (old_flow, old_x, old_h) = (self.flow, self.row_x, self.row_h);
        self.flow = Flow::Row;
        self.row_x = self.x;
        self.row_h = 0.0;
        add(self);
        let h = self.row_h;
        self.flow = old_flow;
        self.row_x = old_x;
        self.row_h = old_h;
        self.cursor_y += h + self.ctx.theme.spacing;
    }

    /// Desenha `add` numa área à parte de largura `width`, com o ponteiro deslocado,
    /// e devolve a imagem e a altura usada.
    /// Área com conteúdo de altura desconhecida (cartão, página), desenhada numa imagem
    /// à parte que cobre só a faixa visível: da linha `top` (nesta área) com `height`
    /// de altura e `width` de largura, colada depois em (`x`, `top`). O conteúdo
    /// começa na linha `start` (nesta área) e pode passar da faixa: o que fica fora
    /// não é desenhado (os componentes conferem `visible`). Devolve a imagem e a altura
    /// usada pelo conteúdo.
    fn sub_area(&mut self, x: f32, top: f32, width: f32, height: f32, start: f32, add: impl FnOnce(&mut Ui)) -> (Pixmap, f32) {
        self.sub_area_from(x, top, width, height, start, false, add)
    }

    /// Como `sub_area`; com `copy`, a imagem começa com os pixels que já estão na área
    /// (para depois voltar com `copy_back`, sem misturar transparência: bem mais rápido
    /// que colar com `draw_pixmap` numa área grande).
    #[allow(clippy::too_many_arguments)]
    fn sub_area_from(&mut self, x: f32, top: f32, width: f32, height: f32, start: f32, copy: bool, add: impl FnOnce(&mut Ui)) -> (Pixmap, f32) {
        let h = height.clamp(1.0, 16384.0) as u32;
        let mut canvas = Pixmap::new(width.max(1.0) as u32, h).unwrap_or_else(|| Pixmap::new(1, 1).unwrap());
        if copy {
            copy_pixels(self.canvas, x as i32, top as i32, &mut canvas, 0, 0);
        }
        let first = start - top;
        let used = {
            let mut child = Ui {
                canvas: &mut canvas,
                ctx: &mut *self.ctx,
                offset: (self.offset.0 + x, self.offset.1 + top),
                x: 0.0,
                width,
                cursor_y: first,
                visible: (0.0, height.max(0.0)),
                flow: Flow::Column,
                row_x: 0.0,
                row_h: 0.0,
                in_card: false,
                card_first: true,
                sidebar_raw: false,
                next_width: None,
                toggled: false,
            };
            add(&mut child);
            child.cursor_y - first
        };
        (canvas, used)
    }

    /// Cartão branco de cantos arredondados; os itens dentro ganham linhas finas entre si.
    pub fn card(&mut self, add: impl FnOnce(&mut Ui)) {
        let t = self.ctx.theme.clone();
        let (pad_x, pad_y) = (16.0, 6.0);
        let x = self.x;
        let inner_w = self.width - 2.0 * pad_x;
        let y0 = self.cursor_y;
        // Só a parte do cartão que aparece na tela ganha imagem.
        let top = (y0 + pad_y).max(self.visible.0);
        let bottom = self.visible.1;
        let mut first = true;
        let (inner, used) = self.sub_area(x + pad_x, top, inner_w, bottom - top, y0 + pad_y, |ui| {
            ui.in_card = true;
            ui.card_first = first;
            add(ui);
            first = ui.card_first;
        });
        let _ = first;
        let h = (used - t.spacing).max(0.0) + 2.0 * pad_y;
        fill_rounded(self.canvas, x, y0, self.width, h, t.radius, color(t.card));
        if let Some(path) = rounded_rect(x + 0.5, y0 + 0.5, self.width - 1.0, h - 1.0, t.radius) {
            let mut paint = Paint::default();
            paint.anti_alias = true;
            paint.set_color(color(t.line));
            self.canvas.stroke_path(&path, &paint, &Stroke::default(), Transform::identity(), None);
        }
        if bottom > top {
            // Recorta no fim do cartão (a imagem cobre a faixa visível inteira).
            let clip_h = ((y0 + h - pad_y) - top).min(inner.height() as f32).max(0.0) as u32;
            if clip_h > 0 {
                if let Some(part) = inner.clone_rect(tiny_skia::IntRect::from_xywh(0, 0, inner.width(), clip_h).unwrap()) {
                    self.canvas.draw_pixmap((x + pad_x) as i32, top as i32, part.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
                }
            }
        }
        self.cursor_y = y0 + h + t.spacing;
    }

    /// Área que rola, ocupando o resto da altura, com margem do tema e barra de
    /// rolagem fina à direita quando o conteúdo não cabe.
    pub fn page(&mut self, add: impl FnOnce(&mut Ui)) {
        let t = self.ctx.theme.clone();
        let id = self.ctx.id();
        let (x, y0) = (self.x, self.cursor_y);
        let (w, h) = (self.width, self.visible.1 - y0);
        let pad = t.padding;
        let scroll = *self.ctx.memory.scroll.get(&id).unwrap_or(&0.0);
        // A imagem cobre só a área visível da página (começando com o que já está lá);
        // o conteúdo começa rolado.
        let (inner, used) = self.sub_area_from(x + pad, y0, w - 2.0 * pad, h, y0 + pad - scroll, true, add);
        let content_h = used + 2.0 * pad;
        let max_scroll = (content_h - h).max(0.0);
        // Roda do mouse sobre a área.
        let over = self.over((x, y0, w, h));
        let mut scroll = scroll;
        if over && self.ctx.input.scroll != 0.0 {
            scroll += self.ctx.input.scroll;
            self.ctx.repaint = true;
        }
        // Barra de rolagem.
        if max_scroll > 0.0 {
            let track_h = h - 8.0;
            let thumb_h = (track_h * h / content_h).max(32.0);
            let thumb_y = |s: f32| y0 + 4.0 + (s / max_scroll).clamp(0.0, 1.0) * (track_h - thumb_h);
            let bar = (x + w - 14.0, y0, 14.0, h);
            if let Some((px, py)) = self.pointer() {
                let on_bar = px >= bar.0 && px <= bar.0 + bar.2 && py >= bar.1 && py <= bar.1 + bar.3;
                if on_bar && self.ctx.input.pressed {
                    let top = thumb_y(scroll);
                    let grab = if py >= top && py <= top + thumb_h { py - top } else { thumb_h / 2.0 };
                    self.ctx.memory.dragging_scroll = Some((id, grab));
                    self.ctx.memory.active = Some(id);
                }
                if let Some((sid, grab)) = self.ctx.memory.dragging_scroll {
                    if sid == id {
                        let k = (py - grab - y0 - 4.0) / (track_h - thumb_h).max(1.0);
                        scroll = k.clamp(0.0, 1.0) * max_scroll;
                        self.ctx.repaint = true;
                    }
                }
            }
            scroll = scroll.clamp(0.0, max_scroll);
            let active = self.ctx.memory.dragging_scroll.is_some_and(|(s, _)| s == id) || self.over(bar);
            let tw = if active { 8.0 } else { 6.0 };
            copy_back(&inner, self.canvas, (x + pad) as i32, y0 as i32);
            fill_rounded(self.canvas, bar.0 + 2.0 + (8.0 - tw) / 2.0, thumb_y(scroll), tw, thumb_h, tw / 2.0, color([0, 0, 0, if active { 120 } else { 80 }]));
        } else {
            // Sem rolagem: se havia rolagem guardada (a janela cresceu), redesenha.
            if scroll != 0.0 {
                self.ctx.repaint = true;
            }
            scroll = 0.0;
            copy_back(&inner, self.canvas, (x + pad) as i32, y0 as i32);
        }
        self.ctx.memory.scroll.insert(id, scroll);
        self.cursor_y = y0 + h;
    }

    /// Barra lateral com seções (como nas Configurações do mide) e, à direita, o
    /// conteúdo da seção escolhida numa área que rola. `items` são (nome, ícone do
    /// tema). Devolve a seção escolhida (a mesma, ou a que foi clicada agora):
    ///
    /// ```ignore
    /// self.secao = ui.sidebar(&secoes, self.secao, |ui, secao| { ... });
    /// ```
    pub fn sidebar(&mut self, items: &[(&str, &str)], selected: usize, content: impl FnOnce(&mut Ui, usize)) -> usize {
        let mut selected = selected;
        let t = self.ctx.theme.clone();
        let side_w = 220.0;
        let (x, y0) = (self.x, self.cursor_y);
        let h = self.visible.1 - y0;
        fill_rounded(self.canvas, x, y0, side_w, h, 0.0, color(t.sidebar));
        fill_rounded(self.canvas, x + side_w - 1.0, y0, 1.0, h, 0.0, color(t.line));
        let mut row_y = y0 + 16.0;
        for (i, (name, icon)) in items.iter().enumerate() {
            // Item sem ícone é um título de grupo ("Favoritos"), não uma seção.
            if icon.is_empty() {
                if i > 0 {
                    row_y += 8.0;
                }
                self.draw_text(name, x + 20.0, row_y + 4.0, t.font_small, FontWeight::Semibold, t.text_secondary);
                row_y += 24.0;
                continue;
            }
            let id = self.ctx.id();
            let rect = (x + 10.0, row_y, side_w - 20.0, 30.0);
            row_y += 34.0;
            let r = self.interact(id, rect);
            if r.clicked && selected != i {
                selected = i;
                self.ctx.repaint = true;
            }
            let is_sel = selected == i;
            if is_sel {
                fill_rounded(self.canvas, rect.0, rect.1, rect.2, rect.3, t.control_radius, color(t.accent));
            } else if r.hovered {
                fill_rounded(self.canvas, rect.0, rect.1, rect.2, rect.3, t.control_radius, color(t.control));
            }
            let key = format!("{icon}@20");
            let pix = self.ctx.icons.entry(key).or_insert_with(|| crate::icons::load_themed_icon(&[icon], 20).map(std::rc::Rc::new)).clone();
            if let Some(pix) = pix {
                crate::ui::draw::blit(self.canvas, &pix, (rect.0 + 10.0) as i32, (rect.1 + 5.0) as i32, 1.0);
            }
            let tc = if is_sel { t.on_accent } else { t.text };
            self.draw_text(name, rect.0 + 40.0, rect.1 + (30.0 - t.font * 1.25) / 2.0, t.font, FontWeight::Regular, tc);
        }
        // Conteúdo à direita.
        let (old_x, old_w) = (self.x, self.width);
        self.x = x + side_w;
        self.width = old_w - side_w;
        if self.sidebar_raw {
            self.sidebar_raw = false;
            content(self, selected);
        } else {
            self.page(|ui| content(ui, selected));
        }
        self.x = old_x;
        self.width = old_w;
        selected
    }

    /// Como `sidebar`, mas o conteúdo recebe a área inteira à direita, sem margem nem
    /// rolagem (para montar uma barra de ferramentas fixa em cima de uma `page`).
    pub fn sidebar_area(&mut self, items: &[(&str, &str)], selected: usize, content: impl FnOnce(&mut Ui, usize)) -> usize {
        self.sidebar_raw = true;
        self.sidebar(items, selected, content)
    }
}

/// Copia pixels de `src` a partir de (`sx`, `sy`) para `dst` em (`dx`, `dy`), o
/// quanto couber nas duas imagens (linha a linha, sem misturar transparência).
fn copy_pixels(src: &Pixmap, sx: i32, sy: i32, dst: &mut Pixmap, dx: i32, dy: i32) {
    let (sw, sh, dw, dh) = (src.width() as i32, src.height() as i32, dst.width() as i32, dst.height() as i32);
    // Ajusta o começo para dentro das duas imagens.
    let (ox, oy) = ((-sx).max(-dx).max(0), (-sy).max(-dy).max(0));
    let (sx, sy, dx, dy) = (sx + ox, sy + oy, dx + ox, dy + oy);
    let w = (sw - sx).min(dw - dx);
    let h = (sh - sy).min(dh - dy);
    if w <= 0 || h <= 0 {
        return;
    }
    let (sstride, dstride, row) = (sw as usize * 4, dw as usize * 4, w as usize * 4);
    let src_data = src.data();
    let dst_data = dst.data_mut();
    for y in 0..h as usize {
        let s0 = (sy as usize + y) * sstride + sx as usize * 4;
        let d0 = (dy as usize + y) * dstride + dx as usize * 4;
        dst_data[d0..d0 + row].copy_from_slice(&src_data[s0..s0 + row]);
    }
}

/// Devolve a imagem de uma área (de `sub_area_from` com cópia) para `dst` em (`x`, `y`).
fn copy_back(inner: &Pixmap, dst: &mut Pixmap, x: i32, y: i32) {
    copy_pixels(inner, 0, 0, dst, x, y);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::theme::Theme;

    /// Roda um quadro do `view` numa imagem de 400x300 e devolve o resultado dele.
    fn frame<R>(ctx: &mut Ctx, view: impl FnOnce(&mut Ui) -> R) -> R {
        let mut canvas = Pixmap::new(400, 300).unwrap();
        ctx.begin_frame();
        let r = {
            let mut ui = Ui::root(&mut canvas, ctx);
            view(&mut ui)
        };
        ctx.end_frame();
        r
    }

    /// Aperta e solta o botão em (x, y), em dois quadros, como um clique de verdade.
    fn click<R>(ctx: &mut Ctx, x: f32, y: f32, mut view: impl FnMut(&mut Ui) -> R) -> R {
        ctx.input.pointer = Some((x, y));
        ctx.input.pressed = true;
        ctx.input.down = true;
        frame(ctx, &mut view);
        ctx.input.released = true;
        ctx.input.down = false;
        frame(ctx, &mut view)
    }

    #[test]
    fn botao_clicado_so_quando_aperta_e_solta_nele() {
        let mut ctx = Ctx::new(Theme::default());
        // Botão no topo da área, em (0, 0) com altura 32.
        let clicked = click(&mut ctx, 10.0, 10.0, |ui| ui.button("OK").clicked());
        assert!(clicked);
        // Apertar fora e soltar em cima não conta.
        ctx.input.pointer = Some((300.0, 200.0));
        ctx.input.pressed = true;
        ctx.input.down = true;
        frame(&mut ctx, |ui| ui.button("OK"));
        ctx.input.pointer = Some((10.0, 10.0));
        ctx.input.released = true;
        ctx.input.down = false;
        assert!(!frame(&mut ctx, |ui| ui.button("OK").clicked()));
    }

    #[test]
    fn interruptor_troca_o_valor() {
        let mut ctx = Ctx::new(Theme::default());
        let mut ligado = false;
        let changed = click(&mut ctx, 200.0, 16.0, |ui| ui.switch("Wi-Fi", &mut ligado).changed());
        assert!(changed);
        assert!(ligado);
    }

    #[test]
    fn campo_de_texto_recebe_digitacao_e_enter() {
        let mut ctx = Ctx::new(Theme::default());
        let mut texto = String::new();
        click(&mut ctx, 50.0, 16.0, |ui| ui.text_input(&mut texto, "nome"));
        ctx.input.text = "olá".into();
        frame(&mut ctx, |ui| ui.text_input(&mut texto, "nome"));
        assert_eq!(texto, "olá");
        ctx.input.keys = vec![Key::Backspace, Key::Enter];
        let enviado = frame(&mut ctx, |ui| ui.text_input(&mut texto, "nome").submitted());
        assert_eq!(texto, "ol");
        assert!(enviado);
    }

    #[test]
    fn itens_em_coluna_e_em_linha() {
        let mut ctx = Ctx::new(Theme::default());
        let (a, b, c) = frame(&mut ctx, |ui| {
            let a = ui.button("A").rect;
            let mut b = (0.0, 0.0, 0.0, 0.0);
            let mut c = (0.0, 0.0, 0.0, 0.0);
            ui.row(|ui| {
                b = ui.button("B").rect;
                c = ui.button("C").rect;
            });
            (a, b, c)
        });
        // B fica abaixo de A; C fica à direita de B, na mesma altura.
        assert!(b.1 > a.1);
        assert_eq!(b.1, c.1);
        assert!(c.0 > b.0 + b.2);
    }

    #[test]
    fn grade_quebra_linha_quando_acaba_a_largura() {
        let mut ctx = Ctx::new(Theme::default());
        let rects = frame(&mut ctx, |ui| {
            let mut rects = Vec::new();
            ui.wrap(|ui| {
                for name in ["a", "b", "c", "d"] {
                    rects.push(ui.tile(Tile::Icon("nenhum-icone"), name, false, 64).rect);
                }
            });
            rects
        });
        // Ladrilhos de 108 px numa área de 400: três por linha, o quarto desce.
        assert_eq!(rects[0].1, rects[2].1);
        assert!(rects[2].0 > rects[1].0);
        assert!(rects[3].1 > rects[0].1);
        assert_eq!(rects[3].0, rects[0].0);
    }

    #[test]
    fn duplo_clique_no_mesmo_componente() {
        let mut ctx = Ctx::new(Theme::default());
        let first = click(&mut ctx, 10.0, 10.0, |ui| ui.button("Pasta"));
        assert!(first.clicked() && !first.double_clicked());
        let second = click(&mut ctx, 10.0, 10.0, |ui| ui.button("Pasta"));
        assert!(second.double_clicked());
        // O terceiro começa uma contagem nova.
        let third = click(&mut ctx, 10.0, 10.0, |ui| ui.button("Pasta"));
        assert!(!third.double_clicked());
    }

    #[test]
    fn menu_de_contexto_abre_escolhe_e_fecha() {
        let mut ctx = Ctx::new(Theme::default());
        let view = |ui: &mut Ui| {
            let r = ui.button("arquivo.txt");
            let mut chosen = None;
            ui.context_menu(&r, |m| {
                if m.item("Abrir") {
                    chosen = Some("Abrir");
                }
                if m.danger("Apagar") {
                    chosen = Some("Apagar");
                }
            });
            chosen
        };
        // Clique direito no botão abre o menu, desenhado na camada de cima.
        ctx.input.pointer = Some((10.0, 10.0));
        ctx.input.secondary = true;
        frame(&mut ctx, view);
        assert!(ctx.memory.menu.is_some());
        let mut canvas = Pixmap::new(400, 300).unwrap();
        ctx.begin_frame();
        view(&mut Ui::root(&mut canvas, &mut ctx));
        assert!(ctx.take_overlay().is_some(), "menu desenhado por cima");
        ctx.end_frame();
        // O menu abre no ponteiro: "Abrir" na primeira linha (y 15..41). O item é
        // escolhido ao apertar, e o clique não chega ao botão que está embaixo.
        ctx.input.pointer = Some((40.0, 28.0));
        ctx.input.pressed = true;
        ctx.input.down = true;
        assert_eq!(frame(&mut ctx, view), Some("Abrir"));
        assert!(ctx.memory.menu.is_none(), "escolher fecha o menu");
        ctx.input.released = true;
        ctx.input.down = false;
        frame(&mut ctx, view);
        // Abrir de novo e clicar fora fecha sem escolher nada.
        ctx.input.pointer = Some((10.0, 10.0));
        ctx.input.secondary = true;
        frame(&mut ctx, view);
        assert!(ctx.memory.menu.is_some());
        assert_eq!(click(&mut ctx, 390.0, 290.0, view), None);
        assert!(ctx.memory.menu.is_none(), "clique fora fecha o menu");
    }

    #[test]
    fn foco_pedido_seleciona_tudo_e_digitar_substitui() {
        let mut ctx = Ctx::new(Theme::default());
        let mut nome = String::from("Nova pasta");
        frame(&mut ctx, |ui| {
            ui.focus_next_input();
            ui.text_input(&mut nome, "nome");
        });
        ctx.input.text = "Fotos".into();
        frame(&mut ctx, |ui| ui.text_input(&mut nome, "nome"));
        assert_eq!(nome, "Fotos");
        // Depois de digitar, a seleção acabou: o resto vai no fim.
        ctx.input.text = "!".into();
        frame(&mut ctx, |ui| ui.text_input(&mut nome, "nome"));
        assert_eq!(nome, "Fotos!");
    }

    /// Bancada: `cargo test --lib bancada_quadro -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn bancada_quadro() {
        let mut ctx = Ctx::new(Theme::default());
        let names: Vec<String> = (0..60).map(|i| format!("arquivo {i}.txt")).collect();
        let side = [("Favoritos", ""), ("Pasta pessoal", "user-home"), ("Documentos", "folder-documents"), ("Lixeira", "user-trash")];
        let mut busca = String::new();
        let mut view = |ui: &mut Ui| {
            ui.sidebar_area(&side, 1, |ui, _| {
                ui.toolbar(|ui| {
                    ui.icon_button("go-previous", "<", true);
                    ui.strong("pasta");
                    ui.push_right(220.0);
                    ui.sized(220.0, |ui| {
                        ui.text_input(&mut busca, "Buscar");
                    });
                });
                ui.page(|ui| {
                    ui.wrap(|ui| {
                        for n in &names {
                            ui.tile(Tile::Icon("text-x-generic"), n, false, 64);
                        }
                    });
                });
            });
        };
        let mut canvas = Pixmap::new(900, 520).unwrap();
        for round in 0..std::env::var("BANCADA_N").ok().and_then(|v| v.parse().ok()).unwrap_or(3) {
            let t0 = std::time::Instant::now();
            canvas.fill(color(ctx.theme.background));
            let t1 = std::time::Instant::now();
            ctx.begin_frame();
            view(&mut Ui::root(&mut canvas, &mut ctx));
            ctx.end_frame();
            let t2 = std::time::Instant::now();
            let mut out = vec![0u8; 900 * 520 * 4];
            crate::ui::draw::to_argb8888(&canvas, &mut out);
            let t3 = std::time::Instant::now();
            println!(
                "rodada {round}: fundo {:.2} ms, view {:.2} ms, conversão {:.2} ms",
                (t1 - t0).as_secs_f32() * 1e3,
                (t2 - t1).as_secs_f32() * 1e3,
                (t3 - t2).as_secs_f32() * 1e3
            );
        }
    }

    #[test]
    #[ignore]
    fn bancada_partes() {
        let mut ctx = Ctx::new(Theme::default());
        let mut canvas = Pixmap::new(900, 520).unwrap();
        let names: Vec<String> = (0..60).map(|i| format!("arquivo {i}.txt")).collect();
        let side = [("Favoritos", ""), ("Pasta pessoal", "user-home"), ("Documentos", "folder-documents"), ("Lixeira", "user-trash")];
        let mut medir = |nome: &str, view: &mut dyn FnMut(&mut Ui)| {
            for _ in 0..3 {
                ctx.begin_frame();
                view(&mut Ui::root(&mut canvas, &mut ctx));
                ctx.end_frame();
            }
            let t = std::time::Instant::now();
            for _ in 0..20 {
                ctx.begin_frame();
                view(&mut Ui::root(&mut canvas, &mut ctx));
                ctx.end_frame();
            }
            println!("{nome}: {:.2} ms", t.elapsed().as_secs_f32() * 1e3 / 20.0);
        };
        medir("barra lateral vazia", &mut |ui| {
            ui.sidebar_area(&side, 1, |_, _| {});
        });
        medir("página vazia", &mut |ui| ui.page(|_| {}));
        medir("60 ladrilhos sem página", &mut |ui| {
            ui.wrap(|ui| {
                for n in &names {
                    ui.tile(Tile::Icon("text-x-generic"), n, false, 64);
                }
            })
        });
        medir("20 ladrilhos só ícone", &mut |ui| {
            ui.wrap(|ui| {
                for _ in 0..20 {
                    ui.tile(Tile::Icon("text-x-generic"), "", false, 64);
                }
            })
        });
        medir("20 textos", &mut |ui| {
            for n in names.iter().take(20) {
                ui.strong(n);
            }
        });
    }
}
