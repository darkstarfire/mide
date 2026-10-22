//! Conversões de unidades da Calculadora (E3b, etapa 2): as categorias, as unidades
//! com o quanto cada uma vale na unidade de base da categoria, e a conta. Os fatores
//! saem de poucas definições oficiais (polegada, libra, gₙ, galão, caloria...): nenhum
//! número final é digitado, e um teste confere contra a tabela B.8 do NIST (SP 811).
//!
//! Também serve à busca do seletor de unidades e ao Math Notes (reconhecer "km/h" ou
//! "quilômetros" num texto).

use std::sync::LazyLock;

use crate::apps::fold;

/// Como 1 unidade vira a unidade de base da categoria.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Scale {
    /// base = x × fator.
    Linear(f64),
    /// base = (x + deslocamento) × mul ÷ div (temperatura, com base em °C). Multiplicar
    /// antes de dividir deixa os pontos fixos exatos: 32 °F = 0 °C e 212 °F = 100 °C, sem
    /// o resto de 5,7e-14 que a conta pelo kelvin deixava.
    Affine { mul: f64, div: f64, offset: f64 },
    /// base = fator ÷ x (consumo em L/100 km: km/L = 100 ÷ x).
    Inverse(f64),
}

impl Scale {
    /// O valor na unidade de base; `None` sem resultado (consumo zero).
    fn base_of(self, x: f64) -> Option<f64> {
        let v = match self {
            Scale::Linear(f) => x * f,
            Scale::Affine { mul, div, offset } => (x * mul + offset * mul) / div,
            Scale::Inverse(_) if x == 0.0 => return None,
            Scale::Inverse(k) => k / x,
        };
        v.is_finite().then_some(v)
    }

    /// O valor da base nesta unidade.
    fn unit_of(self, b: f64) -> Option<f64> {
        let v = match self {
            Scale::Linear(f) => b / f,
            Scale::Affine { mul, div, offset } => (b * div - offset * mul) / mul,
            Scale::Inverse(_) if b == 0.0 => return None,
            Scale::Inverse(k) => k / b,
        };
        v.is_finite().then_some(v)
    }
}

#[derive(Debug, Clone)]
pub struct Unit {
    /// Identificador estável (vai para a configuração): "pol", "kmh", "f".
    pub id: &'static str,
    /// Nome na lista, no plural: "Polegadas".
    pub name: &'static str,
    /// Símbolo na tela: "pol", "km/h", "°F".
    pub symbol: &'static str,
    /// Outros nomes para a busca e as notas: "polegada", "polegadas".
    pub aliases: &'static [&'static str],
    /// Explicação no seletor: "2,42 ha", "1.024 bytes" (vazio se o nome basta).
    pub detail: &'static str,
    /// Símbolo ou nome que também é palavra comum ("sem", "volta", o "nós" dos nós):
    /// nas notas, só vale logo depois de um número. Símbolos de uma letra (h, s, m...)
    /// valem assim sempre.
    pub ambiguous: bool,
    pub scale: Scale,
}

impl Unit {
    /// Só vale como unidade logo depois de um número (ver `ambiguous`).
    fn needs_number(&self) -> bool {
        let mut chars = self.symbol.chars();
        let single_letter = chars.next().is_some_and(|c| c.is_ascii_alphabetic()) && chars.next().is_none();
        self.ambiguous || single_letter
    }
}

#[derive(Debug, Clone)]
pub struct Category {
    /// Identificador estável: "comprimento", "temperatura".
    pub id: &'static str,
    pub name: &'static str,
    /// Outros nomes para a busca ("peso" acha Massa).
    pub aliases: &'static [&'static str],
    pub units: Vec<Unit>,
    /// Par padrão (ids): de, para.
    pub default: (&'static str, &'static str),
}

// Definições (as únicas medidas digitadas; o resto sai da conta).
/// Polegada em metros (acordo internacional de 1959).
const POL: f64 = 0.0254;
/// Libra em quilogramas (1959).
const LB: f64 = 0.453_592_37;
/// Aceleração da gravidade padrão, m/s² (3ª CGPM, 1901).
const GN: f64 = 9.806_65;
/// Caloria internacional em joules (a do BTU).
const CAL_IT: f64 = 4.1868;
/// Caloria termoquímica em joules (a "caloria" da nutrição).
const CAL_TH: f64 = 4.184;
/// Galão imperial em m³ (Weights and Measures Act 1985).
const GAL_RU: f64 = 4.546_09e-3;
/// Velocidade da luz, m/s.
const C: f64 = 299_792_458.0;
/// Unidade astronômica em metros (UAI 2012).
const AU: f64 = 149_597_870_700.0;

