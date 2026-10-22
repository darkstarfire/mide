//! Contas dos modos Básico e Científico: o número da tela, operações pendentes com
//! precedência (uma pilha por nível de parênteses), funções de uma tecla, memória,
//! Rad/Grau e o histórico das contas feitas.

use super::format::{Style, DIGITS};

/// Operações de dois números, na ordem da precedência.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Binary {
    Add,
    Sub,
    Mul,
    Div,
    /// xʸ
    Pow,
    /// yˣ (com 2nd): o segundo número elevado ao primeiro (3 yˣ 2 = 2³ = 8).
    PowRev,
    /// ʸ√x (raiz y-ésima de x)
    Root,
    /// x × 10ʸ (EE)
    Ee,
    /// log na base y de x (com 2nd)
    LogBase,
}

impl Binary {
    fn precedence(self) -> u8 {
        match self {
            Binary::Add | Binary::Sub => 1,
            Binary::Mul | Binary::Div => 2,
            Binary::Pow | Binary::PowRev | Binary::Root | Binary::LogBase => 3,
            Binary::Ee => 4,
        }
    }

    /// xʸ e EE agrupam da direita (2 ^ 3 ^ 2 = 2 ^ 9).
    fn right_assoc(self) -> bool {
        matches!(self, Binary::Pow | Binary::Ee)
    }

    pub fn apply(self, a: f64, b: f64) -> f64 {
        match self {
            Binary::Add => cancel_noise(a + b, a, b),
            Binary::Sub => cancel_noise(a - b, a, b),
            Binary::Mul => a * b,
            Binary::Div => a / b,
            Binary::Pow => a.powf(b),
            Binary::PowRev => b.powf(a),
            Binary::Root => {
                // Raiz ímpar de negativo: o valor real (∛-8 = -2).
                if a < 0.0 && b.fract() == 0.0 && (b as i64) % 2 != 0 {
                    -(-a).powf(1.0 / b)
                } else {
                    a.powf(1.0 / b)
                }
            }
            Binary::Ee => a * 10f64.powf(b),
            Binary::LogBase => a.ln() / b.ln(),
        }
    }

    pub fn symbol(self) -> &'static str {
        match self {
            Binary::Add => "+",
            Binary::Sub => "−",
            Binary::Mul => "×",
            Binary::Div => "÷",
            Binary::Pow => "^",
            Binary::PowRev => "yˣ",
            Binary::Root => "ʸ√",
            Binary::Ee => "E",
            Binary::LogBase => "logᵧ",
        }
    }
}

/// Funções de uma tecla: valem na hora para o número da tela.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unary {
    Square,
    Cube,
    Exp,
    Exp10,
    Exp2,
    Recip,
    Sqrt,
    Cbrt,
    Ln,
    Log10,
    Log2,
    Fact,
    Sin,
    Cos,
    Tan,
    Asin,
    Acos,
    Atan,
    Sinh,
    Cosh,
    Tanh,
    Asinh,
    Acosh,
    Atanh,
    Percent,
    Negate,
}

impl Unary {
    /// Nome no histórico.
    pub fn name(self) -> &'static str {
        match self {
            Unary::Square => "sqr",
            Unary::Cube => "cube",
            Unary::Exp => "exp",
            Unary::Exp10 => "10^",
            Unary::Exp2 => "2^",
            Unary::Recip => "1/",
            Unary::Sqrt => "√",
            Unary::Cbrt => "∛",
            Unary::Ln => "ln",
            Unary::Log10 => "log",
            Unary::Log2 => "log₂",
            Unary::Fact => "fact",
            Unary::Sin => "sen",
            Unary::Cos => "cos",
            Unary::Tan => "tg",
            Unary::Asin => "arcsen",
            Unary::Acos => "arccos",
            Unary::Atan => "arctg",
            Unary::Sinh => "senh",
            Unary::Cosh => "cosh",
            Unary::Tanh => "tgh",
            Unary::Asinh => "arcsenh",
            Unary::Acosh => "arccosh",
            Unary::Atanh => "arctgh",
            Unary::Percent => "%",
            Unary::Negate => "−",
        }
    }

    /// Aplica a `x`; `radians`: ângulos em radianos (senão, em graus).
    pub fn apply(self, x: f64, radians: bool) -> f64 {
        let from_rad = |v: f64| if radians { v } else { v.to_degrees() };
        match self {
            Unary::Square => x * x,
            Unary::Cube => x * x * x,
            Unary::Exp => x.exp(),
            Unary::Exp10 => 10f64.powf(x),
            Unary::Exp2 => 2f64.powf(x),
            Unary::Recip => 1.0 / x,
            Unary::Sqrt => x.sqrt(),
            Unary::Cbrt => x.cbrt(),
            Unary::Ln => x.ln(),
            Unary::Log10 => x.log10(),
            Unary::Log2 => x.log2(),
            Unary::Fact => factorial(x),
            Unary::Sin | Unary::Cos | Unary::Tan => trig(self, x, radians),
            Unary::Asin => from_rad(x.asin()),
            Unary::Acos => from_rad(x.acos()),
            Unary::Atan => from_rad(x.atan()),
            Unary::Sinh => x.sinh(),
            Unary::Cosh => x.cosh(),
            Unary::Tanh => x.tanh(),
            Unary::Asinh => x.asinh(),
            Unary::Acosh => x.acosh(),
            Unary::Atanh => x.atanh(),
            Unary::Percent => x / 100.0,
            Unary::Negate => -x,
        }
    }
}

