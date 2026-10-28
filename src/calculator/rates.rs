//! Busca e cache das cotações do Banco Central do Brasil (E3b, moeda): qual boletim
//! deveria existir agora, quando buscar de novo, o curl como programa externo (sem
//! shell, só https) e a troca segura do arquivo guardado.
//!
//! Sem interface nem thread: o app chama `needs_fetch` e, se for o caso, `refresh` em
//! outra thread. A pasta do cache e o programa entram por parâmetro, para os testes
//! usarem uma pasta temporária e um curl falso (nenhum teste usa a rede).

use std::ffi::OsString;
use std::fs;
use std::io::{self, ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration as StdDuration, SystemTime};

use chrono::{DateTime, Datelike, Duration, FixedOffset, NaiveDate, TimeZone, Utc, Weekday};

use super::currency::{Rates, parse_bcb};

/// Endereço dos boletins de fechamento ("Cotação de todas as moedas").
pub const BCB_URL: &str = "https://www4.bcb.gov.br/Download/fechamento/";
/// O arquivo guardado.
pub const RATES_FILE: &str = "bcb.csv";
/// Fim do nome dos temporários (`bcb.csv.<pid>.<n>.parcial`): o curl grava num deles
/// antes da validação, e um arquivo ruim nunca toma o lugar do bom.
pub const PARTIAL_SUFFIX: &str = ".parcial";
/// O estado da busca (data, resultado, próxima tentativa, datas sem boletim).
pub const STATUS_FILE: &str = "estado.toml";
/// Quantos dias úteis voltar quando o boletim do dia não existe (feriado).
const MAX_BACK: usize = 5;
/// Hora em que o boletim do dia já deve estar publicado, em Brasília (suposta: o 404
/// cobre o caso de sair mais tarde).
const PUBLISHED_AT: (u32, u32) = (13, 30);
/// A maior espera legítima até a próxima busca (de sexta às 13h30 até o próximo dia útil,
/// com folga). Uma `proxima` além disso veio de um relógio adiantado e é ignorada.
const MAX_WAIT_DAYS: i64 = 5;

/// Brasília em UTC−3 fixo: não há horário de verão desde 2019 (se voltar, a agenda erra
/// por 1 h, e o 404 segura o resto).
fn brasilia() -> FixedOffset {
    FixedOffset::west_opt(3 * 3600).expect("deslocamento válido")
}

fn business_day(d: NaiveDate) -> bool {
    !matches!(d.weekday(), Weekday::Sat | Weekday::Sun)
}

/// O dia útil anterior (feriados não entram: o 404 cuida deles).
pub fn previous_business_day(d: NaiveDate) -> NaiveDate {
    let mut d = d.pred_opt().expect("data dentro da faixa");
    while !business_day(d) {
        d = d.pred_opt().expect("data dentro da faixa");
    }
    d
}

fn next_business_day(d: NaiveDate) -> NaiveDate {
    let mut d = d.succ_opt().expect("data dentro da faixa");
    while !business_day(d) {
        d = d.succ_opt().expect("data dentro da faixa");
    }
    d
}

/// A data de hoje em Brasília.
fn today(now: DateTime<Utc>) -> NaiveDate {
    now.with_timezone(&brasilia()).date_naive()
}

/// O instante em que o boletim de `d` deve sair.
fn published_at(d: NaiveDate) -> DateTime<Utc> {
    let (h, m) = PUBLISHED_AT;
    let local = d.and_hms_opt(h, m, 0).expect("hora válida");
    brasilia().from_local_datetime(&local).single().expect("UTC−3 fixo não tem hora ambígua").with_timezone(&Utc)
}

/// O boletim mais novo que deveria existir: o de hoje, se for dia útil e já passou das
/// 13h30 em Brasília; senão, o do dia útil anterior.
pub fn expected_date(now: DateTime<Utc>) -> NaiveDate {
    let day = today(now);
    if business_day(day) && now >= published_at(day) { day } else { previous_business_day(day) }
}

/// Endereço do boletim de um dia.
pub fn url(date: NaiveDate) -> String {
    format!("{BCB_URL}{}.csv", date.format("%Y%m%d"))
}

/// Como terminou a última busca. Cada um tem a sua mensagem e a sua espera.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Boletim novo baixado e guardado.
    Updated,
    /// Nenhum boletim mais novo que o guardado (404: feriado ou ainda não publicado).
    NoBulletin,
    /// Sem conexão (DNS, recusa, tempo esgotado, conexão caída).
    Offline,
    /// Certificado recusado: em geral, rede de hotel ou aeroporto pedindo login.
    NeedsLogin,
    /// O BCB respondeu algo inesperado (HTML, formato novo, resposta grande demais).
    Unexpected,
    /// O programa curl não está instalado.
    NoCurl,
    /// Não deu para gravar na pasta do cache (sem permissão, disco cheio): o problema é
    /// daqui, não do Banco Central.
    Disk,
}

impl Outcome {
    /// O que dizer à pessoa, no idioma da sessão.
    pub fn message(self) -> std::borrow::Cow<'static, str> {
        match self {
            Outcome::Updated => crate::tr!("Exchange rates updated"),
            Outcome::NoBulletin => crate::tr!("The Central Bank of Brazil hasn't published newer rates yet"),
            Outcome::Offline => crate::tr!("There was a network problem and the exchange rates couldn't be fetched now"),
            Outcome::NeedsLogin => crate::tr!("The network seems to require a login (hotel or airport Wi-Fi) and the exchange rates couldn't be fetched now"),
            Outcome::Unexpected => crate::tr!("The Central Bank sent an unexpected response; the last saved rate still applies"),
            Outcome::NoCurl => crate::tr!("To fetch exchange rates, install the curl program"),
            Outcome::Disk => crate::tr!("The exchange rates couldn't be saved in the cache folder"),
        }
    }

    /// Nome no `estado.toml`.
    fn key(self) -> &'static str {
        match self {
            Outcome::Updated => "novo",
            Outcome::NoBulletin => "sem-boletim",
            Outcome::Offline => "sem-conexao",
            Outcome::NeedsLogin => "login",
            Outcome::Unexpected => "estranha",
            Outcome::NoCurl => "sem-curl",
            Outcome::Disk => "disco",
        }
    }

    fn from_key(key: &str) -> Option<Self> {
        [Outcome::Updated, Outcome::NoBulletin, Outcome::Offline, Outcome::NeedsLogin, Outcome::Unexpected, Outcome::NoCurl, Outcome::Disk]
            .into_iter()
            .find(|o| o.key() == key)
    }
}

