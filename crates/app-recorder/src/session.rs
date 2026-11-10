//! Uma sessão de gravação: captura + encoder + áudio + replay buffer.
//!
//! ```text
//! WGC (frame BGRA, GPU) ─► slot "último frame"
//!                               │  (thread de vídeo, ritmo CFR)
//!                               ▼
//!          VideoProcessor BGRA→NV12 (+escala) ─► encoder HW (AMF/NVENC/QSV)
//!                                                      │ pacotes HEVC/H.264
//! WASAPI jogo/sistema ─► AAC ─┐                        ▼
//! WASAPI microfone    ─► AAC ─┴──────────────► ReplayBuffer (RAM, comprimido)
//! ```
//!
//! O daemon abre uma sessão quando um jogo começa e a fecha quando ele
//! termina; o modo manual (console) usa a mesma sessão.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use app_core::{Config, ReplayBuffer, ReplaySnapshot};
use app_media::d3d11::D3D11Encoder;
use app_media::{AudioEncoder, StreamInfo, VideoSettings};
use app_platform_win::audio::{self, AudioSource};
use app_platform_win::capture::{Capture, CaptureShared, CaptureTarget};
use app_platform_win::convert::Converter;
use app_platform_win::gpu::Gpu;
use app_platform_win::windows::core::Interface;
use app_platform_win::windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;
use app_platform_win::{clock, power};
use parking_lot::Mutex;

/// Quantos erros seguidos de encode/conversão contam como "pipeline quebrado".
const MAX_CONSECUTIVE_ERRORS: u64 = 120;

#[derive(Default)]
pub struct Stats {
    pub encoded: AtomicU64,
    pub dropped: AtomicU64,
    pub errors: AtomicU64,
    pub consecutive_errors: AtomicU64,
}

pub struct SessionParams {
    pub target: CaptureTarget,
    /// Processo cujo áudio gravar (só o jogo). `None` = áudio do sistema.
    pub audio_pid: Option<u32>,
    pub label: String,
    pub encoder_override: Option<String>,
}

#[derive(Clone, Debug)]
pub struct SessionInfo {
    pub label: String,
    pub encoder: String,
    pub adapter: String,
    pub src_w: u32,
    pub src_h: u32,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub codec: app_core::Codec,
    pub buffer_secs: u32,
    pub audio_tracks: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Health {
    Ok,
    /// A janela/monitor capturado sumiu.
    SourceClosed,
    /// GPU reiniciou ou o encoder parou de funcionar.
    Broken(String),
}

pub struct Session {
    pub info: SessionInfo,
    pub stats: Arc<Stats>,
    gpu: Gpu,
    capture: Capture,
    shared: Arc<CaptureShared>,
    buffer: Arc<Mutex<ReplayBuffer>>,
    streams: Arc<Vec<StreamInfo>>,
    stop: Arc<AtomicBool>,
    video_thread: Option<JoinHandle<()>>,
    audio_threads: Vec<JoinHandle<()>>,
}

impl Session {
    pub fn start(cfg: &Config, p: SessionParams) -> Result<Self> {
        let gpu = Gpu::new().context("criando dispositivo D3D11")?;
        let fps = cfg.video.fps;
        let capture = Capture::start(&gpu, p.target, fps, cfg.video.capture_backend)
            .context("iniciando captura")?;
        let shared = capture.shared().clone();
        let (src_w, src_h) = wait_first_frame(&shared)?;

        let (out_w, out_h) = cfg.video.resolution.output_size(src_w, src_h);
        let settings = VideoSettings {
            codec: cfg.video.codec,
            width: out_w,
            height: out_h,
            fps,
            bitrate_kbps: cfg.video.bitrate_kbps,
            gop: fps * cfg.video.keyframe_interval_secs.max(1),
        };
        let converter = Converter::new(&gpu, src_w, src_h, out_w, out_h, fps)
            .context("criando conversor NV12")?;
        let encoder = open_encoder(&gpu, &settings, p.encoder_override.as_deref())?;
        let encoder_name = encoder.encoder().name().to_string();

        let buffer = Arc::new(Mutex::new(ReplayBuffer::new(
            cfg.buffer_secs,
            cfg.buffer_max_mb as usize * 1024 * 1024,
        )));
        let mut streams: Vec<StreamInfo> = vec![encoder.encoder().stream_info().clone()];
        let stop = Arc::new(AtomicBool::new(false));
        let audio_threads = start_audio(cfg, p.audio_pid, &buffer, &stop, &mut streams);
        let streams = Arc::new(streams);

        let stats = Arc::new(Stats::default());
        let video_thread = {
            let (stop, buffer, stats, shared, gpu) = (
                stop.clone(),
                buffer.clone(),
                stats.clone(),
                shared.clone(),
                gpu.clone(),
            );
            std::thread::Builder::new()
                .name("video".into())
                .spawn(move || {
                    power::set_current_thread_realtime_ish();
                    video_loop(gpu, converter, encoder, shared, buffer, stats, stop, fps)
                })?
        };

        let info = SessionInfo {
            label: p.label,
            encoder: encoder_name,
            adapter: gpu.adapter_name.clone(),
            src_w,
            src_h,
            width: out_w,
            height: out_h,
            fps,
            codec: cfg.video.codec,
            buffer_secs: cfg.buffer_secs,
            audio_tracks: streams.len() - 1,
        };
        tracing::info!(
            fonte = %info.label,
            gpu = %info.adapter,
            entrada = format!("{}x{}", info.src_w, info.src_h),
            encoder = %info.encoder,
            saida = format!("{out_w}x{out_h}@{fps}"),
            audio = info.audio_tracks,
            "sessão de gravação iniciada"
        );
        Ok(Self {
            info,
            stats,
            gpu,
            capture,
            shared,
            buffer,
            streams,
            stop,
            video_thread: Some(video_thread),
            audio_threads,
        })
    }

