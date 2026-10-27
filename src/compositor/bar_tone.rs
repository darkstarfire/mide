//! Tom do que está atrás da barra superior (claro ou escuro), para a barra sem fundo
//! (Tahoe) escolher letras e ícones brancos ou pretos na hora.
//!
//! A cena mede o brilho médio da faixa da barra desenhando só o que está atrás dela
//! (papel de parede, janelas, dock), numa textura, e lendo os pixels. A medida só é
//! refeita quando a assinatura do que está atrás muda e no máximo a cada
//! `MIN_INTERVAL` (um vídeo atrás da barra não vira uma leitura por quadro). Dois
//! limiares (histerese) evitam piscar quando o brilho fica perto do meio.

use std::time::{Duration, Instant};

/// Abaixo disso (brilho de 0 a 255) o fundo passa a escuro.
pub const DARK_BELOW: f32 = 128.0;
/// Acima disso o fundo passa a claro.
pub const LIGHT_ABOVE: f32 = 156.0;
/// Primeira medida (sem estado anterior): o meio dos dois.
const FIRST_SPLIT: f32 = (DARK_BELOW + LIGHT_ABOVE) / 2.0;
/// Intervalo mínimo entre duas leituras da mesma tela.
pub const MIN_INTERVAL: Duration = Duration::from_millis(120);

/// Novo estado (escuro?) depois de uma medida, com histerese.
pub fn decide(previous: Option<bool>, luma: f32) -> bool {
    match previous {
        Some(true) => luma < LIGHT_ABOVE,
        Some(false) => luma < DARK_BELOW,
        None => luma < FIRST_SPLIT,
    }
}

/// Brilho médio (0 a 255, pesos do Rec. 601) de pixels RGBA, pulando `step` pixels
/// entre amostras (amostrar em resolução baixa). Pixels transparentes contam como o
/// fundo do quadro (escuro), como a tela os mostra.
pub fn mean_luma(rgba: &[u8], step: usize) -> Option<f32> {
    let step = step.max(1);
    let mut sum = 0u64;
    let mut n = 0u64;
    for px in rgba.as_chunks::<4>().0.iter().step_by(step) {
        sum += (px[0] as u64 * 299 + px[1] as u64 * 587 + px[2] as u64 * 114) / 1000;
        n += 1;
    }
    (n > 0).then(|| sum as f32 / n as f32)
}

/// Controle de quando medir uma tela: a assinatura do que estava atrás na última
/// medida e quando ela foi feita.
#[derive(Default)]
pub struct Probe {
    key: Option<u64>,
    at: Option<Instant>,
    /// Mudou algo atrás da barra, mas a medida ficou para depois (intervalo mínimo).
    pub pending: bool,
}

impl Probe {
    /// Precisa medir agora? (O que está atrás mudou e o intervalo mínimo passou.)
    pub fn due(&mut self, key: u64, now: Instant) -> bool {
        if self.key == Some(key) {
            self.pending = false;
            return false;
        }
        if self.at.is_some_and(|at| now.duration_since(at) < MIN_INTERVAL) {
            self.pending = true;
            return false;
        }
        true
    }

    /// Medida feita (com sucesso ou não: não insiste a cada quadro).
    pub fn done(&mut self, key: u64, now: Instant) {
        self.key = Some(key);
        self.at = Some(now);
        self.pending = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn histerese_nao_pisca_perto_do_meio() {
        // Começa claro; um brilho no meio (entre os limiares) não muda nada.
        assert!(!decide(None, 200.0));
        assert!(!decide(Some(false), 140.0));
        assert!(decide(Some(false), 100.0));
        // Escuro continua escuro até passar do limiar de cima.
        assert!(decide(Some(true), 150.0));
        assert!(!decide(Some(true), 170.0));
        // Primeira medida: o meio decide.
        assert!(decide(None, FIRST_SPLIT - 1.0));
        assert!(!decide(None, FIRST_SPLIT + 1.0));
    }

    #[test]
    fn media_do_brilho() {
        let white = [255u8, 255, 255, 255].repeat(10);
        let black = [0u8, 0, 0, 255].repeat(10);
        assert_eq!(mean_luma(&white, 1), Some(255.0));
        assert_eq!(mean_luma(&black, 3), Some(0.0));
        let mut half = white.clone();
        half.extend_from_slice(&black);
        assert!((mean_luma(&half, 1).unwrap() - 127.5).abs() < 0.01);
        // Verde pesa mais que azul (Rec. 601).
        let green = mean_luma(&[0, 255, 0, 255], 1).unwrap();
        let blue = mean_luma(&[0, 0, 255, 255], 1).unwrap();
        assert!(green > 140.0 && blue < 30.0);
        assert_eq!(mean_luma(&[], 1), None);
    }

    #[test]
    fn mede_so_quando_muda_e_respeita_o_intervalo() {
        let t0 = Instant::now();
        let mut p = Probe::default();
        assert!(p.due(1, t0));
        p.done(1, t0);
        assert!(!p.due(1, t0 + Duration::from_millis(10)));
        // Mudou cedo demais: fica pendente.
        assert!(!p.due(2, t0 + Duration::from_millis(50)));
        assert!(p.pending);
        assert!(p.due(2, t0 + MIN_INTERVAL + Duration::from_millis(1)));
        p.done(2, t0 + MIN_INTERVAL + Duration::from_millis(1));
        assert!(!p.pending);
    }
}
