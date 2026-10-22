//! Números na tela da calculadora: vírgula decimal, ponto de milhar, até 12 algarismos
//! e notação científica quando não cabe; casas decimais fixas, se escolhidas.

/// Maior número de algarismos mostrados.
pub const DIGITS: usize = 12;

/// Como mostrar os números (menu Visualizar): casas decimais e ponto de milhar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Style {
    /// Casas decimais fixas nos resultados; `None`: as que precisar (até `DIGITS`
    /// algarismos).
    pub decimals: Option<u8>,
    /// Ponto de milhar ("1.234.567").
    pub grouping: bool,
}

impl Default for Style {
    fn default() -> Self {
        Self { decimals: None, grouping: true }
    }
}

impl Style {
    /// Um resultado na tela.
    pub fn value(&self, v: f64) -> String {
        let text = match self.decimals {
            Some(d) if v.abs() < 1e15 => fixed(v, d as usize),
            _ => to_entry(v),
        };
        self.digits(&text)
    }

    /// Um número já escrito ("1234,5"), com o ponto de milhar se ligado.
    pub fn digits(&self, entry: &str) -> String {
        if self.grouping { group_thousands(entry) } else { entry.to_string() }
    }
}

/// Com `decimals` casas, sem "-0". Empates arredondam para longe do zero (1234,5 com
/// 0 casas dá 1235, como numa calculadora; o `format!` arredondaria para o par), e
/// só os 15 primeiros algarismos são do número (o resto do `f64` é ruído): as casas
/// que passam disso saem como zeros.
fn fixed(v: f64, decimals: usize) -> String {
    let int_digits = if v.abs() >= 1.0 { v.abs().log10().floor() as usize + 1 } else { 0 };
    let exact = decimals.min(15usize.saturating_sub(int_digits));
    let factor = 10f64.powi(exact as i32);
    let mut s = format!("{:.*}", exact, (v * factor).round() / factor);
    if decimals > exact {
        if exact == 0 {
            s.push('.');
        }
        s.push_str(&"0".repeat(decimals - exact));
    }
    let s = s.replace('.', ",");
    match s.strip_prefix('-') {
        Some(rest) if rest.chars().all(|c| c == '0' || c == ',') => rest.to_string(),
        _ => s,
    }
}

/// Algarismos de um texto (sem sinal, vírgula nem expoente).
fn count_digits(s: &str) -> usize {
    s.chars().filter(char::is_ascii_digit).count()
}

/// Menor ordem de grandeza na forma fixa: de 0,000001 para baixo, a forma fixa
/// gastaria os algarismos com zeros (0,00000000012); a científica mostra mais
/// (1,23456789e-10). Até aqui, a fixa mostra pelo menos 6 algarismos significativos.
const SMALLEST_FIXED: i32 = -6;

/// Número como seria digitado: vírgula decimal, sem zeros sobrando, no máximo
/// `DIGITS` algarismos (notação científica quando não cabe).
pub fn to_entry(v: f64) -> String {
    if v == 0.0 || !v.is_finite() {
        return "0".into();
    }
    let magnitude = v.abs().log10().floor() as i32;
    if (SMALLEST_FIXED..DIGITS as i32).contains(&magnitude) {
        let mut decimals = (DIGITS as i32 - 1 - magnitude.max(0)).max(0) as usize;
        loop {
            let s = format!("{:.*}", decimals, v);
            let s = if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s };
            // O arredondamento pode ganhar um algarismo (999999999999,5 vira 13).
            if count_digits(&s) <= DIGITS {
                return if s == "-0" { "0".into() } else { s.replace('.', ",") };
            }
            if decimals == 0 {
                break;
            }
            decimals -= 1;
        }
    }
    scientific(v)
}

/// Com `n` algarismos significativos, sem cortar a parte inteira (364282,08 com 5 dá
/// "364282") e sem zeros sobrando ("1,122", não "1,1220"); notação científica quando a
/// forma fixa passaria de 15 casas ou da faixa do `f64` exato.
pub fn significant(v: f64, n: usize) -> String {
    if v == 0.0 || !v.is_finite() {
        return "0".into();
    }
    let magnitude = v.abs().log10().floor() as i32;
    let decimals = (n as i32 - 1 - magnitude).max(0);
    if decimals > 15 || v.abs() >= 1e15 {
        return to_entry(v);
    }
    let s = fixed(v, decimals as usize);
    if s.contains(',') { s.trim_end_matches('0').trim_end_matches(',').to_string() } else { s }
}

/// Notação científica com o máximo de algarismos que cabe ("1,125899907e15").
fn scientific(v: f64) -> String {
    for decimals in (0..DIGITS).rev() {
        let s = format!("{:.*e}", decimals, v);
        let (mantissa, exp) = s.split_once('e').unwrap_or((&s, "0"));
        let mantissa = if mantissa.contains('.') { mantissa.trim_end_matches('0').trim_end_matches('.') } else { mantissa };
        if count_digits(mantissa) + count_digits(exp) <= DIGITS || decimals == 0 {
            return format!("{mantissa}e{exp}").replace('.', ",");
        }
    }
    unreachable!("o laço termina em decimals == 0")
}