fn unit(id: &'static str, name: &'static str, symbol: &'static str, aliases: &'static [&'static str], scale: Scale) -> Unit {
    Unit { id, name, symbol, aliases, detail: "", ambiguous: false, scale }
}

fn lin(id: &'static str, name: &'static str, symbol: &'static str, factor: f64, aliases: &'static [&'static str]) -> Unit {
    unit(id, name, symbol, aliases, Scale::Linear(factor))
}

/// A mesma unidade com uma explicação para o seletor.
fn detail(mut u: Unit, text: &'static str) -> Unit {
    u.detail = text;
    u
}

/// A mesma unidade, só valendo como unidade nas notas logo depois de um número.
fn ambiguous(mut u: Unit) -> Unit {
    u.ambiguous = true;
    u
}

/// As categorias em ordem alfabética (como no seletor); as unidades, da menor para a
/// maior e as de fora do SI depois.
fn table() -> Vec<Category> {
    let pi = std::f64::consts::PI;
    let pe = 12.0 * POL;
    let jarda = 3.0 * pe;
    let milha = 5280.0 * pe;
    let gal = 231.0 * POL * POL * POL;
    let btu = CAL_IT * 1000.0 * LB * 5.0 / 9.0;
    let ano = 365.0 * 86_400.0;
    fn category(id: &'static str, name: &'static str, aliases: &'static [&'static str], default: (&'static str, &'static str), units: Vec<Unit>) -> Category {
        Category { id, name, aliases, units, default }
    }
    vec![
        category("angulo", "Ângulo", &[], ("grau", "rad"), vec![
            lin("grau", "Graus", "°", pi / 180.0, &["grau", "deg"]),
            lin("rad", "Radianos", "rad", 1.0, &["radiano"]),
            lin("gon", "Grados", "gon", pi / 200.0, &["grado", "grad"]),
            lin("arcmin", "Minutos de arco", "′", pi / 10_800.0, &["minuto de arco", "arcmin"]),
            lin("arcsec", "Segundos de arco", "″", pi / 648_000.0, &["segundo de arco", "arcsec"]),
            ambiguous(lin("volta", "Voltas", "volta", 2.0 * pi, &["rotação", "rotações", "rev"])),
        ]),
        category("area", "Área", &[], ("alq_sp", "ha"), vec![
            lin("mm2", "Milímetros quadrados", "mm²", 1e-6, &["mm2", "milímetro quadrado"]),
            lin("cm2", "Centímetros quadrados", "cm²", 1e-4, &["cm2", "centímetro quadrado"]),
            lin("m2", "Metros quadrados", "m²", 1.0, &["m2", "metro quadrado"]),
            lin("km2", "Quilômetros quadrados", "km²", 1e6, &["km2", "quilômetro quadrado"]),
            lin("are", "Ares", "a", 100.0, &["are"]),
            lin("ha", "Hectares", "ha", 1e4, &["hectare"]),
            lin("pol2", "Polegadas quadradas", "pol²", POL * POL, &["pol2", "polegada quadrada"]),
            lin("ft2", "Pés quadrados", "ft²", pe * pe, &["ft2", "pé quadrado"]),
            lin("yd2", "Jardas quadradas", "yd²", jarda * jarda, &["yd2", "jarda quadrada"]),
            lin("ac", "Acres", "ac", 43_560.0 * pe * pe, &["acre"]),
            lin("mi2", "Milhas quadradas", "mi²", milha * milha, &["mi2", "milha quadrada"]),
            detail(lin("alq_sp", "Alqueires paulistas", "alq SP", 24_200.0, &["alqueire paulista", "alqueire", "alqueires"]), "2,42 ha"),
            detail(lin("alq_mg", "Alqueires mineiros", "alq MG", 48_400.0, &["alqueire mineiro"]), "4,84 ha"),
        ]),
        category("comprimento", "Comprimento", &[], ("pol", "cm"), vec![
            lin("nm", "Nanômetros", "nm", 1e-9, &["nanômetro"]),
            lin("um", "Micrômetros", "µm", 1e-6, &["micrômetro", "mícron"]),
            lin("mm", "Milímetros", "mm", 1e-3, &["milímetro"]),
            lin("cm", "Centímetros", "cm", 1e-2, &["centímetro"]),
            lin("m", "Metros", "m", 1.0, &["metro"]),
            lin("km", "Quilômetros", "km", 1e3, &["quilômetro"]),
            lin("pol", "Polegadas", "pol", POL, &["polegada"]),
            lin("ft", "Pés", "ft", pe, &["pé", "pes"]),
            lin("yd", "Jardas", "yd", jarda, &["jarda"]),
            lin("mi", "Milhas", "mi", milha, &["milha"]),
            lin("nmi", "Milhas náuticas", "M", 1852.0, &["milha náutica", "nmi"]),
            lin("angstrom", "Ångströms", "Å", 1e-10, &["ångström", "angstrom"]),
            lin("au", "Unidades astronômicas", "au", AU, &["unidade astronômica", "ua"]),
            lin("ly", "Anos-luz", "ano-luz", C * 365.25 * 86_400.0, &["anos-luz", "ly"]),
            lin("pc", "Parsecs", "pc", 648_000.0 / pi * AU, &["parsec"]),
        ]),
        category("consumo", "Consumo de combustível", &["combustível"], ("l100", "kml"), vec![
            lin("kml", "Quilômetros por litro", "km/L", 1.0, &["quilômetro por litro", "km/l"]),
            unit("l100", "Litros por 100 km", "L/100 km", &["litros por 100 km", "l/100km", "l/100 km"], Scale::Inverse(100.0)),
            lin("mpg", "Milhas por galão (EUA)", "mpg", milha / 1000.0 / (gal * 1000.0), &["milhas por galão", "milha por galão"]),
            lin("mpg_ru", "Milhas por galão (RU)", "mpg RU", milha / 1000.0 / (GAL_RU * 1000.0), &["milhas por galão imperial"]),
        ]),
        category("dados", "Dados", &["armazenamento"], ("gb", "mb"), vec![
            lin("bit", "Bits", "bit", 1.0, &["bits"]),
            lin("byte", "Bytes", "B", 8.0, &["byte"]),
            detail(lin("kb", "Quilobytes", "kB", 8e3, &["quilobyte"]), "1.000 bytes"),
            detail(lin("mb", "Megabytes", "MB", 8e6, &["megabyte"]), "1.000 kB"),
            detail(lin("gb", "Gigabytes", "GB", 8e9, &["gigabyte"]), "1.000 MB"),
            detail(lin("tb", "Terabytes", "TB", 8e12, &["terabyte"]), "1.000 GB"),
            detail(lin("pb", "Petabytes", "PB", 8e15, &["petabyte"]), "1.000 TB"),
            detail(lin("kib", "Kibibytes", "KiB", 8.0 * 1024.0, &["kibibyte"]), "1.024 bytes"),
            detail(lin("mib", "Mebibytes", "MiB", 8.0 * 1024f64.powi(2), &["mebibyte"]), "1.024 KiB"),
            detail(lin("gib", "Gibibytes", "GiB", 8.0 * 1024f64.powi(3), &["gibibyte"]), "1.024 MiB (o \"GB\" do Windows)"),
            detail(lin("tib", "Tebibytes", "TiB", 8.0 * 1024f64.powi(4), &["tebibyte"]), "1.024 GiB"),
            detail(lin("pib", "Pebibytes", "PiB", 8.0 * 1024f64.powi(5), &["pebibyte"]), "1.024 TiB"),
        ]),
        category("energia", "Energia", &[], ("kcal", "kj"), vec![
            lin("j", "Joules", "J", 1.0, &["joule"]),
            lin("kj", "Quilojoules", "kJ", 1e3, &["quilojoule"]),
            lin("mj", "Megajoules", "MJ", 1e6, &["megajoule"]),
            detail(lin("cal", "Calorias", "cal", CAL_TH, &["caloria"]), "4,184 J"),
            lin("kcal", "Quilocalorias", "kcal", 1000.0 * CAL_TH, &["quilocaloria"]),
            lin("wh", "Watts-hora", "Wh", 3600.0, &["watt-hora"]),
            lin("kwh", "Quilowatts-hora", "kWh", 3.6e6, &["quilowatt-hora"]),
            lin("ev", "Elétrons-volt", "eV", 1.602_176_634e-19, &["elétron-volt"]),
            lin("btu", "BTUs", "BTU", btu, &["btus"]),
        ]),
        category("forca", "Força", &[], ("kgf", "n"), vec![
            lin("n", "Newtons", "N", 1.0, &["newton"]),
            lin("kn", "Quilonewtons", "kN", 1e3, &["quilonewton"]),
            lin("dyn", "Dinas", "dyn", 1e-5, &["dina"]),
            lin("kgf", "Quilogramas-força", "kgf", GN, &["quilograma-força"]),
            lin("tf", "Toneladas-força", "tf", 1000.0 * GN, &["tonelada-força"]),
            lin("lbf", "Libras-força", "lbf", LB * GN, &["libra-força"]),
        ]),
        category("massa", "Massa", &["peso"], ("lb", "kg"), vec![
            lin("ug", "Microgramas", "µg", 1e-9, &["micrograma", "mcg"]),
            lin("mg", "Miligramas", "mg", 1e-6, &["miligrama"]),
            lin("g", "Gramas", "g", 1e-3, &["grama"]),
            lin("kg", "Quilogramas", "kg", 1.0, &["quilograma", "quilo", "quilos"]),
            lin("t", "Toneladas", "t", 1e3, &["tonelada"]),
            lin("ct", "Quilates", "ct", 2e-4, &["quilate"]),
            lin("oz", "Onças", "oz", LB / 16.0, &["onça"]),
            detail(lin("ozt", "Onças troy", "oz t", 480.0 * LB / 7000.0, &["onça troy", "ozt"]), "metais preciosos"),
            lin("lb", "Libras", "lb", LB, &["libra", "lbs"]),
            lin("st", "Stones", "st", 14.0 * LB, &["stone"]),
            detail(lin("arroba", "Arrobas", "@", 15.0, &["arroba"]), "15 kg"),
            detail(lin("tc", "Toneladas curtas (EUA)", "t curta", 2000.0 * LB, &["tonelada curta"]), "2.000 lb"),
            detail(lin("tl", "Toneladas longas (RU)", "t longa", 2240.0 * LB, &["tonelada longa"]), "2.240 lb"),
        ]),
        category("potencia", "Potência", &[], ("cv", "kw"), vec![
            lin("w", "Watts", "W", 1.0, &["watt"]),
            lin("kw", "Quilowatts", "kW", 1e3, &["quilowatt"]),
            lin("mw", "Megawatts", "MW", 1e6, &["megawatt"]),
            detail(lin("cv", "Cavalos-vapor", "cv", 75.0 * GN, &["cavalo-vapor", "cavalo", "cavalos"]), "75 kgf·m/s"),
            detail(lin("hp", "Horsepower", "hp", 550.0 * pe * LB * GN, &["horsepower"]), "550 ft·lbf/s"),
            lin("btuh", "BTUs por hora", "BTU/h", btu / 3600.0, &["btus por hora", "btuh"]),
            detail(lin("tr", "Toneladas de refrigeração", "TR", 12_000.0 * btu / 3600.0, &["tonelada de refrigeração"]), "12.000 BTU/h"),
        ]),
        category("pressao", "Pressão", &[], ("psi", "bar"), vec![
            lin("pa", "Pascals", "Pa", 1.0, &["pascal"]),
            lin("hpa", "Hectopascals", "hPa", 100.0, &["hectopascal", "mbar"]),
            lin("kpa", "Quilopascals", "kPa", 1e3, &["quilopascal"]),
            lin("mpa", "Megapascals", "MPa", 1e6, &["megapascal"]),
            lin("bar", "Bars", "bar", 1e5, &["bares"]),
            lin("atm", "Atmosferas", "atm", 101_325.0, &["atmosfera"]),
            lin("mmhg", "Milímetros de mercúrio", "mmHg", 13_595.1 * GN * 1e-3, &["milímetro de mercúrio"]),
            detail(lin("psi", "Libras por polegada quadrada", "psi", LB * GN / (POL * POL), &["libra por polegada quadrada"]), "\"libras\" do pneu"),
            lin("kgfcm2", "Quilogramas-força por cm²", "kgf/cm²", GN * 1e4, &["kgf/cm2"]),
            lin("mca", "Metros de coluna d'água", "mca", 1000.0 * GN, &["metro de coluna d'água"]),
        ]),
        category("temperatura", "Temperatura", &[], ("f", "c"), vec![
            lin("c", "Celsius", "°C", 1.0, &["celsius", "graus celsius", "ºC"]),
            unit("f", "Fahrenheit", "°F", &["fahrenheit", "graus fahrenheit", "ºF"], Scale::Affine { mul: 5.0, div: 9.0, offset: -32.0 }),
            unit("k", "Kelvin", "K", &["kelvin"], Scale::Affine { mul: 1.0, div: 1.0, offset: -273.15 }),
            unit("r", "Rankine", "°R", &["rankine", "ºR"], Scale::Affine { mul: 5.0, div: 9.0, offset: -491.67 }),
        ]),
        category("tempo", "Tempo", &[], ("h", "min"), vec![
            lin("ns", "Nanossegundos", "ns", 1e-9, &["nanossegundo"]),
            lin("us", "Microssegundos", "µs", 1e-6, &["microssegundo"]),
            lin("ms", "Milissegundos", "ms", 1e-3, &["milissegundo"]),
            lin("s", "Segundos", "s", 1.0, &["segundo", "seg"]),
            ambiguous(lin("min", "Minutos", "min", 60.0, &["minuto"])),
            lin("h", "Horas", "h", 3600.0, &["hora"]),
            lin("d", "Dias", "d", 86_400.0, &["dia"]),
            ambiguous(lin("sem", "Semanas", "sem", 7.0 * 86_400.0, &["semana"])),
            ambiguous(detail(lin("mes", "Meses", "mês", ano / 12.0, &["meses"]), "1/12 de ano")),
            ambiguous(detail(lin("ano", "Anos", "ano", ano, &["anos"]), "365 dias")),
            lin("dec", "Décadas", "década", 10.0 * ano, &["décadas"]),
            lin("sec", "Séculos", "século", 100.0 * ano, &["séculos"]),
        ]),
        category("transferencia", "Transferência de dados", &["velocidade da internet", "banda"], ("mbps", "mbs"), vec![
            lin("bps", "Bits por segundo", "bit/s", 1.0, &["bps"]),
            lin("kbps", "Quilobits por segundo", "kbit/s", 1e3, &["kbps"]),
            lin("mbps", "Megabits por segundo", "Mbit/s", 1e6, &["mbps", "mega", "megas"]),
            lin("gbps", "Gigabits por segundo", "Gbit/s", 1e9, &["gbps", "giga", "gigas"]),
            lin("bs", "Bytes por segundo", "B/s", 8.0, &["byte por segundo"]),
            lin("kbs", "Quilobytes por segundo", "kB/s", 8e3, &["quilobyte por segundo"]),
            lin("mbs", "Megabytes por segundo", "MB/s", 8e6, &["megabyte por segundo"]),
            lin("gbs", "Gigabytes por segundo", "GB/s", 8e9, &["gigabyte por segundo"]),
            lin("kibs", "Kibibytes por segundo", "KiB/s", 8.0 * 1024.0, &["kibibyte por segundo"]),
            lin("mibs", "Mebibytes por segundo", "MiB/s", 8.0 * 1024f64.powi(2), &["mebibyte por segundo"]),
        ]),
        category("velocidade", "Velocidade", &[], ("mph", "kmh"), vec![
            lin("mps", "Metros por segundo", "m/s", 1.0, &["metro por segundo"]),
            lin("kmh", "Quilômetros por hora", "km/h", 1000.0 / 3600.0, &["quilômetro por hora", "kph"]),
            lin("mph", "Milhas por hora", "mph", milha / 3600.0, &["milha por hora"]),
            ambiguous(lin("kn", "Nós", "kn", 1852.0 / 3600.0, &["nó"])),
            lin("fts", "Pés por segundo", "ft/s", pe, &["pé por segundo", "fps"]),
        ]),
        category("volume", "Volume", &[], ("gal", "l"), vec![
            lin("ml", "Mililitros", "mL", 1e-6, &["mililitro", "ml"]),
            lin("cm3", "Centímetros cúbicos", "cm³", 1e-6, &["cm3", "cc", "centímetro cúbico"]),
            lin("l", "Litros", "L", 1e-3, &["litro", "l"]),
            lin("m3", "Metros cúbicos", "m³", 1.0, &["m3", "metro cúbico"]),
            lin("pol3", "Polegadas cúbicas", "pol³", POL * POL * POL, &["pol3", "polegada cúbica"]),
            lin("ft3", "Pés cúbicos", "ft³", pe * pe * pe, &["ft3", "pé cúbico"]),
            detail(lin("gal", "Galões americanos", "gal", gal, &["galão", "galões", "galão americano"]), "231 pol³"),
            lin("pt", "Pintas americanas", "pt", gal / 8.0, &["pinta", "pintas"]),
            lin("floz", "Onças líquidas americanas", "fl oz", gal / 128.0, &["onça líquida", "onças líquidas", "floz"]),
            lin("gal_ru", "Galões imperiais", "gal RU", GAL_RU, &["galão imperial"]),
            detail(lin("bbl", "Barris de petróleo", "bbl", 42.0 * gal, &["barril", "barris"]), "42 galões"),
            lin("xic", "Xícaras (240 mL)", "xíc.", 240e-6, &["xícara", "xícaras"]),
            lin("csopa", "Colheres de sopa (15 mL)", "c. sopa", 15e-6, &["colher de sopa", "colheres de sopa", "colheres"]),
            lin("ccha", "Colheres de chá (5 mL)", "c. chá", 5e-6, &["colher de chá", "colheres de chá"]),
        ]),
    ]
}

static TABLE: LazyLock<Vec<Category>> = LazyLock::new(table);

/// As categorias, na ordem do seletor (alfabética).
pub fn categories() -> &'static [Category] {
    &TABLE
}

