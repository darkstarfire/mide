//! Moedas da Calculadora (E3b, conversor): a tabela própria das moedas ISO 4217 que o
//! Banco Central do Brasil publica, com nomes em português, e as cotações lidas do
//! arquivo "Cotação de todas as moedas" do fechamento PTAX (`AAAAMMDD.csv`).
//!
//! Sem rede aqui: a busca e o cache ficam em `rates`. Tudo em reais por unidade da
//! moeda, pela taxa de venda (a "PTAX" de contratos e notícias).

use std::collections::BTreeMap;
use std::fmt;

use chrono::NaiveDate;

use super::format::{self, Style};
use crate::apps::fold;

/// Uma moeda da tabela.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Currency {
    /// Código ISO 4217: "USD".
    pub code: &'static str,
    /// Nome na lista: "Dólar americano".
    pub name: &'static str,
    /// Plural, para as notas e a busca: "dólares americanos".
    pub plural: &'static str,
    /// Símbolo na tela ("US$"); vazio quando não há um que não se confunda (vale o
    /// código).
    pub symbol: &'static str,
    /// Casas decimais da moeda pelo ISO 4217 (iene 0, dinar kuwaitiano 3).
    pub decimals: u8,
    /// Outros jeitos de escrever: "$", "dólar", "yen".
    pub aliases: &'static [&'static str],
}

impl Currency {
    /// O símbolo, ou o código quando a moeda não tem um próprio.
    pub fn symbol(&self) -> &'static str {
        if self.symbol.is_empty() { self.code } else { self.symbol }
    }

    /// Todos os jeitos de escrever a moeda (código, símbolo, nome, plural e apelidos).
    fn keys(&self) -> impl Iterator<Item = &'static str> + '_ {
        [self.code, self.symbol, self.name, self.plural].into_iter().chain(self.aliases.iter().copied()).filter(|k| !k.is_empty())
    }
}

const fn cur(
    code: &'static str,
    name: &'static str,
    plural: &'static str,
    symbol: &'static str,
    decimals: u8,
    aliases: &'static [&'static str],
) -> Currency {
    Currency { code, name, plural, symbol, decimals, aliases }
}

