//! Decodificação de vídeo para o player da UI.
//!
//! [`VideoDecoder`] lê um clipe, decodifica por hardware quando dá (o
//! dispositivo é escolhido **pelo nome** do FFmpeg — `d3d11va` no Windows,
//! `vaapi` no Linux —, então não há código de SO aqui) e converte o quadro
//! atual para BGRA já no tamanho em que vai aparecer na tela
//! ([`VideoDecoder::convert`]). Quadros pulados (seek exato, atraso) nunca são
//! convertidos nem copiados da GPU: só o que vai para a tela paga a conversão.
//!
//! O laço com thread própria, fila e seek coalescido fica em [`crate::player`].

use std::ffi::{c_void, CStr, CString};
use std::path::Path;
use std::ptr;

use ffmpeg_sys_next as ff;

use crate::error::{check, MediaError, Result, EAGAIN};
use crate::ffi::{CodecCtx, Frame, Input, Packet};

/// Usar ou não decodificação por hardware.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HwPreference {
    /// Tenta os dispositivos de [`HW_DEVICES`]; cai para software se nenhum servir.
    Auto,
    /// Só software (testes, diagnóstico).
    Off,
}

/// Dispositivos de decodificação tentados, em ordem. Os que não existem nesta
/// build do FFmpeg ou neste SO simplesmente falham ao abrir e são pulados.
pub const HW_DEVICES: &[&str] = &["d3d11va", "dxva2", "vaapi", "videotoolbox"];

/// Metadados do vídeo aberto.
#[derive(Clone, Debug, PartialEq)]
pub struct VideoInfo {
    pub width: u32,
    pub height: u32,
    /// Quadros por segundo (média do arquivo; 60 se desconhecido).
    pub fps: f64,
    pub duration_ms: i64,
    /// Nome curto do codec (`hevc`, `h264`...).
    pub codec: String,
    /// Dispositivo de hardware em uso (`d3d11va`...) ou `None` = software.
    pub hw: Option<&'static str>,
}

impl VideoInfo {
    /// Duração de um quadro em ms.
    pub fn frame_ms(&self) -> f64 {
        1000.0 / self.fps.max(1.0)
    }
}

/// Quadro pronto para a tela: BGRA, sem padding entre linhas
/// (`data.len() == width * height * 4`).
#[derive(Clone, Debug)]
pub struct BgraFrame {
    pub pts_ms: i64,
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

// Constantes do libswscale (viraram enums no FFmpeg 8; valores estáveis).
const SWS_BILINEAR: i32 = 2;
const SWS_CS_ITU709: i32 = 1;
const SWS_CS_ITU601: i32 = 5;
const SWS_CS_BT2020: i32 = 9;

#[derive(Clone, Copy, PartialEq, Eq)]
struct SwsKey {
    fmt: i32,
    sw: i32,
    sh: i32,
    dw: i32,
    dh: i32,
    cs: i32,
    full_range: bool,
}

pub struct VideoDecoder {
    input: Input,
    dec: CodecCtx,
    index: usize,
    tb: ff::AVRational,
    start_ts: i64,
    /// Último quadro decodificado (fica guardado até o próximo; é o que
    /// [`Self::convert`] converte).
    cur: Frame,
    cur_pts_ms: Option<i64>,
    tmp: Frame,
    /// Destino da cópia GPU → CPU (quadros de hardware).
    sw: Frame,
    pkt: Packet,
    /// Já mandou o "fim" ao decoder (esvaziando os quadros atrasados).
    draining: bool,
    sws: *mut ff::SwsContext,
    sws_key: Option<SwsKey>,
    info: VideoInfo,
}

// SAFETY: os ponteiros do FFmpeg pertencem só a este valor; o decoder é usado
// por uma thread de cada vez (é movido para a thread do player).
unsafe impl Send for VideoDecoder {}

impl Drop for VideoDecoder {
    fn drop(&mut self) {
        if !self.sws.is_null() {
            unsafe { ff::sws_freeContext(self.sws) };
        }
    }
}

/// Escolhe o formato de hardware pedido (guardado em `opaque`); se o stream não
/// puder usar hardware, deixa o FFmpeg escolher um formato de software.
unsafe extern "C" fn get_hw_format(
    ctx: *mut ff::AVCodecContext,
    fmts: *const ff::AVPixelFormat,
) -> ff::AVPixelFormat {
    let want = (*ctx).opaque as isize as i32;
    let mut p = fmts;
    while *p != ff::AVPixelFormat::AV_PIX_FMT_NONE {
        if *p as i32 == want {
            return *p;
        }
        p = p.add(1);
    }
    ff::avcodec_default_get_format(ctx, fmts)
}

impl VideoDecoder {
    /// Abre `path`. Com [`HwPreference::Auto`], tenta cada dispositivo de
    /// [`HW_DEVICES`] e só aceita um que decodifique de fato o primeiro quadro
    /// na GPU; senão usa software.
    pub fn open(path: &Path, hw: HwPreference) -> Result<Self> {
        if hw == HwPreference::Auto {
            for &name in HW_DEVICES {
                match Self::open_with(path, Some(name)).and_then(|mut d| {
                    d.verify_hw()?;
                    Ok(d)
                }) {
                    Ok(d) => {
                        tracing::info!("decodificação por hardware: {name}");
                        return Ok(d);
                    }
                    Err(e) => tracing::debug!("decoder {name} indisponível: {e}"),
                }
            }
            tracing::info!("decodificação por software");
        }
        Self::open_with(path, None)
    }

