//! Conversor de unidades da Calculadora (E3b, etapa 2): a categoria, as duas unidades
//! (uma em cada linha do visor) e qual linha é o número da tela. As contas ficam em
//! `mide::calculator::units`; aqui ficam as regras de escolha, os textos e os lugares no
//! visor.

use mide::{
    calculator::{
        currency::{self, Currency, Rates},
        format::Style,
        units::{self, Pairs},
    },
    kit::{ListPicker, PickGroup, PickItem},
    ui::{
        draw::{fill_rounded, rgba},
        text::{FontWeight, Text},
    },
};
use tiny_skia::Pixmap;

use crate::{fit_left, PAD, SCIENCE};

/// Altura a mais do visor com o conversor (pontos).
pub const CONVERT_H: u32 = 64;
/// Faixa dos botões da janela no alto do visor (as linhas do conversor ficam abaixo).
pub const TITLE_H: f32 = 36.0;

/// A categoria é a Moeda (as unidades são as moedas, convertidas pelas cotações).
pub fn is_money(category: usize) -> bool {
    category == units::money()
}

/// Nome da categoria ("Comprimento", "Moeda").
pub fn category_name(category: usize) -> &'static str {
    if is_money(category) { "Moeda" } else { units::categories()[category].name }
}

/// Identificador da categoria no log e no config ("comprimento", "moeda").
pub fn category_id(category: usize) -> &'static str {
    if is_money(category) { units::MONEY_ID } else { units::categories()[category].id }
}

/// Todas as categorias, na ordem do seletor e do menu: a Moeda primeiro.
pub fn category_order() -> Vec<usize> {
    std::iter::once(units::money()).chain(0..units::money()).collect()
}

fn money(unit: usize) -> &'static Currency {
    &currency::currencies()[unit]
}

/// (nome, símbolo, identificador) de uma unidade; numa moeda, o símbolo é o código
/// ("USD": "$" sozinho seria ambíguo na tela).
pub fn unit_label(category: usize, unit: usize) -> (&'static str, &'static str, &'static str) {
    if is_money(category) {
        let c = money(unit);
        (c.name, c.code, c.code)
    } else {
        let u = &units::categories()[category].units[unit];
        (u.name, u.symbol, u.id)
    }
}

/// O par padrão de uma categoria (USD → BRL na Moeda).
pub fn default_pair(category: usize) -> Option<(usize, usize)> {
    if is_money(category) {
        let at = |code: &str| currency::currencies().iter().position(|c| c.code == code);
        return Some((at("USD")?, at("BRL")?));
    }
    units::default_pair(category)
}

/// As unidades do conversor: a categoria, a unidade de cada linha (0 em cima) e qual
/// linha é o número da tela (a outra mostra a conversão).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Conv {
    pub category: usize,
    pub unit: [usize; 2],
    pub active: usize,
}

impl Conv {
    /// O par mais recente lembrado; sem nenhum, o padrão do Comprimento (pol → cm).
    pub fn from_pairs(pairs: &Pairs) -> Self {
        let first = pairs.recent(1).first().copied().or_else(|| {
            let c = units::category_index("comprimento")?;
            let (a, b) = units::default_pair(c)?;
            Some((c, a, b))
        });
        let (category, a, b) = first.unwrap_or((0, 0, 1));
        Conv { category, unit: [a, b], active: 0 }
    }

    /// Nome, símbolo e identificador da unidade de uma linha.
    pub fn label(&self, row: usize) -> (&'static str, &'static str, &'static str) {
        unit_label(self.category, self.unit[row])
    }