/// Seno, cosseno e tangente. Em graus, o ângulo é reduzido a uma volta (o resto da
/// divisão por 360 é exato) e os múltiplos de 90° saem exatos (sen 1800° = 0; tg 90°
/// não existe). Em radianos, o que sobra do arredondamento de π (sen π = 1,2e-16) vira
/// 0, sem zerar o seno de um ângulo minúsculo (sen 1e-20 = 1e-20).
fn trig(f: Unary, x: f64, radians: bool) -> f64 {
    let pick = |s: f64, c: f64, t: f64| match f {
        Unary::Sin => s,
        Unary::Cos => c,
        _ => t,
    };
    if !radians {
        let r = x % 360.0;
        if r % 90.0 == 0.0 {
            let (s, c) = [(0.0, 1.0), (1.0, 0.0), (0.0, -1.0), (-1.0, 0.0)][((r / 90.0) as i64).rem_euclid(4) as usize];
            return pick(s, c, if c == 0.0 { f64::NAN } else { s / c });
        }
        let rad = r.to_radians();
        return pick(rad.sin(), rad.cos(), rad.tan());
    }
    let v = pick(x.sin(), x.cos(), x.tan());
    if v.abs() < 1e-15 * x.abs() { 0.0 } else { v }
}

/// Soma ou subtração que deveria dar zero e sobrou o erro de arredondamento do `f64`
/// (0,1 + 0,2 − 0,3 dá 5,55e-17): zero, como na calculadora de mesa. Só quando o
/// resultado é menor que uns 2 algarismos no fim dos números (4 ε relativos).
fn cancel_noise(r: f64, a: f64, b: f64) -> f64 {
    if r != 0.0 && r.abs() < 4.0 * f64::EPSILON * a.abs().max(b.abs()) { 0.0 } else { r }
}

/// x!: inteiros até 170 multiplicando; os outros pela função gama (Γ(x + 1)).
pub fn factorial(x: f64) -> f64 {
    if x < 0.0 && x.fract() == 0.0 {
        return f64::NAN;
    }
    if x.fract() == 0.0 && x <= 170.0 {
        return (1..=x as u64).fold(1.0, |acc, n| acc * n as f64);
    }
    gamma(x + 1.0)
}

/// Função gama pela aproximação de Lanczos (g = 7, 9 coeficientes), com a reflexão
/// para x < 0,5.
fn gamma(x: f64) -> f64 {
    const G: f64 = 7.0;
    const C: [f64; 9] = [
        0.999_999_999_999_809_9,
        676.520_368_121_885_1,
        -1_259.139_216_722_402_8,
        771.323_428_777_653_1,
        -176.615_029_162_140_6,
        12.507_343_278_686_905,
        -0.138_571_095_265_720_12,
        9.984_369_578_019_572e-6,
        1.505_632_735_149_311_6e-7,
    ];
    if x < 0.5 {
        return std::f64::consts::PI / ((std::f64::consts::PI * x).sin() * gamma(1.0 - x));
    }
    let x = x - 1.0;
    let mut a = C[0];
    let t = x + G + 0.5;
    for (i, c) in C.iter().enumerate().skip(1) {
        a += c / (x + i as f64);
    }
    // tˣ⁺⁰·⁵ em duas metades: inteira, passaria do f64 antes de o e⁻ᵗ trazer de volta
    // (170,5! cabe no f64, mas 177¹⁷¹ não).
    let half = t.powf((x + 0.5) / 2.0);
    (2.0 * std::f64::consts::PI).sqrt() * half * (-t).exp() * half * a
}

/// Uma tecla dos modos Básico e Científico.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Key {
    Digit(u8),
    Comma,
    Bin(Binary),
    Un(Unary),
    Open,
    Close,
    Equals,
    Clear,
    Backspace,
    Pi,
    E,
    Rand,
    /// Memória: apagar, somar, subtrair e trazer.
    MemClear,
    MemAdd,
    MemSub,
    MemRecall,
}

/// Contas guardadas no histórico (as mais antigas saem).
pub const HISTORY_MAX: usize = 500;

/// Uma conta feita (para o histórico): a expressão e o resultado, como na tela, e o
/// valor exato (clicar na conta traz ele de volta).
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub expression: String,
    pub result: String,
    pub value: f64,
    /// Conta do Programador: o resultado inteiro de 64 bits (o `f64` perde precisão
    /// acima de 2⁵³).
    pub bits: Option<u64>,
}

#[derive(Debug, Default, Clone, PartialEq)]
struct Level {
    /// Números e operações esperando (a operação vem depois do número).
    pending: Vec<(f64, Binary)>,
    /// Texto desta parte da conta (para o histórico).
    text: String,
}

