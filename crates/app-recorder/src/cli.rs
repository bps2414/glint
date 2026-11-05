//! Modos de linha de comando: gravação manual no console (testes da Fase 0),
//! diagnóstico, lista de janelas e comandos para o gravador da bandeja.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use app_core::library::{ClipKind, Library};
use app_core::{brand, Config};
use app_ipc::{Client, RecorderState, Request, Response};
use app_media::VideoSettings;
use app_platform_win::audio::{self, AudioSource};
use app_platform_win::capture::{Capture, CaptureTarget};
use app_platform_win::convert::Converter;
use app_platform_win::gpu::Gpu;
use app_platform_win::{autostart, hotkey, window};

use crate::args::Args;
use crate::saver::{ClipMeta, SaveJob, SaveOutcome, Saver};
use crate::session::{self, Health, Session, SessionParams};

/// Config do arquivo + ajustes passados na linha de comando.
pub fn load_config(args: &Args) -> Result<Config> {
    let path = Config::default_path();
    let mut cfg =
        Config::load_or_create(&path).with_context(|| format!("lendo {}", path.display()))?;
    if let Some(s) = args.secs {
        cfg.buffer_secs = s.clamp(5, 1200);
    }
    if let Some(c) = args.codec {
        cfg.video.codec = c;
    }
    if let Some(r) = args.res {
        cfg.video.resolution = r;
    }
    if let Some(f) = args.fps {
        cfg.video.fps = f.clamp(10, 240);
    }
    if let Some(b) = args.bitrate {
        cfg.video.bitrate_kbps = b.clamp(1000, 150_000);
    }
    if args.no_audio {
        cfg.audio.system = false;
    }
    if args.no_mic {
        cfg.audio.microphone = false;
    }
    Ok(cfg)
}

fn target(args: &Args) -> Result<(CaptureTarget, Option<u32>, String)> {
    match &args.window {
        Some(needle) => {
            let w = window::find(needle).ok_or_else(|| {
                anyhow!(
                    "nenhuma janela com '{needle}' no título (use --list-windows para ver as janelas)"
                )
            })?;
            tracing::info!(titulo = %w.title, pid = w.pid, "janela alvo encontrada");
            Ok((CaptureTarget::Window(w.hwnd), Some(w.pid), w.title))
        }
        None => Ok((
            CaptureTarget::PrimaryMonitor,
            None,
            "monitor principal".to_string(),
        )),
    }
}

pub fn list_windows() {
    println!("{:<10} {:>6}x{:<6} TÍTULO", "PID", "LARG", "ALT");
    for w in window::list() {
        println!("{:<10} {:>6}x{:<6} {}", w.pid, w.width, w.height, w.title);
    }
}

/// Gravação manual no console: hotkey salva, Ctrl+C encerra.
pub fn manual(args: &Args) -> Result<()> {
    app_media::init_logging();
    let cfg = load_config(args)?;
    let (target, pid, label) = target(args)?;
    let session = Session::start(
        &cfg,
        SessionParams {
            target,
            audio_pid: pid,
            label: label.clone(),
            encoder_override: args.encoder.clone(),
        },
    )?;
    let saver = Saver::start(|o| match o {
        SaveOutcome::Saved {
            path, secs, mb, ms, ..
        } => println!(
            "  ✓ clipe salvo: {} ({secs:.1} s, {mb:.1} MB, {ms} ms)",
            path.display()
        ),
        SaveOutcome::Failed(e) => println!("  ✗ falha ao salvar: {e}"),
        SaveOutcome::Usage(_) => {}
    })?;

    let key = hotkey::parse(&cfg.save_hotkey)
        .ok_or_else(|| anyhow!("hotkey inválida na config: '{}'", cfg.save_hotkey))?;
    let (_listener, hotkeys, status) = hotkey::HotkeyListener::start(vec![key])?;
    if let Some(Err(e)) = status.into_iter().next() {
        return Err(anyhow!(
            "registrando hotkey (gravador da bandeja aberto? feche com --quit ou pelo menu): {e}"
        ));
    }

    let stop = Arc::new(AtomicBool::new(false));
    ctrlc::set_handler({
        let stop = stop.clone();
        move || stop.store(true, Ordering::SeqCst)
    })
    .ok();

    let i = &session.info;
    println!();
    println!("  ● {} gravando: {label}", brand::APP_NAME);
    println!(
        "    {}x{} @ {} fps • {} via {} • buffer {} s • áudio: {} trilha(s)",
        i.width,
        i.height,
        i.fps,
        i.codec.label(),
        i.encoder,
        i.buffer_secs,
        i.audio_tracks
    );
    println!(
        "    Aperte {} para salvar • Ctrl+C para sair",
        cfg.save_hotkey
    );
    println!("    Clipes em: {}", cfg.clips_dir().display());
    println!();

    let mut last_report = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        match session.health() {
            Health::Ok => {}
            Health::SourceClosed => {
                tracing::warn!("fonte fechada; encerrando");
                break;
            }
            Health::Broken(why) => {
                tracing::error!("gravação interrompida: {why}");
                break;
            }
        }
        if hotkeys.recv_timeout(Duration::from_millis(200)).is_ok() {
            let snapshot = session.snapshot(cfg.buffer_secs);
            if snapshot.is_empty() {
                println!("  (buffer ainda vazio)");
                continue;
            }
            saver.save(SaveJob {
                snapshot,
                streams: session.streams(),
                meta: ClipMeta {
                    game: args.window.as_ref().map(|_| label.clone()),
                    game_exe: None,
                    width: i.width,
                    height: i.height,
                    fps: i.fps,
                    codec: i.codec,
                    kind: ClipKind::Replay,
                    kills: None,
                    thumb_at_ms: None,
                },
                clips_dir: cfg.clips_dir(),
                disk_limit_bytes: cfg.disk_limit_bytes(),
                play_sound: cfg.save_sound,
                error_sound: cfg.save_sound,
            });
        }
        if last_report.elapsed() >= Duration::from_secs(10) {
            last_report = Instant::now();
            session.log_status();
        }
    }
    session.stop();
    drop(saver);
    tracing::info!("encerrado");
    Ok(())
}

