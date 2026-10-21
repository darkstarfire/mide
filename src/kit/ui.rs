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
    /// Ponto do clique (o menu que abre ao lado aponta para ele).
    click: (f32, f32),
    height: f32,
    /// Topo do item clicado (para o menu que abre para cima não cobrir o item).
    item_top: Option<f32>,
    /// Quando abriu (entra esmaecendo, `[effects] menu_animation`).
    opened: Option<std::time::Instant>,
}

/// Onde o menu de contexto fica: canto de cima à esquerda, se abre ao lado do clique
/// (`Some(true)` à direita, `Some(false)` à esquerda) e, se não, se abre para cima.
#[derive(Debug, Clone, Copy, PartialEq)]
struct MenuPlace {
    x: f32,
    y: f32,
    side: Option<bool>,
    up: bool,
}

/// Posição do menu de largura `w` numa janela `window`: embaixo do item, em cima se
/// não couber, e ao lado do clique se não couber em nenhum dos dois (sem cobrir o item).
fn menu_place(state: &MenuState, w: f32, window: (f32, f32)) -> MenuPlace {
    let (ww, wh) = window;
    let arrow = crate::ui::draw::ARROW_H;
    let top_anchor = state.item_top.unwrap_or(state.pos.1);
    let below = state.pos.1 + arrow + state.height + 6.0 <= wh;
    let above = top_anchor - arrow - state.height >= 4.0;
    let side = (!below && !above).then(|| state.click.0 + arrow + w + 4.0 <= ww || state.click.0 - arrow - w < 4.0);
    let (x, y) = match side {
        Some(right) => {
            let x = if right { state.click.0 + arrow } else { state.click.0 - arrow - w };
            let y = (state.click.1 - state.height / 2.0).clamp(4.0, (wh - state.height - 4.0).max(4.0));
            (x.clamp(4.0, (ww - w - 4.0).max(4.0)), y)
        }
        None => {
            let x = (state.pos.0 - 24.0).clamp(4.0, (ww - w - 4.0).max(4.0));
            (x, if below { state.pos.1 + arrow } else { top_anchor - arrow - state.height })
        }
    };
    MenuPlace { x, y, side, up: side.is_none() && !below }
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

/// Tela no arranjo das Configurações → Monitores (`Ui::arrangement`), em pixels
/// lógicos.
#[derive(Debug, Clone, PartialEq)]
pub struct ArrangedScreen {
    pub name: String,
    pub pos: (i32, i32),
    pub size: (i32, i32),
    pub primary: bool,
    /// Tamanho físico em milímetros: o arranjo desenha as telas do tamanho de verdade.
    pub mm: (f32, f32),
}

/// Lado em que uma tela encosta na vizinha.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Right,
    Left,
    Below,
    Above,
}

/// Milímetros por ponto da tela (largura e altura).
fn mm_per_px(s: &ArrangedScreen) -> (f32, f32) {
    (s.mm.0 / s.size.0.max(1) as f32, s.mm.1 / s.size.1.max(1) as f32)
}

/// Onde cada tela fica no desenho do arranjo, em milímetros: a principal em (0, 0) e as
/// outras a partir de quem elas encostam (nas posições lógicas), para telas de
/// densidades diferentes ficarem coladas no desenho também.
pub fn physical_layout(screens: &[ArrangedScreen]) -> Vec<(f32, f32)> {
    let n = screens.len();
    let mut out: Vec<Option<(f32, f32)>> = vec![None; n];
    if n == 0 {
        return Vec::new();
    }
    let first = screens.iter().position(|s| s.primary).unwrap_or(0);
    out[first] = Some((0.0, 0.0));
    let touches = |i: usize, j: usize| -> Option<Side> {
        let (a, b) = (&screens[i], &screens[j]);
        let y_overlap = a.pos.1 < b.pos.1 + b.size.1 && b.pos.1 < a.pos.1 + a.size.1;
        let x_overlap = a.pos.0 < b.pos.0 + b.size.0 && b.pos.0 < a.pos.0 + a.size.0;
        if y_overlap && a.pos.0 == b.pos.0 + b.size.0 {
            Some(Side::Right)
        } else if y_overlap && a.pos.0 + a.size.0 == b.pos.0 {
            Some(Side::Left)
        } else if x_overlap && a.pos.1 == b.pos.1 + b.size.1 {
            Some(Side::Below)
        } else if x_overlap && a.pos.1 + a.size.1 == b.pos.1 {
            Some(Side::Above)
        } else {
            None
        }
    };
    for _ in 0..n {
        for i in 0..n {
            if out[i].is_some() {
                continue;
            }
            let neighbor = (0..n).filter(|&j| out[j].is_some()).find_map(|j| touches(i, j).map(|side| (j, side)));
            if let Some((j, side)) = neighbor {
                let (jx, jy) = out[j].unwrap_or_default();
                let (kx, ky) = mm_per_px(&screens[j]);
                let (a, b) = (&screens[i], &screens[j]);
                out[i] = Some(match side {
                    Side::Right => (jx + b.mm.0, jy + (a.pos.1 - b.pos.1) as f32 * ky),
                    Side::Left => (jx - a.mm.0, jy + (a.pos.1 - b.pos.1) as f32 * ky),
                    Side::Below => (jx + (a.pos.0 - b.pos.0) as f32 * kx, jy + b.mm.1),
                    Side::Above => (jx + (a.pos.0 - b.pos.0) as f32 * kx, jy - a.mm.1),
                });
            }
        }
    }
    // Telas soltas (sem encostar em nenhuma): pela escala da principal.
    let (kx, ky) = mm_per_px(&screens[first]);
    let origin = screens[first].pos;
    (0..n)
        .map(|i| out[i].unwrap_or(((screens[i].pos.0 - origin.0) as f32 * kx, (screens[i].pos.1 - origin.1) as f32 * ky)))
        .collect()
}

/// Encaixa a tela `i`, solta em `pos` (milímetros, no desenho), na vizinha mais
/// próxima sem sobrepor nenhuma, e devolve a posição lógica nova dela: encostada do
/// lado escolhido e com o deslocamento ao longo da borda convertido pela vizinha.
pub fn snap_physical(screens: &[ArrangedScreen], layout: &[(f32, f32)], i: usize, pos: (f32, f32)) -> (i32, i32) {
    let me = &screens[i];
    let (w, h) = me.mm;
    let overlaps = |p: (f32, f32)| {
        (0..screens.len()).filter(|&j| j != i).any(|j| {
            let (ox, oy) = layout[j];
            let (ow, oh) = screens[j].mm;
            p.0 < ox + ow - 0.5 && ox < p.0 + w - 0.5 && p.1 < oy + oh - 0.5 && oy < p.1 + h - 0.5
        })
    };
    let mut best: Option<(usize, Side, f32, f32)> = None;
    for j in (0..screens.len()).filter(|&j| j != i) {
        let (ox, oy) = layout[j];
        let (ow, oh) = screens[j].mm;
        let y = pos.1.clamp(oy - h + 1.0, oy + oh - 1.0);
        let x = pos.0.clamp(ox - w + 1.0, ox + ow - 1.0);
        for (side, candidate) in [(Side::Right, (ox + ow, y)), (Side::Left, (ox - w, y)), (Side::Below, (x, oy + oh)), (Side::Above, (x, oy - h))] {
            if overlaps(candidate) {
                continue;
            }
            let d = (candidate.0 - pos.0).powi(2) + (candidate.1 - pos.1).powi(2);
            let along = match side {
                Side::Right | Side::Left => candidate.1 - oy,
                Side::Below | Side::Above => candidate.0 - ox,
            };
            if best.is_none_or(|(_, _, _, bd)| d < bd) {
                best = Some((j, side, along, d));
            }
        }
    }
    let Some((j, side, along, _)) = best else {
        return if screens.len() == 1 { (0, 0) } else { me.pos };
    };
    let other = &screens[j];
    let (kx, ky) = mm_per_px(other);
    match side {
        Side::Right => (other.pos.0 + other.size.0, other.pos.1 + (along / ky).round() as i32),
        Side::Left => (other.pos.0 - me.size.0, other.pos.1 + (along / ky).round() as i32),
        Side::Below => (other.pos.0 + (along / kx).round() as i32, other.pos.1 + other.size.1),
        Side::Above => (other.pos.0 + (along / kx).round() as i32, other.pos.1 - me.size.1),
    }
}

