//! Decodificador dos bytes que um programa escreve no terminal: texto em UTF-8,
//! caracteres de controle e sequências de escape (ESC, CSI, OSC, DCS).
//!
//! Segue a máquina de estados dos terminais DEC VT500 (a mesma que os terminais
//! modernos seguem), sem interpretar nada: cada sequência completa vira uma chamada
//! de [`Perform`], e quem decide o que ela faz é a tela ([`super::screen`]).
//!
//! DCS (imagens Sixel, o modo de controle do tmux) chega aos poucos: `dcs_start`, os
//! bytes do conteúdo em `dcs_put` e `dcs_end` no terminador; o conteúdo pode ser
//! enorme (uma sessão inteira do tmux). APC (imagens do kitty) chega inteiro.

/// Limites para sequências malformadas ou maliciosas não crescerem sem fim.
const MAX_PARAMS: usize = 32;
const MAX_INTERMEDIATES: usize = 2;
/// OSC pode trazer uma imagem inteira (protocolo de imagens do iTerm2): até 48 MB.
const MAX_OSC: usize = 48 << 20;

/// O que fazer com cada parte decodificada.
pub trait Perform {
    /// Um caractere de texto.
    fn print(&mut self, c: char);
    /// Um caractere de controle (BEL, BS, HT, LF, CR...).
    fn execute(&mut self, byte: u8);
    /// `CSI` completa: parâmetros (cada um com os subparâmetros separados por `:`),
    /// o marcador privado (`?`, `>`, `<`, `=`), os intermediários e o byte final.
    fn csi(&mut self, params: &[Vec<u16>], private: Option<u8>, intermediates: &[u8], action: u8);
    /// `ESC` seguido de intermediários e do byte final.
    fn esc(&mut self, intermediates: &[u8], byte: u8);
    /// `OSC` completa (o texto entre `ESC ]` e o terminador).
    fn osc(&mut self, data: &[u8]);
    /// Começo de uma `DCS`: parâmetros, marcador, intermediários e o byte final.
    fn dcs_start(&mut self, _params: &[Vec<u16>], _private: Option<u8>, _intermediates: &[u8], _action: u8) {}
    /// Bytes do conteúdo da `DCS` (em pedaços).
    fn dcs_put(&mut self, _data: &[u8]) {}
    /// Fim da `DCS` (`ST`, ou cancelada).
    fn dcs_end(&mut self) {}
    /// `APC` completa (`ESC _ ... ST`).
    fn apc(&mut self, _data: &[u8]) {}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum State {
    #[default]
    Ground,
    Escape,
    EscapeIntermediate,
    CsiEntry,
    CsiParam,
    CsiIntermediate,
    CsiIgnore,
    OscString,
    /// Cabeçalho da DCS (parâmetros como na CSI) e o conteúdo dela.
    DcsEntry,
    DcsPassthrough,
    ApcString,
    /// SOS e PM: o conteúdo é ignorado até o terminador.
    IgnoredString,
}

#[derive(Debug, Default)]
pub struct Parser {
    state: State,
    params: Vec<Vec<u16>>,
    /// Parâmetro em construção (há dígitos ou separadores desde o último).
    param_started: bool,
    private: Option<u8>,
    intermediates: Vec<u8>,
    osc: Vec<u8>,
    /// Conteúdo da DCS ainda não entregue (vai em pedaços para `dcs_put`).
    dcs: Vec<u8>,
    /// UTF-8: bytes já lidos do caractere atual e quantos faltam.
    utf8: [u8; 4],
    utf8_len: usize,
    utf8_need: usize,
}

impl Parser {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn advance(&mut self, perform: &mut impl Perform, bytes: &[u8]) {
        for &byte in bytes {
            self.byte(perform, byte);
        }
        self.flush(perform);
    }

