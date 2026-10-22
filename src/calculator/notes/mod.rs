//! Notas matemáticas da Calculadora (E3b, etapa 2): texto livre em que a linha que
//! termina em "=" mostra o resultado depois dele ("aluguel × 12 =" → 14.400), com
//! variáveis ("aluguel = 1.200") e prosa antes da conta ("Hoje gastei 2 × 3 =" → 6).
//! Sem interface: o app desenha o texto e chama `evaluate` a cada mudança.
//!
//! Camadas: `lexer` corta a linha, `parser` lê uma conta, `value` e `eval` fazem as
//! contas com o motor do teclado. Aqui ficam as linhas, os enunciados (";"), os "="
//! (definição ou pedido de resultado) e as mensagens de erro.

pub mod eval;
pub mod lexer;
pub mod parser;
pub mod store;
pub mod value;

use std::collections::HashMap;
use std::ops::Range;

use super::format::Style;
use crate::apps::fold;
use eval::{Env, Eval};
use lexer::{Kind, Token};
use parser::{Expr, Fault, Func, Node, Role};

/// Linha mais comprida que é lida, em caracteres; uma maior fica só como texto.
const MAX_LINE: usize = 2000;

const CONJ: &str = "“e” não é uma operação: use +, −, × ou ÷";

/// Opções da Calculadora que valem nas notas.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Context {
    /// Ângulos sem "°" em radianos (senão, em graus).
    pub radians: bool,
    /// Casas decimais e ponto de milhar dos resultados.
    pub style: Style,
}

/// O resultado mostrado depois de um "=".
#[derive(Debug, Clone, PartialEq)]
pub struct Shown {
    /// Byte logo depois do "=", no texto inteiro.
    pub at: usize,
    /// Como aparece: "1.514", "20%", "30°".
    pub text: String,
    /// Só o número, sem ponto de milhar: "1514".
    pub number: String,
    /// O valor inteiro do número que aparece (o 20 de "20%").
    pub value: f64,
}

/// Um erro: o trecho a sublinhar (bytes no texto inteiro) e a mensagem.
#[derive(Debug, Clone, PartialEq)]
pub struct Problem {
    pub range: Range<usize>,
    pub message: String,
}

/// O que uma linha mostra.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LineEval {
    pub result: Option<Shown>,
    /// Variáveis reconhecidas (na definição e no uso), para colorir.
    pub names: Vec<Range<usize>>,
    pub problems: Vec<Problem>,
}

/// Avalia a nota inteira: uma `LineEval` por linha ("\n"). As variáveis valem para
/// baixo e para a direita.
pub fn evaluate(text: &str, ctx: &Context) -> Vec<LineEval> {
    let lines = lex_lines(text);
    let below = definitions(&lines);
    let mut env = Env::default();
    lines
        .iter()
        .enumerate()
        .map(|(n, toks)| match toks {
            Some(toks) => Line { text, n, toks, below: &below, ctx, out: LineEval::default() }.run(&mut env),
            None => LineEval::default(),
        })
        .collect()
}

/// O texto com cada resultado escrito depois do seu "=" ("120 × 3 = 360"), para
/// copiar e exportar.
pub fn with_results(text: &str, lines: &[LineEval]) -> String {
    let mut out = String::with_capacity(text.len() + 16 * lines.len());
    let mut last = 0;
    for shown in lines.iter().filter_map(|l| l.result.as_ref()) {
        let Some(before) = text.get(last..shown.at) else { continue };
        out.push_str(before);
        out.push(' ');
        out.push_str(&shown.text);
        last = shown.at;
    }
    out.push_str(text.get(last..).unwrap_or(""));
    out
}

/// Os tokens de cada linha; `None` para a linha que fica só como texto: sem "="
/// (não define nem mostra nada) ou longa demais.
fn lex_lines(text: &str) -> Vec<Option<Vec<Token>>> {
    let mut offset = 0;
    text.split('\n')
        .map(|line| {
            let short = line.len() <= MAX_LINE || line.chars().count() <= MAX_LINE;
            let toks = (short && line.contains('=')).then(|| lexer::lex(line, offset));
            offset += line.len() + 1;
            toks
        })
        .collect()
}

/// Onde cada nome aparece como "nome = algo" (linha, token), para dizer que uma
/// variável usada antes é "definida mais abaixo".
fn definitions(lines: &[Option<Vec<Token>>]) -> HashMap<String, Vec<(usize, usize)>> {
    let mut map: HashMap<String, Vec<(usize, usize)>> = HashMap::new();
    for (n, toks) in lines.iter().enumerate() {
        for (i, w) in toks.as_deref().unwrap_or_default().windows(3).enumerate() {
            if let [Token { kind: Kind::Name(name), .. }, Token { kind: Kind::Equals, .. }, next] = w
                && next.kind != Kind::Equals
            {
                map.entry(fold(name)).or_default().push((n, i));
            }
        }
    }
    map
}

/// Os "=" e ";" que separam: fora de parênteses que fecham. Um ";" depois de um "("
/// sem par também fica dentro (os argumentos de uma função sendo escrita).
fn separators(toks: &[Token]) -> Vec<bool> {
    let mut inside = vec![0i32; toks.len() + 1];
    let mut open = Vec::new();
    for (i, t) in toks.iter().enumerate() {
        match t.kind {
            Kind::Open => open.push(i),
            Kind::Close => {
                if let Some(o) = open.pop() {
                    inside[o + 1] += 1;
                    inside[i] -= 1;
                }
            }
            _ => {}
        }
    }
    let unclosed = open.first().copied();
    let mut depth = 0;
    toks.iter()
        .enumerate()
        .map(|(i, t)| {
            depth += inside[i];
            match t.kind {
                Kind::Equals => depth == 0,
                Kind::Semi => depth == 0 && unclosed.is_none_or(|o| i < o),
                _ => false,
            }
        })
        .collect()
}

/// Os enunciados da linha (separados por ";"), como trechos de tokens.
fn statements(toks: &[Token], seps: &[bool]) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut start = 0;
    for (i, t) in toks.iter().enumerate() {
        if t.kind == Kind::Semi && seps[i] {
            out.push(start..i);
            start = i + 1;
        }
    }
    out.push(start..toks.len());
    out
}

/// Um trecho entre separadores, com o papel de cada token.
struct Seg<'a> {
    /// Índice do primeiro token na linha.
    start: usize,
    toks: &'a [Token],
    roles: Vec<Role>,
    /// Para cada ")", o "(" que ele fecha.
    openers: Vec<Option<usize>>,
    /// Para cada "(", o ")" que o fecha.
    closers: Vec<Option<usize>>,
    /// O nome desconhecido é definido mais abaixo: conta como variável.
    later: Vec<bool>,
}

/// Os pares de parênteses: para cada ")", o "(" que ele fecha, e para cada "(", o ")"
/// (sem par, `None`).
fn pairs(toks: &[Token]) -> (Vec<Option<usize>>, Vec<Option<usize>>) {
    let mut openers = vec![None; toks.len()];
    let mut closers = vec![None; toks.len()];
    let mut open = Vec::new();
    for (i, t) in toks.iter().enumerate() {
        match t.kind {
            Kind::Open => open.push(i),
            Kind::Close => {
                if let Some(o) = open.pop() {
                    openers[i] = Some(o);
                    closers[o] = Some(i);
                }
            }
            _ => {}
        }
    }
    (openers, closers)
}

impl<'a> Seg<'a> {
    /// Os tokens `range` da linha. `below(chave, token na linha)`: o nome é definido
    /// mais abaixo.
    fn new(line: &'a [Token], range: Range<usize>, env: &Env, below: impl Fn(&str, usize) -> bool) -> Self {
        let toks = line.get(range.clone()).unwrap_or_default();
        let opens = range.start.checked_sub(1).and_then(|p| line.get(p)).is_none_or(|t| t.kind != Kind::Equals);
        let roles = parser::roles(toks, env, opens);
        let later = roles.iter().enumerate().map(|(i, r)| matches!(r, Role::Unknown(key) if below(key, range.start + i))).collect();
        let (openers, closers) = pairs(toks);
        Seg { start: range.start, toks, roles, openers, closers, later }
    }

    fn pair(&self, i: usize) -> Option<(&Token, &Role)> {
        self.toks.get(i).zip(self.roles.get(i))
    }

    fn parse(&self, range: Range<usize>) -> Result<Expr, Fault> {
        match (self.toks.get(range.clone()), self.roles.get(range.clone())) {
            (Some(toks), Some(roles)) => parser::parse(toks, roles).map_err(|f| f.shift(range.start)),
            _ => Err(Fault::Stray(range.start)),
        }
    }

