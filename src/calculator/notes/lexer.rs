//! Léxico das notas: corta uma linha em números, palavras e símbolos, cada um com o
//! trecho em bytes no texto inteiro (para colorir e sublinhar). Aqui só se decide onde
//! o número termina; o valor sai do `format::parse`, a mesma regra da tela e do colar
//! ("1.500" é mil e quinhentos; "1,5" e "1.5" são um e meio).

use std::ops::Range;

use super::super::format;

/// Um pedaço da linha.
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: Kind,
    /// Trecho em bytes no texto da nota.
    pub range: Range<usize>,
    /// Havia espaço antes ("2 e 3" é a conjunção; "2e" é 2 × e).
    pub space_before: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    /// 1.234,5 · 1.5 · 2,5e3 · 1/2 (fração colada).
    Num(f64),
    /// Variável, função, constante, unidade, palavra da conta ou prosa, como escrita.
    Name(String),
    Plus,
    /// "-" e "−".
    Minus,
    /// "×", "*" e "·".
    Times,
    /// "÷" e "/".
    Divide,
    /// "^" e "**".
    Pow,
    /// Expoente sobrescrito: "²", "³", "⁻¹".
    Sup(i32),
    Open,
    Close,
    Semi,
    Equals,
    Percent,
    Bang,
    /// "°" e "º" (o ordinal do teclado ABNT2).
    Degree,
    /// "√" (2), "∛" e "³√" (3).
    Root(u8),
    /// "→" e "->".
    Arrow,
    /// "R$", "US$", "$", "€", "£" e "¥", antes do valor.
    Money(&'static str),
    /// Pontuação e o resto: prosa.
    Other(char),
}

/// Os pedaços de uma linha; `offset` é o byte onde ela começa no texto da nota.
pub fn lex(line: &str, offset: usize) -> Vec<Token> {
    let mut lexer = Lexer { line, chars: line.char_indices().collect(), offset, i: 0, space: false, out: Vec::new() };
    lexer.run();
    lexer.out
}

struct Lexer<'a> {
    line: &'a str,
    chars: Vec<(usize, char)>,
    offset: usize,
    /// Próximo caractere (índice em `chars`).
    i: usize,
    /// Houve espaço depois do último token.
    space: bool,
    out: Vec<Token>,
}

impl<'a> Lexer<'a> {
    fn run(&mut self) {
        while let Some(c) = self.peek(0) {
            if c.is_whitespace() {
                self.space = true;
                self.i += 1;
                continue;
            }
            let start = self.i;
            let kind = self.next_kind(c);
            let range = self.offset + self.byte(start)..self.offset + self.byte(self.i);
            self.out.push(Token { kind, range, space_before: self.space });
            self.space = false;
        }
    }

    fn peek(&self, ahead: usize) -> Option<char> {
        self.chars.get(self.i + ahead).map(|&(_, c)| c)
    }

    fn digit_at(&self, ahead: usize) -> bool {
        self.peek(ahead).is_some_and(|c| c.is_ascii_digit())
    }

    /// Byte do caractere `i` da linha (o fim, depois do último).
    fn byte(&self, i: usize) -> usize {
        self.chars.get(i).map_or(self.line.len(), |&(b, _)| b)
    }

