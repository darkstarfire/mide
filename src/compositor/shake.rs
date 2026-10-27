//! "Agitar o ponteiro para localizá-lo", como no Mac: vários vaivéns rápidos do mouse
//! fazem o cursor crescer por um instante (crescendo e voltando animado), para achar
//! onde ele está. Desligável em Configurações → Efeitos (`[effects] shake_cursor`).

use std::{collections::VecDeque, time::{Duration, Instant}};

/// Quanto o cursor cresce.
pub const MAX_SCALE: f32 = 4.0;
/// Trecho mínimo numa direção para contar como um vaivém e a janela de tempo em que os
/// vaivéns contam.
const MIN_SWING: f64 = 30.0;
const WINDOW: Duration = Duration::from_millis(700);
/// Vaivéns na janela para começar a crescer, e quantos a mais até o tamanho máximo: o
/// cursor cresce acompanhando o chacoalhar (como no Mac), não de uma vez.
const START: usize = 3;
const RAMP: usize = 4;
/// Quanto o cursor segura o tamanho depois do último vaivém.
const HOLD: Duration = Duration::from_millis(300);
/// Rigidez da mola (rad/s) ao crescer e ao voltar: criticamente amortecida, sem passar
/// do ponto. ω = 16 chega perto do alvo em ~300 ms; ω = 10, em ~450 ms (voltar suave).
const OMEGA_GROW: f32 = 16.0;
const OMEGA_SHRINK: f32 = 10.0;
/// Abaixo disto, no voltar, o cursor é o normal (meio pixel de diferença em 24; volta
/// ao plano de cursor da placa sem arrastar a cauda da mola por vários quadros).
const REST: f32 = 1.02;

/// Mola criticamente amortecida: posição e velocidade depois de `t` segundos, saindo de
/// `x` com velocidade `v` em direção a `target`. Forma fechada: o resultado só depende
/// do tempo real, então quadros irregulares (ou lidos várias vezes no mesmo quadro)
/// dão a mesma curva contínua.
fn spring(x: f32, v: f32, target: f32, omega: f32, t: f32) -> (f32, f32) {
    let t = t.max(0.0);
    let e = x - target;
    let c = v + omega * e;
    let decay = (-omega * t).exp();
    (target + (e + c * t) * decay, (v - omega * c * t) * decay)
}

/// Estado da mola no último evento (vaivém novo): daqui ela anda sozinha até o alvo, e
/// depois de `until` volta para 1.
#[derive(Debug, Clone, Copy)]
struct Anchor {
    at: Instant,
    x: f32,
    v: f32,
    target: f32,
    until: Instant,
}

impl Anchor {
    /// Escala e velocidade em `now`.
    fn eval(&self, now: Instant) -> (f32, f32) {
        if now <= self.until {
            return spring(self.x, self.v, self.target, OMEGA_GROW, now.saturating_duration_since(self.at).as_secs_f32());
        }
        // Segurou até `until`; dali em diante volta para 1.
        let held = self.until.saturating_duration_since(self.at).as_secs_f32();
        let (x, v) = spring(self.x, self.v, self.target, OMEGA_GROW, held);
        spring(x, v, 1.0, OMEGA_SHRINK, now.duration_since(self.until).as_secs_f32())
    }
}

