//! Apps do mide como padrão do sistema (B25): as regras, sem janela nem D-Bus, do
//! backend do portal `org.freedesktop.impl.portal.FileChooser` e do serviço
//! `org.freedesktop.FileManager1` (os dois no `mide-portal`).
//!
//! Caminho de um pedido do seletor: o app pede ao `xdg-desktop-portal`, que (com o
//! `mide-portals.conf`, só na sessão do mide) passa ao `mide-portal`. Ele traduz as
//! opções (`a{sv}`) em argumentos do `mide-chooser` ([`chooser_args`]), uma janela do
//! mide-ui por pedido, e traduz a saída dela ([`parse_output`]) no resultado do portal
//! ([`results`]). O `mide-chooser` lê os mesmos argumentos com [`parse_chooser_args`].

use std::{
    collections::HashMap,
    ffi::{OsStr, OsString},
    os::unix::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
};

use zbus::zvariant::{OwnedValue, Value};

use crate::kit::chooser::{ExtraChoice, Filter, Pattern};

/// Nome do backend do portal no barramento (o mesmo do `mide.portal`).
pub const PORTAL_BUS_NAME: &str = "org.freedesktop.impl.portal.desktop.mide";
/// Objeto do backend do portal.
pub const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
/// Nome e objeto do gerenciador de arquivos padrão ("Mostrar na pasta").
pub const FILEMANAGER_BUS_NAME: &str = "org.freedesktop.FileManager1";
pub const FILEMANAGER_PATH: &str = "/org/freedesktop/FileManager1";

/// O que o seletor faz.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChooserMode {
    Open,
    Folder,
    Save,
    SaveFiles,
}

impl ChooserMode {
    fn arg(self) -> &'static str {
        match self {
            ChooserMode::Open => "open",
            ChooserMode::Folder => "folder",
            ChooserMode::Save => "save",
            ChooserMode::SaveFiles => "save-files",
        }
    }
}

/// Um pedido ao seletor, já sem os tipos do D-Bus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChooserRequest {
    pub mode: ChooserMode,
    pub title: String,
    pub multiple: bool,
    pub folder: Option<PathBuf>,
    pub name: Option<String>,
    pub accept_label: Option<String>,
    pub filters: Vec<Filter>,
    pub current_filter: Option<usize>,
    pub choices: Vec<ExtraChoice>,
    /// Nomes dos arquivos do SaveFiles.
    pub files: Vec<String>,
}

impl ChooserRequest {
    fn new(mode: ChooserMode, title: &str) -> Self {
        Self {
            mode,
            title: title.to_string(),
            multiple: false,
            folder: None,
            name: None,
            accept_label: None,
            filters: Vec::new(),
            current_filter: None,
            choices: Vec::new(),
            files: Vec::new(),
        }
    }
}

/// Método do portal que chegou.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    OpenFile,
    SaveFile,
    SaveFiles,
}

fn value<'a>(options: &'a HashMap<String, OwnedValue>, key: &str) -> Option<Value<'static>> {
    let v: &'a OwnedValue = options.get(key)?;
    v.try_clone().ok().map(Value::from)
}

fn get_bool(options: &HashMap<String, OwnedValue>, key: &str) -> Option<bool> {
    value(options, key).and_then(|v| bool::try_from(v).ok())
}

fn get_string(options: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    value(options, key).and_then(|v| String::try_from(v).ok())
}

/// Caminho num `ay` do portal (bytes, com um `\0` no fim).
fn get_path(options: &HashMap<String, OwnedValue>, key: &str) -> Option<PathBuf> {
    let bytes = value(options, key).and_then(|v| Vec::<u8>::try_from(v).ok())?;
    bytes_to_path(bytes)
}

fn bytes_to_path(mut bytes: Vec<u8>) -> Option<PathBuf> {
    while bytes.last() == Some(&0) {
        bytes.pop();
    }
    (!bytes.is_empty()).then(|| PathBuf::from(OsString::from_vec(bytes)))
}

/// Filtro como o portal manda: `(sa(us))`, nome e padrões (0 = nome, 1 = MIME).
type RawFilter = (String, Vec<(u32, String)>);
/// Opção extra como o portal manda: `(ssa(ss)s)`, id, rótulo, opções e a escolhida.
type RawChoice = (String, String, Vec<(String, String)>, String);

