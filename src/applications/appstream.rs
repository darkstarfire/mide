//! Catálogo de apps da distribuição (AppStream, formato DEP-11 do Debian), para a Loja:
//! nome, resumo e descrição no idioma do usuário, categorias, ícone em cache, site e
//! capturas de tela, lidos de `/var/lib/swcatalog/yaml/*.yml.gz` (o apt baixa junto com
//! a lista de pacotes) e os ícones de `/var/lib/swcatalog/icons`.
//!
//! O DEP-11 é YAML, mas só um pedaço dele: mapas, listas, textos simples ou entre
//! aspas e blocos `>-`/`|`. O leitor daqui entende esse pedaço (sem uma biblioteca de
//! YAML inteira).

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Um app do catálogo.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Component {
    pub id: String,
    pub package: String,
    pub name: String,
    pub summary: String,
    /// Texto em parágrafos (o HTML simples do AppStream já tirado).
    pub description: String,
    pub categories: Vec<String>,
    pub keywords: Vec<String>,
    pub homepage: String,
    pub license: String,
    pub developer: String,
    /// `.desktop` que abre o app (sem a extensão não muda nada).
    pub desktop_id: Option<String>,
    /// Ícone em cache (o maior), já com o caminho completo.
    pub icon: Option<PathBuf>,
    /// Nome do ícone no tema (`stock`), quando não há em cache.
    pub stock_icon: Option<String>,
    /// Capturas de tela (a imagem de tamanho médio de cada uma).
    pub screenshots: Vec<String>,
}

/// Um valor do YAML lido.
#[derive(Clone, Debug, PartialEq)]
enum Node {
    Text(String),
    List(Vec<Node>),
    Map(Vec<(String, Node)>),
}

