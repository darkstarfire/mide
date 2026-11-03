//! Recursos de editor que vêm do servidor de linguagem, como no VS Code e no Zed:
//! ajuda de assinatura, renomear, formatar (também ao salvar), ações de código,
//! símbolos do arquivo (lista e caminho na barra de status) e dicas em linha.

use super::*;
use mide::code::lsp::{CodeAction, InlayHint, Signature, Symbol, WorkspaceEdit};

/// Para que serve uma troca pedida ao servidor.
pub(crate) enum PendingEdit {
    Rename,
    /// Formatar o arquivo (e gravar depois, quando veio do salvar).
    Format { path: PathBuf, then_save: bool, at: Instant },
}

/// Estado dos recursos do LSP de cada janela.
#[derive(Default)]
pub(crate) struct LspUi {
    /// Ajuda de assinatura: o pedido e a resposta.
    pub signature: Option<(u64, Option<Signature>)>,
    /// Onde a assinatura foi pedida: a aba e o ponto do texto (ela some fora dali).
    pub signature_at: Option<(usize, usize)>,
    pub edits: HashMap<u64, PendingEdit>,
    /// Ações de código pedidas (o número do pedido).
    pub actions_req: Option<u64>,
    /// Símbolos pedidos: para a lista (`true`) ou só para o caminho na barra.
    pub symbols_req: Option<(u64, PathBuf, u64, bool)>,
    /// Símbolos de cada arquivo e a versão do texto deles.
    pub symbols: HashMap<PathBuf, (u64, Vec<Symbol>)>,
    pub hints_req: Option<(u64, PathBuf, u64)>,
    /// Dicas em linha de cada arquivo, já em bytes, e a versão do texto.
    pub hints: HashMap<PathBuf, (u64, Vec<(usize, String)>)>,
}

impl Code {
    /// Eventos novos do servidor (os de antes ficam em `poll_lsp`).
    pub(crate) fn on_lsp_event(&mut self, event: Event) {
        match event {
            Event::Signature { id, signature } => {
                if self.lsp.signature.as_ref().is_some_and(|(want, _)| *want == id) {
                    self.lsp.signature = signature.map(|s| (id, Some(s)));
                }
            }
            Event::Edit { id: 0, edit } => self.apply_workspace_edit(&edit, None),
            Event::Edit { id, edit } => match self.lsp.edits.remove(&id) {
                Some(PendingEdit::Rename) => {
                    let files = edit.iter().filter(|(_, e)| !e.is_empty()).count();
                    let changes: usize = edit.iter().map(|(_, e)| e.len()).sum();
                    self.apply_workspace_edit(&edit, None);
                    self.say(tr!("Renamed: {n} changes in {files} files", n = changes, files = files));
                }
                Some(PendingEdit::Format { path, then_save, .. }) => {
                    self.apply_workspace_edit(&edit, Some(&path));
                    if then_save && let Some(i) = self.docs.iter().position(|d| d.path.as_ref() == Some(&path)) {
                        self.save_index(i);
                    }
                }
                None => {}
            },
            Event::Actions { id, actions } => {
                if self.lsp.actions_req == Some(id) {
                    self.lsp.actions_req = None;
                    if actions.is_empty() {
                        self.say(tr!("No code actions here."));
                    } else {
                        self.overlay = Overlay::Actions { actions, sel: 0, go: false };
                    }
                } else if let Some(action) = actions.into_iter().next() {
                    // Resposta do `codeAction/resolve`: agora com as trocas.
                    self.run_action(action);
                }
            }
            Event::Symbols { id, symbols } => {
                if let Some((want, path, version, list)) = self.lsp.symbols_req.take() {
                    if want != id {
                        self.lsp.symbols_req = Some((want, path, version, list));
                        return;
                    }
                    if list {
                        self.overlay = Overlay::Symbols { query: String::new(), items: symbols.clone(), sel: 0, focus: true };
                    }
                    self.lsp.symbols.insert(path, (version, symbols));
                }
            }
            Event::Hints { id, hints } => {
                if let Some((want, path, version)) = self.lsp.hints_req.take() {
                    if want != id {
                        self.lsp.hints_req = Some((want, path, version));
                        return;
                    }
                    let Some(doc) = self.docs.iter().find(|d| d.path.as_ref() == Some(&path)) else {
                        return;
                    };
                    if doc.text.version != version {
                        return;
                    }
                    let utf8 = self.client_utf8(doc.lang.id);
                    let inlays = hints_to_inlays(&doc.text.text, &hints, utf8);
                    self.lsp.hints.insert(path, (version, inlays));
                }
            }
            _ => {}
        }
    }