    /// Os tokens `a` e `b`, lado a lado, são da mesma conta. Parênteses de prosa não
    /// se ligam ao que vem colado neles ("(nota) máx(…)", "2 (nota)").
    fn linked(&self, a: usize, b: usize) -> bool {
        let (Some(pa), Some(pb)) = (self.pair(a), self.pair(b)) else {
            return false;
        };
        if !parser::linked(pa, pb) {
            return false;
        }
        // Lado a lado, sem operação entre eles (a multiplicação implícita).
        let side_by_side = parser::juxtaposes(pa, pb, false) && !parser::takes_left(pa, pb) && !parser::cola(pa.0, pa.1);
        !(side_by_side && ((pa.0.kind == Kind::Close && self.prose_group(a)) || self.closer(b).is_some_and(|c| self.prose_group(c))))
    }

    /// O "(" que o ")" em `close` fecha.
    fn opener(&self, close: usize) -> Option<usize> {
        self.openers.get(close).copied().flatten()
    }

    /// O ")" que fecha o "(" em `open`.
    fn closer(&self, open: usize) -> Option<usize> {
        self.closers.get(open).copied().flatten()
    }

    /// Começo da conta que termina em `from`: volta enquanto os tokens se ligam e,
    /// de um ")" da conta, até o "(" dele ("(2 + 3 4)" não começa no "4").
    fn chain_start(&self, from: usize) -> usize {
        let (mut start, mut seen) = (from, from + 1);
        loop {
            while start > 0 && self.linked(start - 1, start) {
                start -= 1;
            }
            let open = (start..seen).filter_map(|i| self.opener(i)).filter(|&o| o < start).min();
            seen = start;
            match open {
                Some(o) => start = o,
                None => return start,
            }
        }
    }

    /// Fim (exclusivo) da conta que começa em `from`: segue enquanto os tokens se
    /// ligam e, de um "(" da conta, até o ")" dele ("máx(1, 2)" vai até o ")").
    fn chain_end(&self, from: usize) -> usize {
        let (mut end, mut seen) = (from + 1, from);
        loop {
            while end < self.toks.len() && self.linked(end - 1, end) {
                end += 1;
            }
            let close = (seen..end).filter_map(|i| self.closer(i)).filter(|&c| c >= end).max();
            seen = end;
            match close {
                Some(c) => end = c + 1,
                None => return end,
            }
        }
    }

    /// Os parênteses que o ")" em `close` fecha são prosa: sem par, ou sem conta dentro
    /// nem função ou operação antes ("(nota)", "(pagar amanhã)"; "cos( )" e "2 ×
    /// (itens)" são conta).
    fn prose_group(&self, close: usize) -> bool {
        let Some(open) = self.opener(close) else {
            return true;
        };
        let bound = open.checked_sub(1).and_then(|p| self.pair(p)).is_some_and(|(t, r)| parser::cola(t, r));
        !bound && !self.is_math(open + 1..close)
    }

    /// O "(" do ")" em `close`, se os parênteses são conta, por dentro ou pelo que se
    /// liga a eles: "(2 3)", "(2 × itens)", "cos( )" e "2 × (nota)" sim; "(nota)" não.
    fn math_group(&self, close: usize) -> Option<usize> {
        let open = self.opener(close)?;
        self.is_math(self.chain_start(open)..close + 1).then_some(open)
    }

    /// O trecho é conta, e não prosa: tem número, nome conhecido ou operação, e as
    /// palavras desconhecidas estão ligadas por operação ("2 × itens"), não soltas
    /// ("(pagar amanhã)", "- pagar aluguel", "(2 itens)"). O nome definido mais abaixo
    /// conta como variável, e o "e" entre números ("2 e 3"), como operação (errada).
    fn is_math(&self, range: Range<usize>) -> bool {
        let (mut evidence, mut words, mut operation) = (false, false, false);
        let first = range.start;
        for i in range {
            let Some((t, role)) = self.pair(i) else { break };
            match role {
                Role::Unknown(_) if !self.later.get(i).copied().unwrap_or(false) => words = true,
                Role::Unknown(_) | Role::Var(_) | Role::Const(_) => evidence = true,
                Role::Prose => words = true,
                Role::Func(_) | Role::Times | Role::Mod | Role::Convert | Role::Conj => (evidence, operation) = (true, true),
                Role::Plain => match t.kind {
                    Kind::Num(_) => evidence = true,
                    Kind::Other(_) | Kind::Money(_) => words = true,
                    // O sinal no começo não liga nada ("- pagar aluguel").
                    Kind::Plus | Kind::Minus => {
                        evidence = true;
                        let binary = |(p, r): (&Token, &Role)| parser::ends_operand(p, r) || matches!(r, Role::Unknown(_));
                        operation |= i > first && self.pair(i - 1).is_some_and(binary);
                    }
                    Kind::Times | Kind::Divide | Kind::Pow | Kind::Root(_) | Kind::Arrow => (evidence, operation) = (true, true),
                    // Pós-fixo numa palavra é conta ("lado²"); num número, não liga a
                    // palavra seguinte ("10² itens", como "2 itens").
                    Kind::Percent | Kind::Degree | Kind::Sup(_) => {
                        evidence = true;
                        operation |= i > first && self.pair(i - 1).is_some_and(|(p, _)| !matches!(p.kind, Kind::Num(_)));
                    }
                    _ => {}
                },
            }
        }
        evidence && (!words || operation)
    }

    /// Antes do sinal que abre a conta (o token `i`) há conta com erro, e não um
    /// rótulo ("Mercado − 2 × 30"): um nome desconhecido ligado à conta pela esquerda
    /// ("2 × itens + 1"), definido mais abaixo ("z + 1" acima de "z = 5") ou com
    /// expoente, porcentagem ou grau ("lado² + 1"; a "!" colada numa palavra é
    /// exclamação), ou parênteses com conta ("(2 × itens)² + 1").
    fn broken_before_sign(&self, i: usize) -> bool {
        let mut j = i;
        let mut postfix = false;
        while j > 0
            && let Some((t, _)) = self.pair(j)
            && matches!(t.kind, Kind::Bang | Kind::Percent | Kind::Degree | Kind::Sup(_))
        {
            postfix |= t.kind != Kind::Bang;
            j -= 1;
        }
        match self.pair(j) {
            Some((t, _)) if t.kind == Kind::Close => self.math_group(j).is_some(),
            Some((_, Role::Unknown(_))) => postfix || self.linked_left(j) || self.later.get(j).copied().unwrap_or(false),
            _ => false,
        }
    }

    /// O token `j` está ligado à conta pela esquerda: por operador, função, número ou
    /// parênteses com conta. O sinal que abre a cadeia (o traço de uma lista: "- Mercado
    /// − 2") e o ")" de prosa ("Oba :) Mercado − 2") não ligam.
    fn linked_left(&self, j: usize) -> bool {
        let start = self.chain_start(j);
        let Some((t, _)) = j.checked_sub(1).filter(|&p| p >= start).and_then(|p| self.pair(p)) else {
            return false;
        };
        match t.kind {
            Kind::Plus | Kind::Minus => j - 1 > start,
            Kind::Close => self.math_group(j - 1).is_some(),
            _ => true,
        }
    }

    /// O valor de uma definição, sem conta no começo, ainda é conta quebrada quando
    /// começa por um nome desconhecido seguido de operação ("preço × 2", "prço + 2",
    /// "lado²") ou definido mais abaixo, ou por "(", sinal, raiz ou função com conta no
    /// trecho ("(2 + 3", "−"). A prosa depois do "=" não ("(pagar amanhã)", "- pagar
    /// aluguel", "pago - falta recibo").
    fn starts_broken(&self) -> bool {
        match self.pair(0) {
            Some((_, Role::Unknown(_))) => self.later.first().copied().unwrap_or(false) || binds_word(self, 1),
            Some((t, role)) => starts_strict(t, role) && self.is_math(0..self.toks.len()),
            None => false,
        }
    }

    /// O menor operando que termina logo antes de `end`: "3" em "2 + 3 4", "(1 + 2)"
    /// em "(1 + 2) 3", "10%" em "10 % 3". Só tenta começos fora de parênteses e para
    /// no primeiro problema que começar antes não resolve, para a busca não reler a
    /// linha a cada começo.
    fn operand_before(&self, end: usize) -> Option<usize> {
        let mut depth = 0usize;
        for s in (0..end).rev() {
            match self.toks.get(s)?.kind {
                Kind::Close => depth += 1,
                Kind::Open if depth == 0 => return None,
                Kind::Open => depth -= 1,
                _ => {}
            }
            if depth > 0 {
                continue;
            }
            let call = s > 0 && self.pair(s - 1).is_some_and(|(_, r)| matches!(r, Role::Func(_)));
            match self.parse(s..end) {
                Ok(_) => return Some(s),
                // Pós-fixo ou operador no começo: começando antes, ele ganha o número; e
                // os parênteses de uma função ("log(8; 2)") valem com o nome dela.
                Err(Fault::NoNumberBefore(_)) => {}
                Err(_) if call => {}
                Err(_) => return None,
            }
        }
        None
    }

