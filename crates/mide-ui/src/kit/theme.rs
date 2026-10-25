//! Tema do mide-ui: cores, cantos e tamanhos num lugar só. Os componentes leem tudo
//! daqui, então mudar o tema muda todos os apps feitos com o mide-ui.

use tiny_skia::Color;

/// Medidas do Tahoe (`docs/tahoe.md`, seção 2).
/// Janela com barra lateral ou barra de ferramentas.
pub const RADIUS_WINDOW: f32 = 24.0;
/// Recuo dos painéis flutuantes (barra lateral) até a borda da janela.
pub const INSET_FLOAT: f32 = 8.0;
/// Barra lateral flutuante (raio da janela menos o recuo).
pub const RADIUS_SIDEBAR: f32 = RADIUS_WINDOW - INSET_FLOAT;
/// Seleção dentro de menus e da barra lateral, linhas de lista.
pub const RADIUS_ITEM: f32 = 8.0;
/// Barra de ferramentas e as cápsulas dela.
pub const TOOLBAR_HEIGHT: f32 = 52.0;
pub const TOOLBAR_ITEM: f32 = 36.0;

/// Cor em RGBA (0 a 255).
pub type Rgba = [u8; 4];

/// Aparência dos componentes.
#[derive(Debug, Clone)]
pub struct Theme {
    /// Fundo da janela.
    pub background: Rgba,
    /// Fundo da barra lateral.
    pub sidebar: Rgba,
    /// Fundo dos cartões.
    pub card: Rgba,
    /// Linhas finas (bordas e separadores).
    pub line: Rgba,
    /// Texto principal e secundário.
    pub text: Rgba,
    pub text_secondary: Rgba,
    /// Nome embaixo dos ladrilhos (`tile`). Claro (sobre o papel de parede), ganha uma
    /// sombra para ser lido em qualquer fundo.
    pub tile_text: Rgba,
    /// Cor de destaque (botão principal, interruptor ligado, seleção).
    pub accent: Rgba,
    /// Texto sobre a cor de destaque.
    pub on_accent: Rgba,
    /// Fundo de botões comuns e de itens sob o ponteiro.
    pub control: Rgba,
    pub hover: Rgba,
    /// Base dos tons de contraste (trilhos, bordas de campos, listras, barras de
    /// rolagem): preto no tema claro, branco no escuro; a transparência vem de quem usa
    /// (`Theme::shade`).
    pub ink: Rgba,
    /// Fundo dos menus de contexto e listas suspensas.
    pub menu: Rgba,
    /// Ações destrutivas (Apagar, Mover para a Lixeira).
    pub danger: Rgba,
    /// Barra lateral sobre o vidro (o desfoque aparece através dela).
    pub sidebar_glass: Rgba,
    /// Tema escuro (apps que desenham por conta própria escolhem as cores por aqui).
    pub dark: bool,
    /// Cantos dos cartões e dos controles.
    pub radius: f32,
    pub control_radius: f32,
    /// Tamanhos de fonte.
    pub font: f32,
    pub font_small: f32,
    pub font_title: f32,
    /// Altura de uma linha de controle (botão, campo, item de lista).
    pub row: f32,
    /// Espaço entre itens empilhados e margem das bordas.
    pub spacing: f32,
    pub padding: f32,
}

impl Default for Theme {
    /// Tema claro do mide, o mesmo das Configurações do Sistema.
    fn default() -> Self {
        Self {
            background: [246, 246, 248, 255],
            sidebar: [232, 232, 236, 255],
            card: [255, 255, 255, 255],
            line: [0, 0, 0, 24],
            text: [24, 24, 28, 255],
            text_secondary: [130, 130, 138, 255],
            tile_text: [24, 24, 28, 255],
            accent: [10, 110, 235, 255],
            on_accent: [255, 255, 255, 255],
            control: [0, 0, 0, 16],
            hover: [0, 0, 0, 28],
            ink: [0, 0, 0, 255],
            menu: [246, 246, 248, 245],
            danger: [210, 40, 40, 255],
            sidebar_glass: [248, 248, 250, 178],
            dark: false,
            radius: 10.0,
            control_radius: 7.0,
            font: 13.5,
            font_small: 12.0,
            font_title: 20.0,
            row: 32.0,
            spacing: 10.0,
            padding: 24.0,
        }
    }
}

