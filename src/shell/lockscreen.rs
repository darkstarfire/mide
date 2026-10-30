//! Peças comuns das telas de login (`mide-login`) e de bloqueio (`mide-lock`), no
//! estilo do macOS Sonoma/Tahoe: a data pequena em cima da hora grande, a foto redonda
//! (ou as iniciais num círculo cinza), os botões redondos de vidro com o nome embaixo
//! (Desligar, Reiniciar, Repousar, Cancelar), os itens de status no canto de cima à
//! direita (sessão, layout do teclado, Wi-Fi e bateria) e o menu claro que eles abrem.
//! Também as opções da janela de início de sessão (`[login]` do config do mide).
//!
//! Tudo é desenhado em pontos pelas funções do mide-ui (nítido em qualquer escala); o
//! papel de parede desfocado atrás é do compositor.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use chrono::{Local, Timelike};
use tiny_skia::Pixmap;

use crate::{
    kit::Ui,
    ui::{
        draw::{blit_tinted, fill_rounded, rgba},
        text::{FontWeight, Text},
    },
};

/// Branco das letras sobre o papel de parede.
pub const WHITE: [u8; 4] = [255, 255, 255, 255];

/// Opções da janela de início de sessão (`[login]` do config do mide), como as do Mac
/// em Usuários e Grupos e Tela de Bloqueio.
#[derive(Debug, Clone, PartialEq)]
pub struct LoginOptions {
    /// Lista de usuários (`true`) ou campos de nome e senha.
    pub list: bool,
    /// Repousar, Reiniciar e Desligar na tela de login.
    pub power_buttons: bool,
    /// A dica de senha depois de três tentativas erradas.
    pub password_hints: bool,
    /// O menu de layout do teclado.
    pub input_menu: bool,
    /// Mensagem nas telas de login e de bloqueio (vazia: nenhuma).
    pub message: String,
}

impl Default for LoginOptions {
    fn default() -> Self {
        Self { list: true, power_buttons: true, password_hints: true, input_menu: true, message: String::new() }
    }
}

impl LoginOptions {
    /// Lê do config do mide (`$XDG_CONFIG_HOME/mide/config.toml`; na tela de login, a
    /// cópia em /etc/mide/login). Sem arquivo ou sem a seção, os padrões.
    pub fn load() -> Self {
        let dir = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")));
        dir.and_then(|d| std::fs::read_to_string(d.join("mide/config.toml")).ok()).map(|t| Self::parse(&t)).unwrap_or_default()
    }

    pub fn parse(text: &str) -> Self {
        let mut options = Self::default();
        let Ok(table) = text.parse::<toml::Table>() else {
            return options;
        };
        let Some(login) = table.get("login").and_then(|v| v.as_table()) else {
            return options;
        };
        let flag = |key: &str, default: bool| login.get(key).and_then(|v| v.as_bool()).unwrap_or(default);
        options.list = login.get("show").and_then(|v| v.as_str()) != Some("name");
        options.power_buttons = flag("power_buttons", true);
        options.password_hints = flag("password_hints", true);
        options.input_menu = flag("input_menu", true);
        options.message = login.get("message").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
        options
    }
}

/// Nome completo, foto e dica de senha de um usuário pelo AccountsService.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Account {
    pub name: String,
    pub icon: Option<PathBuf>,
    pub hint: String,
}

/// O que o AccountsService sabe de `login` (nada, sem ele).
pub fn account(login: &str) -> Option<Account> {
    let conn = zbus::blocking::Connection::system().ok()?;
    let accounts = zbus::blocking::Proxy::new(&conn, "org.freedesktop.Accounts", "/org/freedesktop/Accounts", "org.freedesktop.Accounts").ok()?;
    let path: zbus::zvariant::OwnedObjectPath = accounts.call("FindUserByName", &(login,)).ok()?;
    account_at(&conn, path)
}

