//! Calendário do mês para o painel do relógio da barra: semanas começando no
//! domingo, como no Brasil, e nomes no idioma da interface.

use chrono::{Datelike, NaiveDate};

/// Nome do mês (1 a 12) no idioma da interface.
pub fn month_name(month: u32) -> String {
    let m = (month as usize).clamp(1, 12);
    crate::i18n::trc(
        "month",
        ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"][m - 1],
    )
    .into_owned()
}

/// Nome curto do mês (1 a 12): "Oct" / "out".
pub fn month_short(month: u32) -> String {
    use crate::i18n::trc;
    let m = (month as usize).clamp(1, 12);
    let name = match m {
        1 => trc("short month", "Jan"),
        2 => trc("short month", "Feb"),
        3 => trc("short month", "Mar"),
        4 => trc("short month", "Apr"),
        5 => trc("short month", "May"),
        6 => trc("short month", "Jun"),
        7 => trc("short month", "Jul"),
        8 => trc("short month", "Aug"),
        9 => trc("short month", "Sep"),
        10 => trc("short month", "Oct"),
        11 => trc("short month", "Nov"),
        _ => trc("short month", "Dec"),
    };
    name.into_owned()
}

/// Nome do dia da semana (0 = domingo).
pub fn weekday_name(from_sunday: u32) -> String {
    use crate::i18n::trc;
    let name = match from_sunday % 7 {
        0 => trc("weekday", "Sunday"),
        1 => trc("weekday", "Monday"),
        2 => trc("weekday", "Tuesday"),
        3 => trc("weekday", "Wednesday"),
        4 => trc("weekday", "Thursday"),
        5 => trc("weekday", "Friday"),
        _ => trc("weekday", "Saturday"),
    };
    name.into_owned()
}

/// Iniciais dos dias da semana, de domingo a sábado ("S M T W T F S" / "D S T Q Q S S").
pub fn weekday_initials() -> [String; 7] {
    use crate::i18n::trc;
    [
        trc("Sunday initial", "S"),
        trc("Monday initial", "M"),
        trc("Tuesday initial", "T"),
        trc("Wednesday initial", "W"),
        trc("Thursday initial", "T"),
        trc("Friday initial", "F"),
        trc("Saturday initial", "S"),
    ]
    .map(|s| s.into_owned())
}

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

/// "October 2026" / "outubro de 2026".
pub fn month_title(year: i32, month: u32) -> String {
    crate::tr!("{month} {year}", month = month_name(month), year = year)
}

/// "Saturday, October 17" / "sábado, 17 de outubro".
pub fn long_date(date: NaiveDate) -> String {
    crate::tr!(
        "{weekday}, {month} {day}",
        weekday = weekday_name(date.weekday().num_days_from_sunday()),
        month = month_name(date.month()),
        day = date.day()
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
        crate::i18n::set_language("pt_BR");
        assert_eq!(month_title(2026, 10), "outubro de 2026");
        assert_eq!(long_date(NaiveDate::from_ymd_opt(2026, 10, 17).unwrap()), "sábado, 17 de outubro");
    }
}
