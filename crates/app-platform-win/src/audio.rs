//! Captura de áudio via WASAPI (crate `wasapi`).
//!
//! - `System`: loopback do dispositivo de saída padrão (tudo que toca no PC).
//! - `Process(pid)`: loopback só do processo (e filhos) — áudio do jogo sem
//!   Discord/Spotify. API genérica do Windows (10 2004+), não toca no jogo.
//! - `Microphone`: dispositivo de entrada padrão.
//!
//! Sempre pede 48 kHz float (o Windows converte com `autoconvert`), entregando
//! amostras intercaladas + timestamp no relógio comum (QPC, µs).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

use wasapi::{AudioClient, DeviceEnumerator, Direction, SampleType, StreamMode, WaveFormat};

use crate::clock;
use crate::{Error, Result};

pub const SAMPLE_RATE: u32 = 48_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioSource {
    System,
    Process(u32),
    Microphone,
}

impl AudioSource {
    pub fn channels(self) -> u32 {
        match self {
            AudioSource::Microphone => 1,
            _ => 2,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            AudioSource::System => "sistema",
            AudioSource::Process(_) => "jogo",
            AudioSource::Microphone => "microfone",
        }
    }
}

fn werr(e: impl std::fmt::Display) -> Error {
    Error::Wasapi(e.to_string())
}

fn open_client(source: AudioSource) -> Result<AudioClient> {
    match source {
        AudioSource::Process(pid) => {
            AudioClient::new_application_loopback_client(pid, true).map_err(werr)
        }
        AudioSource::System => DeviceEnumerator::new()
            .and_then(|e| e.get_default_device(&Direction::Render))
            .and_then(|d| d.get_iaudioclient())
            .map_err(werr),
        AudioSource::Microphone => DeviceEnumerator::new()
            .and_then(|e| e.get_default_device(&Direction::Capture))
            .and_then(|d| d.get_iaudioclient())
            .map_err(werr),
    }
}

/// Inicia a captura numa thread própria. `on_samples(amostras_intercaladas,
/// time_us)` é chamado a cada pacote. A thread termina quando `stop` vira true.
///
/// Retorna erro (síncrono) se o dispositivo não puder ser aberto.
pub fn spawn_capture(
    source: AudioSource,
    stop: Arc<AtomicBool>,
    mut on_samples: impl FnMut(&[f32], i64) + Send + 'static,
) -> Result<JoinHandle<()>> {
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<()>>();
    let handle = std::thread::Builder::new()
        .name(format!("audio-{}", source.label()))
        .spawn(move || {
            let _ = wasapi::initialize_mta();
            let channels = source.channels() as usize;
            let setup = (|| -> Result<_> {
                let mut client = open_client(source)?;
                let fmt = WaveFormat::new(
                    32,
                    32,
                    &SampleType::Float,
                    SAMPLE_RATE as usize,
                    channels,
                    None,
                );
                let mode = StreamMode::EventsShared {
                    autoconvert: true,
                    buffer_duration_hns: 200_000, // 20 ms
                };
                client
                    .initialize_client(&fmt, &Direction::Capture, &mode)
                    .map_err(werr)?;
                let event = client.set_get_eventhandle().map_err(werr)?;
                let capture = client.get_audiocaptureclient().map_err(werr)?;
                client.start_stream().map_err(werr)?;
                Ok((client, event, capture))
            })();
            let (client, event, capture) = match setup {
                Ok(v) => {
                    let _ = ready_tx.send(Ok(()));
                    v
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                    return;
                }
            };
            tracing::info!(fonte = source.label(), "captura de áudio iniciada");

            let bytes_per_frame = 4 * channels;
            let mut raw = vec![0u8; SAMPLE_RATE as usize * bytes_per_frame]; // 1 s
            let mut samples: Vec<f32> = Vec::with_capacity(SAMPLE_RATE as usize * channels);
            while !stop.load(Ordering::Relaxed) {
                // Loopback não sinaliza eventos durante silêncio: timeout é normal.
                if event.wait_for_event(100).is_err() {
                    continue;
                }
                loop {
                    match capture.get_next_packet_size() {
                        Ok(Some(n)) if n > 0 => {}
                        Ok(_) => break,
                        Err(e) => {
                            tracing::warn!(fonte = source.label(), "erro no áudio: {e}");
                            break;
                        }
                    }
                    let (frames, info) = match capture.read_from_device(&mut raw) {
                        Ok(v) => v,
                        Err(e) => {
                            tracing::warn!(fonte = source.label(), "erro lendo áudio: {e}");
                            break;
                        }
                    };
                    if frames == 0 {
                        break;
                    }
                    let n = frames as usize * channels;
                    samples.clear();
                    if info.flags.silent {
                        samples.resize(n, 0.0);
                    } else {
                        samples.extend(
                            raw[..n * 4]
                                .chunks_exact(4)
                                .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])),
                        );
                    }
                    let t = if info.timestamp > 0 && !info.flags.timestamp_error {
                        clock::hns_to_us(info.timestamp)
                    } else {
                        clock::now_us() - i64::from(frames) * 1_000_000 / i64::from(SAMPLE_RATE)
                    };
                    on_samples(&samples, t);
                }
            }
            let _ = client.stop_stream();
            tracing::info!(fonte = source.label(), "captura de áudio encerrada");
        })
        .map_err(|e| Error::Other(e.to_string()))?;

    match ready_rx.recv() {
        Ok(Ok(())) => Ok(handle),
        Ok(Err(e)) => {
            let _ = handle.join();
            Err(e)
        }
        Err(_) => Err(Error::Other(
            "thread de áudio morreu na inicialização".into(),
        )),
    }
}
