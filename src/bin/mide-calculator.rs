//! Calculadora do mide (E3, E3b): modos Básico, Científico e Programador, notação
//! polonesa reversa (RPN), memória, histórico, casas decimais e desfazer. As contas
//! ficam em `mide::calculator` (escritas do zero); aqui ficam as teclas, o teclado e
//! os menus.
//!
//! Teclado (também o numérico): números, + - * / ^ ( ), Enter ou =, %, Backspace, Esc,
//! Ctrl+C e Ctrl+V; Ctrl+Z desfaz e Ctrl+Shift+Z refaz; Ctrl+1/2/3 trocam o modo,
//! Ctrl+T mostra o histórico; no Programador, A a F, & | ^ ~ % < >.

use std::error::Error;

use mide::{
    calculator::{
        engine::{Binary, Engine, Entry, Key as EKey, Unary},
        format::Style,
        programmer::{self, Base, Programmer},
        rpn::Rpn,
    },
    kit::{run, window_menu, App, MenuGroup, MenuItem, Rgba, Ui, Window},
    settings_file::{self, Value},
    ui::{
        draw::fill_rounded,
        text::{FontWeight, Text},
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Basic,
    Scientific,
    Programmer,
}

impl Mode {
    fn key(self) -> &'static str {
        match self {
            Mode::Basic => "basica",
            Mode::Scientific => "cientifica",
            Mode::Programmer => "programador",
        }
    }

    fn from_key(key: &str) -> Self {
        match key {
            "cientifica" => Mode::Scientific,
            "programador" => Mode::Programmer,
            _ => Mode::Basic,
        }
    }

    /// Tamanho da janela (sem o histórico) e o mínimo.
    fn size(self) -> ((u32, u32), (u32, u32)) {
        match self {
            Mode::Basic => ((250, 380), (220, 320)),
            Mode::Scientific => ((620, 380), (560, 320)),
            Mode::Programmer => ((520, 560), (480, 500)),
        }
    }
}

/// Uma tecla na tela.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Btn {
    K(EKey),
    Second,
    Angle,
    /// RPN: Enter, troca X e Y, rolar a pilha para baixo e para cima, último X.
    Enter,
    Swap,
    Roll,
    RollUp,
    LastX,
    P(programmer::Key),
}

/// Tipo de tecla (cor).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Number,
    Function,
    Operation,
    Science,
}

const BACKGROUND: Rgba = [36, 36, 40, 255];
const NUMBER: Rgba = [78, 78, 84, 255];
const FUNCTION: Rgba = [150, 150, 158, 255];
const SCIENCE: Rgba = [56, 56, 62, 255];
const OPERATION: Rgba = [10, 110, 235, 255];
const PAD: f32 = 10.0;
const GAP: f32 = 8.0;
const DISPLAY_H: f32 = 86.0;
const HISTORY_W: f32 = 230.0;
/// Fileira a mais do Básico com RPN (x↔y, R↓, R↑ e LASTx): altura de uma tecla e o vão.
const RPN_ROW: u32 = 58;

/// Casas decimais oferecidas no menu Visualizar (`None`: automático).
const DECIMALS: [Option<u8>; 10] = [None, Some(0), Some(1), Some(2), Some(3), Some(4), Some(6), Some(8), Some(10), Some(15)];

/// Passos guardados para desfazer.
const UNDO_STEPS: usize = 100;

/// Estado das contas, para desfazer (Ctrl+Z) e refazer (Ctrl+Shift+Z).
#[derive(Clone, PartialEq)]
struct Snapshot {
    engine: Engine,
    programmer: Programmer,
    rpn: Rpn,
}

struct Calculator {
    mode: Mode,
    engine: Engine,
    programmer: Programmer,
    rpn: Rpn,
    rpn_on: bool,
    second: bool,
    history_shown: bool,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    /// Rolagem do histórico (pontos acima da conta mais nova) e o tamanho dele quando
    /// rolou (uma conta nova volta para o fim).
    history_scroll: (f32, usize),
    /// Última tela registrada no log (para os testes acompanharem as contas).
    logged: String,
}

fn op(o: Binary) -> Btn {
    Btn::K(EKey::Bin(o))
}

fn un(u: Unary) -> Btn {
    Btn::K(EKey::Un(u))
}

fn digit(d: u8) -> Btn {
    Btn::K(EKey::Digit(d))
}

impl Calculator {
    fn new() -> Self {
        let mode = match settings_file::get("calculator", "mode") {
            Some(Value::Text(t)) => Mode::from_key(&t),
            _ => Mode::Basic,
        };
        let flag = |key: &str| matches!(settings_file::get("calculator", key), Some(Value::Bool(true)));
        let style = Style {
            decimals: match settings_file::get("calculator", "decimals") {
                Some(Value::Int(n)) => u8::try_from(n).ok().filter(|n| *n <= 15),
                _ => None,
            },
            grouping: !matches!(settings_file::get("calculator", "thousands"), Some(Value::Bool(false))),
        };
        let radians = flag("radians");
        // A base do Programador fica lembrada (HEX, OCT ou BIN; sem a opção, DEC).
        let mut programmer = Programmer::default();
        if let Some(Value::Text(base)) = settings_file::get("calculator", "base") {
            programmer.set_base(match base.as_str() {
                "hex" => Base::Hex,
                "oct" => Base::Oct,
                "bin" => Base::Bin,
                _ => Base::Dec,
            });
        }
        Self {
            mode,
            engine: Engine::new(radians, style),
            programmer,
            rpn: Rpn::new(radians, style),
            rpn_on: flag("rpn"),
            second: false,
            history_shown: flag("history"),
            undo: Vec::new(),
            redo: Vec::new(),
            history_scroll: (0.0, 0),
            logged: String::new(),
        }
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot { engine: self.engine.clone(), programmer: self.programmer.clone(), rpn: self.rpn.clone() }
    }