impl Theme {
    /// Tema escuro do mide: fundos grafite, texto claro e o mesmo azul de destaque
    /// (um pouco mais claro, para ler bem sobre o escuro).
    pub fn dark() -> Self {
        Self {
            background: [30, 30, 32, 255],
            sidebar: [40, 40, 44, 255],
            card: [44, 44, 48, 255],
            line: [255, 255, 255, 26],
            text: [236, 236, 240, 255],
            text_secondary: [152, 152, 160, 255],
            tile_text: [236, 236, 240, 255],
            accent: [30, 125, 245, 255],
            on_accent: [255, 255, 255, 255],
            control: [255, 255, 255, 22],
            hover: [255, 255, 255, 34],
            ink: [255, 255, 255, 255],
            menu: [44, 44, 48, 245],
            danger: [255, 95, 85, 255],
            sidebar_glass: [36, 36, 40, 178],
            dark: true,
            ..Self::default()
        }
    }

    /// Tema escolhido em Configurações → Aparência (`[appearance] theme` no
    /// `config.toml`: "light" ou "dark"), com a cor de destaque escolhida
    /// (`[appearance] accent`).
    pub fn system() -> Self {
        let dark = system_dark();
        let mut theme = if dark { Self::dark() } else { Self::default() };
        theme.accent = system_accent(dark);
        theme
    }

    /// Tom de contraste com a transparência `alpha` (preto no claro, branco no escuro).
    pub fn shade(&self, alpha: u8) -> Rgba {
        [self.ink[0], self.ink[1], self.ink[2], alpha]
    }
}

/// Cores de destaque (Configurações → Aparência), como as do Mac: chave gravada, nome
/// na tela e a cor no tema claro e no escuro (um pouco mais clara no escuro, para ler
/// bem). Todas aguentam texto branco por cima nos dois temas.
pub const ACCENTS: &[(&str, &str, [u8; 3], [u8; 3])] = &[
    ("blue", "Azul", [10, 110, 235], [30, 125, 245]),
    ("purple", "Roxo", [135, 70, 200], [160, 95, 225]),
    ("pink", "Rosa", [215, 50, 125], [235, 80, 150]),
    ("red", "Vermelho", [215, 45, 45], [235, 75, 70]),
    ("orange", "Laranja", [225, 110, 10], [240, 130, 35]),
    ("yellow", "Amarelo", [195, 140, 0], [215, 160, 10]),
    ("green", "Verde", [35, 150, 65], [50, 170, 80]),
    ("graphite", "Grafite", [115, 115, 125], [140, 140, 150]),
];

/// Chave da cor de destaque escolhida (`blue` se nenhuma ou desconhecida).
pub fn accent_key() -> String {
    match crate::settings_file::cached("appearance", "accent") {
        Some(crate::settings_file::Value::Text(k)) if ACCENTS.iter().any(|(key, ..)| *key == k) => k,
        _ => "blue".into(),
    }
}

/// A cor de destaque da sessão no tema claro ou escuro. Fora do mide, a do sistema
/// (portal do freedesktop, `accent-color`), se ele tiver uma.
pub fn system_accent(dark: bool) -> Rgba {
    if !crate::control_client::on_mide() {
        if let Some(c) = portal_appearance().1 {
            return [c[0], c[1], c[2], 255];
        }
    }
    let key = accent_key();
    let (_, _, light, dim) = ACCENTS.iter().find(|(k, ..)| *k == key).copied().unwrap_or(ACCENTS[0]);
    let c = if dark { dim } else { light };
    [c[0], c[1], c[2], 255]
}