/// O número da tela.
#[derive(Debug, Clone, PartialEq)]
enum Shown {
    /// Sendo digitado, como aparece ("12,5", "-", "0,"); vazio é zero.
    Typed(String),
    /// Resultado, constante, memória ou número colado, com a precisão inteira (as
    /// contas seguintes não partem do número cortado em 12 algarismos da tela).
    Value(f64),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Engine {
    shown: Shown,
    /// Texto do número da tela no histórico (o digitado, ou "√(9)" depois de uma função).
    operand: Option<String>,
    levels: Vec<Level>,
    /// Última operação do `=`, para repetir.
    last: Option<(Binary, f64)>,
    /// A última tecla foi uma operação (+, ×, xʸ...): outra operação troca esta, e o
    /// próximo algarismo começa outro número.
    after_op: bool,
    error: bool,
    pub radians: bool,
    pub memory: Option<f64>,
    pub history: Vec<Entry>,
    /// Casas decimais e ponto de milhar da tela.
    pub style: Style,
    /// A conta terminada no último "=", mostrada acima do resultado até a próxima tecla.
    finished: Option<String>,
    /// Texto da última conversão aplicada (`apply`): sozinha, ela vai para o histórico
    /// sem os parênteses de fora.
    conversion: Option<String>,
    seed: u64,
}

impl Default for Engine {
    fn default() -> Self {
        let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(7) | 1;
        Self {
            shown: Shown::Typed(String::new()),
            operand: None,
            levels: vec![Level::default()],
            last: None,
            after_op: false,
            error: false,
            radians: false,
            memory: None,
            history: Vec::new(),
            style: Style::default(),
            finished: None,
            conversion: None,
            seed,
        }
    }
}

impl Engine {
    /// Motor novo com as opções lembradas (Rad/Grau, casas decimais e ponto de milhar).
    pub fn new(radians: bool, style: Style) -> Self {
        Self { radians, style, ..Self::default() }
    }

    /// Valor do número da tela.
    pub fn value(&self) -> f64 {
        match &self.shown {
            // "2e" e "2e-" (o expoente ainda sem algarismos) valem 2.
            Shown::Typed(t) => super::format::parse(t.trim_end_matches('-').trim_end_matches('e')).unwrap_or(0.0),
            Shown::Value(v) => *v,
        }
    }

    /// O que a tela mostra.
    pub fn display(&self) -> String {
        self.shown_with(self.style)
    }

    /// O número da tela para copiar: sem pontos de milhar ("1234,5"), para colar em
    /// qualquer lugar sem confundir com casas decimais.
    pub fn copy_text(&self) -> String {
        self.shown_with(Style { grouping: false, ..self.style })
    }

    fn shown_with(&self, style: Style) -> String {
        if self.error {
            return "Erro".into();
        }
        match &self.shown {
            Shown::Typed(t) if t.is_empty() => "0".into(),
            Shown::Typed(t) if t == "-" => "-0".into(),
            Shown::Typed(t) => style.digits(t),
            Shown::Value(v) => style.value(*v),
        }
    }

    /// "C" apaga só o número sendo digitado; sem ele, "AC" apaga tudo.
    pub fn clear_label(&self) -> &'static str {
        match &self.shown {
            Shown::Typed(t) if !t.is_empty() && !self.error => "C",
            _ => "AC",
        }
    }

    /// Operação esperando o segundo número (a tecla fica marcada).
    pub fn pending_op(&self) -> Option<Binary> {
        if !self.after_op {
            return None;
        }
        self.levels.last().and_then(|l| l.pending.last()).map(|(_, op)| *op)
    }

    /// Parênteses abertos.
    pub fn open_parens(&self) -> usize {
        self.levels.len() - 1
    }

    pub fn has_error(&self) -> bool {
        self.error
    }

    fn set_value(&mut self, v: f64) {
        if v.is_finite() {
            self.shown = Shown::Value(if v == 0.0 { 0.0 } else { v });
        } else {
            self.error = true;
            self.shown = Shown::Typed(String::new());
        }
    }

    /// Texto do número da tela para o histórico (sem a vírgula de um "2," digitado).
    fn operand_text(&self) -> String {
        self.operand.clone().unwrap_or_else(|| self.display().trim_end_matches(',').to_string())
    }

    /// O número sendo digitado (começa um novo depois de uma operação ou resultado).
    fn typing(&mut self) -> &mut String {
        if self.after_op || !matches!(self.shown, Shown::Typed(_)) {
            self.shown = Shown::Typed(String::new());
        }
        self.after_op = false;
        self.operand = None;
        match &mut self.shown {
            Shown::Typed(t) => t,
            Shown::Value(_) => unreachable!("trocado por Typed acima"),
        }
    }

    /// Recomeça do zero, guardando o que não é da conta (memória, histórico, Rad/Grau,
    /// casas decimais).
    fn reset(&mut self) {
        *self = Engine {
            radians: self.radians,
            memory: self.memory,
            history: std::mem::take(&mut self.history),
            style: self.style,
            seed: self.seed,
            ..Engine::default()
        };
    }

    /// Resolve as operações do nível de cima com `v` à direita, enquanto a de cima
    /// tem precedência maior (ou igual, se `op` agrupa da esquerda). `None`: todas.
    fn reduce(&mut self, mut v: f64, op: Option<Binary>) -> f64 {
        let level = self.levels.last_mut().expect("sempre há um nível");
        while let Some(&(a, top)) = level.pending.last() {
            let go = match op {
                None => true,
                Some(op) if op.right_assoc() => top.precedence() > op.precedence(),
                Some(op) => top.precedence() >= op.precedence(),
            };
            if !go {
                break;
            }
            level.pending.pop();
            v = top.apply(a, v);
        }
        v
    }

