//! Modo de teste do player (spike da Fase 3): `glint --player-test [arquivo.mp4]`.
//!
//! Toca o clipe em repetição com as métricas na tela, para decidir se o GPUI
//! aguenta 1080p60. Atalhos: Espaço
//! (tocar/pausar), F (tela cheia), ←/→ (±5 s), ,/. (quadro a quadro), R (zerar
//! métricas), H (esconder métricas), L (repetir).

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use app_media::HwPreference;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::components::{Btn, BtnKind, Kbd, Toggle};
use crate::format;
use crate::player::{Stats, VideoPlayer};
use crate::theme::{tok, FONT_NUM, FONT_UI, R_SM};
use crate::{tr, tr_args};

/// Valor arrastado no scrub.
#[derive(Clone, Copy)]
struct ScrubDrag;

/// "Fantasma" vazio exigido pelo arrasto do GPUI.
struct DragGhost;

impl Render for DragGhost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

pub struct PlayerTest {
    focus: FocusHandle,
    hw: HwPreference,
    path: Option<PathBuf>,
    player: Option<VideoPlayer>,
    open_error: Option<String>,
    show_stats: bool,
    copied: bool,
    scrub_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
}

impl PlayerTest {
    pub fn new(
        path: Option<PathBuf>,
        hw: HwPreference,
        auto: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let mut this = Self {
            focus,
            hw,
            path: None,
            player: None,
            open_error: None,
            show_stats: true,
            copied: false,
            scrub_bounds: Rc::default(),
        };
        if let Some(p) = path {
            this.load(p, cx);
        }
        if let Some(out) = auto {
            this.run_auto(out, window, cx);
        }
        this
    }

    /// Roteiro do teste guiado: 20 s em janela, 20 s em tela cheia e 10 seeks;
    /// grava os três relatórios em `out` e fecha o app.
    fn run_auto(&mut self, out: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        const SECS: u64 = 20;
        cx.spawn_in(window, async move |this, cx| {
            let mut parts: Vec<String> = Vec::new();
            let reset = |t: &mut Self, cx: &mut Context<Self>| {
                if let Some(p) = t.player.as_mut() {
                    p.looping = true;
                    if !p.playing() {
                        p.toggle();
                    }
                    p.stats.reset();
                }
                cx.notify();
            };
            cx.background_executor().timer(Duration::from_secs(2)).await;

            let _ = this.update(cx, reset);
            cx.background_executor()
                .timer(Duration::from_secs(SECS))
                .await;
            if let Ok(r) = this.update(cx, |t, _| t.report_text()) {
                parts.push(format!("--- Janela ({SECS} s) ---\n{r}"));
            }

            let _ = this.update_in(cx, |_, window, _| window.toggle_fullscreen());
            cx.background_executor()
                .timer(Duration::from_millis(1500))
                .await;
            let _ = this.update(cx, reset);
            cx.background_executor()
                .timer(Duration::from_secs(SECS))
                .await;
            if let Ok(r) = this.update(cx, |t, _| t.report_text()) {
                parts.push(format!("--- Tela cheia ({SECS} s) ---\n{r}"));
            }

            let _ = this.update(cx, reset);
            for i in 0..10i64 {
                let _ = this.update(cx, |t, cx| {
                    if let Some(p) = t.player.as_mut() {
                        let d = p.duration_ms();
                        p.scrub(d * ((i * 37) % 100) / 100);
                    }
                    cx.notify();
                });
                cx.background_executor()
                    .timer(Duration::from_millis(500))
                    .await;
            }
            cx.background_executor().timer(Duration::from_secs(1)).await;
            if let Ok(r) = this.update(cx, |t, _| t.report_text()) {
                parts.push(format!("--- Seeks (10) ---\n{r}"));
            }

            if let Err(e) = std::fs::write(&out, parts.join("\n\n")) {
                eprintln!("gravando {}: {e}", out.display());
            }
            let _ = this.update(cx, |_, cx| cx.quit());
        })
        .detach();
    }

    fn report_text(&self) -> String {
        match (self.player.as_ref(), self.path.as_ref()) {
            (Some(p), Some(path)) => report(p, path),
            _ => format!(
                "ERRO: {}",
                self.open_error
                    .as_deref()
                    .unwrap_or("nenhum clipe para tocar")
            ),
        }
    }

    fn load(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        // Fecha o anterior antes (libera o decoder da GPU).
        self.player = None;
        match VideoPlayer::open(&path, self.hw) {
            Ok(p) => {
                self.player = Some(p);
                self.open_error = None;
            }
            Err(e) => self.open_error = Some(format!("{e:#}")),
        }
        self.path = Some(path);
        self.copied = false;
        cx.notify();
    }

