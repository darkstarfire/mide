//! O que o terminal manda ao programa quando o usuário digita ou cola: sequências
//! do xterm para setas, F1–F12, Home/End..., com os modificadores no formato dele.

use super::screen::Modes;
use crate::kit::KeyPress;

/// Bytes de uma tecla, ou `None` se ela não manda nada (Shift sozinho, por exemplo).
pub fn key_bytes(press: &KeyPress, modes: &Modes) -> Option<Vec<u8>> {
    // Modificadores no formato do xterm: 1 + Shift + 2·Alt + 4·Ctrl.
    let mods = 1 + press.shift as u8 + 2 * press.alt as u8 + 4 * press.ctrl as u8;
    let cursor = |final_byte: char| -> Vec<u8> {
        if mods > 1 {
            format!("\x1b[1;{mods}{final_byte}").into_bytes()
        } else if modes.app_cursor {
            format!("\x1bO{final_byte}").into_bytes()
        } else {
            format!("\x1b[{final_byte}").into_bytes()
        }
    };
    let tilde = |code: u8| -> Vec<u8> {
        if mods > 1 { format!("\x1b[{code};{mods}~").into_bytes() } else { format!("\x1b[{code}~").into_bytes() }
    };
    let ss3 = |final_byte: char| -> Vec<u8> {
        if mods > 1 { format!("\x1b[1;{mods}{final_byte}").into_bytes() } else { format!("\x1bO{final_byte}").into_bytes() }
    };
    // Alt manda ESC antes (como o "Esc+" do iTerm2 para a tecla Option).
    let alt = |bytes: &[u8]| -> Vec<u8> {
        let mut out = Vec::with_capacity(bytes.len() + 1);
        if press.alt {
            out.push(0x1b);
        }
        out.extend_from_slice(bytes);
        out
    };
    let bytes = match press.keysym {
        0xff08 => alt(if press.ctrl { b"\x08" } else { b"\x7f" }),
        0xff09 if press.shift => b"\x1b[Z".to_vec(),
        0xff09 => alt(b"\t"),
        0xfe20 => b"\x1b[Z".to_vec(),
        0xff0d => alt(if modes.newline { b"\r\n" } else { b"\r" }),
        0xff8d if modes.app_keypad => b"\x1bOM".to_vec(),
        0xff8d => alt(b"\r"),
        0xff1b => alt(b"\x1b"),
        0xff52 | 0xff97 => cursor('A'),
        0xff54 | 0xff99 => cursor('B'),
        0xff53 | 0xff98 => cursor('C'),
        0xff51 | 0xff96 => cursor('D'),
        0xff50 | 0xff95 => cursor('H'),
        0xff57 | 0xff9c => cursor('F'),
        0xff63 | 0xff9e => tilde(2),
        0xffff | 0xff9f => tilde(3),
        0xff55 | 0xff9a => tilde(5),
        0xff56 | 0xff9b => tilde(6),
        0xffbe => ss3('P'),
        0xffbf => ss3('Q'),
        0xffc0 => ss3('R'),
        0xffc1 => ss3('S'),
        k @ 0xffc2..=0xffc9 => tilde([15, 17, 18, 19, 20, 21, 23, 24][(k - 0xffc2) as usize]),
        k => {
            if press.ctrl {
                // Ctrl+letra (e @ [ \ ] ^ _ espaço) vira o caractere de controle.
                let c = char::from_u32(k).filter(char::is_ascii)?.to_ascii_uppercase();
                let code = match c {
                    ' ' | '2' => 0,
                    '@'..='_' => c as u8 & 0x1f,
                    '/' | '7' => 0x1f,
                    '8' | '?' => 0x7f,
                    '3' => 0x1b,
                    '4' => 0x1c,
                    '5' => 0x1d,
                    '6' => 0x1e,
                    _ => return press.text.as_ref().filter(|t| !t.is_empty()).map(|t| alt(t.as_bytes())),
                };
                alt(&[code])
            } else {
                let text = press.text.as_ref().filter(|t| !t.is_empty() && t.chars().all(|c| !c.is_control()))?;
                alt(text.as_bytes())
            }
        }
    };
    Some(bytes)
}

/// Texto colado: dentro de marcadores se o programa pediu (bracketed paste, sem
/// deixar o texto fechar os marcadores antes da hora), e com as quebras de linha
/// como Enter.
pub fn paste_bytes(text: &str, bracketed: bool) -> Vec<u8> {
    let text = text.replace("\r\n", "\r").replace('\n', "\r");
    if bracketed {
        format!("\x1b[200~{}\x1b[201~", text.replace("\x1b[201~", "")).into_bytes()
    } else {
        text.into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(keysym: u32, text: &str, ctrl: bool, shift: bool, alt: bool) -> KeyPress {
        KeyPress { keysym, text: (!text.is_empty()).then(|| text.to_string()), ctrl, shift, alt, logo: false }
    }

    #[test]
    fn teclas_viram_sequencias_do_xterm() {
        let normal = Modes::default();
        let app = Modes { app_cursor: true, ..Modes::default() };
        assert_eq!(key_bytes(&press('a' as u32, "a", false, false, false), &normal), Some(b"a".to_vec()));
        assert_eq!(key_bytes(&press('c' as u32, "\x03", true, false, false), &normal), Some(vec![3]));
        assert_eq!(key_bytes(&press('b' as u32, "b", false, false, true), &normal), Some(b"\x1bb".to_vec()));
        assert_eq!(key_bytes(&press(0xff0d, "\r", false, false, false), &normal), Some(b"\r".to_vec()));
        assert_eq!(key_bytes(&press(0xff08, "", false, false, false), &normal), Some(vec![0x7f]));
        assert_eq!(key_bytes(&press(0xff52, "", false, false, false), &normal), Some(b"\x1b[A".to_vec()));
        assert_eq!(key_bytes(&press(0xff52, "", false, false, false), &app), Some(b"\x1bOA".to_vec()));
        assert_eq!(key_bytes(&press(0xff53, "", true, false, false), &normal), Some(b"\x1b[1;5C".to_vec()));
        assert_eq!(key_bytes(&press(0xffff, "", false, false, false), &normal), Some(b"\x1b[3~".to_vec()));
        assert_eq!(key_bytes(&press(0xffc2, "", false, false, false), &normal), Some(b"\x1b[15~".to_vec()));
        assert_eq!(key_bytes(&press(0xffe1, "", false, true, false), &normal), None);
    }

    #[test]
    fn colar_com_e_sem_marcadores() {
        assert_eq!(paste_bytes("a\nb", false), b"a\rb".to_vec());
        assert_eq!(paste_bytes("x\x1b[201~y", true), b"\x1b[200~xy\x1b[201~".to_vec());
    }
}
