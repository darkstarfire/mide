//! Captura de tela do mide (tecla Print): as telas congelam e a Captura aparece por cima
//! de tudo, em todas elas (`Window::on_every_output`: uma camada por tela).
//!
//! - **Seleção** (padrão): arraste para escolher a área, em qualquer tela ou
//!   atravessando de uma para a outra.
//! - **Janela**: clique numa janela.
//! - **Tela inteira**: a tela toda; com mais de uma, a tela sob o ponteiro fica clara
//!   e o clique escolhe qual (Esc volta para a Seleção).
//!
//! Depois de escolher a área, ela se ajusta: com a ferramenta **Ajustar** (a padrão),
//! arrastar dentro da área muda o lugar dela, e as bolinhas (nas pontas e no meio dos
//! lados, na cor de destaque) mudam o tamanho. **Seta**, **Círculo** e **Quadrado**
//! desenham dentro da área na cor escolhida (vermelho, branco, preto, verde, amarelo ou
//! azul). **Copiar** (Enter ou Ctrl+C) põe a imagem na área de transferência;
//! **Salvar** (Ctrl+S) grava em Imagens/Capturas de tela; Esc cancela. O ⌘⇧4 (`--area`)
//! não tem bolinhas: ao soltar o mouse, salva e copia. O ⌘⇧3 (`--full`) salva a tela
//! inteira (com várias, um arquivo por tela) e copia a principal. O ⌘⇧5 (`--record`)
//! grava uma tela: a inteira (escolhida pelo clique, com várias) ou uma área, presa à
//! tela onde o arrasto começou.
//!
//! A imagem vem do compositor (`screenshot all`: uma por tela, em pixels, sem o
//! cursor); a cópia fica com o compositor (`copy-file`), então continua lá depois que
//! a captura fecha.
//!
//! Coordenadas: a seleção, as marcações e as janelas ficam em **pontos no espaço do
//! desktop** (todas as telas). Cada tela guarda a imagem em pixels (`Shot::shot`, para
//! o arquivo) e uma cópia no tamanho em pontos (`view`, `dimmed`, para mostrar na
//! camada, que desenha em escala 1). O arquivo sai na maior escala entre as telas que
//! a área toca (`compose`). O `view` roda uma vez por tela: o desenho vale para todas
//! (cada uma mostra a sua parte, deslocada pela origem dela); teclas, ponteiro e
//! painel só no passe da camada principal (`!ui.mirror()`).

use std::{error::Error, path::PathBuf, time::Duration};

use mide::{
    control_client::ask_compositor,
    kit::{run, App, Key, Ui, Window},
};
use tiny_skia::{Color, FillRule, FilterQuality, LineCap, Paint, PathBuilder, Pixmap, PixmapPaint, Rect, Stroke, Transform};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Selection,
    Window,
    /// A tela inteira: a escolhida no `PickScreen` (com várias telas) ou a principal.
    Full,
    /// Várias telas e "Tela inteira": a tela sob o ponteiro fica clara, com o rótulo em
    /// vidro; o clique escolhe (vira `Full`), Esc volta para a Seleção.
    PickScreen,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tool {
    /// Arrastar dentro da área move a área (a padrão); as bolinhas mudam o tamanho.
    Adjust,
    Arrow,
    Circle,
    Square,
}

/// Cores para desenhar: nome e RGB.
const COLORS: [(&str, [u8; 3]); 6] = [
    ("vermelho", [235, 52, 52]),
    ("branco", [255, 255, 255]),
    ("preto", [20, 20, 22]),
    ("verde", [52, 199, 89]),
    ("amarelo", [255, 204, 0]),
    ("azul", [10, 132, 255]),
];

/// Uma marcação desenhada sobre a captura (coordenadas da tela).
#[derive(Debug, Clone, Copy)]
struct Shape {
    tool: Tool,
    color: [u8; 3],
    from: (f32, f32),
    to: (f32, f32),
}

/// Retângulo (x, y, largura, altura).
type RectF = (f32, f32, f32, f32);

/// Bolinha da área escolhida: as 4 pontas e o meio dos 4 lados.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Handle {
    TopLeft,
    TopRight,
    BottomRight,
    BottomLeft,
    Top,
    Right,
    Bottom,
    Left,
}

impl Handle {
    /// As pontas antes dos meios dos lados: numa área pequena, onde as duas ficam a
    /// menos de um raio, a ponta vence (muda largura e altura juntas).
    const ALL: [Handle; 8] = [Handle::TopLeft, Handle::TopRight, Handle::BottomRight, Handle::BottomLeft, Handle::Top, Handle::Right, Handle::Bottom, Handle::Left];

    /// Que lados a bolinha mexe: x (−1 esquerdo, 1 direito, 0 nenhum) e y (−1 de cima,
    /// 1 de baixo, 0 nenhum).
    fn sides(self) -> (i8, i8) {
        match self {
            Handle::TopLeft => (-1, -1),
            Handle::TopRight => (1, -1),
            Handle::BottomRight => (1, 1),
            Handle::BottomLeft => (-1, 1),
            Handle::Top => (0, -1),
            Handle::Right => (1, 0),
            Handle::Bottom => (0, 1),
            Handle::Left => (-1, 0),
        }
    }
}

/// Raio da bolinha desenhada e da área onde o clique pega nela (pontos).
const HANDLE_RADIUS: f32 = 5.0;
const HANDLE_HIT: f32 = 10.0;

/// Centro de cada bolinha da área `r`.
fn handles(r: RectF) -> [(Handle, (f32, f32)); 8] {
    Handle::ALL.map(|h| {
        let (sx, sy) = h.sides();
        let x = r.0 + r.2 * (sx as f32 + 1.0) / 2.0;
        let y = r.1 + r.3 * (sy as f32 + 1.0) / 2.0;
        (h, (x, y))
    })
}

/// A bolinha sob o ponto `p` (até `HANDLE_HIT` do centro), se houver.
fn handle_at(r: RectF, p: (f32, f32)) -> Option<Handle> {
    handles(r).into_iter().find(|(_, c)| (p.0 - c.0).hypot(p.1 - c.1) <= HANDLE_HIT).map(|(h, _)| h)
}

/// A área `orig` com a bolinha `h` arrastada de `delta`: o lado oposto fica parado, o
/// lado mexido não sai de `limits` (todas as telas), passar do outro lado vira a área
/// (normaliza) e o tamanho mínimo é 2 pontos.
fn resize_area(orig: RectF, h: Handle, delta: (f32, f32), limits: RectF) -> RectF {
    let (lx0, ly0, lx1, ly1) = (limits.0, limits.1, limits.0 + limits.2, limits.1 + limits.3);
    let (mut x0, mut y0, mut x1, mut y1) = (orig.0, orig.1, orig.0 + orig.2, orig.1 + orig.3);
    let (sx, sy) = h.sides();
    match sx {
        -1 => x0 = (x0 + delta.0).clamp(lx0, lx1),
        1 => x1 = (x1 + delta.0).clamp(lx0, lx1),
        _ => {}
    }
    match sy {
        -1 => y0 = (y0 + delta.1).clamp(ly0, ly1),
        1 => y1 = (y1 + delta.1).clamp(ly0, ly1),
        _ => {}
    }
    let fit = |a: f32, b: f32, lo: f32, hi: f32| {
        let (start, len) = (a.min(b), (a - b).abs());
        if len >= 2.0 { (start, len) } else { (start.min(hi - 2.0).max(lo), 2.0) }
    };
    let (x, w) = fit(x0, x1, lx0, lx1);
    let (y, h) = fit(y0, y1, ly0, ly1);
    (x, y, w, h)
}

/// A área `orig` arrastada de `delta`, sem sair de `limits` (o tamanho não muda).
fn moved(orig: RectF, delta: (f32, f32), limits: RectF) -> RectF {
    let x = (orig.0 + delta.0).min(limits.0 + limits.2 - orig.2).max(limits.0);
    let y = (orig.1 + delta.1).min(limits.1 + limits.3 - orig.3).max(limits.1);
    (x, y, orig.2, orig.3)
}

/// O ponto `p` preso dentro de `limits`.
fn clamp_point(p: (f32, f32), limits: RectF) -> (f32, f32) {
    (p.0.clamp(limits.0, limits.0 + limits.2), p.1.clamp(limits.1, limits.1 + limits.3))
}

/// Área `r` com todas as bolinhas numa tela (alcançáveis pelo ponteiro): nenhuma cai
/// no buraco entre telas de tamanhos diferentes.
fn handles_on_screens(r: RectF, screens: &[RectF]) -> bool {
    handles(r).iter().all(|(_, c)| screens.iter().any(|s| inside(*s, *c)))
}

/// Quanto da área `r` fica em alguma tela (as telas não se sobrepõem).
fn coverage(r: RectF, screens: &[RectF]) -> f32 {
    screens
        .iter()
        .map(|s| {
            let w = (r.0 + r.2).min(s.0 + s.2) - r.0.max(s.0);
            let h = (r.1 + r.3).min(s.1 + s.3) - r.1.max(s.1);
            w.max(0.0) * h.max(0.0)
        })
        .sum()
}

/// Das posições `cands`, a que tem as bolinhas nas telas e fica mais perto de `want`
/// (onde o arrasto levaria sem limite); se nenhuma tem, a que deixa mais área nas telas.
fn closest_valid(cands: &[RectF], want: RectF, screens: &[RectF]) -> RectF {
    let dist = |r: &RectF| (r.0 - want.0).abs() + (r.1 - want.1).abs() + (r.2 - want.2).abs() + (r.3 - want.3).abs();
    let valid = cands.iter().filter(|r| handles_on_screens(**r, screens)).min_by(|a, b| dist(a).total_cmp(&dist(b)));
    valid.copied().unwrap_or_else(|| {
        cands.iter().copied().max_by(|a, b| coverage(*a, screens).total_cmp(&coverage(*b, screens)).then(dist(b).total_cmp(&dist(a)))).unwrap_or(want)
    })
}

/// `p` está na tela `r`, contando a borda de cima e a da esquerda, mas não a da direita
/// nem a de baixo: numa borda comum entre telas (x = 1366 com A em 0..1366 e B começando
/// em 1366), o ponto é da tela que começa ali. Só para escolher a tela; a seleção e as
/// janelas usam `inside`.
fn contains_half_open(r: RectF, p: (f32, f32)) -> bool {
    p.0 >= r.0 && p.0 < r.0 + r.2 && p.1 >= r.1 && p.1 < r.1 + r.3
}

/// A tela de `screens` que contém `p` (nas bordas comuns, a que começa ali).
fn screen_index_at(screens: &[RectF], p: (f32, f32)) -> Option<usize> {
    screens.iter().position(|s| contains_half_open(*s, p))
}

/// A tela de `screens` mais perto de `p` (a que contém, se houver; no buraco entre telas,
/// a mais próxima). Sem telas, 0.
fn screen_index_near(screens: &[RectF], p: (f32, f32)) -> usize {
    if let Some(i) = screen_index_at(screens, p) {
        return i;
    }
    let dist = |s: &RectF| {
        let c = clamp_point(p, *s);
        (c.0 - p.0).hypot(c.1 - p.1)
    };
    (0..screens.len()).min_by(|a, b| dist(&screens[*a]).total_cmp(&dist(&screens[*b]))).unwrap_or(0)
}

/// Retângulo que contém as telas `screens` (sem telas, um ponto).
fn bounds_of(screens: &[RectF]) -> RectF {
    let Some(first) = screens.first() else {
        return (0.0, 0.0, 1.0, 1.0);
    };
    let (mut x0, mut y0, mut x1, mut y1) = (first.0, first.1, first.0 + first.2, first.1 + first.3);
    for s in &screens[1..] {
        x0 = x0.min(s.0);
        y0 = y0.min(s.1);
        x1 = x1.max(s.0 + s.2);
        y1 = y1.max(s.1 + s.3);
    }
    (x0, y0, x1 - x0, y1 - y0)
}

