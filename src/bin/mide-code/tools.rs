//! Ferramentas do Code inspiradas no VS Code e no Zed: paleta de comandos
//! (Ctrl+Shift+P), painel de problemas (Ctrl+Shift+M), documentação ao parar o
//! ponteiro sobre um símbolo e o Git na margem.

use super::*;
use mide::code::gitdiff::{self, LineChange};

/// Git de um arquivo: o texto do último commit (`None`: fora do Git), a versão do
/// texto comparada e as linhas mudadas.
pub(crate) type GitState = (Option<String>, u64, Vec<(usize, LineChange)>);

/// Estado das ferramentas.
#[derive(Default)]
pub(crate) struct Tools {
    /// Ponteiro parado sobre o texto: onde, desde quando e se já pediu a dica.
    pub rest: Option<((f32, f32), Instant, bool)>,
    /// Dica do ponteiro: o pedido, onde mostrar e o texto (vazio: esperando).
    pub mouse_hover: Option<(u64, (f32, f32), String)>,
    /// Git de cada arquivo: o texto do último commit (`None`: fora do Git), a versão do
    /// texto comparada e as linhas mudadas.
    pub git: HashMap<PathBuf, GitState>,
    /// Trechos (snippets) lidos de cada linguagem.
    pub snippets: HashMap<&'static str, Vec<mide::code::snippets::Snippet>>,
}

/// Um comando da paleta: ação, nome (com o menu) e atalho.
pub(crate) struct PaletteItem {
    pub action: String,
    pub label: String,
    pub shortcut: String,
}

fn flatten(group: &str, items: &[MenuItem], out: &mut Vec<PaletteItem>) {
    for item in items {
        match item {
            MenuItem::Action { action, label, shortcut, enabled: true, .. } => {
                // Abas e janelas mudam a toda hora: ficam no menu Janela.
                if action.starts_with("tab:") {
                    continue;
                }
                out.push(PaletteItem { action: action.clone(), label: format!("{group}: {label}"), shortcut: shortcut.iter().map(|k| if k.chars().count() == 1 { k.to_uppercase() } else { k.replace("Control", "Ctrl") }).collect::<Vec<_>>().join("+") });
            }
            MenuItem::Submenu { label, items } => flatten(&format!("{group}: {label}"), items, out),
            _ => {}
        }
    }
}

impl Code {
    /// Todos os comandos dos menus, para a paleta.
    pub(crate) fn palette_items(&mut self) -> Vec<PaletteItem> {
        let mut out = Vec::new();
        for group in self.menu_bar() {
            flatten(&group.title, &group.items, &mut out);
        }
        out
    }

