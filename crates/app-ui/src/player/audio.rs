//! Saída de áudio do player (cpal) e relógio de áudio.
//!
//! As trilhas do clipe (PCM 48 kHz estéreo, ver `app_media::pcm`) ficam em
//! memória; o callback da placa mistura as ligadas (jogo, microfone), aplica o
//! volume e reamostra linear para a taxa do dispositivo. A posição tocada vira
//! o relógio do player: o callback anota "a amostra X sai no alto-falante no
//! instante T" e [`AudioOut::position`] interpola a partir disso.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use app_media::pcm::RATE;
use app_media::PcmTrack;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, OutputCallbackInfo, SampleFormat, SizedSample, Stream, StreamConfig};

/// Trilhas que o player sabe ligar/desligar (0 = jogo, 1 = microfone).
pub const MAX_TRACKS: usize = 2;

struct State {
    tracks: Arc<Vec<PcmTrack>>,
    /// Posição em quadros de 48 kHz (fracionária: reamostragem).
    pos: f64,
    /// Muda a cada seek/play/pausa: o callback só grava de volta se não mudou.
    epoch: u64,
    playing: bool,
    enabled: [bool; MAX_TRACKS],
    gain: f32,
    /// (ms da primeira amostra do último buffer, instante em que ela toca).
    anchor: Option<(f64, Instant)>,
    /// Atraso do callback até o alto-falante (último medido).
    latency: Duration,
}

pub struct AudioOut {
    state: Arc<Mutex<State>>,
    stream: Stream,
    running: std::cell::Cell<bool>,
}