    /// Volta a um estado guardado. O histórico, Rad/Grau e as casas decimais são do
    /// momento (desfazer volta a conta, não as opções nem a fita).
    fn restore(&mut self, s: Snapshot) {
        let history = std::mem::take(&mut self.engine.history);
        let (radians, style, base) = (self.engine.radians, self.engine.style, self.programmer.base);
        (self.engine, self.rpn, self.programmer) = (s.engine, s.rpn, s.programmer);
        self.engine.history = history;
        (self.engine.radians, self.engine.style) = (radians, style);
        (self.rpn.radians, self.rpn.style) = (radians, style);
        self.programmer.base = base;
    }

    /// Muda as contas guardando o estado de antes (se algo mudou) para desfazer.
    fn change(&mut self, f: impl FnOnce(&mut Self)) {
        let before = self.snapshot();
        f(self);
        if self.snapshot() != before {
            self.undo.push(before);
            if self.undo.len() > UNDO_STEPS {
                self.undo.remove(0);
            }
            self.redo.clear();
        }
    }

    fn undo(&mut self) {
        if let Some(s) = self.undo.pop() {
            self.redo.push(self.snapshot());
            self.restore(s);
        }
    }

    fn redo(&mut self) {
        if let Some(s) = self.redo.pop() {
            self.undo.push(self.snapshot());
            self.restore(s);
        }
    }

    fn set_style(&mut self, style: Style) {
        self.engine.style = style;
        self.rpn.style = style;
        self.save("decimals", style.decimals.map(|d| Value::Int(d.into())));
        self.save("thousands", (!style.grouping).then_some(Value::Bool(false)));
    }

    fn save(&self, key: &str, value: Option<Value>) {
        if let Err(err) = settings_file::set("calculator", key, value) {
            tracing::warn!("calculadora: opção não gravada: {err}");
        }
    }

    fn set_mode(&mut self, ui: &mut Ui, mode: Mode) {
        // O modo em que já está: nada muda (nem o tamanho que a pessoa escolheu).
        if mode == self.mode {
            return;
        }
        // O número da tela passa de um modo para o outro.
        if mode == Mode::Programmer && self.mode != Mode::Programmer {
            self.programmer.load(self.value());
        } else if mode != Mode::Programmer && self.mode == Mode::Programmer {
            let v = self.programmer.value() as i64 as f64;
            if self.rpn_on {
                self.rpn.push(v);
            } else {
                self.engine.load(v);
            }
        }
        self.mode = mode;
        tracing::info!("calculadora: modo {}", mode.key());
        self.save("mode", (mode != Mode::Basic).then(|| Value::Text(mode.key().into())));
        self.resize(ui);
    }

    /// Tamanho da janela no modo atual (com o histórico, se aparece) e o mínimo.
    fn window_size(&self) -> ((u32, u32), (u32, u32)) {
        let ((w, h), (mw, mh)) = self.mode.size();
        let extra = if self.history_shown { HISTORY_W as u32 } else { 0 };
        let rows = if self.mode == Mode::Basic && self.rpn_on { RPN_ROW } else { 0 };
        ((w + extra, h + rows), (mw + extra, mh + rows))
    }

    fn resize(&self, ui: &mut Ui) {
        let (size, min) = self.window_size();
        ui.request_size(size, min);
    }

    fn value(&self) -> f64 {
        if self.rpn_on { self.rpn.x() } else { self.engine.value() }
    }

    fn display(&self) -> String {
        match (self.mode, self.rpn_on) {
            (Mode::Programmer, _) => self.programmer.display(),
            (_, true) => self.rpn.display(),
            _ => self.engine.display(),
        }
    }

    /// O número da tela para copiar (sem pontos de milhar nem espaços).
    fn copy_text(&self) -> String {
        match (self.mode, self.rpn_on) {
            (Mode::Programmer, _) => self.programmer.copy_text(),
            (_, true) => self.rpn.copy_text(),
            _ => self.engine.copy_text(),
        }
    }

    /// Um número colado ou trazido do histórico, no modo atual (`bits`: o inteiro exato
    /// de uma conta do Programador).
    fn put(&mut self, v: f64, bits: Option<u64>) {
        self.change(|c| match (c.mode, c.rpn_on) {
            (Mode::Programmer, _) => match bits {
                Some(bits) => c.programmer.load_bits(bits),
                None => c.programmer.load(v),
            },
            (_, true) => c.rpn.push(v),
            _ => c.engine.load(v),
        });
    }

    fn paste(&mut self, text: &str) {
        if self.mode == Mode::Programmer {
            self.change(|c| c.programmer.paste(text));
        } else if let Some(v) = mide::calculator::format::parse(text) {
            self.put(v, None);
        }
    }

    /// Uma tecla apertada (na tela ou pelo teclado).
    fn press(&mut self, btn: Btn) {
        match btn {
            Btn::Second => self.second = !self.second,
            Btn::Angle => {
                self.engine.radians = !self.engine.radians;
                self.rpn.radians = self.engine.radians;
                self.save("radians", self.engine.radians.then_some(Value::Bool(true)));
            }
            _ => self.change(|c| c.press_key(btn)),
        }
        // A tecla 2nd vale para uma função só.
        if matches!(btn, Btn::K(EKey::Un(_)) | Btn::K(EKey::Bin(Binary::LogBase | Binary::PowRev)) | Btn::RollUp | Btn::LastX) {
            self.second = false;
        }
    }