    /// Posições em UTF-8 no servidor da linguagem (senão, UTF-16).
    pub(crate) fn client_utf8(&self, lang: &str) -> bool {
        self.config.server_for(lang).and_then(|s| self.servers.get(&s.key())).is_none_or(|c| c.utf8)
    }

    /// Aplica trocas do servidor: nas abas abertas (dá para desfazer) e, nos arquivos
    /// fechados, direto no disco. `default` é o arquivo das trocas sem endereço
    /// (formatar).
    pub(crate) fn apply_workspace_edit(&mut self, edit: &WorkspaceEdit, default: Option<&Path>) {
        for (uri, edits) in edit {
            if edits.is_empty() {
                continue;
            }
            let path = if uri.is_empty() { default.map(Path::to_path_buf) } else { mide::kit::parse_uri_list(uri).into_iter().next() };
            let Some(path) = path else {
                continue;
            };
            if let Some(i) = self.docs.iter().position(|d| d.path.as_ref() == Some(&path)) {
                let utf8 = self.client_utf8(self.docs[i].lang.id);
                let doc = &mut self.docs[i].text;
                let new = lsp::apply_edits(&doc.text, edits, utf8);
                if new != doc.text {
                    doc.before_change(mide::kit::textarea::Change::Other);
                    // O cursor fica na mesma linha e coluna (o mais perto possível).
                    let starts = doc.line_starts();
                    let line = line_of(&starts, doc.edit.cursor);
                    let col = doc.edit.cursor - starts[line];
                    doc.text = new;
                    let starts = doc.line_starts();
                    let line = line.min(starts.len() - 1);
                    let end = starts.get(line + 1).map_or(doc.text.len(), |e| e.saturating_sub(1));
                    let mut at = (starts[line] + col).min(end);
                    while !doc.text.is_char_boundary(at) {
                        at -= 1;
                    }
                    doc.edit = mide::kit::textedit::Edit { cursor: at, anchor: None };
                    doc.modified = true;
                    doc.version += 1;
                    self.last_edit = Some(Instant::now());
                }
            } else if let Ok((text, crlf)) = read_text(&path) {
                let utf8 = self.config.server_for(lang::detect(&path, text.lines().next().unwrap_or("")).id).and_then(|s| self.servers.get(&s.key())).is_none_or(|c| c.utf8);
                let new = lsp::apply_edits(&text, edits, utf8);
                let out = if crlf { new.replace('\n', "\r\n") } else { new };
                if let Err(err) = mide::files::write_atomic(&path, out.as_bytes()) {
                    self.say(tr!("Couldn’t save: {err}", err = err));
                }
            }
        }
    }

    /// Ajuda de assinatura no cursor (ao abrir parênteses ou digitar uma vírgula).
    pub(crate) fn request_signature(&mut self) {
        let cursor = self.doc().text.edit.cursor;
        let text = self.doc().text.text.clone();
        let Some(path) = self.doc().path.clone() else {
            return;
        };
        self.sync_now();
        if let Some(client) = self.client_for_current().filter(|c| c.can("signatureHelpProvider"))
            && let Some(id) = client.signature_help(&path, &text, cursor)
        {
            let shown = self.lsp.signature.take().and_then(|(_, s)| s);
            self.lsp.signature = Some((id, shown));
            // A primeira vez na chamada guarda o lugar; uma vírgula depois não o muda.
            if self.lsp.signature_at.is_none_or(|(d, _)| d != self.cur) {
                self.lsp.signature_at = Some((self.cur, cursor));
            }
        }
    }

    /// Depois de digitar: `(` e `,` pedem a assinatura; `)` (fechando a chamada) e
    /// sair da linha a escondem.
    pub(crate) fn signature_after_typing(&mut self, before: &str, line_changed: bool) {
        if line_changed {
            self.lsp.signature = None;
            self.lsp.signature_at = None;
            return;
        }
        match before.chars().next_back() {
            Some('(' | ',') => self.request_signature(),
            Some(')') => {
                self.lsp.signature = None;
                self.lsp.signature_at = None;
            }
            // Dentro dos parênteses, o parâmetro atual muda com a vírgula: pede de novo
            // só se a ajuda já está à vista.
            Some(_) if self.lsp.signature.as_ref().is_some_and(|(_, s)| s.is_some()) => {}
            _ => {}
        }
    }

