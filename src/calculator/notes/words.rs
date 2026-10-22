//! Unidades e moedas pelo nome, para as notas: os mesmos critérios de `units::lookup`
//! e `currency::lookup`, com índices montados uma vez. As notas são refeitas a cada
//! tecla, e a busca pela tabela inteira desfaria o acento de uns 900 nomes por palavra.

use std::collections::HashMap;
use std::sync::LazyLock;

use super::super::currency::{self, Currency};
use super::super::units;
use super::value::UnitId;
use crate::apps::fold;

struct Units {
    /// Símbolo exato ("m", "M", "km/h") → unidades, na ordem da tabela.
    symbols: HashMap<&'static str, Vec<UnitId>>,
    /// Nome e apelidos sem acento nem maiúscula → unidades, na ordem da tabela.
    names: HashMap<String, Vec<UnitId>>,
}

static UNITS: LazyLock<Units> = LazyLock::new(|| {
    let mut symbols: HashMap<&'static str, Vec<UnitId>> = HashMap::new();
    let mut names: HashMap<String, Vec<UnitId>> = HashMap::new();
    for (category, cat) in units::categories().iter().enumerate() {
        for (unit, u) in cat.units.iter().enumerate() {
            let id = UnitId { category, unit };
            symbols.entry(u.symbol).or_default().push(id);
            for name in std::iter::once(u.name).chain(u.aliases.iter().copied()) {
                let list = names.entry(fold(name)).or_default();
                if !list.contains(&id) {
                    list.push(id);
                }
            }
        }
    }
    Units { symbols, names }
});

/// A unidade escrita em `word`, como o `units::lookup`: primeiro o símbolo exato, depois
/// nome e apelidos sem acento nem maiúscula; as que também são palavra comum (h, min,
/// sem, ano...) só logo depois de um número (`after_number`). O "a" (are) nunca: nas
/// notas ele é a preposição ("de 2 a 5").
pub fn unit(word: &str, after_number: bool) -> Option<UnitId> {
    let word = word.trim();
    if word.is_empty() || word == "a" {
        return None;
    }
    let ok = |id: &&UnitId| after_number || !id.get().needs_number();
    if let Some(&id) = UNITS.symbols.get(word).and_then(|list| list.iter().find(ok)) {
        return Some(id);
    }
    UNITS.names.get(&fold(word)).and_then(|list| list.iter().find(ok)).copied()
}

struct Currencies {
    /// Símbolo ou apelido exato ("R$", "$", "₲").
    exact: HashMap<&'static str, &'static Currency>,
    /// Código ISO em maiúsculas.
    codes: HashMap<&'static str, &'static Currency>,
    /// Código, símbolo, nome, plural e apelidos sem acento nem maiúscula.
    folded: HashMap<String, &'static Currency>,
    /// O mesmo sem os códigos: nas notas, o código em minúscula pode ser palavra comum
    /// ("top", "cup", "gel").
    named: HashMap<String, &'static Currency>,
    /// Símbolos com letras antes do "$" ou do "¥", em maiúsculas: "R$", "CA$", "CN¥".
    prefixed: HashMap<String, &'static Currency>,
    /// Nomes de várias palavras pela primeira palavra (sem acento nem maiúscula): as
    /// palavras de cada um, também sem acento nem maiúscula.
    phrases: HashMap<String, Vec<Vec<String>>>,
}

static CURRENCIES: LazyLock<Currencies> = LazyLock::new(|| {
    let mut exact = HashMap::new();
    let mut codes = HashMap::new();
    let mut folded = HashMap::new();
    let mut named = HashMap::new();
    let mut prefixed = HashMap::new();
    let mut phrases: HashMap<String, Vec<Vec<String>>> = HashMap::new();
    for c in currency::currencies() {
        for key in std::iter::once(c.symbol).chain(c.aliases.iter().copied()).filter(|k| !k.is_empty()) {
            exact.entry(key).or_insert(c);
            let letters = key.trim_end_matches(['$', '¥']);
            if letters.len() < key.len() && !letters.is_empty() && letters.chars().all(|ch| ch.is_ascii_alphabetic()) {
                prefixed.entry(key.to_ascii_uppercase()).or_insert(c);
            }
        }
        codes.entry(c.code).or_insert(c);
        let keys = [c.code, c.symbol, c.name, c.plural].into_iter().chain(c.aliases.iter().copied());
        for key in keys.filter(|k| !k.is_empty()) {
            folded.entry(fold(key)).or_insert(c);
            if key != c.code {
                named.entry(fold(key)).or_insert(c);
            }
            let words: Vec<String> = fold(key).split_whitespace().map(str::to_string).collect();
            if words.len() > 1 {
                let list = phrases.entry(words[0].clone()).or_default();
                if !list.contains(&words) {
                    list.push(words);
                }
            }
        }
    }
    Currencies { exact, codes, folded, named, prefixed, phrases }
});