/// Faixa comum às telas que `r` atravessa: na horizontal (`horizontal`), as telas que
/// cruzam as colunas de `r` limitam a altura (o topo mais baixo e o fim mais alto); na
/// vertical, o contrário. Como duas telas lado a lado com alturas diferentes. Volta o
/// retângulo-limite, ou `None` se não há faixa comum.
fn band_limits(r: RectF, screens: &[RectF], horizontal: bool) -> Option<RectF> {
    let all = bounds_of(screens);
    let over: Vec<&RectF> = screens.iter().filter(|s| if horizontal { r.0 < s.0 + s.2 && s.0 < r.0 + r.2 } else { r.1 < s.1 + s.3 && s.1 < r.1 + r.3 }).collect();
    let lo = over.iter().map(|s| if horizontal { s.1 } else { s.0 }).fold(f32::MIN, f32::max);
    let hi = over.iter().map(|s| if horizontal { s.1 + s.3 } else { s.0 + s.2 }).fold(f32::MAX, f32::min);
    if over.is_empty() || hi <= lo {
        return None;
    }
    Some(if horizontal { (all.0, lo, all.2, hi - lo) } else { (lo, all.1, hi - lo, all.3) })
}

/// `moved` limitado à união das telas, não ao retângulo que as envolve: a área não vai
/// para o buraco entre telas de alturas (ou larguras) diferentes. Tenta onde o arrasto
/// leva, deslizar só num eixo, caber numa tela só e caber na faixa comum às telas que
/// ela atravessa; fica a posição válida mais perto.
fn moved_in(orig: RectF, delta: (f32, f32), screens: &[RectF]) -> RectF {
    let all = bounds_of(screens);
    let want = moved(orig, delta, all);
    let mut cands = vec![want, moved(orig, (delta.0, 0.0), all), moved(orig, (0.0, delta.1), all), orig];
    for s in screens.iter().filter(|s| orig.2 <= s.2 && orig.3 <= s.3) {
        cands.push(moved(orig, delta, *s));
    }
    for band in [band_limits(want, screens, true), band_limits(want, screens, false)].into_iter().flatten() {
        cands.push(moved(want, (0.0, 0.0), band));
    }
    closest_valid(&cands, want, screens)
}

/// O ponto da união das telas mais perto de `p`.
fn nearest_on_screens(p: (f32, f32), screens: &[RectF]) -> (f32, f32) {
    screens.iter().map(|s| clamp_point(p, *s)).min_by(|a, b| (a.0 - p.0).hypot(a.1 - p.1).total_cmp(&(b.0 - p.0).hypot(b.1 - p.1))).unwrap_or(p)
}

/// `resize_area` limitado à união das telas: a bolinha arrastada vai para o ponto de
/// tela mais perto do ponteiro e a área fica com todas as bolinhas alcançáveis (senão,
/// desliza num eixo só, ou não muda).
fn resize_in(orig: RectF, h: Handle, delta: (f32, f32), screens: &[RectF]) -> RectF {
    let all = bounds_of(screens);
    let want = resize_area(orig, h, delta, all);
    let from = handles(orig).iter().find(|(k, _)| *k == h).map_or((orig.0, orig.1), |(_, c)| *c);
    let to = nearest_on_screens((from.0 + delta.0, from.1 + delta.1), screens);
    let d = (to.0 - from.0, to.1 - from.1);
    let mut cands = vec![want, resize_area(orig, h, d, all), resize_area(orig, h, (d.0, 0.0), all), resize_area(orig, h, (0.0, d.1), all), orig];
    // Presa a uma tela só ou à faixa comum das telas que a área atravessa.
    let bands = [band_limits(want, screens, true), band_limits(want, screens, false)];
    for limits in screens.iter().copied().map(Some).chain(bands).flatten() {
        cands.push(resize_area(orig, h, delta, limits));
    }
    closest_valid(&cands, want, screens)
}

/// Arrasto em andamento (pontos no desktop).
#[derive(Debug, Clone, Copy, PartialEq)]
enum Drag {
    /// Escolhendo uma área nova, a partir de `start`.
    NewArea { start: (f32, f32) },
    /// Desenhando uma marcação (seta, círculo ou quadrado).
    Draw { start: (f32, f32) },
    /// Mudando o tamanho da área `orig` pela bolinha `handle`.
    Resize { handle: Handle, orig: RectF, start: (f32, f32) },
    /// Mudando a área `orig` de lugar.
    Move { orig: RectF, start: (f32, f32) },
}

/// Retângulo (x, y, largura, altura) com largura e altura positivas.
fn normalized(a: (f32, f32), b: (f32, f32)) -> (f32, f32, f32, f32) {
    (a.0.min(b.0), a.1.min(b.1), (a.0 - b.0).abs(), (a.1 - b.1).abs())
}

fn inside(r: (f32, f32, f32, f32), p: (f32, f32)) -> bool {
    p.0 >= r.0 && p.0 <= r.0 + r.2 && p.1 >= r.1 && p.1 <= r.1 + r.3
}

/// Uma tela, como o compositor respondeu ao `screenshot all`.
#[derive(Debug, Clone, PartialEq)]
struct ScreenLine {
    name: String,
    /// Posição e tamanho em pontos, no espaço do desktop.
    rect: RectF,
    scale: f32,
    primary: bool,
    path: PathBuf,
}

/// Lê a resposta do `screenshot all` (`NOME X Y LARGURA ALTURA ESCALA principal|-
/// CAMINHO`, separados por tabulação). Linhas que não casam ficam de fora.
fn parse_screens(reply: &[String]) -> Vec<ScreenLine> {
    reply
        .iter()
        .filter_map(|line| {
            let f: Vec<&str> = line.splitn(8, '\t').collect();
            let num = |i: usize| f.get(i)?.parse::<f32>().ok();
            Some(ScreenLine {
                name: f.first()?.to_string(),
                rect: (num(1)?, num(2)?, num(3)?.max(1.0), num(4)?.max(1.0)),
                scale: num(5).filter(|s| *s > 0.0)?,
                primary: *f.get(6)? == "principal",
                path: PathBuf::from(f.get(7)?),
            })
        })
        .collect()
}

/// Uma tela capturada.
struct Shot {
    name: String,
    /// Onde a tela fica, em pontos no espaço do desktop.
    rect: RectF,
    scale: f32,
    primary: bool,
    /// A imagem em pixels da tela (o que vai para o arquivo).
    shot: Pixmap,
    /// A imagem no tamanho em pontos (o que a camada mostra). Igual a `shot` em escala 1.
    view: Pixmap,
    /// A `view` já escurecida (fora da área escolhida), feita uma vez: cada quadro só
    /// copia bytes, sem misturar a tela inteira de novo.
    dimmed: Pixmap,
}

impl Shot {
    fn new(name: &str, rect: RectF, scale: f32, primary: bool, shot: Pixmap) -> Self {
        let (w, h) = (rect.2.round().max(1.0) as u32, rect.3.round().max(1.0) as u32);
        let view = if (shot.width(), shot.height()) == (w, h) { shot.clone() } else { resized(&shot, w, h) };
        let mut dimmed = view.clone();
        for px in dimmed.data_mut().chunks_mut(4) {
            for c in &mut px[..3] {
                *c = (*c as u16 * 145 / 255) as u8;
            }
        }
        Self { name: name.into(), rect, scale, primary, shot, view, dimmed }
    }
}

/// `src` ampliada ou reduzida para `w`×`h` (filtro bilinear).
fn resized(src: &Pixmap, w: u32, h: u32) -> Pixmap {
    let Some(mut out) = Pixmap::new(w, h) else {
        return src.clone();
    };
    let (kx, ky) = (w as f32 / src.width() as f32, h as f32 / src.height() as f32);
    let paint = PixmapPaint { quality: FilterQuality::Bilinear, ..Default::default() };
    out.draw_pixmap(0, 0, src.as_ref(), &paint, Transform::from_scale(kx, ky), None);
    out
}

fn intersects(a: RectF, b: RectF) -> bool {
    a.0 < b.0 + b.2 && b.0 < a.0 + a.2 && a.1 < b.1 + b.3 && b.1 < a.1 + a.3
}

/// Escala do arquivo de uma área: a maior entre as telas que ela toca (nenhum pedaço
/// perde resolução; os de escala menor são ampliados).
fn output_scale(sel: RectF, screens: &[Shot]) -> f32 {
    let k = screens.iter().filter(|s| intersects(sel, s.rect)).map(|s| s.scale).fold(0.0, f32::max);
    if k > 0.0 { k } else { 1.0 }
}

/// A área `sel` (pontos no desktop) com as marcações, em pixels: cada tela que ela toca
/// entra na escala do arquivo (`output_scale`), e o que não é de nenhuma tela (o canto
/// entre telas de alturas diferentes) sai preto.
fn compose(screens: &[Shot], sel: RectF, shapes: &[Shape]) -> Option<Pixmap> {
    let k = output_scale(sel, screens);
    let (w, h) = output_size(screens, sel, k);
    let mut out = Pixmap::new(w, h)?;
    out.fill(Color::BLACK);
    for s in screens.iter().filter(|s| intersects(sel, s.rect)) {
        let f = k / s.scale;
        let (tx, ty) = ((s.rect.0 - sel.0) * k, (s.rect.1 - sel.1) * k);
        // Sem mudar de tamanho, cópia exata dos pixels; senão, filtro bilinear.
        let (quality, tx, ty) = if (f - 1.0).abs() < 1e-6 { (FilterQuality::Nearest, tx.round(), ty.round()) } else { (FilterQuality::Bilinear, tx, ty) };
        let paint = PixmapPaint { quality, ..Default::default() };
        out.draw_pixmap(0, 0, s.shot.as_ref(), &paint, Transform::from_row(f, 0.0, 0.0, f, tx, ty), None);
    }
    let ts = Transform::from_row(k, 0.0, 0.0, k, -sel.0 * k, -sel.1 * k);
    for shape in shapes {
        draw_shape(&mut out, shape, ts);
    }
    Some(out)
}

/// Tamanho em pixels do arquivo da área `sel` na escala `k`. O compositor arredonda o
/// tamanho da tela em pontos para cima (1366x768 em 1,25 vira 1093x615 pontos, que
/// seriam 769 pixels de altura): a área não passa do fim das imagens das telas que ela
/// toca, senão o arquivo ganha uma linha ou coluna preta.
fn output_size(screens: &[Shot], sel: RectF, k: f32) -> (u32, u32) {
    let (mut right, mut bottom) = (f32::MIN, f32::MIN);
    for s in screens.iter().filter(|s| intersects(sel, s.rect)) {
        let f = k / s.scale;
        right = right.max((s.rect.0 - sel.0) * k + s.shot.width() as f32 * f);
        bottom = bottom.max((s.rect.1 - sel.1) * k + s.shot.height() as f32 * f);
    }
    let mut w = (sel.2 * k).round();
    let mut h = (sel.3 * k).round();
    if right > f32::MIN {
        w = w.min(right.round());
        h = h.min(bottom.round());
    }
    (w.max(1.0) as u32, h.max(1.0) as u32)
}

/// Onde gravar uma área escolhida no ⌘⇧5: a tela `screen` (onde o arrasto começou) e a
/// área em pontos dessa tela (`record start x y w h tela NOME`), recortada a ela: a
/// gravação é de uma tela só.
fn record_target(screens: &[Shot], sel: RectF, screen: usize) -> (String, (i32, i32, i32, i32)) {
    let Some(s) = screens.get(screen) else {
        return (String::new(), (sel.0.round() as i32, sel.1.round() as i32, sel.2.round() as i32, sel.3.round() as i32));
    };
    let r = clamp_rect(sel, s.rect);
    let local = ((r.0 - s.rect.0).round() as i32, (r.1 - s.rect.1).round() as i32, r.2.round() as i32, r.3.round() as i32);
    (s.name.clone(), local)
}

/// O pedido ao compositor: `record start full|x y w h [tela NOME] [sistema] [microfone]`.
/// Sem nome (uma tela só ou compositor antigo), sem `tela`: vale a principal.
fn record_command(area: Option<(i32, i32, i32, i32)>, screen: Option<&str>, system_audio: bool, microphone: bool) -> String {
    let mut cmd = match area {
        Some((x, y, w, h)) => format!("record start {x} {y} {w} {h}"),
        None => "record start full".to_string(),
    };
    if let Some(name) = screen.filter(|n| !n.is_empty()) {
        cmd.push_str(&format!(" tela {name}"));
    }
    if system_audio {
        cmd.push_str(" sistema");
    }
    if microphone {
        cmd.push_str(" microfone");
    }
    cmd
}

