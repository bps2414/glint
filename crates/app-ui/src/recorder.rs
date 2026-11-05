//! Conversa com o gravador pelo IPC: status para a barra superior e o
//! botão de iniciar quando ele não está rodando.

use std::path::PathBuf;

use app_core::brand;
use app_ipc::{Client, RecorderState, RecorderStatus, Request, Response};

/// Pergunta o status (bloqueante: chamar fora da thread da UI).
/// `None` = gravador não está rodando.
pub fn status() -> Option<RecorderStatus> {
    match Client::call(&Request::Status) {
        Ok(Response::Status(s)) => Some(s),
        _ => None,
    }
}

/// Manda um pedido simples (bloqueante). `Ok` se o gravador respondeu `Ok`.
pub fn send(req: Request) -> Result<(), String> {
    match Client::call(&req) {
        Ok(Response::Ok) => Ok(()),
        Ok(Response::Error(e)) => Err(e),
        Ok(other) => Err(format!("{other:?}")),
        Err(e) => Err(e.to_string()),
    }
}

/// `<slug>-recorder(.exe)` na mesma pasta do app.
pub fn exe() -> Option<PathBuf> {
    let dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let p = dir.join(format!(
        "{}-recorder{}",
        brand::APP_SLUG,
        std::env::consts::EXE_SUFFIX
    ));
    p.exists().then_some(p)
}

/// Sobe o gravador (ele mesmo garante instância única).
pub fn start() -> Result<(), String> {
    let exe = exe().ok_or_else(|| crate::tr("recorder.not_found").to_string())?;
    std::process::Command::new(exe)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Como mostrar o estado na barra superior.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Shown {
    /// Gravador fechado.
    Off,
    Waiting,
    AutoOff,
    Starting,
    Recording(Option<String>),
    Failed,
}

impl Shown {
    pub fn of(s: Option<&RecorderStatus>) -> Self {
        let Some(s) = s else { return Shown::Off };
        match &s.state {
            RecorderState::Idle if s.auto_detect => Shown::Waiting,
            RecorderState::Idle => Shown::AutoOff,
            RecorderState::Starting => Shown::Starting,
            RecorderState::Recording { game } => Shown::Recording(game.clone()),
            RecorderState::Failed { .. } => Shown::Failed,
        }
    }

    pub fn label(&self) -> String {
        use crate::{tr, tr_args};
        match self {
            Shown::Off => tr("recorder.off"),
            Shown::Waiting => tr("recorder.waiting"),
            Shown::AutoOff => tr("recorder.auto_off"),
            Shown::Starting => tr("recorder.starting"),
            Shown::Recording(Some(g)) => tr_args("recorder.recording", &[("game", g)]),
            Shown::Recording(None) => tr("recorder.recording_screen"),
            Shown::Failed => tr("recorder.failed"),
        }
        .to_string()
    }

    pub fn recording(&self) -> bool {
        matches!(self, Shown::Recording(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotulos_do_estado() {
        rust_i18n::set_locale("pt-BR");
        assert_eq!(Shown::of(None).label(), "Gravador fechado");
        let mut s = RecorderStatus::default();
        assert_eq!(Shown::of(Some(&s)).label(), "Esperando jogo");
        s.auto_detect = false;
        assert_eq!(Shown::of(Some(&s)), Shown::AutoOff);
        s.state = RecorderState::Recording {
            game: Some("VALORANT".into()),
        };
        let shown = Shown::of(Some(&s));
        assert!(shown.recording());
        assert_eq!(shown.label(), "Gravando VALORANT");
    }
}