    pub fn info(&self) -> &VideoInfo {
        &self.info
    }

    /// Timestamp (ms desde o início) do último quadro decodificado.
    pub fn current_pts_ms(&self) -> Option<i64> {
        self.cur_pts_ms
    }

    fn open_with(path: &Path, hw: Option<&'static str>) -> Result<Self> {
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
            let dec = CodecCtx(ff::avcodec_alloc_context3(codec));
            if dec.0.is_null() {
                return Err(MediaError::Other("avcodec_alloc_context3".into()));
            }
            check(
                ff::avcodec_parameters_to_context(dec.0, par),
                "avcodec_parameters_to_context",
            )?;
            (*dec.0).pkt_timebase = (*vs).time_base;

            if let Some(name) = hw {
                let cname = CString::new(name).expect("nome sem NUL");
                let ty = ff::av_hwdevice_find_type_by_name(cname.as_ptr());
                if ty == ff::AVHWDeviceType::AV_HWDEVICE_TYPE_NONE {
                    return Err(MediaError::Other(format!("{name}: não existe nesta build")));
                }
                let mut hw_fmt = None;
                for i in 0.. {
                    let cfg = ff::avcodec_get_hw_config(codec, i);
                    if cfg.is_null() {
                        break;
                    }
                    let cfg = &*cfg;
                    if cfg.methods & ff::AV_CODEC_HW_CONFIG_METHOD_HW_DEVICE_CTX as i32 != 0
                        && cfg.device_type == ty
                    {
                        hw_fmt = Some(cfg.pix_fmt);
                        break;
                    }
                }
                let hw_fmt = hw_fmt.ok_or_else(|| {
                    MediaError::Other(format!("{name}: codec sem suporte de hardware"))
                })?;
                let mut device: *mut ff::AVBufferRef = ptr::null_mut();
                check(
                    ff::av_hwdevice_ctx_create(&mut device, ty, ptr::null(), ptr::null_mut(), 0),
                    "av_hwdevice_ctx_create",
                )?;
                // O contexto do codec passa a ser dono da referência.
                (*dec.0).hw_device_ctx = device;
                (*dec.0).opaque = hw_fmt as i32 as isize as *mut c_void;
                (*dec.0).get_format = Some(get_hw_format);
                // Folga no pool de superfícies: o player segura o quadro atual.
                (*dec.0).extra_hw_frames = 4;
                (*dec.0).thread_count = 1;
            } else {
                // Software: threads automáticas (por fatia, sem atrasar o seek).
                (*dec.0).thread_count = 0;
                (*dec.0).thread_type = ff::FF_THREAD_SLICE as i32;
            }
            check(
                ff::avcodec_open2(dec.0, codec, ptr::null_mut()),
                "avcodec_open2 (decoder)",
            )?;

            let fr = (*vs).avg_frame_rate;
            let fr = if fr.num > 0 && fr.den > 0 {
                fr
            } else {
                (*vs).r_frame_rate
            };
            let fps = if fr.num > 0 && fr.den > 0 {
                f64::from(fr.num) / f64::from(fr.den)
            } else {
                60.0
            };
            let tb = (*vs).time_base;
            let start_ts = if (*vs).start_time == ff::AV_NOPTS_VALUE {
                0
            } else {
                (*vs).start_time
            };
            let duration_ms = if (*input.0).duration > 0 {
                (*input.0).duration / (ff::AV_TIME_BASE as i64 / 1000)
            } else if (*vs).duration > 0 {
                ff::av_rescale_q((*vs).duration, tb, MS)
            } else {
                0
            };
            let codec_name = CStr::from_ptr(ff::avcodec_get_name((*par).codec_id))
                .to_string_lossy()
                .into_owned();
            let info = VideoInfo {
                width: (*par).width.max(0) as u32,
                height: (*par).height.max(0) as u32,
                fps,
                duration_ms,
                codec: codec_name,
                hw,
            };
            Ok(Self {
                input,
                dec,
                index,
                tb,
                start_ts,
                cur: Frame::alloc()?,
                cur_pts_ms: None,
                tmp: Frame::alloc()?,
                sw: Frame::alloc()?,
                pkt: Packet::alloc()?,
                draining: false,
                sws: ptr::null_mut(),
                sws_key: None,
                info,
            })
        }
    }

