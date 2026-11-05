//! Publicar em segundo plano, numa thread própria: login (se preciso) →
//! exportar o trecho em H.264 → enviar em partes → concluir. A interface lê o
//! estado a cada 100 ms ([`Job::poll`]); nada aqui toca no GPUI.
//!
//! Se falhar, o MP4 exportado e o id no servidor ficam guardados no [`Job`]:
//! "Tentar de novo" com o mesmo recorte e as mesmas trilhas só envia as partes
//! que faltam.
//!
//! [`Job::set_paused`] segura a exportação e o envio entre um passo e outro (jogo aberto);
//! a interface liga e desliga conforme o status do gravador.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use app_cloud::{login, Client, CloudError, NewClip, Stage, Visibility};
use app_media::{ExportOptions, Exported, MediaError};

use crate::tr_args;

/// Tempo para o usuário entrar no navegador.
const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);
/// Miniatura do link (o servidor aceita até 1 MB).
const THUMB_MAX_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, PartialEq)]
pub enum Phase {
    /// Esperando o usuário entrar com o Discord no navegador.
    Login,
    Export(f32),
    Upload {
        sent: u64,
        total: u64,
    },
    Finishing,
    Done(String),
    Failed(Failure),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Failure {
    Cancelled,
    /// Token vencido/revogado: refazer o login.
    Session,
    /// O servidor inteiro está cheio: os próximos também falhariam.
    StorageFull {
        used_bytes: u64,
        limit_bytes: u64,
    },
    Error(String),
}

impl Phase {
    pub fn running(&self) -> bool {
        !matches!(self, Phase::Done(_) | Phase::Failed(_))
    }
}

pub struct Request {
    pub clip_id: i64,
    pub src: PathBuf,
    pub opts: ExportOptions,
    pub title: String,
    pub game: Option<String>,
    /// Quem pode ver (escolhido antes de publicar).
    pub visibility: Visibility,
    pub base: String,
    pub token: Option<String>,
    /// Login já escutando (o navegador já foi aberto) quando não há token.
    pub login: Option<login::Pending>,
    /// Página mostrada na aba do navegador depois do login.
    pub login_page: String,
}

#[derive(Default)]
struct Shared {
    phase: Option<Phase>,
    /// Clipe criado no servidor (para retomar ou apagar o envio pela metade).
    server_id: Option<String>,
    /// Token novo, do login feito nesta publicação.
    token: Option<String>,
    /// O que já está exportado em `file`.
    exported: Option<(ExportOptions, Exported)>,
    /// A thread está parada esperando o `pause` desligar.
    waiting: bool,
}

pub struct Job {
    pub clip_id: i64,
    pub phase: Phase,
    pub login_url: Option<String>,
    /// Parado de verdade por causa do jogo (a thread chegou num ponto de pausa).
    pub paused: bool,
    /// "Enviar mesmo assim": não pausa mais por jogo aberto.
    pub forced: bool,
    shared: Arc<Mutex<Shared>>,
    cancel: Arc<AtomicBool>,
    pause: Arc<AtomicBool>,
    file: PathBuf,
    base: String,
}

impl Job {
    /// Começa a publicar. `previous` (falhou antes, mesmo clipe) é reaproveitado.
    /// `paused` = já começa segurado (jogo aberto).
    pub fn start(req: Request, dir: &Path, previous: Option<Job>, paused: bool) -> Job {
        let file = dir.join(format!("publicar-{}.mp4", req.clip_id));
        let mut shared = Shared::default();
        if let Some(prev) = previous {
            let mut old = prev.take_shared();
            if prev.clip_id == req.clip_id
                && old.exported.as_ref().is_some_and(|(o, _)| *o == req.opts)
            {
                shared.exported = old.exported.take();
                shared.server_id = old.server_id.take();
            } else {
                prev.discard_with(old);
            }
        }
        let shared = Arc::new(Mutex::new(shared));
        let cancel = Arc::new(AtomicBool::new(false));
        let pause = Arc::new(AtomicBool::new(paused));
        let job = Job {
            clip_id: req.clip_id,
            phase: if req.login.is_some() {
                Phase::Login
            } else {
                Phase::Export(0.0)
            },
            login_url: req.login.as_ref().map(|p| p.url.clone()),
            paused: false,
            forced: false,
            shared: shared.clone(),
            cancel: cancel.clone(),
            pause: pause.clone(),
            file: file.clone(),
            base: req.base.clone(),
        };
        std::thread::Builder::new()
            .name("publicar".into())
            .spawn(move || {
                let gate = Gate {
                    cancel: &cancel,
                    pause: &pause,
                    shared: &shared,
                };
                let end = match run(req, &file, &gate) {
                    Ok(url) => {
                        // O arquivo publicado não precisa ficar no disco.
                        let _ = std::fs::remove_file(&file);
                        Phase::Done(url)
                    }
                    Err(f) => Phase::Failed(f),
                };
                lock(&shared).phase = Some(end);
            })
            .expect("thread de publicar");
        job
    }

