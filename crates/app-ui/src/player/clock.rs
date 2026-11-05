//! Relógio de reprodução (relógio de parede por enquanto; quando o player
//! tiver áudio, a posição passa a vir das amostras tocadas).

use std::time::Instant;

#[derive(Clone, Debug)]
pub struct Clock {
    /// Posição (ms) no instante `anchor`.
    base_ms: f64,
    /// `None` = parado ou esperando o primeiro quadro depois de um seek.
    anchor: Option<Instant>,
    playing: bool,
}

impl Clock {
    pub fn new(playing: bool) -> Self {
        Self {
            base_ms: 0.0,
            anchor: None,
            playing,
        }
    }

    pub fn playing(&self) -> bool {
        self.playing
    }

    /// Posição atual em ms.
    pub fn position(&self, now: Instant) -> f64 {
        match self.anchor {
            Some(a) if self.playing => {
                self.base_ms + now.saturating_duration_since(a).as_secs_f64() * 1000.0
            }
            _ => self.base_ms,
        }
    }

    /// Seek: congela em `ms` até [`Self::start_at`] (chegou o quadro do novo ponto).
    pub fn seek(&mut self, ms: f64) {
        self.base_ms = ms.max(0.0);
        self.anchor = None;
    }

    /// Chegou o primeiro quadro (em `ms`): o relógio anda a partir dele.
    pub fn start_at(&mut self, ms: f64, now: Instant) {
        self.base_ms = ms.max(0.0);
        self.anchor = Some(now);
    }

    pub fn play(&mut self, now: Instant) {
        if !self.playing {
            self.playing = true;
            self.anchor = Some(now);
        }
    }

    pub fn pause(&mut self, now: Instant) {
        if self.playing {
            self.base_ms = self.position(now);
            self.playing = false;
            self.anchor = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::Clock;

    #[test]
    fn anda_so_tocando_e_depois_do_primeiro_quadro() {
        let t0 = Instant::now();
        let ms = |n| t0 + Duration::from_millis(n);
        let mut c = Clock::new(true);
        // Esperando o primeiro quadro: parado.
        assert_eq!(c.position(ms(500)), 0.0);
        c.start_at(0.0, ms(500));
        assert!((c.position(ms(1500)) - 1000.0).abs() < 0.5);

        c.pause(ms(1500));
        assert!((c.position(ms(9000)) - 1000.0).abs() < 0.5);
        c.play(ms(10_000));
        assert!((c.position(ms(10_250)) - 1250.0).abs() < 0.5);

        // Seek congela até o quadro novo.
        c.seek(30_000.0);
        assert_eq!(c.position(ms(11_000)), 30_000.0);
        c.start_at(30_010.0, ms(11_000));
        assert!((c.position(ms(11_100)) - 30_110.0).abs() < 0.5);

        // Seek pausado: só muda a posição.
        c.pause(ms(11_100));
        c.seek(5_000.0);
        assert_eq!(c.position(ms(20_000)), 5_000.0);
    }
}
