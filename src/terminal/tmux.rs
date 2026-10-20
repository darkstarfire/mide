//! Integração com o tmux (ideia do iTerm2): com `tmux -CC`, o tmux conversa com o
//! terminal por linhas de texto (o "modo de controle") em vez de desenhar a tela, e
//! cada janela do tmux vira uma aba e cada painel, um painel do mide-terminal.
//!
//! Aqui fica só o protocolo: separar as linhas, casar as respostas com os comandos,
//! ler os avisos (`%output`, `%window-add`, `%layout-change`...), decodificar a saída
//! e o layout, e montar os comandos. Quem cria abas e painéis é a janela.

use std::collections::VecDeque;

use super::layout::{Layout, PaneId, Split};

/// Um aviso do tmux (fora de uma resposta).
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// Saída de um painel (`%output %N ...`), já decodificada.
    Output { pane: u32, data: Vec<u8> },
    WindowAdd(u32),
    WindowClose(u32),
    WindowRenamed { window: u32, name: String },
    LayoutChange { window: u32, layout: String },
    /// Janela ou painel ativo mudou.
    WindowActive { window: u32 },
    PaneActive { window: u32, pane: u32 },
    /// Sessões criadas ou fechadas, ou outra sessão em uso: vale reler as janelas.
    SessionsChanged,
    /// O tmux saiu do modo de controle (detach, sessão fechada).
    Exit(String),
}

/// Resposta a um comando enviado: deu certo e as linhas.
#[derive(Debug, Clone, PartialEq)]
pub struct Reply {
    pub ok: bool,
    pub lines: Vec<String>,
}

/// O que veio do tmux, na ordem.
#[derive(Debug, Clone, PartialEq)]
pub enum Message<T> {
    Event(Event),
    /// Resposta ao comando que esperava por ela (o `T` dado em `send`).
    Reply(T, Reply),
}

/// Lado do mide da conversa com o tmux: junta as linhas e casa as respostas com os
/// comandos, na ordem em que foram mandados.
#[derive(Debug)]
pub struct Client<T> {
    buffer: Vec<u8>,
    /// Comandos mandados esperando resposta.
    waiting: VecDeque<T>,
    /// Resposta em andamento (`%begin` ... `%end`).
    block: Option<Vec<String>>,
}

impl<T> Default for Client<T> {
    fn default() -> Self {
        Self { buffer: Vec::new(), waiting: VecDeque::new(), block: None }
    }
}

impl<T> Client<T> {
    /// Linha de comando para mandar ao tmux; a resposta volta com `tag`.
    pub fn send(&mut self, command: &str, tag: T) -> Vec<u8> {
        self.waiting.push_back(tag);
        format!("{command}\n").into_bytes()
    }

    /// Bytes do tmux: devolve as mensagens das linhas completas.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<Message<T>> {
        self.buffer.extend_from_slice(bytes);
        let mut out = Vec::new();
        while let Some(end) = self.buffer.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.buffer.drain(..=end).collect();
            let line = String::from_utf8_lossy(&line[..line.len() - 1]).trim_end_matches('\r').to_string();
            self.line(&line, &mut out);
        }
        out
    }

    fn line(&mut self, line: &str, out: &mut Vec<Message<T>>) {
        if let Some(block) = &mut self.block {
            if line.starts_with("%end ") || line.starts_with("%error ") {
                let lines = std::mem::take(block);
                self.block = None;
                // A primeira resposta (do próprio `tmux -CC`) não tem comando nosso.
                if let Some(tag) = self.waiting.pop_front() {
                    out.push(Message::Reply(tag, Reply { ok: line.starts_with("%end "), lines }));
                }
            } else {
                block.push(line.to_string());
            }
            return;
        }
        if line.starts_with("%begin ") {
            self.block = Some(Vec::new());
            return;
        }
        if let Some(event) = parse_event(line) {
            out.push(Message::Event(event));
        }
    }
}