    /// Atualiza `phase`. Devolve (mudou?, token novo).
    pub fn poll(&mut self) -> (bool, Option<String>) {
        let mut s = lock(&self.shared);
        let token = s.token.take();
        let mut changed = match &s.phase {
            Some(p) if *p != self.phase => {
                self.phase = p.clone();
                true
            }
            _ => false,
        };
        if s.waiting != self.paused {
            self.paused = s.waiting;
            changed = true;
        }
        (changed, token)
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Segura (ou solta) a exportação e o envio no próximo ponto de pausa.
    pub fn set_paused(&self, on: bool) {
        self.pause.store(on, Ordering::Relaxed);
    }

    fn take_shared(&self) -> Shared {
        std::mem::take(&mut *lock(&self.shared))
    }

    /// Desiste de vez: apaga o MP4 exportado e o envio pela metade no servidor.
    pub fn discard(self, token: Option<String>) {
        let mut s = self.take_shared();
        if s.token.is_none() {
            s.token = token;
        }
        self.discard_with(s);
    }

    fn discard_with(&self, s: Shared) {
        self.cancel();
        let _ = std::fs::remove_file(&self.file);
        if let (Some(id), Some(token)) = (s.server_id, s.token.or_else(crate::secret::load)) {
            let base = self.base.clone();
            std::thread::spawn(move || {
                let _ = Client::new(&base, Some(token)).delete_clip(&id);
            });
        }
    }
}

fn lock(m: &Mutex<Shared>) -> std::sync::MutexGuard<'_, Shared> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Cancelar e pausar, vistos pela thread.
struct Gate<'a> {
    cancel: &'a AtomicBool,
    pause: &'a AtomicBool,
    shared: &'a Mutex<Shared>,
}

impl Gate<'_> {
    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// Espera enquanto pausado. `false` = cancelaram (durante a espera ou antes).
    fn go(&self) -> bool {
        if self.pause.load(Ordering::Relaxed) {
            lock(self.shared).waiting = true;
            while self.pause.load(Ordering::Relaxed) && !self.cancelled() {
                std::thread::sleep(Duration::from_millis(200));
            }
            lock(self.shared).waiting = false;
        }
        !self.cancelled()
    }
}

