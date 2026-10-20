//! Calculadora do mide: as quatro operações, porcentagem e troca de sinal, pelo mouse
//! ou pelo teclado (números, + - * /, Enter, Backspace, Esc, Ctrl+C e Ctrl+V).

use std::error::Error;

use mide::{
    kit::{run, App, Key, Rgba, Ui, Window},
    ui::{draw::fill_rounded, text::FontWeight},
};

/// Maior número de algarismos mostrados.
const DIGITS: usize = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Add,
    Sub,
    Mul,
    Div,
}

impl Op {
    fn apply(self, a: f64, b: f64) -> f64 {
        match self {
            Op::Add => a + b,
            Op::Sub => a - b,
            Op::Mul => a * b,
            Op::Div => a / b,
        }
    }
}

/// Uma tecla da calculadora.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Button {
    Digit(u8),
    Comma,
    Op(Op),
    Equals,
    Clear,
    Sign,
    Percent,
    Backspace,
}

/// Estado das contas: o número na tela (como digitado), o acumulado e a operação
/// pendente. `=` repetido refaz a última operação (2 + 3 = = dá 8).
#[derive(Debug, Default)]
struct Calc {
    entry: String,
    acc: Option<f64>,
    op: Option<Op>,
    /// Última operação feita pelo `=`, para repetir.
    last: Option<(Op, f64)>,
    /// O próximo algarismo começa um número novo.
    fresh: bool,
    error: bool,
}

impl Calc {
    fn value(&self) -> f64 {
        self.entry.replace(',', ".").parse().unwrap_or(0.0)
    }

    fn set_value(&mut self, v: f64) {
        if !v.is_finite() {
            self.error = true;
            self.entry.clear();
            return;
        }
        self.entry = to_entry(v);
    }

    /// O que a tela mostra.
    fn display(&self) -> String {
        if self.error {
            return "Erro".into();
        }
        if self.entry.is_empty() {
            return "0".into();
        }
        group_thousands(&self.entry)
    }

    /// "AC" apaga tudo; com um número digitado, "C" apaga só ele.
    fn clear_label(&self) -> &'static str {
        if self.entry.is_empty() || self.fresh { "AC" } else { "C" }
    }

    fn press(&mut self, button: Button) {
        if self.error && button != Button::Clear && !matches!(button, Button::Digit(_)) {
            return;
        }
        if self.error {
            *self = Calc::default();
        }
        match button {
            Button::Digit(d) => {
                if self.fresh {
                    self.entry.clear();
                    self.fresh = false;
                }
                if self.entry.chars().filter(char::is_ascii_digit).count() >= DIGITS {
                    return;
                }
                if self.entry == "0" {
                    self.entry.clear();
                }
                if self.entry == "-0" {
                    self.entry = "-".into();
                }
                self.entry.push((b'0' + d) as char);
            }
            Button::Comma => {
                if self.fresh || self.entry.is_empty() {
                    self.entry = "0".into();
                    self.fresh = false;
                }
                if !self.entry.contains(',') {
                    self.entry.push(',');
                }
            }
            Button::Op(op) => {
                if let (Some(acc), Some(pending), false) = (self.acc, self.op, self.fresh) {
                    let result = pending.apply(acc, self.value());
                    self.set_value(result);
                }
                self.acc = Some(self.value());
                self.op = Some(op);
                self.last = None;
                self.fresh = true;
            }
            Button::Equals => {
                if let (Some(acc), Some(op)) = (self.acc, self.op) {
                    let b = self.value();
                    self.set_value(op.apply(acc, b));
                    self.last = Some((op, b));
                } else if let Some((op, b)) = self.last {
                    let a = self.value();
                    self.set_value(op.apply(a, b));
                }
                self.acc = None;
                self.op = None;
                self.fresh = true;
            }
            Button::Clear => {
                if self.clear_label() == "C" {
                    self.entry.clear();
                } else {
                    *self = Calc::default();
                }
            }
            Button::Sign => {
                if self.entry.is_empty() || self.fresh && self.entry == "0" {
                    self.entry = "-0".into();
                    self.fresh = false;
                } else if let Some(rest) = self.entry.strip_prefix('-') {
                    self.entry = rest.to_string();
                } else {
                    self.entry.insert(0, '-');
                }
            }
            Button::Percent => {
                // Com uma soma ou subtração pendente, é a porcentagem do acumulado
                // (100 + 10% = 110); senão, divide por 100.
                let v = self.value();
                let p = match (self.acc, self.op) {
                    (Some(acc), Some(Op::Add | Op::Sub)) => acc * v / 100.0,
                    _ => v / 100.0,
                };
                self.set_value(p);
                self.fresh = false;
            }
            Button::Backspace => {
                if !self.fresh {
                    self.entry.pop();
                    if self.entry == "-" {
                        self.entry.clear();
                    }
                }
            }
        }
    }

    /// Número colado (aceita "1.234,5", "1234.5" e "-3").
    fn paste(&mut self, text: &str) {
        let text = text.trim();
        let normalized = if text.contains(',') { text.replace('.', "").replace(',', ".") } else { text.to_string() };
        if let Ok(v) = normalized.parse::<f64>() {
            self.error = false;
            self.set_value(v);
            self.fresh = false;
        }
    }
}

