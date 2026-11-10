use std::ffi::{CStr, CString};
use std::ptr;
use std::sync::Arc;

use app_core::{Codec, EncodedPacket};
use ffmpeg_sys_next as ff;

use crate::error::{check, MediaError, Result, EAGAIN};
use crate::stream::StreamInfo;

/// Encoders de hardware em ordem de preferência por codec.
/// AMD primeiro porque é o hardware de teste principal (RX 6000).
pub(crate) const HW_ENCODERS: &[&str] = &[
    "hevc_amf",
    "h264_amf",
    "hevc_nvenc",
    "h264_nvenc",
    "hevc_qsv",
    "h264_qsv",
];

pub fn hw_candidates(codec: Codec) -> &'static [&'static str] {
    match codec {
        Codec::Hevc => &["hevc_amf", "hevc_nvenc", "hevc_qsv"],
        Codec::H264 => &["h264_amf", "h264_nvenc", "h264_qsv"],
    }
}

/// Encoders de hardware que abrem de verdade nesta GPU (tenta cada candidato de
/// [`hw_candidates`] em 1080p, com quadros NV12 na memória). Lento (centenas de ms por
/// encoder): chamar fora da thread da UI.
pub fn available_encoders() -> Vec<&'static str> {
    let mut ok = Vec::new();
    for codec in [Codec::Hevc, Codec::H264] {
        let settings = VideoSettings {
            codec,
            width: 1920,
            height: 1080,
            fps: 60,
            bitrate_kbps: 25_000,
            gop: 60,
        };
        for &name in hw_candidates(codec) {
            if find_encoder(name).is_none() {
                continue;
            }
            // SAFETY: sem contexto de hardware; o encoder é solto logo em seguida.
            let r = unsafe {
                VideoEncoder::open(
                    name,
                    &settings,
                    ff::AVPixelFormat::AV_PIX_FMT_NV12,
                    ptr::null_mut(),
                )
            };
            match r {
                Ok(_) => ok.push(name),
                Err(e) => tracing::info!("encoder {name} não abriu nesta GPU: {e}"),
            }
        }
    }
    ok
}

pub(crate) fn find_encoder(name: &str) -> Option<*const ff::AVCodec> {
    let cname = CString::new(name).ok()?;
    let codec = unsafe { ff::avcodec_find_encoder_by_name(cname.as_ptr()) };
    (!codec.is_null()).then_some(codec)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VideoSettings {
    pub codec: Codec,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub bitrate_kbps: u32,
    /// Frames entre keyframes.
    pub gop: u32,
}

/// Opções privadas por família de encoder. Chaves desconhecidas são
/// ignoradas pelo FFmpeg (e logadas por nós), então variações entre versões
/// do driver/FFmpeg não quebram a abertura.
fn private_options(name: &str) -> &'static [(&'static str, &'static str)] {
    if name.ends_with("_amf") {
        &[
            ("usage", "transcoding"),
            ("quality", "balanced"),
            ("rc", "vbr_peak"),
        ]
    } else if name.ends_with("_nvenc") {
        &[("preset", "p4"), ("tune", "hq"), ("rc", "vbr")]
    } else if name.ends_with("_qsv") {
        &[("preset", "faster")]
    } else if name == "libx264" {
        &[("preset", "ultrafast"), ("tune", "zerolatency")]
    } else if name == "mpeg4" {
        // Sem keyframe por "troca de cena": o GOP fica fixo (o player depende disso
        // para o seek). Nos testes o brilho muda a cada quadro e o mpeg4 virava só I.
        &[("sc_threshold", "1000000000")]
    } else {
        &[]
    }
}

/// Encoder de vídeo com timestamps CFR (pts = índice do frame, time base
/// 1/fps). Sem B-frames, para pts == dts e corte simples.
pub struct VideoEncoder {
    ctx: *mut ff::AVCodecContext,
    pkt: *mut ff::AVPacket,
    name: String,
    fps: u32,
    /// Instante (µs) correspondente a pts = 0.
    origin_us: Option<i64>,
    info: StreamInfo,
}

// SAFETY: o encoder é usado por uma thread por vez (&mut self).
unsafe impl Send for VideoEncoder {}

