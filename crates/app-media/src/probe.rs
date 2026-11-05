//! Leitura de clipes prontos: metadados ([`probe`]) e miniatura JPEG
//! ([`thumbnail`]). Decodifica por software um único quadro (keyframe), então
//! funciona igual no Windows e no Linux e custa poucos ms.

use std::path::Path;
use std::ptr;

use ffmpeg_sys_next as ff;

use crate::error::{check, MediaError, Result, EAGAIN};
use crate::ffi::{CodecCtx as Decoder, Frame, Input, Packet};

#[derive(Clone, Debug, PartialEq)]
pub struct ProbeInfo {
    pub duration_ms: i64,
    pub width: u32,
    pub height: u32,
    /// Quadros por segundo (arredondado).
    pub fps: u32,
    /// Nome curto do codec de vídeo (`hevc`, `h264`...).
    pub video_codec: String,
    pub audio_tracks: u32,
}

pub fn probe(path: &Path) -> Result<ProbeInfo> {
    let input = Input::open(path)?;
    let (_, vs) = input
        .video_stream()
        .ok_or_else(|| MediaError::Other("arquivo sem vídeo".into()))?;
    unsafe {
        let par = &*(*vs).codecpar;
        let fr = (*vs).avg_frame_rate;
        let fps = if fr.den > 0 {
            (f64::from(fr.num) / f64::from(fr.den)).round() as u32
        } else {
            0
        };
        let dur = (*input.0).duration;
        let duration_ms = if dur > 0 {
            dur / (ff::AV_TIME_BASE as i64 / 1000)
        } else {
            0
        };
        let name = std::ffi::CStr::from_ptr(ff::avcodec_get_name(par.codec_id))
            .to_string_lossy()
            .into_owned();
        let audio_tracks = input
            .streams()
            .iter()
            .filter(|s| (*(***s).codecpar).codec_type == ff::AVMediaType::AVMEDIA_TYPE_AUDIO)
            .count() as u32;
        Ok(ProbeInfo {
            duration_ms,
            width: par.width.max(0) as u32,
            height: par.height.max(0) as u32,
            fps,
            video_codec: name,
            audio_tracks,
        })
    }
}

/// Onde tirar a miniatura: perto do fim, onde costuma estar a jogada
/// (o clipe é salvo logo depois dela).
pub fn thumbnail_time_ms(duration_ms: i64) -> i64 {
    (duration_ms - 2_000).max(duration_ms / 2).max(0)
}

/// Salva em `out` (JPEG) o quadro perto de `at_ms`, com no máximo
/// `max_width` de largura. Devolve o tamanho da imagem.
pub fn thumbnail(path: &Path, at_ms: i64, out: &Path, max_width: u32) -> Result<(u32, u32)> {
    let input = Input::open(path)?;
    let (index, vs) = input
        .video_stream()
        .ok_or_else(|| MediaError::Other("arquivo sem vídeo".into()))?;
    unsafe {
        let par = (*vs).codecpar;
        let codec = ff::avcodec_find_decoder((*par).codec_id);
        if codec.is_null() {
            return Err(MediaError::Other("sem decoder para o vídeo".into()));
        }
        let dec = Decoder(ff::avcodec_alloc_context3(codec));
        if dec.0.is_null() {
            return Err(MediaError::Other("avcodec_alloc_context3".into()));
        }
        check(
            ff::avcodec_parameters_to_context(dec.0, par),
            "avcodec_parameters_to_context",
        )?;
        (*dec.0).thread_count = 1;
        check(
            ff::avcodec_open2(dec.0, codec, ptr::null_mut()),
            "avcodec_open2 (decoder)",
        )?;

        let tb = (*vs).time_base;
        let ts = ff::av_rescale_q(at_ms, ff::AVRational { num: 1, den: 1000 }, tb);
        // Falha no seek não é fatal: usa o primeiro quadro.
        let _ = ff::av_seek_frame(input.0, index as i32, ts, ff::AVSEEK_FLAG_BACKWARD as i32);

        let frame = Frame::alloc()?;
        let pkt = Packet::alloc()?;
        let mut got = false;
        let mut eof = false;
        while !got {
            if !eof {
                let r = ff::av_read_frame(input.0, pkt.0);
                if r < 0 {
                    eof = true;
                    ff::avcodec_send_packet(dec.0, ptr::null());
                } else {
                    if (*pkt.0).stream_index == index as i32 {
                        let s = ff::avcodec_send_packet(dec.0, pkt.0);
                        if s < 0 && s != EAGAIN {
                            ff::av_packet_unref(pkt.0);
                            check(s, "avcodec_send_packet")?;
                        }
                    }
                    ff::av_packet_unref(pkt.0);
                }
            }
            let r = ff::avcodec_receive_frame(dec.0, frame.0);
            if r >= 0 {
                got = true;
            } else if r == EAGAIN && !eof {
                continue;
            } else {
                return Err(MediaError::Other(
                    "não foi possível decodificar um quadro".into(),
                ));
            }
        }

        let (sw, sh) = ((*frame.0).width, (*frame.0).height);
        if sw <= 0 || sh <= 0 {
            return Err(MediaError::Other("quadro vazio".into()));
        }
        let dw = (max_width as i32).min(sw).max(2) & !1;
        let dh = ((i64::from(dw) * i64::from(sh) / i64::from(sw)) as i32).max(2) & !1;
        let jpeg = encode_jpeg(frame.0, dw, dh)?;
        if let Some(dir) = out.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = out.with_extension("jpg.part");
        std::fs::write(&tmp, &jpeg)?;
        std::fs::rename(&tmp, out)?;
        Ok((dw as u32, dh as u32))
    }
}