    /// Confere que o primeiro quadro sai mesmo da GPU e volta ao início.
    fn verify_hw(&mut self) -> Result<()> {
        if self.decode_next()?.is_none() {
            return Err(MediaError::Other("nenhum quadro decodificado".into()));
        }
        if unsafe { (*self.cur.0).hw_frames_ctx.is_null() } {
            return Err(MediaError::Other("decoder caiu para software".into()));
        }
        // Prova também a cópia para a CPU (é o que o player faz em todo quadro).
        unsafe {
            ff::av_frame_unref(self.sw.0);
            check(
                ff::av_hwframe_transfer_data(self.sw.0, self.cur.0, 0),
                "av_hwframe_transfer_data",
            )?;
            ff::av_frame_unref(self.sw.0);
        }
        self.seek(0)
    }

    /// Vai para o keyframe em ou antes de `ms`. O próximo [`Self::next`] devolve
    /// esse keyframe; para um seek exato, o chamador pula quadros até o alvo.
    pub fn seek(&mut self, ms: i64) -> Result<()> {
        unsafe {
            let ts = ff::av_rescale_q(ms.max(0), MS, self.tb) + self.start_ts;
            check(
                ff::av_seek_frame(
                    self.input.0,
                    self.index as i32,
                    ts,
                    ff::AVSEEK_FLAG_BACKWARD as i32,
                ),
                "av_seek_frame",
            )?;
            ff::avcodec_flush_buffers(self.dec.0);
        }
        self.draining = false;
        self.cur_pts_ms = None;
        Ok(())
    }

    /// Decodifica o próximo quadro (sem converter). Devolve o timestamp em ms
    /// ou `None` no fim do arquivo.
    pub fn decode_next(&mut self) -> Result<Option<i64>> {
        unsafe {
            loop {
                let r = ff::avcodec_receive_frame(self.dec.0, self.tmp.0);
                if r >= 0 {
                    ff::av_frame_unref(self.cur.0);
                    ff::av_frame_move_ref(self.cur.0, self.tmp.0);
                    let pts = self.frame_pts_ms(self.cur.0);
                    self.cur_pts_ms = Some(pts);
                    return Ok(Some(pts));
                }
                if r == ff::AVERROR_EOF {
                    return Ok(None);
                }
                if r != EAGAIN {
                    check(r, "avcodec_receive_frame")?;
                }
                if self.draining {
                    return Ok(None);
                }
                let r = ff::av_read_frame(self.input.0, self.pkt.0);
                if r < 0 {
                    // Fim do arquivo (ou erro de leitura): esvazia o decoder.
                    ff::avcodec_send_packet(self.dec.0, ptr::null());
                    self.draining = true;
                    continue;
                }
                if (*self.pkt.0).stream_index == self.index as i32 {
                    let s = ff::avcodec_send_packet(self.dec.0, self.pkt.0);
                    ff::av_packet_unref(self.pkt.0);
                    if s == ff::AVERROR_INVALIDDATA {
                        tracing::debug!("pacote de vídeo corrompido ignorado");
                    } else if s < 0 && s != EAGAIN {
                        check(s, "avcodec_send_packet")?;
                    }
                } else {
                    ff::av_packet_unref(self.pkt.0);
                }
            }
        }
    }