impl VideoEncoder {
    /// Abre o encoder `name`. `hw_frames` (opcional) é um `AVBufferRef` de
    /// `AVHWFramesContext` — o encoder recebe uma referência nova.
    pub(crate) unsafe fn open(
        name: &str,
        settings: &VideoSettings,
        pix_fmt: ff::AVPixelFormat,
        hw_frames: *mut ff::AVBufferRef,
    ) -> Result<Self> {
        let codec = find_encoder(name).ok_or_else(|| MediaError::NoEncoder(name.into()))?;
        let mut ctx = ff::avcodec_alloc_context3(codec);
        if ctx.is_null() {
            return Err(MediaError::Other("avcodec_alloc_context3".into()));
        }
        let c = &mut *ctx;
        c.width = settings.width as i32;
        c.height = settings.height as i32;
        c.time_base = ff::AVRational {
            num: 1,
            den: settings.fps as i32,
        };
        c.framerate = ff::AVRational {
            num: settings.fps as i32,
            den: 1,
        };
        c.pix_fmt = pix_fmt;
        c.bit_rate = i64::from(settings.bitrate_kbps) * 1000;
        c.rc_max_rate = c.bit_rate * 3 / 2;
        c.rc_buffer_size = (c.bit_rate * 2) as i32;
        c.gop_size = settings.gop as i32;
        c.max_b_frames = 0;
        c.flags |= ff::AV_CODEC_FLAG_GLOBAL_HEADER as i32;
        c.color_range = ff::AVColorRange::AVCOL_RANGE_MPEG;
        c.colorspace = ff::AVColorSpace::AVCOL_SPC_BT709;
        c.color_primaries = ff::AVColorPrimaries::AVCOL_PRI_BT709;
        c.color_trc = ff::AVColorTransferCharacteristic::AVCOL_TRC_BT709;
        if !hw_frames.is_null() {
            c.hw_frames_ctx = ff::av_buffer_ref(hw_frames);
        }

        let mut opts: *mut ff::AVDictionary = ptr::null_mut();
        for (k, v) in private_options(name) {
            let k = CString::new(*k).expect("sem NUL");
            let v = CString::new(*v).expect("sem NUL");
            ff::av_dict_set(&mut opts, k.as_ptr(), v.as_ptr(), 0);
        }
        let ret = ff::avcodec_open2(ctx, codec, &mut opts);
        log_unused_options(name, opts);
        ff::av_dict_free(&mut opts);
        if let Err(e) = check(ret, "avcodec_open2") {
            ff::avcodec_free_context(&mut ctx);
            return Err(e);
        }

        let info = match StreamInfo::from_context(ctx) {
            Ok(i) => i,
            Err(e) => {
                ff::avcodec_free_context(&mut ctx);
                return Err(e);
            }
        };
        let pkt = ff::av_packet_alloc();
        tracing::info!(
            encoder = name,
            width = settings.width,
            height = settings.height,
            fps = settings.fps,
            kbps = settings.bitrate_kbps,
            "encoder de vídeo aberto"
        );
        Ok(Self {
            ctx,
            pkt,
            name: name.to_string(),
            fps: settings.fps,
            origin_us: None,
            info,
        })
    }