    pub(crate) fn tool_overlays(&mut self, ui: &mut Ui, esc: bool) -> bool {
        match &mut self.overlay {
            Overlay::Palette { query, sel, focus, items } => {
                let (up, down, enter) = (ui.take_key(Key::Up), ui.take_key(Key::Down), ui.input().key(Key::Enter));
                let q = query.to_lowercase();
                let mut shown: Vec<(i32, &PaletteItem)> = items.iter().filter_map(|it| lsp_ui::fuzzy_score(&it.label.to_lowercase(), &q).map(|s| (s, it))).collect();
                if !q.is_empty() {
                    shown.sort_by_key(|a| std::cmp::Reverse(a.0));
                }
                if up {
                    *sel = sel.saturating_sub(1);
                }
                if down {
                    *sel = (*sel + 1).min(shown.len().saturating_sub(1));
                }
                *sel = (*sel).min(shown.len().saturating_sub(1));
                let mut chosen = enter.then(|| shown.get(*sel).map(|(_, it)| it.action.clone())).flatten();
                let first = panes::window_start(*sel, shown.len(), 14);
                ui.modal(620.0, |ui| {
                    if std::mem::take(focus) {
                        ui.focus_next_input();
                    }
                    ui.search_field(query, &tr!("Type a command"));
                    for (k, (_, it)) in shown.iter().enumerate().skip(first).take(14) {
                        if k == *sel {
                            ui.join_selection(false, false);
                        }
                        if ui.list_row("system-run-symbolic", &it.label, &[(&it.shortcut, 130.0)], k == *sel).clicked() {
                            chosen = Some(it.action.clone());
                        }
                    }
                    if shown.is_empty() {
                        ui.note(&tr!("No matching command."));
                    }
                });
                if let Some(action) = chosen {
                    self.overlay = Overlay::None;
                    self.command(&action, ui);
                } else if esc {
                    self.overlay = Overlay::None;
                }
                ui.request_repaint();
                true
            }
            Overlay::Problems { sel, go } => {
                // Os problemas de todas as abas abertas, erros primeiro.
                let mut all: Vec<(usize, &Diagnostic)> = self.docs.iter().enumerate().flat_map(|(i, d)| d.diags.iter().map(move |g| (i, g))).collect();
                all.sort_by_key(|(i, d)| (d.severity != Severity::Error, d.severity != Severity::Warning, *i, d.start));
                // As setas e o Enter foram lidos no começo do quadro (`overlay_keys`).
                let enter = std::mem::take(go);
                *sel = (*sel).min(all.len().saturating_sub(1));
                let mut chosen = enter.then(|| all.get(*sel).map(|(i, d)| (*i, d.start))).flatten();
                let first = panes::window_start(*sel, all.len(), 14);
                let names: Vec<String> = self.docs.iter().map(|d| d.name()).collect();
                ui.modal(720.0, |ui| {
                    ui.strong(&tr!("Problems"));
                    if all.is_empty() {
                        ui.note(&tr!("No problems in the open files."));
                    }
                    for (k, (i, d)) in all.iter().enumerate().skip(first).take(14) {
                        let icon = match d.severity {
                            Severity::Error => "dialog-error-symbolic",
                            Severity::Warning => "dialog-warning-symbolic",
                            Severity::Info => "dialog-information-symbolic",
                        };
                        let place = format!("{}:{}", names[*i], d.start.0 + 1);
                        if k == *sel {
                            ui.join_selection(false, false);
                        }
                        let msg = d.message.lines().next().unwrap_or_default();
                        if ui.list_row(icon, msg, &[(&place, 180.0)], k == *sel).clicked() {
                            chosen = Some((*i, d.start));
                        }
                    }
                });
                if let Some((i, (line, col))) = chosen {
                    self.overlay = Overlay::None;
                    self.cur = i;
                    let utf8 = self.client_utf8(self.docs[i].lang.id);
                    let doc = &mut self.docs[i].text;
                    let at = lsp::to_offset(&doc.text, line, col, utf8);
                    doc.edit = mide::kit::textedit::Edit { cursor: at, anchor: None };
                    self.start = true;
                } else if esc {
                    self.overlay = Overlay::None;
                }
                true
            }
            _ => false,
        }
    }

    /// Documentação ao parar o ponteiro sobre um símbolo (como no VS Code): depois de
    /// 0,7 s parado em cima do texto, pede a dica ao servidor; some ao mexer o ponteiro.
    pub(crate) fn mouse_hover(&mut self, ui: &mut Ui, area: (f32, f32, f32, f32)) {
        let Some((px, py)) = ui.pointer() else {
            self.tools.rest = None;
            return;
        };
        let inside = px >= area.0 && px < area.0 + area.2 && py >= area.1 && py < area.1 + area.3;
        let busy = !matches!(self.overlay, Overlay::None) || self.completion.is_some() || self.dialog.is_some() || ui.input().down;
        if !inside || busy {
            self.tools.rest = None;
            self.tools.mouse_hover = None;
            return;
        }
        let moved = self.tools.rest.is_none_or(|((x, y), _, _)| (x - px).abs() > 3.0 || (y - py).abs() > 3.0);
        if moved {
            // Andou: some a dica que estava longe e o relógio recomeça.
            if self.tools.mouse_hover.as_ref().is_some_and(|(_, (x, y), _)| (x - px).abs() > 24.0 || (y - py).abs() > 14.0) {
                self.tools.mouse_hover = None;
            }
            self.tools.rest = Some(((px, py), Instant::now(), false));
            ui.wake_after(Duration::from_millis(750));
        }
        if let Some((_, since, asked)) = &mut self.tools.rest
            && !*asked
            && since.elapsed() >= Duration::from_millis(700)
        {
            *asked = true;
            if let Some(at) = self.offset_at(px, py, area)
                && let Some(path) = self.doc().path.clone()
            {
                let text = self.doc().text.text.clone();
                self.sync_now();
                if let Some(client) = self.client_for_current().filter(|c| c.can("hoverProvider"))
                    && let Some(id) = client.hover(&path, &text, at)
                {
                    self.tools.mouse_hover = Some((id, (px, py), String::new()));
                }
            }
        }
        if let Some((_, (x, y), text)) = &self.tools.mouse_hover
            && !text.trim().is_empty()
        {
            let body = super::doc_text(text);
            let width = 460.0_f32.min(area.2 - 16.0);
            let x = (*x - 20.0).min(area.0 + area.2 - width).max(area.0);
            let lines = body.lines().count().min(14) as f32;
            let h = 24.0 + lines * 20.0;
            let y = if *y + 18.0 + h < area.1 + area.3 { *y + 18.0 } else { (*y - h - 8.0).max(area.1) };
            ui.glass_panel(x, y, width, 10.0, |ui| {
                ui.label(&body.lines().take(14).collect::<Vec<_>>().join("\n"));
            });
        }
    }

