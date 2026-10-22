//! Leitura de uma conta das notas (Pratt), com o trecho de cada nó em bytes para
//! sublinhar o erro. Quem chama decide onde a conta começa (a prosa fica antes) e
//! passa só os tokens dela; a leitura usa todos ou diz o primeiro problema.
//!
//! Antes, `roles` decide o papel de cada palavra pela posição (tabela B): "x" entre
//! dois operandos é vezes, "e" entre dois operandos com espaços é a conjunção da
//! prosa, "de" depois de porcentagem é vezes, "em" e "para" depois de um operando
//! convertem. Ordem dos nomes: palavra da conta, variável, função, constante.

use std::ops::Range;

use super::eval::Env;
use super::lexer::{Kind, Token};
use super::value::Value;
use crate::apps::fold;

/// Aninhamento máximo (parênteses, funções, sinais, raízes e potências): a leitura é
/// recursiva, e uma linha de 2.000 "(" não pode estourar a pilha.
const MAX_DEPTH: usize = 64;

/// Palavras que nunca são variáveis.
pub const KEYWORDS: [&str; 4] = ["de", "em", "para", "mod"];

/// O papel de cada token na conta.
#[derive(Debug, Clone, PartialEq)]
pub enum Role {
    /// Número ou símbolo: vale o que o token é.
    Plain,
    /// Variável (a chave, sem acento nem maiúscula).
    Var(String),
    Func(Func),
    Const(Const),
    /// Nome que não é variável, função nem constante (a chave).
    Unknown(String),
    /// "x" entre operandos, ou "de" logo depois de porcentagem.
    Times,
    /// "mod" depois de um operando.
    Mod,
    /// "em" ou "para" depois de um operando.
    Convert,
    /// "e" com espaços entre dois operandos: a conjunção da prosa.
    Conj,
    /// "de", "em", "para" ou "mod" fora do lugar de operação.
    Prose,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Const {
    Pi,
    E,
}

impl Const {
    pub fn value(self) -> f64 {
        match self {
            Const::Pi => std::f64::consts::PI,
            Const::E => std::f64::consts::E,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Func {
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
    Sqrt,
    Cbrt,
    Ln,
    Log,
    Log2,
    Exp,
    Abs,
    Round,
    Floor,
    Ceil,
    Trunc,
    Min,
    Max,
    Hypot,
    Fact,
}

/// Nomes aceitos (já sem acento nem maiúscula), em português e em inglês (tabela C).
const FUNCS: &[(&str, Func)] = &[
    ("sen", Func::Sin),
    ("sin", Func::Sin),
    ("cos", Func::Cos),
    ("tg", Func::Tan),
    ("tan", Func::Tan),
    ("arcsen", Func::Asin),
    ("asen", Func::Asin),
    ("asin", Func::Asin),
    ("arccos", Func::Acos),
    ("acos", Func::Acos),
    ("arctg", Func::Atan),
    ("atg", Func::Atan),
    ("atan", Func::Atan),
    ("senh", Func::Sinh),
    ("sinh", Func::Sinh),
    ("cosh", Func::Cosh),
    ("tgh", Func::Tanh),
    ("tanh", Func::Tanh),
    ("arcsenh", Func::Asinh),
    ("asinh", Func::Asinh),
    ("arccosh", Func::Acosh),
    ("acosh", Func::Acosh),
    ("arctgh", Func::Atanh),
    ("atanh", Func::Atanh),
    ("raiz", Func::Sqrt),
    ("sqrt", Func::Sqrt),
    ("cbrt", Func::Cbrt),
    ("ln", Func::Ln),
    ("log", Func::Log),
    ("log2", Func::Log2),
    ("log₂", Func::Log2),
    ("exp", Func::Exp),
    ("abs", Func::Abs),
    ("arred", Func::Round),
    ("round", Func::Round),
    ("piso", Func::Floor),
    ("floor", Func::Floor),
    ("teto", Func::Ceil),
    ("ceil", Func::Ceil),
    ("trunc", Func::Trunc),
    ("min", Func::Min),
    ("max", Func::Max),
    ("hipot", Func::Hypot),
    ("hypot", Func::Hypot),
    ("fat", Func::Fact),
];

impl Func {
    /// A função pelo nome já sem acento nem maiúscula ("máx" → "max").
    pub fn find(key: &str) -> Option<Func> {
        FUNCS.iter().find(|(name, _)| *name == key).map(|&(_, f)| f)
    }

    /// Quantos números recebe: (mínimo, máximo).
    pub fn arity(self) -> (usize, usize) {
        match self {
            Func::Sqrt | Func::Log | Func::Round => (1, 2),
            Func::Hypot => (2, 2),
            Func::Min | Func::Max => (2, usize::MAX),
            _ => (1, 1),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
}

/// Operações escritas depois do número.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Post {
    Fact,
    Percent,
    Degree,
    Power(i32),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Num(f64),
    /// Variável: a chave e o trecho do nome (o nó pode ir além: "(a)", "+a").
    Var { key: String, name: Range<usize> },
    Const(Const),
    Neg(Box<Expr>),
    Binary(Op, Box<Expr>, Box<Expr>),
    Post(Post, Box<Expr>),
    Root(u8, Box<Expr>),
    /// `name`: como foi escrito, para as mensagens ("tg não existe para 90°").
    Call { func: Func, name: String, args: Vec<Expr> },
    /// "em %".
    ToPercent(Box<Expr>),
}

impl Node {
    /// O filho na espinha da esquerda: o primeiro número de uma operação, o de um
    /// pós-fixo ou o da conversão. Uma conta comprida ("1 + 1 + … + 1", "5!!!") tem
    /// uma espinha de mil nós, percorrida num laço: a recursão estouraria a pilha.
    pub fn left(&self) -> Option<&Expr> {
        match self {
            Node::Binary(_, a, _) | Node::Post(_, a) | Node::ToPercent(a) => Some(a),
            _ => None,
        }
    }
}

/// Um nó da conta e o trecho dele em bytes no texto da nota.
#[derive(Debug, Clone, PartialEq)]
pub struct Expr {
    pub node: Node,
    pub range: Range<usize>,
}

impl Expr {
    /// O nó, deixando um número no lugar (o `Drop` não deixa mover o campo).
    fn into_node(mut self) -> Node {
        std::mem::replace(&mut self.node, Node::Num(0.0))
    }
}

impl Drop for Expr {
    /// Desmonta a espinha da esquerda num laço; o resto da árvore tem a altura que a
    /// leitura limita (`MAX_DEPTH`).
    fn drop(&mut self) {
        let mut next = take_left(&mut self.node);
        while let Some(mut e) = next {
            next = take_left(&mut e.node);
        }
    }
}

/// Tira o filho da espinha da esquerda, deixando um número no lugar do nó.
fn take_left(node: &mut Node) -> Option<Box<Expr>> {
    match std::mem::replace(node, Node::Num(0.0)) {
        Node::Binary(_, a, _) | Node::Post(_, a) | Node::ToPercent(a) => Some(a),
        _ => None,
    }
}

/// O primeiro problema da leitura, com o índice do token (na fatia lida).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fault {
    /// Nome que não é variável, função nem constante.
    Unknown(usize),
    /// Operador (ou função, ou "(") sem o número depois.
    NoNumber(usize),
    /// Operador sem o número antes.
    NoNumberBefore(usize),
    /// Dois operandos sem operação entre eles; o índice é o do segundo.
    NoOperation(usize),
    /// O "(" que não fecha.
    Unclosed(usize),
    /// O ")" que não abriu.
    Unopened(usize),
    /// A conjunção "e" no meio da conta.
    Conj(usize),
    /// Depois de "em" (o primeiro índice) não veio unidade; o segundo é o nome que veio.
    NotUnit(usize, Option<usize>),
    /// Função com números de menos ou de mais: o nome, o último token e a função.
    Arity(usize, usize, Func),
    /// Números de uma função de vários números sem ";" entre eles ("máx(1, 2)",
    /// "máx(1 2)"): a vírgula ou o segundo número.
    Separator(usize),
    /// Operação depois da conversão ("30/150 em % + 1"): o operador e o "em".
    AfterConvert(usize, usize),
    /// Aninhamento demais.
    TooDeep(usize),
    /// Token que não cabe numa conta (prosa, ";" fora de função).
    Stray(usize),
}

impl Fault {
    /// O mesmo problema com os índices deslocados (a fatia lida começava em `by`).
    pub fn shift(self, by: usize) -> Fault {
        match self {
            Fault::Unknown(i) => Fault::Unknown(i + by),
            Fault::NoNumber(i) => Fault::NoNumber(i + by),
            Fault::NoNumberBefore(i) => Fault::NoNumberBefore(i + by),
            Fault::NoOperation(i) => Fault::NoOperation(i + by),
            Fault::Unclosed(i) => Fault::Unclosed(i + by),
            Fault::Unopened(i) => Fault::Unopened(i + by),
            Fault::Conj(i) => Fault::Conj(i + by),
            Fault::NotUnit(i, name) => Fault::NotUnit(i + by, name.map(|n| n + by)),
            Fault::Arity(i, end, f) => Fault::Arity(i + by, end + by, f),
            Fault::Separator(i) => Fault::Separator(i + by),
            Fault::AfterConvert(i, c) => Fault::AfterConvert(i + by, c + by),
            Fault::TooDeep(i) => Fault::TooDeep(i + by),
            Fault::Stray(i) => Fault::Stray(i + by),
        }
    }

    /// Primeiro começo que ainda pode dar certo, na busca da conta mais longa que
    /// termina no "=": as tentativas que passariam pelo mesmo problema são puladas.
    /// Na falta de operação (ou de ";") entre os tokens i − 1 e i, nenhuma conta junta
    /// os dois, e a busca recomeça em i.
    pub fn next_start(self) -> Option<usize> {
        match self {
            Fault::NoOperation(i) | Fault::Separator(i) => Some(i),
            other => other.blocker().map(|b| b + 1),
        }
    }

    /// Maior fim que ainda pode dar certo, na busca da conta mais longa depois do "="
    /// de uma definição.
    pub fn next_end(self) -> Option<usize> {
        match self {
            Fault::NoOperation(i) | Fault::Separator(i) => Some(i),
            other => other.blocker(),
        }
    }

    /// O token que derruba qualquer conta que passe por ele, comece onde começar:
    /// nome desconhecido, prosa, "e", ")" sem par, "(" que não fecha, operador sem
    /// número depois, "em" sem unidade, função com números errados, operação depois
    /// da conversão e aninhamento demais (começar antes só aninha mais).
    fn blocker(self) -> Option<usize> {
        match self {
            Fault::Unknown(i)
            | Fault::Stray(i)
            | Fault::Conj(i)
            | Fault::Unopened(i)
            | Fault::Unclosed(i)
            | Fault::NoNumber(i)
            | Fault::NotUnit(i, _)
            | Fault::Arity(i, _, _)
            | Fault::AfterConvert(i, _)
            | Fault::TooDeep(i) => Some(i),
            Fault::NoNumberBefore(_) | Fault::NoOperation(_) | Fault::Separator(_) => None,
        }
    }
}

/// O papel de cada token de um enunciado, decidido da esquerda para a direita.
/// `opens`: os tokens abrem o enunciado (não vêm logo depois de um "=").
pub fn roles(toks: &[Token], env: &Env, opens: bool) -> Vec<Role> {
    let mut roles: Vec<Role> = Vec::with_capacity(toks.len());
    for (i, t) in toks.iter().enumerate() {
        let role = match &t.kind {
            Kind::Name(word) => word_role(word, i, toks, &roles, env, opens),
            _ => Role::Plain,
        };
        roles.push(role);
    }
    roles
}

/// Papel de uma palavra: primeiro as que dependem da posição, depois o significado.
fn word_role(word: &str, i: usize, toks: &[Token], roles: &[Role], env: &Env, opens: bool) -> Role {
    let prev = i.checked_sub(1).and_then(|p| toks.get(p).zip(roles.get(p)));
    let prev_ends = prev.is_some_and(|(t, r)| ends_operand(t, r));
    let next = toks.get(i + 1);
    let is = |k: &str| word.eq_ignore_ascii_case(k);
    if is("x") && prev_ends && next.is_some_and(|n| starts_operand(n, false)) {
        return Role::Times;
    }
    let spaced = toks.get(i).is_some_and(|t| t.space_before) && next.is_some_and(|n| n.space_before);
    if is("e") && spaced && prev_ends && next.is_some_and(|n| starts_operand(n, true)) {
        return Role::Conj;
    }
    if is("e") && env.get("e").is_none() && prose_e(prev.map(|(_, r)| r), opens, spaced, next) {
        return Role::Prose;
    }
    if is("de") && prev.is_some_and(|(t, r)| percent(t, r, env)) {
        return Role::Times;
    }
    if (is("em") || is("para")) && prev_ends {
        return Role::Convert;
    }
    if is("mod") && prev_ends {
        return Role::Mod;
    }
    if KEYWORDS.iter().any(|k| is(k)) {
        return Role::Prose;
    }
    meaning(word, env)
}

/// O "e" da prosa, e não a constante: com espaços, entre uma palavra e um operando
/// ("pão e 3 × 2"; a pontuação não conta como palavra: "Nota: e − e"), ou abrindo o
/// enunciado, antes de um operando sem sinal ("E 2 × 3"; logo depois do "=", é a
/// constante). Nesses lugares a constante nunca faria conta: depois dela, só operação
/// (e o "x" é a de vezes: "e x 2").
fn prose_e(prev: Option<&Role>, opens: bool, spaced: bool, next: Option<&Token>) -> bool {
    let operand = |signs: bool| next.is_some_and(|n| starts_operand(n, signs) && !matches!(&n.kind, Kind::Name(w) if w.eq_ignore_ascii_case("x")));
    match prev {
        Some(role) => spaced && matches!(role, Role::Unknown(_) | Role::Prose) && operand(true),
        None => opens && next.is_some_and(|n| n.space_before) && operand(false),
    }
}

/// O que um nome é fora das palavras da conta: variável, função, constante ou nada.
fn meaning(word: &str, env: &Env) -> Role {
    let key = fold(word);
    if env.get(&key).is_some() {
        Role::Var(key)
    } else if let Some(f) = Func::find(&key) {
        Role::Func(f)
    } else if word == "π" || word.eq_ignore_ascii_case("pi") {
        Role::Const(Const::Pi)
    } else if word.eq_ignore_ascii_case("e") {
        Role::Const(Const::E)
    } else {
        Role::Unknown(key)
    }
}

/// O token é uma porcentagem: o "%" ou uma variável que guarda uma.
fn percent(t: &Token, role: &Role, env: &Env) -> bool {
    t.kind == Kind::Percent || matches!(role, Role::Var(key) if matches!(env.get(key), Some(Ok(Value::Percent { .. }))))
}

/// Um operando pode terminar neste token: número, ")", pós-fixo ou nome conhecido.
pub fn ends_operand(t: &Token, role: &Role) -> bool {
    match t.kind {
        Kind::Num(_) | Kind::Close | Kind::Bang | Kind::Percent | Kind::Degree | Kind::Sup(_) => true,
        Kind::Name(_) => matches!(role, Role::Var(_) | Role::Const(_)),
        _ => false,
    }
}

/// Um operando pode começar neste token (os sinais contam só com `signs`).
fn starts_operand(t: &Token, signs: bool) -> bool {
    match &t.kind {
        Kind::Num(_) | Kind::Open | Kind::Root(_) | Kind::Money(_) => true,
        Kind::Name(word) => !KEYWORDS.iter().any(|k| word.eq_ignore_ascii_case(k)),
        Kind::Plus | Kind::Minus => signs,
        _ => false,
    }
}

/// A leitura de um operando pode começar neste token (com o sinal).
fn can_start(t: &Token, role: &Role) -> bool {
    matches!(t.kind, Kind::Num(_) | Kind::Open | Kind::Root(_) | Kind::Plus | Kind::Minus)
        || matches!(role, Role::Var(_) | Role::Func(_) | Role::Const(_) | Role::Unknown(_))
}

/// O token pede um operando depois dele: operador, "(", raiz, função, "em".
pub fn cola(t: &Token, role: &Role) -> bool {
    matches!(t.kind, Kind::Plus | Kind::Minus | Kind::Times | Kind::Divide | Kind::Pow | Kind::Open | Kind::Root(_) | Kind::Arrow)
        || matches!(role, Role::Func(_) | Role::Times | Role::Mod | Role::Convert)
}

/// Multiplicação implícita entre `a` e `b` ("2x", "2π", "3(1 + 2)", "2√9"): só depois
/// de número, ")", pós-fixo ou π, e nunca antes de número. Em `bare` (o argumento sem
/// parênteses de uma função), uma função ou raiz depois termina o argumento:
/// "sen 30 cos 30" é sen 30 × cos 30.
pub fn juxtaposes(a: (&Token, &Role), b: (&Token, &Role), bare: bool) -> bool {
    let left = matches!(a.0.kind, Kind::Num(_) | Kind::Close | Kind::Bang | Kind::Percent | Kind::Degree | Kind::Sup(_))
        || *a.1 == Role::Const(Const::Pi);
    let right = match (&b.0.kind, b.1) {
        (Kind::Open, _) => true,
        (Kind::Root(_), _) | (_, Role::Func(_)) => !bare,
        (_, Role::Var(_) | Role::Const(_) | Role::Unknown(_)) => true,
        _ => false,
    };
    left && right
}

/// O token é prosa no meio da conta: pontuação, moeda (sem contas ainda) ou palavra da
/// conta fora do lugar. Ele corta a conta ("Paguei 25. × 3": a conta é "× 3").
pub fn prose(t: (&Token, &Role)) -> bool {
    matches!(t.0.kind, Kind::Other(_) | Kind::Money(_)) || *t.1 == Role::Prose
}

/// Um operador: o token pede um número antes ou depois dele.
fn operator(t: &Token, role: &Role) -> bool {
    matches!(
        t.kind,
        Kind::Plus | Kind::Minus | Kind::Times | Kind::Divide | Kind::Pow | Kind::Bang | Kind::Percent | Kind::Degree | Kind::Sup(_) | Kind::Arrow
    ) || matches!(role, Role::Times | Role::Mod | Role::Convert)
}

/// `a` e `b`, lado a lado, são da mesma conta (para achar o trecho a sublinhar).
pub fn linked(a: (&Token, &Role), b: (&Token, &Role)) -> bool {
    if prose(a) || prose(b) {
        return false;
    }
    let semi = a.0.kind == Kind::Semi || b.0.kind == Kind::Semi;
    semi || takes_left(a, b) || cola(a.0, a.1) || juxtaposes(a, b, false)
}

/// `b` é operação ou pós-fixo que pega `a` como o número da esquerda.
pub fn takes_left(a: (&Token, &Role), b: (&Token, &Role)) -> bool {
    match (&b.0.kind, b.1) {
        (Kind::Plus | Kind::Minus, _) => ends_operand(a.0, a.1),
        (Kind::Times | Kind::Divide | Kind::Pow | Kind::Bang | Kind::Percent | Kind::Degree | Kind::Sup(_) | Kind::Close | Kind::Arrow, _) => true,
        (_, Role::Times | Role::Mod | Role::Convert) => true,
        _ => false,
    }
}

/// Lê a conta inteira de `toks` (`roles` do mesmo tamanho).
pub fn parse(toks: &[Token], roles: &[Role]) -> Result<Expr, Fault> {
    if toks.is_empty() || toks.len() != roles.len() {
        return Err(Fault::Stray(0));
    }
    let mut p = Parser { toks, roles, pos: 0, depth: 0, bare: false, converted: None };
    let e = p.expr(0)?;
    if p.pos < toks.len() {
        return Err(p.leftover());
    }
    Ok(e)
}

/// O que vem depois de um operando, com a força de ligação à esquerda.
enum Step {
    /// Operação de dois números e a força para ler o lado direito.
    Binary(Op, u8),
    Post(Post),
    Convert,
    Juxtaposed,
}

struct Parser<'a> {
    toks: &'a [Token],
    roles: &'a [Role],
    pos: usize,
    depth: usize,
    /// Lendo o argumento sem parênteses de uma função.
    bare: bool,
    /// O "em" (ou "para", "→") da última conversão lida.
    converted: Option<usize>,
}

impl<'a> Parser<'a> {
    fn get(&self, i: usize) -> Option<(&'a Token, &'a Role)> {
        self.toks.get(i).zip(self.roles.get(i))
    }

    /// Uma conta com operações de força `min` para cima. Forças: conversão 10,
    /// + − 20, × ÷ mod 30, implícita 40, sinal 45, ^ 50 (à direita), pós-fixos 60.
    fn expr(&mut self, min: u8) -> Result<Expr, Fault> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(Fault::TooDeep(self.pos));
        }
        let mut lhs = self.prefix()?;
        while let Some((bp, step)) = self.step() {
            if bp < min {
                break;
            }
            let convert = matches!(step, Step::Convert);
            lhs = self.apply(step, lhs)?;
            if convert {
                // Uma conversão por conta, no fim.
                break;
            }
        }
        self.depth -= 1;
        Ok(lhs)
    }

