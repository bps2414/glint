//! Métricas do player (modo de teste): fps exibidos, engasgos, tempo de
//! decodificação/conversão, latência de seek, CPU e memória do processo.

use std::time::{Duration, Instant};

/// Intervalo entre quadros acima disto (em quadros) conta como engasgo.
const STUTTER_FRAMES: f64 = 2.5;

#[derive(Default, Clone, Copy)]
struct Window {
    presented: u32,
    renders: u32,
    decoded: u32,
    decode: Duration,
    convert: Duration,
    max_gap_ms: f64,
    /// Teve seek/pausa nesta janela (não entra no mínimo de fps).
    interrupted: bool,
}

pub struct Stats {
    started: Instant,
    win_start: Instant,
    win: Window,
    last_present: Option<Instant>,
    frame_ms: f64,

    // Valores da última janela de 1 s.
    pub fps: f64,
    pub ui_fps: f64,
    pub decode_ms: f64,
    pub convert_ms: f64,
    pub max_gap_ms: f64,

    // Acumulados desde o início (ou o último reset).
    pub late: u32,
    pub dropped: u32,
    pub stutters: u32,
    pub skipped: u64,
    pub seeks: u32,
    pub seek_last_ms: f64,
    pub seek_max_ms: f64,
    seek_sum_ms: f64,
    pub min_fps: Option<f64>,
    fps_sum: f64,
    fps_secs: u32,

    // Processo (só no Windows por enquanto).
    cpu_last: Option<(Duration, Instant)>,
    pub cpu_pct: Option<f64>,
    cpu_sum: f64,
    cpu_n: u32,
    pub mem_mb: Option<f64>,
    pub mem_peak_mb: f64,
}