    fn press_key(&mut self, btn: Btn) {
        match btn {
            Btn::Second | Btn::Angle => {}
            Btn::P(k) => {
                self.programmer.press(k);
                // As contas do Programador também vão para o histórico ("0xFF AND 0xF").
                if let Some((expression, bits)) = self.programmer.take_record() {
                    let base = self.programmer.base;
                    let entry = Entry { expression, result: programmer::format_prefixed(bits, base), value: bits as i64 as f64, bits: Some(bits) };
                    self.engine.record(entry);
                }
            }
            Btn::Enter => self.rpn.enter(),
            Btn::Swap => self.rpn.swap(),
            Btn::Roll => self.rpn.roll_down(),
            Btn::RollUp => self.rpn.roll_up(),
            Btn::LastX => self.rpn.recall_last_x(),
            Btn::K(key) if self.rpn_on => match key {
                EKey::Digit(d) => self.rpn.digit(d),
                EKey::Comma => self.rpn.comma(),
                // EE faz parte do número digitado (2 EE 3 é 2.000).
                EKey::Bin(Binary::Ee) => self.rpn.exponent(),
                // As contas do RPN também vão para o histórico ("9 + 1 = 10").
                EKey::Bin(o) => {
                    let (x, y) = (self.rpn.x(), self.rpn.stack[1]);
                    if self.rpn.binary(o) {
                        let s = self.engine.style;
                        self.record_rpn(format!("{} {} {}", s.value(y), o.symbol(), s.value(x)));
                    }
                }
                EKey::Un(u) => {
                    let (x, y) = (self.rpn.x(), self.rpn.stack[1]);
                    if self.rpn.unary(u) {
                        let s = self.engine.style;
                        self.record_rpn(match u {
                            Unary::Percent => format!("{} × {}%", s.value(y), s.value(x)),
                            _ => format!("{}({})", u.name(), s.value(x)),
                        });
                    }
                }
                EKey::Equals => self.rpn.enter(),
                EKey::Clear => self.rpn.clear(),
                EKey::Backspace => self.rpn.backspace(),
                EKey::Pi => self.rpn.push(std::f64::consts::PI),
                EKey::E => self.rpn.push(std::f64::consts::E),
                EKey::Rand => {
                    let r = self.engine.random();
                    self.rpn.push(r);
                }
                // A memória é a mesma dos dois jeitos de contar.
                EKey::MemClear => self.engine.memory = None,
                EKey::MemAdd | EKey::MemSub => {
                    let x = self.rpn.x();
                    self.engine.memory_add(if key == EKey::MemAdd { x } else { -x });
                    self.rpn.end_entry();
                }
                EKey::MemRecall => {
                    if let Some(m) = self.engine.memory {
                        self.rpn.push(m);
                    }
                }
                // Parênteses não existem no RPN (o teclado ainda manda).
                EKey::Open | EKey::Close => {}
            },
            Btn::K(key) => self.engine.press(key),
        }
    }

    /// Conta do RPN no histórico, com o resultado em X.
    fn record_rpn(&mut self, expression: String) {
        let entry = Entry { expression, result: self.rpn.display(), value: self.rpn.x(), bits: None };
        self.engine.record(entry);
    }

    /// Teclas do modo atual: linhas de (tecla, rótulo, tipo); `None` deixa o espaço
    /// para a tecla larga da esquerda.
    fn grid(&self) -> Vec<Vec<Option<(Btn, String, Kind)>>> {
        let clear = if self.rpn_on { self.rpn.clear_label() } else { self.engine.clear_label() };
        let s = self.second;
        let k = |b: Btn, l: &str, kind: Kind| Some((b, l.to_string(), kind));
        let equals = if self.rpn_on { k(Btn::Enter, "↩", Kind::Operation) } else { k(Btn::K(EKey::Equals), "=", Kind::Operation) };
        let basic: Vec<Vec<Option<(Btn, String, Kind)>>> = vec![
            vec![k(Btn::K(EKey::Clear), clear, Kind::Function), k(un(Unary::Negate), "±", Kind::Function), k(un(Unary::Percent), "%", Kind::Function), k(op(Binary::Div), "÷", Kind::Operation)],
            vec![k(digit(7), "7", Kind::Number), k(digit(8), "8", Kind::Number), k(digit(9), "9", Kind::Number), k(op(Binary::Mul), "×", Kind::Operation)],
            vec![k(digit(4), "4", Kind::Number), k(digit(5), "5", Kind::Number), k(digit(6), "6", Kind::Number), k(op(Binary::Sub), "−", Kind::Operation)],
            vec![k(digit(1), "1", Kind::Number), k(digit(2), "2", Kind::Number), k(digit(3), "3", Kind::Number), k(op(Binary::Add), "+", Kind::Operation)],
            vec![k(digit(0), "0", Kind::Number), None, k(Btn::K(EKey::Comma), ",", Kind::Number), equals],
        ];
        match self.mode {
            // Com RPN, uma fileira a mais com as teclas da pilha.
            Mode::Basic if self.rpn_on => {
                let sc = Kind::Science;
                let mut rows = vec![vec![k(Btn::Swap, "x↔y", sc), k(Btn::Roll, "R↓", sc), k(Btn::RollUp, "R↑", sc), k(Btn::LastX, "LASTx", sc)]];
                rows.extend(basic);
                rows
            }
            Mode::Basic => basic,
            Mode::Scientific => {
                let sc = Kind::Science;
                // No RPN, ( e ) viram as teclas da pilha (com 2nd, LASTx e R↑).
                let (open, close) = match (self.rpn_on, s) {
                    (true, false) => (k(Btn::Swap, "x↔y", sc), k(Btn::Roll, "R↓", sc)),
                    (true, true) => (k(Btn::LastX, "LASTx", sc), k(Btn::RollUp, "R↑", sc)),
                    (false, _) => (k(Btn::K(EKey::Open), "(", sc), k(Btn::K(EKey::Close), ")", sc)),
                };
                let left: Vec<Vec<Option<(Btn, String, Kind)>>> = vec![
                    vec![open, close, k(Btn::K(EKey::MemClear), "mc", sc), k(Btn::K(EKey::MemAdd), "m+", sc), k(Btn::K(EKey::MemSub), "m−", sc), k(Btn::K(EKey::MemRecall), "mr", sc)],
                    vec![
                        k(Btn::Second, "2nd", sc),
                        k(un(Unary::Square), "x²", sc),
                        k(un(Unary::Cube), "x³", sc),
                        k(op(Binary::Pow), "xʸ", sc),
                        if s { k(op(Binary::PowRev), "yˣ", sc) } else { k(un(Unary::Exp), "eˣ", sc) },
                        if s { k(un(Unary::Exp2), "2ˣ", sc) } else { k(un(Unary::Exp10), "10ˣ", sc) },
                    ],
                    vec![
                        k(un(Unary::Recip), "1/x", sc),
                        k(un(Unary::Sqrt), "√x", sc),
                        k(un(Unary::Cbrt), "∛x", sc),
                        k(op(Binary::Root), "ʸ√x", sc),
                        if s { k(op(Binary::LogBase), "logᵧ", sc) } else { k(un(Unary::Ln), "ln", sc) },
                        if s { k(un(Unary::Log2), "log₂", sc) } else { k(un(Unary::Log10), "log₁₀", sc) },
                    ],
                    vec![
                        k(un(Unary::Fact), "x!", sc),
                        if s { k(un(Unary::Asin), "sen⁻¹", sc) } else { k(un(Unary::Sin), "sen", sc) },
                        if s { k(un(Unary::Acos), "cos⁻¹", sc) } else { k(un(Unary::Cos), "cos", sc) },
                        if s { k(un(Unary::Atan), "tg⁻¹", sc) } else { k(un(Unary::Tan), "tg", sc) },
                        k(Btn::K(EKey::E), "e", sc),
                        k(op(Binary::Ee), "EE", sc),
                    ],
                    vec![
                        k(Btn::Angle, if self.engine.radians { "Grau" } else { "Rad" }, sc),
                        if s { k(un(Unary::Asinh), "senh⁻¹", sc) } else { k(un(Unary::Sinh), "senh", sc) },
                        if s { k(un(Unary::Acosh), "cosh⁻¹", sc) } else { k(un(Unary::Cosh), "cosh", sc) },
                        if s { k(un(Unary::Atanh), "tgh⁻¹", sc) } else { k(un(Unary::Tanh), "tgh", sc) },
                        k(Btn::K(EKey::Pi), "π", sc),
                        k(Btn::K(EKey::Rand), "Rand", sc),
                    ],
                ];
                left.into_iter().zip(basic).map(|(mut l, b)| {
                    l.extend(b);
                    l
                }).collect()
            }
            Mode::Programmer => {
                use programmer::{Key as P, Op as O, Unary as U};
                let p = |key: P, l: &str, kind: Kind| k(Btn::P(key), l, kind);
                let hex = |d: u8| p(P::Digit(d), &format!("{d:X}"), Kind::Number);
                let f = Kind::Function;
                let o = Kind::Operation;
                vec![
                    vec![p(P::Op(O::And), "AND", f), p(P::Op(O::Or), "OR", f), hex(0xD), hex(0xE), hex(0xF), p(P::Clear, self.programmer.clear_label(), f)],
                    vec![p(P::Op(O::Nor), "NOR", f), p(P::Op(O::Xor), "XOR", f), hex(0xA), hex(0xB), hex(0xC), p(P::Op(O::Div), "÷", o)],
                    vec![p(P::Un(U::Shl1), "<<", f), p(P::Un(U::Shr1), ">>", f), hex(7), hex(8), hex(9), p(P::Op(O::Mul), "×", o)],
                    vec![p(P::Op(O::Shl), "X<<Y", f), p(P::Op(O::Shr), "X>>Y", f), hex(4), hex(5), hex(6), p(P::Op(O::Sub), "−", o)],
                    vec![p(P::Un(U::RotL), "RoL", f), p(P::Un(U::RotR), "RoR", f), hex(1), hex(2), hex(3), p(P::Op(O::Add), "+", o)],
                    vec![p(P::Un(U::ByteFlip), "byte⇄", f), p(P::Un(U::Not), "~", f), p(P::Un(U::Negate), "±", f), hex(0), p(P::Op(O::Mod), "mod", f), p(P::Equals, "=", o)],
                ]
            }
        }
    }