/// `SWS_BICUBIC` (= 4 em todas as versões; no FFmpeg 8 virou membro do enum `SwsFlags`).
const SWS_BICUBIC: i32 = 4;

/// Escala o quadro para `w`×`h` em YUVJ420P e codifica com o MJPEG do FFmpeg.
unsafe fn encode_jpeg(src: *const ff::AVFrame, w: i32, h: i32) -> Result<Vec<u8>> {
    let dst_fmt = ff::AVPixelFormat::AV_PIX_FMT_YUVJ420P;
    // SAFETY: `format` de um quadro de vídeo decodificado é um AVPixelFormat válido.
    let src_fmt: ff::AVPixelFormat = std::mem::transmute::<i32, ff::AVPixelFormat>((*src).format);
    let sws = ff::sws_getContext(
        (*src).width,
        (*src).height,
        src_fmt,
        w,
        h,
        dst_fmt,
        SWS_BICUBIC,
        ptr::null_mut(),
        ptr::null_mut(),
        ptr::null(),
    );
    if sws.is_null() {
        return Err(MediaError::Other("sws_getContext".into()));
    }
    let scaled = Frame::alloc()?;
    (*scaled.0).width = w;
    (*scaled.0).height = h;
    (*scaled.0).format = dst_fmt as i32;
    let r = ff::av_frame_get_buffer(scaled.0, 0);
    if r < 0 {
        ff::sws_freeContext(sws);
        check(r, "av_frame_get_buffer")?;
    }
    ff::sws_scale(
        sws,
        (*src).data.as_ptr() as *const *const u8,
        (*src).linesize.as_ptr(),
        0,
        (*src).height,
        (*scaled.0).data.as_ptr(),
        (*scaled.0).linesize.as_ptr(),
    );
    ff::sws_freeContext(sws);

    let codec = ff::avcodec_find_encoder(ff::AVCodecID::AV_CODEC_ID_MJPEG);
    if codec.is_null() {
        return Err(MediaError::NoEncoder("mjpeg".into()));
    }
    let enc = Decoder(ff::avcodec_alloc_context3(codec));
    if enc.0.is_null() {
        return Err(MediaError::Other("avcodec_alloc_context3".into()));
    }
    let c = &mut *enc.0;
    c.width = w;
    c.height = h;
    c.pix_fmt = dst_fmt;
    c.time_base = ff::AVRational { num: 1, den: 25 };
    c.color_range = ff::AVColorRange::AVCOL_RANGE_JPEG;
    // Qualidade fixa (q 3 ≈ JPEG ~90).
    c.flags |= ff::AV_CODEC_FLAG_QSCALE as i32;
    c.global_quality = ff::FF_QP2LAMBDA as i32 * 3;
    check(
        ff::avcodec_open2(enc.0, codec, ptr::null_mut()),
        "avcodec_open2 (mjpeg)",
    )?;
    (*scaled.0).quality = c.global_quality;
    (*scaled.0).pts = 0;
    check(
        ff::avcodec_send_frame(enc.0, scaled.0),
        "avcodec_send_frame",
    )?;
    ff::avcodec_send_frame(enc.0, ptr::null());
    let pkt = Packet::alloc()?;
    check(
        ff::avcodec_receive_packet(enc.0, pkt.0),
        "avcodec_receive_packet",
    )?;
    let data = std::slice::from_raw_parts((*pkt.0).data, (*pkt.0).size.max(0) as usize).to_vec();
    ff::av_packet_unref(pkt.0);
    Ok(data)
}
