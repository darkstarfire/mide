//! Seletor de lista do mide-ui: um diálogo com busca para escolher um item de uma lista
//! longa em grupos (as unidades da Calculadora, em categorias). Sem busca, mostra os
//! itens do grupo atual, com "‹ Todos os grupos" no topo; com busca, uma lista só, sem
//! acento nem maiúscula. ↑/↓ andam, Enter escolhe o marcado, Esc e "Cancelar" fecham.
//!
//! Como o `FileChooser`, o app guarda o seletor enquanto ele está aberto e chama `show`
//! a cada quadro, por cima do resto:
//!
//! ```ignore
//! if let Some(picker) = self.escolher.as_mut() {
//!     match picker.show(ui) {
//!         Some(Picked::Chosen { group, item }) => { self.usar(group, item); self.escolher = None; }
//!         Some(Picked::Cancelled) => self.escolher = None,
//!         None => {}
//!     }
//! }
//! ```
//!
//! O seletor guarda cópias (`String`) dos nomes: assim ele não empresta o estado do app,
//! que continua mudando enquanto o diálogo está aberto.

use std::time::{Duration, Instant};

use super::{input::Key, Ui};
use crate::apps::fold;

/// Depois de um clique que troca a lista (um grupo, "‹ Todos"), os cliques nos itens
/// esperam isto: o segundo clique de um duplo clique não escolhe o item que caiu sob o
/// ponteiro (é o intervalo do duplo clique do mide-ui).
const SETTLE: Duration = Duration::from_millis(450);

/// Um item da lista.
#[derive(Debug, Clone, PartialEq)]
pub struct PickItem {
    pub label: String,
    /// Texto cinza à direita ("cm", "2,42 ha").
    pub detail: String,
    /// Palavras exatas do item (símbolo, apelidos): a busca igual a uma delas põe o
    /// item primeiro ("MB" é o megabyte, não o "mbar" que só contém "mb").
    pub keys: Vec<String>,
    /// Tudo o que a busca procura (rótulo, detalhe e chaves), sem acento nem maiúscula.
    pub search: String,
    pub enabled: bool,
}

impl PickItem {
    /// Item com o rótulo, o detalhe e as palavras exatas (símbolo, apelidos).
    pub fn new(label: &str, detail: &str, keys: &[&str]) -> Self {
        Self {
            label: label.into(),
            detail: detail.into(),
            keys: keys.iter().map(|k| k.to_string()).collect(),
            search: fold(&format!("{label} {detail} {}", keys.join(" "))),
            enabled: true,
        }
    }
}

/// Um grupo de itens (uma categoria).
#[derive(Debug, Clone, PartialEq)]
pub struct PickGroup {
    pub name: String,
    pub items: Vec<PickItem>,
}

/// Resultado do seletor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Picked {
    Chosen { group: usize, item: usize },
    Cancelled,
}

/// Uma linha da lista mostrada.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Row {
    /// "‹ Todos os grupos".
    AllGroups,
    Group(usize),
    Item(usize, usize),
}

/// O que a lista mostra sem busca.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum View {
    Group(usize),
    Groups,
}

/// Itens que casam com a busca (todas as palavras, no item ou no nome do grupo), do
/// melhor para o pior: chave igual com maiúsculas ("MB"); rótulo ou chave igual sem
/// acento; rótulo ou chave que começa com a busca; e o resto. Grupos na ordem.
pub fn filter(groups: &[PickGroup], query: &str) -> Vec<(usize, usize)> {
    let raw = query.trim();
    let q = fold(raw);
    let words: Vec<&str> = q.split_whitespace().collect();
    if words.is_empty() {
        return Vec::new();
    }
    let mut found: Vec<(u8, usize, usize)> = Vec::new();
    for (g, group) in groups.iter().enumerate() {
        let group_name = fold(&group.name);
        for (i, item) in group.items.iter().enumerate() {
            if !words.iter().all(|w| item.search.contains(w) || group_name.contains(w)) {
                continue;
            }
            let names = || std::iter::once(fold(&item.label)).chain(item.keys.iter().map(|k| fold(k)));
            let rank = if item.keys.iter().any(|k| k == raw) {
                0
            } else if names().any(|n| n == q) {
                1
            } else if names().any(|n| n.starts_with(&q)) {
                2
            } else {
                3
            };
            found.push((rank, g, i));
        }
    }
    found.sort_by_key(|&(rank, g, i)| (rank, g, i));
    found.into_iter().map(|(_, g, i)| (g, i)).collect()
}

