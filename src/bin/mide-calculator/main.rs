//! Calculadora do mide (E3, E3b): modos Básico, Científico e Programador, notação
//! polonesa reversa (RPN), memória, histórico, casas decimais, desfazer, o conversor
//! de unidades e as notas matemáticas. As contas ficam em `mide::calculator` (escritas
//! do zero); aqui ficam as teclas, o teclado e os menus.
//!
//! Teclado (também o numérico): números, + - * / ^ ( ), Enter ou =, %, Backspace, Esc,
//! Ctrl+C e Ctrl+V; Ctrl+Z desfaz e Ctrl+Shift+Z refaz; Ctrl+1/2/3/4 trocam o modo,
//! Ctrl+T mostra o histórico; no Programador, A a F, & | ^ ~ % < >. Conversor: Ctrl+U
//! liga, Ctrl+Shift+U troca as unidades, Ctrl+L escolhe a unidade, ↑/↓ usam a outra
//! linha. Notas: Ctrl+N cria uma nota e Ctrl+T mostra a lista.

mod converter;
mod money;
mod notes;

use std::error::Error;

use converter::{Conv, CONVERT_H};

use mide::{
    calculator::{
        engine::{Binary, Engine, Entry, Key as EKey, Unary},
        format::Style,
        programmer::{self, Base, Programmer},
        rpn::Rpn,
        units::Pairs,
    },
    kit::{run, window_menu, App, ListPicker, MenuGroup, MenuItem, Picked, Rgba, Ui, Window},
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
    /// Notas matemáticas: contas digitadas em texto (`notes.rs`).
    Notes,
}

