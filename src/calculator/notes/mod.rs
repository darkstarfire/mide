//! Notas matemáticas da Calculadora (E3b, etapa 2): texto livre em que a linha que
//! termina em "=" mostra o resultado depois dele ("aluguel × 12 =" → 14.400), com
//! variáveis ("aluguel = 1.200") e prosa antes da conta ("Hoje gastei 2 × 3 =" → 6).
//! Sem interface: o app desenha o texto e chama `evaluate` a cada mudança.
//!
//! Unidades e moedas (etapa 11): "10 km em milhas =", "5 km + 300 m =" (a primeira
//! unidade vence), "1 h 30 min em min =", "R$ 1.200 + 300 =", "US$ 20 em reais =". As
//! cotações vêm prontas no `Context` (as do cache do app; sem rede aqui), e a linha que
//! troca moedas fica marcada (`needs_rates`) para o app buscar as do dia.
//!
//! Camadas: `lexer` corta a linha, `parser` lê uma conta, `value` e `eval` fazem as
//! contas com o motor do teclado, e `words` acha unidades e moedas pelo nome. Aqui
//! ficam as linhas, os enunciados (";"), os "=" (definição ou pedido de resultado) e as
//! mensagens de erro.

pub mod eval;
pub mod lexer;
pub mod parser;
pub mod store;
pub mod value;
pub mod words;

use std::cell::Cell;
use std::collections::HashMap;
use std::ops::Range;

use super::currency::Rates;
use super::format::Style;
use crate::apps::fold;
use eval::{Env, Eval};
use lexer::{Kind, Token};
use parser::{Expr, Fault, Func, Node, Role};

/// Linha mais comprida que é lida, em caracteres; uma maior fica só como texto.
const MAX_LINE: usize = 2000;

/// "e" é a palavra da nota (em português); a mensagem passa por `tr!` onde aparece.
const CONJ: &str = "“e” isn't an operation: use +, −, × or ÷";

/// Opções da Calculadora que valem nas notas, e as cotações.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Context<'a> {
    /// Ângulos sem "°" em radianos (senão, em graus).
    pub radians: bool,
    /// Casas decimais e ponto de milhar dos resultados.
    pub style: Style,
    /// Cotações para trocar moedas (as últimas guardadas); sem elas, só contas numa
    /// moeda só.
    pub rates: Option<&'a Rates>,
}

/// O resultado mostrado depois de um "=".
#[derive(Debug, Clone, PartialEq)]
pub struct Shown {
    /// Byte logo depois do "=", no texto inteiro.
    pub at: usize,
    /// Como aparece: "1.514", "20%", "30°", "5,3 km", "R$ 1.500,00".
    pub text: String,
    /// Só o número, sem ponto de milhar: "1514", "1500,00".
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
    /// A linha troca uma moeda por outra: precisa das cotações (o app busca as do dia
    /// e refaz a nota quando chegam).
    pub needs_rates: bool,
}