/// A unidade pelos índices (categoria, unidade).
pub fn get(category: usize, unit: usize) -> Option<&'static Unit> {
    categories().get(category)?.units.get(unit)
}

/// A categoria pelo identificador.
pub fn category_index(id: &str) -> Option<usize> {
    categories().iter().position(|c| c.id == id)
}

/// Categoria e unidade pelos identificadores (da configuração e dos menus).
pub fn find(category: &str, unit: &str) -> Option<(usize, usize)> {
    let c = category_index(category)?;
    let u = categories()[c].units.iter().position(|u| u.id == unit)?;
    Some((c, u))
}

/// O par padrão de uma categoria (índices de, para).
pub fn default_pair(category: usize) -> Option<(usize, usize)> {
    let cat = categories().get(category)?;
    let (_, from) = find(cat.id, cat.default.0)?;
    let (_, to) = find(cat.id, cat.default.1)?;
    Some((from, to))
}

/// `x` em `from` convertido para `to`; `None` sem resultado (consumo zero, número que
/// não cabe no `f64`).
pub fn convert(x: f64, from: &Unit, to: &Unit) -> Option<f64> {
    if !x.is_finite() {
        return None;
    }
    let v = match (from.scale, to.scale) {
        // Duas lineares: a razão dos fatores direto (um arredondamento a menos).
        (Scale::Linear(a), Scale::Linear(b)) => x * (a / b),
        _ => to.scale.unit_of(from.scale.base_of(x)?)?,
    };
    v.is_finite().then_some(if v == 0.0 { 0.0 } else { v })
}