/// As moedas, em ordem alfabética do código. Símbolos só quando não se confundem
/// ("$" sozinho é o dólar americano, como no Brasil; "kr" ficaria para quatro coroas)
/// e com glifos que as fontes comuns têm; os outros sinais ficam como apelidos.
/// Casas decimais pela tabela A.1 do ISO 4217.
static CURRENCIES: &[Currency] = &[
    cur("AED", "Dirham dos Emirados Árabes Unidos", "dirhams dos Emirados Árabes Unidos", "", 2, &["dirham dos emirados"]),
    cur("AFN", "Afegane afegão", "afeganes afegãos", "", 2, &[]),
    cur("ALL", "Lek albanês", "leks albaneses", "", 2, &[]),
    cur("AMD", "Dram armênio", "drams armênios", "", 2, &[]),
    cur("AOA", "Kwanza angolano", "kwanzas angolanos", "", 2, &["kwanza"]),
    cur("ARS", "Peso argentino", "pesos argentinos", "", 2, &[]),
    cur("AUD", "Dólar australiano", "dólares australianos", "AU$", 2, &[]),
    cur("AWG", "Florim arubano", "florins arubanos", "", 2, &[]),
    cur("AZN", "Manat azerbaijano", "manats azerbaijanos", "", 2, &["₼"]),
    cur("BBD", "Dólar de Barbados", "dólares de Barbados", "", 2, &[]),
    cur("BDT", "Taka bengalesa", "takas bengalesas", "", 2, &["৳"]),
    cur("BHD", "Dinar bareinita", "dinares bareinitas", "", 3, &[]),
    cur("BIF", "Franco burundiano", "francos burundianos", "", 0, &[]),
    cur("BMD", "Dólar bermudense", "dólares bermudenses", "", 2, &[]),
    cur("BND", "Dólar de Brunei", "dólares de Brunei", "", 2, &[]),
    cur("BOB", "Boliviano", "bolivianos", "", 2, &[]),
    cur("BRL", "Real brasileiro", "reais brasileiros", "R$", 2, &["real", "reais"]),
    cur("BSD", "Dólar bahamense", "dólares bahamenses", "", 2, &[]),
    cur("BTN", "Ngultrum butanês", "ngultruns butaneses", "", 2, &[]),
    cur("BWP", "Pula de Botsuana", "pulas de Botsuana", "", 2, &[]),
    cur("BYN", "Rublo bielorrusso", "rublos bielorrussos", "", 2, &[]),
    cur("BZD", "Dólar de Belize", "dólares de Belize", "", 2, &[]),
    cur("CAD", "Dólar canadense", "dólares canadenses", "CA$", 2, &[]),
    cur("CDF", "Franco congolês", "francos congoleses", "", 2, &[]),
    cur("CHF", "Franco suíço", "francos suíços", "", 2, &[]),
    cur("CLP", "Peso chileno", "pesos chilenos", "", 0, &[]),
    cur("CNY", "Yuan chinês", "yuans chineses", "CN¥", 2, &["yuan", "yuans", "iuane", "renminbi"]),
    cur("COP", "Peso colombiano", "pesos colombianos", "", 2, &[]),
    cur("CRC", "Colón costarriquenho", "colóns costarriquenhos", "₡", 2, &[]),
    cur("CUP", "Peso cubano", "pesos cubanos", "", 2, &[]),
    cur("CVE", "Escudo cabo-verdiano", "escudos cabo-verdianos", "", 2, &[]),
    cur("CZK", "Coroa tcheca", "coroas tchecas", "", 2, &[]),
    cur("DJF", "Franco djibutiano", "francos djibutianos", "", 0, &[]),
    cur("DKK", "Coroa dinamarquesa", "coroas dinamarquesas", "", 2, &[]),
    cur("DOP", "Peso dominicano", "pesos dominicanos", "", 2, &[]),
    cur("DZD", "Dinar argelino", "dinares argelinos", "", 2, &[]),
    cur("EGP", "Libra egípcia", "libras egípcias", "", 2, &[]),
    cur("ERN", "Nakfa eritreia", "nakfas eritreias", "", 2, &[]),
    cur("ETB", "Birr etíope", "birrs etíopes", "", 2, &[]),
    cur("EUR", "Euro", "euros", "€", 2, &[]),
    cur("FJD", "Dólar fijiano", "dólares fijianos", "", 2, &[]),
    cur("FKP", "Libra das Ilhas Malvinas", "libras das Ilhas Malvinas", "", 2, &[]),
    cur("GBP", "Libra esterlina", "libras esterlinas", "£", 2, &["libra", "libras"]),
    cur("GEL", "Lari georgiano", "laris georgianos", "", 2, &["₾"]),
    cur("GHS", "Cedi ganês", "cedis ganeses", "", 2, &[]),
    cur("GIP", "Libra de Gibraltar", "libras de Gibraltar", "", 2, &[]),
    cur("GMD", "Dalasi gambiano", "dalasis gambianos", "", 2, &[]),
    cur("GNF", "Franco guineano", "francos guineanos", "", 0, &[]),
    cur("GTQ", "Quetzal guatemalteco", "quetzais guatemaltecos", "", 2, &[]),
    cur("GYD", "Dólar guianense", "dólares guianenses", "", 2, &[]),
    cur("HKD", "Dólar de Hong Kong", "dólares de Hong Kong", "HK$", 2, &[]),
    cur("HNL", "Lempira hondurenha", "lempiras hondurenhas", "", 2, &[]),
    cur("HTG", "Gourde haitiano", "gourdes haitianos", "", 2, &[]),
    cur("HUF", "Florim húngaro", "florins húngaros", "", 2, &["forint"]),
    cur("IDR", "Rupia indonésia", "rupias indonésias", "", 2, &[]),
    cur("ILS", "Novo shekel israelense", "novos shekels israelenses", "₪", 2, &["shekel", "shekels"]),
    cur("INR", "Rupia indiana", "rupias indianas", "₹", 2, &[]),
    cur("IQD", "Dinar iraquiano", "dinares iraquianos", "", 3, &[]),
    cur("IRR", "Rial iraniano", "riais iranianos", "", 2, &[]),
    cur("ISK", "Coroa islandesa", "coroas islandesas", "", 0, &[]),
    cur("JMD", "Dólar jamaicano", "dólares jamaicanos", "", 2, &[]),
    cur("JOD", "Dinar jordaniano", "dinares jordanianos", "", 3, &[]),
    cur("JPY", "Iene japonês", "ienes japoneses", "¥", 0, &["iene", "ienes", "yen"]),
    cur("KES", "Xelim queniano", "xelins quenianos", "", 2, &[]),
    cur("KGS", "Som quirguiz", "soms quirguizes", "", 2, &[]),
    cur("KHR", "Riel cambojano", "riels cambojanos", "", 2, &[]),
    cur("KMF", "Franco comorense", "francos comorenses", "", 0, &[]),
    cur("KRW", "Won sul-coreano", "wons sul-coreanos", "₩", 0, &["won"]),
    cur("KWD", "Dinar kuwaitiano", "dinares kuwaitianos", "", 3, &[]),
    cur("KYD", "Dólar das Ilhas Cayman", "dólares das Ilhas Cayman", "", 2, &[]),
    cur("KZT", "Tenge cazaque", "tenges cazaques", "", 2, &["₸"]),
    cur("LAK", "Kip laosiano", "kips laosianos", "₭", 2, &[]),
    cur("LBP", "Libra libanesa", "libras libanesas", "", 2, &[]),
    cur("LKR", "Rupia cingalesa", "rupias cingalesas", "", 2, &["rupia do sri lanka"]),
    cur("LRD", "Dólar liberiano", "dólares liberianos", "", 2, &[]),
    cur("LSL", "Loti do Lesoto", "malotis do Lesoto", "", 2, &["loti"]),
    cur("LYD", "Dinar líbio", "dinares líbios", "", 3, &[]),
    cur("MAD", "Dirham marroquino", "dirhams marroquinos", "", 2, &[]),
    cur("MDL", "Leu moldávio", "leus moldávios", "", 2, &[]),
    cur("MGA", "Ariary malgaxe", "ariarys malgaxes", "", 2, &[]),
    cur("MKD", "Dinar macedônio", "dinares macedônios", "", 2, &["denar"]),
    cur("MMK", "Quiate mianmarense", "quiates mianmarenses", "", 2, &["kyat"]),
    cur("MNT", "Tugrik mongol", "tugriks mongóis", "₮", 2, &[]),
    cur("MOP", "Pataca macaense", "patacas macaenses", "", 2, &[]),
    cur("MRU", "Uguia mauritana", "uguias mauritanas", "", 2, &["ouguiya"]),
    cur("MUR", "Rupia mauriciana", "rupias mauricianas", "", 2, &[]),
    cur("MVR", "Rupia maldiva", "rupias maldivas", "", 2, &["rufiyaa"]),
    cur("MWK", "Kwacha malauiana", "kwachas malauianas", "", 2, &[]),
    cur("MXN", "Peso mexicano", "pesos mexicanos", "MX$", 2, &[]),
    cur("MYR", "Ringgit malaio", "ringgits malaios", "", 2, &["ringgit"]),
    cur("MZN", "Metical moçambicano", "meticais moçambicanos", "", 2, &["metical"]),
    cur("NAD", "Dólar namibiano", "dólares namibianos", "", 2, &[]),
    cur("NGN", "Naira nigeriana", "nairas nigerianas", "₦", 2, &[]),
    cur("NIO", "Córdoba nicaraguense", "córdobas nicaraguenses", "", 2, &[]),
    cur("NOK", "Coroa norueguesa", "coroas norueguesas", "", 2, &[]),
    cur("NPR", "Rupia nepalesa", "rupias nepalesas", "", 2, &[]),
    cur("NZD", "Dólar neozelandês", "dólares neozelandeses", "NZ$", 2, &[]),
    cur("OMR", "Rial omanense", "riais omanenses", "", 3, &[]),
    cur("PAB", "Balboa panamenho", "balboas panamenhos", "", 2, &[]),
    cur("PEN", "Sol peruano", "soles peruanos", "", 2, &[]),
    cur("PGK", "Kina papuásia", "kinas papuásias", "", 2, &[]),
    cur("PHP", "Peso filipino", "pesos filipinos", "₱", 2, &[]),
    cur("PKR", "Rupia paquistanesa", "rupias paquistanesas", "", 2, &[]),
    cur("PLN", "Zloty polonês", "zlotys poloneses", "", 2, &["zloty"]),
    cur("PYG", "Guarani paraguaio", "guaranis paraguaios", "₲", 0, &["guarani", "guaranis"]),
    cur("QAR", "Rial catariano", "riais catarianos", "", 2, &[]),
    cur("RON", "Leu romeno", "leus romenos", "", 2, &[]),
    cur("RSD", "Dinar sérvio", "dinares sérvios", "", 2, &[]),
    cur("RUB", "Rublo russo", "rublos russos", "₽", 2, &["rublo", "rublos"]),
    cur("RWF", "Franco ruandês", "francos ruandeses", "", 0, &[]),
    cur("SAR", "Rial saudita", "riais sauditas", "", 2, &["riyal"]),
    cur("SBD", "Dólar das Ilhas Salomão", "dólares das Ilhas Salomão", "", 2, &[]),
    cur("SCR", "Rupia seichelense", "rupias seichelenses", "", 2, &[]),
    cur("SDG", "Libra sudanesa", "libras sudanesas", "", 2, &[]),
    cur("SEK", "Coroa sueca", "coroas suecas", "", 2, &[]),
    cur("SGD", "Dólar de Singapura", "dólares de Singapura", "S$", 2, &[]),
    cur("SHP", "Libra de Santa Helena", "libras de Santa Helena", "", 2, &[]),
    cur("SOS", "Xelim somali", "xelins somalis", "", 2, &[]),
    cur("SRD", "Dólar surinamês", "dólares surinameses", "", 2, &[]),
    cur("SSP", "Libra sul-sudanesa", "libras sul-sudanesas", "", 2, &[]),
    cur("STN", "Dobra de São Tomé e Príncipe", "dobras de São Tomé e Príncipe", "", 2, &["dobra"]),
    cur("SVC", "Colón salvadorenho", "colóns salvadorenhos", "", 2, &[]),
    cur("SYP", "Libra síria", "libras sírias", "", 2, &[]),
    cur("SZL", "Lilangeni suazi", "emalangeni suazis", "", 2, &["lilangeni"]),
    cur("THB", "Baht tailandês", "bahts tailandeses", "฿", 2, &["baht"]),
    cur("TJS", "Somoni tadjique", "somonis tadjiques", "", 2, &[]),
    cur("TMT", "Manat turcomeno", "manats turcomenos", "", 2, &[]),
    cur("TND", "Dinar tunisiano", "dinares tunisianos", "", 3, &[]),
    cur("TOP", "Pa'anga tonganesa", "pa'angas tonganesas", "", 2, &["paanga"]),
    cur("TRY", "Lira turca", "liras turcas", "₺", 2, &[]),
    cur("TTD", "Dólar de Trinidad e Tobago", "dólares de Trinidad e Tobago", "", 2, &[]),
    cur("TWD", "Novo dólar taiwanês", "novos dólares taiwaneses", "NT$", 2, &[]),
    cur("TZS", "Xelim tanzaniano", "xelins tanzanianos", "", 2, &[]),
    cur("UAH", "Hryvnia ucraniana", "hryvnias ucranianas", "₴", 2, &["hryvnia"]),
    cur("UGX", "Xelim ugandense", "xelins ugandenses", "", 0, &[]),
    cur("USD", "Dólar americano", "dólares americanos", "US$", 2, &["$", "dólar", "dólares", "dólar dos Estados Unidos"]),
    cur("UYU", "Peso uruguaio", "pesos uruguaios", "", 2, &[]),
    cur("UZS", "Som uzbeque", "soms uzbeques", "", 2, &[]),
    cur("VES", "Bolívar venezuelano", "bolívares venezuelanos", "", 2, &["bolívar"]),
    cur("VND", "Dong vietnamita", "dongs vietnamitas", "₫", 0, &["dong"]),
    cur("VUV", "Vatu de Vanuatu", "vatus de Vanuatu", "", 0, &["vatu"]),
    cur("WST", "Tala samoano", "talas samoanos", "", 2, &["tala"]),
    cur("XAF", "Franco CFA da África Central", "francos CFA da África Central", "", 0, &[]),
    cur("XCD", "Dólar do Caribe Oriental", "dólares do Caribe Oriental", "EC$", 2, &[]),
    cur("XCG", "Florim do Caribe", "florins do Caribe", "", 2, &[]),
    cur("XOF", "Franco CFA da África Ocidental", "francos CFA da África Ocidental", "", 0, &[]),
    cur("XPF", "Franco CFP", "francos CFP", "", 0, &[]),
    cur("YER", "Rial iemenita", "riais iemenitas", "", 2, &[]),
    cur("ZAR", "Rand sul-africano", "rands sul-africanos", "", 2, &["rand"]),
    cur("ZMW", "Kwacha zambiana", "kwachas zambianas", "", 2, &[]),
];

