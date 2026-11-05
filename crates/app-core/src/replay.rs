//! Replay buffer: janela deslizante de pacotes codificados.
//!
//! Invariantes:
//! - A stream de vídeo sempre começa num keyframe (para o clipe poder ser
//!   salvo por *remux*, sem re-encode).
//! - Pacotes de áudio anteriores ao primeiro frame de vídeo retido são
//!   descartados.

use std::collections::VecDeque;

use crate::packet::{EncodedPacket, StreamId};

/// Stream de vídeo é sempre a 0.
pub const VIDEO_STREAM: StreamId = 0;

#[derive(Debug)]
pub struct ReplayBuffer {
    capacity_us: i64,
    max_bytes: usize,
    video: VecDeque<EncodedPacket>,
    audio: VecDeque<EncodedPacket>,
    bytes: usize,
}

/// Cópia barata (os dados são `Arc`) de um trecho do buffer, pronta para mux.
#[derive(Debug, Default)]
pub struct ReplaySnapshot {
    /// Pacotes em ordem de chegada por stream: primeiro vídeo, depois áudio.
    pub video: Vec<EncodedPacket>,
    pub audio: Vec<EncodedPacket>,
}

impl ReplaySnapshot {
    pub fn is_empty(&self) -> bool {
        self.video.is_empty()
    }

    /// Instante (µs) do primeiro frame de vídeo.
    pub fn start_us(&self) -> Option<i64> {
        self.video.first().map(|p| p.time_us)
    }

    /// Duração aproximada do vídeo em µs.
    pub fn duration_us(&self) -> i64 {
        match (self.video.first(), self.video.last()) {
            (Some(a), Some(b)) => b.time_us - a.time_us,
            _ => 0,
        }
    }
}

impl ReplayBuffer {
    /// `capacity_secs`: quanto tempo para trás manter.
    /// `max_bytes`: teto de memória (proteção contra bitrate absurdo).
    pub fn new(capacity_secs: u32, max_bytes: usize) -> Self {
        Self {
            capacity_us: i64::from(capacity_secs) * 1_000_000,
            max_bytes,
            video: VecDeque::new(),
            audio: VecDeque::new(),
            bytes: 0,
        }
    }

    pub fn capacity_secs(&self) -> u32 {
        (self.capacity_us / 1_000_000) as u32
    }

    pub fn set_capacity_secs(&mut self, secs: u32) {
        self.capacity_us = i64::from(secs) * 1_000_000;
        self.trim();
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }

    pub fn len(&self) -> usize {
        self.video.len() + self.audio.len()
    }

    pub fn is_empty(&self) -> bool {
        self.video.is_empty() && self.audio.is_empty()
    }

    /// Duração de vídeo retida em µs.
    pub fn video_span_us(&self) -> i64 {
        match (self.video.front(), self.video.back()) {
            (Some(a), Some(b)) => b.time_us - a.time_us,
            _ => 0,
        }
    }

    pub fn clear(&mut self) {
        self.video.clear();
        self.audio.clear();
        self.bytes = 0;
    }

    pub fn push(&mut self, pkt: EncodedPacket) {
        self.bytes += pkt.size();
        if pkt.stream == VIDEO_STREAM {
            // Nunca começar a stream de vídeo sem keyframe.
            if self.video.is_empty() && !pkt.keyframe {
                self.bytes -= pkt.size();
                return;
            }
            self.video.push_back(pkt);
        } else {
            self.audio.push_back(pkt);
        }
        self.trim();
    }

    fn latest_us(&self) -> Option<i64> {
        let v = self.video.back().map(|p| p.time_us);
        let a = self.audio.back().map(|p| p.time_us);
        match (v, a) {
            (Some(v), Some(a)) => Some(v.max(a)),
            (v, a) => v.or(a),
        }
    }

    fn trim(&mut self) {
        let Some(latest) = self.latest_us() else {
            return;
        };
        let cutoff = latest - self.capacity_us;

        // Vídeo: manter a partir do último keyframe <= cutoff, garantindo
        // pelo menos `capacity` de conteúdo e começo em keyframe.
        if let Some(idx) = self
            .video
            .iter()
            .rposition(|p| p.keyframe && p.time_us <= cutoff)
        {
            self.drop_video_front(idx);
        }

        // Teto de memória: descartar GOPs inteiros do início.
        while self.bytes > self.max_bytes && self.video.len() > 1 {
            let next_key = self
                .video
                .iter()
                .skip(1)
                .position(|p| p.keyframe)
                .map(|i| i + 1);
            match next_key {
                Some(i) => self.drop_video_front(i),
                None => break,
            }
        }

        // Áudio: nada antes do primeiro frame de vídeo (ou do cutoff, se
        // ainda não há vídeo).
        let audio_cut = self.video.front().map(|p| p.time_us).unwrap_or(cutoff);
        while let Some(front) = self.audio.front() {
            if front.time_us < audio_cut {
                let p = self.audio.pop_front().expect("front existe");
                self.bytes -= p.size();
            } else {
                break;
            }
        }
    }