    pub fn streams(&self) -> Arc<Vec<StreamInfo>> {
        self.streams.clone()
    }

    pub fn snapshot(&self, secs: u32) -> ReplaySnapshot {
        self.buffer.lock().snapshot(Some(secs))
    }

    /// Trecho `[start_us, end_us]` do buffer (instantes no relógio QPC de `clock::now_us`).
    pub fn snapshot_range(&self, start_us: i64, end_us: i64) -> ReplaySnapshot {
        self.buffer.lock().snapshot_range(start_us, end_us)
    }

    /// (segundos de vídeo, bytes) no buffer.
    pub fn buffer_usage(&self) -> (f32, u64) {
        let b = self.buffer.lock();
        (b.video_span_us() as f32 / 1e6, b.bytes() as u64)
    }

    pub fn health(&self) -> Health {
        if self.shared.closed.load(Ordering::Relaxed) {
            return Health::SourceClosed;
        }
        if self.gpu.is_lost() {
            return Health::Broken("a GPU reiniciou (driver)".into());
        }
        if self.stats.consecutive_errors.load(Ordering::Relaxed) >= MAX_CONSECUTIVE_ERRORS {
            return Health::Broken("o encoder parou de funcionar".into());
        }
        if self.video_thread.as_ref().is_some_and(|h| h.is_finished()) {
            return Health::Broken("a thread de vídeo terminou".into());
        }
        Health::Ok
    }

    pub fn log_status(&self) {
        let (secs, bytes) = self.buffer_usage();
        tracing::info!(
            buffer_s = format!("{secs:.1}"),
            buffer_mb = format!("{:.1}", bytes as f64 / 1e6),
            frames = self.stats.encoded.load(Ordering::Relaxed),
            descartados = self.stats.dropped.load(Ordering::Relaxed),
            erros = self.stats.errors.load(Ordering::Relaxed),
            "status"
        );
    }

    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.video_thread.take() {
            let _ = h.join();
        }
        for h in self.audio_threads.drain(..) {
            let _ = h.join();
        }
        self.capture.stop();
        tracing::info!(fonte = %self.info.label, "sessão de gravação encerrada");
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if self.video_thread.is_some() {
            self.shutdown();
        }
    }
}