/// Testa GPU, captura, encoder, áudio e o resto da instalação; imprime um resumo.
pub fn diagnose(args: &Args) -> Result<()> {
    app_media::init_logging();
    let cfg = load_config(args)?;
    tracing::info!(ffmpeg = %app_media::ffmpeg_version(), "FFmpeg carregado");
    let gpu = Gpu::new().context("criando dispositivo D3D11")?;
    let (target, _, label) = target(args)?;
    let capture = Capture::start(&gpu, target, cfg.video.fps).context("iniciando captura")?;
    let shared = capture.shared().clone();
    let (src_w, src_h) = session::first_frame_size(&shared)?;
    let (out_w, out_h) = cfg.video.resolution.output_size(src_w, src_h);
    let fps = cfg.video.fps;
    let settings = VideoSettings {
        codec: cfg.video.codec,
        width: out_w,
        height: out_h,
        fps,
        bitrate_kbps: cfg.video.bitrate_kbps,
        gop: fps * cfg.video.keyframe_interval_secs.max(1),
    };
    let mut converter = Converter::new(&gpu, src_w, src_h, out_w, out_h, fps)?;
    let mut encoder = session::open_encoder(&gpu, &settings, args.encoder.as_deref())?;
    let encoder_name = encoder.encoder().name().to_string();
    let r = session::encode_test_frames(&gpu, &mut converter, &mut encoder, &shared);
    capture.stop();

    println!("\n=== Diagnóstico {} ===", brand::APP_NAME);
    println!("GPU:        {}", gpu.adapter_name);
    println!("FFmpeg:     {}", app_media::ffmpeg_version());
    println!("Encoders:   {:?}", app_media::available_video_encoders());
    println!("Escolhido:  {encoder_name}");
    println!("Captura:    {src_w}x{src_h} ({label})");
    println!(
        "Saída:      {out_w}x{out_h} @ {fps} fps, {} kbps",
        settings.bitrate_kbps
    );
    match r {
        Ok(n) => println!("Encode:     OK ({n} pacotes de teste)"),
        Err(e) => println!("Encode:     FALHOU: {e:#}"),
    }
    for src in [AudioSource::System, AudioSource::Microphone] {
        let stop = Arc::new(AtomicBool::new(false));
        let r = audio::spawn_capture(src, stop.clone(), |_, _| {});
        stop.store(true, Ordering::SeqCst);
        match r {
            Ok(h) => {
                let _ = h.join();
                println!("Áudio {:<10} OK", src.label());
            }
            Err(e) => println!("Áudio {:<10} FALHOU: {e}", src.label()),
        }
    }
    match hotkey::parse(&cfg.save_hotkey) {
        Some(k) => match hotkey::HotkeyListener::start(vec![k]) {
            Ok((_, _, status)) => match &status[0] {
                Ok(()) => println!("Hotkey:     {} OK", cfg.save_hotkey),
                Err(e) => println!(
                    "Hotkey:     {} indisponível ({e}); normal com a bandeja aberta",
                    cfg.save_hotkey
                ),
            },
            Err(e) => println!("Hotkey:     {} indisponível ({e})", cfg.save_hotkey),
        },
        None => println!("Hotkey:     '{}' inválida", cfg.save_hotkey),
    }
    match Library::open(&Config::library_path()) {
        Ok(lib) => println!(
            "Biblioteca: {} clipe(s), {:.1} GB de {} GB",
            lib.count().unwrap_or(0),
            lib.total_bytes().unwrap_or(0) as f64 / 1e9,
            cfg.disk_limit_gb
        ),
        Err(e) => println!("Biblioteca: FALHOU: {e}"),
    }
    println!(
        "Iniciar c/ Windows: {}",
        if autostart::current(brand::APP_NAME).is_some() {
            "ligado"
        } else {
            "desligado"
        }
    );
    println!(
        "Bandeja:    {}",
        match Client::call(&Request::Status) {
            Ok(_) => "gravador rodando",
            Err(_) => "gravador não está rodando",
        }
    );
    println!("Clipes:     {}", cfg.clips_dir().display());
    println!("Config:     {}", Config::default_path().display());
    println!("Logs:       {}", Config::data_dir().join("logs").display());
    Ok(())
}

