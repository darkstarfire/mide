//! Divisão do editor do `mide-code` em dois painéis (lado a lado ou um em cima do
//! outro) e onde soltar um arquivo ou aba arrastado: perto de uma borda do editor
//! divide para aquele lado; no meio abre no painel. Sem interface (testável).

/// Retângulo (x, y, largura, altura).
pub type Rect = (f32, f32, f32, f32);

/// Borda do editor onde algo foi solto.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Left,
    Right,
    Top,
    Bottom,
}

impl Edge {
    /// Lado a lado (borda esquerda/direita) ou um em cima do outro.
    pub fn side_by_side(self) -> bool {
        matches!(self, Edge::Left | Edge::Right)
    }

    /// O painel novo fica no segundo lugar (direita/embaixo)?
    pub fn second(self) -> bool {
        matches!(self, Edge::Right | Edge::Bottom)
    }
}

/// Onde soltar: `Some(borda)` perto de uma borda (o quarto do tamanho mais perto
/// dela), `None` no meio (abre no painel) ou fora do retângulo.
pub fn drop_edge(rect: Rect, (px, py): (f32, f32)) -> Option<Edge> {
    let (x, y, w, h) = rect;
    if px < x || py < y || px > x + w || py > y + h || w <= 0.0 || h <= 0.0 {
        return None;
    }
    // Distância relativa de cada borda (0 na borda, 1 na oposta).
    let d = [(Edge::Left, (px - x) / w), (Edge::Right, (x + w - px) / w), (Edge::Top, (py - y) / h), (Edge::Bottom, (y + h - py) / h)];
    let (edge, dist) = d.into_iter().fold((Edge::Left, f32::MAX), |best, c| if c.1 < best.1 { c } else { best });
    (dist < 0.25).then_some(edge)
}

/// A metade (do retângulo) que um painel novo ocuparia na borda: para pintar a
/// prévia enquanto arrasta.
pub fn edge_preview(rect: Rect, edge: Edge) -> Rect {
    let (x, y, w, h) = rect;
    match edge {
        Edge::Left => (x, y, w / 2.0, h),
        Edge::Right => (x + w / 2.0, y, w / 2.0, h),
        Edge::Top => (x, y, w, h / 2.0),
        Edge::Bottom => (x, y + h / 2.0, w, h / 2.0),
    }
}

/// Os dois painéis de `rect` divididos em `ratio` (parte do primeiro), com `gap`
/// entre eles para a divisória.
pub fn split_rects(rect: Rect, side_by_side: bool, ratio: f32, gap: f32) -> (Rect, Rect) {
    let (x, y, w, h) = rect;
    let ratio = ratio.clamp(0.15, 0.85);
    if side_by_side {
        let a = ((w - gap) * ratio).round();
        ((x, y, a, h), (x + a + gap, y, w - a - gap, h))
    } else {
        let a = ((h - gap) * ratio).round();
        ((x, y, w, a), (x, y + a + gap, w, h - a - gap))
    }
}

/// Passo na lista de completar: Tab desce, Shift+Tab sobe, dando a volta.
pub fn cycle(sel: usize, len: usize, forward: bool) -> usize {
    if len == 0 {
        return 0;
    }
    if forward {
        (sel + 1) % len
    } else {
        (sel + len - 1) % len
    }
}

/// Primeira linha mostrada numa lista de `shown` linhas para `sel` ficar à vista.
pub fn window_start(sel: usize, len: usize, shown: usize) -> usize {
    if len <= shown {
        return 0;
    }
    sel.saturating_sub(shown - 1).min(len - shown)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bordas_para_soltar() {
        let r = (100.0, 50.0, 400.0, 200.0);
        assert_eq!(drop_edge(r, (110.0, 150.0)), Some(Edge::Left));
        assert_eq!(drop_edge(r, (490.0, 150.0)), Some(Edge::Right));
        assert_eq!(drop_edge(r, (300.0, 60.0)), Some(Edge::Top));
        assert_eq!(drop_edge(r, (300.0, 240.0)), Some(Edge::Bottom));
        assert_eq!(drop_edge(r, (300.0, 150.0)), None);
        assert_eq!(drop_edge(r, (50.0, 150.0)), None);
        assert!(Edge::Right.side_by_side() && Edge::Right.second());
        assert!(!Edge::Top.side_by_side() && !Edge::Top.second());
        assert_eq!(edge_preview(r, Edge::Bottom), (100.0, 150.0, 400.0, 100.0));
    }

    #[test]
    fn paineis_divididos() {
        let (a, b) = split_rects((0.0, 0.0, 806.0, 300.0), true, 0.5, 6.0);
        assert_eq!(a, (0.0, 0.0, 400.0, 300.0));
        assert_eq!(b, (406.0, 0.0, 400.0, 300.0));
        let (a, b) = split_rects((0.0, 10.0, 200.0, 106.0), false, 0.0, 6.0);
        assert_eq!(a.3, 15.0);
        assert_eq!(b.1, 10.0 + 15.0 + 6.0);
    }

    #[test]
    fn lista_de_completar() {
        assert_eq!(cycle(0, 3, true), 1);
        assert_eq!(cycle(2, 3, true), 0);
        assert_eq!(cycle(0, 3, false), 2);
        assert_eq!(cycle(0, 0, false), 0);
        assert_eq!(window_start(0, 30, 10), 0);
        assert_eq!(window_start(12, 30, 10), 3);
        assert_eq!(window_start(29, 30, 10), 20);
        assert_eq!(window_start(5, 4, 10), 0);
    }
}
