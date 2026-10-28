//! Modo RPN (notação polonesa reversa): uma pilha de 4 registros (X, Y, Z, T), como
//! nas calculadoras RPN clássicas. Digita-se o número, Enter sobe a pilha, e a
//! operação usa Y e X (3 Enter 4 + dá 7).

use super::engine::{Binary, Unary};
use super::format::{Style, DIGITS};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Rpn {
    /// X (a tela), Y, Z, T.
    pub stack: [f64; 4],
    /// Número sendo digitado em X.
    entry: Option<String>,
    /// Depois de Enter (e de apagar X), o próximo número substitui X em vez de subir a
    /// pilha.
    lift_disabled: bool,
    /// A última operação não deu um número (divisão por zero): a tela mostra "Erro" e a
    /// pilha fica como estava; a próxima operação só apaga o aviso.
    error: bool,
    /// A última tecla foi C: o próximo C apaga a pilha toda.
    cleared: bool,
    /// LASTx: o X de antes da última conta (para corrigir ou refazer).
    last_x: f64,
    pub radians: bool,
    /// Casas decimais e ponto de milhar da tela.
    pub style: Style,
}

impl Rpn {
    /// Pilha nova com as opções lembradas (Rad/Grau, casas decimais e ponto de milhar).
    pub fn new(radians: bool, style: Style) -> Self {
        Self { radians, style, ..Self::default() }
    }

    pub fn x(&self) -> f64 {
        match &self.entry {
            Some(e) => Self::entry_value(e),
            None => self.stack[0],
        }
    }

    pub fn display(&self) -> String {
        self.shown_with(self.style)
    }

    /// X para copiar, sem pontos de milhar.
    pub fn copy_text(&self) -> String {
        self.shown_with(Style { grouping: false, ..self.style })
    }

    fn shown_with(&self, style: Style) -> String {
        if self.error {
            return crate::tr!("Error").into_owned();
        }
        match self.entry.as_deref() {
            Some("") => "0".into(),
            Some("-") => "-0".into(),
            Some(e) => style.digits(e),
            None => style.value(self.stack[0]),
        }
    }

    /// Y, Z e T para mostrar acima de X.
    pub fn upper(&self) -> [String; 3] {
        [1, 2, 3].map(|i| self.style.value(self.stack[i]))
    }

    /// Há um número sendo digitado (Backspace apaga um algarismo dele).
    pub fn typing(&self) -> bool {
        self.entry.is_some()
    }

    pub fn has_error(&self) -> bool {
        self.error
    }