    pub fn press(&mut self, key: Key) {
        self.finished = None;
        if self.error {
            // Depois de um erro, só o que começa um número novo (ou limpa) vale.
            if !matches!(key, Key::Clear | Key::Digit(_) | Key::Comma | Key::Pi | Key::E | Key::Rand | Key::MemRecall | Key::Open) {
                return;
            }
            self.reset();
            if key == Key::Clear {
                return;
            }
        }
        match key {
            Key::Digit(d) => {
                let t = self.typing();
                // No expoente (depois do EE): até 3 algarismos, sem passar do maior número.
                if let Some((_, exp)) = t.split_once('e') {
                    let next = format!("{t}{d}");
                    if exp.trim_start_matches('-').len() < 3 && super::format::parse(&next).is_some() {
                        *t = next;
                    }
                    return;
                }
                if t.chars().filter(char::is_ascii_digit).count() >= DIGITS {
                    return;
                }
                if t == "0" {
                    t.clear();
                } else if t == "-0" {
                    *t = "-".into();
                }
                t.push((b'0' + d) as char);
            }
            Key::Comma => {
                let t = self.typing();
                if t.contains('e') {
                    return;
                }
                if t.is_empty() || t == "-" {
                    t.push('0');
                }
                if !t.contains(',') {
                    t.push(',');
                }
            }
            // Com um número sendo digitado, o EE faz parte dele: 2 EE 3 mostra "2e3" (e ±
            // depois do EE troca o sinal do expoente). Depois de um resultado ou de uma
            // operação, o EE é a operação x × 10ʸ.
            Key::Bin(Binary::Ee) if !self.after_op && matches!(&self.shown, Shown::Typed(t) if !t.is_empty() && t != "-") => {
                if let Shown::Typed(t) = &mut self.shown
                    && !t.contains('e')
                {
                    if t.ends_with(',') {
                        t.pop();
                    }
                    t.push('e');
                }
            }
            Key::Bin(op) => {
                // Duas operações seguidas: vale a última (2 + × 3 = 6), refazendo a
                // precedência (2 − 3 × trocado por − dá 2 − 3 − ...).
                if self.after_op
                    && let Some(level) = self.levels.last_mut()
                    && let Some((a, _)) = level.pending.pop()
                {
                    let cut = level.text.trim_end().rfind(' ').unwrap_or(0);
                    level.text.truncate(cut);
                    level.text.push_str(&format!(" {} ", op.symbol()));
                    let v = self.reduce(a, Some(op));
                    self.levels.last_mut().expect("nível").pending.push((v, op));
                    self.set_value(v);
                    return;
                }
                let text = self.operand_text();
                let v = self.reduce(self.value(), Some(op));
                let level = self.levels.last_mut().expect("nível");
                level.text.push_str(&format!("{text} {} ", op.symbol()));
                level.pending.push((v, op));
                self.set_value(v);
                self.operand = None;
                self.last = None;
                self.after_op = true;
            }
            Key::Un(Unary::Percent) => {
                // Com soma ou subtração esperando: a porcentagem do número de antes
                // (100 + 10% = 110); senão, dividido por 100.
                let v = self.value();
                let base = self.levels.last().and_then(|l| l.pending.last()).filter(|(_, op)| matches!(op, Binary::Add | Binary::Sub)).map(|(a, _)| *a);
                let text = self.operand_text();
                self.set_value(match base {
                    Some(a) => a * v / 100.0,
                    None => v / 100.0,
                });
                self.operand = Some(format!("{text}%"));
                self.after_op = false;
            }
            Key::Un(Unary::Negate) => {
                if self.after_op {
                    // Logo depois de uma operação: começa um número negativo (5 × ± 3 = −15).
                    *self.typing() = "-".into();
                    return;
                }
                match &mut self.shown {
                    Shown::Typed(t) => {
                        // Depois do EE, troca o sinal do expoente.
                        let at = t.find('e').map_or(0, |i| i + 1);
                        if t[at..].starts_with('-') {
                            t.remove(at);
                        } else {
                            t.insert(at, '-');
                        }
                    }
                    Shown::Value(v) => {
                        *v = -*v;
                        if let Some(text) = &self.operand {
                            self.operand = Some(format!("−({text})"));
                        }
                    }
                }
            }
            Key::Un(f) => {
                let text = self.operand_text();
                let v = f.apply(self.value(), self.radians);
                self.set_value(v);
                self.operand = Some(format!("{}({text})", f.name()));
                self.after_op = false;
            }
            Key::Open => {
                // Número na tela antes do "(": multiplica (2 (3 + 4) = 14), como nas
                // calculadoras científicas. Depois de uma operação ou de um "=", não.
                let typed = matches!(&self.shown, Shown::Typed(t) if !t.is_empty());
                if !self.after_op && (typed || self.operand.is_some()) {
                    self.press(Key::Bin(Binary::Mul));
                    if self.error {
                        return;
                    }
                }
                self.levels.push(Level::default());
                self.shown = Shown::Typed(String::new());
                self.operand = None;
                self.after_op = false;
            }
            Key::Close => {
                if self.levels.len() < 2 {
                    return;
                }
                let text = self.operand_text();
                let v = self.reduce(self.value(), None);
                let level = self.levels.pop().expect("nível");
                self.set_value(v);
                self.operand = Some(format!("({}{text})", level.text));
                self.after_op = false;
            }
            Key::Equals => self.equals(),
            Key::Clear => {
                if self.clear_label() == "C" {
                    self.shown = Shown::Typed(String::new());
                    self.operand = None;
                } else {
                    self.reset();
                }
            }
            Key::Backspace => {
                if let Shown::Typed(t) = &mut self.shown
                    && !self.after_op
                {
                    t.pop();
                    if t == "-" {
                        t.clear();
                    }
                }
            }
            Key::Pi => self.constant(std::f64::consts::PI, "π"),
            Key::E => self.constant(std::f64::consts::E, "e"),
            Key::Rand => {
                let r = self.random();
                self.constant(r, "rand");
            }
            Key::MemClear => self.memory = None,
            // m+ e m− fecham o número da tela: o próximo algarismo começa outro.
            Key::MemAdd | Key::MemSub => {
                let v = self.value();
                self.memory_add(if key == Key::MemAdd { v } else { -v });
                if matches!(self.shown, Shown::Typed(_)) {
                    self.shown = Shown::Value(v);
                }
            }
            Key::MemRecall => {
                if let Some(m) = self.memory {
                    self.set_value(m);
                    self.operand = None;
                    self.after_op = false;
                }
            }
        }
    }