impl Node {
    fn get(&self, key: &str) -> Option<&Node> {
        match self {
            Node::Map(m) => m.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    fn text(&self) -> Option<&str> {
        match self {
            Node::Text(t) => Some(t),
            _ => None,
        }
    }
    fn list(&self) -> &[Node] {
        match self {
            Node::List(l) => l,
            _ => &[],
        }
    }
}

/// Uma linha: recuo e o conteúdo sem ele.
struct Line<'a> {
    indent: usize,
    text: &'a str,
}

/// Lê um documento do YAML (entre `---`).
fn parse_document(text: &str) -> Node {
    let lines: Vec<Line> = text.lines().map(|l| Line { indent: l.len() - l.trim_start().len(), text: l.trim_end() }).collect();
    let mut i = 0;
    parse_block(&lines, &mut i, 0)
}

/// Lê o bloco que começa em `lines[*i]` com recuo `indent` (mapa ou lista).
fn parse_block(lines: &[Line], i: &mut usize, indent: usize) -> Node {
    // Pula linhas vazias para descobrir o tipo.
    while *i < lines.len() && lines[*i].text.trim().is_empty() {
        *i += 1;
    }
    if *i >= lines.len() {
        return Node::Map(Vec::new());
    }
    if lines[*i].text.trim_start().starts_with("- ") || lines[*i].text.trim() == "-" {
        parse_list(lines, i, lines[*i].indent)
    } else {
        parse_map(lines, i, indent.max(lines[*i].indent))
    }
}

fn parse_list(lines: &[Line], i: &mut usize, indent: usize) -> Node {
    let mut items = Vec::new();
    while *i < lines.len() {
        let line = &lines[*i];
        if line.text.trim().is_empty() {
            *i += 1;
            continue;
        }
        let trimmed = line.text.trim_start();
        if line.indent != indent || !(trimmed.starts_with("- ") || trimmed == "-") {
            break;
        }
        let rest = trimmed.strip_prefix('-').unwrap_or_default().trim_start();
        if rest.contains(": ") || rest.ends_with(':') {
            // Item que é um mapa: a primeira chave na mesma linha do "-".
            let inner = indent + (line.text.len() - line.indent - rest.len());
            let (key, value) = split_key(rest);
            *i += 1;
            let mut map = vec![(key.to_string(), parse_value(lines, i, inner, value))];
            if let Node::Map(more) = parse_map(lines, i, inner) {
                map.extend(more);
            }
            items.push(Node::Map(map));
        } else {
            items.push(Node::Text(unquote(rest)));
            *i += 1;
        }
    }
    Node::List(items)
}

fn parse_map(lines: &[Line], i: &mut usize, indent: usize) -> Node {
    let mut map = Vec::new();
    while *i < lines.len() {
        let line = &lines[*i];
        if line.text.trim().is_empty() {
            *i += 1;
            continue;
        }
        if line.indent != indent || line.text.trim_start().starts_with("- ") {
            break;
        }
        let (key, value) = split_key(line.text.trim_start());
        *i += 1;
        let node = parse_value(lines, i, indent, value);
        map.push((key.to_string(), node));
    }
    Node::Map(map)
}

/// O valor de `chave: valor`: texto na mesma linha, bloco `>-`/`|` ou um bloco mais
/// recuado nas linhas seguintes.
fn parse_value(lines: &[Line], i: &mut usize, indent: usize, value: &str) -> Node {
    let value = value.trim();
    if value.starts_with('>') || value.starts_with('|') {
        let folded = value.starts_with('>');
        let mut parts: Vec<String> = Vec::new();
        while *i < lines.len() && (lines[*i].text.trim().is_empty() || lines[*i].indent > indent) {
            parts.push(lines[*i].text.trim().to_string());
            *i += 1;
        }
        while parts.last().is_some_and(|p| p.is_empty()) {
            parts.pop();
        }
        return Node::Text(if folded { fold(&parts) } else { parts.join("\n") });
    }
    if !value.is_empty() {
        return Node::Text(unquote(value));
    }
    // Bloco nas próximas linhas (mais recuado; uma lista pode ficar no mesmo recuo).
    let next = lines[*i..].iter().find(|l| !l.text.trim().is_empty());
    match next {
        Some(n) if n.indent > indent || (n.indent == indent && n.text.trim_start().starts_with("- ")) => parse_block(lines, i, n.indent),
        _ => Node::Text(String::new()),
    }
}

/// Junta as linhas de um bloco `>`: linhas seguidas viram uma, linha vazia separa.
fn fold(parts: &[String]) -> String {
    let mut out = String::new();
    for p in parts {
        if p.is_empty() {
            out.push('\n');
        } else {
            if !out.is_empty() && !out.ends_with('\n') {
                out.push(' ');
            }
            out.push_str(p);
        }
    }
    out
}

fn split_key(text: &str) -> (&str, &str) {
    match text.split_once(": ") {
        Some((k, v)) => (k.trim(), v),
        None => (text.trim_end_matches(':').trim(), ""),
    }
}

fn unquote(text: &str) -> String {
    let t = text.trim();
    if t.len() >= 2 && t.starts_with('\'') && t.ends_with('\'') {
        return t[1..t.len() - 1].replace("''", "'");
    }
    if t.len() >= 2 && t.starts_with('"') && t.ends_with('"') {
        return t[1..t.len() - 1].replace("\\\"", "\"");
    }
    t.to_string()
}

/// Texto no idioma `lang` (ex.: "pt_BR" → "pt-BR", "pt_BR", "pt"), ou o padrão (`C`).
fn localized(node: Option<&Node>, lang: &str) -> String {
    let Some(node) = node else {
        return String::new();
    };
    let base = lang.split(['.', '@']).next().unwrap_or("C");
    let short = base.split('_').next().unwrap_or(base);
    let dashed = base.replace('_', "-");
    for key in [dashed.as_str(), base, short, "C", "en"] {
        if let Some(t) = node.get(key).and_then(Node::text).filter(|t| !t.is_empty()) {
            return t.to_string();
        }
    }
    node.text().unwrap_or_default().to_string()
}

/// O HTML simples das descrições (`<p>`, `<ul><li>`, `<em>`...) em texto com parágrafos.
pub fn html_to_text(html: &str) -> String {
    let mut out = String::new();
    let mut tag = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => {
                in_tag = true;
                tag.clear();
            }
            '>' if in_tag => {
                in_tag = false;
                let name = tag.trim_start_matches('/').split_whitespace().next().unwrap_or_default().to_lowercase();
                let closing = tag.starts_with('/');
                match (name.as_str(), closing) {
                    ("p", true) | ("ul", true) | ("ol", true) => out.push_str("\n\n"),
                    ("li", false) => out.push_str("• "),
                    ("li", true) => out.push('\n'),
                    _ => {}
                }
            }
            _ if in_tag => tag.push(c),
            _ => out.push(c),
        }
    }
    let text = out.replace("&apos;", "'").replace("&quot;", "\"").replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&");
    // Espaços repetidos e linhas vazias demais.
    let mut clean = String::new();
    let mut blank = 0;
    for line in text.lines() {
        let l = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if l.is_empty() {
            blank += 1;
            if blank == 1 && !clean.is_empty() {
                clean.push('\n');
            }
        } else {
            blank = 0;
            clean.push_str(&l);
            clean.push('\n');
        }
    }
    clean.trim().to_string()
}