/// Número como seria digitado: vírgula decimal, sem zeros sobrando, no máximo
/// `DIGITS` algarismos (notação científica quando não cabe).
fn to_entry(v: f64) -> String {
    if v == 0.0 {
        return "0".into();
    }
    let magnitude = v.abs().log10().floor() as i32;
    if !(-(DIGITS as i32 - 2)..DIGITS as i32).contains(&magnitude) {
        let s = format!("{:.6e}", v);
        let (mantissa, exp) = s.split_once('e').unwrap_or((&s, "0"));
        let mantissa = mantissa.trim_end_matches('0').trim_end_matches('.');
        return format!("{}e{}", mantissa, exp).replace('.', ",");
    }
    let decimals = (DIGITS as i32 - 1 - magnitude.max(0)).max(0) as usize;
    let s = format!("{:.*}", decimals, v);
    let s = if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s };
    if s == "-0" { "0".into() } else { s.replace('.', ",") }
}

/// Pontos de milhar na parte inteira ("1234567,5" vira "1.234.567,5").
fn group_thousands(entry: &str) -> String {
    if entry.contains('e') {
        return entry.to_string();
    }
    let (sign, rest) = entry.strip_prefix('-').map_or(("", entry), |r| ("-", r));
    let (int, frac) = rest.split_once(',').map_or((rest, None), |(i, f)| (i, Some(f)));
    let mut grouped = String::new();
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            grouped.push('.');
        }
        grouped.push(c);
    }
    match frac {
        Some(f) => format!("{sign}{grouped},{f}"),
        None => format!("{sign}{grouped}"),
    }
}

/// Teclas na ordem da grade (5 linhas de 4; o 0 ocupa duas colunas).
const GRID: [[Option<Button>; 4]; 5] = [
    [Some(Button::Clear), Some(Button::Sign), Some(Button::Percent), Some(Button::Op(Op::Div))],
    [Some(Button::Digit(7)), Some(Button::Digit(8)), Some(Button::Digit(9)), Some(Button::Op(Op::Mul))],
    [Some(Button::Digit(4)), Some(Button::Digit(5)), Some(Button::Digit(6)), Some(Button::Op(Op::Sub))],
    [Some(Button::Digit(1)), Some(Button::Digit(2)), Some(Button::Digit(3)), Some(Button::Op(Op::Add))],
    [Some(Button::Digit(0)), None, Some(Button::Comma), Some(Button::Equals)],
];