/// Arrasto em andamento: componente, tela, ponto agarrado (na tela, em pixels da
/// janela) e a escala e origem do desenho no começo do arrasto (o desenho não muda de
/// escala enquanto a tela anda).
#[derive(Debug, Clone, Copy)]
struct ArrangeDrag {
    id: u64,
    index: usize,
    grab: (f32, f32),
    k: f32,
    origin: (f32, f32),
    min: (i32, i32),
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
    /// Tela sendo arrastada no arranjo (`Ui::arrangement`).
    arrange_drag: Option<ArrangeDrag>,
    /// O próximo `text_input` desenhado recebe o foco (`Ui::focus_next_input`).
    focus_next: bool,
    /// Cursor e seleção do campo com foco.
    edit: super::textedit::Edit,
    /// Rolagem horizontal do texto do campo com foco (texto maior que o campo).
    text_scroll: f32,
    /// Último clique (componente e quando), para reconhecer o duplo clique.
    last_click: Option<(u64, std::time::Instant)>,
    /// Menu de contexto aberto e o retângulo dele no último quadro (na janela).
    menu: Option<MenuState>,
    menu_rect: Option<(f32, f32, f32, f32)>,
    menu_drawn: bool,
    /// Modal aberto (retângulo do quadro anterior, na janela) e se foi desenhado agora.
    modal_rect: Option<(f32, f32, f32, f32)>,
    modal_drawn: bool,
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
    /// Carregamento de ícones numa thread: pedidos (chave, de app?, nomes, tamanho),
    /// resultados e os que ainda estão carregando. Carregar e rasterizar um SVG leva
    /// milissegundos; desenhar não pode esperar por isso.
    icon_jobs: Option<std::sync::mpsc::Sender<(String, bool, Vec<String>, u32)>>,
    icon_done: Option<std::sync::mpsc::Receiver<(String, Option<Pixmap>)>>,
    icon_pending: std::collections::HashSet<String>,
    /// Camada por cima de tudo (menus de contexto) e o tamanho da janela.
    overlay: Option<Pixmap>,
    window: (u32, u32),
    /// Algum componente está sob o ponteiro neste quadro.
    hover_claimed: bool,
    /// Desenhando o conteúdo do modal (só ele reage enquanto está aberto).
    in_modal: bool,
    /// O app pediu outro quadro em breve (`Ui::request_tick`).
    pub ticking: bool,
    /// Título pedido pelo app (`Ui::set_title`), aplicado pela janela.
    pub title: Option<String>,
    /// Painéis de vidro deste quadro (x, y, largura, altura, raio), na janela.
    pub glass: Vec<(f32, f32, f32, f32, f32)>,
    /// Este quadro desenhou algo que usa vidro quando ligado (barra lateral de vidro):
    /// a janela acompanha a opção nas Configurações.
    pub glass_capable: bool,
    /// O app pediu para fechar (`Ui::close`).
    pub close_requested: bool,
    /// Quadro agendado pelo app (`Ui::wake_after`).
    pub wake_at: Option<std::time::Instant>,
    /// O app é uma camada (`Window::overlay`): painéis de vidro deixam ver o fundo.
    pub layer: bool,
    /// Texto copiado (Ctrl+C, Ctrl+X num campo): a janela põe na área de transferência.
    pub clipboard_out: Option<String>,
    /// Ctrl+V num campo: a janela lê a área de transferência e entrega em `input.paste`.
    pub paste_requested: bool,
    /// Acorda a janela de outra thread (`Ui::waker`).
    pub waker: Option<super::app::Waker>,
    /// Área que recebe cliques neste quadro (`Ui::input_area`; `None`: a superfície toda).
    pub input_region: Option<(f32, f32, f32, f32)>,
    /// Pedido para esconder (`true`) ou mostrar a superfície (`Ui::set_hidden`).
    pub hide_request: Option<bool>,
    /// Tela de bloqueio: a senha foi aceita (`Ui::unlock`).
    pub unlock_request: bool,
    /// Desenhando a cópia da tela de bloqueio de outro monitor (`Ui::mirror`).
    pub mirror: bool,
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
            icon_jobs: None,
            icon_done: None,
            icon_pending: std::collections::HashSet::new(),
            overlay: None,
            window: (0, 0),
            hover_claimed: false,
            in_modal: false,
            ticking: false,
            title: None,
            glass: Vec::new(),
            glass_capable: false,
            close_requested: false,
            wake_at: None,
            layer: false,
            clipboard_out: None,
            paste_requested: false,
            waker: None,
            input_region: None,
            hide_request: None,
            unlock_request: false,
            mirror: false,
        }
    }

    /// Ícone pelo nome (ou o primeiro que existir de `names`), no tamanho pedido. Se
    /// ainda não foi carregado, pede à thread de ícones e devolve `None` (o componente
    /// desenha sem ícone e o ícone aparece num dos próximos quadros). `app`: nome de
    /// ícone de app (procura também no tema `hicolor` e aceita caminhos de arquivo).
    fn icon(&mut self, names: &[&str], size: u32, app: bool) -> Option<std::rc::Rc<Pixmap>> {
        // Em telas com escala, o ícone é carregado no tamanho em pixels (nítido).
        let size = (size as f32 * crate::ui::draw::scale()).round() as u32;
        let key = format!("{}{}@{size}", if app { "app:" } else { "" }, names.first().copied().unwrap_or(""));
        if let Some(icon) = self.icons.get(&key) {
            return icon.clone();
        }
        if self.icon_pending.contains(&key) {
            return None;
        }
        if self.icon_jobs.is_none() {
            let (jobs, rx) = std::sync::mpsc::channel::<(String, bool, Vec<String>, u32)>();
            let (tx, done) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                for (key, app, names, size) in rx {
                    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
                    let icon = if app {
                        crate::icons::load_icon(refs[0], size).or_else(|| crate::icons::load_themed_icon(&refs[1..], size))
                    } else {
                        crate::icons::load_themed_icon(&refs, size)
                    };
                    if tx.send((key, icon)).is_err() {
                        break;
                    }
                }
            });
            self.icon_jobs = Some(jobs);
            self.icon_done = Some(done);
        }
        let names: Vec<String> = names.iter().map(|n| n.to_string()).collect();
        if self.icon_jobs.as_ref().is_some_and(|jobs| jobs.send((key.clone(), app, names, size)).is_ok()) {
            self.icon_pending.insert(key);
            // Redesenha quando o ícone chegar (sem isso, só no próximo movimento do mouse).
            self.ticking = true;
        }
        None
    }

    /// Camada de cima (menus) deste quadro, se houver.
    pub fn take_overlay(&mut self) -> Option<Pixmap> {
        self.overlay.take()
    }

    /// Opacidade e deslocamento vertical da camada de cima (o menu que acabou de abrir
    /// entra esmaecendo e descendo 6 px em 120 ms). Enquanto anima, pede quadros.
    pub fn overlay_look(&mut self) -> (f32, i32) {
        let Some(opened) = self.memory.menu.as_ref().and_then(|m| m.opened) else {
            return (1.0, 0);
        };
        let t = (opened.elapsed().as_secs_f32() / 0.12).clamp(0.0, 1.0);
        if t < 1.0 {
            self.ticking = true;
        }
        let e = 1.0 - (1.0 - t) * (1.0 - t);
        (e, (-6.0 * (1.0 - e)).round() as i32)
    }

    /// Começo de um quadro: os ids voltam a contar do início.
    pub fn begin_frame(&mut self) {
        self.next_id = 1;
        self.repaint = false;
        self.hover_claimed = false;
        self.ticking = false;
        self.glass.clear();
        self.glass_capable = false;
        // Ícones que ficaram prontos; enquanto houver algum carregando, mais quadros.
        if let Some(done) = &self.icon_done {
            while let Ok((key, icon)) = done.try_recv() {
                self.icon_pending.remove(&key);
                self.icons.insert(key, icon.map(std::rc::Rc::new));
            }
        }
        self.ticking = !self.icon_pending.is_empty();
        self.memory.menu_drawn = false;
        self.memory.modal_drawn = false;
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
        // Modal que não foi desenhado neste quadro fechou.
        if !self.memory.modal_drawn && self.memory.modal_rect.is_some() {
            self.memory.modal_rect = None;
            self.repaint = true;
        }
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
    /// A próxima barra lateral é de vidro (`Ui::glass_sidebar`).
    sidebar_glass: bool,
    /// Largura forçada para os componentes dentro de `sized`.
    next_width: Option<f32>,
    /// O próximo `icon_button` aparece marcado (usado por `icon_toggle`).
    toggled: bool,
}

impl<'a> Ui<'a> {
    /// Área da janela inteira.
    pub fn root(canvas: &'a mut Pixmap, ctx: &'a mut Ctx) -> Self {
        // A imagem está em pixels; o layout, em pontos (`draw::scale`).
        let k = crate::ui::draw::scale();
        let (w, h) = (canvas.width() as f32 / k, canvas.height() as f32 / k);
        ctx.window = (w.round() as u32, h.round() as u32);
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
            sidebar_glass: false,
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
        // Com um modal aberto, só o que está dentro dele reage.
        let blocked = self.ctx.memory.modal_rect.is_some() && !self.ctx.in_modal;
        let hovered = !under_menu && !blocked && self.over(rect) && self.ctx.memory.active.is_none_or(|a| a == id);
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
        let blocked = self.ctx.memory.modal_rect.is_some() && !self.ctx.in_modal;
        let hovered = !under_menu && !blocked && self.over(rect);
        Response {
            hovered,
            clicked: hovered && self.ctx.input.pressed,
            secondary_clicked: hovered && self.ctx.input.secondary,
            rect,
            id,
            ..Default::default()
        }
    }

    /// Janela modal (diálogo): escurece a janela e mostra um painel de vidro branco no
    /// centro, com o conteúdo montado no `add`. Enquanto o modal é desenhado (a cada
    /// quadro em que ele está aberto), nada atrás dele reage. Chamar no fim do `view`,
    /// na área de cima (a janela inteira):
    ///
    /// ```ignore
    /// if let Some(d) = &mut self.dialogo {
    ///     ui.modal(380.0, |ui| {
    ///         ui.title("Nova pasta");
    ///         ui.text_input(&mut d.nome, "Nome");
    ///     });
    /// }
    /// ```
    pub fn modal(&mut self, width: f32, add: impl FnOnce(&mut Ui)) -> (f32, f32, f32, f32) {
        let (ww, wh) = (self.ctx.window.0 as f32, self.ctx.window.1 as f32);
        let mut dim = Paint::default();
        dim.set_color(color([0, 0, 0, 56]));
        if let Some(r) = tiny_skia::Rect::from_xywh(0.0, 0.0, ww, wh) {
            self.canvas.fill_rect(r, &dim, crate::ui::draw::tf(), None);
        }
        let width = width.min(ww - 32.0);
        let (x, y) = (((ww - width) / 2.0).round(), (wh * 0.24).round());
        self.ctx.in_modal = true;
        let rect = self.glass_panel(x, y, width, 14.0, add);
        self.ctx.in_modal = false;
        self.ctx.memory.modal_rect = Some((self.offset.0 + rect.0, self.offset.1 + rect.1, rect.2, rect.3));
        self.ctx.memory.modal_drawn = true;
        rect
    }

    /// Diálogo de aviso pronto: título, texto e botões (o último é o principal, em
    /// azul). Devolve o botão clicado (Esc conta como o primeiro, que costuma ser
    /// "Cancelar"; Enter, como o principal). O app guarda se o aviso está aberto:
    ///
    /// ```ignore
    /// if self.confirmar {
    ///     match ui.alert("Apagar o arquivo?", "Não dá para desfazer.", &["Cancelar", "Apagar"]) {
    ///         Some(1) => { apagar(); self.confirmar = false; }
    ///         Some(_) => self.confirmar = false,
    ///         None => {}
    ///     }
    /// }
    /// ```
    pub fn alert(&mut self, title: &str, message: &str, buttons: &[&str]) -> Option<usize> {
        let mut chosen = None;
        let bw = 120.0;
        let width = (buttons.len() as f32 * (bw + 8.0) + 40.0).max(420.0);
        self.modal(width, |ui| {
            ui.strong(title);
            if !message.is_empty() {
                ui.note(message);
            }
            ui.row(|ui| {
                ui.push_right(buttons.len() as f32 * (bw + 8.0) - 8.0);
                for (i, label) in buttons.iter().enumerate() {
                    let primary = i + 1 == buttons.len();
                    ui.sized(bw, |ui| {
                        let r = if primary { ui.primary_button(label) } else { ui.button(label) };
                        if r.clicked() {
                            chosen = Some(i);
                        }
                    });
                }
            });
        });
        if chosen.is_none() && !buttons.is_empty() {
            if self.input().key(super::input::Key::Escape) {
                chosen = Some(0);
            } else if self.input().key(super::input::Key::Enter) && !self.typing() {
                chosen = Some(buttons.len() - 1);
            }
        }
        chosen
    }

    /// Entrega um ícone já carregado (por exemplo, numa thread do app) para os
    /// componentes que usam `nome` nesse tamanho: assim eles não carregam o ícone na
    /// hora de desenhar (carregar e rasterizar um SVG leva milissegundos).
    pub fn provide_icon(&mut self, name: &str, size: u32, icon: Option<Pixmap>) {
        let icon = icon.map(std::rc::Rc::new);
        self.ctx.icons.insert(format!("app:{name}@{size}"), icon.clone());
        self.ctx.icons.insert(format!("{name}@{size}"), icon);
    }

    /// Fecha o app (por exemplo, o buscador depois de abrir um resultado).
    pub fn close(&mut self) {
        self.ctx.close_requested = true;
    }