impl Mode {
    fn key(self) -> &'static str {
        match self {
            Mode::Basic => "basica",
            Mode::Scientific => "cientifica",
            Mode::Programmer => "programador",
            Mode::Notes => "notas",
        }
    }

    fn from_key(key: &str) -> Self {
        match key {
            "cientifica" => Mode::Scientific,
            "programador" => Mode::Programmer,
            "notas" => Mode::Notes,
            _ => Mode::Basic,
        }
    }

    /// Tamanho da janela (sem o histórico) e o mínimo.
    fn size(self) -> ((u32, u32), (u32, u32)) {
        match self {
            Mode::Basic => ((300, 490), (260, 430)),
            Mode::Scientific => ((680, 470), (600, 420)),
            Mode::Programmer => ((520, 600), (480, 540)),
            Mode::Notes => ((720, 480), (520, 320)),
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

// Cores da Calculadora do Tahoe: fundo grafite, números em cinza escuro, funções em
// cinza mais claro e as operações em laranja.
const BACKGROUND: Rgba = [30, 30, 32, 255];
const NUMBER: Rgba = [66, 66, 70, 255];
const FUNCTION: Rgba = [108, 108, 112, 255];
const SCIENCE: Rgba = [50, 50, 54, 255];
const OPERATION: Rgba = [255, 159, 10, 255];
const PAD: f32 = 14.0;
const GAP: f32 = 10.0;
/// Visor (com os botões da janela por cima, no canto).
const DISPLAY_H: f32 = 132.0;
/// Faixa de cima com os botões da janela (Tahoe): o conteúdo do visor começa abaixo.
const TITLE_H: f32 = converter::TITLE_H;
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
    conv: Conv,
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
    /// Conversor de unidades ligado (aparece na Básica e na Científica).
    convert_on: bool,
    conv: Conv,
    /// Pares de unidades lembrados por categoria e o texto gravado na configuração.
    pairs: Pairs,
    saved_pairs: String,
    /// Seletor de unidades aberto e a linha que ele muda.
    picker: Option<(usize, ListPicker)>,
    /// Largura do nome da unidade em cada linha, medida no último desenho (os cliques no
    /// nome e o ⌫ seguem ela).
    conv_label_w: [f32; 2],
    /// Última tela e conversão registradas no log (para os testes acompanharem).
    logged: String,
    logged_conv: String,
    /// Notas matemáticas (abertas na primeira vez que o modo aparece).
    notes: notes::Notes,
    /// Cotações das moedas (lidas na primeira vez que a Moeda aparece).
    money: money::Money,
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
        let saved_pairs = match settings_file::get("calculator", "convert_pairs") {
            Some(Value::Text(t)) => t,
            _ => String::new(),
        };
        let pairs = Pairs::from_text(&saved_pairs);
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
            convert_on: flag("convert"),
            conv: Conv::from_pairs(&pairs),
            pairs,
            saved_pairs,
            picker: None,
            conv_label_w: [80.0; 2],
            logged: String::new(),
            logged_conv: String::new(),
            money: money::Money::default(),
            notes: notes::Notes::new(!matches!(settings_file::get("calculator", "notes_list"), Some(Value::Bool(false)))),
        }
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot { engine: self.engine.clone(), programmer: self.programmer.clone(), rpn: self.rpn.clone(), conv: self.conv }
    }

    /// Volta a um estado guardado. O histórico, Rad/Grau e as casas decimais são do
    /// momento (desfazer volta a conta, não as opções nem a fita).
    fn restore(&mut self, s: Snapshot) {
        let history = std::mem::take(&mut self.engine.history);
        let (radians, style, base) = (self.engine.radians, self.engine.style, self.programmer.base);
        (self.engine, self.rpn, self.programmer, self.conv) = (s.engine, s.rpn, s.programmer, s.conv);
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
            let before = self.conv;
            self.redo.push(self.snapshot());
            self.restore(s);
            if self.conv != before {
                self.remember_conv();
            }
        }
    }

    fn redo(&mut self) {
        if let Some(s) = self.redo.pop() {
            let before = self.conv;
            self.undo.push(self.snapshot());
            self.restore(s);
            if self.conv != before {
                self.remember_conv();
            }
        }
    }

    /// Ângulos em radianos (ou em graus): no teclado e nas notas.
    fn set_radians(&mut self, radians: bool) {
        self.engine.radians = radians;
        self.rpn.radians = radians;
        self.save("radians", radians.then_some(Value::Bool(true)));
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
        // Saindo das notas, a nota fica gravada; entrando, o texto recebe as teclas e o
        // modo de teclado fica lembrado ("Levar para a calculadora" volta para ele).
        if self.mode == Mode::Notes {
            self.notes.save_now();
            ui.blur();
        }
        if mode == Mode::Notes {
            self.notes.keypad = Some(self.mode);
            ui.focus_next_input();
        }
        // O número da tela passa entre os modos de teclado. As Notas não usam esse
        // número: entrar nelas não passa nada, e sair conta a partir do modo de teclado
        // lembrado (Programador → Notas → Programador deixa a conta como estava).
        let from = if self.mode == Mode::Notes { self.notes.keypad.unwrap_or(Mode::Basic) } else { self.mode };
        if mode != Mode::Notes {
            if mode == Mode::Programmer && from != Mode::Programmer {
                self.programmer.load(self.value());
            } else if mode != Mode::Programmer && from == Mode::Programmer {
                let v = self.programmer.value() as i64 as f64;
                if self.rpn_on {
                    self.rpn.push(v);
                } else {
                    self.engine.load(v);
                }
            }
        }
        self.mode = mode;
        tracing::info!("calculadora: modo {}", mode.key());
        self.save("mode", (mode != Mode::Basic).then(|| Value::Text(mode.key().into())));
        self.resize(ui);
    }

    /// Tamanho da janela no modo atual (com o histórico, se aparece) e o mínimo.
    fn window_size(&self) -> ((u32, u32), (u32, u32)) {
        window_size_for(self.mode, self.rpn_on, self.history_shown, self.converting())
    }

    /// Conversor ligado e à vista (na Básica e na Científica).
    fn converting(&self) -> bool {
        self.convert_on && matches!(self.mode, Mode::Basic | Mode::Scientific)
    }

    /// Liga ou desliga o conversor: a janela cresce ou diminui só a altura dele.
    fn toggle_converter(&mut self, ui: &mut Ui) {
        if !matches!(self.mode, Mode::Basic | Mode::Scientific) {
            return;
        }
        self.convert_on = !self.convert_on;
        tracing::info!("calculadora: conversor {}", if self.convert_on { "ligado" } else { "desligado" });
        self.save("convert", self.convert_on.then_some(Value::Bool(true)));
        let (w, h) = ui.window_size();
        let h = if self.convert_on { h + CONVERT_H as f32 } else { h - CONVERT_H as f32 };
        let (_, min) = self.window_size();
        ui.request_size(((w.round() as u32).max(min.0), (h.round() as u32).max(min.1)), min);
    }

    /// Outras unidades no conversor (⇅, seletor, menus): entram no desfazer, e o par fica
    /// lembrado para a categoria.
    fn set_conv(&mut self, conv: Conv) {
        if conv == self.conv {
            return;
        }
        self.change(|c| c.conv = conv);
        self.remember_conv();
    }

    /// O par do conversor fica lembrado para a categoria (também depois de desfazer).
    fn remember_conv(&mut self) {
        let conv = self.conv;
        self.pairs.touch(conv.category, conv.unit[0], conv.unit[1]);
        let text = self.pairs.to_text();
        if text != self.saved_pairs {
            self.save("convert_pairs", Some(Value::Text(text.clone())));
            self.saved_pairs = text;
        }
        tracing::info!("calculadora: unidades {} {}>{}", converter::category_id(conv.category), conv.label(0).2, conv.label(1).2);
    }

    /// A pessoa deixou buscar cotações na internet (`None`: ainda não perguntada).
    fn currency_online(&self) -> Option<bool> {
        match settings_file::get("calculator", "currency_online") {
            Some(Value::Bool(on)) => Some(on),
            _ => None,
        }
    }

    /// As janelas das cotações e o que a pessoa respondeu nelas.
    fn money_dialogs(&mut self, ui: &mut Ui) {
        let pair = if converter::is_money(self.conv.category) { (self.conv.symbol(0), self.conv.symbol(1)) } else { ("USD", "BRL") };
        let online = self.currency_online();
        let Some(answer) = money::dialogs(ui, &mut self.money, online, pair) else {
            return;
        };
        match answer {
            money::Answer::Online(on) | money::Answer::Toggle(on) => {
                self.save("currency_online", Some(Value::Bool(on)));
                tracing::info!("calculadora: buscar cotações na internet: {}", if on { "sim" } else { "não" });
                if on {
                    self.money.ensure(Some(true), false, ui.waker());
                }
            }
            money::Answer::Refresh => self.money.ensure(self.currency_online(), true, ui.waker()),
        }
        ui.request_repaint();
    }

    /// A tela mostra Erro (no motor da vez).
    fn has_error(&self) -> bool {
        if self.rpn_on { self.rpn.has_error() } else { self.engine.has_error() }
    }

    /// O número da outra linha do conversor (`None` com Erro na tela ou sem conversão,
    /// como consumo zero: a linha mostra "—" e não há o que copiar nem usar).
    fn conversion(&self) -> Option<f64> {
        if self.has_error() { None } else { self.conv.convert(self.value(), self.money.rates.as_deref()) }
    }

    /// Usa a outra linha do conversor: o número convertido vira o da tela, como uma
    /// tecla de função (no meio da conta, vira o operando; no RPN, troca o X), e a outra
    /// linha passa a ser a ativa.
    fn use_other_row(&mut self) {
        if (self.rpn_on && self.rpn.has_error()) || (!self.rpn_on && self.engine.has_error()) {
            return;
        }
        let x = self.value();
        let Some(v) = self.conv.convert(x, self.money.rates.as_deref()) else {
            return;
        };
        let text = self.conv.history(x, self.engine.style);
        self.change(|c| {
            if c.rpn_on {
                if c.rpn.apply(Some(v)) {
                    c.record_rpn(text);
                }
            } else {
                c.engine.apply(v, &text);
            }
            c.conv.active = c.conv.other();
        });
    }

    /// Abre o seletor de unidades para uma linha do conversor.
    fn open_picker(&mut self, row: usize) {
        if self.converting() {
            self.picker = Some((row, self.conv.picker(row)));
        }
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
            Btn::Angle => self.set_radians(!self.engine.radians),
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
        // Como a Calculadora do Tahoe: ⌫ AC % ÷ em cima e ± 0 , = embaixo.
        let basic: Vec<Vec<Option<(Btn, String, Kind)>>> = vec![
            vec![k(Btn::K(EKey::Backspace), "⌫", Kind::Function), k(Btn::K(EKey::Clear), clear, Kind::Function), k(un(Unary::Percent), "%", Kind::Function), k(op(Binary::Div), "÷", Kind::Operation)],
            vec![k(digit(7), "7", Kind::Number), k(digit(8), "8", Kind::Number), k(digit(9), "9", Kind::Number), k(op(Binary::Mul), "×", Kind::Operation)],
            vec![k(digit(4), "4", Kind::Number), k(digit(5), "5", Kind::Number), k(digit(6), "6", Kind::Number), k(op(Binary::Sub), "−", Kind::Operation)],
            vec![k(digit(1), "1", Kind::Number), k(digit(2), "2", Kind::Number), k(digit(3), "3", Kind::Number), k(op(Binary::Add), "+", Kind::Operation)],
            vec![k(un(Unary::Negate), "±", Kind::Number), k(digit(0), "0", Kind::Number), k(Btn::K(EKey::Comma), ",", Kind::Number), equals],
        ];
        match self.mode {
            // Com RPN, uma fileira a mais com as teclas da pilha.
            Mode::Basic if self.rpn_on => {
                let sc = Kind::Science;
                let mut rows = vec![vec![k(Btn::Swap, "x↔y", sc), k(Btn::Roll, "R↓", sc), k(Btn::RollUp, "R↑", sc), k(Btn::LastX, "LASTx", sc)]];
                rows.extend(basic);
                rows
            }
            // As notas não têm teclas (o `view` delas não chama o `grid`).
            Mode::Basic | Mode::Notes => basic,
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
        // Conversor: Ctrl+U liga, Ctrl+Shift+U troca as unidades, Ctrl+L escolhe a
        // unidade (com o seletor aberto, as teclas deste quadro param aqui).
        match input.shortcut_shift("u") {
            Some(false) => self.toggle_converter(ui),
            Some(true) if self.converting() => self.set_conv(self.conv.swapped()),
            _ => {}
        }
        if input.shortcut("l") && self.converting() {
            self.open_picker(self.conv.active);
            return;
        }
        // Ctrl+1/2/3/4: modos; Ctrl+T: histórico; Ctrl+R: RPN.
        for (key, mode) in [("1", Mode::Basic), ("2", Mode::Scientific), ("3", Mode::Programmer), ("4", Mode::Notes)] {
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
            // ↑/↓ (também do teclado numérico): usar a outra linha do conversor.
            if self.converting() && matches!(press.keysym, 0xff52 | 0xff54 | 0xff97 | 0xff99) {
                self.use_other_row();
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
            // Nas notas, Editar e Arquivo são da nota.
            if self.mode == Mode::Notes && self.notes_menu_action(ui, &action) {
                continue;
            }
            match action.as_str() {
                "basica" => self.set_mode(ui, Mode::Basic),
                "cientifica" => self.set_mode(ui, Mode::Scientific),
                "programador" => self.set_mode(ui, Mode::Programmer),
                "notas" => self.set_mode(ui, Mode::Notes),
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
                "conversor" => self.toggle_converter(ui),
                "radianos" => self.set_radians(!self.engine.radians),
                "conv-unidade" => self.open_picker(self.conv.active),
                "conv-trocar" if self.converting() => self.set_conv(self.conv.swapped()),
                "conv-outra" if self.converting() => self.use_other_row(),
                "copiar-conversao" if self.converting() && self.conversion().is_some() => ui.copy(&self.conv.text(self.value(), self.engine.style, self.money.rates.as_deref())),
                "cotacoes" => self.money.window = true,
                other => {
                    if let Some(d) = other.strip_prefix("casas:") {
                        let style = Style { decimals: d.parse().ok(), ..self.engine.style };
                        self.set_style(style);
                    } else if let Some(i) = other.strip_prefix("conv-recente:").and_then(|i| i.parse::<usize>().ok()) {
                        if let Some(&(category, a, b)) = converter::recent_pairs(&self.pairs, &self.conv).get(i) {
                            self.set_conv(Conv { category, unit: [a, b], active: self.conv.active });
                        }
                    } else if let Some(category) = other.strip_prefix("conv-cat:").and_then(|id| converter::category_order().into_iter().find(|&c| converter::category_id(c) == id)) {
                        // Outra categoria: o par lembrado dela (ou o padrão).
                        let (a, b) = self.pairs.get(category).or_else(|| converter::default_pair(category)).unwrap_or((0, 1));
                        self.set_conv(Conv { category, unit: [a, b], active: self.conv.active });
                    }
                }
            }
        }
    }
}

impl App for Calculator {
    fn view(&mut self, ui: &mut Ui) {
        self.menu_actions(ui);
        // Notas: outra tela, sem o teclado da calculadora.
        if self.mode == Mode::Notes {
            self.picker = None;
            self.notes_view(ui);
            let (_, _, total_w, _) = ui.region();
            ui.window_controls(16.0, 16.0);
            ui.window_drag_area((0.0, 0.0, total_w, 40.0));
            return;
        }
        // O conversor sumiu (Programador, desligado): o seletor dele fecha junto.
        if !self.converting() {
            self.picker = None;
        }
        // Moeda: o resultado de uma busca que terminou, e buscar se precisar (a busca
        // só acontece quando a pessoa usa moeda).
        if self.money.poll() {
            ui.request_repaint();
        }
        if self.converting() && converter::is_money(self.conv.category) {
            self.money.ensure(self.currency_online(), false, ui.waker());
            if self.money.fetching() {
                ui.wake_after(std::time::Duration::from_millis(200));
            }
        }
        // Com o seletor de unidades ou uma janela das cotações aberta, as teclas são dela.
        let money_open = self.money.asking || self.money.window || self.money.failure.is_some();
        if self.picker.is_none() && !money_open {
            self.keyboard(ui);
        }
        let (_, _, total_w, h) = ui.region();
        let history = self.history_shown;
        let w = if history { total_w - HISTORY_W } else { total_w };
        let grid = self.grid();
        let cols = grid.iter().map(Vec::len).max().unwrap_or(4) as f32;
        let rows = grid.len() as f32;
        // Programador: a tela e os 64 bits em cima; conversor: as duas linhas.
        let converting = self.converting();
        let top = match self.mode {
            Mode::Programmer => DISPLAY_H + TITLE_H + 112.0,
            _ if converting => DISPLAY_H + CONVERT_H as f32,
            _ => DISPLAY_H,
        };
        let bw = (w - 2.0 * PAD - (cols - 1.0) * GAP) / cols;
        // Teclas redondas (quadradas por dentro), como no Tahoe: no máximo a largura.
        let bh = ((h - top - PAD - (rows - 1.0) * GAP) / rows).max(20.0).min(bw.max(20.0));
        // O que sobra de altura vai para o visor (as teclas ficam embaixo).
        let top = h - PAD - rows * bh - (rows - 1.0) * GAP;
        let keys_top = if self.mode == Mode::Programmer || converting { top } else { top };
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
        let erase = match self.mode {
            Mode::Programmer => (w - PAD - 30.0, TITLE_H + 6.0, 30.0, 24.0),
            _ if converting => converter::erase_rect(w, self.conv.active, self.conv_label_w[self.conv.active]),
            _ => (PAD, DISPLAY_H - GAP - 30.0, 30.0, 24.0),
        };
        // Na Básica e na Científica, o ⌫ é uma tecla (como no Tahoe); a do visor fica
        // para o Programador e o conversor.
        let erase_on_screen = typing && (self.mode == Mode::Programmer || converting);
        if erase_on_screen && ui.background(erase).clicked {
            self.press(if self.mode == Mode::Programmer { Btn::P(programmer::Key::Backspace) } else { Btn::K(EKey::Backspace) });
        }
        // Conversor: o nome da unidade abre o seletor; a outra linha usa a conversão; o
        // ⇅ troca as unidades; clique direito numa linha, o menu dela.
        if converting {
            let mut act: Option<(usize, u8)> = None;
            let has_conversion = self.conversion().is_some();
            for row in 0..2 {
                // O nome vem antes da linha: clicar nele abre o seletor (também na linha
                // que não é a ativa).
                let name = ui.background(converter::name_rect(w, row, self.conv_label_w[row])).clicked;
                let r = ui.background(converter::row_rect(w, row));
                if name {
                    act = Some((row, 0));
                } else if r.clicked && row != self.conv.active {
                    act = Some((row, 1));
                }
                let active = row == self.conv.active;
                ui.context_menu(&r, |m| {
                    if (active || has_conversion) && m.item("Copiar o número") {
                        act = Some((row, 2));
                    }
                    if has_conversion && m.item("Copiar a conversão") {
                        act = Some((row, 3));
                    }
                    if m.item("Escolher a unidade…") {
                        act = Some((row, 0));
                    }
                    if m.item("Trocar as unidades") {
                        act = Some((row, 4));
                    }
                });
            }
            if ui.background(converter::swap_rect()).clicked {
                act = Some((0, 4));
            }
            // Na Moeda, a faixa (fonte e data) abre "Cotações de moedas".
            if converter::is_money(self.conv.category) && ui.background((w / 2.0, TITLE_H + 65.0, w / 2.0 - PAD, 20.0)).clicked {
                self.money.window = true;
            }
            match act {
                Some((row, 0)) => self.open_picker(row),
                Some((_, 1)) => self.use_other_row(),
                Some((row, 2)) => {
                    // O número da linha, sem pontos de milhar.
                    let plain = Style { grouping: false, ..self.engine.style };
                    let text = if row == self.conv.active { Some(self.copy_text()) } else { self.conversion().map(|v| plain.value(v)) };
                    if let Some(text) = text {
                        ui.copy(&text);
                    }
                }
                Some((_, 3)) => ui.copy(&self.conv.text(self.value(), self.engine.style, self.money.rates.as_deref())),
                Some((_, 4)) => self.set_conv(self.conv.swapped()),
                _ => {}
            }
        }

        // Programador: a base escolhida e os bits clicáveis.
        let mut bit_rects = Vec::new();
        let mut base_rects = Vec::new();
        if self.mode == Mode::Programmer {
            for (i, base) in [Base::Hex, Base::Dec, Base::Oct, Base::Bin].into_iter().enumerate() {
                let rect = (PAD + i as f32 * 58.0, TITLE_H + 8.0, 52.0, 22.0);
                if ui.background(rect).clicked {
                    self.programmer.set_base(base);
                    self.save("base", (base != Base::Dec).then(|| Value::Text(base.label().to_lowercase())));
                }
                base_rects.push((base, rect));
            }
            let bit_w = (w - 2.0 * PAD) / 32.0;
            for n in 0..64u32 {
                let (row, col) = ((63 - n) / 32, (63 - n) % 32);
                let rect = (PAD + col as f32 * bit_w, DISPLAY_H + TITLE_H + 14.0 + row as f32 * 44.0, bit_w, 26.0);
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
        // Conversor: a unidade e o número de cada linha (a outra mostra a conversão).
        let error = if self.rpn_on { self.rpn.has_error() } else { self.engine.has_error() };
        let conv_view = converting.then(|| {
            let c = self.conv;
            let mut numbers = [String::new(), String::new()];
            numbers[c.active] = screen.clone();
            let rates = self.money.rates.as_deref();
            numbers[c.other()] = if error {
                "—".to_string()
            } else {
                c.convert(self.value(), rates).map_or_else(|| "—".to_string(), |v| c.style(c.other(), self.engine.style).value(v))
            };
            // Na Moeda, a faixa mostra a fonte e a data da cotação (ou o estado).
            let category = if converter::is_money(c.category) { self.money.band(self.currency_online()) } else { converter::category_name(c.category).to_string() };
            converter::Shown {
                names: [c.label(0).0.to_string(), c.label(1).0.to_string()],
                symbols: [c.symbol(0).to_string(), c.symbol(1).to_string()],
                numbers,
                active: c.active,
                category,
            }
        });
        if converting && !error {
            let mut line = self.conv.text(self.value(), self.engine.style, self.money.rates.as_deref());
            if converter::is_money(self.conv.category) {
                line = format!("{line} {}", self.money.log_tag());
            }
            if line != self.logged_conv {
                tracing::debug!("calculadora: conversão {line}");
                self.logged_conv = line;
            }
        }
        let mut label_w = self.conv_label_w;
        ui.paint_text(|pixmap, text, (ox, oy)| {
            let bg = |c: Rgba| mide::ui::draw::rgba(c[0], c[1], c[2], c[3]);
            fill_rounded(pixmap, ox, oy, total_w, h, 0.0, bg(BACKGROUND));
            let small = [180, 180, 188, 255];
            if let Some(cv) = &conv_view {
                label_w = converter::draw(pixmap, text, (ox, oy), w, cv, typing);
            } else {
                // Tela: o número à direita, menor quando não cabe; com o ⌫ na tela, o
                // número não passa por cima dele.
                let room = if typing && mode != Mode::Programmer { 36.0 } else { 0.0 };
                let (screen, size) = fit_display(text, &screen, w - 2.0 * PAD - 8.0 - room, mode == Mode::Programmer, 48.0);
                let tw = text.measure(&screen, size, FontWeight::Regular);
                // Pela base: no Programador, logo acima da fileira de bits.
                // Pela base, logo acima das teclas (no Programador, acima dos bits).
                let display_y = if mode == Mode::Programmer { DISPLAY_H + TITLE_H + 8.0 - size * 1.3 } else { keys_top - GAP - size * 1.3 };
                text.draw(pixmap, &screen, ox + w - PAD - 4.0 - tw, oy + display_y, size, FontWeight::Regular, [255, 255, 255, 255]);
            }
            // Indicadores pequenos à esquerda da tela: Rad, memória, RPN (Y Z T).
            // A conta em andamento, pequena, acima do número.
            if !expression.is_empty() {
                // Como no Tahoe: a conta em cinza, logo acima do número (no Programador e no
                // conversor, pequena no alto).
                let big = mode != Mode::Programmer && conv_view.is_none();
                let (es, ey) = if big { (24.0, keys_top - GAP - 48.0 * 1.3 - 24.0 * 1.2) } else { (13.0, TITLE_H + 8.0) };
                let x0 = if mode == Mode::Scientific { PAD + 70.0 } else { PAD + 4.0 };
                let shown = fit_terms(text, &expression, es, w - x0 - PAD - 4.0);
                let ew = text.measure(&shown, es, FontWeight::Regular);
                text.draw(pixmap, &shown, ox + w - PAD - 4.0 - ew, oy + ey, es, FontWeight::Regular, [152, 152, 158, 255]);
            }
            if erase_on_screen {
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
                text.draw(pixmap, &tags.join("  "), ox + PAD + 4.0, oy + TITLE_H + 2.0, 12.0, FontWeight::Regular, small);
            }
            if let Some([y, z, t]) = &rpn_stack {
                // A pilha acima de X (no Científico, depois de Rad e M); cortada pela
                // esquerda se não cabe (Y é o que mais importa).
                let x0 = if mode == Mode::Scientific { PAD + 70.0 } else { PAD + 4.0 };
                let stack = fit_left(text, &format!("T {t}   Z {z}   Y {y}"), 12.0, w - x0 - PAD);
                text.draw(pixmap, &stack, ox + x0, oy + TITLE_H + 2.0, 12.0, FontWeight::Regular, small);
            }
            if mode == Mode::Programmer {
                for (b, (x, y, bw2, bh2)) in &base_rects {
                    let on = *b == base;
                    fill_rounded(pixmap, ox + x, oy + y, *bw2, *bh2, 6.0, bg(if on { OPERATION } else { SCIENCE }));
                    let lw = text.measure(b.label(), 12.0, FontWeight::Semibold);
                    text.draw(pixmap, b.label(), ox + x + (bw2 - lw) / 2.0, oy + y + 4.0, 12.0, FontWeight::Semibold, [255, 255, 255, 255]);
                }
                if !character.is_empty() {
                    text.draw(pixmap, &format!("“{character}”"), ox + PAD + 4.0, oy + TITLE_H + 46.0, 14.0, FontWeight::Regular, small);
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
                    Kind::Function => (FUNCTION, [255, 255, 255, 255]),
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
                // Redonda: o círculo no meio da casa (a casa larga vira cápsula).
                let (x, bw) = if *wide { (*x, *bw) } else { (*x + (*bw - bh.min(*bw)) / 2.0, bh.min(*bw)) };
                let (x, bw) = (&x, &bw);
                fill_rounded(pixmap, ox + x, oy + y, *bw, *bh, radius, bg(if *enabled { fill } else { SCIENCE }));
                let fs = match kind {
                    Kind::Science | Kind::Function if label.chars().count() > 3 => 13.0,
                    Kind::Science => 16.0,
                    Kind::Number => 24.0,
                    // Sinais das operações grandes, como no Tahoe.
                    Kind::Operation if label.chars().count() == 1 => 28.0,
                    _ => 22.0,
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
        // O nome mudou de largura (o símbolo no lugar do nome): os cliques e o ⌫ seguem a
        // largura nova no quadro seguinte, pedido já.
        if label_w != self.conv_label_w {
            self.conv_label_w = label_w;
            ui.request_repaint();
        }
        // O seletor de unidades fica por cima de tudo.
        if let Some((row, picker)) = self.picker.as_mut() {
            let row = *row;
            match picker.show(ui) {
                Some(Picked::Chosen { group, item }) => {
                    self.picker = None;
                    // Os grupos do seletor vêm na ordem de `category_order` (a Moeda primeiro).
                    let category = converter::category_order()[group];
                    let conv = self.conv.choose(&self.pairs, row, category, item);
                    self.set_conv(conv);
                }
                Some(Picked::Cancelled) => self.picker = None,
                None => {}
            }
        }
        // Janelas das cotações (sem o seletor aberto) e avisos das notas (fechar sem
        // gravar), por cima do teclado.
        if self.picker.is_none() {
            self.money_dialogs(ui);
        }
        self.notes_dialogs(ui);
        // Tahoe: os botões da janela por cima do visor, e o alto do visor arrasta.
        ui.window_controls(16.0, 16.0);
        ui.window_drag_area((0.0, 0.0, total_w, 44.0));
    }

    fn close_requested(&mut self) -> bool {
        // As notas com alterações são gravadas; se falhar, um aviso pergunta o que fazer.
        self.notes_close()
    }

    fn menu_bar(&mut self) -> Vec<MenuGroup> {
        let mode = self.mode;
        let notes = mode == Mode::Notes;
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
        // Nas notas, Arquivo e Editar são da nota; o resto do Visualizar é o mesmo.
        let keypad_groups = if notes {
            self.notes_menus()
        } else {
            vec![MenuGroup {
                title: "Editar".into(),
                items: vec![
                    MenuItem::action("desfazer", "Desfazer").shortcut("Ctrl+Z").enabled(!self.undo.is_empty()),
                    MenuItem::action("refazer", "Refazer").shortcut("Ctrl+Shift+Z").enabled(!self.redo.is_empty()),
                    MenuItem::Separator,
                    MenuItem::action("copiar", "Copiar").shortcut("Ctrl+C"),
                    MenuItem::action("colar", "Colar").shortcut("Ctrl+V"),
                    MenuItem::action("copiar-conversao", "Copiar a conversão").enabled(self.converting() && self.conversion().is_some()),
                    MenuItem::Separator,
                    MenuItem::action("copiar-historico", "Copiar o histórico").enabled(has_history),
                    MenuItem::action("limpar-historico", "Limpar o histórico").enabled(has_history),
                ],
            }]
        };
        let keypad = matches!(mode, Mode::Basic | Mode::Scientific);
        let list_item = if notes {
            MenuItem::action("notas-lista", "Mostrar a lista de notas").shortcut("Ctrl+T").checked(self.notes.list_shown)
        } else {
            MenuItem::action("historico", "Mostrar o histórico").shortcut("Ctrl+T").checked(self.history_shown)
        };
        let mut groups = keypad_groups;
        groups.push(MenuGroup {
            title: "Visualizar".into(),
            items: vec![
                MenuItem::action("basica", "Básica").shortcut("Ctrl+1").checked(mode == Mode::Basic),
                MenuItem::action("cientifica", "Científica").shortcut("Ctrl+2").checked(mode == Mode::Scientific),
                MenuItem::action("programador", "Programador").shortcut("Ctrl+3").checked(mode == Mode::Programmer),
                MenuItem::action("notas", "Notas matemáticas").shortcut("Ctrl+4").checked(notes),
                MenuItem::Separator,
                MenuItem::action("rpn", "Notação polonesa reversa (RPN)").shortcut("Ctrl+R").checked(self.rpn_on).enabled(keypad),
                list_item,
                MenuItem::action("conversor", "Conversor de unidades").shortcut("Ctrl+U").checked(self.convert_on).enabled(keypad),
                MenuItem::Separator,
                MenuItem::action("radianos", "Ângulos em radianos").checked(self.engine.radians).enabled(mode != Mode::Programmer),
                MenuItem::action("milhar", "Separador de milhar").checked(style.grouping).enabled(mode != Mode::Programmer),
                MenuItem::Submenu { label: "Casas decimais".into(), items: decimals },
            ],
        });
        // O menu Converter só aparece com o conversor à vista.
        if self.converting() {
            let recent = converter::recent_pairs(&self.pairs, &self.conv)
                .into_iter()
                .enumerate()
                .map(|(i, pair)| MenuItem::action(&format!("conv-recente:{i}"), &converter::pair_label(pair)))
                .collect();
            let categories = converter::category_order()
                .into_iter()
                .map(|c| MenuItem::action(&format!("conv-cat:{}", converter::category_id(c)), converter::category_name(c)).checked(c == self.conv.category))
                .collect();
            groups.push(MenuGroup {
                title: "Converter".into(),
                items: vec![
                    MenuItem::action("conv-unidade", "Escolher a unidade…").shortcut("Ctrl+L"),
                    MenuItem::action("conv-trocar", "Trocar as unidades").shortcut("Ctrl+Shift+U"),
                    MenuItem::action("conv-outra", "Usar a outra linha"),
                    MenuItem::Separator,
                    MenuItem::Submenu { label: "Recentes".into(), items: recent },
                    MenuItem::Submenu { label: "Categoria".into(), items: categories },
                    MenuItem::Separator,
                    MenuItem::action("cotacoes", "Cotações de moedas…"),
                ],
            });
        }
        groups.push(window_menu(Vec::new()));
        groups
    }
}

/// Tamanho da janela e o mínimo: o do modo, mais a fileira da pilha do RPN (Básica), o
/// conversor (Básica e Científica) e o histórico ao lado.
fn window_size_for(mode: Mode, rpn: bool, history: bool, converting: bool) -> ((u32, u32), (u32, u32)) {
    // As notas têm a lista dentro da janela: o histórico, o RPN e o conversor não contam.
    if mode == Mode::Notes {
        return mode.size();
    }
    let ((w, h), (mw, mh)) = mode.size();
    let extra_w = if history { HISTORY_W as u32 } else { 0 };
    let mut extra_h = if mode == Mode::Basic && rpn { RPN_ROW } else { 0 };
    if converting {
        extra_h += CONVERT_H;
    }
    ((w + extra_w, h + extra_h), (mw + extra_w, mh + extra_h))
}

/// O número da tela e o tamanho da letra para caber em `width`: de `max` até 16 pontos;
/// no Programador (64 bits em binário), até 10, depois sem os espaços entre os grupos
/// e, por fim, cortado pela esquerda (os bits de cima continuam na fileira de bits).
fn fit_display(text: &mut Text, screen: &str, width: f32, small: bool, max: f32) -> (String, f32) {
    let min = if small { 10.0 } else { 16.0 };
    let candidates = [screen.to_string(), screen.replace(' ', "")];
    for candidate in candidates.iter().take(if small { 2 } else { 1 }) {
        let mut size = max;
        while size > min && text.measure(candidate, size, FontWeight::Regular) > width {
            size -= 2.0;
        }
        if text.measure(candidate, size, FontWeight::Regular) <= width {
            return (candidate.clone(), size);
        }
    }
    (fit_left(text, &candidates[usize::from(small)], min, width), min)
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
    let window = Window::new("Calculadora").app_id("mide-calculator").size(w, h).min_size(mw, mh).full_size_content();
    run(window, calculator)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tamanhos_da_janela() {
        assert_eq!(window_size_for(Mode::Basic, false, false, false).0, (300, 490));
        assert_eq!(window_size_for(Mode::Basic, false, false, true), ((300, 554), (260, 494)));
        assert_eq!(window_size_for(Mode::Basic, true, false, true).0, (300, 612));
        assert_eq!(window_size_for(Mode::Scientific, false, false, true).0, (680, 534));
        assert_eq!(window_size_for(Mode::Basic, false, true, true).0, (530, 554));
        // O Programador não mostra o conversor (o `converting` já vem falso).
        assert_eq!(window_size_for(Mode::Programmer, false, false, false).0, (520, 600));
        // As notas não somam o histórico, o RPN nem o conversor.
        assert_eq!(window_size_for(Mode::Notes, true, true, true), ((720, 480), (520, 320)));
    }
}