/// Códigos do arquivo do BCB que não são moeda corrente do ISO 4217: ouro (cotado por
/// grama, não por onça), o DES do FMI, unidades de conta (CLF, COU), o yuan de fora da
/// China (CNH) e moedas já trocadas (MRO pela MRU, SLL pela SLE).
const LEFT_OUT: &[&str] = &["XAU", "SDR", "CLF", "COU", "CNH", "MRO", "SLL"];

/// Todas as moedas da tabela.
pub fn currencies() -> &'static [Currency] {
    CURRENCIES
}

/// Moeda pelo código ISO ("usd" também vale).
pub fn find(code: &str) -> Option<&'static Currency> {
    let code = code.trim();
    CURRENCIES.iter().find(|c| c.code.eq_ignore_ascii_case(code))
}

/// Moeda pelo texto: primeiro o símbolo exato ("R$", "$", "€", "₲"), depois o código,
/// depois nome, plural e apelidos sem acento nem maiúscula ("dólares", "Peso
/// Argentino").
pub fn lookup(text: &str) -> Option<&'static Currency> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    if let Some(c) = CURRENCIES.iter().find(|c| c.symbol == text || c.aliases.contains(&text)) {
        return Some(c);
    }
    if let Some(c) = find(text) {
        return Some(c);
    }
    let folded = fold(text);
    CURRENCIES.iter().find(|c| c.keys().any(|k| fold(k) == folded))
}

