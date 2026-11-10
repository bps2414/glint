//! Gravador na bandeja: fica ocioso até um jogo abrir, grava enquanto ele
//! roda e salva clipes na hotkey.
//!
//! Uma thread principal recebe todos os eventos num canal (hotkey, menu da
//! bandeja, pedidos do IPC, resultados do salvamento) e, a cada segundo, roda
//! a detecção de jogo, confere a saúde da sessão e atualiza o estado.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use app_core::game::{DetectorEvent, ForegroundSample, GameDetector};
use app_core::library::ClipKind;
use app_core::{brand, CaptureMode, Config, ReplaySnapshot};
use app_ipc::{RecorderState, RecorderStatus, Request, Response};
use app_platform_win::capture::CaptureTarget;
use app_platform_win::foreground::{self, GpuWatch, ProcessWatch};
use app_platform_win::hotkey::{self, HotkeyListener};
use app_platform_win::tray::{Tray, TrayCommand, TrayState};
use app_platform_win::windows::Win32::Foundation::HWND;
use app_platform_win::{autostart, clock, instance, power, shell, sound, window};
use parking_lot::Mutex;
use rust_i18n::t;

use crate::plan::{self, Backoff, Want};
use crate::presence::Presence;
use crate::saver::{ClipMeta, SaveJob, SaveOutcome, Saver};
use crate::session::{Health, Session, SessionParams};

const TICK: Duration = Duration::from_secs(1);
/// Uso do motor 3D (%) a partir do qual a janela em tela cheia "usa a GPU".
const GPU_ACTIVE_PERCENT: f64 = 1.0;
const STATUS_LOG_EVERY: Duration = Duration::from_secs(30);
/// Prazo para o encerramento limpo depois de "Sair"; passou disso, o processo sai à força.
const QUIT_GRACE: Duration = Duration::from_secs(8);

/// Índices das hotkeys registradas (ordem de `Daemon::hotkey_ok`).
const HOTKEY_SAVE: usize = 0;
const HOTKEY_KILL: usize = 1;

enum Event {
    /// Índice da hotkey apertada (`HOTKEY_SAVE` ou `HOTKEY_KILL`).
    Hotkey(usize),
    Tray(TrayCommand),
    Ipc(Request, Sender<Response>),
    Saved(SaveOutcome),
}

pub fn run(autostart_launch: bool) -> Result<()> {
    let lock_name = format!("{}-recorder", brand::APP_SLUG);
    let Some(_lock) = instance::acquire(&lock_name) else {
        tracing::info!("o gravador já está rodando; esta instância vai sair");
        return Ok(());
    };
    app_media::init_logging();
    // Dados do nome antigo (Clipe → Glint), 1×, já com a instância garantida.
    app_ipc::legacy::migrate();
    tracing::info!(
        autostart = autostart_launch,
        "gravador na bandeja iniciando"
    );

    let cfg_path = Config::default_path();
    let cfg = match Config::load_or_create(&cfg_path) {
        Ok(c) => c,
        Err(e) => {
            // Rodando escondido: não dá para morrer por config inválida.
            tracing::error!("{e}; usando padrões até a config ser corrigida");
            Config::default()
        }
    };

    let (tx, rx) = std::sync::mpsc::channel::<Event>();
    let status = Arc::new(Mutex::new(RecorderStatus::default()));

    let saver = {
        let tx = tx.clone();
        Saver::start(move |o| {
            let _ = tx.send(Event::Saved(o));
        })?
    };

    let ipc = {
        let tx = tx.clone();
        let status = status.clone();
        app_ipc::Server::start(&app_ipc::default_socket_name(), move |req| {
            if req == Request::Status {
                return Response::Status(status.lock().clone());
            }
            if req == Request::Shutdown {
                arm_quit_watchdog();
            }
            let (rtx, rrx) = std::sync::mpsc::channel();
            if tx.send(Event::Ipc(req, rtx)).is_err() {
                return Response::Error("gravador encerrando".into());
            }
            rrx.recv_timeout(Duration::from_secs(15))
                .unwrap_or_else(|_| Response::Error("o gravador não respondeu a tempo".into()))
        })
    };
    let _ipc = match ipc {
        Ok(s) => Some(s),
        Err(e) => {
            tracing::warn!("IPC indisponível (a interface não verá o estado): {e}");
            None
        }
    };

    let mut d = Daemon {
        detector: GameDetector::new(&cfg.game),
        presence: Presence::start(cfg.discord_presence),
        auto_detect: cfg.game.auto_detect,
        cfg,
        cfg_path,
        tx: tx.clone(),
        manual: false,
        paused_pid: None,
        game_watch: None,
        game_hwnd: None,
        gpu_watch: None,
        session: None,
        backoff: Backoff::default(),
        last_error: None,
        saver,
        tray: None,
        hotkey: None,
        hotkey_ok: [false; 2],
        pending_kill: None,
        status,
        clips_saved: 0,
        last_clip: None,
        disk_used: 0,
        last_status_log: Instant::now(),
        quit: false,
    };
    d.register_hotkey();
    d.sync_autostart();
    power::set_process_eco(true);
    let tray_tx = tx;
    match Tray::start(d.tray_state(), move |cmd| {
        if cmd == TrayCommand::Quit {
            arm_quit_watchdog();
        }
        let _ = tray_tx.send(Event::Tray(cmd));
    }) {
        Ok(t) => d.tray = Some(t),
        Err(e) => tracing::error!("sem ícone na bandeja: {e}"),
    }
    d.publish();
    d.run(rx);
    Ok(())
}