    fn byte(&mut self, perform: &mut impl Perform, byte: u8) {
        // Continuação de um caractere UTF-8 (só no texto comum).
        if self.utf8_need > 0 {
            if byte & 0xC0 == 0x80 {
                self.utf8[self.utf8_len] = byte;
                self.utf8_len += 1;
                self.utf8_need -= 1;
                if self.utf8_need == 0 {
                    let c = std::str::from_utf8(&self.utf8[..self.utf8_len]).ok().and_then(|s| s.chars().next());
                    perform.print(c.unwrap_or(char::REPLACEMENT_CHARACTER));
                    self.utf8_len = 0;
                }
                return;
            }
            // Sequência interrompida: o caractere incompleto vira �, e o byte segue.
            self.utf8_need = 0;
            self.utf8_len = 0;
            perform.print(char::REPLACEMENT_CHARACTER);
        }

        // Valem em qualquer estado: cancelar e começar outra sequência.
        match byte {
            0x18 | 0x1A => {
                if self.state == State::DcsPassthrough {
                    self.flush_dcs(perform);
                    perform.dcs_end();
                }
                self.state = State::Ground;
                perform.execute(byte);
                return;
            }
            0x1B => {
                match self.state {
                    State::OscString => self.dispatch_osc(perform),
                    State::DcsPassthrough => {
                        self.flush_dcs(perform);
                        perform.dcs_end();
                    }
                    State::ApcString => {
                        perform.apc(&self.osc);
                        self.osc.clear();
                    }
                    _ => {}
                }
                self.enter_escape();
                return;
            }
            _ => {}
        }

        match self.state {
            State::Ground => match byte {
                0x00..=0x1F => perform.execute(byte),
                0x20..=0x7E => perform.print(byte as char),
                0x7F => {}
                _ => self.start_utf8(perform, byte),
            },
            State::Escape => match byte {
                0x00..=0x1F => perform.execute(byte),
                0x20..=0x2F => {
                    self.collect(byte);
                    self.state = State::EscapeIntermediate;
                }
                b'[' => self.enter(State::CsiEntry),
                b']' => {
                    self.osc.clear();
                    self.state = State::OscString;
                }
                b'P' => self.enter(State::DcsEntry),
                b'_' => {
                    self.osc.clear();
                    self.state = State::ApcString;
                }
                b'X' | b'^' => self.state = State::IgnoredString,
                0x30..=0x7E => {
                    perform.esc(&self.intermediates, byte);
                    self.state = State::Ground;
                }
                _ => {}
            },
            State::EscapeIntermediate => match byte {
                0x00..=0x1F => perform.execute(byte),
                0x20..=0x2F => self.collect(byte),
                0x30..=0x7E => {
                    perform.esc(&self.intermediates, byte);
                    self.state = State::Ground;
                }
                _ => {}
            },
            State::CsiEntry | State::CsiParam => match byte {
                0x00..=0x1F => perform.execute(byte),
                b'0'..=b'9' => {
                    self.digit(byte);
                    self.state = State::CsiParam;
                }
                b';' => {
                    self.next_param();
                    self.state = State::CsiParam;
                }
                b':' => {
                    self.next_subparam();
                    self.state = State::CsiParam;
                }
                0x3C..=0x3F if self.state == State::CsiEntry => {
                    self.private = Some(byte);
                    self.state = State::CsiParam;
                }
                0x3C..=0x3F => self.state = State::CsiIgnore,
                0x20..=0x2F => {
                    self.collect(byte);
                    self.state = State::CsiIntermediate;
                }
                0x40..=0x7E => self.dispatch_csi(perform, byte),
                _ => {}
            },
            State::CsiIntermediate => match byte {
                0x00..=0x1F => perform.execute(byte),
                0x20..=0x2F => self.collect(byte),
                0x30..=0x3F => self.state = State::CsiIgnore,
                0x40..=0x7E => self.dispatch_csi(perform, byte),
                _ => {}
            },
            State::CsiIgnore => match byte {
                0x00..=0x1F => perform.execute(byte),
                0x40..=0x7E => self.state = State::Ground,
                _ => {}
            },
            State::OscString => match byte {
                // BEL também termina (o xterm aceita, e quase todo programa usa).
                0x07 => {
                    self.dispatch_osc(perform);
                    self.state = State::Ground;
                }
                0x00..=0x1F => {}
                _ => {
                    if self.osc.len() < MAX_OSC {
                        self.osc.push(byte);
                    }
                }
            },
            State::DcsEntry => match byte {
                b'0'..=b'9' => self.digit(byte),
                b';' => self.next_param(),
                b':' => self.next_subparam(),
                0x3C..=0x3F if self.params.is_empty() && self.private.is_none() => self.private = Some(byte),
                0x20..=0x2F => self.collect(byte),
                0x40..=0x7E => {
                    perform.dcs_start(&self.params, self.private, &self.intermediates, byte);
                    self.dcs.clear();
                    self.state = State::DcsPassthrough;
                }
                _ => {}
            },
            State::DcsPassthrough => {
                self.dcs.push(byte);
                if self.dcs.len() >= 4096 {
                    self.flush_dcs(perform);
                }
            }
            State::ApcString => {
                if self.osc.len() < MAX_OSC {
                    self.osc.push(byte);
                }
            }
            State::IgnoredString => {}
        }
    }