/// Nome do arquivo de uma captura: `Captura de tela 2026-10-20 21-13-38.png`; com várias
/// telas no ⌘⇧3, o nome da tela entre parênteses (`... 21-13-38 (HDMI-A-1).png`).
fn shot_file_name(stamp: &str, screen: Option<&str>) -> String {
    match screen.filter(|n| !n.is_empty()) {
        Some(name) => format!("Captura de tela {stamp} ({name}).png"),
        None => format!("Captura de tela {stamp}.png"),
    }
}

struct Capture {
    /// As telas capturadas; a principal (onde a camada fica) está entre elas.
    screens: Vec<Shot>,
    mode: Mode,
    tool: Tool,
    color: usize,
    /// Área escolhida (pontos no desktop).
    selection: Option<(f32, f32, f32, f32)>,
    /// Arrasto em andamento.
    drag: Option<Drag>,
    shapes: Vec<Shape>,
    /// Janelas à mostra (a de cima primeiro, pontos no desktop), para o modo Janela.
    windows: Vec<(f32, f32, f32, f32)>,
    /// Retângulo das cores no painel (para o clique nelas).
    swatches: Vec<(f32, f32, f32, f32)>,
    /// Modo rápido (⌘⇧4, `--area`): sem painel; ao soltar o mouse, a área é salva e
    /// copiada, como no Mac.
    quick: bool,
    /// Gravar a tela (⌘⇧5, `--record`): tela inteira ou área, com som do sistema e
    /// microfone; "Gravar" pede ao compositor e fecha.
    record: bool,
    system_audio: bool,
    microphone: bool,
    /// Altura do painel no último quadro (0 antes do primeiro) e a altura escolhida
    /// para ele (`place_panel`), que fica parada durante um arrasto.
    panel_h: f32,
    panel_y: Option<f32>,
    /// A tela escolhida no `PickScreen` (índice em `screens`); sem ela, `Full` é a
    /// principal.
    full_screen: Option<usize>,
    /// Na gravação, a tela onde a área começou (a área fica presa a ela).
    record_screen: Option<usize>,
}

impl Capture {
    fn new(screens: Vec<Shot>, windows: Vec<(f32, f32, f32, f32)>) -> Self {
        let flag = |key: &str, default: bool| mide::settings_file::flag("record", key, default);
        Self {
            screens,
            mode: Mode::Selection,
            tool: Tool::Adjust,
            color: 0,
            selection: None,
            drag: None,
            shapes: Vec::new(),
            windows,
            swatches: Vec::new(),
            quick: false,
            record: false,
            system_audio: flag("system_audio", true),
            microphone: flag("microphone", false),
            panel_h: 0.0,
            panel_y: None,
            full_screen: None,
            record_screen: None,
        }
    }

    /// A tela onde a camada fica (a principal).
    fn primary(&self) -> &Shot {
        &self.screens[self.primary_index()]
    }

    fn primary_index(&self) -> usize {
        self.screens.iter().position(|s| s.primary).unwrap_or(0)
    }

    /// Posição e tamanho de cada tela (pontos no desktop).
    fn rects(&self) -> Vec<RectF> {
        self.screens.iter().map(|s| s.rect).collect()
    }

    /// A tela inteira do modo `Full`: a escolhida ou a principal.
    fn full_index(&self) -> usize {
        self.full_screen.filter(|i| *i < self.screens.len()).unwrap_or_else(|| self.primary_index())
    }

    /// "Tela inteira": com uma tela, ela; com várias, escolher qual (`PickScreen`).
    fn choose_full(&mut self) {
        self.shapes.clear();
        self.full_screen = None;
        if self.screens.len() > 1 {
            self.mode = Mode::PickScreen;
            self.selection = None;
        } else {
            self.mode = Mode::Full;
        }
    }

    /// Escolhe a tela `i` no `PickScreen`: a área vira a tela toda.
    fn pick_screen(&mut self, i: usize) {
        self.full_screen = Some(i);
        self.mode = Mode::Full;
        self.selection = Some(self.screens[i].rect);
        self.shapes.clear();
        tracing::info!("captura: tela {} escolhida", self.screens[i].name);
    }

    /// A área escolhida com as marcações, em pixels.
    fn result(&self) -> Option<Pixmap> {
        compose(&self.screens, self.selection?, &self.shapes)
    }

    fn copy(&self) -> bool {
        let Some(image) = self.result() else {
            return false;
        };
        let path = runtime_dir().join(format!("mide-captura-{}.png", std::process::id()));
        if let Err(err) = image.save_png(&path) {
            tracing::warn!("captura: não foi possível preparar a cópia: {err}");
            mide::notify::send_and_wait("Captura de tela", "A captura não foi copiada", &err.to_string());
            return false;
        }
        let ok = ask_compositor(&format!("copy-file image/png {}", path.display())).is_some_and(|r| r.first().map(String::as_str) == Some("ok"));
        let _ = std::fs::remove_file(&path);
        if ok {
            tracing::info!("captura copiada ({}x{})", image.width(), image.height());
            shutter();
            mide::notify::send_and_wait("Captura de tela", "Captura copiada", "Cole com Ctrl+V (⌘V no estilo Mac).");
        } else {
            mide::notify::send_and_wait("Captura de tela", "A captura não foi copiada", "O mide não respondeu.");
        }
        ok
    }

    /// Grava em Imagens/Capturas de tela, com o nome "Captura de tela" mais a data e a
    /// hora (`Captura de tela 2026-10-20 21-13-38.png`). Avisa onde ficou (ou o erro).
    fn save(&self) -> Option<PathBuf> {
        let image = self.result()?;
        save_images(&[(&image, None)]).into_iter().next()
    }

    /// ⌘⇧3: a tela inteira, salva e copiada. Com várias telas, um arquivo por tela (com
    /// o nome dela) e a principal na área de transferência. Verdadeiro se salvou ou
    /// copiou alguma coisa.
    fn save_full(&mut self) -> bool {
        let main = self.primary().rect;
        self.selection = Some(main);
        tracing::info!("captura rápida da tela inteira ({} tela(s), principal {}x{} pontos)", self.screens.len(), main.2, main.3);
        let copied = self.copy();
        let saved = if self.screens.len() > 1 {
            // A imagem de cada tela já é o arquivo (em pixels, sem marcações).
            let images: Vec<(&Pixmap, Option<&str>)> = self.screens.iter().map(|s| (&s.shot, Some(s.name.as_str()))).collect();
            !save_images(&images).is_empty()
        } else {
            self.save().is_some()
        };
        saved || copied
    }

    /// Retângulo que envolve as telas onde a área pode ficar (`areas`).
    fn limits(&self) -> RectF {
        bounds_of(&self.areas())
    }

    /// As telas onde a área pode ficar (a união delas, não o retângulo que as
    /// envolve): todas; na gravação, só a principal.
    fn areas(&self) -> Vec<RectF> {
        if self.record {
            let i = self.record_screen.filter(|i| *i < self.screens.len()).unwrap_or_else(|| self.primary_index());
            vec![self.screens[i].rect]
        } else {
            self.rects()
        }
    }

    /// As bolinhas aparecem (e pegam o clique) com uma área escolhida no modo Seleção,
    /// fora do ⌘⇧4 e sem uma área nova ou uma marcação sendo arrastada.
    fn handles_visible(&self) -> bool {
        !self.quick && self.mode == Mode::Selection && self.selection.is_some() && !matches!(self.drag, Some(Drag::NewArea { .. } | Drag::Draw { .. }))
    }

    /// Registra a área escolhida (para a bateria e para quem lê o log).
    fn log_area(&self) {
        if let Some(sel) = self.selection {
            let (pw, ph) = output_size(&self.screens, sel, output_scale(sel, &self.screens));
            tracing::info!("captura: área {}x{} em ({}, {}) [{pw}x{ph} px]", sel.2.round() as i32, sel.3.round() as i32, sel.0.round() as i32, sel.1.round() as i32);
        }
    }

    /// Clique e arrasto (`pos` em pontos no desktop, em qualquer tela; fora do painel).
    fn pointer(&mut self, ui: &Ui, pos: (f32, f32), over_panel: bool) {
        let input = ui.input();
        // Sobre o painel, o clique é dele; menos numa bolinha (o painel sai de cima
        // delas, mas numa área que não deixa lugar livre a bolinha ainda vence).
        let on_handle = self.selection.filter(|_| self.handles_visible()).and_then(|s| handle_at(s, pos)).is_some();
        if over_panel && self.drag.is_none() && !on_handle {
            return;
        }
        let mut limits = self.limits();
        match self.mode {
            Mode::PickScreen => {
                // O clique (ao soltar) escolhe a tela sob o ponteiro (no buraco entre
                // telas, nada).
                if input.released
                    && let Some(i) = screen_index_at(&self.rects(), pos)
                {
                    self.pick_screen(i);
                }
            }
            Mode::Window => {
                if input.released {
                    if let Some(rect) = self.windows.iter().find(|r| inside(**r, pos)) {
                        self.selection = Some(clamp_rect(*rect, limits));
                        self.mode = Mode::Selection;
                        self.shapes.clear();
                        self.log_area();
                    }
                }
            }
            Mode::Full | Mode::Selection => {
                if input.pressed {
                    // Na gravação não há ferramentas de desenho: dentro da área, move.
                    let tool = if self.record { Tool::Adjust } else { self.tool };
                    let handle = self.selection.filter(|_| self.handles_visible()).and_then(|s| handle_at(s, pos));
                    let within = self.selection.filter(|s| inside(*s, pos));
                    self.drag = match (handle, within) {
                        _ if self.quick => None,
                        (Some(handle), _) => self.selection.map(|orig| Drag::Resize { handle, orig, start: pos }),
                        (None, Some(orig)) if tool == Tool::Adjust => (self.mode == Mode::Selection).then_some(Drag::Move { orig, start: pos }),
                        (None, Some(_)) => Some(Drag::Draw { start: pos }),
                        (None, None) => None,
                    };
                    if self.drag.is_none() && (self.quick || within.is_none()) {
                        // Fora da área escolhida: outra área (as marcações saem).
                        self.shapes.clear();
                        self.mode = Mode::Selection;
                        if self.record {
                            // A gravação é de uma tela: a área fica presa à tela onde
                            // começou.
                            self.record_screen = Some(screen_index_near(&self.rects(), pos));
                            limits = self.limits();
                        }
                        self.drag = Some(Drag::NewArea { start: clamp_point(pos, limits) });
                    }
                }
                let delta = |start: (f32, f32)| (pos.0 - start.0, pos.1 - start.1);
                match self.drag {
                    Some(Drag::NewArea { start }) => {
                        let r = normalized(start, clamp_point(pos, limits));
                        self.selection = (r.2 >= 2.0 && r.3 >= 2.0).then_some(r);
                    }
                    Some(Drag::Resize { handle, orig, start }) => self.selection = Some(resize_in(orig, handle, delta(start), &self.areas())),
                    Some(Drag::Move { orig, start }) => self.selection = Some(moved_in(orig, delta(start), &self.areas())),
                    Some(Drag::Draw { .. }) | None => {}
                }
                if input.released {
                    match self.drag.take() {
                        Some(Drag::Draw { start }) if (pos.0 - start.0).hypot(pos.1 - start.1) >= 4.0 => {
                            self.shapes.push(Shape { tool: self.tool, color: COLORS[self.color].1, from: start, to: pos });
                        }
                        Some(Drag::NewArea { .. } | Drag::Resize { .. } | Drag::Move { .. }) => self.log_area(),
                        _ => {}
                    }
                }
            }
        }
    }
}

/// `r` recortado aos limites `b` (todas as telas).
fn clamp_rect(r: RectF, b: RectF) -> RectF {
    let x = r.0.max(b.0);
    let y = r.1.max(b.1);
    let right = (r.0 + r.2).min(b.0 + b.2);
    let bottom = (r.1 + r.3).min(b.1 + b.3);
    (x, y, (right - x).max(1.0), (bottom - y).max(1.0))
}

