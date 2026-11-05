// No Linux o daemon não existe ainda; o módulo fica só pelos testes.
#![cfg_attr(not(windows), allow(dead_code))]

//! Decisões do daemon que não dependem do SO (testáveis no Linux):
//! o que deveria estar gravando agora e quando tentar de novo após falha.

use std::time::{Duration, Instant};

use app_core::game::GameInfo;

/// O que o gravador deveria estar fazendo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Want {
    Nothing,
    /// Usuário pediu "gravar a tela agora".
    Manual,
    /// Gravar o jogo com este pid.
    Game(u32),
}

/// - Manual vence tudo (o usuário pediu explicitamente).
/// - Jogo detectado grava se a detecção está ligada e o usuário não parou
///   a gravação deste jogo (`paused_pid`).
pub fn want(
    manual: bool,
    auto_detect: bool,
    game: Option<&GameInfo>,
    paused_pid: Option<u32>,
) -> Want {
    if manual {
        return Want::Manual;
    }
    match game {
        Some(g) if auto_detect && paused_pid != Some(g.pid) => Want::Game(g.pid),
        _ => Want::Nothing,
    }
}

/// Espera crescente entre tentativas de (re)abrir a captura/encoder.
#[derive(Clone, Debug, Default)]
pub struct Backoff {
    failures: u32,
    next: Option<Instant>,
}

const DELAYS_SECS: &[u64] = &[2, 5, 15, 30, 60];

impl Backoff {
    pub fn ready(&self, now: Instant) -> bool {
        self.next.is_none_or(|t| now >= t)
    }

    /// Registra uma falha; devolve quanto vai esperar.
    pub fn fail(&mut self, now: Instant) -> Duration {
        let i = (self.failures as usize).min(DELAYS_SECS.len() - 1);
        let d = Duration::from_secs(DELAYS_SECS[i]);
        self.failures += 1;
        self.next = Some(now + d);
        d
    }

    pub fn failures(&self) -> u32 {
        self.failures
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game(pid: u32) -> GameInfo {
        GameInfo {
            pid,
            exe: "game.exe".into(),
            name: "Game".into(),
        }
    }

    #[test]
    fn wants() {
        let g = game(7);
        assert_eq!(want(false, true, Some(&g), None), Want::Game(7));
        assert_eq!(want(false, false, Some(&g), None), Want::Nothing);
        assert_eq!(want(false, true, Some(&g), Some(7)), Want::Nothing);
        assert_eq!(want(false, true, Some(&g), Some(8)), Want::Game(7));
        assert_eq!(want(true, true, Some(&g), None), Want::Manual);
        assert_eq!(want(false, true, None, None), Want::Nothing);
    }

    #[test]
    fn backoff_grows_and_caps() {
        let t0 = Instant::now();
        let mut b = Backoff::default();
        assert!(b.ready(t0));
        assert_eq!(b.fail(t0), Duration::from_secs(2));
        assert!(!b.ready(t0 + Duration::from_secs(1)));
        assert!(b.ready(t0 + Duration::from_secs(2)));
        for _ in 0..10 {
            b.fail(t0);
        }
        assert_eq!(b.fail(t0), Duration::from_secs(60));
        b.reset();
        assert!(b.ready(t0));
        assert_eq!(b.failures(), 0);
    }
}
