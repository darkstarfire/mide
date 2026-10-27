//! "Agitar o ponteiro para localizá-lo", como no Mac: vários vaivéns rápidos do mouse
//! fazem o cursor crescer por um instante (crescendo e voltando animado), para achar
//! onde ele está. Desligável em Configurações → Efeitos (`[effects] shake_cursor`).

use std::{collections::VecDeque, time::{Duration, Instant}};

/// Quanto o cursor cresce.
pub const MAX_SCALE: f32 = 4.0;
/// Trecho mínimo numa direção para contar como um vaivém, a janela de tempo e quantos
/// vaivéns nela ligam o efeito.
const MIN_SWING: f64 = 30.0;
const WINDOW: Duration = Duration::from_millis(700);
const FLIPS: usize = 4;
/// Quanto o cursor fica grande depois do último vaivém, e o crescer e o voltar.
const HOLD: Duration = Duration::from_millis(500);
const GROW: Duration = Duration::from_millis(300);
const SHRINK: Duration = Duration::from_millis(420);

/// Crescer: começa e termina devagar, como o voltar (sem o salto do primeiro quadro).
fn ease_grow(t: f32) -> f32 {
    ease_shrink(t)
}

/// O inverso de `ease_grow` (de quanto já cresceu, em que ponto do tempo está), por
/// bisseção (a curva é crescente).
fn ease_grow_inv(p: f32) -> f32 {
    let p = p.clamp(0.0, 1.0);
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    for _ in 0..24 {
        let mid = (lo + hi) / 2.0;
        if ease_grow(mid) < p { lo = mid } else { hi = mid }
    }
    (lo + hi) / 2.0
}

/// Voltar: começa e termina devagar (cúbica de entrada e saída), sem tranco.
fn ease_shrink(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 { 4.0 * t * t * t } else { 1.0 - (-2.0 * t + 2.0).powi(3) / 2.0 }
}

#[derive(Debug, Default)]
pub struct Shake {
    last_x: Option<f64>,
    /// Direção do trecho atual (−1, 0, 1) e quanto ele andou.
    dir: i8,
    run: f64,
    flips: VecDeque<Instant>,
    /// Desde quando está crescido e até quando segura (cada vaivém estende).
    since: Option<Instant>,
    until: Option<Instant>,
    /// Escala no começo do voltar (para voltar de onde parou).
    shrink_from: Option<(Instant, f32)>,
}

impl Shake {
    /// Um movimento do ponteiro (só o x conta: o chacoalhar é de um lado para o outro).
    pub fn feed(&mut self, x: f64, now: Instant, enabled: bool) {
        let Some(last) = self.last_x.replace(x) else {
            return;
        };
        if !enabled {
            return;
        }
        let dx = x - last;
        if dx.abs() < 0.5 {
            return;
        }
        let dir = if dx > 0.0 { 1 } else { -1 };
        if dir == self.dir {
            self.run += dx.abs();
            return;
        }
        // Virou: conta se o trecho anterior foi longo o bastante.
        if self.run >= MIN_SWING {
            self.flips.push_back(now);
        }
        self.dir = dir;
        self.run = dx.abs();
        while self.flips.front().is_some_and(|t| now.duration_since(*t) > WINDOW) {
            self.flips.pop_front();
        }
        if self.flips.len() >= FLIPS {
            if self.since.is_none() || self.shrink_from.is_some() {
                let current = self.scale(now);
                // Começa a crescer de onde estiver (pode estar voltando).
                self.since = Some(now - GROW.mul_f32(ease_grow_inv((current - 1.0) / (MAX_SCALE - 1.0))));
                self.shrink_from = None;
                tracing::debug!("ponteiro agitado: cursor grande");
            }
            self.until = Some(now + HOLD);
        }
    }

    /// Tamanho do cursor agora (1 = normal).
    pub fn scale(&self, now: Instant) -> f32 {
        let Some(since) = self.since else {
            return 1.0;
        };
        match self.until {
            Some(until) if now < until => {
                let t = now.duration_since(since).as_secs_f32() / GROW.as_secs_f32();
                1.0 + (MAX_SCALE - 1.0) * ease_grow(t)
            }
            Some(until) => {
                let grown = 1.0 + (MAX_SCALE - 1.0) * ease_grow(until.duration_since(since).as_secs_f32() / GROW.as_secs_f32());
                let t = now.duration_since(until).as_secs_f32() / SHRINK.as_secs_f32();
                1.0 + (grown - 1.0) * (1.0 - ease_shrink(t))
            }
            None => 1.0,
        }
    }

    /// Ainda crescido ou animando (o compositor continua pedindo quadros).
    pub fn active(&mut self, now: Instant) -> bool {
        match self.until {
            Some(until) if now >= until + SHRINK => {
                self.since = None;
                self.until = None;
                self.flips.clear();
                false
            }
            Some(_) => true,
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vaivens_rapidos_fazem_crescer_e_depois_volta() {
        let mut s = Shake::default();
        let t0 = Instant::now();
        let mut x = 500.0;
        // Cinco vaivéns de 80 pontos em ~250 ms.
        for i in 0..40 {
            let dir = if (i / 4) % 2 == 0 { 1.0 } else { -1.0 };
            x += dir * 20.0;
            s.feed(x, t0 + Duration::from_millis(i * 6), true);
        }
        let now = t0 + Duration::from_millis(240 + 200);
        assert!(s.scale(now) > 3.5, "escala {}", s.scale(now));
        assert!(s.active(now));
        let later = now + HOLD + SHRINK + Duration::from_millis(50);
        assert!(!s.active(later));
        assert_eq!(s.scale(later), 1.0);
    }

    #[test]
    fn movimento_comum_nao_cresce() {
        let mut s = Shake::default();
        let t0 = Instant::now();
        for i in 0..50 {
            s.feed(100.0 + i as f64 * 10.0, t0 + Duration::from_millis(i * 10), true);
        }
        assert_eq!(s.scale(t0 + Duration::from_millis(600)), 1.0);
        // Desligado: nada.
        let mut off = Shake::default();
        for i in 0..40 {
            off.feed(if (i / 4) % 2 == 0 { i as f64 * 20.0 } else { 0.0 }, t0 + Duration::from_millis(i * 6), false);
        }
        assert_eq!(off.scale(t0 + Duration::from_millis(300)), 1.0);
    }

    #[test]
    fn crescer_e_voltar_sem_saltos() {
        let mut s = Shake::default();
        let t0 = Instant::now();
        let mut x = 500.0;
        for i in 0..40 {
            let dir = if (i / 4) % 2 == 0 { 1.0 } else { -1.0 };
            x += dir * 20.0;
            s.feed(x, t0 + Duration::from_millis(i * 6), true);
        }
        // Quadro a quadro (60 por segundo), a escala nunca pula mais que um pedaço
        // pequeno: a animação é contínua no crescer, no segurar e no voltar.
        let mut last = s.scale(t0);
        for f in 0..90 {
            let now = t0 + Duration::from_millis(f * 16);
            let k = s.scale(now);
            assert!((k - last).abs() < 0.5, "quadro {f}: {last} -> {k}");
            last = k;
        }
        // O inverso da curva de crescer.
        for p in [0.0, 0.3, 0.7, 1.0] {
            assert!((ease_grow(ease_grow_inv(p)) - p).abs() < 1e-4);
        }
    }
}