impl Stats {
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            started: now,
            win_start: now,
            win: Window::default(),
            last_present: None,
            frame_ms: 1000.0 / 60.0,
            fps: 0.0,
            ui_fps: 0.0,
            decode_ms: 0.0,
            convert_ms: 0.0,
            max_gap_ms: 0.0,
            late: 0,
            dropped: 0,
            stutters: 0,
            skipped: 0,
            seeks: 0,
            seek_last_ms: 0.0,
            seek_max_ms: 0.0,
            seek_sum_ms: 0.0,
            min_fps: None,
            fps_sum: 0.0,
            fps_secs: 0,
            cpu_last: process_cpu().map(|c| (c, now)),
            cpu_pct: None,
            cpu_sum: 0.0,
            cpu_n: 0,
            mem_mb: None,
            mem_peak_mb: 0.0,
        }
    }

    pub fn set_frame_ms(&mut self, frame_ms: f64) {
        self.frame_ms = frame_ms;
    }

    /// Zera os acumulados (tecla R).
    pub fn reset(&mut self) {
        let frame_ms = self.frame_ms;
        *self = Self::new();
        self.frame_ms = frame_ms;
    }

    /// Seek ou pausa: o próximo intervalo não conta como engasgo.
    pub fn interrupted(&mut self) {
        self.last_present = None;
        self.win.interrupted = true;
    }

    pub fn decoded(&mut self, decode: Duration, convert: Duration, skipped: u32) {
        self.win.decoded += 1 + skipped;
        self.win.decode += decode;
        self.win.convert += convert;
        self.skipped += u64::from(skipped);
    }

    pub fn seek_done(&mut self, latency: Duration) {
        let ms = latency.as_secs_f64() * 1000.0;
        self.seeks += 1;
        self.seek_last_ms = ms;
        self.seek_max_ms = self.seek_max_ms.max(ms);
        self.seek_sum_ms += ms;
    }

    pub fn seek_avg_ms(&self) -> f64 {
        if self.seeks == 0 {
            0.0
        } else {
            self.seek_sum_ms / f64::from(self.seeks)
        }
    }

    pub fn avg_fps(&self) -> Option<f64> {
        (self.fps_secs > 0).then(|| self.fps_sum / f64::from(self.fps_secs))
    }

    pub fn avg_cpu(&self) -> Option<f64> {
        (self.cpu_n > 0).then(|| self.cpu_sum / f64::from(self.cpu_n))
    }

    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    pub fn presented(&mut self, now: Instant, playing: bool) {
        self.win.presented += 1;
        if playing {
            if let Some(last) = self.last_present {
                let gap = (now - last).as_secs_f64() * 1000.0;
                self.win.max_gap_ms = self.win.max_gap_ms.max(gap);
                if gap > self.frame_ms * STUTTER_FRAMES {
                    self.stutters += 1;
                }
            }
            self.last_present = Some(now);
        } else {
            self.last_present = None;
        }
    }

    pub fn rendered(&mut self, now: Instant) {
        self.win.renders += 1;
        let dt = now - self.win_start;
        if dt < Duration::from_secs(1) {
            return;
        }
        let secs = dt.as_secs_f64();
        let w = std::mem::take(&mut self.win);
        self.win_start = now;
        self.fps = f64::from(w.presented) / secs;
        self.ui_fps = f64::from(w.renders) / secs;
        let per = |d: Duration| {
            if w.decoded == 0 {
                0.0
            } else {
                d.as_secs_f64() * 1000.0 / f64::from(w.decoded)
            }
        };
        self.decode_ms = per(w.decode);
        self.convert_ms = per(w.convert);
        self.max_gap_ms = w.max_gap_ms;
        // Janela "limpa" (tocando o tempo todo): entra no mínimo e na média.
        if !w.interrupted && self.last_present.is_some() {
            self.min_fps = Some(self.min_fps.map_or(self.fps, |m: f64| m.min(self.fps)));
            self.fps_sum += self.fps;
            self.fps_secs += 1;
        }

        if let (Some(cpu), Some((last, at))) = (process_cpu(), self.cpu_last) {
            let wall = (now - at).as_secs_f64();
            let cores = std::thread::available_parallelism().map_or(1, |n| n.get()) as f64;
            if wall > 0.0 {
                // Como o Gerenciador de Tarefas: % do total de núcleos.
                let pct = cpu.saturating_sub(last).as_secs_f64() / wall / cores * 100.0;
                self.cpu_pct = Some(pct);
                self.cpu_sum += pct;
                self.cpu_n += 1;
            }
            self.cpu_last = Some((cpu, now));
        }
        if let Some(bytes) = process_memory() {
            let mb = bytes as f64 / (1024.0 * 1024.0);
            self.mem_mb = Some(mb);
            self.mem_peak_mb = self.mem_peak_mb.max(mb);
        }
    }
}

impl Default for Stats {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(windows)]
fn process_cpu() -> Option<Duration> {
    app_platform_win::process::cpu_time()
}

#[cfg(windows)]
fn process_memory() -> Option<u64> {
    app_platform_win::process::working_set()
}

#[cfg(not(windows))]
fn process_cpu() -> Option<Duration> {
    None
}

#[cfg(not(windows))]
fn process_memory() -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::Stats;

    #[test]
    fn conta_fps_e_engasgos() {
        let mut s = Stats::new();
        s.set_frame_ms(1000.0 / 60.0);
        let t0 = Instant::now();
        // 1 s a 60 fps com um buraco de 100 ms no meio.
        for i in 0..54 {
            let t = t0 + Duration::from_micros(16_667 * i + if i >= 30 { 100_000 } else { 0 });
            s.presented(t, true);
            s.rendered(t);
        }
        s.rendered(t0 + Duration::from_millis(1001));
        assert_eq!(s.stutters, 1);
        assert!((s.fps - 54.0).abs() < 1.0, "{}", s.fps);
        assert!(s.max_gap_ms > 100.0);
        assert!(s.min_fps.is_some());

        // Seek/pausa: o intervalo seguinte não conta.
        s.interrupted();
        s.presented(t0 + Duration::from_secs(5), true);
        assert_eq!(s.stutters, 1);

        s.seek_done(Duration::from_millis(40));
        s.seek_done(Duration::from_millis(80));
        assert_eq!(s.seeks, 2);
        assert!((s.seek_avg_ms() - 60.0).abs() < 0.01);
        assert!((s.seek_max_ms - 80.0).abs() < 0.01);
    }
}