impl App for Capture {
    fn view(&mut self, ui: &mut Ui) {
        // O `view` roda uma vez por tela (`on_every_output`): `scr` é a tela deste passe.
        // O desenho vale para todas; o resto (teclas, ponteiro, painel) só no passe da
        // camada principal, para o estado mudar uma vez por quadro.
        let scr = ui.screen();
        let mirror = ui.mirror();
        if !mirror {
            if self.mode == Mode::Full {
                self.selection = Some(self.screens[self.full_index()].rect);
            }
            // Teclado: Esc cancela (escolhendo a tela, volta para a Seleção), Enter e
            // Ctrl+C copiam, Ctrl+S salva.
            let keys = ui.input().keys.clone();
            let shortcuts = ui.input().shortcuts.clone();
            if keys.contains(&Key::Escape) && self.mode == Mode::PickScreen && !self.record {
                tracing::info!("captura: escolha da tela cancelada");
                self.mode = Mode::Selection;
                self.selection = None;
            } else if keys.contains(&Key::Escape) {
                tracing::info!("captura cancelada");
                ui.close();
                return;
            }
            if (keys.contains(&Key::Enter) || shortcuts.iter().any(|s| s == "c")) && self.copy() {
                ui.close();
                return;
            }
            if shortcuts.iter().any(|s| s == "s") && self.save().is_some() {
                ui.close();
                return;
            }
        }

        self.draw_screen(ui, &scr, mirror);
        if !mirror {
            self.primary_view(ui, &scr);
        }
        // As bolinhas por último: por cima do rótulo do tamanho e do painel.
        self.paint_handles(ui, &scr, mirror);
    }
}

impl Capture {
    /// O que só a camada principal faz: ponteiro, modo rápido, painel da gravação ou
    /// da captura.
    fn primary_view(&mut self, ui: &mut Ui, scr: &mide::kit::Screen) {
        // Ponteiro em pontos no desktop (em qualquer tela), e o painel embaixo e no meio
        // desta tela (ou fora do caminho das bolinhas). O desenho do `view` é do
        // estado de antes deste clique: apertar ou soltar pede mais um quadro (senão a
        // área que acabou de ser escolhida só aparece no próximo movimento do mouse).
        let pointer = ui.pointer_global();
        if ui.input().pressed || ui.input().released {
            ui.request_repaint();
        }
        let (sw, sh) = scr.size;

        // Modo rápido: só a seleção; ao soltar, salva e copia (como o ⌘⇧4 do Mac).
        if self.quick {
            if let Some(pos) = pointer {
                let releasing = ui.input().released && self.drag.is_some();
                self.pointer(ui, pos, false);
                if releasing && self.selection.is_some() {
                    tracing::info!("captura rápida de uma área");
                    let copied = self.copy();
                    if self.save().is_some() || copied {
                        ui.close();
                    }
                }
            }
            if self.drag.is_some() {
                ui.request_repaint();
            }
            return;
        }

        if self.record {
            self.record_view(ui, scr.size);
            return;
        }

        // Painel: modo, ferramenta, cor e ações, embaixo e no meio.
        let panel_w = 720.0;
        let px = (sw - panel_w) / 2.0;
        let py = self.place_panel(scr.origin, (px, panel_w), sh - 128.0, 96.0);
        let mut action = None;
        let rect = ui.glass_panel(px, py, panel_w, 14.0, |ui| {
            ui.row(|ui| {
                for (mode, label) in [(Mode::Selection, "Seleção"), (Mode::Window, "Janela"), (Mode::Full, "Tela inteira")] {
                    let current = self.mode == mode || (mode == Mode::Full && self.mode == Mode::PickScreen);
                    let r = if current { ui.primary_button(label) } else { ui.button(label) };
                    if r.clicked() {
                        if mode == Mode::Full {
                            self.choose_full();
                        } else {
                            self.mode = mode;
                            self.selection = None;
                            self.shapes.clear();
                        }
                    }
                }
                ui.push_right(268.0);
                if ui.button("Cancelar").clicked() {
                    action = Some("cancelar");
                }
                if ui.button("Salvar").clicked() {
                    action = Some("salvar");
                }
                if ui.primary_button("Copiar").clicked() {
                    action = Some("copiar");
                }
            });
            ui.row(|ui| {
                for (tool, label) in [(Tool::Adjust, "Ajustar"), (Tool::Arrow, "Seta"), (Tool::Circle, "Círculo"), (Tool::Square, "Quadrado")] {
                    let r = if self.tool == tool { ui.primary_button(label) } else { ui.button(label) };
                    if r.clicked() {
                        self.tool = tool;
                        tracing::info!("captura: ferramenta {label}");
                    }
                }
            });
        });
        self.panel_h = rect.3;
        // Cores: círculos à direita das ferramentas, na segunda linha do painel.
        let row_y = rect.1 + rect.3 - 30.0;
        self.swatches = (0..COLORS.len()).map(|i| (rect.0 + rect.2 - 14.0 - (COLORS.len() - i) as f32 * 30.0, row_y, 24.0, 24.0)).collect();
        let swatches = self.swatches.clone();
        let chosen = self.color;
        let accent = ui.theme().accent;
        ui.paint(|canvas, _| {
            for (i, (x, y, w, _)) in swatches.iter().enumerate() {
                let [r, g, b] = COLORS[i].1;
                let center = (x + w / 2.0, y + w / 2.0);
                if let Some(path) = PathBuilder::from_circle(center.0, center.1, w / 2.0 - 2.0) {
                    let mut paint = Paint::default();
                    paint.anti_alias = true;
                    paint.set_color(Color::from_rgba8(r, g, b, 255));
                    canvas.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
                    paint.set_color(Color::from_rgba8(0, 0, 0, 70));
                    canvas.stroke_path(&path, &paint, &Stroke { width: 1.0, ..Default::default() }, Transform::identity(), None);
                }
                if i == chosen {
                    if let Some(ring) = PathBuilder::from_circle(center.0, center.1, w / 2.0 + 1.5) {
                        let mut paint = Paint::default();
                        paint.anti_alias = true;
                        paint.set_color(Color::from_rgba8(accent[0], accent[1], accent[2], 255));
                        canvas.stroke_path(&ring, &paint, &Stroke { width: 2.0, ..Default::default() }, Transform::identity(), None);
                    }
                }
            }
        });
        if let (Some(local), Some(pos)) = (ui.input().pointer, pointer) {
            // O painel é desta camada: o teste é no ponteiro local a ela.
            let over_panel = inside(rect, local);
            if over_panel && ui.input().released {
                if let Some(i) = self.swatches.iter().position(|s| inside(*s, local)) {
                    self.color = i;
                    tracing::info!("captura: cor {}", COLORS[i].0);
                }
            }
            self.pointer(ui, pos, over_panel);
        }
        match action {
            Some("cancelar") => ui.close(),
            Some("salvar") if self.save().is_some() => ui.close(),
            Some("copiar") if self.copy() => ui.close(),
            _ => {}
        }
        if self.drag.is_some() {
            ui.request_repaint();
        }
    }
}