    fn pick_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: None,
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await {
                if let Some(p) = paths.into_iter().next() {
                    let _ = this.update(cx, |this, cx| this.load(p, cx));
                }
            }
        })
        .detach();
    }

    fn scrub_to(&mut self, x: Pixels, cx: &mut Context<Self>) {
        let (Some(b), Some(p)) = (self.scrub_bounds.get(), self.player.as_mut()) else {
            return;
        };
        if b.size.width <= px(0.) {
            return;
        }
        let frac = ((x - b.left()) / b.size.width).clamp(0., 1.);
        p.scrub((f64::from(frac) * p.duration_ms() as f64) as i64);
        cx.notify();
    }

    fn on_key(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = e.keystroke.key.as_str();
        match key {
            "f" => window.toggle_fullscreen(),
            "escape" if window.is_fullscreen() => window.toggle_fullscreen(),
            "h" => self.show_stats = !self.show_stats,
            "o" => self.pick_file(window, cx),
            _ => {}
        }
        if let Some(p) = self.player.as_mut() {
            let frame = p.info().frame_ms().round() as i64;
            match key {
                "space" => p.toggle(),
                "left" => p.step(-5_000),
                "right" => p.step(5_000),
                "," if !p.playing() => p.step(-frame),
                "." if !p.playing() => p.step(frame),
                "r" => p.stats.reset(),
                "l" => p.looping = !p.looping,
                _ => {}
            }
        }
        cx.notify();
    }

    fn copy_report(&mut self, cx: &mut Context<Self>) {
        if let (Some(p), Some(path)) = (self.player.as_ref(), self.path.as_ref()) {
            cx.write_to_clipboard(ClipboardItem::new_string(report(p, path)));
            self.copied = true;
            cx.notify();
        }
    }

    fn empty_state(&self, cx: &mut Context<Self>) -> Div {
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(14.))
            .child(
                div()
                    .text_size(px(18.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(tr("player_test.title")),
            )
            .child(
                div()
                    .max_w(px(520.))
                    .text_center()
                    .text_size(px(13.))
                    .text_color(tok::fg_2())
                    .child(match &self.open_error {
                        Some(e) => tr_args("player_test.open_error", &[("error", e)]),
                        None => tr("player_test.empty"),
                    }),
            )
            .child(
                Btn::new("pt-open", BtnKind::Primary, tr("player_test.open"))
                    .on_click(cx.listener(|this, _, window, cx| this.pick_file(window, cx))),
            )
    }

    fn controls(&self, p: &VideoPlayer, cx: &mut Context<Self>) -> Div {
        let dur = p.duration_ms().max(1);
        let pos = p.position_ms().clamp(0, dur);
        let frac = pos as f32 / dur as f32;
        let slot = self.scrub_bounds.clone();
        let secs = |ms: i64| ms as f32 / 1000.0;

        let scrub = div()
            .id("pt-scrub")
            .flex_1()
            .min_w_0()
            .h(px(28.))
            .relative()
            .flex()
            .items_center()
            .cursor(CursorStyle::PointingHand)
            .child(
                canvas(move |b, _, _| slot.set(Some(b)), |_, _, _, _| {})
                    .absolute()
                    .inset_0(),
            )
            .child(
                div()
                    .w_full()
                    .h(px(4.))
                    .rounded(px(2.))
                    .bg(tok::surf_2())
                    .child(
                        div()
                            .h_full()
                            .w(relative(frac))
                            .rounded(px(2.))
                            .bg(tok::blue()),
                    ),
            )
            .child(
                div()
                    .absolute()
                    .top(px(8.))
                    .left(relative(frac))
                    .ml(px(-6.))
                    .size(px(12.))
                    .rounded_full()
                    .bg(rgb(0xffffff)),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, e: &MouseDownEvent, _, cx| this.scrub_to(e.position.x, cx)),
            )
            .on_drag(ScrubDrag, |_, _, _, cx| cx.new(|_| DragGhost))
            .on_drag_move::<ScrubDrag>(cx.listener(|this, e: &DragMoveEvent<ScrubDrag>, _, cx| {
                this.scrub_to(e.event.position.x, cx)
            }));

        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(14.))
            .px(px(16.))
            .py(px(10.))
            .bg(tok::panel())
            .border_t_1()
            .border_color(tok::line())
            .child(
                Btn::new(
                    "pt-play",
                    BtnKind::Secondary,
                    if p.playing() {
                        tr("player_test.pause")
                    } else {
                        tr("player_test.play")
                    },
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    if let Some(p) = this.player.as_mut() {
                        p.toggle();
                    }
                    cx.notify();
                })),
            )
            .child(
                div()
                    .font_family(FONT_NUM)
                    .text_size(px(12.))
                    .text_color(tok::fg_2())
                    .child(format!(
                        "{} / {}",
                        format::clock(secs(pos)),
                        format::clock(secs(dur))
                    )),
            )
            .child(scrub)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.))
                    .text_size(px(12.))
                    .text_color(tok::fg_2())
                    .child(tr("player_test.loop"))
                    .child(Toggle::new("pt-loop", p.looping).on_click(cx.listener(
                        |this, _, _, cx| {
                            if let Some(p) = this.player.as_mut() {
                                p.looping = !p.looping;
                            }
                            cx.notify();
                        },
                    ))),
            )
            .child(
                Btn::new(
                    "pt-copy",
                    BtnKind::Ghost,
                    if self.copied {
                        tr("player_test.copied")
                    } else {
                        tr("player_test.copy")
                    },
                )
                .on_click(cx.listener(|this, _, _, cx| this.copy_report(cx))),
            )
            .child(
                Btn::new("pt-open2", BtnKind::Ghost, tr("player_test.open"))
                    .on_click(cx.listener(|this, _, window, cx| this.pick_file(window, cx))),
            )
            .child(
                Btn::new("pt-full", BtnKind::Ghost, tr("player_test.fullscreen"))
                    .on_click(|_, window, _| window.toggle_fullscreen()),
            )
    }
}