pub struct ListPicker {
    title: String,
    /// Rótulo da linha que mostra os grupos ("Todas as categorias").
    all_label: String,
    placeholder: String,
    groups: Vec<PickGroup>,
    current: (usize, usize),
    view: View,
    query: String,
    /// Linha marcada (a pílula azul): a que o Enter escolhe.
    marked: Option<usize>,
    /// A marca mudou (abriu, ↑/↓, lista nova): a lista rola até ela uma vez; depois a
    /// roda e a barra de rolagem ficam livres.
    reveal: bool,
    /// Pede o foco no campo de busca (ao abrir e depois de um clique que troca a lista).
    focus: bool,
    /// O botão do mouse já foi solto desde que o seletor abriu: o clique que abriu o
    /// seletor não escolhe nada nele.
    armed: bool,
    /// Quando um clique trocou a lista (ver `SETTLE`).
    changed_at: Option<Instant>,
}

impl ListPicker {
    /// `current`: o item escolhido agora (grupo, item), que abre marcado.
    pub fn new(title: &str, all_label: &str, placeholder: &str, groups: Vec<PickGroup>, current: (usize, usize)) -> Self {
        let mut picker = Self {
            title: title.into(),
            all_label: all_label.into(),
            placeholder: placeholder.into(),
            groups,
            current,
            view: View::Group(current.0),
            query: String::new(),
            marked: None,
            reveal: true,
            focus: true,
            armed: false,
            changed_at: None,
        };
        picker.mark_default();
        picker
    }

    /// As linhas mostradas agora.
    fn rows(&self) -> Vec<Row> {
        if !self.query.trim().is_empty() {
            return filter(&self.groups, &self.query).into_iter().map(|(g, i)| Row::Item(g, i)).collect();
        }
        match self.view {
            View::Groups => (0..self.groups.len()).map(Row::Group).collect(),
            View::Group(g) => {
                let n = self.groups.get(g).map_or(0, |group| group.items.len());
                std::iter::once(Row::AllGroups).chain((0..n).map(|i| Row::Item(g, i))).collect()
            }
        }
    }

    /// A linha conta como escolha possível (itens ativos e as linhas de navegação).
    fn usable(&self, row: Row) -> bool {
        match row {
            Row::Item(g, i) => self.groups[g].items[i].enabled,
            _ => true,
        }
    }

    /// Marca a linha que o Enter deve escolher na lista de agora. Na busca, o melhor
    /// resultado (o primeiro usável), mesmo que o item atual também apareça ("metro"
    /// acha os Centímetros, mas o Enter escolhe os Metros); sem busca, o item atual (ou
    /// o grupo dele), se está na lista, e senão o primeiro item usável.
    fn mark_default(&mut self) {
        let rows = self.rows();
        let searching = !self.query.trim().is_empty();
        let current = || rows.iter().position(|r| *r == Row::Item(self.current.0, self.current.1) || *r == Row::Group(self.current.0));
        let first = || rows.iter().position(|r| matches!(r, Row::Item(..)) && self.usable(*r));
        self.marked = if searching { first() } else { current().or_else(first) };
        self.reveal = true;
    }

    /// Escolher a linha: um item fecha o seletor; as outras trocam a lista.
    fn activate(&mut self, row: Row) -> Option<Picked> {
        match row {
            Row::Item(group, item) if self.groups[group].items[item].enabled => Some(Picked::Chosen { group, item }),
            Row::Item(..) => None,
            Row::AllGroups => {
                self.view = View::Groups;
                self.mark_default();
                None
            }
            Row::Group(g) => {
                self.view = View::Group(g);
                self.mark_default();
                None
            }
        }
    }