impl Capture {
    /// Desenha a parte desta tela (`scr`): a captura congelada dela, escurecida fora da
    /// área escolhida, com o contorno, as marcações e o tamanho da área (as bolinhas
    /// vêm depois, em `paint_handles`).
    /// Tudo em pontos no desktop, deslocado pela origem da tela.
    fn draw_screen(&self, ui: &mut Ui, scr: &mide::kit::Screen, mirror: bool) {
        let pointer = ui.pointer_global();
        // A imagem desta tela; a camada principal sem nome (compositor antigo, uma tela
        // só) mostra a principal.
        let shot = self.screens.iter().find(|s| s.name == scr.name).or_else(|| (!mirror).then(|| self.primary()));
        let (ox, oy) = shot.map_or(scr.origin, |s| (s.rect.0, s.rect.1));
        let hovered_window = (self.mode == Mode::Window).then(|| pointer.and_then(|p| self.windows.iter().find(|r| inside(**r, p)).copied())).flatten();
        let drawing = match self.drag {
            Some(Drag::Draw { start }) => pointer.map(|p| Shape { tool: self.tool, color: COLORS[self.color].1, from: start, to: p }),
            _ => None,
        };
        let selection = self.selection;
        let shapes = &self.shapes;
        // Escolhendo a tela: a que está sob o ponteiro fica clara, com o contorno na cor
        // de destaque.
        let picking = self.mode == Mode::PickScreen && shot.is_some_and(|s| pointer.is_some_and(|p| contains_half_open(s.rect, p)));
        let accent = ui.theme().accent;
        ui.paint(|canvas, _| {
            // Tela escurecida e, por cima, a área escolhida com as cores de verdade:
            // só cópia de bytes (rápido mesmo numa tela grande). A imagem já está no
            // tamanho da camada (pontos); se não estiver (arredondamento), desenhada.
            let Some(shot) = shot else {
                // Tela sem imagem (não chegou a tempo): escura, sem nada por cima.
                canvas.fill(Color::from_rgba8(20, 20, 22, 255));
                return;
            };
            let base = if picking { &shot.view } else { &shot.dimmed };
            if canvas.data().len() == base.data().len() {
                canvas.data_mut().copy_from_slice(base.data());
            } else {
                canvas.draw_pixmap(0, 0, base.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
            }
            if picking && let Some(rect) = Rect::from_xywh(2.0, 2.0, (shot.rect.2 - 4.0).max(1.0), (shot.rect.3 - 4.0).max(1.0)) {
                let paint = Paint { shader: tiny_skia::Shader::SolidColor(Color::from_rgba8(accent[0], accent[1], accent[2], 255)), anti_alias: true, ..Default::default() };
                canvas.stroke_path(&PathBuilder::from_rect(rect), &paint, &Stroke { width: 4.0, ..Default::default() }, Transform::identity(), None);
            }
            if let Some((x, y, w, h)) = selection.or(hovered_window) {
                let local = (x - ox, y - oy, w, h);
                copy_rect(canvas, &shot.view, local);
                outline(canvas, local);
            }
            for shape in shapes.iter().chain(drawing.iter()) {
                draw_shape(canvas, shape, Transform::from_translate(-ox, -oy));
            }
        });

        // O rótulo em vidro no meio da tela que vai ser escolhida: o que o clique faz, o
        // nome e o tamanho em pixels.
        if picking && let Some(shot) = shot {
            let (w, h) = (360.0, shot.rect.3);
            let (x, y) = (((shot.rect.2 - w) / 2.0).max(0.0), (h / 2.0 - 40.0).max(0.0));
            let what = if self.record { "Clique para gravar esta tela" } else { "Clique para capturar esta tela" };
            let detail = format!("{} · {} × {}", shot.name, shot.shot.width(), shot.shot.height());
            ui.glass_panel(x, y, w, 14.0, |ui| {
                ui.label(what);
                ui.note(&detail);
            });
        }

        // Tamanho da área, logo acima dela (ou dentro, se não couber), na tela onde fica
        // o canto de cima à esquerda (num buraco entre telas, na camada principal).
        if let Some((x, y, w, h)) = self.selection.filter(|_| self.mode != Mode::Full) {
            let screens = ui.screens();
            let here = match mide::kit::screens::screen_at(&screens, (x, y)) {
                Some(i) => screens[i].name == scr.name,
                None => !mirror,
            };
            if here {
                let (x, y) = (x - scr.origin.0, y - scr.origin.1);
                let text = format!("{} × {}", w.round() as i32, h.round() as i32);
                let ly = if y >= 44.0 { y - 40.0 } else { y + 6.0 };
                ui.glass_panel(x, ly, 120.0, 8.0, |ui| ui.label(&text));
            }
        }
    }

    /// As bolinhas da área na parte desta tela (`scr`), por cima de tudo (rótulo do
    /// tamanho e painel).
    fn paint_handles(&self, ui: &mut Ui, scr: &mide::kit::Screen, mirror: bool) {
        let shot = self.screens.iter().find(|s| s.name == scr.name).or_else(|| (!mirror).then(|| self.primary()));
        let (Some(shot), Some((x, y, w, h))) = (shot, self.selection.filter(|_| self.handles_visible())) else {
            return;
        };
        let (ox, oy) = (shot.rect.0, shot.rect.1);
        let accent = ui.theme().accent;
        ui.paint(|canvas, _| draw_handles(canvas, (x - ox, y - oy, w, h), accent));
    }

    /// Altura do painel (na tela principal, de origem `origin`) que não cobre as
    /// bolinhas da área: embaixo (`default_y`) se der; senão em cima; senão o primeiro
    /// lugar livre subindo de baixo. Durante um arrasto, fica onde estava (não pula a
    /// cada movimento). `(px, pw)`: x e largura do painel; `est_h`: altura até o
    /// primeiro quadro dizer a de verdade.
    fn place_panel(&mut self, origin: (f32, f32), (px, pw): (f32, f32), default_y: f32, est_h: f32) -> f32 {
        if let (Some(_), Some(y)) = (self.drag, self.panel_y) {
            return y;
        }
        let h = if self.panel_h > 0.0 { self.panel_h } else { est_h };
        let centers: Vec<(f32, f32)> = match self.selection.filter(|_| self.handles_visible()) {
            Some(sel) => handles(sel).iter().map(|(_, c)| (c.0 - origin.0, c.1 - origin.1)).collect(),
            None => Vec::new(),
        };
        let y = panel_spot(&centers, (px, pw, h), default_y);
        self.panel_y = Some(y);
        y
    }

    /// Painel do ⌘⇧5: o que gravar (tela inteira ou área), os sons e Gravar.
    /// `size`: tamanho da tela da camada principal (o painel fica embaixo e no meio).
    fn record_view(&mut self, ui: &mut Ui, (sw, sh): (f32, f32)) {
        let (ox, oy, _, _) = self.primary().rect;
        let mut picked = false;
        let panel_w = 740.0;
        let px = (sw - panel_w) / 2.0;
        let py = self.place_panel((ox, oy), (px, panel_w), sh - 84.0, 56.0);
        let mut start = ui.input().keys.contains(&Key::Enter);
        let mut cancel = false;
        let rect = ui.glass_panel(px, py, panel_w, 14.0, |ui| {
            ui.row(|ui| {
                for (mode, label) in [(Mode::Full, "Tela inteira"), (Mode::Selection, "Área")] {
                    let current = self.mode == mode || (mode == Mode::Full && self.mode == Mode::PickScreen);
                    let r = if current { ui.primary_button(label) } else { ui.button(label) };
                    if r.clicked() {
                        if mode == Mode::Full {
                            self.choose_full();
                        } else {
                            self.mode = mode;
                            self.selection = None;
                            self.record_screen = None;
                        }
                    }
                }
                for (on, label) in [(self.system_audio, "Som do sistema"), (self.microphone, "Microfone")] {
                    let text = format!("{} {label}", if on { "☑" } else { "☐" });
                    if ui.button(&text).clicked() {
                        if label == "Microfone" {
                            self.microphone = !self.microphone;
                            let _ = mide::settings_file::set("record", "microphone", self.microphone.then_some(mide::settings_file::Value::Bool(true)));
                        } else {
                            self.system_audio = !self.system_audio;
                            let _ = mide::settings_file::set("record", "system_audio", (!self.system_audio).then_some(mide::settings_file::Value::Bool(false)));
                        }
                    }
                }
                ui.push_right(200.0);
                cancel = ui.button("Cancelar").clicked();
                start |= ui.primary_button("Gravar").clicked();
            });
        });
        self.panel_h = rect.3;
        if let (Some(local), Some(pos)) = (ui.input().pointer, ui.pointer_global()) {
            let over_panel = inside(rect, local);
            match self.mode {
                Mode::Selection => self.pointer(ui, pos, over_panel),
                // Com várias telas, o clique numa tela começa a gravar ela inteira (como
                // no Mac).
                Mode::PickScreen if !over_panel && ui.input().released => {
                    if let Some(i) = screen_index_at(&self.rects(), pos) {
                        self.pick_screen(i);
                        picked = true;
                    }
                }
                _ => {}
            }
        }
        start |= picked;
        if self.drag.is_some() {
            ui.request_repaint();
        }
        if cancel {
            tracing::info!("gravação cancelada");
            ui.close();
            return;
        }
        if start {
            // A gravação é de uma tela: a área vai em pontos dela, com o nome (`tela
            // NOME`) quando há mais de uma.
            let (screen, area) = match (self.mode, self.selection) {
                (Mode::Selection, Some(sel)) => {
                    let i = self.record_screen.unwrap_or_else(|| screen_index_near(&self.rects(), (sel.0, sel.1)));
                    let (name, area) = record_target(&self.screens, sel, i);
                    (name, Some(area))
                }
                (Mode::Selection, None) => return,
                _ => (self.screens[self.full_index()].name.clone(), None),
            };
            let screen = (self.screens.len() > 1).then_some(screen.as_str());
            let command = record_command(area, screen, self.system_audio, self.microphone);
            let reply = ask_compositor(&command).and_then(|r| r.into_iter().next()).unwrap_or_default();
            if let Some(err) = reply.strip_prefix("erro: ") {
                mide::notify::send_and_wait("Gravação de tela", "A gravação não começou", err);
            } else {
                tracing::info!("gravação começou: {reply}");
            }
            ui.close();
        }
    }
}

/// Grava as imagens em Imagens/Capturas de tela (`shot_file_name`, com o nome da tela
/// quando dado) e avisa uma vez onde ficaram (ou o erro). Devolve os arquivos gravados.
fn save_images(images: &[(&Pixmap, Option<&str>)]) -> Vec<PathBuf> {
    let dir = mide::places::pictures_dir().join("Capturas de tela");
    let stamp = chrono_name();
    let mut saved = Vec::new();
    let mut error = None;
    for (image, screen) in images {
        let path = dir.join(shot_file_name(&stamp, *screen));
        match std::fs::create_dir_all(&dir).map_err(|e| e.to_string()).and_then(|()| image.save_png(&path).map_err(|e| e.to_string())) {
            Ok(()) => {
                tracing::info!("captura salva: {}", path.display());
                saved.push(path);
            }
            Err(err) => {
                tracing::warn!("captura não salva em {}: {err}", dir.display());
                error = Some(err);
            }
        }
    }
    if !saved.is_empty() {
        shutter();
    }
    match (saved.as_slice(), error) {
        (_, Some(err)) => mide::notify::send_and_wait("Captura de tela", "A captura não foi salva", &format!("{}: {err}", dir.display())),
        ([one], None) => mide::notify::send_and_wait("Captura de tela", "Captura salva", &one.display().to_string()),
        (many, None) if !many.is_empty() => mide::notify::send_and_wait("Captura de tela", "Capturas salvas", &format!("{} telas em {}", many.len(), dir.display())),
        _ => {}
    }
    saved
}

/// Copia o retângulo `r` de `src` para o mesmo lugar em `dst`, linha a linha. Se os
/// tamanhos diferem (um ponto a mais ou a menos num compositor antigo), copia só o que
/// as duas têm em comum.
fn copy_rect(dst: &mut Pixmap, src: &Pixmap, r: (f32, f32, f32, f32)) {
    let w = src.width().min(dst.width()) as i64;
    let h = src.height().min(dst.height()) as i64;
    let x0 = (r.0.floor() as i64).clamp(0, w);
    let x1 = ((r.0 + r.2).ceil() as i64).clamp(0, w);
    let y0 = (r.1.floor() as i64).clamp(0, h);
    let y1 = ((r.1 + r.3).ceil() as i64).clamp(0, h);
    if x1 <= x0 {
        return;
    }
    let (src_stride, dst_stride) = (src.width() as usize * 4, dst.width() as usize * 4);
    let (a, b) = (x0 as usize * 4, x1 as usize * 4);
    let src = src.data();
    let dst = dst.data_mut();
    for y in y0 as usize..y1 as usize {
        let (s, d) = (y * src_stride, y * dst_stride);
        dst[d + a..d + b].copy_from_slice(&src[s + a..s + b]);
    }
}

/// Contorno branco da área escolhida.
fn outline(canvas: &mut Pixmap, (x, y, w, h): (f32, f32, f32, f32)) {
    if let Some(rect) = Rect::from_xywh(x, y, w, h) {
        let path = PathBuilder::from_rect(rect);
        let mut paint = Paint::default();
        paint.set_color(Color::from_rgba8(255, 255, 255, 230));
        canvas.stroke_path(&path, &paint, &Stroke { width: 1.5, ..Default::default() }, Transform::identity(), None);
    }
}

/// Topo do painel (x `px`, largura `pw`, altura `ph`) sem nenhuma bolinha de `centers`
/// (coordenadas da tela) embaixo dele nem na margem do clique: `default_y` (embaixo)
/// se der; senão 44 (em cima, abaixo da barra); senão o primeiro lugar livre subindo
/// de baixo; sem lugar livre, `default_y` (a bolinha vence o clique no painel).
fn panel_spot(centers: &[(f32, f32)], (px, pw, ph): (f32, f32, f32), default_y: f32) -> f32 {
    let m = HANDLE_HIT + 2.0;
    let free = |y: f32| !centers.iter().any(|c| inside((px - m, y - m, pw + 2.0 * m, ph + 2.0 * m), *c));
    let top = 44.0_f32.min(default_y);
    let mut tries = vec![default_y, top];
    let mut y = default_y - 8.0;
    while y > top {
        tries.push(y);
        y -= 8.0;
    }
    tries.into_iter().find(|y| free(*y)).unwrap_or(default_y)
}

/// Bolinhas da área `r` (no canvas): círculo na cor de destaque com um anel branco.
fn draw_handles(canvas: &mut Pixmap, r: RectF, accent: [u8; 4]) {
    let solid = |c: Color| Paint { shader: tiny_skia::Shader::SolidColor(c), anti_alias: true, ..Default::default() };
    let fill = solid(Color::from_rgba8(accent[0], accent[1], accent[2], 255));
    let ring = solid(Color::WHITE);
    for (_, (x, y)) in handles(r) {
        if let Some(path) = PathBuilder::from_circle(x, y, HANDLE_RADIUS) {
            canvas.fill_path(&path, &fill, FillRule::Winding, Transform::identity(), None);
            canvas.stroke_path(&path, &ring, &Stroke { width: 1.5, ..Default::default() }, Transform::identity(), None);
        }
    }
}

/// Desenha a seta, o círculo ou o quadrado. `ts` leva das coordenadas da marcação
/// (pontos no desktop) ao canvas; numa ampliação, o traço engrossa junto.
fn draw_shape(canvas: &mut Pixmap, shape: &Shape, ts: Transform) {
    let (a, b) = (shape.from, shape.to);
    let [r, g, bl] = shape.color;
    let mut paint = Paint::default();
    paint.anti_alias = true;
    paint.set_color(Color::from_rgba8(r, g, bl, 255));
    let stroke = Stroke { width: 4.0, line_cap: LineCap::Round, ..Default::default() };
    match shape.tool {
        // Ajustar não desenha (não vira marcação).
        Tool::Adjust => {}
        Tool::Arrow => {
            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
            let len = dx.hypot(dy).max(1.0);
            let (ux, uy) = (dx / len, dy / len);
            let head = 18.0f32.min(len * 0.6);
            // Haste até a base da ponta.
            let base = (b.0 - ux * head * 0.8, b.1 - uy * head * 0.8);
            let mut pb = PathBuilder::new();
            pb.move_to(a.0, a.1);
            pb.line_to(base.0, base.1);
            if let Some(path) = pb.finish() {
                canvas.stroke_path(&path, &paint, &stroke, ts, None);
            }
            // Ponta: triângulo cheio.
            let (px, py) = (-uy, ux);
            let mut pb = PathBuilder::new();
            pb.move_to(b.0, b.1);
            pb.line_to(b.0 - ux * head + px * head * 0.55, b.1 - uy * head + py * head * 0.55);
            pb.line_to(b.0 - ux * head - px * head * 0.55, b.1 - uy * head - py * head * 0.55);
            pb.close();
            if let Some(path) = pb.finish() {
                canvas.fill_path(&path, &paint, FillRule::Winding, ts, None);
            }
        }
        Tool::Circle => {
            let (x, y, w, h) = normalized(a, b);
            if let Some(rect) = Rect::from_xywh(x, y, w.max(1.0), h.max(1.0)) {
                if let Some(path) = PathBuilder::from_oval(rect) {
                    canvas.stroke_path(&path, &paint, &stroke, ts, None);
                }
            }
        }
        Tool::Square => {
            let (x, y, w, h) = normalized(a, b);
            if let Some(rect) = Rect::from_xywh(x, y, w.max(1.0), h.max(1.0)) {
                let path = PathBuilder::from_rect(rect);
                let stroke = Stroke { width: 4.0, line_join: tiny_skia::LineJoin::Round, ..Default::default() };
                canvas.stroke_path(&path, &paint, &stroke, ts, None);
            }
        }
    }
}

/// Som do obturador, uma vez por captura (salvar e copiar juntos tocam um só). Espera
/// o som começar: a captura fecha logo depois.
fn shutter() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        mide::sounds::play("screen-capture");
        std::thread::sleep(Duration::from_millis(60));
    });
}