/// `--status`, `--save`, `--quit`: fala com o gravador da bandeja.
pub fn ipc(args: &Args) -> Result<()> {
    let req = if args.save {
        Request::SaveClip
    } else if args.quit {
        Request::Shutdown
    } else if args.record_screen {
        Request::StartManual
    } else if args.stop {
        Request::StopRecording
    } else {
        Request::Status
    };
    match Client::call(&req) {
        Ok(Response::Status(s)) => {
            let state = match &s.state {
                RecorderState::Idle => "ocioso (aguardando um jogo)".to_string(),
                RecorderState::Starting => "iniciando".to_string(),
                RecorderState::Recording { game: Some(g) } => format!("gravando {g}"),
                RecorderState::Recording { game: None } => "gravando a tela".to_string(),
                RecorderState::Failed { error } => format!("erro: {error}"),
            };
            println!("Estado:     {state}");
            println!(
                "Automático: {}",
                if s.auto_detect { "ligado" } else { "desligado" }
            );
            if let Some(enc) = &s.encoder {
                println!(
                    "Vídeo:      {}x{} @ {} fps via {enc}, {} quadros perdidos",
                    s.width, s.height, s.fps, s.dropped_frames
                );
                println!(
                    "Buffer:     {:.1} s, {:.1} MB",
                    s.buffer_secs,
                    s.buffer_bytes as f64 / 1e6
                );
            }
            println!("Hotkey:     {} (kill: {})", s.hotkey, s.kill_hotkey);
            println!("Salvos:     {} nesta sessão", s.clips_saved);
            if let Some(c) = &s.last_clip {
                println!("Último:     {c}");
            }
            println!(
                "Disco:      {:.1} GB de {:.0} GB",
                s.disk_used_bytes as f64 / 1e9,
                s.disk_limit_bytes as f64 / 1e9
            );
            Ok(())
        }
        Ok(Response::Ok) => {
            println!("OK");
            Ok(())
        }
        Ok(Response::Error(e)) => Err(anyhow!(e)),
        Ok(other) => Err(anyhow!("resposta inesperada: {other:?}")),
        Err(e) => Err(anyhow!("{e}")),
    }
}

/// `--measure-fps <pid>`: FPS de outro processo (teste guiado). Saída em `chave=valor`
/// para o script ler, uma linha cada.
pub fn measure_fps(args: &Args) -> Result<()> {
    let pid = args
        .measure_fps
        .ok_or_else(|| anyhow!("--measure-fps sem pid"))?;
    let secs = args.secs.unwrap_or(30).clamp(1, 600);
    let r = app_platform_win::present::measure(pid, Duration::from_secs(secs.into()))
        .map_err(|e| anyhow!("{e}"))?;
    println!("frames={}", r.frames);
    println!("secs={:.1}", r.secs);
    println!("fps_avg={:.1}", r.avg_fps);
    println!("fps_low1={:.1}", r.low1_fps);
    println!("worst_ms={:.1}", r.worst_ms);
    Ok(())
}