/// Número depois do prefixo (`@12` → 12, `%3` → 3, `$1` → 1).
fn id(word: &str, prefix: char) -> Option<u32> {
    word.strip_prefix(prefix)?.parse().ok()
}

/// Um aviso (`%...`) fora de resposta.
pub fn parse_event(line: &str) -> Option<Event> {
    let (name, rest) = line.split_once(' ').unwrap_or((line, ""));
    let mut words = rest.splitn(2, ' ');
    let first = words.next().unwrap_or("");
    let second = words.next().unwrap_or("");
    Some(match name {
        "%output" => Event::Output { pane: id(first, '%')?, data: unescape(second) },
        "%window-add" => Event::WindowAdd(id(first, '@')?),
        "%window-close" | "%unlinked-window-close" => Event::WindowClose(id(first, '@')?),
        "%window-renamed" => Event::WindowRenamed { window: id(first, '@')?, name: second.to_string() },
        "%layout-change" => Event::LayoutChange { window: id(first, '@')?, layout: second.split(' ').next().unwrap_or("").to_string() },
        "%session-window-changed" => Event::WindowActive { window: id(second, '@')? },
        "%window-pane-changed" => Event::PaneActive { window: id(first, '@')?, pane: id(second, '%')? },
        "%sessions-changed" | "%session-changed" => Event::SessionsChanged,
        "%exit" => Event::Exit(rest.to_string()),
        _ => return None,
    })
}

/// Saída do `%output`: caracteres de controle e `\` vêm como `\ooo` (octal).
pub fn unescape(text: &str) -> Vec<u8> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 3 < bytes.len() && bytes[i + 1..i + 4].iter().all(|b| (b'0'..=b'7').contains(b)) {
            let v = (bytes[i + 1] - b'0') as u32 * 64 + (bytes[i + 2] - b'0') as u32 * 8 + (bytes[i + 3] - b'0') as u32;
            out.push(v as u8);
            i += 4;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    out
}

/// Layout de uma janela do tmux (`b25d,158x40,0,0{79x40,0,0,1,78x40,80,0,2}`).
#[derive(Debug, Clone, PartialEq)]
pub enum TmuxLayout {
    Pane { id: u32, width: u32, height: u32 },
    /// `{...}` lado a lado ou `[...]` um em cima do outro, com o tamanho de cada parte.
    Split { side: bool, width: u32, height: u32, children: Vec<TmuxLayout> },
}

impl TmuxLayout {
    fn size(&self) -> (u32, u32) {
        match self {
            TmuxLayout::Pane { width, height, .. } | TmuxLayout::Split { width, height, .. } => (*width, *height),
        }
    }

    /// Painéis na ordem.
    pub fn panes(&self) -> Vec<u32> {
        match self {
            TmuxLayout::Pane { id, .. } => vec![*id],
            TmuxLayout::Split { children, .. } => children.iter().flat_map(|c| c.panes()).collect(),
        }
    }

    /// A árvore de painéis do mide (divisões de dois em dois, com as proporções do
    /// tmux), com o painel do mide de cada painel do tmux (`local`).
    pub fn to_layout(&self, local: &impl Fn(u32) -> Option<PaneId>) -> Option<Layout> {
        match self {
            TmuxLayout::Pane { id, .. } => local(*id).map(Layout::Pane),
            TmuxLayout::Split { side, children, .. } => {
                let dir = if *side { Split::Side } else { Split::Stack };
                let size = |c: &TmuxLayout| if *side { c.size().0 } else { c.size().1 } as f32;
                // De trás para frente: [a, b, c] vira a | (b | c).
                let mut iter = children.iter().rev();
                let last = iter.next()?;
                let mut acc = last.to_layout(local);
                let mut acc_size = size(last);
                for child in iter {
                    let this = child.to_layout(local);
                    let child_size = size(child);
                    acc = match (this, acc) {
                        (Some(a), Some(b)) => {
                            let ratio = (child_size / (child_size + acc_size).max(1.0)).clamp(0.1, 0.9);
                            Some(Layout::Split { dir, ratio, a: Box::new(a), b: Box::new(b) })
                        }
                        (one, None) | (None, one) => one,
                    };
                    acc_size += child_size + 1.0;
                }
                acc
            }
        }
    }
}