    /// Texto entre dois caracteres da linha.
    fn slice(&self, from: usize, to: usize) -> &'a str {
        self.line.get(self.byte(from)..self.byte(to)).unwrap_or("")
    }

    /// Lê um token a partir de `c` (o caractere atual) e diz o que ele é.
    fn next_kind(&mut self, c: char) -> Kind {
        if c.is_ascii_digit() {
            return self.number();
        }
        if c == 'π' {
            self.i += 1;
            return Kind::Name("π".into());
        }
        if c == '°' || c == 'º' {
            return self.degree();
        }
        if c == '³' && self.peek(1) == Some('√') {
            self.i += 2;
            return Kind::Root(3);
        }
        if superscript(c).is_some() || (matches!(c, '⁻' | '⁺') && self.peek(1).and_then(superscript).is_some()) {
            return self.sup();
        }
        if is_letter(c) {
            return self.word();
        }
        self.i += 1;
        self.symbol(c)
    }

    /// Um símbolo de um caractere (ou dois: "->" e "**"); `c` já foi consumido.
    fn symbol(&mut self, c: char) -> Kind {
        let next = self.peek(0);
        let mut pair = |kind: Kind| {
            self.i += 1;
            kind
        };
        match c {
            '-' if next == Some('>') => pair(Kind::Arrow),
            '*' if next == Some('*') => pair(Kind::Pow),
            '+' => Kind::Plus,
            '-' | '−' => Kind::Minus,
            '×' | '*' | '·' => Kind::Times,
            '÷' | '/' => Kind::Divide,
            '^' => Kind::Pow,
            '(' => Kind::Open,
            ')' => Kind::Close,
            ';' => Kind::Semi,
            '=' => Kind::Equals,
            '%' => Kind::Percent,
            '!' => Kind::Bang,
            '√' => Kind::Root(2),
            '∛' => Kind::Root(3),
            '→' => Kind::Arrow,
            '$' => Kind::Money("$"),
            '€' => Kind::Money("€"),
            '£' => Kind::Money("£"),
            '¥' => Kind::Money("¥"),
            other => Kind::Other(other),
        }
    }

    /// "°" ou "º"; colados em C ou F ("°C", "ºF"), viram o nome da escala.
    fn degree(&mut self) -> Kind {
        let scale = matches!(self.peek(1), Some('C' | 'F')) && !self.peek(2).is_some_and(char::is_alphanumeric);
        let start = self.i;
        if scale {
            self.i += 2;
            return Kind::Name(self.slice(start, self.i).to_string());
        }
        self.i += 1;
        Kind::Degree
    }

    /// Expoente sobrescrito, com sinal: "²", "¹⁰", "⁻¹".
    fn sup(&mut self) -> Kind {
        let negative = self.peek(0) == Some('⁻');
        if matches!(self.peek(0), Some('⁻' | '⁺')) {
            self.i += 1;
        }
        let mut n: i32 = 0;
        while let Some(d) = self.peek(0).and_then(superscript) {
            n = n.saturating_mul(10).saturating_add(d);
            self.i += 1;
        }
        Kind::Sup(if negative { -n } else { n })
    }

    /// Uma palavra. Colada depois de um número, só letras: "47x96" é 47, x, 96 e
    /// "1h30min" é 1, h, 30, min. "R$" e "US$" são moeda.
    fn word(&mut self) -> Kind {
        let after_number = !self.space && matches!(self.out.last(), Some(Token { kind: Kind::Num(_), .. }));
        let start = self.i;
        while let Some(c) = self.peek(0) {
            let more = is_letter(c) || (!after_number && (c.is_ascii_digit() || is_subscript(c)));
            if !more {
                break;
            }
            self.i += 1;
        }
        let word = self.slice(start, self.i);
        if self.peek(0) == Some('$') && matches!(word, "R" | "US") {
            self.i += 1;
            return Kind::Money(if word == "R" { "R$" } else { "US$" });
        }
        Kind::Name(word.to_string())
    }

    /// Um número: algarismos com ponto de milhar ou casas decimais, expoente colado
    /// ("2,5e3") ou fração colada ("1/2"). O texto vai normalizado para o
    /// `format::parse` (sem os pontos de milhar, com vírgula decimal).
    fn number(&mut self) -> Kind {
        let start = self.i;
        self.skip_digits();
        let mut text = self.slice(start, self.i).to_string();
        let groups = self.thousands(start);
        let mut plain = groups.is_none();
        text.push_str(&groups.unwrap_or_default());
        // A vírgula sempre é decimal; o ponto, só quando não há milhar ("1.234,5").
        let sep = self.peek(0);
        if (sep == Some(',') || (sep == Some('.') && plain)) && self.digit_at(1) {
            self.i += 1;
            let from = self.i;
            self.skip_digits();
            text.push(',');
            text.push_str(self.slice(from, self.i));
            plain = false;
        }
        if let Some(exp) = self.exponent() {
            text.push_str(&exp);
            plain = false;
        }
        if plain && let Some(v) = self.fraction(&text) {
            return Kind::Num(v);
        }
        // Só falha quando não cabe no f64 (1e400): a conta dá "Número grande demais".
        Kind::Num(format::parse(&text).unwrap_or(f64::INFINITY))
    }

    fn skip_digits(&mut self) {
        while self.digit_at(0) {
            self.i += 1;
        }
    }

    /// Grupos de milhar depois dos algarismos lidos desde `start` ("1.234",
    /// "12.345.678"): o primeiro grupo com 1 a 3 algarismos, sem zero à esquerda, e
    /// os outros com 3, sem algarismo logo depois ("1.2345" é decimal).
    fn thousands(&mut self, start: usize) -> Option<String> {
        let first = self.i - start;
        if !(1..=3).contains(&first) || self.chars.get(start).is_some_and(|&(_, c)| c == '0') {
            return None;
        }
        let mut groups = String::new();
        while self.peek(0) == Some('.') && (1..=3).all(|k| self.digit_at(k)) && !self.digit_at(4) {
            groups.push_str(self.slice(self.i + 1, self.i + 4));
            self.i += 4;
        }
        (!groups.is_empty()).then_some(groups)
    }

    /// Expoente colado ("e3", "E10", "e−3"), já como "e-3"; sem algarismo depois do
    /// "e", não é expoente ("2e" é 2 × e).
    fn exponent(&mut self) -> Option<String> {
        if !matches!(self.peek(0), Some('e' | 'E')) {
            return None;
        }
        let (sign, skip) = match self.peek(1) {
            Some('+') => ("", 2),
            Some('-' | '−') => ("-", 2),
            _ => ("", 1),
        };
        if !self.digit_at(skip) {
            return None;
        }
        self.i += skip;
        let from = self.i;
        self.skip_digits();
        Some(format!("e{sign}{}", self.slice(from, self.i)))
    }

    /// Fração colada depois do numerador já lido ("1/2"). Não é fração quando o
    /// denominador é zero (a conta mostra a divisão por zero) ou continua como número
    /// ("1/2,5" é 1 ÷ 2,5).
    fn fraction(&mut self, numerator: &str) -> Option<f64> {
        if self.peek(0) != Some('/') || !self.digit_at(1) {
            return None;
        }
        let save = self.i;
        self.i += 1;
        let from = self.i;
        self.skip_digits();
        let value = format::parse(numerator).zip(format::parse(self.slice(from, self.i))).map(|(n, d)| n / d);
        match value {
            Some(v) if v.is_finite() && !self.number_continues() => Some(v),
            _ => {
                self.i = save;
                None
            }
        }
    }

    /// Depois do denominador vem mais número: casas decimais ou expoente.
    fn number_continues(&self) -> bool {
        match self.peek(0) {
            Some(',' | '.') => self.digit_at(1),
            Some('e' | 'E') => self.digit_at(1) || (matches!(self.peek(1), Some('+' | '-' | '−')) && self.digit_at(2)),
            _ => false,
        }
    }
}

