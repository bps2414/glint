//! Player de vídeo dentro do GPUI.
//!
//! Caminho de um quadro (no Windows/Linux o GPUI só aceita imagem BGRA na CPU;
//! `surface()` com textura direta só existe no macOS):
//!
//! ```text
//! MP4 ─► app_media::Player (thread: decode d3d11va → cópia p/ CPU → BGRA no tamanho exibido)
//!     ─► fila curta ─► VideoPlayer::tick (relógio escolhe o quadro) ─► RenderImage ─► atlas da GPU
//! ```
//!
//! Cada quadro exibido vira um `RenderImage` novo; o anterior sai do atlas com
//! `window.drop_image` (sem isso o atlas cresce sem parar).
//!
//! Com áudio ([`VideoPlayer::set_audio`]) o relógio é o do som: o quadro mostrado
//! segue a amostra que está saindo no alto-falante.

pub mod audio;
mod clock;
pub mod stats;

use std::cell::Cell;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use app_media::{BgraFrame, HwPreference, Player, PlayerEvent, VideoInfo};
use gpui_kit::*;

use crate::components::glass::{Area, Backdrop};

pub use audio::AudioOut;
pub use clock::Clock;
pub use stats::Stats;

/// Onde o vídeo foi desenhado no último quadro (pixels lógicos + escala).
type SurfaceSlot = Rc<Cell<Option<(Bounds<Pixels>, f32)>>>;

pub struct VideoPlayer {
    player: Player,
    info: VideoInfo,
    clock: Clock,
    gen: u64,
    /// Seek feito, esperando o primeiro quadro dele.
    awaiting: bool,
    seek_started: Option<Instant>,
    /// Próximo quadro já recebido, esperando a hora dele.
    staged: Option<BgraFrame>,
    /// O `staged` é a prévia (keyframe) de um scrub.
    staged_preview: bool,
    current: Option<Arc<RenderImage>>,
    current_pts: i64,
    current_size: (u32, u32),
    eof: bool,
    pub looping: bool,
    error: Option<String>,
    surface: SurfaceSlot,
    target: (u32, u32),
    pub stats: Stats,
    /// Saída de som (relógio mestre enquanto toca).
    audio: Option<Rc<AudioOut>>,
    /// Faixa de baixo do quadro, desfocada (vidro da cápsula): imagem, pts e quando.
    backdrop: Option<(Arc<RenderImage>, i64, Instant)>,
    backdrop_area: Area,
}

/// No máximo tantos desfoques por segundo para o vidro da cápsula (tocando).
const BACKDROP_EVERY: std::time::Duration = std::time::Duration::from_millis(66);

impl VideoPlayer {
    pub fn open(path: &Path, hw: HwPreference) -> anyhow::Result<Self> {
        Ok(Self::from_player(Player::open(path, hw, (1920, 1080))?))
    }

    /// Embrulha um [`Player`] já aberto (abrir pode ser feito fora da thread da UI).
    pub fn from_player(player: Player) -> Self {
        let info = player.info().clone();
        let mut stats = Stats::new();
        stats.set_frame_ms(info.frame_ms());
        Self {
            target: (info.width, info.height),
            stats,
            player,
            info,
            clock: Clock::new(true),
            gen: 0,
            awaiting: true,
            seek_started: Some(Instant::now()),
            staged: None,
            staged_preview: false,
            current: None,
            current_pts: 0,
            current_size: (0, 0),
            eof: false,
            looping: true,
            error: None,
            surface: Rc::default(),
            audio: None,
            backdrop: None,
            backdrop_area: Area::default(),
        }
    }

    /// Fecha o player tirando as imagens dele do atlas da GPU.
    pub fn release(mut self, window: &mut Window) {
        for img in self
            .current
            .take()
            .into_iter()
            .chain(self.backdrop.take().map(|b| b.0))
        {
            let _ = window.drop_image(img);
        }
    }