    fn equals(&mut self) {
        let text;
        if self.levels.len() == 1 && self.levels[0].pending.is_empty() {
            if let Some((op, b)) = self.last {
                // Sem nada esperando: repete a última operação (2 + 3 = = dá 8).
                text = format!("{} {} {}", self.operand_text(), op.symbol(), self.style.value(b));
                self.set_value(op.apply(self.value(), b));
            } else if let Some(operand) = self.operand.take() {
                // Só uma função, parênteses ou uma conversão (√9 =): também vai para o
                // histórico; a conversão sozinha, sem os parênteses de fora.
                text = match &self.conversion {
                    Some(c) if operand == format!("({c})") => c.clone(),
                    _ => operand,
                };
                let v = self.value();
                self.set_value(v);
            } else {
                // Só um número: fica como resultado (o próximo algarismo começa outro).
                let v = self.value();
                self.set_value(v);
                return;
            }
        } else {
            // Fecha os parênteses que ficaram abertos.
            let mut v = self.value();
            let mut operand = self.operand_text();
            while self.levels.len() > 1 {
                v = self.reduce(v, None);
                let level = self.levels.pop().expect("nível");
                operand = format!("({}{operand})", level.text);
            }
            // O "=" repetido refaz a operação de fora dos parênteses: 2 × (3 + 4 = = repete × 7.
            let b = v;
            let last_op = self.levels[0].pending.last().map(|(_, op)| *op);
            v = self.reduce(v, None);
            let level = std::mem::take(&mut self.levels[0]);
            text = format!("{}{operand}", level.text);
            self.last = last_op.map(|op| (op, b));
            self.set_value(v);
        }
        if !self.error {
            let expression = text.trim().to_string();
            self.record(Entry { expression: expression.clone(), result: self.display(), value: self.value(), bits: None });
            self.finished = Some(expression);
        }
        self.levels = vec![Level::default()];
        self.operand = None;
        self.after_op = false;
    }

    /// A conta em andamento, em letra pequena acima do número ("2 × (3 +"); logo depois
    /// do "=", a conta feita ("2 + 3 × 4 =").
    pub fn expression(&self) -> String {
        if self.error {
            return String::new();
        }
        let pending: String = self.levels.iter().enumerate().map(|(i, l)| if i == 0 { l.text.clone() } else { format!("({}", l.text) }).collect();
        let pending = pending.trim_end();
        if !pending.is_empty() {
            return pending.to_string();
        }
        self.finished.as_ref().map(|e| format!("{e} =")).unwrap_or_default()
    }

    /// Guarda uma conta no histórico (também as do RPN e do Programador).
    pub fn record(&mut self, entry: Entry) {
        self.history.push(entry);
        if self.history.len() > HISTORY_MAX {
            self.history.remove(0);
        }
    }

    /// Uma conversão (ou outra conta feita fora do motor) aplicada ao número da tela,
    /// como uma tecla de função: o resultado vira o número da tela e, no histórico, a
    /// conta aparece entre parênteses ("2 + (12,5 pol → cm)"). Com Erro, nada muda.
    pub fn apply(&mut self, v: f64, text: &str) {
        if self.error {
            return;
        }
        self.finished = None;
        self.set_value(v);
        self.operand = Some(format!("({text})"));
        self.conversion = Some(text.to_string());
        self.after_op = false;
    }

