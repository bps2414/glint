use std::ptr;
use std::sync::Arc;

use app_core::{EncodedPacket, StreamId};
use ffmpeg_sys_next as ff;

use crate::error::{check, MediaError, Result, EAGAIN};
use crate::stream::StreamInfo;

pub const OUT_RATE: i32 = 48_000;
const OUT_CHANNELS: i32 = 2;
/// Buracos maiores que isso nos timestamps viram silêncio (o loopback do
/// WASAPI não entrega pacotes quando nada está tocando).
const GAP_FILL_US: i64 = 30_000;

/// Encoder AAC (encoder nativo do FFmpeg, LGPL) com reamostragem de
/// qualquer formato float intercalado para 48 kHz estéreo.
pub struct AudioEncoder {
    ctx: *mut ff::AVCodecContext,
    swr: *mut ff::SwrContext,
    fifo: *mut ff::AVAudioFifo,
    frame: *mut ff::AVFrame,
    pkt: *mut ff::AVPacket,
    stream: StreamId,
    in_rate: u32,
    in_channels: u32,
    /// Instante (µs) da primeira amostra recebida.
    start_us: Option<i64>,
    /// Amostras de entrada consumidas (inclui silêncio inserido).
    in_samples: i64,
    /// Amostras de saída já enviadas ao encoder.
    out_samples: i64,
    info: StreamInfo,
}

unsafe impl Send for AudioEncoder {}

impl AudioEncoder {
    pub fn new(
        stream: StreamId,
        in_rate: u32,
        in_channels: u32,
        bitrate_kbps: u32,
    ) -> Result<Self> {
        unsafe {
            let codec = ff::avcodec_find_encoder(ff::AVCodecID::AV_CODEC_ID_AAC);
            if codec.is_null() {
                return Err(MediaError::NoEncoder("aac".into()));
            }
            let mut ctx = ff::avcodec_alloc_context3(codec);
            let c = &mut *ctx;
            c.sample_fmt = ff::AVSampleFormat::AV_SAMPLE_FMT_FLTP;
            c.sample_rate = OUT_RATE;
            ff::av_channel_layout_default(&mut c.ch_layout, OUT_CHANNELS);
            c.bit_rate = i64::from(bitrate_kbps) * 1000;
            c.time_base = ff::AVRational {
                num: 1,
                den: OUT_RATE,
            };
            c.flags |= ff::AV_CODEC_FLAG_GLOBAL_HEADER as i32;
            if let Err(e) = check(
                ff::avcodec_open2(ctx, codec, ptr::null_mut()),
                "avcodec_open2(aac)",
            ) {
                ff::avcodec_free_context(&mut ctx);
                return Err(e);
            }
            let info = StreamInfo::from_context(ctx)?;

            let mut in_layout: ff::AVChannelLayout = std::mem::zeroed();
            ff::av_channel_layout_default(&mut in_layout, in_channels as i32);
            let mut swr: *mut ff::SwrContext = ptr::null_mut();
            let ret = ff::swr_alloc_set_opts2(
                &mut swr,
                &(*ctx).ch_layout,
                ff::AVSampleFormat::AV_SAMPLE_FMT_FLTP,
                OUT_RATE,
                &in_layout,
                ff::AVSampleFormat::AV_SAMPLE_FMT_FLT,
                in_rate as i32,
                0,
                ptr::null_mut(),
            );
            ff::av_channel_layout_uninit(&mut in_layout);
            check(ret, "swr_alloc_set_opts2")?;
            check(ff::swr_init(swr), "swr_init")?;

            let fifo = ff::av_audio_fifo_alloc(
                ff::AVSampleFormat::AV_SAMPLE_FMT_FLTP,
                OUT_CHANNELS,
                OUT_RATE,
            );
            let frame = ff::av_frame_alloc();
            let pkt = ff::av_packet_alloc();
            tracing::info!(stream, in_rate, in_channels, "encoder AAC aberto");
            Ok(Self {
                ctx,
                swr,
                fifo,
                frame,
                pkt,
                stream,
                in_rate,
                in_channels,
                start_us: None,
                in_samples: 0,
                out_samples: 0,
                info,
            })
        }
    }

    pub fn stream_info(&self) -> &StreamInfo {
        &self.info
    }

    pub fn input_format(&self) -> (u32, u32) {
        (self.in_rate, self.in_channels)
    }