    /// Símbolo da unidade de uma linha ("cm", "USD").
    pub fn symbol(&self, row: usize) -> &'static str {
        self.label(row).1
    }

    /// Casas decimais da moeda da linha (só na Moeda).
    pub fn decimals(&self, row: usize) -> Option<u8> {
        is_money(self.category).then(|| money(self.unit[row]).decimals)
    }

    /// A linha que não é a ativa.
    pub fn other(&self) -> usize {
        1 - self.active
    }

    /// O número da linha ativa convertido para a outra (`None`: sem resultado; na
    /// Moeda, também sem a cotação das duas).
    pub fn convert(&self, x: f64, rates: Option<&Rates>) -> Option<f64> {
        let (from, to) = (self.unit[self.active], self.unit[self.other()]);
        if is_money(self.category) {
            return rates?.convert(x, money(from).code, money(to).code);
        }
        let all = &units::categories()[self.category].units;
        units::convert(x, &all[from], &all[to])
    }

    /// O formato da linha: na Moeda, com "Casas decimais" em Automático, as casas da
    /// moeda (2 no real, 0 no iene).
    pub fn style(&self, row: usize, style: Style) -> Style {
        match self.decimals(row) {
            Some(d) if style.decimals.is_none() => Style { decimals: Some(d), ..style },
            _ => style,
        }
    }

    /// "12,5 pol = 31,75 cm" (Copiar a conversão e o log).
    pub fn text(&self, x: f64, style: Style, rates: Option<&Rates>) -> String {
        let other = self.convert(x, rates).map_or_else(|| "—".to_string(), |v| self.style(self.other(), style).value(v));
        format!("{} {} = {} {}", style.value(x), self.symbol(self.active), other, self.symbol(self.other()))
    }

    /// "12,5 pol → cm": a conversão como conta, no histórico.
    pub fn history(&self, x: f64, style: Style) -> String {
        format!("{} {} → {}", style.value(x), self.symbol(self.active), self.symbol(self.other()))
    }

    /// As unidades trocam de linha; a linha ativa fica (o número passa a ser da outra
    /// unidade: "100 °F = 37,8 °C" vira "100 °C = 212 °F").
    pub fn swapped(self) -> Conv {
        Conv { unit: [self.unit[1], self.unit[0]], ..self }
    }

    /// Escolheu a unidade `unit` da categoria `category` para a linha `row`. Na mesma
    /// categoria, só aquela linha muda (se for a unidade da outra, as duas trocam). Em
    /// outra, a escolhida fica na linha e a outra recebe o par lembrado dali (ou o
    /// padrão). O número não muda.
    pub fn choose(self, pairs: &Pairs, row: usize, category: usize, unit: usize) -> Conv {
        if category == self.category {
            return if self.unit[1 - row] == unit { self.swapped() } else { Conv { unit: with(self.unit, row, unit), ..self } };
        }
        let (a, b) = pairs.get(category).or_else(|| default_pair(category)).unwrap_or((0, 1));
        let mut pair = with([a, b], row, unit);
        if pair[1 - row] == unit {
            pair[1 - row] = [a, b][row];
        }
        Conv { category, unit: pair, active: self.active }
    }

    /// O seletor de unidades para a linha `row`: todas as categorias (a Moeda primeiro,
    /// na ordem de `category_order`), com a unidade da linha marcada. O grupo escolhido
    /// volta para a categoria por `category_order()[grupo]`.
    pub fn picker(&self, row: usize) -> ListPicker {
        let order = category_order();
        let title = if row == self.active { "Converter de" } else { "Converter para" };
        let group = order.iter().position(|&c| c == self.category).unwrap_or(0);
        ListPicker::new(title, "Todas as categorias", "Buscar unidade", picker_groups(), (group, self.unit[row]))
    }
}

/// Os grupos do seletor de unidades, na ordem de `category_order` (a Moeda primeiro).
fn picker_groups() -> Vec<PickGroup> {
    category_order()
            .iter()
            .map(|&c| {
                if is_money(c) {
                    let items = currency::currencies()
                        .iter()
                        .map(|m| {
                            let keys: Vec<&str> = [m.code, m.symbol(), m.plural, "moeda", "dinheiro", "câmbio"].into_iter().chain(m.aliases.iter().copied()).collect();
                            PickItem::new(m.name, m.code, &keys)
                        })
                        .collect();
                    return PickGroup { name: "Moeda".into(), items };
                }
                let cat = &units::categories()[c];
                PickGroup {
                    name: cat.name.into(),
                    items: cat
                        .units
                        .iter()
                        .map(|u| {
                            // O símbolo e os apelidos da unidade são as chaves exatas da busca
                            // ("MB"); os da categoria ("peso" na Massa) só entram na busca,
                            // sem passar à frente ("peso" acha antes o Peso argentino).
                            let detail = if u.detail.is_empty() { u.symbol.to_string() } else { format!("{} · {}", u.symbol, u.detail) };
                            let keys: Vec<&str> = std::iter::once(u.symbol).chain(u.aliases.iter().copied()).collect();
                            let mut item = PickItem::new(u.name, &detail, &keys);
                            for alias in cat.aliases {
                                item.search.push(' ');
                                item.search.push_str(&mide::apps::fold(alias));
                            }
                            item
                        })
                        .collect(),
                }
            })
            .collect()
}

