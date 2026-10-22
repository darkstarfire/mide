//! Modo Programador: inteiros de 64 bits (complemento de dois) em hexadecimal,
//! decimal, octal ou binário, com operações de bits.

/// Base mostrada e digitada.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Base {
    Hex,
    Dec,
    Oct,
    Bin,
}

impl Base {
    pub fn radix(self) -> u32 {
        match self {
            Base::Hex => 16,
            Base::Dec => 10,
            Base::Oct => 8,
            Base::Bin => 2,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Base::Hex => "HEX",
            Base::Dec => "DEC",
            Base::Oct => "OCT",
            Base::Bin => "BIN",
        }
    }
}

/// Operações de dois números (precedência como em C).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    And,
    Or,
    Xor,
    Nor,
    /// X << Y e X >> Y
    Shl,
    Shr,
}

impl Op {
    /// Nome no histórico.
    pub fn symbol(self) -> &'static str {
        match self {
            Op::Add => "+",
            Op::Sub => "−",
            Op::Mul => "×",
            Op::Div => "÷",
            Op::Mod => "mod",
            Op::And => "AND",
            Op::Or => "OR",
            Op::Xor => "XOR",
            Op::Nor => "NOR",
            Op::Shl => "<<",
            Op::Shr => ">>",
        }
    }

    fn precedence(self) -> u8 {
        match self {
            Op::Mul | Op::Div | Op::Mod => 5,
            Op::Add | Op::Sub => 4,
            Op::Shl | Op::Shr => 3,
            Op::And => 2,
            Op::Xor => 1,
            Op::Or | Op::Nor => 0,
        }
    }

    /// `None`: divisão por zero. Os números têm sinal (complemento de dois), como um
    /// inteiro de 64 bits num programa: divisão, resto e deslocamento à direita
    /// aritmético (−8 >> 1 = −4); o estouro dá a volta (i64::MIN ÷ −1 = i64::MIN).
    fn apply(self, a: u64, b: u64) -> Option<u64> {
        Some(match self {
            Op::Add => a.wrapping_add(b),
            Op::Sub => a.wrapping_sub(b),
            Op::Mul => a.wrapping_mul(b),
            Op::Div | Op::Mod if b == 0 => return None,
            Op::Div => (a as i64).wrapping_div(b as i64) as u64,
            Op::Mod => (a as i64).wrapping_rem(b as i64) as u64,
            Op::And => a & b,
            Op::Or => a | b,
            Op::Xor => a ^ b,
            Op::Nor => !(a | b),
            Op::Shl => if b >= 64 { 0 } else { a << b },
            Op::Shr => ((a as i64) >> b.min(63)) as u64,
        })
    }
}

/// Funções de uma tecla.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unary {
    Not,
    Shl1,
    Shr1,
    RotL,
    RotR,
    /// Inverte a ordem dos bytes (endianness).
    ByteFlip,
    /// Complemento de dois (troca o sinal).
    Negate,
}