/// Lê o layout do tmux (com ou sem a soma de verificação na frente).
pub fn parse_layout(text: &str) -> Option<TmuxLayout> {
    let text = text.trim();
    // A soma de verificação: 4 algarismos hexadecimais e uma vírgula.
    let body = match text.split_once(',') {
        Some((sum, rest)) if sum.len() == 4 && sum.chars().all(|c| c.is_ascii_hexdigit()) && !rest.is_empty() => rest,
        _ => text,
    };
    let bytes = body.as_bytes();
    let mut i = 0;
    let layout = cell(bytes, &mut i)?;
    (i == bytes.len()).then_some(layout)
}

fn number(b: &[u8], i: &mut usize) -> Option<u32> {
    let start = *i;
    while *i < b.len() && b[*i].is_ascii_digit() {
        *i += 1;
    }
    std::str::from_utf8(&b[start..*i]).ok()?.parse().ok()
}

fn expect(b: &[u8], i: &mut usize, c: u8) -> Option<()> {
    (b.get(*i) == Some(&c)).then(|| *i += 1)
}

/// `LxA,x,y` seguido de `,id`, `{...}` ou `[...]`.
fn cell(b: &[u8], i: &mut usize) -> Option<TmuxLayout> {
    let width = number(b, i)?;
    expect(b, i, b'x')?;
    let height = number(b, i)?;
    expect(b, i, b',')?;
    number(b, i)?;
    expect(b, i, b',')?;
    number(b, i)?;
    match b.get(*i) {
        Some(&open @ (b'{' | b'[')) => {
            *i += 1;
            let close = if open == b'{' { b'}' } else { b']' };
            let mut children = vec![cell(b, i)?];
            while b.get(*i) == Some(&b',') {
                *i += 1;
                children.push(cell(b, i)?);
            }
            expect(b, i, close)?;
            Some(TmuxLayout::Split { side: open == b'{', width, height, children })
        }
        Some(b',') => {
            *i += 1;
            Some(TmuxLayout::Pane { id: number(b, i)?, width, height })
        }
        _ => None,
    }
}

/// Comando que manda bytes (o que o usuário digitou ou colou) a um painel.
pub fn send_keys(pane: u32, bytes: &[u8]) -> Vec<String> {
    // Em pedaços, para a linha de comando não ficar enorme.
    bytes
        .chunks(256)
        .map(|chunk| {
            let hex: Vec<String> = chunk.iter().map(|b| format!("{b:02x}")).collect();
            format!("send-keys -t %{pane} -H {}", hex.join(" "))
        })
        .collect()
}

/// Comando que lista as janelas da sessão: número, layout, ativa e nome.
pub const LIST_WINDOWS: &str = "list-windows -F \"#{window_id} #{window_layout} #{window_active} #{window_name}\"";

/// Uma linha da resposta do `LIST_WINDOWS`.
pub fn parse_window_line(line: &str) -> Option<(u32, TmuxLayout, bool, String)> {
    let mut parts = line.splitn(4, ' ');
    let window = id(parts.next()?, '@')?;
    let layout = parse_layout(parts.next()?)?;
    let active = parts.next()? == "1";
    let name = parts.next().unwrap_or("").to_string();
    Some((window, layout, active, name))
}

/// Histórico e tela de um painel (com cores), para mostrar o que já havia nele.
pub fn capture(pane: u32) -> String {
    format!("capture-pane -p -e -t %{pane} -S -2000")
}