    /// Entrega o conteúdo da DCS acumulado até agora.
    fn flush_dcs(&mut self, perform: &mut impl Perform) {
        if !self.dcs.is_empty() {
            perform.dcs_put(&self.dcs);
            self.dcs.clear();
        }
    }

    /// Fim de um pedaço de bytes: o que a DCS juntou já vai para quem usa (o tmux
    /// precisa da saída sem esperar o fim da sessão).
    pub fn flush(&mut self, perform: &mut impl Perform) {
        if self.state == State::DcsPassthrough {
            self.flush_dcs(perform);
        }
    }

    fn start_utf8(&mut self, perform: &mut impl Perform, byte: u8) {
        let need = match byte {
            0xC2..=0xDF => 1,
            0xE0..=0xEF => 2,
            0xF0..=0xF4 => 3,
            _ => {
                perform.print(char::REPLACEMENT_CHARACTER);
                return;
            }
        };
        self.utf8[0] = byte;
        self.utf8_len = 1;
        self.utf8_need = need;
    }

    fn enter_escape(&mut self) {
        self.intermediates.clear();
        self.state = State::Escape;
    }

    fn enter(&mut self, state: State) {
        self.params.clear();
        self.param_started = false;
        self.private = None;
        self.intermediates.clear();
        self.state = state;
    }

    fn collect(&mut self, byte: u8) {
        if self.intermediates.len() < MAX_INTERMEDIATES {
            self.intermediates.push(byte);
        }
    }

    fn digit(&mut self, byte: u8) {
        if !self.param_started {
            if self.params.len() >= MAX_PARAMS {
                return;
            }
            self.params.push(vec![0]);
            self.param_started = true;
        }
        if let Some(value) = self.params.last_mut().and_then(|p| p.last_mut()) {
            *value = value.saturating_mul(10).saturating_add((byte - b'0') as u16);
        }
    }

    /// `;`: fecha o parâmetro atual (vazio vale 0) e começa outro.
    fn next_param(&mut self) {
        if !self.param_started && self.params.len() < MAX_PARAMS {
            self.params.push(vec![0]);
        }
        self.param_started = false;
    }

    /// `:`: outro subparâmetro do mesmo parâmetro (`38:2::255:0:0`).
    fn next_subparam(&mut self) {
        if !self.param_started {
            if self.params.len() >= MAX_PARAMS {
                return;
            }
            self.params.push(vec![0]);
            self.param_started = true;
        }
        if let Some(param) = self.params.last_mut() {
            if param.len() < 8 {
                param.push(0);
            }
        }
    }

    fn dispatch_csi(&mut self, perform: &mut impl Perform, action: u8) {
        perform.csi(&self.params, self.private, &self.intermediates, action);
        self.state = State::Ground;
    }

