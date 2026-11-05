//! Camada de mídia do app (FFmpeg via FFI).
//!
//! - [`VideoEncoder`]: encoder de vídeo (hardware: AMF/NVENC/QSV; software só
//!   para testes).
//! - [`d3d11`]: entrada zero-copy de texturas D3D11 (Windows), sem depender do
//!   crate `windows` — recebe ponteiros COM crus da camada de plataforma.
//! - [`AudioEncoder`]: AAC com reamostragem e preenchimento de silêncio.
//! - [`mux::write_mp4`]: salva um snapshot do replay buffer por *remux*.
//! - [`probe`] / [`thumbnail`]: metadados e miniatura JPEG de um clipe pronto.
//! - [`VideoDecoder`] / [`Player`]: decodificação (hardware quando dá) e motor do
//!   player da UI (thread própria, fila curta, seek exato coalescido); [`strip`]:
//!   miniaturas da linha do tempo.
//! - [`decode_tracks`]: trilhas de áudio inteiras em PCM (player e publicação).
//! - [`export_mp4`]: publicação (recorte em H.264 + AAC mixado, `+faststart`), com
//!   [`Watermark`] opcional.

// Os tipos das constantes do FFmpeg variam entre versões/plataformas; casts
// explícitos mantêm o código portável.
#![allow(clippy::unnecessary_cast)]

pub mod audio;
pub mod d3d11;
pub mod decode;
mod error;
pub mod export;
mod ffi;
pub mod mux;
pub mod pcm;
pub mod player;
pub mod probe;
mod stream;
pub mod video;
pub mod watermark;

pub use audio::AudioEncoder;
pub use decode::{strip, BgraFrame, HwPreference, VideoDecoder, VideoInfo};
pub use error::{MediaError, Result};
pub use export::{export_mp4, h264_encoders, ExportOptions, Exported};
pub use pcm::{decode_tracks, PcmTrack};
pub use player::{Player, PlayerEvent};
pub use probe::{probe, thumbnail, thumbnail_time_ms, ProbeInfo};
pub use stream::StreamInfo;
pub use video::{VideoEncoder, VideoSettings};
pub use watermark::{Corner, Watermark};

/// Versão do libavcodec carregado em runtime (para diagnóstico).
pub fn ffmpeg_version() -> String {
    let v = unsafe { ffmpeg_sys_next::avcodec_version() };
    format!("libavcodec {}.{}.{}", v >> 16, (v >> 8) & 0xff, v & 0xff)
}

/// Lista quais encoders de vídeo de hardware conhecidos existem nesta build do
/// FFmpeg (existir não garante que a GPU suporte — isso só se sabe abrindo).
pub fn available_video_encoders() -> Vec<&'static str> {
    video::HW_ENCODERS
        .iter()
        .copied()
        .filter(|name| video::find_encoder(name).is_some())
        .collect()
}

/// Encaminha os logs do FFmpeg para `tracing` (nível warning ou pior).
pub fn init_logging() {
    unsafe {
        ffmpeg_sys_next::av_log_set_level(ffmpeg_sys_next::AV_LOG_WARNING as i32);
    }
}
