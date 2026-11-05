//! Teste de ponta a ponta (sem GPU): encode por software → replay buffer →
//! snapshot → MP4 por remux → verificação com ffprobe (se disponível).

use app_core::{Codec, ReplayBuffer};
use app_media::{mux, AudioEncoder, VideoEncoder, VideoSettings};

fn ffprobe(path: &std::path::Path, entries: &str) -> Option<String> {
    let out = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            entries,
            "-of",
            "default=nw=1",
        ])
        .arg(path)
        .output()
        .ok()?;
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[test]
fn replay_snapshot_writes_valid_mp4() {
    let fps = 30;
    let settings = VideoSettings {
        codec: Codec::H264,
        width: 320,
        height: 180,
        fps,
        bitrate_kbps: 500,
        gop: fps,
    };
    let mut video = VideoEncoder::open_software(&settings).expect("encoder sw");
    // Áudio de entrada como o WASAPI costuma entregar: 44,1 kHz estéreo float.
    let mut game = AudioEncoder::new(1, 44_100, 2, 128).expect("aac jogo");
    // Microfone mono 48 kHz.
    let mut mic = AudioEncoder::new(2, 48_000, 1, 96).expect("aac mic");

    let mut buf = ReplayBuffer::new(3, 512 << 20);
    let t0 = 1_000_000_000i64; // relógio arbitrário
    let total_frames = fps as i64 * 6;
    for i in 0..total_frames {
        let t = t0 + i * 1_000_000 / fps as i64;
        let pkts = video
            .encode_yuv420p(i, t, |y, u, v, sy, _| {
                for (row, line) in y.chunks_mut(sy).enumerate() {
                    for (x, px) in line.iter_mut().enumerate() {
                        *px = ((x + row + i as usize * 4) % 256) as u8;
                    }
                }
                u.fill(128);
                v.fill(128);
            })
            .unwrap();
        pkts.into_iter().for_each(|p| buf.push(p));

        // 1/fps de áudio por frame; simula silêncio (sem pacotes) entre 2 s e 3 s.
        let secs = i as f64 / fps as f64;
        if !(2.0..3.0).contains(&secs) {
            let n = 44_100 / fps as usize;
            let s: Vec<f32> = (0..n * 2)
                .map(|k| ((k / 2) as f32 * 0.05).sin() * 0.3)
                .collect();
            game.push(&s, t)
                .unwrap()
                .into_iter()
                .for_each(|p| buf.push(p));
            let m = vec![0.01f32; 48_000 / fps as usize];
            mic.push(&m, t)
                .unwrap()
                .into_iter()
                .for_each(|p| buf.push(p));
        }
    }
    video.flush().unwrap().into_iter().for_each(|p| buf.push(p));

    let snap = buf.snapshot(None);
    assert!(snap.video[0].keyframe);
    let dir = std::env::temp_dir().join(format!("clipe-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("clip.mp4");
    mux::write_mp4(
        &path,
        &[video.stream_info(), game.stream_info(), mic.stream_info()],
        &snap,
    )
    .expect("mux");
    assert!(path.exists());
    assert!(!path.with_extension("mp4.part").exists());

    if let Some(info) = ffprobe(&path, "format=duration:stream=codec_type") {
        println!("{info}");
        let v = info.matches("codec_type=video").count();
        let a = info.matches("codec_type=audio").count();
        assert_eq!((v, a), (1, 2), "streams: {info}");
        let dur: f64 = info
            .lines()
            .find_map(|l| l.strip_prefix("duration="))
            .and_then(|d| d.parse().ok())
            .unwrap();
        assert!((2.9..=4.2).contains(&dur), "duração {dur}");
    } else {
        eprintln!("ffprobe ausente: verificação de streams pulada");
    }

    // Metadados e miniatura do arquivo pronto.
    let info = app_media::probe(&path).expect("probe");
    assert_eq!((info.width, info.height, info.fps), (320, 180, 30));
    assert_eq!(info.audio_tracks, 2);
    // libx264 no Linux; no Windows (FFmpeg LGPL) o encoder de software é o mpeg4.
    assert!(
        matches!(info.video_codec.as_str(), "h264" | "mpeg4"),
        "{info:?}"
    );
    assert!((2900..=4200).contains(&info.duration_ms), "{info:?}");
    let thumb = dir.join("thumb.jpg");
    let at = app_media::thumbnail_time_ms(info.duration_ms);
    let (w, h) = app_media::thumbnail(&path, at, &thumb, 160).expect("thumbnail");
    assert_eq!((w, h), (160, 90));
    let jpg = std::fs::read(&thumb).unwrap();
    assert_eq!(&jpg[..2], &[0xFF, 0xD8], "assinatura JPEG");
    assert!(jpg.len() > 500);

    // Trilhas em PCM 48 kHz estéreo, alinhadas ao vídeo.
    let tracks = app_media::decode_tracks(&path).expect("pcm");
    assert_eq!(tracks.len(), 2);
    for t in &tracks {
        assert!(
            (t.duration_ms() - info.duration_ms).abs() < 300,
            "trilha {} ms, vídeo {} ms",
            t.duration_ms(),
            info.duration_ms
        );
    }
    // Jogo: seno de amplitude 0,3 (~9800); microfone: constante baixa.
    let peak = |t: &app_media::PcmTrack| t.samples.iter().map(|s| s.unsigned_abs()).max();
    assert!(peak(&tracks[0]).unwrap() > 6_000, "{:?}", peak(&tracks[0]));
    assert!(peak(&tracks[1]).unwrap() < 3_000, "{:?}", peak(&tracks[1]));

    // Tira de miniaturas da linha do tempo.
    let strip = app_media::strip(&path, 4, 64, 64).expect("strip");
    assert_eq!(strip.len(), 4);
    assert!(strip.iter().all(|f| (f.width, f.height) == (64, 36)));

    // Publicação: 0,5 s a 2,5 s, só a trilha do jogo, em H.264 + AAC com faststart.
    if app_media::h264_encoders().is_empty() {
        eprintln!("sem encoder H.264 nesta build: publicação pulada");
    } else {
        let out = dir.join("pub.mp4");
        let opts = app_media::ExportOptions {
            start_ms: 500,
            end_ms: 2500,
            tracks: vec![true, false],
            max_height: 1080,
            watermark: None,
        };
        let mut last = 0.0f32;
        let r = app_media::export_mp4(&path, &out, &opts, &mut |p| {
            assert!(p >= last, "progresso voltou: {last} -> {p}");
            last = p;
            true
        })
        .expect("publicar");
        println!("publicado: {r:?}");
        assert_eq!(last, 1.0);
        assert_eq!((r.width, r.height), (320, 180));
        assert!((1900..=2100).contains(&r.duration_ms), "{r:?}");
        let pi = app_media::probe(&out).expect("probe publicado");
        assert_eq!(pi.video_codec, "h264", "{pi:?}");
        assert_eq!(pi.audio_tracks, 1);
        assert!((1800..=2300).contains(&pi.duration_ms), "{pi:?}");
        // faststart: o moov vem antes do mdat.
        let bytes = std::fs::read(&out).unwrap();
        let find = |tag: &[u8]| bytes.windows(4).position(|w| w == tag);
        assert!(
            find(b"moov").unwrap() < find(b"mdat").unwrap(),
            "moov depois do mdat"
        );
        // Trilha mixada = só o jogo (o seno), sem o deslocamento do microfone.
        let mixed = app_media::decode_tracks(&out).expect("pcm publicado");
        assert_eq!(mixed.len(), 1);
        assert!(peak(&mixed[0]).unwrap() > 6_000);

        // Com marca d'água (branca, opaca, no canto inferior direito): o canto clareia e o
        // resto do quadro não muda.
        let marked = dir.join("marca.mp4");
        let with_mark = app_media::ExportOptions {
            watermark: Some(app_media::Watermark {
                rgba: [255u8; 4].repeat(40 * 10).into(),
                width: 40,
                height: 10,
                corner: app_media::Corner::BottomRight,
            }),
            ..opts.clone()
        };
        assert_ne!(
            with_mark, opts,
            "a marca entra na comparação de reaproveitar"
        );
        app_media::export_mp4(&path, &marked, &with_mark, &mut |_| true).expect("com marca");
        let frame = |p: &std::path::Path| {
            let f = app_media::strip(p, 1, 320, 180).expect("quadro").remove(0);
            assert_eq!((f.width, f.height), (320, 180));
            f.data
        };
        let (plain, mark) = (frame(&out), frame(&marked));
        // 180p: marca de 6 px de altura (24 de largura) a 4 px da borda. Canal verde (BGRA).
        let at = |d: &[u8], x: usize, y: usize| d[(y * 320 + x) * 4 + 1];
        assert!(
            at(&mark, 320 - 12, 180 - 7) > at(&plain, 320 - 12, 180 - 7).saturating_add(60),
            "marca não apareceu"
        );
        assert!(at(&mark, 20, 20).abs_diff(at(&plain, 20, 20)) < 24);

        // Cancelar no meio não deixa arquivo.
        let out2 = dir.join("cancel.mp4");
        let e = app_media::export_mp4(&path, &out2, &opts, &mut |p| p < 0.5);
        assert!(matches!(e, Err(app_media::MediaError::Cancelled)), "{e:?}");
        assert!(!out2.exists());
    }
    std::fs::remove_dir_all(&dir).ok();
}
