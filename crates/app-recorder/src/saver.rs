//! Salvamento em segundo plano (uma thread, um clipe por vez):
//! MP4 por remux → "blip" → miniatura → registro no SQLite → limite de disco.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use app_core::library::{ClipKind, Library, NewClip};
use app_core::{naming, Codec, Config, ReplaySnapshot};
use app_media::{mux, StreamInfo};
use app_platform_win::{power, sound};

/// Largura das miniaturas (cards da biblioteca em telas 2x).
const THUMB_WIDTH: u32 = 640;

pub struct ClipMeta {
    pub game: Option<String>,
    pub game_exe: Option<String>,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub codec: Codec,
    pub kind: ClipKind,
    /// Apertos do combo (só clipes de kill).
    pub kills: Option<u32>,
    /// Instante da miniatura dentro do clipe. `None` = ponto padrão (perto do fim).
    pub thumb_at_ms: Option<i64>,
}

pub struct SaveJob {
    pub snapshot: ReplaySnapshot,
    pub streams: Arc<Vec<StreamInfo>>,
    pub meta: ClipMeta,
    pub clips_dir: PathBuf,
    pub disk_limit_bytes: u64,
    /// Toca o blip ao terminar de salvar.
    pub play_sound: bool,
    /// Toca o blip de erro se o salvamento falhar.
    pub error_sound: bool,
}

#[derive(Debug)]
pub enum SaveOutcome {
    Saved {
        path: PathBuf,
        secs: f64,
        mb: f64,
        ms: u128,
        /// Total ocupado pelos clipes da biblioteca depois do salvamento/limpeza.
        disk_used: u64,
        removed: usize,
    },
    Failed(String),
    /// Uso de disco inicial (ao abrir a biblioteca).
    Usage(u64),
}

enum Msg {
    Job(Box<SaveJob>),
    Quit,
}

pub struct Saver {
    tx: Sender<Msg>,
    handle: Option<JoinHandle<()>>,
}

impl Saver {
    /// `report` recebe o resultado de cada trabalho (e o uso inicial do disco).
    pub fn start(report: impl Fn(SaveOutcome) + Send + 'static) -> Result<Self> {
        let (tx, rx) = std::sync::mpsc::channel::<Msg>();
        let handle = std::thread::Builder::new()
            .name("saver".into())
            .spawn(move || {
                power::set_current_thread_background();
                worker(rx, report)
            })?;
        Ok(Self {
            tx,
            handle: Some(handle),
        })
    }

    pub fn save(&self, job: SaveJob) {
        let _ = self.tx.send(Msg::Job(Box::new(job)));
    }
}