fn to_filter((name, patterns): RawFilter) -> Filter {
    let patterns = patterns
        .into_iter()
        .filter_map(|(kind, p)| match kind {
            0 => Some(Pattern::Glob(p)),
            1 => Some(Pattern::Mime(p)),
            _ => None,
        })
        .collect();
    Filter { name, patterns }
}

fn from_filter(filter: &Filter) -> RawFilter {
    let patterns = filter
        .patterns
        .iter()
        .map(|p| match p {
            Pattern::Glob(g) => (0, g.clone()),
            Pattern::Mime(m) => (1, m.clone()),
        })
        .collect();
    (filter.name.clone(), patterns)
}

/// Traduz as opções do portal (`a{sv}`) num pedido ao seletor. Opções desconhecidas ou
/// com tipo errado ficam de fora (o pedido continua).
pub fn decode_request(method: Method, title: &str, options: &HashMap<String, OwnedValue>) -> ChooserRequest {
    let mode = match method {
        Method::OpenFile if get_bool(options, "directory").unwrap_or(false) => ChooserMode::Folder,
        Method::OpenFile => ChooserMode::Open,
        Method::SaveFile => ChooserMode::Save,
        Method::SaveFiles => ChooserMode::SaveFiles,
    };
    let mut req = ChooserRequest::new(mode, title);
    req.multiple = mode == ChooserMode::Open && get_bool(options, "multiple").unwrap_or(false);
    req.accept_label = get_string(options, "accept_label").filter(|l| !l.is_empty());
    req.folder = get_path(options, "current_folder");
    if mode == ChooserMode::Save {
        req.name = get_string(options, "current_name").filter(|n| !n.is_empty());
        // Arquivo que já existe (Salvar como de um documento aberto): a pasta e o nome dele.
        if let Some(file) = get_path(options, "current_file") {
            req.folder = file.parent().map(Path::to_path_buf).or(req.folder);
            req.name = file.file_name().map(|n| n.to_string_lossy().into_owned()).or(req.name);
        }
    }
    if mode == ChooserMode::SaveFiles {
        req.files = value(options, "files")
            .and_then(|v| Vec::<Vec<u8>>::try_from(v).ok())
            .unwrap_or_default()
            .into_iter()
            .filter_map(bytes_to_path)
            .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .collect();
    }
    let filters: Vec<RawFilter> = value(options, "filters").and_then(|v| Vec::<RawFilter>::try_from(v).ok()).unwrap_or_default();
    req.filters = filters.into_iter().map(to_filter).collect();
    if let Some(current) = value(options, "current_filter").and_then(|v| RawFilter::try_from(v).ok()).map(to_filter) {
        // Pela especificação: sem `filters`, o filtro atual é o único; com eles, tem
        // de ser um deles.
        match req.filters.iter().position(|f| *f == current) {
            Some(i) => req.current_filter = Some(i),
            None if req.filters.is_empty() => {
                req.filters.push(current);
                req.current_filter = Some(0);
            }
            None => {}
        }
    }
    let choices: Vec<RawChoice> = value(options, "choices").and_then(|v| Vec::<RawChoice>::try_from(v).ok()).unwrap_or_default();
    req.choices = choices
        .into_iter()
        .map(|(id, label, options, initial)| {
            let value = if options.is_empty() {
                (initial == "true").to_string()
            } else if options.iter().any(|(o, _)| *o == initial) {
                initial
            } else {
                options[0].0.clone()
            };
            ExtraChoice { id, label: label.replace('_', ""), options, value }
        })
        .collect();
    req
}

/// Argumentos do `mide-chooser` para o pedido.
pub fn chooser_args(req: &ChooserRequest) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec!["--mode".into(), req.mode.arg().into(), "--title".into(), req.title.clone().into()];
    if req.multiple {
        args.push("--multiple".into());
    }
    if let Some(folder) = &req.folder {
        args.push("--folder".into());
        args.push(folder.clone().into_os_string());
    }
    if let Some(name) = &req.name {
        args.extend(["--name".into(), name.into()]);
    }
    if let Some(label) = &req.accept_label {
        args.extend(["--accept".into(), label.into()]);
    }
    for filter in &req.filters {
        args.extend(["--filter".into(), filter.name.clone().into()]);
        for p in &filter.patterns {
            match p {
                Pattern::Glob(g) => args.extend(["--glob".into(), g.into()]),
                Pattern::Mime(m) => args.extend(["--mime".into(), m.into()]),
            }
        }
    }
    if let Some(i) = req.current_filter {
        args.extend(["--current-filter".into(), i.to_string().into()]);
    }
    for choice in &req.choices {
        args.extend(["--choice".into(), choice.id.clone().into(), choice.label.clone().into(), choice.value.clone().into()]);
        for (id, label) in &choice.options {
            args.extend(["--option".into(), id.into(), label.into()]);
        }
    }
    for name in &req.files {
        args.extend(["--save-name".into(), name.into()]);
    }
    args
}

