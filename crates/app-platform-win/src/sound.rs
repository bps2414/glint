//! "Blip" de confirmação ao salvar um clipe e "tick" do aperto de kill (sem overlay no jogo).

use std::sync::OnceLock;

use windows::core::PCWSTR;
use windows::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};

/// WAV PCM 16-bit mono gerado em memória a partir de tons (Hz, ms).
fn tones_wav(tones: &[(f32, u32)]) -> Vec<u8> {
    {
        let rate = 44_100u32;
        let mut pcm: Vec<i16> = Vec::new();
        for &(freq, ms) in tones {
            let n = rate * ms / 1000;
            for i in 0..n {
                let t = i as f32 / rate as f32;
                // envelope suave para não estalar
                let env = (std::f32::consts::PI * i as f32 / n as f32).sin();
                let s = (2.0 * std::f32::consts::PI * freq * t).sin() * env * 0.35;
                pcm.push((s * i16::MAX as f32) as i16);
            }
        }
        let data_len = (pcm.len() * 2) as u32;
        let mut wav = Vec::with_capacity(44 + data_len as usize);
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data_len).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
        wav.extend_from_slice(&1u16.to_le_bytes()); // mono
        wav.extend_from_slice(&rate.to_le_bytes());
        wav.extend_from_slice(&(rate * 2).to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_len.to_le_bytes());
        for s in pcm {
            wav.extend_from_slice(&s.to_le_bytes());
        }
        wav
    }
}

/// Dois tons curtos ascendentes: clipe salvo.
fn blip_wav() -> &'static [u8] {
    static WAV: OnceLock<Vec<u8>> = OnceLock::new();
    WAV.get_or_init(|| tones_wav(&[(880.0, 55), (1320.0, 70)]))
}

/// Um toque curto e agudo: apertou a hotkey de kill (o clipe só é salvo depois do combo).
fn tick_wav() -> &'static [u8] {
    static WAV: OnceLock<Vec<u8>> = OnceLock::new();
    WAV.get_or_init(|| tones_wav(&[(1760.0, 40)]))
}

/// Dois tons graves descendentes: nada foi salvo (não está gravando ou falhou).
fn error_wav() -> &'static [u8] {
    static WAV: OnceLock<Vec<u8>> = OnceLock::new();
    WAV.get_or_init(|| tones_wav(&[(440.0, 90), (294.0, 140)]))
}

pub fn play_save_blip() {
    play(blip_wav());
}

pub fn play_kill_tick() {
    play(tick_wav());
}

pub fn play_error_blip() {
    play(error_wav());
}

fn play(wav: &'static [u8]) {
    unsafe {
        let _ = PlaySoundW(
            PCWSTR(wav.as_ptr() as *const u16),
            None,
            SND_MEMORY | SND_ASYNC | SND_NODEFAULT,
        );
    }
}
