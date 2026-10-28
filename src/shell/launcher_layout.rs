//! Organização da grade do launcher: ordem dos apps e pastas criadas pelo usuário,
//! salva em `~/.config/mide/launcher.toml`.
//!
//! Regras, como no macOS e no GNOME:
//! - Arrastar um app sobre outro cria uma pasta com os dois.
//! - Arrastar um app sobre uma pasta põe o app nela.
//! - Arrastar para outro lugar muda a ordem.
//! - Uma pasta que fica com um app só deixa de existir (o app volta para a grade).

use std::{fs, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::apps::{self, App};

/// Um lugar da grade principal: app (pelo id do `.desktop`) ou pasta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Slot {
    App(String),
    Folder { name: String, apps: Vec<String> },
}

/// A grade principal inteira, na ordem.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Layout {
    pub slots: Vec<Slot>,
}

#[derive(Serialize, Deserialize, Default)]
struct FileFormat {
    #[serde(default)]
    slot: Vec<SlotFormat>,
}

#[derive(Serialize, Deserialize)]
struct SlotFormat {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    app: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    folder: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    apps: Vec<String>,
}

/// Caminho de `launcher.toml`, respeitando `XDG_CONFIG_HOME`.
pub fn config_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("mide").join("launcher.toml"))
}

impl Layout {
    /// Organização padrão: apps soltos em ordem alfabética e as pastas no fim.
    pub fn default_for(list: &[App]) -> Self {
        let (loose, folders) = apps::arrange(list);
        let mut slots: Vec<Slot> = loose.into_iter().map(|i| Slot::App(list[i].id.clone())).collect();
        slots.extend(folders.into_iter().map(|f| Slot::Folder {
            name: f.name.to_string(),
            apps: f.apps.into_iter().map(|i| list[i].id.clone()).collect(),
        }));
        Self { slots }
    }

    /// Ajusta uma organização salva aos apps instalados agora: tira os que foram
    /// desinstalados e põe os novos no fim.
    pub fn reconcile(mut self, list: &[App]) -> Self {
        let installed = |id: &String| list.iter().any(|a| &a.id == id);
        for slot in &mut self.slots {
            if let Slot::Folder { apps, .. } = slot {
                apps.retain(installed);
            }
        }
        self.slots.retain(|s| match s {
            Slot::App(id) => installed(id),
            Slot::Folder { apps, .. } => !apps.is_empty(),
        });
        self.unwrap_single_folders();
        let placed: Vec<String> = self.app_ids();
        let mut new: Vec<&App> = list.iter().filter(|a| !placed.contains(&a.id)).collect();
        new.sort_by_key(|a| apps::fold(&a.name));
        self.slots.extend(new.into_iter().map(|a| Slot::App(a.id.clone())));
        self
    }

    /// Todos os ids de app na organização, inclusive os de dentro das pastas.
    pub fn app_ids(&self) -> Vec<String> {
        self.slots
            .iter()
            .flat_map(|s| match s {
                Slot::App(id) => vec![id.clone()],
                Slot::Folder { apps, .. } => apps.clone(),
            })
            .collect()
    }

    fn unwrap_single_folders(&mut self) {
        for slot in &mut self.slots {
            if let Slot::Folder { apps, .. } = slot {
                if apps.len() == 1 {
                    *slot = Slot::App(apps[0].clone());
                }
            }
        }
    }

    /// Move o lugar `from` para a posição `to` (índice na lista final).
    pub fn move_slot(&mut self, from: usize, to: usize) {
        if from >= self.slots.len() || from == to {
            return;
        }
        let slot = self.slots.remove(from);
        let to = to.min(self.slots.len());
        self.slots.insert(to, slot);
    }

    /// Solta o lugar `from` sobre o lugar `target`: app sobre app cria uma pasta
    /// (com o nome `name`), app sobre pasta entra na pasta. Pasta sobre algo só
    /// muda de lugar. Devolve o índice da pasta resultante, se houver.
    pub fn drop_on(&mut self, from: usize, target: usize, name: &str) -> Option<usize> {
        if from == target || from >= self.slots.len() || target >= self.slots.len() {
            return None;
        }
        let Slot::App(dragged) = self.slots[from].clone() else {
            self.move_slot(from, target);
            return None;
        };
        match &mut self.slots[target] {
            Slot::Folder { apps, .. } => apps.push(dragged),
            Slot::App(other) => {
                let other = other.clone();
                self.slots[target] = Slot::Folder { name: name.to_string(), apps: vec![other, dragged] };
            }
        }
        self.slots.remove(from);
        Some(if from < target { target - 1 } else { target })
    }

    /// Tira um app de uma pasta e põe na grade principal, logo depois da pasta. Se a
    /// pasta ficar com um app só, ela vira esse app.
    pub fn take_out(&mut self, folder: usize, app: &str) {
        let Some(Slot::Folder { apps, .. }) = self.slots.get_mut(folder) else {
            return;
        };
        let before = apps.len();
        apps.retain(|a| a != app);
        if apps.len() == before {
            return;
        }
        self.slots.insert(folder + 1, Slot::App(app.to_string()));
        self.unwrap_single_folders();
    }

    /// Muda a ordem de um app dentro da pasta.
    pub fn move_in_folder(&mut self, folder: usize, from: usize, to: usize) {
        if let Some(Slot::Folder { apps, .. }) = self.slots.get_mut(folder) {
            if from < apps.len() && from != to {
                let app = apps.remove(from);
                apps.insert(to.min(apps.len()), app);
            }
        }
    }