/// Pontos de milhar na parte inteira ("1234567,5" vira "1.234.567,5").
pub fn group_thousands(entry: &str) -> String {
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

/// "1.234" e "12.345.678": pontos só entre grupos de 3 algarismos (o primeiro grupo
/// com 1 a 3, sem zero à esquerda) são de milhar, como na tela.
fn thousands_only(text: &str) -> bool {
    let digits = text.strip_prefix('-').unwrap_or(text);
    let mut groups = digits.split('.');
    let first = groups.next().unwrap_or("");
    let rest: Vec<&str> = groups.collect();
    let all_digits = |g: &str| !g.is_empty() && g.chars().all(|c| c.is_ascii_digit());
    !rest.is_empty()
        && (1..=3).contains(&first.len())
        && all_digits(first)
        && !first.starts_with('0')
        && rest.iter().all(|g| g.len() == 3 && all_digits(g))
}

/// Lê um número digitado ou colado ("1.234,5", "1.234", "1234.5", "1,234.5", "-3",
/// "2,5e3"). Com vírgula e ponto juntos, o último dos dois separa as casas decimais.
pub fn parse(text: &str) -> Option<f64> {
    let text: String = text.trim().chars().filter(|c| !c.is_whitespace()).map(|c| if c == '−' { '-' } else { c }).collect();
    let normalized = if text.contains(',') && text.contains('.') && text.rfind('.') > text.rfind(',') {
        // Formato inglês: "1,234.5".
        text.replace(',', "")
    } else if text.contains(',') {
        text.replace('.', "").replace(',', ".")
    } else if thousands_only(&text) {
        text.replace('.', "")
    } else {
        text
    };
    normalized.parse::<f64>().ok().filter(|v| v.is_finite())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeros_na_tela() {
        assert_eq!(to_entry(2.5), "2,5");
        assert_eq!(to_entry(1.0 / 3.0), "0,33333333333");
        assert_eq!(to_entry(1e15), "1e15");
        assert_eq!(to_entry(-0.0), "0");
        assert_eq!(group_thousands("1234567,5"), "1.234.567,5");
        assert_eq!(group_thousands("-1234"), "-1.234");
        assert_eq!(parse("1.234,5"), Some(1234.5));
        assert_eq!(parse("3.25"), Some(3.25));
        assert_eq!(parse("abc"), None);
    }

    #[test]
    fn nunca_mais_de_12_algarismos() {
        // O arredondamento ganhava um algarismo: "1000000000000".
        assert_eq!(to_entry(999_999_999_999.5), "1e12");
        assert_eq!(to_entry(99_999_999_999.99), "100000000000");
        assert_eq!(to_entry(123_456_789_012.0), "123456789012");
        // Notação científica com os algarismos que cabem (antes, 7).
        assert_eq!(to_entry(2f64.powi(50)), "1,125899907e15");
        assert_eq!(to_entry(-1.5e-20), "-1,5e-20");
        // Número pequeno: a científica quando a fixa perderia algarismos.
        assert_eq!(to_entry(1.234_567_89e-10), "1,23456789e-10");
        assert_eq!(to_entry(0.000_05), "0,00005");
        assert_eq!(to_entry(0.000_012_345), "0,000012345");
        assert_eq!(to_entry(0.1 + 0.2), "0,3");
        // 1 ÷ 12 continua na forma fixa (não "8,3333333333e-2").
        assert_eq!(to_entry(1.0 / 12.0), "0,08333333333");
        assert_eq!(to_entry(1.0 / 81.0), "0,01234567901");
        assert_eq!(to_entry(0.000_000_1), "1e-7");
        for v in [1.0 / 7.0, 2f64.sqrt() * 1e9, 98_765.432_109_876, -0.000_012_345_678_9] {
            assert!(count_digits(&to_entry(v)) <= DIGITS, "{v}: {}", to_entry(v));
        }
    }

    #[test]
    fn copiar_e_colar_de_volta() {
        // O que a tela mostra (com pontos de milhar) volta ao mesmo número.
        for v in [1234.0, 1_234_567.0, 1234.5, -9876.25, 0.125, 3.25, 1e20] {
            let shown = Style::default().value(v);
            assert_eq!(parse(&shown), Some(v), "{shown}");
        }
        assert_eq!(parse("1.234"), Some(1234.0));
        assert_eq!(parse("12.345.678"), Some(12_345_678.0));
        assert_eq!(parse("0.125"), Some(0.125));
        assert_eq!(parse("1.5e3"), Some(1500.0));
        assert_eq!(parse("2,5e3"), Some(2500.0));
        assert_eq!(parse("−3"), Some(-3.0));
        assert_eq!(parse(" 1 234,5 "), Some(1234.5));
        assert_eq!(parse("1,234.5"), Some(1234.5));
        assert_eq!(parse("1,234,567.25"), Some(1_234_567.25));
    }

    #[test]
    fn casas_decimais_e_milhar() {
        let two = Style { decimals: Some(2), grouping: true };
        assert_eq!(two.value(1234.5), "1.234,50");
        assert_eq!(two.value(1.0 / 3.0), "0,33");
        assert_eq!(two.value(-0.001), "0,00");
        let zero = Style { decimals: Some(0), grouping: false };
        assert_eq!(zero.value(1234.5), "1235");
        assert_eq!(zero.value(-2.5), "-3");
        assert_eq!(Style { decimals: None, grouping: false }.value(1234.5), "1234,5");
        assert_eq!(two.value(1e20), "1e20");
        let many = Style { decimals: Some(15), grouping: false };
        assert_eq!(many.value(1.0 / 3.0), "0,333333333333333");
        assert_eq!(many.value(123_456.789), "123456,789000000000000");
    }

    #[test]
    fn algarismos_significativos() {
        assert_eq!(significant(5.0277, 5), "5,0277");
        assert_eq!(significant(364_282.08, 5), "364282");
        assert_eq!(significant(1.122_000_1, 5), "1,122");
        assert_eq!(significant(2.745_126_9e-6, 5), "0,0000027451");
        assert_eq!(significant(9.999_99, 5), "10");
        assert_eq!(significant(-0.012_345_6, 3), "-0,0123");
        assert_eq!(significant(0.0, 5), "0");
        assert_eq!(significant(1.5e-20, 5), "1,5e-20");
    }
}