/// Os dados do usuário no caminho do AccountsService.
pub fn account_at(conn: &zbus::blocking::Connection, path: zbus::zvariant::OwnedObjectPath) -> Option<Account> {
    let user = zbus::blocking::Proxy::new(conn, "org.freedesktop.Accounts", path, "org.freedesktop.Accounts.User").ok()?;
    let icon: String = user.get_property("IconFile").unwrap_or_default();
    Some(Account {
        name: user.get_property("RealName").unwrap_or_default(),
        icon: Some(PathBuf::from(icon)).filter(|p| p.is_file()),
        hint: user.get_property("PasswordHint").unwrap_or_default(),
    })
}

/// Iniciais do nome (como no Mac: primeira letra do primeiro e do último nome).
pub fn initials(name: &str) -> String {
    let words: Vec<&str> = name.split_whitespace().collect();
    let first = |w: &str| w.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
    match words.as_slice() {
        [] => String::new(),
        [one] => first(one),
        [a, .., b] => first(a) + &first(b),
    }
}

/// Texto branco com uma sombra leve embaixo (legível sobre qualquer papel de parede).
#[allow(clippy::too_many_arguments)]
pub fn shadowed(pixmap: &mut Pixmap, text: &mut Text, s: &str, x: f32, y: f32, size: f32, weight: FontWeight, color: [u8; 4]) {
    text.draw(pixmap, s, x, y + 1.0, size, weight, [0, 0, 0, (color[3] as u32 * 70 / 255) as u8]);
    text.draw(pixmap, s, x, y, size, weight, color);
}

/// Largura de `s` em pontos.
pub fn measure(ui: &mut Ui, s: &str, size: f32, weight: FontWeight) -> f32 {
    let mut width = 0.0;
    ui.paint_text(|_, text, _| width = text.measure(s, size, weight));
    width
}

/// Texto centrado em `cx`, com sombra.
pub fn centered(ui: &mut Ui, s: &str, cx: f32, y: f32, size: f32, weight: FontWeight, color: [u8; 4]) {
    let s = s.to_string();
    ui.paint_text(move |pixmap, text, (ox, oy)| {
        let w = text.measure(&s, size, weight);
        shadowed(pixmap, text, &s, ox + cx - w / 2.0, oy + y, size, weight, color);
    });
}

/// A data pequena em cima da hora grande, como no Sonoma e no Tahoe; `top` é o alto da
/// data. Devolve onde a hora termina.
pub fn clock(ui: &mut Ui, cx: f32, top: f32) -> f32 {
    let now = Local::now();
    let date = crate::calendar::long_date(now.date_naive());
    let time = format!("{}:{:02}", now.hour(), now.minute());
    centered(ui, &date, cx, top, 21.0, FontWeight::Semibold, [255, 255, 255, 235]);
    let size = 104.0;
    ui.paint_text(move |pixmap, text, (ox, oy)| {
        let w = text.measure(&time, size, FontWeight::Bold);
        let (x, y) = (ox + cx - w / 2.0, oy + top + 22.0);
        text.draw(pixmap, &time, x, y + 2.0, size, FontWeight::Bold, [0, 0, 0, 40]);
        text.draw(pixmap, &time, x, y, size, FontWeight::Bold, [255, 255, 255, 236]);
    });
    top + 22.0 + size * 1.2
}

/// Fotos redondas já reduzidas, por caminho e tamanho em pixels.
#[derive(Default)]
pub struct Avatars {
    cache: HashMap<(PathBuf, u32), Option<Pixmap>>,
}

impl Avatars {
    fn photo(&mut self, path: &Path, px: u32) -> Option<Pixmap> {
        self.cache
            .entry((path.to_path_buf(), px))
            .or_insert_with(|| {
                let image = image::open(path).ok()?.to_rgba8();
                // Recorte quadrado do meio, como as fotos de conta do Mac.
                let side = image.width().min(image.height());
                let square = image::imageops::crop_imm(&image, (image.width() - side) / 2, (image.height() - side) / 2, side, side).to_image();
                let image = image::imageops::resize(&square, px, px, image::imageops::FilterType::Lanczos3);
                let mut pixmap = Pixmap::new(px, px)?;
                for (i, p) in image.pixels().enumerate() {
                    let a = p[3] as u16;
                    let pm = |c: u8| ((c as u16 * a) / 255) as u8;
                    pixmap.pixels_mut()[i] = tiny_skia::PremultipliedColorU8::from_rgba(pm(p[0]), pm(p[1]), pm(p[2]), p[3]).unwrap_or(tiny_skia::PremultipliedColorU8::TRANSPARENT);
                }
                Some(pixmap)
            })
            .clone()
    }