    fn step(&self) -> Option<(u8, Step)> {
        let (t, role) = self.get(self.pos)?;
        Some(match (&t.kind, role) {
            (Kind::Plus, _) => (20, Step::Binary(Op::Add, 21)),
            (Kind::Minus, _) => (20, Step::Binary(Op::Sub, 21)),
            (Kind::Times, _) | (_, Role::Times) => (30, Step::Binary(Op::Mul, 31)),
            (Kind::Divide, _) => (30, Step::Binary(Op::Div, 31)),
            (_, Role::Mod) => (30, Step::Binary(Op::Mod, 31)),
            (Kind::Pow, _) => (50, Step::Binary(Op::Pow, 50)),
            (Kind::Bang, _) => (60, Step::Post(Post::Fact)),
            (Kind::Percent, _) => (60, Step::Post(Post::Percent)),
            (Kind::Degree, _) => (60, Step::Post(Post::Degree)),
            (Kind::Sup(n), _) => (60, Step::Post(Post::Power(*n))),
            (Kind::Arrow, _) | (_, Role::Convert) => (10, Step::Convert),
            _ => {
                let prev = self.get(self.pos.checked_sub(1)?)?;
                if !juxtaposes(prev, (t, role), self.bare) {
                    return None;
                }
                (40, Step::Juxtaposed)
            }
        })
    }