impl AudioOut {
    /// Abre o dispositivo de saída padrão. `None` = sem placa de som (o player
    /// usa o relógio de parede).
    pub fn open() -> Option<Self> {
        let host = cpal::default_host();
        let device = host.default_output_device()?;
        let supported = device
            .default_output_config()
            .map_err(|e| tracing::warn!("áudio: sem configuração de saída: {e}"))
            .ok()?;
        let format = supported.sample_format();
        let config: StreamConfig = supported.into();
        let state = Arc::new(Mutex::new(State {
            tracks: Arc::new(Vec::new()),
            pos: 0.0,
            epoch: 0,
            playing: false,
            enabled: [true; MAX_TRACKS],
            gain: 1.0,
            anchor: None,
            latency: Duration::from_millis(20),
        }));
        let stream = match format {
            SampleFormat::F32 => build::<f32>(&device, config, state.clone()),
            SampleFormat::I16 => build::<i16>(&device, config, state.clone()),
            SampleFormat::I32 => build::<i32>(&device, config, state.clone()),
            SampleFormat::U16 => build::<u16>(&device, config, state.clone()),
            SampleFormat::F64 => build::<f64>(&device, config, state.clone()),
            other => {
                tracing::warn!("áudio: formato de saída {other} não suportado");
                return None;
            }
        }
        .map_err(|e| tracing::warn!("áudio: não abriu a saída: {e}"))
        .ok()?;
        Some(Self {
            state,
            stream,
            running: std::cell::Cell::new(false),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Troca as trilhas (clipe novo) e para.
    pub fn set_tracks(&self, tracks: Vec<PcmTrack>) {
        let mut s = self.lock();
        s.tracks = Arc::new(tracks);
        s.playing = false;
        s.anchor = None;
        s.epoch += 1;
    }

    pub fn set_enabled(&self, track: usize, on: bool) {
        if track < MAX_TRACKS {
            self.lock().enabled[track] = on;
        }
    }

    pub fn set_gain(&self, gain: f32) {
        self.lock().gain = gain.clamp(0.0, 2.0);
    }

    /// Começa a tocar em `ms` (chegou o quadro de vídeo desse ponto).
    pub fn start_at(&self, ms: f64) {
        {
            let mut s = self.lock();
            s.pos = ms.max(0.0) * f64::from(RATE) / 1000.0;
            s.epoch += 1;
            s.playing = true;
            s.anchor = Some((ms, Instant::now() + s.latency));
        }
        self.run(true);
    }

    /// Para (pausa ou seek em andamento).
    pub fn stop(&self) {
        {
            let mut s = self.lock();
            s.playing = false;
            s.anchor = None;
            s.epoch += 1;
        }
        self.run(false);
    }

    /// Posição audível agora, em ms. `None` = parado (vale o relógio de parede).
    pub fn position(&self, now: Instant) -> Option<f64> {
        let s = self.lock();
        if !s.playing {
            return None;
        }
        let (ms, at) = s.anchor?;
        Some(if now >= at {
            ms + (now - at).as_secs_f64() * 1000.0
        } else {
            ms - (at - now).as_secs_f64() * 1000.0
        })
    }

    fn run(&self, on: bool) {
        if self.running.get() == on {
            return;
        }
        let r = if on {
            self.stream.play()
        } else {
            self.stream.pause()
        };
        match r {
            Ok(()) => self.running.set(on),
            Err(e) => tracing::warn!("áudio: play/pause falhou: {e}"),
        }
    }
}

fn build<T>(
    device: &cpal::Device,
    config: StreamConfig,
    state: Arc<Mutex<State>>,
) -> Result<Stream, cpal::Error>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = usize::from(config.channels).max(1);
    let step = f64::from(RATE) / f64::from(config.sample_rate.max(1));
    let mut scratch: Vec<f32> = Vec::new();
    let stream = device.build_output_stream(
        config,
        move |out: &mut [T], info: &OutputCallbackInfo| {
            let frames = out.len() / channels;
            scratch.clear();
            scratch.resize(frames * 2, 0.0);
            let ts = info.timestamp();
            let latency = ts
                .playback
                .duration_since(ts.callback)
                .min(Duration::from_millis(500));

            // Lê o estado (trava curta; sem alocação nem E/S dentro dela).
            let snapshot = {
                let s = state.lock().unwrap_or_else(|e| e.into_inner());
                s.playing
                    .then(|| (s.tracks.clone(), s.pos, s.epoch, s.enabled, s.gain))
            };
            let now = Instant::now();
            if let Some((tracks, start, epoch, enabled, gain)) = snapshot {
                let pos = mix(&tracks, enabled, gain, start, step, &mut scratch);
                let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
                s.latency = latency;
                if s.epoch == epoch {
                    s.pos = pos;
                    s.anchor = Some((start * 1000.0 / f64::from(RATE), now + latency));
                }
            } else if let Ok(mut s) = state.try_lock() {
                s.latency = latency;
            }

            for (frame, lr) in out.chunks_mut(channels).zip(scratch.chunks(2)) {
                for (c, sample) in frame.iter_mut().enumerate() {
                    let v = if c < 2 { lr[c.min(1)] } else { 0.0 };
                    *sample = T::from_sample(v);
                }
            }
        },
        |e| tracing::warn!("áudio: erro na saída: {e}"),
        None,
    )?;
    Ok(stream)
}

/// Mistura as trilhas ligadas a partir de `start` (quadros de 48 kHz), andando
/// `step` por quadro de saída, em `out` (L, R...). Devolve a posição final.
fn mix(
    tracks: &[PcmTrack],
    enabled: [bool; MAX_TRACKS],
    gain: f32,
    start: f64,
    step: f64,
    out: &mut [f32],
) -> f64 {
    let mut p = start;
    for lr in out.chunks_mut(2) {
        let i = p as usize;
        let frac = (p - i as f64) as f32;
        let (mut l, mut r) = (0.0f32, 0.0f32);
        for (k, t) in tracks.iter().enumerate() {
            if !enabled.get(k).copied().unwrap_or(true) {
                continue;
            }
            let s = &t.samples;
            let at = |j: usize, c: usize| s.get(j * 2 + c).map_or(0.0, |&v| f32::from(v));
            l += at(i, 0) + (at(i + 1, 0) - at(i, 0)) * frac;
            r += at(i, 1) + (at(i + 1, 1) - at(i, 1)) * frac;
        }
        let k = gain / 32768.0;
        lr[0] = (l * k).clamp(-1.0, 1.0);
        lr[1] = (r * k).clamp(-1.0, 1.0);
        p += step;
    }
    p
}

#[cfg(test)]
mod tests {
    use super::{mix, MAX_TRACKS};
    use app_media::PcmTrack;

    #[test]
    fn mistura_trilhas_ligadas() {
        let game = PcmTrack {
            samples: vec![1000, -1000, 2000, -2000, 3000, -3000],
        };
        let mic = PcmTrack {
            samples: vec![500, 500, 500, 500, 500, 500],
        };
        let tracks = vec![game, mic];
        let mut out = vec![0.0; 6];
        let end = mix(&tracks, [true; MAX_TRACKS], 1.0, 0.0, 1.0, &mut out);
        assert_eq!(end, 3.0);
        assert!((out[0] - 1500.0 / 32768.0).abs() < 1e-6);
        assert!((out[1] - -500.0 / 32768.0).abs() < 1e-6);

        // Microfone desligado, metade da taxa (interpola entre amostras).
        let mut out = vec![0.0; 4];
        mix(&tracks, [true, false], 1.0, 0.0, 0.5, &mut out);
        assert!((out[2] - 1500.0 / 32768.0).abs() < 1e-6, "{out:?}");

        // Depois do fim: silêncio.
        let mut out = vec![1.0; 2];
        mix(&tracks, [true; MAX_TRACKS], 1.0, 10.0, 1.0, &mut out);
        assert_eq!(out, vec![0.0, 0.0]);
    }
}