/// Lê um catálogo DEP-11 (já descompactado): os apps de desktop, no idioma `lang`.
/// `icons` é a pasta dos ícones em cache (`/var/lib/swcatalog/icons`).
pub fn parse_catalog(text: &str, lang: &str, icons: &Path) -> Vec<Component> {
    // Nos testes (pasta de ícones que não existe), sem conferir os arquivos.
    let check_icons = icons.is_dir();
    let mut docs = text.split("\n---\n");
    let header = parse_document(docs.next().unwrap_or_default().trim_start_matches("---\n"));
    let origin = header.get("Origin").and_then(Node::text).unwrap_or_default().to_string();
    let media = header.get("MediaBaseUrl").and_then(Node::text).unwrap_or_default().to_string();
    let mut out = Vec::new();
    for doc in docs {
        // Só os apps de desktop (sem fontes, complementos, firmware...). Uma olhada rápida
        // antes de ler o documento inteiro.
        if !doc.starts_with("Type: desktop-application") {
            continue;
        }
        let node = parse_document(doc);
        let text_of = |k: &str| node.get(k).and_then(Node::text).unwrap_or_default().to_string();
        let package = text_of("Package");
        if package.is_empty() {
            continue;
        }
        // O maior ícone em cache.
        let icon = node
            .get("Icon")
            .and_then(|i| i.get("cached"))
            .map(|c| c.list())
            .unwrap_or_default()
            .iter()
            .filter_map(|c| {
                let name = c.get("name")?.text()?;
                let w: u32 = c.get("width")?.text()?.parse().ok()?;
                // Só os tamanhos baixados (o apt traz os pacotes de ícones de 48 e 64).
                let path = icons.join(&origin).join(format!("{w}x{w}")).join(name);
                if check_icons && !path.is_file() {
                    return None;
                }
                Some((w, path))
            })
            .max_by_key(|(w, _)| *w)
            .map(|(_, path)| path);
        let stock_icon = node.get("Icon").and_then(|i| i.get("stock")).and_then(Node::text).map(str::to_string);
        // A captura padrão primeiro; de cada uma, a miniatura de largura mais perto de 752.
        let mut shots: Vec<(bool, String)> = node
            .get("Screenshots")
            .map(|s| s.list())
            .unwrap_or_default()
            .iter()
            .filter_map(|s| {
                let default = s.get("default").and_then(Node::text) == Some("true");
                let thumb = s
                    .get("thumbnails")
                    .map(|t| t.list())
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|t| {
                        let url = t.get("url")?.text()?;
                        let w: i64 = t.get("width").and_then(Node::text).and_then(|w| w.parse().ok()).unwrap_or(0);
                        Some(((w - 752).abs(), url.to_string()))
                    })
                    .min_by_key(|(d, _)| *d)
                    .map(|(_, u)| u)
                    .or_else(|| s.get("source-image").and_then(|i| i.get("url")).and_then(Node::text).map(str::to_string))?;
                Some((default, thumb))
            })
            .collect();
        shots.sort_by_key(|(d, _)| !*d);
        let url = |u: &str| if u.starts_with("http") { u.to_string() } else { format!("{}/{}", media.trim_end_matches('/'), u) };
        let keywords = node.get("Keywords").map(|k| {
            let base = lang.split(['.', '@', '_']).next().unwrap_or("C");
            k.get(base).or_else(|| k.get("C")).map(|l| l.list().iter().filter_map(Node::text).map(str::to_string).collect()).unwrap_or_default()
        });
        out.push(Component {
            id: text_of("ID"),
            package,
            name: localized(node.get("Name"), lang),
            summary: localized(node.get("Summary"), lang),
            description: html_to_text(&localized(node.get("Description"), lang)),
            categories: node.get("Categories").map(|c| c.list().iter().filter_map(Node::text).map(str::to_string).collect()).unwrap_or_default(),
            keywords: keywords.unwrap_or_default(),
            homepage: node.get("Url").and_then(|u| u.get("homepage")).and_then(Node::text).unwrap_or_default().to_string(),
            license: text_of("ProjectLicense"),
            developer: localized(node.get("Developer").and_then(|d| d.get("name")), lang),
            desktop_id: node.get("Launchable").and_then(|l| l.get("desktop-id")).and_then(|d| d.list().first()).and_then(Node::text).map(str::to_string),
            icon,
            stock_icon,
            screenshots: shots.into_iter().map(|(_, u)| url(&u)).collect(),
        });
    }
    out
}

