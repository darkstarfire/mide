//! Os valores das notas (número, porcentagem e ângulo), as regras de cada conta entre
//! eles (tabela G) e o texto do resultado. Soma, subtração e o resto passam pelo
//! `Binary::apply` do teclado, para a precisão ser a mesma (0,1 + 0,2 − 0,3 = 0). As
//! unidades e as moedas (etapas N2 e N3) entram aqui como novas variantes.

use super::super::engine::Binary;
use super::super::format::Style;

pub const DIV_ZERO: &str = "Divisão por zero";
pub const TOO_BIG: &str = "Número grande demais";
/// Conta que junta um ângulo com o que não combina (30° × 30°).
pub const MIXED: &str = "Conta entre unidades diferentes: ainda não";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AngleUnit {
    Degrees,
    Radians,
}

impl AngleUnit {
    /// Nesta unidade, o ângulo `v` medido em `from`.
    pub fn from(self, v: f64, from: AngleUnit) -> f64 {
        match (from, self) {
            (AngleUnit::Degrees, AngleUnit::Radians) => v.to_radians(),
            (AngleUnit::Radians, AngleUnit::Degrees) => v.to_degrees(),
            _ => v,
        }
    }

    fn suffix(self) -> &'static str {
        match self {
            AngleUnit::Degrees => "°",
            AngleUnit::Radians => " rad",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Value {
    Num(f64),
    /// Porcentagem, guardada como fração ("10%" é 0,1): na soma e na subtração vale
    /// sobre o outro número (100 + 10% = 110). `shown`: veio de "em %" e aparece como
    /// "20%"; senão, aparece como número (50% = 0,5).
    Percent { v: f64, shown: bool },
    /// Ângulo: 30°, ou o que as inversas da trigonometria devolvem.
    Angle(f64, AngleUnit),
}

impl Value {
    /// O número guardado: a fração da porcentagem, o ângulo na unidade dele.
    pub fn number(self) -> f64 {
        match self {
            Value::Num(v) | Value::Percent { v, .. } | Value::Angle(v, _) => v,
        }
    }

    /// O número sem unidade; `None` para ângulo.
    pub fn plain(self) -> Option<f64> {
        (!matches!(self, Value::Angle(..))).then_some(self.number())
    }

    pub fn angle(self) -> Option<AngleUnit> {
        match self {
            Value::Angle(_, unit) => Some(unit),
            _ => None,
        }
    }

    /// O número em `unit`, se for ângulo; senão, o próprio número.
    pub fn in_unit(self, unit: AngleUnit) -> f64 {
        match self {
            Value::Angle(v, from) => unit.from(v, from),
            other => other.number(),
        }
    }

    /// O mesmo tipo de valor, com o número trocado por `f(número)`.
    pub fn map(self, f: impl FnOnce(f64) -> f64) -> Value {
        match self {
            Value::Num(v) => Value::Num(f(v)),
            Value::Percent { v, shown } => Value::Percent { v: f(v), shown },
            Value::Angle(v, unit) => Value::Angle(f(v), unit),
        }
    }

    /// a + b (ou a − b). Com porcentagem só à direita, vale sobre a: a × (1 ± p). Com
    /// ângulo, o resultado fica na unidade do primeiro ângulo.
    pub fn add(self, b: Value, sub: bool) -> Value {
        let op = if sub { Binary::Sub } else { Binary::Add };
        match (self, b) {
            (Value::Percent { v: p, shown }, Value::Percent { v: q, .. }) => Value::Percent { v: op.apply(p, q), shown },
            (a, Value::Percent { v: p, .. }) => a.map(|x| op.apply(x, x * p)),
            (a, b) => match a.angle().or(b.angle()) {
                Some(unit) => Value::Angle(op.apply(a.in_unit(unit), b.in_unit(unit)), unit),
                None => Value::Num(op.apply(a.number(), b.number())),
            },
        }
    }

    /// a × b: ângulo vezes número continua ângulo; dois ângulos ainda não.
    pub fn times(self, b: Value) -> Result<Value, &'static str> {
        let v = Binary::Mul.apply(self.number(), b.number());
        match (self.angle(), b.angle()) {
            (Some(_), Some(_)) => Err(MIXED),
            (Some(unit), None) | (None, Some(unit)) => Ok(Value::Angle(v, unit)),
            (None, None) => Ok(Value::Num(v)),
        }
    }

    /// a ÷ b: dois ângulos dão o número da razão; número ÷ ângulo ainda não.
    pub fn over(self, b: Value) -> Result<Value, &'static str> {
        self.ratio(b, |x, y| Binary::Div.apply(x, y))
    }

