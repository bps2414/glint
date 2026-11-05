//! Entrada zero-copy de texturas D3D11 no encoder de hardware.
//!
//! Fluxo por frame (tudo na GPU):
//! `textura NV12 da plataforma → CopySubresourceRegion → frame do pool do
//! FFmpeg (AV_PIX_FMT_D3D11) → AMF/NVENC/QSV`.
//!
//! Este módulo não depende do crate `windows`: recebe e devolve ponteiros
//! COM crus. A cópia da textura é feita pela camada de plataforma via closure.

use std::ffi::c_void;
use std::ptr;

use app_core::EncodedPacket;
use ffmpeg_sys_next as ff;

use crate::error::{check, MediaError, Result};
use crate::video::{hw_candidates, VideoEncoder, VideoSettings};

/// Espelho de `AVD3D11VADeviceContext` (libavutil/hwcontext_d3d11va.h).
/// Layout estável desde FFmpeg 4; declarado aqui para compilar em qualquer
/// plataforma sem os headers do Windows SDK.
#[repr(C)]
struct AvD3D11VaDeviceContext {
    device: *mut c_void,
    device_context: *mut c_void,
    video_device: *mut c_void,
    video_context: *mut c_void,
    lock: Option<unsafe extern "C" fn(*mut c_void)>,
    unlock: Option<unsafe extern "C" fn(*mut c_void)>,
    lock_ctx: *mut c_void,
}

/// Espelho de `AVD3D11VAFramesContext` (libavutil/hwcontext_d3d11va.h).
#[repr(C)]
struct AvD3D11VaFramesContext {
    texture: *mut c_void,
    bind_flags: u32,
    misc_flags: u32,
    texture_infos: *mut c_void,
}

/// `D3D11_BIND_RENDER_TARGET` (o mesmo da textura NV12 do conversor).
const D3D11_BIND_RENDER_TARGET: u32 = 0x20;

/// Contexto de dispositivo + pool de frames D3D11 do FFmpeg.
struct HwContext {
    device_ref: *mut ff::AVBufferRef,
    frames_ref: *mut ff::AVBufferRef,
}

impl Drop for HwContext {
    fn drop(&mut self) {
        unsafe {
            ff::av_buffer_unref(&mut self.frames_ref);
            ff::av_buffer_unref(&mut self.device_ref);
        }
    }
}

impl HwContext {
    unsafe fn new(
        device: *mut c_void,
        add_ref: &dyn Fn(*mut c_void),
        w: u32,
        h: u32,
    ) -> Result<Self> {
        let device_ref = ff::av_hwdevice_ctx_alloc(ff::AVHWDeviceType::AV_HWDEVICE_TYPE_D3D11VA);
        if device_ref.is_null() {
            return Err(MediaError::Other(
                "FFmpeg sem suporte a D3D11VA (av_hwdevice_ctx_alloc)".into(),
            ));
        }
        let mut hw = Self {
            device_ref,
            frames_ref: ptr::null_mut(),
        };
        let dev_ctx = (*device_ref).data as *mut ff::AVHWDeviceContext;
        let d3d = (*dev_ctx).hwctx as *mut AvD3D11VaDeviceContext;
        // O FFmpeg dá Release no device ao destruir o contexto.
        add_ref(device);
        (*d3d).device = device;
        check(ff::av_hwdevice_ctx_init(device_ref), "av_hwdevice_ctx_init")?;

        let frames_ref = ff::av_hwframe_ctx_alloc(device_ref);
        if frames_ref.is_null() {
            return Err(MediaError::Other("av_hwframe_ctx_alloc".into()));
        }
        hw.frames_ref = frames_ref;
        let frames = (*frames_ref).data as *mut ff::AVHWFramesContext;
        (*frames).format = ff::AVPixelFormat::AV_PIX_FMT_D3D11;
        (*frames).sw_format = ff::AVPixelFormat::AV_PIX_FMT_NV12;
        (*frames).width = w as i32;
        (*frames).height = h as i32;
        // Pool dinâmico de texturas avulsas (initial_pool_size = 0). Com pool fixo o
        // FFmpeg 8.1 cria um array de texturas NV12, que o driver AMD (RX 6600) recusa
        // com E_INVALIDARG em `av_hwframe_ctx_init`.
        (*frames).initial_pool_size = 0;
        let d3d_frames = (*frames).hwctx as *mut AvD3D11VaFramesContext;
        (*d3d_frames).bind_flags = D3D11_BIND_RENDER_TARGET;
        check(ff::av_hwframe_ctx_init(frames_ref), "av_hwframe_ctx_init")?;
        Ok(hw)
    }
}

