use std::ffi::CString;
use std::path::Path;
use std::ptr;

use app_core::{EncodedPacket, ReplaySnapshot};
use ffmpeg_sys_next as ff;

use crate::error::{check, MediaError, Result};
use crate::stream::StreamInfo;

/// Escreve um snapshot do replay buffer em MP4 **sem re-encode**.
///
/// `streams[0]` é o vídeo; `streams[i]` é a stream de áudio com
/// `EncodedPacket::stream == i`. O arquivo é escrito num temporário e
/// renomeado no fim, para a galeria nunca ver arquivo pela metade.
pub fn write_mp4(path: &Path, streams: &[&StreamInfo], snap: &ReplaySnapshot) -> Result<()> {
    write_mp4_with(path, streams, snap, false)
}

/// Como [`write_mp4`]; com `faststart` o índice (moov) vai para o começo do
/// arquivo, para o navegador e o embed do Discord tocarem sem baixar tudo antes.
pub fn write_mp4_with(
    path: &Path,
    streams: &[&StreamInfo],
    snap: &ReplaySnapshot,
    faststart: bool,
) -> Result<()> {
    let start_us = snap
        .start_us()
        .ok_or_else(|| MediaError::Other("buffer vazio: nada para salvar".into()))?;
    let tmp = path.with_extension("mp4.part");
    let tmp_str = tmp
        .to_str()
        .ok_or_else(|| MediaError::Other("caminho não é UTF-8".into()))?;
    let c_path = CString::new(tmp_str).map_err(|e| MediaError::Other(e.to_string()))?;

    unsafe {
        let mut oc: *mut ff::AVFormatContext = ptr::null_mut();
        check(
            ff::avformat_alloc_output_context2(
                &mut oc,
                ptr::null(),
                c"mp4".as_ptr(),
                c_path.as_ptr(),
            ),
            "avformat_alloc_output_context2",
        )?;
        let guard = OutputGuard(oc);
        let result = write_inner(oc, &c_path, streams, snap, start_us, faststart);
        drop(guard);
        match result {
            Ok(()) => {
                std::fs::rename(&tmp, path)?;
                Ok(())
            }
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                Err(e)
            }
        }
    }
}

struct OutputGuard(*mut ff::AVFormatContext);

impl Drop for OutputGuard {
    fn drop(&mut self) {
        unsafe {
            if !(*self.0).pb.is_null() {
                ff::avio_closep(&mut (*self.0).pb);
            }
            ff::avformat_free_context(self.0);
        }
    }
}

unsafe fn write_inner(
    oc: *mut ff::AVFormatContext,
    c_path: &CString,
    streams: &[&StreamInfo],
    snap: &ReplaySnapshot,
    start_us: i64,
    faststart: bool,
) -> Result<()> {
    let mut first_audio = true;
    for info in streams {
        let st = ff::avformat_new_stream(oc, ptr::null());
        if st.is_null() {
            return Err(MediaError::Other("avformat_new_stream".into()));
        }
        check(
            ff::avcodec_parameters_copy((*st).codecpar, info.params),
            "avcodec_parameters_copy",
        )?;
        (*st).time_base = info.time_base;
        let par = &mut *(*st).codecpar;
        par.codec_tag = 0;
        if par.codec_id == ff::AVCodecID::AV_CODEC_ID_HEVC {
            // 'hvc1' é o que navegadores/Apple esperam para HEVC em MP4.
            par.codec_tag = u32::from_le_bytes(*b"hvc1");
        }
        if par.codec_type == ff::AVMediaType::AVMEDIA_TYPE_AUDIO {
            (*st).disposition = if first_audio {
                ff::AV_DISPOSITION_DEFAULT as i32
            } else {
                0
            };
            first_audio = false;
        }
    }

    check(
        ff::avio_open(&mut (*oc).pb, c_path.as_ptr(), ff::AVIO_FLAG_WRITE as i32),
        "avio_open",
    )?;
    let mut opts: *mut ff::AVDictionary = ptr::null_mut();
    if faststart {
        ff::av_dict_set(&mut opts, c"movflags".as_ptr(), c"+faststart".as_ptr(), 0);
    }
    let ret = ff::avformat_write_header(oc, &mut opts);
    ff::av_dict_free(&mut opts);
    check(ret, "avformat_write_header")?;

    // Primeiro pacote de cada stream: base para reescrever timestamps a partir de 0.
    let mut firsts: Vec<Option<&EncodedPacket>> = vec![None; streams.len()];
    for p in snap.video.iter().chain(snap.audio.iter()) {
        if p.stream < firsts.len() && firsts[p.stream].is_none() {
            firsts[p.stream] = Some(p);
        }
    }

    // Intercala por tempo.
    let mut all: Vec<&EncodedPacket> = snap
        .video
        .iter()
        .chain(snap.audio.iter())
        .filter(|p| p.stream < streams.len())
        .collect();
    all.sort_by_key(|p| (p.time_us, p.stream));

    let pkt = ff::av_packet_alloc();
    let mut result = Ok(());
    for p in all {
        let info = streams[p.stream];
        let first = firsts[p.stream].expect("stream tem pacotes");
        let enc_tb = info.time_base;
        let us_tb = ff::AVRational {
            num: 1,
            den: 1_000_000,
        };
        let base = ff::av_rescale_q(first.time_us - start_us, us_tb, enc_tb);
        let shift = base - first.dts.min(first.pts);

        if ff::av_new_packet(pkt, p.data.len() as i32) < 0 {
            result = Err(MediaError::Other("av_new_packet".into()));
            break;
        }
        ptr::copy_nonoverlapping(p.data.as_ptr(), (*pkt).data, p.data.len());
        (*pkt).pts = p.pts + shift;
        (*pkt).dts = p.dts + shift;
        (*pkt).duration = p.duration;
        (*pkt).stream_index = p.stream as i32;
        (*pkt).flags = if p.keyframe {
            ff::AV_PKT_FLAG_KEY as i32
        } else {
            0
        };
        let st = *(*oc).streams.add(p.stream);
        ff::av_packet_rescale_ts(pkt, enc_tb, (*st).time_base);
        let ret = ff::av_interleaved_write_frame(oc, pkt);
        if let Err(e) = check(ret, "av_interleaved_write_frame") {
            result = Err(e);
            break;
        }
    }
    let mut pkt = pkt;
    ff::av_packet_free(&mut pkt);
    result?;
    check(ff::av_write_trailer(oc), "av_write_trailer")?;
    Ok(())
}