/// Todos os apps dos catálogos do sistema (`/var/lib/swcatalog/yaml/*.yml.gz`), no
/// idioma da sessão; um pacote aparece uma vez só.
pub fn load(lang: &str) -> Vec<Component> {
    let dir = Path::new("/var/lib/swcatalog/yaml");
    let icons = Path::new("/var/lib/swcatalog/icons");
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.to_string_lossy().ends_with(".yml.gz")).collect();
    files.sort();
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut out: Vec<Component> = Vec::new();
    for file in files {
        let Ok(f) = std::fs::File::open(&file) else { continue };
        let mut text = String::new();
        if flate2::read::GzDecoder::new(f).read_to_string(&mut text).is_err() {
            continue;
        }
        for c in parse_catalog(&text, lang, icons) {
            if !seen.contains_key(&c.package) {
                seen.insert(c.package.clone(), out.len());
                out.push(c);
            }
        }
    }
    out
}

/// Pacotes instalados e as versões deles, pelo estado do dpkg (rápido, sem D-Bus).
pub fn installed_packages() -> HashMap<String, String> {
    let text = std::fs::read_to_string("/var/lib/dpkg/status").unwrap_or_default();
    let mut out = HashMap::new();
    for entry in text.split("\n\n") {
        let field = |k: &str| entry.lines().find_map(|l| l.strip_prefix(k)).map(str::trim);
        if let (Some(name), Some(status)) = (field("Package:"), field("Status:"))
            && status.ends_with(" installed")
        {
            out.insert(name.to_string(), field("Version:").unwrap_or_default().to_string());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "---
File: DEP-11
Origin: debian-trixie-main
MediaBaseUrl: https://appstream.debian.org/media/trixie
---
Type: desktop-application
ID: org.example.Draw
Package: example-draw
ProjectLicense: GPL-3.0+
Name:
  C: Draw
  pt-BR: Desenhar
Summary:
  C: Draw things
Description:
  C: >-
    <p>A drawing app with <em>layers</em>.</p>

    <ul>
      <li>Fast</li>
      <li>Free</li>
    </ul>
Categories:
- Graphics
- 2DGraphics
Url:
  homepage: https://example.org
Icon:
  cached:
  - name: example-draw_org.example.Draw.png
    width: 48
    height: 48
  - name: example-draw_org.example.Draw.png
    width: 128
    height: 128
  stock: org.example.Draw
Launchable:
  desktop-id:
  - org.example.Draw.desktop
Screenshots:
- caption:
    C: Window
  thumbnails:
  - url: org/example/shot_224x125@1.png
    width: 224
    height: 125
  - url: org/example/shot_752x421@1.png
    width: 752
    height: 421
  source-image:
    url: org/example/shot.png
- default: true
  thumbnails:
  - url: org/example/main_752x421@1.png
    width: 752
---
Type: font
ID: some.font
Package: fonts-some
";

    #[test]
    fn le_um_app_do_catalogo() {
        let apps = parse_catalog(SAMPLE, "pt_BR.UTF-8", Path::new("/icons"));
        assert_eq!(apps.len(), 1);
        let a = &apps[0];
        assert_eq!((a.id.as_str(), a.package.as_str(), a.name.as_str(), a.summary.as_str()), ("org.example.Draw", "example-draw", "Desenhar", "Draw things"));
        assert_eq!(a.description, "A drawing app with layers.\n\n• Fast\n• Free");
        assert_eq!(a.categories, vec!["Graphics", "2DGraphics"]);
        assert_eq!(a.icon, Some(PathBuf::from("/icons/debian-trixie-main/128x128/example-draw_org.example.Draw.png")));
        assert_eq!(a.desktop_id.as_deref(), Some("org.example.Draw.desktop"));
        assert_eq!(a.homepage, "https://example.org");
        // A padrão primeiro, e de cada uma a miniatura de 752.
        assert_eq!(a.screenshots, vec!["https://appstream.debian.org/media/trixie/org/example/main_752x421@1.png", "https://appstream.debian.org/media/trixie/org/example/shot_752x421@1.png"]);
        assert_eq!(parse_catalog(SAMPLE, "C", Path::new("/icons"))[0].name, "Draw");
    }

    /// Precisa do catálogo do sistema (`appstream` instalado): `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn le_o_catalogo_do_sistema() {
        let apps = load("pt_BR.UTF-8");
        assert!(apps.len() > 500, "{} apps", apps.len());
        let inkscape = apps.iter().find(|a| a.package == "inkscape").expect("inkscape");
        assert!(!inkscape.summary.is_empty() && !inkscape.description.is_empty() && inkscape.icon.is_some());
    }
}