    unsafe fn frame_pts_ms(&self, f: *const ff::AVFrame) -> i64 {
        let mut ts = (*f).best_effort_timestamp;
        if ts == ff::AV_NOPTS_VALUE {
            ts = (*f).pts;
        }
        if ts == ff::AV_NOPTS_VALUE {
            return self.cur_pts_ms.unwrap_or(0);
        }
        ff::av_rescale_q(ts - self.start_ts, self.tb, MS)
    }

    /// Último quadro decodificado na memória da CPU (copiado da GPU se for de
    /// hardware). Vale até o próximo `decode_next`.
    pub(crate) fn sw_frame(&mut self) -> Result<*const ff::AVFrame> {
        if self.cur_pts_ms.is_none() {
            return Err(MediaError::Other("nenhum quadro decodificado".into()));
        }
        unsafe {
            if (*self.cur.0).hw_frames_ctx.is_null() {
                return Ok(self.cur.0);
            }
            ff::av_frame_unref(self.sw.0);
            check(
                ff::av_hwframe_transfer_data(self.sw.0, self.cur.0, 0),
                "av_hwframe_transfer_data",
            )?;
            (*self.sw.0).color_range = (*self.cur.0).color_range;
            (*self.sw.0).colorspace = (*self.cur.0).colorspace;
            Ok(self.sw.0)
        }
    }

