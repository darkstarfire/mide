//! Regras das notificações (`org.freedesktop.Notifications`), sem interface:
//! texto, ações e tempo de exibição. O servidor e os banners ficam em
//! `src/bin/mide-notifications.rs`.

use std::time::Duration;

/// Tempo padrão de um banner na tela, como no macOS.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

/// Urgência da notificação (dica `urgency` da especificação).
pub const URGENCY_CRITICAL: u8 = 2;

/// Motivos do sinal `NotificationClosed`.
pub mod reason {
    pub const EXPIRED: u32 = 1;
    pub const DISMISSED: u32 = 2;
    pub const CLOSED_BY_CALL: u32 = 3;
}

/// Quanto tempo o banner fica na tela. `expire_timeout` vem do app: -1 usa o
/// padrão, 0 nunca expira e um valor positivo é em milissegundos. Notificação
/// crítica fica até o usuário fechar, a menos que o app peça um tempo.
pub fn timeout(expire_timeout: i32, urgency: u8) -> Option<Duration> {
    match expire_timeout {
        0 => None,
        t if t > 0 => Some(Duration::from_millis(t as u64)),
        _ if urgency >= URGENCY_CRITICAL => None,
        _ => Some(DEFAULT_TIMEOUT),
    }
}

/// Texto puro a partir do corpo com marcação simples (`<b>`, `<i>`, `<a>`...):
/// tira as tags e troca as entidades mais comuns.
pub fn plain_text(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut in_tag = false;
    for c in body.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

/// Ações vêm como lista plana `[chave, rótulo, chave, rótulo, ...]`. A ação
/// `default` é a do clique no banner e não vira botão.
pub fn actions(list: &[String]) -> (bool, Vec<(String, String)>) {
    let mut has_default = false;
    let mut buttons = Vec::new();
    for pair in list.chunks(2) {
        let key = pair[0].clone();
        let label = pair.get(1).cloned().unwrap_or_default();
        if key == "default" {
            has_default = true;
        } else {
            buttons.push((key, label));
        }
    }
    (has_default, buttons)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tempo_de_exibicao() {
        assert_eq!(timeout(-1, 1), Some(DEFAULT_TIMEOUT));
        assert_eq!(timeout(0, 1), None);
        assert_eq!(timeout(1500, 1), Some(Duration::from_millis(1500)));
        assert_eq!(timeout(-1, URGENCY_CRITICAL), None);
        assert_eq!(timeout(3000, URGENCY_CRITICAL), Some(Duration::from_millis(3000)));
    }

    #[test]
    fn texto_sem_marcacao() {
        assert_eq!(plain_text("<b>Olá</b> &amp; <i>tchau</i>"), "Olá & tchau");
        assert_eq!(plain_text("a &lt; b"), "a < b");
        assert_eq!(plain_text("<a href=\"x\">link</a>"), "link");
    }

    #[test]
    fn acoes_padrao_e_botoes() {
        let list: Vec<String> = ["default", "Abrir", "reply", "Responder"].iter().map(|s| s.to_string()).collect();
        let (default, buttons) = actions(&list);
        assert!(default);
        assert_eq!(buttons, vec![("reply".to_string(), "Responder".to_string())]);
        assert_eq!(actions(&[]), (false, vec![]));
    }
}