    /// A ajuda de assinatura em cima da linha do cursor.
    pub(crate) fn signature_popup(&mut self, ui: &mut Ui, area: (f32, f32, f32, f32)) {
        // Some ao trocar de aba, ao clicar, com Esc, ao sair da linha ou ao voltar para
        // antes do parêntese da chamada (antes, ficava aberta até em outro arquivo).
        let away = match self.lsp.signature_at {
            Some((doc, at)) => {
                let d = &self.docs[self.cur.min(self.docs.len() - 1)].text;
                let cursor = d.edit.cursor;
                doc != self.cur || cursor < at || d.text.get(at.min(cursor)..cursor).is_none_or(|t| t.contains('\n'))
            }
            None => true,
        };
        if away || ui.input().key(Key::Escape) || ui.input().pressed {
            self.lsp.signature = None;
            self.lsp.signature_at = None;
            return;
        }
        let Some((_, Some(sig))) = &self.lsp.signature else {
            return;
        };
        let sig = sig.clone();
        let size = self.config.size;
        let doc = &self.docs[self.cur].text;
        let starts = doc.line_starts();
        let line = line_of(&starts, doc.edit.cursor);
        let line_h = (size * 1.5).round();
        let gutter = if self.config.line_numbers { size * 0.9 * 0.6 * starts.len().to_string().len().max(2) as f32 + 20.0 } else { 0.0 };
        let col = doc.text[starts[line]..doc.edit.cursor].chars().count() as f32;
        let width = 460.0_f32.min(area.2 - 16.0);
        let x = (area.0 + gutter + 8.0 + col * size * 0.6 - doc.scroll_x - 40.0).min(area.0 + area.2 - width - 8.0).max(area.0 + 8.0);
        let y_line = area.1 + 6.0 + line as f32 * line_h - doc.scroll;
        let doc_text = super::doc_text(&sig.doc);
        let lines_doc = doc_text.lines().count().min(6) as f32;
        let est = 20.0 + (sig.label.chars().count() as f32 * size * 0.6 / width).ceil() * size * 1.45 + lines_doc * 20.0;
        // Em cima da linha (como no VS Code); sem lugar, embaixo.
        let y = if y_line - est - 6.0 > area.1 { y_line - est - 6.0 } else { y_line + line_h + 4.0 };
        let spans: Vec<(&str, bool)> = match sig.active {
            Some((a, b)) if a <= b && b <= sig.label.len() && sig.label.is_char_boundary(a) && sig.label.is_char_boundary(b) => vec![(&sig.label[..a], false), (&sig.label[a..b], true), (&sig.label[b..], false)],
            _ => vec![(sig.label.as_str(), false)],
        };
        ui.glass_panel(x, y, width, 10.0, |ui| {
            ui.code_spans(&spans, size);
            if !doc_text.is_empty() {
                ui.note(&doc_text.lines().take(6).collect::<Vec<_>>().join("\n"));
            }
        });
    }

    pub(crate) fn rename_symbol(&mut self) {
        if self.doc().path.is_none() || self.client_for_current().is_none_or(|c| !c.can("renameProvider")) {
            self.say(tr!("Rename needs a language server that supports it."));
            return;
        }
        let doc = &self.doc().text;
        let at = doc.edit.cursor;
        let start = Self::word_start(&doc.text, at);
        let end = at + doc.text[at..].find(|c: char| !(c.is_alphanumeric() || c == '_')).unwrap_or(doc.text.len() - at);
        let word = doc.text[start..end].to_string();
        self.overlay = Overlay::Rename { text: word, focus: true };
    }

    fn send_rename(&mut self, new_name: &str) {
        let cursor = self.doc().text.edit.cursor;
        let text = self.doc().text.text.clone();
        let Some(path) = self.doc().path.clone() else {
            return;
        };
        self.sync_now();
        if let Some(client) = self.client_for_current()
            && let Some(id) = client.rename(&path, &text, cursor, new_name)
        {
            self.lsp.edits.insert(id, PendingEdit::Rename);
        }
    }

