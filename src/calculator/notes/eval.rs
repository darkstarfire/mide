//! O valor de uma conta já lida. As contas são as do teclado da Calculadora
//! (`Binary::apply`, `Unary::apply`, `factorial`), para a precisão ser a mesma
//! (0,1 + 0,2 − 0,3 = 0; sen 1800° = 0). Cada erro sai com o trecho do nó que falhou.

use std::collections::HashMap;
use std::ops::Range;

use super::super::engine::{Binary, Unary};
use super::parser::{Expr, Func, Node, Op, Post};
use super::value::{AngleUnit, DIV_ZERO, MIXED, TOO_BIG, Value};
use super::{Context, Problem};

const NEGATIVE_ROOT: &str = "Raiz de número negativo";

/// As variáveis definidas até aqui: a chave (sem acento nem maiúscula) e o valor, ou
/// a linha (contando de 0) da definição que deu erro.
#[derive(Debug, Default)]
pub struct Env {
    vars: HashMap<String, Result<Value, usize>>,
}

impl Env {
    pub fn get(&self, key: &str) -> Option<&Result<Value, usize>> {
        self.vars.get(key)
    }

    pub fn set(&mut self, key: String, value: Result<Value, usize>) {
        self.vars.insert(key, value);
    }
}

/// Avaliação dentro de uma nota: as variáveis, Graus/Radianos e o `Style`, e o texto
/// (para os nomes nas mensagens, como foram escritos).
pub struct Eval<'a> {
    pub env: &'a Env,
    pub ctx: &'a Context,
    pub text: &'a str,
}

fn problem(e: &Expr, message: impl Into<String>) -> Problem {
    Problem { range: e.range.clone(), message: message.into() }
}