    fn apply(&mut self, step: Step, lhs: Expr) -> Result<Expr, Fault> {
        let at = self.pos;
        let start = lhs.range.start;
        match step {
            Step::Binary(op, rbp) => {
                self.pos += 1;
                let rhs = self.operand(rbp, at)?;
                Ok(Expr { range: start..rhs.range.end, node: Node::Binary(op, Box::new(lhs), Box::new(rhs)) })
            }
            Step::Post(post) => {
                self.pos += 1;
                let end = self.toks.get(at).map_or(lhs.range.end, |t| t.range.end);
                Ok(Expr { range: start..end, node: Node::Post(post, Box::new(lhs)) })
            }
            Step::Convert => self.convert(lhs),
            Step::Juxtaposed => {
                let rhs = self.expr(41)?;
                Ok(Expr { range: start..rhs.range.end, node: Node::Binary(Op::Mul, Box::new(lhs), Box::new(rhs)) })
            }
        }
    }

    /// O operando depois do token `op` (operador, sinal, raiz, função ou "(").
    fn operand(&mut self, min: u8, op: usize) -> Result<Expr, Fault> {
        if !self.can_start() {
            return Err(Fault::NoNumber(op));
        }
        self.expr(min)
    }

    fn can_start(&self) -> bool {
        self.get(self.pos).is_some_and(|(t, role)| can_start(t, role))
    }

