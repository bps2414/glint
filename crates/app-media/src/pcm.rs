//! Áudio de um clipe inteiro em PCM: cada trilha (jogo, microfone) vira
//! 48 kHz estéreo `i16` intercalado, alinhada ao vídeo (amostra 0 = quadro 0).
//!
//! Um clipe de 60 s ocupa ~11 MB por trilha. Decodificar tudo de uma vez deixa
//! o seek do player e a mixagem da publicação triviais (é só um índice).

use std::path::Path;
use std::ptr;

use ffmpeg_sys_next as ff;

use crate::error::{check, MediaError, Result, EAGAIN};
use crate::ffi::{CodecCtx, Frame, Input, Packet};

/// Taxa de saída (a do gravador).
pub const RATE: u32 = 48_000;

/// Uma trilha decodificada: `samples` = L, R, L, R...
#[derive(Clone, Debug, Default)]
pub struct PcmTrack {
    pub samples: Vec<i16>,
}

impl PcmTrack {
    /// Quantidade de quadros estéreo.
    pub fn frames(&self) -> usize {
        self.samples.len() / 2
    }

    pub fn duration_ms(&self) -> i64 {
        self.frames() as i64 * 1000 / i64::from(RATE)
    }
}

struct TrackDec {
    index: usize,
    tb: ff::AVRational,
    dec: CodecCtx,
    swr: *mut ff::SwrContext,
    /// Quadros (48 kHz) a descartar do começo (áudio que começou antes do vídeo).
    skip: usize,
    started: bool,
    out: Vec<i16>,
}

impl Drop for TrackDec {
    fn drop(&mut self) {
        unsafe { ff::swr_free(&mut self.swr) };
    }
}

/// Decodifica todas as trilhas de áudio de `path`, na ordem em que aparecem no
/// arquivo (no gravador: jogo, depois microfone). Arquivo sem áudio = lista vazia.
pub fn decode_tracks(path: &Path) -> Result<Vec<PcmTrack>> {
    let input = Input::open(path)?;
    // Início do vídeo em segundos: o zero do player.
    let video_start = input
        .video_stream()
        .map(|(_, vs)| unsafe {
            let st = (*vs).start_time;
            if st == ff::AV_NOPTS_VALUE {
                0.0
            } else {
                st as f64 * q((*vs).time_base)
            }
        })
        .unwrap_or(0.0);

    let mut tracks: Vec<TrackDec> = Vec::new();
    for (i, &s) in input.streams().iter().enumerate() {
        unsafe {
            let par = (*s).codecpar;
            if (*par).codec_type != ff::AVMediaType::AVMEDIA_TYPE_AUDIO {
                continue;
            }
            let codec = ff::avcodec_find_decoder((*par).codec_id);
            if codec.is_null() {
                tracing::warn!("trilha de áudio {i} sem decoder; ignorada");
                continue;
            }
            let dec = CodecCtx(ff::avcodec_alloc_context3(codec));
            if dec.0.is_null() {
                return Err(MediaError::Other("avcodec_alloc_context3 (áudio)".into()));
            }
            check(
                ff::avcodec_parameters_to_context(dec.0, par),
                "avcodec_parameters_to_context (áudio)",
            )?;
            (*dec.0).pkt_timebase = (*s).time_base;
            check(
                ff::avcodec_open2(dec.0, codec, ptr::null_mut()),
                "avcodec_open2 (áudio)",
            )?;
            tracks.push(TrackDec {
                index: i,
                tb: (*s).time_base,
                dec,
                swr: ptr::null_mut(),
                skip: 0,
                started: false,
                out: Vec::new(),
            });
        }
    }
    if tracks.is_empty() {
        return Ok(Vec::new());
    }

    let pkt = Packet::alloc()?;
    let frame = Frame::alloc()?;
    unsafe {
        while ff::av_read_frame(input.0, pkt.0) >= 0 {
            let si = (*pkt.0).stream_index as usize;
            if let Some(t) = tracks.iter_mut().find(|t| t.index == si) {
                let r = ff::avcodec_send_packet(t.dec.0, pkt.0);
                if r < 0 && r != EAGAIN && r != ff::AVERROR_INVALIDDATA {
                    check(r, "avcodec_send_packet (áudio)")?;
                }
                t.receive(&frame, video_start)?;
            }
            ff::av_packet_unref(pkt.0);
        }
        for t in &mut tracks {
            ff::avcodec_send_packet(t.dec.0, ptr::null());
            t.receive(&frame, video_start)?;
            t.flush()?;
        }
    }
    Ok(tracks
        .into_iter()
        .map(|mut t| PcmTrack {
            samples: std::mem::take(&mut t.out),
        })
        .collect())
}

