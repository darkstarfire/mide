//! Cotações de moedas na Calculadora (E3b, etapa 2): o cache do Banco Central, a busca
//! em segundo plano (o `curl` numa thread, que acorda a janela quando termina), a
//! pergunta da primeira vez, o aviso de falha de rede e a janela "Cotações de moedas".
//! As contas e o arquivo ficam em `mide::calculator::{currency, rates}`.

use std::{
    path::PathBuf,
    sync::mpsc::{Receiver, TryRecvError},
};

use chrono::{DateTime, NaiveDate, Utc};
use mide::{
    calculator::{
        currency::{self, Rates},
        rates::{self, Decision, Outcome, Report, Status},
    },
    kit::{Ui, Waker},
};

/// Cotação mais de 7 dias atrás da esperada: a faixa avisa que está antiga.
const OLD_DAYS: i64 = 7;

/// Estado das cotações (lido do cache na primeira vez que a Moeda aparece).
#[derive(Default)]
pub struct Money {
    loaded: bool,
    dir: Option<PathBuf>,
    pub status: Status,
    pub rates: Option<Rates>,
    fetching: Option<Receiver<(Report, Status)>>,
    /// A pergunta "Buscar cotações na internet?" está aberta.
    pub asking: bool,
    /// A janela "Cotações de moedas" está aberta.
    pub window: bool,
    /// O aviso de uma busca que falhou (a mensagem), aberto até a pessoa fechar.
    pub failure: Option<String>,
}

/// Agora; nos testes, `MIDE_RATES_NOW` (RFC 3339) fixa a hora para a data esperada
/// do boletim não depender do dia em que a bateria roda.
pub fn now() -> DateTime<Utc> {
    std::env::var("MIDE_RATES_NOW")
        .ok()
        .and_then(|t| DateTime::parse_from_rfc3339(&t).ok())
        .map_or_else(Utc::now, |t| t.with_timezone(&Utc))
}

/// "21/10/2026".
pub fn date_br(d: NaiveDate) -> String {
    d.format("%d/%m/%Y").to_string()
}

/// "21/10".
fn day_month(d: NaiveDate) -> String {
    d.format("%d/%m").to_string()
}

impl Money {
    /// Lê o cache na primeira vez.
    fn load(&mut self) {
        if self.loaded {
            return;
        }
        self.loaded = true;
        self.dir = rates::cache_dir();
        if let Some(dir) = &self.dir {
            let (status, cached) = rates::load(dir);
            self.status = status;
            self.rates = cached;
        }
        match &self.rates {
            Some(r) => tracing::info!("calculadora: cotações BCB de {} (cache)", r.date),
            None => tracing::info!("calculadora: cotações: nada guardado"),
        }
    }

    /// Há uma busca em andamento.
    pub fn fetching(&self) -> bool {
        self.fetching.is_some()
    }

    /// A pessoa usou moeda: lê o cache e busca se precisar (`online`: a escolha dela,
    /// `None` se ainda não perguntada; `force`: "Atualizar agora").
    pub fn ensure(&mut self, online: Option<bool>, force: bool, waker: Option<Waker>) {
        self.load();
        if self.fetching.is_some() {
            return;
        }
        match rates::needs_fetch(&self.status, now(), online, force) {
            Decision::Fetch => self.spawn(waker),
            Decision::Ask => self.asking = true,
            Decision::Off | Decision::Fresh | Decision::Wait => {}
        }
    }