/// O tema da sessão é o escuro. No mide, a escolha das Configurações; fora dele, a do
/// sistema (portal do freedesktop, `color-scheme`: GNOME, KDE e outros).
pub fn system_dark() -> bool {
    if !crate::control_client::on_mide() {
        return portal_appearance().0;
    }
    matches!(crate::settings_file::cached("appearance", "theme"), Some(crate::settings_file::Value::Text(s)) if s == "dark")
}

/// Aparência pedida pelo sistema fora do mide, pelo portal do freedesktop
/// (`org.freedesktop.appearance`): escuro e a cor de destaque. Lida no máximo a cada
/// 2 s (os apps perguntam o tempo todo); sem portal, claro e sem cor.
fn portal_appearance() -> (bool, Option<[u8; 3]>) {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    };
    type Cache = Option<(std::time::Instant, (bool, Option<[u8; 3]>))>;
    static CACHE: Mutex<Cache> = Mutex::new(None);
    static READING: AtomicBool = AtomicBool::new(false);
    let cached = CACHE.lock().ok().and_then(|c| *c);
    // Velha ou nunca lida: lê numa thread (o portal pode demorar ou nem existir; a
    // interface nunca espera). Até chegar, vale o último valor (claro na primeira vez).
    if cached.is_none_or(|(at, _)| at.elapsed() > std::time::Duration::from_secs(2)) && !READING.swap(true, Ordering::AcqRel) {
        std::thread::spawn(|| {
            let value = read_portal_appearance().unwrap_or((false, None));
            if let Ok(mut cache) = CACHE.lock() {
                *cache = Some((std::time::Instant::now(), value));
            }
            READING.store(false, Ordering::Release);
        });
    }
    cached.map(|(_, v)| v).unwrap_or((false, None))
}

fn read_portal_appearance() -> Option<(bool, Option<[u8; 3]>)> {
    static CONN: std::sync::OnceLock<Option<zbus::blocking::Connection>> = std::sync::OnceLock::new();
    let conn = CONN.get_or_init(|| zbus::blocking::Connection::session().ok()).as_ref()?;
    let read = |key: &str| -> Option<zbus::zvariant::OwnedValue> {
        let reply = conn
            .call_method(Some("org.freedesktop.portal.Desktop"), "/org/freedesktop/portal/desktop", Some("org.freedesktop.portal.Settings"), "ReadOne", &("org.freedesktop.appearance", key))
            .ok()?;
        reply.body().deserialize::<zbus::zvariant::OwnedValue>().ok()
    };
    let dark = read("color-scheme").and_then(|v| u32::try_from(unwrap_variant(&v)).ok()).is_some_and(|n| n == 1);
    let accent = read("accent-color").and_then(|v| {
        let s = zbus::zvariant::Structure::try_from(unwrap_variant(&v)).ok()?;
        let f: Vec<f64> = s.fields().iter().filter_map(|x| f64::try_from(x).ok()).collect();
        // Fora de 0..1: o sistema não escolheu cor.
        (f.len() == 3 && f.iter().all(|c| (0.0..=1.0).contains(c))).then(|| [(f[0] * 255.0) as u8, (f[1] * 255.0) as u8, (f[2] * 255.0) as u8])
    });
    Some((dark, accent))
}

/// O portal às vezes manda o valor dentro de outro variant.
fn unwrap_variant(v: &zbus::zvariant::OwnedValue) -> zbus::zvariant::Value<'static> {
    let mut value: zbus::zvariant::Value<'static> = v.try_clone().map(Into::into).unwrap_or(zbus::zvariant::Value::from(0u32));
    while let zbus::zvariant::Value::Value(inner) = value {
        value = *inner;
    }
    value
}

/// Converte para a cor do tiny-skia.
pub fn color(c: Rgba) -> Color {
    Color::from_rgba8(c[0], c[1], c[2], c[3])
}