/// Encoder de hardware que consome texturas NV12 D3D11.
pub struct D3D11Encoder {
    // Ordem dos campos = ordem de drop: frame e encoder antes do contexto HW.
    frame: *mut ff::AVFrame,
    encoder: VideoEncoder,
    hw: HwContext,
}

// SAFETY: usado por uma thread por vez; o dispositivo D3D11 deve ter
// proteção multithread ativada (ID3D10Multithread) pela plataforma.
unsafe impl Send for D3D11Encoder {}

impl D3D11Encoder {
    /// Cria o encoder sobre o `ID3D11Device` dado (ponteiro COM cru),
    /// tentando os encoders de hardware em ordem (`preferred` primeiro).
    ///
    /// # Safety
    /// `device` deve ser um `ID3D11Device*` válido; `add_ref` deve chamar
    /// `AddRef` nele (o FFmpeg fica com essa referência).
    pub unsafe fn new(
        device: *mut c_void,
        add_ref: impl Fn(*mut c_void),
        settings: &VideoSettings,
        preferred: Option<&str>,
    ) -> Result<Self> {
        let hw = HwContext::new(device, &add_ref, settings.width, settings.height)?;
        let mut names: Vec<&str> = Vec::new();
        if let Some(p) = preferred {
            names.push(p);
        }
        names.extend(
            hw_candidates(settings.codec)
                .iter()
                .filter(|n| Some(**n) != preferred),
        );

        let mut errors = Vec::new();
        for name in names {
            match VideoEncoder::open(
                name,
                settings,
                ff::AVPixelFormat::AV_PIX_FMT_D3D11,
                hw.frames_ref,
            ) {
                Ok(encoder) => {
                    let frame = ff::av_frame_alloc();
                    return Ok(Self { frame, encoder, hw });
                }
                Err(e) => {
                    tracing::warn!("encoder {name} indisponível: {e}");
                    errors.push(format!("{name}: {e}"));
                }
            }
        }
        Err(MediaError::NoEncoder(errors.join("; ")))
    }

    pub fn encoder(&self) -> &VideoEncoder {
        &self.encoder
    }

    pub fn encoder_mut(&mut self) -> &mut VideoEncoder {
        &mut self.encoder
    }

    /// Codifica um frame. `copy_into(textura_destino, subresource)` deve
    /// copiar a textura NV12 de origem para o destino (ex.:
    /// `ID3D11DeviceContext::CopySubresourceRegion`).
    pub fn encode(
        &mut self,
        pts: i64,
        time_us: i64,
        copy_into: impl FnOnce(*mut c_void, u32),
    ) -> Result<Vec<EncodedPacket>> {
        unsafe {
            ff::av_frame_unref(self.frame);
            check(
                ff::av_hwframe_get_buffer(self.hw.frames_ref, self.frame, 0),
                "av_hwframe_get_buffer",
            )?;
            let texture = (*self.frame).data[0] as *mut c_void;
            let index = (*self.frame).data[1] as usize as u32;
            copy_into(texture, index);
            (*self.frame).pts = pts;
            let mut out = Vec::new();
            let r = self.encoder.send_frame(self.frame, time_us, &mut out);
            ff::av_frame_unref(self.frame);
            r.map(|_| out)
        }
    }
}

impl Drop for D3D11Encoder {
    fn drop(&mut self) {
        unsafe { ff::av_frame_free(&mut self.frame) };
    }
}