    fn prefix(&mut self) -> Result<Expr, Fault> {
        let at = self.pos;
        let Some((t, role)) = self.get(at) else {
            return Err(Fault::Stray(at));
        };
        let leaf = |node| Ok(Expr { node, range: t.range.clone() });
        match (&t.kind, role) {
            (Kind::Num(v), _) => {
                self.pos += 1;
                leaf(Node::Num(*v))
            }
            (_, Role::Var(key)) => {
                self.pos += 1;
                leaf(Node::Var { key: key.clone(), name: t.range.clone() })
            }
            (_, Role::Const(c)) => {
                self.pos += 1;
                leaf(Node::Const(*c))
            }
            (Kind::Name(name), Role::Func(f)) => self.call(*f, name, t),
            (_, Role::Unknown(_)) => Err(Fault::Unknown(at)),
            (_, Role::Conj) => Err(Fault::Conj(at)),
            (Kind::Plus | Kind::Minus, _) => self.sign(t),
            (Kind::Open, _) => self.group(t),
            (Kind::Root(n), _) => self.root(*n, t),
            (Kind::Close, _) => Err(Fault::Unopened(at)),
            (Kind::Times | Kind::Divide | Kind::Pow | Kind::Bang | Kind::Percent | Kind::Degree | Kind::Sup(_) | Kind::Arrow, _)
            | (_, Role::Times | Role::Mod | Role::Convert) => Err(Fault::NoNumberBefore(at)),
            _ => Err(Fault::Stray(at)),
        }
    }

