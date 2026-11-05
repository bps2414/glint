//! Decoder e motor do player sobre um MP4 gerado aqui (encode por software).

use std::path::{Path, PathBuf};
use std::time::Duration;

use app_core::{Codec, ReplayBuffer};
use app_media::{
    mux, HwPreference, Player, PlayerEvent, VideoDecoder, VideoEncoder, VideoSettings,
};

const FPS: u32 = 30;
const SECS: i64 = 4;

/// Clipe 320×180, 30 fps, 4 s, keyframe a cada 1 s. O brilho do quadro `i`
/// codifica o índice (permite conferir que o seek caiu no quadro certo).
fn make_clip(dir: &Path) -> PathBuf {
    let settings = VideoSettings {
        codec: Codec::H264,
        width: 320,
        height: 180,
        fps: FPS,
        bitrate_kbps: 800,
        gop: FPS,
    };
    let mut video = VideoEncoder::open_software(&settings).expect("encoder sw");
    let mut buf = ReplayBuffer::new(30, 512 << 20);
    let t0 = 5_000_000i64;
    for i in 0..FPS as i64 * SECS {
        let t = t0 + i * 1_000_000 / FPS as i64;
        let pkts = video
            .encode_yuv420p(i, t, |y, u, v, _, _| {
                y.fill(16 + (i as u8 % 120) * 2);
                u.fill(128);
                v.fill(128);
            })
            .unwrap();
        pkts.into_iter().for_each(|p| buf.push(p));
    }
    video.flush().unwrap().into_iter().for_each(|p| buf.push(p));
    let path = dir.join("clip.mp4");
    mux::write_mp4(&path, &[video.stream_info()], &buf.snapshot(None)).expect("mux");
    path
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("clipe-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn decoder_le_todos_os_quadros_e_converte() {
    let dir = temp_dir("decoder");
    let path = make_clip(&dir);
    let mut dec = VideoDecoder::open(&path, HwPreference::Off).expect("abrir");
    let info = dec.info().clone();
    assert_eq!((info.width, info.height), (320, 180));
    assert!((info.fps - 30.0).abs() < 0.5, "{info:?}");
    assert!(info.hw.is_none());
    assert!((3_800..=4_200).contains(&info.duration_ms), "{info:?}");

    let mut n = 0;
    let mut last = -1;
    while let Some(pts) = dec.decode_next().expect("next") {
        assert!(pts > last, "pts fora de ordem: {pts} depois de {last}");
        last = pts;
        n += 1;
    }
    assert_eq!(n, FPS as i64 * SECS);

    // Último quadro convertido para metade do tamanho.
    let f = dec.convert(160, 160).expect("convert");
    assert_eq!((f.width, f.height), (160, 90));
    assert_eq!(f.data.len(), 160 * 90 * 4);
    // BGRA opaco e cinza (u = v = 128).
    let px = &f.data[(45 * 160 + 80) * 4..][..4];
    assert_eq!(px[3], 255);
    assert!(px[0].abs_diff(px[2]) <= 3, "{px:?}");

    // Seek leva ao keyframe anterior.
    dec.seek(2_500).expect("seek");
    let pts = dec.decode_next().unwrap().unwrap();
    assert!((1_900..=2_100).contains(&pts), "keyframe em {pts}");
}

fn next_frame(p: &Player, gen: u64) -> Option<(i64, u32)> {
    loop {
        match p
            .next_timeout(Duration::from_secs(5))
            .expect("evento do player")
        {
            PlayerEvent::Frame {
                gen: g,
                frame,
                skipped,
                ..
            } if g == gen => return Some((frame.pts_ms, skipped)),
            PlayerEvent::Eof { gen: g } if g == gen => return None,
            PlayerEvent::Error { message, .. } => panic!("erro do player: {message}"),
            _ => {}
        }
    }
}

#[test]
fn player_entrega_em_ordem_e_faz_seek_exato() {
    let dir = temp_dir("player");
    let path = make_clip(&dir);
    let mut p = Player::open(&path, HwPreference::Off, (320, 180)).expect("player");

    // Geração 0: começa do início.
    let (pts, _) = next_frame(&p, 0).unwrap();
    assert!(pts < 40, "primeiro quadro em {pts}");

    // Seek exato no meio de um GOP: pula até o alvo e entrega o quadro certo.
    let g = p.seek(2_500);
    let (pts, skipped) = next_frame(&p, g).unwrap();
    assert!((pts - 2_500).abs() <= 17, "seek caiu em {pts}");
    assert!(
        skipped >= 10,
        "deveria pular quadros desde o keyframe: {skipped}"
    );

    // Seek logo à frente: decodifica para a frente, sem voltar ao keyframe.
    let g = p.seek(2_800);
    let (pts, skipped) = next_frame(&p, g).unwrap();
    assert!((pts - 2_800).abs() <= 17, "seek caiu em {pts}");
    assert!(skipped < 12, "não deveria voltar ao keyframe: {skipped}");

    // Rajada de seeks (scrub): só o último vale.
    for ms in [100, 900, 1_700, 3_300] {
        p.seek(ms);
    }
    let g = p.generation();
    let (pts, _) = next_frame(&p, g).unwrap();
    assert!((pts - 3_300).abs() <= 17, "seek caiu em {pts}");

    // Tamanho menor vale para os próximos quadros.
    p.set_target(160, 90);
    let g = p.seek(1_000);
    // A fila ainda pode ter quadros da geração anterior: a UI os descarta, o teste também.
    loop {
        match p.next_timeout(Duration::from_secs(5)) {
            Some(PlayerEvent::Frame { gen, frame, .. }) if gen == g => {
                assert_eq!((frame.width, frame.height), (160, 90));
                break;
            }
            Some(PlayerEvent::Frame { gen, .. }) | Some(PlayerEvent::Eof { gen }) if gen < g => {}
            other => panic!("esperava quadro: {other:?}"),
        }
    }

    // Seek além do fim: entrega o último quadro e o fim.
    let g = p.seek(60_000);
    let (pts, _) = next_frame(&p, g).unwrap();
    assert!(pts >= 3_900, "último quadro em {pts}");
    assert!(next_frame(&p, g).is_none());

    // Depois do fim, voltar ao início funciona.
    let g = p.seek(0);
    let (pts, _) = next_frame(&p, g).unwrap();
    assert!(pts < 40);
    drop(p);
}

#[test]
fn scrub_mostra_o_keyframe_e_depois_o_quadro_exato() {
    let dir = temp_dir("player-scrub");
    let path = make_clip(&dir);
    let mut p = Player::open(&path, HwPreference::Off, (160, 90)).expect("player");
    let g = p.scrub(2_500);
    let mut got = Vec::new();
    while got.len() < 2 {
        match p.next_timeout(Duration::from_secs(5)).expect("evento") {
            PlayerEvent::Frame {
                gen,
                frame,
                preview,
                ..
            } if gen == g => got.push((frame.pts_ms, preview)),
            PlayerEvent::Error { message, .. } => panic!("erro do player: {message}"),
            _ => {}
        }
    }
    assert!(got[0].1, "primeiro deveria ser prévia: {got:?}");
    assert!((1_900..=2_100).contains(&got[0].0), "prévia em {got:?}");
    assert!(!got[1].1 && (got[1].0 - 2_500).abs() <= 17, "{got:?}");

    // Alvo no próprio keyframe: sem prévia, sai direto o exato.
    let g = p.scrub(3_000);
    loop {
        match p.next_timeout(Duration::from_secs(5)).expect("evento") {
            PlayerEvent::Frame {
                gen,
                frame,
                preview,
                ..
            } if gen == g => {
                assert!(!preview);
                assert!((frame.pts_ms - 3_000).abs() <= 17);
                break;
            }
            _ => {}
        }
    }
}

#[test]
fn player_toca_ate_o_fim() {
    let dir = temp_dir("player-eof");
    let path = make_clip(&dir);
    let p = Player::open(&path, HwPreference::Off, (64, 64)).expect("player");
    let mut n = 0;
    let mut last = -1;
    while let Some((pts, _)) = next_frame(&p, 0) {
        assert!(pts > last);
        last = pts;
        n += 1;
    }
    assert_eq!(n, FPS as i64 * SECS);
}