    /// Índice no texto da aba atual sob o ponto (x, y) da área do editor (letras de
    /// largura fixa).
    fn offset_at(&self, px: f32, py: f32, area: (f32, f32, f32, f32)) -> Option<usize> {
        let size = self.config.size;
        let doc = &self.docs.get(self.cur)?.text;
        let starts = doc.line_starts();
        let line_h = (size * 1.5).round();
        let gutter = if self.config.line_numbers { size * 0.9 * 0.6 * starts.len().to_string().len().max(2) as f32 + 20.0 } else { 0.0 };
        let line = ((py - area.1 - 6.0 + doc.scroll) / line_h).floor();
        if line < 0.0 || line as usize >= starts.len() {
            return None;
        }
        let line = line as usize;
        let col = ((px - area.0 - gutter - 8.0 + doc.scroll_x) / (size * 0.6)).floor();
        if col < 0.0 {
            return None;
        }
        let end = starts.get(line + 1).map_or(doc.text.len(), |e| e - 1);
        let text = &doc.text[starts[line]..end];
        let at = text.char_indices().nth(col as usize).map(|(i, _)| starts[line] + i)?;
        // Só em cima de uma palavra.
        doc.text[at..].chars().next().filter(|c| c.is_alphanumeric() || *c == '_').map(|_| at)
    }

    /// Resposta de uma dica pedida pelo ponteiro. Devolve se era dela.
    pub(crate) fn mouse_hover_answer(&mut self, id: u64, text: &str) -> bool {
        match &mut self.tools.mouse_hover {
            Some((want, _, t)) if *want == id => {
                *t = text.to_string();
                true
            }
            _ => false,
        }
    }

    /// Compara o arquivo da aba atual com o último commit (com o texto parado um
    /// instante; o do commit é lido uma vez por arquivo e de novo ao salvar).
    pub(crate) fn refresh_git(&mut self) {
        if self.last_edit.is_some_and(|t| t.elapsed() < Duration::from_millis(400)) {
            return;
        }
        let Some(doc) = self.docs.get(self.cur) else {
            return;
        };
        let Some(path) = doc.path.clone() else {
            return;
        };
        let version = doc.text.version;
        let entry = self.tools.git.entry(path.clone()).or_insert_with(|| (gitdiff::head_text(&path), u64::MAX, Vec::new()));
        if entry.1 == version {
            return;
        }
        entry.1 = version;
        entry.2 = match &entry.0 {
            Some(head) => gitdiff::line_changes(head, &doc.text.text),
            None => Vec::new(),
        };
    }

    /// Marcas do Git da aba `i` para a margem.
    pub(crate) fn git_marks(&self, i: usize) -> Vec<(usize, [u8; 4], bool)> {
        let Some((_, _, changes)) = self.docs[i].path.as_ref().and_then(|p| self.tools.git.get(p)) else {
            return Vec::new();
        };
        changes
            .iter()
            .map(|(line, c)| match c {
                LineChange::Added => (*line, [46, 160, 67, 255], false),
                LineChange::Modified => (*line, [47, 129, 247, 255], false),
                LineChange::DeletedAbove => (*line, [215, 58, 73, 255], true),
            })
            .collect()
    }
}