    /// Sinal na frente: "−2^2" é −(2²); "−2 × 3" é (−2) × 3.
    fn sign(&mut self, t: &Token) -> Result<Expr, Fault> {
        let at = self.pos;
        self.pos += 1;
        let inner = self.operand(45, at)?;
        let range = t.range.start..inner.range.end;
        Ok(match t.kind {
            Kind::Minus => Expr { range, node: Node::Neg(Box::new(inner)) },
            _ => Expr { range, node: inner.into_node() },
        })
    }

    /// Raiz: pega o operando com os pós-fixos ("√16²" é √256) e aceita sinal ("∛−8").
    fn root(&mut self, n: u8, t: &Token) -> Result<Expr, Fault> {
        let at = self.pos;
        self.pos += 1;
        let inner = self.operand(60, at)?;
        Ok(Expr { range: t.range.start..inner.range.end, node: Node::Root(n, Box::new(inner)) })
    }

    fn group(&mut self, open: &Token) -> Result<Expr, Fault> {
        let at = self.pos;
        self.pos += 1;
        let bare = std::mem::replace(&mut self.bare, false);
        let inner = self.operand(0, at)?;
        self.bare = bare;
        match self.get(self.pos) {
            Some((close, _)) if close.kind == Kind::Close => {
                self.pos += 1;
                Ok(Expr { range: open.range.start..close.range.end, node: inner.into_node() })
            }
            _ => Err(self.inner_leftover(at)),
        }
    }

