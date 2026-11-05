//! "Exportar MP4": salva o trecho recortado num arquivo escolhido pelo usuário, numa thread
//! própria, pelo mesmo `app_media::export_mp4` do publicar. Não usa servidor nem conta, e
//! grava num arquivo próprio (nunca no temporário do publicar). A interface lê o andamento a
//! cada 100 ms ([`Export::poll`]); nada aqui toca no GPUI.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use app_media::{ExportOptions, MediaError};

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Running(f32),
    Done,
    Cancelled,
    Failed(String),
}

pub struct Export {
    pub clip_id: i64,
    pub dst: PathBuf,
    pub state: State,
    shared: Arc<Mutex<State>>,
    cancel: Arc<AtomicBool>,
}

impl Export {
    pub fn start(clip_id: i64, src: PathBuf, dst: PathBuf, opts: ExportOptions) -> Export {
        let shared = Arc::new(Mutex::new(State::Running(0.0)));
        let cancel = Arc::new(AtomicBool::new(false));
        let (s, c, out) = (shared.clone(), cancel.clone(), dst.clone());
        std::thread::Builder::new()
            .name("exportar".into())
            .spawn(move || {
                let end = run(&src, &out, &opts, &s, &c);
                *lock(&s) = end;
            })
            .expect("thread de exportar");
        Export {
            clip_id,
            dst,
            state: State::Running(0.0),
            shared,
            cancel,
        }
    }

    /// Atualiza `state`. `true` = mudou.
    pub fn poll(&mut self) -> bool {
        let s = lock(&self.shared).clone();
        if s == self.state {
            return false;
        }
        self.state = s;
        true
    }

    pub fn running(&self) -> bool {
        matches!(self.state, State::Running(_))
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

fn lock(m: &Mutex<State>) -> std::sync::MutexGuard<'_, State> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Grava em `<destino>.part` e renomeia no fim: cancelar ou falhar não deixa arquivo pela
/// metade nem estraga um arquivo que já existia com esse nome.
fn run(
    src: &Path,
    dst: &Path,
    opts: &ExportOptions,
    shared: &Mutex<State>,
    cancel: &AtomicBool,
) -> State {
    let mut name = dst.file_name().unwrap_or_default().to_os_string();
    name.push(".part");
    let part = dst.with_file_name(name);
    let r = app_media::export_mp4(src, &part, opts, &mut |f| {
        *lock(shared) = State::Running(f);
        !cancel.load(Ordering::Relaxed)
    });
    let end = match r {
        Ok(e) => {
            tracing::info!(
                "exportado com {}: {}x{}, {} ms, {} bytes → {}",
                e.encoder,
                e.width,
                e.height,
                e.duration_ms,
                e.size_bytes,
                dst.display()
            );
            match std::fs::rename(&part, dst) {
                Ok(()) => return State::Done,
                Err(e) => State::Failed(e.to_string()),
            }
        }
        Err(MediaError::Cancelled) => State::Cancelled,
        Err(e) => State::Failed(e.to_string()),
    };
    let _ = std::fs::remove_file(&part);
    end
}
