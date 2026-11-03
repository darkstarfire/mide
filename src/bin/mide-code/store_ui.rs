//! Loja de extensões do Code (Ctrl+Shift+X): busca no registro das extensões do
//! Zed, instala, atualiza e remove, com a busca e o download em segundo plano.

use std::sync::mpsc::{channel, Receiver};

use super::*;
use mide::code::store::{self, Listing};

#[derive(Default)]
pub(crate) struct StoreUi {
    pub query: String,
    /// A busca que gerou `results` e quando a busca atual mudou pela última vez.
    searched: Option<String>,
    changed_at: Option<Instant>,
    results: Vec<Listing>,
    sel: usize,
    searching: Option<Receiver<Result<Vec<Listing>, String>>>,
    /// Instalação ou remoção em andamento: o id e a resposta.
    working: Option<(String, Receiver<Result<String, String>>)>,
    error: Option<String>,
}

impl Code {
    pub(crate) fn open_store(&mut self) {
        self.overlay = Overlay::Extensions { focus: true };
        if self.store_ui.searched.is_none() {
            self.store_ui.changed_at = Some(Instant::now() - Duration::from_secs(1));
        }
    }

    /// Respostas da busca e da instalação (a cada quadro: uma instalação termina
    /// mesmo com a loja fechada).
    pub(crate) fn poll_store(&mut self) {
        if let Some(rx) = &self.store_ui.searching
            && let Ok(r) = rx.try_recv()
        {
            self.store_ui.searching = None;
            match r {
                Ok(list) => {
                    self.store_ui.results = list;
                    self.store_ui.error = None;
                }
                Err(e) => self.store_ui.error = Some(e),
            }
            self.store_ui.sel = 0;
        }
        if let Some((id, rx)) = &self.store_ui.working
            && let Ok(r) = rx.try_recv()
        {
            let id = id.clone();
            self.store_ui.working = None;
            match r {
                Ok(msg) => {
                    self.say(msg);
                    self.extensions_changed();
                }
                Err(e) => self.say(tr!("Extension {id}: {err}", id = id, err = e)),
            }
        }
        // A busca roda meio segundo depois de parar de digitar.
        let due = self.store_ui.changed_at.is_some_and(|t| t.elapsed() > Duration::from_millis(450));
        if due && self.store_ui.searching.is_none() {
            self.store_ui.changed_at = None;
            let q = self.store_ui.query.clone();
            self.store_ui.searched = Some(q.clone());
            let (tx, rx) = channel();
            std::thread::spawn(move || {
                let _ = tx.send(store::search(&q));
            });
            self.store_ui.searching = Some(rx);
        }
    }

    /// Extensão instalada ou removida: temas, trechos e servidores lidos de novo.
    fn extensions_changed(&mut self) {
        self.apply_themes();
        self.tools.snippets.clear();
        self.ext_servers.reset();
        for d in &mut self.docs {
            d.recolor();
        }
    }

    fn start_work(&mut self, id: String, remove: bool) {
        let (tx, rx) = channel();
        let who = id.clone();
        std::thread::spawn(move || {
            let r = if remove {
                store::uninstall(&who).map(|_| tr!("Extension {id} removed.", id = who))
            } else {
                store::install(&who).map(|_| tr!("Extension {id} installed. New languages appear the next time the Code opens.", id = who))
            };
            let _ = tx.send(r);
        });
        self.store_ui.working = Some((id, rx));
    }

    /// A janela da loja. Devolve se tratou o painel aberto.
    pub(crate) fn store_overlay(&mut self, ui: &mut Ui, esc: bool) -> bool {
        let Overlay::Extensions { focus } = &mut self.overlay else {
            return false;
        };
        let focus_now = std::mem::take(focus);
        if self.store_ui.searching.is_some() || self.store_ui.working.is_some() || self.store_ui.changed_at.is_some() {
            ui.wake_after(Duration::from_millis(150));
        }
        let (up, down) = (ui.take_key(Key::Up), ui.take_key(Key::Down));
        let st = &mut self.store_ui;
        if up {
            st.sel = st.sel.saturating_sub(1);
        }
        if down {
            st.sel = (st.sel + 1).min(st.results.len().saturating_sub(1));
        }
        let mut action: Option<(String, bool)> = None;
        let busy = st.working.as_ref().map(|(id, _)| id.clone());
        let first = panes::window_start(st.sel, st.results.len(), 9);
        ui.modal(760.0, |ui| {
            ui.strong(&tr!("Extensions"));
            if focus_now {
                ui.focus_next_input();
            }
            let before = st.query.clone();
            ui.search_field(&mut st.query, &tr!("Search extensions (Zed registry)"));
            if st.query != before {
                st.changed_at = Some(Instant::now());
            }
            if let Some(e) = &st.error {
                ui.note(&tr!("Couldn’t reach the registry: {err}", err = e));
            } else if st.searching.is_some() && st.results.is_empty() {
                ui.note(&tr!("Searching…"));
            }
            let icon = mide::code::icons::kind_icon(9).unwrap_or_default();
            for (k, l) in st.results.iter().enumerate().skip(first).take(9) {
                let status = match store::installed_version(&l.id) {
                    _ if !l.compatible() => tr!("not supported").into_owned(),
                    Some((v, _)) if store::newer(&l.version, &v) => tr!("update {v}", v = l.version),
                    Some((_, true)) => tr!("installed (Zed)").into_owned(),
                    Some(_) => tr!("installed").into_owned(),
                    None => l.version.clone(),
                };
                if k == st.sel {
                    ui.join_selection(false, false);
                }
                let label = format!("{} — {}", l.name, l.description);
                if ui.list_row(&icon, &label, &[(&status, 150.0)], k == st.sel).clicked() {
                    st.sel = k;
                }
            }
            if let Some(l) = st.results.get(st.sel) {
                ui.space(6.0);
                ui.note(&format!("{} · {} · {}", l.authors.join(", "), l.repository, tr!("{n} downloads", n = l.download_count)));
                let installed = store::installed_version(&l.id);
                let mide_install = store::install_dir(&l.id).is_some_and(|d| d.join("extension.toml").exists());
                ui.row(|ui| {
                    if let Some(id) = &busy {
                        ui.note(&tr!("Working on {id}…", id = id));
                        return;
                    }
                    if !l.compatible() {
                        ui.note(&tr!("Needs a newer extension interface than the Code has."));
                        return;
                    }
                    let can_update = installed.as_ref().is_some_and(|(v, _)| store::newer(&l.version, v));
                    if installed.is_none() || can_update || installed.as_ref().is_some_and(|(_, zed)| *zed && !mide_install) {
                        let label = if can_update { tr!("Update") } else { tr!("Install") };
                        if ui.primary_button(&label).clicked() {
                            action = Some((l.id.clone(), false));
                        }
                    }
                    if mide_install && ui.button(&tr!("Remove")).clicked() {
                        action = Some((l.id.clone(), true));
                    }
                });
            }
        });
        if let Some((id, remove)) = action {
            self.start_work(id, remove);
        }
        if esc {
            self.overlay = Overlay::None;
        }
        ui.request_repaint();
        true
    }
}