/// Lê os argumentos do `mide-chooser` (o contrário de [`chooser_args`]).
pub fn parse_chooser_args(args: Vec<OsString>) -> Result<ChooserRequest, String> {
    let mut req = ChooserRequest::new(ChooserMode::Open, "Abrir");
    let mut it = args.into_iter();
    let text = |v: Option<OsString>, flag: &str| v.map(|v| v.to_string_lossy().into_owned()).ok_or_else(|| format!("{flag} sem valor"));
    while let Some(arg) = it.next() {
        match arg.to_string_lossy().as_ref() {
            "--mode" => {
                req.mode = match text(it.next(), "--mode")?.as_str() {
                    "open" => ChooserMode::Open,
                    "folder" => ChooserMode::Folder,
                    "save" => ChooserMode::Save,
                    "save-files" => ChooserMode::SaveFiles,
                    other => return Err(format!("modo desconhecido: {other}")),
                }
            }
            "--title" => req.title = text(it.next(), "--title")?,
            "--multiple" => req.multiple = true,
            "--folder" => req.folder = Some(PathBuf::from(it.next().ok_or("--folder sem valor")?)),
            "--name" => req.name = Some(text(it.next(), "--name")?),
            "--accept" => req.accept_label = Some(text(it.next(), "--accept")?),
            "--filter" => req.filters.push(Filter { name: text(it.next(), "--filter")?, patterns: Vec::new() }),
            "--glob" | "--mime" => {
                let mime = arg == "--mime";
                let p = text(it.next(), "--glob")?;
                let filter = req.filters.last_mut().ok_or("padrão sem --filter")?;
                filter.patterns.push(if mime { Pattern::Mime(p) } else { Pattern::Glob(p) });
            }
            "--current-filter" => req.current_filter = text(it.next(), "--current-filter")?.parse().ok(),
            "--choice" => {
                let id = text(it.next(), "--choice")?;
                let label = text(it.next(), "--choice")?;
                let value = text(it.next(), "--choice")?;
                req.choices.push(ExtraChoice { id, label, options: Vec::new(), value });
            }
            "--option" => {
                let id = text(it.next(), "--option")?;
                let label = text(it.next(), "--option")?;
                req.choices.last_mut().ok_or("--option sem --choice")?.options.push((id, label));
            }
            "--save-name" => req.files.push(text(it.next(), "--save-name")?),
            other => return Err(format!("argumento desconhecido: {other}")),
        }
    }
    Ok(req)
}

/// O que o `mide-chooser` escolheu.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChooserOutput {
    pub paths: Vec<PathBuf>,
    pub filter: Option<usize>,
    pub choices: Vec<(String, String)>,
}

/// Saída do `mide-chooser` (uma linha por item; os caminhos como `file://`, sem perder
/// bytes de nomes que não são UTF-8).
pub fn format_output(out: &ChooserOutput) -> String {
    let mut text = String::new();
    for path in &out.paths {
        text.push_str(&format!("uri\t{}\n", crate::places::file_uri(path)));
    }
    if let Some(i) = out.filter {
        text.push_str(&format!("filter\t{i}\n"));
    }
    for (id, value) in &out.choices {
        text.push_str(&format!("choice\t{}\t{}\n", id.replace(['\t', '\n'], " "), value.replace(['\t', '\n'], " ")));
    }
    text
}

/// Lê a saída do `mide-chooser`.
pub fn parse_output(text: &str) -> ChooserOutput {
    let mut out = ChooserOutput::default();
    for line in text.lines() {
        let mut parts = line.splitn(3, '\t');
        match (parts.next(), parts.next(), parts.next()) {
            (Some("uri"), Some(uri), None) => out.paths.extend(crate::places::path_from_uri(uri)),
            (Some("filter"), Some(i), None) => out.filter = i.parse().ok(),
            (Some("choice"), Some(id), Some(value)) => out.choices.push((id.to_string(), value.to_string())),
            _ => {}
        }
    }
    out
}