    fn keyboard(&mut self, ui: &mut Ui) {
        let input = ui.input().clone();
        // Ctrl+1/2/3: modos; Ctrl+T: histórico; Ctrl+R: RPN.
        for (key, mode) in [("1", Mode::Basic), ("2", Mode::Scientific), ("3", Mode::Programmer)] {
            if input.shortcut(key) {
                self.set_mode(ui, mode);
            }
        }
        if input.shortcut("t") {
            self.toggle_history(ui);
        }
        if input.shortcut("r") {
            self.toggle_rpn(ui);
        }
        if input.shortcut("c") {
            ui.copy(&self.copy_text());
        }
        if input.shortcut("v") {
            ui.request_paste();
        }
        match input.shortcut_shift("z") {
            Some(false) => self.undo(),
            Some(true) => self.redo(),
            None => {}
        }
        if let Some(text) = &input.paste {
            self.paste(text);
        }
        // Teclas na ordem em que foram apertadas (texto e teclas especiais misturados:
        // "1 2 ⌫ 3" dá 13 mesmo chegando no mesmo quadro). Com Ctrl, Alt ou ⌘ são atalhos.
        for press in &input.presses {
            if press.ctrl || press.alt || press.logo {
                continue;
            }
            if let Some(btn) = self.special_button(press.keysym) {
                self.press(btn);
            } else if let Some(text) = &press.text {
                for c in text.chars() {
                    if let Some(btn) = self.char_button(c) {
                        self.press(btn);
                    }
                }
            }
        }
    }

    /// Enter, Backspace, Esc, Delete e Tab (pelo keysym, como no Terminal).
    fn special_button(&self, keysym: u32) -> Option<Btn> {
        let programmer = self.mode == Mode::Programmer;
        Some(match keysym {
            // Return e Enter do teclado numérico.
            0xff0d | 0xff8d if programmer => Btn::P(programmer::Key::Equals),
            0xff0d | 0xff8d => Btn::K(EKey::Equals),
            0xff08 if programmer => Btn::P(programmer::Key::Backspace),
            0xff08 => Btn::K(EKey::Backspace),
            // Esc, Delete e o Delete do teclado numérico.
            0xff1b | 0xffff | 0xff9f if programmer => Btn::P(programmer::Key::Clear),
            0xff1b | 0xffff | 0xff9f => Btn::K(EKey::Clear),
            // Tab no RPN: x↔y.
            0xff09 if self.rpn_on && !programmer => Btn::Swap,
            // F9: ±.
            0xffc6 if programmer => Btn::P(programmer::Key::Un(programmer::Unary::Negate)),
            0xffc6 => un(Unary::Negate),
            _ => return None,
        })
    }