    /// Função: "sen(30)", "log(8; 2)" ou sem parênteses, "sen 30°", "sen 2π".
    fn call(&mut self, func: Func, name: &str, t: &Token) -> Result<Expr, Fault> {
        let at = self.pos;
        self.pos += 1;
        let args = match self.get(self.pos) {
            Some((open, _)) if open.kind == Kind::Open => self.arguments(func.arity().1 > 1)?,
            _ => vec![self.bare_argument(at)?],
        };
        let last = self.pos.saturating_sub(1);
        let (min, max) = func.arity();
        if args.len() < min || args.len() > max {
            return Err(Fault::Arity(at, last, func));
        }
        let end = self.toks.get(last).map_or(t.range.end, |l| l.range.end);
        Ok(Expr { range: t.range.start..end, node: Node::Call { func, name: name.to_string(), args } })
    }

    fn bare_argument(&mut self, at: usize) -> Result<Expr, Fault> {
        if !self.can_start() {
            return Err(Fault::NoNumber(at));
        }
        let bare = std::mem::replace(&mut self.bare, true);
        let arg = self.expr(40);
        self.bare = bare;
        arg
    }

    /// Os argumentos entre parênteses, separados por ";". `many`: a função aceita mais
    /// de um número.
    fn arguments(&mut self, many: bool) -> Result<Vec<Expr>, Fault> {
        let open = self.pos;
        let bare = std::mem::replace(&mut self.bare, false);
        let mut args = Vec::new();
        loop {
            let sep = self.pos;
            self.pos += 1;
            args.push(self.operand(0, sep)?);
            match self.get(self.pos).map(|(t, _)| &t.kind) {
                Some(Kind::Semi) => {}
                Some(Kind::Close) => break,
                _ => return Err(self.arguments_leftover(open, many)),
            }
        }
        self.pos += 1;
        self.bare = bare;
        Ok(args)
    }