    /// Painel de vidro em (`x`, `y`) com `width` de largura: o conteúdo é montado no
    /// `add` (em coluna) e a altura do painel acompanha. Atrás dele vai o vidro do
    /// mide (desfoque e vidro líquido, desenhados pelo compositor) quando o app é uma
    /// camada (`Window::overlay`) e o efeito está ligado; senão, só o fundo claro.
    /// Devolve o retângulo do painel.
    pub fn glass_panel(&mut self, x: f32, y: f32, width: f32, radius: f32, add: impl FnOnce(&mut Ui)) -> (f32, f32, f32, f32) {
        // Margem interna de 10 px em volta do conteúdo.
        let pad = 10.0;
        let max_h = (self.visible.1 - y).max(1.0);
        let (inner, used) = self.sub_area(x + pad, y + pad, width - 2.0 * pad, max_h - 2.0 * pad, y + pad, add);
        let h = ((used - self.ctx.theme.spacing).max(0.0) + 2.0 * pad).min(max_h);
        // Vidro de verdade só em camadas (o compositor desfoca atrás); numa janela, o
        // painel é branco opaco (senão abriria um buraco para a área de trabalho).
        let glass = self.ctx.layer && crate::control_client::panel_glass();
        // Sombra só na faixa de fora e o véu escrito direto (sem misturar no miolo):
        // painéis grandes continuam baratos.
        crate::ui::draw::soft_shadow(self.canvas, x, y, width, h, radius, 16.0, 6.0, 60);
        let veil = if glass { 178 } else if self.ctx.layer { 246 } else { 255 };
        let [vr, vg, vb, _] = if self.ctx.theme.dark { self.ctx.theme.card } else { [248, 248, 250, 255] };
        crate::ui::draw::paint_rounded_solid(self.canvas, x, y, width, h, radius, [vr, vg, vb, veil]);
        if let Some(path) = rounded_rect(x + 0.5, y + 0.5, width - 1.0, h - 1.0, radius) {
            let mut paint = Paint::default();
            paint.anti_alias = true;
            paint.set_color(color([255, 255, 255, 90]));
            self.canvas.stroke_path(&path, &paint, &Stroke::default(), crate::ui::draw::tf(), None);
        }
        let k = crate::ui::draw::scale();
        let rows = (((h - 2.0 * pad) * k).ceil().max(1.0) as u32).min(inner.height());
        if let Some(part) = tiny_skia::IntRect::from_xywh(0, 0, inner.width(), rows).and_then(|r| inner.clone_rect(r)) {
            crate::ui::draw::blit_px(self.canvas, &part, crate::ui::draw::px(x + pad), crate::ui::draw::px(y + pad), 1.0);
        }
        self.ctx.glass.push((self.offset.0 + x, self.offset.1 + y, width, h, radius));
        self.cursor_y = self.cursor_y.max(y + h + self.ctx.theme.spacing);
        (x, y, width, h)
    }