    /// Liga o som (as trilhas já estão no `AudioOut`). Se já está tocando,
    /// o som entra do ponto atual.
    pub fn set_audio(&mut self, out: Option<Rc<AudioOut>>) {
        if let Some(old) = self.audio.take() {
            old.stop();
        }
        if let Some(a) = &out {
            if self.clock.playing() && !self.awaiting {
                a.start_at(self.clock.position(Instant::now()));
            }
        }
        self.audio = out;
    }

    /// Posição do relógio: a do som quando ele está tocando.
    fn clock_pos(&self, now: Instant) -> f64 {
        if self.clock.playing() && !self.awaiting {
            if let Some(p) = self.audio.as_ref().and_then(|a| a.position(now)) {
                return p;
            }
        }
        self.clock.position(now)
    }

    pub fn info(&self) -> &VideoInfo {
        &self.info
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn playing(&self) -> bool {
        self.clock.playing()
    }

    /// Posição em ms (a do quadro na tela, para a UI não "pular" à frente dele).
    pub fn position_ms(&self) -> i64 {
        if self.awaiting {
            self.clock.position(Instant::now()) as i64
        } else {
            self.current_pts
        }
    }

    pub fn duration_ms(&self) -> i64 {
        self.info.duration_ms
    }

    /// Tamanho do último quadro convertido (o que a thread entrega).
    pub fn output_size(&self) -> (u32, u32) {
        self.current_size
    }

    pub fn seek(&mut self, ms: i64) {
        self.seek_with(ms, false);
    }

    /// Seek de arrasto: mostra logo o keyframe e depois o quadro exato.
    pub fn scrub(&mut self, ms: i64) {
        self.seek_with(ms, true);
    }

    fn seek_with(&mut self, ms: i64, scrub: bool) {
        let now = Instant::now();
        self.gen = if scrub {
            self.player.scrub(ms)
        } else {
            self.player.seek(ms)
        };
        if let Some(a) = &self.audio {
            a.stop();
        }
        self.clock.seek(ms as f64);
        self.awaiting = true;
        self.seek_started = Some(now);
        self.staged = None;
        self.staged_preview = false;
        self.eof = false;
        self.error = None;
        self.stats.interrupted();
    }

    pub fn toggle(&mut self) {
        let now = Instant::now();
        if self.clock.playing() {
            self.clock.pause(now);
            if let Some(a) = &self.audio {
                a.stop();
            }
            self.stats.interrupted();
        } else {
            let at_end = self.eof
                && self.staged.is_none()
                && self.current_pts as f64 + self.info.frame_ms() * 2.0
                    >= self.info.duration_ms as f64;
            // O relógio volta a andar do quadro que está na tela.
            self.clock.seek(self.current_pts as f64);
            self.clock.play(now);
            if at_end {
                self.seek(0);
            } else if let Some(a) = &self.audio {
                a.start_at(self.current_pts as f64);
            }
        }
    }

    pub fn step(&mut self, delta_ms: i64) {
        let to = (self.position_ms() + delta_ms).clamp(0, self.info.duration_ms);
        self.seek(to);
    }

    /// Avança o player: troca o quadro se for a hora, atualiza métricas.
    /// Devolve `true` se precisa de outro quadro de animação.
    pub fn tick(&mut self, window: &mut Window) -> bool {
        let now = Instant::now();
        self.update_target();

        let frame_ms = self.info.frame_ms();
        let mut pos = self.clock_pos(now);
        let mut chosen: Option<BgraFrame> = None;
        loop {
            if self.staged.is_none() {
                match self.player.try_next() {
                    None => break,
                    Some(PlayerEvent::Frame {
                        gen,
                        frame,
                        decode,
                        convert,
                        skipped,
                        preview,
                    }) => {
                        if gen == self.gen {
                            self.stats.decoded(decode, convert, skipped);
                            self.staged = Some(frame);
                            self.staged_preview = preview;
                        }
                    }
                    Some(PlayerEvent::Eof { gen }) => self.eof |= gen == self.gen,
                    Some(PlayerEvent::Error { gen, message }) => {
                        if gen == self.gen {
                            self.error = Some(message);
                        }
                    }
                }
                continue;
            }
            let pts = self.staged.as_ref().map_or(0, |f| f.pts_ms) as f64;
            if self.awaiting {
                if let Some(t) = self.seek_started.take() {
                    self.stats.seek_done(now - t);
                }
                if std::mem::take(&mut self.staged_preview) {
                    // Prévia do scrub: aparece já, mas o relógio espera o quadro exato.
                    chosen = self.staged.take();
                    continue;
                }
                // Primeiro quadro depois do seek: aparece já, e o relógio parte dele.
                self.awaiting = false;
                if self.clock.playing() {
                    self.clock.start_at(pts, now);
                    if let Some(a) = &self.audio {
                        a.start_at(pts);
                    }
                } else {
                    self.clock.seek(pts);
                }
                pos = pts;
                chosen = self.staged.take();
                if !self.clock.playing() {
                    break;
                }
                continue;
            }
            // Mostra o quadro cuja hora já chegou (meio quadro de folga contra o jitter do vsync).
            if !self.clock.playing() || pts > pos + frame_ms / 2.0 {
                break;
            }
            if chosen.is_some() {
                self.stats.dropped += 1;
            }
            chosen = self.staged.take();
        }

        if let Some(f) = chosen {
            if self.clock.playing() && pos - f.pts_ms as f64 > frame_ms * 1.5 {
                self.stats.late += 1;
            }
            self.current_pts = f.pts_ms;
            self.current_size = (f.width, f.height);
            let img = to_image(f);
            if let Some(old) = self.current.replace(img) {
                let _ = window.drop_image(old);
            }
            self.stats.presented(now, self.clock.playing());
        }

        // Fim: repete (modo de teste) ou para no último quadro.
        if self.eof
            && self.staged.is_none()
            && !self.awaiting
            && self.clock.playing()
            && pos >= self.current_pts as f64 + frame_ms
        {
            if self.looping {
                self.seek(0);
            } else {
                self.clock.pause(now);
                self.clock.seek(self.current_pts as f64);
                if let Some(a) = &self.audio {
                    a.stop();
                }
            }
        }

        self.stats.rendered(now);
        self.clock.playing() || self.awaiting
    }

    /// Manda a thread converter no tamanho em que o vídeo aparece (pixels físicos).
    fn update_target(&mut self) {
        let Some((bounds, scale)) = self.surface.get() else {
            return;
        };
        let fit = fit_bounds(bounds, self.info.width, self.info.height);
        let w = (f32::from(fit.size.width) * scale).round() as u32;
        let h = (f32::from(fit.size.height) * scale).round() as u32;
        let (tw, th) = self.target;
        if w.abs_diff(tw) < 8 && h.abs_diff(th) < 8 {
            return;
        }
        self.target = (w, h);
        self.player.set_target(w.max(16), h.max(2));
        // Pausado: reconverte o quadro da tela no tamanho novo.
        if !self.clock.playing() && !self.awaiting && self.current.is_some() {
            let keep = self.current_pts;
            self.seek(keep);
        }
    }

    /// Vidro da cápsula: a parte de baixo do quadro atual (`frac` da altura), reduzida e
    /// desfocada, e onde ela fica na janela. Refeita só quando o quadro muda e no máximo
    /// a cada [`BACKDROP_EVERY`]; chamar só com a cápsula visível (escondida não calcula).
    pub fn backdrop(&mut self, window: &mut Window, frac: f32) -> Option<Backdrop> {
        let current = self.current.clone()?;
        let (w, h) = self.current_size;
        let band = ((h as f32 * frac).ceil() as u32).clamp(1, h.max(1));
        let stale = match &self.backdrop {
            None => true,
            Some((_, pts, at)) => *pts != self.current_pts && at.elapsed() >= BACKDROP_EVERY,
        };
        if stale {
            let bytes = current.as_bytes(0)?;
            let b = crate::blur::blur_bgra(bytes, w, h, h - band..h);
            if let Some(img) = crate::thumbs::image_from_bgra(b.width, b.height, b.data) {
                if let Some((old, _, _)) =
                    self.backdrop
                        .replace((img, self.current_pts, Instant::now()))
                {
                    let _ = window.drop_image(old);
                }
            }
        }
        // Onde a faixa cai na tela: o fim do retângulo do vídeo (com letterbox).
        if let Some((bounds, _)) = self.surface.get() {
            let fit = fit_bounds(bounds, self.info.width, self.info.height);
            let bh = fit.size.height * (band as f32 / h.max(1) as f32);
            self.backdrop_area.set(Some(Bounds {
                origin: point(fit.origin.x, fit.origin.y + fit.size.height - bh),
                size: size(fit.size.width, bh),
            }));
        }
        let (image, _, _) = self.backdrop.as_ref()?;
        Some(Backdrop {
            image: image.clone(),
            area: self.backdrop_area.clone(),
        })
    }

    /// Elemento que desenha o quadro atual com letterbox, ocupando o pai.
    pub fn surface(&self) -> impl IntoElement {
        self.surface_rounded(px(0.))
    }

    /// Como [`Self::surface`], arredondando os cantos do quadro quando ele ocupa a caixa
    /// inteira (dentro de um player de cantos redondos).
    pub fn surface_rounded(&self, radius: Pixels) -> impl IntoElement {
        let image = self.current.clone();
        let slot = self.surface.clone();
        let (vw, vh) = (self.info.width, self.info.height);
        canvas(
            move |bounds, window, _| {
                slot.set(Some((bounds, window.scale_factor())));
            },
            move |bounds, _, window, _| {
                if let Some(image) = image {
                    let fit = fit_bounds(bounds, vw, vh);
                    let full = (bounds.size.width - fit.size.width).abs() < px(1.)
                        && (bounds.size.height - fit.size.height).abs() < px(1.);
                    let corners = if full {
                        Corners::all(radius)
                    } else {
                        Corners::default()
                    };
                    let _ = window.paint_image(fit, fit, corners, image, 0, false);
                }
            },
        )
        .size_full()
    }
}

/// Converte o quadro BGRA (sem cópia) em imagem do GPUI.
fn to_image(f: BgraFrame) -> Arc<RenderImage> {
    let buf = image::RgbaImage::from_raw(f.width, f.height, f.data)
        .expect("quadro BGRA com tamanho certo");
    Arc::new(RenderImage::new(vec![image::Frame::new(buf)]))
}

/// Retângulo do vídeo centralizado em `area`, mantendo a proporção.
pub fn fit_bounds(area: Bounds<Pixels>, vw: u32, vh: u32) -> Bounds<Pixels> {
    let (aw, ah) = (f32::from(area.size.width), f32::from(area.size.height));
    if vw == 0 || vh == 0 || aw <= 0.0 || ah <= 0.0 {
        return area;
    }
    let (vw, vh) = (vw as f32, vh as f32);
    // O lado que limita fica exato (sem erro de arredondamento na borda).
    let (w, h) = if aw / vw <= ah / vh {
        (aw, vh * aw / vw)
    } else {
        (vw * ah / vh, ah)
    };
    Bounds {
        origin: point(
            area.origin.x + px((aw - w) / 2.0),
            area.origin.y + px((ah - h) / 2.0),
        ),
        size: size(px(w), px(h)),
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::{point, px, size, Bounds};

    #[test]
    fn fit_bounds_centraliza_com_letterbox() {
        let area = Bounds {
            origin: point(px(10.), px(20.)),
            size: size(px(1000.), px(1000.)),
        };
        let r = super::fit_bounds(area, 1920, 1080);
        assert_eq!(r.size.width, px(1000.));
        assert!((f32::from(r.size.height) - 562.5).abs() < 0.01);
        assert!((f32::from(r.origin.y) - (20. + 218.75)).abs() < 0.01);
        assert_eq!(r.origin.x, px(10.));
    }
}