/// Cores: fundo escuro, números em cinza, funções em cinza claro e operações na cor
/// de destaque do mide.
const BACKGROUND: Rgba = [36, 36, 40, 255];
const NUMBER: Rgba = [78, 78, 84, 255];
const FUNCTION: Rgba = [150, 150, 158, 255];
const OPERATION: Rgba = [10, 110, 235, 255];
const PAD: f32 = 10.0;
const GAP: f32 = 8.0;
const DISPLAY_H: f32 = 86.0;

struct Calculator {
    calc: Calc,
}

impl Calculator {
    fn keyboard(&mut self, ui: &mut Ui) {
        let input = ui.input().clone();
        if input.shortcut("c") {
            let text = self.calc.display();
            ui.copy(&text);
        }
        if input.shortcut("v") {
            ui.request_paste();
        }
        if let Some(text) = &input.paste {
            self.calc.paste(text);
        }
        for c in input.text.chars() {
            let button = match c {
                '0'..='9' => Some(Button::Digit(c as u8 - b'0')),
                ',' | '.' => Some(Button::Comma),
                '+' => Some(Button::Op(Op::Add)),
                '-' => Some(Button::Op(Op::Sub)),
                '*' | 'x' | '×' => Some(Button::Op(Op::Mul)),
                '/' | '÷' => Some(Button::Op(Op::Div)),
                '=' => Some(Button::Equals),
                '%' => Some(Button::Percent),
                _ => None,
            };
            if let Some(b) = button {
                self.calc.press(b);
            }
        }
        for key in &input.keys {
            match key {
                Key::Enter => self.calc.press(Button::Equals),
                Key::Backspace => self.calc.press(Button::Backspace),
                Key::Escape | Key::Delete => self.calc.press(Button::Clear),
                _ => {}
            }
        }
    }
}

impl App for Calculator {
    fn view(&mut self, ui: &mut Ui) {
        self.keyboard(ui);
        let (_, _, w, h) = ui.region();
        let bw = (w - 2.0 * PAD - 3.0 * GAP) / 4.0;
        let bh = ((h - DISPLAY_H - PAD - 4.0 * GAP) / 5.0).max(20.0);
        let mut pressed = None;
        let mut buttons = Vec::new();
        for (row, keys) in GRID.iter().enumerate() {
            for (col, key) in keys.iter().enumerate() {
                let Some(button) = *key else {
                    continue;
                };
                let width = if button == Button::Digit(0) { 2.0 * bw + GAP } else { bw };
                let rect = (PAD + col as f32 * (bw + GAP), DISPLAY_H + row as f32 * (bh + GAP), width, bh);
                let r = ui.background(rect);
                if r.clicked {
                    pressed = Some(button);
                }
                let held = r.hovered && ui.input().down;
                buttons.push((button, rect, held));
            }
        }
        if let Some(b) = pressed {
            self.calc.press(b);
        }

        let display = self.calc.display();
        let clear = self.calc.clear_label();
        let pending = self.calc.op.filter(|_| self.calc.fresh);
        ui.paint_text(|pixmap, text, (ox, oy)| {
            fill_rounded(pixmap, ox, oy, w, h, 0.0, mide::ui::draw::rgba(BACKGROUND[0], BACKGROUND[1], BACKGROUND[2], 255));
            // Tela: o número à direita, menor quando não cabe.
            let mut size = 48.0;
            while size > 16.0 && text.measure(&display, size, FontWeight::Regular) > w - 2.0 * PAD - 8.0 {
                size -= 2.0;
            }
            let tw = text.measure(&display, size, FontWeight::Regular);
            text.draw(pixmap, &display, ox + w - PAD - 4.0 - tw, oy + DISPLAY_H - GAP - size * 1.3, size, FontWeight::Regular, [255, 255, 255, 255]);
            for (button, (x, y, bw, bh), held) in &buttons {
                let (mut bg, fg) = match button {
                    Button::Op(op) if pending == Some(*op) => ([255, 255, 255, 255], OPERATION),
                    Button::Op(_) | Button::Equals => (OPERATION, [255, 255, 255, 255]),
                    Button::Clear | Button::Sign | Button::Percent => (FUNCTION, [20, 20, 24, 255]),
                    _ => (NUMBER, [255, 255, 255, 255]),
                };
                if *held {
                    for c in bg.iter_mut().take(3) {
                        *c = (*c as u32 * 4 / 5 + 40).min(255) as u8;
                    }
                }
                let radius = bh.min(*bw) / 2.0;
                fill_rounded(pixmap, ox + x, oy + y, *bw, *bh, radius, mide::ui::draw::rgba(bg[0], bg[1], bg[2], bg[3]));
                let label = label(*button, clear);
                let fs = if matches!(button, Button::Digit(_) | Button::Comma) { 22.0 } else { 20.0 };
                let lw = text.measure(label, fs, FontWeight::Regular);
                // O 0 largo tem o algarismo alinhado com a coluna da esquerda.
                let lx = if *button == Button::Digit(0) { x + (bh.min(*bw) - lw) / 2.0 } else { x + (bw - lw) / 2.0 };
                text.draw(pixmap, label, ox + lx, oy + y + (bh - fs * 1.25) / 2.0, fs, FontWeight::Regular, fg);
            }
        });
    }
}