    /// O resto de a ÷ b, com o sinal de a (−7 mod 3 = −1), como no Programador.
    pub fn modulo(self, b: Value) -> Result<Value, &'static str> {
        self.ratio(b, |x, y| x % y)
    }

    fn ratio(self, b: Value, f: impl Fn(f64, f64) -> f64) -> Result<Value, &'static str> {
        if b.number() == 0.0 {
            return Err(DIV_ZERO);
        }
        match (self.angle(), b.angle()) {
            (None, Some(_)) => Err(MIXED),
            (Some(unit), Some(_)) => Ok(Value::Num(f(self.number(), b.in_unit(unit)))),
            (Some(unit), None) => Ok(Value::Angle(f(self.number(), b.number()), unit)),
            (None, None) => Ok(Value::Num(f(self.number(), b.number()))),
        }
    }

    /// "em %": a porcentagem que aparece como tal (0,2 vira "20%").
    pub fn to_percent(self) -> Option<Value> {
        self.plain().map(|v| Value::Percent { v, shown: true })
    }

    /// O texto do valor: "1.514", "20%", "30°", "0,523598775598 rad".
    pub fn text(self, style: Style) -> String {
        format!("{}{}", style.value(self.shown_number()), self.suffix())
    }

    /// O número que aparece no texto, sem a unidade (o 20 de "20%").
    pub fn shown_number(self) -> f64 {
        match self {
            Value::Percent { v, shown: true } => v * 100.0,
            other => other.number(),
        }
    }

    fn suffix(self) -> &'static str {
        match self {
            Value::Percent { shown: true, .. } => "%",
            Value::Angle(_, unit) => unit.suffix(),
            _ => "",
        }
    }

    /// Texto, número sem ponto de milhar ("1514") e valor, para o resultado. O número
    /// é escrito uma vez só (o `Style::value` é a parte cara) e ganha o milhar depois.
    pub fn show(self, style: Style) -> (String, String, f64) {
        let n = self.shown_number();
        let number = Style { grouping: false, ..style }.value(n);
        let text = format!("{}{}", style.digits(&number), self.suffix());
        (text, number, n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEG: AngleUnit = AngleUnit::Degrees;

    #[test]
    fn porcentagem_soma_sobre_o_outro() {
        let p = |v| Value::Percent { v, shown: false };
        assert_eq!(Value::Num(100.0).add(p(0.1), false), Value::Num(110.0));
        assert_eq!(Value::Num(150.0).add(p(0.2), true), Value::Num(120.0));
        assert_eq!(p(0.1).add(p(0.05), false), p(0.15000000000000002));
        assert_eq!(Value::Num(200.0).times(p(0.15)), Ok(Value::Num(30.0)));
        assert_eq!(Value::Num(0.2).to_percent().map(|v| v.text(Style::default())), Some("20%".into()));
        assert_eq!(p(0.5).text(Style::default()), "0,5");
    }

    #[test]
    fn angulos() {
        let a = Value::Angle(30.0, DEG);
        assert_eq!(a.add(Value::Num(15.0), false), Value::Angle(45.0, DEG));
        assert_eq!(a.times(Value::Num(2.0)), Ok(Value::Angle(60.0, DEG)));
        assert_eq!(a.times(a), Err(MIXED));
        assert_eq!(a.over(a), Ok(Value::Num(1.0)));
        assert_eq!(Value::Num(1.0).over(a), Err(MIXED));
        assert_eq!(a.over(Value::Num(0.0)), Err(DIV_ZERO));
        assert_eq!(a.text(Style::default()), "30°");
        assert_eq!(Value::Angle(0.5, AngleUnit::Radians).text(Style::default()), "0,5 rad");
        assert_eq!(a.to_percent(), None);
    }

    #[test]
    fn resto_com_sinal_de_quem_e_dividido() {
        assert_eq!(Value::Num(-7.0).modulo(Value::Num(3.0)), Ok(Value::Num(-1.0)));
        assert_eq!(Value::Num(10.0).modulo(Value::Num(0.0)), Err(DIV_ZERO));
    }

    #[test]
    fn texto_do_resultado() {
        let s = Style::default();
        assert_eq!(Value::Num(1234.5).show(s), ("1.234,5".into(), "1234,5".into(), 1234.5));
        let shown = Value::Percent { v: 0.2, shown: true }.show(s);
        assert_eq!((shown.0.as_str(), shown.1.as_str()), ("20%", "20"));
    }
}