/// Códigos em minúscula que as notas aceitam ("100 usd em brl"): os das moedas mais
/// usadas, que não são palavras do português. Os outros só em maiúsculas ("2 TOP";
/// "2 top" é prosa).
const LOWERCASE_CODES: &[&str] = &[
    "usd", "eur", "brl", "gbp", "jpy", "chf", "cad", "aud", "nzd", "cny", "hkd", "sgd", "ars", "clp", "mxn", "cop", "pyg", "uyu", "krw", "inr",
];

/// Palavras de ligação nos nomes compostos: sozinhas, não mostram que o nome continua
/// ("100 dólares de lucro" é dólar americano).
const CONNECTORS: &[&str] = &["de", "do", "dos", "da", "das", "e"];

/// O símbolo de moeda de letras coladas num "$" ou "¥" ("R", "r", "US", "CA", "CN" e o
/// sinal), sem diferenciar maiúsculas; `None` se não é de moeda nenhuma.
pub fn prefixed_symbol(letters: &str, sign: char) -> Option<&'static Currency> {
    let mut key = letters.to_ascii_uppercase();
    key.push(sign);
    CURRENCIES.prefixed.get(&key).copied()
}

/// A moeda de uma palavra das notas: como `currency`, mas o código só em maiúsculas
/// (fora os de `LOWERCASE_CODES`), porque em minúscula muitos são palavras comuns.
pub fn currency_word(word: &str) -> Option<&'static Currency> {
    let word = word.trim();
    if word.is_empty() {
        return None;
    }
    let index = &*CURRENCIES;
    if let Some(&c) = index.exact.get(word).or_else(|| index.codes.get(word)) {
        return Some(c);
    }
    let key = fold(word);
    if LOWERCASE_CODES.contains(&key.as_str()) {
        return index.codes.get(key.to_ascii_uppercase().as_str()).copied();
    }
    index.named.get(&key).copied()
}

/// Quantas palavras depois de `first` (`next(k)`: a k-ésima) continuam um nome de
/// moeda de várias palavras ("dólares" + "canadenses"). Vale o nome inteiro, ou o nome
/// pela metade que já mostra ser outra moeda: com uma palavra do nome que não é de
/// ligação ("dólares de Hong"), ou só com o plural trocado ("dólares canadense"); esse
/// fica como moeda desconhecida. `None` se nenhum nome continua.
pub fn currency_phrase<'a>(first: &str, next: impl Fn(usize) -> Option<&'a str>) -> Option<usize> {
    let list = CURRENCIES.phrases.get(&fold(first))?;
    let longest = list.iter().map(Vec::len).max().unwrap_or(1) - 1;
    let following: Vec<String> = (0..longest).map_while(|k| next(k).map(fold)).collect();
    let stem = |w: &str| w.trim_end_matches('s').to_string();
    let mut best: Option<(usize, bool)> = None;
    for words in list {
        let (mut taken, mut exact, mut content) = (0, true, false);
        for (want, got) in words[1..].iter().zip(&following) {
            if got != want {
                if stem(got) != stem(want) {
                    break;
                }
                exact = false;
            }
            taken += 1;
            content |= !CONNECTORS.contains(&want.as_str());
        }
        let whole = exact && taken == words.len() - 1;
        if taken > 0 && (whole || content) && best.is_none_or(|b| (taken, whole) > b) {
            best = Some((taken, whole));
        }
    }
    best.map(|(taken, _)| taken)
}

