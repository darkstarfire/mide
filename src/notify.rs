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

/// Uma notificação no histórico (central de notificações): quando chegou (segundos
/// desde 1970), app, título e texto.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub time: u64,
    pub app: String,
    pub summary: String,
    pub body: String,
}

/// Quantas notificações o histórico guarda.
pub const HISTORY_MAX: usize = 50;

/// Arquivo do histórico, na pasta de execução e separado por sessão (socket):
/// `$XDG_RUNTIME_DIR/<socket>-notificacoes`.
pub fn history_path() -> Option<std::path::PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")?;
    let display = std::env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| "mide-0".into());
    Some(std::path::PathBuf::from(runtime).join(format!("{display}-notificacoes")))
}

/// Uma linha do arquivo: campos separados por tabulação, com `\`, tabulação e
/// quebra de linha escapados.
pub fn encode(entry: &Entry) -> String {
    let esc = |s: &str| s.replace('\\', "\\\\").replace('\t', "\\t").replace('\n', "\\n");
    format!("{}\t{}\t{}\t{}", entry.time, esc(&entry.app), esc(&entry.summary), esc(&entry.body))
}

pub fn decode(line: &str) -> Option<Entry> {
    let unesc = |s: &str| {
        let mut out = String::new();
        let mut chars = s.chars();
        while let Some(c) = chars.next() {
            if c == '\\' {
                match chars.next() {
                    Some('t') => out.push('\t'),
                    Some('n') => out.push('\n'),
                    Some(other) => out.push(other),
                    None => {}
                }
            } else {
                out.push(c);
            }
        }
        out
    };
    let mut parts = line.split('\t');
    let time = parts.next()?.parse().ok()?;
    Some(Entry { time, app: unesc(parts.next()?), summary: unesc(parts.next()?), body: unesc(parts.next().unwrap_or("")) })
}

/// Histórico, da mais nova para a mais antiga.
pub fn read_history() -> Vec<Entry> {
    let Some(path) = history_path() else {
        return Vec::new();
    };
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let mut list: Vec<Entry> = text.lines().filter_map(decode).collect();
    list.reverse();
    list
}

/// Junta uma notificação ao histórico (guarda só as `HISTORY_MAX` mais novas).
pub fn add_to_history(entry: &Entry) {
    let Some(path) = history_path() else {
        return;
    };
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    lines.push(encode(entry));
    let start = lines.len().saturating_sub(HISTORY_MAX);
    let _ = std::fs::write(&path, lines[start..].join("\n") + "\n");
}

/// Apaga o histórico (botão "Limpar" da central de notificações).
pub fn clear_history() {
    if let Some(path) = history_path() {
        let _ = std::fs::remove_file(path);
    }
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

    #[test]
    fn historico_guarda_e_le_de_volta() {
        let entry = Entry { time: 1_700_000_000, app: "Correio".into(), summary: "Nova\tmensagem".into(), body: "linha 1\nlinha 2 \\ fim".into() };
        assert_eq!(decode(&encode(&entry)), Some(entry));
        assert_eq!(decode("lixo"), None);
        assert_eq!(decode("12\tApp\tTítulo").unwrap().body, "");
    }
}

/// Manda uma notificação ao servidor de notificações da sessão (o do mide, ou outro),
/// numa thread: quem chama não espera o barramento.
pub fn send(app: &str, summary: &str, body: &str) {
    let (app, summary, body) = (app.to_string(), summary.to_string(), body.to_string());
    std::thread::spawn(move || {
        let Ok(conn) = zbus::blocking::Connection::session() else {
            return;
        };
        let hints: std::collections::HashMap<&str, zbus::zvariant::Value> = std::collections::HashMap::new();
        let result = conn.call_method(
            Some("org.freedesktop.Notifications"),
            "/org/freedesktop/Notifications",
            Some("org.freedesktop.Notifications"),
            "Notify",
            &(app.as_str(), 0u32, "", summary.as_str(), body.as_str(), Vec::<&str>::new(), hints, -1i32),
        );
        if let Err(err) = result {
            tracing::debug!("notificação não enviada: {err}");
        }
    });
}