    /// Desenha a foto (ou as iniciais num círculo cinza; sem nome, a silhueta de uma
    /// pessoa) com `size` pontos em (x, y). `lift` clareia (ponteiro em cima).
    #[allow(clippy::too_many_arguments)]
    pub fn paint(&mut self, ui: &mut Ui, icon: Option<&Path>, name: &str, x: f32, y: f32, size: f32, lift: bool) {
        let k = crate::ui::draw::scale();
        let px = (size * k).round() as u32;
        let photo = icon.and_then(|p| self.photo(p, px));
        let letters = initials(name);
        let person = letters.is_empty().then(|| crate::icons::symbol("avatar-default-symbolic", (size * 0.5 * k).round() as u32)).flatten();
        ui.paint_text(move |pixmap, text, (ox, oy)| {
            let (x, y) = (ox + x, oy + y);
            // Sombra suave embaixo do círculo.
            fill_rounded(pixmap, x - 1.0, y + 2.0, size + 2.0, size + 2.0, size / 2.0 + 1.0, rgba(0, 0, 0, 34));
            match photo {
                Some(photo) => {
                    let shader = tiny_skia::Pattern::new(photo.as_ref(), tiny_skia::SpreadMode::Pad, tiny_skia::FilterQuality::Bilinear, 1.0, tiny_skia::Transform::from_translate((x * k).round(), (y * k).round()));
                    let paint = tiny_skia::Paint { shader, anti_alias: true, ..Default::default() };
                    if let Some(circle) = tiny_skia::PathBuilder::from_circle((x + size / 2.0) * k, (y + size / 2.0) * k, size * k / 2.0) {
                        pixmap.fill_path(&circle, &paint, tiny_skia::FillRule::Winding, tiny_skia::Transform::identity(), None);
                    }
                }
                None => {
                    // Cinza em degradê, como os contatos sem foto no Mac.
                    if let Some(path) = crate::ui::draw::rounded_rect(x, y, size, size, size / 2.0) {
                        let paint = tiny_skia::Paint {
                            shader: crate::ui::draw::vertical_gradient(y, y + size, rgba(168, 172, 182, 255), rgba(116, 120, 130, 255)),
                            anti_alias: true,
                            ..Default::default()
                        };
                        pixmap.fill_path(&path, &paint, tiny_skia::FillRule::Winding, crate::ui::draw::tf(), None);
                    }
                    if let Some(icon) = &person {
                        let s = size * 0.5;
                        blit_tinted(pixmap, icon, (x + (size - s) / 2.0).round() as i32, (y + size * 0.24).round() as i32, [255, 255, 255]);
                    } else {
                        let font = size * if letters.chars().count() > 1 { 0.38 } else { 0.44 };
                        let w = text.measure(&letters, font, FontWeight::Semibold);
                        text.draw(pixmap, &letters, x + (size - w) / 2.0, y + (size - font * 1.25) / 2.0, font, FontWeight::Semibold, WHITE);
                    }
                }
            }
            if lift {
                fill_rounded(pixmap, x, y, size, size, size / 2.0, rgba(255, 255, 255, 40));
            }
        });
    }
}