fn runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir)
}

/// Data e hora para o nome do arquivo (`2026-10-19 21-40-05`), pelo relógio local.
fn chrono_name() -> String {
    let out = std::process::Command::new("date").arg("+%Y-%m-%d %H-%M-%S").output().ok();
    out.and_then(|o| String::from_utf8(o.stdout).ok()).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| {
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        secs.to_string()
    })
}

/// Pede a captura de todas as telas ao compositor e espera os arquivos (até 3 s).
/// `wait_launcher`: aberta pelo launcher (o app Captura de Tela), espera ele fechar,
/// senão a captura sai com o launcher por cima; pelos atalhos (⌘⇧3, ⌘⇧4), a tela sai
/// como está.
fn capture(wait_launcher: bool) -> Result<Vec<Shot>, Box<dyn Error>> {
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    while wait_launcher && ask_compositor("launcher state").is_some_and(|r| r.first().map(String::as_str) == Some("open")) {
        if std::time::Instant::now() > deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    // O último quadro do esmaecer e os menus que fecham junto.
    std::thread::sleep(Duration::from_millis(60));
    let prefix = runtime_dir().join(format!("mide-tela-{}", std::process::id()));
    // Restos de uma captura anterior com o mesmo PID, apagados ANTES do pedido: depois
    // dele o compositor já pode ter gravado a imagem nova (no DRM, em uns 40 ms).
    remove_stale(&prefix);
    let reply = ask_compositor(&format!("screenshot all {}", prefix.display())).ok_or("o mide não respondeu")?;
    let mut lines = parse_screens(&reply);
    if lines.is_empty() {
        // Compositor de antes do `screenshot all` (atualizado sem `restart all`): ele
        // responde `ok` e toma "all <prefixo>" por um caminho relativo, numa pasta "all "
        // que não existe (só avisa no log). Pede do jeito antigo: só a tela principal,
        // com a posição e a escala do comando `outputs`.
        if let Some(err) = reply.first().and_then(|l| l.strip_prefix("erro: ")) {
            return Err(err.to_string().into());
        }
        let path = runtime_dir().join(format!("mide-tela-{}.png", std::process::id()));
        let _ = std::fs::remove_file(&path);
        ask_compositor(&format!("screenshot {}", path.display())).ok_or("o mide não respondeu")?;
        let (origin, scale) = ask_compositor("outputs").and_then(|r| primary_from_outputs(&r)).unwrap_or(((0.0, 0.0), 1.0));
        lines.push(ScreenLine { name: String::new(), rect: (origin.0, origin.1, 0.0, 0.0), scale, primary: true, path });
    }
    // O compositor salva no próximo quadro de cada tela: espera todas.
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while lines.iter().any(|l| !l.path.exists()) && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(15));
    }
    let mut shots = Vec::new();
    for line in &lines {
        let image = match load_png(&line.path) {
            Ok(image) => image,
            // Sem a principal não há o que mostrar; outra tela que falhou fica de fora.
            Err(err) if line.primary => return Err(format!("a captura não chegou: {err}").into()),
            Err(err) => {
                tracing::warn!("captura: a tela {} não chegou: {err}", line.name);
                continue;
            }
        };
        let _ = std::fs::remove_file(&line.path);
        // Sem o tamanho em pontos (compositor antigo), ele vem da imagem e da escala,
        // arredondado para cima como o compositor faz com a camada.
        let mut rect = line.rect;
        if rect.2 <= 0.0 {
            (rect.2, rect.3) = points_size((image.width(), image.height()), line.scale);
        }
        shots.push(Shot::new(&line.name, rect, line.scale, line.primary, image));
    }
    if !shots.iter().any(|s| s.primary)
        && let Some(first) = shots.first_mut()
    {
        first.primary = true;
    }
    Ok(shots)
}

/// Tamanho em pontos de uma tela de `px` pixels na escala `scale`, arredondado para cima
/// como o Smithay faz (`to_i32_ceil`): 1366x768 em 1,25 dá 1093x615.
fn points_size(px: (u32, u32), scale: f32) -> (f32, f32) {
    ((px.0 as f32 / scale).ceil(), (px.1 as f32 / scale).ceil())
}