fn run(req: Request, file: &Path, gate: &Gate) -> Result<String, Failure> {
    let shared = gate.shared;
    let set = |p: Phase| lock(shared).phase = Some(p);
    let cancelled = || gate.cancelled();
    let cloud = |e: CloudError| match e {
        CloudError::Cancelled => Failure::Cancelled,
        CloudError::Unauthorized => Failure::Session,
        CloudError::StorageFull {
            used_bytes,
            limit_bytes,
        } => Failure::StorageFull {
            used_bytes,
            limit_bytes,
        },
        e => Failure::Error(e.to_string()),
    };

    // 1. Conta: login pelo navegador na primeira vez.
    let token = match (req.token, req.login) {
        (Some(t), _) => t,
        (None, Some(pending)) => {
            set(Phase::Login);
            let code = pending
                .wait(LOGIN_TIMEOUT, &req.login_page, &cancelled)
                .map_err(cloud)?;
            let t = Client::new(&req.base, None)
                .exchange_code(&code, &pending.verifier)
                .map_err(cloud)?;
            lock(shared).token = Some(t.clone());
            t
        }
        (None, None) => return Err(Failure::Session),
    };
    let client = Client::new(&req.base, Some(token));

    // 2. Exportar o trecho (pula se o mesmo recorte já está pronto).
    let ready = lock(shared)
        .exported
        .as_ref()
        .filter(|(o, _)| *o == req.opts && file.exists())
        .map(|(_, e)| e.clone());
    let exported = match ready {
        Some(e) => e,
        None => {
            set(Phase::Export(0.0));
            if !gate.go() {
                return Err(Failure::Cancelled);
            }
            if let Some(dir) = file.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let r = app_media::export_mp4(&req.src, file, &req.opts, &mut |f| {
                set(Phase::Export(f));
                gate.go()
            });
            match r {
                Ok(e) => {
                    tracing::info!(
                        "exportado com {}: {}x{}, {} ms, {} bytes",
                        e.encoder,
                        e.width,
                        e.height,
                        e.duration_ms,
                        e.size_bytes
                    );
                    let mut s = lock(shared);
                    s.exported = Some((req.opts.clone(), e.clone()));
                    s.server_id = None;
                    e
                }
                Err(MediaError::Cancelled) => return Err(Failure::Cancelled),
                Err(e) => {
                    return Err(Failure::Error(
                        tr_args("publish.export_error", &[("error", &e.to_string())]).to_string(),
                    ))
                }
            }
        }
    };

    // 3. Enviar.
    let thumb = make_thumb(file, exported.duration_ms);
    let meta = NewClip {
        title: req.title,
        game: req.game,
        visibility: req.visibility,
        size_bytes: exported.size_bytes,
        duration_ms: exported.duration_ms,
        width: exported.width,
        height: exported.height,
    };
    if !gate.go() {
        return Err(Failure::Cancelled);
    }
    let resume = lock(shared).server_id.clone();
    let mut id_out = None;
    let r = app_cloud::publish(
        &client,
        file,
        thumb.as_deref(),
        &meta,
        resume.as_deref(),
        &mut id_out,
        &mut |p| match p.stage {
            // Entre as partes dá para esperar o jogo fechar; no meio da parte, não.
            Stage::NextPart => gate.go(),
            Stage::Uploading => {
                set(Phase::Upload {
                    sent: p.sent,
                    total: p.total,
                });
                !cancelled()
            }
            Stage::Finishing => {
                set(Phase::Finishing);
                !cancelled()
            }
        },
    );
    lock(shared).server_id = id_out.clone();
    match r {
        Ok(clip) => clip
            .url
            .ok_or_else(|| Failure::Error("o servidor não devolveu o link".into())),
        Err(CloudError::Cancelled) => {
            // Libera a cota agora em vez de esperar as 24 h do servidor.
            if let Some(id) = id_out {
                let _ = client.delete_clip(&id);
                lock(shared).server_id = None;
            }
            Err(Failure::Cancelled)
        }
        Err(e) => Err(cloud(e)),
    }
}

/// Imagem do link (og:image 1200×630, ≤ 1 MB): o quadro do clipe composto com o botão de
/// vidro e a marca ([`crate::og`]). Sem ela o link ainda funciona.
fn make_thumb(file: &Path, duration_ms: i64) -> Option<Vec<u8>> {
    let out = file.with_extension("jpg");
    let at = app_media::thumbnail_time_ms(duration_ms);
    if let Err(e) = app_media::thumbnail(file, at, &out, 1280) {
        tracing::warn!("miniatura do link: {e}");
        return None;
    }
    let frame = image::open(&out).ok().map(|i| i.to_rgba8());
    let _ = std::fs::remove_file(&out);
    let frame = frame?;
    [85, 70]
        .into_iter()
        .filter_map(|q| crate::og::compose(&frame, q))
        .find(|b| b.len() <= THUMB_MAX_BYTES)
}