/// O que fica guardado em `estado.toml` (a data também vem do `bcb.csv` ao carregar).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Status {
    /// Data do boletim guardado.
    pub date: Option<NaiveDate>,
    /// Resultado da última busca.
    pub outcome: Option<Outcome>,
    /// Quando foi a última busca.
    pub checked: Option<DateTime<Utc>>,
    /// Antes disso, não buscar de novo (a não ser em "Atualizar agora").
    pub next_check: Option<DateTime<Utc>>,
    /// Datas que deram 404 depois de o dia acabar (feriados), para não pedir de novo.
    pub missing: Vec<NaiveDate>,
}

impl Status {
    /// O `estado.toml`, num subconjunto mínimo do TOML (chave = "texto" e listas de
    /// textos), escrito à mão para não trazer um crate só para isso.
    pub fn to_toml(&self) -> String {
        let mut out = String::from("# Estado das cotações da Calculadora do mide (gerado; pode apagar)\n");
        if let Some(d) = self.date {
            out.push_str(&format!("data = \"{d}\"\n"));
        }
        if let Some(o) = self.outcome {
            out.push_str(&format!("resultado = \"{}\"\n", o.key()));
        }
        if let Some(t) = self.checked {
            out.push_str(&format!("verificado = \"{}\"\n", t.to_rfc3339()));
        }
        if let Some(t) = self.next_check {
            out.push_str(&format!("proxima = \"{}\"\n", t.to_rfc3339()));
        }
        let missing: Vec<String> = self.missing.iter().map(|d| format!("\"{d}\"")).collect();
        out.push_str(&format!("sem_boletim = [{}]\n", missing.join(", ")));
        out
    }

    /// Lê o `estado.toml`; `None` se qualquer linha estiver fora do formato (um estado
    /// pela metade vale menos que nenhum).
    pub fn from_toml(text: &str) -> Option<Self> {
        fn quoted(v: &str) -> Option<&str> {
            v.strip_prefix('"')?.strip_suffix('"').filter(|s| !s.contains('"'))
        }
        fn date(v: &str) -> Option<NaiveDate> {
            NaiveDate::parse_from_str(quoted(v)?, "%Y-%m-%d").ok()
        }
        fn time(v: &str) -> Option<DateTime<Utc>> {
            DateTime::parse_from_rfc3339(quoted(v)?).ok().map(|t| t.with_timezone(&Utc))
        }
        let mut s = Status::default();
        for line in text.lines().map(str::trim) {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (key, value) = line.split_once('=')?;
            let value = value.trim();
            match key.trim() {
                "data" => s.date = Some(date(value)?),
                "resultado" => s.outcome = Some(Outcome::from_key(quoted(value)?)?),
                "verificado" => s.checked = Some(time(value)?),
                "proxima" => s.next_check = Some(time(value)?),
                "sem_boletim" => {
                    let inner = value.strip_prefix('[')?.strip_suffix(']')?.trim();
                    s.missing = if inner.is_empty() { Vec::new() } else { inner.split(',').map(|v| date(v.trim())).collect::<Option<_>>()? };
                }
                // Chave de uma versão mais nova: ignorada.
                _ => {}
            }
        }
        Some(s)
    }

    /// A data de boletim que basta ter: a esperada, ou a anterior quando o dia esperado
    /// já passou e deu 404 (feriado; enquanto é hoje, o boletim ainda pode sair).
    fn target(&self, now: DateTime<Utc>) -> NaiveDate {
        let mut d = expected_date(now);
        for _ in 0..MAX_BACK {
            if self.missing.contains(&d) && today(now) > d {
                d = previous_business_day(d);
            } else {
                break;
            }
        }
        d
    }
}

/// Lê o cache da pasta: o estado e as cotações guardadas. Arquivo ilegível ou fora do
/// formato conta como ausente; a data vale a do `bcb.csv`, não a do estado.
pub fn load(dir: &Path) -> (Status, Option<Rates>) {
    let mut status = fs::read_to_string(dir.join(STATUS_FILE)).ok().and_then(|t| Status::from_toml(&t)).unwrap_or_default();
    let rates = fs::read(dir.join(RATES_FILE)).ok().and_then(|b| parse_bcb(&String::from_utf8_lossy(&b)).ok());
    status.date = rates.as_ref().map(|r| r.date);
    (status, rates)
}

/// Cria um temporário novo para `name` na pasta, só desta busca: o pid e um contador
/// separam processos e threads, e o `create_new` garante que ninguém mais o usa (duas
/// Calculadoras buscando ao mesmo tempo não gravam no mesmo arquivo).
fn temp_file(dir: &Path, name: &str) -> io::Result<(PathBuf, fs::File)> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    loop {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = dir.join(format!("{name}.{}.{n}{PARTIAL_SUFFIX}", std::process::id()));
        match fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            // Sobra de um processo antigo com o mesmo pid: tenta o próximo número.
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
}

/// Apaga temporários esquecidos (busca interrompida no meio). Só os velhos: um novo
/// pode ser de outra busca em andamento (o curl desiste em 20 s).
fn remove_stale(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| SystemTime::now().duration_since(t).ok())
            .is_some_and(|age| age > StdDuration::from_secs(600));
        if old && entry.file_name().to_string_lossy().ends_with(PARTIAL_SUFFIX) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// Grava o `estado.toml` (por um temporário próprio e rename).
pub fn save(dir: &Path, status: &Status) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    let (tmp, mut file) = temp_file(dir, STATUS_FILE)?;
    let result = file.write_all(status.to_toml().as_bytes()).and_then(|_| fs::rename(&tmp, dir.join(STATUS_FILE)));
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// O que fazer quando a pessoa usa moeda.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Buscar agora (em outra thread).
    Fetch,
    /// Precisaria buscar, mas a pessoa ainda não respondeu se pode usar a internet.
    Ask,
    /// A busca na internet está desligada: vale o cache.
    Off,
    /// O boletim guardado é o mais novo que deveria existir.
    Fresh,
    /// Tentou há pouco e não deu: esperar até `next_check`.
    Wait,
}