/// Apaga os arquivos `<prefixo>-*.png` que sobraram de uma captura anterior.
fn remove_stale(prefix: &std::path::Path) {
    let (Some(dir), Some(stem)) = (prefix.parent(), prefix.file_name().and_then(|n| n.to_str())) else {
        return;
    };
    let start = format!("{stem}-");
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = entry.file_name();
        if name.to_str().is_some_and(|n| n.starts_with(&start) && n.ends_with(".png")) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Posição e escala da tela principal na resposta do `outputs`
/// (`nome descrição modo x,y escala ligada principal|- modos`).
fn primary_from_outputs(reply: &[String]) -> Option<((f32, f32), f32)> {
    let line = reply.iter().find(|l| l.split('\t').nth(6) == Some("principal"))?;
    let f: Vec<&str> = line.split('\t').collect();
    let (x, y) = f.get(3)?.split_once(',')?;
    Some(((x.parse().ok()?, y.parse().ok()?), f.get(4)?.parse().ok().filter(|s: &f32| *s > 0.0)?))
}

/// Lê o PNG salvo pelo compositor.
fn load_png(path: &std::path::Path) -> Result<Pixmap, Box<dyn Error>> {
    let image = image::open(path)?.to_rgba8();
    let (w, h) = image.dimensions();
    // Opaca: o RGBA já serve como pré-multiplicado.
    Pixmap::from_vec(image.into_raw(), tiny_skia::IntSize::from_wh(w, h).ok_or("tamanho inválido")?).ok_or_else(|| "imagem inválida".into())
}

/// Janelas à mostra, a de cima primeiro.
fn window_rects() -> Vec<(f32, f32, f32, f32)> {
    ask_compositor("window_rects")
        .unwrap_or_default()
        .iter()
        .filter_map(|line| {
            let f: Vec<&str> = line.split('\t').collect();
            Some((f.get(1)?.parse().ok()?, f.get(2)?.parse().ok()?, f.get(3)?.parse().ok()?, f.get(4)?.parse().ok()?))
        })
        .collect()
}

fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "mide_screenshot=info".into()))
        .init();
    // `--full` (⌘⇧3): a tela inteira, salva e copiada, sem janela. `--area` (⌘⇧4):
    // escolher uma área, salva e copiada ao soltar o mouse.
    let mode = std::env::args().nth(1).unwrap_or_default();
    // ⌘⇧5 com uma gravação em andamento: para (como o botão na barra).
    if mode == "--record" && ask_compositor("record status").is_some_and(|r| r.first().is_some_and(|l| l.starts_with("on"))) {
        ask_compositor("record stop");
        return Ok(());
    }
    let screens = capture(mode.is_empty())?;
    if mode == "--full" {
        // A tela inteira (com várias telas, um arquivo de cada; a principal é copiada).
        let mut full = Capture::new(screens, Vec::new());
        return if full.save_full() { Ok(()) } else { Err("a captura não foi salva nem copiada".into()) };
    }
    let names: Vec<String> = screens.iter().map(|s| format!("{} {}x{} em {:.2}", s.name, s.rect.2, s.rect.3, s.scale)).collect();
    tracing::info!("captura de tela aberta ({} tela(s): {})", screens.len(), names.join(", "));
    let windows = window_rects();
    let mut app = Capture::new(screens, windows);
    app.quick = mode == "--area";
    if mode == "--record" {
        app.record = true;
        // Tela inteira; com várias telas, escolher qual (o clique começa a gravar).
        app.choose_full();
    }
    // A imagem capturada é pintada pixel a pixel (`ui.paint`): escala 1, em pontos. Uma
    // camada em cada tela; só a principal recebe o teclado.
    run(Window::new("Captura de tela").app_id("mide-screenshot").overlay("mide-screenshot").on_every_output().fixed_scale(), app)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tela de cor única, `w`×`h` pixels, com a posição e a escala dadas.
    fn screen(name: &str, at: (f32, f32), px: (u32, u32), scale: f32, rgb: [u8; 3], primary: bool) -> Shot {
        let mut shot = Pixmap::new(px.0, px.1).unwrap();
        shot.fill(Color::from_rgba8(rgb[0], rgb[1], rgb[2], 255));
        let size = points_size(px, scale);
        let rect = (at.0, at.1, size.0, size.1);
        Shot::new(name, rect, scale, primary, shot)
    }

    fn rgb(p: &Pixmap, x: u32, y: u32) -> [u8; 3] {
        let c = p.pixel(x, y).unwrap();
        [c.red(), c.green(), c.blue()]
    }

    const RED: [u8; 3] = [255, 0, 0];
    const BLUE: [u8; 3] = [0, 0, 255];

    #[test]
    fn recorta_a_area_com_as_marcacoes() {
        let mut capture = Capture::new(vec![screen("A", (0.0, 0.0), (200, 100), 1.0, [255, 255, 255], true)], vec![(10.0, 10.0, 50.0, 40.0)]);
        capture.selection = Some((20.0, 10.0, 100.0, 60.0));
        capture.shapes.push(Shape { tool: Tool::Square, color: [235, 52, 52], from: (30.0, 20.0), to: (60.0, 50.0) });
        let out = capture.result().unwrap();
        assert_eq!((out.width(), out.height()), (100, 60));
        // A borda do quadrado vermelho cai em (10, 10) no recorte.
        let px = out.pixel(10, 20).unwrap();
        assert!(px.red() > 200 && px.green() < 100, "quadrado vermelho no recorte");
        // Fora das marcações, o branco da captura.
        assert_eq!(out.pixel(80, 50).unwrap().green(), 255);
    }

    #[test]
    fn janela_fora_da_tela_e_recortada() {
        assert_eq!(clamp_rect((-10.0, 20.0, 100.0, 50.0), (0.0, 0.0, 80.0, 60.0)), (0.0, 20.0, 80.0, 40.0));
        // Limites de duas telas lado a lado: a janela na segunda não é cortada na primeira.
        assert_eq!(clamp_rect((1400.0, 100.0, 300.0, 200.0), (0.0, 0.0, 3286.0, 1080.0)), (1400.0, 100.0, 300.0, 200.0));
    }

    #[test]
    fn le_screenshot_all() {
        let reply = vec![
            "HEADLESS-1\t0\t0\t1366\t768\t1\tprincipal\t/run/t-HEADLESS-1.png".to_string(),
            "HEADLESS-2\t1366\t0\t1536\t864\t1.25\t-\t/run/minha pasta/t-HEADLESS-2.png".to_string(),
            "uso: screenshot".to_string(),
        ];
        let screens = parse_screens(&reply);
        assert_eq!(screens.len(), 2);
        assert_eq!(screens[0], ScreenLine { name: "HEADLESS-1".into(), rect: (0.0, 0.0, 1366.0, 768.0), scale: 1.0, primary: true, path: "/run/t-HEADLESS-1.png".into() });
        assert_eq!((screens[1].rect, screens[1].scale, screens[1].primary), ((1366.0, 0.0, 1536.0, 864.0), 1.25, false));
        assert_eq!(screens[1].path, PathBuf::from("/run/minha pasta/t-HEADLESS-2.png"));
        // A resposta de uso de um compositor antigo não vira tela.
        assert!(parse_screens(&["uso: screenshot <arquivo.png>".to_string()]).is_empty());
    }

    #[test]
    fn principal_pelo_comando_outputs() {
        let reply = vec![
            "HEADLESS-1\t-\t1366x768@60\t0,0\t1\tligada\t-\t1366x768@60".to_string(),
            "HEADLESS-2\t-\t1920x1080@60\t1366,0\t1.25\tligada\tprincipal\t1920x1080@60".to_string(),
        ];
        assert_eq!(primary_from_outputs(&reply), Some(((1366.0, 0.0), 1.25)));
    }

    #[test]
    fn imagem_em_pontos_para_mostrar() {
        // 1920x1080 pixels em 1,25: a camada (escala 1) tem 1536x864 pontos, e a imagem
        // mostrada e o escurecido têm esse tamanho (antes, o escurecido não cabia e a
        // tela não escurecia).
        let s = screen("A", (0.0, 0.0), (1920, 1080), 1.25, RED, true);
        assert_eq!(s.rect, (0.0, 0.0, 1536.0, 864.0));
        assert_eq!((s.view.width(), s.view.height()), (1536, 864));
        assert_eq!((s.dimmed.width(), s.dimmed.height()), (1536, 864));
        assert_eq!((s.shot.width(), s.shot.height()), (1920, 1080));
        assert!(rgb(&s.dimmed, 700, 400)[0] < 160, "escurecido");
    }

    #[test]
    fn output_scale_e_a_maior_tocada() {
        let screens = vec![screen("A", (0.0, 0.0), (1366, 768), 1.0, RED, true), screen("B", (1366.0, 0.0), (2400, 1350), 1.25, BLUE, false)];
        assert_eq!(output_scale((1266.0, 100.0, 200.0, 100.0), &screens), 1.25);
        assert_eq!(output_scale((100.0, 100.0, 200.0, 100.0), &screens), 1.0);
        assert_eq!(bounds_of(&screens.iter().map(|s| s.rect).collect::<Vec<_>>()), (0.0, 0.0, 3286.0, 1080.0));
    }

    #[test]
    fn recorte_principal_125() {
        // O bug de antes: a área em pontos recortada da imagem em pixels.
        let mut shot = Pixmap::new(1920, 1080).unwrap();
        shot.fill(Color::from_rgba8(255, 255, 255, 255));
        // Um quadrado azul em (300, 200) pontos = (375, 250) pixels.
        shot.fill_rect(Rect::from_xywh(375.0, 250.0, 20.0, 20.0).unwrap(), &Paint { shader: tiny_skia::Shader::SolidColor(Color::from_rgba8(0, 0, 255, 255)), ..Default::default() }, Transform::identity(), None);
        let screens = vec![Shot::new("A", (0.0, 0.0, 1536.0, 864.0), 1.25, true, shot)];
        let out = compose(&screens, (300.0, 200.0, 400.0, 250.0), &[]).unwrap();
        assert_eq!((out.width(), out.height()), (500, 313));
        assert_eq!(rgb(&out, 5, 5), BLUE, "o canto da área é o quadrado azul");
        assert_eq!(rgb(&out, 30, 30), [255, 255, 255]);
    }

    #[test]
    fn compoe_atravessando() {
        let screens = vec![screen("A", (0.0, 0.0), (1366, 768), 1.0, RED, true), screen("B", (1366.0, 0.0), (1920, 1080), 1.0, BLUE, false)];
        let out = compose(&screens, (1266.0, 100.0, 200.0, 100.0), &[]).unwrap();
        assert_eq!((out.width(), out.height()), (200, 100));
        assert_eq!(rgb(&out, 50, 50), RED);
        assert_eq!(rgb(&out, 150, 50), BLUE);
    }

    #[test]
    fn compoe_na_maior_escala() {
        let screens = vec![screen("A", (0.0, 0.0), (1366, 768), 1.0, RED, true), screen("B", (1366.0, 0.0), (2400, 1350), 1.25, BLUE, false)];
        let out = compose(&screens, (1266.0, 100.0, 200.0, 100.0), &[]).unwrap();
        assert_eq!((out.width(), out.height()), (250, 125));
        assert_eq!(rgb(&out, 10, 50), RED);
        assert_eq!(rgb(&out, 240, 50), BLUE);
    }

    #[test]
    fn buraco_entre_telas() {
        // A tem 768 de altura e B, 1080: abaixo de A, à esquerda de B, não há tela.
        let screens = vec![screen("A", (0.0, 0.0), (1366, 768), 1.0, RED, true), screen("B", (1366.0, 0.0), (1920, 1080), 1.0, BLUE, false)];
        let out = compose(&screens, (1300.0, 700.0, 200.0, 100.0), &[]).unwrap();
        assert_eq!(rgb(&out, 30, 30), RED);
        assert_eq!(rgb(&out, 30, 90), [0, 0, 0], "buraco preto");
        assert_eq!(out.pixel(30, 90).unwrap().alpha(), 255, "opaco");
        assert_eq!(rgb(&out, 150, 90), BLUE);
    }

    #[test]
    fn marcacoes_seguem_a_escala() {
        // Quadrado de (310, 210) a (340, 240) em pontos, numa tela a 2x: no arquivo, a
        // borda vai para (20, 20) pixels a partir do canto da área (300, 200).
        let screens = vec![screen("A", (0.0, 0.0), (2732, 1536), 2.0, [255, 255, 255], true)];
        let shape = Shape { tool: Tool::Square, color: [235, 52, 52], from: (310.0, 210.0), to: (340.0, 240.0) };
        let out = compose(&screens, (300.0, 200.0, 100.0, 100.0), &[shape]).unwrap();
        assert_eq!((out.width(), out.height()), (200, 200));
        assert!(rgb(&out, 20, 40)[1] < 100, "borda do quadrado ampliada");
        // O traço de 4 pontos vira 8 pixels (de 16 a 24); sem escala, iria de 18 a 22.
        assert!(rgb(&out, 17, 40)[1] < 100 && rgb(&out, 23, 40)[1] < 100, "traço engrossa com a escala");
        assert_eq!(rgb(&out, 27, 40), [255, 255, 255]);
        assert_eq!(rgb(&out, 60, 60), [255, 255, 255]);
    }

    #[test]
    fn tela_inteira_em_125_sem_linha_preta() {
        // 1366x768 em 1,25: o compositor dá 1093x615 pontos (para cima), que vezes 1,25
        // seriam 1366x769. O arquivo tem o tamanho da imagem, sem a linha preta no fim.
        assert_eq!(points_size((1366, 768), 1.25), (1093.0, 615.0));
        let s = screen("A", (0.0, 0.0), (1366, 768), 1.25, RED, true);
        let main = s.rect;
        let screens = vec![s];
        let out = compose(&screens, main, &[]).unwrap();
        assert_eq!((out.width(), out.height()), (1366, 768));
        assert_eq!(rgb(&out, 1365, 767), RED, "o canto de baixo não fica preto");
        // Em 1,5: 911 pontos de largura seriam 1367 pixels (coluna preta à direita).
        let s = screen("A", (0.0, 0.0), (1366, 768), 1.5, BLUE, true);
        let main = s.rect;
        let out = compose(&[s], main, &[]).unwrap();
        assert_eq!((out.width(), out.height()), (1366, 768));
        assert_eq!(rgb(&out, 1365, 767), BLUE);
        // Uma janela recortada no fim da tela também para na imagem.
        let window = clamp_rect((900.0, 500.0, 400.0, 300.0), bounds_of(&[screens[0].rect]));
        let out = compose(&screens, window, &[]).unwrap();
        assert_eq!((out.width(), out.height()), (1366 - 1125, 768 - 625));
        assert_eq!(rgb(&out, out.width() - 1, out.height() - 1), RED);
    }

    #[test]
    fn copia_com_tamanhos_diferentes() {
        // Camada com uma linha a mais que a imagem (compositor antigo): copia o que dá.
        let mut dst = Pixmap::new(10, 11).unwrap();
        let mut src = Pixmap::new(10, 10).unwrap();
        src.fill(Color::from_rgba8(255, 0, 0, 255));
        copy_rect(&mut dst, &src, (2.0, 2.0, 20.0, 20.0));
        assert_eq!(rgb(&dst, 5, 5), RED);
        assert_eq!(rgb(&dst, 9, 9), RED);
        assert_eq!(dst.pixel(1, 1).unwrap().alpha(), 0, "fora da área não copia");
        assert_eq!(dst.pixel(5, 10).unwrap().alpha(), 0, "linha que a imagem não tem");
    }

    #[test]
    fn alcas_nas_pontas_e_no_meio() {
        let r = (100.0, 100.0, 200.0, 100.0);
        let h = handles(r);
        let at = |which: Handle| h.iter().find(|(k, _)| *k == which).unwrap().1;
        assert_eq!(at(Handle::TopLeft), (100.0, 100.0));
        assert_eq!(at(Handle::Top), (200.0, 100.0));
        assert_eq!(at(Handle::Right), (300.0, 150.0));
        assert_eq!(at(Handle::BottomRight), (300.0, 200.0));
        assert_eq!(at(Handle::Bottom), (200.0, 200.0));
        assert_eq!(at(Handle::Left), (100.0, 150.0));
        // Pega até 10 pontos do centro; a 15, não.
        assert_eq!(handle_at(r, (308.0, 152.0)), Some(Handle::Right));
        assert_eq!(handle_at(r, (315.0, 150.0)), None);
        assert_eq!(handle_at(r, (200.0, 150.0)), None, "o meio da área não é bolinha");
        // Área pequena: a ponta vence o meio do lado.
        assert_eq!(handle_at((100.0, 100.0, 8.0, 8.0), (104.0, 100.0)), Some(Handle::TopLeft));
    }

    #[test]
    fn redimensionar() {
        let all = (0.0, 0.0, 3286.0, 1080.0);
        assert_eq!(resize_area((300.0, 200.0, 400.0, 250.0), Handle::BottomRight, (100.0, 50.0), all), (300.0, 200.0, 500.0, 300.0));
        assert_eq!(resize_area((100.0, 100.0, 200.0, 100.0), Handle::TopLeft, (-50.0, -40.0), all), (50.0, 60.0, 250.0, 140.0));
        // Meio do lado: só uma direção muda.
        assert_eq!(resize_area((300.0, 200.0, 400.0, 250.0), Handle::Bottom, (90.0, 50.0), all), (300.0, 200.0, 400.0, 300.0));
        // A bolinha da esquerda passa do lado direito: a área vira (normaliza).
        assert_eq!(resize_area((300.0, 200.0, 400.0, 250.0), Handle::Left, (700.0, 0.0), all), (700.0, 200.0, 300.0, 250.0));
        // Mínimo de 2 pontos.
        assert_eq!(resize_area((300.0, 200.0, 400.0, 250.0), Handle::Right, (-399.5, 0.0), all), (300.0, 200.0, 2.0, 250.0));
        // Não sai das telas.
        assert_eq!(resize_area((3000.0, 900.0, 200.0, 100.0), Handle::BottomRight, (500.0, 500.0), all), (3000.0, 900.0, 286.0, 180.0));
        assert_eq!(resize_area((10.0, 10.0, 100.0, 100.0), Handle::TopLeft, (-50.0, -50.0), all), (0.0, 0.0, 110.0, 110.0));
        // Encostada na borda e menor que 2: cabe dentro.
        assert_eq!(resize_area((3280.0, 0.0, 6.0, 10.0), Handle::Left, (10.0, 0.0), all), (3284.0, 0.0, 2.0, 10.0));
    }

    #[test]
    fn mover_fica_dentro() {
        let all = (0.0, 0.0, 3286.0, 1080.0);
        assert_eq!(moved((3200.0, 100.0, 100.0, 100.0), (200.0, 0.0), all), (3186.0, 100.0, 100.0, 100.0));
        assert_eq!(moved((300.0, 200.0, 400.0, 250.0), (100.0, 50.0), all), (400.0, 250.0, 400.0, 250.0));
        assert_eq!(moved((50.0, 50.0, 100.0, 100.0), (-80.0, -90.0), all), (0.0, 0.0, 100.0, 100.0));
        // De uma tela para a outra: atravessa sem problema.
        assert_eq!(moved((1200.0, 200.0, 100.0, 100.0), (300.0, 0.0), all), (1500.0, 200.0, 100.0, 100.0));
        // Limites de uma tela só (a gravação): fica nela.
        assert_eq!(moved((1200.0, 200.0, 100.0, 100.0), (300.0, 0.0), (0.0, 0.0, 1366.0, 768.0)), (1266.0, 200.0, 100.0, 100.0));
        assert_eq!(clamp_point((-5.0, 2000.0), all), (0.0, 1080.0));
    }

    /// A (1366x768) e, à direita, B (1920x1080): embaixo de A, à esquerda de B, não há
    /// tela (o buraco).
    const DUAS: [RectF; 2] = [(0.0, 0.0, 1366.0, 768.0), (1366.0, 0.0, 1920.0, 1080.0)];

    #[test]
    fn mover_nao_cai_no_buraco() {
        // O caso do achado: área em B levada para o canto de baixo à esquerda. Antes
        // ficava em (1080, 780), quase toda no buraco; agora fica em B, encostada.
        let r = moved_in((1400.0, 600.0, 300.0, 300.0), (-320.0, 185.0), &DUAS);
        assert!(handles_on_screens(r, &DUAS), "bolinhas alcançáveis: {r:?}");
        assert_eq!(r, (1366.0, 780.0, 300.0, 300.0));
        // Atravessando as duas telas e levada para baixo: desliza para B (o lugar
        // válido mais perto), sem bolinha no buraco.
        let r = moved_in((1300.0, 300.0, 200.0, 100.0), (0.0, 600.0), &DUAS);
        assert_eq!(r, (1366.0, 900.0, 200.0, 100.0));
        // Atravessando e um pouco para baixo: para no fim de A (faixa comum).
        let r = moved_in((1300.0, 600.0, 200.0, 100.0), (0.0, 120.0), &DUAS);
        assert_eq!(r, (1300.0, 668.0, 200.0, 100.0));
        // Movimentos dentro da união não mudam.
        assert_eq!(moved_in((300.0, 200.0, 400.0, 250.0), (100.0, 50.0), &DUAS), (400.0, 250.0, 400.0, 250.0));
        assert_eq!(moved_in((1200.0, 200.0, 100.0, 100.0), (300.0, 0.0), &DUAS), (1500.0, 200.0, 100.0, 100.0));
        // Uma tela só (a gravação) continua igual a `moved`.
        let one = [DUAS[0]];
        assert_eq!(moved_in((1200.0, 200.0, 100.0, 100.0), (300.0, 0.0), &one), (1266.0, 200.0, 100.0, 100.0));
    }

    #[test]
    fn redimensionar_nao_cai_no_buraco() {
        // Bolinha de baixo à esquerda de uma área em B arrastada para o buraco: a área
        // cresce para a esquerda até onde as telas deixam (o fim de A embaixo), e todas
        // as bolinhas ficam em tela.
        let orig = (1400.0, 600.0, 300.0, 300.0);
        let r = resize_in(orig, Handle::BottomLeft, (-200.0, 200.0), &DUAS);
        assert!(handles_on_screens(r, &DUAS), "bolinhas alcançáveis: {r:?}");
        assert_eq!(r, (1200.0, 600.0, 500.0, 168.0));
        // Só para baixo, em B: vai até o fim de B (nada no buraco).
        let r = resize_in(orig, Handle::BottomLeft, (0.0, 400.0), &DUAS);
        assert_eq!(r, (1400.0, 600.0, 300.0, 480.0));
        // Bolinha de baixo de uma área que atravessa as telas: para no fim de A.
        let r = resize_in((1300.0, 300.0, 200.0, 100.0), Handle::Bottom, (0.0, 600.0), &DUAS);
        assert_eq!(r, (1300.0, 300.0, 200.0, 468.0));
        // Dentro da união, igual ao `resize_area`.
        let all = bounds_of(&DUAS);
        assert_eq!(resize_in((300.0, 200.0, 400.0, 250.0), Handle::BottomRight, (100.0, 50.0), &DUAS), resize_area((300.0, 200.0, 400.0, 250.0), Handle::BottomRight, (100.0, 50.0), all));
    }

    #[test]
    fn painel_sai_de_cima_das_bolinhas() {
        // Tela 1366x768, painel de 720x96 em x 323 e y 640 (o caso do achado: área de
        // (300, 200) a (700, 700) tem a borda de baixo embaixo do painel).
        let centers: Vec<(f32, f32)> = handles((300.0, 200.0, 400.0, 500.0)).iter().map(|(_, c)| *c).collect();
        let y = panel_spot(&centers, (323.0, 720.0, 96.0), 640.0);
        assert_eq!(y, 44.0, "vai para cima");
        // Área sem bolinhas na faixa do painel: fica embaixo.
        let centers: Vec<(f32, f32)> = handles((300.0, 200.0, 400.0, 300.0)).iter().map(|(_, c)| *c).collect();
        assert_eq!(panel_spot(&centers, (323.0, 720.0, 96.0), 640.0), 640.0);
        // Bolinhas embaixo e em cima: o primeiro lugar livre entre elas.
        let centers: Vec<(f32, f32)> = handles((400.0, 60.0, 500.0, 640.0)).iter().map(|(_, c)| *c).collect();
        let y = panel_spot(&centers, (323.0, 720.0, 96.0), 640.0);
        assert!(y > 72.0 && y + 96.0 < 380.0 - 10.0 || y > 380.0 + 12.0 && y + 96.0 < 700.0 - 12.0, "lugar livre: {y}");
    }

    #[test]
    fn bolinha_na_cor_de_destaque() {
        let mut canvas = Pixmap::new(300, 200).unwrap();
        canvas.fill(Color::from_rgba8(40, 40, 40, 255));
        let accent = [10, 110, 235, 255];
        draw_handles(&mut canvas, (50.0, 40.0, 200.0, 120.0), accent);
        for (x, y) in [(50, 40), (150, 40), (250, 100), (150, 160), (50, 160)] {
            assert_eq!(rgb(&canvas, x, y), [10, 110, 235], "centro da bolinha em ({x}, {y})");
        }
        // Anel branco em volta (raio 5).
        let ring = rgb(&canvas, 55, 40);
        assert!(ring[0] > 180 && ring[1] > 180, "anel branco: {ring:?}");
        // No meio da área, nada.
        assert_eq!(rgb(&canvas, 150, 100), [40, 40, 40]);
    }

    #[test]
    fn gravacao_presa_a_tela() {
        let screens = vec![screen("HEADLESS-1", (0.0, 0.0), (1366, 768), 1.0, RED, true), screen("HEADLESS-2", (1366.0, 0.0), (1920, 1080), 1.0, BLUE, false)];
        // Área na segunda tela: pontos locais dela.
        let (name, area) = record_target(&screens, (1500.0, 300.0, 300.0, 200.0), 1);
        assert_eq!((name.as_str(), area), ("HEADLESS-2", (134, 300, 300, 200)));
        assert_eq!(record_command(Some(area), Some(&name), true, false), "record start 134 300 300 200 tela HEADLESS-2 sistema");
        // Começou na principal e passou para a outra: fica recortada à principal.
        let (name, area) = record_target(&screens, (1200.0, 200.0, 366.0, 250.0), 0);
        assert_eq!((name.as_str(), area), ("HEADLESS-1", (1200, 200, 166, 250)));
        // Tela inteira, os dois sons; uma tela só (sem nome): o comando de antes.
        assert_eq!(record_command(None, Some("HEADLESS-2"), true, true), "record start full tela HEADLESS-2 sistema microfone");
        assert_eq!(record_command(Some((100, 100, 400, 300)), None, false, true), "record start 100 100 400 300 microfone");
        assert_eq!(record_command(None, Some(""), false, false), "record start full");
    }

    #[test]
    fn tela_sob_o_ponteiro() {
        assert_eq!(screen_index_at(&DUAS, (2000.0, 500.0)), Some(1));
        assert_eq!(screen_index_at(&DUAS, (100.0, 100.0)), Some(0));
        assert_eq!(screen_index_at(&DUAS, (1000.0, 900.0)), None, "buraco entre as telas");
        // A mais perto: (1000, 900) fica a 132 pontos de A e a 366 de B.
        assert_eq!(screen_index_near(&DUAS, (1000.0, 900.0)), 0);
        assert_eq!(screen_index_near(&DUAS, (1360.0, 1000.0)), 1);
        assert_eq!(screen_index_near(&[], (5.0, 5.0)), 0);
    }

    #[test]
    fn borda_comum_e_da_tela_que_comeca_nela() {
        // x = 1366 é a primeira coluna de B, não a última de A.
        assert_eq!(screen_index_at(&DUAS, (1366.0, 100.0)), Some(1));
        assert_eq!(screen_index_near(&DUAS, (1366.0, 100.0)), 1);
        assert_eq!(screen_index_at(&DUAS, (1365.9, 100.0)), Some(0));
        assert!(contains_half_open(DUAS[1], (1366.0, 0.0)));
        assert!(!contains_half_open(DUAS[0], (1366.0, 0.0)));
        // Empilhadas: y = 768 é a primeira linha da de baixo.
        let pilha: [RectF; 2] = [(0.0, 0.0, 1366.0, 768.0), (0.0, 768.0, 1366.0, 768.0)];
        assert_eq!(screen_index_at(&pilha, (10.0, 768.0)), Some(1));
        assert_eq!(screen_index_near(&pilha, (10.0, 768.0)), 1);
        // Fora de tudo, à direita da última coluna de B: a mais perto continua sendo B.
        assert_eq!(screen_index_at(&DUAS, (3286.0, 100.0)), None);
        assert_eq!(screen_index_near(&DUAS, (3286.0, 100.0)), 1);
    }

    #[test]
    fn tela_inteira_com_varias_telas_pergunta_qual() {
        let one = vec![screen("A", (0.0, 0.0), (1366, 768), 1.0, RED, true)];
        let mut c = Capture::new(one, Vec::new());
        c.choose_full();
        assert_eq!(c.mode, Mode::Full, "uma tela: como antes");
        let two = vec![screen("A", (0.0, 0.0), (1366, 768), 1.0, RED, true), screen("B", (1366.0, 0.0), (2400, 1350), 1.25, BLUE, false)];
        let mut c = Capture::new(two, Vec::new());
        c.selection = Some((10.0, 10.0, 50.0, 50.0));
        c.choose_full();
        assert_eq!((c.mode, c.selection), (Mode::PickScreen, None));
        assert_eq!(c.full_index(), 0, "antes de escolher, a principal");
        c.pick_screen(1);
        assert_eq!((c.mode, c.selection, c.full_index()), (Mode::Full, Some((1366.0, 0.0, 1920.0, 1080.0)), 1));
        // O arquivo sai em pixels da tela escolhida.
        let out = c.result().unwrap();
        assert_eq!((out.width(), out.height()), (2400, 1350));
        assert_eq!(rgb(&out, 5, 5), BLUE);
    }

    #[test]
    fn nome_do_arquivo_com_a_tela() {
        assert_eq!(shot_file_name("2026-10-23 10-00-00", None), "Captura de tela 2026-10-23 10-00-00.png");
        assert_eq!(shot_file_name("2026-10-23 10-00-00", Some("HDMI-A-1")), "Captura de tela 2026-10-23 10-00-00 (HDMI-A-1).png");
        assert_eq!(shot_file_name("x", Some("")), "Captura de tela x.png");
    }

    #[test]
    fn restos_apagados_pelo_prefixo() {
        let dir = std::env::temp_dir().join(format!("mide-teste-restos-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["mide-tela-7-HEADLESS-1.png", "mide-tela-7-DP-1.png", "mide-tela-70-HEADLESS-1.png", "outro.png"] {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        remove_stale(&dir.join("mide-tela-7"));
        let mut left: Vec<String> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        left.sort();
        assert_eq!(left, ["mide-tela-70-HEADLESS-1.png", "outro.png"]);
        for name in &left {
            std::fs::remove_file(dir.join(name)).unwrap();
        }
        std::fs::remove_dir(&dir).unwrap();
    }
}