    /// Recebe amostras float intercaladas cujo primeiro sample ocorre em
    /// `time_us` (relógio comum). Preenche buracos com silêncio.
    pub fn push(&mut self, samples: &[f32], time_us: i64) -> Result<Vec<EncodedPacket>> {
        let ch = self.in_channels as usize;
        let start = *self.start_us.get_or_insert(time_us);
        let expected = start + self.in_samples * 1_000_000 / i64::from(self.in_rate);
        let mut out = Vec::new();
        if time_us - expected > GAP_FILL_US {
            let gap = ((time_us - expected) * i64::from(self.in_rate) / 1_000_000) as usize;
            // Limite de 2 s de silêncio por vez para não explodir memória.
            let silence = vec![0f32; gap.min(self.in_rate as usize * 2) * ch];
            self.convert(&silence, &mut out)?;
        }
        self.convert(samples, &mut out)?;
        Ok(out)
    }

    fn convert(&mut self, samples: &[f32], out: &mut Vec<EncodedPacket>) -> Result<()> {
        let ch = self.in_channels as usize;
        let n_in = (samples.len() / ch) as i32;
        if n_in == 0 {
            return Ok(());
        }
        self.in_samples += i64::from(n_in);
        unsafe {
            let max_out = ff::swr_get_out_samples(self.swr, n_in).max(0) as usize + 32;
            let mut left = vec![0f32; max_out];
            let mut right = vec![0f32; max_out];
            let mut out_ptrs = [left.as_mut_ptr() as *mut u8, right.as_mut_ptr() as *mut u8];
            let in_ptrs = [samples.as_ptr() as *const u8];
            let got = check(
                ff::swr_convert(
                    self.swr,
                    out_ptrs.as_mut_ptr() as _,
                    max_out as i32,
                    in_ptrs.as_ptr() as _,
                    n_in,
                ),
                "swr_convert",
            )?;
            if got > 0 {
                let mut planes = [
                    left.as_mut_ptr() as *mut std::ffi::c_void,
                    right.as_mut_ptr() as *mut std::ffi::c_void,
                ];
                ff::av_audio_fifo_write(self.fifo, planes.as_mut_ptr() as _, got);
            }
        }
        self.encode_available(out)
    }

    fn encode_available(&mut self, out: &mut Vec<EncodedPacket>) -> Result<()> {
        unsafe {
            let frame_size = (*self.ctx).frame_size.max(1024);
            while ff::av_audio_fifo_size(self.fifo) >= frame_size {
                ff::av_frame_unref(self.frame);
                let f = &mut *self.frame;
                f.nb_samples = frame_size;
                f.format = ff::AVSampleFormat::AV_SAMPLE_FMT_FLTP as i32;
                f.sample_rate = OUT_RATE;
                check(
                    ff::av_channel_layout_copy(&mut f.ch_layout, &(*self.ctx).ch_layout),
                    "av_channel_layout_copy",
                )?;
                check(
                    ff::av_frame_get_buffer(self.frame, 0),
                    "av_frame_get_buffer",
                )?;
                ff::av_audio_fifo_read(self.fifo, (*self.frame).data.as_mut_ptr() as _, frame_size);
                (*self.frame).pts = self.out_samples;
                self.out_samples += i64::from(frame_size);
                check(
                    ff::avcodec_send_frame(self.ctx, self.frame),
                    "avcodec_send_frame(aac)",
                )?;
                self.drain(out)?;
            }
        }
        Ok(())
    }

    unsafe fn drain(&mut self, out: &mut Vec<EncodedPacket>) -> Result<()> {
        let start = self.start_us.unwrap_or(0);
        loop {
            let ret = ff::avcodec_receive_packet(self.ctx, self.pkt);
            if ret == EAGAIN || ret == ff::AVERROR_EOF {
                return Ok(());
            }
            check(ret, "avcodec_receive_packet(aac)")?;
            let p = &*self.pkt;
            out.push(EncodedPacket {
                stream: self.stream,
                pts: p.pts,
                dts: p.dts,
                duration: p.duration,
                time_us: start + p.pts * 1_000_000 / i64::from(OUT_RATE),
                keyframe: true,
                data: Arc::from(std::slice::from_raw_parts(p.data, p.size as usize)),
            });
            ff::av_packet_unref(self.pkt);
        }
    }
}

impl Drop for AudioEncoder {
    fn drop(&mut self) {
        unsafe {
            ff::av_packet_free(&mut self.pkt);
            ff::av_frame_free(&mut self.frame);
            ff::av_audio_fifo_free(self.fifo);
            ff::swr_free(&mut self.swr);
            ff::avcodec_free_context(&mut self.ctx);
        }
    }
}