    /// Busca numa thread: a janela não trava, e o resultado chega pelo `poll`.
    fn spawn(&mut self, waker: Option<Waker>) {
        let Some(dir) = self.dir.clone() else {
            return;
        };
        // Nos testes (`MIDE_NO_NETWORK`), só o curl falso da bateria: nunca a internet.
        let curl = match std::env::var_os("MIDE_CURL") {
            Some(curl) => PathBuf::from(curl),
            None if std::env::var_os("MIDE_NO_NETWORK").is_some() => {
                tracing::info!("calculadora: cotações (teste): sem MIDE_CURL, nada buscado");
                return;
            }
            None => PathBuf::from("curl"),
        };
        let mut status = self.status.clone();
        let at = now();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = std::fs::create_dir_all(&dir);
            let report = rates::refresh(&curl, &dir, at, &mut status);
            let _ = tx.send((report, status));
            if let Some(waker) = waker {
                waker.wake();
            }
        });
        self.fetching = Some(rx);
    }

    /// A busca terminou? Aplica o resultado; `true` quando algo mudou (desenhar de novo).
    pub fn poll(&mut self) -> bool {
        let Some(rx) = &self.fetching else {
            return false;
        };
        let (report, status) = match rx.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return false,
            Err(TryRecvError::Disconnected) => {
                self.fetching = None;
                return true;
            }
        };
        self.fetching = None;
        self.status = status;
        if let Some(r) = report.rates {
            self.rates = Some(r);
        }
        let date = self.rates.as_ref().map_or_else(|| "sem cotação".to_string(), |r| r.date.to_string());
        let how = match report.outcome {
            Outcome::Updated => "rede".to_string(),
            Outcome::NoBulletin => "sem boletim novo".to_string(),
            other => format!("{}: {}", key(other), report.detail),
        };
        tracing::info!("calculadora: cotações BCB de {date} ({how})");
        // Falha de rede: a pessoa fica sabendo (e qual cotação vale).
        if matches!(report.outcome, Outcome::Offline | Outcome::NeedsLogin | Outcome::Unexpected | Outcome::NoCurl | Outcome::Disk) {
            let using = match &self.rates {
                Some(r) => format!(" Vale a cotação guardada de {}.", date_br(r.date)),
                None => " Ainda não há cotação guardada.".to_string(),
            };
            self.failure = Some(format!("{}.{using}", report.outcome.message()));
        }
        true
    }

    /// O texto curto da faixa do conversor (cabe nos 250 pt da Básica).
    pub fn band(&self, online: Option<bool>) -> String {
        if self.fetching.is_some() {
            return "Buscando cotações…".into();
        }
        let Some(r) = &self.rates else {
            return match (online, self.status.outcome) {
                (Some(false), _) => "Busca desligada · sem cotações".into(),
                (_, Some(Outcome::NoCurl)) => "Instale o curl para buscar cotações".into(),
                _ => "Sem cotações · Tentar de novo".into(),
            };
        };
        match self.status.outcome {
            Some(Outcome::Offline | Outcome::NeedsLogin) => format!("Sem conexão · cotação de {}", day_month(r.date)),
            Some(Outcome::Unexpected) => format!("Resposta inesperada · usando {}", day_month(r.date)),
            Some(Outcome::NoCurl) => "Instale o curl para buscar cotações".into(),
            _ if (rates::expected_date(now()) - r.date).num_days() > OLD_DAYS => format!("Cotação antiga: {}", date_br(r.date)),
            _ if online == Some(false) => format!("Busca desligada · BCB {}", day_month(r.date)),
            _ => format!("BCB · {}", date_br(r.date)),
        }
    }

    /// "(BCB 2026-10-21)" para o log das conversões.
    pub fn log_tag(&self) -> String {
        self.rates.as_ref().map_or_else(|| "(sem cotação)".into(), |r| format!("(BCB {})", r.date))
    }
}

fn key(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::Updated => "rede",
        Outcome::NoBulletin => "sem boletim",
        Outcome::Offline => "sem conexão",
        Outcome::NeedsLogin => "rede pede login",
        Outcome::Unexpected => "resposta inesperada",
        Outcome::NoCurl => "sem curl",
        Outcome::Disk => "cache não gravado",
    }
}

/// O que a pessoa fez nas janelas das cotações.
pub enum Answer {
    /// Respondeu à pergunta da primeira vez (`true`: pode buscar).
    Online(bool),
    /// Trocou o interruptor da janela de cotações.
    Toggle(bool),
    /// "Atualizar agora".
    Refresh,
}