    pub fn rename(&mut self, folder: usize, new_name: &str) {
        let new_name = new_name.trim();
        if new_name.is_empty() {
            return;
        }
        if let Some(Slot::Folder { name, .. }) = self.slots.get_mut(folder) {
            *name = new_name.to_string();
        }
    }

    /// Lê a organização salva (ou `None` se não houver).
    pub fn load() -> Option<Self> {
        let text = fs::read_to_string(config_path()?).ok()?;
        Self::from_toml(&text)
    }

    pub fn from_toml(text: &str) -> Option<Self> {
        let file: FileFormat = toml::from_str(text).ok()?;
        let slots = file
            .slot
            .into_iter()
            .filter_map(|s| match (s.app, s.folder) {
                (Some(app), _) => Some(Slot::App(app)),
                (None, Some(name)) => Some(Slot::Folder { name, apps: s.apps }),
                _ => None,
            })
            .collect();
        Some(Self { slots })
    }

    pub fn to_toml(&self) -> String {
        let file = FileFormat {
            slot: self
                .slots
                .iter()
                .map(|s| match s {
                    Slot::App(id) => SlotFormat { app: Some(id.clone()), folder: None, apps: Vec::new() },
                    Slot::Folder { name, apps } => SlotFormat { app: None, folder: Some(name.clone()), apps: apps.clone() },
                })
                .collect(),
        };
        let body = toml::to_string(&file).unwrap_or_default();
        format!("# Organização do launcher do mide: ordem dos apps e pastas.\n\n{body}")
    }

    pub fn save(&self) {
        let Some(path) = config_path() else {
            return;
        };
        if let Some(dir) = path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        match fs::write(&path, self.to_toml()) {
            Ok(()) => tracing::info!("launcher salvo ({} lugar(es))", self.slots.len()),
            Err(err) => tracing::warn!("falha ao salvar o launcher: {err}"),
        }
    }
}

/// Nome de uma pasta nova com dois apps: o grupo deles, se for o mesmo, senão "Folder"
/// (traduzido; o nome é gravado como o usuário o viu).
pub fn folder_name_for(a: &App, b: &App) -> String {
    if a.group == b.group && a.group != "Outros" {
        crate::apps::group_label(a.group).into_owned()
    } else {
        crate::tr!("Folder").into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(id: &str, group: &'static str) -> App {
        App {
            id: id.into(),
            name: id.into(),
            command: "true".into(),
            icon: None,
            group,
            terminal: false,
            keywords: String::new(),
            mime: Vec::new(),
        }
    }

    fn ids(layout: &Layout) -> Vec<String> {
        layout
            .slots
            .iter()
            .map(|s| match s {
                Slot::App(id) => id.clone(),
                Slot::Folder { name, apps } => format!("[{name}: {}]", apps.join(",")),
            })
            .collect()
    }

    #[test]
    fn app_sobre_app_cria_pasta_e_sobre_pasta_entra() {
        let mut l = Layout { slots: ["a", "b", "c"].iter().map(|s| Slot::App(s.to_string())).collect() };
        assert_eq!(l.drop_on(0, 2, "Pasta"), Some(1));
        assert_eq!(ids(&l), vec!["b", "[Pasta: c,a]"]);
        l.drop_on(0, 1, "x");
        assert_eq!(ids(&l), vec!["[Pasta: c,a,b]"]);
    }

    #[test]
    fn mover_e_tirar_da_pasta() {
        let mut l = Layout {
            slots: vec![
                Slot::App("a".into()),
                Slot::Folder { name: "F".into(), apps: vec!["b".into(), "c".into()] },
                Slot::App("d".into()),
            ],
        };
        l.move_slot(2, 0);
        assert_eq!(ids(&l), vec!["d", "a", "[F: b,c]"]);
        l.move_in_folder(2, 0, 1);
        assert_eq!(ids(&l), vec!["d", "a", "[F: c,b]"]);
        // Tirar um de dois: a pasta vira o app que sobrou.
        l.take_out(2, "c");
        assert_eq!(ids(&l), vec!["d", "a", "b", "c"]);
    }

    #[test]
    fn renomear_ignora_nome_vazio() {
        let mut l = Layout { slots: vec![Slot::Folder { name: "F".into(), apps: vec!["a".into(), "b".into()] }] };
        l.rename(0, "  Jogos ");
        l.rename(0, "   ");
        assert_eq!(ids(&l), vec!["[Jogos: a,b]"]);
    }

    #[test]
    fn salvo_e_lido_de_volta() {
        let l = Layout {
            slots: vec![Slot::App("a".into()), Slot::Folder { name: "Pasta 1".into(), apps: vec!["b".into(), "c".into()] }],
        };
        assert_eq!(Layout::from_toml(&l.to_toml()), Some(l));
    }

    #[test]
    fn ajusta_aos_apps_instalados() {
        let saved = Layout {
            slots: vec![
                Slot::App("velho".into()),
                Slot::App("a".into()),
                Slot::Folder { name: "F".into(), apps: vec!["b".into(), "sumiu".into()] },
            ],
        };
        let list = vec![app("a", "Internet"), app("b", "Sistema"), app("novo", "Jogos")];
        let l = saved.reconcile(&list);
        // "velho" e "sumiu" saem; a pasta com um app só vira o app; "novo" vai no fim.
        assert_eq!(ids(&l), vec!["a", "b", "novo"]);
    }

    #[test]
    fn nome_da_pasta_nova() {
        crate::i18n::set_language("pt_BR");
        assert_eq!(folder_name_for(&app("a", "Jogos"), &app("b", "Jogos")), "Jogos");
        assert_eq!(folder_name_for(&app("a", "Jogos"), &app("b", "Internet")), "Pasta");
    }
}