    /// Encoder por software em YUV420P — usado em testes/fallback de dev.
    /// Tenta libx264, depois o encoder nativo mpeg4.
    pub fn open_software(settings: &VideoSettings) -> Result<Self> {
        let mut tried = Vec::new();
        for name in ["libx264", "mpeg4"] {
            if find_encoder(name).is_none() {
                continue;
            }
            tried.push(name);
            match unsafe {
                Self::open(
                    name,
                    settings,
                    ff::AVPixelFormat::AV_PIX_FMT_YUV420P,
                    ptr::null_mut(),
                )
            } {
                Ok(e) => return Ok(e),
                Err(e) => tracing::warn!("falha ao abrir {name}: {e}"),
            }
        }
        Err(MediaError::NoEncoder(tried.join(", ")))
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn stream_info(&self) -> &StreamInfo {
        &self.info
    }

    pub fn fps(&self) -> u32 {
        self.fps
    }

    /// Converte pts (frames) em µs no relógio comum.
    fn pts_to_us(&self, pts: i64) -> i64 {
        self.origin_us.unwrap_or(0) + pts * 1_000_000 / i64::from(self.fps)
    }

    /// Envia um frame já preenchido. `frame.pts` deve estar em frames.
    /// `time_us` é o instante do frame no relógio comum.
    pub(crate) unsafe fn send_frame(
        &mut self,
        frame: *mut ff::AVFrame,
        time_us: i64,
        out: &mut Vec<EncodedPacket>,
    ) -> Result<()> {
        if self.origin_us.is_none() {
            let pts = (*frame).pts;
            self.origin_us = Some(time_us - pts * 1_000_000 / i64::from(self.fps));
        }
        check(
            ff::avcodec_send_frame(self.ctx, frame),
            "avcodec_send_frame",
        )?;
        self.drain(out)
    }

    /// Envia frame YUV420P (software): `fill(y, u, v, stride_y, stride_uv)`.
    pub fn encode_yuv420p(
        &mut self,
        pts: i64,
        time_us: i64,
        fill: impl FnOnce(&mut [u8], &mut [u8], &mut [u8], usize, usize),
    ) -> Result<Vec<EncodedPacket>> {
        unsafe {
            let mut frame = ff::av_frame_alloc();
            let c = &*self.ctx;
            (*frame).format = c.pix_fmt as i32;
            (*frame).width = c.width;
            (*frame).height = c.height;
            let r = check(ff::av_frame_get_buffer(frame, 0), "av_frame_get_buffer");
            if let Err(e) = r {
                ff::av_frame_free(&mut frame);
                return Err(e);
            }
            let f = &*frame;
            let h = c.height as usize;
            let (sy, suv) = (f.linesize[0] as usize, f.linesize[1] as usize);
            let y = std::slice::from_raw_parts_mut(f.data[0], sy * h);
            let u = std::slice::from_raw_parts_mut(f.data[1], suv * h.div_ceil(2));
            let v = std::slice::from_raw_parts_mut(f.data[2], suv * h.div_ceil(2));
            fill(y, u, v, sy, suv);
            (*frame).pts = pts;
            let mut out = Vec::new();
            let r = self.send_frame(frame, time_us, &mut out);
            ff::av_frame_free(&mut frame);
            r.map(|_| out)
        }
    }

    /// Esvazia o encoder (fim da gravação).
    pub fn flush(&mut self) -> Result<Vec<EncodedPacket>> {
        let mut out = Vec::new();
        unsafe {
            let ret = ff::avcodec_send_frame(self.ctx, ptr::null());
            if ret < 0 && ret != ff::AVERROR_EOF {
                check(ret, "flush")?;
            }
            self.drain(&mut out)?;
        }
        Ok(out)
    }

    unsafe fn drain(&mut self, out: &mut Vec<EncodedPacket>) -> Result<()> {
        loop {
            let ret = ff::avcodec_receive_packet(self.ctx, self.pkt);
            if ret == EAGAIN || ret == ff::AVERROR_EOF {
                return Ok(());
            }
            check(ret, "avcodec_receive_packet")?;
            let p = &*self.pkt;
            let data = std::slice::from_raw_parts(p.data, p.size as usize);
            out.push(EncodedPacket {
                stream: 0,
                pts: p.pts,
                dts: p.dts,
                duration: if p.duration > 0 { p.duration } else { 1 },
                time_us: self.pts_to_us(p.pts),
                keyframe: p.flags & ff::AV_PKT_FLAG_KEY as i32 != 0,
                data: Arc::from(data),
            });
            ff::av_packet_unref(self.pkt);
        }
    }
}

impl Drop for VideoEncoder {
    fn drop(&mut self) {
        unsafe {
            ff::av_packet_free(&mut self.pkt);
            ff::avcodec_free_context(&mut self.ctx);
        }
    }
}

unsafe fn log_unused_options(name: &str, opts: *mut ff::AVDictionary) {
    let mut e: *const ff::AVDictionaryEntry = ptr::null();
    loop {
        e = ff::av_dict_get(opts, c"".as_ptr(), e, ff::AV_DICT_IGNORE_SUFFIX as i32);
        if e.is_null() {
            break;
        }
        let k = CStr::from_ptr((*e).key).to_string_lossy();
        tracing::debug!(encoder = name, option = %k, "opção não reconhecida pelo encoder");
    }
}