    /// O que sobrou depois de um argumento. Na função de vários números, a vírgula
    /// antes de outro número (o hábito do inglês, "máx(1, 2)") ou um número sem nada
    /// no meio ("máx(1 2)") pedem o ";".
    fn arguments_leftover(&self, open: usize, many: bool) -> Fault {
        let comma = self.get(self.pos).is_some_and(|(t, _)| t.kind == Kind::Other(','));
        let then_operand = self.get(self.pos + 1).is_some_and(|(t, r)| can_start(t, r));
        match self.inner_leftover(open) {
            Fault::NoOperation(at) if many => Fault::Separator(at),
            _ if many && comma && then_operand => Fault::Separator(self.pos),
            other => other,
        }
    }

    /// Converte com "em", "para" ou "→": nesta etapa, só para "%".
    fn convert(&mut self, lhs: Expr) -> Result<Expr, Fault> {
        let at = self.pos;
        self.converted = Some(at);
        self.pos += 1;
        match self.get(self.pos) {
            Some((t, _)) if t.kind == Kind::Percent => {
                self.pos += 1;
                Ok(Expr { range: lhs.range.start..t.range.end, node: Node::ToPercent(Box::new(lhs)) })
            }
            Some((t, _)) if matches!(t.kind, Kind::Name(_)) => Err(Fault::NotUnit(at, Some(self.pos))),
            _ => Err(Fault::NotUnit(at, None)),
        }
    }

    /// O que sobrou depois de uma conta completa.
    fn leftover(&self) -> Fault {
        let at = self.pos;
        // Só sobra operador depois de uma conversão: ela fica no fim da conta.
        if let Some(convert) = self.converted
            && self.get(at).is_some_and(|(t, r)| operator(t, r))
        {
            return Fault::AfterConvert(at, convert);
        }
        match self.get(at) {
            Some((t, _)) if t.kind == Kind::Close => Fault::Unopened(at),
            Some((_, Role::Conj)) => Fault::Conj(at),
            Some((_, Role::Prose)) => Fault::Stray(at),
            Some((t, _)) if matches!(t.kind, Kind::Num(_) | Kind::Open | Kind::Root(_) | Kind::Name(_)) => Fault::NoOperation(at),
            _ => Fault::Stray(at),
        }
    }

    /// O que sobrou dentro de parênteses abertos em `open`: operando ou "e" dizem o
    /// problema deles; ";" fora de função também. Prosa antes de outro número, com os
    /// parênteses fechando adiante, é a operação que falta ("(10 de 3)", "(1, 2)"); o
    /// resto (prosa, fim) deixa o parêntese aberto.
    fn inner_leftover(&self, open: usize) -> Fault {
        match self.leftover() {
            Fault::Stray(at) if self.get(at).is_some_and(|(t, _)| t.kind == Kind::Semi) => Fault::Stray(at),
            Fault::Stray(at) if self.number_after(at) && self.closes_later() => Fault::NoOperation(at + 1),
            Fault::Stray(_) | Fault::Unopened(_) => Fault::Unclosed(open),
            other => other,
        }
    }

    /// Depois do token `at` vem um número, "(", raiz ou nome conhecido.
    fn number_after(&self, at: usize) -> bool {
        self.get(at + 1)
            .is_some_and(|(t, r)| matches!(t.kind, Kind::Num(_) | Kind::Open | Kind::Root(_)) || matches!(r, Role::Var(_) | Role::Const(_) | Role::Func(_)))
    }