impl Eval<'_> {
    /// O valor da conta. A espinha da esquerda ("1 + 1 + … + 1", "5!!!") desce num
    /// laço e volta calculando: numa linha comprida ela tem mil nós, e a recursão
    /// estouraria a pilha. O resto da árvore tem a altura que a leitura limita.
    pub fn value(&self, e: &Expr) -> Result<Value, Problem> {
        let mut spine = Vec::new();
        let mut bottom = e;
        while let Some(left) = bottom.node.left() {
            spine.push(bottom);
            bottom = left;
        }
        let mut v = self.node(bottom, None)?;
        for e in spine.into_iter().rev() {
            v = self.node(e, Some(v))?;
        }
        Ok(v)
    }

    /// O valor de um nó; `left`: o do filho na espinha, já calculado.
    fn node(&self, e: &Expr, left: Option<Value>) -> Result<Value, Problem> {
        let v = match (&e.node, left) {
            (Node::Binary(op, _, b), Some(x)) => self.binary(*op, x, b, e)?,
            (Node::Post(post, _), Some(x)) => self.post(*post, x, e)?,
            (Node::ToPercent(_), Some(x)) => x.to_percent().ok_or_else(|| problem(e, format!("Não dá para converter {} em %", unit_name(x))))?,
            // A espinha chega sempre com o filho calculado.
            (Node::Binary(..) | Node::Post(..) | Node::ToPercent(_), None) => return self.value(e),
            (Node::Num(v), _) => Value::Num(*v),
            (Node::Var { key, name }, _) => self.var(key, name)?,
            (Node::Const(c), _) => Value::Num(c.value()),
            (Node::Neg(x), _) => self.value(x)?.map(|v| -v),
            (Node::Root(n, x), _) => self.root(*n, self.value(x)?, e)?,
            (Node::Call { func, name, args }, _) => self.call(*func, name, args, e)?,
        };
        // Os nós de cada conta tratam o que não existe (NaN); o infinito é número grande.
        if v.number().is_finite() { Ok(v) } else { Err(problem(e, TOO_BIG)) }
    }

    /// Variável; o erro sublinha o nome, como foi escrito.
    fn var(&self, key: &str, name: &Range<usize>) -> Result<Value, Problem> {
        let written = self.text.get(name.clone()).unwrap_or("");
        let message = match self.env.get(key) {
            Some(Ok(v)) => return Ok(*v),
            Some(Err(line)) => format!("“{written}” tem um erro (linha {})", line + 1),
            None => format!("“{written}” não é uma variável nem uma unidade"),
        };
        Err(Problem { range: name.clone(), message })
    }

    /// `x` (já calculado) com o lado direito `b`.
    fn binary(&self, op: Op, x: Value, b: &Expr, e: &Expr) -> Result<Value, Problem> {
        let y = self.value(b)?;
        let r = match op {
            Op::Add => Ok(x.add(y, false)),
            Op::Sub => Ok(x.add(y, true)),
            Op::Mul => x.times(y),
            Op::Div => x.over(y),
            Op::Mod => x.modulo(y),
            Op::Pow => return self.power(x, y, e),
        };
        r.map_err(|m| problem(e, m))
    }

    /// xʸ só entre números: 0 elevado a negativo é divisão por zero, e negativo
    /// elevado a fração é raiz de negativo.
    fn power(&self, x: Value, y: Value, e: &Expr) -> Result<Value, Problem> {
        let (Some(a), Some(b)) = (x.plain(), y.plain()) else {
            return Err(problem(e, MIXED));
        };
        if a == 0.0 && b < 0.0 {
            return Err(problem(e, DIV_ZERO));
        }
        let r = Binary::Pow.apply(a, b);
        if r.is_nan() { Err(problem(e, NEGATIVE_ROOT)) } else { Ok(Value::Num(r)) }
    }

    fn post(&self, post: Post, v: Value, e: &Expr) -> Result<Value, Problem> {
        let Some(n) = v.plain() else {
            return Err(problem(e, MIXED));
        };
        match post {
            Post::Fact => self.plain_func(Func::Fact, "fat", n, e),
            Post::Percent => Ok(Value::Percent { v: n / 100.0, shown: false }),
            Post::Degree => Ok(Value::Angle(n, AngleUnit::Degrees)),
            Post::Power(p) => self.power(v, Value::Num(p as f64), e),
        }
    }

    fn root(&self, n: u8, v: Value, e: &Expr) -> Result<Value, Problem> {
        let Some(x) = v.plain() else {
            return Err(problem(e, MIXED));
        };
        if n == 2 && x < 0.0 {
            return Err(problem(e, NEGATIVE_ROOT));
        }
        let f = if n == 2 { Unary::Sqrt } else { Unary::Cbrt };
        Ok(Value::Num(f.apply(x, false)))
    }

    fn call(&self, func: Func, name: &str, args: &[Expr], e: &Expr) -> Result<Value, Problem> {
        let values = args.iter().map(|a| self.value(a)).collect::<Result<Vec<_>, _>>()?;
        match (func, values.as_slice()) {
            (Func::Min | Func::Max, vs) if vs.len() >= 2 => self.extreme(func == Func::Max, vs, e),
            (Func::Hypot, &[a, b]) => self.two(name, a, b, e, |x, y| Ok(x.hypot(y))),
            (Func::Sqrt, &[a, n]) => self.two(name, a, n, e, nth_root),
            (Func::Log, &[a, b]) => self.two(name, a, b, e, log_base),
            (Func::Round, &[a, d]) => self.round(name, a, d, e),
            (_, &[a]) => self.one(func, name, a, e),
            _ => Err(problem(e, arity_message(name, func))),
        }
    }

    /// Funções de um número.
    fn one(&self, func: Func, name: &str, v: Value, e: &Expr) -> Result<Value, Problem> {
        match func {
            Func::Sin | Func::Cos | Func::Tan => self.trig(func, name, v, e),
            Func::Asin | Func::Acos | Func::Atan => self.inverse(func, name, v, e),
            Func::Abs => Ok(v.map(f64::abs)),
            Func::Round => Ok(v.map(|x| round_to(x, 0.0).unwrap_or(x))),
            Func::Floor => Ok(v.map(f64::floor)),
            Func::Ceil => Ok(v.map(f64::ceil)),
            Func::Trunc => Ok(v.map(f64::trunc)),
            _ => match v.plain() {
                Some(x) => self.plain_func(func, name, x, e),
                None => Err(self.missing(name, v, e)),
            },
        }
    }

    /// "{função} não existe para {valor}".
    fn missing(&self, name: &str, v: Value, e: &Expr) -> Problem {
        problem(e, format!("{name} não existe para {}", v.text(self.ctx.style)))
    }

    /// Funções de número sem unidade, pela tecla do teclado. O que não existe (NaN, ou
    /// infinito no ponto onde a função não existe, como ln 0) dá a mensagem da
    /// função; o resto do infinito é número grande.
    fn plain_func(&self, func: Func, name: &str, x: f64, e: &Expr) -> Result<Value, Problem> {
        if func == Func::Sqrt && x < 0.0 {
            return Err(problem(e, NEGATIVE_ROOT));
        }
        let Some(key) = unary(func) else {
            return Err(self.missing(name, Value::Num(x), e));
        };
        let r = key.apply(x, self.ctx.radians);
        let singular = matches!(func, Func::Ln | Func::Log | Func::Log2 | Func::Atanh) && r.is_infinite();
        if r.is_nan() || singular {
            return Err(self.missing(name, Value::Num(x), e));
        }
        Ok(Value::Num(r))
    }

    /// Seno, cosseno e tangente: o ângulo na unidade dele ("30°" vale também em
    /// Radianos) ou o número no modo da Calculadora.
    fn trig(&self, func: Func, name: &str, v: Value, e: &Expr) -> Result<Value, Problem> {
        let (x, radians) = match v {
            Value::Angle(x, unit) => (x, unit == AngleUnit::Radians),
            other => (other.number(), self.ctx.radians),
        };
        let r = unary(func).map_or(f64::NAN, |key| key.apply(x, radians));
        if r.is_nan() {
            let angle = if radians { v } else { Value::Angle(x, AngleUnit::Degrees) };
            return Err(self.missing(name, angle, e));
        }
        Ok(Value::Num(r))
    }

    /// Arco-seno, arco-cosseno e arco-tangente: um ângulo na unidade do modo.
    fn inverse(&self, func: Func, name: &str, v: Value, e: &Expr) -> Result<Value, Problem> {
        let Some(x) = v.plain() else {
            return Err(self.missing(name, v, e));
        };
        let r = unary(func).map_or(f64::NAN, |key| key.apply(x, self.ctx.radians));
        if r.is_nan() {
            return Err(self.missing(name, v, e));
        }
        let unit = if self.ctx.radians { AngleUnit::Radians } else { AngleUnit::Degrees };
        Ok(Value::Angle(r, unit))
    }

    /// Função de dois números sem unidade; `f` devolve `Err(i)` quando o i-ésimo
    /// número está fora do domínio.
    fn two(&self, name: &str, a: Value, b: Value, e: &Expr, f: impl Fn(f64, f64) -> Result<f64, usize>) -> Result<Value, Problem> {
        let (Some(x), Some(y)) = (a.plain(), b.plain()) else {
            return Err(self.missing(name, if a.plain().is_none() { a } else { b }, e));
        };
        match f(x, y) {
            Ok(r) if r.is_nan() && x < 0.0 => Err(problem(e, NEGATIVE_ROOT)),
            Ok(r) => Ok(Value::Num(r)),
            Err(i) => Err(self.missing(name, if i == 0 { a } else { b }, e)),
        }
    }

    /// arred(x; casas): as casas são um número inteiro (negativo arredonda as dezenas).
    fn round(&self, name: &str, v: Value, digits: Value, e: &Expr) -> Result<Value, Problem> {
        let rounded = digits.plain().and_then(|d| round_to(v.number(), d));
        match rounded {
            Some(r) => Ok(v.map(|_| r)),
            None => Err(self.missing(name, digits, e)),
        }
    }

    /// mín e máx: todos números ou todos ângulos (comparados em radianos).
    fn extreme(&self, max: bool, vs: &[Value], e: &Expr) -> Result<Value, Problem> {
        let angles = vs.iter().filter(|v| v.angle().is_some()).count();
        if angles != 0 && angles != vs.len() {
            return Err(problem(e, MIXED));
        }
        let key = |v: &Value| v.in_unit(AngleUnit::Radians);
        let best = vs.iter().copied().reduce(|a, b| {
            let better = if max { key(&b) > key(&a) } else { key(&b) < key(&a) };
            if better { b } else { a }
        });
        best.ok_or_else(|| problem(e, MIXED))
    }
}

