//! Lista de apps instalados (arquivos `.desktop`), com grupos por categoria e
//! busca sem diferenciar maiúsculas nem acentos. Usada pelo launcher.

use std::{collections::HashSet, process::Command};

/// Um app que aparece no launcher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct App {
    /// Id do `.desktop` (ex.: `org.gnome.Calculator`).
    pub id: String,
    /// Nome no idioma do usuário.
    pub name: String,
    /// Comando do `Exec`, já sem os códigos de campo (`%u`, `%F`...).
    pub command: String,
    /// Nome do ícone no tema.
    pub icon: Option<String>,
    /// Grupo mostrado no launcher (ex.: "Internet").
    pub group: &'static str,
    /// Precisa abrir num terminal (`Terminal=true`).
    pub terminal: bool,
    /// Palavras extras para a busca (nome genérico e palavras-chave).
    pub keywords: String,
}

/// Grupos do launcher, na ordem em que aparecem.
pub const GROUPS: &[&str] = &[
    "Internet",
    "Escritório",
    "Gráficos",
    "Multimídia",
    "Desenvolvimento",
    "Jogos",
    "Educação",
    "Ciência",
    "Utilitários",
    "Sistema",
    "Configurações",
    "Outros",
];

/// Grupo de um app pelas categorias do `.desktop` (a primeira principal que casar).
pub fn group_of(categories: &[&str]) -> &'static str {
    // Categorias mais específicas primeiro: "Settings" costuma vir junto de "System".
    const ORDER: &[(&str, &str)] = &[
        ("Settings", "Configurações"),
        ("Game", "Jogos"),
        ("Development", "Desenvolvimento"),
        ("Network", "Internet"),
        ("Office", "Escritório"),
        ("Graphics", "Gráficos"),
        ("AudioVideo", "Multimídia"),
        ("Audio", "Multimídia"),
        ("Video", "Multimídia"),
        ("Education", "Educação"),
        ("Science", "Ciência"),
        ("Utility", "Utilitários"),
        ("System", "Sistema"),
    ];
    ORDER
        .iter()
        .find(|(cat, _)| categories.iter().any(|c| c.eq_ignore_ascii_case(cat)))
        .map(|(_, group)| *group)
        .unwrap_or("Outros")
}

/// Texto em minúsculas e sem acentos, para comparar na busca.
pub fn fold(text: &str) -> String {
    text.chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ã' | 'ä' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            'ñ' => 'n',
            other => other,
        })
        .collect()
}

/// Nota da busca: maior é melhor, `None` não casa. Começo do nome vale mais que
/// começo de palavra, que vale mais que qualquer trecho do nome ou das palavras-chave.
pub fn score(app: &App, query: &str) -> Option<u32> {
    let q = fold(query.trim());
    if q.is_empty() {
        return Some(0);
    }
    let name = fold(&app.name);
    if name.starts_with(&q) {
        return Some(300);
    }
    if name.split_whitespace().any(|w| w.starts_with(&q)) {
        return Some(200);
    }
    if name.contains(&q) {
        return Some(150);
    }
    if fold(&app.keywords).contains(&q) || fold(&app.id).contains(&q) {
        return Some(100);
    }
    None
}

/// Apps que casam com a busca e o grupo (`None` = todos), do melhor para o pior e
/// depois por nome.
pub fn filter<'a>(apps: &'a [App], query: &str, group: Option<&str>) -> Vec<&'a App> {
    let mut found: Vec<(u32, &App)> = apps
        .iter()
        .filter(|a| group.is_none_or(|g| a.group == g))
        .filter_map(|a| score(a, query).map(|s| (s, a)))
        .collect();
    found.sort_by(|(sa, a), (sb, b)| sb.cmp(sa).then_with(|| fold(&a.name).cmp(&fold(&b.name))));
    found.into_iter().map(|(_, a)| a).collect()
}

/// Grupos que têm pelo menos um app, na ordem de `GROUPS`.
pub fn groups_present(apps: &[App]) -> Vec<&'static str> {
    GROUPS
        .iter()
        .copied()
        .filter(|g| apps.iter().any(|a| a.group == *g))
        .collect()
}

/// Pastas do launcher, como no macOS: só os grupos de apps de apoio viram pasta
/// (o resto fica solto na grade). Cada pasta junta os grupos listados.
pub const FOLDERS: &[(&str, &[&str])] = &[
    ("Utilitários", &["Utilitários"]),
    ("Sistema", &["Sistema", "Configurações"]),
    ("Outros", &["Outros", "Educação", "Ciência"]),
];

/// Uma pasta do launcher: nome e índices dos apps (em `apps`), em ordem alfabética.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folder {
    pub name: &'static str,
    pub apps: Vec<usize>,
}

/// Organiza a grade do launcher: apps soltos (em ordem alfabética) e, no fim, as
/// pastas. Uma pasta com um app só não vale a pena: o app fica solto.
pub fn arrange(apps: &[App]) -> (Vec<usize>, Vec<Folder>) {
    let mut loose = Vec::new();
    let mut folders: Vec<Folder> = FOLDERS.iter().map(|(name, _)| Folder { name, apps: Vec::new() }).collect();
    for (i, app) in apps.iter().enumerate() {
        match FOLDERS.iter().position(|(_, groups)| groups.contains(&app.group)) {
            Some(f) => folders[f].apps.push(i),
            None => loose.push(i),
        }
    }
    for folder in &mut folders {
        if folder.apps.len() == 1 {
            loose.push(folder.apps[0]);
            folder.apps.clear();
        }
    }
    folders.retain(|f| !f.apps.is_empty());
    let by_name = |i: &usize| fold(&apps[*i].name);
    loose.sort_by_key(by_name);
    for folder in &mut folders {
        folder.apps.sort_by_key(by_name);
    }
    (loose, folders)
}