    fn dispatch_osc(&mut self, perform: &mut impl Perform) {
        perform.osc(&self.osc);
        self.osc.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Log(Vec<String>);

    impl Perform for Log {
        fn print(&mut self, c: char) {
            self.0.push(format!("p{c}"));
        }
        fn execute(&mut self, byte: u8) {
            self.0.push(format!("x{byte}"));
        }
        fn csi(&mut self, params: &[Vec<u16>], private: Option<u8>, intermediates: &[u8], action: u8) {
            let private = private.map(|p| (p as char).to_string()).unwrap_or_default();
            let inter = String::from_utf8_lossy(intermediates);
            self.0.push(format!("csi{private}{params:?}{inter}{}", action as char));
        }
        fn esc(&mut self, intermediates: &[u8], byte: u8) {
            self.0.push(format!("esc{}{}", String::from_utf8_lossy(intermediates), byte as char));
        }
        fn osc(&mut self, data: &[u8]) {
            self.0.push(format!("osc{}", String::from_utf8_lossy(data)));
        }
        fn dcs_start(&mut self, params: &[Vec<u16>], _private: Option<u8>, _intermediates: &[u8], action: u8) {
            self.0.push(format!("dcs{params:?}{}", action as char));
        }
        fn dcs_put(&mut self, data: &[u8]) {
            self.0.push(format!("put{}", String::from_utf8_lossy(data)));
        }
        fn dcs_end(&mut self) {
            self.0.push("fim".into());
        }
        fn apc(&mut self, data: &[u8]) {
            self.0.push(format!("apc{}", String::from_utf8_lossy(data)));
        }
    }

    fn parse(bytes: &[u8]) -> Vec<String> {
        let mut log = Log::default();
        Parser::new().advance(&mut log, bytes);
        log.0
    }

    #[test]
    fn texto_utf8_e_controles() {
        assert_eq!(parse("aç\r\n".as_bytes()), ["pa", "pç", "x13", "x10"]);
        assert_eq!(parse("€😀".as_bytes()), ["p€", "p😀"]);
        // Byte inválido e sequência interrompida viram �.
        assert_eq!(parse(b"\xffa"), ["p\u{fffd}", "pa"]);
        assert_eq!(parse(b"\xe2\x82a"), ["p\u{fffd}", "pa"]);
    }

    #[test]
    fn csi_com_parametros_vazios_privados_e_subparametros() {
        assert_eq!(parse(b"\x1b[H"), ["csi[]H"]);
        assert_eq!(parse(b"\x1b[1;2H"), ["csi[[1], [2]]H"]);
        assert_eq!(parse(b"\x1b[;5H"), ["csi[[0], [5]]H"]);
        assert_eq!(parse(b"\x1b[?1049h"), ["csi?[[1049]]h"]);
        assert_eq!(parse(b"\x1b[38:2::255:0:0m"), ["csi[[38, 2, 0, 255, 0, 0]]m"]);
        assert_eq!(parse(b"\x1b[2 q"), ["csi[[2]] q"]);
        assert_eq!(parse(b"\x1b[!p"), ["csi[]!p"]);
    }

    #[test]
    fn controle_no_meio_de_csi_e_cancelamento() {
        assert_eq!(parse(b"\x1b[1\n;2H"), ["x10", "csi[[1], [2]]H"]);
        assert_eq!(parse(b"\x1b[12\x18a"), ["x24", "pa"]);
        // Marcador privado fora do lugar: a sequência é ignorada.
        assert_eq!(parse(b"\x1b[1?2hx"), ["px"]);
    }

    #[test]
    fn osc_com_bel_e_com_st() {
        assert_eq!(parse(b"\x1b]0;t\xc3\xadtulo\x07a"), ["osc0;título", "pa"]);
        assert_eq!(parse(b"\x1b]2;x\x1b\\b"), ["osc2;x", "esc\\", "pb"]);
    }

    #[test]
    fn esc_e_strings_ignoradas() {
        assert_eq!(parse(b"\x1b7\x1b(0\x1b#8"), ["esc7", "esc(0", "esc#8"]);
        assert_eq!(parse(b"\x1bXnada\x1b\\a"), ["esc\\", "pa"]);
    }

    #[test]
    fn dcs_em_pedacos_e_apc() {
        assert_eq!(parse(b"\x1bP0;1q#0;2;0;0;0\x1b\\a"), ["dcs[[0], [1]]q", "put#0;2;0;0;0", "fim", "esc\\", "pa"]);
        // Cada chamada a `advance` entrega o que a DCS juntou (o tmux não espera o fim).
        let mut log = Log::default();
        let mut p = Parser::new();
        p.advance(&mut log, b"\x1bP1000p%begin");
        p.advance(&mut log, b" 1\n");
        assert_eq!(log.0, ["dcs[[1000]]p", "put%begin", "put 1\n"]);
        assert_eq!(parse(b"\x1b_Ga=T;QUJD\x1b\\"), ["apcGa=T;QUJD", "esc\\"]);
    }
}