impl Render for PlayerTest {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let animate = self.player.as_mut().is_some_and(|p| p.tick(window));
        if animate {
            window.request_animation_frame();
        }

        let root = div()
            .id("player-test")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .on_drop::<ExternalPaths>(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                if let Some(p) = paths.paths().first() {
                    this.load(p.clone(), cx);
                }
            }))
            .size_full()
            .flex()
            .flex_col()
            .bg(tok::black())
            .font_family(FONT_UI)
            .text_color(tok::fg());

        let Some(p) = self.player.as_ref() else {
            return root.child(self.empty_state(cx));
        };

        let video = div()
            .id("pt-video")
            .flex_1()
            .min_h_0()
            .relative()
            .child(p.surface())
            .on_click(cx.listener(|this, e: &ClickEvent, window, cx| {
                if e.click_count() >= 2 {
                    window.toggle_fullscreen();
                } else if let Some(p) = this.player.as_mut() {
                    p.toggle();
                }
                cx.notify();
            }))
            .when(self.show_stats, |d| {
                d.child(stats_panel(p, self.path.as_deref()))
            })
            .when_some(p.error(), |d, e| {
                d.child(
                    div()
                        .absolute()
                        .bottom(px(16.))
                        .left(px(16.))
                        .px(px(12.))
                        .py(px(8.))
                        .rounded(R_SM)
                        .bg(tok::blue())
                        .text_size(px(12.))
                        .child(tr_args("player_test.play_error", &[("error", e)])),
                )
            });

        let controls = self.controls(p, cx);
        root.child(video).child(controls)
    }
}

fn kv(label: SharedString, value: String) -> Div {
    div()
        .flex()
        .flex_row()
        .justify_between()
        .gap(px(18.))
        .child(div().text_color(tok::muted()).child(label))
        .child(div().text_color(tok::fg()).child(value))
}

fn dec(v: f64, places: usize) -> String {
    format::decimal(v as f32, places)
}

fn opt(v: Option<f64>, places: usize, unit: &str) -> String {
    v.map_or_else(|| "—".to_string(), |v| format!("{}{unit}", dec(v, places)))
}

fn decoder_label(p: &VideoPlayer) -> SharedString {
    match p.info().hw {
        Some(name) => tr_args("player_test.hw", &[("name", name)]),
        None => tr("player_test.sw"),
    }
}