/// A moeda escrita em `word`, como o `currency::lookup`: símbolo ou apelido exato,
/// depois o código ("usd" também), depois nome, plural e apelidos sem acento.
pub fn currency(word: &str) -> Option<&'static Currency> {
    let word = word.trim();
    if word.is_empty() {
        return None;
    }
    let index = &*CURRENCIES;
    if let Some(&c) = index.exact.get(word) {
        return Some(c);
    }
    if let Some(&c) = index.codes.get(word.to_ascii_uppercase().as_str()) {
        return Some(c);
    }
    index.folded.get(&fold(word)).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Os jeitos de escrever cada unidade e moeda, com variações de maiúscula e acento.
    fn variants(key: &str) -> Vec<String> {
        vec![key.to_string(), key.to_uppercase(), key.to_lowercase(), fold(key)]
    }

    #[test]
    fn igual_ao_lookup_das_unidades() {
        for cat in units::categories() {
            for u in &cat.units {
                let keys = std::iter::once(u.symbol).chain(std::iter::once(u.name)).chain(u.aliases.iter().copied());
                for key in keys.flat_map(variants) {
                    for after in [false, true] {
                        let want = units::lookup(&key, after).map(|(category, unit)| UnitId { category, unit });
                        let want = if key == "a" { None } else { want };
                        assert_eq!(unit(&key, after), want, "{key:?} {after}");
                    }
                }
            }
        }
        assert_eq!(unit("a", true), None);
        assert_eq!(unit("ares", true).map(|u| u.symbol()), Some("a"));
        assert_eq!(unit("min", false), None);
        assert_eq!(unit("min", true).map(|u| u.symbol()), Some("min"));
    }

    #[test]
    fn igual_ao_lookup_das_moedas() {
        for c in currency::currencies() {
            let keys = [c.code, c.symbol, c.name, c.plural].into_iter().chain(c.aliases.iter().copied()).filter(|k| !k.is_empty());
            for key in keys.flat_map(variants) {
                assert_eq!(currency(&key).map(|c| c.code), currency::lookup(&key).map(|c| c.code), "{key:?}");
            }
        }
        for word in ["", "xyz", "itens", "R$", "$", "€", "£", "¥", "reais", "dolares"] {
            assert_eq!(currency(word).map(|c| c.code), currency::lookup(word).map(|c| c.code), "{word:?}");
        }
    }

    #[test]
    fn moedas_nas_notas() {
        let code = |c: Option<&Currency>| c.map(|c| c.code);
        // Código em minúscula só os da lista; os outros, em maiúsculas.
        assert_eq!(code(currency_word("usd")), Some("USD"));
        assert_eq!(code(currency_word("Brl")), Some("BRL"));
        assert_eq!(code(currency_word("TOP")), Some("TOP"));
        for word in ["top", "cup", "gel", "Top", "all", "mop"] {
            assert_eq!(currency_word(word), None, "{word}");
        }
        assert_eq!(code(currency_word("dólares")), Some("USD"));
        assert_eq!(code(currency_word("reais brasileiros")), Some("BRL"));
        // Símbolos com letras, sem diferenciar maiúsculas.
        assert_eq!(code(prefixed_symbol("r", '$')), Some("BRL"));
        assert_eq!(code(prefixed_symbol("Ca", '$')), Some("CAD"));
        assert_eq!(code(prefixed_symbol("CN", '¥')), Some("CNY"));
        assert_eq!(prefixed_symbol("XX", '$'), None);
        assert_eq!(prefixed_symbol("CN", '$'), None);
        // Cada símbolo da tabela com letras antes do "$" ou do "¥" é achado.
        for c in currency::currencies() {
            for key in std::iter::once(c.symbol).chain(c.aliases.iter().copied()) {
                if let Some(sign @ ('$' | '¥')) = key.chars().last()
                    && key.len() > sign.len_utf8()
                    && key[..key.len() - sign.len_utf8()].chars().all(|ch| ch.is_ascii_alphabetic())
                {
                    assert_eq!(code(prefixed_symbol(&key[..key.len() - sign.len_utf8()], sign)), Some(c.code), "{key}");
                }
            }
        }
        // Nomes compostos: inteiro, pela metade com palavra do nome, ou só com o plural
        // trocado; o "de" sozinho não continua.
        let phrase = |first: &str, rest: &[&'static str]| currency_phrase(first, |k| rest.get(k).copied());
        assert_eq!(phrase("dólares", &["canadenses", "e"]), Some(1));
        assert_eq!(phrase("Dolares", &["de", "Hong", "Kong"]), Some(3));
        assert_eq!(phrase("dólares", &["de", "Hong"]), Some(2));
        assert_eq!(phrase("dólares", &["canadense"]), Some(1));
        assert_eq!(phrase("dólares", &["de", "lucro"]), None);
        assert_eq!(phrase("dólares", &[]), None);
        assert_eq!(phrase("itens", &["canadenses"]), None);
    }
}
