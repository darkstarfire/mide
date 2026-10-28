//! Os valores das notas (número, porcentagem, ângulo, quantidade com unidade e
//! dinheiro), as regras de cada conta entre eles (tabela G) e o texto do resultado.
//! Soma, subtração e o resto passam pelo `Binary::apply` do teclado, para a precisão
//! ser a mesma (0,1 + 0,2 − 0,3 = 0).
//!
//! Unidades nesta etapa: uma unidade por valor. Soma e razão só na mesma grandeza (a
//! primeira unidade vence); × e ÷ por número. Contas que criariam outra grandeza
//! (km ÷ h, m × m) ficam para depois, com o aviso `MIXED`. A troca entre moedas
//! diferentes precisa das cotações e fica no `eval`; aqui as moedas já chegam iguais.

use std::sync::LazyLock;

use super::super::currency::{self, Currency};
use super::super::engine::Binary;
use super::super::format::Style;
use super::super::units;

// As mensagens ficam em inglês aqui; a nota mostra a tradução (`eval::problem`).
pub const DIV_ZERO: &str = "Division by zero";
pub const TOO_BIG: &str = "Number too large";
/// Conta entre unidades que criaria outra grandeza (km ÷ h, 30° × 30°).
pub const MIXED: &str = "Calculations between different units aren't supported yet";

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

    /// A mesma unidade na tabela de conversões.
    pub fn id(self) -> UnitId {
        match self {
            AngleUnit::Degrees => ANGLES.0,
            AngleUnit::Radians => ANGLES.1,
        }
    }

    fn suffix(self) -> &'static str {
        match self {
            AngleUnit::Degrees => "°",
            AngleUnit::Radians => " rad",
        }
    }
}

/// Uma unidade da tabela de conversões (`units::categories`): categoria e posição.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UnitId {
    pub category: usize,
    pub unit: usize,
}

/// Grau e radiano na tabela (os ângulos que viram `Value::Angle`).
static ANGLES: LazyLock<(UnitId, UnitId)> = LazyLock::new(|| {
    let id = |unit: &str| units::find("angulo", unit).map_or(UnitId { category: 0, unit: 0 }, |(category, unit)| UnitId { category, unit });
    (id("grau"), id("rad"))
});