/// A tecla do teclado para cada função de um número (as outras são tratadas antes).
fn unary(func: Func) -> Option<Unary> {
    Some(match func {
        Func::Sin => Unary::Sin,
        Func::Cos => Unary::Cos,
        Func::Tan => Unary::Tan,
        Func::Asin => Unary::Asin,
        Func::Acos => Unary::Acos,
        Func::Atan => Unary::Atan,
        Func::Sinh => Unary::Sinh,
        Func::Cosh => Unary::Cosh,
        Func::Tanh => Unary::Tanh,
        Func::Asinh => Unary::Asinh,
        Func::Acosh => Unary::Acosh,
        Func::Atanh => Unary::Atanh,
        Func::Sqrt => Unary::Sqrt,
        Func::Cbrt => Unary::Cbrt,
        Func::Ln => Unary::Ln,
        Func::Log => Unary::Log10,
        Func::Log2 => Unary::Log2,
        Func::Exp => Unary::Exp,
        Func::Fact => Unary::Fact,
        _ => return None,
    })
}

/// raiz(x; n), com a raiz ímpar de negativo (raiz(−8; 3) = −2).
fn nth_root(x: f64, n: f64) -> Result<f64, usize> {
    if n == 0.0 { Err(1) } else { Ok(Binary::Root.apply(x, n)) }
}