#[derive(Debug, Default)]
pub struct Shake {
    last_x: Option<f64>,
    /// Direção do trecho atual (−1, 0, 1) e quanto ele andou.
    dir: i8,
    run: f64,
    flips: VecDeque<Instant>,
    /// Mola do tamanho (nada: cursor normal).
    anchor: Option<Anchor>,
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
        let counted = self.run >= MIN_SWING;
        if counted {
            self.flips.push_back(now);
        }
        self.dir = dir;
        self.run = dx.abs();
        while self.flips.front().is_some_and(|t| now.duration_since(*t) > WINDOW) {
            self.flips.pop_front();
        }
        if !counted || self.flips.len() < START {
            return;
        }
        // Alvo pela intensidade (vaivéns na janela), sem nunca diminuir enquanto chacoalha.
        let level = ((self.flips.len() + 1 - START) as f32 / RAMP as f32).min(1.0);
        let wanted = 1.0 + (MAX_SCALE - 1.0) * level;
        // A mola continua de onde está (crescendo, segurando ou voltando).
        let (x, v, held) = match self.anchor {
            Some(a) => {
                let (x, v) = a.eval(now);
                (x, v, if now <= a.until { a.target } else { 1.0 })
            }
            None => {
                tracing::debug!("ponteiro agitado: cursor crescendo");
                (1.0, 0.0, 1.0)
            }
        };
        self.anchor = Some(Anchor { at: now, x, v, target: wanted.max(held), until: now + HOLD });
    }

    /// Tamanho do cursor agora (1 = normal).
    pub fn scale(&self, now: Instant) -> f32 {
        match self.anchor {
            Some(a) => {
                let (x, _) = a.eval(now);
                if now > a.until && x < REST { 1.0 } else { x.clamp(1.0, MAX_SCALE) }
            }
            None => 1.0,
        }
    }

    /// A mola ainda anda (sem mudar nada): o backend continua pedindo quadros, também
    /// no primeiro instante, quando a escala ainda é quase 1.
    pub fn animating(&self, now: Instant) -> bool {
        self.anchor.is_some_and(|a| now <= a.until || self.scale(now) > 1.0)
    }

    /// Ainda crescido ou animando (o compositor continua pedindo quadros).
    pub fn active(&mut self, now: Instant) -> bool {
        let Some(a) = self.anchor else {
            return false;
        };
        if now > a.until && self.scale(now) <= 1.0 {
            self.anchor = None;
            self.flips.clear();
            return false;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Chacoalha `n` vaivéns de 80 pontos, um a cada `every` ms (cada movimento em 4
    /// passos de 20). Devolve o instante do último movimento.
    fn shake(s: &mut Shake, t0: Instant, n: u64, every: u64) -> Instant {
        let mut x = 500.0;
        let mut last = t0;
        for i in 0..n * 4 {
            let dir = if (i / 4) % 2 == 0 { 1.0 } else { -1.0 };
            x += dir * 20.0;
            last = t0 + Duration::from_millis(i * every / 4);
            s.feed(x, last, true);
        }
        last
    }

    #[test]
    fn vaivens_rapidos_fazem_crescer_e_depois_volta() {
        let mut s = Shake::default();
        let t0 = Instant::now();
        let end = shake(&mut s, t0, 10, 40);
        let now = end + Duration::from_millis(200);
        assert!(s.scale(now) > 3.5, "escala {}", s.scale(now));
        assert!(s.active(now));
        let later = end + HOLD + Duration::from_millis(1500);
        assert!(!s.active(later));
        assert!(!s.animating(later));
        assert_eq!(s.scale(later), 1.0);
    }

    #[test]
    fn cresce_com_a_intensidade() {
        // Poucos vaivéns: cresce pouco; chacoalhando mais, cresce até o máximo.
        let t0 = Instant::now();
        let mut pouco = Shake::default();
        let end = shake(&mut pouco, t0, 4, 40);
        let k_pouco = pouco.scale(end + Duration::from_millis(250));
        let mut muito = Shake::default();
        let end = shake(&mut muito, t0, 10, 40);
        let k_muito = muito.scale(end + Duration::from_millis(250));
        assert!(k_pouco > 1.2 && k_pouco < 2.6, "pouco: {k_pouco}");
        assert!(k_muito > 3.8, "muito: {k_muito}");
    }

    #[test]
    fn movimento_comum_nao_cresce() {
        let mut s = Shake::default();
        let t0 = Instant::now();
        for i in 0..50 {
            s.feed(100.0 + i as f64 * 10.0, t0 + Duration::from_millis(i * 10), true);
        }
        assert_eq!(s.scale(t0 + Duration::from_millis(600)), 1.0);
        assert!(!s.animating(t0 + Duration::from_millis(600)));
        // Desligado: nada.
        let mut off = Shake::default();
        for i in 0..40 {
            off.feed(if (i / 4) % 2 == 0 { i as f64 * 20.0 } else { 0.0 }, t0 + Duration::from_millis(i * 6), false);
        }
        assert_eq!(off.scale(t0 + Duration::from_millis(300)), 1.0);
    }

    /// Lê a escala quadro a quadro com os intervalos dados e confere que ela muda de
    /// forma contínua: nunca mais rápido que a mola permite (velocidade máxima ~ ω·A/e,
    /// ~18/s para ω = 16 e A = 3), sem passar do máximo nem de 1.
    fn continuous(intervals: impl Iterator<Item = u64>, chacoalho: u64) {
        let mut s = Shake::default();
        let t0 = Instant::now();
        let mut now = t0;
        let mut last = s.scale(now);
        let mut feed_x = 500.0;
        let mut feeds = 0;
        let mut peak = 1.0f32;
        for dt in intervals {
            let before = now;
            now += Duration::from_millis(dt);
            // Chacoalha durante os primeiros `chacoalho` ms (vaivéns de 80 a cada 40 ms).
            while feeds * 10 < chacoalho && t0 + Duration::from_millis(feeds * 10) <= now {
                let dir = if (feeds / 4) % 2 == 0 { 1.0 } else { -1.0 };
                feed_x += dir * 20.0;
                s.feed(feed_x, t0 + Duration::from_millis(feeds * 10), true);
                feeds += 1;
            }
            let k = s.scale(now);
            let secs = now.duration_since(before).as_secs_f32();
            assert!((1.0..=MAX_SCALE).contains(&k), "escala fora: {k}");
            assert!((k - last).abs() <= 20.0 * secs + 0.01, "salto {last} -> {k} em {secs} s");
            peak = peak.max(k);
            last = k;
        }
        assert!(peak > 3.5, "não cresceu: {peak}");
        assert_eq!(last, 1.0, "não voltou");
    }

    #[test]
    fn crescer_e_voltar_sem_saltos_a_60_hz() {
        continuous(std::iter::repeat_n(16, 200), 600);
    }

    #[test]
    fn crescer_e_voltar_sem_saltos_com_quadros_irregulares() {
        // Intervalos de 4 a 45 ms, como num DRM com quadros perdidos.
        let pattern = [16u64, 4, 33, 16, 45, 8, 16, 25, 12, 16];
        continuous(pattern.iter().copied().cycle().take(200), 600);
    }

    #[test]
    fn voltar_e_chacoalhar_de_novo_continua_de_onde_esta() {
        let mut s = Shake::default();
        let t0 = Instant::now();
        let end = shake(&mut s, t0, 10, 40);
        // No meio do voltar, chacoalha de novo: cresce a partir dali, sem pular.
        let mid = end + HOLD + Duration::from_millis(150);
        let k = s.scale(mid);
        assert!(k > 1.3 && k < 3.9, "meio do voltar: {k}");
        let mut x = 500.0;
        for i in 0..40u64 {
            let at = mid + Duration::from_millis(i * 10);
            let before = s.scale(at);
            x += if (i / 4) % 2 == 0 { 20.0 } else { -20.0 };
            s.feed(x, at, true);
            assert!((s.scale(at) - before).abs() < 1e-3, "o vaivém mexeu na escala de uma vez");
        }
    }
}
