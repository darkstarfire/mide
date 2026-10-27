//! Vigia do laço de eventos do compositor: uma thread à parte confere se o laço ainda
//! está girando e avisa no log quando ele fica parado (o congelamento da tela, que a
//! interface não consegue mostrar). Não age sobre o laço: só registra, para achar a fase.

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Quanto tempo sem voltar ao laço até registrar um travamento.
const LIMITE: Duration = Duration::from_millis(1500);

/// Última volta do laço e a fase em que ele estava (a última marcada por `phase`).
static ULTIMA: Mutex<Option<(Instant, &'static str)>> = Mutex::new(None);

/// O laço voltou ao começo de uma volta (depois de despachar as fontes de evento).
pub fn beat() {
    if let Ok(mut ultima) = ULTIMA.lock() {
        *ultima = Some((Instant::now(), "laço"));
    }
}

/// Marca em que parte do trabalho o laço está (ex.: `render`), para o aviso de travamento
/// dizer onde ele parou.
pub fn phase(nome: &'static str) {
    if let Ok(mut ultima) = ULTIMA.lock() {
        let instante = ultima.map(|(t, _)| t).unwrap_or_else(Instant::now);
        *ultima = Some((instante, nome));
    }
}

/// Inicia a thread do vigia. Registra no log uma vez por travamento e outra quando o laço volta.
pub fn start() {
    let _ = std::thread::Builder::new().name("mide-vigia".into()).spawn(|| {
        let mut parado: Option<(Instant, &'static str)> = None;
        loop {
            std::thread::sleep(Duration::from_millis(250));
            let Some((ultima, fase)) = ULTIMA.lock().ok().and_then(|u| *u) else {
                continue;
            };
            let atraso = ultima.elapsed();
            if atraso >= LIMITE && parado.is_none() {
                tracing::warn!("laço do compositor parado há {:.1} s (fase: {fase}): a tela pode congelar", atraso.as_secs_f32());
                parado = Some((ultima, fase));
            } else if atraso < LIMITE {
                if let Some((desde, fase_parada)) = parado.take() {
                    tracing::warn!(
                        "laço do compositor voltou depois de {:.1} s (parado na fase: {fase_parada})",
                        desde.elapsed().as_secs_f32()
                    );
                }
            }
        }
    });
}
