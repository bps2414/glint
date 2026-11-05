use ffmpeg_sys_next as ff;

use crate::error::{check, MediaError, Result};

/// Parâmetros de uma stream codificada (codec, extradata, dimensões...),
/// copiados do encoder, necessários para muxar os pacotes depois.
pub struct StreamInfo {
    pub(crate) params: *mut ff::AVCodecParameters,
    pub(crate) time_base: ff::AVRational,
}

// SAFETY: AVCodecParameters é um bloco de dados próprio (sem referências a
// estado do encoder); acesso é exclusivo via &/&mut.
unsafe impl Send for StreamInfo {}
unsafe impl Sync for StreamInfo {}

impl StreamInfo {
    pub(crate) unsafe fn from_context(ctx: *const ff::AVCodecContext) -> Result<Self> {
        let params = ff::avcodec_parameters_alloc();
        if params.is_null() {
            return Err(MediaError::Other("avcodec_parameters_alloc".into()));
        }
        let info = Self {
            params,
            time_base: (*ctx).time_base,
        };
        check(
            ff::avcodec_parameters_from_context(params, ctx),
            "avcodec_parameters_from_context",
        )?;
        Ok(info)
    }

    /// Time base (num, den) dos timestamps dos pacotes desta stream.
    pub fn time_base(&self) -> (i32, i32) {
        (self.time_base.num, self.time_base.den)
    }

    pub fn is_video(&self) -> bool {
        unsafe { (*self.params).codec_type == ff::AVMediaType::AVMEDIA_TYPE_VIDEO }
    }

    pub fn codec_name(&self) -> String {
        unsafe {
            let name = ff::avcodec_get_name((*self.params).codec_id);
            std::ffi::CStr::from_ptr(name)
                .to_string_lossy()
                .into_owned()
        }
    }
}

impl Clone for StreamInfo {
    fn clone(&self) -> Self {
        unsafe {
            let params = ff::avcodec_parameters_alloc();
            assert!(!params.is_null(), "avcodec_parameters_alloc");
            ff::avcodec_parameters_copy(params, self.params);
            Self {
                params,
                time_base: self.time_base,
            }
        }
    }
}

impl Drop for StreamInfo {
    fn drop(&mut self) {
        unsafe { ff::avcodec_parameters_free(&mut self.params) };
    }
}