    /// Fim (exclusivo) do menor operando que começa em `start`: "4" em "3 4",
    /// "sen(30)" em "2 sen(30)". Como em `operand_before`, sem reler a linha toda.
    fn operand_after(&self, start: usize) -> Option<usize> {
        let mut depth = 0usize;
        for e in start..self.toks.len() {
            match self.toks.get(e)?.kind {
                Kind::Open => depth += 1,
                Kind::Close if depth == 0 => return None,
                Kind::Close => depth -= 1,
                _ => {}
            }
            if depth > 0 {
                continue;
            }
            match self.parse(start..e + 1) {
                Ok(_) => return Some(e + 1),
                // O sinal, a função ou a raiz no fim esperam o número que vem depois.
                Err(Fault::NoNumber(i)) if i == e => {}
                Err(_) => return None,
            }
        }
        None
    }

    /// Número mal escrito em volta do token `k`: um "." ou "," colado entre dois
    /// números ("1,234.5", em inglês, ou "1.2.3"). Devolve os tokens do número todo.
    fn malformed(&self, k: usize) -> Option<Range<usize>> {
        let piece = |i: usize| self.toks.get(i).is_some_and(|t| matches!(t.kind, Kind::Num(_) | Kind::Other('.' | ',')));
        let num = |i: usize| self.toks.get(i).is_some_and(|t| matches!(t.kind, Kind::Num(_)));
        let glued = |i: usize| self.toks.get(i).is_some_and(|t| !t.space_before);
        let sep = self.toks.get(k).is_some_and(|t| matches!(t.kind, Kind::Other('.' | ',')));
        if !(sep && glued(k) && glued(k + 1) && num(k + 1) && k.checked_sub(1).is_some_and(num)) {
            return None;
        }
        let mut start = k - 1;
        while start > 0 && glued(start) && piece(start - 1) {
            start -= 1;
        }
        let mut end = k + 1;
        while glued(end + 1) && piece(end + 1) {
            end += 1;
        }
        while !num(start) {
            start += 1;
        }
        while !num(end) {
            end -= 1;
        }
        Some(start..end + 1)
    }
}

/// O que uma definição achou depois do "=".
enum Def {
    Value(Expr),
    /// A conta está quebrada: o primeiro problema e o trecho da conta.
    Broken(Fault, Range<usize>),
    /// Número mal escrito (os tokens dele).
    Malformed(Range<usize>),
}

/// A avaliação de uma linha.
struct Line<'a> {
    text: &'a str,
    /// Linha, contando de 0.
    n: usize,
    toks: &'a [Token],
    below: &'a HashMap<String, Vec<(usize, usize)>>,
    ctx: &'a Context,
    out: LineEval,
}

impl<'a> Line<'a> {
    fn run(mut self, env: &mut Env) -> LineEval {
        let seps = separators(self.toks);
        // O resultado aparece só no "=" que termina a linha.
        let last = self.toks.len().checked_sub(1).filter(|&i| self.toks[i].kind == Kind::Equals && seps[i]);
        for stmt in statements(self.toks, &seps) {
            let eqs: Vec<usize> = stmt.clone().filter(|&i| seps[i] && self.toks[i].kind == Kind::Equals).collect();
            let mut start = stmt.start;
            let problems = self.out.problems.len();
            for (k, &eq) in eqs.iter().enumerate() {
                let after = eq + 1..eqs.get(k + 1).copied().unwrap_or(stmt.end);
                // Com erro no enunciado (a definição de "custo = 25 × 4 ="), o "=" fica
                // sem resultado.
                if !self.define(env, start..eq, after) && Some(eq) == last && self.out.problems.len() == problems {
                    self.request(env, start..eq, eq);
                }
                start = eq + 1;
            }
        }
        self.out.names.sort_by_key(|r| r.start);
        self.out.names.dedup();
        self.out
    }

    fn slice(&self, range: Range<usize>) -> &str {
        self.text.get(range).unwrap_or("")
    }