/// "Sair" tem que fechar o gravador mesmo se a thread principal estiver presa (driver, captura,
/// encoder) ou o encerramento travar num join. Armado fora da thread principal, por quem recebeu
/// o pedido (bandeja ou IPC); só a primeira chamada conta.
fn arm_quit_watchdog() {
    static ARMED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if ARMED.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("quit-watchdog".into())
        .spawn(|| {
            std::thread::sleep(QUIT_GRACE);
            tracing::error!(
                "o encerramento não terminou em {} s; saindo à força",
                QUIT_GRACE.as_secs()
            );
            // Dá tempo do log chegar ao arquivo.
            std::thread::sleep(Duration::from_millis(200));
            std::process::exit(0);
        });
}

/// Clipe de kill esperando o fim da janela do combo para ser salvo.
struct PendingKill {
    /// Primeiro e último aperto (µs, relógio QPC dos pacotes).
    first_us: i64,
    last_us: i64,
    presses: u32,
    /// Quando o clipe fecha: último aperto + max(combo, pós-roll).
    deadline: Instant,
}

struct Daemon {
    cfg: Config,
    cfg_path: PathBuf,
    tx: Sender<Event>,
    detector: GameDetector,
    auto_detect: bool,
    /// Usuário pediu "gravar a tela agora".
    manual: bool,
    /// Usuário parou a gravação deste jogo: não religar até ele fechar.
    paused_pid: Option<u32>,
    game_watch: Option<ProcessWatch>,
    /// Janela em foco quando o jogo foi detectado.
    game_hwnd: Option<HWND>,
    gpu_watch: Option<GpuWatch>,
    /// Rich Presence do Discord (thread própria; liga/desliga pela config).
    presence: Presence,
    session: Option<(Want, Session)>,
    backoff: Backoff,
    last_error: Option<String>,
    saver: Saver,
    tray: Option<Tray>,
    hotkey: Option<HotkeyListener>,
    /// Por hotkey (`HOTKEY_SAVE`, `HOTKEY_KILL`): registrada de fato.
    hotkey_ok: [bool; 2],
    pending_kill: Option<PendingKill>,
    status: Arc<Mutex<RecorderStatus>>,
    clips_saved: u64,
    last_clip: Option<PathBuf>,
    disk_used: u64,
    last_status_log: Instant,
    quit: bool,
}