    /// Linha de resultado (buscador): ícone de 28 px, título, detalhe cinza à
    /// direita; selecionada, na cor de destaque. O ícone pode ser nome do tema, de app
    /// ou caminho de arquivo.
    pub fn result_row(&mut self, icon: &str, title: &str, detail: &str, selected: bool) -> Response {
        let t = self.ctx.theme.clone();
        let id = self.ctx.id();
        let rect = self.alloc(None, 44.0);
        // Resultados ficam colados uns nos outros (lista do buscador).
        if self.flow == Flow::Column {
            self.cursor_y -= t.spacing - 2.0;
        }
        let r = self.interact(id, rect);
        if !self.visible(rect) {
            return r;
        }
        if selected {
            fill_rounded(self.canvas, rect.0, rect.1, rect.2, rect.3, 9.0, color(t.accent));
        } else if r.hovered {
            fill_rounded(self.canvas, rect.0, rect.1, rect.2, rect.3, 9.0, color(t.hover));
        }
        let pix = self.ctx.icon(&[icon, "application-x-executable", "text-x-generic"], 28, true);
        if let Some(pix) = pix {
            crate::ui::draw::blit(self.canvas, &pix, (rect.0 + 10.0) as i32, (rect.1 + 8.0) as i32, 1.0);
        }
        let (main, side) = if selected { (t.on_accent, [230, 236, 255, 255]) } else { (t.text, t.text_secondary) };
        let dw = if detail.is_empty() { 0.0 } else { self.measure(detail, t.font_small, FontWeight::Regular) + 16.0 };
        let shown = self.fit(title, t.font + 1.0, FontWeight::Regular, (rect.2 - 56.0 - dw).max(0.0));
        self.draw_text(&shown, rect.0 + 48.0, rect.1 + (44.0 - (t.font + 1.0) * 1.25) / 2.0, t.font + 1.0, FontWeight::Regular, main);
        if !detail.is_empty() {
            self.draw_text(detail, rect.0 + rect.2 - dw + 4.0, rect.1 + (44.0 - t.font_small * 1.25) / 2.0, t.font_small, FontWeight::Regular, side);
        }
        r
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

    /// Pede um quadro daqui a `delay` (uma vez), mesmo sem mouse nem teclado: para
    /// mostrar dados que mudam devagar (estado do som, da rede) sem redesenhar à toa.
    pub fn wake_after(&mut self, delay: std::time::Duration) {
        let at = std::time::Instant::now() + delay;
        self.ctx.wake_at = Some(self.ctx.wake_at.map_or(at, |w| w.min(at)));
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

    /// Tamanho da janela inteira (diálogos se ajustam a ele).
    pub fn window_size(&self) -> (f32, f32) {
        (self.ctx.window.0 as f32, self.ctx.window.1 as f32)
    }

    /// Parte visível desta região (x, y, largura, altura), por exemplo para o fundo de
    /// uma pasta reagir ao clique no vazio.
    pub fn region(&self) -> (f32, f32, f32, f32) {
        (self.x, self.visible.0, self.width, self.visible.1 - self.visible.0)
    }

    /// Altura que sobra da área atual abaixo do que já foi colocado (para um item que
    /// ocupa o resto da janela, como a área de texto de um editor).
    pub fn remaining_height(&self) -> f32 {
        (self.visible.1 - self.cursor_y).max(0.0)
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
        self.menu_impl(response, response.secondary_clicked, build);
    }

    /// Menu que abre com `open` (clique direito no menu de contexto, clique comum na
    /// lista de escolha) e fica aberto até escolher ou clicar fora.
    fn menu_impl(&mut self, response: &Response, open: bool, build: impl FnOnce(&mut Menu)) {
        if open {
            // Num item (linha, ladrilho), a ponta sai da borda de baixo dele, sem cobrir
            // o nome; numa área grande (fundo), sai do ponto do clique.
            let small = response.rect.3 < 200.0;
            let item_bottom = small.then(|| self.offset.1 + response.rect.1 + response.rect.3);
            let item_top = small.then(|| self.offset.1 + response.rect.1);
            if let Some(click) = self.ctx.input.pointer {
                let pos = (click.0, item_bottom.unwrap_or(click.1));
                let opened = crate::settings_file::effect("menu_animation").then(std::time::Instant::now);
                self.ctx.memory.menu = Some(MenuState { owner: response.id, pos, click, height: 0.0, item_top, opened });
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
        // A ponta do menu fica no ponto do clique (como os menus da dock e da barra):
        // o menu abre para baixo, ou para cima se não couber, e um pouco à esquerda.
        // Sem espaço em cima nem embaixo do item, abre ao lado do clique (à direita, ou
        // à esquerda perto da borda), com a ponta na lateral apontando para o item.
        let MenuPlace { x, y: y0, side, up } = menu_place(&state, w, (ww, wh));
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
        let k = crate::ui::draw::scale();
        let overlay = self.ctx.overlay.get_or_insert_with(|| Pixmap::new((ww * k).max(1.0).round() as u32, (wh * k).max(1.0).round() as u32).unwrap_or_else(|| Pixmap::new(1, 1).unwrap()));
        for (grow, alpha) in [(6.0, 12u8), (3.0, 20), (1.0, 32)] {
            fill_rounded(overlay, x - grow, y0 - grow + 4.0, w + 2.0 * grow, h + 2.0 * grow, 10.0 + grow, color([0, 0, 0, alpha]));
        }
        fill_rounded(overlay, x, y0, w, h, 10.0, color(t.menu));
        match side {
            Some(right) => {
                let tip_y = state.click.1.clamp(y0 + 18.0, (y0 + h - 18.0).max(y0 + 18.0));
                crate::ui::draw::menu_arrow_side(overlay, tip_y, if right { x } else { x + w }, right, t.menu);
            }
            None => {
                let tip_x = state.pos.0.clamp(x + 18.0, x + w - 18.0);
                crate::ui::draw::menu_arrow(overlay, tip_x, if up { y0 + h } else { y0 }, up, t.menu);
            }
        }
        let mut y = y0 + 5.0;
        let text = &mut self.ctx.text;
        for (label, kind, hovered) in rows {
            if kind == 2 {
                fill_rounded(overlay, x + 12.0, y + 4.0, w - 24.0, 1.0, 0.0, color(t.shade(30)));
                y += 9.0;
                continue;
            }
            if hovered {
                fill_rounded(overlay, x + 5.0, y, w - 10.0, 26.0, 6.0, color(t.accent));
            }
            let c = match (kind, hovered) {
                (_, true) => t.on_accent,
                (1, false) => t.danger,
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
            (false, _, true) => t.shade(46),
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
        self.switch_detail(label, "", value)
    }

    /// Interruptor com uma linha de explicação cinza embaixo do texto.
    pub fn switch_detail(&mut self, label: &str, detail: &str, value: &mut bool) -> Response {
        let t = self.ctx.theme.clone();
        let id = self.ctx.id();
        let rect = self.alloc(None, if detail.is_empty() { t.row } else { t.row + 14.0 });
        self.card_line(rect.1);
        let mut r = self.interact(id, rect);
        if r.clicked {
            *value = !*value;
            r.changed = true;
        }
        if !self.visible(rect) {
            return r;
        }
        let label = self.fit(label, t.font, FontWeight::Regular, rect.2 - 60.0);
        if detail.is_empty() {
            self.draw_text(&label, rect.0, rect.1 + (rect.3 - t.font * 1.25) / 2.0, t.font, FontWeight::Regular, t.text);
        } else {
            let top = rect.1 + (rect.3 - t.font * 1.25 - t.font_small * 1.3) / 2.0;
            self.draw_text(&label, rect.0, top, t.font, FontWeight::Regular, t.text);
            let detail = self.fit(detail, t.font_small, FontWeight::Regular, rect.2 - 60.0);
            self.draw_text(&detail, rect.0, top + t.font * 1.3, t.font_small, FontWeight::Regular, t.text_secondary);
        }
        let (sx, sy) = (rect.0 + rect.2 - 40.0, rect.1 + (rect.3 - 22.0) / 2.0);
        let fill = if *value { t.accent } else { t.shade(40) };
        fill_rounded(self.canvas, sx, sy, 40.0, 22.0, 11.0, color(fill));
        let kx = if *value { sx + 29.0 } else { sx + 11.0 };
        if let Some(knob) = PathBuilder::from_circle(kx, sy + 11.0, 9.0) {
            let mut paint = Paint::default();
            paint.anti_alias = true;
            paint.set_color(color([255, 255, 255, 255]));
            self.canvas.fill_path(&knob, &paint, FillRule::Winding, crate::ui::draw::tf(), None);
        }
        r
    }

    /// Desenho livre no canvas da janela (imagens, anotações): `f` recebe o canvas e o
    /// deslocamento da área atual.
    pub fn paint(&mut self, f: impl FnOnce(&mut Pixmap, (f32, f32))) {
        let offset = self.offset;
        f(self.canvas, offset);
    }

    /// Desenho livre num espaço de `height` reservado no lugar do próximo item: `f`
    /// recebe o canvas e o retângulo (x, y, largura, altura) no canvas.
    pub fn custom(&mut self, height: f32, f: impl FnOnce(&mut Pixmap, (f32, f32, f32, f32))) {
        let rect = self.alloc(None, height);
        f(self.canvas, rect);
    }

    /// Vidro atrás de `rect` (na janela): o compositor desfoca o que está atrás dela, e
    /// o app desenha ali com transparência. Devolve se o vidro está ligado nas
    /// Configurações (desligado, o app deve desenhar opaco).
    pub fn glass_behind(&mut self, rect: (f32, f32, f32, f32), radius: f32) -> bool {
        self.ctx.glass_capable = true;
        let on = crate::control_client::panel_glass();
        if on {
            self.ctx.glass.push((self.offset.0 + rect.0, self.offset.1 + rect.1, rect.2, rect.3, radius));
        }
        on
    }

    /// Como `paint`, com o texto do mide-ui para escrever (por exemplo, `draw_mono`).
    pub fn paint_text(&mut self, f: impl FnOnce(&mut Pixmap, &mut crate::ui::text::Text, (f32, f32))) {
        let offset = self.offset;
        f(self.canvas, &mut self.ctx.text, offset);
    }

    /// Põe o texto na área de transferência.
    pub fn copy(&mut self, text: &str) {
        self.ctx.clipboard_out = Some(text.to_string());
    }

    /// Pede o texto da área de transferência: chega num dos próximos quadros em
    /// `input().paste`.
    pub fn request_paste(&mut self) {
        self.ctx.paste_requested = true;
    }

    /// Só esta área (na janela) recebe cliques; fora dela, eles passam para o que
    /// está atrás (camadas transparentes, como `Window::dropdown`). Vale por quadro.
    pub fn input_area(&mut self, rect: (f32, f32, f32, f32)) {
        self.ctx.input_region = Some(rect);
    }

    /// Este `view` desenha a cópia da tela de bloqueio de outro monitor: sem teclas
    /// e sem consumir nada do estado (a resposta da senha é da tela principal).
    pub fn mirror(&self) -> bool {
        self.ctx.mirror
    }

    /// Tela de bloqueio (`Window::lock`): desbloqueia a sessão e fecha o app.
    pub fn unlock(&mut self) {
        self.ctx.unlock_request = true;
    }

    /// Esconde a superfície (sem fechar o app) ou mostra de novo.
    pub fn set_hidden(&mut self, hidden: bool) {
        self.ctx.hide_request = Some(hidden);
    }

    /// Para acordar a janela de outra thread (ver [`Waker`](super::Waker)).
    pub fn waker(&self) -> Option<super::app::Waker> {
        self.ctx.waker.clone()
    }

    /// Linha com um texto e, à direita, o valor escolhido entre `options`; o clique abre
    /// a lista para trocar. `changed` quando outro valor é escolhido.
    pub fn dropdown(&mut self, label: &str, options: &[String], selected: &mut usize) -> Response {
        let t = self.ctx.theme.clone();
        let id = self.ctx.id();
        let rect = self.alloc(None, t.row);
        self.card_line(rect.1);
        let current = options.get(*selected).cloned().unwrap_or_default();
        let value_w = self.measure(&current, t.font, FontWeight::Regular) + 34.0;
        let button = (rect.0 + rect.2 - value_w, rect.1 + (rect.3 - 26.0) / 2.0, value_w, 26.0);
        let mut r = self.interact(id, button);
        let label = self.fit(label, t.font, FontWeight::Regular, rect.2 - value_w - 16.0);
        self.draw_text(&label, rect.0, rect.1 + (rect.3 - t.font * 1.25) / 2.0, t.font, FontWeight::Regular, t.text);
        let mut fill = t.control;
        if r.hovered {
            fill[3] = fill[3].saturating_add(30);
        }
        fill_rounded(self.canvas, button.0, button.1, button.2, button.3, t.control_radius, color(fill));
        self.draw_text(&current, button.0 + 10.0, button.1 + (26.0 - t.font * 1.25) / 2.0, t.font, FontWeight::Regular, t.text);
        // Setinha para baixo.
        let (ax, ay) = (button.0 + button.2 - 16.0, button.1 + 11.0);
        let mut pb = PathBuilder::new();
        pb.move_to(ax - 4.0, ay);
        pb.line_to(ax, ay + 4.0);
        pb.line_to(ax + 4.0, ay);
        if let Some(path) = pb.finish() {
            let mut paint = Paint::default();
            paint.anti_alias = true;
            paint.set_color(color(t.text_secondary));
            let stroke = Stroke { width: 1.6, line_cap: tiny_skia::LineCap::Round, ..Default::default() };
            self.canvas.stroke_path(&path, &paint, &stroke, crate::ui::draw::tf(), None);
        }
        let mut chosen = None;
        self.menu_impl(&r, r.clicked, |m| {
            for (i, option) in options.iter().enumerate() {
                if m.item(option) {
                    chosen = Some(i);
                }
            }
        });
        if let Some(i) = chosen {
            if i != *selected {
                *selected = i;
                r.changed = true;
                self.ctx.repaint = true;
            }
        }
        r
    }

    /// Arranjo das telas (Configurações → Monitores): cada tela é um retângulo em
    /// escala, com o nome; a principal tem a faixa da barra em cima. Clicar escolhe
    /// (`selected`); arrastar move e, ao soltar, encaixa na vizinha (`changed`).
    pub fn arrangement(&mut self, screens: &mut [ArrangedScreen], selected: &mut usize) -> Response {
        let t = self.ctx.theme.clone();
        let id = self.ctx.id();
        let rect = self.alloc(None, 230.0);
        let mut r = Response { rect, id, ..Default::default() };
        fill_rounded(self.canvas, rect.0, rect.1, rect.2, rect.3, 10.0, color(t.shade(14)));
        if screens.is_empty() {
            return r;
        }
        // Desenho em milímetros: cada tela do tamanho físico dela.
        let layout = physical_layout(screens);
        let drag = self.ctx.memory.arrange_drag.filter(|d| d.id == id);
        let (k, origin, min) = match drag {
            Some(d) => (d.k, d.origin, d.min),
            None => {
                let min_x = layout.iter().map(|p| p.0).fold(f32::INFINITY, f32::min);
                let min_y = layout.iter().map(|p| p.1).fold(f32::INFINITY, f32::min);
                let max_x = layout.iter().zip(screens.iter()).map(|(p, s)| p.0 + s.mm.0).fold(f32::NEG_INFINITY, f32::max);
                let max_y = layout.iter().zip(screens.iter()).map(|(p, s)| p.1 + s.mm.1).fold(f32::NEG_INFINITY, f32::max);
                let (bw, bh) = ((max_x - min_x).max(1.0), (max_y - min_y).max(1.0));
                let k = ((rect.2 - 60.0) / bw).min((rect.3 - 50.0) / bh) * 0.85;
                let origin = (rect.0 + (rect.2 - bw * k) / 2.0, rect.1 + (rect.3 - bh * k) / 2.0);
                (k, origin, ((min_x * 1000.0) as i32, (min_y * 1000.0) as i32))
            }
        };
        let min = (min.0 as f32 / 1000.0, min.1 as f32 / 1000.0);
        let to_view = |p: (f32, f32)| (origin.0 + (p.0 - min.0) * k, origin.1 + (p.1 - min.1) * k);
        let pointer = self.pointer();
        let mut drawn: Vec<(f32, f32)> = layout.clone();
        // Começa o arrasto na tela sob o ponteiro (a de cima primeiro).
        if self.ctx.input.pressed && drag.is_none() {
            if let Some((px, py)) = pointer {
                if let Some(i) = (0..screens.len()).rev().find(|&i| {
                    let (x, y) = to_view(layout[i]);
                    px >= x && px <= x + screens[i].mm.0 * k && py >= y && py <= y + screens[i].mm.1 * k
                }) {
                    let (x, y) = to_view(layout[i]);
                    *selected = i;
                    r.clicked = true;
                    self.ctx.memory.active = Some(id);
                    let min_milli = ((min.0 * 1000.0) as i32, (min.1 * 1000.0) as i32);
                    self.ctx.memory.arrange_drag = Some(ArrangeDrag { id, index: i, grab: (px - x, py - y), k, origin, min: min_milli });
                    self.ctx.repaint = true;
                }
            }
        }
        if let (Some(d), Some((px, py))) = (self.ctx.memory.arrange_drag.filter(|d| d.id == id), pointer) {
            // Posição no desenho (milímetros) de onde a tela está sendo arrastada.
            let pos = ((px - d.grab.0 - origin.0) / k + min.0, (py - d.grab.1 - origin.1) / k + min.1);
            if self.ctx.input.down {
                drawn[d.index] = pos;
                self.ctx.repaint = true;
            } else {
                // Soltou: encaixa na vizinha e vira posição lógica.
                screens[d.index].pos = snap_physical(screens, &layout, d.index, pos);
                drawn = physical_layout(screens);
                self.ctx.memory.arrange_drag = None;
                r.changed = true;
                self.ctx.repaint = true;
            }
        }
        for (i, screen) in screens.iter().enumerate() {
            let (x, y) = to_view(drawn[i]);
            let (w, h) = (screen.mm.0 * k, screen.mm.1 * k);
            let chosen = i == *selected;
            fill_rounded(self.canvas, x, y, w, h, 6.0, color(if chosen { [92, 150, 245, 255] } else { [120, 130, 150, 255] }));
            if screen.primary {
                // Faixa da barra superior, como no macOS.
                fill_rounded(self.canvas, x + 2.0, y + 2.0, w - 4.0, (h * 0.07).max(4.0), 2.0, color([255, 255, 255, 220]));
            }
            if chosen {
                if let Some(path) = rounded_rect(x - 2.0, y - 2.0, w + 4.0, h + 4.0, 8.0) {
                    let mut paint = Paint::default();
                    paint.anti_alias = true;
                    paint.set_color(color(t.accent));
                    self.canvas.stroke_path(&path, &paint, &Stroke { width: 2.0, ..Default::default() }, crate::ui::draw::tf(), None);
                }
            }
            let name = self.fit(&screen.name, t.font_small, FontWeight::Semibold, w - 8.0);
            let tw = self.measure(&name, t.font_small, FontWeight::Semibold);
            self.draw_text(&name, x + (w - tw) / 2.0, y + (h - t.font_small * 1.25) / 2.0, t.font_small, FontWeight::Semibold, [255, 255, 255, 255]);
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
        // Também no quadro em que solta: quem relê o valor a cada quadro (do arquivo)
        // recebe o ponto final do arraste junto com o `submitted`.
        if self.ctx.memory.active == Some(id) && (self.ctx.input.down || self.ctx.input.released) {
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
        // Soltou o trilho: `submitted` (para gravar só no fim do arraste).
        if self.ctx.memory.active == Some(id) && self.ctx.input.released {
            r.submitted = true;
        }
        let label = self.fit(label, t.font, FontWeight::Regular, label_w - 12.0);
        self.draw_text(&label, rect.0, rect.1 + (rect.3 - t.font * 1.25) / 2.0, t.font, FontWeight::Regular, t.text);
        let k = ((*value - min) / (max - min).max(f32::EPSILON)).clamp(0.0, 1.0);
        let cy = rect.1 + rect.3 / 2.0;
        fill_rounded(self.canvas, tx, cy - 3.0, tw, 6.0, 3.0, color(t.shade(30)));
        fill_rounded(self.canvas, tx, cy - 3.0, tw * k, 6.0, 3.0, color(t.accent));
        if let Some(knob) = PathBuilder::from_circle(tx + tw * k, cy, 9.0) {
            let mut paint = Paint::default();
            paint.anti_alias = true;
            paint.set_color(color([255, 255, 255, 255]));
            self.canvas.fill_path(&knob, &paint, FillRule::Winding, crate::ui::draw::tf(), None);
            paint.set_color(color([0, 0, 0, 60]));
            self.canvas.stroke_path(&knob, &paint, &Stroke::default(), crate::ui::draw::tf(), None);
        }
        let shown = format!("{}", value.round() as i64);
        self.draw_text(&shown, tx + tw + 16.0, rect.1 + (rect.3 - t.font_small * 1.25) / 2.0, t.font_small, FontWeight::Regular, t.text_secondary);
        r
    }

    /// Campo de texto de uma linha. Clique dá o foco; Enter responde `submitted`.
    pub fn text_input(&mut self, value: &mut String, placeholder: &str) -> Response {
        use super::textedit::Edit;
        let t = self.ctx.theme.clone();
        let id = self.ctx.id();
        let rect = self.alloc(None, t.row);
        let mut r = self.interact(id, rect);
        // Posição do texto no campo e onde o ponteiro está nele (para o cursor).
        let inner = (rect.0 + 10.0, rect.2 - 20.0);
        let pointer_x = self.ctx.input.pointer.map(|(px, _)| px - self.offset.0 - inner.0 + self.ctx.memory.text_scroll);
        if std::mem::take(&mut self.ctx.memory.focus_next) {
            self.ctx.memory.focus = Some(id);
            self.ctx.memory.edit = Edit::all(value);
            self.ctx.memory.text_scroll = 0.0;
            self.ctx.repaint = true;
        } else if r.hovered && self.ctx.input.pressed {
            // Clique: foco e cursor no ponto clicado (arrastar seleciona).
            if self.ctx.memory.focus != Some(id) {
                self.ctx.memory.text_scroll = 0.0;
            }
            self.ctx.memory.focus = Some(id);
            let at = pointer_x.map_or(value.len(), |x| self.index_at(value, x));
            self.ctx.memory.edit = Edit { cursor: at, anchor: Some(at) };
            self.ctx.repaint = true;
        } else if self.ctx.input.pressed && !r.hovered && self.ctx.memory.focus == Some(id) {
            self.ctx.memory.focus = None;
        }
        let focused = self.ctx.memory.focus == Some(id);
        if focused {
            let mut edit = self.ctx.memory.edit;
            edit.clamp(value);
            let before = value.clone();
            let input = &self.ctx.input;
            // Arrastar com o botão apertado estende a seleção; duplo clique pega a palavra.
            if input.down && !input.pressed && self.ctx.memory.active == Some(id) {
                if let Some(x) = pointer_x {
                    let at = self.index_at(value, x);
                    edit.move_to(at, true);
                }
            }
            if r.double_clicked {
                edit.select_word(value, edit.cursor);
            }
            let input = &self.ctx.input;
            if !input.text.is_empty() {
                edit.insert(value, &input.text.clone());
            }
            if let Some(pasted) = input.paste.clone() {
                // Uma linha só: quebras de linha viram espaço.
                edit.insert(value, &pasted.replace(['\r', '\n'], " "));
            }
            for (key, ctrl, shift) in self.ctx.input.key_events() {
                match key {
                    Key::Backspace => {
                        edit.backspace(value, ctrl);
                    }
                    Key::Delete => {
                        edit.delete(value, ctrl);
                    }
                    Key::Left => edit.left(value, ctrl, shift),
                    Key::Right => edit.right(value, ctrl, shift),
                    Key::Home => edit.move_to(0, shift),
                    Key::End => edit.move_to(value.len(), shift),
                    Key::Enter => r.submitted = true,
                    Key::Escape => self.ctx.memory.focus = None,
                    _ => {}
                }
            }
            // Ctrl+A, C, X e V são do campo (o app não recebe esses atalhos).
            let shifts = std::mem::take(&mut self.ctx.input.shortcut_shift);
            for (i, shortcut) in std::mem::take(&mut self.ctx.input.shortcuts).into_iter().enumerate() {
                let shift = shifts.get(i).copied().unwrap_or(false);
                match shortcut.as_str() {
                    "a" => edit = Edit::all(value),
                    "c" => self.ctx.clipboard_out = edit.selected(value).map(str::to_string),
                    "x" => {
                        self.ctx.clipboard_out = edit.selected(value).map(str::to_string);
                        edit.delete_selection(value);
                    }
                    "v" => self.ctx.paste_requested = true,
                    _ => {
                        self.ctx.input.shortcuts.push(shortcut);
                        self.ctx.input.shortcut_shift.push(shift);
                    }
                }
            }
            r.changed = *value != before;
            if edit != self.ctx.memory.edit || r.changed || r.submitted {
                self.ctx.repaint = true;
            }
            self.ctx.memory.edit = edit;
        }
        fill_rounded(self.canvas, rect.0, rect.1, rect.2, rect.3, t.control_radius, color(t.card));
        let border = if focused { t.accent } else { t.shade(40) };
        if let Some(path) = rounded_rect(rect.0 + 0.5, rect.1 + 0.5, rect.2 - 1.0, rect.3 - 1.0, t.control_radius) {
            let mut paint = Paint::default();
            paint.anti_alias = true;
            paint.set_color(color(border));
            let width = if focused { 2.0 } else { 1.0 };
            self.canvas.stroke_path(&path, &paint, &Stroke { width, ..Default::default() }, crate::ui::draw::tf(), None);
        }
        let ty = rect.1 + (rect.3 - t.font * 1.25) / 2.0;
        if value.is_empty() && !focused {
            self.draw_text(placeholder, inner.0, ty, t.font, FontWeight::Regular, t.text_secondary);
        } else if !focused {
            let shown = self.fit(value, t.font, FontWeight::Regular, inner.1 - 4.0);
            self.draw_text(&shown, inner.0, ty, t.font, FontWeight::Regular, t.text);
        } else {
            self.edit_text(value, rect, inner, ty);
        }
        r
    }

    /// Texto do campo com foco: rolado para o cursor ficar à vista, com a seleção e o
    /// cursor, recortado dentro do campo.
    fn edit_text(&mut self, value: &str, rect: (f32, f32, f32, f32), inner: (f32, f32), ty: f32) {
        let t = self.ctx.theme.clone();
        let edit = self.ctx.memory.edit;
        let cursor_x = self.measure(&value[..edit.cursor], t.font, FontWeight::Regular);
        let total = self.measure(value, t.font, FontWeight::Regular);
        let mut scroll = self.ctx.memory.text_scroll;
        if cursor_x - scroll > inner.1 {
            scroll = cursor_x - inner.1;
        }
        if cursor_x < scroll {
            scroll = cursor_x;
        }
        scroll = scroll.clamp(0.0, (total - inner.1).max(0.0));
        self.ctx.memory.text_scroll = scroll;
        // Desenha numa imagem do tamanho de dentro do campo: o que passa fica de fora.
        let (w, h) = ((inner.1 + 4.0).ceil().max(1.0) as u32, rect.3.ceil().max(1.0) as u32);
        let k = crate::ui::draw::scale();
        let Some(mut clip) = Pixmap::new((w as f32 * k).ceil() as u32, (h as f32 * k).ceil() as u32) else {
            return;
        };
        let x0 = 2.0 - scroll;
        if let Some((a, b)) = edit.selection() {
            let (xa, xb) = (self.measure(&value[..a], t.font, FontWeight::Regular), self.measure(&value[..b], t.font, FontWeight::Regular));
            let [r_, g, b_, _] = t.accent;
            fill_rounded(&mut clip, x0 + xa - 1.0, 6.0, xb - xa + 2.0, rect.3 - 12.0, 3.0, color([r_, g, b_, 70]));
        }
        self.ctx.text.draw(&mut clip, value, x0, ty - rect.1, t.font, FontWeight::Regular, t.text);
        if edit.selection().is_none() {
            fill_rounded(&mut clip, x0 + cursor_x, 7.0, 1.5, rect.3 - 14.0, 0.5, color(t.text));
        }
        self.canvas.draw_pixmap(crate::ui::draw::px((inner.0 - 2.0).floor()), crate::ui::draw::px(rect.1.floor()), clip.as_ref(), &tiny_skia::PixmapPaint::default(), Transform::identity(), None);
    }

    /// Posição no texto (índice de byte) mais perto de `x`, medido do começo do texto.
    fn index_at(&mut self, value: &str, x: f32) -> usize {
        let font = self.ctx.theme.font;
        let mut left = 0.0;
        for (i, c) in value.char_indices() {
            let right = self.measure(&value[..i + c.len_utf8()], font, FontWeight::Regular);
            if x < (left + right) / 2.0 {
                return i;
            }
            left = right;
        }
        value.len()
    }

    /// Campo de senha (telas de bloqueio e de senha): mostra bolinhas, recebe o
    /// teclado sempre (é o único campo da tela) e não copia. Enter responde
    /// `submitted`; Esc e Ctrl+U apagam tudo; Backspace apaga o último caractere.
    pub fn password_input(&mut self, value: &mut String, placeholder: &str) -> Response {
        let t = self.ctx.theme.clone();
        let rect = self.alloc(None, t.row + 6.0);
        let id = self.ctx.id();
        let mut r = Response { rect, id, ..Default::default() };
        let input = self.ctx.input.clone();
        let before = value.len();
        value.extend(input.text.chars().filter(|c| !c.is_control()));
        if let Some(pasted) = &input.paste {
            value.extend(pasted.chars().filter(|c| !c.is_control()));
        }
        for (key, ctrl, _) in input.key_events() {
            match key {
                Key::Backspace if ctrl => value.clear(),
                Key::Backspace => {
                    value.pop();
                }
                Key::Escape => value.clear(),
                Key::Enter => r.submitted = true,
                _ => {}
            }
        }
        for shortcut in &input.shortcuts {
            match shortcut.as_str() {
                "u" => value.clear(),
                "v" => self.ctx.paste_requested = true,
                _ => {}
            }
        }
        r.changed = value.len() != before;
        if r.changed || r.submitted {
            self.ctx.repaint = true;
        }
        fill_rounded(self.canvas, rect.0, rect.1, rect.2, rect.3, rect.3 / 2.0, color([255, 255, 255, 120]));
        let ty = rect.1 + (rect.3 - t.font * 1.25) / 2.0;
        let x = rect.0 + 16.0;
        if value.is_empty() {
            self.draw_text(placeholder, x, ty, t.font, FontWeight::Regular, [40, 40, 48, 170]);
        } else {
            let dots = "•".repeat(value.chars().count().min(40));
            let w = self.measure(&dots, t.font + 2.0, FontWeight::Regular);
            self.draw_text(&dots, x, ty - 1.0, t.font + 2.0, FontWeight::Regular, [24, 24, 28, 255]);
            fill_rounded(self.canvas, x + w + 2.0, rect.1 + 8.0, 1.5, rect.3 - 16.0, 0.5, color([24, 24, 28, 255]));
        }
        r
    }

    /// Área de texto de várias linhas (editor): números de linha, cursor, seleção com
    /// mouse e teclado, desfazer (Ctrl+Z) e refazer (Ctrl+Shift+Z, Ctrl+Y), área de
    /// transferência e rolagem. Só as linhas à vista são desenhadas. Os outros atalhos
    /// com Ctrl (salvar, buscar...) ficam para o app. `changed` diz se o texto mudou.
    pub fn text_area(&mut self, doc: &mut super::textarea::TextDoc, height: f32) -> Response {
        use super::textarea::{index_in_line, line_end, line_of, Change};
        use super::textedit::Edit;
        let t = self.ctx.theme.clone();
        let size = t.font;
        let line_h = (size * 1.5).round();
        let pad = 6.0;
        let id = self.ctx.id();
        let rect = self.alloc(None, height);
        let mut r = self.interact(id, rect);
        let mut starts = doc.line_starts();
        let digits = starts.len().to_string().len().max(2);
        let gutter = self.ctx.text.measure_mono(&"0".repeat(digits), size * 0.9) + 20.0;
        let text_x = rect.0 + gutter + 8.0;
        let view = (rect.2 - gutter - 16.0, rect.3 - 2.0 * pad);
        let rows = (view.1 / line_h).floor().max(1.0) as usize;
        // Linha e posição sob o ponteiro.
        let pointer_at = |ui: &mut Self, doc: &super::textarea::TextDoc, starts: &[usize]| -> Option<usize> {
            let (px, py) = ui.pointer()?;
            let line = (((py - rect.1 - pad + doc.scroll) / line_h).floor().max(0.0) as usize).min(starts.len() - 1);
            let x = px - text_x + doc.scroll_x;
            Some(index_in_line(&doc.text, starts, line, x, |s| ui.ctx.text.measure_mono(s, size)))
        };
        if std::mem::take(&mut self.ctx.memory.focus_next) {
            self.ctx.memory.focus = Some(id);
        }
        if r.hovered && self.ctx.input.pressed {
            self.ctx.memory.focus = Some(id);
            if let Some(at) = pointer_at(self, doc, &starts) {
                doc.edit = Edit { cursor: at, anchor: Some(at) };
                doc.goal_x = None;
            }
            self.ctx.repaint = true;
        }
        if r.hovered && self.ctx.input.scroll != 0.0 {
            doc.scroll += std::mem::take(&mut self.ctx.input.scroll);
            self.ctx.repaint = true;
        }
        let focused = self.ctx.memory.focus == Some(id);
        let mut moved = false;
        if focused {
            doc.edit.clamp(&doc.text);
            let before = (doc.edit, doc.version);
            if self.ctx.input.down && !self.ctx.input.pressed && self.ctx.memory.active == Some(id) {
                if let Some(at) = pointer_at(self, doc, &starts) {
                    doc.edit.move_to(at, true);
                    moved = true;
                }
            }
            if r.double_clicked {
                let at = doc.edit.cursor;
                doc.edit.select_word(&doc.text, at);
            }
            let typed = std::mem::take(&mut self.ctx.input.text);
            doc.type_text(&typed);
            if let Some(pasted) = self.ctx.input.paste.take() {
                doc.before_change(Change::Other);
                doc.edit.insert(&mut doc.text, &pasted.replace("\r\n", "\n"));
                moved = true;
            }
            for (key, ctrl, shift) in self.ctx.input.key_events() {
                starts = doc.line_starts();
                let line = line_of(&starts, doc.edit.cursor);
                // Coluna em pixels do cursor (para subir e descer mantendo a coluna).
                let col_x = |ui: &mut Self, doc: &super::textarea::TextDoc| {
                    doc.goal_x.unwrap_or_else(|| ui.ctx.text.measure_mono(&doc.text[starts[line]..doc.edit.cursor], size))
                };
                let vertical = |ui: &mut Self, doc: &mut super::textarea::TextDoc, to: usize| {
                    let x = col_x(ui, doc);
                    let at = index_in_line(&doc.text, &starts, to, x, |s| ui.ctx.text.measure_mono(s, size));
                    doc.edit.move_to(at, shift);
                    doc.goal_x = Some(x);
                };
                match key {
                    Key::Enter => doc.newline(),
                    Key::Tab => doc.type_text("    "),
                    Key::Backspace | Key::Delete => {
                        let mut probe = (doc.text.clone(), doc.edit);
                        let deleted = if key == Key::Backspace { probe.1.backspace(&mut probe.0, ctrl) } else { probe.1.delete(&mut probe.0, ctrl) };
                        if deleted {
                            doc.before_change(Change::Deleting);
                            (doc.text, doc.edit) = probe;
                        }
                        doc.goal_x = None;
                    }
                    Key::Left => {
                        doc.edit.left(&doc.text, ctrl, shift);
                        doc.goal_x = None;
                    }
                    Key::Right => {
                        doc.edit.right(&doc.text, ctrl, shift);
                        doc.goal_x = None;
                    }
                    Key::Up if line > 0 => vertical(self, doc, line - 1),
                    Key::Up => doc.edit.move_to(0, shift),
                    Key::Down if line + 1 < starts.len() => vertical(self, doc, line + 1),
                    Key::Down => doc.edit.move_to(doc.text.len(), shift),
                    Key::PageUp => vertical(self, doc, line.saturating_sub(rows)),
                    Key::PageDown => vertical(self, doc, (line + rows).min(starts.len() - 1)),
                    Key::Home => {
                        doc.edit.move_to(if ctrl { 0 } else { starts[line] }, shift);
                        doc.goal_x = None;
                    }
                    Key::End => {
                        let end = if ctrl { doc.text.len() } else { line_end(&doc.text, &starts, line) };
                        doc.edit.move_to(end, shift);
                        doc.goal_x = None;
                    }
                    _ => continue,
                }
                moved = true;
            }
            let shifts = std::mem::take(&mut self.ctx.input.shortcut_shift);
            for (i, shortcut) in std::mem::take(&mut self.ctx.input.shortcuts).into_iter().enumerate() {
                let shift = shifts.get(i).copied().unwrap_or(self.ctx.input.shift);
                match shortcut.as_str() {
                    "a" => doc.edit = Edit::all(&doc.text),
                    "c" => self.ctx.clipboard_out = doc.edit.selected(&doc.text).map(str::to_string),
                    "x" => {
                        if let Some(cut) = doc.edit.selected(&doc.text).map(str::to_string) {
                            doc.before_change(Change::Other);
                            doc.edit.delete_selection(&mut doc.text);
                            self.ctx.clipboard_out = Some(cut);
                        }
                    }
                    "v" => self.ctx.paste_requested = true,
                    "z" if shift => {
                        doc.redo();
                    }
                    "z" => {
                        doc.undo();
                    }
                    "y" => {
                        doc.redo();
                    }
                    _ => {
                        self.ctx.input.shortcuts.push(shortcut);
                        self.ctx.input.shortcut_shift.push(shift);
                        continue;
                    }
                }
                moved = true;
            }
            r.changed = doc.version != before.1;
            moved |= r.changed || doc.edit != before.0;
            if moved {
                self.ctx.repaint = true;
            }
        }
        starts = doc.line_starts();
        let cursor_line = line_of(&starts, doc.edit.cursor);
        let cursor_x = self.ctx.text.measure_mono(&doc.text[starts[cursor_line]..doc.edit.cursor], size);
        // O cursor fica à vista depois de mexer nele.
        let content_h = starts.len() as f32 * line_h;
        if moved {
            let top = cursor_line as f32 * line_h;
            if top < doc.scroll {
                doc.scroll = top;
            }
            if top + line_h > doc.scroll + view.1 {
                doc.scroll = top + line_h - view.1;
            }
            if cursor_x < doc.scroll_x {
                doc.scroll_x = (cursor_x - 40.0).max(0.0);
            }
            if cursor_x > doc.scroll_x + view.0 - 8.0 {
                doc.scroll_x = cursor_x - view.0 + 40.0;
            }
        }
        doc.scroll = doc.scroll.clamp(0.0, (content_h - view.1).max(0.0));

        // Desenho: fundo, margem com os números e só as linhas à vista.
        fill_rounded(self.canvas, rect.0, rect.1, rect.2, rect.3, t.control_radius, color(t.card));
        fill_rounded(self.canvas, rect.0, rect.1, gutter, rect.3, t.control_radius, color(t.shade(10)));
        let first = (doc.scroll / line_h).floor() as usize;
        let last = (first + rows + 2).min(starts.len());
        let (w, h) = ((view.0 + 8.0).ceil().max(1.0) as u32, rect.3.ceil().max(1.0) as u32);
        let k = crate::ui::draw::scale();
        let Some(mut clip) = Pixmap::new((w as f32 * k).ceil() as u32, (h as f32 * k).ceil() as u32) else {
            return r;
        };
        let selection = doc.edit.selection();
        let [ar, ag, ab, _] = t.accent;
        for line in first..last {
            let (start, end) = (starts[line], line_end(&doc.text, &starts, line));
            let y = pad + line as f32 * line_h - doc.scroll;
            let x0 = -doc.scroll_x;
            if focused && line == cursor_line && selection.is_none() {
                fill_rounded(&mut clip, 0.0, y, w as f32, line_h, 0.0, color([ar, ag, ab, 14]));
            }
            if let Some((a, b)) = selection.filter(|(a, b)| *a <= end && *b >= start && !(a == b)) {
                let (sa, sb) = (a.max(start), b.min(end));
                let xa = self.ctx.text.measure_mono(&doc.text[start..sa], size);
                let xb = self.ctx.text.measure_mono(&doc.text[start..sb], size);
                // A quebra de linha selecionada aparece como um pouco a mais no fim.
                let extra = if b > end { size * 0.5 } else { 0.0 };
                fill_rounded(&mut clip, x0 + xa, y, xb - xa + extra, line_h, 0.0, color([ar, ag, ab, 70]));
            }
            let content = &doc.text[start..end];
            if !content.is_empty() {
                self.ctx.text.draw_mono(&mut clip, content, x0, y + (line_h - size * 1.25) / 2.0, size, t.text);
            }
            let number = (line + 1).to_string();
            let nw = self.ctx.text.measure_mono(&number, size * 0.9);
            let ny = rect.1 + y + (line_h - size * 0.9 * 1.25) / 2.0;
            if ny > rect.1 && ny < rect.1 + rect.3 - size {
                let shade = if line == cursor_line && focused { t.text } else { t.text_secondary };
                self.ctx.text.draw_mono(self.canvas, &number, rect.0 + gutter - 10.0 - nw, ny, size * 0.9, shade);
            }
        }
        if focused && selection.is_none() {
            let y = pad + cursor_line as f32 * line_h - doc.scroll;
            fill_rounded(&mut clip, cursor_x - doc.scroll_x, y + 3.0, 1.5, line_h - 6.0, 0.5, color(t.text));
        }
        self.canvas.draw_pixmap(crate::ui::draw::px(text_x.floor()), crate::ui::draw::px(rect.1.floor()), clip.as_ref(), &tiny_skia::PixmapPaint::default(), Transform::identity(), None);
        // Indicador de rolagem à direita quando o texto passa da altura.
        if content_h > view.1 {
            let bar_h = (view.1 * view.1 / content_h).max(24.0);
            let bar_y = rect.1 + pad + (view.1 - bar_h) * doc.scroll / (content_h - view.1);
            fill_rounded(self.canvas, rect.0 + rect.2 - 8.0, bar_y, 4.0, bar_h, 2.0, color(t.shade(60)));
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
        // Fora da área à vista: só o lugar e o clique (listas longas desenham só o visível).
        if !self.visible(rect) {
            return r;
        }
        // Destaque como o da barra lateral: pílula arredondada com folga, cinza claro.
        if r.hovered {
            fill_rounded(self.canvas, rect.0 - 8.0, rect.1 + 3.0, rect.2 + 16.0, rect.3 - 6.0, t.control_radius, color(t.control));
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
        let icon = self.ctx.icon(&[name], size, false);
        let rect = self.alloc(Some(size as f32), size as f32);
        if let Some(icon) = icon {
            crate::ui::draw::blit(self.canvas, &icon, rect.0 as i32, rect.1 as i32, 1.0);
        }
    }

    /// Ladrilho de uma grade de ícones (como no Finder): imagem com o nome embaixo.
    /// Selecionado, ganha o vidro do mide atrás e o nome numa pílula na cor de destaque.
    pub fn tile(&mut self, image: Tile, label: &str, selected: bool, size: u32) -> Response {
        self.tile_wide(image, label, selected, size, size)
    }

    /// Ladrilho com área de imagem retangular (`iw` x `ih`), para miniaturas largas
    /// (papéis de parede). Ícones ficam centralizados no lado menor.
    pub fn tile_wide(&mut self, image: Tile, label: &str, selected: bool, iw: u32, ih: u32) -> Response {
        let t = self.ctx.theme.clone();
        let id = self.ctx.id();
        let (fw, fh) = (iw as f32, ih as f32);
        let (w, h) = (fw + 44.0, fh + 46.0);
        let rect = self.alloc(Some(w), h);
        let r = self.interact(id, rect);
        if !self.visible(rect) {
            return r;
        }
        let (ix, iy) = (rect.0 + (w - fw) / 2.0, rect.1 + 6.0);
        if selected {
            crate::ui::draw::glass(self.canvas, ix - 8.0, iy - 6.0, fw + 16.0, fh + 12.0, 10.0);
        } else if r.hovered {
            fill_rounded(self.canvas, ix - 8.0, iy - 6.0, fw + 16.0, fh + 12.0, 10.0, color(t.control));
        }
        match image {
            Tile::Image(pix) => {
                let (pw, ph) = (pix.width() as f32, pix.height() as f32);
                let (dx, dy) = (ix + (fw - pw) / 2.0, iy + (fh - ph) / 2.0);
                crate::ui::draw::blit_image(self.canvas, pix, dx.floor(), dy.floor());
            }
            Tile::Icon(name) => {
                let side = iw.min(ih);
                let icon = self.ctx.icon(&[name, "text-x-generic"], side, false);
                if let Some(icon) = icon {
                    let (dx, dy) = (ix + (fw - side as f32) / 2.0, iy + (fh - side as f32) / 2.0);
                    crate::ui::draw::blit(self.canvas, &icon, dx as i32, dy as i32, 1.0);
                }
            }
        }
        let max = w - 6.0;
        let shown = self.fit(label, t.font_small, FontWeight::Regular, max);
        let lw = self.measure(&shown, t.font_small, FontWeight::Regular);
        let (lx, ly) = (rect.0 + (w - lw) / 2.0, iy + fh + 10.0);
        if selected {
            fill_rounded(self.canvas, lx - 5.0, ly - 1.0, lw + 10.0, t.font_small * 1.4, 5.0, color(t.accent));
        }
        let c = if selected { t.on_accent } else { t.tile_text };
        // Nome claro (sobre o papel de parede): sombra escura embaixo para ler bem.
        let light = (t.tile_text[0] as u32 + t.tile_text[1] as u32 + t.tile_text[2] as u32) > 600;
        if light && !selected {
            self.draw_text(&shown, lx, ly + 1.0, t.font_small, FontWeight::Regular, [0, 0, 0, 160]);
        }
        self.draw_text(&shown, lx, ly, t.font_small, FontWeight::Regular, c);
        r
    }

    /// Monta componentes num lugar escolhido (`x`, `y`) desta área, com `width` de
    /// largura, em coluna (para posições livres, como os ícones da área de trabalho).
    pub fn at(&mut self, x: f32, y: f32, width: f32, add: impl FnOnce(&mut Ui)) {
        let saved = (self.x, self.width, self.cursor_y, self.flow, self.row_x, self.row_h);
        self.x = x;
        self.width = width;
        self.cursor_y = y;
        self.flow = Flow::Column;
        add(self);
        (self.x, self.width, self.cursor_y, self.flow, self.row_x, self.row_h) = saved;
    }

    /// Cabeçalho de uma lista em colunas (como no Finder): "Nome" à esquerda e as
    /// outras colunas à direita, com as larguras de `columns`. `sorted` é a coluna
    /// ordenada (0 = nome) e se é crescente. Devolve a coluna clicada.
    pub fn list_header(&mut self, name: &str, columns: &[(&str, f32)], sorted: (usize, bool)) -> Option<usize> {
        let t = self.ctx.theme.clone();
        let rect = self.alloc(None, 24.0);
        let cols: f32 = columns.iter().map(|c| c.1).sum();
        let mut clicked = None;
        let mut spans = vec![(name, rect.0, rect.2 - cols)];
        let mut cx = rect.0 + rect.2 - cols;
        for (label, w) in columns {
            spans.push((label, cx, *w));
            cx += w;
        }
        for (i, (label, x, w)) in spans.into_iter().enumerate() {
            let id = self.ctx.id();
            let cell = (x, rect.1, w, rect.3);
            let r = self.interact(id, cell);
            if r.clicked {
                clicked = Some(i);
            }
            if r.hovered {
                fill_rounded(self.canvas, x, rect.1 + 2.0, w - 4.0, rect.3 - 4.0, 4.0, color(t.hover));
            }
            let lx = if i == 0 { x + 36.0 } else { x };
            let weight = if sorted.0 == i { FontWeight::Semibold } else { FontWeight::Regular };
            self.draw_text(label, lx, rect.1 + (24.0 - t.font_small * 1.25) / 2.0, t.font_small, weight, t.text_secondary);
            if sorted.0 == i {
                let lw = self.measure(label, t.font_small, weight);
                let arrow = if sorted.1 { "▲" } else { "▼" };
                self.draw_text(arrow, lx + lw + 6.0, rect.1 + (24.0 - 9.0 * 1.25) / 2.0, 9.0, FontWeight::Regular, t.text_secondary);
            }
        }
        fill_rounded(self.canvas, rect.0, rect.1 + rect.3 - 1.0, rect.2, 1.0, 0.0, color(t.line));
        // Sem espaço entre o cabeçalho e as linhas.
        if self.flow == Flow::Column {
            self.cursor_y -= t.spacing;
        }
        clicked
    }

    /// Linha de lista em árvore (visualização em lista do Finder): recuo pelo nível,
    /// setinha para abrir pastas (`expandable`), ícone, nome e colunas à direita.
    /// `striped` pinta o fundo cinza das linhas alternadas. Devolve a resposta da
    /// linha e se a setinha foi clicada.
    #[allow(clippy::too_many_arguments)]
    pub fn tree_row(&mut self, depth: usize, expandable: bool, expanded: bool, icon: &str, label: &str, columns: &[(&str, f32)], selected: bool, striped: bool) -> (Response, bool) {
        let t = self.ctx.theme.clone();
        let rect = self.alloc(None, 26.0);
        if self.flow == Flow::Column {
            self.cursor_y -= t.spacing;
        }
        let indent = rect.0 + 6.0 + depth as f32 * 18.0;
        // A setinha é um alvo próprio, antes da linha (que fica com o resto).
        let mut toggled = false;
        if expandable {
            let id = self.ctx.id();
            let r = self.interact(id, (indent - 4.0, rect.1, 22.0, rect.3));
            toggled = r.clicked;
        }
        let id = self.ctx.id();
        let r = self.interact(id, rect);
        if !self.visible(rect) {
            return (r, toggled);
        }
        if selected {
            fill_rounded(self.canvas, rect.0, rect.1, rect.2, rect.3, 5.0, color(t.accent));
        } else if striped {
            fill_rounded(self.canvas, rect.0, rect.1, rect.2, rect.3, 0.0, color(t.shade(10)));
        }
        let (main, side) = if selected { (t.on_accent, t.on_accent) } else { (t.text, t.text_secondary) };
        if expandable {
            // Triângulo: para a direita fechado, para baixo aberto.
            let (cx, cy) = (indent + 6.0, rect.1 + 13.0);
            let mut pb = PathBuilder::new();
            if expanded {
                pb.move_to(cx - 4.0, cy - 2.0);
                pb.line_to(cx + 4.0, cy - 2.0);
                pb.line_to(cx, cy + 3.0);
            } else {
                pb.move_to(cx - 2.0, cy - 4.0);
                pb.line_to(cx + 3.0, cy);
                pb.line_to(cx - 2.0, cy + 4.0);
            }
            pb.close();
            if let Some(path) = pb.finish() {
                let mut paint = Paint::default();
                paint.anti_alias = true;
                paint.set_color(color(if selected { t.on_accent } else { t.text_secondary }));
                self.canvas.fill_path(&path, &paint, FillRule::Winding, crate::ui::draw::tf(), None);
            }
        }
        let ix = indent + 16.0;
        if let Some(pix) = self.ctx.icon(&[icon, "text-x-generic"], 18, false) {
            crate::ui::draw::blit(self.canvas, &pix, ix as i32, (rect.1 + 4.0) as i32, 1.0);
        }
        let cols: f32 = columns.iter().map(|c| c.1).sum();
        let shown = self.fit(label, t.font, FontWeight::Regular, (rect.0 + rect.2 - cols - ix - 30.0).max(0.0));
        self.draw_text(&shown, ix + 24.0, rect.1 + (26.0 - t.font * 1.25) / 2.0, t.font, FontWeight::Regular, main);
        let mut cx = rect.0 + rect.2 - cols;
        for (text, w) in columns {
            let shown = self.fit(text, t.font_small, FontWeight::Regular, w - 8.0);
            self.draw_text(&shown, cx, rect.1 + (26.0 - t.font_small * 1.25) / 2.0, t.font_small, FontWeight::Regular, side);
            cx += w;
        }
        (r, toggled)
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
        let pix = self.ctx.icon(&[icon, "text-x-generic"], 20, false);
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
        let pix = self.ctx.icon(&[icon], 16, false);
        match pix {
            // Ícones "symbolic" na cor do texto (como os da barra superior; no tema
            // escuro acompanham); desativados, na cor secundária.
            Some(pix) if icon.ends_with("-symbolic") => {
                let c = if enabled { t.text } else { t.text_secondary };
                crate::ui::draw::blit_tinted(self.canvas, &pix, (rect.0 + 8.0) as i32, (rect.1 + 6.0) as i32, [c[0], c[1], c[2]]);
            }
            Some(pix) => {
                crate::ui::draw::blit(self.canvas, &pix, (rect.0 + 8.0) as i32, (rect.1 + 6.0) as i32, if enabled { 1.0 } else { 0.35 });
            }
            // Texto de reserva só se o ícone não existe (não enquanto carrega).
            // (A chave tem o tamanho em pixels, como em `Ctx::icon`.)
            None if self.ctx.icons.contains_key(&format!("{icon}@{}", (16.0 * crate::ui::draw::scale()).round() as u32)) => {
                let c = if enabled { t.text } else { t.text_secondary };
                let fw = self.measure(fallback, 16.0, FontWeight::Semibold);
                self.draw_text(fallback, rect.0 + (32.0 - fw) / 2.0, rect.1 + 3.0, 16.0, FontWeight::Semibold, c);
            }
            None => {}
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
        let k = crate::ui::draw::scale();
        let mut canvas = Pixmap::new((width.max(1.0) * k).ceil() as u32, (h as f32 * k).ceil() as u32).unwrap_or_else(|| Pixmap::new(1, 1).unwrap());
        if copy {
            copy_pixels(self.canvas, crate::ui::draw::px(x), crate::ui::draw::px(top), &mut canvas, 0, 0);
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
            sidebar_glass: false,
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
            self.canvas.stroke_path(&path, &paint, &Stroke::default(), crate::ui::draw::tf(), None);
        }
        if bottom > top {
            // Recorta no fim do cartão (a imagem cobre a faixa visível inteira).
            let k = crate::ui::draw::scale();
            let clip_h = (((y0 + h - pad_y) - top) * k).min(inner.height() as f32).max(0.0) as u32;
            if clip_h > 0 {
                if let Some(part) = inner.clone_rect(tiny_skia::IntRect::from_xywh(0, 0, inner.width(), clip_h).unwrap()) {
                    self.canvas.draw_pixmap(crate::ui::draw::px(x + pad_x), crate::ui::draw::px(top), part.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
                }
            }
        }
        self.cursor_y = y0 + h + t.spacing;
    }

    /// Área que rola, ocupando o resto da altura, com margem do tema e barra de
    /// rolagem fina à direita quando o conteúdo não cabe.
    pub fn page(&mut self, add: impl FnOnce(&mut Ui)) {
        let h = self.visible.1 - self.cursor_y;
        let pad = self.ctx.theme.padding;
        self.scroll_impl(h, pad, add);
    }

    /// Área que rola com altura fixa (`height`), sem margem: listas dentro de
    /// diálogos (o seletor de arquivos) e painéis.
    pub fn scroll_area(&mut self, height: f32, add: impl FnOnce(&mut Ui)) {
        self.scroll_impl(height, 0.0, add);
    }

    fn scroll_impl(&mut self, h: f32, pad: f32, add: impl FnOnce(&mut Ui)) {
        let id = self.ctx.id();
        let (x, y0) = (self.x, self.cursor_y);
        let w = self.width;
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
            copy_back(&inner, self.canvas, crate::ui::draw::px(x + pad), crate::ui::draw::px(y0));
            fill_rounded(self.canvas, bar.0 + 2.0 + (8.0 - tw) / 2.0, thumb_y(scroll), tw, thumb_h, tw / 2.0, color(self.ctx.theme.shade(if active { 120 } else { 80 })));
        } else {
            // Sem rolagem: se havia rolagem guardada (a janela cresceu), redesenha.
            if scroll != 0.0 {
                self.ctx.repaint = true;
            }
            scroll = 0.0;
            copy_back(&inner, self.canvas, crate::ui::draw::px(x + pad), crate::ui::draw::px(y0));
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
        let items: Vec<(&str, &str, Option<[u8; 3]>)> = items.iter().map(|(n, i)| (*n, *i, None)).collect();
        let raw = std::mem::take(&mut self.sidebar_raw);
        self.sidebar_impl(&items, selected, None, raw, content)
    }

    /// Barra lateral das Configurações: campo de busca no topo e cada seção com o
    /// ícone num quadradinho colorido (`tint`). O conteúdo vai numa página que rola.
    pub fn sidebar_search(
        &mut self,
        items: &[(&str, &str, Option<[u8; 3]>)],
        selected: usize,
        query: &mut String,
        placeholder: &str,
        content: impl FnOnce(&mut Ui, usize),
    ) -> usize {
        self.sidebar_impl(items, selected, Some((query, placeholder)), false, content)
    }

    fn sidebar_impl(
        &mut self,
        items: &[(&str, &str, Option<[u8; 3]>)],
        selected: usize,
        search: Option<(&mut String, &str)>,
        raw: bool,
        content: impl FnOnce(&mut Ui, usize),
    ) -> usize {
        let mut selected = selected;
        let t = self.ctx.theme.clone();
        let side_w = 220.0;
        let (x, y0) = (self.x, self.cursor_y);
        let h = self.visible.1 - y0;
        // Barra lateral de vidro (como o acesso rápido do Finder): o mesmo véu dos
        // menus da barra, com o compositor desfocando o que está atrás da janela.
        // Com o vidro desligado nas Configurações, fica opaca.
        if std::mem::take(&mut self.sidebar_glass) {
            self.ctx.glass_capable = true;
            if crate::control_client::panel_glass() {
                crate::ui::draw::paint_rounded_solid(self.canvas, x, y0, side_w, h, 0.0, t.sidebar_glass);
                self.ctx.glass.push((self.offset.0 + x, self.offset.1 + y0, side_w, h, 0.0));
            } else {
                fill_rounded(self.canvas, x, y0, side_w, h, 0.0, color(t.sidebar));
            }
        } else {
            fill_rounded(self.canvas, x, y0, side_w, h, 0.0, color(t.sidebar));
        }
        fill_rounded(self.canvas, x + side_w - 1.0, y0, 1.0, h, 0.0, color(t.line));
        let mut row_y = y0 + 16.0;
        if let Some((query, placeholder)) = search {
            // Campo de busca no topo da barra lateral.
            let (old_x, old_w, old_y) = (self.x, self.width, self.cursor_y);
            self.x = x + 10.0;
            self.width = side_w - 20.0;
            self.cursor_y = y0 + 12.0;
            self.text_input(query, placeholder);
            row_y = self.cursor_y + 4.0;
            self.x = old_x;
            self.width = old_w;
            self.cursor_y = old_y;
        }
        for (i, (name, icon, tint)) in items.iter().enumerate() {
            // Item sem ícone é um título de grupo ("Favoritos"); nome vazio é só espaço.
            if icon.is_empty() {
                if name.is_empty() {
                    row_y += 12.0;
                    continue;
                }
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
            if let Some([r_, g, b]) = tint {
                fill_rounded(self.canvas, rect.0 + 8.0, rect.1 + 3.0, 24.0, 24.0, 6.0, color([*r_, *g, *b, 255]));
            }
            // Ícones "symbolic" (de uma cor só) ficam na cor do texto, como os da
            // barra superior; na linha escolhida, brancos.
            let symbolic = icon.ends_with("-symbolic");
            let size = if symbolic { 18 } else { 20 };
            let pix = self.ctx.icon(&[icon], size, false);
            if let Some(pix) = pix {
                let (ix, iy) = ((rect.0 + 10.0) as i32, (rect.1 + (30.0 - size as f32) / 2.0) as i32);
                if symbolic {
                    let tint = if is_sel { t.on_accent } else { t.text };
                    crate::ui::draw::blit_tinted(self.canvas, &pix, ix, iy, [tint[0], tint[1], tint[2]]);
                } else {
                    crate::ui::draw::blit(self.canvas, &pix, ix, iy, 1.0);
                }
            }
            let tc = if is_sel { t.on_accent } else { t.text };
            self.draw_text(name, rect.0 + 42.0, rect.1 + (30.0 - t.font * 1.25) / 2.0, t.font, FontWeight::Regular, tc);
        }
        // Conteúdo à direita.
        let (old_x, old_w) = (self.x, self.width);
        self.x = x + side_w;
        self.width = old_w - side_w;
        if raw {
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
    /// A próxima barra lateral (`sidebar`, `sidebar_area`) é de vidro.
    pub fn glass_sidebar(&mut self) {
        self.sidebar_glass = true;
    }

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

    fn menu(click: (f32, f32), item: (f32, f32), height: f32) -> MenuState {
        MenuState { owner: 1, pos: (click.0, item.1), click, height, item_top: Some(item.0), opened: None }
    }

    #[test]
    fn menu_abre_embaixo_do_item_quando_cabe() {
        let place = menu_place(&menu((100.0, 50.0), (40.0, 60.0), 200.0), 220.0, (900.0, 520.0));
        assert_eq!(place.side, None);
        assert!(!place.up);
        assert!(place.y >= 60.0);
    }

    #[test]
    fn menu_abre_em_cima_quando_nao_cabe_embaixo() {
        let place = menu_place(&menu((100.0, 450.0), (440.0, 460.0), 200.0), 220.0, (900.0, 520.0));
        assert_eq!(place.side, None);
        assert!(place.up);
        assert!(place.y + 200.0 <= 440.0);
    }

    #[test]
    fn menu_abre_ao_lado_sem_espaco_em_cima_e_embaixo() {
        // Item no meio de uma janela baixa: o menu fica à direita do clique, dentro da janela.
        let place = menu_place(&menu((400.0, 280.0), (270.0, 290.0), 320.0), 220.0, (900.0, 520.0));
        assert_eq!(place.side, Some(true));
        assert!(place.x > 400.0);
        assert!(place.y >= 4.0 && place.y + 320.0 <= 520.0);
        // Perto da borda direita, à esquerda do clique.
        let place = menu_place(&menu((800.0, 280.0), (270.0, 290.0), 320.0), 220.0, (900.0, 520.0));
        assert_eq!(place.side, Some(false));
        assert!(place.x + 220.0 < 800.0);
    }

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
        // O menu abre embaixo do botão (a ponta encosta na borda de baixo dele):
        // "Abrir" é a primeira linha. O item é escolhido ao apertar, e o clique não
        // chega ao botão que está embaixo.
        let (_, menu_y, _, _) = ctx.memory.menu_rect.expect("menu aberto");
        let button_bottom = ctx.memory.menu.map(|m| m.pos.1).unwrap();
        assert!(menu_y >= button_bottom, "menu abaixo do item");
        ctx.input.pointer = Some((40.0, menu_y + 18.0));
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

    #[test]
    #[ignore]
    fn bancada_painel_de_vidro() {
        let mut ctx = Ctx::new(Theme::default());
        let mut canvas = Pixmap::new(1366, 768).unwrap();
        let mut q = String::from("cal");
        let mut medir = |nome: &str, ctx: &mut Ctx, view: &mut dyn FnMut(&mut Ui)| {
            for _ in 0..3 {
                ctx.begin_frame();
                view(&mut Ui::root(&mut canvas, ctx));
                ctx.end_frame();
            }
            let t = std::time::Instant::now();
            for _ in 0..10 {
                ctx.begin_frame();
                view(&mut Ui::root(&mut canvas, ctx));
                ctx.end_frame();
            }
            println!("{nome}: {:.2} ms", t.elapsed().as_secs_f32() * 1e3 / 10.0);
        };
        medir("painel vazio", &mut ctx, &mut |ui| {
            ui.glass_panel(343.0, 154.0, 680.0, 18.0, |_| {});
        });
        medir("painel com campo", &mut ctx, &mut |ui| {
            ui.glass_panel(343.0, 154.0, 680.0, 18.0, |ui| {
                ui.text_input(&mut q, "Buscar");
            });
        });
        medir("painel com 8 linhas", &mut ctx, &mut |ui| {
            ui.glass_panel(343.0, 154.0, 680.0, 18.0, |ui| {
                ui.text_input(&mut q, "Buscar");
                for i in 0..8 {
                    ui.result_row("text-x-generic", &format!("Resultado {i}"), "Aplicativo", i == 0);
                }
            });
        });
        medir("8 linhas sem painel", &mut ctx, &mut |ui| {
            for i in 0..8 {
                ui.result_row("text-x-generic", &format!("Resultado {i}"), "Aplicativo", i == 0);
            }
        });
    }

    #[test]
    fn arranjo_em_milimetros() {
        // Tela grande (530 mm) à esquerda e uma de 17" (340 mm) à direita, topo alinhado.
        let screens = vec![
            ArrangedScreen { name: "A".into(), pos: (0, 0), size: (1920, 1080), primary: true, mm: (530.0, 300.0) },
            ArrangedScreen { name: "B".into(), pos: (1920, 0), size: (1280, 1024), primary: false, mm: (340.0, 270.0) },
        ];
        let layout = physical_layout(&screens);
        assert_eq!(layout[0], (0.0, 0.0));
        // B começa onde A termina (530 mm), mesmo com densidades diferentes.
        assert_eq!(layout[1], (530.0, 0.0));
        // Soltar B do lado esquerdo de A: encosta à esquerda (posição lógica -1280).
        assert_eq!(snap_physical(&screens, &layout, 1, (-300.0, 0.0)), (-1280, 0));
        // Soltar B à direita, 30 mm abaixo do topo: 30 mm de A são ~108 pontos.
        let (x, y) = snap_physical(&screens, &layout, 1, (540.0, 30.0));
        assert_eq!(x, 1920);
        assert!((y - 108).abs() <= 1, "y = {y}");
    }
}