    fn drop_video_front(&mut self, count: usize) {
        for _ in 0..count {
            if let Some(p) = self.video.pop_front() {
                self.bytes -= p.size();
            }
        }
    }

    /// Copia os últimos `secs` segundos (ou tudo, se `None`).
    /// O vídeo começa no keyframe mais próximo **antes** do início pedido.
    pub fn snapshot(&self, secs: Option<u32>) -> ReplaySnapshot {
        let Some(latest) = self.latest_us() else {
            return ReplaySnapshot::default();
        };
        let start = match secs {
            Some(s) => latest - i64::from(s) * 1_000_000,
            None => i64::MIN,
        };
        let first = self
            .video
            .iter()
            .rposition(|p| p.keyframe && p.time_us <= start)
            .unwrap_or(0);
        let video: Vec<_> = self.video.iter().skip(first).cloned().collect();
        let v_start = video.first().map(|p| p.time_us).unwrap_or(start);
        let audio = self
            .audio
            .iter()
            .filter(|p| p.time_us >= v_start)
            .cloned()
            .collect();
        ReplaySnapshot { video, audio }
    }

    /// Copia o trecho `[start_us, end_us]` (instantes no mesmo relógio dos pacotes).
    /// O vídeo começa no último keyframe `<= start_us` (ou no primeiro pacote, se o buffer já
    /// perdeu esse trecho) e termina no último pacote `<= end_us`; o áudio cobre o mesmo trecho.
    pub fn snapshot_range(&self, start_us: i64, end_us: i64) -> ReplaySnapshot {
        let first = self
            .video
            .iter()
            .rposition(|p| p.keyframe && p.time_us <= start_us)
            .unwrap_or(0);
        let video: Vec<_> = self
            .video
            .iter()
            .skip(first)
            .take_while(|p| p.time_us <= end_us)
            .cloned()
            .collect();
        let v_start = video.first().map(|p| p.time_us).unwrap_or(start_us);
        let audio = self
            .audio
            .iter()
            .filter(|p| (v_start..=end_us).contains(&p.time_us))
            .cloned()
            .collect();
        ReplaySnapshot { video, audio }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    const FPS: i64 = 60;
    const FRAME_US: i64 = 1_000_000 / FPS;

    fn video(i: i64, gop: i64) -> EncodedPacket {
        EncodedPacket {
            stream: VIDEO_STREAM,
            pts: i,
            dts: i,
            duration: 1,
            time_us: i * FRAME_US,
            keyframe: i % gop == 0,
            data: Arc::from(vec![0u8; 1000]),
        }
    }

    fn audio(i: i64) -> EncodedPacket {
        // AAC: 1024 amostras a 48 kHz ≈ 21,333 ms
        let t = i * 1024 * 1_000_000 / 48_000;
        EncodedPacket {
            stream: 1,
            pts: i * 1024,
            dts: i * 1024,
            duration: 1024,
            time_us: t,
            keyframe: true,
            data: Arc::from(vec![0u8; 100]),
        }
    }

    fn fill(buf: &mut ReplayBuffer, secs: i64, gop: i64) {
        let frames = secs * FPS;
        let mut a = 0;
        for i in 0..frames {
            buf.push(video(i, gop));
            while audio(a).time_us <= i * FRAME_US {
                buf.push(audio(a));
                a += 1;
            }
        }
    }

    #[test]
    fn keeps_about_capacity_starting_at_keyframe() {
        let mut buf = ReplayBuffer::new(10, usize::MAX);
        fill(&mut buf, 30, 60);
        let span = buf.video_span_us();
        assert!(span >= 10_000_000, "span {span}");
        assert!(span <= 11_000_000 + FRAME_US, "span {span}");
        assert!(buf.video.front().unwrap().keyframe);
    }

    #[test]
    fn drops_leading_non_keyframes() {
        let mut buf = ReplayBuffer::new(10, usize::MAX);
        buf.push(video(1, 60));
        buf.push(video(2, 60));
        assert!(buf.video.is_empty());
        buf.push(video(60, 60));
        assert_eq!(buf.video.len(), 1);
    }

    #[test]
    fn audio_never_precedes_video() {
        let mut buf = ReplayBuffer::new(5, usize::MAX);
        fill(&mut buf, 20, 120);
        let v0 = buf.video.front().unwrap().time_us;
        assert!(buf.audio.iter().all(|p| p.time_us >= v0));
        assert!(!buf.audio.is_empty());
    }

    #[test]
    fn snapshot_starts_on_keyframe_before_request() {
        let mut buf = ReplayBuffer::new(60, usize::MAX);
        fill(&mut buf, 60, 60);
        let snap = buf.snapshot(Some(15));
        assert!(snap.video[0].keyframe);
        let d = snap.duration_us();
        assert!((15_000_000 - FRAME_US..=16_000_000).contains(&d), "dur {d}");
        let v0 = snap.start_us().unwrap();
        assert!(snap.audio.iter().all(|p| p.time_us >= v0));
    }

    #[test]
    fn snapshot_range_cuts_both_ends() {
        let mut buf = ReplayBuffer::new(60, usize::MAX);
        fill(&mut buf, 40, 60);
        // 10 s a 14 s: começa no keyframe de até 1 s antes (GOP de 1 s) e termina em 14 s.
        let snap = buf.snapshot_range(10_000_000, 14_000_000);
        assert!(snap.video[0].keyframe);
        let v0 = snap.start_us().unwrap();
        assert!((9_000_000..=10_000_000).contains(&v0), "início {v0}");
        let last = snap.video.last().unwrap().time_us;
        assert!(
            (14_000_000 - FRAME_US..=14_000_000).contains(&last),
            "fim {last}"
        );
        let in_range = |p: &EncodedPacket| (v0..=14_000_000).contains(&p.time_us);
        assert!(snap.audio.iter().all(in_range));
        assert!(!snap.audio.is_empty());
    }

    #[test]
    fn snapshot_range_starts_on_keyframe_before_start() {
        let mut buf = ReplayBuffer::new(60, usize::MAX);
        fill(&mut buf, 40, 60);
        // 10,5 s cai no meio de um GOP: volta para o keyframe anterior.
        let snap = buf.snapshot_range(10_500_000, 12_000_000);
        assert!(snap.video[0].keyframe);
        assert!(snap.start_us().unwrap() <= 10_500_000);
        assert!(snap.start_us().unwrap() > 10_500_000 - 1_000_000 - FRAME_US);
        let v0 = snap.start_us().unwrap();
        assert!(snap.audio.iter().all(|p| p.time_us >= v0));
    }

    #[test]
    fn snapshot_range_clamps_to_what_the_buffer_has() {
        let mut buf = ReplayBuffer::new(10, usize::MAX);
        fill(&mut buf, 30, 60);
        let first = buf.video.front().unwrap().time_us;
        let latest = buf.video.back().unwrap().time_us;
        // Início já descartado e fim no futuro: devolve o buffer inteiro.
        let snap = buf.snapshot_range(0, latest + 5_000_000);
        assert_eq!(snap.start_us(), Some(first));
        assert_eq!(snap.video.last().unwrap().time_us, latest);
        // Intervalo fora do que existe: vazio.
        assert!(buf.snapshot_range(0, first - 1).is_empty());
    }

    #[test]
    fn byte_accounting_is_exact() {
        let mut buf = ReplayBuffer::new(3, usize::MAX);
        fill(&mut buf, 10, 30);
        let real: usize = buf
            .video
            .iter()
            .chain(buf.audio.iter())
            .map(|p| p.size())
            .sum();
        assert_eq!(real, buf.bytes());
    }

    #[test]
    fn memory_cap_drops_whole_gops() {
        // ~60 frames/s * 1000 B = 60 KB/s; teto de 100 KB => < 2 s retidos
        let mut buf = ReplayBuffer::new(60, 100_000);
        fill(&mut buf, 10, 30);
        assert!(buf.bytes() <= 100_000 + 30 * 1000);
        assert!(buf.video.front().unwrap().keyframe);
    }

    #[test]
    fn shrinking_capacity_trims() {
        let mut buf = ReplayBuffer::new(60, usize::MAX);
        fill(&mut buf, 30, 60);
        buf.set_capacity_secs(5);
        assert!(buf.video_span_us() <= 6_000_000 + FRAME_US);
    }
}