/// O que o visor do conversor mostra num quadro.
pub struct Shown {
    /// Nome e símbolo da unidade de cada linha (o símbolo quando o nome não cabe).
    pub names: [String; 2],
    pub symbols: [String; 2],
    /// O número de cada linha: a ativa como na tela; a outra, a conversão ou "—".
    pub numbers: [String; 2],
    pub active: usize,
    pub category: String,
}

/// Menor letra do número de uma linha do conversor.
const MIN_SIZE: f32 = 12.0;

/// O número no maior tamanho que cabe em `room`, de `max` até `MIN_SIZE`; se nem assim
/// cabe, cortado pela esquerda com "…".
fn fit_number(text: &mut Text, number: &str, room: f32, max: f32) -> (String, f32) {
    let mut size = max;
    while size > MIN_SIZE && text.measure(number, size, FontWeight::Regular) > room {
        size -= 2.0;
    }
    (fit_left(text, number, size, room), size)
}

/// Desenha as duas linhas do conversor (a ativa branca e maior), o ⇅ e a categoria;
/// `typing`: o ⌫ está na linha ativa (o número abre espaço para ele). O nome da unidade
/// fica inteiro se sobra lugar para o número; senão, o símbolo. Devolve a largura do
/// nome de cada linha (o clique e o ⌫ do próximo quadro seguem ela).
pub fn draw(pixmap: &mut Pixmap, text: &mut Text, (ox, oy): (f32, f32), w: f32, shown: &Shown, typing: bool) -> [f32; 2] {
    let small = [180, 180, 188, 255];
    let mut widths = [0.0; 2];
    for (row, width) in widths.iter_mut().enumerate() {
        let (rx, ry, rw, rh) = row_rect(w, row);
        let active = row == shown.active;
        let erase = if active && typing { 34.0 } else { 0.0 };
        let max = if active { 40.0 } else { 28.0 };
        let room = |label_w: f32| rw - label_w - 16.0 - erase;
        let mut label = format!("{} ⌄", shown.names[row]);
        let mut label_w = text.measure(&label, 13.0, FontWeight::Regular);
        let number_w = text.measure(&shown.numbers[row], 16.0, FontWeight::Regular);
        if label_w > rw * 0.5 || number_w > room(label_w) {
            label = format!("{} ⌄", shown.symbols[row]);
            label_w = text.measure(&label, 13.0, FontWeight::Regular);
        }
        *width = label_w;
        text.draw(pixmap, &label, ox + rx + 4.0, oy + ry + (rh - 13.0 * 1.25) / 2.0, 13.0, FontWeight::Regular, small);
        let (number, size) = fit_number(text, &shown.numbers[row], room(label_w), max);
        let nw = text.measure(&number, size, FontWeight::Regular);
        let color = if active { [255, 255, 255, 255] } else { small };
        text.draw(pixmap, &number, ox + rx + rw - 4.0 - nw, oy + ry + (rh - size * 1.25) / 2.0, size, FontWeight::Regular, color);
    }
    let (sx, sy, sw, sh) = swap_rect();
    fill_rounded(pixmap, ox + sx, oy + sy, sw, sh, sh / 2.0, rgba(SCIENCE[0], SCIENCE[1], SCIENCE[2], SCIENCE[3]));
    let lw = text.measure("⇅", 13.0, FontWeight::Regular);
    text.draw(pixmap, "⇅", ox + sx + (sw - lw) / 2.0, oy + sy + (sh - 13.0 * 1.25) / 2.0, 13.0, FontWeight::Regular, [235, 235, 240, 255]);
    let cw = text.measure(&shown.category, 11.0, FontWeight::Regular);
    text.draw(pixmap, &shown.category, ox + w - PAD - 4.0 - cw, oy + sy + (sh - 11.0 * 1.25) / 2.0, 11.0, FontWeight::Regular, small);
    widths
}

/// Os pares de Converter → Recentes: o de agora e os lembrados, até 5 (a ação
/// "conv-recente:N" usa o N desta lista).
pub fn recent_pairs(pairs: &Pairs, conv: &Conv) -> Vec<(usize, usize, usize)> {
    let mut list = vec![(conv.category, conv.unit[0], conv.unit[1])];
    for pair in pairs.recent(6) {
        if !list.contains(&pair) {
            list.push(pair);
        }
    }
    list.truncate(5);
    list
}

/// Um par como aparece no menu: "°F → °C".
pub fn pair_label((category, a, b): (usize, usize, usize)) -> String {
    format!("{} → {}", unit_label(category, a).1, unit_label(category, b).1)
}