fn owned(v: Value<'_>) -> OwnedValue {
    // Sem descritores de arquivo nestes valores: a conversão não falha.
    OwnedValue::try_from(v).expect("valor sem descritores")
}

/// Resultado do portal (`a{sv}`): `uris`, `choices` e `current_filter`.
pub fn results(req: &ChooserRequest, out: &ChooserOutput) -> HashMap<String, OwnedValue> {
    let mut map = HashMap::new();
    let uris: Vec<String> = out.paths.iter().map(|p| crate::places::file_uri(p)).collect();
    map.insert("uris".to_string(), owned(Value::from(uris)));
    if !req.choices.is_empty() {
        map.insert("choices".to_string(), owned(Value::from(out.choices.clone())));
    }
    if let Some(filter) = out.filter.and_then(|i| req.filters.get(i)) {
        map.insert("current_filter".to_string(), owned(Value::from(from_filter(filter))));
    }
    if req.mode == ChooserMode::Save || req.mode == ChooserMode::SaveFiles {
        map.insert("writable".to_string(), owned(Value::from(true)));
    }
    map
}

/// Pedidos do `FileManager1`: o que abrir no Arquivos. Cada item é a lista de
/// argumentos de um `mide-files` (uma janela).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShowKind {
    Folders,
    Items,
    Properties,
}

/// Traduz os endereços de um pedido `ShowFolders`/`ShowItems`/`ShowItemProperties` em
/// janelas do Arquivos. `ShowItems` junta os itens da mesma pasta numa janela só;
/// endereços que não são de arquivos ficam de fora (a Lixeira, `trash:`, abre a Lixeira).
/// Devolve as janelas e os endereços recusados.
pub fn file_manager_windows(kind: ShowKind, uris: &[String]) -> (Vec<Vec<OsString>>, Vec<String>) {
    let mut windows: Vec<Vec<OsString>> = Vec::new();
    let mut refused = Vec::new();
    let mut groups: Vec<(PathBuf, Vec<PathBuf>)> = Vec::new();
    for uri in uris {
        if uri.starts_with("trash:") {
            if kind != ShowKind::Properties && !windows.iter().any(|w| w.first().is_some_and(|a| a == OsStr::new("trash:///"))) {
                windows.push(vec!["trash:///".into()]);
            }
            continue;
        }
        let Some(path) = crate::places::path_from_uri(uri) else {
            refused.push(uri.clone());
            continue;
        };
        match kind {
            ShowKind::Folders => windows.push(vec![path.into_os_string()]),
            ShowKind::Properties => windows.push(vec!["--info".into(), path.into_os_string()]),
            ShowKind::Items => {
                let parent = path.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("/"));
                match groups.iter_mut().find(|(p, _)| *p == parent) {
                    Some((_, items)) => items.push(path),
                    None => groups.push((parent, vec![path])),
                }
            }
        }
    }
    for (_, items) in groups {
        let mut args: Vec<OsString> = vec!["--select".into()];
        args.extend(items.into_iter().map(PathBuf::into_os_string));
        windows.push(args);
    }
    (windows, refused)
}

/// Pasta de portais montada pelo mide quando o `mide.portal` não está instalado em
/// `/usr/share/xdg-desktop-portal/portals`: o `xdg-desktop-portal` só lê os `.portal`
/// de um lugar (`XDG_DESKTOP_PORTAL_DIR` ou o do sistema), então a pasta tem links para
/// todos os do sistema mais o do mide. Devolve a pasta quando ela é necessária.
pub fn portal_dir(system: &Path, ours: &Path, target: &Path) -> std::io::Result<Option<PathBuf>> {
    if system.join("mide.portal").is_file() || !ours.is_file() {
        return Ok(None);
    }
    std::fs::create_dir_all(target)?;
    // Links velhos (portal desinstalado) saem; os do sistema e o do mide entram de novo.
    for entry in std::fs::read_dir(target)?.flatten() {
        if entry.path().extension().is_some_and(|e| e == "portal") {
            std::fs::remove_file(entry.path())?;
        }
    }
    if let Ok(read) = std::fs::read_dir(system) {
        for entry in read.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "portal") {
                std::os::unix::fs::symlink(&path, target.join(entry.file_name()))?;
            }
        }
    }
    std::os::unix::fs::symlink(ours, target.join("mide.portal"))?;
    Ok(Some(target.to_path_buf()))
}