impl Drop for Saver {
    fn drop(&mut self) {
        // Termina os clipes já na fila antes de sair.
        let _ = self.tx.send(Msg::Quit);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn open_library() -> Option<Library> {
    let path = Config::library_path();
    match Library::open(&path) {
        Ok(lib) => Some(lib),
        Err(e) => {
            tracing::error!("biblioteca indisponível ({}): {e}", path.display());
            None
        }
    }
}

fn worker(rx: Receiver<Msg>, report: impl Fn(SaveOutcome)) {
    let lib = open_library();
    if let Some(lib) = &lib {
        // Só esquece clipes cuja pasta existe (HD externo desconectado não apaga a biblioteca).
        let keep = |p: &Path| p.exists() || !p.parent().is_some_and(|d| d.exists());
        match lib.forget_missing(keep) {
            Ok(gone) if !gone.is_empty() => {
                tracing::info!(
                    n = gone.len(),
                    "clipes apagados fora do app saíram da biblioteca"
                );
                for c in gone {
                    if let Some(t) = c.thumb_path {
                        let _ = std::fs::remove_file(t);
                    }
                }
            }
            Ok(_) => {}
            Err(e) => tracing::warn!("conferindo biblioteca: {e}"),
        }
        report(SaveOutcome::Usage(lib.total_bytes().unwrap_or(0)));
    }
    while let Ok(Msg::Job(job)) = rx.recv() {
        match save(&job, lib.as_ref()) {
            Ok(outcome) => report(outcome),
            Err(e) => {
                tracing::error!("falha ao salvar clipe: {e:#}");
                if job.error_sound {
                    sound::play_error_blip();
                }
                report(SaveOutcome::Failed(format!("{e:#}")));
            }
        }
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn save(job: &SaveJob, lib: Option<&Library>) -> Result<SaveOutcome> {
    let t = Instant::now();
    std::fs::create_dir_all(&job.clips_dir)
        .with_context(|| format!("criando {}", job.clips_dir.display()))?;
    let stamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string();
    let kill = job.meta.kind == ClipKind::Kill;
    let stem = naming::clip_stem(job.meta.game.as_deref(), kill, &stamp);
    let path = naming::unique_path(&job.clips_dir, &stem, "mp4");

    let refs: Vec<&StreamInfo> = job.streams.iter().collect();
    mux::write_mp4(&path, &refs, &job.snapshot).context("gravando MP4")?;
    if job.play_sound {
        sound::play_save_blip();
    }
    let ms = t.elapsed().as_millis();
    let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    let duration_ms = job.snapshot.duration_us() / 1000;
    let secs = duration_ms as f64 / 1000.0;
    tracing::info!(
        arquivo = %path.display(),
        duracao_s = format!("{secs:.1}"),
        mb = format!("{:.1}", size as f64 / 1e6),
        ms = ms as u64,
        "clipe salvo"
    );

    let thumb = make_thumbnail(&path, duration_ms, job.meta.thumb_at_ms);
    let mut disk_used = 0;
    let mut removed = 0;
    if let Some(lib) = lib {
        let clip = NewClip {
            path: path.clone(),
            created_ms: now_ms(),
            duration_ms,
            size_bytes: size,
            width: job.meta.width,
            height: job.meta.height,
            fps: job.meta.fps,
            codec: match job.meta.codec {
                Codec::Hevc => "hevc".into(),
                Codec::H264 => "h264".into(),
            },
            audio_tracks: job.streams.len().saturating_sub(1) as u32,
            game: job.meta.game.clone(),
            game_exe: job.meta.game_exe.clone(),
            thumb_path: thumb,
            kind: job.meta.kind,
            kills: job.meta.kills,
        };
        if let Err(e) = lib.insert(&clip) {
            tracing::warn!("clipe salvo mas não registrado na biblioteca: {e}");
        }
        removed = enforce_limit(lib, job.disk_limit_bytes);
        disk_used = lib.total_bytes().unwrap_or(0);
    }
    Ok(SaveOutcome::Saved {
        path,
        secs,
        mb: size as f64 / 1e6,
        ms,
        disk_used,
        removed,
    })
}

fn make_thumbnail(clip: &Path, duration_ms: i64, at_ms: Option<i64>) -> Option<PathBuf> {
    let name = clip.file_stem()?.to_string_lossy().into_owned();
    let out = Config::thumbs_dir().join(format!("{name}.jpg"));
    let at = match at_ms {
        Some(ms) => ms.clamp(0, duration_ms.max(0)),
        None => app_media::thumbnail_time_ms(duration_ms),
    };
    match app_media::thumbnail(clip, at, &out, THUMB_WIDTH) {
        Ok(_) => Some(out),
        Err(e) => {
            tracing::warn!("sem miniatura para {}: {e}", clip.display());
            None
        }
    }
}

/// Apaga os clipes antigos não protegidos até caber no limite. `0` = sem limite.
fn enforce_limit(lib: &Library, limit: u64) -> usize {
    if limit == 0 {
        return 0;
    }
    let plan = match lib.cleanup_plan(limit) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("limite de disco: {e}");
            return 0;
        }
    };
    let mut removed = 0;
    for c in plan {
        match std::fs::remove_file(&c.path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                // Arquivo aberto em outro programa etc.: tenta de novo no próximo clipe.
                tracing::warn!("não foi possível apagar {}: {e}", c.path.display());
                continue;
            }
        }
        if let Some(t) = &c.thumb_path {
            let _ = std::fs::remove_file(t);
        }
        if lib.remove(c.id).is_ok() {
            removed += 1;
            tracing::info!(arquivo = %c.path.display(), "clipe antigo apagado (limite de disco)");
        }
    }
    removed
}