/// Moedas que casam com a busca do seletor (todas as palavras em algum nome, sem
/// acento), da melhor para a pior: código ou símbolo igual; nome que começa com a
/// busca; o resto, em ordem do nome.
pub fn search(query: &str) -> Vec<&'static Currency> {
    let q = fold(query.trim());
    let words: Vec<&str> = q.split_whitespace().collect();
    if words.is_empty() {
        return Vec::new();
    }
    let mut found: Vec<(u8, &'static Currency)> = CURRENCIES
        .iter()
        .filter(|c| {
            let hay = fold(&c.keys().collect::<Vec<_>>().join(" "));
            words.iter().all(|w| hay.contains(w))
        })
        .map(|c| {
            let rank = if fold(c.code) == q || fold(c.symbol()) == q {
                0
            } else if fold(c.name).starts_with(&q) || fold(c.plural).starts_with(&q) {
                1
            } else {
                2
            };
            (rank, c)
        })
        .collect();
    found.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| fold(a.1.name).cmp(&fold(b.1.name))));
    found.into_iter().map(|(_, c)| c).collect()
}

/// Cotações de um dia: quantos reais vale 1 unidade de cada moeda (BRL = 1).
#[derive(Debug, Clone, PartialEq)]
pub struct Rates {
    /// Data do boletim.
    pub date: NaiveDate,
    brl: BTreeMap<&'static str, f64>,
    /// Códigos do arquivo deixados de fora (fora do ISO ou fora da tabela), para o log
    /// de debug.
    pub skipped: Vec<String>,
}