/// O texto todo da unidade para a busca: nome, símbolo, apelidos e detalhe, sem acento.
fn haystack(cat: &Category, u: &Unit) -> String {
    let mut parts = vec![u.name, u.symbol, u.detail, cat.name];
    parts.extend(u.aliases);
    parts.extend(cat.aliases);
    fold(&parts.join(" "))
}

/// Unidades que casam com a busca (todas as palavras), da melhor para a pior: símbolo
/// ou apelido igual, com maiúsculas ("MB" é megabyte, não o "mbar" do hectopascal);
/// nome, símbolo ou apelido igual sem acento; algum que começa com a busca; e o resto
/// (no detalhe ou na categoria: "peso" acha a Massa).
pub fn search(query: &str) -> Vec<(usize, usize)> {
    let raw = query.trim();
    let q = fold(raw);
    let words: Vec<&str> = q.split_whitespace().collect();
    if words.is_empty() {
        return Vec::new();
    }
    let mut found: Vec<(u8, usize, usize)> = Vec::new();
    for (ci, cat) in categories().iter().enumerate() {
        for (ui, u) in cat.units.iter().enumerate() {
            let hay = haystack(cat, u);
            if !words.iter().all(|w| hay.contains(w)) {
                continue;
            }
            let keys = || std::iter::once(u.name).chain(std::iter::once(u.symbol)).chain(u.aliases.iter().copied());
            let rank = if u.symbol == raw || u.aliases.contains(&raw) {
                0
            } else if keys().any(|k| fold(k) == q) {
                1
            } else if keys().any(|k| fold(k).starts_with(&q)) {
                2
            } else {
                3
            };
            found.push((rank, ci, ui));
        }
    }
    found.sort_by_key(|&(rank, ci, ui)| (rank, ci, ui));
    found.into_iter().map(|(_, ci, ui)| (ci, ui)).collect()
}