    /// Os tokens `range` da linha, com os papéis e os nomes definidos mais abaixo.
    fn seg(&self, range: Range<usize>, env: &Env) -> Seg<'a> {
        Seg::new(self.toks, range, env, |key, at| self.later(key, at).is_some())
    }

    fn push(&mut self, problem: Problem) {
        if !self.out.problems.contains(&problem) {
            self.out.problems.push(problem);
        }
    }

    /// As variáveis usadas na conta, para colorir. Com uma pilha, sem recursão: a
    /// espinha de uma conta comprida tem mil nós.
    fn mark(&mut self, e: &Expr) {
        let mut stack = vec![e];
        while let Some(e) = stack.pop() {
            match &e.node {
                Node::Var { name, .. } => self.out.names.push(name.clone()),
                Node::Neg(x) | Node::Post(_, x) | Node::Root(_, x) | Node::ToPercent(x) => stack.push(x),
                Node::Binary(_, a, b) => stack.extend([&**a, &**b]),
                Node::Call { args, .. } => stack.extend(args),
                Node::Num(_) | Node::Const(_) => {}
            }
        }
    }

    fn eval(&self, env: &Env, e: &Expr) -> Result<value::Value, Problem> {
        Eval { env, ctx: self.ctx, text: self.text }.value(e)
    }

    /// "nome = valor": o nome é a palavra colada antes do "=" (o que vem antes é
    /// prosa) e o valor, a conta mais longa logo depois dele ("1.200 por mês" vale
    /// 1.200). Devolve se era uma definição.
    fn define(&mut self, env: &mut Env, region: Range<usize>, after: Range<usize>) -> bool {
        let toks = self.toks;
        let Some(at) = region.end.checked_sub(1).filter(|&i| i >= region.start && !after.is_empty()) else {
            return false;
        };
        let Some(Token { kind: Kind::Name(name), range, .. }) = toks.get(at) else {
            return false;
        };
        let seg = self.seg(region.clone(), env);
        if at > region.start && seg.pair(at - region.start - 1).is_some_and(|(t, r)| matches!(t.kind, Kind::Num(_)) || parser::cola(t, r)) {
            return false;
        }
        let value_seg = self.seg(after, env);
        let Some(def) = definition(&value_seg) else {
            return false;
        };
        if let Some(message) = bad_name(name) {
            self.push(Problem { range: range.clone(), message });
            return true;
        }
        let value = match def {
            Def::Value(expr) => {
                self.mark(&expr);
                self.eval(env, &expr)
            }
            Def::Broken(fault, chain) => Err(self.message(fault, &value_seg, chain)),
            Def::Malformed(run) => Err(self.malformed(&value_seg, run)),
        };
        let value = match value {
            Ok(v) => Ok(v),
            Err(problem) => {
                self.push(problem);
                Err(self.n)
            }
        };
        env.set(fold(name), value);
        self.out.names.push(range.clone());
        true
    }

    /// O "=" do fim da linha pede o resultado da conta mais longa que termina nele: o
    /// que vem antes é prosa, e a pontuação logo antes do "=" também ("Quanto dá
    /// 2 × 3? ="). Sem conta, só há erro se o fim do trecho é conta.
    fn request(&mut self, env: &Env, region: Range<usize>, eq: usize) {
        let mut end = region.end;
        while end > region.start && self.toks.get(end - 1).is_some_and(|t| matches!(t.kind, Kind::Other(_))) {
            end -= 1;
        }
        let seg = self.seg(region.start..end, env);
        let n = seg.toks.len();
        let mut s = 0;
        while s < n {
            let expr = match seg.parse(s..n) {
                Ok(expr) => expr,
                // As que começam antes do problema falham do mesmo jeito.
                Err(fault) => {
                    s = fault.next_start().filter(|&next| next > s).unwrap_or(s + 1);
                    continue;
                }
            };
            if s > 0 {
                if let Some(problem) = self.left_break(&seg, s) {
                    self.push(problem);
                    return;
                }
                if n - s == 1 && matches!(seg.toks[s].kind, Kind::Num(_)) {
                    // "Somar 2 e 3" não mostra 3; "Paguei 30 =" fica como texto.
                    return;
                }
            }
            self.show(env, &expr, eq);
            return;
        }
        // A conta que vai até o fim, sem a prosa que sobra depois dela ("2 + de").
        let Some(last) = (0..n).rev().find(|&i| seg.pair(i).is_some_and(|p| !parser::prose(p))) else {
            return;
        };
        let chain = seg.chain_start(last)..last + 1;
        if seg.is_math(chain.clone())
            && let Some(problem) = self.chain_problem(&seg, chain)
        {
            self.push(problem);
        }
    }

    /// A conta achada começa em `s`, mas o que vem antes mostra que ela está quebrada:
    /// um operador pedindo número, a conjunção "e", outro operando sem operação ou,
    /// antes do sinal que abre a conta, conta com erro ("2 × itens + 1").
    fn left_break(&self, seg: &Seg, s: usize) -> Option<Problem> {
        let (prev, role) = seg.pair(s - 1)?;
        if let Some(run) = seg.malformed(s - 1) {
            return Some(self.malformed(seg, run));
        }
        let n = seg.toks.len();
        // Operador pedindo número, ou o ";" de uma função que não fechou ("máx(3; 7").
        if parser::cola(prev, role) || prev.kind == Kind::Semi {
            return self.chain_problem(seg, seg.chain_start(s - 1)..n);
        }
        // Parênteses com conta: antes também é conta ("(2 3) + 1", "(2 × itens) + 1");
        // sem par ou com prosa dentro, são prosa ("Oba :) 2 × 3", "(nota) 2 × 3").
        if let Some(open) = seg.math_group(s - 1) {
            return self.chain_problem(seg, seg.chain_start(open)..n);
        }
        if *role == Role::Conj {
            return Some(Problem { range: prev.range.clone(), message: CONJ.into() });
        }
        let sign = seg.toks.get(s).is_some_and(|t| matches!(t.kind, Kind::Plus | Kind::Minus));
        if !sign && parser::ends_operand(prev, role) {
            return self.missing_operation(seg, s);
        }
        if sign && seg.broken_before_sign(s - 1) {
            return self.chain_problem(seg, seg.chain_start(s - 1)..n);
        }
        None
    }

    /// A linha (contando de 0) em que o nome é definido mais abaixo do token `at`
    /// desta linha.
    fn later(&self, key: &str, at: usize) -> Option<usize> {
        let places = self.below.get(key)?;
        places.iter().find(|&&place| place > (self.n, at)).map(|&(line, _)| line)
    }

    /// "“1,234.5” não é um número", sublinhando o número todo.
    fn malformed(&self, seg: &Seg, run: Range<usize>) -> Problem {
        let range = seg.toks.get(run.start).map_or(0, |t| t.range.start)..seg.toks.get(run.end.saturating_sub(1)).map_or(0, |t| t.range.end);
        let message = format!("“{}” não é um número", self.slice(range.clone()));
        Problem { range, message }
    }

    /// O primeiro problema da conta em `chain` (tokens do trecho).
    fn chain_problem(&self, seg: &Seg, chain: Range<usize>) -> Option<Problem> {
        let fault = seg.parse(chain.clone()).err()?;
        Some(self.message(fault, seg, chain))
    }

    fn show(&mut self, env: &Env, expr: &Expr, eq: usize) {
        self.mark(expr);
        match self.eval(env, expr) {
            Ok(v) => {
                let (text, number, value) = v.show(self.ctx.style);
                let at = self.toks[eq].range.end;
                self.out.result = Some(Shown { at, text, number, value });
            }
            Err(problem) => self.push(problem),
        }
    }

    /// A mensagem de um problema da leitura. Nome desconhecido e "e" sublinham só o
    /// token; o resto, a conta inteira (`chain`, em tokens do trecho).
    fn message(&self, fault: Fault, seg: &Seg, chain: Range<usize>) -> Problem {
        let range = |i: usize| seg.toks.get(i).map_or(0..0, |t| t.range.clone());
        let word = |i: usize| self.slice(range(i));
        let whole = range(chain.start).start..range(chain.end.saturating_sub(1)).end;
        let (range, message) = match fault {
            Fault::Unknown(i) => (range(i), self.unknown(word(i), seg.start + i)),
            Fault::NoNumber(i) => (whole, format!("Falta um número depois de “{}”", word(i))),
            Fault::NoNumberBefore(i) => (whole, format!("Falta um número antes de “{}”", word(i))),
            Fault::NoOperation(i) => match self.missing_operation(seg, i) {
                Some(problem) => return problem,
                None => (whole, format!("Falta uma operação antes de “{}”", word(i))),
            },
            Fault::Unclosed(_) => (whole, "Parêntese aberto sem fechar".into()),
            Fault::Unopened(_) => (whole, "Parêntese fechado sem abrir".into()),
            Fault::Conj(i) => (range(i), CONJ.into()),
            Fault::NotUnit(_, Some(i)) => (range(i), format!("“{}” não é uma unidade", word(i))),
            Fault::NotUnit(i, None) => (whole, format!("Esperava uma unidade depois de “{}”", word(i))),
            Fault::Arity(i, end, func) => (range(i).start..range(end).end, eval::arity_message(word(i), func)),
            Fault::Separator(_) => (whole, "Use “;” para separar os números".into()),
            Fault::AfterConvert(_, convert) => (whole, format!("A conversão com “{}” fica no fim da conta", word(convert))),
            Fault::TooDeep(_) => (whole, "Conta comprida demais".into()),
            Fault::Stray(i) if word(i) == ";" => (whole, "“;” só separa os números de uma função".into()),
            // Reserva: a prosa fica fora das contas relidas aqui (`linked` corta nela, e
            // a pontuação antes do "=" sai antes da busca).
            Fault::Stray(i) => (whole, format!("“{}” não faz parte da conta", word(i))),
        };
        Problem { range, message }
    }

    /// "“x” é definida mais abaixo…" se o nome é definido depois (`at`: o token na
    /// linha); senão, "“x” não é uma variável nem uma unidade".
    fn unknown(&self, word: &str, at: usize) -> String {
        match self.later(&fold(word), at) {
            Some(line) => format!("“{word}” é definida mais abaixo (linha {}): defina antes de usar", line + 1),
            None => format!("“{word}” não é uma variável nem uma unidade"),
        }
    }

    /// "Falta uma operação entre “a” e “b”": `a` é o menor operando que termina logo
    /// antes de `at`, e `b` o menor que começa em `at`. `None` se antes não há operando
    /// ("Oba :) 2 × 3": o ")" é prosa).
    fn missing_operation(&self, seg: &Seg, at: usize) -> Option<Problem> {
        // A prosa entre os dois ("10 de 3", "1, 2") fica no trecho, fora dos números.
        let mut end = at;
        while end > 0 && seg.pair(end - 1).is_some_and(parser::prose) {
            end -= 1;
        }
        let a = seg.operand_before(end)?;
        let b = seg.operand_after(at).unwrap_or(at + 1);
        let left = seg.toks.get(a..end)?;
        let right = seg.toks.get(at..b)?;
        let range = left.first()?.range.start..right.last()?.range.end;
        let message = format!("Falta uma operação entre “{}” e “{}”", self.snippet(left), self.snippet(right));
        Some(Problem { range, message })
    }

    /// O texto de alguns tokens como foram escritos, com os espaços entre eles (sem o
    /// espaço antes de "%", "!", "°" e expoente: "10 %" vira "10%").
    fn snippet(&self, toks: &[Token]) -> String {
        let mut s = String::new();
        for (i, t) in toks.iter().enumerate() {
            let postfix = matches!(t.kind, Kind::Percent | Kind::Bang | Kind::Degree | Kind::Sup(_));
            if i > 0 && t.space_before && !postfix {
                s.push(' ');
            }
            s.push_str(self.slice(t.range.clone()));
        }
        s
    }
}

/// O valor de uma definição: a conta mais longa no começo de `seg`. Se ela para num
/// lugar que mostra conta quebrada (operador ou nome desconhecido colado num número),
/// ou se nem começa mas o trecho começa como conta ("(2 + 3", "preço × 2"), é o
/// problema; se para em prosa ("1.200 por mês"), vale o que leu; sem conta nenhuma,
/// não é definição.
fn definition(seg: &Seg) -> Option<Def> {
    let n = seg.toks.len();
    let found = longest_prefix(seg);
    if let Some(run) = found.as_ref().and_then(|(k, _)| seg.malformed(*k)) {
        return Some(Def::Malformed(run));
    }
    let broken = match &found {
        Some((k, _)) if *k == n => false,
        Some((k, _)) => breaks(seg, *k),
        None => seg.starts_broken(),
    };
    if broken {
        let fault = seg.parse(0..n).err()?;
        return Some(Def::Broken(fault, 0..seg.chain_end(0)));
    }
    found.map(|(_, e)| Def::Value(e))
}