/// Botão redondo de vidro com um símbolo e o nome embaixo (Desligar, Reiniciar,
/// Repousar, Cancelar), centrado em `cx`. Responde o clique (nunca nas telas-cópia).
pub fn circle_button(ui: &mut Ui, symbol: &str, label: &str, cx: f32, y: f32) -> bool {
    const SIZE: f32 = 42.0;
    let (x, w) = (cx - 48.0, 96.0);
    let over = ui.pointer().is_some_and(|(px, py)| px >= x && px < x + w && py >= y && py < y + SIZE + 28.0);
    let pressed = over && ui.input().down;
    let k = crate::ui::draw::scale();
    let icon = crate::icons::symbol(symbol, (18.0 * k).round() as u32);
    let label = label.to_string();
    ui.paint_text(move |pixmap, text, (ox, oy)| {
        let (bx, by) = (ox + cx - SIZE / 2.0, oy + y);
        crate::ui::draw::glass(pixmap, bx, by, SIZE, SIZE, SIZE / 2.0);
        let fill = if pressed { 70 } else if over { 46 } else { 14 };
        fill_rounded(pixmap, bx, by, SIZE, SIZE, SIZE / 2.0, rgba(255, 255, 255, fill));
        if let Some(icon) = &icon {
            blit_tinted(pixmap, icon, (bx + (SIZE - 18.0) / 2.0).round() as i32, (by + (SIZE - 18.0) / 2.0).round() as i32, [255, 255, 255]);
        }
        let lw = text.measure(&label, 12.0, FontWeight::Semibold);
        shadowed(pixmap, text, &label, ox + cx - lw / 2.0, by + SIZE + 7.0, 12.0, FontWeight::Semibold, [255, 255, 255, 240]);
    });
    if over {
        ui.request_tick();
    }
    over && ui.input().released && !ui.mirror()
}

/// Um item do canto de cima à direita: texto (sessão, layout do teclado) que abre um
/// menu. Desenha da direita para a esquerda a partir de `right`; devolve a área.
pub fn status_text(ui: &mut Ui, s: &str, right: f32, open: bool) -> (f32, f32, f32, f32) {
    let s = s.to_string();
    let (y, h) = (5.0, 24.0);
    let width = measure(ui, &s, 13.0, FontWeight::Semibold) + 20.0;
    let rect = (right - width, y, width, h);
    let over = ui.pointer().is_some_and(|(px, py)| px >= rect.0 && px < rect.0 + rect.2 && py >= rect.1 && py < rect.1 + rect.3);
    ui.paint_text(move |pixmap, text, (ox, oy)| {
        if open || over {
            fill_rounded(pixmap, ox + rect.0, oy + rect.1, rect.2, rect.3, 7.0, rgba(255, 255, 255, if open { 70 } else { 40 }));
        }
        shadowed(pixmap, text, &s, ox + rect.0 + 10.0, oy + y + (h - 13.0 * 1.25) / 2.0, 13.0, FontWeight::Semibold, WHITE);
    });
    rect
}

/// Ícone de Wi-Fi (quatro níveis, como na barra) terminando em `right`, com o sinal de
/// 0 a 100. Devolve a borda esquerda.
pub fn status_wifi(ui: &mut Ui, signal: u8, right: f32) -> f32 {
    let (w, x) = (18.0, right - 18.0 - 6.0);
    let bars = match signal {
        0..=24 => 1,
        25..=49 => 2,
        50..=74 => 3,
        _ => 4,
    };
    ui.paint_text(move |pixmap, _, (ox, oy)| {
        let (cx, cy) = (ox + x + w / 2.0, oy + 5.0 + 19.0);
        for i in 0..3 {
            let r = 4.5 + i as f32 * 4.2;
            let on = bars > i + 1;
            let mut pb = tiny_skia::PathBuilder::new();
            for s in 0..=16 {
                let a = std::f32::consts::PI * (1.25 + 0.5 * s as f32 / 16.0);
                let (px, py) = (cx + r * a.cos(), cy + r * a.sin());
                if s == 0 { pb.move_to(px, py) } else { pb.line_to(px, py) }
            }
            if let Some(path) = pb.finish() {
                let mut paint = tiny_skia::Paint { anti_alias: true, ..Default::default() };
                paint.set_color(rgba(255, 255, 255, if on { 255 } else { 90 }));
                let stroke = tiny_skia::Stroke { width: 2.2, line_cap: tiny_skia::LineCap::Round, ..Default::default() };
                pixmap.stroke_path(&path, &paint, &stroke, crate::ui::draw::tf(), None);
            }
        }
        fill_rounded(pixmap, cx - 1.6, cy - 2.0, 3.2, 3.2, 1.6, rgba(255, 255, 255, 255));
    });
    x
}