/// Unidade pelo texto (para as notas): primeiro o símbolo exato, com maiúsculas (m e M,
/// b e B, min e mi), depois o nome e os apelidos sem acento nem maiúscula. As unidades
/// que também são palavra comum (ver `Unit::ambiguous`) só valem logo depois de um
/// número (`after_number`).
pub fn lookup(text: &str, after_number: bool) -> Option<(usize, usize)> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let all = || {
        categories()
            .iter()
            .enumerate()
            .flat_map(|(ci, c)| c.units.iter().enumerate().map(move |(ui, u)| (ci, ui, u)))
            .filter(move |(_, _, u)| after_number || !u.needs_number())
    };
    if let Some((ci, ui, _)) = all().find(|(_, _, u)| u.symbol == text) {
        return Some((ci, ui));
    }
    let folded = fold(text);
    all().find(|(_, _, u)| fold(u.name) == folded || u.aliases.iter().any(|a| fold(a) == folded)).map(|(ci, ui, _)| (ci, ui))
}

/// A unidade mais longa escrita a partir do começo de `text` ("km/h em m/s" acha
/// "km/h", não "km"), terminando numa fronteira de palavra. Devolve (categoria,
/// unidade, bytes usados). Símbolos casam exatos; nomes e apelidos, sem acento nem
/// maiúscula.
pub fn longest_at(text: &str, after_number: bool) -> Option<(usize, usize, usize)> {
    let mut best: Option<(usize, usize, usize)> = None;
    for (ci, cat) in categories().iter().enumerate() {
        for (ui, u) in cat.units.iter().enumerate() {
            if !after_number && u.needs_number() {
                continue;
            }
            let names = std::iter::once(u.name).chain(u.aliases.iter().copied());
            let candidates = std::iter::once((u.symbol, true)).chain(names.map(|n| (n, false)));
            for (candidate, exact) in candidates {
                if let Some(len) = prefix_len(text, candidate, exact)
                    && best.is_none_or(|(_, _, b)| len > b)
                {
                    best = Some((ci, ui, len));
                }
            }
        }
    }
    best
}