fn stats_panel(p: &VideoPlayer, path: Option<&Path>) -> Div {
    let info = p.info();
    let s: &Stats = &p.stats;
    let (ow, oh) = p.output_size();
    let name = path
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    div()
        .absolute()
        .top(px(12.))
        .left(px(12.))
        .w(px(300.))
        .p(px(12.))
        .rounded(R_SM)
        .bg(tok::chip())
        .flex()
        .flex_col()
        .gap(px(3.))
        .font_family(FONT_NUM)
        .text_size(px(11.5))
        .child(
            div()
                .font_family(FONT_UI)
                .font_weight(FontWeight::SEMIBOLD)
                .text_size(px(12.))
                .pb(px(4.))
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(name),
        )
        .child(kv(
            tr("player_test.source"),
            format!(
                "{}×{} · {} fps · {}",
                info.width,
                info.height,
                dec(info.fps, 0),
                info.codec
            ),
        ))
        .child(kv(tr("player_test.decoder"), decoder_label(p).to_string()))
        .child(kv(tr("player_test.output"), format!("{ow}×{oh}")))
        .child(div().h(px(6.)))
        .child(kv(tr("player_test.fps"), dec(s.fps, 1)))
        .child(kv(
            tr("player_test.fps_avg_min"),
            format!("{} / {}", opt(s.avg_fps(), 1, ""), opt(s.min_fps, 1, "")),
        ))
        .child(kv(tr("player_test.ui_fps"), dec(s.ui_fps, 0)))
        .child(kv(
            tr("player_test.max_gap"),
            format!("{} ms", dec(s.max_gap_ms, 1)),
        ))
        .child(kv(tr("player_test.stutters"), s.stutters.to_string()))
        .child(kv(tr("player_test.late"), s.late.to_string()))
        .child(kv(tr("player_test.dropped"), s.dropped.to_string()))
        .child(div().h(px(6.)))
        .child(kv(
            tr("player_test.decode"),
            format!("{} ms", dec(s.decode_ms, 2)),
        ))
        .child(kv(
            tr("player_test.convert"),
            format!("{} ms", dec(s.convert_ms, 2)),
        ))
        .child(kv(
            tr("player_test.seek"),
            format!(
                "{} / {} ms ({})",
                dec(s.seek_last_ms, 0),
                dec(s.seek_max_ms, 0),
                s.seeks
            ),
        ))
        .child(div().h(px(6.)))
        .child(kv(tr("player_test.cpu"), opt(s.cpu_pct, 1, "%")))
        .child(kv(
            tr("player_test.memory"),
            format!("{} MB", opt(s.mem_mb, 0, "")),
        ))
        .child(
            div()
                .pt(px(8.))
                .flex()
                .flex_row()
                .flex_wrap()
                .gap(px(6.))
                .font_family(FONT_UI)
                .text_size(px(11.))
                .text_color(tok::muted())
                .child(Kbd(tr("player_test.key_space")))
                .child(Kbd("F".into()))
                .child(Kbd("← →".into()))
                .child(Kbd(", .".into()))
                .child(Kbd("R".into()))
                .child(Kbd("H".into()))
                .child(tr("player_test.keys")),
        )
}

/// Texto para o usuário colar no chat com o resultado do teste.
fn report(p: &VideoPlayer, path: &Path) -> String {
    let info = p.info();
    let s = &p.stats;
    let (ow, oh) = p.output_size();
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let lines = [
        tr("player_test.report_title").to_string(),
        format!(
            "{}: {name} ({}×{}, {} fps, {}, {} s)",
            tr("player_test.file"),
            info.width,
            info.height,
            dec(info.fps, 0),
            info.codec,
            dec(info.duration_ms as f64 / 1000.0, 1)
        ),
        format!("{}: {}", tr("player_test.decoder"), decoder_label(p)),
        format!("{}: {ow}×{oh}", tr("player_test.output")),
        format!(
            "{}: {} s",
            tr("player_test.duration"),
            s.elapsed().as_secs()
        ),
        format!(
            "{}: {} / {}",
            tr("player_test.fps_avg_min"),
            opt(s.avg_fps(), 1, ""),
            opt(s.min_fps, 1, "")
        ),
        format!(
            "{}: {} · {}: {} · {}: {}",
            tr("player_test.stutters"),
            s.stutters,
            tr("player_test.late"),
            s.late,
            tr("player_test.dropped"),
            s.dropped
        ),
        format!(
            "{}: {} ms · {}: {} ms",
            tr("player_test.decode"),
            dec(s.decode_ms, 2),
            tr("player_test.convert"),
            dec(s.convert_ms, 2)
        ),
        format!(
            "{}: {} {} · {} {} ms · {} {} ms",
            tr("player_test.seek_report"),
            s.seeks,
            tr("player_test.seeks"),
            tr("player_test.avg"),
            dec(s.seek_avg_ms(), 0),
            tr("player_test.worst"),
            dec(s.seek_max_ms, 0)
        ),
        format!(
            "{}: {} · {}: {} MB ({} {} MB)",
            tr("player_test.cpu_avg"),
            opt(s.avg_cpu(), 1, "%"),
            tr("player_test.memory"),
            opt(s.mem_mb, 0, ""),
            tr("player_test.peak"),
            dec(s.mem_peak_mb, 0)
        ),
    ];
    lines.join("\n")
}