impl Rates {
    /// Reais por 1 unidade da moeda.
    pub fn rate(&self, code: &str) -> Option<f64> {
        let c = find(code)?;
        if c.code == "BRL" {
            return Some(1.0);
        }
        self.brl.get(c.code).copied()
    }

    /// A moeda tem cotação neste boletim.
    pub fn has(&self, code: &str) -> bool {
        self.rate(code).is_some()
    }

    /// `amount` em `from` convertido para `to`, passando pelo real.
    pub fn convert(&self, amount: f64, from: &str, to: &str) -> Option<f64> {
        let v = amount * self.rate(from)? / self.rate(to)?;
        v.is_finite().then_some(v)
    }

    /// Códigos com cotação (sem o BRL), em ordem alfabética.
    pub fn codes(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.brl.keys().copied()
    }
}

/// Por que um arquivo do BCB foi recusado.
#[derive(Debug, Clone, PartialEq)]
pub enum ParseError {
    /// Veio uma página (o 404 do site, uma rede com login), não o CSV.
    Html,
    /// Arquivo sem linhas.
    Empty,
    /// Linha (contada de 1) sem os 8 campos, com data, código ou tipo inválidos.
    Line(usize),
    /// Número que não está no formato do BCB ("5,02770000").
    Number(usize),
    /// Taxa ou paridade zero ou negativa.
    NotPositive(usize),
    /// Linhas de dias diferentes.
    MixedDates,
    /// Sem o dólar, base de todas as contas (ou com paridade diferente de 1).
    NoUsd,
    /// Taxa calculada pela paridade longe da publicada: colunas trocadas ou formato novo.
    Mismatch(String),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::Html => write!(f, "veio uma página HTML, não o arquivo de cotações"),
            ParseError::Empty => write!(f, "arquivo vazio"),
            ParseError::Line(n) => write!(f, "linha {n} fora do formato (8 campos com data, código e tipo)"),
            ParseError::Number(n) => write!(f, "número fora do formato na linha {n}"),
            ParseError::NotPositive(n) => write!(f, "taxa zero ou negativa na linha {n}"),
            ParseError::MixedDates => write!(f, "linhas de datas diferentes"),
            ParseError::NoUsd => write!(f, "arquivo sem o dólar americano com paridade 1"),
            ParseError::Mismatch(code) => write!(f, "taxa do {code} não confere com a paridade"),
        }
    }
}

impl std::error::Error for ParseError {}

/// Uma linha do arquivo, já lida.
struct Row {
    code: String,
    kind: char,
    sell: f64,
    /// Algarismos significativos da venda publicada (para saber se dá para conferir).
    sell_digits: usize,
    parity_buy: f64,
    parity_sell: f64,
}

/// Número do BCB: algarismos, uma vírgula e algarismos ("5,02770000"). O sinal só é
/// aceito para dar o erro certo ("negativo", não "fora do formato"). Um número que
/// estoura o f64 (infinito) também fica fora: daria taxa zero ou NaN nas contas.
fn bcb_number(field: &str) -> Option<f64> {
    let digits = field.strip_prefix('-').unwrap_or(field);
    let (int, frac) = digits.split_once(',')?;
    let ok = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !ok(int) || !ok(frac) {
        return None;
    }
    field.replace(',', ".").parse().ok().filter(|v: &f64| v.is_finite())
}

/// Algarismos significativos de um número escrito ("0,00000270" tem 2: os zeros das
/// pontas não contam).
fn significant_digits(field: &str) -> usize {
    let digits: String = field.chars().filter(char::is_ascii_digit).collect();
    digits.trim_start_matches('0').trim_end_matches('0').len()
}