    /// Desenha o diálogo; `Some` quando escolheu ou cancelou.
    pub fn show(&mut self, ui: &mut Ui) -> Option<Picked> {
        let keys = ui.input().keys.clone();
        let (esc, enter) = (keys.contains(&Key::Escape), keys.contains(&Key::Enter));
        // ↑/↓ andam pelas linhas usáveis.
        let rows = self.rows();
        for key in &keys {
            let step: isize = match key {
                Key::Down => 1,
                Key::Up => -1,
                _ => continue,
            };
            let mut at = self.marked.map_or(if step > 0 { -1 } else { rows.len() as isize }, |m| m as isize);
            loop {
                at += step;
                if at < 0 || at >= rows.len() as isize {
                    break;
                }
                if self.usable(rows[at as usize]) {
                    self.marked = Some(at as usize);
                    self.reveal = true;
                    break;
                }
            }
        }
        let clicks_ok = self.armed && self.changed_at.is_none_or(|t| t.elapsed() >= SETTLE);
        if !ui.input().down {
            self.armed = true;
        }
        let t = ui.theme().clone();
        let pitch = t.row + t.spacing;
        let (ww, wh) = ui.window_size();
        let width = (ww - 32.0).min(360.0);
        let list_h = (wh * 0.6 - 90.0).clamp(4.0 * pitch, 12.0 * pitch);
        let mut clicked: Option<Row> = None;
        let mut cancel = false;
        let mut submitted = false;
        let title = self.title.clone();
        let query_before = self.query.clone();
        let searching = !self.query.trim().is_empty();
        let marked = self.marked;
        // A marca nova fica à vista uma vez; depois a rolagem é de quem usa.
        let keep = if std::mem::take(&mut self.reveal) { Some(marked.map_or((0.0, 0.0), |m| (m as f32 * pitch, m as f32 * pitch + t.row))) } else { None };
        let all_label = self.all_label.clone();
        let placeholder = self.placeholder.clone();
        let groups = &self.groups;
        let focus = std::mem::take(&mut self.focus);
        let query = &mut self.query;
        ui.modal(width, |ui| {
            // Título e "Cancelar" (numa janela estreita, o título encolhe com "…").
            let (_, _, inner, _) = ui.region();
            ui.row(|ui| {
                ui.sized((inner - 100.0).max(40.0), |ui| ui.strong(&title));
                ui.push_right(92.0);
                ui.sized(92.0, |ui| {
                    if ui.button("Cancelar").clicked() {
                        cancel = true;
                    }
                });
            });
            if focus {
                ui.focus_next_input();
            }
            if ui.text_input(query, &placeholder).submitted() {
                submitted = true;
            }
            ui.scroll_area_showing(list_h, keep, |ui| {
                if rows.is_empty() {
                    ui.note("Nada encontrado.");
                }
                for (k, row) in rows.iter().enumerate() {
                    let selected = marked == Some(k);
                    let r = match *row {
                        Row::AllGroups => ui.choice_item(&format!("‹ {all_label}"), "", selected, true),
                        Row::Group(g) => {
                            let count = groups[g].items.len().to_string();
                            ui.choice_item(&groups[g].name, &count, selected, true)
                        }
                        Row::Item(g, i) => {
                            // Na busca, o detalhe é o grupo (resultados de grupos diferentes).
                            let item = &groups[g].items[i];
                            let detail = if searching { &groups[g].name } else { &item.detail };
                            ui.choice_item(&item.label, detail, selected, item.enabled)
                        }
                    };
                    if r.clicked() {
                        clicked = Some(*row);
                    }
                }
            });
            // O painel cresce um pouco embaixo: a última linha não sai cortada.
            ui.space(t.spacing);
        });
        // A busca mudou: a marca vai para o que o Enter vai escolher, no topo da lista.
        if self.query != query_before {
            self.mark_default();
        }
        ui.request_repaint();
        let result = if cancel || esc {
            Some(Picked::Cancelled)
        } else if let Some(row) = clicked.filter(|r| clicks_ok || !matches!(r, Row::Item(..))) {
            if !matches!(row, Row::Item(..)) {
                self.changed_at = Some(Instant::now());
                self.focus = true;
            }
            self.activate(row)
        } else if submitted || enter {
            let rows = self.rows();
            self.marked.and_then(|m| rows.get(m).copied()).and_then(|row| self.activate(row))
        } else {
            None
        };
        // Fechou: o campo de busca devolve as teclas ao app.
        if result.is_some() {
            ui.blur();
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::{theme::Theme, ui::Ctx};
    use tiny_skia::Pixmap;

    fn groups() -> Vec<PickGroup> {
        vec![
            PickGroup { name: "Comprimento".into(), items: vec![PickItem::new("Metros", "m", &["m"]), PickItem::new("Milhas", "mi", &["mi", "milha"]), PickItem::new("Milímetros", "mm", &["mm"])] },
            PickGroup { name: "Massa".into(), items: vec![PickItem::new("Miligramas", "mg", &["mg"]), PickItem { enabled: false, ..PickItem::new("Arrobas", "@", &["@"]) }] },
            PickGroup {
                name: "Pressão".into(),
                items: vec![PickItem::new("Hectopascals", "hPa", &["hPa", "mbar"]), PickItem::new("Megabytes por algo", "MB", &["MB"])],
            },
        ]
    }

    /// Um quadro do seletor numa janela de 400x400.
    fn frame(ctx: &mut Ctx, picker: &mut ListPicker) -> Option<Picked> {
        let mut canvas = Pixmap::new(400, 400).expect("imagem");
        ctx.window = (400, 400);
        ctx.begin_frame();
        let r = {
            let mut ui = Ui::root(&mut canvas, ctx);
            picker.show(&mut ui)
        };
        ctx.end_frame();
        r
    }

    #[test]
    fn busca_sem_acento_e_todas_as_palavras() {
        let g = groups();
        assert_eq!(filter(&g, "mili"), [(0, 2), (1, 0)]);
        assert_eq!(filter(&g, "MILÍMETROS"), [(0, 2)]);
        // A chave igual vem primeiro (Milhas, "mi").
        assert_eq!(filter(&g, "mi"), [(0, 1), (0, 2), (1, 0)]);
        assert_eq!(filter(&g, "massa mili"), [(1, 0)]);
        // Com maiúsculas: "MB" é a chave exata, antes do "mbar" que só contém "mb".
        assert_eq!(filter(&g, "MB").first(), Some(&(2, 1)));
        assert!(filter(&g, "xyz").is_empty());
        assert!(filter(&g, "  ").is_empty());
    }

    #[test]
    fn teclado_escolhe_e_cancela() {
        let mut ctx = Ctx::new(Theme::default());
        // Abre com o atual marcado; Enter escolhe ele.
        let mut p = ListPicker::new("Unidade", "Todas", "Buscar", groups(), (0, 1));
        assert_eq!(frame(&mut ctx, &mut p), None);
        ctx.input.keys.push(Key::Enter);
        assert_eq!(frame(&mut ctx, &mut p), Some(Picked::Chosen { group: 0, item: 1 }));
        // Busca "mili": a marca vai para o primeiro resultado; ↓ e Enter, o segundo.
        let mut p = ListPicker::new("Unidade", "Todas", "Buscar", groups(), (0, 0));
        frame(&mut ctx, &mut p);
        ctx.input.text = "mili".into();
        frame(&mut ctx, &mut p);
        ctx.input.keys.push(Key::Down);
        frame(&mut ctx, &mut p);
        ctx.input.keys.push(Key::Enter);
        assert_eq!(frame(&mut ctx, &mut p), Some(Picked::Chosen { group: 1, item: 0 }));
        // Enter logo depois de buscar escolhe o primeiro resultado.
        let mut p = ListPicker::new("Unidade", "Todas", "Buscar", groups(), (0, 0));
        frame(&mut ctx, &mut p);
        ctx.input.text = "mil".into();
        frame(&mut ctx, &mut p);
        ctx.input.keys.push(Key::Enter);
        assert_eq!(frame(&mut ctx, &mut p), Some(Picked::Chosen { group: 0, item: 1 }));
        // O atual também casa com a busca, mas o Enter escolhe o melhor resultado ("mi"
        // é a chave das Milhas, não os Milímetros abertos agora).
        let mut p = ListPicker::new("Unidade", "Todas", "Buscar", groups(), (0, 2));
        frame(&mut ctx, &mut p);
        ctx.input.text = "mi".into();
        frame(&mut ctx, &mut p);
        ctx.input.keys.push(Key::Enter);
        assert_eq!(frame(&mut ctx, &mut p), Some(Picked::Chosen { group: 0, item: 1 }));
        // Esc cancela.
        let mut p = ListPicker::new("Unidade", "Todas", "Buscar", groups(), (0, 0));
        frame(&mut ctx, &mut p);
        ctx.input.keys.push(Key::Escape);
        assert_eq!(frame(&mut ctx, &mut p), Some(Picked::Cancelled));
    }

    #[test]
    fn item_desativado_nao_e_escolhido() {
        let mut ctx = Ctx::new(Theme::default());
        let mut p = ListPicker::new("Unidade", "Todas", "Buscar", groups(), (1, 0));
        frame(&mut ctx, &mut p);
        // ↓ pula as Arrobas (desativadas): a marca fica nos Miligramas.
        ctx.input.keys.push(Key::Down);
        frame(&mut ctx, &mut p);
        ctx.input.keys.push(Key::Enter);
        assert_eq!(frame(&mut ctx, &mut p), Some(Picked::Chosen { group: 1, item: 0 }));
        // Buscando só as arrobas: nada para escolher.
        let mut p = ListPicker::new("Unidade", "Todas", "Buscar", groups(), (1, 0));
        frame(&mut ctx, &mut p);
        ctx.input.text = "arroba".into();
        frame(&mut ctx, &mut p);
        ctx.input.keys.push(Key::Enter);
        assert_eq!(frame(&mut ctx, &mut p), None);
    }

    #[test]
    fn todos_os_grupos_marca_o_grupo_atual() {
        let mut p = ListPicker::new("Unidade", "Todas", "Buscar", groups(), (1, 0));
        assert_eq!(p.activate(Row::AllGroups), None);
        assert_eq!(p.rows(), [Row::Group(0), Row::Group(1), Row::Group(2)]);
        assert_eq!(p.marked, Some(1));
        // Entrar em outro grupo: marca o primeiro item usável dele.
        assert_eq!(p.activate(Row::Group(2)), None);
        assert_eq!(p.marked, Some(1));
    }
}