/// Remove os códigos de campo do `Exec` (`%u`, `%F`, `%i`...), que o launcher não usa.
pub fn clean_exec(exec: &str) -> String {
    exec.split_whitespace()
        .filter(|t| !(t.starts_with('%') && t.len() == 2))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Apps instalados que devem aparecer (sem `NoDisplay`, `Hidden` nem restrição de
/// desktop que exclua o mide), sem repetir id, em ordem alfabética.
pub fn load() -> Vec<App> {
    let locales = freedesktop_desktop_entry::get_languages_from_env();
    let mut seen = HashSet::new();
    let mut apps: Vec<App> = freedesktop_desktop_entry::desktop_entries(&locales)
        .into_iter()
        .filter(|e| !e.no_display() && !e.hidden())
        .filter(|e| e.only_show_in().is_none_or(|list| list.iter().any(|d| d.eq_ignore_ascii_case("mide"))))
        .filter_map(|e| {
            let command = clean_exec(e.exec()?);
            if command.is_empty() || !seen.insert(e.id().to_string()) {
                return None;
            }
            let categories = e.categories().unwrap_or_default();
            let mut keywords = e
                .generic_name(&locales)
                .map(|g| g.into_owned())
                .unwrap_or_default();
            if let Some(words) = e.keywords(&locales) {
                for w in words {
                    keywords.push(' ');
                    keywords.push_str(&w);
                }
            }
            Some(App {
                id: e.id().to_string(),
                name: e
                    .name(&locales)
                    .map(|n| n.into_owned())
                    .unwrap_or_else(|| e.id().to_string()),
                command,
                icon: e.icon().map(str::to_string),
                group: group_of(&categories),
                terminal: e.terminal(),
                keywords,
            })
        })
        .collect();
    apps.sort_by_key(|a| fold(&a.name));
    apps
}

/// Abre o app num processo separado. Apps de terminal abrem no foot (ou no
/// terminal padrão do sistema).
pub fn launch(app: &App) -> std::io::Result<()> {
    let command = if app.terminal {
        format!("foot -e {0} || x-terminal-emulator -e {0}", app.command)
    } else {
        app.command.clone()
    };
    let mut child = Command::new("sh").arg("-c").arg(&command).spawn()?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(name: &str, group: &'static str, keywords: &str) -> App {
        App {
            id: format!("test.{}", name.replace(' ', "")),
            name: name.into(),
            command: "true".into(),
            icon: None,
            group,
            terminal: false,
            keywords: keywords.into(),
        }
    }

    #[test]
    fn grupo_pela_categoria_principal() {
        assert_eq!(group_of(&["GTK", "Utility", "Calculator"]), "Utilitários");
        assert_eq!(group_of(&["System", "Settings"]), "Configurações");
        assert_eq!(group_of(&["Network", "WebBrowser"]), "Internet");
        assert_eq!(group_of(&["Audio", "Player"]), "Multimídia");
        assert_eq!(group_of(&[]), "Outros");
    }

    #[test]
    fn busca_ignora_acentos_e_maiusculas() {
        let a = app("Calculadora", "Utilitários", "");
        assert!(score(&a, "CALC").is_some());
        let b = app("Configurações", "Configurações", "");
        assert!(score(&b, "configuracoes").is_some());
        assert!(score(&b, "xyz").is_none());
    }

    #[test]
    fn busca_prefere_comeco_do_nome() {
        let apps = vec![
            app("Editor de Texto", "Utilitários", ""),
            app("Terminal", "Sistema", "texto shell"),
            app("Textos Antigos", "Outros", ""),
        ];
        let found: Vec<&str> = filter(&apps, "tex", None).iter().map(|a| a.name.as_str()).collect();
        assert_eq!(found, vec!["Textos Antigos", "Editor de Texto", "Terminal"]);
    }

    #[test]
    fn filtro_por_grupo_e_grupos_presentes() {
        let apps = vec![app("Foot", "Sistema", ""), app("Calculadora", "Utilitários", "")];
        assert_eq!(filter(&apps, "", Some("Sistema")).len(), 1);
        assert_eq!(filter(&apps, "", None).len(), 2);
        assert_eq!(groups_present(&apps), vec!["Utilitários", "Sistema"]);
    }

    #[test]
    fn grade_com_apps_soltos_e_pastas_no_fim() {
        let apps = vec![
            app("Navegador", "Internet", ""),
            app("Calculadora", "Utilitários", ""),
            app("Arquivos", "Utilitários", ""),
            app("Monitor", "Sistema", ""),
            app("Ajustes", "Configurações", ""),
            app("Atlas", "Educação", ""),
            app("Editor", "Desenvolvimento", ""),
        ];
        let (loose, folders) = arrange(&apps);
        let names = |ids: &[usize]| ids.iter().map(|i| apps[*i].name.as_str()).collect::<Vec<_>>();
        // "Outros" teria um app só (Atlas): ele fica solto.
        assert_eq!(names(&loose), vec!["Atlas", "Editor", "Navegador"]);
        assert_eq!(folders.len(), 2);
        assert_eq!(folders[0].name, "Utilitários");
        assert_eq!(names(&folders[0].apps), vec!["Arquivos", "Calculadora"]);
        assert_eq!(folders[1].name, "Sistema");
        assert_eq!(names(&folders[1].apps), vec!["Ajustes", "Monitor"]);
    }

    #[test]
    fn exec_sem_codigos_de_campo() {
        assert_eq!(clean_exec("firefox %u"), "firefox");
        assert_eq!(clean_exec("app --flag %F --x"), "app --flag --x");
        assert_eq!(clean_exec("printf 100%"), "printf 100%");
    }
}
