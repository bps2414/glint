//! Wrappers RAII mínimos sobre estruturas do FFmpeg (entrada, codec, quadro,
//! pacote), compartilhados por [`crate::probe`] e [`crate::decode`].

use std::ffi::CString;
use std::path::Path;
use std::ptr;

use ffmpeg_sys_next as ff;

use crate::error::{check, MediaError, Result};

pub(crate) fn c_path(path: &Path) -> Result<CString> {
    let s = path
        .to_str()
        .ok_or_else(|| MediaError::Other("caminho não é UTF-8".into()))?;
    CString::new(s).map_err(|e| MediaError::Other(e.to_string()))
}

/// Contexto de entrada aberto (fecha no `drop`).
pub(crate) struct Input(pub(crate) *mut ff::AVFormatContext);

impl Input {
    pub(crate) fn open(path: &Path) -> Result<Self> {
        let cpath = c_path(path)?;
        let mut ctx: *mut ff::AVFormatContext = ptr::null_mut();
        unsafe {
            check(
                ff::avformat_open_input(&mut ctx, cpath.as_ptr(), ptr::null(), ptr::null_mut()),
                "avformat_open_input",
            )?;
            let input = Self(ctx);
            check(
                ff::avformat_find_stream_info(ctx, ptr::null_mut()),
                "avformat_find_stream_info",
            )?;
            Ok(input)
        }
    }

    pub(crate) fn streams(&self) -> &[*mut ff::AVStream] {
        unsafe {
            let n = (*self.0).nb_streams as usize;
            if n == 0 {
                return &[];
            }
            std::slice::from_raw_parts((*self.0).streams, n)
        }
    }

    pub(crate) fn video_stream(&self) -> Option<(usize, *mut ff::AVStream)> {
        self.streams()
            .iter()
            .copied()
            .enumerate()
            .find(|(_, s)| unsafe {
                (*(**s).codecpar).codec_type == ff::AVMediaType::AVMEDIA_TYPE_VIDEO
            })
    }
}

impl Drop for Input {
    fn drop(&mut self) {
        unsafe { ff::avformat_close_input(&mut self.0) };
    }
}

pub(crate) struct CodecCtx(pub(crate) *mut ff::AVCodecContext);
impl Drop for CodecCtx {
    fn drop(&mut self) {
        unsafe { ff::avcodec_free_context(&mut self.0) };
    }
}

pub(crate) struct Frame(pub(crate) *mut ff::AVFrame);
impl Frame {
    pub(crate) fn alloc() -> Result<Self> {
        let f = unsafe { ff::av_frame_alloc() };
        if f.is_null() {
            Err(MediaError::Other("av_frame_alloc".into()))
        } else {
            Ok(Self(f))
        }
    }
}
impl Drop for Frame {
    fn drop(&mut self) {
        unsafe { ff::av_frame_free(&mut self.0) };
    }
}

pub(crate) struct Packet(pub(crate) *mut ff::AVPacket);
impl Packet {
    pub(crate) fn alloc() -> Result<Self> {
        let p = unsafe { ff::av_packet_alloc() };
        if p.is_null() {
            Err(MediaError::Other("av_packet_alloc".into()))
        } else {
            Ok(Self(p))
        }
    }
}
impl Drop for Packet {
    fn drop(&mut self) {
        unsafe { ff::av_packet_free(&mut self.0) };
    }
}
