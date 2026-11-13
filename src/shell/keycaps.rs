//! Atalhos desenhados como no macOS (⌃⌥⇧⌘ e as teclas ↩ ⎋ ⌫ ⇥) para os menus da
//! barra: os apps mandam o atalho em texto ("Ctrl+Shift+Z" no dbusmenu, "<Primary>z" no
//! GTK, "Super+H" nos do compositor), e a barra mostra "⇧⌘Z".
//!
//! Qual tecla é o ⌘ depende do teclado no estilo Mac (`[keyboard] mac_style`):
//! - estilo Mac: os apps recebem o ⌘ como Ctrl, então o Ctrl de um atalho de app é o
//!   ⌘ ("Ctrl+S" → "⌘S"); nos atalhos do mide (que têm o Super, o ⌘ do compositor), o
//!   Ctrl é o ⌃ de verdade ("Ctrl+Super+Q" → "⌃⌘Q");
//! - teclado comum: Ctrl → ⌃, Alt → ⌥, Shift → ⇧ e a tecla Super → ⌘.
//!
//! A ordem é a do macOS: ⌃ ⌥ ⇧ ⌘ e a tecla.

use std::borrow::Cow;

/// O atalho `text` desenhado; vazio continua vazio. Texto que não é um atalho
/// conhecido volta como veio.
pub fn glyphs(text: &str, mac_style: bool) -> String {
    let text = text.trim();
    if text.is_empty() {
        return String::new();
    }
    // Partes separadas por "+"; um "+" no fim é a própria tecla (Ctrl++).
    let (mods_part, key) = if let Some(rest) = text.strip_suffix("++") {
        (rest, "+")
    } else if text == "+" {
        ("", "+")
    } else {
        match text.rsplit_once('+') {
            Some((mods, key)) => (mods, key),
            None => ("", text),
        }
    };
    let (mut ctrl, mut alt, mut shift, mut sup) = (false, false, false, false);
    for part in mods_part.split('+').filter(|p| !p.is_empty()) {
        match part.trim().to_ascii_lowercase().as_str() {
            "ctrl" | "control" | "primary" => ctrl = true,
            "alt" | "option" | "opt" | "mod1" => alt = true,
            "shift" => shift = true,
            "super" | "logo" | "meta" | "cmd" | "command" | "win" => sup = true,
            // Algo que não é modificador: deixa o texto como veio.
            _ => return text.to_string(),
        }
    }
    // Atalho de app no estilo Mac: o Ctrl é o ⌘.
    let (control, command) = if mac_style && !sup { (false, ctrl) } else { (ctrl, sup) };
    let mut out = String::new();
    if control {
        out.push('⌃');
    }
    if alt {
        out.push('⌥');
    }
    if shift {
        out.push('⇧');
    }
    if command {
        out.push('⌘');
    }
    out.push_str(&key_glyph(key));
    out
}

/// A tecla, como no macOS.
fn key_glyph(key: &str) -> Cow<'static, str> {
    let name = key.trim();
    let glyph = match name.to_ascii_lowercase().as_str() {
        "enter" | "return" | "kp_enter" => "↩",
        "escape" | "esc" => "⎋",
        "backspace" => "⌫",
        "delete" | "del" => "⌦",
        "tab" | "iso_left_tab" => "⇥",
        "pgup" | "page_up" | "pageup" | "prior" => "⇞",
        "pgdn" | "page_down" | "pagedown" | "next" => "⇟",
        "home" => "↖",
        "end" => "↘",
        "up" => "↑",
        "down" => "↓",
        "left" => "←",
        "right" => "→",
        "plus" | "kp_add" => "+",
        "minus" | "kp_subtract" => "−",
        "equal" => "=",
        "comma" => ",",
        "period" => ".",
        "slash" => "/",
        "backslash" => "\\",
        "semicolon" => ";",
        "apostrophe" => "'",
        "grave" => "`",
        "bracketleft" => "[",
        "bracketright" => "]",
        "space" => return crate::tr!("Space"),
        _ => {
            return if name.chars().count() == 1 { Cow::Owned(name.to_uppercase()) } else { Cow::Owned(name.to_string()) };
        }
    };
    Cow::Borrowed(glyph)
}

/// O teclado está no estilo Mac (`[keyboard] mac_style`)? Lido do arquivo de novo a
/// cada 2 segundos no máximo (os menus montam muitos itens de uma vez).
pub fn mac_style() -> bool {
    use std::cell::Cell;
    use std::time::{Duration, Instant};
    thread_local! {
        static CACHE: Cell<Option<(Instant, bool)>> = const { Cell::new(None) };
    }
    CACHE.with(|cache| {
        if let Some((at, value)) = cache.get()
            && at.elapsed() < Duration::from_secs(2)
        {
            return value;
        }
        let value = matches!(crate::settings_file::get("keyboard", "mac_style"), Some(crate::settings_file::Value::Bool(true)));
        cache.set(Some((Instant::now(), value)));
        value
    })
}

#[cfg(test)]
mod tests {
    use super::glyphs;

    #[test]
    fn teclado_comum() {
        assert_eq!(glyphs("Ctrl+Shift+Z", false), "⌃⇧Z");
        assert_eq!(glyphs("Super+H", false), "⌘H");
        assert_eq!(glyphs("Ctrl+Alt+N", false), "⌃⌥N");
        assert_eq!(glyphs("Ctrl+Super+F", false), "⌃⌘F");
        assert_eq!(glyphs("Shift+Ctrl+Page_Down", false), "⌃⇧⇟");
    }

    #[test]
    fn estilo_mac() {
        // Atalho de app: o Ctrl é o ⌘.
        assert_eq!(glyphs("Ctrl+S", true), "⌘S");
        assert_eq!(glyphs("Ctrl+Alt+N", true), "⌥⌘N");
        assert_eq!(glyphs("Ctrl+Shift+T", true), "⇧⌘T");
        // Atalho do mide (com o Super): o Ctrl é o ⌃.
        assert_eq!(glyphs("Ctrl+Super+Q", true), "⌃⌘Q");
        assert_eq!(glyphs("Super+H", true), "⌘H");
    }

    #[test]
    fn teclas_e_casos_especiais() {
        assert_eq!(glyphs("", true), "");
        assert_eq!(glyphs("F11", false), "F11");
        assert_eq!(glyphs("Delete", false), "⌦");
        assert_eq!(glyphs("Ctrl+Return", false), "⌃↩");
        assert_eq!(glyphs("Ctrl++", true), "⌘+");
        assert_eq!(glyphs("Ctrl+minus", true), "⌘−");
        assert_eq!(glyphs("Shift+Tab", false), "⇧⇥");
        // Já desenhado (vem assim de algum lugar): fica como está.
        assert_eq!(glyphs("⌘⇧D", true), "⌘⇧D");
    }
}