/// Letra de palavra: letras (com acento) e "_"; o π e o º ficam sozinhos.
fn is_letter(c: char) -> bool {
    (c.is_alphabetic() || c == '_') && c != 'π' && c != 'º'
}

/// Algarismo subscrito, para o "log₂".
fn is_subscript(c: char) -> bool {
    ('₀'..='₉').contains(&c)
}

fn superscript(c: char) -> Option<i32> {
    match c {
        '⁰' => Some(0),
        '¹' => Some(1),
        '²' => Some(2),
        '³' => Some(3),
        '⁴'..='⁹' => Some(c as i32 - '⁴' as i32 + 4),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(line: &str) -> Vec<Kind> {
        lex(line, 0).into_iter().map(|t| t.kind).collect()
    }

    fn name(s: &str) -> Kind {
        Kind::Name(s.into())
    }

    #[test]
    fn numeros_pt_br() {
        for (text, v) in [
            ("1.234", 1234.0),
            ("12.345.678", 12_345_678.0),
            ("1.234,56", 1234.56),
            ("1,5", 1.5),
            ("1.5", 1.5),
            ("0.125", 0.125),
            ("1,500", 1.5),
            ("1.2345", 1.2345),
            ("2,5e3", 2500.0),
            ("4E10", 4e10),
            ("1e−3", 0.001),
            ("1.5e3", 1500.0),
            ("1.500e3", 1.5e6),
            ("1/2", 0.5),
            ("3/4", 0.75),
        ] {
            assert_eq!(kinds(text), vec![Kind::Num(v)], "{text}");
        }
        // O ponto que sobra é pontuação.
        assert_eq!(kinds("25."), vec![Kind::Num(25.0), Kind::Other('.')]);
        assert_eq!(kinds("1.2.3"), vec![Kind::Num(1.2), Kind::Other('.'), Kind::Num(3.0)]);
        // Vírgula de milhar do inglês: a vírgula é decimal e o resto sobra (a conta
        // mostra o erro).
        assert_eq!(kinds("1,234.5"), vec![Kind::Num(1.234), Kind::Other('.'), Kind::Num(5.0)]);
        // Fração só colada, com denominador que não é zero e não continua.
        assert_eq!(kinds("1 / 2"), vec![Kind::Num(1.0), Kind::Divide, Kind::Num(2.0)]);
        assert_eq!(kinds("1/0"), vec![Kind::Num(1.0), Kind::Divide, Kind::Num(0.0)]);
        assert_eq!(kinds("1/2,5"), vec![Kind::Num(1.0), Kind::Divide, Kind::Num(2.5)]);
        assert_eq!(kinds("1/2/4"), vec![Kind::Num(0.5), Kind::Divide, Kind::Num(4.0)]);
        // Não cabe no f64: infinito (a conta diz "Número grande demais").
        assert_eq!(kinds("1e400"), vec![Kind::Num(f64::INFINITY)]);
    }

    #[test]
    fn palavras_coladas() {
        assert_eq!(kinds("47x96"), vec![Kind::Num(47.0), name("x"), Kind::Num(96.0)]);
        assert_eq!(kinds("1h30min"), vec![Kind::Num(1.0), name("h"), Kind::Num(30.0), name("min")]);
        assert_eq!(kinds("2x"), vec![Kind::Num(2.0), name("x")]);
        assert_eq!(kinds("2e"), vec![Kind::Num(2.0), name("e")]);
        assert_eq!(kinds("x2"), vec![name("x2")]);
        assert_eq!(kinds("πr²"), vec![name("π"), name("r"), Kind::Sup(2)]);
        assert_eq!(kinds("2πr"), vec![Kind::Num(2.0), name("π"), name("r")]);
        assert_eq!(kinds("preço_total2"), vec![name("preço_total2")]);
        assert_eq!(kinds("log₂ 8"), vec![name("log₂"), Kind::Num(8.0)]);
    }

    #[test]
    fn simbolos() {
        assert_eq!(kinds("× * ·"), vec![Kind::Times; 3]);
        assert_eq!(kinds("÷ /"), vec![Kind::Divide; 2]);
        assert_eq!(kinds("− -"), vec![Kind::Minus; 2]);
        assert_eq!(kinds("** ^"), vec![Kind::Pow; 2]);
        assert_eq!(kinds("-> →"), vec![Kind::Arrow; 2]);
        assert_eq!(kinds("√ ∛ ³√"), vec![Kind::Root(2), Kind::Root(3), Kind::Root(3)]);
        assert_eq!(kinds("x⁻¹ 10³"), vec![name("x"), Kind::Sup(-1), Kind::Num(10.0), Kind::Sup(3)]);
        assert_eq!(kinds("R$100"), vec![Kind::Money("R$"), Kind::Num(100.0)]);
        assert_eq!(kinds("US$ 5"), vec![Kind::Money("US$"), Kind::Num(5.0)]);
        assert_eq!(kinds("€2 £3 ¥4 $1"), [("€", 2.0), ("£", 3.0), ("¥", 4.0), ("$", 1.0)].iter().flat_map(|&(m, v)| [Kind::Money(m), Kind::Num(v)]).collect::<Vec<_>>());
        assert_eq!(kinds("25 °C"), vec![Kind::Num(25.0), name("°C")]);
        assert_eq!(kinds("30º"), vec![Kind::Num(30.0), Kind::Degree]);
        assert_eq!(kinds("30°"), vec![Kind::Num(30.0), Kind::Degree]);
        assert_eq!(kinds("( ) ; = % !"), vec![Kind::Open, Kind::Close, Kind::Semi, Kind::Equals, Kind::Percent, Kind::Bang]);
        assert_eq!(kinds("Oba :) 🙂"), vec![name("Oba"), Kind::Other(':'), Kind::Close, Kind::Other('🙂')]);
    }

    #[test]
    fn trechos_em_bytes() {
        let toks = lex("preço = 8", 0);
        assert_eq!(toks[0].kind, name("preço"));
        assert_eq!(toks[0].range, 0..6);
        assert_eq!(toks[1].range, 7..8);
        assert_eq!(toks[2].range, 9..10);
        assert!(toks[1].space_before && !lex("a=1", 0)[1].space_before);
        // Com o deslocamento da linha e caracteres de 2 bytes (×, −, π).
        let line = "2×3 − π";
        let toks = lex(line, 10);
        let ranges: Vec<_> = toks.iter().map(|t| t.range.clone()).collect();
        assert_eq!(ranges, vec![10..11, 11..13, 13..14, 15..18, 19..21]);
        for t in &toks {
            let r = t.range.start - 10..t.range.end - 10;
            assert!(line.get(r).is_some(), "fora de fronteira: {:?}", t.range);
        }
    }

    #[test]
    fn qualquer_texto_sem_estourar() {
        for line in ["", " ", "1.", "1,", "1e", "1e-", "1/", "²", "⁻", "°", "R", "$", "1.234.", "9".repeat(400).as_str(), "³", "√√", "--->"] {
            let toks = lex(line, 3);
            for t in &toks {
                assert!(t.range.start >= 3 && t.range.end <= line.len() + 3, "{line:?}");
            }
        }
    }
}
