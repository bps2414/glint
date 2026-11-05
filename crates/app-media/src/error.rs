use std::ffi::CStr;

#[derive(Debug, thiserror::Error)]
pub enum MediaError {
    #[error("FFmpeg: {context}: {message} ({code})")]
    Ffmpeg {
        context: &'static str,
        code: i32,
        message: String,
    },
    #[error("nenhum encoder disponível (tentados: {0})")]
    NoEncoder(String),
    #[error("{0}")]
    Other(String),
    #[error("cancelado")]
    Cancelled,
    #[error("E/S: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, MediaError>;

pub(crate) fn av_err_string(code: i32) -> String {
    let mut buf = [0 as libc::c_char; 256];
    unsafe {
        ffmpeg_sys_next::av_strerror(code, buf.as_mut_ptr(), buf.len());
        CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned()
    }
}

/// Converte um código de retorno do FFmpeg em `Result`.
pub(crate) fn check(code: i32, context: &'static str) -> Result<i32> {
    if code < 0 {
        Err(MediaError::Ffmpeg {
            context,
            code,
            message: av_err_string(code),
        })
    } else {
        Ok(code)
    }
}

pub(crate) const EAGAIN: i32 = ffmpeg_sys_next::AVERROR(libc::EAGAIN);