    /// "C" apaga X; logo depois de um C, "AC" apaga a pilha toda.
    pub fn clear_label(&self) -> &'static str {
        if self.cleared { "AC" } else { "C" }
    }

    /// Valor do texto digitado ("2e" e "2e-" ainda sem o expoente valem 2).
    fn entry_value(e: &str) -> f64 {
        super::format::parse(e.trim_end_matches('-').trim_end_matches('e')).unwrap_or(0.0)
    }

    fn commit(&mut self) {
        if let Some(e) = self.entry.take() {
            self.stack[0] = Self::entry_value(&e);
        }
    }

    /// Uma operação com o aviso de erro na tela: só apaga o aviso (como nas RPN
    /// clássicas). `true`: a operação pode seguir.
    fn ready(&mut self) -> bool {
        self.cleared = false;
        !std::mem::take(&mut self.error)
    }

    fn lift(&mut self) {
        self.stack = [self.stack[0], self.stack[0], self.stack[1], self.stack[2]];
    }

    /// O número sendo digitado; o primeiro algarismo sobe a pilha (menos depois de Enter).
    fn entry(&mut self) -> &mut String {
        self.error = false;
        self.cleared = false;
        if self.entry.is_none() {
            if !self.lift_disabled {
                self.lift();
            }
            self.lift_disabled = false;
            self.entry = Some(String::new());
        }
        self.entry.as_mut().expect("número")
    }

    pub fn digit(&mut self, d: u8) {
        let e = self.entry();
        // O limite de algarismos não conta o expoente (2e15), que tem até 3 e não pode
        // passar do maior número (2e999 não existe no f64).
        if let Some((_, exp)) = e.split_once('e') {
            let next = format!("{e}{d}");
            if exp.trim_start_matches('-').len() < 3 && super::format::parse(&next).is_some() {
                *e = next;
            }
            return;
        }
        if e.chars().filter(char::is_ascii_digit).count() < DIGITS {
            if e == "0" {
                e.clear();
            } else if e == "-0" {
                *e = "-".into();
            }
            e.push((b'0' + d) as char);
        }
    }

    pub fn comma(&mut self) {
        let e = self.entry();
        if e.contains('e') {
            return;
        }
        if e.is_empty() || e == "-" {
            e.push('0');
        }
        if !e.contains(',') {
            e.push(',');
        }
    }

    /// EE: o expoente faz parte do número digitado (2 EE 3 é 2.000); sem número sendo
    /// digitado, começa em 1 (EE 3 é 1.000).
    pub fn exponent(&mut self) {
        let e = self.entry();
        if e.contains('e') {
            return;
        }
        if e.is_empty() || e == "-" {
            e.push('1');
        }
        e.push('e');
    }

    /// Enter: X sobe para Y e o próximo número substitui X.
    pub fn enter(&mut self) {
        if !self.ready() {
            return;
        }
        self.commit();
        self.lift();
        self.lift_disabled = true;
    }

    /// Põe um número em X (constante, memória, número colado), subindo a pilha como
    /// um número digitado.
    pub fn push(&mut self, v: f64) {
        self.error = false;
        self.cleared = false;
        self.commit();
        if !self.lift_disabled {
            self.lift();
        }
        self.stack[0] = v;
        self.lift_disabled = false;
    }

    /// Termina o número sendo digitado (m+ e m− usam X sem mexer na pilha); o próximo
    /// número sobe a pilha.
    pub fn end_entry(&mut self) {
        if !self.ready() {
            return;
        }
        self.commit();
        self.lift_disabled = false;
    }

    /// Resultado de uma operação em X; `None` se não deu um número (a pilha fica).
    fn finite(v: f64) -> Option<f64> {
        v.is_finite().then_some(if v == 0.0 { 0.0 } else { v })
    }

    /// Operação de dois números (Y e X). `true` se a conta foi feita.
    pub fn binary(&mut self, op: Binary) -> bool {
        if !self.ready() {
            return false;
        }
        self.commit();
        let (x, y) = (self.stack[0], self.stack[1]);
        let Some(v) = Self::finite(op.apply(y, x)) else {
            // A pilha fica como estava, e o próximo número entra no lugar do X que deu erro.
            self.error = true;
            self.lift_disabled = true;
            return false;
        };
        // A pilha desce: Z vai para Y, T para Z (e T fica).
        self.stack = [v, self.stack[2], self.stack[3], self.stack[3]];
        self.last_x = x;
        self.lift_disabled = false;
        true
    }

    /// Função de X. `true` se a conta foi feita.
    pub fn unary(&mut self, f: Unary) -> bool {
        if f == Unary::Negate {
            return self.negate();
        }
        if !self.ready() {
            return false;
        }
        self.commit();
        let v = match f {
            // %: X por cento de Y (Y fica, para somar ou subtrair depois: 200 Enter 10 % + dá 220).
            Unary::Percent => self.stack[1] * self.stack[0] / 100.0,
            _ => f.apply(self.stack[0], self.radians),
        };
        let Some(v) = Self::finite(v) else {
            self.error = true;
            self.lift_disabled = true;
            return false;
        };
        self.last_x = self.stack[0];
        self.stack[0] = v;
        self.lift_disabled = false;
        true
    }

    /// ± (CHS): no número sendo digitado, troca o sinal (do expoente, depois de EE) e a
    /// digitação continua (5 ± 3 dá -53); senão, troca o sinal de X.
    fn negate(&mut self) -> bool {
        if !self.ready() {
            return false;
        }
        match &mut self.entry {
            Some(e) => {
                let at = e.find('e').map_or(0, |i| i + 1);
                if e[at..].starts_with('-') {
                    e.remove(at);
                } else {
                    e.insert(at, '-');
                }
            }
            None => self.stack[0] = -self.stack[0],
        }
        false
    }

    /// x↔y
    pub fn swap(&mut self) {
        if !self.ready() {
            return;
        }
        self.commit();
        self.stack.swap(0, 1);
        self.lift_disabled = false;
    }

    /// R↑: T vai para X e os outros sobem.
    pub fn roll_up(&mut self) {
        if !self.ready() {
            return;
        }
        self.commit();
        self.stack.rotate_right(1);
        self.lift_disabled = false;
    }

    /// Uma conversão aplicada a X, como uma função: X vira `v` e o LASTx guarda o de
    /// antes; `None` (sem resultado) mostra Erro e a pilha fica. `true` se aplicou.
    pub fn apply(&mut self, v: Option<f64>) -> bool {
        if !self.ready() {
            return false;
        }
        self.commit();
        let Some(v) = v.and_then(Self::finite) else {
            self.error = true;
            self.lift_disabled = true;
            return false;
        };
        self.last_x = self.stack[0];
        self.stack[0] = v;
        self.lift_disabled = false;
        true
    }

    /// LASTx: traz de volta o X de antes da última conta.
    pub fn recall_last_x(&mut self) {
        let v = self.last_x;
        self.push(v);
    }

    /// R↓: X vai para T e os outros descem.
    pub fn roll_down(&mut self) {
        if !self.ready() {
            return;
        }
        self.commit();
        self.stack.rotate_left(1);
        self.lift_disabled = false;
    }

    /// C apaga X (ou o número sendo digitado, ou o aviso de erro); um segundo C
    /// seguido apaga a pilha toda.
    pub fn clear(&mut self) {
        if self.cleared {
            self.stack = [0.0; 4];
        } else {
            self.entry = None;
            self.stack[0] = 0.0;
        }
        self.error = false;
        self.cleared = true;
        self.lift_disabled = true;
    }

    /// Apaga o último algarismo; sem número sendo digitado (ou ao apagar o último),
    /// apaga X (CLx) e o próximo número entra no lugar dele.
    pub fn backspace(&mut self) {
        if !self.ready() {
            return;
        }
        if let Some(e) = self.entry.as_mut() {
            e.pop();
            if !e.is_empty() && e != "-" {
                return;
            }
            self.entry = None;
        }
        self.stack[0] = 0.0;
        self.lift_disabled = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digits(r: &mut Rpn, n: &str) {
        for c in n.chars() {
            match c {
                ',' => r.comma(),
                d => r.digit(d as u8 - b'0'),
            }
        }
    }

    #[test]
    fn pilha_rpn() {
        let mut r = Rpn::default();
        r.digit(3);
        r.enter();
        r.digit(4);
        r.binary(Binary::Add);
        assert_eq!(r.display(), "7");
        // (2 + 3) × 4 = 2 Enter 3 + 4 ×.
        let mut r = Rpn::default();
        r.digit(2);
        r.enter();
        r.digit(3);
        r.binary(Binary::Add);
        r.digit(4);
        r.binary(Binary::Mul);
        assert_eq!(r.display(), "20");
        // 10 Enter 2 ÷ = 5; x↔y.
        let mut r = Rpn::default();
        digits(&mut r, "10");
        r.enter();
        r.digit(2);
        r.swap();
        assert_eq!(r.display(), "10");
        r.swap();
        r.binary(Binary::Div);
        assert_eq!(r.display(), "5");
        r.unary(Unary::Square);
        assert_eq!(r.display(), "25");
    }

    #[test]
    fn sinal_durante_a_digitacao() {
        // 5 ± 3: o número continua sendo digitado (-53), sem subir a pilha.
        let mut r = Rpn::default();
        r.digit(5);
        r.unary(Unary::Negate);
        r.digit(3);
        assert_eq!(r.display(), "-53");
        assert_eq!(r.stack[1], 0.0);
        // Num resultado, troca o sinal de X.
        r.enter();
        r.digit(2);
        r.binary(Binary::Mul);
        r.unary(Unary::Negate);
        assert_eq!(r.display(), "106");
    }

    #[test]
    fn subida_da_pilha() {
        // Constante depois de um resultado sobe a pilha; o algarismo seguinte também.
        let mut r = Rpn::default();
        r.digit(2);
        r.enter();
        r.digit(3);
        r.binary(Binary::Add);
        r.push(std::f64::consts::PI);
        r.digit(1);
        assert_eq!(r.upper()[0], "3,14159265359");
        assert_eq!(r.upper()[1], "5");
        r.binary(Binary::Add);
        r.binary(Binary::Add);
        assert_eq!(r.display(), "9,14159265359");
        // Depois de Enter, a constante substitui a cópia de X.
        let mut r = Rpn::default();
        r.digit(4);
        r.enter();
        r.push(2.0);
        r.binary(Binary::Mul);
        assert_eq!(r.display(), "8");
        // x↔y religa a subida: 5 Enter x↔y 3 + dá 8 (5 + 3), não 3.
        let mut r = Rpn::default();
        r.digit(5);
        r.enter();
        r.swap();
        r.digit(3);
        r.binary(Binary::Add);
        assert_eq!(r.display(), "8");
    }

    #[test]
    fn porcentagem_apagar_e_erro() {
        // 200 Enter 10 % + = 220.
        let mut r = Rpn::default();
        digits(&mut r, "200");
        r.enter();
        digits(&mut r, "10");
        r.unary(Unary::Percent);
        assert_eq!(r.display(), "20");
        r.binary(Binary::Add);
        assert_eq!(r.display(), "220");
        // Backspace fora da digitação apaga X, e o próximo número entra no lugar.
        r.backspace();
        assert_eq!(r.display(), "0");
        r.digit(7);
        assert_eq!(r.display(), "7");
        assert_eq!(r.upper()[0], "0");
        // Divisão por zero: "Erro", e a pilha continua (1 e 0); a tecla seguinte só
        // apaga o aviso.
        let mut r = Rpn::default();
        r.digit(1);
        r.enter();
        r.digit(0);
        assert!(!r.binary(Binary::Div));
        assert_eq!(r.display(), crate::tr!("Error"));
        r.binary(Binary::Add);
        assert_eq!(r.display(), "0");
        r.swap();
        assert_eq!(r.display(), "1");
        // Um número novo também apaga o aviso, e entra no lugar do X que deu erro.
        r.digit(0);
        r.binary(Binary::Div);
        r.push(42.0);
        assert_eq!(r.display(), "42");
        assert_eq!(r.upper()[0], "1");
        // ⌫ com o aviso na tela só apaga o aviso (X continua).
        let mut r = Rpn::default();
        r.digit(4);
        r.unary(Unary::Negate);
        r.unary(Unary::Sqrt);
        assert_eq!(r.display(), crate::tr!("Error"));
        r.backspace();
        assert_eq!(r.display(), "-4");
    }

    #[test]
    fn rolar_para_cima_e_ultimo_x() {
        // 1 Enter 2 Enter 3 Enter 4: X=4 Y=3 Z=2 T=1; R↑ traz o T para X.
        let mut r = Rpn::default();
        for d in [1, 2, 3] {
            r.digit(d);
            r.enter();
        }
        r.digit(4);
        r.roll_up();
        assert_eq!(r.display(), "1");
        assert_eq!(r.upper(), ["4".to_string(), "3".to_string(), "2".to_string()]);
        // LASTx: 3 Enter 4 × = 12; LASTx traz o 4 (e o 12 sobe para Y).
        let mut r = Rpn::default();
        r.digit(3);
        r.enter();
        r.digit(4);
        r.binary(Binary::Mul);
        r.recall_last_x();
        assert_eq!(r.display(), "4");
        assert_eq!(r.upper()[0], "12");
        // Depois de uma função também: 9 √ dá 3; LASTx traz o 9.
        r.digit(9);
        r.unary(Unary::Sqrt);
        r.recall_last_x();
        assert_eq!(r.display(), "9");
    }

    #[test]
    fn conversao_no_x() {
        // 5 Enter 12, converter 12 pol em cm: X = 30,48, Y continua 5, LASTx = 12.
        let mut r = Rpn::default();
        r.digit(5);
        r.enter();
        r.digit(1);
        r.digit(2);
        assert!(r.apply(Some(30.48)));
        assert_eq!(r.display(), "30,48");
        assert_eq!(r.upper()[0], "5");
        r.recall_last_x();
        assert_eq!(r.display(), "12");
        // Sem resultado: Erro, e a pilha fica.
        assert!(!r.apply(None));
        assert_eq!(r.display(), crate::tr!("Error"));
        r.swap();
        assert_eq!(r.display(), "12");
    }

    #[test]
    fn limpar_e_expoente() {
        // C apaga X; só um segundo C seguido apaga a pilha (mesmo com X = 0).
        let mut r = Rpn::default();
        digits(&mut r, "10");
        r.enter();
        digits(&mut r, "5");
        r.enter();
        r.binary(Binary::Sub);
        assert_eq!(r.display(), "0");
        assert_eq!(r.clear_label(), "C");
        r.clear();
        assert_eq!(r.upper()[0], "10");
        assert_eq!(r.clear_label(), "AC");
        r.clear();
        assert_eq!(r.upper()[0], "0");
        // EE faz parte da digitação: 2 EE 3 é 2.000, e ± depois do EE troca o sinal do expoente.
        let mut r = Rpn::default();
        r.digit(2);
        r.exponent();
        r.digit(3);
        assert_eq!(r.display(), "2e3");
        r.enter();
        assert_eq!(r.display(), "2.000");
        r.digit(5);
        r.exponent();
        r.unary(Unary::Negate);
        r.digit(1);
        assert_eq!(r.display(), "5e-1");
        r.binary(Binary::Mul);
        assert_eq!(r.display(), "1.000");
        // O expoente tem até 3 algarismos e não passa do maior número.
        let mut r = Rpn::default();
        r.digit(2);
        r.exponent();
        for d in [3, 0, 7] {
            r.digit(d);
        }
        assert_eq!(r.display(), "2e307");
        let mut r = Rpn::default();
        r.digit(2);
        r.exponent();
        for d in [3, 0, 9] {
            r.digit(d);
        }
        assert_eq!(r.display(), "2e30");
        let mut r = Rpn::default();
        r.digit(2);
        r.exponent();
        for d in [1, 2, 3, 4] {
            r.digit(d);
        }
        assert_eq!(r.display(), "2e123");
    }
}
