//! Estado do recorte (início/fim) independente da UI, com as regras de limite.

/// Duração mínima de um recorte, em segundos.
pub const MIN_LEN: f32 = 1.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Trim {
    /// Duração total do clipe.
    pub total: f32,
    pub start: f32,
    pub end: f32,
    /// Posição do player (fica sempre dentro do recorte).
    pub playhead: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle {
    Start,
    End,
}

impl Trim {
    pub fn new(total: f32) -> Self {
        Self {
            total,
            start: 0.0,
            end: total,
            playhead: 0.0,
        }
    }

    /// Move uma alça para o tempo `t`, respeitando os limites e o tamanho mínimo.
    pub fn set(&mut self, handle: Handle, t: f32) {
        let t = t.clamp(0.0, self.total);
        match handle {
            Handle::Start => self.start = t.min(self.end - MIN_LEN).max(0.0),
            Handle::End => self.end = t.max(self.start + MIN_LEN).min(self.total),
        }
        self.playhead = self.playhead.clamp(self.start, self.end);
    }

    /// Move a alça a partir de uma fração (0..1) da largura da linha do tempo.
    pub fn set_fraction(&mut self, handle: Handle, f: f32) {
        self.set(handle, f * self.total);
    }

    pub fn len(&self) -> f32 {
        self.end - self.start
    }

    /// Frações (0..1) de início, fim e player, para desenhar.
    pub fn fractions(&self) -> (f32, f32, f32) {
        let t = self.total.max(f32::EPSILON);
        (self.start / t, self.end / t, self.playhead / t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alcas_respeitam_limites() {
        let mut t = Trim::new(42.0);
        t.set(Handle::Start, -5.0);
        assert_eq!(t.start, 0.0);
        t.set(Handle::End, 99.0);
        assert_eq!(t.end, 42.0);
    }

    #[test]
    fn alcas_nao_se_cruzam() {
        let mut t = Trim::new(42.0);
        t.set(Handle::End, 10.0);
        t.set(Handle::Start, 30.0);
        assert_eq!(t.start, 10.0 - MIN_LEN);
        t.set(Handle::End, 0.0);
        assert_eq!(t.end, t.start + MIN_LEN);
    }

    #[test]
    fn player_fica_dentro_do_recorte() {
        let mut t = Trim::new(42.0);
        t.playhead = 5.0;
        t.set(Handle::Start, 12.4);
        assert_eq!(t.playhead, 12.4);
        t.playhead = 40.0;
        t.set(Handle::End, 31.1);
        assert_eq!(t.playhead, 31.1);
    }

    #[test]
    fn fracao() {
        let mut t = Trim::new(40.0);
        t.set_fraction(Handle::Start, 0.25);
        assert_eq!(t.start, 10.0);
        assert_eq!(t.fractions().0, 0.25);
    }
}