/// Posição do cursor de um painel.
pub fn cursor(pane: u32) -> String {
    format!("display-message -p -t %{pane} \"#{{cursor_x}} #{{cursor_y}}\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saida_com_escapes_octais() {
        assert_eq!(unescape("ol\\303\\241\\015\\012\\033[1m\\134x"), "olá\r\n\x1b[1m\\x".as_bytes());
        assert_eq!(unescape("fim\\01"), b"fim\\01");
    }

    #[test]
    fn avisos() {
        assert_eq!(parse_event("%output %3 ls\\015\\012"), Some(Event::Output { pane: 3, data: b"ls\r\n".to_vec() }));
        assert_eq!(parse_event("%window-add @4"), Some(Event::WindowAdd(4)));
        assert_eq!(parse_event("%window-close @4"), Some(Event::WindowClose(4)));
        assert_eq!(parse_event("%window-renamed @2 meu nome"), Some(Event::WindowRenamed { window: 2, name: "meu nome".into() }));
        assert_eq!(parse_event("%layout-change @1 b25d,80x24,0,0,1 b25d,80x24,0,0,1 *"), Some(Event::LayoutChange { window: 1, layout: "b25d,80x24,0,0,1".into() }));
        assert_eq!(parse_event("%window-pane-changed @1 %5"), Some(Event::PaneActive { window: 1, pane: 5 }));
        assert_eq!(parse_event("%exit"), Some(Event::Exit(String::new())));
        assert_eq!(parse_event("%pause %1"), None);
    }

    #[test]
    fn respostas_casadas_com_os_comandos_e_avisos_no_meio() {
        let mut c: Client<&str> = Client::default();
        // A primeira resposta (do próprio tmux -CC) não tem comando nosso.
        let m = c.feed(b"%begin 1 0 0\n%end 1 0 0\n");
        assert!(m.is_empty());
        assert_eq!(c.send("list-windows", "janelas"), b"list-windows\n");
        c.send("kill-pane -t %9", "fechar");
        let m = c.feed(b"%begin 2 1 1\n@1 b25d,80x24,0,0,1 1 bash\n%end 2 1 1\n%output %1 a\n%begin 3 2 1\nnao existe\n%err");
        assert_eq!(m.len(), 2);
        assert_eq!(m[0], Message::Reply("janelas", Reply { ok: true, lines: vec!["@1 b25d,80x24,0,0,1 1 bash".into()] }));
        assert_eq!(m[1], Message::Event(Event::Output { pane: 1, data: b"a".to_vec() }));
        let m = c.feed(b"or 3 2 1\n");
        assert_eq!(m, vec![Message::Reply("fechar", Reply { ok: false, lines: vec!["nao existe".into()] })]);
    }

    #[test]
    fn layout_do_tmux_vira_paineis() {
        // [ 1 | ( 2 em cima do 3 ) ]
        let l = parse_layout("b25d,159x40,0,0{79x40,0,0,1,79x40,80,0[79x20,80,0,2,79x19,80,21,3]}").unwrap();
        assert_eq!(l.panes(), vec![1, 2, 3]);
        let local = |id: u32| Some(id + 100);
        let layout = l.to_layout(&local).unwrap();
        assert_eq!(layout.panes(), vec![101, 102, 103]);
        let Layout::Split { dir: Split::Side, ratio, b, .. } = &layout else { panic!("{layout:?}") };
        assert!((ratio - 0.5).abs() < 0.02);
        assert!(matches!(**b, Layout::Split { dir: Split::Stack, .. }));
        // Três lado a lado: proporções aproximadas de 1/3.
        let l = parse_layout("80x24,0,0{26x24,0,0,1,26x24,27,0,2,26x24,54,0,3}").unwrap();
        let Some(Layout::Split { ratio, .. }) = l.to_layout(&local) else { panic!() };
        assert!((ratio - 0.33).abs() < 0.03);
        assert!(parse_layout("lixo").is_none());
        assert_eq!(parse_window_line("@2 80x24,0,0,7 0 vim").map(|w| (w.0, w.2, w.3)), Some((2, false, "vim".into())));
    }

    #[test]
    fn enviar_teclas_em_hexadecimal() {
        assert_eq!(send_keys(3, b"ls\r"), vec!["send-keys -t %3 -H 6c 73 0d".to_string()]);
        assert_eq!(send_keys(1, &[b'a'; 300]).len(), 2);
    }
}