    /// Um caractere digitado (também do teclado numérico).
    fn char_button(&self, c: char) -> Option<Btn> {
        if self.mode == Mode::Programmer {
            use programmer::{Key as P, Op as O};
            return match c {
                '0'..='9' | 'a'..='f' | 'A'..='F' => c.to_digit(16).map(|d| Btn::P(P::Digit(d as u8))),
                '+' => Some(Btn::P(P::Op(O::Add))),
                '-' => Some(Btn::P(P::Op(O::Sub))),
                '*' => Some(Btn::P(P::Op(O::Mul))),
                '/' => Some(Btn::P(P::Op(O::Div))),
                '&' => Some(Btn::P(P::Op(O::And))),
                '|' => Some(Btn::P(P::Op(O::Or))),
                '^' => Some(Btn::P(P::Op(O::Xor))),
                '%' => Some(Btn::P(P::Op(O::Mod))),
                '<' => Some(Btn::P(P::Op(O::Shl))),
                '>' => Some(Btn::P(P::Op(O::Shr))),
                '~' => Some(Btn::P(P::Un(programmer::Unary::Not))),
                '=' => Some(Btn::P(P::Equals)),
                _ => None,
            };
        }
        match c {
            '0'..='9' => Some(digit(c as u8 - b'0')),
            ',' | '.' => Some(Btn::K(EKey::Comma)),
            '+' => Some(op(Binary::Add)),
            '-' => Some(op(Binary::Sub)),
            '*' | 'x' | '×' => Some(op(Binary::Mul)),
            '/' | '÷' => Some(op(Binary::Div)),
            '^' => Some(op(Binary::Pow)),
            // No RPN, ( e ) são x↔y e R↓ (as teclas que ficam no lugar deles).
            '(' if self.rpn_on => Some(Btn::Swap),
            ')' if self.rpn_on => Some(Btn::Roll),
            '(' => Some(Btn::K(EKey::Open)),
            ')' => Some(Btn::K(EKey::Close)),
            '=' => Some(Btn::K(EKey::Equals)),
            '%' => Some(un(Unary::Percent)),
            // Letras para as funções do Científico (no Básico, só o x, que é ×).
            '!' if self.mode == Mode::Scientific => Some(un(Unary::Fact)),
            'p' if self.mode == Mode::Scientific => Some(Btn::K(EKey::Pi)),
            'e' if self.mode == Mode::Scientific => Some(Btn::K(EKey::E)),
            's' if self.mode == Mode::Scientific => Some(un(Unary::Sin)),
            'c' if self.mode == Mode::Scientific => Some(un(Unary::Cos)),
            't' if self.mode == Mode::Scientific => Some(un(Unary::Tan)),
            'n' if self.mode == Mode::Scientific => Some(un(Unary::Ln)),
            'l' if self.mode == Mode::Scientific => Some(un(Unary::Log10)),
            'r' if self.mode == Mode::Scientific => Some(un(Unary::Recip)),
            'q' if self.mode == Mode::Scientific => Some(un(Unary::Sqrt)),
            _ => None,
        }
    }

    /// Liga ou desliga o RPN, e o número da tela vai junto. No Básico, a fileira das
    /// teclas da pilha entra ou sai (a janela cresce ou diminui só essa altura).
    fn toggle_rpn(&mut self, ui: &mut Ui) {
        if self.mode == Mode::Programmer {
            return;
        }
        self.rpn_on = !self.rpn_on;
        if self.rpn_on {
            if !self.engine.has_error() {
                let v = self.engine.value();
                self.rpn.push(v);
            }
        } else {
            let v = self.rpn.x();
            self.engine.load(v);
        }
        tracing::info!("calculadora: RPN {}", if self.rpn_on { "ligada" } else { "desligada" });
        self.save("rpn", self.rpn_on.then_some(Value::Bool(true)));
        if self.mode == Mode::Basic {
            let (w, h) = ui.window_size();
            let h = if self.rpn_on { h + RPN_ROW as f32 } else { h - RPN_ROW as f32 };
            let (_, min) = self.window_size();
            ui.request_size(((w.round() as u32).max(min.0), (h.round() as u32).max(min.1)), min);
        }
    }

    /// Mostra ou esconde o histórico mantendo o tamanho que a pessoa escolheu: só a
    /// largura do painel entra ou sai.
    fn toggle_history(&mut self, ui: &mut Ui) {
        self.history_shown = !self.history_shown;
        self.save("history", self.history_shown.then_some(Value::Bool(true)));
        let (w, h) = ui.window_size();
        let w = if self.history_shown { w + HISTORY_W } else { w - HISTORY_W };
        let (_, min) = self.window_size();
        ui.request_size(((w.round() as u32).max(min.0), (h.round() as u32).max(min.1)), min);
    }

    fn menu_actions(&mut self, ui: &mut Ui) {
        for action in ui.input().menu.clone() {
            match action.as_str() {
                "basica" => self.set_mode(ui, Mode::Basic),
                "cientifica" => self.set_mode(ui, Mode::Scientific),
                "programador" => self.set_mode(ui, Mode::Programmer),
                "rpn" => self.toggle_rpn(ui),
                "historico" => self.toggle_history(ui),
                "limpar-historico" => self.engine.history.clear(),
                "copiar-historico" => {
                    let text: Vec<String> = self.engine.history.iter().map(|e| format!("{} = {}", e.expression, e.result)).collect();
                    if !text.is_empty() {
                        ui.copy(&text.join("\n"));
                    }
                }
                "copiar" => ui.copy(&self.copy_text()),
                "colar" => ui.request_paste(),
                "desfazer" => self.undo(),
                "refazer" => self.redo(),
                "milhar" => {
                    let style = Style { grouping: !self.engine.style.grouping, ..self.engine.style };
                    self.set_style(style);
                }
                other => {
                    if let Some(d) = other.strip_prefix("casas:") {
                        let style = Style { decimals: d.parse().ok(), ..self.engine.style };
                        self.set_style(style);
                    }
                }
            }
        }
    }
}