/// Alguma linha precisa das cotações.
pub fn needs_rates(lines: &[LineEval]) -> bool {
    lines.iter().any(|l| l.needs_rates)
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
        // "1 h 30 min": a unidade, o número e a unidade seguinte.
        if parser::mixes(pa.1, pb.0, self.pair(b + 1).map(|(_, r)| r)) {
            return true;
        }
        if !parser::linked(pa, pb) {
            return false;
        }
        // Lado a lado, sem operação entre eles (a multiplicação implícita).
        let side_by_side = parser::juxtaposes(pa, pb, false) && !parser::takes_left(pa, pb) && !parser::cola(pa.0, pa.1);
        !(side_by_side && ((pa.0.kind == Kind::Close && self.prose_group(a)) || self.closer(b).is_some_and(|c| self.prose_group(c))))
    }

    /// A conta achada começa em `s`, logo depois de prosa, e a prosa cortou uma conta
    /// maior: o que sobra é só a última parcela, que não é o total. Duas regras:
    ///
    /// A) Operador binário solto antes da prosa, com o operando dele antes ("R$ 200 +
    ///    farmácia R$ 50", "5 km + depois: 300 m"): o operador pede a parcela seguinte,
    ///    então a conta foi cortada, com ou sem pontuação no meio ("farmácia, R$ 50").
    ///
    /// B) A conta começa pelo sinal, com a prosa entre ele e um operando ("R$ 200 no
    ///    mercado + R$ 50", "5 km de manhã − 3 km"): sem a prosa, o sinal seria o
    ///    operador da conta, e lido como unário daria "R$ -50,00". A não ser que entre
    ///    o operando e o sinal haja pontuação que fecha frase ou item (`separates`):
    ///    ela separa a conta da prosa de antes ("Item 1: −5 + 3", "Paguei 25. Depois −
    ///    2 × 3", "Item 1 — −5 + 3"). O sinal depois de um rótulo sem operando antes
    ///    abre a conta ("Mercado − 2 × 30", "- Mercado − 2 × 30").
    ///
    /// Limitação aceita: a vírgula separa mesmo quando a frase continua, então "R$ 200,
    /// no mercado − R$ 50 =" mostra R$ -50,00. A "!" depois de número, ")", variável ou
    /// constante é o fatorial (o operando: "5! no total + 1" fica mudo); depois de
    /// palavra de prosa, unidade ou moeda é exclamação, pontuação que entra nas duas
    /// regras ("R$ 200 + farmácia! R$ 50" muda; "Paguei 25 reais! −2 × 3" dá −6).
    fn cut_by_prose(&self, s: usize) -> bool {
        // Volta pela prosa e pela pontuação (o ";" também, quando o trecho junta os
        // enunciados: ver `request`), lembrando se passou por pontuação que separa.
        let mut j = s;
        let mut separated = false;
        while j > 0
            && let Some((t, role)) = self.pair(j - 1)
            && (matches!(t.kind, Kind::Other(_) | Kind::Semi)
                || matches!(role, Role::Prose)
                || matches!(role, Role::Unknown(_) if !self.later.get(j - 1).copied().unwrap_or(false)))
        {
            separated |= separates(t);
            j -= 1;
        }
        if j == s || j == 0 {
            return false;
        }
        // Regra A: o operador binário com o operando dele antes.
        if j >= 2
            && let Some((op, role)) = self.pair(j - 1)
            && (matches!(op.kind, Kind::Plus | Kind::Minus | Kind::Times | Kind::Divide | Kind::Pow) || matches!(role, Role::Times | Role::Mod))
            && self.ends_operand_at(j - 2)
        {
            return true;
        }
        // Regra B: o sinal com a prosa (sem pontuação que separa) entre ele e o operando.
        let sign = self.toks.get(s).is_some_and(|t| matches!(t.kind, Kind::Plus | Kind::Minus));
        sign && !separated && self.ends_operand_at(j - 1)
    }

    /// Um operando termina no token `k`: número, quantidade, moeda, variável, pós-fixo
    /// num operando ("10%", "5!") ou parênteses com conta. A "!" depois de palavra é
    /// exclamação ("Oba! Mercado − 2"), e o ")" de prosa não fecha conta ("Oba :)").
    fn ends_operand_at(&self, k: usize) -> bool {
        let Some((t, role)) = self.pair(k) else {
            return false;
        };
        match t.kind {
            Kind::Close => self.math_group(k).is_some(),
            Kind::Bang if *role == Role::Prose => false,
            Kind::Bang | Kind::Percent | Kind::Degree | Kind::Sup(_) => k > 0 && self.ends_operand_at(k - 1),
            _ => parser::ends_operand(t, role),
        }
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
                Role::Unknown(_) | Role::Var(_) | Role::Const(_) | Role::Unit(_) | Role::Cur(_) => evidence = true,
                Role::Prose => words = true,
                Role::Func(_) | Role::Times | Role::Mod | Role::Convert | Role::Conj => (evidence, operation) = (true, true),
                Role::Plain => match t.kind {
                    Kind::Num(_) => evidence = true,
                    Kind::Other(_) => words = true,
                    // O símbolo depois de um "em" de prosa é o alvo de uma conversão
                    // que não pegou ("10 maçãs em R$"): palavra, como o "km" de "10 xyz
                    // em km", e a linha fica muda.
                    // O símbolo que não é de moeda nenhuma ("10 maçãs em XX$") segue
                    // como conta, para a linha mostrar o erro de moeda desconhecida.
                    Kind::Money(Some(_)) if self.dangling_target(i) => words = true,
                    // A moeda pede o valor: liga como um operador ("R$ 1.200").
                    Kind::Money(_) => (evidence, operation) = (true, true),
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

    /// O token `i` vem logo depois de "em" ou "para" lidos como prosa (sem operando
    /// antes deles, então sem conversão): "R$" em "10 maçãs em R$".
    fn dangling_target(&self, i: usize) -> bool {
        i.checked_sub(1).and_then(|p| self.pair(p)).is_some_and(|(t, r)| {
            *r == Role::Prose && matches!(&t.kind, Kind::Name(w) if w.eq_ignore_ascii_case("em") || w.eq_ignore_ascii_case("para"))
        })
    }

    /// Antes do sinal que abre a conta (o token `i`) há conta com erro, e não um
    /// rótulo ("Mercado − 2 × 30"): um nome desconhecido ligado à conta pela esquerda
    /// ("2 × itens + 1"), definido mais abaixo ("z + 1" acima de "z = 5") ou com
    /// expoente, porcentagem ou grau ("lado² + 1"; a "!" depois de palavra é
    /// exclamação, que separa como o "."), ou parênteses com conta ("(2 × itens)² +
    /// 1").
    fn broken_before_sign(&self, i: usize) -> bool {
        let mut j = i;
        let mut postfix = false;
        while j > 0
            && let Some((t, role)) = self.pair(j)
            && matches!(t.kind, Kind::Bang | Kind::Percent | Kind::Degree | Kind::Sup(_))
            && *role != Role::Prose
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
        let Some((t, role)) = j.checked_sub(1).filter(|&p| p >= start).and_then(|p| self.pair(p)) else {
            return false;
        };
        // A quantidade já tem a unidade ou a moeda ("R$ 50 gastei", "5 km depois"): o
        // nome depois dela é prosa, não uma unidade mal escrita como em "2 pães − 1".
        // Fica para `cut_by_prose` (regra B): "Ganhei R$ 50 gastei −R$ 20 + R$ 5 =" é
        // muda.
        let money = j >= 2 && matches!(t.kind, Kind::Num(_)) && self.toks.get(j - 2).is_some_and(|m| matches!(m.kind, Kind::Money(_)));
        if money || matches!(role, Role::Unit(_) | Role::Cur(_)) {
            return false;
        }
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
    ctx: &'a Context<'a>,
    out: LineEval,
}

impl<'a> Line<'a> {
    fn run(mut self, env: &mut Env) -> LineEval {
        let seps = separators(self.toks);
        // O resultado aparece só no "=" que termina a linha.
        let last = self.toks.len().checked_sub(1).filter(|&i| self.toks[i].kind == Kind::Equals && seps[i]);
        let mut prev: Option<usize> = None;
        for stmt in statements(self.toks, &seps) {
            // O enunciado anterior, para a conta cortada por prosa e ";" ("R$ 1.200 ×
            // condomínio; R$ 300 =").
            let before = prev.replace(stmt.start);
            let eqs: Vec<usize> = stmt.clone().filter(|&i| seps[i] && self.toks[i].kind == Kind::Equals).collect();
            let mut start = stmt.start;
            let problems = self.out.problems.len();
            for (k, &eq) in eqs.iter().enumerate() {
                let after = eq + 1..eqs.get(k + 1).copied().unwrap_or(stmt.end);
                // Com erro no enunciado (a definição de "custo = 25 × 4 ="), o "=" fica
                // sem resultado.
                if !self.define(env, start..eq, after) && Some(eq) == last && self.out.problems.len() == problems {
                    self.request(env, start..eq, eq, before.filter(|_| start == stmt.start));
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
                Node::Neg(x) | Node::Post(_, x) | Node::Root(_, x) | Node::ToPercent(x) | Node::Money(_, x) | Node::Convert(x, _) => stack.push(x),
                Node::Binary(_, a, b) => stack.extend([&**a, &**b]),
                Node::Call { args, .. } => stack.extend(args),
                Node::Num(_) | Node::Const(_) | Node::Unit(_) | Node::Cur(_) => {}
            }
        }
    }

    fn eval(&mut self, env: &Env, e: &Expr) -> Result<value::Value, Problem> {
        let eval = Eval { env, ctx: self.ctx, text: self.text, used_rates: Cell::new(false) };
        let v = eval.value(e);
        self.out.needs_rates |= eval.used_rates.get();
        v
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
            Def::Value(_) if let Some(problem) = self.unknown_currency(&value_seg) => Err(problem),
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
    /// 2 × 3? ="). Sem conta, só há erro se o fim do trecho é conta. `prev`: o começo
    /// do enunciado anterior, quando o trecho começa logo depois de um ";".
    fn request(&mut self, env: &Env, region: Range<usize>, eq: usize, prev: Option<usize>) {
        let mut end = region.end;
        while end > region.start && self.toks.get(end - 1).is_some_and(|t| matches!(t.kind, Kind::Other(_))) {
            end -= 1;
        }
        let seg = self.seg(region.start..end, env);
        if let Some(problem) = self.unknown_currency(&seg) {
            self.push(problem);
            return;
        }
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
            // A conta logo depois de "…operador prosa;": o ";" fechou o enunciado, mas a
            // prosa cortou a conta (regra A de `cut_by_prose`), lida com o enunciado
            // anterior junto. Só com palavra no meio: "2 +; 3 =" não é prosa.
            if let Some(p) = prev
                && seg.toks[..s].iter().all(|t| matches!(t.kind, Kind::Other(_)) || t.kind == Kind::Semi || matches!(t.kind, Kind::Name(_)))
            {
                let wide = self.seg(p..end, env);
                let at = s + region.start - p;
                let word = wide.toks[..at].iter().rev().take_while(|t| !matches!(t.kind, Kind::Plus | Kind::Minus | Kind::Times | Kind::Divide | Kind::Pow)).any(|t| matches!(t.kind, Kind::Name(_)));
                if word && wide.cut_by_prose(at) {
                    return;
                }
            }
            if s > 0 {
                if let Some(problem) = self.left_break(&seg, s) {
                    self.push(problem);
                    return;
                }
                if n - s == 1 && matches!(seg.toks[s].kind, Kind::Num(_)) {
                    // "Somar 2 e 3" não mostra 3; "Paguei 30 =" fica como texto.
                    return;
                }
                if seg.cut_by_prose(s) {
                    // "Mercado R$ 200 + farmácia R$ 50": a última parcela não é o total.
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
            return Some(Problem { range: prev.range.clone(), message: crate::i18n::tr(CONJ).into_owned() });
        }
        let sign = seg.toks.get(s).is_some_and(|t| matches!(t.kind, Kind::Plus | Kind::Minus));
        // A quantidade antes de um número que não continua a mistura ("Corri 5 km 2 × 3",
        // "Em 2 anos 3 × 4") é prosa, como a palavra desconhecida.
        let quantity = matches!(role, Role::Unit(_) | Role::Cur(_)) && seg.toks.get(s).is_some_and(|t| matches!(t.kind, Kind::Num(_)));
        if !sign && !quantity && parser::ends_operand(prev, role) {
            return self.missing_operation(seg, s);
        }
        if sign && seg.broken_before_sign(s - 1) {
            return self.chain_problem(seg, seg.chain_start(s - 1)..n);
        }
        None
    }

    /// O nome de moeda pela metade ("100 dólares canadense", "dólares de Hong"), que o
    /// léxico junta numa palavra só: sublinhado como moeda desconhecida, em vez de a
    /// conta seguir sem ele (ou de valer o dólar americano do "dólares").
    fn unknown_currency(&self, seg: &Seg) -> Option<Problem> {
        let (t, _) = (0..seg.toks.len()).filter_map(|i| seg.pair(i)).find(|(t, r)| matches!(&t.kind, Kind::Name(w) if w.contains(' ')) && matches!(r, Role::Unknown(_)))?;
        let word = self.slice(t.range.clone());
        Some(Problem { range: t.range.clone(), message: crate::tr!("“{word}” isn't a known currency", word = word) })
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
        let message = crate::tr!("“{text}” isn't a number", text = self.slice(range.clone()));
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
            Fault::NoNumber(i) => (whole, crate::tr!("Missing a number after “{word}”", word = word(i))),
            Fault::NoNumberBefore(i) => (whole, crate::tr!("Missing a number before “{word}”", word = word(i))),
            Fault::NoOperation(i) => match self.missing_operation(seg, i) {
                Some(problem) => return problem,
                None => (whole, crate::tr!("Missing an operation before “{word}”", word = word(i))),
            },
            Fault::Unclosed(_) => (whole, crate::tr!("Unclosed parenthesis").into_owned()),
            Fault::Unopened(_) => (whole, crate::tr!("Closing parenthesis without an opening one").into_owned()),
            Fault::Conj(i) => (range(i), crate::i18n::tr(CONJ).into_owned()),
            Fault::NotUnit(_, Some(i)) => (range(i), crate::tr!("“{word}” isn't a unit", word = word(i))),
            Fault::NotUnit(i, None) => (whole, crate::tr!("Expected a unit after “{word}”", word = word(i))),
            Fault::Arity(i, end, func) => (range(i).start..range(end).end, eval::arity_message(word(i), func)),
            Fault::Separator(_) => (whole, crate::tr!("Use “;” to separate the numbers").into_owned()),
            Fault::AfterConvert(_, convert) => (whole, crate::tr!("The conversion with “{word}” goes at the end of the calculation", word = word(convert))),
            Fault::TooDeep(_) => (whole, crate::tr!("Calculation too long").into_owned()),
            Fault::Stray(i) if word(i) == ";" => (whole, crate::tr!("“;” only separates the numbers of a function").into_owned()),
            // Reserva: a prosa fica fora das contas relidas aqui (`linked` corta nela, e
            // a pontuação antes do "=" sai antes da busca).
            Fault::Stray(i) => (whole, crate::tr!("“{word}” isn't part of the calculation", word = word(i))),
        };
        Problem { range, message }
    }

    /// "“x” é definida mais abaixo…" se o nome é definido depois (`at`: o token na
    /// linha); unidade ou moeda sem número antes ("km × 2"), "Falta um número antes de
    /// “km”"; senão, "“x” não é uma variável nem uma unidade".
    fn unknown(&self, word: &str, at: usize) -> String {
        match self.later(&fold(word), at) {
            Some(line) => crate::tr!("“{word}” is defined further down (line {line}): define it before using it", word = word, line = line + 1),
            // Símbolo ("XX$") ou nome composto ("dólares canadense") que não é de moeda.
            None if word.ends_with(['$', '¥']) || word.contains(' ') => crate::tr!("“{word}” isn't a known currency", word = word),
            None if words::unit(word, false).is_some() || words::currency_word(word).is_some() => crate::tr!("Missing a number before “{word}”", word = word),
            None => crate::tr!("“{name}” isn't a variable or a unit", name = word),
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
        let message = crate::tr!("Missing an operation between “{left}” and “{right}”", left = self.snippet(left), right = self.snippet(right));
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

/// Pontuação que fecha frase ou item e separa a conta da prosa de antes: ":", ",",
/// ".", ";", "?", "!" (a exclamação), "…" e os travessões "—" e "–" (o hífen "-" é o
/// sinal de menos).
fn separates(t: &Token) -> bool {
    // A "!" só chega aqui como exclamação (prosa): o fatorial não volta pela prosa.
    matches!(t.kind, Kind::Semi | Kind::Bang | Kind::Other(':' | ',' | '.' | ';' | '?' | '…' | '—' | '–'))
}

/// O token começa uma conta de verdade (um nome desconhecido não: é prosa).
fn starts_strict(t: &Token, role: &Role) -> bool {
    matches!(t.kind, Kind::Num(_) | Kind::Open | Kind::Root(_) | Kind::Plus | Kind::Minus | Kind::Money(_))
        || matches!(role, Role::Var(_) | Role::Func(_) | Role::Const(_))
}

/// O erro de um nome que não pode ser variável: palavra da conta, função ou π.
fn bad_name(name: &str) -> Option<String> {
    if parser::KEYWORDS.iter().any(|k| name.eq_ignore_ascii_case(k)) {
        Some(crate::tr!("“{name}” is a calculation word: choose another name", name = name))
    } else if Func::find(&fold(name)).is_some() {
        Some(crate::tr!("“{name}” is the name of a function: choose another name", name = name))
    } else if name == "π" || name.eq_ignore_ascii_case("pi") {
        Some(crate::tr!("π is a constant").into_owned())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Por linha: o resultado, "erro: …" (o primeiro problema) ou "".
    fn lines_with(text: &str, ctx: Context) -> Vec<String> {
        crate::i18n::set_language("pt_BR");
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
        crate::i18n::set_language("pt_BR");
        let lines = evaluate(text, &Context::default());
        let p = lines.last().and_then(|l| l.problems.first()).cloned().expect("sem problema");
        (text[p.range.clone()].to_string(), p.message)
    }

    fn check(cases: &[(&str, &str)]) {
        let bad: Vec<String> = cases.iter().filter(|(t, w)| last(t) != *w).map(|(t, w)| format!("{t} -> {:?} (queria {w:?})", last(t))).collect();
        assert!(bad.is_empty(), "{}", bad.join("\n"));
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
                for ctx in [Context::default(), Context { radians: true, style: Style { decimals: Some(3), grouping: false }, ..Context::default() }] {
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

    const MIXED: &str = "erro: Conta entre unidades diferentes: ainda não";

    #[test]
    fn unidades() {
        check(&[
            ("10 km em milhas =", "6,21371192237 mi"),
            ("5 km + 300 m =", "5,3 km"),
            ("1 h 30 min em min =", "90 min"),
            ("1h30min em min =", "90 min"),
            ("1/2 kg em g =", "500 g"),
            ("1/2 h em min =", "30 min"),
            ("25 °C em °F =", "77 °F"),
            ("20 °C + 5 °C =", "25 °C"),
            ("20 °C + 5 °F =", "erro: Não dá para somar °C com °F"),
            ("2 kg + 3 m =", "erro: Não dá para somar massa com comprimento"),
            ("10 km em kg =", "erro: Não dá para converter km em kg"),
            ("100 km / 2 h =", MIXED),
            ("10 km / 2 km =", "5"),
            ("1 GB em MB =", "1.000 MB"),
            ("30° em rad =", "0,5235987756 rad"),
            ("1 rad em ° =", "57,2957795131°"),
            ("10 km em m =", "10.000 m"),
        ]);
        assert_eq!(r("d = 5 km\nd em m ="), ["", "5.000 m"]);
        assert_eq!(last("h = 10\nh × 2 ="), "20");
        // Variável vence unidade, também depois do número.
        assert_eq!(last("h = 10\n2 h ="), "20");
    }

    #[test]
    fn unidades_contas_e_misturas() {
        check(&[
            // × e ÷ por número; porcentagem sobre a quantidade.
            ("5 km × 2 =", "10 km"),
            ("5 km ÷ 2 =", "2,5 km"),
            ("2 ÷ 5 km =", MIXED),
            ("3 m × 2 m =", MIXED),
            ("200 m − 10% =", "180 m"),
            ("10% de 5 km =", "0,5 km"),
            ("5 km + 2 =", "7 km"),
            ("2 + 5 km =", "7 km"),
            // Misturas: a primeira unidade vence.
            ("1 h 30 min =", "1,5 h"),
            ("2 × 1 h 30 min =", "3 h"),
            ("1 h 30 min 30 s em s =", "5.430 s"),
            ("6 ft 4 pol em cm =", "193,04 cm"),
            ("5 km 3 kg =", "erro: Não dá para somar comprimento com massa"),
            // Unidades de vários pedaços.
            ("3 m² + 1 m² =", "4 m²"),
            ("60 km/h em m/s =", "16,6666666667 m/s"),
            ("8 L/100 km em km/L =", "12,5 km/L"),
            ("2 metros quadrados em cm² =", "20.000 cm²"),
            // "min" é minuto depois do número, e a função com "(".
            ("2 min(3; 4) =", "6"),
            ("min(3; 4) =", "3"),
            ("máx(1 km; 300 m) =", "1 km"),
            ("mín(1 km; 300 m) =", "300 m"),
            ("máx(1 km; 2 kg) =", MIXED),
            // Ângulos de outras unidades na trigonometria.
            ("sen 0,25 volta =", "1"),
            ("sen 30° =", "0,5"),
            ("sen 2 km =", "erro: sen não existe para 2 km"),
            // Sem o número, a unidade não faz conta.
            ("km × 2 =", "erro: Falta um número antes de “km”"),
            ("10 em km =", "erro: Não dá para converter número em km"),
            ("5 km em % =", "erro: Não dá para converter km em %"),
            ("5 km² =", "5 km²"),
            ("(5 km)² =", MIXED),
            ("√(9 m²) =", MIXED),
            // Unidade e moeda depois de ")", π, variável, raiz e pós-fixo, não só de número.
            ("(2 + 3) kg em g =", "5.000 g"),
            ("(5) km =", "5 km"),
            ("2³ m =", "8 m"),
            ("2π m =", "6,28318530718 m"),
            ("π rad =", "3,14159265359 rad"),
            ("2π rad em ° =", "360°"),
            ("(10 + 5) dólares =", "US$ 15,00"),
            ("√(9) km =", "3 km"),
            ("2^−1 km =", "0,5 km"),
            // O sinal vale para a mistura inteira.
            ("−1 h 30 min em min =", "-90 min"),
            ("-1 kg 500 g em g =", "-1.500 g"),
            ("−(1 h 30 min) em min =", "-90 min"),
            ("−2 × 3 =", "-6"),
            ("−7 mod 3 =", "-1"),
        ]);
        assert_eq!(last("x = 3\nx kg em g ="), "3.000 g");
    }

    #[test]
    fn temperaturas() {
        check(&[
            // A diferença converte só pelo tamanho do grau.
            ("(30 °C − 20 °C) em °F =", "18 °F"),
            ("30 °C − 20 °C em °F =", "18 °F"),
            ("30 °C − 20 °C =", "10 °C"),
            ("(30 °C − 20 °C) em K =", "10 K"),
            ("20 °C + (30 °C − 20 °C) =", "30 °C"),
            ("20 °C + 5 °C =", "25 °C"),
            ("25 °C em °F =", "77 °F"),
            // mín e máx comparam pela conversão (70 °F são 21,1 °C).
            ("máx(20 °C; 70 °F) =", "70 °F"),
            ("mín(20 °C; 70 °F) =", "20 °C"),
            ("máx(1 km; 2 kg) =", MIXED),
        ]);
    }

    #[test]
    fn unidades_na_prosa() {
        check(&[
            ("Andei 5 km + 300 m =", "5,3 km"),
            ("Corrida: 1h30min em min =", "90 min"),
            // "a" é a preposição, não o are.
            ("De 2 a 5 × 3 =", "15"),
            ("Faltam 2 a 3 × 2 =", "6"),
            // Unidade de palavra comum só depois de número.
            ("Hoje h × 2 =", "erro: “h” não é uma variável nem uma unidade"),
            ("Ano 2 × 3 =", "6"),
            ("Café sem açúcar 2 × 3 =", "6"),
            ("5 km =", "5 km"),
            // A quantidade antes de um número que não continua a mistura é prosa.
            ("Corri 5 km 2 × 3 =", "6"),
            ("Em 2 anos 3 × 4 =", "12"),
            ("20 dólares 3 × 2 =", "6"),
            ("1 h 30 min =", "1,5 h"),
            // Prosa no meio de uma soma: a última parcela não é o total.
            ("Mercado 200 + farmácia 50 =", ""),
            ("Mercado R$ 200 + farmácia R$ 50 =", ""),
            ("Aluguel R$ 1.200 + condomínio R$ 300 =", ""),
            ("Andei 5 km + depois 300 m =", ""),
            ("Mercado 2 × 3 + farmácia 5 × 1 =", ""),
            // Com pontuação no meio, também: o operador solto antes da prosa pede a
            // parcela seguinte, e a pontuação não fecha a conta.
            ("Mercado R$ 200 + farmácia: R$ 50 =", ""),
            ("Mercado R$ 200 + farmácia, R$ 50 =", ""),
            ("Aluguel R$ 1.200 + condomínio, R$ 300 =", ""),
            ("Aluguel R$ 1.200 × condomínio; R$ 300 =", ""),
            ("Andei 5 km + depois: 300 m =", ""),
            ("Mercado 2 × 3 + farmácia, 5 × 1 =", ""),
            // Prosa entre a primeira parcela e o sinal: o sinal não abre a conta.
            ("R$ 200 no mercado + R$ 50 =", ""),
            ("R$ 200 no mercado − R$ 50 =", ""),
            ("5 km de manhã + 3 km =", ""),
            ("200 no mercado + 50 =", ""),
            ("200 reais no mercado − 50 =", ""),
            ("(2 + 3) no mercado − 1 =", ""),
            ("5! no total + 1 =", ""),
            ("10 maçãs em R$ =", ""),
            ("10 abc em US$ =", ""),
            ("10 xyz em km =", ""),
            ("10 maçãs em XX$ =", "erro: “XX$” não é uma moeda conhecida"),
            // A pontuação que fecha frase ou item separa a conta da prosa de antes: o
            // sinal depois dela abre a conta.
            ("Item 1: −5 + 3 =", "-2"),
            ("2026: −10 + 5 =", "-5"),
            ("Dia 5, −3 × 2 =", "-6"),
            ("Paguei 25. −2 × 3 =", "-6"),
            ("Paguei 25. Depois − 2 × 3 =", "-6"),
            ("Em 05/10: −200 + 50 =", "-150"),
            ("Saldo de 100 reais. Hoje −20 + 5 =", "-15"),
            ("Às 10 h: −5 + 2 =", "-3"),
            ("Ganhei R$ 50, gastei −R$ 20 + R$ 5 =", "R$ -15,00"),
            ("Somei 3 itens: +5 + 2 =", "7"),
            ("1. Mercado − 2 × 30 =", "-60"),
            ("Item 1? −5 + 3 =", "-2"),
            ("Item 1 — −5 + 3 =", "-2"),
            ("Item 1 … −5 + 3 =", "-2"),
            ("Paguei 25? Depois − 2 × 3 =", "-6"),
            ("Paguei 25 – Depois − 2 × 3 =", "-6"),
            ("Mercado − 2 × 30 =", "-60"),
            ("Hoje gastei 2 × 3 =", "6"),
            // Sem a vírgula, "gastei" é prosa entre a parcela e o sinal: muda.
            ("Ganhei R$ 50 gastei −R$ 20 + R$ 5 =", ""),
            // Limitação aceita: a vírgula separa a conta da prosa de antes mesmo quando
            // a frase continua ("R$ 200, no mercado"), e o sinal abre a conta.
            ("R$ 200, no mercado − R$ 50 =", "R$ -50,00"),
            // Depois de rótulo ou de ")" de prosa, o sinal abre a conta.
            ("Oba :) Mercado − 2 =", "-2"),
            ("(nota) Mercado − 2 =", "-2"),
            ("Paguei 25. × 3 =", "erro: Falta um número antes de “×”"),
            ("pão e 3 × 2 =", "6"),
            ("- Mercado − 2 × 30 =", "-60"),
            ("Mercado: R$ 200 =", "R$ 200,00"),
            // Código em minúscula que é palavra comum não é moeda.
            ("Tenho 2 top × 3 =", "erro: “top” não é uma variável nem uma unidade"),
            ("Paguei 3 cup × 2 =", "erro: “cup” não é uma variável nem uma unidade"),
            ("Usei 2 gel × 3 =", "erro: “gel” não é uma variável nem uma unidade"),
            ("2 TOP =", "TOP 2,00"),
        ]);
        // "5 km = 3,1 mi" não define km nem mostra nada.
        let lines = evaluate("5 km = 3,1 mi\nkm =", &Context::default());
        assert!(lines.iter().all(|l| l.result.is_none() && l.problems.is_empty() && l.names.is_empty()));
        let shown = evaluate("5 km + 300 m =", &Context::default()).remove(0).result.expect("resultado");
        assert_eq!((shown.text.as_str(), shown.number.as_str(), shown.value), ("5,3 km", "5,3", 5.3));
    }

    /// A "!" é fatorial só logo depois de um operando; depois de palavra de prosa, é
    /// pontuação que fecha a frase, como "." e "?".
    #[test]
    fn exclamacao_depois_de_prosa() {
        check(&[
            // Regra A: o operador solto antes da prosa pede a parcela seguinte.
            ("Mercado R$ 200 + farmácia! R$ 50 =", ""),
            ("Mercado R$ 200 + farmácia!R$ 50 =", ""),
            ("Mercado R$ 200 + farmácia ! R$ 50 =", ""),
            ("Mercado 200 + farmácia! 50 m =", ""),
            ("Andei 5 km + depois! 300 m =", ""),
            // Regra B: a exclamação separa a conta da prosa de antes.
            ("Paguei 25 reais! −2 × 3 =", "-6"),
            ("Paguei 25 reais! Depois − 2 × 3 =", "-6"),
            ("Paguei 25 reais! depois − 2 × 3 =", "-6"),
            ("Ok! Mercado − 2 × 30 =", "-60"),
            ("Que bom! Depois − 2 × 3 =", "-6"),
            ("Oba! Mercado − 2 =", "-2"),
            // Depois de operando, continua o fatorial.
            ("5! no total + 1 =", ""),
            ("5! =", "120"),
            ("(2+1)! =", "6"),
            ("Item 1! −5 + 3 =", "-1"),
        ]);
        assert_eq!(last("n = 4\nn! ="), "24");
    }

    /// Trecho do arquivo real do Banco Central do Brasil, fechamento PTAX de 21/10/2026.
    const BCB_20261021: &str = "\
21/10/2026;220;A;USD;5,02700000;5,02770000;1,00000000;1,00000000
21/10/2026;470;A;JPY;0,03179000;0,03179000;158,13000000;158,15000000
21/10/2026;978;B;EUR;5,63980000;5,64110000;1,12190000;1,12200000
";

    /// Mais moedas, com cotações redondas de teste (não são do BCB), para os símbolos
    /// com letras ("CA$", "CN¥") e os nomes compostos.
    const OUTRAS: &str = "\
21/10/2026;165;A;CAD;3,60000000;3,60000000;1,40000000;1,40000000
21/10/2026;796;A;CNY;0,70000000;0,70000000;7,20000000;7,20000000
21/10/2026;706;A;ARS;0,00500000;0,00500000;1000,00000000;1000,00000000
";

    fn rates() -> Rates {
        super::super::currency::parse_bcb(&format!("{BCB_20261021}{OUTRAS}")).expect("arquivo do BCB")
    }

    /// Com as cotações de 21/10/2026, a última linha.
    fn quoted(text: &str) -> String {
        let rates = rates();
        lines_with(text, Context { rates: Some(&rates), ..Context::default() }).pop().unwrap_or_default()
    }

    #[test]
    fn moedas() {
        assert_eq!(quoted("R$ 100 + US$ 20 ="), "R$ 200,55");
        assert_eq!(quoted("US$ 20 em reais ="), "R$ 100,55");
        assert_eq!(quoted("100 usd em brl ="), "R$ 502,77");
        assert_eq!(quoted("100 dólares em reais ="), "R$ 502,77");
        assert_eq!(quoted("50 EUR em US$ ="), "US$ 56,10");
        assert_eq!(quoted("€ 10 → $ ="), "US$ 11,22");
        assert_eq!(quoted("¥ 1.000 em R$ ="), "R$ 31,79");
        // O símbolo vem antes do valor; depois dele, pede outro número.
        assert_eq!(quoted("1.000 ¥ ="), "erro: Falta um número depois de “¥”");
        assert_eq!(quoted("US$ 10 / R$ 5 ="), "10,0554");
        assert_eq!(quoted("R$ 10 em CHF ="), "erro: Sem cotação para CHF (última: 21/10/2026)");
        // Uma moeda só: sem cotação.
        for text in ["R$ 10 + R$ 5 =", "R$ 10 + 5 ="] {
            assert_eq!(last(text), "R$ 15,00", "{text}");
            assert_eq!(quoted(text), "R$ 15,00", "{text}");
        }
        check(&[
            ("R$ 1.200 + 300 =", "R$ 1.500,00"),
            ("R$ 1.200 =", "R$ 1.200,00"),
            ("R$1.200,50 × 2 =", "R$ 2.401,00"),
            ("20 dólares =", "US$ 20,00"),
            ("50 EUR =", "€ 50,00"),
            ("¥ 1.234,5 =", "¥ 1.235"),
            ("R$ 10 em US$ =", "erro: Sem cotação para USD (nenhuma cotação guardada)"),
            ("R$ 100 + US$ 20 =", "erro: Sem cotação para USD (nenhuma cotação guardada)"),
            ("R$ 100 × US$ 2 =", MIXED),
            ("R$ 10 + 2 kg =", "erro: Não dá para somar dinheiro com massa"),
            ("10 km em reais =", "erro: Não dá para converter km em R$"),
            ("R$ =", "erro: Falta um número depois de “R$”"),
            // O "em" sem operando antes é prosa, e o símbolo depois dele, o alvo de uma
            // conversão que não pegou: mudo, como "10 xyz em km =".
            ("10 maçãs em R$ =", ""),
            ("10 abc em US$ =", ""),
            ("abc para R$ =", ""),
            ("10 xyz em km =", ""),
            ("Paguei em R$ 50 + R$ 20 =", "R$ 70,00"),
            ("2 × reais =", "erro: Falta um número antes de “reais”"),
        ]);
        assert_eq!(last("dólar = R$ 5,40\n100 dólar ="), "R$ 540,00");
        assert_eq!(last("aluguel = R$ 1.200\naluguel × 12 ="), "R$ 14.400,00");
        let rates = rates();
        let ctx = Context { rates: Some(&rates), ..Context::default() };
        assert_eq!(lines_with("d = US$ 20\nd em reais =", ctx), ["", "R$ 100,55"]);
    }

    #[test]
    fn simbolos_com_letras_e_nomes_compostos() {
        // As letras coladas no "$" ou no "¥" são da moeda, sem diferenciar maiúsculas.
        // Pela paridade com o dólar (5,0277 ÷ 1,4 e 5,0277 ÷ 7,2): não é o dólar
        // americano (R$ 502,77) nem o iene (R$ 3,18).
        assert_eq!(quoted("CA$ 100 em reais ="), "R$ 359,12");
        assert_eq!(quoted("CN¥ 100 em reais ="), "R$ 69,83");
        assert_eq!(quoted("r$ 10 em reais ="), "R$ 10,00");
        check(&[
            ("CA$ 100 =", "CA$ 100,00"),
            ("AU$ 100 =", "AU$ 100,00"),
            ("MX$ 100 =", "MX$ 100,00"),
            ("HK$ 100 =", "HK$ 100,00"),
            ("NT$ 100 =", "NT$ 100,00"),
            ("S$ 100 =", "S$ 100,00"),
            ("EC$ 100 =", "EC$ 100,00"),
            ("r$ 10 =", "R$ 10,00"),
            ("us$ 5 =", "US$ 5,00"),
            // Letras que não são de moeda nenhuma: erro, nunca o dólar americano.
            ("XX$ 100 =", "erro: “XX$” não é uma moeda conhecida"),
            ("Ganhei XX$ 100 + 5 =", "erro: “XX$” não é uma moeda conhecida"),
            ("10 em XX$ =", "erro: “XX$” não é uma moeda conhecida"),
        ]);
        // Nomes de várias palavras.
        assert_eq!(quoted("100 dólares canadenses em reais ="), "R$ 359,12");
        assert_eq!(quoted("10 reais brasileiros em US$ ="), "US$ 1,99");
        assert_eq!(quoted("1.000 pesos argentinos em reais ="), "R$ 5,03");
        check(&[
            ("100 dólares canadenses + 5 =", "CA$ 105,00"),
            ("10 pesos argentinos =", "ARS 10,00"),
            ("100 dólares de Hong Kong =", "HK$ 100,00"),
            ("100 dólares americanos =", "US$ 100,00"),
            ("10 em dólares canadenses =", "erro: Não dá para converter número em CA$"),
            // O "de" sozinho não continua o nome.
            ("100 dólares de lucro × 2 =", "erro: “lucro” não é uma variável nem uma unidade"),
            // Pela metade: moeda desconhecida, e não o dólar americano do "dólares".
            ("100 dólares canadense em reais =", "erro: “dólares canadense” não é uma moeda conhecida"),
            ("100 dólares de Hong =", "erro: “dólares de Hong” não é uma moeda conhecida"),
            ("100 dólares canadense + 5 =", "erro: “dólares canadense” não é uma moeda conhecida"),
        ]);
        assert_eq!(r("a = 100 dólares canadense\na =")[0], "erro: “dólares canadense” não é uma moeda conhecida");
        assert_eq!(underlined("Paguei 100 dólares canadense em reais =").0, "dólares canadense");
    }

    #[test]
    fn moedas_marcam_as_cotacoes() {
        let needs = |text: &str| evaluate(text, &Context::default()).iter().map(|l| l.needs_rates).collect::<Vec<_>>();
        assert_eq!(needs("R$ 100 + US$ 20 =\nR$ 10 + R$ 5 =\n10 km em m ="), [true, false, false]);
        assert_eq!(needs("d = US$ 20 em reais\nd ="), [true, false]);
        assert!(needs_rates(&evaluate("x = 1\nUS$ 1 em reais =", &Context::default())));
        assert!(!needs_rates(&evaluate("R$ 1 + 2 =", &Context::default())));
        let shown = evaluate("R$ 1.200 + 300 =", &Context::default()).remove(0).result.expect("resultado");
        assert_eq!((shown.text.as_str(), shown.number.as_str(), shown.value), ("R$ 1.500,00", "1500,00", 1500.0));
        let text = "R$ 1.200 + 300 =\n5 km + 300 m =";
        assert_eq!(with_results(text, &evaluate(text, &Context::default())), "R$ 1.200 + 300 = R$ 1.500,00\n5 km + 300 m = 5,3 km");
        // Casas do Visualizar também no dinheiro.
        let two = Context { style: Style { decimals: Some(3), grouping: false }, ..Context::default() };
        assert_eq!(lines_with("R$ 1.200 + 300 =", two), ["R$ 1500,000"]);
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
            "“…” não é uma moeda conhecida",
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
        // "{função} não existe para {valor}", "{função} precisa de …", "Não dá para converter …",
        // "Não dá para somar {grandeza} com {grandeza}", "Sem cotação para {moeda} (última: …)".
        let patterns = [" não existe para ", " precisa de ", "Não dá para converter ", "Não dá para somar ", "Sem cotação para "];
        KNOWN.contains(&norm.as_str()) || patterns.iter().any(|p| m.contains(p))
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

    #[test]
    fn so_mensagens_conhecidas_com_unidades_e_moedas() {
        let pieces = [
            "2", "10", "1/2", " ", " ", " ", "+", "−", "×", "÷", "^", "%", "°", "²", "(", ")", ";", "=", "em", "para", "→", "de", "x", "e", "min", "h", "km",
            "m", "km/h", "°C", "°F", "kg", "GB", "reais", "dólares", "usd", "R$", "US$", "€", "a", "sen", "máx", "itens",
        ];
        let rates = rates();
        let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = |n: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n as u64) as usize
        };
        let mut odd = Vec::new();
        for i in 0..20_000 {
            let len = 1 + next(10);
            let text: String = (0..len).map(|_| pieces[next(pieces.len())]).collect();
            let ctx = Context { rates: (i % 2 == 0).then_some(&rates), ..Context::default() };
            let note = format!("a = 3 km\n{text} =");
            let lines = evaluate(&note, &ctx);
            for line in &lines {
                for r in line.names.iter().chain(line.problems.iter().map(|p| &p.range)) {
                    assert!(note.get(r.clone()).is_some(), "{r:?} em {note:?}");
                }
            }
            for line in lines {
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
            .map(|i| match i % 16 {
                0 => format!("v{i} = {i} × 3"),
                1 => format!("Hoje gastei {i} × 2 ="),
                2 => format!("v{} × 12 + 1.500 =", i - 2),
                3 => "Uma linha só de texto, sem conta nenhuma, como numa nota.".to_string(),
                4 => format!("sen {} + cos 30° =", i % 90),
                5 => format!("(1.500 + {i}) / 2 − 10% ="),
                6 => format!("Total de 2 itens × {i} ="),
                7 => format!("a{i} = 2; b{i} = 3; a{i} × b{i} ="),
                8 => format!("arred({i},567; 2) + máx(3; 7; 5) ="),
                9 => format!("Hotel: {i} mod 7 + 2^10 ="),
                // Unidades, moedas e prosa depois de números (cada palavra passa pela
                // busca das unidades de vários pedaços e das moedas de nome composto).
                10 => format!("Hoje andei {i} km + 300 m em milhas ="),
                11 => format!("1 h 30 min + {i} min em s ="),
                12 => format!("R$ 1.200 + {i} dólares canadenses − 10% ="),
                13 => format!("Gastei {i} reais com 2 dias de hotel e 3 horas de táxi, {i} × 2 ="),
                14 => format!("60 km/h em m/s + {i} m/s ="),
                _ => format!("Comprei 2 caixas de leite, 5 pacotes de arroz e {i} °C em °F ="),
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