/// O token `at` liga a palavra antes dele à conta: "×", "÷", "^", "→", pós-fixo
/// ("lado²") ou sinal antes de número ("prço + 2"). O sinal antes de outra palavra é
/// traço de prosa ("pago - falta recibo"), e a "!", exclamação.
fn binds_word(seg: &Seg, at: usize) -> bool {
    let Some((t, _)) = seg.pair(at) else {
        return false;
    };
    match t.kind {
        Kind::Plus | Kind::Minus => seg.pair(at + 1).is_some_and(|(n, r)| starts_strict(n, r) && !matches!(n.kind, Kind::Plus | Kind::Minus)),
        Kind::Times | Kind::Divide | Kind::Pow | Kind::Arrow | Kind::Percent | Kind::Degree | Kind::Sup(_) => true,
        _ => false,
    }
}

/// A conta mais longa no começo do trecho, e quantos tokens ela usa.
fn longest_prefix(seg: &Seg) -> Option<(usize, Expr)> {
    let mut k = seg.toks.len();
    while k > 0 {
        match seg.parse(0..k) {
            Ok(e) => return Some((k, e)),
            // As que vão além do problema falham do mesmo jeito.
            Err(fault) => k = fault.next_end().filter(|&end| end < k).unwrap_or(k - 1),
        }
    }
    None
}

/// A conta da definição parou no token `k` de um jeito que mostra que está quebrada:
/// operador, ou nome desconhecido colado no número ("2itens"; a palavra da conta
/// colada, "10de", é prosa).
fn breaks(seg: &Seg, k: usize) -> bool {
    let Some((stop, role)) = seg.pair(k) else {
        return false;
    };
    let operator = matches!(stop.kind, Kind::Plus | Kind::Minus | Kind::Times | Kind::Divide | Kind::Pow | Kind::Arrow | Kind::Close)
        || matches!(role, Role::Times | Role::Mod | Role::Convert);
    let glued = matches!(role, Role::Unknown(_))
        && !stop.space_before
        && k.checked_sub(1).and_then(|p| seg.toks.get(p)).is_some_and(|p| matches!(p.kind, Kind::Num(_)));
    operator || glued
}

/// O token começa uma conta de verdade (um nome desconhecido não: é prosa).
fn starts_strict(t: &Token, role: &Role) -> bool {
    matches!(t.kind, Kind::Num(_) | Kind::Open | Kind::Root(_) | Kind::Plus | Kind::Minus)
        || matches!(role, Role::Var(_) | Role::Func(_) | Role::Const(_))
}