impl App for Calculator {
    fn view(&mut self, ui: &mut Ui) {
        self.menu_actions(ui);
        self.keyboard(ui);
        let (_, _, total_w, h) = ui.region();
        let history = self.history_shown;
        let w = if history { total_w - HISTORY_W } else { total_w };
        let grid = self.grid();
        let cols = grid.iter().map(Vec::len).max().unwrap_or(4) as f32;
        let rows = grid.len() as f32;
        // Programador: a tela e os 64 bits em cima.
        let top = if self.mode == Mode::Programmer { DISPLAY_H + 112.0 } else { DISPLAY_H };
        let bw = (w - 2.0 * PAD - (cols - 1.0) * GAP) / cols;
        let bh = ((h - top - PAD - (rows - 1.0) * GAP) / rows).max(20.0);
        let mut pressed = None;
        let mut keys = Vec::new();
        for (row, line) in grid.iter().enumerate() {
            for (col, cell) in line.iter().enumerate() {
                let Some((btn, label, kind)) = cell.clone() else {
                    continue;
                };
                // A tecla da esquerda de um espaço vazio ocupa os dois (o 0).
                let wide = line.get(col + 1).is_some_and(Option::is_none);
                let width = if wide { 2.0 * bw + GAP } else { bw };
                let rect = (PAD + col as f32 * (bw + GAP), top + row as f32 * (bh + GAP), width, bh);
                let enabled = match btn {
                    Btn::P(programmer::Key::Digit(d)) => self.programmer.accepts(d),
                    _ => true,
                };
                let r = ui.background(rect);
                if r.clicked && enabled {
                    pressed = Some(btn);
                }
                let held = r.hovered && ui.input().down;
                keys.push((btn, label, kind, rect, held, enabled, wide));
            }
        }
        if let Some(b) = pressed {
            self.press(b);
        }
        // ⌫ na tela enquanto um número é digitado (para quem usa o mouse ou o toque):
        // no canto esquerdo da tela do número; no Programador, à direita das bases.
        let typing = match (self.mode, self.rpn_on) {
            (Mode::Programmer, _) => self.programmer.clear_label() == "C",
            (_, true) => self.rpn.typing(),
            _ => self.engine.clear_label() == "C",
        };
        let erase = if self.mode == Mode::Programmer { (w - PAD - 30.0, 6.0, 30.0, 24.0) } else { (PAD, DISPLAY_H - GAP - 30.0, 30.0, 24.0) };
        if typing && ui.background(erase).clicked {
            self.press(if self.mode == Mode::Programmer { Btn::P(programmer::Key::Backspace) } else { Btn::K(EKey::Backspace) });
        }

        // Programador: a base escolhida e os bits clicáveis.
        let mut bit_rects = Vec::new();
        let mut base_rects = Vec::new();
        if self.mode == Mode::Programmer {
            for (i, base) in [Base::Hex, Base::Dec, Base::Oct, Base::Bin].into_iter().enumerate() {
                let rect = (PAD + i as f32 * 58.0, 8.0, 52.0, 22.0);
                if ui.background(rect).clicked {
                    self.programmer.set_base(base);
                    self.save("base", (base != Base::Dec).then(|| Value::Text(base.label().to_lowercase())));
                }
                base_rects.push((base, rect));
            }
            let bit_w = (w - 2.0 * PAD) / 32.0;
            for n in 0..64u32 {
                let (row, col) = ((63 - n) / 32, (63 - n) % 32);
                let rect = (PAD + col as f32 * bit_w, DISPLAY_H + 14.0 + row as f32 * 44.0, bit_w, 26.0);
                if ui.background(rect).clicked {
                    self.change(|c| c.programmer.toggle_bit(n));
                }
                bit_rects.push((n, rect));
            }
        }

        // Histórico: as contas feitas, a mais nova embaixo; clique traz o resultado. A
        // roda do mouse rola para as mais antigas.
        let mut history_rows = Vec::new();
        if history {
            let row_h = 46.0;
            let visible = ((h - 44.0) / row_h).floor().max(1.0) as usize;
            let total = self.engine.history.len();
            let (mut scroll, seen) = self.history_scroll;
            if seen != total {
                scroll = 0.0;
            }
            let input = ui.input();
            if input.pointer.is_some_and(|(px, _)| px >= w) && input.scroll != 0.0 {
                scroll -= input.scroll;
            }
            scroll = scroll.clamp(0.0, total.saturating_sub(visible) as f32 * row_h);
            self.history_scroll = (scroll, total);
            let skip = (scroll / row_h).round() as usize;
            let entries: Vec<_> = self.engine.history.iter().rev().skip(skip).take(visible).cloned().collect();
            for (i, entry) in entries.iter().enumerate() {
                let rect = (w + 10.0, h - 12.0 - (i + 1) as f32 * row_h, HISTORY_W - 20.0, row_h - 4.0);
                let r = ui.background(rect);
                if r.clicked {
                    self.put(entry.value, entry.bits);
                }
                // Clique direito: copiar a conta ou só o resultado (sem pontos de milhar).
                let style = Style { grouping: false, ..self.engine.style };
                let mut copy = None;
                ui.context_menu(&r, |m| {
                    if m.item("Copiar a conta") {
                        copy = Some(format!("{} = {}", entry.expression, entry.result));
                    }
                    if m.item("Copiar o resultado") {
                        copy = Some(if entry.bits.is_some() { entry.result.clone() } else { style.value(entry.value) });
                    }
                });
                if let Some(text) = copy {
                    ui.copy(&text);
                }
                history_rows.push((entry.clone(), rect));
            }
        }

        let screen = self.display();
        if screen != self.logged {
            tracing::debug!("calculadora: tela {screen}");
            self.logged = screen.clone();
        }
        let pending = if self.rpn_on { None } else { self.engine.pending_op() };
        let pending_bits = self.programmer.pending_op();
        let second = self.second;
        let mode = self.mode;
        let radians = self.engine.radians;
        let memory = self.engine.memory.is_some();
        let rpn_stack = (self.rpn_on && mode != Mode::Programmer).then(|| self.rpn.upper());
        let base = self.programmer.base;
        let value = self.programmer.value();
        let character = self.programmer.character();
        let expression = if self.rpn_on || self.mode == Mode::Programmer { String::new() } else { self.engine.expression() };
        ui.paint_text(|pixmap, text, (ox, oy)| {
            let bg = |c: Rgba| mide::ui::draw::rgba(c[0], c[1], c[2], c[3]);
            fill_rounded(pixmap, ox, oy, total_w, h, 0.0, bg(BACKGROUND));
            // Tela: o número à direita, menor quando não cabe.
            // Com o ⌫ na tela, o número não passa por cima dele.
            let room = if typing && mode != Mode::Programmer { 36.0 } else { 0.0 };
            let (screen, size) = fit_display(text, &screen, w - 2.0 * PAD - 8.0 - room, mode == Mode::Programmer);
            let tw = text.measure(&screen, size, FontWeight::Regular);
            // Pela base: no Programador, logo acima da fileira de bits.
            let display_y = if mode == Mode::Programmer { DISPLAY_H + 8.0 - size * 1.3 } else { DISPLAY_H - GAP - size * 1.3 };
            text.draw(pixmap, &screen, ox + w - PAD - 4.0 - tw, oy + display_y, size, FontWeight::Regular, [255, 255, 255, 255]);
            // Indicadores pequenos à esquerda da tela: Rad, memória, RPN (Y Z T).
            let small = [180, 180, 188, 255];
            // A conta em andamento, pequena, acima do número.
            if !expression.is_empty() {
                let x0 = if mode == Mode::Scientific { PAD + 70.0 } else { PAD + 4.0 };
                let shown = fit_terms(text, &expression, 13.0, w - x0 - PAD - 4.0);
                let ew = text.measure(&shown, 13.0, FontWeight::Regular);
                text.draw(pixmap, &shown, ox + w - PAD - 4.0 - ew, oy + 8.0, 13.0, FontWeight::Regular, small);
            }
            if typing {
                let (x, y, ew, eh) = erase;
                fill_rounded(pixmap, ox + x, oy + y, ew, eh, eh / 2.0, bg(SCIENCE));
                let lw = text.measure("⌫", 14.0, FontWeight::Regular);
                text.draw(pixmap, "⌫", ox + x + (ew - lw) / 2.0, oy + y + (eh - 14.0 * 1.25) / 2.0, 14.0, FontWeight::Regular, [235, 235, 240, 255]);
            }
            if mode == Mode::Scientific {
                let mut tags = vec![if radians { "Rad" } else { "Grau" }];
                if memory {
                    tags.push("M");
                }
                text.draw(pixmap, &tags.join("  "), ox + PAD + 4.0, oy + 10.0, 12.0, FontWeight::Regular, small);
            }
            if let Some([y, z, t]) = &rpn_stack {
                // A pilha acima de X (no Científico, depois de Rad e M); cortada pela
                // esquerda se não cabe (Y é o que mais importa).
                let x0 = if mode == Mode::Scientific { PAD + 70.0 } else { PAD + 4.0 };
                let stack = fit_left(text, &format!("T {t}   Z {z}   Y {y}"), 12.0, w - x0 - PAD);
                text.draw(pixmap, &stack, ox + x0, oy + 10.0, 12.0, FontWeight::Regular, small);
            }
            if mode == Mode::Programmer {
                for (b, (x, y, bw2, bh2)) in &base_rects {
                    let on = *b == base;
                    fill_rounded(pixmap, ox + x, oy + y, *bw2, *bh2, 6.0, bg(if on { OPERATION } else { SCIENCE }));
                    let lw = text.measure(b.label(), 12.0, FontWeight::Semibold);
                    text.draw(pixmap, b.label(), ox + x + (bw2 - lw) / 2.0, oy + y + 4.0, 12.0, FontWeight::Semibold, [255, 255, 255, 255]);
                }
                if !character.is_empty() {
                    text.draw(pixmap, &format!("“{character}”"), ox + PAD + 4.0, oy + 46.0, 14.0, FontWeight::Regular, small);
                }
                for (n, (x, y, bw2, _)) in &bit_rects {
                    let on = value >> n & 1 == 1;
                    let label = if on { "1" } else { "0" };
                    let lw = text.measure(label, 13.0, FontWeight::Regular);
                    text.draw(pixmap, label, ox + x + (bw2 - lw) / 2.0, oy + y, 13.0, FontWeight::Regular, if on { [255, 255, 255, 255] } else { [110, 110, 118, 255] });
                    // Posição a cada 4 bits, embaixo.
                    if n % 4 == 0 {
                        let pos = n.to_string();
                        let pw = text.measure(&pos, 9.0, FontWeight::Regular);
                        text.draw(pixmap, &pos, ox + x + (bw2 - pw) / 2.0, oy + y + 16.0, 9.0, FontWeight::Regular, [110, 110, 118, 255]);
                    }
                }
            }
            for (btn, label, kind, (x, y, bw, bh), held, enabled, wide) in &keys {
                let marked = matches!(btn, Btn::K(EKey::Bin(o)) if pending == Some(*o))
                    || matches!(btn, Btn::P(programmer::Key::Op(o)) if pending_bits == Some(*o))
                    || (*btn == Btn::Second && second);
                let (mut fill, fg) = match kind {
                    _ if marked => ([255, 255, 255, 255], OPERATION),
                    Kind::Operation => (OPERATION, [255, 255, 255, 255]),
                    Kind::Function => (FUNCTION, [20, 20, 24, 255]),
                    Kind::Science => (SCIENCE, [235, 235, 240, 255]),
                    Kind::Number => (NUMBER, [255, 255, 255, 255]),
                };
                if *held {
                    for c in fill.iter_mut().take(3) {
                        *c = (*c as u32 * 4 / 5 + 40).min(255) as u8;
                    }
                }
                let fg = if *enabled { fg } else { [120, 120, 126, 255] };
                let radius = bh.min(*bw) / 2.0;
                fill_rounded(pixmap, ox + x, oy + y, *bw, *bh, radius, bg(if *enabled { fill } else { SCIENCE }));
                let fs = match kind {
                    Kind::Science | Kind::Function if label.chars().count() > 3 => 13.0,
                    Kind::Science => 16.0,
                    Kind::Number => 22.0,
                    _ => 20.0,
                };
                let lw = text.measure(label, fs, FontWeight::Regular);
                // O 0 largo tem o algarismo alinhado com a coluna da esquerda.
                let lx = if *wide { x + (bh.min(*bw) - lw) / 2.0 } else { x + (bw - lw) / 2.0 };
                text.draw(pixmap, label, ox + lx, oy + y + (bh - fs * 1.25) / 2.0, fs, FontWeight::Regular, fg);
            }
            if history {
                fill_rounded(pixmap, ox + w, oy, 1.0, h, 0.0, bg([60, 60, 66, 255]));
                text.draw(pixmap, "Histórico", ox + w + 14.0, oy + 12.0, 13.0, FontWeight::Semibold, small);
                for (entry, (x, y, ew, _)) in &history_rows {
                    let expr = fit_terms(text, &entry.expression, 12.0, *ew);
                    let ew2 = text.measure(&expr, 12.0, FontWeight::Regular);
                    text.draw(pixmap, &expr, ox + x + ew - ew2, oy + y, 12.0, FontWeight::Regular, small);
                    let result = fit_left(text, &format!("= {}", entry.result), 16.0, *ew);
                    let rw = text.measure(&result, 16.0, FontWeight::Regular);
                    text.draw(pixmap, &result, ox + x + ew - rw, oy + y + 18.0, 16.0, FontWeight::Regular, [255, 255, 255, 255]);
                }
            }
        });
    }