/// Quantos bytes de `text` o `candidate` ocupa, se `text` começa com ele (`exact`: com
/// maiúsculas e acentos; senão, sem) e depois dele vem uma fronteira (fim, espaço ou
/// pontuação; não uma letra nem um algarismo).
fn prefix_len(text: &str, candidate: &str, exact: bool) -> Option<usize> {
    let mut chars = text.char_indices();
    for want in candidate.chars() {
        let (_, got) = chars.next()?;
        let same = if exact { got == want } else { fold(&got.to_string()) == fold(&want.to_string()) };
        if !same {
            return None;
        }
    }
    let end = chars.next();
    match end {
        Some((_, c)) if c.is_alphanumeric() => None,
        Some((i, _)) => Some(i),
        None => Some(text.len()),
    }
}

/// Índice da categoria Moeda no conversor: logo depois das categorias físicas (as
/// unidades dela são as moedas de `currency::currencies()`, na ordem de lá).
pub fn money() -> usize {
    categories().len()
}

/// Identificador da Moeda nos pares lembrados.
pub const MONEY_ID: &str = "moeda";

/// (categoria, unidade) de um par lembrado, também na Moeda (pelo código: "USD").
fn find_pair_unit(category: &str, unit: &str) -> Option<(usize, usize)> {
    if category == MONEY_ID {
        let code = unit.to_uppercase();
        return super::currency::currencies().iter().position(|c| c.code == code).map(|i| (money(), i));
    }
    find(category, unit)
}

/// Pares de unidades lembrados por categoria, do mais recente ao mais antigo; como
/// texto para a configuração: "temperatura=f>c;comprimento=pol>cm;moeda=USD>BRL".
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Pairs(Vec<(usize, usize, usize)>);

impl Pairs {
    /// Lê o texto da configuração (pares com identificador desconhecido ficam de fora).
    pub fn from_text(text: &str) -> Self {
        let mut pairs = Vec::new();
        for part in text.split(';') {
            let Some((cat, units)) = part.split_once('=') else {
                continue;
            };
            let Some((from, to)) = units.split_once('>') else {
                continue;
            };
            if let (Some((c, a)), Some((_, b))) = (find_pair_unit(cat.trim(), from.trim()), find_pair_unit(cat.trim(), to.trim()))
                && !pairs.iter().any(|(pc, _, _)| *pc == c)
            {
                pairs.push((c, a, b));
            }
        }
        Pairs(pairs)
    }

    pub fn to_text(&self) -> String {
        let all = categories();
        let money = super::currency::currencies();
        self.0
            .iter()
            .map(|&(c, a, b)| {
                if c == self::money() {
                    format!("{MONEY_ID}={}>{}", money[a].code, money[b].code)
                } else {
                    format!("{}={}>{}", all[c].id, all[c].units[a].id, all[c].units[b].id)
                }
            })
            .collect::<Vec<_>>()
            .join(";")
    }

    /// O par lembrado de uma categoria.
    pub fn get(&self, category: usize) -> Option<(usize, usize)> {
        self.0.iter().find(|(c, _, _)| *c == category).map(|&(_, a, b)| (a, b))
    }

    /// Usou o par: vai para a frente.
    pub fn touch(&mut self, category: usize, from: usize, to: usize) {
        self.0.retain(|(c, _, _)| *c != category);
        self.0.insert(0, (category, from, to));
    }

    /// Os `n` pares mais recentes: (categoria, de, para).
    pub fn recent(&self, n: usize) -> Vec<(usize, usize, usize)> {
        self.0.iter().take(n).copied().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calculator::format::Style;

    fn unit(cat: &str, id: &str) -> &'static Unit {
        let (c, u) = find(cat, id).unwrap_or_else(|| panic!("{cat}/{id}"));
        &categories()[c].units[u]
    }

    /// Conversão como aparece na tela (12 algarismos, vírgula, ponto de milhar).
    fn shown(x: f64, cat: &str, from: &str, to: &str) -> String {
        Style::default().value(convert(x, unit(cat, from), unit(cat, to)).expect("conversão"))
    }

