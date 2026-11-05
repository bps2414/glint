//! Publicação: re-encode do trecho recortado em H.264 + AAC (uma trilha, com as
//! trilhas ligadas mixadas) num MP4 com `+faststart`, o formato que o navegador
//! e o embed do Discord tocam.
//!
//! O vídeo é decodificado na GPU quando dá ([`VideoDecoder`]), copiado para a
//! CPU, escalado se passar de `max_height` e codificado pelo primeiro encoder
//! H.264 que abrir (AMF, NVENC, QSV, Media Foundation; software por último).
//! A marca d'água opcional ([`Watermark`]) é composta no quadro que vai para o
//! encoder; o arquivo de origem nunca muda.

use std::path::Path;
use std::ptr;

use app_core::{Codec, ReplaySnapshot};
use ffmpeg_sys_next as ff;

use crate::audio::AudioEncoder;
use crate::decode::{HwPreference, VideoDecoder};
use crate::error::{check, MediaError, Result};
use crate::ffi::Frame;
use crate::pcm::{self, RATE};
use crate::video::{find_encoder, VideoEncoder, VideoSettings};
use crate::watermark::{Overlay, Watermark};

/// Encoders H.264 em ordem de preferência; `true` = recebe NV12 (os de
/// hardware), `false` = YUV420P.
const H264_ENCODERS: &[(&str, bool)] = &[
    ("h264_amf", true),
    ("h264_nvenc", true),
    ("h264_qsv", true),
    ("h264_mf", true),
    ("libx264", false),
    ("libopenh264", false),
];