impl UnitId {
    pub fn get(self) -> &'static units::Unit {
        &units::categories()[self.category].units[self.unit]
    }

    pub fn symbol(self) -> &'static str {
        self.get().symbol
    }

    /// Nome da grandeza nas mensagens, traduzido: "comprimento", "massa".
    fn grandeza(self) -> String {
        crate::i18n::tr(units::categories()[self.category].display).to_lowercase()
    }

    /// Temperatura: escalas com deslocamento, que só somam na mesma escala.
    fn temperature(self) -> bool {
        units::categories()[self.category].id == "temperatura"
    }

    /// Ângulo de grau ou radiano.
    fn angle_unit(self) -> Option<AngleUnit> {
        if self == ANGLES.0 {
            Some(AngleUnit::Degrees)
        } else if self == ANGLES.1 {
            Some(AngleUnit::Radians)
        } else {
            None
        }
    }

    /// O símbolo depois do número: colado nos de ângulo ("30°", "5′"), com espaço nos
    /// outros ("5,3 km", "25 °C").
    fn suffix(self) -> String {
        let symbol = self.symbol();
        if matches!(symbol, "°" | "′" | "″") { symbol.to_string() } else { format!(" {symbol}") }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Value {
    Num(f64),
    /// Porcentagem, guardada como fração ("10%" é 0,1): na soma e na subtração vale
    /// sobre o outro número (100 + 10% = 110). `shown`: veio de "em %" e aparece como
    /// "20%"; senão, aparece como número (50% = 0,5).
    Percent { v: f64, shown: bool },
    /// Ângulo em graus ou radianos: 30°, ou o que as inversas da trigonometria
    /// devolvem. Os outros ângulos (grado, volta) são `Qty`.
    Angle(f64, AngleUnit),
    /// Quantidade: o número na unidade (5 km é 5 e km, não 5.000 m).
    Qty(f64, UnitId),
    /// Diferença entre duas temperaturas (30 °C − 20 °C): aparece como a temperatura,
    /// mas converte só pelo tamanho do grau (10 °C de diferença são 18 °F, não 50 °F).
    Interval(f64, UnitId),
    /// Dinheiro: o valor na moeda.
    Money(f64, &'static Currency),
}

/// O que um valor mede, fora os números: uma unidade ou uma moeda.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Measure {
    Unit(UnitId),
    Money(&'static Currency),
}

impl Measure {
    /// A grandeza nas mensagens: "comprimento", "dinheiro".
    fn grandeza(self) -> String {
        match self {
            Measure::Unit(u) => u.grandeza(),
            Measure::Money(_) => crate::tr!("money").into_owned(),
        }
    }
}

impl Value {
    /// `v` na unidade `unit`; grau e radiano viram `Angle`.
    pub fn quantity(v: f64, unit: UnitId) -> Value {
        match unit.angle_unit() {
            Some(a) => Value::Angle(v, a),
            None => Value::Qty(v, unit),
        }
    }

    /// O número guardado: a fração da porcentagem, a quantidade na unidade dela.
    pub fn number(self) -> f64 {
        match self {
            Value::Num(v) | Value::Percent { v, .. } | Value::Angle(v, _) | Value::Qty(v, _) | Value::Interval(v, _) | Value::Money(v, _) => v,
        }
    }

    /// O número sem unidade; `None` para ângulo, quantidade e dinheiro.
    pub fn plain(self) -> Option<f64> {
        matches!(self, Value::Num(_) | Value::Percent { .. }).then_some(self.number())
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

    /// Ângulo em outra unidade (grado, volta) passado para radianos; o resto fica igual.
    pub fn as_angle(self) -> Value {
        match self {
            Value::Qty(v, u) if u.category == ANGLES.1.category => {
                units::convert(v, u.get(), ANGLES.1.get()).map_or(Value::Num(f64::NAN), |r| Value::Angle(r, AngleUnit::Radians))
            }
            other => other,
        }
    }

    fn measure(self) -> Option<Measure> {
        match self {
            Value::Angle(_, a) => Some(Measure::Unit(a.id())),
            Value::Qty(_, u) | Value::Interval(_, u) => Some(Measure::Unit(u)),
            Value::Money(_, c) => Some(Measure::Money(c)),
            Value::Num(_) | Value::Percent { .. } => None,
        }
    }

    /// O mesmo tipo de valor, com o número trocado por `f(número)`.
    pub fn map(self, f: impl FnOnce(f64) -> f64) -> Value {
        match self {
            Value::Num(v) => Value::Num(f(v)),
            Value::Percent { v, shown } => Value::Percent { v: f(v), shown },
            Value::Angle(v, unit) => Value::Angle(f(v), unit),
            Value::Qty(v, unit) => Value::Qty(f(v), unit),
            Value::Interval(v, unit) => Value::Interval(f(v), unit),
            Value::Money(v, c) => Value::Money(f(v), c),
        }
    }

    /// a + b (ou a − b), com as unidades já iguais (ver `sum`). Com porcentagem só à
    /// direita, vale sobre a: a × (1 ± p). Um número sem unidade entra na unidade do
    /// outro (R$ 1.200 + 300 = R$ 1.500); com ângulo, o resultado fica na unidade do
    /// primeiro ângulo.
    pub fn add(self, b: Value, sub: bool) -> Value {
        let op = if sub { Binary::Sub } else { Binary::Add };
        match (self, b) {
            (Value::Percent { v: p, shown }, Value::Percent { v: q, .. }) => Value::Percent { v: op.apply(p, q), shown },
            (a, Value::Percent { v: p, .. }) => a.map(|x| op.apply(x, x * p)),
            (a, b) => match a.angle().or(b.angle()) {
                Some(unit) => Value::Angle(op.apply(a.in_unit(unit), b.in_unit(unit)), unit),
                None if a.measure().is_some() => a.map(|x| op.apply(x, b.number())),
                None if b.measure().is_some() => b.map(|y| op.apply(a.number(), y)),
                None => Value::Num(op.apply(a.number(), b.number())),
            },
        }
    }

    /// a + b (ou a − b) conferindo as unidades: só na mesma grandeza, com b passado
    /// para a unidade de a (5 km + 300 m = 5,3 km). Temperatura só soma na mesma
    /// escala (20 °C + 5 °C; 20 °C + 5 °F não), e temperatura menos temperatura é a
    /// diferença entre elas (`Interval`); temperatura mais diferença é temperatura.
    pub fn sum(self, b: Value, sub: bool) -> Result<Value, String> {
        let aligned = match (self, b) {
            // Graus e radianos: o `add` passa de um para o outro.
            (Value::Angle(..), Value::Angle(..)) => b,
            _ => match (self.measure(), b.measure()) {
                (Some(ma), Some(mb)) => self.align(b, ma, mb)?,
                _ => b,
            },
        };
        Ok(match (self, b, self.add(aligned, sub)) {
            (Value::Qty(_, x), Value::Qty(..), r) if sub && x.temperature() => Value::Interval(r.number(), x),
            (Value::Interval(..), Value::Qty(_, y), r) if !sub => Value::Qty(r.number(), y),
            (_, _, r) => r,
        })
    }

    /// `b` na unidade de `self` (mesma grandeza), ou a mensagem de por que não dá.
    fn align(self, b: Value, ma: Measure, mb: Measure) -> Result<Value, String> {
        match (ma, mb) {
            (Measure::Money(x), Measure::Money(y)) if x.code == y.code => Ok(b),
            (Measure::Unit(x), Measure::Unit(y)) if x == y => Ok(b),
            (Measure::Unit(x), Measure::Unit(y)) if x.category == y.category && x.temperature() => {
                Err(crate::tr!("Can't add {a} and {b}", a = x.symbol(), b = y.symbol()))
            }
            (Measure::Unit(x), Measure::Unit(y)) if x.category == y.category => {
                units::convert(b.number(), y.get(), x.get()).map(|v| Value::quantity(v, x)).ok_or_else(|| TOO_BIG.to_string())
            }
            _ => Err(crate::tr!("Can't add {a} and {b}", a = ma.grandeza(), b = mb.grandeza())),
        }
    }

    /// a × b: com unidade só de um lado, ela fica (5 km × 2 = 10 km; 30° × 2 = 60°);
    /// dos dois lados, ainda não (m × m daria m²).
    pub fn times(self, b: Value) -> Result<Value, &'static str> {
        let v = Binary::Mul.apply(self.number(), b.number());
        match (self.measure().is_some(), b.measure().is_some()) {
            (true, true) => Err(MIXED),
            (true, false) => Ok(self.map(|_| v)),
            (false, true) => Ok(b.map(|_| v)),
            (false, false) => Ok(Value::Num(v)),
        }
    }

    /// a ÷ b: duas medidas da mesma grandeza dão o número da razão (10 km ÷ 2 km = 5);
    /// medida ÷ número continua medida; número ÷ medida, ou grandezas diferentes
    /// (km ÷ h), ainda não.
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
        match (self.measure(), b.measure()) {
            (None, Some(_)) => Err(MIXED),
            (Some(_), None) => Ok(self.map(|x| f(x, b.number()))),
            (None, None) => Ok(Value::Num(f(self.number(), b.number()))),
            (Some(ma), Some(mb)) => {
                let y = match (self, b) {
                    (Value::Angle(_, unit), Value::Angle(..)) => b.in_unit(unit),
                    _ => self.align(b, ma, mb).map_err(|_| MIXED)?.number(),
                };
                if y == 0.0 { Err(DIV_ZERO) } else { Ok(Value::Num(f(self.number(), y))) }
            }
        }
    }

    /// O número deste valor na unidade de `of` (a mesma grandeza; moedas já iguais),
    /// para comparar (mín e máx); `None` se não dá. Temperaturas de escalas diferentes
    /// se comparam pela conversão (70 °F são 21,1 °C), que a soma não aceita.
    pub fn number_in(self, of: Value) -> Option<f64> {
        match (of, self) {
            (Value::Angle(_, unit), Value::Angle(..)) => Some(self.in_unit(unit)),
            (Value::Qty(_, x), Value::Qty(v, y)) if x.category == y.category => units::convert(v, y.get(), x.get()),
            _ => match (of.measure(), self.measure()) {
                (None, None) => Some(self.number()),
                (Some(ma), Some(mb)) => of.align(self, ma, mb).ok().map(Value::number),
                _ => None,
            },
        }
    }

    /// "em %": a porcentagem que aparece como tal (0,2 vira "20%").
    pub fn to_percent(self) -> Option<Value> {
        self.plain().map(|v| Value::Percent { v, shown: true })
    }

    /// "em km": a quantidade em outra unidade da mesma grandeza (25 °C em °F = 77 °F).
    pub fn convert_to(self, to: UnitId) -> Result<Value, String> {
        let from = match self {
            Value::Angle(_, a) => a.id(),
            Value::Qty(_, u) | Value::Interval(_, u) => u,
            _ => return Err(crate::tr!("Can't convert {from} to {to}", from = self.label(), to = to.symbol())),
        };
        if from.category != to.category {
            return Err(crate::tr!("Can't convert {from} to {to}", from = from.symbol(), to = to.symbol()));
        }
        // Sem resultado: consumo zero (L/100 km ↔ km/L) ou fora do f64.
        let fail = if self.number() == 0.0 { DIV_ZERO } else { TOO_BIG };
        if let Value::Interval(v, _) = self {
            // Só o tamanho do grau: a conversão de v menos a do zero.
            let at = |x: f64| units::convert(x, from.get(), to.get());
            return at(v).zip(at(0.0)).map(|(a, z)| Value::Interval(a - z, to)).ok_or_else(|| fail.to_string());
        }
        units::convert(self.number(), from.get(), to.get()).map(|v| Value::quantity(v, to)).ok_or_else(|| fail.to_string())
    }

    /// O nome da unidade nas mensagens: "número" (traduzido), "°", "km", "R$".
    pub fn label(self) -> std::borrow::Cow<'static, str> {
        match self {
            Value::Num(_) | Value::Percent { .. } => crate::tr!("number"),
            Value::Angle(_, AngleUnit::Degrees) => "°".into(),
            Value::Angle(_, AngleUnit::Radians) => "rad".into(),
            Value::Qty(_, u) | Value::Interval(_, u) => u.symbol().into(),
            Value::Money(_, c) => c.symbol().into(),
        }
    }

    /// O texto do valor: "1.514", "20%", "30°", "5,3 km", "R$ 1.500,00".
    pub fn text(self, style: Style) -> String {
        self.show(style).0
    }

    /// O número que aparece no texto, sem a unidade (o 20 de "20%").
    pub fn shown_number(self) -> f64 {
        match self {
            Value::Percent { v, shown: true } => v * 100.0,
            other => other.number(),
        }
    }

    fn suffix(self) -> String {
        match self {
            Value::Percent { shown: true, .. } => "%".into(),
            Value::Angle(_, unit) => unit.suffix().into(),
            Value::Qty(_, unit) | Value::Interval(_, unit) => unit.suffix(),
            _ => String::new(),
        }
    }

    /// Texto, número sem ponto de milhar ("1514") e valor, para o resultado. O número
    /// é escrito uma vez só (o `Style::value` é a parte cara) e ganha o milhar depois.
    /// Dinheiro vai com o símbolo na frente e as casas da moeda ("R$ 1.234,50"), ou as
    /// do Visualizar.
    pub fn show(self, style: Style) -> (String, String, f64) {
        let n = self.shown_number();
        let plain = Style { grouping: false, ..style };
        if let Value::Money(v, c) = self {
            let number = currency::format_amount(v, c.code, &plain);
            return (format!("{} {}", c.symbol(), style.digits(&number)), number, n);
        }
        let number = plain.value(n);
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

    fn unit(category: &str, id: &str) -> UnitId {
        let (category, unit) = units::find(category, id).expect("unidade");
        UnitId { category, unit }
    }

    fn money(code: &str) -> &'static Currency {
        currency::find(code).expect("moeda")
    }

    #[test]
    fn quantidades() {
        let s = Style::default();
        let (km, m, kg) = (unit("comprimento", "km"), unit("comprimento", "m"), unit("massa", "kg"));
        let d = Value::Qty(5.0, km);
        assert_eq!(d.sum(Value::Qty(300.0, m), false).map(|v| v.text(s)), Ok("5,3 km".into()));
        assert_eq!(d.sum(Value::Num(2.0), false), Ok(Value::Qty(7.0, km)));
        assert_eq!(Value::Num(2.0).sum(d, false), Ok(Value::Qty(7.0, km)));
        assert_eq!(d.sum(Value::Qty(1.0, kg), false), Err("Não dá para somar comprimento com massa".into()));
        assert_eq!(d.times(Value::Num(2.0)), Ok(Value::Qty(10.0, km)));
        assert_eq!(d.times(d), Err(MIXED));
        assert_eq!(d.over(Value::Qty(2.0, km)), Ok(Value::Num(2.5)));
        assert_eq!(d.over(Value::Qty(500.0, m)), Ok(Value::Num(10.0)));
        assert_eq!(d.over(Value::Qty(2.0, kg)), Err(MIXED));
        assert_eq!(Value::Num(1.0).over(d), Err(MIXED));
        assert_eq!(d.over(Value::Qty(0.0, m)), Err(DIV_ZERO));
        assert_eq!(d.convert_to(m).map(|v| v.text(s)), Ok("5.000 m".into()));
        assert_eq!(d.convert_to(kg), Err("Não dá para converter km em kg".into()));
        assert_eq!(Value::Num(2.0).convert_to(kg), Err("Não dá para converter número em kg".into()));
        // Porcentagem sobre a quantidade.
        assert_eq!(Value::Qty(200.0, m).sum(Value::Percent { v: 0.1, shown: false }, true), Ok(Value::Qty(180.0, m)));
    }

    #[test]
    fn temperatura_e_angulos_da_tabela() {
        let s = Style::default();
        let (c, f) = (unit("temperatura", "c"), unit("temperatura", "f"));
        assert_eq!(Value::Qty(25.0, c).convert_to(f).map(|v| v.text(s)), Ok("77 °F".into()));
        assert_eq!(Value::Qty(20.0, c).sum(Value::Qty(5.0, c), false), Ok(Value::Qty(25.0, c)));
        assert_eq!(Value::Qty(20.0, c).sum(Value::Qty(5.0, f), false), Err("Não dá para somar °C com °F".into()));
        // Temperatura menos temperatura é a diferença, que converte só pelo grau.
        let diff = Value::Qty(30.0, c).sum(Value::Qty(20.0, c), true);
        assert_eq!(diff, Ok(Value::Interval(10.0, c)));
        let in_f = Value::Interval(10.0, c).convert_to(f).map(|v| v.text(s));
        assert!(in_f.as_deref().is_ok_and(|t| t.starts_with("18") && t.ends_with(" °F")), "{in_f:?}");
        assert_eq!(Value::Qty(20.0, c).sum(Value::Interval(10.0, c), false), Ok(Value::Qty(30.0, c)));
        assert_eq!(Value::Interval(10.0, c).sum(Value::Qty(20.0, c), false), Ok(Value::Qty(30.0, c)));
        assert_eq!(Value::Interval(10.0, c).text(s), "10 °C");
        // mín e máx: escalas diferentes comparam pela conversão.
        let seventy = Value::Qty(70.0, f).number_in(Value::Qty(20.0, c)).unwrap_or(0.0);
        assert!((seventy - 21.111_111_111).abs() < 1e-6);
        // Grau e radiano da tabela viram `Angle`.
        assert_eq!(Value::quantity(30.0, unit("angulo", "grau")), Value::Angle(30.0, DEG));
        let rad = Value::Angle(30.0, DEG).convert_to(unit("angulo", "rad"));
        assert_eq!(rad.map(|v| v.text(s)), Ok("0,5235987756 rad".into()));
        let turn = Value::Qty(0.25, unit("angulo", "volta")).as_angle();
        assert_eq!(turn.angle(), Some(AngleUnit::Radians));
        assert!((turn.number() - std::f64::consts::FRAC_PI_2).abs() < 1e-15);
        assert_eq!(Value::Angle(30.0, DEG).sum(Value::Qty(1.0, unit("comprimento", "m")), false), Err("Não dá para somar ângulo com comprimento".into()));
    }

    #[test]
    fn dinheiro() {
        let s = Style::default();
        let (brl, usd) = (money("BRL"), money("USD"));
        let r = Value::Money(1200.0, brl);
        assert_eq!(r.sum(Value::Num(300.0), false).map(|v| v.text(s)), Ok("R$ 1.500,00".into()));
        assert_eq!(r.sum(Value::Money(5.0, brl), false), Ok(Value::Money(1205.0, brl)));
        assert_eq!(r.sum(Value::Qty(1.0, unit("massa", "kg")), false), Err("Não dá para somar dinheiro com massa".into()));
        assert_eq!(r.times(Value::Num(12.0)).map(|v| v.text(s)), Ok("R$ 14.400,00".into()));
        assert_eq!(r.over(Value::Money(600.0, brl)), Ok(Value::Num(2.0)));
        assert_eq!(r.times(Value::Money(1.0, brl)), Err(MIXED));
        let (text, number, value) = Value::Money(1234.5, usd).show(s);
        assert_eq!((text.as_str(), number.as_str(), value), ("US$ 1.234,50", "1234,50", 1234.5));
        assert_eq!(Value::Money(1234.5, money("JPY")).text(s), "¥ 1.235");
        assert_eq!(Value::Money(2.0, brl).text(Style { decimals: Some(3), grouping: true }), "R$ 2,000");
    }
}