/// As janelas das cotações (por cima de tudo, no fim do `view`): a pergunta da
/// primeira vez, o aviso de falha e a janela "Cotações de moedas". `pair`: os códigos
/// das duas linhas do conversor.
pub fn dialogs(ui: &mut Ui, money: &mut Money, online: Option<bool>, pair: (&str, &str)) -> Option<Answer> {
    if money.asking {
        let message = "A Calculadora busca na internet a tabela pública do Banco Central do Brasil, no máximo uma vez por dia útil, sem dizer quais moedas você usa. Dá para mudar em Converter → Cotações de moedas….";
        return match ui.alert("Buscar cotações?", message, &["Não buscar", "Buscar"]) {
            Some(i) => {
                money.asking = false;
                Some(Answer::Online(i == 1))
            }
            None => None,
        };
    }
    if let Some(text) = money.failure.clone() {
        if ui.alert("Cotações não atualizadas", &text, &["OK"]).is_some() {
            money.failure = None;
        }
        return None;
    }
    if !money.window {
        return None;
    }
    let mut answer = None;
    let mut close = ui.input().key(mide::kit::Key::Escape);
    let rate_line = money.rates.as_ref().and_then(|r| currency::format_rate(r, pair.0, pair.1));
    let next = money
        .status
        .next_check
        .map(|t| t.with_timezone(&chrono::FixedOffset::west_opt(3 * 3600).expect("UTC−3")).format("%d/%m %H:%M").to_string());
    let busy = money.fetching.is_some();
    let band = money.band(online);
    let mut on = online == Some(true);
    // Curta: cabe na Calculadora Básica (250 pt).
    ui.modal(360.0, |ui| {
        ui.strong("Cotações de moedas");
        ui.label(&rate_line.unwrap_or_else(|| "Sem cotação para este par.".into()));
        ui.note(&match next {
            Some(next) if on => format!("{band} · próxima busca {next}"),
            _ => band,
        });
        if ui.switch("Buscar na internet", &mut on).changed() {
            answer = Some(Answer::Toggle(on));
        }
        ui.note("Taxa de referência do Banco Central (PTAX de venda, bcb.gov.br): bancos e casas de câmbio cobram outra taxa e o IOF.");
        ui.row(|ui| {
            if ui.button(if busy { "Buscando…" } else { "Atualizar" }).clicked() && !busy {
                answer = Some(Answer::Refresh);
            }
            if ui.primary_button("Fechar").clicked() {
                close = true;
            }
        });
    });
    if close {
        money.window = false;
    }
    answer
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rates_of(date: &str) -> Rates {
        currency::parse_bcb(&format!("{date};220;A;USD;5,02700000;5,02770000;1,00000000;1,00000000\n")).unwrap()
    }

    #[test]
    fn textos_da_faixa() {
        // SAFETY: os testes do app não leem `MIDE_RATES_NOW` em outra thread.
        unsafe { std::env::set_var("MIDE_RATES_NOW", "2026-10-21T15:00:00-03:00") };
        let mut m = Money::default();
        assert_eq!(m.band(None), "Sem cotações · Tentar de novo");
        assert_eq!(m.band(Some(false)), "Busca desligada · sem cotações");
        m.rates = Some(rates_of("21/10/2026"));
        assert_eq!(m.band(Some(true)), "BCB · 21/10/2026");
        m.status.outcome = Some(Outcome::Offline);
        assert_eq!(m.band(Some(true)), "Sem conexão · cotação de 21/10");
        m.status.outcome = Some(Outcome::Updated);
        m.rates = Some(rates_of("02/10/2026"));
        assert_eq!(m.band(Some(true)), "Cotação antiga: 02/10/2026");
        assert_eq!(m.log_tag(), "(BCB 2026-10-02)");
    }
}
