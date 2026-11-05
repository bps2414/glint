//! Motor do player: uma thread que decodifica à frente numa fila curta e
//! atende seeks sem travar a UI.
//!
//! - A UI é dona do relógio: ela decide qual quadro mostrar e quando
//!   (`pts_ms` de cada [`PlayerEvent::Frame`]). A thread só mantém a fila cheia.
//! - Cada seek recebe uma **geração**; eventos de gerações antigas devem ser
//!   descartados pela UI. Seeks seguidos (arrastar o scrub) são coalescidos:
//!   vale só o último, e um alvo logo à frente do quadro atual é alcançado
//!   decodificando para a frente, sem voltar ao keyframe.
//! - Seek é exato: decodifica a partir do keyframe anterior e só converte o
//!   primeiro quadro em/depois do alvo (keyframe a cada 1 s ⇒ ≤ 60 quadros).
//! - Scrub ([`Player::scrub`]): igual, mas entrega antes o keyframe como prévia
//!   (`preview = true`), para a tela responder logo enquanto o exato decodifica.

use std::path::Path;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crossbeam_channel::{bounded, unbounded, Receiver, Select, Sender, TryRecvError};

use crate::decode::{BgraFrame, HwPreference, VideoDecoder, VideoInfo};
use crate::error::Result;

/// Quantos quadros prontos a fila segura (≈ 50 ms a 60 fps; 8 MB cada em 1080p).
pub const QUEUE_FRAMES: usize = 3;

/// Até quanto à frente do quadro atual um seek é feito decodificando para a
/// frente em vez de voltar ao keyframe.
const FORWARD_SEEK_MS: i64 = 1_000;
/// No scrub o caminho para a frente não mostra nada até chegar; só vale para alvos bem perto.
const FORWARD_SCRUB_MS: i64 = 250;

#[derive(Debug)]
pub enum PlayerEvent {
    Frame {
        gen: u64,
        frame: BgraFrame,
        /// Tempo decodificando desde o quadro anterior entregue (inclui os pulados).
        decode: Duration,
        /// Cópia da GPU + conversão para BGRA deste quadro.
        convert: Duration,
        /// Quadros decodificados e descartados (seek exato) antes deste.
        skipped: u32,
        /// Keyframe mostrado como prévia de um scrub; o quadro exato vem depois.
        preview: bool,
    },
    /// Fim do arquivo na geração `gen` (o último quadro já foi entregue).
    Eof {
        gen: u64,
    },
    Error {
        gen: u64,
        message: String,
    },
}

enum Cmd {
    Seek { gen: u64, ms: i64, scrub: bool },
    Target { w: u32, h: u32 },
    Stop,
}

pub struct Player {
    info: VideoInfo,
    gen: u64,
    cmd: Sender<Cmd>,
    events: Receiver<PlayerEvent>,
    thread: Option<JoinHandle<()>>,
}

impl Player {
    /// Abre o arquivo e começa a decodificar do início (geração 0), com quadros
    /// cabendo em `target` (pixels físicos).
    pub fn open(path: &Path, hw: HwPreference, target: (u32, u32)) -> Result<Self> {
        let dec = VideoDecoder::open(path, hw)?;
        let info = dec.info().clone();
        let (cmd_tx, cmd_rx) = unbounded();
        let (ev_tx, ev_rx) = bounded(QUEUE_FRAMES);
        let thread = std::thread::Builder::new()
            .name("player-video".into())
            .spawn(move || Worker::new(dec, target, cmd_rx, ev_tx).run())?;
        Ok(Self {
            info,
            gen: 0,
            cmd: cmd_tx,
            events: ev_rx,
            thread: Some(thread),
        })
    }

    pub fn info(&self) -> &VideoInfo {
        &self.info
    }

    /// Geração atual (a do último seek).
    pub fn generation(&self) -> u64 {
        self.gen
    }

    /// Seek exato para `ms`. Devolve a nova geração.
    pub fn seek(&mut self, ms: i64) -> u64 {
        self.send_seek(ms, false)
    }

    /// Seek para arrastar: entrega o keyframe anterior como prévia e depois o quadro exato.
    pub fn scrub(&mut self, ms: i64) -> u64 {
        self.send_seek(ms, true)
    }

    fn send_seek(&mut self, ms: i64, scrub: bool) -> u64 {
        self.gen += 1;
        let ms = ms.clamp(0, self.info.duration_ms.max(0));
        let _ = self.cmd.send(Cmd::Seek {
            gen: self.gen,
            ms,
            scrub,
        });
        self.gen
    }

    /// Tamanho máximo dos próximos quadros (pixels físicos).
    pub fn set_target(&self, w: u32, h: u32) {
        let _ = self.cmd.send(Cmd::Target { w, h });
    }

    /// Próximo evento, se houver (não bloqueia).
    pub fn try_next(&self) -> Option<PlayerEvent> {
        self.events.try_recv().ok()
    }

    /// Próximo evento, esperando até `timeout` (testes e ferramentas).
    pub fn next_timeout(&self, timeout: Duration) -> Option<PlayerEvent> {
        self.events.recv_timeout(timeout).ok()
    }