/// Decide se busca. `online` é a escolha da pessoa (`None`: ainda não perguntada);
/// `force` é "Atualizar agora", que passa por cima da agenda, mas não da escolha.
pub fn needs_fetch(status: &Status, now: DateTime<Utc>, online: Option<bool>, force: bool) -> Decision {
    if online == Some(false) {
        return Decision::Off;
    }
    if !force {
        if status.date.is_some_and(|d| d >= status.target(now)) {
            return Decision::Fresh;
        }
        if status.next_check.is_some_and(|t| t > now && t - now <= Duration::days(MAX_WAIT_DAYS)) {
            return Decision::Wait;
        }
    }
    if online.is_none() { Decision::Ask } else { Decision::Fetch }
}

/// Quando tentar de novo depois de uma busca: com o boletim em dia, no próximo dia
/// útil às 13h30; sem boletim novo ou com resposta estranha, daqui a 1 h; sem conexão,
/// daqui a 15 min. `cached` é a data do boletim guardado depois da busca.
pub fn next_check(outcome: Outcome, cached: Option<NaiveDate>, now: DateTime<Utc>) -> DateTime<Utc> {
    match (outcome, cached) {
        (Outcome::Updated, Some(d)) if d >= expected_date(now) => published_at(next_business_day(d)).max(now),
        (Outcome::Offline | Outcome::NeedsLogin, _) => now + Duration::minutes(15),
        // Boletim de um dia anterior (o esperado deu 404), sem boletim, estranha, sem curl.
        _ => now + Duration::hours(1),
    }
}

/// Os argumentos do curl, em vetor (sem shell): `-q` primeiro (só vale nessa posição),
/// para o `~/.curlrc` da pessoa não mudar a busca (um `insecure` ou `fail-with-body` lá
/// enfraqueceria ou quebraria tudo); só https, também nos redirecionamentos; `--fail`
/// para o 404 não virar arquivo; tempos e tamanho limitados; a resposta vai para `out`
/// (o temporário) e o código HTTP sai na saída padrão. A URL vem por último.
pub fn curl_args(url: &str, out: &Path) -> Vec<OsString> {
    let mut args: Vec<OsString> = [
        "-q", "--silent", "--show-error", "--fail", "--location", "--max-redirs", "3", "--proto", "=https", "--proto-redir", "=https",
        "--tlsv1.2", "--connect-timeout", "8", "--max-time", "20", "--max-filesize", "1000000", "--user-agent", "mide", "--output",
    ]
    .into_iter()
    .map(OsString::from)
    .collect();
    args.push(out.as_os_str().to_owned());
    args.extend(["--write-out", "%{http_code}", url].map(OsString::from));
    args
}

/// Resultado de um pedido.
#[derive(Debug)]
pub enum Fetched {
    /// O boletim veio e passou na validação; está no temporário do caminho, ainda sem
    /// rename (quem chama renomeia ou apaga).
    Rates(Rates, PathBuf),
    /// Não há boletim nessa data.
    NotFound,
    /// Não deu; com o detalhe para o log.
    Failed(Outcome, String),
}

/// Pede o boletim de `date` com o `curl`, num temporário só deste pedido, e lê os
/// códigos de saída: 200 grava e valida; HTTP 404 ou 410 é sem boletim (só esses: o
/// `--fail` sai com 22 em qualquer HTTP de erro, e um 503 não diz nada sobre o dia);
/// 408, 429 e 5xx são o BCB fora do ar ou ocupado, um problema de rede para a pessoa;
/// os outros 4xx são resposta inesperada. Do curl: 5, 6, 7, 28, 35, 52 e 56 são sem
/// conexão, e 16, 18, 55 e 92 são a conexão caindo no meio (HTTP/2, transferência,
/// envio), também problema de rede; 23 é falha ao gravar aqui; 60 é certificado
/// recusado (rede pedindo login); 63, resposta grande demais; programa ausente, curl
/// não instalado.
pub fn fetch(curl: &Path, dir: &Path, date: NaiveDate) -> Fetched {
    let disk = |e: io::Error| Fetched::Failed(Outcome::Disk, format!("pasta {}: {e}", dir.display()));
    if let Err(e) = fs::create_dir_all(dir) {
        return disk(e);
    }
    let partial = match temp_file(dir, RATES_FILE) {
        Ok((path, _)) => path,
        Err(e) => return disk(e),
    };
    let output = match Command::new(curl).args(curl_args(&url(date), &partial)).output() {
        Ok(o) => o,
        Err(e) => {
            let _ = fs::remove_file(&partial);
            let outcome = if e.kind() == ErrorKind::NotFound { Outcome::NoCurl } else { Outcome::Unexpected };
            return Fetched::Failed(outcome, format!("{}: {e}", curl.display()));
        }
    };
    let http = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let status: Option<u16> = http.parse().ok();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let failed = |outcome, what: &str| {
        let _ = fs::remove_file(&partial);
        Fetched::Failed(outcome, format!("{what} (HTTP {http}) {stderr}").trim().to_string())
    };
    match output.status.code() {
        Some(0) if http == "200" => {}
        Some(0 | 22) if matches!(status, Some(404 | 410)) => {
            let _ = fs::remove_file(&partial);
            return Fetched::NotFound;
        }
        Some(22) if matches!(status, Some(408 | 429 | 500..=599)) => return failed(Outcome::Offline, "BCB fora do ar ou ocupado"),
        Some(0) => return failed(Outcome::Unexpected, "resposta sem o arquivo"),
        Some(22) => return failed(Outcome::Unexpected, "HTTP de erro"),
        Some(c @ (5 | 6 | 7 | 16 | 18 | 28 | 35 | 52 | 55 | 56 | 92)) => return failed(Outcome::Offline, &format!("curl saiu com {c}")),
        Some(23) => return failed(Outcome::Disk, "curl saiu com 23"),
        Some(60) => return failed(Outcome::NeedsLogin, "curl saiu com 60"),
        Some(63) => return failed(Outcome::Unexpected, "resposta grande demais"),
        Some(c) => return failed(Outcome::Unexpected, &format!("curl saiu com {c}")),
        None => return failed(Outcome::Unexpected, "curl interrompido"),
    }
    let text = match fs::read(&partial) {
        Ok(b) => String::from_utf8_lossy(&b).into_owned(),
        Err(e) => return failed(Outcome::Unexpected, &format!("sem o arquivo baixado: {e}")),
    };
    match parse_bcb(&text) {
        Ok(rates) if rates.date == date => Fetched::Rates(rates, partial),
        Ok(rates) => failed(Outcome::Unexpected, &format!("boletim de {} no lugar de {date}", rates.date)),
        Err(e) => failed(Outcome::Unexpected, &format!("arquivo recusado: {e}")),
    }
}