impl Unary {
    /// O número depois da função, para o histórico ("~(0x5)", "(0x1) << 1").
    fn describe(self, x: &str) -> String {
        match self {
            Unary::Not => format!("~({x})"),
            Unary::Shl1 => format!("({x}) << 1"),
            Unary::Shr1 => format!("({x}) >> 1"),
            Unary::RotL => format!("RoL({x})"),
            Unary::RotR => format!("RoR({x})"),
            Unary::ByteFlip => format!("byte⇄({x})"),
            Unary::Negate => format!("−({x})"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Digit(u8),
    Op(Op),
    Un(Unary),
    Equals,
    Clear,
    Backspace,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Programmer {
    pub base: Base,
    value: u64,
    /// Um número está sendo digitado (o próximo algarismo entra nele).
    typing: bool,
    pending: Vec<(u64, Op)>,
    /// A última tecla foi uma operação: outra operação troca esta.
    after_op: bool,
    /// Última operação do `=`, para repetir.
    last: Option<(Op, u64)>,
    error: bool,
    /// Texto da conta esperando, para o histórico ("0xFF AND ").
    text: String,
    /// Texto do número da tela no histórico depois de uma função ("~(0x5)").
    operand: Option<String>,
    /// A conta terminada no último "=" (expressão e resultado), para o app pôr no
    /// histórico (`take_record`).
    record: Option<(String, u64)>,
}

impl Default for Programmer {
    fn default() -> Self {
        Self {
            base: Base::Dec,
            value: 0,
            typing: false,
            pending: Vec::new(),
            after_op: false,
            last: None,
            error: false,
            text: String::new(),
            operand: None,
            record: None,
        }
    }
}

impl Programmer {
    pub fn value(&self) -> u64 {
        self.value
    }

    /// O número na base escolhida (decimal com sinal; binário em grupos de 4).
    pub fn display(&self) -> String {
        if self.error {
            return "Erro".into();
        }
        format_in(self.value, self.base)
    }

    /// O número para copiar (sem os espaços do binário).
    pub fn copy_text(&self) -> String {
        self.display().replace(' ', "")
    }

    /// O número como caractere Unicode (U+0041 é "A"), se for um caractere que dá
    /// para mostrar; senão, vazio.
    pub fn character(&self) -> String {
        u32::try_from(self.value)
            .ok()
            .and_then(char::from_u32)
            .filter(|c| !c.is_control() && !c.is_whitespace())
            .map(String::from)
            .unwrap_or_default()
    }

    /// "C" apaga só o número sendo digitado; sem ele, "AC" apaga tudo.
    pub fn clear_label(&self) -> &'static str {
        if self.typing && self.value != 0 && !self.error { "C" } else { "AC" }
    }

    /// Operação esperando o segundo número (a tecla fica marcada).
    pub fn pending_op(&self) -> Option<Op> {
        if self.after_op { self.pending.last().map(|(_, op)| *op) } else { None }
    }

    /// Troca a base (o número continua o mesmo; digitar continua acrescentando).
    pub fn set_base(&mut self, base: Base) {
        self.base = base;
    }

    /// Algarismo válido na base atual.
    pub fn accepts(&self, digit: u8) -> bool {
        (digit as u32) < self.base.radix()
    }

    /// Bit `n` (0 é o de baixo) clicado: inverte.
    pub fn toggle_bit(&mut self, n: u32) {
        if n < 64 && !self.error {
            self.value ^= 1 << n;
            self.typing = true;
            self.after_op = false;
            self.operand = None;
        }
    }

    /// Resolve as operações esperando com `v` à direita, enquanto a de cima tem
    /// precedência maior ou igual à de `op` (`None`: todas). `None` na volta: divisão
    /// por zero.
    fn reduce(&mut self, mut v: u64, op: Option<Op>) -> Option<u64> {
        while let Some(&(a, top)) = self.pending.last() {
            if op.is_some_and(|op| top.precedence() < op.precedence()) {
                break;
            }
            self.pending.pop();
            v = top.apply(a, v)?;
        }
        Some(v)
    }

    fn fail(&mut self) {
        self.error = true;
        self.pending.clear();
        self.after_op = false;
        self.typing = false;
        self.text.clear();
        self.operand = None;
    }

    /// Texto do número da tela para o histórico.
    fn operand_text(&self) -> String {
        self.operand.clone().unwrap_or_else(|| format_prefixed(self.value, self.base))
    }

    /// A conta terminada no último "=" (expressão e resultado), uma vez só.
    pub fn take_record(&mut self) -> Option<(String, u64)> {
        self.record.take()
    }

    pub fn press(&mut self, key: Key) {
        if self.error {
            if !matches!(key, Key::Clear | Key::Digit(_)) {
                return;
            }
            *self = Programmer { base: self.base, ..Default::default() };
            if key == Key::Clear {
                return;
            }
        }
        match key {
            Key::Digit(d) => {
                if !self.accepts(d) {
                    return;
                }
                if !self.typing {
                    self.value = 0;
                    self.typing = true;
                }
                self.after_op = false;
                self.operand = None;
                let radix = self.base.radix() as u64;
                if self.base == Base::Dec {
                    // Decimal com sinal: -5 seguido de 3 é -53; sem passar dos limites do i64.
                    let v = self.value as i64;
                    let next = v.checked_mul(radix as i64).and_then(|v| if v < 0 { v.checked_sub(d as i64) } else { v.checked_add(d as i64) });
                    if let Some(next) = next {
                        self.value = next as u64;
                    }
                } else if let Some(v) = self.value.checked_mul(radix).and_then(|v| v.checked_add(d as u64)) {
                    // Sem passar de 64 bits.
                    self.value = v;
                }
            }
            Key::Op(op) => {
                // Duas operações seguidas: vale a última, refazendo a precedência.
                if self.after_op
                    && let Some((a, _)) = self.pending.pop()
                {
                    let cut = self.text.trim_end().rfind(' ').unwrap_or(0);
                    self.text.truncate(cut);
                    self.text.push_str(&format!(" {} ", op.symbol()));
                    let Some(v) = self.reduce(a, Some(op)) else {
                        return self.fail();
                    };
                    self.pending.push((v, op));
                    self.value = v;
                    return;
                }
                let text = self.operand_text();
                let Some(v) = self.reduce(self.value, Some(op)) else {
                    return self.fail();
                };
                self.text.push_str(&format!("{text} {} ", op.symbol()));
                self.operand = None;
                self.pending.push((v, op));
                self.value = v;
                self.typing = false;
                self.after_op = true;
                self.last = None;
            }
            Key::Equals => {
                if self.pending.is_empty() {
                    // Sem nada esperando: repete a última operação (2 + 3 = = dá 8).
                    if let Some((op, b)) = self.last {
                        let expression = format!("{} {} {}", self.operand_text(), op.symbol(), format_prefixed(b, self.base));
                        match op.apply(self.value, b) {
                            Some(v) => {
                                self.value = v;
                                self.record = Some((expression, v));
                            }
                            None => return self.fail(),
                        }
                    }
                } else {
                    let b = self.value;
                    let expression = format!("{}{}", self.text, self.operand_text());
                    let last_op = self.pending.last().map(|(_, op)| *op);
                    let Some(v) = self.reduce(b, None) else {
                        return self.fail();
                    };
                    self.value = v;
                    self.last = last_op.map(|op| (op, b));
                    self.record = Some((expression.trim().to_string(), v));
                }
                self.text.clear();
                self.operand = None;
                self.typing = false;
                self.after_op = false;
            }
            Key::Un(f) => {
                // No histórico, a função em volta do número ("~(0x5)"); ± num número
                // sendo digitado é só o sinal dele.
                let described = (f != Unary::Negate || !self.typing).then(|| f.describe(&self.operand_text()));
                let v = self.value;
                self.value = match f {
                    Unary::Not => !v,
                    Unary::Shl1 => v << 1,
                    Unary::Shr1 => ((v as i64) >> 1) as u64,
                    Unary::RotL => v.rotate_left(1),
                    Unary::RotR => v.rotate_right(1),
                    Unary::ByteFlip => v.swap_bytes(),
                    Unary::Negate => v.wrapping_neg(),
                };
                // ± no número sendo digitado: a digitação continua (com o sinal novo).
                if f != Unary::Negate {
                    self.typing = false;
                }
                self.after_op = false;
                self.operand = described;
            }
            Key::Clear => {
                if self.clear_label() == "C" {
                    self.value = 0;
                    self.operand = None;
                } else {
                    *self = Programmer { base: self.base, ..Default::default() };
                }
            }
            Key::Backspace => {
                if self.typing {
                    let radix = self.base.radix() as u64;
                    self.value = if self.base == Base::Dec { ((self.value as i64) / radix as i64) as u64 } else { self.value / radix };
                }
            }
        }
    }

    /// Valor vindo dos outros modos: a parte inteira começa uma conta nova. De −2⁶³ até
    /// 2⁶⁴ − 1 cabe nos 64 bits (acima de 2⁶³ − 1, aparece negativo no DEC, como o mesmo
    /// padrão de bits); fora disso, Erro.
    pub fn load(&mut self, v: f64) {
        let base = self.base;
        *self = Programmer { base, ..Default::default() };
        let v = v.trunc();
        if v.is_finite() && (-(2f64.powi(63))..2f64.powi(64)).contains(&v) {
            self.value = v as i128 as u64;
        } else {
            self.error = true;
        }
    }

    /// Um número inteiro exato (de uma conta do histórico): começa uma conta nova.
    pub fn load_bits(&mut self, v: u64) {
        let base = self.base;
        *self = Programmer { base, ..Default::default() };
        self.value = v;
    }

    /// Número colado: na base atual ("FF" em HEX, com ou sem "0x"; espaços e "_"
    /// ignorados), ou um número decimal comum ("1.234", "-3,7" vira -3).
    pub fn paste(&mut self, text: &str) {
        let clean: String = text.trim().chars().filter(|c| !c.is_whitespace() && *c != '_').collect();
        let (radix, digits) = match clean.get(..2).map(str::to_ascii_lowercase).as_deref() {
            Some("0x") => (16, &clean[2..]),
            Some("0b") => (2, &clean[2..]),
            Some("0o") => (8, &clean[2..]),
            _ => (self.base.radix(), clean.as_str()),
        };
        let parsed = if radix == 10 {
            digits.parse::<i64>().ok().map(|v| v as u64)
        } else {
            u64::from_str_radix(digits, radix).ok()
        };
        match parsed {
            Some(v) => {
                self.value = v;
                self.typing = false;
                self.after_op = false;
                self.error = false;
                self.operand = None;
            }
            None => {
                if let Some(v) = super::format::parse(text) {
                    self.load(v);
                }
            }
        }
    }
}

/// O número para o histórico, com a base no prefixo ("0xFF", "0o17", "0b101"); o
/// decimal, com sinal e sem prefixo.
pub fn format_prefixed(v: u64, base: Base) -> String {
    match base {
        Base::Dec => (v as i64).to_string(),
        Base::Hex => format!("0x{v:X}"),
        Base::Oct => format!("0o{v:o}"),
        Base::Bin => format!("0b{v:b}"),
    }
}

/// Número na base: decimal com sinal, as outras sem sinal; binário em grupos de 4.
pub fn format_in(v: u64, base: Base) -> String {
    match base {
        Base::Dec => (v as i64).to_string(),
        Base::Hex => format!("{v:X}"),
        Base::Oct => format!("{v:o}"),
        Base::Bin => {
            let s = format!("{v:b}");
            let pad = (4 - s.len() % 4) % 4;
            let s = format!("{}{s}", "0".repeat(pad));
            s.as_bytes().chunks(4).map(|c| std::str::from_utf8(c).unwrap_or("")).collect::<Vec<_>>().join(" ")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press_all(p: &mut Programmer, keys: &[Key]) {
        for k in keys {
            p.press(*k);
        }
    }

    #[test]
    fn bases_e_bits() {
        let mut p = Programmer::default();
        press_all(&mut p, &[Key::Digit(2), Key::Digit(5), Key::Digit(5)]);
        p.set_base(Base::Hex);
        assert_eq!(p.display(), "FF");
        p.set_base(Base::Bin);
        assert_eq!(p.display(), "1111 1111");
        p.set_base(Base::Oct);
        assert_eq!(p.display(), "377");
        p.set_base(Base::Hex);
        press_all(&mut p, &[Key::Clear, Key::Clear, Key::Digit(0xA), Key::Op(Op::And), Key::Digit(0xC), Key::Equals]);
        assert_eq!(p.display(), "8");
        press_all(&mut p, &[Key::Clear, Key::Digit(1), Key::Op(Op::Shl), Key::Digit(4), Key::Equals]);
        assert_eq!(p.display(), "10");
        // Precedência: 1 | 2 & 3 = 1 | (2 & 3) = 3.
        p.set_base(Base::Dec);
        press_all(&mut p, &[Key::Clear, Key::Digit(1), Key::Op(Op::Or), Key::Digit(2), Key::Op(Op::And), Key::Digit(3), Key::Equals]);
        assert_eq!(p.display(), "3");
        press_all(&mut p, &[Key::Clear, Key::Digit(5), Key::Un(Unary::Negate)]);
        assert_eq!(p.display(), "-5");
        p.set_base(Base::Hex);
        assert_eq!(p.display(), "FFFFFFFFFFFFFFFB");
        p.set_base(Base::Dec);
        press_all(&mut p, &[Key::Clear, Key::Digit(6), Key::Digit(5)]);
        assert_eq!(p.character(), "A");
        p.toggle_bit(5);
        assert_eq!(p.display(), "97");
        press_all(&mut p, &[Key::Clear, Key::Digit(1), Key::Op(Op::Div), Key::Digit(0), Key::Equals]);
        assert_eq!(p.display(), "Erro");
        press_all(&mut p, &[Key::Clear, Key::Digit(0x1)]);
        p.set_base(Base::Hex);
        p.press(Key::Digit(2));
        p.press(Key::Un(Unary::ByteFlip));
        assert_eq!(p.display(), "1200000000000000");
        assert!(!Programmer { base: Base::Oct, ..Default::default() }.accepts(8));
    }

    #[test]
    fn operacoes_seguidas_e_sinal() {
        let mut p = Programmer::default();
        // Duas operações seguidas: vale a última (2 + × 3 = 6).
        press_all(&mut p, &[Key::Digit(2), Key::Op(Op::Add), Key::Op(Op::Mul), Key::Digit(3), Key::Equals]);
        assert_eq!(p.display(), "6");
        // "=" repetido refaz a última operação.
        press_all(&mut p, &[Key::Clear, Key::Digit(2), Key::Op(Op::Add), Key::Digit(3), Key::Equals, Key::Equals]);
        assert_eq!(p.display(), "8");
        // ± durante a digitação: -5 seguido de 3 é -53.
        press_all(&mut p, &[Key::Clear, Key::Clear, Key::Digit(5), Key::Un(Unary::Negate), Key::Digit(3)]);
        assert_eq!(p.display(), "-53");
        press_all(&mut p, &[Key::Backspace]);
        assert_eq!(p.display(), "-5");
        // i64::MIN ÷ −1 dá a volta (como × −1), e o resto é 0; só ÷ 0 é erro.
        p.load(i64::MIN as f64);
        press_all(&mut p, &[Key::Op(Op::Div), Key::Digit(1), Key::Un(Unary::Negate), Key::Equals]);
        assert_eq!(p.display(), "-9223372036854775808");
        press_all(&mut p, &[Key::Op(Op::Mod), Key::Digit(1), Key::Un(Unary::Negate), Key::Equals]);
        assert_eq!(p.display(), "0");
        press_all(&mut p, &[Key::Op(Op::Div), Key::Digit(0), Key::Equals]);
        assert_eq!(p.display(), "Erro");
        press_all(&mut p, &[Key::Digit(7)]);
        assert_eq!(p.display(), "7");
        // Deslocamento à direita aritmético, igual à divisão por 2.
        press_all(&mut p, &[Key::Clear, Key::Digit(8), Key::Un(Unary::Negate), Key::Un(Unary::Shr1)]);
        assert_eq!(p.display(), "-4");
        press_all(&mut p, &[Key::Op(Op::Shr), Key::Digit(1), Key::Equals]);
        assert_eq!(p.display(), "-2");
        // Valores de fora: 2⁶⁴ − 1 vira o padrão de bits (−1 no DEC); 1e30 é erro.
        p.load(18_446_744_073_709_549_568.0);
        p.set_base(Base::Hex);
        assert_eq!(p.display(), "FFFFFFFFFFFFF800");
        p.load(1e30);
        assert_eq!(p.display(), "Erro");
        // Carregar começa uma conta nova (a operação de antes sai).
        p.set_base(Base::Dec);
        press_all(&mut p, &[Key::Clear, Key::Digit(5), Key::Op(Op::Add)]);
        p.load(3.0);
        press_all(&mut p, &[Key::Equals]);
        assert_eq!(p.display(), "3");
    }

    #[test]
    fn historico_do_programador() {
        let mut p = Programmer::default();
        p.set_base(Base::Hex);
        press_all(&mut p, &[Key::Digit(0xF), Key::Digit(0xF), Key::Op(Op::And), Key::Digit(0xF), Key::Equals]);
        assert_eq!(p.take_record(), Some(("0xFF AND 0xF".to_string(), 0xF)));
        assert_eq!(p.take_record(), None);
        // "=" repetido: F AND F.
        press_all(&mut p, &[Key::Equals]);
        assert_eq!(p.take_record(), Some(("0xF AND 0xF".to_string(), 0xF)));
        // Função em volta do número.
        press_all(&mut p, &[Key::Clear, Key::Digit(5), Key::Un(Unary::Not), Key::Op(Op::Add), Key::Digit(1), Key::Equals]);
        assert_eq!(p.take_record().map(|r| r.0), Some("~(0x5) + 0x1".to_string()));
        // Troca de operação: o texto troca junto.
        press_all(&mut p, &[Key::Clear, Key::Digit(2), Key::Op(Op::Add), Key::Op(Op::Shl), Key::Digit(4), Key::Equals]);
        assert_eq!(p.take_record(), Some(("0x2 << 0x4".to_string(), 0x20)));
        // Decimal com sinal, sem prefixo; ± durante a digitação é só o sinal.
        p.set_base(Base::Dec);
        press_all(&mut p, &[Key::Clear, Key::Digit(5), Key::Un(Unary::Negate), Key::Op(Op::Mul), Key::Digit(3), Key::Equals]);
        assert_eq!(p.take_record(), Some(("-5 × 3".to_string(), (-15i64) as u64)));
        // Clicar na conta do histórico traz o número exato.
        p.load_bits(u64::MAX - 1);
        assert_eq!(p.display(), "-2");
    }

    #[test]
    fn caractere_e_colar() {
        let mut p = Programmer::default();
        p.paste("234");
        assert_eq!(p.character(), "ê");
        p.paste("128512");
        assert_eq!(p.character(), "😀");
        p.paste("10");
        assert_eq!(p.character(), "");
        p.set_base(Base::Hex);
        p.paste("ff");
        assert_eq!(p.value(), 255);
        p.paste("0x1F");
        assert_eq!(p.value(), 31);
        p.set_base(Base::Dec);
        p.paste("-3");
        assert_eq!(p.display(), "-3");
        p.paste("1.234");
        assert_eq!(p.display(), "1234");
        assert_eq!(p.copy_text(), "1234");
        p.set_base(Base::Bin);
        assert_eq!(p.copy_text(), "010011010010");
    }
}