    /// Converte o último quadro decodificado para BGRA cabendo em
    /// `max_w`×`max_h` (proporção mantida, nunca aumenta).
    pub fn convert(&mut self, max_w: u32, max_h: u32) -> Result<BgraFrame> {
        let pts_ms = self
            .cur_pts_ms
            .ok_or_else(|| MediaError::Other("nenhum quadro decodificado".into()))?;
        unsafe {
            let mut src: *const ff::AVFrame = self.cur.0;
            if !(*src).hw_frames_ctx.is_null() {
                ff::av_frame_unref(self.sw.0);
                check(
                    ff::av_hwframe_transfer_data(self.sw.0, self.cur.0, 0),
                    "av_hwframe_transfer_data",
                )?;
                src = self.sw.0;
            }
            let (sw, sh) = ((*src).width, (*src).height);
            if sw <= 0 || sh <= 0 {
                return Err(MediaError::Other("quadro vazio".into()));
            }
            let (dw, dh) = fit_size(sw as u32, sh as u32, max_w, max_h);
            let key = SwsKey {
                fmt: (*src).format,
                sw,
                sh,
                dw: dw as i32,
                dh: dh as i32,
                cs: colorspace(&*src),
                full_range: (*src).color_range == ff::AVColorRange::AVCOL_RANGE_JPEG,
            };
            self.ensure_sws(key)?;

            let stride = dw as usize * 4;
            let len = stride * dh as usize;
            let mut data: Vec<u8> = Vec::with_capacity(len);
            let dst = [
                data.as_mut_ptr(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
            ];
            let dst_stride = [stride as i32, 0, 0, 0];
            let rows = ff::sws_scale(
                self.sws,
                (*src).data.as_ptr() as *const *const u8,
                (*src).linesize.as_ptr(),
                0,
                sh,
                dst.as_ptr(),
                dst_stride.as_ptr(),
            );
            if rows != dh as i32 {
                return Err(MediaError::Other(format!(
                    "sws_scale: {rows} de {dh} linhas"
                )));
            }
            // SAFETY: o sws_scale escreveu todas as `dh` linhas de `stride` bytes.
            data.set_len(len);
            Ok(BgraFrame {
                pts_ms,
                width: dw,
                height: dh,
                data,
            })
        }
    }

    unsafe fn ensure_sws(&mut self, key: SwsKey) -> Result<()> {
        if self.sws_key == Some(key) && !self.sws.is_null() {
            return Ok(());
        }
        if !self.sws.is_null() {
            ff::sws_freeContext(self.sws);
            self.sws = ptr::null_mut();
        }
        // SAFETY: `format` de um quadro de vídeo decodificado é um AVPixelFormat válido.
        let src_fmt: ff::AVPixelFormat = std::mem::transmute::<i32, ff::AVPixelFormat>(key.fmt);
        let sws = ff::sws_getContext(
            key.sw,
            key.sh,
            src_fmt,
            key.dw,
            key.dh,
            ff::AVPixelFormat::AV_PIX_FMT_BGRA,
            SWS_BILINEAR,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null(),
        );
        if sws.is_null() {
            return Err(MediaError::Other("sws_getContext".into()));
        }
        // Matriz certa (o padrão do swscale é BT.601, que deixaria o 709 com cor errada).
        let coeffs = ff::sws_getCoefficients(key.cs);
        ff::sws_setColorspaceDetails(
            sws,
            coeffs,
            i32::from(key.full_range),
            coeffs,
            1,
            0,
            1 << 16,
            1 << 16,
        );
        self.sws = sws;
        self.sws_key = Some(key);
        Ok(())
    }
}

/// Miniaturas para a linha do tempo: `count` quadros espalhados pelo clipe,
/// cabendo em `max_w`×`max_h`. Usa o keyframe em/antes de cada ponto (sem seek
/// exato: é só uma tira de referência), decodificando por software.
pub fn strip(path: &Path, count: usize, max_w: u32, max_h: u32) -> Result<Vec<BgraFrame>> {
    let mut dec = VideoDecoder::open(path, HwPreference::Off)?;
    let d = dec.info().duration_ms.max(1);
    let n = count.max(1) as i64;
    let mut out = Vec::with_capacity(count);
    for i in 0..n {
        dec.seek(d * (2 * i + 1) / (2 * n))?;
        if dec.decode_next()?.is_none() {
            break;
        }
        out.push(dec.convert(max_w, max_h)?);
    }
    Ok(out)
}

const MS: ff::AVRational = ff::AVRational { num: 1, den: 1000 };

unsafe fn colorspace(f: &ff::AVFrame) -> i32 {
    use ff::AVColorSpace::*;
    match f.colorspace {
        AVCOL_SPC_BT709 => SWS_CS_ITU709,
        AVCOL_SPC_BT2020_NCL | AVCOL_SPC_BT2020_CL => SWS_CS_BT2020,
        AVCOL_SPC_BT470BG | AVCOL_SPC_SMPTE170M => SWS_CS_ITU601,
        // Sem marcação: HD é 709 na prática (o encoder do gravador marca 709).
        _ if f.height >= 720 => SWS_CS_ITU709,
        _ => SWS_CS_ITU601,
    }
}

/// Tamanho de saída: cabe em `max_w`×`max_h`, mantém a proporção, nunca aumenta.
/// Largura múltipla de 16 (linhas alinhadas para o SIMD do swscale) e altura par.
pub fn fit_size(src_w: u32, src_h: u32, max_w: u32, max_h: u32) -> (u32, u32) {
    let (src_w, src_h) = (src_w.max(1), src_h.max(1));
    let max_w = max_w.clamp(16, src_w.max(16));
    let max_h = max_h.clamp(2, src_h.max(2));
    let scale = (f64::from(max_w) / f64::from(src_w)).min(f64::from(max_h) / f64::from(src_h));
    let scale = scale.min(1.0);
    let w = ((f64::from(src_w) * scale) as u32 / 16 * 16).max(16);
    let h = ((f64::from(src_h) * scale).round() as u32 / 2 * 2).max(2);
    (w, h)
}

#[cfg(test)]
mod tests {
    use super::fit_size;

    #[test]
    fn fit_size_mantem_proporcao_e_alinha() {
        assert_eq!(fit_size(1920, 1080, 1920, 1080), (1920, 1080));
        // Nunca aumenta.
        assert_eq!(fit_size(1280, 720, 3840, 2160), (1280, 720));
        // Player menor que o vídeo.
        let (w, h) = fit_size(1920, 1080, 1000, 700);
        assert_eq!(w % 16, 0);
        assert!(w <= 1000 && h <= 700, "{w}x{h}");
        assert!(
            (f64::from(w) / f64::from(h) - 16.0 / 9.0).abs() < 0.02,
            "{w}x{h}"
        );
        // Limitado pela altura.
        let (w, h) = fit_size(1920, 1080, 1920, 540);
        assert_eq!(h, 540);
        assert_eq!(w, 960);
        // Tamanhos degenerados não quebram.
        assert_eq!(fit_size(0, 0, 0, 0), (16, 2));
    }
}