/// O resultado de `refresh`.
#[derive(Debug)]
pub struct Report {
    /// Como terminou a busca (a mensagem sai de `Outcome::message`).
    pub outcome: Outcome,
    /// As cotações novas, quando vieram.
    pub rates: Option<Rates>,
    /// Para o log: o erro do curl, a validação que falhou.
    pub detail: String,
    /// As datas pedidas, em ordem.
    pub tried: Vec<NaiveDate>,
}

/// Busca o boletim mais novo e atualiza o cache e o `status` (que também é gravado). Em
/// 404, volta dias úteis (no máximo 5), sem chegar à data do cache, e lembra as datas
/// sem boletim. O arquivo baixado só toma o lugar do guardado depois de validado, por
/// rename do temporário daquela busca (duas Calculadoras buscando ao mesmo tempo não
/// estragam o cache: cada uma renomeia só o arquivo que ela validou).
pub fn refresh(curl: &Path, dir: &Path, now: DateTime<Utc>, status: &mut Status) -> Report {
    remove_stale(dir);
    let mut report = Report { outcome: Outcome::NoBulletin, rates: None, detail: String::new(), tried: Vec::new() };
    let mut date = expected_date(now);
    for _ in 0..=MAX_BACK {
        if status.date.is_some_and(|cached| date <= cached) {
            break;
        }
        // Um 404 de um dia que já passou não vai mudar: não pede de novo.
        let known_missing = status.missing.contains(&date) && today(now) > date;
        if !known_missing {
            report.tried.push(date);
            match fetch(curl, dir, date) {
                Fetched::Rates(rates, partial) => {
                    if let Err(e) = fs::rename(&partial, dir.join(RATES_FILE)) {
                        let _ = fs::remove_file(&partial);
                        report.outcome = Outcome::Disk;
                        report.detail = format!("não deu para guardar o boletim: {e}");
                    } else {
                        report.outcome = Outcome::Updated;
                        status.date = Some(rates.date);
                        report.rates = Some(rates);
                    }
                    break;
                }
                // Um 404 de hoje pode ser só atraso: vale para a espera de 1 h, mas a
                // data só vira "sem boletim" depois que o dia acaba.
                Fetched::NotFound => {
                    if today(now) > date && !status.missing.contains(&date) {
                        status.missing.push(date);
                    }
                }
                Fetched::Failed(outcome, detail) => {
                    report.outcome = outcome;
                    report.detail = detail;
                    break;
                }
            }
        }
        date = previous_business_day(date);
    }
    // Só as datas que ainda podem ser pedidas ficam lembradas.
    let oldest = (0..=MAX_BACK).fold(today(now), |d, _| previous_business_day(d));
    status.missing.retain(|d| *d >= oldest && status.date.is_none_or(|cached| *d > cached));
    status.missing.sort();
    status.outcome = Some(report.outcome);
    status.checked = Some(now);
    status.next_check = Some(next_check(report.outcome, status.date, now));
    if let Err(e) = save(dir, status) {
        // Sem o estado, a próxima abertura só busca de novo: vai para o log e segue.
        report.detail = format!("{} (estado não gravado: {e})", report.detail).trim().to_string();
    }
    report
}

