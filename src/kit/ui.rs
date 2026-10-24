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

/// Aparência de uma área de texto (`Ui::text_area_with`). O padrão é o do editor:
/// números de linha e fonte de largura fixa no tamanho do tema.
pub struct TextAreaLook<'a> {
    /// Margem com os números das linhas.
    pub line_numbers: bool,
    /// Fonte proporcional (texto corrido) no lugar da de largura fixa.
    pub sans: bool,
    /// Tamanho da fonte; `None`, o do tema.
    pub size: Option<f32>,
    /// Texto cinza com o documento vazio.
    pub placeholder: &'a str,
    /// Cores, sublinhados e texto no fim das linhas: chamado a cada quadro, depois das
    /// teclas do quadro e antes do desenho (o resultado nunca atrasa um quadro).
    pub decorate: Option<&'a mut dyn FnMut(&super::textarea::TextDoc) -> super::textarea::Decorations>,
}

impl Default for TextAreaLook<'_> {
    fn default() -> Self {
        Self { line_numbers: true, sans: false, size: None, placeholder: "", decorate: None }
    }
}

/// Resposta da `text_area_with`: a do componente e a posição do texto sob o ponteiro
/// (o menu de contexto do app sabe em que linha foi o clique).
pub struct TextAreaResponse {
    pub response: Response,
    pub pointer_index: Option<usize>,
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
    rows: Vec<MenuRow>,
}

/// Uma linha do menu: texto, tipo (0 comum, 1 perigo, 2 separador, 3 etiquetas),
/// se está sob o ponteiro, símbolo, atalho e marca (✓).
struct MenuRow {
    label: String,
    kind: u8,
    hovered: bool,
    icon: Option<String>,
    shortcut: Option<String>,
    checked: bool,
    /// Etiquetas: as cores marcadas e a bolinha sob o ponteiro.
    tags: Vec<bool>,
    tag_hover: Option<usize>,
}

impl Menu<'_> {
    fn push(&mut self, label: &str, kind: u8, icon: Option<&str>, shortcut: Option<&str>, checked: bool) -> bool {
        let h = if kind == 2 { 9.0 } else { 26.0 };
        let rect = (self.x, self.y, self.w, h);
        self.y += h;
        let hovered = kind != 2 && self.ctx.input.over(rect);
        self.rows.push(MenuRow {
            label: label.to_string(),
            kind,
            hovered,
            icon: icon.map(str::to_string),
            shortcut: shortcut.map(str::to_string),
            checked,
            tags: Vec::new(),
            tag_hover: None,
        });
        if hovered && self.ctx.input.pressed {
            self.close();
            return true;
        }
        false
    }

    /// Escolhido: fecha o menu, e o clique não passa para o que está atrás.
    fn close(&mut self) {
        self.ctx.memory.menu = None;
        self.ctx.memory.menu_rect = None;
        self.ctx.repaint = true;
        self.ctx.input.pressed = false;
    }

    /// Item comum; devolve `true` quando escolhido.
    pub fn item(&mut self, label: &str) -> bool {
        self.push(label, 0, None, None, false)
    }

    /// Item com símbolo à esquerda (nome de ícone do tema, de preferência
    /// `-symbolic`) e, se houver, o atalho à direita, como nos menus do Tahoe.
    pub fn item_icon(&mut self, icon: &str, label: &str, shortcut: Option<&str>) -> bool {
        self.push(label, 0, Some(icon), shortcut, false)
    }

    /// Item que pode estar marcado (✓ à esquerda).
    pub fn item_checked(&mut self, label: &str, checked: bool) -> bool {
        self.push(label, 0, None, None, checked)
    }

    /// Item que apaga coisas (texto em vermelho).
    pub fn danger(&mut self, label: &str) -> bool {
        self.push(label, 1, None, None, false)
    }

    /// Como `danger`, com símbolo à esquerda.
    pub fn danger_icon(&mut self, icon: &str, label: &str) -> bool {
        self.push(label, 1, Some(icon), None, false)
    }

    pub fn separator(&mut self) {
        self.push("", 2, None, None, false);
    }

    /// Linha das etiquetas do Finder: as bolinhas de `TAG_COLORS` (marcadas com um
    /// anel) e o texto embaixo. Devolve a cor clicada.
    pub fn tags(&mut self, label: &str, marked: &[bool]) -> Option<usize> {
        let h = 44.0;
        let rect = (self.x, self.y, self.w, h);
        self.y += h;
        let n = TAG_COLORS.len();
        let step = 24.0;
        let x0 = self.x + 12.0;
        let tag_hover = (0..n).find(|i| self.ctx.input.over((x0 + *i as f32 * step - 2.0, rect.1 + 2.0, 22.0, 22.0)));
        self.rows.push(MenuRow {
            label: label.to_string(),
            kind: 3,
            hovered: false,
            icon: None,
            shortcut: None,
            checked: false,
            tags: (0..n).map(|i| marked.get(i).copied().unwrap_or(false)).collect(),
            tag_hover,
        });
        if let Some(i) = tag_hover {
            if self.ctx.input.pressed {
                self.close();
                return Some(i);
            }
        }
        None
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
#[derive(Default, Clone)]
pub struct Memory {
    /// Componente onde o botão foi apertado (o clique vale se soltar nele).
    pub active: Option<u64>,
    /// Largura pedida para o próximo menu (listas suspensas com nomes longos).
    pub menu_width: Option<f32>,
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
    /// Com o foco dado por `focus_next_input_selecting`: escolhe só até este byte.
    focus_select_end: Option<usize>,
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
    /// Último aperto no vazio que arrasta a janela (duplo clique maximiza).
    drag_area_press: Option<std::time::Instant>,
    /// Dica (`Ui::tooltip`): componente sob o ponteiro e desde quando.
    tooltip_hover: Option<(u64, std::time::Instant)>,
    /// Controles segmentados: opção mostrada e, se a pílula está deslizando, de onde
    /// ela saiu e quando.
    segments: HashMap<u64, (usize, Option<(usize, std::time::Instant)>)>,
}

/// O que aconteceu na barra de abas (`Ui::tab_bar`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabAction {
    None,
    Select(usize),
    Close(usize),
    New,
}

/// Pedido de um app de conteúdo inteiro à janela: começar a mover (com o botão
/// segurado), minimizar, maximizar ou restaurar, fechar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowRequest {
    Move,
    Minimize,
    ToggleMaximize,
    Close,
}

/// Botões da janela no estilo do Tahoe: diâmetro, distância entre os centros e as
/// cores (fechar, minimizar, maximizar).
pub const LIGHT_SIZE: f32 = 14.0;
pub const LIGHT_STEP: f32 = 20.0;
const LIGHT_COLORS: [[u8; 4]; 3] = [[255, 95, 87, 255], [254, 188, 46, 255], [40, 200, 64, 255]];

/// Tudo que os componentes compartilham num quadro.
pub struct Ctx {
    pub text: Text,
    pub theme: Theme,
    /// Tamanho novo pedido pelo app (`Ui::request_size`), e o mínimo.
    pub size_request: Option<((u32, u32), (u32, u32))>,
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
    pub(crate) window: (u32, u32),
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
    /// Arrasto para fora pedido pelo app (`Ui::start_drag`): endereços `file://` e o
    /// ícone que segue o ponteiro.
    pub drag_out: Option<(Vec<String>, Option<Pixmap>)>,
    /// Pedido à janela (`Ui::window_controls`, `Ui::window_drag_area`).
    pub window_request: Option<WindowRequest>,
    /// Retângulo do último componente posto (`Ui::last_rect`), na janela.
    pub last_rect: (f32, f32, f32, f32),
    /// Janela de conteúdo inteiro (`Window::full_size_content`): o app desenha os
    /// botões da janela e as áreas de arrastar.
    pub content_window: bool,
    /// A janela está maximizada (ou presa a um lado da tela).
    pub maximized: bool,
    /// Janela de tamanho fixo (`Window::fixed_size`): minimizar e maximizar desativados.
    pub fixed_size: bool,
    /// Acorda a janela de outra thread (`Ui::waker`).
    pub waker: Option<super::app::Waker>,
    /// Área que recebe cliques neste quadro (`Ui::input_area`; `None`: a superfície toda).
    pub input_region: Option<(f32, f32, f32, f32)>,
    /// Pedido para esconder (`true`) ou mostrar a superfície (`Ui::set_hidden`).
    pub hide_request: Option<bool>,
    /// Tela de bloqueio: a senha foi aceita (`Ui::unlock`).
    pub unlock_request: bool,
    /// Desenhando a cópia da tela de bloqueio de outro monitor, ou outra tela de um app
    /// em todas as telas (`Ui::mirror`).
    pub mirror: bool,
    /// Telas do app em todas as telas (`Window::on_every_output`), a principal primeiro;
    /// vazio sem a opção. E o índice da tela deste passe.
    pub screens: Vec<super::Screen>,
    pub screen: usize,
    /// Faixa (topo e base, na janela) que a área que rola em volta deve mostrar
    /// (`Ui::reveal`): a mais próxima que termina depois do pedido rola até ela.
    reveal: Option<(f32, f32)>,
    /// As luzes da janela já foram desenhadas neste quadro (a barra lateral do Tahoe as
    /// desenha; um segundo pedido no mesmo quadro é ignorado).
    controls_drawn: bool,
}

/// O que o passe da tela principal deixou no `Ctx` e que um passe de outra tela não
/// pode mexer nem apagar (`Ctx::begin_mirror`/`end_mirror`). O `begin_frame` do passe
/// extra limparia o vidro e o `end_frame` soltaria o botão apertado na principal.
pub struct MainPass {
    input: Input,
    memory: Memory,
    glass: Vec<(f32, f32, f32, f32, f32)>,
    glass_capable: bool,
    input_region: Option<(f32, f32, f32, f32)>,
    close_requested: bool,
    unlock_request: bool,
    size_request: Option<((u32, u32), (u32, u32))>,
    title: Option<String>,
    hide_request: Option<bool>,
    clipboard_out: Option<String>,
    paste_requested: bool,
    repaint: bool,
    ticking: bool,
    window: (u32, u32),
    screen: usize,
}

impl Ctx {
    pub fn new(theme: Theme) -> Self {
        Self {
            text: Text::new(),
            theme,
            size_request: None,
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
            drag_out: None,
            window_request: None,
            last_rect: (0.0, 0.0, 0.0, 0.0),
            controls_drawn: false,
            content_window: false,
            maximized: false,
            fixed_size: false,
            waker: None,
            input_region: None,
            hide_request: None,
            unlock_request: false,
            mirror: false,
            screens: Vec::new(),
            screen: 0,
            reveal: None,
        }
    }

    /// Começo do passe de outra tela (`screen`, índice em `screens`): guarda o estado
    /// da principal e entrega uma entrada só com o ponteiro (nas coordenadas da tela).
    pub fn begin_mirror(&mut self, screen: usize, pointer: Option<(f32, f32)>) -> MainPass {
        let saved = MainPass {
            input: std::mem::take(&mut self.input),
            memory: self.memory.clone(),
            glass: std::mem::take(&mut self.glass),
            glass_capable: self.glass_capable,
            input_region: self.input_region.take(),
            close_requested: self.close_requested,
            unlock_request: self.unlock_request,
            size_request: self.size_request.take(),
            title: self.title.take(),
            hide_request: self.hide_request.take(),
            clipboard_out: self.clipboard_out.take(),
            paste_requested: self.paste_requested,
            repaint: self.repaint,
            ticking: self.ticking,
            window: self.window,
            screen: self.screen,
        };
        self.input.pointer = pointer;
        self.mirror = true;
        self.screen = screen;
        saved
    }

    /// Fim do passe de outra tela: tudo volta a ser da principal. Animação pedida no
    /// passe extra continua valendo (`ticking`); o resto que ele pediu é descartado.
    pub fn end_mirror(&mut self, saved: MainPass) {
        let ticking = self.ticking;
        self.input = saved.input;
        self.memory = saved.memory;
        self.glass = saved.glass;
        self.glass_capable = saved.glass_capable;
        self.input_region = saved.input_region;
        self.close_requested = saved.close_requested;
        self.unlock_request = saved.unlock_request;
        self.size_request = saved.size_request;
        self.title = saved.title;
        self.hide_request = saved.hide_request;
        self.clipboard_out = saved.clipboard_out;
        self.paste_requested = saved.paste_requested;
        self.repaint = saved.repaint;
        self.ticking = saved.ticking || ticking;
        self.window = saved.window;
        self.screen = saved.screen;
        self.overlay = None;
        self.mirror = false;
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
        let t = crate::motion::progress(opened, crate::motion::MENU_APPEAR);
        if t < 1.0 {
            self.ticking = true;
        }
        let e = 1.0 - (1.0 - t) * (1.0 - t);
        (e, (-6.0 * (1.0 - e)).round() as i32)
    }