    /// Quadros prontos na fila.
    pub fn queued(&self) -> usize {
        self.events.len()
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        let _ = self.cmd.send(Cmd::Stop);
        // A thread pode estar bloqueada entregando um quadro: esvaziar a fila a solta.
        while self.events.try_recv().is_ok() {}
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

struct Worker {
    dec: VideoDecoder,
    target: (u32, u32),
    cmd: Receiver<Cmd>,
    tx: Sender<PlayerEvent>,
    gen: u64,
    /// Seek exato em andamento: descarta quadros antes deste ms.
    skip_until: Option<i64>,
    /// Scrub: o primeiro quadro depois do keyframe sai como prévia.
    preview_pending: bool,
    skipped: u32,
    /// Fim do arquivo (ou erro): espera um seek.
    ended: bool,
    pending: std::collections::VecDeque<PlayerEvent>,
    decode_acc: Duration,
    half_frame_ms: i64,
}

enum Flow {
    Continue,
    Stop,
}

impl Worker {
    fn new(
        dec: VideoDecoder,
        target: (u32, u32),
        cmd: Receiver<Cmd>,
        tx: Sender<PlayerEvent>,
    ) -> Self {
        let half_frame_ms = (dec.info().frame_ms() / 2.0).floor() as i64;
        Self {
            dec,
            target,
            cmd,
            tx,
            gen: 0,
            skip_until: None,
            preview_pending: false,
            skipped: 0,
            ended: false,
            pending: Default::default(),
            decode_acc: Duration::ZERO,
            half_frame_ms,
        }
    }

    fn run(mut self) {
        loop {
            // 1. Comandos pendentes (seeks em rajada: o último vence).
            loop {
                match self.cmd.try_recv() {
                    Ok(c) => {
                        if let Flow::Stop = self.apply(c) {
                            return;
                        }
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => return,
                }
            }

            // 2. Entregar o que já está pronto (bloqueia até a fila ter espaço ou chegar comando).
            if let Some(ev) = self.pending.pop_front() {
                let mut sel = Select::new();
                let send_op = sel.send(&self.tx);
                let recv_op = sel.recv(&self.cmd);
                let op = sel.select();
                if op.index() == send_op {
                    if op.send(&self.tx, ev).is_err() {
                        return;
                    }
                } else {
                    debug_assert_eq!(op.index(), recv_op);
                    let Ok(c) = op.recv(&self.cmd) else { return };
                    self.pending.push_front(ev);
                    if let Flow::Stop = self.apply(c) {
                        return;
                    }
                }
                continue;
            }

            // 3. Fim do arquivo: dorme até o próximo comando.
            if self.ended {
                match self.cmd.recv() {
                    Ok(c) => {
                        if let Flow::Stop = self.apply(c) {
                            return;
                        }
                    }
                    Err(_) => return,
                }
                continue;
            }

            // 4. Decodificar o próximo quadro.
            self.step();
        }
    }

    fn step(&mut self) {
        let t = Instant::now();
        match self.dec.decode_next() {
            Ok(Some(pts)) => {
                self.decode_acc += t.elapsed();
                if let Some(target) = self.skip_until {
                    if pts + self.half_frame_ms < target {
                        if std::mem::take(&mut self.preview_pending) {
                            self.emit_current(true);
                        } else {
                            self.skipped += 1;
                        }
                        return;
                    }
                    self.skip_until = None;
                }
                self.preview_pending = false;
                self.emit_current(false);
            }
            Ok(None) => {
                self.decode_acc += t.elapsed();
                // Seek além do último quadro: mostra o último.
                self.preview_pending = false;
                if self.skip_until.take().is_some() && self.dec.current_pts_ms().is_some() {
                    self.emit_current(false);
                }
                self.ended = true;
                self.pending.push_back(PlayerEvent::Eof { gen: self.gen });
            }
            Err(e) => self.fail(e.to_string()),
        }
    }

    fn emit_current(&mut self, preview: bool) {
        let t = Instant::now();
        match self.dec.convert(self.target.0, self.target.1) {
            Ok(frame) => {
                self.pending.push_back(PlayerEvent::Frame {
                    gen: self.gen,
                    frame,
                    decode: std::mem::take(&mut self.decode_acc),
                    convert: t.elapsed(),
                    skipped: std::mem::take(&mut self.skipped),
                    preview,
                });
            }
            Err(e) => self.fail(e.to_string()),
        }
    }

    fn fail(&mut self, message: String) {
        tracing::warn!("player: {message}");
        self.ended = true;
        self.skip_until = None;
        self.preview_pending = false;
        self.pending.push_back(PlayerEvent::Error {
            gen: self.gen,
            message,
        });
    }

    fn apply(&mut self, c: Cmd) -> Flow {
        match c {
            Cmd::Stop => return Flow::Stop,
            Cmd::Target { w, h } => self.target = (w.max(1), h.max(1)),
            Cmd::Seek { gen, ms, scrub } => {
                self.gen = gen;
                // O que estava pronto é de outra geração.
                self.pending.clear();
                self.decode_acc = Duration::ZERO;
                self.skipped = 0;
                self.preview_pending = false;
                let forward = if scrub {
                    FORWARD_SCRUB_MS
                } else {
                    FORWARD_SEEK_MS
                };
                let cur = self.dec.current_pts_ms();
                match cur {
                    // O quadro atual já é o alvo (ex.: redimensionou pausado): reconverte.
                    Some(p) if !self.ended && (ms - p).abs() <= self.half_frame_ms => {
                        self.skip_until = None;
                        self.emit_current(false);
                    }
                    // Logo à frente: decodifica para a frente, sem voltar ao keyframe.
                    Some(p) if !self.ended && ms > p && ms - p <= forward => {
                        self.skip_until = Some(ms);
                    }
                    _ => match self.dec.seek(ms) {
                        Ok(()) => {
                            self.ended = false;
                            self.skip_until = Some(ms);
                            self.preview_pending = scrub;
                        }
                        Err(e) => self.fail(e.to_string()),
                    },
                }
            }
        }
        Flow::Continue
    }
}