/// Ícone da bateria com a carga (e o raio carregando), terminando em `right`. Devolve a
/// borda esquerda.
pub fn status_battery(ui: &mut Ui, percent: u8, charging: bool, right: f32) -> f32 {
    let (w, h) = (25.0, 12.0);
    let x = right - w - 8.0;
    let y = 5.0 + (24.0 - h) / 2.0;
    ui.paint_text(move |pixmap, _, (ox, oy)| {
        let (x, y) = (ox + x, oy + y);
        if let Some(path) = crate::ui::draw::rounded_rect(x + 0.5, y + 0.5, w - 1.0, h - 1.0, 3.5) {
            let mut paint = tiny_skia::Paint { anti_alias: true, ..Default::default() };
            paint.set_color(rgba(255, 255, 255, 150));
            pixmap.stroke_path(&path, &paint, &tiny_skia::Stroke { width: 1.0, ..Default::default() }, crate::ui::draw::tf(), None);
        }
        fill_rounded(pixmap, x + w + 1.0, y + 4.0, 1.6, 4.0, 0.8, rgba(255, 255, 255, 150));
        let fill = (w - 4.0) * (percent.min(100) as f32 / 100.0).max(0.06);
        let color = if percent <= 10 && !charging { rgba(255, 69, 58, 255) } else { rgba(255, 255, 255, 255) };
        fill_rounded(pixmap, x + 2.0, y + 2.0, fill, h - 4.0, 2.0, color);
        if charging {
            let mut pb = tiny_skia::PathBuilder::new();
            let (cx, cy) = (x + w / 2.0, y + h / 2.0);
            pb.move_to(cx + 1.0, cy - 5.0);
            pb.line_to(cx - 3.2, cy + 0.8);
            pb.line_to(cx - 0.2, cy + 0.8);
            pb.line_to(cx - 1.0, cy + 5.0);
            pb.line_to(cx + 3.2, cy - 0.8);
            pb.line_to(cx + 0.2, cy - 0.8);
            pb.close();
            if let Some(path) = pb.finish() {
                let mut paint = tiny_skia::Paint { anti_alias: true, ..Default::default() };
                paint.set_color(rgba(40, 40, 44, 255));
                pixmap.stroke_path(&path, &paint, &tiny_skia::Stroke { width: 1.4, ..Default::default() }, crate::ui::draw::tf(), None);
                paint.set_color(rgba(255, 255, 255, 255));
                pixmap.fill_path(&path, &paint, tiny_skia::FillRule::Winding, crate::ui::draw::tf(), None);
            }
        }
    });
    x
}

