//! Painéis de uma aba do terminal: uma árvore em que cada folha é um painel e cada
//! nó divide a área em dois, lado a lado ou um em cima do outro, numa proporção que
//! o usuário ajusta arrastando o divisor.

pub type PaneId = u32;

/// Retângulo (x, y, largura, altura).
pub type Rect = (f32, f32, f32, f32);

/// Espaço entre dois painéis (onde fica a linha divisória).
pub const GAP: f32 = 1.0;
/// Menor proporção de um lado (os painéis não somem ao arrastar).
const MIN_RATIO: f32 = 0.1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Split {
    /// Lado a lado (divisor vertical).
    Side,
    /// Um em cima do outro (divisor horizontal).
    Stack,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Layout {
    Pane(PaneId),
    Split { dir: Split, ratio: f32, a: Box<Layout>, b: Box<Layout> },
}

/// Um divisor na tela: onde ele está, o caminho até o nó dele e a área do nó.
#[derive(Debug, Clone, PartialEq)]
pub struct Divider {
    pub rect: Rect,
    pub dir: Split,
    pub path: Vec<bool>,
    pub area: Rect,
}

impl Layout {
    /// Painéis na ordem de leitura (esquerda para a direita, de cima para baixo).
    pub fn panes(&self) -> Vec<PaneId> {
        match self {
            Layout::Pane(id) => vec![*id],
            Layout::Split { a, b, .. } => {
                let mut v = a.panes();
                v.extend(b.panes());
                v
            }
        }
    }

    /// Divide o painel `target`: ele fica no primeiro lado e `new` no segundo.
    pub fn split(&mut self, target: PaneId, new: PaneId, dir: Split) -> bool {
        match self {
            Layout::Pane(id) if *id == target => {
                *self = Layout::Split { dir, ratio: 0.5, a: Box::new(Layout::Pane(target)), b: Box::new(Layout::Pane(new)) };
                true
            }
            Layout::Pane(_) => false,
            Layout::Split { a, b, .. } => a.split(target, new, dir) || b.split(target, new, dir),
        }
    }

    /// Tira o painel `target`; o outro lado ocupa o lugar do nó. Não tira o último
    /// painel (quem chama fecha a aba).
    pub fn remove(&mut self, target: PaneId) -> bool {
        let Layout::Split { a, b, .. } = self else {
            return false;
        };
        if **a == Layout::Pane(target) {
            *self = std::mem::replace(&mut **b, Layout::Pane(0));
            return true;
        }
        if **b == Layout::Pane(target) {
            *self = std::mem::replace(&mut **a, Layout::Pane(0));
            return true;
        }
        a.remove(target) || b.remove(target)
    }

    /// Área de cada painel dentro de `area`.
    pub fn rects(&self, area: Rect) -> Vec<(PaneId, Rect)> {
        let mut out = Vec::new();
        self.walk(area, &mut Vec::new(), &mut |node, rect, _| {
            if let Layout::Pane(id) = node {
                out.push((*id, rect));
            }
        });
        out
    }

    /// Divisores dentro de `area` (para desenhar e arrastar).
    pub fn dividers(&self, area: Rect) -> Vec<Divider> {
        let mut out = Vec::new();
        self.walk(area, &mut Vec::new(), &mut |node, rect, path| {
            if let Layout::Split { dir, ratio, .. } = node {
                let (first, _) = halves(rect, *dir, *ratio);
                let line = match dir {
                    Split::Side => (first.0 + first.2, rect.1, GAP, rect.3),
                    Split::Stack => (rect.0, first.1 + first.3, rect.2, GAP),
                };
                out.push(Divider { rect: line, dir: *dir, path: path.to_vec(), area: rect });
            }
        });
        out
    }

    fn walk(&self, area: Rect, path: &mut Vec<bool>, f: &mut impl FnMut(&Layout, Rect, &[bool])) {
        f(self, area, path);
        if let Layout::Split { dir, ratio, a, b } = self {
            let (ra, rb) = halves(area, *dir, *ratio);
            path.push(false);
            a.walk(ra, path, f);
            path.pop();
            path.push(true);
            b.walk(rb, path, f);
            path.pop();
        }
    }

    /// Muda a proporção do nó em `path`.
    pub fn set_ratio(&mut self, path: &[bool], ratio: f32) {
        match (self, path.split_first()) {
            (Layout::Split { ratio: r, .. }, None) => *r = ratio.clamp(MIN_RATIO, 1.0 - MIN_RATIO),
            (Layout::Split { a, b, .. }, Some((side, rest))) => {
                if *side { b.set_ratio(rest, ratio) } else { a.set_ratio(rest, ratio) }
            }
            _ => {}
        }
    }

    /// Volta todas as divisões ao meio.
    pub fn equalize(&mut self) {
        if let Layout::Split { ratio, a, b, .. } = self {
            *ratio = 0.5;
            a.equalize();
            b.equalize();
        }
    }

    /// Painel vizinho de `from` na direção pedida (o mais perto, com sobreposição).
    pub fn neighbor(&self, area: Rect, from: PaneId, direction: Direction) -> Option<PaneId> {
        let rects = self.rects(area);
        let (_, r) = *rects.iter().find(|(id, _)| *id == from)?;
        let overlap = |a0: f32, a1: f32, b0: f32, b1: f32| a0.max(b0) < a1.min(b1);
        rects
            .iter()
            .filter(|(id, _)| *id != from)
            .filter_map(|&(id, o)| {
                let distance = match direction {
                    Direction::Left if o.0 + o.2 <= r.0 + 0.5 && overlap(r.1, r.1 + r.3, o.1, o.1 + o.3) => r.0 - (o.0 + o.2),
                    Direction::Right if o.0 >= r.0 + r.2 - 0.5 && overlap(r.1, r.1 + r.3, o.1, o.1 + o.3) => o.0 - (r.0 + r.2),
                    Direction::Up if o.1 + o.3 <= r.1 + 0.5 && overlap(r.0, r.0 + r.2, o.0, o.0 + o.2) => r.1 - (o.1 + o.3),
                    Direction::Down if o.1 >= r.1 + r.3 - 0.5 && overlap(r.0, r.0 + r.2, o.0, o.0 + o.2) => o.1 - (r.1 + r.3),
                    _ => return None,
                };
                Some((distance, id))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, id)| id)
    }
}

/// As duas metades de `area` (sem o espaço do divisor), em pixels inteiros.
fn halves(area: Rect, dir: Split, ratio: f32) -> (Rect, Rect) {
    let (x, y, w, h) = area;
    match dir {
        Split::Side => {
            let first = ((w - GAP) * ratio).round().max(0.0);
            ((x, y, first, h), (x + first + GAP, y, (w - first - GAP).max(0.0), h))
        }
        Split::Stack => {
            let first = ((h - GAP) * ratio).round().max(0.0);
            ((x, y, w, first), (x, y + first + GAP, w, (h - first - GAP).max(0.0)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dividir_e_fechar() {
        let mut l = Layout::Pane(1);
        assert!(l.split(1, 2, Split::Side));
        assert!(l.split(2, 3, Split::Stack));
        assert_eq!(l.panes(), vec![1, 2, 3]);
        assert!(l.remove(2));
        assert_eq!(l.panes(), vec![1, 3]);
        assert!(l.remove(1));
        assert_eq!(l, Layout::Pane(3));
        assert!(!l.remove(3));
    }

    #[test]
    fn areas_e_divisores() {
        let mut l = Layout::Pane(1);
        l.split(1, 2, Split::Side);
        let rects = l.rects((0.0, 0.0, 201.0, 100.0));
        assert_eq!(rects, vec![(1, (0.0, 0.0, 100.0, 100.0)), (2, (101.0, 0.0, 100.0, 100.0))]);
        let d = l.dividers((0.0, 0.0, 201.0, 100.0));
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].rect, (100.0, 0.0, 1.0, 100.0));
        l.set_ratio(&d[0].path, 0.99);
        assert_eq!(l.rects((0.0, 0.0, 201.0, 100.0))[0].1 .2, 180.0);
        l.equalize();
        assert_eq!(l.rects((0.0, 0.0, 201.0, 100.0))[0].1 .2, 100.0);
    }

    #[test]
    fn vizinhos_por_direcao() {
        // [1 | 2]
        //     [3]
        let mut l = Layout::Pane(1);
        l.split(1, 2, Split::Side);
        l.split(2, 3, Split::Stack);
        let area = (0.0, 0.0, 201.0, 201.0);
        assert_eq!(l.neighbor(area, 1, Direction::Right), Some(2));
        assert_eq!(l.neighbor(area, 3, Direction::Up), Some(2));
        assert_eq!(l.neighbor(area, 3, Direction::Left), Some(1));
        assert_eq!(l.neighbor(area, 1, Direction::Left), None);
    }
}