fn q(r: ff::AVRational) -> f64 {
    if r.den == 0 {
        0.0
    } else {
        f64::from(r.num) / f64::from(r.den)
    }
}

impl TrackDec {
    unsafe fn receive(&mut self, frame: &Frame, video_start: f64) -> Result<()> {
        loop {
            let r = ff::avcodec_receive_frame(self.dec.0, frame.0);
            if r == EAGAIN || r == ff::AVERROR_EOF {
                return Ok(());
            }
            if r == ff::AVERROR_INVALIDDATA {
                continue;
            }
            check(r, "avcodec_receive_frame (áudio)")?;
            let f = &*frame.0;
            if self.swr.is_null() {
                self.open_swr(f)?;
            }
            if !self.started {
                self.started = true;
                let mut ts = f.best_effort_timestamp;
                if ts == ff::AV_NOPTS_VALUE {
                    ts = f.pts;
                }
                if ts != ff::AV_NOPTS_VALUE {
                    let offset = ts as f64 * q(self.tb) - video_start;
                    let n = (offset.abs() * f64::from(RATE)).round() as usize;
                    if offset > 0.0 {
                        // Áudio começa depois do vídeo: silêncio na frente.
                        self.out.resize(n * 2, 0);
                    } else {
                        self.skip = n;
                    }
                }
            }
            self.convert(f.extended_data as *const *const u8, f.nb_samples)?;
            ff::av_frame_unref(frame.0);
        }
    }

    unsafe fn open_swr(&mut self, f: &ff::AVFrame) -> Result<()> {
        let mut out_layout: ff::AVChannelLayout = std::mem::zeroed();
        ff::av_channel_layout_default(&mut out_layout, 2);
        // SAFETY: `format` de um quadro de áudio decodificado é um AVSampleFormat válido.
        let in_fmt: ff::AVSampleFormat = std::mem::transmute::<i32, ff::AVSampleFormat>(f.format);
        let r = ff::swr_alloc_set_opts2(
            &mut self.swr,
            &out_layout,
            ff::AVSampleFormat::AV_SAMPLE_FMT_S16,
            RATE as i32,
            &f.ch_layout,
            in_fmt,
            f.sample_rate,
            0,
            ptr::null_mut(),
        );
        ff::av_channel_layout_uninit(&mut out_layout);
        check(r, "swr_alloc_set_opts2 (decode)")?;
        check(ff::swr_init(self.swr), "swr_init (decode)")?;
        Ok(())
    }

    /// Converte `n` amostras de entrada (ou esvazia o swr com `input` nulo).
    unsafe fn convert(&mut self, input: *const *const u8, n: i32) -> Result<()> {
        if self.swr.is_null() {
            return Ok(());
        }
        let cap = ff::swr_get_out_samples(self.swr, n.max(0)).max(0) as usize + 64;
        let mut buf = vec![0i16; cap * 2];
        let mut out = [buf.as_mut_ptr() as *mut u8];
        let got = check(
            ff::swr_convert(self.swr, out.as_mut_ptr() as _, cap as i32, input as _, n),
            "swr_convert (decode)",
        )? as usize;
        let mut frames = &buf[..got * 2];
        if self.skip > 0 {
            let s = self.skip.min(got);
            self.skip -= s;
            frames = &frames[s * 2..];
        }
        self.out.extend_from_slice(frames);
        Ok(())
    }

    unsafe fn flush(&mut self) -> Result<()> {
        // O swr guarda algumas amostras do filtro de reamostragem.
        for _ in 0..4 {
            let before = self.out.len();
            self.convert(ptr::null(), 0)?;
            if self.out.len() == before {
                break;
            }
        }
        Ok(())
    }
}