    /// Formata o arquivo pelo servidor (e grava depois, com `then_save`). Devolve
    /// falso se não há servidor que formate (quem chamou grava direto).
    pub(crate) fn format_document(&mut self, then_save: bool) -> bool {
        let Some(path) = self.doc().path.clone() else {
            return false;
        };
        let (tab, spaces) = (self.config.tab_size, self.config.soft_tabs);
        self.sync_now();
        let Some(client) = self.client_for_current().filter(|c| c.can("documentFormattingProvider")) else {
            if !then_save {
                self.say(tr!("Formatting needs a language server that supports it."));
            }
            return false;
        };
        match client.formatting(&path, tab, spaces) {
            Some(id) => {
                self.lsp.edits.insert(id, PendingEdit::Format { path, then_save, at: Instant::now() });
                true
            }
            None => false,
        }
    }

    pub(crate) fn code_actions(&mut self) {
        let doc = &self.doc().text;
        let (from, to) = doc.edit.selection().unwrap_or((doc.edit.cursor, doc.edit.cursor));
        let text = doc.text.clone();
        let diags = self.doc().diags.clone();
        let Some(path) = self.doc().path.clone() else {
            return;
        };
        self.sync_now();
        match self.client_for_current().filter(|c| c.can("codeActionProvider")) {
            Some(client) => self.lsp.actions_req = client.code_actions(&path, &text, from, to, &diags),
            None => self.say(tr!("Code actions need a language server that supports them.")),
        }
    }

    /// Roda uma ação: trocas primeiro, depois o comando no servidor; sem os dois,
    /// pede os detalhes (`codeAction/resolve`).
    fn run_action(&mut self, action: CodeAction) {
        if action.edit.is_empty() && action.command.is_none() {
            if let Some(raw) = action.raw
                && let Some(client) = self.client_for_current()
            {
                client.resolve_action(raw);
            }
            return;
        }
        self.apply_workspace_edit(&action.edit, None);
        if let Some(command) = &action.command
            && let Some(client) = self.client_for_current()
        {
            client.execute_command(command);
        }
        tracing::info!("ação de código: {}", action.title);
    }

    /// Pede os símbolos do arquivo atual (para a lista, ou só para o caminho na barra).
    pub(crate) fn request_symbols(&mut self, list: bool) {
        let Some(path) = self.doc().path.clone() else {
            return;
        };
        let version = self.doc().text.version;
        if list && let Some((v, items)) = self.lsp.symbols.get(&path).filter(|(v, _)| *v == version).cloned() {
            let _ = v;
            self.overlay = Overlay::Symbols { query: String::new(), items, sel: 0, focus: true };
            return;
        }
        self.sync_now();
        match self.client_for_current().filter(|c| c.can("documentSymbolProvider")) {
            Some(client) => {
                if let Some(id) = client.document_symbols(&path) {
                    self.lsp.symbols_req = Some((id, path, version, list));
                }
            }
            None if list => self.say(tr!("The symbol list needs a language server.")),
            None => {}
        }
    }

    /// Com o texto parado um instante, pede o que muda com ele: símbolos (para o
    /// caminho na barra) e dicas em linha.
    pub(crate) fn refresh_lsp_extras(&mut self) {
        // Formatar ao salvar sem resposta em 1,5 s: grava sem formatar.
        let stale: Vec<u64> = self.lsp.edits.iter().filter(|(_, e)| matches!(e, PendingEdit::Format { then_save: true, at, .. } if at.elapsed() > Duration::from_millis(1500))).map(|(id, _)| *id).collect();
        for id in stale {
            if let Some(PendingEdit::Format { path, .. }) = self.lsp.edits.remove(&id)
                && let Some(i) = self.docs.iter().position(|d| d.path.as_ref() == Some(&path))
            {
                self.save_index(i);
            }
        }
        if self.last_edit.is_some_and(|t| t.elapsed() < Duration::from_millis(500)) {
            return;
        }
        let Some(path) = self.docs.get(self.cur).and_then(|d| d.path.clone()) else {
            return;
        };
        let version = self.doc().text.version;
        if self.doc().lsp_seen != Some(version) {
            return;
        }
        if self.lsp.symbols_req.is_none() && self.lsp.symbols.get(&path).is_none_or(|(v, _)| *v != version) {
            self.request_symbols(false);
        }
        if self.config.inlay_hints && self.lsp.hints_req.is_none() && self.lsp.hints.get(&path).is_none_or(|(v, _)| *v != version) {
            let lines = self.doc().text.line_starts().len() as u32;
            if let Some(client) = self.client_for_current().filter(|c| c.can("inlayHintProvider"))
                && let Some(id) = client.inlay_hints(&path, 0, lines)
            {
                self.lsp.hints_req = Some((id, path, version));
            } else {
                // Sem suporte: não pergunta de novo nesta versão.
                self.lsp.hints.insert(path, (version, Vec::new()));
            }
        }
    }