fn label(button: Button, clear: &'static str) -> &'static str {
    match button {
        Button::Digit(d) => ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"][d as usize],
        Button::Comma => ",",
        Button::Op(Op::Add) => "+",
        Button::Op(Op::Sub) => "−",
        Button::Op(Op::Mul) => "×",
        Button::Op(Op::Div) => "÷",
        Button::Equals => "=",
        Button::Clear => clear,
        Button::Sign => "±",
        Button::Percent => "%",
        Button::Backspace => "⌫",
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "mide_calculator=info".into()))
        .init();
    let window = Window::new("Calculadora").app_id("mide-calculator").size(250, 380).min_size(220, 320);
    run(window, Calculator { calc: Calc::default() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(calc: &mut Calc, seq: &str) {
        for c in seq.chars() {
            calc.press(match c {
                '0'..='9' => Button::Digit(c as u8 - b'0'),
                ',' => Button::Comma,
                '+' => Button::Op(Op::Add),
                '-' => Button::Op(Op::Sub),
                '*' => Button::Op(Op::Mul),
                '/' => Button::Op(Op::Div),
                '=' => Button::Equals,
                '%' => Button::Percent,
                'n' => Button::Sign,
                'c' => Button::Clear,
                '<' => Button::Backspace,
                _ => unreachable!(),
            });
        }
    }

    fn result(seq: &str) -> String {
        let mut calc = Calc::default();
        keys(&mut calc, seq);
        calc.display()
    }

    #[test]
    fn contas() {
        assert_eq!(result("2+3="), "5");
        assert_eq!(result("2+3*4="), "20");
        assert_eq!(result("10/4="), "2,5");
        assert_eq!(result("1/3="), "0,33333333333");
        assert_eq!(result("0,1+0,2="), "0,3");
        assert_eq!(result("2+3=="), "8");
        assert_eq!(result("5-8="), "-3");
        assert_eq!(result("1234567*1="), "1.234.567");
    }

    #[test]
    fn porcentagem_sinal_e_apagar() {
        assert_eq!(result("100+10%="), "110");
        assert_eq!(result("50%"), "0,5");
        assert_eq!(result("12n"), "-12");
        assert_eq!(result("123<"), "12");
        assert_eq!(result("12c"), "0");
        assert_eq!(result("1/0="), "Erro");
        assert_eq!(result("1/0=c7"), "7");
    }

    #[test]
    fn numeros_grandes_e_colar() {
        assert_eq!(result("999999999999*1000="), "1e15");
        assert_eq!(result("123456789*1000000="), "1,234568e14");
        let mut calc = Calc::default();
        calc.paste("1.234,5");
        assert_eq!(calc.display(), "1.234,5");
        calc.paste("3.25");
        assert_eq!(calc.display(), "3,25");
    }
}