/// Menu claro aberto embaixo de `anchor` (alinhado à direita dele), com `items` e o ✓
/// no `selected`. Desenhado por cima do resto: chamar no fim do quadro. Devolve o item
/// escolhido; `Some(None)` quando o clique foi fora (fechar).
pub fn menu(ui: &mut Ui, anchor: (f32, f32, f32, f32), items: &[String], selected: usize) -> Option<Option<usize>> {
    const ROW: f32 = 26.0;
    let font = 13.0;
    let width = items.iter().map(|s| measure(ui, s, font, FontWeight::Regular)).fold(120.0f32, f32::max) + 46.0;
    let height = ROW * items.len() as f32 + 10.0;
    let x = (anchor.0 + anchor.2 - width).max(8.0);
    let y = anchor.1 + anchor.3 + 4.0;
    let pointer = ui.pointer();
    let hover = pointer.and_then(|(px, py)| (px >= x && px < x + width && py >= y + 5.0 && py < y + height - 5.0).then(|| ((py - y - 5.0) / ROW) as usize));
    let accent = crate::kit::system_accent(false);
    let items = items.to_vec();
    ui.paint_text(move |pixmap, text, (ox, oy)| {
        let (x, y) = (ox + x, oy + y);
        crate::ui::draw::soft_shadow(pixmap, x, y, width, height, 10.0, 18.0, 6.0, 60);
        fill_rounded(pixmap, x, y, width, height, 10.0, rgba(246, 246, 248, 240));
        for (i, item) in items.iter().enumerate() {
            let ry = y + 5.0 + i as f32 * ROW;
            let hot = hover == Some(i);
            if hot {
                fill_rounded(pixmap, x + 5.0, ry, width - 10.0, ROW, 6.0, rgba(accent[0], accent[1], accent[2], 255));
            }
            let color = if hot { WHITE } else { [28, 28, 32, 255] };
            if i == selected {
                text.draw(pixmap, "✓", x + 12.0, ry + (ROW - font * 1.25) / 2.0, font, FontWeight::Semibold, color);
            }
            text.draw(pixmap, item, x + 30.0, ry + (ROW - font * 1.25) / 2.0, font, FontWeight::Regular, color);
        }
    });
    if ui.mirror() {
        return None;
    }
    if ui.input().released {
        if let Some(i) = hover {
            return Some(Some(i));
        }
        let on_anchor = pointer.is_some_and(|(px, py)| px >= anchor.0 && px < anchor.0 + anchor.2 && py >= anchor.1 && py < anchor.1 + anchor.3);
        if !on_anchor {
            return Some(None);
        }
    }
    None
}

/// Wi-Fi e bateria lidos numa thread de tempos em tempos (o `nmcli` pode demorar):
/// a tela só lê o último valor.
#[derive(Clone, Default)]
pub struct Status {
    inner: Arc<Mutex<StatusData>>,
}

#[derive(Debug, Clone, Default)]
pub struct StatusData {
    /// Sinal da rede Wi-Fi conectada (0–100), se houver.
    pub wifi: Option<u8>,
    /// Carga e se está carregando, se houver bateria.
    pub battery: Option<(u8, bool)>,
}

impl Status {
    /// Começa a leitura (a primeira logo, depois a cada 15 s).
    pub fn start() -> Self {
        let status = Self::default();
        let inner = status.inner.clone();
        std::thread::spawn(move || loop {
            let started = Instant::now();
            let wifi = std::process::Command::new("nmcli")
                .args(["-t", "-f", "ACTIVE,SSID,SIGNAL,SECURITY", "dev", "wifi", "list", "--rescan", "no"])
                .env("LC_ALL", "C")
                .output()
                .ok()
                .filter(|o| o.status.success())
                .and_then(|o| crate::system::parse_networks(&String::from_utf8_lossy(&o.stdout)).into_iter().find(|n| n.active))
                .map(|n| n.signal);
            let battery = crate::battery::read().map(|b| (b.percent, b.charging));
            if let Ok(mut data) = inner.lock() {
                *data = StatusData { wifi, battery };
            }
            std::thread::sleep(Duration::from_secs(15).saturating_sub(started.elapsed()));
        });
        status
    }

    pub fn get(&self) -> StatusData {
        self.inner.lock().map(|d| d.clone()).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opcoes_da_janela_de_login() {
        assert_eq!(LoginOptions::parse(""), LoginOptions::default());
        let o = LoginOptions::parse("[login]\nshow = \"name\"\npower_buttons = false\ninput_menu = false\nmessage = \" Este computador é da Ana \"\n");
        assert!(!o.list && !o.power_buttons && !o.input_menu && o.password_hints);
        assert_eq!(o.message, "Este computador é da Ana");
        assert!(LoginOptions::parse("[login]\nshow = \"list\"\n").list);
    }

    #[test]
    fn iniciais_como_no_mac() {
        assert_eq!(initials("Ana Souza"), "AS");
        assert_eq!(initials("Fabio Augusto Carneiro"), "FC");
        assert_eq!(initials("bruno"), "B");
        assert_eq!(initials(""), "");
    }
}