/// Lê o arquivo "Cotação de todas as moedas" do BCB:
/// `data;código;tipo;moeda;compra;venda;paridade_compra;paridade_venda`, com vírgula
/// decimal. A taxa em reais sai da paridade com o dólar, que tem mais algarismos que a
/// coluna de venda (o rial iraniano aparece como 0,00000270): no tipo A a paridade é
/// moeda por dólar (venda = dólar ÷ paridade de compra); no tipo B, dólar por moeda
/// (venda = dólar × paridade de venda). Quando a venda publicada tem 4 ou mais
/// algarismos, a conta precisa conferir com ela em 0,5%: é o que pega colunas
/// trocadas ou um formato novo.
pub fn parse_bcb(text: &str) -> Result<Rates, ParseError> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    if text.trim_start().starts_with('<') || text.to_ascii_lowercase().contains("<html") {
        return Err(ParseError::Html);
    }
    let mut date: Option<NaiveDate> = None;
    let mut rows = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let n = i + 1;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split(';').map(str::trim).collect();
        if f.len() != 8 {
            return Err(ParseError::Line(n));
        }
        let day = NaiveDate::parse_from_str(f[0], "%d/%m/%Y").map_err(|_| ParseError::Line(n))?;
        let code_ok = f[1].len() == 3 && f[1].bytes().all(|b| b.is_ascii_digit());
        let iso_ok = f[3].len() == 3 && f[3].bytes().all(|b| b.is_ascii_uppercase());
        let kind = match f[2] {
            "A" => 'A',
            "B" => 'B',
            _ => return Err(ParseError::Line(n)),
        };
        if !code_ok || !iso_ok {
            return Err(ParseError::Line(n));
        }
        if date.is_some_and(|d| d != day) {
            return Err(ParseError::MixedDates);
        }
        date = Some(day);
        let mut nums = [0.0; 4];
        for (k, field) in f[4..].iter().enumerate() {
            nums[k] = bcb_number(field).ok_or(ParseError::Number(n))?;
            if nums[k] <= 0.0 {
                return Err(ParseError::NotPositive(n));
            }
        }
        rows.push((n, Row { code: f[3].to_string(), kind, sell: nums[1], sell_digits: significant_digits(f[5]), parity_buy: nums[2], parity_sell: nums[3] }));
    }
    let date = date.ok_or(ParseError::Empty)?;
    let usd = rows
        .iter()
        .find(|(_, r)| r.code == "USD" && r.parity_buy == 1.0 && r.parity_sell == 1.0)
        .map(|(_, r)| r.sell)
        .ok_or(ParseError::NoUsd)?;
    let mut brl = BTreeMap::new();
    let mut skipped = Vec::new();
    for (n, row) in &rows {
        let currency = match find(&row.code) {
            Some(c) if !LEFT_OUT.contains(&row.code.as_str()) && c.code != "BRL" => c,
            _ => {
                skipped.push(row.code.clone());
                continue;
            }
        };
        let rate = if row.kind == 'A' { usd / row.parity_buy } else { usd * row.parity_sell };
        // A conta pode sair de 0 ou infinito mesmo com números finitos (paridade enorme).
        if !rate.is_finite() || rate <= 0.0 {
            return Err(ParseError::NotPositive(*n));
        }
        if row.sell_digits >= 4 && ((rate - row.sell) / row.sell).abs() > 0.005 {
            return Err(ParseError::Mismatch(row.code.clone()));
        }
        brl.insert(currency.code, rate);
    }
    Ok(Rates { date, brl, skipped })
}

/// Um valor em dinheiro: com as casas da moeda (2 no real, 0 no iene, 3 no dinar
/// kuwaitiano), ou com as escolhidas em Visualizar → Casas decimais.
pub fn format_amount(v: f64, code: &str, style: &Style) -> String {
    let decimals = style.decimals.unwrap_or_else(|| find(code).map_or(2, |c| c.decimals));
    Style { decimals: Some(decimals), grouping: style.grouping }.value(v)
}