    #[test]
    fn tabela_consistente() {
        assert_eq!(categories().len(), 15);
        let total: usize = categories().iter().map(|c| c.units.len()).sum();
        assert_eq!(total, 140);
        // Ordem alfabética, como no seletor.
        let names: Vec<String> = categories().iter().map(|c| fold(c.name)).collect();
        assert!(names.windows(2).all(|w| w[0] < w[1]), "{names:?}");
        for cat in categories() {
            assert!(cat.units.len() >= 2, "{}", cat.id);
            assert!(cat.id.is_ascii());
            let (from, to) = cat.default;
            assert!(find(cat.id, from).is_some() && find(cat.id, to).is_some(), "par padrão de {}", cat.id);
            for (i, u) in cat.units.iter().enumerate() {
                assert!(u.id.is_ascii(), "{}", u.id);
                for other in &cat.units[i + 1..] {
                    assert_ne!(u.id, other.id, "id repetido em {}", cat.id);
                    assert_ne!(u.symbol, other.symbol, "símbolo repetido em {}", cat.id);
                }
                let ok = match u.scale {
                    Scale::Linear(f) | Scale::Inverse(f) => f.is_finite() && f > 0.0,
                    Scale::Affine { mul, div, offset } => mul > 0.0 && div > 0.0 && offset.is_finite(),
                };
                assert!(ok, "{}", u.id);
            }
        }
    }

    #[test]
    fn confere_com_o_nist() {
        // Tabela B.8 do NIST SP 811 (7 algarismos), contra os fatores tirados das definições.
        let nist = [
            ("comprimento", "ly", 9.460_73e15),
            ("comprimento", "pc", 3.085_678e16),
            ("area", "yd2", 8.361_274e-1),
            ("area", "mi2", 2.589_988e6),
            ("volume", "pol3", 1.638_706e-5),
            ("volume", "ft3", 2.831_685e-2),
            ("volume", "gal", 3.785_412e-3),
            ("volume", "floz", 2.957_353e-5),
            ("volume", "bbl", 1.589_873e-1),
            ("massa", "oz", 2.834_952e-2),
            ("massa", "tc", 9.071_847e2),
            ("massa", "tl", 1.016_047e3),
            ("velocidade", "kn", 5.144_444e-1),
            ("energia", "btu", 1.055_056e3),
            ("potencia", "cv", 7.354_988e2),
            ("potencia", "hp", 7.456_999e2),
            ("potencia", "btuh", 2.930_711e-1),
            ("potencia", "tr", 3.516_853e3),
            ("pressao", "mmhg", 1.333_224e2),
            ("pressao", "psi", 6.894_757e3),
            ("forca", "lbf", 4.448_222),
            ("consumo", "mpg", 4.251_437e-1),
        ];
        for (cat, id, want) in nist {
            let Scale::Linear(f) = unit(cat, id).scale else { panic!("{id}") };
            assert!((f / want - 1.0).abs() < 6e-7, "{id}: {f} contra {want}");
        }
    }

    #[test]
    fn na_tela() {
        assert_eq!(shown(1.0, "comprimento", "pol", "cm"), "2,54");
        assert_eq!(shown(1.0, "comprimento", "mi", "cm"), "160.934,4");
        assert_eq!(shown(7.0, "comprimento", "km", "mi"), "4,34959834566");
        assert_eq!(shown(1.0, "massa", "lb", "g"), "453,59237");
        assert_eq!(shown(1.0, "volume", "gal", "l"), "3,785411784");
        assert_eq!(shown(1.0, "area", "ac", "ha"), "0,40468564224");
        assert_eq!(shown(1.0, "energia", "btu", "j"), "1.055,05585262");
        assert_eq!(shown(100.0, "temperatura", "f", "c"), "37,7777777778");
        assert_eq!(shown(100.0, "temperatura", "c", "f"), "212");
        assert_eq!(shown(-40.0, "temperatura", "c", "f"), "-40");
        assert_eq!(shown(0.0, "temperatura", "k", "c"), "-273,15");
        assert_eq!(shown(30.0, "consumo", "mpg", "kml"), "12,7543112229");
        assert_eq!(shown(1.0, "dados", "tb", "gib"), "931,322574615");
        assert_eq!(shown(300.0, "transferencia", "mbps", "mbs"), "37,5");
        assert_eq!(shown(32.0, "pressao", "psi", "bar"), "2,20632233381");
        assert_eq!(shown(120.0, "pressao", "mmhg", "kpa"), "15,9986864898");
        assert_eq!(shown(12_000.0, "potencia", "btuh", "w"), "3.516,85284207");
        assert_eq!(shown(1.0, "angulo", "rad", "grau"), "57,2957795131");
        assert_eq!(shown(90.0, "angulo", "grau", "gon"), "100");
        assert_eq!(shown(2.0, "tempo", "h", "min"), "120");
    }

    #[test]
    fn temperatura_exata() {
        // Pontos fixos sem o resto do arredondamento (antes, 32 °F dava 5,7e-14 °C).
        let t = |x: f64, from: &str, to: &str| convert(x, unit("temperatura", from), unit("temperatura", to)).unwrap();
        assert_eq!(t(32.0, "f", "c"), 0.0);
        assert_eq!(t(491.67, "r", "c"), 0.0);
        assert_eq!(t(0.0, "c", "f"), 32.0);
        assert_eq!(t(212.0, "f", "c"), 100.0);
        assert_eq!(t(33.8, "f", "c"), 1.0);
        assert_eq!(t(273.15, "k", "c"), 0.0);
        assert_eq!(t(0.0, "c", "k"), 273.15);
    }

    #[test]
    fn consumo() {
        assert_eq!(shown(10.0, "consumo", "l100", "kml"), "10");
        assert_eq!(shown(8.0, "consumo", "l100", "kml"), "12,5");
        assert_eq!(convert(0.0, unit("consumo", "l100"), unit("consumo", "kml")), None);
        assert_eq!(convert(0.0, unit("consumo", "kml"), unit("consumo", "l100")), None);
        assert_eq!(convert(f64::INFINITY, unit("comprimento", "m"), unit("comprimento", "cm")), None);
    }