    /// Número entre 0 e 1 (xorshift64).
    pub fn random(&mut self) -> f64 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 7;
        self.seed ^= self.seed << 17;
        (self.seed >> 11) as f64 / (1u64 << 53) as f64
    }

    /// m+ (e m− com `v` negativo); vale também para o RPN.
    pub fn memory_add(&mut self, v: f64) {
        let m = self.memory.unwrap_or(0.0);
        self.memory = Some(cancel_noise(m + v, m, v));
    }

    /// Põe um valor na tela como resultado (vindo do Programador ou do histórico).
    pub fn load(&mut self, v: f64) {
        if self.error {
            self.reset();
        }
        self.set_value(v);
        self.operand = None;
        self.after_op = false;
    }

    fn constant(&mut self, v: f64, name: &str) {
        self.set_value(v);
        self.operand = Some(name.to_string());
        self.after_op = false;
    }

    /// Número colado (aceita "1.234,5", "1234.5" e "-3"); com um erro na tela, recomeça.
    pub fn paste(&mut self, text: &str) {
        if let Some(v) = super::format::parse(text) {
            self.load(v);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(seq: &[Key]) -> Engine {
        let mut e = Engine::default();
        for k in seq {
            e.press(*k);
        }
        e
    }

    fn keys(text: &str) -> Vec<Key> {
        text.chars()
            .map(|c| match c {
                '0'..='9' => Key::Digit(c as u8 - b'0'),
                ',' => Key::Comma,
                '+' => Key::Bin(Binary::Add),
                '-' => Key::Bin(Binary::Sub),
                '*' => Key::Bin(Binary::Mul),
                '/' => Key::Bin(Binary::Div),
                '^' => Key::Bin(Binary::Pow),
                '(' => Key::Open,
                ')' => Key::Close,
                '=' => Key::Equals,
                '%' => Key::Un(Unary::Percent),
                'n' => Key::Un(Unary::Negate),
                'c' => Key::Clear,
                '<' => Key::Backspace,
                'M' => Key::MemAdd,
                'R' => Key::MemRecall,
                _ => unreachable!("{c}"),
            })
            .collect()
    }

    fn result(text: &str) -> String {
        run(&keys(text)).display()
    }

    #[test]
    fn precedencia_e_parenteses() {
        assert_eq!(result("2+3*4="), "14");
        assert_eq!(result("2*3+4="), "10");
        assert_eq!(result("(2+3)*4="), "20");
        assert_eq!(result("2*(3+4="), "14");
        assert_eq!(result("2^3^2="), "512");
        assert_eq!(result("10-2-3="), "5");
        assert_eq!(result("8/2/2="), "2");
        assert_eq!(result("2+3=="), "8");
        assert_eq!(result("2+*3="), "6");
        // Trocar a operação refaz a precedência: 2 − 3 × vira 2 − 3 −.
        assert_eq!(result("2-3*-4="), "-5");
        // Resultado parcial na tela ao apertar uma operação de precedência igual ou menor.
        assert_eq!(result("2+3*4+"), "14");
        assert_eq!(result("2+3*"), "3");
    }

    #[test]
    fn basico_continua_igual() {
        assert_eq!(result("10/4="), "2,5");
        assert_eq!(result("1/3="), "0,33333333333");
        assert_eq!(result("0,1+0,2="), "0,3");
        assert_eq!(result("100+10%="), "110");
        assert_eq!(result("50%"), "0,5");
        assert_eq!(result("12n"), "-12");
        assert_eq!(result("123<"), "12");
        assert_eq!(result("1/0="), "Erro");
        assert_eq!(result("1/0=c7"), "7");
        assert_eq!(result("1/0=7"), "7");
    }

    #[test]
    fn precisao_inteira_entre_contas() {
        // O resultado da tela (12 algarismos) não é o que entra na conta seguinte.
        assert_eq!(result("1/3=*3="), "1");
        assert_eq!(result("2^0,5=^2="), "2");
        assert_eq!(result("0,1+0,2-0,3="), "0");
        assert_eq!(result("1/3*3-1="), "0");
    }

    #[test]
    fn numero_novo_depois_de_resultados() {
        // Depois de %, de m+ e de um resultado, o algarismo começa outro número.
        assert_eq!(result("100+10%5="), "105");
        assert_eq!(result("5M3"), "3");
        assert_eq!(result("2+3=7"), "7");
        // ± logo depois da operação começa um negativo.
        assert_eq!(result("5*n3="), "-15");
        // Duas operações depois de mr não trocam a de antes.
        let mut e = run(&keys("4M"));
        for k in keys("2+R*3=") {
            e.press(k);
        }
        assert_eq!(e.display(), "14");
    }

    #[test]
    fn limpar_e_apagar() {
        let e = run(&keys("5+3"));
        assert_eq!(e.clear_label(), "C");
        let mut e = run(&keys("5+3c"));
        assert_eq!(e.clear_label(), "AC");
        e.press(Key::Digit(4));
        e.press(Key::Equals);
        assert_eq!(e.display(), "9");
        // Apagar não mexe num resultado.
        assert_eq!(result("2+3=<"), "5");
        assert_eq!(result("5n<"), "0");
    }

    #[test]
    fn funcoes_cientificas() {
        let mut e = run(&keys("30"));
        e.press(Key::Un(Unary::Sin));
        assert_eq!(e.display(), "0,5");
        let mut e = run(&keys("180"));
        e.press(Key::Un(Unary::Sin));
        assert_eq!(e.display(), "0");
        let mut e = run(&keys("90"));
        e.press(Key::Un(Unary::Tan));
        assert_eq!(e.display(), "Erro");
        let mut e = run(&keys("5"));
        e.press(Key::Un(Unary::Fact));
        assert_eq!(e.display(), "120");
        assert!((factorial(0.5) - 0.886_226_925_452_758).abs() < 1e-10);
        let mut e = run(&keys("27"));
        e.press(Key::Bin(Binary::Root));
        e.press(Key::Digit(3));
        e.press(Key::Equals);
        assert_eq!(e.display(), "3");
        let mut e = Engine { radians: true, ..Default::default() };
        e.press(Key::Pi);
        e.press(Key::Un(Unary::Cos));
        assert_eq!(e.display(), "-1");
        // Ângulo minúsculo em radianos: o seno não vira zero.
        assert_eq!(Unary::Sin.apply(1e-20, true), 1e-20);
        let mut e = run(&keys("2"));
        e.press(Key::Bin(Binary::Ee));
        e.press(Key::Digit(3));
        e.press(Key::Equals);
        assert_eq!(e.display(), "2.000");
    }

    #[test]
    fn memoria_e_historico() {
        let mut e = run(&keys("5"));
        e.press(Key::MemAdd);
        e.press(Key::Digit(3));
        e.press(Key::MemAdd);
        e.press(Key::MemRecall);
        assert_eq!(e.display(), "8");
        e.press(Key::MemSub);
        assert_eq!(e.memory, Some(0.0));
        let e = run(&keys("2+3*4=c(1+1)*2="));
        assert_eq!(e.history.len(), 2);
        assert_eq!(e.history[0], Entry { expression: "2 + 3 × 4".into(), result: "14".into(), value: 14.0, bits: None });
        assert_eq!(e.history[1].expression, "(1 + 1) × 2");
        assert_eq!(e.history[1].result, "4");
        // Só uma função e "=": também entra.
        let mut e = run(&keys("9"));
        e.press(Key::Un(Unary::Sqrt));
        e.press(Key::Equals);
        assert_eq!(e.history.last(), Some(&Entry { expression: "√(9)".into(), result: "3".into(), value: 3.0, bits: None }));
    }

    #[test]
    fn memoria_no_meio_da_conta() {
        // m+ e m− no meio da conta não trocam a operação seguinte; mr é um operando.
        assert_eq!(result("2+3M*4="), "14");
        let mut e = run(&keys("2+3"));
        e.press(Key::MemSub);
        for k in keys("*4=") {
            e.press(k);
        }
        assert_eq!(e.display(), "14");
        let mut e = run(&keys("5M"));
        for k in keys("c2+R*3=") {
            e.press(k);
        }
        assert_eq!(e.display(), "17");
        // A operação fica marcada só até o segundo número entrar (por função também).
        let mut e = run(&keys("2+"));
        assert_eq!(e.pending_op(), Some(Binary::Add));
        e.press(Key::Digit(9));
        assert_eq!(e.pending_op(), None);
        let mut e = run(&keys("2*"));
        e.press(Key::Pi);
        assert_eq!(e.pending_op(), None);
    }

    #[test]
    fn resultado_nao_se_edita() {
        // ⌫ não mexe no resultado do %; "=" arruma o número digitado.
        assert_eq!(result("100+10%<="), "110");
        assert_eq!(result("5,="), "5");
        assert_eq!(result("2,50="), "2,5");
        // Colar depois do Erro começa uma conta nova.
        let mut e = run(&keys("1/0*"));
        assert_eq!(e.display(), "Erro");
        e.paste("5");
        e.press(Key::Equals);
        assert_eq!(e.display(), "5");
        // Número grande colado: o próximo algarismo começa outro (não vira 1e157).
        let mut e = Engine::default();
        e.paste("1e15");
        e.press(Key::Digit(7));
        assert_eq!(e.display(), "7");
    }

    #[test]
    fn parenteses_e_repeticao() {
        // "=" fecha os parênteses abertos; a conta seguinte começa do zero.
        let mut e = run(&keys("(5="));
        assert_eq!(e.open_parens(), 0);
        assert_eq!(e.history.last().map(|h| h.expression.as_str()), Some("(5)"));
        for k in keys("*2=") {
            e.press(k);
        }
        assert_eq!(e.display(), "10");
        // Multiplicação implícita antes do "(".
        assert_eq!(result("2(3+4)="), "14");
        assert_eq!(result("(1+1)(2+3)="), "10");
        assert_eq!(result("2+(3)="), "5");
        // "=" repetido depois de fechar pelo "=": repete a operação de fora (× 7).
        assert_eq!(result("2*(3+4=="), "98");
        assert_eq!(result("2*(3+4)=="), "98");
        // Parênteses fechados antes do "=" também vão para o histórico.
        let e = run(&keys("(3+4)="));
        assert_eq!(e.history.last().map(|h| h.expression.as_str()), Some("(3 + 4)"));
        // Texto do histórico: sem a vírgula do fim, com a função antes do "=" repetido.
        let e = run(&keys("2,+3="));
        assert_eq!(e.history[0].expression, "2 + 3");
        let mut e = run(&keys("2+3=9"));
        e.press(Key::Un(Unary::Sqrt));
        e.press(Key::Equals);
        assert_eq!(e.history.last().map(|h| h.expression.as_str()), Some("√(9) + 3"));
        assert_eq!(e.display(), "6");
    }

    #[test]
    fn trigonometria_exata() {
        let deg = |f: Unary, x: f64| f.apply(x, false);
        assert_eq!(deg(Unary::Sin, 1800.0), 0.0);
        assert_eq!(deg(Unary::Sin, 1620.0), 0.0);
        assert_eq!(deg(Unary::Cos, 2790.0), 0.0);
        assert_eq!(deg(Unary::Tan, 1800.0), 0.0);
        assert!(deg(Unary::Tan, 270.0).is_nan());
        assert!(deg(Unary::Tan, -90.0).is_nan());
        assert_eq!(deg(Unary::Sin, -90.0), -1.0);
        assert_eq!(deg(Unary::Cos, 180.0), -1.0);
        assert!((deg(Unary::Sin, 1e-14) - 1.745_329_251_994_33e-16).abs() < 1e-28);
        assert_eq!(Unary::Sin.apply(std::f64::consts::PI, true), 0.0);
        assert_eq!(Unary::Cos.apply(std::f64::consts::FRAC_PI_2, true), 0.0);
        // sen 30° × 2 − 1 = 0 (sem o 1e-16 que sobra do arredondamento).
        let mut e = run(&keys("30"));
        e.press(Key::Un(Unary::Sin));
        for k in keys("*2-1=") {
            e.press(k);
        }
        assert_eq!(e.display(), "0");
        // Fatorial de não inteiro grande ainda cabe no f64.
        assert!((factorial(170.5) / 9.483_368e307 - 1.0).abs() < 1e-6);
        assert!(factorial(141.5).is_finite());
        assert!(!factorial(171.0).is_finite());
    }

    #[test]
    fn expoente_potencia_e_conta_na_tela() {
        // EE no número digitado: o expoente aparece enquanto é digitado.
        let mut e = run(&keys("2"));
        e.press(Key::Bin(Binary::Ee));
        e.press(Key::Digit(3));
        assert_eq!(e.display(), "2e3");
        e.press(Key::Un(Unary::Negate));
        assert_eq!(e.display(), "2e-3");
        e.press(Key::Equals);
        assert_eq!(e.display(), "0,002");
        // ⌫ apaga o expoente e depois o "e".
        let mut e = run(&keys("2"));
        e.press(Key::Bin(Binary::Ee));
        e.press(Key::Digit(5));
        e.press(Key::Backspace);
        e.press(Key::Backspace);
        assert_eq!(e.display(), "2");
        // Depois de um resultado, EE continua sendo a operação: 5 EE 2 = 500.
        let mut e = run(&keys("2+3="));
        e.press(Key::Bin(Binary::Ee));
        e.press(Key::Digit(2));
        e.press(Key::Equals);
        assert_eq!(e.display(), "500");
        // yˣ (2nd): 3 yˣ 2 = 2³ = 8.
        let mut e = run(&keys("3"));
        e.press(Key::Bin(Binary::PowRev));
        e.press(Key::Digit(2));
        e.press(Key::Equals);
        assert_eq!(e.display(), "8");
        // A conta em andamento e, depois do "=", a conta feita.
        assert_eq!(run(&keys("2*(3+4")).expression(), "2 × (3 +");
        assert_eq!(run(&keys("2+3*4=")).expression(), "2 + 3 × 4 =");
        assert_eq!(run(&keys("2+3*4=1")).expression(), "");
    }

    #[test]
    fn conversao_como_funcao() {
        // 2 + (12,5 pol → cm) = 33,75; o próximo algarismo começa outro número.
        let mut e = run(&keys("2+12,5"));
        e.apply(31.75, "12,5 pol → cm");
        e.press(Key::Equals);
        assert_eq!(e.display(), "33,75");
        assert_eq!(e.history.last().map(|h| h.expression.as_str()), Some("2 + (12,5 pol → cm)"));
        e.press(Key::Digit(4));
        assert_eq!(e.display(), "4");
        // Sozinha, também entra no histórico.
        let mut e = run(&keys("100"));
        e.apply(37.5, "100 °F → °C");
        e.press(Key::Equals);
        assert_eq!(e.history.last().map(|h| h.expression.as_str()), Some("100 °F → °C"));
        assert_eq!(e.expression(), "100 °F → °C =");
        // Com Erro, nada acontece.
        let mut e = run(&keys("1/0="));
        e.apply(5.0, "x");
        assert_eq!(e.display(), "Erro");
    }

    #[test]
    fn casas_decimais_e_copiar() {
        let mut e = run(&keys("1234,5+0="));
        assert_eq!(e.display(), "1.234,5");
        assert_eq!(e.copy_text(), "1234,5");
        e.style = Style { decimals: Some(2), grouping: false };
        assert_eq!(e.display(), "1234,50");
        // O número digitado aparece como foi digitado.
        e.press(Key::Digit(7));
        e.press(Key::Comma);
        assert_eq!(e.display(), "7,");
    }
}