/// A cotação de um par, com 5 algarismos significativos (sem cortar a parte inteira):
/// "1 USD = 5,0277 BRL", "1 BRL = 364.282 IRR".
pub fn format_rate(rates: &Rates, from: &str, to: &str) -> Option<String> {
    let v = rates.convert(1.0, from, to)?;
    let (from, to) = (find(from)?.code, find(to)?.code);
    Some(format!("1 {from} = {} {to}", format::group_thousands(&format::significant(v, 5))))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Trechos do arquivo real do Banco Central do Brasil, "Cotação de todas as moedas"
    /// do fechamento PTAX de 21/10/2026
    /// (https://www4.bcb.gov.br/Download/fechamento/20261021.csv).
    const BCB_20261021: &str = "\
21/10/2026;100;A;KWD;16,31610000;16,32370000;0,30800000;0,30810000
21/10/2026;220;A;USD;5,02700000;5,02770000;1,00000000;1,00000000
21/10/2026;450;A;PYG;0,00087350;0,00087370;5754,26000000;5754,77000000
21/10/2026;470;A;JPY;0,03179000;0,03179000;158,13000000;158,15000000
21/10/2026;540;B;GBP;6,72910000;6,73210000;1,33860000;1,33900000
21/10/2026;706;A;ARS;0,00329200;0,00329400;1526,50000000;1527,00000000
21/10/2026;815;A;IRR;0,00000270;0,00000270;1831500,00000000;1833250,00000000
21/10/2026;978;B;EUR;5,63980000;5,64110000;1,12190000;1,12200000
21/10/2026;998;A;XAU;667,77360000;667,86660000;0,00752800;0,00752800
";

    fn rates() -> Rates {
        parse_bcb(BCB_20261021).expect("arquivo do BCB")
    }

    fn close(a: f64, b: f64, tol: f64) -> bool {
        ((a - b) / b).abs() < tol
    }

    #[test]
    fn le_o_arquivo_do_bcb() {
        let r = rates();
        assert_eq!(r.date, NaiveDate::from_ymd_opt(2026, 10, 21).unwrap());
        assert_eq!(r.rate("USD"), Some(5.0277));
        assert_eq!(r.rate("BRL"), Some(1.0));
        assert!(close(r.rate("EUR").unwrap(), 5.6411, 1e-4), "{:?}", r.rate("EUR"));
        assert!(close(r.rate("JPY").unwrap(), 0.03179, 1e-3));
        assert!(close(r.rate("PYG").unwrap(), 0.0008737, 1e-3));
        // Pela paridade, com mais algarismos que o 0,00000270 publicado.
        assert!(close(r.rate("IRR").unwrap(), 2.745_126_9e-6, 1e-7), "{:?}", r.rate("IRR"));
        assert!(!r.has("XAU"));
        assert_eq!(r.skipped, ["XAU"]);
        assert!(r.has("usd"));
        assert_eq!(r.codes().count(), 8);
    }

    #[test]
    fn recusa_arquivos_estranhos() {
        let html = "<!DOCTYPE html>\n<html><head><title>404</title></head><body>Not Found</body></html>";
        assert_eq!(parse_bcb(html), Err(ParseError::Html));
        assert_eq!(parse_bcb(""), Err(ParseError::Empty));
        assert_eq!(parse_bcb("21/10/2026;220;A;USD;5,02700000;5,02770000;1,00000000"), Err(ParseError::Line(1)));
        let mixed = "21/10/2026;220;A;USD;5,02700000;5,02770000;1,00000000;1,00000000\n\
                     20/10/2026;978;B;EUR;5,63980000;5,64110000;1,12190000;1,12200000";
        assert_eq!(parse_bcb(mixed), Err(ParseError::MixedDates));
        let zero = BCB_20261021.replace("0,03179000;0,03179000", "0,00000000;0,03179000");
        assert_eq!(parse_bcb(&zero), Err(ParseError::NotPositive(4)));
        let negative = BCB_20261021.replace("158,13000000", "-158,13000000");
        assert_eq!(parse_bcb(&negative), Err(ParseError::NotPositive(4)));
        let dot = BCB_20261021.replace("158,13000000", "158.13");
        assert_eq!(parse_bcb(&dot), Err(ParseError::Number(4)));
        let no_usd: String = BCB_20261021.lines().filter(|l| !l.contains("USD")).map(|l| format!("{l}\n")).collect();
        assert_eq!(parse_bcb(&no_usd), Err(ParseError::NoUsd));
        // Venda e paridade de venda trocadas: a conta pela paridade não confere.
        let swapped = BCB_20261021.replace("5,64110000;1,12190000;1,12200000", "1,12200000;1,12190000;5,64110000");
        assert_eq!(parse_bcb(&swapped), Err(ParseError::Mismatch("EUR".into())));
        // Tipo e código trocados.
        let kind = BCB_20261021.replace("470;A;JPY", "A;470;JPY");
        assert_eq!(parse_bcb(&kind), Err(ParseError::Line(4)));
        // Número que estoura o f64 (infinito): recusado, não vira taxa zero.
        let huge = format!("1{},00000000", "0".repeat(400));
        let inf = BCB_20261021.replace("1831500,00000000;1833250,00000000", &format!("{huge};{huge}"));
        assert_eq!(parse_bcb(&inf), Err(ParseError::Number(7)));
        let inf_sell = BCB_20261021.replace("0,03179000;0,03179000", &format!("0,03179000;{huge}"));
        assert_eq!(parse_bcb(&inf_sell), Err(ParseError::Number(4)));
        // Paridade finita (1e308), mas a conta dólar × paridade estoura: recusada.
        let big = format!("1{},0", "0".repeat(308));
        let overflow = BCB_20261021.replace("1,33860000;1,33900000", &format!("{big};{big}"));
        assert_eq!(parse_bcb(&overflow), Err(ParseError::NotPositive(5)));
        // CRLF e linhas em branco passam.
        assert!(parse_bcb(&BCB_20261021.replace('\n', "\r\n\r\n")).is_ok());
    }

    #[test]
    fn converte_pelo_real() {
        let r = rates();
        let style = Style::default();
        let show = |v: Option<f64>, code| format_amount(v.unwrap(), code, &style);
        assert_eq!(show(r.convert(100.0, "USD", "BRL"), "BRL"), "502,77");
        assert_eq!(show(r.convert(100.0, "BRL", "USD"), "USD"), "19,89");
        assert_eq!(show(r.convert(100.0, "EUR", "USD"), "USD"), "112,20");
        assert_eq!(show(r.convert(1_000_000.0, "IRR", "BRL"), "BRL"), "2,75");
        for (a, b) in [("USD", "EUR"), ("IRR", "JPY"), ("BRL", "PYG"), ("KWD", "GBP")] {
            let there = r.convert(123.45, a, b).unwrap();
            let back = r.convert(there, b, a).unwrap();
            assert!(((back - 123.45) / 123.45).abs() < 1e-12, "{a} {b}: {back}");
        }
        assert_eq!(r.convert(1.0, "USD", "XYZ"), None);
        assert_eq!(r.convert(1.0, "XAU", "BRL"), None);
        assert_eq!(r.convert(1.0, "CHF", "BRL"), None);
    }

    #[test]
    fn valores_e_cotacoes_na_tela() {
        let r = rates();
        let style = Style::default();
        assert_eq!(format_amount(502.77, "BRL", &style), "502,77");
        assert_eq!(format_amount(r.convert(100.0, "BRL", "JPY").unwrap(), "JPY", &style), "3.145");
        assert_eq!(format_amount(r.convert(100.0, "BRL", "KWD").unwrap(), "KWD", &style), "6,126");
        let four = Style { decimals: Some(4), grouping: true };
        assert_eq!(format_amount(1234.5, "USD", &four), "1.234,5000");
        assert_eq!(format_amount(1234.5, "JPY", &Style { decimals: None, grouping: false }), "1235");
        assert_eq!(format_rate(&r, "USD", "BRL").as_deref(), Some("1 USD = 5,0277 BRL"));
        assert_eq!(format_rate(&r, "BRL", "IRR").as_deref(), Some("1 BRL = 364.282 IRR"));
        assert_eq!(format_rate(&r, "eur", "USD").as_deref(), Some("1 EUR = 1,122 USD"));
        assert_eq!(format_rate(&r, "BRL", "XYZ"), None);
    }

    #[test]
    fn reconhece_moedas_pelo_texto() {
        let code = |t: &str| lookup(t).map(|c| c.code);
        assert_eq!(code("dólares"), Some("USD"));
        assert_eq!(code("dolares"), Some("USD"));
        assert_eq!(code("US$"), Some("USD"));
        assert_eq!(code("$"), Some("USD"));
        assert_eq!(code("usd"), Some("USD"));
        assert_eq!(code("reais"), Some("BRL"));
        assert_eq!(code("R$"), Some("BRL"));
        assert_eq!(code("Peso Argentino"), Some("ARS"));
        assert_eq!(code("pesos argentinos"), Some("ARS"));
        assert_eq!(code("₲"), Some("PYG"));
        assert_eq!(code("¥"), Some("JPY"));
        assert_eq!(code("€"), Some("EUR"));
        assert_eq!(code("£"), Some("GBP"));
        assert_eq!(code("xyz"), None);
        assert_eq!(code(""), None);
        assert_eq!(find("eur").map(|c| c.code), Some("EUR"));
        assert_eq!(find("dólar"), None);
        let first = |q: &str| search(q).first().map(|c| c.code);
        assert_eq!(first("dolar"), Some("USD"));
        assert_eq!(first("JPY"), Some("JPY"));
        assert_eq!(first("peso arg"), Some("ARS"));
        assert!(search("coroa").len() >= 4);
    }

    #[test]
    fn tabela_sem_repetidos() {
        let mut seen: BTreeMap<String, &str> = BTreeMap::new();
        for c in currencies() {
            assert!(c.code.len() == 3 && c.code.bytes().all(|b| b.is_ascii_uppercase()), "{}", c.code);
            assert!(c.decimals <= 3, "{}", c.code);
            for key in c.keys() {
                // Cada jeito de escrever leva a uma moeda só (sem acento e maiúscula).
                if let Some(other) = seen.insert(fold(key), c.code) {
                    assert_eq!(other, c.code, "{key} em {other} e {}", c.code);
                }
            }
        }
        // Em ordem de código, sem repetir.
        assert!(currencies().windows(2).all(|w| w[0].code < w[1].code));
        assert!(LEFT_OUT.iter().all(|code| find(code).is_none()));
    }
}
