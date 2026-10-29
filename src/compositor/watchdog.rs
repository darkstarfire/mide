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
        *ultima = Some((Instant::now(), "laço (fonte sem fase)"));
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

/// Intervalo entre as conferências do vigia.
const PASSO: Duration = Duration::from_millis(250);

/// O próprio vigia ficou parado (o sono de `PASSO` durou `dormiu`): o processo inteiro
/// estava congelado, não só o laço. É o que acontece na suspensão: o kernel congela
/// todos os programas, e o relógio monotônico continua contando enquanto os aparelhos
/// suspendem e voltam (segundos, às vezes mais de 15). Sem isto, a volta da suspensão
/// aparecia no log como "laço parado há 17.9 s (fase: laço)".
fn processo_congelado(dormiu: Duration) -> bool {
    dormiu >= LIMITE
}

/// Inicia a thread do vigia. Registra no log uma vez por travamento e outra quando o laço volta.
pub fn start() {
    let _ = std::thread::Builder::new().name("mide-vigia".into()).spawn(|| {
        let mut parado: Option<(Instant, &'static str)> = None;
        loop {
            let antes = Instant::now();
            std::thread::sleep(PASSO);
            let dormiu = antes.elapsed();
            if processo_congelado(dormiu) {
                // Recomeça a contagem do laço agora: ele também acabou de descongelar.
                tracing::info!("vigia: o processo ficou congelado {:.1} s (suspensão); a contagem do laço recomeça", dormiu.as_secs_f32());
                if let Ok(mut ultima) = ULTIMA.lock() {
                    if let Some((_, fase)) = *ultima {
                        *ultima = Some((Instant::now(), fase));
                    }
                }
                parado = None;
                continue;
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suspensao_nao_conta_como_laco_parado() {
        // O vigia acordou no tempo: conta normal.
        assert!(!processo_congelado(PASSO));
        assert!(!processo_congelado(Duration::from_millis(400)));
        // O vigia ficou 18 s sem rodar: o processo inteiro estava congelado.
        assert!(processo_congelado(Duration::from_secs(18)));
    }
}