    fn menu_bar(&mut self) -> Vec<MenuGroup> {
        let mode = self.mode;
        let style = self.engine.style;
        let has_history = !self.engine.history.is_empty();
        let decimals = DECIMALS
            .iter()
            .map(|d| {
                let (action, label) = match d {
                    Some(n) => (format!("casas:{n}"), n.to_string()),
                    None => ("casas:auto".to_string(), "Automático".to_string()),
                };
                MenuItem::action(&action, &label).checked(style.decimals == *d).enabled(mode != Mode::Programmer)
            })
            .collect();
        vec![
            MenuGroup {
                title: "Editar".into(),
                items: vec![
                    MenuItem::action("desfazer", "Desfazer").shortcut("Ctrl+Z").enabled(!self.undo.is_empty()),
                    MenuItem::action("refazer", "Refazer").shortcut("Ctrl+Shift+Z").enabled(!self.redo.is_empty()),
                    MenuItem::Separator,
                    MenuItem::action("copiar", "Copiar").shortcut("Ctrl+C"),
                    MenuItem::action("colar", "Colar").shortcut("Ctrl+V"),
                    MenuItem::Separator,
                    MenuItem::action("copiar-historico", "Copiar o histórico").enabled(has_history),
                    MenuItem::action("limpar-historico", "Limpar o histórico").enabled(has_history),
                ],
            },
            MenuGroup {
                title: "Visualizar".into(),
                items: vec![
                    MenuItem::action("basica", "Básica").shortcut("Ctrl+1").checked(mode == Mode::Basic),
                    MenuItem::action("cientifica", "Científica").shortcut("Ctrl+2").checked(mode == Mode::Scientific),
                    MenuItem::action("programador", "Programador").shortcut("Ctrl+3").checked(mode == Mode::Programmer),
                    MenuItem::Separator,
                    MenuItem::action("rpn", "Notação polonesa reversa (RPN)").shortcut("Ctrl+R").checked(self.rpn_on).enabled(mode != Mode::Programmer),
                    MenuItem::action("historico", "Mostrar o histórico").shortcut("Ctrl+T").checked(self.history_shown),
                    MenuItem::Separator,
                    MenuItem::action("milhar", "Separador de milhar").checked(style.grouping).enabled(mode != Mode::Programmer),
                    MenuItem::Submenu { label: "Casas decimais".into(), items: decimals },
                ],
            },
            window_menu(Vec::new()),
        ]
    }
}