/// Encoders H.264 desta build do FFmpeg (existir não garante que a GPU abra).
pub fn h264_encoders() -> Vec<&'static str> {
    H264_ENCODERS
        .iter()
        .map(|(n, _)| *n)
        .filter(|n| find_encoder(n).is_some())
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportOptions {
    pub start_ms: i64,
    /// Fim do trecho; `<= start_ms` = até o fim do arquivo.
    pub end_ms: i64,
    /// Trilhas de áudio ligadas, na ordem do arquivo (faltando = ligada).
    pub tracks: Vec<bool>,
    /// Altura máxima do vídeo publicado (nunca aumenta).
    pub max_height: u32,
    /// Marca d'água no canto (`None` = vídeo limpo).
    pub watermark: Option<Watermark>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Exported {
    pub encoder: String,
    pub width: u32,
    pub height: u32,
    pub duration_ms: i64,
    pub size_bytes: u64,
}

/// Bitrate do vídeo publicado: ~0,1 bit por pixel por quadro (12 Mbps em
/// 1080p60), entre 2 e 16 Mbps.
pub fn bitrate_kbps(width: u32, height: u32, fps: u32) -> u32 {
    let bits = f64::from(width) * f64::from(height) * f64::from(fps) * 0.1;
    ((bits / 1000.0) as u32).clamp(2_000, 16_000)
}

/// Tamanho de saída: cabe em `max_height`, mantém a proporção, lados pares.
pub fn output_size(width: u32, height: u32, max_height: u32) -> (u32, u32) {
    let (w, h) = if max_height > 0 && height > max_height {
        let w = (f64::from(width) * f64::from(max_height) / f64::from(height)).round() as u32;
        (w, max_height)
    } else {
        (width, height)
    };
    ((w & !1).max(2), (h & !1).max(2))
}

/// Gera `dst` a partir de `src`. `progress(0..1)` é chamado durante o trabalho;
/// devolver `false` cancela (erro [`MediaError::Cancelled`], sem arquivo).
pub fn export_mp4(
    src: &Path,
    dst: &Path,
    opts: &ExportOptions,
    progress: &mut dyn FnMut(f32) -> bool,
) -> Result<Exported> {
    let mut dec = VideoDecoder::open(src, HwPreference::Auto)?;
    let info = dec.info().clone();
    let fps = info.fps.round().clamp(1.0, 240.0) as u32;
    let start = opts.start_ms.clamp(0, info.duration_ms.max(0));
    let end = if opts.end_ms <= start {
        info.duration_ms
    } else {
        opts.end_ms.min(info.duration_ms)
    };
    if end - start < 100 {
        return Err(MediaError::Other(
            "trecho curto demais para publicar".into(),
        ));
    }

    // Áudio primeiro (rápido): é ~10% da barra.
    let audio_pkts;
    let mut aenc = AudioEncoder::new(1, RATE, 2, 160)?;
    {
        let tracks = pcm::decode_tracks(src)?;
        let mix = mix_range(&tracks, &opts.tracks, start, end);
        let mut out = Vec::new();
        // Pedaços de 100 ms, contíguos (o encoder não insere silêncio).
        let chunk = RATE as usize / 10 * 2;
        for (i, c) in mix.chunks(chunk).enumerate() {
            out.extend(aenc.push(c, i as i64 * 100_000)?);
        }
        audio_pkts = out;
    }
    if !progress(0.1) {
        return Err(MediaError::Cancelled);
    }

    let (w, h) = output_size(info.width, info.height, opts.max_height);
    let settings = VideoSettings {
        codec: Codec::H264,
        width: w,
        height: h,
        fps,
        bitrate_kbps: bitrate_kbps(w, h, fps),
        gop: fps * 2,
    };
    let (mut venc, pix) = open_h264(&settings)?;
    let total = ((end - start) as f64 * f64::from(fps) / 1000.0).max(1.0);
    let half = info.frame_ms() / 2.0;
    let mut video_pkts = Vec::new();
    let mut conv = Converter::new(w, h, pix)?;
    let overlay = opts.watermark.as_ref().and_then(|m| Overlay::new(m, w, h));
    let mut n: i64 = 0;
    dec.seek(start)?;
    while let Some(pts) = dec.decode_next()? {
        let t = pts as f64;
        if t < start as f64 - half {
            continue;
        }
        if t >= end as f64 - half {
            break;
        }
        let frame = conv.convert(dec.sw_frame()?)?;
        unsafe {
            if let Some(o) = &overlay {
                // SAFETY: quadro recém-alocado pelo `Converter`, no formato do encoder.
                o.apply(frame);
            }
            (*frame).pts = n;
            venc.send_frame(frame, n * 1_000_000 / i64::from(fps), &mut video_pkts)?;
        }
        n += 1;
        if !progress(0.1 + 0.88 * (n as f64 / total).min(1.0) as f32) {
            return Err(MediaError::Cancelled);
        }
    }
    if n == 0 {
        return Err(MediaError::Other("nenhum quadro no trecho".into()));
    }
    video_pkts.extend(venc.flush()?);

    let snap = ReplaySnapshot {
        video: video_pkts,
        audio: audio_pkts,
    };
    crate::mux::write_mp4_with(dst, &[venc.stream_info(), aenc.stream_info()], &snap, true)?;
    progress(1.0);
    Ok(Exported {
        encoder: venc.name().to_string(),
        width: w,
        height: h,
        duration_ms: n * 1000 / i64::from(fps),
        size_bytes: std::fs::metadata(dst)?.len(),
    })
}

/// Mistura as trilhas ligadas no trecho `[start, end)` (ms) em f32 estéreo.
fn mix_range(tracks: &[pcm::PcmTrack], on: &[bool], start: i64, end: i64) -> Vec<f32> {
    let rate = i64::from(RATE);
    let s0 = (start * rate / 1000) as usize;
    let frames = ((end - start) * rate / 1000) as usize;
    let mut out = vec![0f32; frames * 2];
    for (k, t) in tracks.iter().enumerate() {
        if !on.get(k).copied().unwrap_or(true) {
            continue;
        }
        let src = t.samples.get(s0 * 2..).unwrap_or(&[]);
        for (o, &s) in out.iter_mut().zip(src) {
            *o += f32::from(s) / 32768.0;
        }
    }
    for o in &mut out {
        *o = o.clamp(-1.0, 1.0);
    }
    out
}

fn open_h264(settings: &VideoSettings) -> Result<(VideoEncoder, ff::AVPixelFormat)> {
    let mut tried = Vec::new();
    for &(name, nv12) in H264_ENCODERS {
        if find_encoder(name).is_none() {
            continue;
        }
        tried.push(name);
        let pix = if nv12 {
            ff::AVPixelFormat::AV_PIX_FMT_NV12
        } else {
            ff::AVPixelFormat::AV_PIX_FMT_YUV420P
        };
        match unsafe { VideoEncoder::open(name, settings, pix, ptr::null_mut()) } {
            Ok(e) => return Ok((e, pix)),
            Err(e) => tracing::info!("encoder {name} não abriu: {e}"),
        }
    }
    Err(MediaError::NoEncoder(if tried.is_empty() {
        "nenhum encoder H.264 nesta build".into()
    } else {
        tried.join(", ")
    }))
}

/// Converte quadros decodificados (qualquer formato/tamanho) para o formato e
/// o tamanho do encoder.
struct Converter {
    w: u32,
    h: u32,
    pix: ff::AVPixelFormat,
    sws: *mut ff::SwsContext,
    out: Frame,
}

impl Drop for Converter {
    fn drop(&mut self) {
        if !self.sws.is_null() {
            unsafe { ff::sws_freeContext(self.sws) };
        }
    }
}

impl Converter {
    fn new(w: u32, h: u32, pix: ff::AVPixelFormat) -> Result<Self> {
        Ok(Self {
            w,
            h,
            pix,
            sws: ptr::null_mut(),
            out: Frame::alloc()?,
        })
    }

    fn convert(&mut self, src: *const ff::AVFrame) -> Result<*mut ff::AVFrame> {
        unsafe {
            let s = &*src;
            // SAFETY: `format` de um quadro de vídeo decodificado é um AVPixelFormat válido.
            let src_fmt = std::mem::transmute::<i32, ff::AVPixelFormat>(s.format);
            self.sws = ff::sws_getCachedContext(
                self.sws,
                s.width,
                s.height,
                src_fmt,
                self.w as i32,
                self.h as i32,
                self.pix,
                2, // SWS_BILINEAR (o valor não muda entre versões)
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null(),
            );
            if self.sws.is_null() {
                return Err(MediaError::Other("sws_getCachedContext (publicar)".into()));
            }
            let out = self.out.0;
            ff::av_frame_unref(out);
            (*out).format = self.pix as i32;
            (*out).width = self.w as i32;
            (*out).height = self.h as i32;
            check(
                ff::av_frame_get_buffer(out, 0),
                "av_frame_get_buffer (publicar)",
            )?;
            ff::sws_scale(
                self.sws,
                s.data.as_ptr() as *const *const u8,
                s.linesize.as_ptr(),
                0,
                s.height,
                (*out).data.as_ptr(),
                (*out).linesize.as_ptr(),
            );
            (*out).color_range = ff::AVColorRange::AVCOL_RANGE_MPEG;
            (*out).colorspace = ff::AVColorSpace::AVCOL_SPC_BT709;
            (*out).color_primaries = ff::AVColorPrimaries::AVCOL_PRI_BT709;
            (*out).color_trc = ff::AVColorTransferCharacteristic::AVCOL_TRC_BT709;
            Ok(out)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{bitrate_kbps, mix_range, output_size};
    use crate::pcm::PcmTrack;

    #[test]
    fn tamanho_e_bitrate() {
        assert_eq!(output_size(1920, 1080, 1080), (1920, 1080));
        assert_eq!(output_size(2560, 1440, 1080), (1920, 1080));
        assert_eq!(output_size(3440, 1440, 1080), (2580, 1080));
        assert_eq!(output_size(1365, 767, 1080), (1364, 766));
        assert_eq!(bitrate_kbps(1920, 1080, 60), 12_441);
        assert_eq!(bitrate_kbps(640, 360, 30), 2_000);
    }

    #[test]
    fn mistura_so_o_trecho_e_as_trilhas_ligadas() {
        // 2 ms a 48 kHz = 96 quadros; trilha 0 = 1000, trilha 1 = 2000.
        let t0 = PcmTrack {
            samples: vec![1000; 96 * 2 * 3],
        };
        let t1 = PcmTrack {
            samples: vec![2000; 96 * 2 * 3],
        };
        let both = mix_range(&[t0.clone(), t1.clone()], &[], 2, 4);
        assert_eq!(both.len(), 96 * 2);
        assert!((both[0] - 3000.0 / 32768.0).abs() < 1e-6);
        let game = mix_range(&[t0, t1], &[true, false], 2, 4);
        assert!((game[5] - 1000.0 / 32768.0).abs() < 1e-6);
    }
}