    /// Dicas em linha do arquivo da aba `i` (só as da versão atual do texto).
    pub(crate) fn inlays_for(&self, i: usize) -> Vec<(usize, String)> {
        let doc = &self.docs[i];
        if !self.config.inlay_hints {
            return Vec::new();
        }
        doc.path.as_ref().and_then(|p| self.lsp.hints.get(p)).filter(|(v, _)| *v == doc.text.version).map(|(_, h)| h.clone()).unwrap_or_default()
    }

    /// O caminho dos símbolos em volta do cursor ("Classe › método"), para a barra.
    pub(crate) fn breadcrumbs(&self) -> String {
        let doc = &self.docs[self.cur];
        let Some((_, symbols)) = doc.path.as_ref().and_then(|p| self.lsp.symbols.get(p)) else {
            return String::new();
        };
        let starts = doc.text.line_starts();
        let line = line_of(&starts, doc.text.edit.cursor) as u32;
        let mut chain: Vec<&Symbol> = Vec::new();
        for s in symbols.iter().filter(|s| s.line <= line && s.end_line >= line) {
            chain.retain(|c| c.depth < s.depth);
            chain.push(s);
        }
        chain.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join(" › ")
    }

    /// Painéis sem campo de texto (ações de código) leem as setas e o Enter no começo
    /// do quadro, antes do editor (senão o Enter também quebraria a linha).
    pub(crate) fn overlay_keys(&mut self, ui: &mut Ui) {
        if let Overlay::Actions { actions, sel, go } = &mut self.overlay {
            if ui.take_key(Key::Up) {
                *sel = panes::cycle(*sel, actions.len(), false);
            }
            if ui.take_key(Key::Down) {
                *sel = panes::cycle(*sel, actions.len(), true);
            }
            if ui.take_key(Key::Enter) {
                *go = true;
            }
        }
        if let Overlay::Problems { sel, go } = &mut self.overlay {
            if ui.take_key(Key::Up) {
                *sel = sel.saturating_sub(1);
            }
            if ui.take_key(Key::Down) {
                *sel += 1;
            }
            if ui.take_key(Key::Enter) {
                *go = true;
            }
        }
    }