/// O erro de um nome que não pode ser variável: palavra da conta, função ou π.
fn bad_name(name: &str) -> Option<String> {
    if parser::KEYWORDS.iter().any(|k| name.eq_ignore_ascii_case(k)) {
        Some(format!("“{name}” é uma palavra da conta: escolha outro nome"))
    } else if Func::find(&fold(name)).is_some() {
        Some(format!("“{name}” é o nome de uma função: escolha outro nome"))
    } else if name == "π" || name.eq_ignore_ascii_case("pi") {
        Some("π é uma constante".into())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Por linha: o resultado, "erro: …" (o primeiro problema) ou "".
    fn lines_with(text: &str, ctx: Context) -> Vec<String> {
        evaluate(text, &ctx)
            .iter()
            .map(|l| match (&l.result, l.problems.first()) {
                (Some(s), _) => s.text.clone(),
                (None, Some(p)) => format!("erro: {}", p.message),
                (None, None) => String::new(),
            })
            .collect()
    }

    /// Em Graus, com o `Style` padrão.
    fn r(text: &str) -> Vec<String> {
        lines_with(text, Context::default())
    }

    /// A última linha.
    fn last(text: &str) -> String {
        r(text).pop().unwrap_or_default()
    }

    fn rad(text: &str) -> String {
        lines_with(text, Context { radians: true, ..Context::default() }).pop().unwrap_or_default()
    }

    /// O primeiro problema da última linha, com o texto sublinhado.
    fn underlined(text: &str) -> (String, String) {
        let lines = evaluate(text, &Context::default());
        let p = lines.last().and_then(|l| l.problems.first()).cloned().expect("sem problema");
        (text[p.range.clone()].to_string(), p.message)
    }

    fn check(cases: &[(&str, &str)]) {
        for (text, want) in cases {
            assert_eq!(last(text), *want, "{text}");
        }
    }

    #[test]
    fn contas_e_precedencia() {
        check(&[
            ("120 × 3 =", "360"),
            ("2 + 3 × 4 =", "14"),
            ("(2 + 3) × 4 =", "20"),
            ("2^3^2 =", "512"),
            ("−2^2 =", "-4"),
            ("2^−1 =", "0,5"),
            ("10 − 2 − 3 =", "5"),
            ("8 / 2 / 2 =", "2"),
            ("5! =", "120"),
            ("√16² =", "16"),
            ("2√9 =", "6"),
            ("1.500 + 2 × 7 =", "1.514"),
            ("120*3=", "360"),
            ("2 ** 3 =", "8"),
            ("3!² =", "36"),
            ("√(9 + 16) =", "5"),
        ]);
    }

    #[test]
    fn precisao_como_o_engine() {
        check(&[
            ("0,1 + 0,2 − 0,3 =", "0"),
            ("1/3 × 3 =", "1"),
            ("1 ÷ 3 =", "0,33333333333"),
            ("2^0,5 × 2^0,5 =", "2"),
            ("10^400 =", "erro: Número grande demais"),
            ("1,500 + 1 =", "2,5"),
            // 12 algarismos, como na tela (a tabela C da proposta mostra 13).
            ("0,5! =", "0,88622692545"),
            // Vírgula de milhar do inglês: não é aceita.
            ("1,234.5 + 1 =", "erro: “1,234.5” não é um número"),
            ("1 + 1,234,567.25 =", "erro: “1,234,567.25” não é um número"),
            ("Versão 1.2.3 =", "erro: “1.2.3” não é um número"),
        ]);
        assert_eq!(last("a = 1,234.5"), "erro: “1,234.5” não é um número");
        assert_eq!(last("Paguei 25. Depois 2 × 3 ="), "6");
    }

    #[test]
    fn implicita_e_fracao() {
        check(&[
            ("2π =", "6,28318530718"),
            ("3(1 + 2) =", "9"),
            ("(1 + 1)(2 + 3) =", "10"),
            ("6/2(1+2) =", "9"),
            ("6 / 2(1+2) =", "1"),
            ("1/2π =", "1,57079632679"),
            ("2e =", "5,43656365692"),
            ("2e3 =", "2.000"),
        ]);
    }

    #[test]
    fn x_vezes_e_variavel() {
        check(&[("47x96=", "4.512"), ("2 x 3 =", "6"), ("120 x 3 =", "360"), ("120x3=", "360")]);
        assert_eq!(r("x = 4\n3x ="), ["", "12"]);
        assert_eq!(last("x = 4\nx x x ="), "16");
        assert_eq!(last("x = 4\n2 x −3 ="), "5");
        assert_eq!(last("x × 2 ="), "erro: “x” não é uma variável nem uma unidade");
    }

    #[test]
    fn variaveis() {
        assert_eq!(r("aluguel = 1.200\naluguel × 12 ="), ["", "14.400"]);
        assert_eq!(last("a = 1\na = a + 1\na ="), "2");
        assert_eq!(r("x × 2 =\nx = 3")[0], "erro: “x” é definida mais abaixo (linha 2): defina antes de usar");
        assert_eq!(last("Preço = 8,50\npreco × 2 ="), "17");
        assert_eq!(r("custo = 25 × 4 =\ncusto + 1 ="), ["100", "101"]);
        assert_eq!(last("a = 2; b = 3; a × b ="), "6");
        assert_eq!(last("sen = 3"), "erro: “sen” é o nome de uma função: escolha outro nome");
        assert_eq!(last("π = 3"), "erro: π é uma constante");
        assert_eq!(last("de = 3"), "erro: “de” é uma palavra da conta: escolha outro nome");
        assert_eq!(last("h = 10\nh × 2 ="), "20");
        assert_eq!(r("a = 1/0\na + 1 ="), ["erro: Divisão por zero", "erro: “a” tem um erro (linha 1)"]);
        assert_eq!(last("Meu aluguel = 1.200\naluguel ="), "1.200");
        assert_eq!(last("aluguel = 1.200 por mês\naluguel ="), "1.200");
        // Operador quebrado: a variável fica com o erro.
        assert_eq!(r("a = 2 × itens\na ="), ["erro: “itens” não é uma variável nem uma unidade", "erro: “a” tem um erro (linha 1)"]);
        assert_eq!(last("a = 2itens"), "erro: “itens” não é uma variável nem uma unidade");
        assert_eq!(r("Total = 5 itens\ntotal ="), ["", "5"]);
        assert_eq!(last("a = (2 + 3"), "erro: Parêntese aberto sem fechar");
        // A constante e vira variável; o valor fica com a precisão inteira.
        assert_eq!(last("e = 2\ne × 3 ="), "6");
        assert_eq!(last("t = 1/3\nt × 3 − 1 ="), "0");
        // Na mesma linha, para a direita; nome sem acento nem maiúscula.
        assert_eq!(last("Área = 2; ÁREA × 3 ="), "6");
    }

    #[test]
    fn prosa_e_igual() {
        check(&[
            ("Hoje gastei 2 × 3 =", "6"),
            // O resultado só aparece no "=" do fim da linha (decisão do plano).
            ("Se comprar 2 × 3 = caixas", ""),
            ("2 + 2 = 4", ""),
            ("Hotel: 3 × 180 =", "540"),
            ("Total de 2 itens × 3 =", "erro: “itens” não é uma variável nem uma unidade"),
            ("Somar 2 e 3 =", "erro: “e” não é uma operação: use +, −, × ou ÷"),
            ("Resultado =", ""),
            ("Linha 2 × 3", ""),
            ("Mercado − 2 × 30 =", "-60"),
            ("Oba :) 2 × 3 =", "6"),
            ("Paguei 30 =", ""),
            ("O total é 2 × 3 =", "6"),
            ("2 × 3 =   ", "6"),
        ]);
        assert_eq!(underlined("Total de 2 itens × 3 =").0, "itens");
        // "5 km = 3,1 mi" não define km.
        let lines = evaluate("5 km = 3,1 mi\nkm =", &Context::default());
        assert!(lines.iter().all(|l| l.result.is_none() && l.problems.is_empty() && l.names.is_empty()));
        // O "=" do resultado é o último, também depois de ";".
        assert_eq!(last("a = 5; a × 2 ="), "10");
        assert_eq!(last("2 × 3 =; 4"), "");
    }

    #[test]
    fn porcentagem() {
        check(&[
            ("50% =", "0,5"),
            ("100 + 10% =", "110"),
            ("150 − 20% =", "120"),
            ("20% de 150 =", "30"),
            ("200 × 15% =", "30"),
            ("30/150 em % =", "20%"),
            ("0,2 para % =", "20%"),
            ("10 mod 3 =", "1"),
            ("−7 mod 3 =", "-1"),
            ("10 % 3 =", "erro: Falta uma operação entre “10%” e “3”"),
            ("10% + 5% =", "0,15"),
        ]);
        assert_eq!(last("taxa = 10%\n100 + taxa ="), "110");
        assert_eq!(last("taxa = 10%\ntaxa de 150 ="), "15");
    }

    #[test]
    fn funcoes_e_angulos() {
        check(&[
            ("sen 30 =", "0,5"),
            ("sen(30) + 1 =", "1,5"),
            ("cos 60 =", "0,5"),
            ("tg 45 =", "1"),
            ("tg 90 =", "erro: tg não existe para 90°"),
            ("arcsen 0,5 =", "30°"),
            ("sen 30º =", "0,5"),
            ("sen 1800 =", "0"),
            ("ln e =", "1"),
            ("log 1000 =", "3"),
            ("log(8; 2) =", "3"),
            ("log₂ 8 =", "3"),
            ("raiz(27; 3) =", "3"),
            ("∛−8 =", "-2"),
            ("máx(3; 7; 5) =", "7"),
            ("mín(3; 7; 5) =", "3"),
            ("arred(2,567; 2) =", "2,57"),
            ("arred(2,5) =", "3"),
            ("arred(−2,5) =", "-3"),
            ("hipot(3; 4) =", "5"),
            ("√−1 =", "erro: Raiz de número negativo"),
            ("abs(−3) =", "3"),
            ("exp 1 =", "2,71828182846"),
            ("fat 5 =", "120"),
            ("2 sen 30 =", "1"),
            ("sen 30 cos 60 =", "0,25"),
            ("ln 0 =", "erro: ln não existe para 0"),
            ("hipot(3) =", "erro: hipot precisa de 2 números"),
        ]);
        assert_eq!(rad("sen(π/6) ="), "0,5");
        assert_eq!(rad("cos π ="), "-1");
        assert_eq!(rad("sen 30° ="), "0,5");
        assert_eq!(rad("arcsen 1 ="), "1,57079632679 rad");
    }

    #[test]
    fn erros_com_trecho() {
        assert_eq!(underlined("2 + ="), ("2 +".into(), "Falta um número depois de “+”".into()));
        assert_eq!(underlined("(2 + 3 =").1, "Parêntese aberto sem fechar");
        assert_eq!(underlined("2 + 3) =").1, "Parêntese fechado sem abrir");
        assert_eq!(underlined("1/0 ="), ("1/0".into(), "Divisão por zero".into()));
        assert_eq!(underlined("2 3 ="), ("2 3".into(), "Falta uma operação entre “2” e “3”".into()));
        assert_eq!(underlined("Somar 2 e 3 =").0, "e");
        assert_eq!(underlined("Ação: preço × 2 =").0, "preço");
        assert_eq!(underlined("Gastei 2 × 30 em comida ="), ("comida".into(), "“comida” não é uma unidade".into()));
        assert_eq!(underlined("Gastei 2 × 30 em ="), ("2 × 30 em".into(), "Esperava uma unidade depois de “em”".into()));
        assert_eq!(underlined("tg 90 =").0, "tg 90");
        // Parênteses que fecham: o que vem antes também é conta.
        assert_eq!(last("(2 3) + 1 ="), "erro: Falta uma operação entre “2” e “3”");
        assert_eq!(last("(1 + 2) 3 ="), "erro: Falta uma operação entre “(1 + 2)” e “3”");
        assert_eq!(last("Oba :) + 1 ="), "1");
        assert_eq!(last("(nota) 2 × 3 ="), "6");
    }

    #[test]
    fn formato() {
        let two = Context { style: Style { decimals: Some(2), grouping: true }, ..Context::default() };
        assert_eq!(lines_with("120 × 3 =", two), ["360,00"]);
        let plain = Context { style: Style { decimals: None, grouping: false }, ..Context::default() };
        assert_eq!(lines_with("aluguel = 1.200\naluguel × 12 =", plain), ["", "14400"]);
        let shown = evaluate("1.234,5 + 0 =", &Context::default()).remove(0).result.expect("resultado");
        assert_eq!((shown.text.as_str(), shown.number.as_str(), shown.value), ("1.234,5", "1234,5", 1234.5));
        assert_eq!(shown.at, "1.234,5 + 0 =".len());
    }

    #[test]
    fn editar_atualiza() {
        assert_eq!(last("a = 2\na × 3 ="), "6");
        assert_eq!(last("a = 5\na × 3 ="), "15");
    }

    #[test]
    fn with_results_escreve_depois_do_igual() {
        let text = "120 × 3 =\nSe comprar 2 × 3 = caixas\naluguel = 1.200\naluguel × 12 =";
        let lines = evaluate(text, &Context::default());
        assert_eq!(with_results(text, &lines), "120 × 3 = 360\nSe comprar 2 × 3 = caixas\naluguel = 1.200\naluguel × 12 = 14.400");
        // Resultados velhos (de outro texto) não estouram.
        assert_eq!(with_results("ab", &lines), "ab");
    }

    #[test]
    fn variaveis_coloridas() {
        let text = "aluguel = 1.200\nTotal: aluguel × 12 =";
        let lines = evaluate(text, &Context::default());
        let names: Vec<&str> = lines.iter().flat_map(|l| l.names.iter().map(|r| &text[r.clone()])).collect();
        assert_eq!(names, ["aluguel", "aluguel"]);
    }

    #[test]
    fn linhas_longas_e_aninhadas() {
        let soma = format!("{}1=", "1+".repeat(999));
        assert_eq!(soma.chars().count(), 2000);
        assert_eq!(last(&soma), "1.000");
        assert_eq!(last(&format!("1{soma}")), "");
        assert_eq!(last(&format!("{}1{}=", "(".repeat(600), ")".repeat(600))), "erro: Conta comprida demais");
        assert_eq!(last(&format!("{}1 =", "−".repeat(1500))), "erro: Conta comprida demais");
    }

    #[test]
    fn nunca_estoura() {
        let note = "Preço = 8,50; a = (2 + 3) × 4!\nHoje gastei 2 × preço − 10% =\nsen 30° + log(8; 2) + √16² + ∛−8 + máx(1; 2) =\n30/150 em % =\nR$ 1.200 + 2e3 − 1,234.5 =\nx = π/2; 2x³ + e^2 → % =\nÁrea: 3 m² × ½ = ok 🙂 ;;; == ))((";
        let chars: Vec<(usize, char)> = note.char_indices().collect();
        for (i, _) in chars {
            for text in [&note[..i], &note[i..]] {
                for ctx in [Context::default(), Context { radians: true, style: Style { decimals: Some(3), grouping: false } }] {
                    let lines = evaluate(text, &ctx);
                    assert_eq!(lines.len(), text.split('\n').count());
                    for l in &lines {
                        let ranges = l.names.iter().chain(l.problems.iter().map(|p| &p.range)).cloned();
                        for r in ranges.chain(l.result.iter().map(|s| s.at..s.at)) {
                            assert!(text.get(r.clone()).is_some(), "{r:?} em {text:?}");
                        }
                    }
                    with_results(text, &lines);
                }
            }
        }
    }

    const ITENS: &str = "erro: “itens” não é uma variável nem uma unidade";
    const ABERTO: &str = "erro: Parêntese aberto sem fechar";
    const SEPARAR: &str = "erro: Use “;” para separar os números";
    const GRANDE: &str = "erro: Número grande demais";

    fn below(name: &str, line: usize) -> String {
        format!("erro: “{name}” é definida mais abaixo (linha {line}): defina antes de usar")
    }

    #[test]
    fn nome_desconhecido_antes_de_sinal() {
        // A conta não perde o começo: o sinal é a operação, e o nome, o erro.
        check(&[
            ("2 × itens =", ITENS),
            ("2 × itens + 1 =", ITENS),
            ("(2 × itens) + 1 =", ITENS),
            ("1.500 + itens − 2 =", ITENS),
            ("Aluguel = 1.200\nTotal: 12 × aluguell + 500 =", "erro: “aluguell” não é uma variável nem uma unidade"),
            ("lado² + 1 =", "erro: “lado” não é uma variável nem uma unidade"),
            ("sen z + 1 =", "erro: “z” não é uma variável nem uma unidade"),
            ("Comprei 2 pães − 1 =", "erro: “pães” não é uma variável nem uma unidade"),
            // Palavra solta antes do sinal continua rótulo (também depois do traço de
            // uma lista ou de um ")" de prosa), e "!" colada, exclamação.
            ("Mercado − 2 × 30 =", "-60"),
            ("- Mercado − 2 × 30 =", "-60"),
            ("Oba :) Mercado − 2 =", "-2"),
            ("Oba :) + 1 =", "1"),
            ("(nota) + 1 =", "1"),
            ("Que bom! −5 + 10 =", "5"),
        ]);
        assert_eq!(r("2 × x + 1 =\nx = 3")[0], below("x", 2));
        // Com erro no enunciado, o "=" fica sem resultado (aqui, o da definição).
        for text in ["Total = 3 × prço + 5 =", "x = b + 1 =", "preço = preço^ %1.500²e ="] {
            let line = evaluate(text, &Context::default()).remove(0);
            assert_eq!((line.result, line.problems.len()), (None, 1), "{text}");
        }
        assert_eq!(last("a = 1/0; 2 × 3 ="), "6");
    }

    #[test]
    fn prosa_no_fim_e_grupos() {
        check(&[
            // A prosa que sobra no fim não esconde o operador sem número.
            ("2 + de =", "erro: Falta um número depois de “+”"),
            ("10 para em =", "erro: Esperava uma unidade depois de “para”"),
            ("2 × 3 de =", ""),
            // Parênteses vazios de uma função são conta; "10² itens", como "2 itens", não.
            ("cos( ) 2e =", "erro: Falta um número depois de “(”"),
            ("10² itens =", ""),
            ("Total de 2 itens =", ""),
            ("lado² =", "erro: “lado” não é uma variável nem uma unidade"),
            ("2 (nota) =", ""),
            // O "e" entre números dentro dos parênteses é o erro da conjunção.
            ("(10 e π) =", "erro: “e” não é uma operação: use +, −, × ou ÷"),
        ]);
        // Parênteses de prosa não entram na conta colada a eles.
        assert_eq!(underlined("(nota) máx(preço; 2) =").0, "preço");
        assert_eq!(underlined("(nota) (2 × itens) + 1 =").0, "itens");
        // Na definição, o trecho vai até o ")" do "(" da conta.
        assert_eq!(underlined("a = máx(π 0,5)"), ("máx(π 0,5)".into(), "Use “;” para separar os números".into()));
        // Prosa entre números dentro de parênteses que fecham: falta a operação.
        assert_eq!(underlined("a = (2 + 3 de 4)"), ("3 de 4".into(), "Falta uma operação entre “3” e “4”".into()));
        assert_eq!(underlined("sen(30, 60) ="), ("30, 60".into(), "Falta uma operação entre “30” e “60”".into()));
        assert_eq!(underlined("(log(8; 2) de 3) =").1, "Falta uma operação entre “log(8; 2)” e “3”");
        assert_eq!(underlined("2 × ((itens)) reais =").0, "itens");
    }

    #[test]
    fn variavel_de_baixo_antes_de_usar() {
        let erro_total = "erro: “total” tem um erro (linha 1)";
        assert_eq!(r("total = preço × 2\npreço = 10\ntotal ="), [below("preço", 2).as_str(), "", erro_total]);
        assert_eq!(r("total = 2 × preço\npreço = 10\ntotal ="), [below("preço", 2).as_str(), "", erro_total]);
        assert_eq!(r("a = preço\npreço = 5")[0], below("preço", 2));
        assert_eq!(r("x + 2 =\nx = 3")[0], below("x", 2));
        assert_eq!(r("z + 1 =\nz = 5")[0], below("z", 2));
        assert_eq!(r("preço + 10% =\npreço = 50")[0], below("preço", 2));
        assert_eq!(r("total − 10% =\ntotal = 200")[0], below("total", 2));
        assert_eq!(r("preço =\npreço = 5")[0], below("preço", 2));
        assert_eq!(last("total = prço × 2"), "erro: “prço” não é uma variável nem uma unidade");
        assert_eq!(last("total = prço + 2"), "erro: “prço” não é uma variável nem uma unidade");
        // Prosa depois do "=" continua prosa (o traço entre palavras não liga à conta).
        check(&[("Status = pago - falta recibo", ""), ("Nome = Fabio", ""), ("Resultado =", "")]);
    }

    #[test]
    fn funcao_sem_fechar_com_ponto_e_virgula() {
        check(&[
            ("máx(3 =", ABERTO),
            ("máx(3; 7 =", ABERTO),
            ("máx(3; 7 + 1 =", ABERTO),
            ("log(8; 1 + 1 =", ABERTO),
            ("hipot(3; 4 =", ABERTO),
            ("máx(10; 20; 3 × 1 =", ABERTO),
            ("máx(1; 2) + máx(3; 4 + 5 =", ABERTO),
            ("(2; 3 + 1 =", "erro: “;” só separa os números de uma função"),
        ]);
    }

    #[test]
    fn e_de_prosa_antes_da_conta() {
        check(&[
            ("Pão, leite e 3 × 2 =", "6"),
            ("Arroz e 2 × 5 =", "10"),
            ("Café e 2 × 4,50 =", "9"),
            ("Comprei pão e 3 × 2,50 =", "7,5"),
            ("Luz e 120 + 80 =", "200"),
            ("Comprei pão e −3 + 10 =", "7"),
            ("E 2 × 3 =", "6"),
            // A conjunção entre números e a constante continuam.
            ("Somar 2 e 3 =", "erro: “e” não é uma operação: use +, −, × ou ÷"),
            ("ln e + 1 =", "2"),
            ("Nota: e − e =", "0"),
            ("e − e =", "0"),
            ("e × 2 =", "5,43656365692"),
            ("e x (2) =", "5,43656365692"),
            ("Nota e x 2 =", "5,43656365692"),
        ]);
        assert_eq!(last("v = e x 2\nv ="), "5,43656365692");
        // Logo depois do "=", o "e" é a constante (o valor da definição é conta).
        assert_eq!(last("v = e 1,5 gastei\nv ="), "2,71828182846");
    }

    #[test]
    fn pontuacao_e_conversao_sem_mensagem_solta() {
        check(&[
            // A pontuação antes do "=" é prosa: a conta vem antes dela.
            ("Quanto custa 2 × 3? =", "6"),
            ("Total 2 + 3. =", "5"),
            ("Paguei 25. =", ""),
            ("Oba :) =", ""),
            ("Hoje (sábado) gastei muito =", ""),
            ("(pagar amanhã) =", ""),
            ("Paguei 25. × 3 =", "erro: Falta um número antes de “×”"),
            ("30/150 em % + 1 =", "erro: A conversão com “em” fica no fim da conta"),
            ("30/150 em % =", "20%"),
            ("2 × 3 de =", ""),
        ]);
        // A palavra da conta colada no número é prosa, como solta ("10 de 2").
        assert_eq!(r("a = 10de 2\na ="), ["", "10"]);
    }

    #[test]
    fn prosa_com_parentese_ou_traco_depois_do_igual() {
        assert_eq!(r("Obs = (pagar amanhã)\nobs ="), ["", ""]);
        check(&[
            ("Lembrete = - pagar aluguel", ""),
            ("Nota = (ver abaixo)", ""),
            // Conta de verdade depois do "=" continua com o erro.
            ("a = (2 + 3", ABERTO),
            ("a = −", "erro: Falta um número depois de “−”"),
            ("a = (2 × itens", ITENS),
        ]);
    }

    #[test]
    fn fatorial_grande_demais() {
        check(&[("1000! =", GRANDE), ("750! =", GRANDE), ("fat 1000 =", GRANDE), ("fat(1e300) =", GRANDE), ("1e10! =", GRANDE), ("171! =", GRANDE), ("(−1000,5)! =", "0")]);
    }

    #[test]
    fn parenteses_equilibrados() {
        assert_eq!(underlined("(2 + 3 4) ="), ("3 4".into(), "Falta uma operação entre “3” e “4”".into()));
        assert_eq!(underlined("sen(30 60) ="), ("30 60".into(), "Falta uma operação entre “30” e “60”".into()));
        assert_eq!(underlined("(2 + 3 e 4) =").0, "e");
        assert_eq!(last("a = (2 + 3 4)"), "erro: Falta uma operação entre “3” e “4”");
        // Vírgula (ou nada) entre os números de uma função de vários números.
        check(&[("máx(1, 2) =", SEPARAR), ("hipot(3, 4) =", SEPARAR), ("máx(1 2) =", SEPARAR), ("log(8 2) =", SEPARAR)]);
        assert_eq!(last("a = máx(1, 2)"), SEPARAR);
        assert_eq!(underlined("máx(1, 2) =").0, "máx(1, 2)");
    }

    /// O melhor tempo de `text` em algumas rodadas, em milissegundos.
    fn best_ms(text: &str) -> f64 {
        (0..5)
            .map(|_| {
                let t0 = std::time::Instant::now();
                std::hint::black_box(evaluate(std::hint::black_box(text), &Context::default()));
                t0.elapsed().as_secs_f64() * 1e3
            })
            .fold(f64::INFINITY, f64::min)
    }

    #[test]
    fn linha_quebrada_comprida_custa_pouco() {
        // Antes, cada uma levava dezenas de milissegundos (a busca era quadrática).
        let lines = [format!("({}1) 2 =", "1+".repeat(990)), format!("{}{}1 =", "1+".repeat(930), "(".repeat(70)), format!("{}1)) 2 =", "1+".repeat(995))];
        for text in &lines {
            assert!(text.chars().count() <= MAX_LINE);
            let ms = best_ms(text);
            assert!(ms < 10.0, "{ms:.2} ms em {}…", &text[..20]);
        }
        assert_eq!(last(&lines[1]), "erro: Conta comprida demais");
    }

    #[test]
    fn arvore_funda_cabe_numa_pilha_pequena() {
        // A espinha de uma conta de 2.000 caracteres tem mil nós: avaliar, colorir e
        // desmontar sem recursão.
        let texts = [
            format!("1{}=", "!".repeat(1998)),
            format!("{}1=", "1+".repeat(999)),
            format!("x = 1\n{}x =", "x+".repeat(997)),
            format!("{}1 em % =", "2×".repeat(995)),
            format!("{}1{}=", "(".repeat(999), ")".repeat(999)),
            format!("{}1 =", "−".repeat(1990)),
        ];
        let run = std::thread::Builder::new().stack_size(256 << 10).spawn(move || texts.iter().map(|t| r(t).pop().unwrap_or_default()).collect::<Vec<_>>());
        let out = run.ok().and_then(|h| h.join().ok()).unwrap_or_default();
        assert_eq!(out.len(), 6);
        assert_eq!((out[1].as_str(), out[2].as_str()), ("1.000", "998"));
        assert_eq!(out[4], "erro: Conta comprida demais");
    }

    /// As mensagens que as notas podem dar: as da tabela I e as declaradas no motor,
    /// com o que vem entre aspas trocado por "…" e os números, por "N".
    fn known_message(m: &str) -> bool {
        const KNOWN: &[&str] = &[
            "“…” não é uma variável nem uma unidade",
            "“…” é definida mais abaixo (linha N): defina antes de usar",
            "“…” tem um erro (linha N)",
            "Falta um número depois de “…”",
            "Falta um número antes de “…”",
            "Falta uma operação entre “…” e “…”",
            "Falta uma operação antes de “…”",
            "“…” não é uma operação: use +, −, × ou ÷",
            "Parêntese aberto sem fechar",
            "Parêntese fechado sem abrir",
            "Divisão por zero",
            "Número grande demais",
            "Raiz de número negativo",
            "Esperava uma unidade depois de “…”",
            "“…” não é uma unidade",
            "“…” não é um número",
            "“…” é o nome de uma função: escolha outro nome",
            "“…” é uma palavra da conta: escolha outro nome",
            "π é uma constante",
            "Conta comprida demais",
            "Conta entre unidades diferentes: ainda não",
            "“…” só separa os números de uma função",
            "Use “…” para separar os números",
            "A conversão com “…” fica no fim da conta",
        ];
        let mut norm = String::new();
        let mut quoted = false;
        for c in m.chars() {
            match c {
                '“' => {
                    quoted = true;
                    norm.push_str("“…");
                }
                '”' => {
                    quoted = false;
                    norm.push('”');
                }
                _ if quoted => {}
                c if c.is_ascii_digit() => {
                    if !norm.ends_with('N') {
                        norm.push('N');
                    }
                }
                c => norm.push(c),
            }
        }
        // "{função} não existe para {valor}", "{função} precisa de …", "Não dá para converter …".
        KNOWN.contains(&norm.as_str()) || [" não existe para ", " precisa de ", "Não dá para converter "].iter().any(|p| m.contains(p))
    }

    #[test]
    fn so_mensagens_conhecidas() {
        let pieces = [
            "2", "10", "1.500", "1,5", "1/2", " ", " ", " ", "+", "−", "×", "÷", "^", "!", "%", "°", "²", "√", "(", ")", "(", ")", ";", "=", ",", ".", "?", ":", "…",
            "e", "x", "de", "em", "para", "mod", "π", "sen", "máx", "log", "a", "itens", "Oba", "R$", "🙂", "→",
        ];
        let mut seed: u64 = 0x2545_F491_4F6C_DD1D;
        let mut next = |n: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n as u64) as usize
        };
        let mut odd = Vec::new();
        for _ in 0..20_000 {
            let len = 1 + next(12);
            let text: String = (0..len).map(|_| pieces[next(pieces.len())]).collect();
            for line in evaluate(&format!("a = 3\n{text} ="), &Context::default()) {
                odd.extend(line.problems.into_iter().map(|p| p.message).filter(|m| !known_message(m)).map(|m| format!("{text:?}: {m}")));
            }
        }
        odd.sort();
        odd.dedup();
        assert!(odd.is_empty(), "{}", odd.iter().take(20).cloned().collect::<Vec<_>>().join("\n"));
    }

    /// Notas variadas para a bancada.
    fn sample(lines: usize) -> String {
        (0..lines)
            .map(|i| match i % 10 {
                0 => format!("v{i} = {i} × 3"),
                1 => format!("Hoje gastei {i} × 2 ="),
                2 => format!("v{} × 12 + 1.500 =", i - 2),
                3 => "Uma linha só de texto, sem conta nenhuma, como numa nota.".to_string(),
                4 => format!("sen {} + cos 30° =", i % 90),
                5 => format!("(1.500 + {i}) / 2 − 10% ="),
                6 => format!("Total de 2 itens × {i} ="),
                7 => format!("a{i} = 2; b{i} = 3; a{i} × b{i} ="),
                8 => format!("arred({i},567; 2) + máx(3; 7; 5) ="),
                _ => format!("Hotel: {i} mod 7 + 2^10 ="),
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Bancada: `cargo test --lib bancada_notas -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn bancada_notas() {
        for (lines, limit_ms) in [(500, 2.0), (5000, 20.0)] {
            let text = sample(lines);
            let ctx = Context::default();
            let best = (0..7)
                .map(|_| {
                    let t0 = std::time::Instant::now();
                    std::hint::black_box(evaluate(std::hint::black_box(&text), &ctx));
                    t0.elapsed().as_secs_f64() * 1e3
                })
                .fold(f64::INFINITY, f64::min);
            println!("{lines} linhas: {best:.3} ms (limite {limit_ms} ms)");
            assert!(best < limit_ms, "{lines} linhas em {best:.3} ms");
        }
    }
}