    /// Começo de um quadro: os ids voltam a contar do início.
    pub fn begin_frame(&mut self) {
        self.next_id = 1;
        self.reveal = None;
        self.repaint = false;
        self.hover_claimed = false;
        self.controls_drawn = false;
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
    /// O próximo `text_input` é um campo de busca em cápsula (`Ui::search_field`).
    search_look: bool,
    /// Dentro de um cartão: o recuo dos itens até a borda dele (o destaque das linhas
    /// vai de borda a borda).
    card_inset: f32,
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
            search_look: false,
            card_inset: 0.0,
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

    /// Numa linha (`row`, `toolbar`): quanto ainda cabe até o fim da linha.
    pub fn row_remaining(&self) -> f32 {
        (self.x + self.width - self.row_x).max(0.0)
    }

    /// Pede para desenhar de novo (por exemplo, depois de mudar o estado fora de um clique).
    pub fn request_repaint(&mut self) {
        self.ctx.repaint = true;
    }

    /// Ponteiro nas coordenadas desta área.
    /// Posição do ponteiro nas coordenadas desta área (`None` fora da janela).
    pub fn pointer(&self) -> Option<(f32, f32)> {
        self.ctx.input.pointer.map(|(x, y)| (x - self.offset.0, y - self.offset.1))
    }

    /// O ponteiro está sobre `rect` (desta área), mesmo com o botão segurado em outro
    /// componente: para achar onde soltar algo arrastado.
    pub fn over(&self, rect: (f32, f32, f32, f32)) -> bool {
        self.pointer().is_some_and(|(px, py)| px >= rect.0 && px <= rect.0 + rect.2 && py >= rect.1 && py <= rect.1 + rect.3)
    }

    /// Reserva um lugar para o próximo item: em coluna ocupa a largura toda (ou
    /// `width`); em linha, vai para a direita do anterior.
    fn alloc(&mut self, width: Option<f32>, height: f32) -> (f32, f32, f32, f32) {
        let rect = self.alloc_inner(width, height);
        self.ctx.last_rect = (self.offset.0 + rect.0, self.offset.1 + rect.1, rect.2, rect.3);
        rect
    }

    /// Retângulo (na janela) do último componente posto: para ancorar um popover num
    /// grupo de botões (`toolbar_buttons`), por exemplo.
    pub fn last_rect(&self) -> (f32, f32, f32, f32) {
        self.ctx.last_rect
    }

    fn alloc_inner(&mut self, width: Option<f32>, height: f32) -> (f32, f32, f32, f32) {
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
        // Numa área que rola, só a parte à vista recebe o ponteiro (um item escondido
        // pela rolagem não pode ser clicado por cima do que está fora da área).
        let in_view = self.pointer().is_some_and(|(_, py)| py >= self.visible.0 && py <= self.visible.1);
        let hovered = !under_menu && !blocked && in_view && self.over(rect) && self.ctx.memory.active.is_none_or(|a| a == id);
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

    /// Cor da seleção: o destaque com a janela em foco e cinza sem foco, como no Mac.
    fn selection_fill(&self) -> Rgba {
        let t = &self.ctx.theme;
        if self.ctx.input.focused { t.accent } else { t.shade(if t.dark { 50 } else { 30 }) }
    }

    /// Os três botões da janela (fechar, minimizar, maximizar), com o canto de cima à
    /// esquerda do primeiro em (`x`, `y`), para apps de conteúdo inteiro
    /// (`Window::full_size_content`): no Tahoe ficam dentro da barra lateral ou no
    /// canto da barra de ferramentas. Sem foco, ficam cinza (coloridos com o ponteiro
    /// sobre o grupo); o símbolo aparece só no botão sob o ponteiro. Devolve a
    /// largura ocupada.
    pub fn window_controls(&mut self, x: f32, y: f32) -> f32 {
        let t = self.ctx.theme.clone();
        let width = LIGHT_STEP * 2.0 + LIGHT_SIZE;
        if std::mem::replace(&mut self.ctx.controls_drawn, true) {
            return width;
        }
        let group = (x - 4.0, y - 4.0, width + 8.0, LIGHT_SIZE + 8.0);
        let blocked = self.ctx.memory.modal_rect.is_some() && !self.ctx.in_modal;
        let group_hover = !blocked && self.ctx.input.over(group);
        let focused = self.ctx.input.focused;
        // Regra do Tahoe: janela de tamanho fixo tem minimizar e maximizar cinza e sem ação.
        if self.ctx.fixed_size {
            self.window_close_only(x, y);
            return width;
        }
        for (i, request) in [WindowRequest::Close, WindowRequest::Minimize, WindowRequest::ToggleMaximize].into_iter().enumerate() {
            let id = self.ctx.id();
            let cx = x + i as f32 * LIGHT_STEP;
            let rect = (cx - 3.0, y - 3.0, LIGHT_SIZE + 6.0, LIGHT_SIZE + 6.0);
            let r = self.interact(id, rect);
            if r.clicked {
                self.ctx.window_request = Some(request);
            }
            let fill = if focused || group_hover { LIGHT_COLORS[i] } else { t.shade(if t.dark { 60 } else { 38 }) };
            fill_rounded(self.canvas, cx, y, LIGHT_SIZE, LIGHT_SIZE, LIGHT_SIZE / 2.0, color(fill));
            // Contorno fino (mais escuro que a cor), como uma lente.
            if let Some(path) = crate::ui::draw::rounded_rect(cx + 0.25, y + 0.25, LIGHT_SIZE - 0.5, LIGHT_SIZE - 0.5, LIGHT_SIZE / 2.0) {
                let mut paint = tiny_skia::Paint::default();
                paint.anti_alias = true;
                paint.set_color(color([0, 0, 0, 30]));
                self.canvas.stroke_path(&path, &paint, &tiny_skia::Stroke { width: 0.5, ..Default::default() }, crate::ui::draw::tf(), None);
            }
            // Símbolo só no botão sob o ponteiro (os outros ficam só com a cor).
            if r.hovered {
                let mut pb = tiny_skia::PathBuilder::new();
                let (mx, my) = (cx + LIGHT_SIZE / 2.0, y + LIGHT_SIZE / 2.0);
                let k = 3.4;
                match request {
                    WindowRequest::Close => {
                        pb.move_to(mx - k, my - k);
                        pb.line_to(mx + k, my + k);
                        pb.move_to(mx + k, my - k);
                        pb.line_to(mx - k, my + k);
                    }
                    WindowRequest::Minimize => {
                        pb.move_to(mx - k - 0.6, my);
                        pb.line_to(mx + k + 0.6, my);
                    }
                    _ => {
                        pb.move_to(mx - k - 0.4, my);
                        pb.line_to(mx + k + 0.4, my);
                        pb.move_to(mx, my - k - 0.4);
                        pb.line_to(mx, my + k + 0.4);
                    }
                }
                if let Some(path) = pb.finish() {
                    let mut paint = tiny_skia::Paint::default();
                    paint.anti_alias = true;
                    paint.set_color(color([0, 0, 0, 150]));
                    let stroke = tiny_skia::Stroke { width: 1.4, line_cap: tiny_skia::LineCap::Round, ..Default::default() };
                    self.canvas.stroke_path(&path, &paint, &stroke, crate::ui::draw::tf(), None);
                }
            }
        }
        if group_hover {
            self.ctx.hover_claimed = true;
        }
        width
    }

    /// Os botões da janela de um painel que não muda de tamanho (Sobre este
    /// computador): só o de fechar funciona; minimizar e maximizar ficam cinza.
    pub fn window_close_only(&mut self, x: f32, y: f32) {
        let t = self.ctx.theme.clone();
        let id = self.ctx.id();
        let rect = (x - 3.0, y - 3.0, LIGHT_SIZE + 6.0, LIGHT_SIZE + 6.0);
        let r = self.interact(id, rect);
        if r.clicked {
            self.ctx.window_request = Some(WindowRequest::Close);
        }
        let focused = self.ctx.input.focused;
        let red = if focused || r.hovered { LIGHT_COLORS[0] } else { t.shade(if t.dark { 60 } else { 38 }) };
        fill_rounded(self.canvas, x, y, LIGHT_SIZE, LIGHT_SIZE, LIGHT_SIZE / 2.0, color(red));
        if r.hovered {
            let (mx, my, k) = (x + LIGHT_SIZE / 2.0, y + LIGHT_SIZE / 2.0, 3.4);
            let mut pb = tiny_skia::PathBuilder::new();
            pb.move_to(mx - k, my - k);
            pb.line_to(mx + k, my + k);
            pb.move_to(mx + k, my - k);
            pb.line_to(mx - k, my + k);
            if let Some(path) = pb.finish() {
                let mut paint = tiny_skia::Paint::default();
                paint.anti_alias = true;
                paint.set_color(color([0, 0, 0, 150]));
                let stroke = tiny_skia::Stroke { width: 1.4, line_cap: tiny_skia::LineCap::Round, ..Default::default() };
                self.canvas.stroke_path(&path, &paint, &stroke, crate::ui::draw::tf(), None);
            }
        }
        for i in 1..3 {
            let cx = x + i as f32 * LIGHT_STEP;
            fill_rounded(self.canvas, cx, y, LIGHT_SIZE, LIGHT_SIZE, LIGHT_SIZE / 2.0, color(t.shade(if t.dark { 60 } else { 38 })));
        }
    }

    /// Vazio que arrasta a janela (barra de ferramentas, topo da barra lateral) num app
    /// de conteúdo inteiro: chamar **depois** de desenhar os controles da área, que têm
    /// prioridade. Apertar e arrastar move a janela; duplo clique maximiza ou restaura.
    pub fn window_drag_area(&mut self, rect: (f32, f32, f32, f32)) {
        if !self.ctx.content_window {
            return;
        }
        let r = self.background(rect);
        if !r.clicked {
            return;
        }
        let now = std::time::Instant::now();
        let double = self.ctx.memory.drag_area_press.is_some_and(|at| now.duration_since(at) < std::time::Duration::from_millis(450));
        if double && !self.ctx.fixed_size {
            self.ctx.memory.drag_area_press = None;
            self.ctx.window_request = Some(WindowRequest::ToggleMaximize);
        } else {
            self.ctx.memory.drag_area_press = Some(now);
            self.ctx.window_request = Some(WindowRequest::Move);
        }
    }

    /// A janela é de conteúdo inteiro (`Window::full_size_content`).
    pub fn content_window(&self) -> bool {
        self.ctx.content_window
    }

    /// A janela está maximizada.
    pub fn maximized(&self) -> bool {
        self.ctx.maximized
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
        // Tahoe: cantos grandes nos diálogos.
        let rect = self.glass_panel(x, y, width, 20.0, add);
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
        let n = buttons.len().max(1) as f32;
        let width = (n * 128.0 + 40.0).max(420.0).min(self.ctx.window.0 as f32 - 32.0);
        // Os botões têm a largura do rótulo; janela estreita (a Calculadora básica):
        // se não cabem lado a lado no painel, ficam um embaixo do outro (o principal em
        // cima).
        let font = self.ctx.theme.font;
        let widths: Vec<f32> = buttons.iter().map(|b| self.measure(b, font, FontWeight::Semibold) + 32.0).collect();
        let row_w = widths.iter().sum::<f32>() + (n - 1.0) * self.ctx.theme.spacing;
        let stacked = row_w > width - 20.0;
        self.modal(width, |ui| {
            ui.strong(title);
            if !message.is_empty() {
                ui.note(message);
            }
            if stacked {
                for (i, label) in buttons.iter().enumerate().rev() {
                    let r = if i + 1 == buttons.len() { ui.primary_button(label) } else { ui.button(label) };
                    if r.clicked() {
                        chosen = Some(i);
                    }
                }
                return;
            }
            chosen = ui.dialog_buttons(buttons);
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

    /// Linha de botões de diálogo alinhada à direita, cada um do tamanho do rótulo: o
    /// último é o principal (azul), como "Cancelar" e "OK". Devolve o botão clicado.
    pub fn dialog_buttons(&mut self, buttons: &[&str]) -> Option<usize> {
        let font = self.ctx.theme.font;
        let n = buttons.len().max(1) as f32;
        let widths: Vec<f32> = buttons.iter().map(|b| self.measure(b, font, FontWeight::Semibold) + 32.0).collect();
        let row_w = widths.iter().sum::<f32>() + (n - 1.0) * self.ctx.theme.spacing;
        let mut chosen = None;
        self.row(|ui| {
            ui.push_right(row_w);
            for (i, label) in buttons.iter().enumerate() {
                let primary = i + 1 == buttons.len();
                ui.sized(widths[i], |ui| {
                    let r = if primary { ui.primary_button(label) } else { ui.button(label) };
                    if r.clicked() {
                        chosen = Some(i);
                    }
                });
            }
        });
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

    /// Pede que a área que rola em volta mostre `rect` (um item escolhido de fora,
    /// como o arquivo pedido ao abrir o Arquivos). Chamado uma vez; a área rola no
    /// quadro seguinte.
    pub fn reveal(&mut self, rect: (f32, f32, f32, f32)) {
        // Na janela (os retângulos dos componentes são da área onde estão).
        let top = self.offset.1 + rect.1;
        self.ctx.reveal = Some((top, top + rect.3));
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

    /// Pede outro tamanho para a janela (em pontos), com o tamanho mínimo novo: por
    /// exemplo, a Calculadora trocando de modo. Maximizada, a janela não muda.
    pub fn request_size(&mut self, size: (u32, u32), min: (u32, u32)) {
        self.ctx.size_request = Some((size, min));
        self.ctx.repaint = true;
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
        self.ctx.memory.focus_select_end = None;
    }

    /// Como `focus_next_input`, escolhendo só o começo do texto até o byte `end` (o
    /// nome sem a extensão, ao renomear um arquivo, como no Finder).
    pub fn focus_next_input_selecting(&mut self, end: usize) {
        self.ctx.memory.focus_next = true;
        self.ctx.memory.focus_select_end = Some(end);
    }

    /// Tira o foco do campo de texto (um diálogo que fechou devolve as teclas ao app).
    pub fn blur(&mut self) {
        self.ctx.memory.focus = None;
    }

    /// Algum campo de texto está com o foco (as teclas são dele).
    pub fn typing(&self) -> bool {
        self.ctx.memory.focus.is_some()
    }

    /// Numa linha (`row`, `toolbar`), pula para que os próximos `width` pixels
    /// terminem na borda direita.
    /// Retângulo desta área (de um `Response`) nas coordenadas da janela: para ancorar
    /// um `popover` desenhado no fim do `view`.
    pub fn to_window(&self, rect: (f32, f32, f32, f32)) -> (f32, f32, f32, f32) {
        (rect.0 + self.offset.0, rect.1 + self.offset.1, rect.2, rect.3)
    }

    /// O próximo item encosta neste, sem o espaçamento do tema (barras do rodapé).
    fn tight_after(&mut self) {
        if self.flow == Flow::Column {
            self.cursor_y -= self.ctx.theme.spacing;
        }
    }

    /// Largura que ainda sobra na linha atual (numa `row`/`toolbar`), a partir do
    /// próximo item.
    pub fn remaining_width(&self) -> f32 {
        if self.flow == Flow::Column {
            self.width
        } else {
            (self.x + self.width - self.row_x).max(0.0)
        }
    }

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
        // Dentro de `sized`, a largura pedida também limita: o texto encolhe com "…".
        let room = self.next_width.map_or(room, |w| room.min(w));
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

    /// Fica com a rolagem deste quadro (as áreas que rolam não a recebem): Ctrl+roda
    /// para ampliar ícones, por exemplo.
    pub fn take_scroll(&mut self) -> f32 {
        std::mem::take(&mut self.ctx.input.scroll)
    }

    /// Página que rola com altura escolhida (o resto fica para uma barra embaixo, como a
    /// barra de status do Arquivos).
    pub fn page_sized(&mut self, height: f32, add: impl FnOnce(&mut Ui)) {
        let pad = self.ctx.theme.padding;
        self.scroll_impl(height.max(0.0), pad, None, add);
    }

    /// Área que rola com altura e margem escolhidas (colunas do Arquivos, prévia).
    pub fn scroll_area_padded(&mut self, height: f32, pad: f32, add: impl FnOnce(&mut Ui)) {
        self.scroll_impl(height.max(0.0), pad, None, add);
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

    /// Há um menu de contexto aberto (e o retângulo dele no último quadro, na janela,
    /// se já foi desenhado)? Para um painel pequeno aumentar a área que recebe cliques
    /// enquanto o menu aparece (`input_area`).
    pub fn menu_open(&self) -> Option<Option<(f32, f32, f32, f32)>> {
        self.ctx.memory.menu.map(|_| self.ctx.memory.menu_rect)
    }

    /// Fecha o menu de contexto aberto (o ponteiro saiu de uma camada pequena, onde o
    /// clique fora não chega ao app).
    pub fn close_menu(&mut self) {
        if self.ctx.memory.menu.take().is_some() {
            self.ctx.memory.menu_rect = None;
            self.ctx.repaint = true;
        }
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
        // Largura padrão, ou a pedida pela lista suspensa (cabe o nome mais longo).
        let w = self.ctx.memory.menu_width.take().unwrap_or(220.0).clamp(220.0, (ww - 16.0).max(220.0));
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
        // Tahoe: vidro de cantos 12, sem a setinha; destaque em pílula de 8.
        let radius = 12.0;
        for (grow, alpha) in [(6.0, 12u8), (3.0, 20), (1.0, 32)] {
            fill_rounded(overlay, x - grow, y0 - grow + 4.0, w + 2.0 * grow, h + 2.0 * grow, radius + grow, color([0, 0, 0, alpha]));
        }
        fill_rounded(overlay, x, y0, w, h, radius, color(t.menu));
        outline(overlay, (x, y0, w, h), radius, if t.dark { [255, 255, 255, 30] } else { [0, 0, 0, 16] });
        let _ = (side, up);
        // Com algum símbolo ou marca no menu, todos os textos alinham depois deles.
        let indent = rows.iter().any(|r| r.icon.is_some() || r.checked);
        let text_x = if indent { x + 36.0 } else { x + 16.0 };
        let mut y = y0 + 5.0;
        let mut icons = Vec::new();
        for row in &rows {
            match row.kind {
                2 => {
                    fill_rounded(overlay, x + 12.0, y + 4.0, w - 24.0, 1.0, 0.0, color(t.shade(30)));
                    y += 9.0;
                }
                3 => {
                    for (i, on) in row.tags.iter().enumerate() {
                        let cx = x + 12.0 + i as f32 * 24.0;
                        let r = if row.tag_hover == Some(i) { 10.0 } else { 8.0 };
                        let c = TAG_COLORS[i];
                        fill_rounded(overlay, cx + 9.0 - r, y + 13.0 - r, 2.0 * r, 2.0 * r, r, color(c));
                        if *on {
                            outline(overlay, (cx - 2.0, y + 2.0, 22.0, 22.0), 11.0, t.text);
                        }
                    }
                    self.ctx.text.draw(overlay, &row.label, x + 14.0, y + 26.0, t.font_small, FontWeight::Regular, t.text_secondary);
                    y += 44.0;
                }
                kind => {
                    if row.hovered {
                        fill_rounded(overlay, x + 5.0, y, w - 10.0, 26.0, super::theme::RADIUS_ITEM, color(t.accent));
                    }
                    let c = match (kind, row.hovered) {
                        (_, true) => t.on_accent,
                        (1, false) => t.danger,
                        _ => t.text,
                    };
                    if row.checked {
                        self.ctx.text.draw(overlay, "✓", x + 15.0, y + (26.0 - t.font * 1.25) / 2.0, t.font, FontWeight::Semibold, c);
                    }
                    if let Some(icon) = &row.icon {
                        icons.push((icon.clone(), x + 13.0, y + 5.0, c));
                    }
                    self.ctx.text.draw(overlay, &row.label, text_x, y + (26.0 - t.font * 1.25) / 2.0, t.font, FontWeight::Regular, c);
                    if let Some(sc) = &row.shortcut {
                        let sc_c = if row.hovered { t.on_accent } else { t.text_secondary };
                        let sw = self.ctx.text.measure(sc, t.font, FontWeight::Regular);
                        self.ctx.text.draw(overlay, sc, x + w - 14.0 - sw, y + (26.0 - t.font * 1.25) / 2.0, t.font, FontWeight::Regular, sc_c);
                    }
                    y += 26.0;
                }
            }
        }
        for (icon, ix, iy, c) in icons {
            if let Some(pix) = self.ctx.icon(&[icon.as_str()], 16, false) {
                if let Some(overlay) = self.ctx.overlay.as_mut() {
                    crate::ui::draw::blit_tinted(overlay, &pix, ix as i32, iy as i32, [c[0], c[1], c[2]]);
                }
            }
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
    /// Como `fit`, na fonte proporcional (`sans`) ou na de largura fixa.
    fn fit_face(&mut self, s: &str, size: f32, sans: bool, max: f32) -> String {
        if sans {
            return self.fit(s, size, FontWeight::Regular, max);
        }
        if self.ctx.text.measure_mono(s, size) <= max {
            return s.to_string();
        }
        let mut chars: Vec<char> = s.chars().collect();
        while !chars.is_empty() {
            chars.pop();
            let c = format!("{}…", chars.iter().collect::<String>().trim_end());
            if self.ctx.text.measure_mono(&c, size) <= max {
                return c;
            }
        }
        "…".into()
    }

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

    /// Título de grupo que recolhe e expande (setinha à esquerda, como as pastas na
    /// lista do Arquivos), com um detalhe à direita (por exemplo "12 atalhos"). O
    /// clique troca `open`; a resposta traz `changed`.
    pub fn disclosure(&mut self, s: &str, detail: &str, open: &mut bool) -> Response {
        let t = self.ctx.theme.clone();
        let id = self.ctx.id();
        let rect = self.alloc(None, t.font_small * 1.4 + 10.0);
        let mut r = self.interact(id, rect);
        if r.clicked {
            *open = !*open;
            r.changed = true;
            self.ctx.repaint = true;
        }
        if !self.visible(rect) {
            return r;
        }
        if r.hovered {
            fill_rounded(self.canvas, rect.0 - 4.0, rect.1, rect.2 + 8.0, rect.3, t.control_radius, color(t.control));
        }
        // Setinha: para a direita fechado, para baixo aberto.
        let (cx, cy) = (rect.0 + 10.0, rect.1 + rect.3 / 2.0);
        let mut pb = PathBuilder::new();
        if *open {
            pb.move_to(cx - 4.0, cy - 2.0);
            pb.line_to(cx, cy + 2.0);
            pb.line_to(cx + 4.0, cy - 2.0);
        } else {
            pb.move_to(cx - 2.0, cy - 4.0);
            pb.line_to(cx + 2.0, cy);
            pb.line_to(cx - 2.0, cy + 4.0);
        }
        if let Some(path) = pb.finish() {
            let mut paint = Paint::default();
            paint.anti_alias = true;
            paint.set_color(color(t.text_secondary));
            let stroke = Stroke { width: 1.6, line_cap: tiny_skia::LineCap::Round, ..Default::default() };
            self.canvas.stroke_path(&path, &paint, &stroke, crate::ui::draw::tf(), None);
        }
        let ty = rect.1 + (rect.3 - t.font_small * 1.25) / 2.0;
        self.draw_text(s, rect.0 + 22.0, ty, t.font_small, FontWeight::Semibold, t.text_secondary);
        if !detail.is_empty() {
            let dw = self.measure(detail, t.font_small, FontWeight::Regular);
            self.draw_text(detail, rect.0 + rect.2 - dw - 4.0, ty, t.font_small, FontWeight::Regular, t.text_secondary);
        }
        r
    }

    /// Texto comum (quebrado em linhas se não couber).
    pub fn label(&mut self, s: &str) {
        let t = self.ctx.theme.clone();
        self.paragraph(s, t.font, t.text);
    }

    /// Título grande e centralizado (o nome do computador no "Sobre").
    pub fn centered_title(&mut self, s: &str) {
        let t = self.ctx.theme.clone();
        let size = 22.0;
        let shown = self.fit(s, size, FontWeight::Bold, self.width);
        let rect = self.alloc(None, size * 1.35);
        let w = self.measure(&shown, size, FontWeight::Bold);
        self.draw_text(&shown, rect.0 + (rect.2 - w) / 2.0, rect.1, size, FontWeight::Bold, t.text);
    }

    /// Texto pequeno e cinza, centralizado, quebrando a linha quando não cabe.
    pub fn centered_note(&mut self, s: &str) {
        let t = self.ctx.theme.clone();
        let size = t.font_small;
        let mut lines: Vec<String> = Vec::new();
        let mut current = String::new();
        for word in s.split_whitespace() {
            let candidate = if current.is_empty() { word.to_string() } else { format!("{current} {word}") };
            if self.measure(&candidate, size, FontWeight::Regular) > self.width && !current.is_empty() {
                lines.push(std::mem::take(&mut current));
                current = word.to_string();
            } else {
                current = candidate;
            }
        }
        lines.push(current);
        let line_h = size * 1.4;
        let rect = self.alloc(None, line_h * lines.len() as f32);
        for (i, line) in lines.iter().enumerate() {
            let w = self.measure(line, size, FontWeight::Regular);
            self.draw_text(line, rect.0 + (rect.2 - w) / 2.0, rect.1 + i as f32 * line_h, size, FontWeight::Regular, t.text_secondary);
        }
    }

    /// Espaço na horizontal dentro de uma linha (`row`).
    pub fn space_x(&mut self, width: f32) {
        if self.flow != Flow::Column {
            self.row_x += width.max(0.0);
        }
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
        self.button_styled(s, if accent { ButtonStyle::Prominent } else { ButtonStyle::Bordered })
    }

    /// Botão em cápsula do Tahoe no estilo pedido (`ButtonStyle`).
    pub fn button_styled(&mut self, s: &str, style: ButtonStyle) -> Response {
        let t = self.ctx.theme.clone();
        let id = self.ctx.id();
        let w = self.measure(s, t.font, FontWeight::Semibold) + 32.0;
        let rect = self.alloc(Some(w), t.row);
        let r = self.interact(id, rect);
        let pressed = self.ctx.memory.active == Some(id) && r.hovered;
        let darker = |c: Rgba, d: u8| [c[0].saturating_sub(d), c[1].saturating_sub(d), c[2].saturating_sub(d), c[3]];
        let radius = rect.3 / 2.0;
        let (fill, tc) = match style {
            ButtonStyle::Prominent => (Some(if pressed { darker(t.accent, 30) } else { t.accent }), t.on_accent),
            ButtonStyle::Destructive => (Some(if pressed { darker(t.danger, 30) } else { t.danger }), [255, 255, 255, 255]),
            ButtonStyle::Bordered => (Some(if pressed { t.shade(46) } else if r.hovered { t.hover } else { t.control }), t.text),
            ButtonStyle::Plain => (if pressed { Some(t.shade(30)) } else if r.hovered { Some(t.control) } else { None }, t.accent),
            ButtonStyle::Glass => {
                capsule(self.canvas, rect, &t);
                (if pressed { Some(t.shade(30)) } else if r.hovered { Some(t.shade(12)) } else { None }, t.text)
            }
        };
        if let Some(fill) = fill {
            fill_rounded(self.canvas, rect.0, rect.1, rect.2, rect.3, radius, color(fill));
        }
        // Brilho de borda em cima dos botões cheios (vidro líquido).
        if matches!(style, ButtonStyle::Prominent | ButtonStyle::Destructive) {
            if let Some(path) = rounded_rect(rect.0 + 0.5, rect.1 + 0.5, rect.2 - 1.0, rect.3 - 1.0, radius - 0.5) {
                let mut paint = Paint { anti_alias: true, ..Default::default() };
                paint.set_color(color([255, 255, 255, 50]));
                self.canvas.stroke_path(&path, &paint, &Stroke { width: 1.0, ..Default::default() }, crate::ui::draw::tf(), None);
            }
        }
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
        // Botão branco em cápsula que estica enquanto é apertado (Tahoe).
        let pressed = self.ctx.memory.active == Some(id) && self.ctx.input.down;
        let kw = if pressed { 24.0 } else { 18.0 };
        let kx = if *value { sx + 40.0 - 2.0 - kw } else { sx + 2.0 };
        knob(self.canvas, (kx, sy + 2.0, kw, 18.0));
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

    /// Começa um arrasto entre apps (só com o botão segurado): os arquivos vão como
    /// `text/uri-list` para quem receber (outro app, um site no navegador ou esta mesma
    /// janela, que recebe em `input().dropped`). O `icon` segue o ponteiro, centrado.
    /// Enquanto o arrasto dura, o ponteiro é do compositor: o botão é dado como solto.
    pub fn start_drag(&mut self, paths: &[std::path::PathBuf], icon: Option<Pixmap>) {
        let uris = paths.iter().map(|p| crate::thumbnails::file_uri(p)).collect();
        self.ctx.drag_out = Some((uris, icon));
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

    /// Este `view` desenha a cópia da tela de bloqueio de outro monitor, ou outra tela
    /// de um app em todas as telas (`Window::on_every_output`): sem teclas e sem
    /// consumir nada do estado (a lógica do app roda só no passe com `!ui.mirror()`).
    pub fn mirror(&self) -> bool {
        self.ctx.mirror
    }

    /// A tela deste passe, em pontos. Sem `Window::on_every_output`: origem (0, 0), o
    /// tamanho da janela e a escala atual, marcada como principal.
    pub fn screen(&self) -> super::Screen {
        match self.ctx.screens.get(self.ctx.screen) {
            Some(screen) => screen.clone(),
            None => super::Screen {
                name: String::new(),
                origin: (0.0, 0.0),
                size: (self.ctx.window.0 as f32, self.ctx.window.1 as f32),
                scale: crate::ui::draw::scale(),
                primary: true,
            },
        }
    }

    /// Todas as telas do app, a da superfície principal primeiro (a tela principal de
    /// quando o app abriu). Sem `Window::on_every_output`, só a de [`Ui::screen`].
    pub fn screens(&self) -> Vec<super::Screen> {
        if self.ctx.screens.is_empty() { vec![self.screen()] } else { self.ctx.screens.clone() }
    }

    /// Ponteiro em coordenadas globais (do desktop), em qualquer tela do app; `None`
    /// fora das superfícies dele. Sem `Window::on_every_output`, igual ao ponteiro na
    /// janela.
    pub fn pointer_global(&self) -> Option<(f32, f32)> {
        let origin = self.ctx.screens.get(self.ctx.screen).map_or((0.0, 0.0), |s| s.origin);
        self.ctx.input.pointer.map(|p| (p.0 + origin.0, p.1 + origin.1))
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
        let widest = options.iter().map(|o| self.measure(o, t.font, FontWeight::Regular)).fold(0.0, f32::max);
        self.ctx.memory.menu_width = Some(widest + 48.0);
        self.menu_impl(&r, r.clicked, |m| {
            for (i, option) in options.iter().enumerate() {
                if m.item(option) {
                    chosen = Some(i);
                }
            }
        });
        // Menu fechado: a largura pedida não vale para o menu de outro componente.
        self.ctx.memory.menu_width = None;
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
        fill_rounded(self.canvas, tx, cy - 2.0, tw, 4.0, 2.0, color(t.shade(30)));
        fill_rounded(self.canvas, tx, cy - 2.0, tw * k, 4.0, 2.0, color(t.accent));
        // Botão de vidro em cápsula, que alonga enquanto é arrastado (Tahoe).
        let dragging = self.ctx.memory.active == Some(id) && self.ctx.input.down;
        let kw = if dragging { 30.0 } else { 22.0 };
        knob(self.canvas, (tx + tw * k - kw / 2.0, cy - 8.0, kw, 16.0));
        let shown = format!("{}", value.round() as i64);
        self.draw_text(&shown, tx + tw + 16.0, rect.1 + (rect.3 - t.font_small * 1.25) / 2.0, t.font_small, FontWeight::Regular, t.text_secondary);
        r
    }

    /// Controle segmentado do Tahoe: cápsula com as opções lado a lado e uma pílula
    /// que desliza até a escolhida. `width` `None` ocupa a linha inteira.
    pub fn segmented(&mut self, options: &[&str], selected: &mut usize, width: Option<f32>) -> Response {
        let t = self.ctx.theme.clone();
        let id = self.ctx.id();
        let n = options.len().max(1);
        let natural: f32 = options.iter().map(|o| self.measure(o, t.font, FontWeight::Regular) + 28.0).sum::<f32>() + 4.0;
        let rect = self.alloc(Some(width.unwrap_or(natural)), t.row);
        let mut r = Response { rect, id, ..Default::default() };
        let seg = (rect.2 - 4.0) / n as f32;
        fill_rounded(self.canvas, rect.0, rect.1, rect.2, rect.3, rect.3 / 2.0, color(t.control));
        for i in 0..n {
            let sid = self.ctx.id();
            let sr = self.interact(sid, (rect.0 + 2.0 + i as f32 * seg, rect.1, seg, rect.3));
            r.hovered |= sr.hovered;
            if sr.clicked && *selected != i {
                *selected = i;
                r.clicked = true;
                r.changed = true;
            }
        }
        // De onde a pílula está vindo (a escolha anterior) e quanto já andou.
        let now = std::time::Instant::now();
        let slide = crate::settings_file::effect("transitions");
        let entry = self.ctx.memory.segments.entry(id).or_insert((*selected, None));
        if entry.0 != *selected {
            entry.1 = slide.then_some((entry.0, now));
            entry.0 = *selected;
        }
        let pos = match entry.1 {
            Some((from, at)) => {
                let p = crate::motion::ease_out(crate::motion::progress(at, crate::motion::CONTROL_SLIDE));
                if p >= 1.0 {
                    entry.1 = None;
                } else {
                    self.ctx.ticking = true;
                }
                from as f32 + (*selected as f32 - from as f32) * p
            }
            None => *selected as f32,
        };
        let pill = (rect.0 + 2.0 + pos * seg, rect.1 + 2.0, seg, rect.3 - 4.0);
        knob(self.canvas, pill);
        for (i, o) in options.iter().enumerate() {
            let tw = self.measure(o, t.font, FontWeight::Regular);
            let x = rect.0 + 2.0 + i as f32 * seg + (seg - tw) / 2.0;
            let tc = if i == *selected { [24, 24, 28, 255] } else { t.text };
            self.draw_text(o, x, rect.1 + (rect.3 - t.font * 1.25) / 2.0, t.font, FontWeight::Regular, tc);
        }
        r
    }

    /// Caixa de seleção (quadrado arredondado com ✓) com o texto ao lado.
    pub fn checkbox(&mut self, label: &str, value: &mut bool) -> Response {
        let t = self.ctx.theme.clone();
        let id = self.ctx.id();
        let w = self.measure(label, t.font, FontWeight::Regular) + 28.0;
        let rect = self.alloc(Some(w), 24.0);
        let mut r = self.interact(id, rect);
        if r.clicked {
            *value = !*value;
            r.changed = true;
        }
        let (bx, by) = (rect.0, rect.1 + 4.0);
        if *value {
            fill_rounded(self.canvas, bx, by, 16.0, 16.0, 5.0, color(t.accent));
            let mut pb = PathBuilder::new();
            pb.move_to(bx + 4.0, by + 8.5);
            pb.line_to(bx + 7.0, by + 11.5);
            pb.line_to(bx + 12.0, by + 4.5);
            if let Some(path) = pb.finish() {
                let mut paint = Paint { anti_alias: true, ..Default::default() };
                paint.set_color(color(t.on_accent));
                let stroke = Stroke { width: 2.0, line_cap: tiny_skia::LineCap::Round, line_join: tiny_skia::LineJoin::Round, ..Default::default() };
                self.canvas.stroke_path(&path, &paint, &stroke, crate::ui::draw::tf(), None);
            }
        } else {
            fill_rounded(self.canvas, bx, by, 16.0, 16.0, 5.0, color(if t.dark { t.shade(30) } else { [255, 255, 255, 255] }));
            outline(self.canvas, (bx, by, 16.0, 16.0), 5.0, t.shade(if r.hovered { 90 } else { 60 }));
        }
        self.draw_text(label, rect.0 + 24.0, rect.1 + (rect.3 - t.font * 1.25) / 2.0, t.font, FontWeight::Regular, t.text);
        r
    }

    /// Botão de opção (rádio): círculo com o ponto quando `selected`. Devolve o clique;
    /// quem chama troca a escolha.
    pub fn radio(&mut self, label: &str, selected: bool) -> Response {
        let t = self.ctx.theme.clone();
        let id = self.ctx.id();
        let w = self.measure(label, t.font, FontWeight::Regular) + 28.0;
        let rect = self.alloc(Some(w), 24.0);
        let r = self.interact(id, rect);
        let (bx, by) = (rect.0, rect.1 + 4.0);
        if selected {
            fill_rounded(self.canvas, bx, by, 16.0, 16.0, 8.0, color(t.accent));
            fill_rounded(self.canvas, bx + 5.0, by + 5.0, 6.0, 6.0, 3.0, color(t.on_accent));
        } else {
            fill_rounded(self.canvas, bx, by, 16.0, 16.0, 8.0, color(if t.dark { t.shade(30) } else { [255, 255, 255, 255] }));
            outline(self.canvas, (bx, by, 16.0, 16.0), 8.0, t.shade(if r.hovered { 90 } else { 60 }));
        }
        self.draw_text(label, rect.0 + 24.0, rect.1 + (rect.3 - t.font * 1.25) / 2.0, t.font, FontWeight::Regular, t.text);
        r
    }

    /// Passo (▲▼ em cápsula) com o valor ao lado, entre `min` e `max`.
    pub fn stepper(&mut self, value: &mut i64, min: i64, max: i64) -> Response {
        let t = self.ctx.theme.clone();
        let id = self.ctx.id();
        let shown = value.to_string();
        let tw = self.measure(&shown, t.font, FontWeight::Regular).max(24.0);
        let rect = self.alloc(Some(tw + 16.0 + 22.0), t.row);
        let mut r = Response { rect, id, ..Default::default() };
        self.draw_text(&shown, rect.0 + 4.0, rect.1 + (rect.3 - t.font * 1.25) / 2.0, t.font, FontWeight::Regular, t.text);
        let caps = (rect.0 + rect.2 - 20.0, rect.1 + 2.0, 20.0, rect.3 - 4.0);
        fill_rounded(self.canvas, caps.0, caps.1, caps.2, caps.3, 10.0, color(t.control));
        for (i, step) in [1i64, -1].into_iter().enumerate() {
            let sid = self.ctx.id();
            let half = (caps.0, caps.1 + i as f32 * caps.3 / 2.0, caps.2, caps.3 / 2.0);
            let sr = self.interact(sid, half);
            if sr.hovered {
                fill_rounded(self.canvas, half.0 + 1.0, half.1 + 1.0, half.2 - 2.0, half.3 - 2.0, 8.0, color(t.hover));
            }
            if sr.clicked {
                let new = (*value + step).clamp(min, max);
                if new != *value {
                    *value = new;
                    r.changed = true;
                }
            }
            let (cx, cy) = (half.0 + half.2 / 2.0, half.1 + half.3 / 2.0);
            let d = if step > 0 { -1.5 } else { 1.5 };
            let mut pb = PathBuilder::new();
            pb.move_to(cx - 3.5, cy - d);
            pb.line_to(cx, cy + d);
            pb.line_to(cx + 3.5, cy - d);
            if let Some(path) = pb.finish() {
                let mut paint = Paint { anti_alias: true, ..Default::default() };
                paint.set_color(color(t.text));
                let stroke = Stroke { width: 1.5, line_cap: tiny_skia::LineCap::Round, line_join: tiny_skia::LineJoin::Round, ..Default::default() };
                self.canvas.stroke_path(&path, &paint, &stroke, crate::ui::draw::tf(), None);
            }
        }
        r
    }

    /// Barra de progresso em cápsula: `Some(0..1)` mostra o andamento; `None`, uma faixa
    /// que anda (sem saber quanto falta).
    pub fn progress_bar(&mut self, value: Option<f32>) {
        let t = self.ctx.theme.clone();
        let rect = self.alloc(None, 14.0);
        let (x, y, w) = (rect.0, rect.1 + 4.0, rect.2);
        fill_rounded(self.canvas, x, y, w, 6.0, 3.0, color(t.shade(26)));
        match value {
            Some(v) => fill_rounded(self.canvas, x, y, w * v.clamp(0.0, 1.0), 6.0, 3.0, color(t.accent)),
            None => {
                let phase = (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0) % 1400) as f32 / 1400.0;
                let bw = w * 0.3;
                let bx = x - bw + (w + bw) * phase;
                let (a, b) = (bx.max(x), (bx + bw).min(x + w));
                if b > a {
                    fill_rounded(self.canvas, a, y, b - a, 6.0, 3.0, color(t.accent));
                }
                self.ctx.ticking = true;
            }
        }
    }

    /// Selo vermelho com um número ou texto curto (abas, barra lateral, dock).
    pub fn badge(&mut self, text: &str) {
        let tw = self.measure(text, 11.0, FontWeight::Semibold);
        let w = (tw + 12.0).max(18.0);
        let rect = self.alloc(Some(w), 18.0);
        fill_rounded(self.canvas, rect.0, rect.1, rect.2, rect.3, 9.0, color([255, 59, 48, 255]));
        self.draw_text(text, rect.0 + (w - tw) / 2.0, rect.1 + 2.0, 11.0, FontWeight::Semibold, [255, 255, 255, 255]);
    }

    /// Bolinha de etiqueta clicável (Obter informações, menus): a cor cheia e, se
    /// marcada, um anel em volta.
    pub fn tag_toggle(&mut self, c: Rgba, on: bool) -> Response {
        let id = self.ctx.id();
        let rect = self.alloc(Some(22.0), 22.0);
        let r = self.interact(id, rect);
        let grow = if r.hovered { 1.0 } else { 0.0 };
        fill_rounded(self.canvas, rect.0 + 3.0 - grow, rect.1 + 3.0 - grow, 16.0 + 2.0 * grow, 16.0 + 2.0 * grow, 8.0 + grow, color(c));
        if on {
            let t = self.ctx.theme.clone();
            outline(self.canvas, rect, 11.0, t.text);
        }
        r
    }

    /// Linha "chave: valor" (Obter informações, Sobre este computador): a chave em
    /// cinza, alinhada à direita numa coluna de `key_w`, e o valor ao lado, quebrando
    /// a linha quando não cabe. Devolve a resposta da área do valor (clique direito).
    pub fn key_value(&mut self, key: &str, value: &str, key_w: f32) -> Response {
        let t = self.ctx.theme.clone();
        let size = t.font_small;
        let line_h = size * 1.4;
        let value_w = (self.width - key_w - 8.0).max(40.0);
        let mut lines: Vec<String> = Vec::new();
        for para in value.split('\n') {
            let mut current = String::new();
            for word in para.split_whitespace() {
                let candidate = if current.is_empty() { word.to_string() } else { format!("{current} {word}") };
                if self.measure(&candidate, size, FontWeight::Regular) > value_w && !current.is_empty() {
                    lines.push(std::mem::take(&mut current));
                    current = word.to_string();
                } else {
                    current = candidate;
                }
            }
            lines.push(current);
        }
        let h = line_h * lines.len().max(1) as f32;
        let rect = self.alloc(None, h);
        let id = self.ctx.id();
        let r = self.interact(id, (rect.0 + key_w + 8.0, rect.1, value_w, h));
        if self.visible(rect) {
            let kw = self.measure(key, size, FontWeight::Semibold);
            self.draw_text(key, rect.0 + key_w - kw, rect.1, size, FontWeight::Semibold, t.text_secondary);
            for (i, line) in lines.iter().enumerate() {
                self.draw_text(line, rect.0 + key_w + 8.0, rect.1 + i as f32 * line_h, size, FontWeight::Regular, t.text);
            }
        }
        r
    }

    /// Bolinhas de etiqueta (cores do Finder), uma sobre a outra como no Tahoe.
    pub fn tag_dots(&mut self, colors: &[Rgba]) {
        let n = colors.len();
        if n == 0 {
            return;
        }
        let rect = self.alloc(Some(12.0 + (n as f32 - 1.0) * 8.0), 12.0);
        for (i, c) in colors.iter().enumerate() {
            let x = rect.0 + i as f32 * 8.0;
            fill_rounded(self.canvas, x - 1.0, rect.1 - 1.0, 14.0, 14.0, 7.0, color(self.ctx.theme.background));
            fill_rounded(self.canvas, x, rect.1, 12.0, 12.0, 6.0, color(*c));
        }
    }

    /// Barra de abas do Tahoe: abas de largura igual numa faixa em cápsula, a escolhida
    /// em destaque, × ao passar o ponteiro e + no fim para abrir outra.
    pub fn tab_bar(&mut self, titles: &[String], selected: usize) -> TabAction {
        let t = self.ctx.theme.clone();
        let rect = self.alloc(None, 32.0);
        let mut action = TabAction::None;
        let plus = 28.0;
        let track = (rect.0, rect.1 + 1.0, rect.2 - plus - 8.0, 30.0);
        fill_rounded(self.canvas, track.0, track.1, track.2, track.3, 15.0, color(t.control));
        let n = titles.len().max(1);
        let tw = (track.2 - 4.0) / n as f32;
        for (i, title) in titles.iter().enumerate() {
            let id = self.ctx.id();
            let tab = (track.0 + 2.0 + i as f32 * tw, track.1 + 2.0, tw, track.3 - 4.0);
            let r = self.interact(id, tab);
            let close = (tab.0 + 6.0, tab.1 + (tab.3 - 16.0) / 2.0, 16.0, 16.0);
            if i == selected {
                knob(self.canvas, tab);
            } else if r.hovered {
                fill_rounded(self.canvas, tab.0, tab.1, tab.2, tab.3, tab.3 / 2.0, color(t.hover));
            }
            if r.hovered && n > 1 {
                let over_close = self.over(close);
                if over_close {
                    fill_rounded(self.canvas, close.0, close.1, 16.0, 16.0, 8.0, color(t.shade(30)));
                }
                let (cx, cy) = (close.0 + 8.0, close.1 + 8.0);
                let mut pb = PathBuilder::new();
                pb.move_to(cx - 3.0, cy - 3.0);
                pb.line_to(cx + 3.0, cy + 3.0);
                pb.move_to(cx + 3.0, cy - 3.0);
                pb.line_to(cx - 3.0, cy + 3.0);
                if let Some(path) = pb.finish() {
                    let mut paint = Paint { anti_alias: true, ..Default::default() };
                    paint.set_color(color(if i == selected { [60, 60, 66, 255] } else { t.text }));
                    let stroke = Stroke { width: 1.4, line_cap: tiny_skia::LineCap::Round, ..Default::default() };
                    self.canvas.stroke_path(&path, &paint, &stroke, crate::ui::draw::tf(), None);
                }
                if r.clicked && over_close {
                    action = TabAction::Close(i);
                    continue;
                }
            }
            if r.clicked && i != selected {
                action = TabAction::Select(i);
            }
            let room = tab.2 - 48.0;
            let shown = self.fit(title, t.font_small, FontWeight::Regular, room.max(20.0));
            let sw = self.measure(&shown, t.font_small, FontWeight::Regular);
            let tc = if i == selected { [24, 24, 28, 255] } else { t.text_secondary };
            self.draw_text(&shown, tab.0 + (tab.2 - sw) / 2.0, tab.1 + (tab.3 - t.font_small * 1.25) / 2.0, t.font_small, FontWeight::Regular, tc);
        }
        // + no fim.
        let id = self.ctx.id();
        let add = (rect.0 + rect.2 - plus, rect.1 + 2.0, plus, plus);
        let r = self.interact(id, add);
        fill_rounded(self.canvas, add.0, add.1, plus, plus, plus / 2.0, color(if r.hovered { t.hover } else { t.control }));
        let (cx, cy) = (add.0 + plus / 2.0, add.1 + plus / 2.0);
        fill_rounded(self.canvas, cx - 5.0, cy - 0.75, 10.0, 1.5, 0.75, color(t.text));
        fill_rounded(self.canvas, cx - 0.75, cy - 5.0, 1.5, 10.0, 0.75, color(t.text));
        if r.clicked {
            action = TabAction::New;
        }
        action
    }

    /// Dica do Tahoe: depois de o ponteiro parar 0,8 s sobre o componente da
    /// `response`, uma cápsula pequena com `text` aparece embaixo dele.
    pub fn tooltip(&mut self, response: &Response, text: &str) {
        let now = std::time::Instant::now();
        if !response.hovered {
            if self.ctx.memory.tooltip_hover.is_some_and(|(id, _)| id == response.id) {
                self.ctx.memory.tooltip_hover = None;
            }
            return;
        }
        let since = match self.ctx.memory.tooltip_hover {
            Some((id, at)) if id == response.id => at,
            _ => {
                self.ctx.memory.tooltip_hover = Some((response.id, now));
                now
            }
        };
        let wait = std::time::Duration::from_millis(800);
        if now.duration_since(since) < wait {
            self.ctx.wake_at = Some(since + wait);
            return;
        }
        let t = self.ctx.theme.clone();
        let (ww, wh) = (self.ctx.window.0 as f32, self.ctx.window.1 as f32);
        let tw = self.ctx.text.measure(text, t.font_small, FontWeight::Regular) + 16.0;
        let r = response.rect;
        let x = (self.offset.0 + r.0 + (r.2 - tw) / 2.0).clamp(4.0, (ww - tw - 4.0).max(4.0));
        let mut y = self.offset.1 + r.1 + r.3 + 6.0;
        if y + 22.0 > wh {
            y = self.offset.1 + r.1 - 28.0;
        }
        let k = crate::ui::draw::scale();
        let overlay = self.ctx.overlay.get_or_insert_with(|| Pixmap::new((ww * k).max(1.0).round() as u32, (wh * k).max(1.0).round() as u32).unwrap_or_else(|| Pixmap::new(1, 1).unwrap()));
        fill_rounded(overlay, x - 1.0, y + 1.0, tw + 2.0, 24.0, 12.0, color([0, 0, 0, 30]));
        fill_rounded(overlay, x, y, tw, 22.0, 11.0, color(t.menu));
        outline(overlay, (x, y, tw, 22.0), 11.0, if t.dark { [255, 255, 255, 30] } else { [0, 0, 0, 18] });
        self.ctx.text.draw(overlay, text, x + 8.0, y + (22.0 - t.font_small * 1.25) / 2.0, t.font_small, FontWeight::Regular, t.text);
    }

    /// Popover do Tahoe: painel de vidro com ponta, embaixo de `anchor` (na janela:
    /// use `ui.to_window(resposta.rect)` onde o botão foi desenhado),
    /// com o conteúdo do `add`. Chamar no fim do `view` (por cima de tudo); fecha
    /// (`*open = false`) com clique fora ou Esc.
    pub fn popover(&mut self, open: &mut bool, anchor: (f32, f32, f32, f32), width: f32, add: impl FnOnce(&mut Ui)) {
        if !*open {
            return;
        }
        let t = self.ctx.theme.clone();
        let ww = self.ctx.window.0 as f32;
        let arrow = crate::ui::draw::ARROW_H;
        let x = (anchor.0 + anchor.2 / 2.0 - width / 2.0).clamp(8.0, (ww - width - 8.0).max(8.0));
        let y = anchor.1 + anchor.3 + arrow + 2.0;
        // O clique deste quadro: dentro do painel (último quadro) vale; fora fecha.
        let last = self.ctx.memory.modal_rect;
        let pressed_outside = self.ctx.input.pressed && last.is_some_and(|r| !self.ctx.input.over(r)) && !self.ctx.input.over(anchor);
        if pressed_outside || self.ctx.input.key(super::input::Key::Escape) {
            *open = false;
            self.ctx.memory.modal_rect = None;
            self.ctx.repaint = true;
            return;
        }
        self.ctx.in_modal = true;
        let rect = self.glass_panel(x, y, width, super::theme::RADIUS_ITEM + 10.0, add);
        self.ctx.in_modal = false;
        let fill = if t.dark { t.card } else { [248, 248, 250, 255] };
        crate::ui::draw::menu_arrow(self.canvas, anchor.0 + anchor.2 / 2.0, y, false, fill);
        self.ctx.memory.modal_rect = Some((self.offset.0 + rect.0, self.offset.1 + rect.1, rect.2, rect.3));
        self.ctx.memory.modal_drawn = true;
    }

    /// Campo de texto de uma linha. Clique dá o foco; Enter responde `submitted`.
    pub fn text_input(&mut self, value: &mut String, placeholder: &str) -> Response {
        use super::textedit::Edit;
        let t = self.ctx.theme.clone();
        let search = std::mem::take(&mut self.search_look);
        let id = self.ctx.id();
        let rect = self.alloc(None, if search { super::theme::TOOLBAR_ITEM } else { t.row });
        // Busca: o ✕ que limpa fica à direita, fora da área do texto.
        let clear_rect = (rect.0 + rect.2 - 30.0, rect.1, 30.0, rect.3);
        let clear_hit = search && !value.is_empty() && self.ctx.input.pressed && self.over(clear_rect);
        let mut r = self.interact(id, rect);
        // Posição do texto no campo e onde o ponteiro está nele (para o cursor).
        let inner = if search { (rect.0 + 32.0, rect.2 - 62.0) } else { (rect.0 + 10.0, rect.2 - 20.0) };
        let pointer_x = self.ctx.input.pointer.map(|(px, _)| px - self.offset.0 - inner.0 + self.ctx.memory.text_scroll);
        if std::mem::take(&mut self.ctx.memory.focus_next) {
            self.ctx.memory.focus = Some(id);
            self.ctx.memory.edit = match self.ctx.memory.focus_select_end.take() {
                Some(end) if end <= value.len() && value.is_char_boundary(end) => Edit { cursor: end, anchor: Some(0) },
                _ => Edit::all(value),
            };
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
        if clear_hit {
            value.clear();
            self.ctx.memory.edit = super::textedit::Edit::at_end(value);
            r.changed = true;
            self.ctx.repaint = true;
        }
        if search {
            // Cápsula de vidro com a lupa; o anel de foco segue a cápsula.
            capsule(self.canvas, rect, &t);
            if focused {
                if let Some(path) = rounded_rect(rect.0 - 1.5, rect.1 - 1.5, rect.2 + 3.0, rect.3 + 3.0, (rect.3 + 3.0) / 2.0) {
                    let mut paint = Paint { anti_alias: true, ..Default::default() };
                    paint.set_color(color([t.accent[0], t.accent[1], t.accent[2], 128]));
                    self.canvas.stroke_path(&path, &paint, &Stroke { width: 3.0, ..Default::default() }, crate::ui::draw::tf(), None);
                }
            }
            if let Some(pix) = self.ctx.icon(&["system-search-symbolic", "edit-find-symbolic"], 16, false) {
                let c = t.text_secondary;
                crate::ui::draw::blit_tinted(self.canvas, &pix, (rect.0 + 11.0) as i32, (rect.1 + (rect.3 - 16.0) / 2.0) as i32, [c[0], c[1], c[2]]);
            }
            if !value.is_empty() {
                let (cx, cy) = (clear_rect.0 + 15.0, clear_rect.1 + rect.3 / 2.0);
                fill_rounded(self.canvas, cx - 7.0, cy - 7.0, 14.0, 14.0, 7.0, color(t.shade(70)));
                let mut pb = tiny_skia::PathBuilder::new();
                pb.move_to(cx - 3.0, cy - 3.0);
                pb.line_to(cx + 3.0, cy + 3.0);
                pb.move_to(cx + 3.0, cy - 3.0);
                pb.line_to(cx - 3.0, cy + 3.0);
                if let Some(path) = pb.finish() {
                    let mut paint = Paint { anti_alias: true, ..Default::default() };
                    paint.set_color(color(t.background));
                    let stroke = Stroke { width: 1.5, line_cap: tiny_skia::LineCap::Round, ..Default::default() };
                    self.canvas.stroke_path(&path, &paint, &stroke, crate::ui::draw::tf(), None);
                }
            }
        } else {
            fill_rounded(self.canvas, rect.0, rect.1, rect.2, rect.3, t.control_radius, color(t.card));
        }
        let border = if focused { t.accent } else { t.shade(40) };
        if search {
        } else if let Some(path) = rounded_rect(rect.0 + 0.5, rect.1 + 0.5, rect.2 - 1.0, rect.3 - 1.0, t.control_radius) {
            let mut paint = Paint { anti_alias: true, ..Default::default() };
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
        let r = self.password_edit(value, rect);
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

    /// Campo de senha com a cara dos campos de texto do tema (diálogos de senha, como o
    /// pedido de senha de administrador): bolinhas, sempre com o teclado, sem copiar.
    /// `error` pinta a borda de vermelho (senha recusada). Mesmas teclas do
    /// `password_input`.
    pub fn password_field(&mut self, value: &mut String, placeholder: &str, error: bool) -> Response {
        let t = self.ctx.theme.clone();
        let rect = self.alloc(None, t.row);
        let r = self.password_edit(value, rect);
        fill_rounded(self.canvas, rect.0, rect.1, rect.2, rect.3, t.control_radius, color(t.card));
        let border = if error { t.danger } else { t.accent };
        if let Some(path) = rounded_rect(rect.0 + 0.5, rect.1 + 0.5, rect.2 - 1.0, rect.3 - 1.0, t.control_radius) {
            let mut paint = Paint { anti_alias: true, ..Default::default() };
            paint.set_color(color(border));
            self.canvas.stroke_path(&path, &paint, &Stroke { width: 2.0, ..Default::default() }, crate::ui::draw::tf(), None);
        }
        let ty = rect.1 + (rect.3 - t.font * 1.25) / 2.0;
        let x = rect.0 + 10.0;
        if value.is_empty() {
            self.draw_text(placeholder, x, ty, t.font, FontWeight::Regular, t.text_secondary);
            fill_rounded(self.canvas, x - 1.0, rect.1 + 7.0, 1.5, rect.3 - 14.0, 0.5, color(t.text));
        } else {
            let room = rect.2 - 24.0;
            let mut dots = "•".repeat(value.chars().count().min(60));
            while dots.chars().count() > 1 && self.measure(&dots, t.font + 2.0, FontWeight::Regular) > room {
                dots.pop();
            }
            let w = self.measure(&dots, t.font + 2.0, FontWeight::Regular);
            self.draw_text(&dots, x, ty - 1.0, t.font + 2.0, FontWeight::Regular, t.text);
            fill_rounded(self.canvas, x + w + 2.0, rect.1 + 7.0, 1.5, rect.3 - 14.0, 0.5, color(t.text));
        }
        r
    }

    /// Texto pequeno em vermelho (a cor de perigo do tema): erros, como "Senha incorreta".
    pub fn error_note(&mut self, s: &str) {
        let t = self.ctx.theme.clone();
        self.paragraph(s, t.font_small, t.danger);
    }

    /// Teclas de um campo de senha: texto, colar, Backspace (com Ctrl apaga tudo), Esc e
    /// Ctrl+U apagam tudo, Enter responde `submitted`.
    fn password_edit(&mut self, value: &mut String, rect: (f32, f32, f32, f32)) -> Response {
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
        r
    }

    /// Área de texto de várias linhas (editor): números de linha, cursor, seleção com
    /// mouse e teclado, desfazer (Ctrl+Z) e refazer (Ctrl+Shift+Z, Ctrl+Y), área de
    /// transferência e rolagem. Só as linhas à vista são desenhadas. Os outros atalhos
    /// com Ctrl (salvar, buscar...) ficam para o app. `changed` diz se o texto mudou.
    pub fn text_area(&mut self, doc: &mut super::textarea::TextDoc, height: f32) -> Response {
        self.text_area_with(doc, height, TextAreaLook::default()).response
    }

    /// A área de texto com outra aparência (`TextAreaLook`): sem números de linha, em
    /// fonte proporcional, com uma dica no documento vazio e decorações (cores,
    /// sublinhados de erro, texto depois do fim da linha). Os atalhos são os da
    /// `text_area`: com a área em foco, Ctrl+A/C/X/V, Ctrl+Z e Ctrl+Y, também com Shift
    /// (Ctrl+Shift+Z refaz), são dela; os outros atalhos com Ctrl ficam para o app.
    pub fn text_area_with(&mut self, doc: &mut super::textarea::TextDoc, height: f32, mut look: TextAreaLook) -> TextAreaResponse {
        use super::textarea::{color_runs, index_in_line, line_end, line_of, tab_pieces, underline_runs, Change};
        use super::textedit::Edit;
        let t = self.ctx.theme.clone();
        let size = look.size.unwrap_or(t.font);
        let sans = look.sans;
        let line_h = (size * 1.5).round();
        let pad = 6.0;
        let id = self.ctx.id();
        let rect = self.alloc(None, height);
        let mut r = self.interact(id, rect);
        let mut starts = doc.line_starts();
        let digits = starts.len().to_string().len().max(2);
        let gutter = if look.line_numbers { self.ctx.text.measure_mono(&"0".repeat(digits), size * 0.9) + 20.0 } else { 0.0 };
        let text_x = rect.0 + gutter + 8.0;
        let view = (rect.2 - gutter - 16.0, rect.3 - 2.0 * pad);
        let rows = (view.1 / line_h).floor().max(1.0) as usize;
        // Largura do começo de uma linha até um ponto, na fonte da área.
        let width = |ui: &mut Self, s: &str| if sans { ui.ctx.text.measure(s, size, FontWeight::Regular) } else { ui.ctx.text.measure_mono(s, size) };
        // Linha e posição sob o ponteiro.
        let pointer_at = |ui: &mut Self, doc: &super::textarea::TextDoc, starts: &[usize]| -> Option<usize> {
            let (px, py) = ui.pointer()?;
            let line = (((py - rect.1 - pad + doc.scroll) / line_h).floor().max(0.0) as usize).min(starts.len() - 1);
            let x = px - text_x + doc.scroll_x;
            Some(index_in_line(&doc.text, starts, line, x, |s| width(ui, s)))
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
                let col_x = |ui: &mut Self, doc: &super::textarea::TextDoc| doc.goal_x.unwrap_or_else(|| width(ui, &doc.text[starts[line]..doc.edit.cursor]));
                let vertical = |ui: &mut Self, doc: &mut super::textarea::TextDoc, to: usize| {
                    let x = col_x(ui, doc);
                    let at = index_in_line(&doc.text, &starts, to, x, |s| width(ui, s));
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
        // Posição do texto sob o ponteiro só no quadro de um clique (o menu de contexto
        // do app usa): medir a cada movimento do mouse custaria caro em linhas longas.
        let clicking = r.secondary_clicked || (r.hovered && (self.ctx.input.pressed || self.ctx.input.released));
        let pointer_index = if clicking { pointer_at(self, doc, &starts) } else { None };
        let cursor_line = line_of(&starts, doc.edit.cursor);
        let cursor_x = width(self, &doc.text[starts[cursor_line]..doc.edit.cursor]);
        // As decorações já com a tecla deste quadro, antes da rolagem: o texto do fim da
        // linha do cursor também precisa ficar à vista.
        let deco = look.decorate.as_mut().map(|f| f(doc)).unwrap_or_default();
        let has_trailing = |line: usize| deco.trailing.iter().any(|(l, _, _)| *l == line);
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
        // Com o cursor no fim de uma linha que tem texto depois do fim (o resultado das
        // notas), esse texto também fica à vista, sem o cursor sair da área. Vale em todo
        // quadro: o texto muda sem tecla (Graus/Radianos, casas decimais).
        let clip_w = view.0 + 8.0;
        if doc.edit.cursor == line_end(&doc.text, &starts, cursor_line) && has_trailing(cursor_line) {
            let mut right = cursor_x;
            for (_, extra, _) in deco.trailing.iter().filter(|(l, _, _)| *l == cursor_line) {
                right += 4.0 + width(self, extra);
            }
            if right > doc.scroll_x + clip_w - 4.0 {
                doc.scroll_x = (right - clip_w + 8.0).min(cursor_x).max(0.0);
            }
        }
        doc.scroll = doc.scroll.clamp(0.0, (content_h - view.1).max(0.0));

        // Desenho: fundo, margem com os números e só as linhas à vista.
        fill_rounded(self.canvas, rect.0, rect.1, rect.2, rect.3, t.control_radius, color(t.card));
        if look.line_numbers {
            fill_rounded(self.canvas, rect.0, rect.1, gutter, rect.3, t.control_radius, color(t.shade(10)));
        }
        let first = (doc.scroll / line_h).floor() as usize;
        let last = (first + rows + 2).min(starts.len());
        let (w, h) = (clip_w.ceil().max(1.0) as u32, rect.3.ceil().max(1.0) as u32);
        let k = crate::ui::draw::scale();
        let Some(mut clip) = Pixmap::new((w as f32 * k).ceil() as u32, (h as f32 * k).ceil() as u32) else {
            return TextAreaResponse { response: r, pointer_index };
        };
        let selection = doc.edit.selection();
        let [ar, ag, ab, _] = t.accent;
        let text_y = (line_h - size * 1.25) / 2.0;
        // Desenha um texto na fonte da área; devolve a largura desenhada.
        let draw = |ui: &mut Self, pixmap: &mut Pixmap, s: &str, x: f32, y: f32, c: Rgba| -> f32 {
            if sans {
                ui.ctx.text.draw(pixmap, s, x, y, size, FontWeight::Regular, c)
            } else {
                ui.ctx.text.draw_mono(pixmap, s, x, y, size, c)
            }
        };
        if doc.text.is_empty() && !look.placeholder.is_empty() {
            // A dica cortada com "…" quando a área é estreita.
            let hint = self.fit_face(look.placeholder, size, sans, view.0);
            draw(self, &mut clip, &hint, -doc.scroll_x, pad + text_y, t.text_secondary);
        }
        for line in first..last {
            let (start, end) = (starts[line], line_end(&doc.text, &starts, line));
            let y = pad + line as f32 * line_h - doc.scroll;
            let x0 = -doc.scroll_x;
            let trailing = has_trailing(line);
            if focused && line == cursor_line && selection.is_none() {
                fill_rounded(&mut clip, 0.0, y, w as f32, line_h, 0.0, color([ar, ag, ab, 14]));
            }
            if let Some((a, b)) = selection.filter(|(a, b)| *a <= end && *b >= start && !(a == b)) {
                let (sa, sb) = (a.max(start), b.min(end));
                let xa = width(self, &doc.text[start..sa]);
                let xb = width(self, &doc.text[start..sb]);
                // A quebra de linha selecionada aparece como um pouco a mais no fim (menos
                // numa linha com texto depois do fim: a marca ficaria embaixo dele).
                let extra = if b > end && !trailing { size * 0.5 } else { 0.0 };
                fill_rounded(&mut clip, x0 + xa, y, xb - xa + extra, line_h, 0.0, color([ar, ag, ab, 70]));
            }
            // O texto em pedaços por cor; o x de cada pedaço é medido do começo da linha
            // (sem somar pedaço a pedaço, que acumularia erro). Um pedaço do meio da linha
            // com tabulação é cortado nela (`tab_pieces`).
            for (a, b, c) in color_runs(&doc.text, start, end, &deco.colors) {
                let rgba = c.unwrap_or(t.text);
                for (p, q) in tab_pieces(&doc.text, start, a, b) {
                    let x = x0 + if p == start { 0.0 } else { width(self, &doc.text[start..p]) };
                    draw(self, &mut clip, &doc.text[p..q], x, y + text_y, rgba);
                }
            }
            // Sublinhado pontilhado (erros), logo abaixo das letras.
            for (a, b) in underline_runs(&doc.text, start, end, &deco.underlines) {
                let xa = x0 + width(self, &doc.text[start..a]);
                let xb = x0 + width(self, &doc.text[start..b]);
                dotted_line(&mut clip, xa, xb, y + text_y + size * 1.2, t.danger);
            }
            // O texto do fim da linha (vários vêm um depois do outro, com 4 pt antes de
            // cada um); o fim da linha só é medido quando ela tem algum. O que não cabe
            // até a borda termina com "…": um número cortado não pode parecer inteiro.
            if trailing {
                let mut x = x0 + width(self, &doc.text[start..end]) + 4.0;
                for (_, extra, rgba) in deco.trailing.iter().filter(|(l, _, _)| *l == line) {
                    let room = w as f32 - 4.0 - x;
                    let shown = if width(self, extra) <= room { extra.clone() } else { self.fit_face(extra, size, sans, room) };
                    if room <= 0.0 || width(self, &shown) > room {
                        break;
                    }
                    x += draw(self, &mut clip, &shown, x, y + text_y, *rgba) + 4.0;
                }
            }
            if look.line_numbers {
                let number = (line + 1).to_string();
                let nw = self.ctx.text.measure_mono(&number, size * 0.9);
                let ny = rect.1 + y + (line_h - size * 0.9 * 1.25) / 2.0;
                if ny > rect.1 && ny < rect.1 + rect.3 - size {
                    let shade = if line == cursor_line && focused { t.text } else { t.text_secondary };
                    self.ctx.text.draw_mono(self.canvas, &number, rect.0 + gutter - 10.0 - nw, ny, size * 0.9, shade);
                }
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
        TextAreaResponse { response: r, pointer_index }
    }

    /// Item de lista clicável: texto à esquerda, detalhe cinza à direita.
    pub fn list_item(&mut self, label: &str, detail: &str) -> Response {
        let t = self.ctx.theme.clone();
        let id = self.ctx.id();
        let rect = self.alloc(None, t.row);
        let first = self.card_first;
        self.card_line(rect.1);
        // Num cartão, a linha inteira (de borda a borda, de divisória a divisória)
        // recebe o clique e o destaque; fora dele, uma pílula com folga.
        let full = if self.in_card {
            let up = if first { 6.0 } else { t.spacing / 2.0 - 0.5 };
            let inset = self.card_inset;
            (rect.0 - inset, rect.1 - up, rect.2 + 2.0 * inset, rect.3 + up + t.spacing / 2.0 + 0.5)
        } else {
            (rect.0 - 8.0, rect.1 + 3.0, rect.2 + 16.0, rect.3 - 6.0)
        };
        let r = self.interact(id, (full.0, rect.1, full.2, rect.3));
        // Fora da área à vista: só o lugar e o clique (listas longas desenham só o visível).
        if !self.visible(rect) {
            return r;
        }
        if r.hovered {
            let radius = if self.in_card { 0.0 } else { t.control_radius };
            fill_rounded(self.canvas, full.0, full.1, full.2, full.3, radius, color(t.control));
        }
        let dw = if detail.is_empty() { 0.0 } else { self.measure(detail, t.font_small, FontWeight::Regular) + 12.0 };
        let label = self.fit(label, t.font, FontWeight::Regular, rect.2 - dw);
        self.draw_text(&label, rect.0, rect.1 + (rect.3 - t.font * 1.25) / 2.0, t.font, FontWeight::Regular, t.text);
        if !detail.is_empty() {
            self.draw_text(detail, rect.0 + rect.2 - dw + 12.0, rect.1 + (rect.3 - t.font_small * 1.25) / 2.0, t.font_small, FontWeight::Regular, t.text_secondary);
        }
        r
    }

    /// Item de uma lista de escolha (o seletor de lista): texto, detalhe cinza à direita;
    /// o marcado numa pílula na cor de destaque; desativado, em cinza e sem clique. Ocupa
    /// a altura de uma linha (`theme().row`).
    pub fn choice_item(&mut self, label: &str, detail: &str, selected: bool, enabled: bool) -> Response {
        let t = self.ctx.theme.clone();
        let id = self.ctx.id();
        let rect = self.alloc(None, t.row);
        let mut r = self.interact(id, (rect.0 - 8.0, rect.1, rect.2 + 16.0, rect.3));
        if !enabled {
            r.clicked = false;
            r.double_clicked = false;
        }
        if !self.visible(rect) {
            return r;
        }
        // A pílula fica dentro da linha (numa lista sem margem, nada sai cortado); o texto
        // recua 10 pt, e o detalhe para 14 pt antes da borda (lugar da barra de rolagem).
        let (fg, sub) = if selected {
            fill_rounded(self.canvas, rect.0, rect.1 + 2.0, rect.2, rect.3 - 4.0, t.control_radius, color(t.accent));
            (t.on_accent, t.on_accent)
        } else {
            if r.hovered && enabled {
                fill_rounded(self.canvas, rect.0, rect.1 + 2.0, rect.2, rect.3 - 4.0, t.control_radius, color(t.control));
            }
            (if enabled { t.text } else { t.text_secondary }, t.text_secondary)
        };
        let (left, right) = (rect.0 + 10.0, rect.0 + rect.2 - 14.0);
        let room = ((right - left) * 0.4).max(0.0);
        let measured = if detail.is_empty() { 0.0 } else { self.measure(detail, t.font_small, FontWeight::Regular) };
        let detail = if measured <= room { detail.to_string() } else { self.fit(detail, t.font_small, FontWeight::Regular, room) };
        let dw = if detail.is_empty() { 0.0 } else { measured.min(room) + 12.0 };
        let label = self.fit(label, t.font, FontWeight::Regular, right - left - dw);
        self.draw_text(&label, left, rect.1 + (rect.3 - t.font * 1.25) / 2.0, t.font, FontWeight::Regular, fg);
        if !detail.is_empty() {
            self.draw_text(&detail, right - dw + 12.0, rect.1 + (rect.3 - t.font_small * 1.25) / 2.0, t.font_small, FontWeight::Regular, sub);
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
        // Nome claro: ladrilho sobre o papel de parede (área de trabalho).
        let on_wallpaper = (t.tile_text[0] as u32 + t.tile_text[1] as u32 + t.tile_text[2] as u32) > 600;
        if selected && on_wallpaper {
            crate::ui::draw::glass(self.canvas, ix - 8.0, iy - 6.0, fw + 16.0, fh + 12.0, 10.0);
        } else if selected {
            // Como no Finder: fundo cinza arredondado atrás do ícone.
            fill_rounded(self.canvas, ix - 6.0, iy - 5.0, fw + 12.0, fh + 10.0, super::theme::RADIUS_ITEM, color(t.shade(if t.dark { 46 } else { 26 })));
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
            let h = t.font_small * 1.4;
            fill_rounded(self.canvas, lx - 6.0, ly - 1.0, lw + 12.0, h, h / 2.0, color(self.selection_fill()));
        }
        let c = if selected && self.ctx.input.focused { t.on_accent } else { t.tile_text };
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

    /// Desenha `add` recortado no retângulo `rect` (nesta área): o que passa das bordas
    /// não aparece (colunas do Finder deslizando para o lado). Dentro, as coordenadas
    /// começam no canto do retângulo.
    pub fn clip(&mut self, rect: (f32, f32, f32, f32), add: impl FnOnce(&mut Ui)) {
        let (inner, _) = self.sub_area_from(rect.0, rect.1, rect.2, rect.3, rect.1, true, add);
        copy_back(&inner, self.canvas, crate::ui::draw::px(rect.0), crate::ui::draw::px(rect.1));
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
            fill_rounded(self.canvas, rect.0, rect.1, rect.2, rect.3, super::theme::RADIUS_ITEM, color(self.selection_fill()));
        } else if striped {
            fill_rounded(self.canvas, rect.0, rect.1, rect.2, rect.3, super::theme::RADIUS_ITEM, color(t.shade(10)));
        }
        let on = selected && self.ctx.input.focused;
        let (main, side) = if on { (t.on_accent, t.on_accent) } else { (t.text, t.text_secondary) };
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
                paint.set_color(color(if on { t.on_accent } else { t.text_secondary }));
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
            fill_rounded(self.canvas, rect.0, rect.1, rect.2, rect.3, super::theme::RADIUS_ITEM, color(self.selection_fill()));
        } else if r.hovered {
            fill_rounded(self.canvas, rect.0, rect.1, rect.2, rect.3, super::theme::RADIUS_ITEM, color(t.hover));
        }
        let pix = self.ctx.icon(&[icon, "text-x-generic"], 20, false);
        if let Some(pix) = pix {
            crate::ui::draw::blit(self.canvas, &pix, (rect.0 + 8.0) as i32, (rect.1 + 4.0) as i32, 1.0);
        }
        let cols: f32 = columns.iter().map(|c| c.1).sum();
        let ty = rect.1 + (28.0 - t.font * 1.25) / 2.0;
        let (main, side) = if selected && self.ctx.input.focused { (t.on_accent, t.on_accent) } else { (t.text, t.text_secondary) };
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
    /// centralizados na altura, e uma linha fina embaixo. Numa janela de conteúdo
    /// inteiro (Tahoe), sem fundo nem linha, com itens de 36 (`toolbar_buttons`,
    /// `toolbar_title`, `search_field`), os botões da janela no começo quando a barra
    /// encosta na borda esquerda, e o vazio arrasta a janela.
    pub fn toolbar(&mut self, add: impl FnOnce(&mut Ui)) {
        let t = self.ctx.theme.clone();
        let tahoe = self.ctx.content_window;
        let (x, y0, w, h) = (self.x, self.cursor_y, self.width, super::theme::TOOLBAR_HEIGHT);
        if !tahoe {
            fill_rounded(self.canvas, x, y0 + h - 1.0, w, 1.0, 0.0, color(t.line));
        }
        let (old_flow, old_x, old_w, old_h) = (self.flow, self.row_x, self.x, self.row_h);
        self.flow = Flow::Row;
        let mut start = x + 12.0;
        // Sem barra lateral à esquerda: os botões da janela abrem a barra.
        if tahoe && self.offset.0 + x < 1.0 {
            let lights_y = y0 + (h - LIGHT_SIZE) / 2.0;
            start += 8.0 + self.window_controls(x + 20.0, lights_y) + 8.0;
        }
        self.x = start;
        self.row_x = start;
        self.row_h = 0.0;
        let old_y = self.cursor_y;
        let item_h = if tahoe { super::theme::TOOLBAR_ITEM } else { 28.0 };
        self.cursor_y = y0 + (h - item_h) / 2.0;
        let old_width = self.width;
        self.width = w - (start - x) - 12.0;
        add(self);
        self.flow = old_flow;
        self.row_x = old_x;
        self.x = old_w;
        self.row_h = old_h;
        self.width = old_width;
        self.cursor_y = old_y + h;
        // O vazio da barra arrasta a janela (depois dos itens, que têm prioridade).
        self.window_drag_area((x, y0, w, h));
    }

    /// Grupo de botões de ícone numa cápsula de vidro, para a barra de ferramentas do
    /// Tahoe: cada item é (ícone, texto de reserva, marcado, ativo); o marcado ganha
    /// uma pílula dentro da cápsula. Devolve o índice clicado.
    pub fn toolbar_buttons(&mut self, items: &[(&str, &str, bool, bool)]) -> Option<usize> {
        let t = self.ctx.theme.clone();
        let seg = 40.0;
        let h = super::theme::TOOLBAR_ITEM;
        let rect = self.alloc(Some(seg * items.len() as f32 + 4.0), h);
        capsule(self.canvas, rect, &t);
        let mut clicked = None;
        for (i, (icon, fallback, on, enabled)) in items.iter().enumerate() {
            let id = self.ctx.id();
            let r_rect = (rect.0 + 2.0 + i as f32 * seg, rect.1, seg, h);
            let r = self.interact(id, r_rect);
            if r.clicked && *enabled {
                clicked = Some(i);
            }
            let pill = (r_rect.0 + 3.0, r_rect.1 + 4.0, r_rect.2 - 6.0, r_rect.3 - 8.0);
            if *on {
                fill_rounded(self.canvas, pill.0, pill.1, pill.2, pill.3, pill.3 / 2.0, color(t.shade(if t.dark { 40 } else { 22 })));
            } else if r.hovered && *enabled {
                fill_rounded(self.canvas, pill.0, pill.1, pill.2, pill.3, pill.3 / 2.0, color(t.shade(if t.dark { 24 } else { 12 })));
            }
            let size = 18;
            let (ix, iy) = ((r_rect.0 + (seg - size as f32) / 2.0) as i32, (r_rect.1 + (h - size as f32) / 2.0) as i32);
            let c = if *enabled { t.text } else { t.shade(70) };
            match self.ctx.icon(&[icon], size, false) {
                Some(pix) if icon.ends_with("-symbolic") => crate::ui::draw::blit_tinted(self.canvas, &pix, ix, iy, [c[0], c[1], c[2]]),
                Some(pix) => crate::ui::draw::blit(self.canvas, &pix, ix, iy, if *enabled { 1.0 } else { 0.35 }),
                None => {
                    let fw = self.measure(fallback, 16.0, FontWeight::Semibold);
                    self.draw_text(fallback, r_rect.0 + (seg - fw) / 2.0, r_rect.1 + 8.0, 16.0, FontWeight::Semibold, c);
                }
            }
        }
        clicked
    }

    /// Título da janela na barra de ferramentas do Tahoe (à esquerda, semibold 15),
    /// cortado com reticências em `max_width`.
    pub fn toolbar_title(&mut self, title: &str, max_width: f32) {
        let t = self.ctx.theme.clone();
        let shown = self.fit(title, 15.0, FontWeight::Semibold, max_width);
        let w = self.measure(&shown, 15.0, FontWeight::Semibold);
        let rect = self.alloc(Some(w + 8.0), super::theme::TOOLBAR_ITEM);
        self.draw_text(&shown, rect.0 + 4.0, rect.1 + (rect.3 - 15.0 * 1.25) / 2.0, 15.0, FontWeight::Semibold, t.text);
    }

    /// Campo de busca em cápsula (Tahoe): lupa à esquerda, ✕ para limpar.
    pub fn search_field(&mut self, value: &mut String, placeholder: &str) -> Response {
        self.search_look = true;
        self.text_input(value, placeholder)
    }

    /// Imagem grande centralizada numa faixa de `size` de altura (prévia do Arquivos):
    /// a miniatura, se houver, senão o ícone `icon` do tema.
    pub fn image_centered(&mut self, image: Option<&Pixmap>, icon: &str, size: u32) {
        let rect = self.alloc(None, size as f32);
        if !self.visible(rect) {
            return;
        }
        match image {
            Some(pix) => {
                let (pw, ph) = (pix.width() as f32, pix.height() as f32);
                let (x, y) = (rect.0 + (rect.2 - pw) / 2.0, rect.1 + (rect.3 - ph) / 2.0);
                crate::ui::draw::blit_image(self.canvas, pix, x.floor(), y.floor());
            }
            None => {
                if let Some(pix) = self.ctx.icon(&[icon, "text-x-generic"], size, false) {
                    let x = rect.0 + (rect.2 - size as f32) / 2.0;
                    crate::ui::draw::blit(self.canvas, &pix, x as i32, rect.1 as i32, 1.0);
                }
            }
        }
    }

    /// Barra de caminho (como a do Finder, embaixo da janela): ícone e nome de cada
    /// pasta, separados por "›". `parts` são (nome, ícone do tema). Devolve o pedaço
    /// clicado.
    pub fn path_bar(&mut self, parts: &[(String, String)]) -> Option<usize> {
        let t = self.ctx.theme.clone();
        // Rodapé do Finder: 28 de altura, linha fina só em cima, sem espaço até a
        // barra de status embaixo.
        let h = 28.0;
        let rect = self.alloc(None, h);
        self.tight_after();
        fill_rounded(self.canvas, rect.0, rect.1, rect.2, 1.0, 0.0, color(t.line));
        // Sem espaço para tudo: os primeiros nomes encolhem antes do último.
        let room = rect.2 - 16.0;
        let n = parts.len().max(1);
        let each = (room / n as f32 - 34.0).clamp(40.0, 200.0);
        let mut x = rect.0 + 8.0;
        let mut clicked = None;
        for (i, (name, icon)) in parts.iter().enumerate() {
            let last = i + 1 == parts.len();
            let limit = if last { (rect.0 + rect.2 - x - 28.0).max(40.0) } else { each };
            let shown = self.fit(name, t.font_small, FontWeight::Regular, limit);
            let w = 20.0 + self.measure(&shown, t.font_small, FontWeight::Regular);
            let cell = (x - 4.0, rect.1 + (h - 20.0) / 2.0, w + 8.0, 20.0);
            let id = self.ctx.id();
            let r = self.interact(id, cell);
            if r.hovered {
                fill_rounded(self.canvas, cell.0, cell.1, cell.2, cell.3, 10.0, color(t.control));
            }
            if let Some(pix) = self.ctx.icon(&[icon.as_str(), "folder"], 16, false) {
                crate::ui::draw::blit(self.canvas, &pix, x as i32, (rect.1 + (h - 16.0) / 2.0) as i32, 1.0);
            }
            self.draw_text(&shown, x + 20.0, rect.1 + (h - t.font_small * 1.25) / 2.0, t.font_small, FontWeight::Regular, t.text);
            if r.clicked {
                clicked = Some(i);
            }
            x += w + 6.0;
            if !last {
                self.draw_text("›", x, rect.1 + (h - t.font_small * 1.25) / 2.0, t.font_small, FontWeight::Regular, t.text_secondary);
                x += 12.0;
            }
        }
        clicked
    }

    /// Linha de status (como a do Finder: "1 de 12 itens escolhido, 20 GB
    /// disponíveis"), com o texto no meio. Devolve o retângulo (para pôr controles
    /// por cima, como o zoom dos ícones).
    pub fn status_bar(&mut self, text: &str) -> (f32, f32, f32, f32) {
        let t = self.ctx.theme.clone();
        let h = 26.0;
        let rect = self.alloc(None, h);
        self.tight_after();
        let shown = self.fit(text, t.font_small, FontWeight::Regular, rect.2 - 360.0);
        let w = self.measure(&shown, t.font_small, FontWeight::Regular);
        self.draw_text(&shown, rect.0 + (rect.2 - w) / 2.0, rect.1 + (h - t.font_small * 1.25) / 2.0, t.font_small, FontWeight::Regular, t.text_secondary);
        rect
    }

    /// Controle deslizante pequeno, sem rótulo (zoom dos ícones na barra de status),
    /// com `width` de largura. Muda o valor enquanto arrasta.
    pub fn mini_slider(&mut self, width: f32, value: &mut f32, min: f32, max: f32) -> Response {
        let t = self.ctx.theme.clone();
        let id = self.ctx.id();
        let rect = self.alloc(Some(width), 18.0);
        let mut r = self.interact(id, rect);
        let (tx, tw) = (rect.0 + 7.0, rect.2 - 14.0);
        if self.ctx.memory.active == Some(id) && (self.ctx.input.down || self.ctx.input.released) {
            if let Some((px, _)) = self.pointer() {
                let new = min + ((px - tx) / tw).clamp(0.0, 1.0) * (max - min);
                if (new - *value).abs() > f32::EPSILON {
                    *value = new;
                    r.changed = true;
                    self.ctx.repaint = true;
                }
            }
        }
        let k = ((*value - min) / (max - min).max(f32::EPSILON)).clamp(0.0, 1.0);
        let cy = rect.1 + rect.3 / 2.0;
        fill_rounded(self.canvas, tx, cy - 2.0, tw, 4.0, 2.0, color(t.shade(40)));
        fill_rounded(self.canvas, tx, cy - 2.0, tw * k, 4.0, 2.0, color(t.accent));
        if let Some(knob) = PathBuilder::from_circle(tx + tw * k, cy, 6.0) {
            let mut paint = Paint::default();
            paint.anti_alias = true;
            paint.set_color(color([255, 255, 255, 255]));
            self.canvas.fill_path(&knob, &paint, FillRule::Winding, Transform::identity(), None);
            paint.set_color(color(t.shade(70)));
            self.canvas.stroke_path(&knob, &paint, &tiny_skia::Stroke { width: 1.0, ..Default::default() }, Transform::identity(), None);
        }
        r
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
                search_look: false,
                card_inset: 0.0,
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
        let card_w = self.width;
        let inner_w = card_w - 2.0 * pad_x;
        let y0 = self.cursor_y;
        // Só a parte do cartão que aparece na tela ganha imagem. A imagem tem a largura
        // do cartão inteiro (os itens ficam recuados): o destaque das linhas vai de borda
        // a borda e os cantos são recortados na volta.
        let top = (y0 + pad_y).max(self.visible.0);
        let bottom = self.visible.1;
        let mut first = true;
        let (inner, used) = self.sub_area(x, top, card_w, bottom - top, y0 + pad_y, |ui| {
            ui.x = pad_x;
            ui.width = inner_w;
            ui.in_card = true;
            ui.card_inset = pad_x;
            ui.card_first = first;
            add(ui);
            first = ui.card_first;
        });
        let _ = first;
        let h = (used - t.spacing).max(0.0) + 2.0 * pad_y;
        fill_rounded(self.canvas, x, y0, card_w, h, t.radius, color(t.card));
        if bottom > top {
            // Recorta no fim do cartão (a imagem cobre a faixa visível inteira) e nos
            // cantos arredondados dele.
            let k = crate::ui::draw::scale();
            let clip_h = (((y0 + h) - top) * k).min(inner.height() as f32).max(0.0) as u32;
            if clip_h > 0 {
                if let Some(part) = inner.clone_rect(tiny_skia::IntRect::from_xywh(0, 0, inner.width(), clip_h).unwrap()) {
                    let mask = tiny_skia::Mask::new(self.canvas.width(), self.canvas.height()).and_then(|mut m| {
                        let path = rounded_rect(x, y0, card_w, h, t.radius)?;
                        m.fill_path(&path, FillRule::Winding, true, crate::ui::draw::tf());
                        Some(m)
                    });
                    self.canvas.draw_pixmap(crate::ui::draw::px(x), crate::ui::draw::px(top), part.as_ref(), &PixmapPaint::default(), Transform::identity(), mask.as_ref());
                }
            }
        }
        if let Some(path) = rounded_rect(x + 0.5, y0 + 0.5, card_w - 1.0, h - 1.0, t.radius) {
            let mut paint = Paint::default();
            paint.anti_alias = true;
            paint.set_color(color(t.line));
            self.canvas.stroke_path(&path, &paint, &Stroke::default(), crate::ui::draw::tf(), None);
        }
        self.cursor_y = y0 + h + t.spacing;
    }

    /// Área que rola, ocupando o resto da altura, com margem do tema e barra de
    /// rolagem fina à direita quando o conteúdo não cabe.
    pub fn page(&mut self, add: impl FnOnce(&mut Ui)) {
        let h = self.visible.1 - self.cursor_y;
        let pad = self.ctx.theme.padding;
        self.scroll_impl(h, pad, None, add);
    }

    /// Área que rola com altura fixa (`height`), sem margem: listas dentro de
    /// diálogos (o seletor de arquivos) e painéis.
    pub fn scroll_area(&mut self, height: f32, add: impl FnOnce(&mut Ui)) {
        self.scroll_impl(height, 0.0, None, add);
    }

    /// Como `scroll_area`, mas rola o mínimo para a faixa `keep` (do topo do conteúdo,
    /// em pontos) ficar à vista: o item marcado pelo teclado numa lista longa.
    pub fn scroll_area_showing(&mut self, height: f32, keep: Option<(f32, f32)>, add: impl FnOnce(&mut Ui)) {
        self.scroll_impl(height, 0.0, keep, add);
    }

    fn scroll_impl(&mut self, h: f32, pad: f32, keep: Option<(f32, f32)>, add: impl FnOnce(&mut Ui)) {
        let id = self.ctx.id();
        let (x, y0) = (self.x, self.cursor_y);
        let w = self.width;
        let mut scroll = *self.ctx.memory.scroll.get(&id).unwrap_or(&0.0);
        if let Some((top, bottom)) = keep {
            if top < scroll {
                scroll = top;
            } else if bottom > scroll + h {
                scroll = bottom - h;
            }
        }
        // A imagem cobre só a área visível da página (começando com o que já está lá);
        // o conteúdo começa rolado.
        let (inner, used) = self.sub_area_from(x + pad, y0, w - 2.0 * pad, h, y0 + pad - scroll, true, add);
        let content_h = used + 2.0 * pad;
        let max_scroll = (content_h - h).max(0.0);
        // Um item lá dentro pediu para aparecer (`Ui::reveal`): rola o mínimo e redesenha.
        if let Some((top, bottom)) = self.ctx.reveal.take() {
            let before = scroll;
            let (view_top, view_bottom) = (self.offset.1 + y0, self.offset.1 + y0 + h);
            if top < view_top {
                scroll -= view_top - top + pad;
            } else if bottom > view_bottom {
                scroll += bottom - view_bottom + pad;
            }
            if (scroll - before).abs() > 0.5 {
                tracing::debug!("rolagem até o item pedido: {before:.0} → {:.0}", scroll.clamp(0.0, max_scroll));
                self.ctx.repaint = true;
            }
        }
        // Roda do mouse sobre a área.
        let over = self.over((x, y0, w, h));
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
        // Tahoe (janela de conteúdo inteiro): painel de vidro solto da borda, com os
        // botões da janela dentro, seleção em pílula e o vazio arrastando a janela.
        let tahoe = self.ctx.content_window;
        let inset = if tahoe { super::theme::INSET_FLOAT } else { 0.0 };
        let panel = (x + inset, y0 + inset, side_w - inset, h - 2.0 * inset);
        let panel_r = if tahoe { super::theme::RADIUS_SIDEBAR } else { 0.0 };
        let glass = std::mem::take(&mut self.sidebar_glass);
        if tahoe {
            fill_rounded(self.canvas, x, y0, side_w, h, 0.0, color(t.background));
        }
        // Barra lateral de vidro (como o acesso rápido do Finder): o mesmo véu dos
        // menus da barra, com o compositor desfocando o que está atrás da janela.
        // Com o vidro desligado nas Configurações, fica opaca.
        if glass {
            self.ctx.glass_capable = true;
            if crate::control_client::panel_glass() {
                if tahoe {
                    crate::ui::draw::clear_rounded(self.canvas, panel.0, panel.1, panel.2, panel.3, panel_r);
                }
                crate::ui::draw::paint_rounded_solid(self.canvas, panel.0, panel.1, panel.2, panel.3, panel_r, t.sidebar_glass);
                self.ctx.glass.push((self.offset.0 + panel.0, self.offset.1 + panel.1, panel.2, panel.3, panel_r));
            } else {
                fill_rounded(self.canvas, panel.0, panel.1, panel.2, panel.3, panel_r, color(t.sidebar));
            }
        } else {
            fill_rounded(self.canvas, panel.0, panel.1, panel.2, panel.3, panel_r, color(t.sidebar));
        }
        if tahoe {
            // Borda de luz do vidro e uma sombra curta embaixo.
            if let Some(path) = rounded_rect(panel.0 + 0.5, panel.1 + 0.5, panel.2 - 1.0, panel.3 - 1.0, panel_r - 0.5) {
                let mut paint = Paint { anti_alias: true, ..Default::default() };
                paint.set_color(color(if t.dark { [255, 255, 255, 34] } else { [0, 0, 0, 20] }));
                self.canvas.stroke_path(&path, &paint, &Stroke { width: 1.0, ..Default::default() }, crate::ui::draw::tf(), None);
            }
            self.window_controls(x + 20.0, y0 + (super::theme::TOOLBAR_HEIGHT - LIGHT_SIZE) / 2.0);
        } else {
            fill_rounded(self.canvas, x + side_w - 1.0, y0, 1.0, h, 0.0, color(t.line));
        }
        // Medidas das linhas: (ícone, título de grupo, espaço, primeiro título).
        let (row_icon, row_h_icon, title_h, first_title_h) = if tahoe { (28.0, 30.0, 28.0, 22.0) } else { (30.0, 34.0, 32.0, 24.0) };
        let heights: Vec<f32> = items
            .iter()
            .enumerate()
            .map(|(i, (name, icon, _))| if !icon.is_empty() { row_h_icon } else if name.is_empty() { 12.0 } else if i > 0 { title_h } else { first_title_h })
            .collect();
        let mut row_y = if tahoe { y0 + super::theme::TOOLBAR_HEIGHT } else { y0 + 16.0 };
        if let Some((query, placeholder)) = search {
            // Campo de busca no topo da barra lateral.
            let (old_x, old_w, old_y) = (self.x, self.width, self.cursor_y);
            self.x = panel.0 + 10.0;
            self.width = panel.2 - 20.0;
            self.cursor_y = if tahoe { row_y } else { y0 + 12.0 };
            if tahoe {
                self.search_field(query, placeholder);
            } else {
                self.text_input(query, placeholder);
            }
            row_y = self.cursor_y + 4.0;
            self.x = old_x;
            self.width = old_w;
            self.cursor_y = old_y;
        }
        // Itens que não cabem: a lista rola com a roda do mouse sobre a barra lateral
        // (o campo de busca fica parado), e o item escolhido aparece ao abrir.
        let list_top = row_y;
        let list_bottom = panel.1 + panel.3 - 8.0;
        let total: f32 = heights.iter().sum();
        let scroll_id = self.ctx.id();
        let max_scroll = (total - (list_bottom - list_top)).max(0.0);
        let mut scroll = self.ctx.memory.scroll.get(&scroll_id).copied().unwrap_or_else(|| {
            // Primeira vez: rola até o item escolhido, se ele estiver fora.
            let before: f32 = heights[..selected.min(items.len())].iter().sum();
            (before + row_h_icon - (list_bottom - list_top)).max(0.0)
        });
        // Com um modal aberto, a roda é dele (a lista atrás não rola).
        let blocked = self.ctx.memory.modal_rect.is_some() && !self.ctx.in_modal;
        if !blocked && self.over((x, list_top, side_w, list_bottom - list_top)) && self.ctx.input.scroll != 0.0 && max_scroll > 0.0 {
            scroll += std::mem::take(&mut self.ctx.input.scroll);
            self.ctx.repaint = true;
        }
        let scroll = scroll.clamp(0.0, max_scroll);
        self.ctx.memory.scroll.insert(scroll_id, scroll);
        row_y -= scroll;
        // Seleção: pílula de destaque (Configurações, com quadradinhos coloridos, e o
        // Big Sur); no Finder do Tahoe, pílula cinza com o ícone na cor de destaque.
        let accent_pill = !tahoe || items.iter().any(|(_, _, tint)| tint.is_some());
        for (i, (name, icon, tint)) in items.iter().enumerate() {
            // Fora da faixa da lista (rolada): só conta o lugar.
            let row_h = heights[i];
            if row_y < list_top - 0.5 || row_y + row_h - 4.0 > list_bottom + 0.5 {
                if !icon.is_empty() {
                    let _ = self.ctx.id();
                }
                row_y += row_h;
                continue;
            }
            // Item sem ícone é um título de grupo ("Favoritos"); nome vazio é só espaço.
            if icon.is_empty() {
                if name.is_empty() {
                    row_y += 12.0;
                    continue;
                }
                if i > 0 {
                    row_y += 8.0;
                }
                let (fx, fs) = if tahoe { (panel.0 + 14.0, 11.0) } else { (x + 20.0, t.font_small) };
                self.draw_text(name, fx, row_y + 4.0, fs, FontWeight::Semibold, t.text_secondary);
                row_y += row_h - 8.0 * (i > 0) as u8 as f32;
                continue;
            }
            let id = self.ctx.id();
            let rect = if tahoe { (panel.0 + 8.0, row_y, panel.2 - 16.0, row_icon) } else { (x + 10.0, row_y, side_w - 20.0, row_icon) };
            row_y += row_h;
            let r = self.interact(id, rect);
            if r.clicked && selected != i {
                selected = i;
                self.ctx.repaint = true;
            }
            let is_sel = selected == i;
            let radius = if tahoe { super::theme::RADIUS_ITEM } else { t.control_radius };
            let on_accent = is_sel && accent_pill;
            if is_sel {
                let fill = if accent_pill { t.accent } else { t.shade(if t.dark { 40 } else { 22 }) };
                fill_rounded(self.canvas, rect.0, rect.1, rect.2, rect.3, radius, color(fill));
            } else if r.hovered {
                fill_rounded(self.canvas, rect.0, rect.1, rect.2, rect.3, radius, color(t.control));
            }
            if let Some([r_, g, b]) = tint {
                fill_rounded(self.canvas, rect.0 + 6.0, rect.1 + (rect.3 - 22.0) / 2.0, 22.0, 22.0, 6.0, color([*r_, *g, *b, 255]));
            }
            // Ícones "symbolic" (de uma cor só) na cor do texto (no Tahoe sem
            // quadradinho, na cor de destaque); na linha em destaque, brancos.
            let symbolic = icon.ends_with("-symbolic");
            // No quadradinho colorido (22), o símbolo de 16 fica com 3 de cada lado.
            let size = if symbolic { if tint.is_some() { 16 } else if tahoe { 17 } else { 18 } } else { 20 };
            let pix = self.ctx.icon(&[icon], size, false);
            if let Some(pix) = pix {
                // No quadradinho colorido (22 pontos), o símbolo fica no meio dele.
                let ix = if tint.is_some() { rect.0 + 6.0 + (22.0 - size as f32) / 2.0 } else if tahoe { rect.0 + 8.0 } else { rect.0 + 10.0 };
                // O quadradinho fica em `rect.1 + (rect.3 - 22) / 2`; o símbolo, no mesmo
                // centro (arredondado para baixo, como o quadradinho).
                let iy = if tint.is_some() { rect.1 + (rect.3 - 22.0) / 2.0 + (22.0 - size as f32) / 2.0 } else { rect.1 + (rect.3 - size as f32) / 2.0 };
                let (ix, iy) = (ix.floor() as i32, iy.floor() as i32);
                if symbolic {
                    let tint_c = if on_accent { t.on_accent } else if tahoe && tint.is_none() { t.accent } else if tint.is_some() { [255, 255, 255, 255] } else { t.text };
                    crate::ui::draw::blit_tinted(self.canvas, &pix, ix, iy, [tint_c[0], tint_c[1], tint_c[2]]);
                } else {
                    crate::ui::draw::blit(self.canvas, &pix, ix, iy, 1.0);
                }
            }
            let tc = if on_accent { t.on_accent } else { t.text };
            let tx = if tahoe { rect.0 + 34.0 } else { rect.0 + 42.0 };
            self.draw_text(name, tx, rect.1 + (rect.3 - t.font * 1.25) / 2.0, t.font, FontWeight::Regular, tc);
        }
        // Barra de rolagem fina quando a lista não cabe.
        if max_scroll > 0.0 {
            let view = list_bottom - list_top;
            let thumb = (view * view / total).max(24.0);
            let ty = list_top + (view - thumb) * (scroll / max_scroll);
            let tx = panel.0 + panel.2 - 7.0;
            fill_rounded(self.canvas, tx, ty, 4.0, thumb, 2.0, color(t.shade(if t.dark { 90 } else { 70 })));
        }
        // O vazio da barra lateral arrasta a janela (os itens têm prioridade).
        if tahoe {
            self.window_drag_area(panel);
        }
        // Conteúdo à direita.
        let (old_x, old_w) = (self.x, self.width);
        self.x = x + side_w;
        self.width = old_w - side_w;
        if raw {
            content(self, selected);
        } else {
            self.page(|ui| content(ui, selected));
            // Tahoe: a faixa de cima da página (onde fica o título) arrasta a janela.
            if tahoe {
                self.window_drag_area((x + side_w, y0, old_w - side_w, super::theme::TOOLBAR_HEIGHT));
            }
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

/// Estilos de botão do Tahoe (`Ui::button_styled`): todos em cápsula.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonStyle {
    /// Cheio na cor de destaque (ação principal).
    Prominent,
    /// Preenchimento suave (o botão comum).
    Bordered,
    /// Só o texto na cor de destaque; fundo ao passar o ponteiro.
    Plain,
    /// Cápsula de vidro (sobre fotos, barras de ferramentas).
    Glass,
    /// Cheio em vermelho (apagar).
    Destructive,
}

/// Cores das etiquetas do Finder (vermelho, laranja, amarelo, verde, azul, roxo, cinza).
pub const TAG_COLORS: [Rgba; 7] = [
    [255, 69, 58, 255],
    [255, 159, 10, 255],
    [255, 214, 10, 255],
    [48, 209, 88, 255],
    [10, 132, 255, 255],
    [191, 90, 242, 255],
    [142, 142, 147, 255],
];

/// Botão branco dos controles do Tahoe (interruptor, deslizante, segmentado): cápsula
/// com sombra curta e borda fina.
fn knob(canvas: &mut Pixmap, rect: (f32, f32, f32, f32)) {
    let (x, y, w, h) = rect;
    let r = h.min(w) / 2.0;
    fill_rounded(canvas, x - 0.5, y + 0.5, w + 1.0, h + 1.5, r + 0.5, color([0, 0, 0, 28]));
    fill_rounded(canvas, x, y, w, h, r, color([255, 255, 255, 255]));
    outline(canvas, rect, r, [0, 0, 0, 22]);
}

/// Contorno fino de 1 pt dentro do retângulo arredondado.
fn outline(canvas: &mut Pixmap, rect: (f32, f32, f32, f32), radius: f32, c: Rgba) {
    if let Some(path) = crate::ui::draw::rounded_rect(rect.0 + 0.5, rect.1 + 0.5, rect.2 - 1.0, rect.3 - 1.0, (radius - 0.5).max(0.0)) {
        let mut paint = Paint { anti_alias: true, ..Default::default() };
        paint.set_color(color(c));
        canvas.stroke_path(&path, &paint, &Stroke { width: 1.0, ..Default::default() }, crate::ui::draw::tf(), None);
    }
}

/// Cápsula de vidro do Tahoe (barra de ferramentas, busca): véu claro, borda de luz e
/// sombra curta. Dentro da janela o fundo é o próprio conteúdo do app.
fn capsule(canvas: &mut Pixmap, rect: (f32, f32, f32, f32), t: &Theme) {
    let (x, y, w, h) = rect;
    let r = h / 2.0;
    // Sombra curta (duas camadas finas, mais barato que desfocar).
    fill_rounded(canvas, x - 0.5, y + 1.0, w + 1.0, h + 1.0, r + 0.5, color(t.shade(if t.dark { 40 } else { 10 })));
    let fill = if t.dark { [58, 58, 62, 235] } else { [255, 255, 255, 235] };
    fill_rounded(canvas, x, y, w, h, r, color(fill));
    if let Some(path) = crate::ui::draw::rounded_rect(x + 0.5, y + 0.5, w - 1.0, h - 1.0, r - 0.5) {
        let mut paint = Paint { anti_alias: true, ..Default::default() };
        paint.set_color(color(if t.dark { [255, 255, 255, 30] } else { [0, 0, 0, 18] }));
        canvas.stroke_path(&path, &paint, &Stroke { width: 1.0, ..Default::default() }, crate::ui::draw::tf(), None);
    }
}

/// Pontilhado de `x0` a `x1` na altura `y` (o sublinhado dos erros): pontos redondos de
/// 2 pt a cada 3 pt, centrados no trecho, num caminho só (um preenchimento por trecho,
/// não um por ponto); um trecho curto ganha pelo menos um ponto.
fn dotted_line(pixmap: &mut Pixmap, x0: f32, x1: f32, y: f32, c: Rgba) {
    let step = 3.0;
    let n = (((x1 - x0) / step).floor().max(0.0) as usize) + 1;
    let first = x0 + ((x1 - x0) - (n - 1) as f32 * step) / 2.0;
    let mut pb = tiny_skia::PathBuilder::new();
    for i in 0..n {
        pb.push_circle(first + i as f32 * step, y, 1.0);
    }
    if let Some(path) = pb.finish() {
        let mut paint = Paint::default();
        paint.set_color(color(c));
        paint.anti_alias = true;
        pixmap.fill_path(&path, &paint, tiny_skia::FillRule::Winding, crate::ui::draw::tf(), None);
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
    fn area_de_texto_com_decoracoes() {
        use super::super::textarea::{index_in_line, line_starts, Decorations, TextDoc};
        let mut ctx = Ctx::new(Theme::default());
        let t = Theme::default();
        let size = t.font;
        let mut doc = TextDoc::new("2 + 3 =\nx".into());
        let mut seen: Vec<String> = Vec::new();
        let (green, blue) = ([0, 200, 0, 255], [0, 0, 255, 255]);
        // Fonte proporcional, sem números de linha: o "2" em verde, o "+" sublinhado e o
        // resultado "5" depois do fim da primeira linha.
        let view = |ui: &mut Ui, doc: &mut TextDoc, seen: &mut Vec<String>| {
            let mut decorate = |d: &TextDoc| {
                seen.push(d.text.clone());
                Decorations { colors: vec![(0..1, green)], underlines: std::iter::once(2..3).collect(), trailing: vec![(0, "5".into(), blue)] }
            };
            ui.text_area_with(doc, 200.0, TextAreaLook { line_numbers: false, sans: true, decorate: Some(&mut decorate), ..Default::default() })
        };
        let sans = |ctx: &mut Ctx, s: &str| ctx.text.measure(s, size, FontWeight::Regular);
        // A área começa em x = 8 (sem margem de números); a linha 0, em y = 6.
        let line_h = (size * 1.5).round();
        let line_y = 6.0 + line_h / 2.0;
        let line_w = sans(&mut ctx, "2 + 3 =");
        let (five_x, five_w) = (8.0 + line_w + 4.0, sans(&mut ctx, "5"));
        // Clique em cima do resultado: o cursor vai para o fim da linha (o resultado não
        // é texto); a posição sob o ponteiro vem no quadro do clique.
        let r = click(&mut ctx, five_x + five_w / 2.0, line_y, |ui| view(ui, &mut doc, &mut seen));
        assert_eq!(doc.edit.cursor, 7);
        assert_eq!(r.pointer_index, Some(7));
        // Só passando o ponteiro, sem clicar, ela não é medida.
        let hover = frame(&mut ctx, |ui| view(ui, &mut doc, &mut seen));
        assert_eq!(hover.pointer_index, None);
        // Clique no meio, num x em que a fonte proporcional e a fixa dariam posições
        // diferentes: vale a proporcional, que é a desenhada. Com uma fonte só no sistema
        // (as duas são a mesma face), não há o que distinguir.
        let same_face = ["iiii", "MMMM"].iter().all(|s| ctx.text.measure(s, size, FontWeight::Regular) == ctx.text.measure_mono(s, size));
        if same_face {
            eprintln!("aviso: a fonte proporcional e a fixa são a mesma face; clique no meio não conferido");
        } else {
            let starts = line_starts(&doc.text);
            let text = doc.text.clone();
            let x = (1..120)
                .map(|i| i as f32 / 2.0)
                .find(|x| {
                    index_in_line(&text, &starts, 0, *x, |s| ctx.text.measure(s, size, FontWeight::Regular)) != index_in_line(&text, &starts, 0, *x, |s| ctx.text.measure_mono(s, size))
                })
                .expect("um x em que as fontes dão posições diferentes");
            let expected = index_in_line(&text, &starts, 0, x, |s| ctx.text.measure(s, size, FontWeight::Regular));
            click(&mut ctx, 8.0 + x, line_y, |ui| view(ui, &mut doc, &mut seen));
            assert_eq!(doc.edit.cursor, expected);
        }
        // O decorate do mesmo quadro já vê o que foi digitado.
        doc.edit = super::super::textedit::Edit { cursor: 7, anchor: Some(7) };
        ctx.input.text = "4".into();
        frame(&mut ctx, |ui| view(ui, &mut doc, &mut seen));
        assert_eq!(doc.text, "2 + 3 =4\nx");
        assert_eq!(seen.last().map(String::as_str), Some("2 + 3 =4\nx"));
        // O desenho: o "2" verde, o pontilhado vermelho embaixo do "+" e o resultado
        // azul depois do fim da linha.
        let mut canvas = Pixmap::new(400, 300).unwrap();
        ctx.begin_frame();
        {
            let mut ui = Ui::root(&mut canvas, &mut ctx);
            view(&mut ui, &mut doc, &mut seen);
        }
        ctx.end_frame();
        let count = |x0: f32, x1: f32, y0: f32, y1: f32, pick: &dyn Fn([i32; 3]) -> bool| {
            let mut n = 0;
            for y in y0.floor() as u32..y1.ceil() as u32 {
                for x in x0.floor() as u32..x1.ceil() as u32 {
                    if let Some(p) = canvas.pixel(x, y) {
                        let c = p.demultiply();
                        n += usize::from(pick([c.red().into(), c.green().into(), c.blue().into()]));
                    }
                }
            }
            n
        };
        let text_y = (line_h - size * 1.25) / 2.0;
        let (top, base) = (6.0 + text_y, 6.0 + text_y + size * 1.1);
        let two_w = sans(&mut ctx, "2");
        assert!(count(8.0, 8.0 + two_w, top, base, &|[r, g, b]| g > r + 60 && g > b + 60) > 3, "o 2 em verde");
        let plus = (8.0 + sans(&mut ctx, "2 "), 8.0 + sans(&mut ctx, "2 +"));
        let dots = 6.0 + text_y + size * 1.2;
        assert!(count(plus.0, plus.1, dots - 1.5, dots + 1.5, &|[r, g, b]| r > g + 60 && r > b + 60) > 0, "o pontilhado vermelho");
        let end = 8.0 + sans(&mut ctx, "2 + 3 =4") + 4.0;
        assert!(count(end, end + five_w + 1.0, top, base, &|[r, g, b]| b > r + 60 && b > g + 60) > 3, "o resultado em azul");
    }

    #[test]
    fn texto_do_fim_da_linha_fica_a_vista() {
        use super::super::textarea::{Decorations, TextDoc};
        let mut ctx = Ctx::new(Theme::default());
        // Linha quase da largura da área, com o resultado depois: com o cursor no fim, a
        // área rola o bastante para o resultado aparecer inteiro, sem perder o cursor.
        let mut doc = TextDoc::new(format!("{} =", "1 + ".repeat(40)));
        doc.edit = super::super::textedit::Edit::at_end(&doc.text);
        let view = |ui: &mut Ui, doc: &mut TextDoc| {
            let mut decorate = |_: &TextDoc| Decorations { trailing: vec![(0, "1.234.567,89".into(), [0, 0, 255, 255])], ..Default::default() };
            ui.focus_next_input();
            ui.text_area_with(doc, 100.0, TextAreaLook { line_numbers: false, sans: true, decorate: Some(&mut decorate), ..Default::default() })
        };
        frame(&mut ctx, |ui| view(ui, &mut doc));
        let t = Theme::default();
        let line_w = ctx.text.measure(&doc.text, t.font, FontWeight::Regular);
        let result_w = ctx.text.measure("1.234.567,89", t.font, FontWeight::Regular);
        // A área tem 400 de largura: o recorte vai de 8 a 400 − 16 + 8.
        let clip_w = 400.0 - 16.0 + 8.0;
        assert!(doc.scroll_x > 0.0);
        assert!(line_w + 4.0 + result_w - doc.scroll_x <= clip_w - 4.0 + 0.5, "o resultado inteiro à vista");
        assert!(line_w - doc.scroll_x >= 0.0, "o cursor à vista");
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