    /// Os painéis por cima que são deste arquivo. Devolve se tratou o painel aberto.
    pub(crate) fn lsp_overlays(&mut self, ui: &mut Ui, esc: bool) -> bool {
        match &mut self.overlay {
            Overlay::Rename { text, focus } => {
                let mut go = None;
                ui.modal(380.0, |ui| {
                    ui.strong(&tr!("Rename Symbol"));
                    if std::mem::take(focus) {
                        ui.focus_next_input();
                    }
                    if ui.text_input(text, &tr!("New name")).submitted() && !text.trim().is_empty() {
                        go = Some(text.trim().to_string());
                    }
                });
                if let Some(name) = go {
                    self.overlay = Overlay::None;
                    self.send_rename(&name);
                } else if esc {
                    self.overlay = Overlay::None;
                }
                true
            }
            Overlay::Actions { actions, sel, go } => {
                // As teclas foram lidas no começo do quadro (`overlay_keys`).
                let mut chosen = std::mem::take(go).then_some(*sel);
                let icon = mide::code::icons::kind_icon(15).unwrap_or_default();
                ui.modal(520.0, |ui| {
                    ui.strong(&tr!("Code Actions"));
                    for (k, a) in actions.iter().enumerate() {
                        if k == *sel {
                            ui.join_selection(false, false);
                        }
                        if ui.list_row(&icon, &a.title, &[(&a.kind, 120.0)], k == *sel).clicked() {
                            chosen = Some(k);
                        }
                    }
                });
                if let Some(k) = chosen {
                    let action = actions.get(k).cloned();
                    self.overlay = Overlay::None;
                    if let Some(action) = action {
                        self.run_action(action);
                    }
                } else if esc {
                    self.overlay = Overlay::None;
                }
                true
            }
            Overlay::Symbols { query, items, sel, focus } => {
                let (up, down, enter) = (ui.take_key(Key::Up), ui.take_key(Key::Down), ui.input().key(Key::Enter));
                let q = query.to_lowercase();
                let shown: Vec<&Symbol> = items.iter().filter(|s| q.is_empty() || fuzzy_score(&s.name.to_lowercase(), &q).is_some()).collect();
                if up {
                    *sel = sel.saturating_sub(1);
                }
                if down {
                    *sel = (*sel + 1).min(shown.len().saturating_sub(1));
                }
                *sel = (*sel).min(shown.len().saturating_sub(1));
                let mut chosen = enter.then(|| shown.get(*sel).map(|s| (s.line, s.col))).flatten();
                let first = panes::window_start(*sel, shown.len(), 14);
                ui.modal(560.0, |ui| {
                    ui.strong(&tr!("Go to Symbol"));
                    if std::mem::take(focus) {
                        ui.focus_next_input();
                    }
                    ui.text_input(query, &tr!("Symbol name"));
                    for (k, s) in shown.iter().enumerate().skip(first).take(14) {
                        let icon = mide::code::icons::kind_icon(s.kind).unwrap_or_default();
                        let label = format!("{}{}", "  ".repeat(s.depth.min(6)), s.name);
                        if k == *sel {
                            ui.join_selection(false, false);
                        }
                        if ui.list_row(&icon, &label, &[(&s.detail, 200.0)], k == *sel).clicked() {
                            chosen = Some((s.line, s.col));
                        }
                    }
                });
                if let Some((line, col)) = chosen {
                    self.overlay = Overlay::None;
                    let lang = self.doc().lang.id;
                    let utf8 = self.client_utf8(lang);
                    let doc = &mut self.doc().text;
                    let at = lsp::to_offset(&doc.text, line, col, utf8);
                    doc.edit = mide::kit::textedit::Edit { cursor: at, anchor: None };
                    self.start = true;
                } else if esc {
                    self.overlay = Overlay::None;
                }
                ui.request_repaint();
                true
            }
            _ => false,
        }
    }
}

/// Dicas em linha do LSP em bytes do texto, com o espaço pedido em volta.
fn hints_to_inlays(text: &str, hints: &[InlayHint], utf8: bool) -> Vec<(usize, String)> {
    hints
        .iter()
        .map(|h| {
            let at = lsp::to_offset(text, h.line, h.col, utf8);
            let label = format!("{}{}{}", if h.pad_left { " " } else { "" }, h.label.trim(), if h.pad_right { " " } else { "" });
            (at, label)
        })
        .collect()
}

/// Busca aproximada: as letras de `query` aparecem em ordem em `text` (minúsculas).
/// Pontua mais as seguidas e as do começo; `None` se não casa.
pub(crate) fn fuzzy_score(text: &str, query: &str) -> Option<i32> {
    let mut score = 0;
    let mut last: Option<usize> = None;
    let mut chars = text.char_indices();
    for q in query.chars() {
        let (i, _) = chars.by_ref().find(|(_, c)| *c == q)?;
        score += match last {
            Some(l) if i == l + 1 => 6,
            _ if i == 0 => 8,
            _ => 1,
        };
        last = Some(i);
    }
    Some(score - text.len() as i32 / 8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn busca_aproximada() {
        assert!(fuzzy_score("titlecase", "tc").is_some());
        assert!(fuzzy_score("titlecase", "xz").is_none());
        assert!(fuzzy_score("title", "ti") > fuzzy_score("subtitle", "ti"));
    }

    #[test]
    fn dicas_em_bytes() {
        let hints = vec![InlayHint { line: 0, col: 4, label: "i32".into(), pad_left: false, pad_right: true }];
        assert_eq!(hints_to_inlays("let x = 1;", &hints, true), vec![(4, "i32 ".to_string())]);
    }
}
