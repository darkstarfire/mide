//! Calendário do mês para o painel do relógio da barra: semanas começando no
//! domingo, como no Brasil, e nomes em português.

use chrono::{Datelike, NaiveDate};

pub const MONTHS: [&str; 12] = [
    "janeiro", "fevereiro", "março", "abril", "maio", "junho", "julho", "agosto", "setembro", "outubro", "novembro",
    "dezembro",
];

/// Iniciais dos dias da semana, de domingo a sábado.
pub const WEEKDAYS: [&str; 7] = ["D", "S", "T", "Q", "Q", "S", "S"];

const WEEKDAY_NAMES: [&str; 7] = ["domingo", "segunda-feira", "terça-feira", "quarta-feira", "quinta-feira", "sexta-feira", "sábado"];

/// Um dia da grade: data e se é do mês mostrado (os de fora aparecem apagados).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Day {
    pub date: NaiveDate,
    pub in_month: bool,
}

/// As 6 semanas (42 dias) que mostram o mês, começando no domingo.
pub fn month_grid(year: i32, month: u32) -> Vec<Day> {
    let first = NaiveDate::from_ymd_opt(year, month, 1).unwrap_or_default();
    let back = first.weekday().num_days_from_sunday() as i64;
    let start = first - chrono::Duration::days(back);
    (0..42)
        .map(|i| {
            let date = start + chrono::Duration::days(i);
            Day { date, in_month: date.month() == month && date.year() == year }
        })
        .collect()
}

/// Mês anterior ou seguinte (`delta` = -1 ou +1, ou mais).
pub fn shift_month(year: i32, month: u32, delta: i32) -> (i32, u32) {
    let index = year * 12 + month as i32 - 1 + delta;
    (index.div_euclid(12), (index.rem_euclid(12) + 1) as u32)
}

/// "outubro de 2026".
pub fn month_title(year: i32, month: u32) -> String {
    format!("{} de {year}", MONTHS[(month as usize).saturating_sub(1).min(11)])
}

/// "sábado, 17 de outubro".
pub fn long_date(date: NaiveDate) -> String {
    format!(
        "{}, {} de {}",
        WEEKDAY_NAMES[date.weekday().num_days_from_sunday() as usize],
        date.day(),
        MONTHS[date.month0() as usize]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grade_de_outubro_de_2026() {
        let days = month_grid(2026, 10);
        assert_eq!(days.len(), 42);
        // 1º de outubro de 2026 é uma quinta: a grade começa no domingo, 27 de setembro.
        assert_eq!(days[0].date, NaiveDate::from_ymd_opt(2026, 9, 27).unwrap());
        assert!(!days[0].in_month);
        assert_eq!(days[4].date.day(), 1);
        assert!(days[4].in_month);
        assert_eq!(days.iter().filter(|d| d.in_month).count(), 31);
    }

    #[test]
    fn troca_de_mes_e_ano() {
        assert_eq!(shift_month(2026, 1, -1), (2025, 12));
        assert_eq!(shift_month(2026, 12, 1), (2027, 1));
        assert_eq!(shift_month(2026, 10, 3), (2027, 1));
    }

    #[test]
    fn textos_em_portugues() {
        assert_eq!(month_title(2026, 10), "outubro de 2026");
        assert_eq!(long_date(NaiveDate::from_ymd_opt(2026, 10, 17).unwrap()), "sábado, 17 de outubro");
    }
}