/// O par com `unit` na posição `row`.
fn with(mut pair: [usize; 2], row: usize, unit: usize) -> [usize; 2] {
    pair[row] = unit;
    pair
}

/// Lugar de uma linha do visor (x, y, largura, altura), com o visor `w` de largura.
pub fn row_rect(w: f32, row: usize) -> (f32, f32, f32, f32) {
    (PAD, TITLE_H + 14.0 + row as f32 * 72.0, w - 2.0 * PAD, 50.0)
}

/// A parte da linha com o nome da unidade (o clique abre o seletor), com o nome de
/// `label_w` de largura.
pub fn name_rect(w: f32, row: usize, label_w: f32) -> (f32, f32, f32, f32) {
    let (x, y, _, h) = row_rect(w, row);
    (x, y, (label_w + 8.0).max(40.0), h)
}

/// O botão ⇅, entre as linhas.
pub fn swap_rect() -> (f32, f32, f32, f32) {
    (PAD, TITLE_H + 65.0, 30.0, 20.0)
}

/// O ⌫ do toque, na linha ativa, logo depois do nome da unidade.
pub fn erase_rect(w: f32, active: usize, label_w: f32) -> (f32, f32, f32, f32) {
    let (x, y, nw, h) = name_rect(w, active, label_w);
    (x + nw + 2.0, y + (h - 24.0) / 2.0, 30.0, 24.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(c: &Conv) -> (&'static str, [&'static str; 2]) {
        (category_id(c.category), [c.label(0).2, c.label(1).2])
    }

    /// Trecho do boletim do Banco Central de 21/10/2026 (bcb.gov.br, "Cotação de todas
    /// as moedas"): o dólar (tipo A) e o euro (tipo B).
    fn rates() -> Rates {
        currency::parse_bcb("21/10/2026;220;A;USD;5,02700000;5,02770000;1,00000000;1,00000000\n21/10/2026;978;B;EUR;5,63980000;5,64110000;1,12190000;1,12200000\n").unwrap()
    }

    #[test]
    fn par_inicial() {
        assert_eq!(ids(&Conv::from_pairs(&Pairs::default())), ("comprimento", ["pol", "cm"]));
        assert_eq!(ids(&Conv::from_pairs(&Pairs::from_text("temperatura=f>c"))), ("temperatura", ["f", "c"]));
        assert_eq!(ids(&Conv::from_pairs(&Pairs::from_text("xyz=a>b"))), ("comprimento", ["pol", "cm"]));
    }

    #[test]
    fn textos() {
        let c = Conv::from_pairs(&Pairs::default());
        let style = Style::default();
        assert_eq!(c.text(12.5, style, None), "12,5 pol = 31,75 cm");
        assert_eq!(c.history(12.5, style), "12,5 pol → cm");
        let t = Conv::from_pairs(&Pairs::from_text("temperatura=f>c"));
        assert_eq!(t.text(100.0, style, None), "100 °F = 37,7777777778 °C");
        assert_eq!(t.swapped().text(100.0, style, None), "100 °C = 212 °F");
        // Sem resultado: "—".
        let fuel = Conv::from_pairs(&Pairs::from_text("consumo=l100>kml"));
        assert_eq!(fuel.text(0.0, style, None), "0 L/100 km = — km/L");
        // A linha ativa embaixo: o número é o de baixo.
        let low = Conv { active: 1, ..c };
        assert_eq!(low.text(2.54, style, None), "2,54 cm = 1 pol");
    }

    #[test]
    fn escolher_unidade() {
        let pairs = Pairs::from_text("temperatura=c>k");
        let c = Conv::from_pairs(&Pairs::default());
        let (length, km) = units::find("comprimento", "km").unwrap();
        let (_, cm) = units::find("comprimento", "cm").unwrap();
        // Mesma categoria: só a linha muda.
        assert_eq!(ids(&c.choose(&pairs, 0, length, km)), ("comprimento", ["km", "cm"]));
        // A unidade da outra linha: as duas trocam.
        assert_eq!(ids(&c.choose(&pairs, 0, length, cm)), ("comprimento", ["cm", "pol"]));
        // Outra categoria: o par lembrado dali, com a escolhida na linha.
        let (temp, f) = units::find("temperatura", "f").unwrap();
        assert_eq!(ids(&c.choose(&pairs, 0, temp, f)), ("temperatura", ["f", "k"]));
        assert_eq!(ids(&c.choose(&pairs, 1, temp, f)), ("temperatura", ["c", "f"]));
        let (_, k) = units::find("temperatura", "k").unwrap();
        assert_eq!(ids(&c.choose(&pairs, 0, temp, k)), ("temperatura", ["k", "c"]));
        // Sem par lembrado: o padrão.
        let (mass, kg) = units::find("massa", "kg").unwrap();
        assert_eq!(ids(&c.choose(&pairs, 0, mass, kg)), ("massa", ["kg", "lb"]));
        // A linha ativa não muda.
        assert_eq!(Conv { active: 1, ..c }.choose(&pairs, 0, mass, kg).active, 1);
    }

    #[test]
    fn recentes() {
        let c = Conv::from_pairs(&Pairs::default());
        // Sem nada lembrado: só o par de agora.
        let list = recent_pairs(&Pairs::default(), &c);
        assert_eq!(list.iter().map(|&p| pair_label(p)).collect::<Vec<_>>(), ["pol → cm"]);
        // O de agora primeiro, sem repetir, até 5.
        let pairs = Pairs::from_text("temperatura=f>c;comprimento=pol>cm;massa=lb>kg;tempo=h>min;volume=gal>l;area=alq_sp>ha");
        let list = recent_pairs(&pairs, &c);
        assert_eq!(list.len(), 5);
        assert_eq!(list.iter().map(|&p| pair_label(p)).collect::<Vec<_>>(), ["pol → cm", "°F → °C", "lb → kg", "h → min", "gal → L"]);
    }

    #[test]
    fn moeda() {
        let pairs = Pairs::from_text("moeda=USD>BRL");
        let c = Conv::from_pairs(&pairs);
        assert_eq!(ids(&c), ("moeda", ["USD", "BRL"]));
        let style = Style::default();
        // Pela taxa de venda: 100 USD = 502,77 BRL; sem cotações, "—".
        assert_eq!(c.text(100.0, style, Some(&rates())), "100 USD = 502,77 BRL");
        assert_eq!(c.text(100.0, style, None), "100 USD = — BRL");
        // Com Automático, as casas da moeda (o real com 2); a casa escolhida vale.
        assert_eq!(c.swapped().text(1.0, style, Some(&rates())), "1 BRL = 0,20 USD");
        let two = Style { decimals: Some(4), ..style };
        assert_eq!(c.swapped().text(1.0, two, Some(&rates())), "1,0000 BRL = 0,1989 USD");
        // Moeda sem cotação no boletim: sem resultado.
        let (_, jpy) = (0, currency::currencies().iter().position(|m| m.code == "JPY").unwrap());
        assert_eq!(c.choose(&pairs, 1, units::money(), jpy).convert(1.0, Some(&rates())), None);
        // A Moeda vem primeiro no seletor e no menu.
        assert_eq!(category_order()[0], units::money());
        assert_eq!(pair_label((units::money(), c.unit[0], c.unit[1])), "USD → BRL");
        // Sem par lembrado, USD → BRL.
        let (len, km) = units::find("comprimento", "km").unwrap();
        let other = Conv { category: len, unit: [km, km], active: 0 };
        let usd = currency::currencies().iter().position(|m| m.code == "USD").unwrap();
        assert_eq!(ids(&other.choose(&Pairs::default(), 0, units::money(), usd)), ("moeda", ["USD", "BRL"]));
    }

    #[test]
    fn busca_por_peso_acha_as_moedas_antes_da_massa() {
        let groups = picker_groups();
        let found = mide::kit::picker::filter(&groups, "peso");
        // O primeiro resultado é uma moeda (grupo 0, a Moeda); a Massa vem depois (o
        // apelido "peso" da categoria ainda vale na busca).
        assert_eq!(found.first().map(|&(g, _)| g), Some(0));
        let mass = category_order().iter().position(|&c| category_id(c) == "massa").unwrap();
        assert!(found.iter().any(|&(g, _)| g == mass));
        // A busca exata pelo símbolo continua na frente ("kg").
        let (g, i) = mide::kit::picker::filter(&groups, "kg")[0];
        assert_eq!(groups[g].items[i].label, "Quilogramas");
    }

    #[test]
    fn seletor_marca_a_unidade_da_linha() {
        let c = Conv::from_pairs(&Pairs::default());
        // Só confere que monta (os grupos e o item marcado são do mide-ui).
        let _ = c.picker(0);
        let _ = c.picker(1);
    }
}