fn wait_first_frame(shared: &CaptureShared) -> Result<(u32, u32)> {
    let t0 = Instant::now();
    loop {
        if let Some(l) = shared.latest.lock().as_ref() {
            return Ok((l.width, l.height));
        }
        if shared.closed.load(Ordering::Relaxed) {
            bail!("a fonte da captura fechou antes do primeiro quadro");
        }
        if t0.elapsed() > Duration::from_secs(5) {
            bail!(
                "nenhum quadro recebido em 5 s. Se o jogo estiver em tela cheia exclusiva, \
                 tente 'Janela sem borda'"
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

pub fn open_encoder(
    gpu: &Gpu,
    settings: &VideoSettings,
    force: Option<&str>,
) -> Result<D3D11Encoder> {
    unsafe { D3D11Encoder::new(gpu.device_ptr(), |p| Gpu::add_ref(p), settings, force) }
        .context("abrindo encoder de vídeo por hardware")
}

fn start_audio(
    cfg: &Config,
    pid: Option<u32>,
    buffer: &Arc<Mutex<ReplayBuffer>>,
    stop: &Arc<AtomicBool>,
    streams: &mut Vec<StreamInfo>,
) -> Vec<JoinHandle<()>> {
    let mut sources = Vec::new();
    if cfg.audio.system {
        sources.push(match pid {
            Some(pid) => AudioSource::Process(pid),
            None => AudioSource::System,
        });
    }
    if cfg.audio.microphone {
        sources.push(AudioSource::Microphone);
    }
    let mut threads = Vec::new();
    for src in sources {
        let id = streams.len();
        let enc = match AudioEncoder::new(
            id,
            audio::SAMPLE_RATE,
            src.channels(),
            cfg.audio.bitrate_kbps,
        ) {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!("sem áudio {}: {e}", src.label());
                continue;
            }
        };
        let info = enc.stream_info().clone();
        let enc = Arc::new(Mutex::new(enc));
        let spawn = |source: AudioSource| {
            let enc = enc.clone();
            let buf = buffer.clone();
            audio::spawn_capture(source, stop.clone(), move |samples, t| {
                match enc.lock().push(samples, t) {
                    Ok(pkts) => {
                        let mut b = buf.lock();
                        pkts.into_iter().for_each(|p| b.push(p));
                    }
                    Err(e) => tracing::warn!("erro no AAC: {e}"),
                }
            })
        };
        let started = match (spawn(src), src) {
            // Áudio só do processo pode falhar (Windows antigo): cai para o sistema.
            (Err(e), AudioSource::Process(_)) => {
                tracing::warn!("áudio só do jogo indisponível ({e}); usando áudio do sistema");
                spawn(AudioSource::System)
            }
            (r, _) => r,
        };
        match started {
            Ok(h) => {
                streams.push(info);
                threads.push(h);
            }
            Err(e) => tracing::warn!("sem áudio {}: {e}", src.label()),
        }
    }
    threads
}

/// Copia a saída NV12 do conversor para a textura do pool do encoder.
pub fn copy_to_encoder(gpu: &Gpu, converter: &Converter, dst: *mut std::ffi::c_void, index: u32) {
    unsafe {
        if let Some(dst) = ID3D11Texture2D::from_raw_borrowed(&dst) {
            gpu.context
                .CopySubresourceRegion(dst, index, 0, 0, 0, converter.output(), 0, None);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn video_loop(
    gpu: Gpu,
    mut converter: Converter,
    mut encoder: D3D11Encoder,
    shared: Arc<CaptureShared>,
    buffer: Arc<Mutex<ReplayBuffer>>,
    stats: Arc<Stats>,
    stop: Arc<AtomicBool>,
    fps: u32,
) {
    let frame_us = 1_000_000 / i64::from(fps);
    let start = clock::now_us();
    let mut next_pts: i64 = 0;
    let fail = |stats: &Stats, what: &str, e: &dyn std::fmt::Display| {
        let n = stats.errors.fetch_add(1, Ordering::Relaxed);
        stats.consecutive_errors.fetch_add(1, Ordering::Relaxed);
        if n < 5 || n % 600 == 0 {
            tracing::warn!("erro {what}: {e}");
        }
    };
    while !stop.load(Ordering::Relaxed) {
        let now = clock::now_us();
        let pts = (now - start) / frame_us;
        if pts < next_pts {
            let wait = start + next_pts * frame_us - now;
            std::thread::sleep(Duration::from_micros(wait.max(0) as u64));
            continue;
        }
        if pts > next_pts {
            stats
                .dropped
                .fetch_add((pts - next_pts) as u64, Ordering::Relaxed);
        }
        next_pts = pts + 1;
        let time_us = start + pts * frame_us;

        let converted = {
            let latest = shared.latest.lock();
            match latest.as_ref() {
                Some(l) => converter.convert(&l.texture, l.width, l.height),
                None => continue,
            }
        };
        if let Err(e) = converted {
            fail(&stats, "na conversão", &e);
            continue;
        }
        match encoder.encode(pts, time_us, |dst, idx| {
            copy_to_encoder(&gpu, &converter, dst, idx)
        }) {
            Ok(pkts) => {
                stats.encoded.fetch_add(1, Ordering::Relaxed);
                stats.consecutive_errors.store(0, Ordering::Relaxed);
                let mut b = buffer.lock();
                pkts.into_iter().for_each(|p| b.push(p));
            }
            Err(e) => fail(&stats, "no encode", &e),
        }
    }
    match encoder.encoder_mut().flush() {
        Ok(pkts) => {
            let mut b = buffer.lock();
            pkts.into_iter().for_each(|p| b.push(p));
        }
        Err(e) => tracing::debug!("flush: {e}"),
    }
}

/// Codifica ~1 s de quadros da fonte para validar o pipeline (`--diagnose`).
pub fn encode_test_frames(
    gpu: &Gpu,
    converter: &mut Converter,
    encoder: &mut D3D11Encoder,
    shared: &CaptureShared,
) -> Result<usize> {
    let mut packets = 0;
    let fps = i64::from(encoder.encoder().fps());
    for pts in 0..fps {
        {
            let latest = shared.latest.lock();
            let l = latest.as_ref().ok_or_else(|| anyhow!("sem frame"))?;
            converter.convert(&l.texture, l.width, l.height)?;
        }
        let pkts = encoder.encode(pts, pts * 1_000_000 / fps, |dst, idx| {
            copy_to_encoder(gpu, converter, dst, idx)
        })?;
        packets += pkts.len();
        std::thread::sleep(Duration::from_millis(16));
    }
    packets += encoder.encoder_mut().flush()?.len();
    if packets == 0 {
        bail!("encoder não produziu pacotes");
    }
    Ok(packets)
}

/// Espera o primeiro quadro (usado pelo diagnóstico).
pub fn first_frame_size(shared: &CaptureShared) -> Result<(u32, u32)> {
    wait_first_frame(shared)
}