/// Bytes de um caminho com o `\0` do fim, como o portal manda (`ay`).
pub fn path_bytes(path: &Path) -> Vec<u8> {
    let mut bytes = path.as_os_str().as_bytes().to_vec();
    bytes.push(0);
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(pairs: Vec<(&str, Value<'static>)>) -> HashMap<String, OwnedValue> {
        pairs.into_iter().map(|(k, v)| (k.to_string(), owned(v))).collect()
    }

    #[test]
    fn opcoes_de_abrir() {
        let filters: Vec<RawFilter> = vec![
            ("Imagens".into(), vec![(1, "image/*".into()), (0, "*.xcf".into())]),
            ("Textos".into(), vec![(0, "*.txt".into())]),
        ];
        let choices: Vec<RawChoice> = vec![
            ("ro".into(), "Só _leitura".into(), vec![], "true".into()),
            ("enc".into(), "Codificação".into(), vec![("utf8".into(), "UTF-8".into()), ("latin1".into(), "Latin-1".into())], "latin1".into()),
        ];
        let o = opts(vec![
            ("multiple", Value::from(true)),
            ("accept_label", Value::from("_Abrir")),
            ("current_folder", Value::from(path_bytes(Path::new("/tmp/Área de trabalho")))),
            ("filters", Value::from(filters.clone())),
            ("current_filter", Value::from(filters[1].clone())),
            ("choices", Value::from(choices)),
            ("handle_token", Value::from("t1")),
        ]);
        let req = decode_request(Method::OpenFile, "Abrir arquivo", &o);
        assert_eq!(req.mode, ChooserMode::Open);
        assert!(req.multiple);
        assert_eq!(req.folder.as_deref(), Some(Path::new("/tmp/Área de trabalho")));
        assert_eq!(req.accept_label.as_deref(), Some("_Abrir"));
        assert_eq!(req.filters.len(), 2);
        assert_eq!(req.filters[0].patterns, vec![Pattern::Mime("image/*".into()), Pattern::Glob("*.xcf".into())]);
        assert_eq!(req.current_filter, Some(1));
        assert_eq!(req.choices[0].label, "Só leitura");
        assert_eq!(req.choices[0].value, "true");
        assert_eq!(req.choices[1].value, "latin1");
        // Ida e volta pelos argumentos do mide-chooser.
        assert_eq!(parse_chooser_args(chooser_args(&req)).unwrap(), req);
        // Pasta em vez de arquivo; tipo errado é ignorado.
        let req = decode_request(Method::OpenFile, "Pasta", &opts(vec![("directory", Value::from(true)), ("multiple", Value::from("sim"))]));
        assert_eq!(req.mode, ChooserMode::Folder);
        assert!(!req.multiple);
        // Só o filtro atual: vira o único.
        let req = decode_request(Method::OpenFile, "x", &opts(vec![("current_filter", Value::from(filters[0].clone()))]));
        assert_eq!((req.filters.len(), req.current_filter), (1, Some(0)));
    }

    #[test]
    fn opcoes_de_salvar() {
        let o = opts(vec![("current_name", Value::from("nota.txt")), ("current_folder", Value::from(path_bytes(Path::new("/tmp"))))]);
        let req = decode_request(Method::SaveFile, "Salvar", &o);
        assert_eq!((req.mode, req.name.as_deref(), req.folder.as_deref()), (ChooserMode::Save, Some("nota.txt"), Some(Path::new("/tmp"))));
        let o = opts(vec![("current_name", Value::from("x")), ("current_file", Value::from(path_bytes(Path::new("/home/ana/doc velho.odt"))))]);
        let req = decode_request(Method::SaveFile, "Salvar", &o);
        assert_eq!((req.name.as_deref(), req.folder.as_deref()), (Some("doc velho.odt"), Some(Path::new("/home/ana"))));
        let files: Vec<Vec<u8>> = vec![path_bytes(Path::new("a.png")), path_bytes(Path::new("/x/b c.png"))];
        let req = decode_request(Method::SaveFiles, "Salvar", &opts(vec![("files", Value::from(files))]));
        assert_eq!(req.files, ["a.png", "b c.png"]);
        assert_eq!(parse_chooser_args(chooser_args(&req)).unwrap(), req);
    }

    #[test]
    fn saida_e_resultado() {
        let req = decode_request(
            Method::OpenFile,
            "x",
            &opts(vec![
                ("filters", Value::from(vec![("Textos".to_string(), vec![(0u32, "*.txt".to_string())])])),
                ("choices", Value::from(vec![("ro".to_string(), "RO".to_string(), Vec::<(String, String)>::new(), "false".to_string())])),
            ]),
        );
        let out = ChooserOutput { paths: vec![PathBuf::from("/tmp/a b/ç.txt"), PathBuf::from("/tmp/x.txt")], filter: Some(0), choices: vec![("ro".into(), "true".into())] };
        let text = format_output(&out);
        assert!(text.contains("uri\tfile:///tmp/a%20b/%C3%A7.txt\n"));
        assert_eq!(parse_output(&text), out);
        let map = results(&req, &out);
        let uris = Vec::<String>::try_from(Value::from(map["uris"].try_clone().unwrap())).unwrap();
        assert_eq!(uris, ["file:///tmp/a%20b/%C3%A7.txt", "file:///tmp/x.txt"]);
        let choices = Vec::<(String, String)>::try_from(Value::from(map["choices"].try_clone().unwrap())).unwrap();
        assert_eq!(choices, [("ro".to_string(), "true".to_string())]);
        let filter = RawFilter::try_from(Value::from(map["current_filter"].try_clone().unwrap())).unwrap();
        assert_eq!(filter.0, "Textos");
        assert!(!map.contains_key("writable"));
    }

    #[test]
    fn janelas_do_gerenciador_de_arquivos() {
        let uris = vec![
            "file:///tmp/a/x.txt".to_string(),
            "file:///tmp/b/y.txt".to_string(),
            "file:///tmp/a/z%20w.txt".to_string(),
            "trash:///".to_string(),
            "sftp://host/a".to_string(),
        ];
        let (windows, refused) = file_manager_windows(ShowKind::Items, &uris);
        let w: Vec<Vec<String>> = windows.iter().map(|w| w.iter().map(|a| a.to_string_lossy().into_owned()).collect()).collect();
        assert_eq!(w, vec![vec!["trash:///"], vec!["--select", "/tmp/a/x.txt", "/tmp/a/z w.txt"], vec!["--select", "/tmp/b/y.txt"]]);
        assert_eq!(refused, ["sftp://host/a"]);
        let (windows, _) = file_manager_windows(ShowKind::Folders, &uris[..2]);
        assert_eq!(windows, vec![vec![OsString::from("/tmp/a/x.txt")], vec![OsString::from("/tmp/b/y.txt")]]);
        let (windows, _) = file_manager_windows(ShowKind::Properties, &uris[..1]);
        assert_eq!(windows, vec![vec![OsString::from("--info"), OsString::from("/tmp/a/x.txt")]]);
    }

    #[test]
    fn pasta_de_portais_com_links() {
        let base = std::env::temp_dir().join(format!("mide-portais-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (system, target) = (base.join("sistema"), base.join("alvo"));
        std::fs::create_dir_all(&system).unwrap();
        std::fs::write(system.join("gtk.portal"), "[portal]").unwrap();
        let ours = base.join("mide.portal");
        std::fs::write(&ours, "[portal]").unwrap();
        assert_eq!(portal_dir(&system, &ours, &target).unwrap(), Some(target.clone()));
        let mut names: Vec<String> = std::fs::read_dir(&target).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        assert_eq!(names, ["gtk.portal", "mide.portal"]);
        // De novo, depois de um portal sair do sistema: a pasta acompanha.
        std::fs::remove_file(system.join("gtk.portal")).unwrap();
        portal_dir(&system, &ours, &target).unwrap();
        assert_eq!(std::fs::read_dir(&target).unwrap().count(), 1);
        // Instalado no sistema: a pasta não é necessária.
        std::fs::write(system.join("mide.portal"), "[portal]").unwrap();
        assert_eq!(portal_dir(&system, &ours, &target).unwrap(), None);
        let _ = std::fs::remove_dir_all(&base);
    }
}