impl Daemon {
    fn run(&mut self, rx: Receiver<Event>) {
        let mut next_tick = Instant::now();
        while !self.quit {
            // Acorda também no fim do combo, senão o TICK de 1 s atrasaria o salvamento.
            let wake = match &self.pending_kill {
                Some(p) => next_tick.min(p.deadline),
                None => next_tick,
            };
            let timeout = wake.saturating_duration_since(Instant::now());
            match rx.recv_timeout(timeout) {
                Ok(ev) => self.handle(ev),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            let kill_due = self
                .pending_kill
                .as_ref()
                .is_some_and(|p| Instant::now() >= p.deadline);
            if kill_due {
                self.flush_kill();
            }
            if Instant::now() >= next_tick {
                self.tick();
                next_tick = Instant::now() + TICK;
            }
            self.publish();
        }
        tracing::info!("gravador encerrando");
        self.flush_kill();
        if let Some((_, s)) = self.session.take() {
            s.stop();
        }
        self.tray = None;
        self.presence.stop();
        // `saver` termina a fila ao ser solto (fim de `run`).
    }

    // ---------------------------------------------------------------- eventos

    fn handle(&mut self, ev: Event) {
        match ev {
            Event::Hotkey(HOTKEY_KILL) => self.kill_press(),
            Event::Hotkey(_) => self.save_clip(),
            Event::Tray(cmd) => self.tray_command(cmd),
            Event::Ipc(req, reply) => {
                let resp = self.ipc(req);
                let _ = reply.send(resp);
            }
            Event::Saved(o) => match o {
                SaveOutcome::Saved {
                    path,
                    disk_used,
                    removed,
                    ..
                } => {
                    self.clips_saved += 1;
                    self.last_clip = Some(path);
                    self.disk_used = disk_used;
                    if removed > 0 {
                        tracing::info!(removidos = removed, "limite de disco aplicado");
                    }
                }
                SaveOutcome::Usage(bytes) => self.disk_used = bytes,
                SaveOutcome::Failed(_) => {}
            },
        }
    }

    fn tray_command(&mut self, cmd: TrayCommand) {
        match cmd {
            TrayCommand::SaveClip => self.save_clip(),
            TrayCommand::SaveKill => self.kill_press(),
            TrayCommand::OpenLibrary => self.open_library(),
            TrayCommand::OpenFolder => {
                let dir = self.cfg.clips_dir();
                let _ = std::fs::create_dir_all(&dir);
                shell::open_folder(&dir);
            }
            TrayCommand::ToggleAutoDetect => self.set_auto_detect(!self.auto_detect),
            TrayCommand::ToggleManual => {
                if self.session.is_some() {
                    self.stop_recording();
                } else {
                    self.start_manual();
                }
            }
            TrayCommand::ToggleStartWithOs => {
                self.cfg.start_with_os = !self.cfg.start_with_os;
                self.save_config();
                self.sync_autostart();
            }
            TrayCommand::Quit => self.quit = true,
        }
    }

    fn ipc(&mut self, req: Request) -> Response {
        match req {
            Request::Hello { .. } | Request::Status => Response::Status(self.status_now()),
            Request::SaveClip => {
                let recording = self.session.is_some();
                self.save_clip();
                if recording {
                    Response::Ok
                } else {
                    Response::Error("nada gravando agora".into())
                }
            }
            Request::SaveKill => {
                let recording = self.session.is_some();
                self.kill_press();
                if recording {
                    Response::Ok
                } else {
                    Response::Error("nada gravando agora".into())
                }
            }
            Request::SetAutoDetect(on) => {
                self.set_auto_detect(on);
                Response::Ok
            }
            Request::StartManual => {
                self.start_manual();
                match &self.last_error {
                    Some(e) if self.session.is_none() => Response::Error(e.clone()),
                    _ => Response::Ok,
                }
            }
            Request::StopRecording => {
                self.stop_recording();
                Response::Ok
            }
            Request::ReloadConfig => match self.reload_config() {
                Ok(()) => Response::Ok,
                Err(e) => Response::Error(e),
            },
            Request::Shutdown => {
                self.quit = true;
                Response::Ok
            }
            Request::Activate { .. } => Response::Error("pedido da interface".into()),
        }
    }

    // ------------------------------------------------------------ ações

    fn save_clip(&mut self) {
        let Some((_, s)) = &self.session else {
            tracing::info!("hotkey: nada gravando agora");
            if self.cfg.save_sound {
                sound::play_error_blip();
            }
            return;
        };
        let snapshot = s.snapshot(self.cfg.buffer_secs);
        if snapshot.is_empty() {
            tracing::info!("hotkey: buffer ainda vazio");
            if self.cfg.save_sound {
                sound::play_error_blip();
            }
            return;
        }
        self.queue_save(snapshot, ClipKind::Replay, None, None);
    }

    /// Aperto da hotkey de kill: abre um clipe pendente ou estende o que já está aberto.
    /// O salvamento acontece quando a janela do combo fecha (`flush_kill`).
    fn kill_press(&mut self) {
        if self.session.is_none() {
            tracing::info!("hotkey de kill: nada gravando agora");
            if self.cfg.save_sound {
                sound::play_error_blip();
            }
            return;
        }
        let now_us = clock::now_us();
        let now = Instant::now();
        let kill = &self.cfg.kill;
        let (pre_us, post_us) = (secs_us(kill.pre_secs), secs_us(kill.post_secs));
        // O clipe só fecha depois do pós-roll do último aperto, mesmo sem combo.
        let hold = Duration::from_secs(u64::from(kill.combo_secs.max(kill.post_secs)));
        let max_span_us = secs_us(self.cfg.buffer_secs);
        // Estende se a janela ainda está aberta e o clipe cabe no buffer.
        let extend = self.pending_kill.as_ref().is_some_and(|p| {
            let fits = now_us + post_us - (p.first_us - pre_us) <= max_span_us;
            now < p.deadline && fits
        });
        if extend {
            if let Some(p) = self.pending_kill.as_mut() {
                p.last_us = now_us;
                p.presses += 1;
                p.deadline = now + hold;
            }
        } else {
            self.flush_kill();
            self.pending_kill = Some(PendingKill {
                first_us: now_us,
                last_us: now_us,
                presses: 1,
                deadline: now + hold,
            });
        }
        if self.cfg.save_sound {
            sound::play_kill_tick();
        }
    }

    /// Salva o clipe de kill pendente (se houver) com o que o buffer tem agora.
    fn flush_kill(&mut self) {
        let Some(p) = self.pending_kill.take() else {
            return;
        };
        let Some((_, s)) = &self.session else {
            return;
        };
        let start_us = p.first_us - secs_us(self.cfg.kill.pre_secs);
        let end_us = p.last_us + secs_us(self.cfg.kill.post_secs);
        let snapshot = s.snapshot_range(start_us, end_us);
        if snapshot.is_empty() {
            tracing::info!("clipe de kill: buffer vazio nesse trecho");
            if self.cfg.save_sound {
                sound::play_error_blip();
            }
            return;
        }
        // Miniatura no primeiro aperto (o vídeo começa no keyframe antes do pedido).
        let thumb_at_ms = snapshot.start_us().map(|v0| (p.first_us - v0) / 1000);
        tracing::info!(apertos = p.presses, "salvando clipe de kill");
        self.queue_save(snapshot, ClipKind::Kill, Some(p.presses), thumb_at_ms);
    }

    /// Enfileira o salvamento de `snapshot` (da sessão atual). Clipe de kill não toca o blip ao
    /// salvar (o salvamento vem segundos depois do último aperto); falha toca nos dois.
    fn queue_save(
        &self,
        snapshot: ReplaySnapshot,
        kind: ClipKind,
        kills: Option<u32>,
        thumb_at_ms: Option<i64>,
    ) {
        let Some((want, s)) = &self.session else {
            return;
        };
        let game = match want {
            Want::Game(_) => self.detector.current().cloned(),
            _ => None,
        };
        self.saver.save(SaveJob {
            snapshot,
            streams: s.streams(),
            meta: ClipMeta {
                game: game.as_ref().map(|g| g.name.clone()),
                game_exe: game.map(|g| g.exe),
                width: s.info.width,
                height: s.info.height,
                fps: s.info.fps,
                codec: s.info.codec,
                kind,
                kills,
                thumb_at_ms,
            },
            clips_dir: self.cfg.clips_dir(),
            disk_limit_bytes: self.cfg.disk_limit_bytes(),
            play_sound: kind == ClipKind::Replay && self.cfg.save_sound,
            error_sound: self.cfg.save_sound,
        });
    }

    fn start_manual(&mut self) {
        self.manual = true;
        self.backoff.reset();
        self.reconcile();
    }

    fn stop_recording(&mut self) {
        if self.manual {
            self.manual = false;
        } else if let Some(g) = self.detector.current() {
            tracing::info!(jogo = %g.name, "gravação parada pelo usuário até o jogo fechar");
            self.paused_pid = Some(g.pid);
        }
        self.reconcile();
    }

    fn set_auto_detect(&mut self, on: bool) {
        if self.auto_detect == on {
            return;
        }
        self.auto_detect = on;
        self.cfg.game.auto_detect = on;
        self.save_config();
        if !on {
            self.forget_game();
        }
        tracing::info!(ligada = on, "gravação automática");
        self.reconcile();
    }

    fn forget_game(&mut self) {
        self.detector.reset();
        self.game_watch = None;
        self.game_hwnd = None;
        self.gpu_watch = None;
        self.paused_pid = None;
    }

    fn open_library(&self) {
        let ui = std::env::current_exe().ok().and_then(|p| {
            p.parent()
                .map(|d| d.join(format!("{}.exe", brand::APP_SLUG)))
        });
        match ui {
            Some(exe) if exe.exists() => {
                if let Err(e) = shell::spawn_detached(&exe, &[]) {
                    tracing::warn!("não foi possível abrir a interface: {e}");
                }
            }
            _ => {
                // Interface ainda não instalada junto: abre a pasta.
                let dir = self.cfg.clips_dir();
                let _ = std::fs::create_dir_all(&dir);
                shell::open_folder(&dir);
            }
        }
    }

    fn save_config(&self) {
        if let Err(e) = self.cfg.save(&self.cfg_path) {
            tracing::warn!("não foi possível salvar a config: {e}");
        }
    }

    fn reload_config(&mut self) -> std::result::Result<(), String> {
        let cfg = Config::load_or_create(&self.cfg_path).map_err(|e| e.to_string())?;
        let hotkey_changed =
            cfg.save_hotkey != self.cfg.save_hotkey || cfg.kill.hotkey != self.cfg.kill.hotkey;
        let auto = cfg.game.auto_detect;
        self.detector.set_config(&cfg.game);
        if cfg.discord_presence != self.cfg.discord_presence {
            self.presence.set_enabled(cfg.discord_presence);
        }
        self.cfg = cfg;
        if hotkey_changed {
            self.register_hotkey();
        }
        self.sync_autostart();
        if auto != self.auto_detect {
            self.auto_detect = auto;
            if !auto {
                self.forget_game();
            }
        }
        tracing::info!("config recarregada (vídeo/áudio valem na próxima gravação)");
        self.reconcile();
        Ok(())
    }

    /// Registra as hotkeys de salvar e de kill, cada uma por conta própria: uma tecla ocupada
    /// por outro app não derruba a outra.
    fn register_hotkey(&mut self) {
        self.hotkey = None; // solta as anteriores antes de registrar outras
        self.hotkey_ok = [false; 2];
        let specs = [self.cfg.save_hotkey.clone(), self.cfg.kill.hotkey.clone()];
        // (índice da hotkey, tecla) das que valem registrar.
        let mut slots: Vec<(usize, hotkey::Hotkey)> = Vec::new();
        for (id, spec) in specs.iter().enumerate() {
            let Some(key) = hotkey::parse(spec) else {
                tracing::warn!("hotkey inválida na config: '{spec}'");
                continue;
            };
            if slots.iter().any(|(_, k)| *k == key) {
                tracing::warn!("hotkey '{spec}' repetida; vale só a de salvar clipe");
                continue;
            }
            slots.push((id, key));
        }
        if slots.is_empty() {
            return;
        }
        let keys: Vec<hotkey::Hotkey> = slots.iter().map(|(_, k)| *k).collect();
        match HotkeyListener::start(keys) {
            Ok((listener, events, status)) => {
                let ids: Vec<usize> = slots.iter().map(|(id, _)| *id).collect();
                let tx = self.tx.clone();
                let _ = std::thread::Builder::new()
                    .name("hotkey-fwd".into())
                    .spawn(move || {
                        for pos in events {
                            let Some(&id) = ids.get(pos) else {
                                continue;
                            };
                            if tx.send(Event::Hotkey(id)).is_err() {
                                break;
                            }
                        }
                    });
                for ((id, _), st) in slots.iter().zip(status) {
                    let spec = &specs[*id];
                    match st {
                        Ok(()) => {
                            self.hotkey_ok[*id] = true;
                            tracing::info!(hotkey = %spec, "hotkey registrada");
                        }
                        Err(e) => tracing::warn!("hotkey {spec}: {e}"),
                    }
                }
                self.hotkey = Some(listener);
            }
            Err(e) => tracing::warn!("hotkeys: {e}"),
        }
    }

    fn sync_autostart(&self) {
        let name = brand::APP_NAME;
        let exe = std::env::current_exe().ok();
        // Build de desenvolvimento (target\debug, sem as DLLs do FFmpeg ao lado):
        // não registra, senão o Windows abriria um exe que falha por falta de DLL.
        let deployable = !cfg!(windows)
            || exe
                .as_ref()
                .and_then(|p| p.parent())
                .and_then(|d| std::fs::read_dir(d).ok())
                .is_some_and(|mut it| {
                    it.any(|e| {
                        e.is_ok_and(|e| e.file_name().to_string_lossy().starts_with("avcodec"))
                    })
                });
        if !deployable {
            return;
        }
        let want = if self.cfg.start_with_os {
            exe.map(|p| format!("\"{}\" --autostart", p.display()))
        } else {
            None
        };
        if autostart::current(name) == want {
            return;
        }
        match autostart::set(name, want.as_deref()) {
            Ok(()) => tracing::info!(ligado = want.is_some(), "iniciar com o Windows"),
            Err(e) => tracing::warn!("iniciar com o Windows: {e}"),
        }
    }

    // ------------------------------------------------------------ ciclo

    fn tick(&mut self) {
        if self.auto_detect {
            self.detect();
        }
        self.reconcile();
        self.check_health();
        let game = match &self.session {
            Some((Want::Game(_), s)) => Some(s.info.label.clone()),
            _ => None,
        };
        self.presence.set_game(game);
        if let Some((_, s)) = &self.session {
            if self.last_status_log.elapsed() >= STATUS_LOG_EVERY {
                self.last_status_log = Instant::now();
                s.log_status();
            }
        }
    }

    fn detect(&mut self) {
        let alive = match self.detector.current() {
            Some(g) => match (&self.game_watch, self.game_hwnd) {
                (Some(w), _) => w.alive(),
                // Sem acesso ao processo: a janela do jogo ainda existe?
                (None, Some(h)) => {
                    foreground::window_alive(h, g.pid) || game_window(g.pid).is_some()
                }
                (None, None) => foreground::process_image(g.pid).is_some(),
            },
            None => false,
        };
        let fg = if self.detector.current().is_none() {
            foreground::sample()
        } else {
            None
        };
        let sample = fg.as_ref().map(|f| ForegroundSample {
            pid: f.pid,
            exe: f.exe.clone(),
            title: f.title.clone(),
            fullscreen: f.fullscreen,
            gpu_active: self.gpu_active(f.pid, &f.exe, f.fullscreen),
        });
        match self.detector.tick(sample.as_ref(), alive) {
            Some(DetectorEvent::Start(g)) => {
                tracing::info!(jogo = %g.name, exe = %g.exe, pid = g.pid, "jogo detectado");
                self.game_watch = ProcessWatch::open(g.pid);
                if self.game_watch.is_none() {
                    tracing::info!("sem acesso ao processo do jogo; acompanhando pela janela");
                }
                self.game_hwnd = fg.map(|f| f.hwnd);
                self.gpu_watch = None;
            }
            Some(DetectorEvent::Stop(g)) => {
                tracing::info!(jogo = %g.name, "jogo fechou");
                self.game_watch = None;
                self.game_hwnd = None;
                if self.paused_pid == Some(g.pid) {
                    self.paused_pid = None;
                }
            }
            None => {}
        }
    }

    /// Mede a GPU do candidato (só quando decide algo; a 1ª leitura não tem valor).
    fn gpu_active(&mut self, pid: u32, exe: &str, fullscreen: bool) -> Option<bool> {
        if !self.detector.wants_gpu_check(exe, fullscreen) {
            self.gpu_watch = None;
            return None;
        }
        match &mut self.gpu_watch {
            Some(w) if w.pid() == pid => w.utilization().map(|u| u >= GPU_ACTIVE_PERCENT),
            _ => {
                self.gpu_watch = GpuWatch::new(pid);
                None
            }
        }
    }

    fn want(&self) -> Want {
        plan::want(
            self.manual,
            self.auto_detect,
            self.detector.current(),
            self.paused_pid,
        )
    }

    /// Abre/fecha a sessão para bater com o que deveria estar acontecendo.
    fn reconcile(&mut self) {
        let want = self.want();
        if self.session.as_ref().map(|(w, _)| *w) == Some(want) {
            return;
        }
        // A sessão vai embora: salva o combo pendente com o que o buffer tem.
        self.flush_kill();
        if let Some((_, s)) = self.session.take() {
            s.stop();
            power::set_process_eco(true);
        }
        if want == Want::Nothing {
            self.backoff.reset();
            self.last_error = None;
            return;
        }
        let now = Instant::now();
        if !self.backoff.ready(now) {
            return;
        }
        match self.start_session(want) {
            Ok(s) => {
                power::set_process_eco(false);
                self.session = Some((want, s));
                self.backoff.reset();
                self.last_error = None;
                self.last_status_log = Instant::now();
            }
            Err(e) => {
                let wait = self.backoff.fail(now);
                tracing::warn!(
                    tentativa = self.backoff.failures(),
                    "não foi possível começar a gravar: {e:#} (tentando de novo em {} s)",
                    wait.as_secs()
                );
                self.last_error = Some(format!("{e:#}"));
            }
        }
    }

    fn start_session(&self, want: Want) -> Result<Session> {
        let params = match want {
            Want::Nothing => unreachable!("reconcile não abre sessão para Nothing"),
            Want::Manual => SessionParams {
                target: CaptureTarget::PrimaryMonitor,
                audio_pid: None,
                label: t!("tray.screen").to_string(),
                encoder_override: None,
            },
            Want::Game(pid) => {
                let name = self
                    .detector
                    .current()
                    .map(|g| g.name.clone())
                    .unwrap_or_default();
                let hwnd = game_window(pid).or(self.game_hwnd);
                let target = match (self.cfg.game.capture, hwnd) {
                    (CaptureMode::Window, Some(h)) => CaptureTarget::Window(h),
                    (_, Some(h)) => CaptureTarget::MonitorOf(h),
                    (_, None) => CaptureTarget::PrimaryMonitor,
                };
                SessionParams {
                    target,
                    audio_pid: Some(pid),
                    label: name,
                    encoder_override: None,
                }
            }
        };
        Session::start(&self.cfg, params)
    }

    fn check_health(&mut self) {
        let Some((_, s)) = &self.session else {
            return;
        };
        let problem = match s.health() {
            Health::Ok => return,
            Health::SourceClosed => "a janela/monitor capturado fechou".to_string(),
            Health::Broken(why) => why,
        };
        tracing::warn!("gravação interrompida: {problem}; reabrindo");
        self.flush_kill();
        if let Some((_, s)) = self.session.take() {
            s.stop();
        }
        power::set_process_eco(true);
        self.last_error = Some(problem);
        // Primeira nova tentativa em 2 s (dá tempo para o driver/jogo se ajeitar).
        self.backoff.fail(Instant::now());
    }

    // ------------------------------------------------------------ estado

    fn source_label(&self) -> Option<String> {
        self.session.as_ref().map(|(_, s)| s.info.label.clone())
    }

    fn status_now(&self) -> RecorderStatus {
        let want = self.want();
        let state = match (&self.session, &self.last_error) {
            (Some((w, _)), _) => RecorderState::Recording {
                game: match w {
                    Want::Game(_) => self.source_label(),
                    _ => None,
                },
            },
            (None, Some(e)) if want != Want::Nothing => RecorderState::Failed { error: e.clone() },
            _ => RecorderState::Idle,
        };
        let mut st = RecorderStatus {
            state,
            auto_detect: self.auto_detect,
            clips_saved: self.clips_saved,
            last_clip: self.last_clip.as_ref().map(|p| p.display().to_string()),
            hotkey: self.cfg.save_hotkey.clone(),
            hotkey_ok: self.hotkey_ok[HOTKEY_SAVE],
            kill_hotkey: self.cfg.kill.hotkey.clone(),
            kill_hotkey_ok: self.hotkey_ok[HOTKEY_KILL],
            disk_used_bytes: self.disk_used,
            disk_limit_bytes: self.cfg.disk_limit_bytes(),
            ..RecorderStatus::default()
        };
        if let Some((_, s)) = &self.session {
            let (secs, bytes) = s.buffer_usage();
            st.buffer_secs = secs;
            st.buffer_bytes = bytes;
            st.encoder = Some(s.info.encoder.clone());
            st.width = s.info.width;
            st.height = s.info.height;
            st.fps = s.info.fps;
            st.dropped_frames = s.stats.dropped.load(std::sync::atomic::Ordering::Relaxed);
        }
        st
    }

    fn tray_state(&self) -> TrayState {
        let app = brand::APP_NAME;
        let key = self.cfg.save_hotkey.as_str();
        let kill_key = self.cfg.kill.hotkey.as_str();
        let recording = self.session.is_some();
        let failed = !recording && self.last_error.is_some() && self.want() != Want::Nothing;
        let source = self.source_label().unwrap_or_default();
        let (tooltip, mut status) = if recording {
            (
                t!("tray.tooltip_recording", app = app, source = source).to_string(),
                t!("tray.status_recording", source = source).to_string(),
            )
        } else if failed {
            let err = short(self.last_error.as_deref().unwrap_or_default(), 60);
            (
                t!("tray.tooltip_failed", app = app).to_string(),
                t!("tray.status_failed", error = err).to_string(),
            )
        } else if !self.auto_detect {
            (
                t!("tray.tooltip_paused", app = app).to_string(),
                t!("tray.status_paused").to_string(),
            )
        } else {
            (
                t!("tray.tooltip_idle", app = app).to_string(),
                t!("tray.status_idle").to_string(),
            )
        };
        for (ok, k) in [
            (self.hotkey_ok[HOTKEY_SAVE], key),
            (self.hotkey_ok[HOTKEY_KILL], kill_key),
        ] {
            if !ok {
                status = format!("{status} · {}", t!("tray.hotkey_busy", key = k));
            }
        }
        TrayState {
            tooltip: short(&tooltip, 120),
            status,
            recording,
            failed,
            save_label: if self.hotkey_ok[HOTKEY_SAVE] {
                t!("tray.save", key = key).to_string()
            } else {
                t!("tray.save_nokey").to_string()
            },
            kill_label: if self.hotkey_ok[HOTKEY_KILL] {
                t!("tray.kill", key = kill_key).to_string()
            } else {
                t!("tray.kill_nokey").to_string()
            },
            can_save: recording,
            manual_label: if recording {
                t!("tray.stop").to_string()
            } else {
                t!("tray.record_screen").to_string()
            },
            auto_detect_label: t!("tray.auto_detect").to_string(),
            auto_detect: self.auto_detect,
            library_label: t!("tray.library").to_string(),
            folder_label: t!("tray.folder").to_string(),
            start_with_os_label: t!("tray.start_with_os").to_string(),
            start_with_os: self.cfg.start_with_os,
            quit_label: t!("tray.quit").to_string(),
        }
    }

    /// Atualiza o estado visto pelo IPC e pela bandeja.
    fn publish(&self) {
        *self.status.lock() = self.status_now();
        if let Some(tray) = &self.tray {
            tray.update(self.tray_state());
        }
    }
}

/// Maior janela visível do processo (o jogo pode recriar a janela ao mudar de modo).
fn game_window(pid: u32) -> Option<HWND> {
    window::list()
        .into_iter()
        .filter(|w| w.pid == pid)
        .max_by_key(|w| i64::from(w.width) * i64::from(w.height))
        .map(|w| w.hwnd)
}

/// Segundos → µs (mesma unidade dos timestamps dos pacotes).
fn secs_us(secs: u32) -> i64 {
    i64::from(secs) * 1_000_000
}

fn short(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}