/// A pasta do cache: `$XDG_CACHE_HOME/mide/cotacoes` (ou `~/.cache/mide/cotacoes`).
pub fn cache_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
    Some(base.join("mide").join("cotacoes"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration as StdDuration;

    /// Trecho do arquivo real do Banco Central do Brasil, fechamento PTAX de 21/10/2026
    /// (https://www4.bcb.gov.br/Download/fechamento/20261021.csv).
    const BCB_20261021: &str = "\
21/10/2026;220;A;USD;5,02700000;5,02770000;1,00000000;1,00000000
21/10/2026;978;B;EUR;5,63980000;5,64110000;1,12190000;1,12200000
";

    /// Instante em Brasília (UTC−3).
    fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<Utc> {
        brasilia().with_ymd_and_hms(y, mo, d, h, mi, 0).single().unwrap().with_timezone(&Utc)
    }

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    /// O mesmo boletim com outra data ("21/10/2026" trocado).
    fn bulletin(date: NaiveDate) -> String {
        BCB_20261021.replace("21/10/2026", &date.format("%d/%m/%Y").to_string())
    }

    /// Pasta temporária própria do teste, apagada no fim.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let p = std::env::temp_dir().join(format!("mide-cotacoes-{}-{name}", std::process::id()));
            let _ = fs::remove_dir_all(&p);
            fs::create_dir_all(&p).unwrap();
            TempDir(p)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// Um curl falso, sem rede: responde com os arquivos `AAAAMMDD.csv` da pasta dele
    /// (200), sai com 6 se houver o arquivo `sem-rede`, e dá 404 (saída 22) no resto.
    /// As URLs pedidas vão para `chamadas`.
    fn fake_curl(dir: &Path) -> PathBuf {
        let script = dir.join("curl");
        install_script(
            &script,
            r#"#!/bin/sh
dir=$(dirname "$0")
out=""
url=""
while [ $# -gt 0 ]; do
    case "$1" in
        --output) out="$2"; shift ;;
        *) url="$1" ;;
    esac
    shift
done
echo "$url" >> "$dir/chamadas"
if [ -e "$dir/sem-rede" ]; then
    echo "curl: (6) Could not resolve host: www4.bcb.gov.br" >&2
    exit 6
fi
name=${url##*/}
if [ -e "$dir/$name" ]; then
    cp "$dir/$name" "$out"
    printf 200
    exit 0
fi
echo "curl: (22) The requested URL returned error: 404" >&2
printf 404
exit 22
"#,
        );
        script
    }

    /// Grava um script executável. Quem escreve é o `install`, outro processo: se este
    /// processo abrisse o arquivo para escrita, um teste em paralelo que fizesse fork
    /// nesse instante herdaria o descritor e o exec do script daria ETXTBSY.
    fn install_script(path: &Path, text: &str) {
        let src = path.with_extension("txt");
        fs::write(&src, text).unwrap();
        let ok = Command::new("install").arg("-m").arg("755").arg(&src).arg(path).status().unwrap().success();
        assert!(ok, "install {}", path.display());
    }

    /// Um curl falso que sempre responde igual: imprime o código HTTP, a mensagem de erro
    /// e sai com `code`.
    fn fake_reply(dir: &Path, http: &str, code: i32, stderr: &str) -> PathBuf {
        let script = dir.join("curl");
        install_script(&script, &format!("#!/bin/sh\necho '{stderr}' >&2\nprintf '{http}'\nexit {code}\n"));
        script
    }

    /// Os temporários que sobraram na pasta.
    fn partials(dir: &Path) -> Vec<String> {
        fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(PARTIAL_SUFFIX))
            .collect()
    }

    fn calls(dir: &Path) -> Vec<String> {
        fs::read_to_string(dir.join("chamadas")).unwrap_or_default().lines().map(String::from).collect()
    }

    #[test]
    fn data_esperada() {
        // 21/10/2026 é uma quarta.
        assert_eq!(expected_date(at(2026, 10, 21, 14, 0)), day(2026, 10, 21));
        assert_eq!(expected_date(at(2026, 10, 21, 13, 30)), day(2026, 10, 21));
        assert_eq!(expected_date(at(2026, 10, 21, 12, 0)), day(2026, 10, 20));
        assert_eq!(expected_date(at(2026, 10, 24, 15, 0)), day(2026, 10, 23));
        assert_eq!(expected_date(at(2026, 10, 25, 15, 0)), day(2026, 10, 23));
        assert_eq!(expected_date(at(2026, 10, 26, 9, 0)), day(2026, 10, 23));
        // Terça às 23h30 em Brasília é quarta 02h30 em UTC: vale a terça.
        let late = at(2026, 10, 20, 23, 30);
        assert_eq!(late, Utc.with_ymd_and_hms(2026, 10, 21, 2, 30, 0).unwrap());
        assert_eq!(expected_date(late), day(2026, 10, 20));
        assert_eq!(url(day(2026, 10, 21)), "https://www4.bcb.gov.br/Download/fechamento/20261021.csv");
    }

    #[test]
    fn quando_buscar() {
        let now = at(2026, 10, 21, 15, 0);
        let fresh = Status { date: Some(day(2026, 10, 21)), ..Status::default() };
        let old = Status { date: Some(day(2026, 10, 20)), ..Status::default() };
        let empty = Status::default();
        assert_eq!(needs_fetch(&fresh, now, Some(true), false), Decision::Fresh);
        assert_eq!(needs_fetch(&old, now, Some(true), false), Decision::Fetch);
        assert_eq!(needs_fetch(&empty, now, Some(true), false), Decision::Fetch);
        // Antes das 13h30, o de terça basta.
        assert_eq!(needs_fetch(&old, at(2026, 10, 21, 10, 0), Some(true), false), Decision::Fresh);
        // Ainda não perguntado: só pergunta se precisar buscar.
        assert_eq!(needs_fetch(&old, now, None, false), Decision::Ask);
        assert_eq!(needs_fetch(&fresh, now, None, false), Decision::Fresh);
        assert_eq!(needs_fetch(&fresh, now, None, true), Decision::Ask);
        // Desligada: nunca busca, nem com "Atualizar agora".
        assert_eq!(needs_fetch(&old, now, Some(false), false), Decision::Off);
        assert_eq!(needs_fetch(&old, now, Some(false), true), Decision::Off);
        // Tentou há pouco: espera, a não ser com "Atualizar agora".
        let waiting = Status { next_check: Some(now + Duration::minutes(10)), ..old.clone() };
        assert_eq!(needs_fetch(&waiting, now, Some(true), false), Decision::Wait);
        assert_eq!(needs_fetch(&waiting, now, Some(true), true), Decision::Fetch);
        assert_eq!(needs_fetch(&fresh, now, Some(true), true), Decision::Fetch);
        let past = Status { next_check: Some(now - Duration::minutes(1)), ..old.clone() };
        assert_eq!(needs_fetch(&past, now, Some(true), false), Decision::Fetch);
        // Quarta deu 404 (feriado). Na quinta de manhã, a terça basta; na própria quarta,
        // o boletim ainda pode sair.
        let holiday = Status { date: Some(day(2026, 10, 20)), missing: vec![day(2026, 10, 21)], ..Status::default() };
        assert_eq!(needs_fetch(&holiday, at(2026, 10, 22, 10, 0), Some(true), false), Decision::Fresh);
        assert_eq!(needs_fetch(&holiday, now, Some(true), false), Decision::Fetch);
    }

    #[test]
    fn proxima_busca() {
        let now = at(2026, 10, 21, 15, 0);
        // Em dia: no próximo dia útil às 13h30 (sexta → segunda).
        assert_eq!(next_check(Outcome::Updated, Some(day(2026, 10, 21)), now), at(2026, 10, 22, 13, 30));
        assert_eq!(next_check(Outcome::Updated, Some(day(2026, 10, 23)), at(2026, 10, 23, 14, 0)), at(2026, 10, 26, 13, 30));
        // Veio um boletim anterior ao esperado: tentar o do dia de novo em 1 h.
        assert_eq!(next_check(Outcome::Updated, Some(day(2026, 10, 20)), now), now + Duration::hours(1));
        assert_eq!(next_check(Outcome::NoBulletin, Some(day(2026, 10, 20)), now), now + Duration::hours(1));
        assert_eq!(next_check(Outcome::Offline, None, now), now + Duration::minutes(15));
        assert_eq!(next_check(Outcome::NeedsLogin, None, now), now + Duration::minutes(15));
        assert_eq!(next_check(Outcome::Unexpected, None, now), now + Duration::hours(1));
        assert_eq!(next_check(Outcome::NoCurl, None, now), now + Duration::hours(1));
    }

    #[test]
    fn argumentos_do_curl() {
        let args: Vec<String> =
            curl_args("https://exemplo/20261021.csv", Path::new("/c/bcb.csv.1.0.parcial")).into_iter().map(|a| a.into_string().unwrap()).collect();
        // O `-q` só desliga o ~/.curlrc se vier primeiro.
        assert_eq!(args[0], "-q");
        let pair = |flag: &str, value: &str| args.windows(2).any(|w| w[0] == flag && w[1] == value);
        assert!(pair("--proto", "=https") && pair("--proto-redir", "=https"));
        assert!(args.contains(&"--fail".to_string()));
        assert!(pair("--max-time", "20"));
        assert!(pair("--output", "/c/bcb.csv.1.0.parcial"));
        assert!(pair("--write-out", "%{http_code}"));
        assert_eq!(args.last().unwrap(), "https://exemplo/20261021.csv");
        assert!(!args.iter().any(|a| a.starts_with("http:")));
    }

    #[test]
    fn estado_ida_e_volta() {
        let s = Status {
            date: Some(day(2026, 10, 21)),
            outcome: Some(Outcome::Updated),
            checked: Some(at(2026, 10, 21, 15, 0)),
            next_check: Some(at(2026, 10, 22, 13, 30)),
            missing: vec![day(2026, 10, 19), day(2026, 10, 20)],
        };
        assert_eq!(Status::from_toml(&s.to_toml()), Some(s.clone()));
        assert_eq!(Status::from_toml(&Status::default().to_toml()), Some(Status::default()));
        assert_eq!(Status::from_toml("data = \"2026-13-40\""), None);
        assert_eq!(Status::from_toml("resultado = \"talvez\""), None);
        assert_eq!(Status::from_toml("sem_boletim = [\"2026-10-20\""), None);
        assert_eq!(Status::from_toml("\u{0}\u{1}lixo"), None);
        // Na pasta: corrompido conta como ausente.
        let tmp = TempDir::new("estado");
        fs::write(tmp.0.join(STATUS_FILE), "data = \"2026-10").unwrap();
        fs::write(tmp.0.join(RATES_FILE), "<html>").unwrap();
        assert_eq!(load(&tmp.0), (Status::default(), None));
        save(&tmp.0, &s).unwrap();
        fs::write(tmp.0.join(RATES_FILE), BCB_20261021).unwrap();
        let (loaded, rates) = load(&tmp.0);
        assert_eq!(loaded, s);
        assert_eq!(rates.unwrap().date, day(2026, 10, 21));
    }

    #[test]
    fn busca_com_curl_falso() {
        let now = at(2026, 10, 21, 15, 0);
        let bin = TempDir::new("curl");
        let cache = TempDir::new("cache");
        let curl = fake_curl(&bin.0);
        fs::write(bin.0.join("20261021.csv"), bulletin(day(2026, 10, 21))).unwrap();

        // 200: grava e lembra a data.
        let mut status = Status::default();
        let report = refresh(&curl, &cache.0, now, &mut status);
        assert_eq!(report.outcome, Outcome::Updated, "{}", report.detail);
        assert_eq!(report.rates.unwrap().date, day(2026, 10, 21));
        assert_eq!(load(&cache.0).0, status);
        assert_eq!(status.date, Some(day(2026, 10, 21)));
        assert_eq!(status.next_check, Some(at(2026, 10, 22, 13, 30)));
        assert!(partials(&cache.0).is_empty(), "{:?}", partials(&cache.0));
        assert_eq!(needs_fetch(&status, now, Some(true), false), Decision::Fresh);

        // No dia seguinte, saída 6: sem conexão, e o cache fica.
        let later = at(2026, 10, 22, 15, 0);
        fs::write(bin.0.join("sem-rede"), "").unwrap();
        let report = refresh(&curl, &cache.0, later, &mut status);
        assert_eq!(report.outcome, Outcome::Offline);
        assert_eq!(report.outcome.message(), "Houve um problema de rede e não foi possível buscar as cotações agora");
        assert!(report.detail.contains("Could not resolve host"), "{}", report.detail);
        assert_eq!(load(&cache.0).1.unwrap().date, day(2026, 10, 21));
        assert_eq!(status.date, Some(day(2026, 10, 21)));
        assert_eq!(status.next_check, Some(later + Duration::minutes(15)));
        assert_eq!(needs_fetch(&status, later, Some(true), false), Decision::Wait);
        assert_eq!(calls(&bin.0).len(), 2);
    }

    #[test]
    fn volta_dias_sem_boletim() {
        // Sexta 23/10 e quinta 22/10 sem boletim (404, 404); quarta 21/10 vem (200).
        let now = at(2026, 10, 23, 15, 0);
        let bin = TempDir::new("curl-volta");
        let cache = TempDir::new("cache-volta");
        let curl = fake_curl(&bin.0);
        fs::write(bin.0.join("20261021.csv"), bulletin(day(2026, 10, 21))).unwrap();
        let mut status = Status { date: Some(day(2026, 10, 19)), ..Status::default() };
        let report = refresh(&curl, &cache.0, now, &mut status);
        assert_eq!(report.outcome, Outcome::Updated, "{}", report.detail);
        assert_eq!(report.tried, [day(2026, 10, 23), day(2026, 10, 22), day(2026, 10, 21)]);
        assert_eq!(calls(&bin.0).len(), 3);
        assert_eq!(status.date, Some(day(2026, 10, 21)));
        // Só a quinta, que já passou, vira "sem boletim"; o de hoje ainda pode sair e
        // vale só para a espera de 1 h.
        assert_eq!(status.missing, [day(2026, 10, 22)]);
        assert_eq!(status.next_check, Some(now + Duration::hours(1)));

        // Segunda de manhã: a sexta é pedida de novo uma vez (agora com o dia acabado);
        // a quinta não, porque já tinha passado quando deu 404.
        let monday = at(2026, 10, 26, 10, 0);
        let (loaded, _) = load(&cache.0);
        assert_eq!(needs_fetch(&loaded, monday, Some(true), false), Decision::Fetch);
        let mut again = loaded.clone();
        let report = refresh(&curl, &cache.0, monday, &mut again);
        assert_eq!(report.outcome, Outcome::NoBulletin);
        assert_eq!(report.tried, [day(2026, 10, 23)]);
        assert_eq!(calls(&bin.0).len(), 4);
        assert_eq!(again.missing, [day(2026, 10, 22), day(2026, 10, 23)]);
        // Daqui em diante, nada a pedir.
        assert_eq!(needs_fetch(&again, monday, Some(true), false), Decision::Fresh);
        let report = refresh(&curl, &cache.0, monday, &mut again);
        assert!(report.tried.is_empty());
        assert_eq!(calls(&bin.0).len(), 4);

        // Sem cache, volta no máximo 5 dias úteis além do esperado.
        let empty = TempDir::new("cache-vazio");
        let mut status = Status::default();
        let report = refresh(&curl, &empty.0, at(2026, 11, 20, 15, 0), &mut status);
        assert_eq!(report.outcome, Outcome::NoBulletin);
        assert_eq!(report.tried.len(), MAX_BACK + 1);
        assert_eq!(status.date, None);
        assert!(!empty.0.join(RATES_FILE).exists());
    }

    #[test]
    fn nunca_troca_o_bom_pelo_ruim() {
        let now = at(2026, 10, 22, 15, 0);
        let bin = TempDir::new("curl-html");
        let cache = TempDir::new("cache-html");
        let curl = fake_curl(&bin.0);
        fs::write(cache.0.join(RATES_FILE), bulletin(day(2026, 10, 21))).unwrap();
        let (mut status, _) = load(&cache.0);
        // Página HTML com 200 (rede com login sem HTTPS de verdade, endereço mudado).
        fs::write(bin.0.join("20261022.csv"), "<!DOCTYPE html><html><body>Login</body></html>").unwrap();
        let report = refresh(&curl, &cache.0, now, &mut status);
        assert_eq!(report.outcome, Outcome::Unexpected);
        assert!(report.detail.contains("HTML"), "{}", report.detail);
        assert_eq!(load(&cache.0).1.unwrap().date, day(2026, 10, 21));
        assert!(partials(&cache.0).is_empty(), "{:?}", partials(&cache.0));
        // Boletim de outra data no lugar do pedido também não entra.
        fs::write(bin.0.join("20261022.csv"), bulletin(day(2026, 10, 20))).unwrap();
        let report = refresh(&curl, &cache.0, now, &mut status);
        assert_eq!(report.outcome, Outcome::Unexpected);
        assert_eq!(load(&cache.0).1.unwrap().date, day(2026, 10, 21));
        assert_eq!(status.date, Some(day(2026, 10, 21)));
    }

    #[test]
    fn sem_curl() {
        let cache = TempDir::new("sem-curl");
        let mut status = Status::default();
        let missing = cache.0.join("nao-existe").join("curl");
        let report = refresh(&missing, &cache.0, at(2026, 10, 21, 15, 0), &mut status);
        assert_eq!(report.outcome, Outcome::NoCurl);
        assert_eq!(report.tried, [day(2026, 10, 21)]);
        assert_eq!(status.outcome, Some(Outcome::NoCurl));
        assert_eq!(load(&cache.0).0.outcome, Some(Outcome::NoCurl));
    }

    #[test]
    fn erro_do_servidor_nao_e_feriado() {
        // Quinta 10h: o esperado é o de quarta; o cache tem o de terça.
        let thursday = at(2026, 10, 22, 10, 0);
        for (http, outcome) in [
            ("503", Outcome::Offline),
            ("500", Outcome::Offline),
            ("429", Outcome::Offline),
            ("408", Outcome::Offline),
            ("403", Outcome::Unexpected),
            ("400", Outcome::Unexpected),
        ] {
            let bin = TempDir::new(&format!("http-{http}"));
            let cache = TempDir::new(&format!("http-{http}-cache"));
            let curl = fake_reply(&bin.0, http, 22, &format!("curl: (22) The requested URL returned error: {http}"));
            fs::write(cache.0.join(RATES_FILE), bulletin(day(2026, 10, 20))).unwrap();
            let (mut status, _) = load(&cache.0);
            let report = refresh(&curl, &cache.0, thursday, &mut status);
            assert_eq!(report.outcome, outcome, "HTTP {http}: {}", report.detail);
            assert_eq!(report.tried, [day(2026, 10, 21)], "HTTP {http}: para no primeiro erro");
            assert!(status.missing.is_empty(), "HTTP {http} marcou feriado: {:?}", status.missing);
            assert!(partials(&cache.0).is_empty());
            // Com o BCB de volta, a quarta ainda é pedida.
            let later = thursday + Duration::hours(2);
            assert_eq!(needs_fetch(&status, later, Some(true), false), Decision::Fetch, "HTTP {http}");
        }
        let thursday_wait = |outcome| next_check(outcome, Some(day(2026, 10, 20)), thursday);
        assert_eq!(thursday_wait(Outcome::Offline), thursday + Duration::minutes(15));
        // 410 (removido) conta como sem boletim, como o 404.
        let bin = TempDir::new("http-410");
        let cache = TempDir::new("http-410-cache");
        let curl = fake_reply(&bin.0, "410", 22, "curl: (22) The requested URL returned error: 410");
        let mut status = Status::default();
        assert!(matches!(fetch(&curl, &cache.0, day(2026, 10, 21)), Fetched::NotFound));
        let report = refresh(&curl, &cache.0, thursday, &mut status);
        assert_eq!(report.outcome, Outcome::NoBulletin);
    }

    #[test]
    fn atraso_de_hoje_nao_vira_feriado() {
        let bin = TempDir::new("atraso");
        let cache = TempDir::new("atraso-cache");
        let curl = fake_curl(&bin.0);
        fs::write(cache.0.join(RATES_FILE), bulletin(day(2026, 10, 20))).unwrap();
        let (mut status, _) = load(&cache.0);
        // Quarta 13h40: o BCB ainda não publicou (404).
        let report = refresh(&curl, &cache.0, at(2026, 10, 21, 13, 40), &mut status);
        assert_eq!(report.outcome, Outcome::NoBulletin);
        assert!(status.missing.is_empty(), "{:?}", status.missing);
        // Sai às 14h; a pessoa só volta a abrir a Calculadora na quinta de manhã.
        fs::write(bin.0.join("20261021.csv"), bulletin(day(2026, 10, 21))).unwrap();
        let (mut loaded, _) = load(&cache.0);
        let thursday = at(2026, 10, 22, 10, 0);
        assert_eq!(needs_fetch(&loaded, thursday, Some(true), false), Decision::Fetch);
        let report = refresh(&curl, &cache.0, thursday, &mut loaded);
        assert_eq!(report.outcome, Outcome::Updated, "{}", report.detail);
        assert_eq!(report.tried, [day(2026, 10, 21)]);
        assert_eq!(load(&cache.0).1.unwrap().date, day(2026, 10, 21));
    }

    #[test]
    fn buscas_simultaneas_nao_dividem_o_temporario() {
        let bin_a = TempDir::new("par-a");
        let bin_b = TempDir::new("par-b");
        let cache = TempDir::new("par-cache");
        let wed = day(2026, 10, 21);
        fs::write(cache.0.join(RATES_FILE), bulletin(day(2026, 10, 20))).unwrap();
        let curl_a = fake_curl(&bin_a.0);
        fs::write(bin_a.0.join("20261021.csv"), bulletin(wed)).unwrap();
        // B começa a gravar e a conexão cai no meio (28).
        let curl_b = bin_b.0.join("curl");
        install_script(
            &curl_b,
            "#!/bin/sh\nwhile [ $# -gt 0 ]; do [ \"$1\" = --output ] && out=\"$2\"; shift; done\nprintf '21/10/2026;220;A;US' > \"$out\"\nsleep 1\nexit 28\n",
        );

        // A baixou e validou: o arquivo está no temporário de A, ainda sem rename.
        let Fetched::Rates(_, partial_a) = fetch(&curl_a, &cache.0, wed) else { panic!("A deveria baixar") };
        let dir = cache.0.clone();
        let b = std::thread::spawn(move || fetch(&curl_b, &dir, wed));
        // B está gravando no temporário dele, não no de A (espera o de B aparecer, sem
        // depender de um tempo fixo numa máquina carregada).
        let start = std::time::Instant::now();
        while partials(&cache.0).len() < 2 && start.elapsed() < StdDuration::from_secs(5) {
            std::thread::sleep(StdDuration::from_millis(10));
        }
        assert_eq!(partials(&cache.0).len(), 2, "{:?}", partials(&cache.0));
        // O passo seguinte de `refresh` em A.
        fs::rename(&partial_a, cache.0.join(RATES_FILE)).unwrap();
        assert!(matches!(b.join().unwrap(), Fetched::Failed(Outcome::Offline, _)));
        let rates = load(&cache.0).1.expect("o boletim validado por A");
        assert_eq!(rates.date, wed);
        assert!(rates.has("EUR"));
        assert!(partials(&cache.0).is_empty(), "{:?}", partials(&cache.0));
        // O estado também vai por um temporário próprio.
        save(&cache.0, &Status::default()).unwrap();
        assert!(partials(&cache.0).is_empty());
    }

    #[test]
    fn ignora_o_curlrc_da_pessoa() {
        if Command::new("curl").arg("--version").output().is_err() {
            return;
        }
        // Um ~/.curlrc com `fail-with-body` faria o curl recusar o `--fail` (saída 2).
        let home = TempDir::new("curlrc");
        fs::write(home.0.join(".curlrc"), "fail-with-body\ninsecure\n").unwrap();
        let out = TempDir::new("curlrc-out");
        // 127.0.0.1:1: recusa local, nada sai da máquina. Esperado: 7 (sem conexão).
        let output = Command::new("curl")
            .args(curl_args("https://127.0.0.1:1/20261021.csv", &out.0.join("x.parcial")))
            .env("HOME", &home.0)
            .env_remove("CURL_HOME")
            // Com um proxy no ambiente, o curl sairia da máquina.
            .env_remove("https_proxy")
            .env_remove("HTTPS_PROXY")
            .env_remove("http_proxy")
            .env_remove("ALL_PROXY")
            .env_remove("all_proxy")
            .env_remove("XDG_CONFIG_HOME")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(7), "{}", String::from_utf8_lossy(&output.stderr));
    }

    #[test]
    fn queda_no_meio_e_problema_de_rede() {
        for code in [16, 18, 55, 92] {
            let bin = TempDir::new(&format!("queda-{code}"));
            let cache = TempDir::new(&format!("queda-{code}-cache"));
            let curl = fake_reply(&bin.0, "200", code, &format!("curl: ({code}) conexão caiu"));
            let mut status = Status::default();
            let now = at(2026, 10, 21, 15, 0);
            let report = refresh(&curl, &cache.0, now, &mut status);
            assert_eq!(report.outcome, Outcome::Offline, "curl {code}");
            assert_eq!(status.next_check, Some(now + Duration::minutes(15)));
            assert!(partials(&cache.0).is_empty());
        }
    }

    #[test]
    fn proxima_no_futuro_distante_e_ignorada() {
        let now = at(2026, 10, 22, 15, 0);
        let old = Status { date: Some(day(2026, 10, 20)), ..Status::default() };
        // Gravada com o relógio adiantado em um ano: não segura a busca.
        let far = Status { next_check: Some(at(2027, 10, 21, 16, 0)), ..old.clone() };
        assert_eq!(needs_fetch(&far, now, Some(true), false), Decision::Fetch);
        // A maior espera de verdade (sexta → segunda às 13h30) continua valendo.
        let friday = at(2026, 10, 23, 14, 0);
        let weekend = Status { next_check: Some(at(2026, 10, 26, 13, 30)), ..old };
        assert_eq!(needs_fetch(&weekend, friday, Some(true), false), Decision::Wait);
    }

    #[test]
    fn falha_ao_gravar_e_problema_local() {
        // O curl não conseguiu gravar (23): a mensagem não culpa o Banco Central.
        let bin = TempDir::new("disco");
        let cache = TempDir::new("disco-cache");
        let curl = fake_reply(&bin.0, "200", 23, "curl: (23) Failure writing output to destination");
        let mut status = Status::default();
        let report = refresh(&curl, &cache.0, at(2026, 10, 21, 15, 0), &mut status);
        assert_eq!(report.outcome, Outcome::Disk);
        assert_eq!(report.outcome.message(), "Não foi possível gravar as cotações na pasta do cache");
        assert!(!report.outcome.message().contains("Banco Central"));
        assert_eq!(load(&cache.0).0.outcome, Some(Outcome::Disk));
        // A pasta do cache não pode ser criada (um arquivo no caminho).
        fs::write(cache.0.join("arquivo"), "").unwrap();
        let blocked = cache.0.join("arquivo").join("cotacoes");
        assert!(matches!(fetch(&curl, &blocked, day(2026, 10, 21)), Fetched::Failed(Outcome::Disk, _)));
        assert_eq!(Status::from_toml("resultado = \"disco\"").unwrap().outcome, Some(Outcome::Disk));
    }

    /// Com a rede de verdade, rodado à mão:
    /// `cargo test cotacoes_de_verdade -- --ignored --nocapture`.
    #[test]
    #[ignore = "usa a internet"]
    fn cotacoes_de_verdade() {
        let cache = TempDir::new("de-verdade");
        let mut status = Status::default();
        let report = refresh(Path::new("curl"), &cache.0, Utc::now(), &mut status);
        println!("{:?} {:?} {}", report.outcome, report.tried, report.detail);
        assert_eq!(report.outcome, Outcome::Updated, "{}", report.detail);
        let rates = report.rates.unwrap();
        let usd = rates.rate("USD").unwrap();
        assert!((1.0..20.0).contains(&usd), "{usd}");
        assert!(rates.codes().count() > 100, "{}", rates.codes().count());
        println!("BCB de {}: {} moedas; fora: {:?}", rates.date, rates.codes().count(), rates.skipped);
    }
}