/// O número da tela e o tamanho da letra para caber em `width`: de 48 até 16 pontos;
/// no Programador (64 bits em binário), até 10, depois sem os espaços entre os grupos
/// e, por fim, cortado pela esquerda (os bits de cima continuam na fileira de bits).
fn fit_display(text: &mut Text, screen: &str, width: f32, small: bool) -> (String, f32) {
    let min = if small { 10.0 } else { 16.0 };
    let candidates = [screen.to_string(), screen.replace(' ', "")];
    for candidate in candidates.iter().take(if small { 2 } else { 1 }) {
        let mut size = 48.0;
        while size > min && text.measure(candidate, size, FontWeight::Regular) > width {
            size -= 2.0;
        }
        if text.measure(candidate, size, FontWeight::Regular) <= width {
            return (candidate.clone(), size);
        }
    }
    let ellipsis = text.measure("…", min, FontWeight::Regular);
    (format!("…{}", fit_left(text, &candidates[usize::from(small)], min, width - ellipsis)), min)
}

/// Corta o começo do texto até caber em `width`, com "…" no lugar do que saiu (o fim
/// de uma conta longa é o que importa).
fn fit_left(text: &mut Text, s: &str, size: f32, width: f32) -> String {
    if text.measure(s, size, FontWeight::Regular) <= width {
        return s.to_string();
    }
    let mut rest: String = s.to_string();
    let mut shown = format!("…{rest}");
    while rest.chars().count() > 1 && text.measure(&shown, size, FontWeight::Regular) > width {
        rest = rest.chars().skip(1).collect();
        shown = format!("…{rest}");
    }
    shown
}

/// Como `fit_left`, mas cortando entre os termos da conta (nos espaços entre número e
/// operação): "…456.789.012 + 1" pareceria outro número; "… + 987 × 2", não.
fn fit_terms(text: &mut Text, s: &str, size: f32, width: f32) -> String {
    if text.measure(s, size, FontWeight::Regular) <= width {
        return s.to_string();
    }
    let mut rest = s;
    while let Some((_, tail)) = rest.split_once(' ') {
        rest = tail;
        let shown = format!("… {rest}");
        if text.measure(&shown, size, FontWeight::Regular) <= width {
            return shown;
        }
    }
    fit_left(text, rest, size, width)
}

fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "mide_calculator=info".into()))
        .init();
    // A janela já nasce no tamanho do modo lembrado.
    let calculator = Calculator::new();
    let ((w, h), (mw, mh)) = calculator.window_size();
    let window = Window::new("Calculadora").app_id("mide-calculator").size(w, h).min_size(mw, mh);
    run(window, calculator)
}