    #[test]
    fn ida_e_volta() {
        for cat in categories() {
            for a in &cat.units {
                for b in &cat.units {
                    for x in [1.0, 0.5, 123.456, 1e-3, 1e6, -7.0] {
                        let Some(there) = convert(x, a, b) else { continue };
                        let back = convert(there, b, a).expect("volta");
                        assert!((back - x).abs() <= 1e-12 * x.abs().max(1.0), "{} {} → {} → {back}", cat.id, a.id, b.id);
                    }
                }
            }
        }
    }

    #[test]
    fn busca() {
        let names = |q: &str| search(q).iter().map(|&(c, u)| categories()[c].units[u].id).collect::<Vec<_>>();
        assert_eq!(names("quilometro").first(), Some(&"km"));
        assert_eq!(names("km/h").first(), Some(&"kmh"));
        assert_eq!(&names("pol")[..3], ["pol", "pol2", "pol3"]);
        assert_eq!(names("temperatura"), ["c", "f", "k", "r"]);
        assert_eq!(&names("btu")[..2], ["btu", "btuh"]);
        assert!(search("peso").iter().all(|&(c, _)| categories()[c].id == "massa"));
        assert!(!search("peso").is_empty());
        assert_eq!(names("QUILÔMETROS POR HORA"), ["kmh"]);
        assert!(names("xyz").is_empty());
        // O símbolo ou o apelido igual vem primeiro (o Enter do seletor escolhe ele).
        for (q, first) in [("MB", "mb"), ("quilo", "kg"), ("ano", "ano"), ("onça", "oz"), ("mega", "mbps"), ("giga", "gbps"), ("TR", "tr"), ("m2", "m2"), ("watt", "w"), ("milha", "mi"), ("gal", "gal")] {
            assert_eq!(names(q).first(), Some(&first), "{q}");
        }
    }

    #[test]
    fn reconhece_no_texto() {
        let id = |r: Option<(usize, usize)>| r.map(|(c, u)| categories()[c].units[u].id);
        assert_eq!(id(lookup("km", true)), Some("km"));
        assert_eq!(id(lookup("quilômetros", true)), Some("km"));
        assert_eq!(id(lookup("Quilometros", true)), Some("km"));
        assert_eq!(id(lookup("M", true)), Some("nmi"));
        assert_eq!(id(lookup("m", true)), Some("m"));
        assert_eq!(id(lookup("B", true)), Some("byte"));
        assert_eq!(id(lookup("bit", true)), Some("bit"));
        assert_eq!(id(lookup("min", true)), Some("min"));
        assert_eq!(id(lookup("mi", true)), Some("mi"));
        assert_eq!(id(lookup("a", true)), Some("are"));
        assert_eq!(id(lookup("galões", true)), Some("gal"));
        assert_eq!(id(lookup("quilos", true)), Some("kg"));
        assert_eq!(id(lookup("cavalos", true)), Some("cv"));
        // Palavras comuns só são unidade depois de um número.
        for word in ["a", "h", "no", "nos", "sem", "volta", "ano", "min", "M"] {
            assert_eq!(lookup(word, false), None, "{word}");
        }
        assert_eq!(id(lookup("nós", true)), Some("kn"));
        let longest = |t: &str| longest_at(t, true).map(|(c, u, len)| (categories()[c].units[u].id, len));
        assert_eq!(longest("km/h em m/s"), Some(("kmh", 4)));
        assert_eq!(longest("km / 2 h"), Some(("km", 2)));
        assert_eq!(longest("L/100 km"), Some(("l100", 8)));
        assert_eq!(longest("m²"), Some(("m2", 3)));
        assert_eq!(longest("mmHg."), Some(("mmhg", 4)));
        assert_eq!(longest("milhas por galão"), Some(("mpg", "milhas por galão".len())));
        assert_eq!(longest("kmx"), None);
        assert_eq!(longest_at("sem açúcar", false), None);
    }

    #[test]
    fn pares_lembrados_com_moeda() {
        let p = Pairs::from_text("moeda=usd>BRL;comprimento=pol>cm;moeda=EUR>BRL;moeda=XXX>BRL");
        let usd = super::super::currency::currencies().iter().position(|c| c.code == "USD").unwrap();
        let brl = super::super::currency::currencies().iter().position(|c| c.code == "BRL").unwrap();
        assert_eq!(p.get(money()), Some((usd, brl)));
        assert_eq!(p.to_text(), "moeda=USD>BRL;comprimento=pol>cm");
    }

    #[test]
    fn pares_lembrados() {
        let mut p = Pairs::from_text("temperatura=f>c;comprimento=pol>cm;xyz=a>b;massa=lb>zzz");
        assert_eq!(p.to_text(), "temperatura=f>c;comprimento=pol>cm");
        let length = category_index("comprimento").unwrap();
        let (_, km) = find("comprimento", "km").unwrap();
        let (_, mi) = find("comprimento", "mi").unwrap();
        p.touch(length, km, mi);
        assert_eq!(p.to_text(), "comprimento=km>mi;temperatura=f>c");
        assert_eq!(p.get(length), Some((km, mi)));
        assert_eq!(p.recent(1).len(), 1);
        assert_eq!(Pairs::from_text(&p.to_text()), p);
        assert_eq!(default_pair(category_index("temperatura").unwrap()), Some((1, 0)));
    }
}