/// log(x; base).
fn log_base(x: f64, base: f64) -> Result<f64, usize> {
    if x <= 0.0 {
        Err(0)
    } else if base <= 0.0 || base == 1.0 {
        Err(1)
    } else {
        Ok(Binary::LogBase.apply(x, base))
    }
}

/// `x` com `digits` casas, o meio para longe do zero (arred(2,5) = 3, arred(−2,5) =
/// −3), sem o erro do f64 (2,675 × 100 dá 267,49999…: a folga de 4 ε acerta). `None`
/// se as casas não são inteiras.
pub fn round_to(x: f64, digits: f64) -> Option<f64> {
    if digits.fract() != 0.0 || !digits.is_finite() {
        return None;
    }
    let away = |v: f64| (v + v.signum() * v.abs() * 4.0 * f64::EPSILON).round();
    Some(match digits {
        d if d > 15.0 => x,
        d if d < -308.0 => 0.0,
        d if d >= 0.0 => {
            let f = 10f64.powi(d as i32);
            away(x * f) / f
        }
        d => {
            let f = 10f64.powi(-d as i32);
            away(x / f) * f
        }
    })
}

/// O nome da unidade de um valor, para as mensagens de conversão.
fn unit_name(v: Value) -> &'static str {
    match v.angle() {
        Some(AngleUnit::Degrees) => "°",
        Some(AngleUnit::Radians) => "rad",
        None => "número",
    }
}

/// "{função} precisa de …", pelo número de argumentos que ela aceita.
pub fn arity_message(name: &str, func: Func) -> String {
    let need = match func.arity() {
        (1, 1) => "1 número".to_string(),
        (1, 2) => "1 ou 2 números".to_string(),
        (min, usize::MAX) => format!("{min} números ou mais"),
        (min, _) => format!("{min} números"),
    };
    format!("{name} precisa de {need}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arredondar_longe_do_zero() {
        assert_eq!(round_to(2.567, 2.0), Some(2.57));
        assert_eq!(round_to(2.5, 0.0), Some(3.0));
        assert_eq!(round_to(-2.5, 0.0), Some(-3.0));
        assert_eq!(round_to(2.675, 2.0), Some(2.68));
        assert_eq!(round_to(1.005, 2.0), Some(1.01));
        assert_eq!(round_to(1234.0, -2.0), Some(1200.0));
        assert_eq!(round_to(2.5, 1.5), None);
        assert_eq!(round_to(2.5, 400.0), Some(2.5));
    }

    #[test]
    fn mensagem_de_argumentos() {
        assert_eq!(arity_message("hipot", Func::Hypot), "hipot precisa de 2 números");
        assert_eq!(arity_message("máx", Func::Max), "máx precisa de 2 números ou mais");
        assert_eq!(arity_message("sen", Func::Sin), "sen precisa de 1 número");
        assert_eq!(arity_message("log", Func::Log), "log precisa de 1 ou 2 números");
    }
}