    /// Os parênteses em que a leitura está fecham mais adiante.
    fn closes_later(&self) -> bool {
        let mut depth = 0usize;
        for t in self.toks.get(self.pos..).unwrap_or_default() {
            match t.kind {
                Kind::Open => depth += 1,
                Kind::Close if depth == 0 => return true,
                Kind::Close => depth -= 1,
                _ => {}
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::super::lexer::lex;
    use super::*;

    fn read(line: &str) -> Result<Expr, Fault> {
        let toks = lex(line, 0);
        parse(&toks, &roles(&toks, &Env::default(), true))
    }

    /// A conta como texto com parênteses, para conferir a precedência.
    fn tree(line: &str) -> String {
        fn show(e: &Expr) -> String {
            match &e.node {
                Node::Num(v) => format!("{v}"),
                Node::Var { key, .. } => key.clone(),
                Node::Const(Const::Pi) => "π".into(),
                Node::Const(Const::E) => "e".into(),
                Node::Neg(x) => format!("-{}", show(x)),
                Node::Binary(op, a, b) => format!("({} {op:?} {})", show(a), show(b)),
                Node::Post(p, x) => format!("{}{p:?}", show(x)),
                Node::Root(n, x) => format!("root{n}({})", show(x)),
                Node::Call { func, args, .. } => format!("{func:?}({})", args.iter().map(show).collect::<Vec<_>>().join("; ")),
                Node::ToPercent(x) => format!("{} em %", show(x)),
            }
        }
        read(line).map_or_else(|f| format!("{f:?}"), |e| show(&e))
    }

    #[test]
    fn precedencia() {
        assert_eq!(tree("2 + 3 × 4"), "(2 Add (3 Mul 4))");
        assert_eq!(tree("2^3^2"), "(2 Pow (3 Pow 2))");
        assert_eq!(tree("−2^2"), "-(2 Pow 2)");
        assert_eq!(tree("2^−1"), "(2 Pow -1)");
        assert_eq!(tree("6 / 2(1+2)"), "(6 Div (2 Mul (1 Add 2)))");
        assert_eq!(tree("√16²"), "root2(16Power(2))");
        assert_eq!(tree("2√9"), "(2 Mul root2(9))");
        assert_eq!(tree("∛−8"), "root3(-8)");
        assert_eq!(tree("sen 30 cos 30"), "(Sin(30) Mul Cos(30))");
        assert_eq!(tree("sen 2π + 1"), "(Sin((2 Mul π)) Add 1)");
        assert_eq!(tree("log(8; 2)"), "Log(8; 2)");
        assert_eq!(tree("30/150 em %"), "0.2 em %");
        assert_eq!(tree("20% de 150"), "(20Percent Mul 150)");
        assert_eq!(tree("10 mod 3"), "(10 Mod 3)");
    }

    #[test]
    fn problemas() {
        assert_eq!(read("2 +"), Err(Fault::NoNumber(1)));
        assert_eq!(read("2 3"), Err(Fault::NoOperation(1)));
        assert_eq!(read("(2 + 3"), Err(Fault::Unclosed(0)));
        assert_eq!(read("2 + 3)"), Err(Fault::Unopened(3)));
        assert_eq!(read("2 e 3"), Err(Fault::Conj(1)));
        assert_eq!(read("2 × itens"), Err(Fault::Unknown(2)));
        assert_eq!(read("× 2"), Err(Fault::NoNumberBefore(0)));
        assert_eq!(read("hipot(3)"), Err(Fault::Arity(0, 3, Func::Hypot)));
        assert_eq!(read("2 em kg"), Err(Fault::NotUnit(1, Some(2))));
        assert_eq!(read("(2; 3)"), Err(Fault::Stray(2)));
        assert!(matches!(read(&"(".repeat(500)), Err(Fault::TooDeep(_))));
        assert!(matches!(read(&"-".repeat(500)), Err(Fault::TooDeep(_))));
    }

    #[test]
    fn separador_conversao_e_prosa_nos_parenteses() {
        // Função de vários números sem ";"; com um número só, falta a operação.
        assert_eq!(read("máx(1, 2)"), Err(Fault::Separator(4)));
        assert_eq!(read("máx(1 2)"), Err(Fault::Separator(3)));
        assert_eq!(read("sen(30 60)"), Err(Fault::NoOperation(3)));
        // Prosa entre números, com os parênteses fechando: falta a operação.
        assert_eq!(read("(1, 2)"), Err(Fault::NoOperation(3)));
        assert_eq!(read("(2 + 3, fim"), Err(Fault::Unclosed(0)));
        // A conversão fica no fim.
        assert_eq!(read("30/150 em % + 1"), Err(Fault::AfterConvert(3, 1)));
        assert!(read("(30/150 em %) + 1").is_ok());
    }

    #[test]
    fn e_de_prosa_ou_constante() {
        let role = |line: &str, i: usize| {
            let toks = lex(line, 0);
            roles(&toks, &Env::default(), true)[i].clone()
        };
        assert_eq!(role("pão e 3", 1), Role::Prose);
        assert_eq!(role("E 2 × 3", 0), Role::Prose);
        assert_eq!(role("2 e 3", 1), Role::Conj);
        assert_eq!(role("Nota: e − e", 2), Role::Const(Const::E));
        assert_eq!(role("e x 2", 0), Role::Const(Const::E));
        // Logo depois do "=" (não abre o enunciado), é a constante.
        let toks = lex("e 2", 0);
        assert_eq!(roles(&toks, &Env::default(), false)[0], Role::Const(Const::E));
    }
}
