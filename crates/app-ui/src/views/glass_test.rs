//! Modo de teste do vidro (spike da Fase 5): `glint --glass-test`.
//!
//! Janela com acrílico e barra de título própria, três painéis do mockup e o uso de
//! CPU/GPU/memória do processo, para medir no PC se a janela transparente é leve.
//! Atalhos: B troca o fundo da janela (acrílico, Mica Alt, Mica, transparente, opaco),
//! T troca claro/escuro, Esc fecha.

use std::time::{Duration, Instant};

use gpui_kit::*;

use crate::components::glass;
use crate::components::window_controls::{drag_area, WindowControls};
use crate::theme::glass::{Palette, DARK, LIGHT};
use crate::theme::{FONT_NUM, FONT_UI};

const BACKDROPS: [(WindowBackgroundAppearance, &str); 5] = [
    (WindowBackgroundAppearance::Blurred, "Acrílico (Blurred)"),
    (WindowBackgroundAppearance::MicaAltBackdrop, "Mica Alt"),
    (WindowBackgroundAppearance::MicaBackdrop, "Mica"),
    (WindowBackgroundAppearance::Transparent, "Transparente"),
    (WindowBackgroundAppearance::Opaque, "Opaco"),
];

/// Amostra de uso do processo, 1×/s (é o único redesenho com a janela parada).
#[derive(Default)]
struct Usage {
    cpu: Option<f64>,
    cpu_avg: Option<f64>,
    gpu: Option<f64>,
    mem_mb: Option<f64>,
    frames: u32,
}

pub struct GlassTest {
    focus: FocusHandle,
    backdrop: usize,
    dark: bool,
    usage: Usage,
    started: Instant,
}

impl GlassTest {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let dark = matches!(
            window.appearance(),
            WindowAppearance::Dark | WindowAppearance::VibrantDark
        );
        cx.observe_window_appearance(window, |this, window, cx| {
            this.dark = matches!(
                window.appearance(),
                WindowAppearance::Dark | WindowAppearance::VibrantDark
            );
            cx.notify();
        })
        .detach();
        Self::sample(window, cx);
        Self {
            focus,
            backdrop: 0,
            dark,
            usage: Usage::default(),
            started: Instant::now(),
        }
    }

    fn sample(window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |this, cx| {
            let mut probe = Probe::new();
            let mut sum = 0.0;
            let mut n = 0u32;
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let (cpu, gpu, mem) = probe.read();
                if let Some(c) = cpu {
                    sum += c;
                    n += 1;
                }
                let ok = this.update(cx, |t, cx| {
                    t.usage.cpu = cpu;
                    t.usage.cpu_avg = (n > 0).then(|| sum / f64::from(n));
                    t.usage.gpu = gpu;
                    t.usage.mem_mb = mem;
                    cx.notify();
                });
                if ok.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn on_key(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        match e.keystroke.key.as_str() {
            "b" => {
                self.backdrop = (self.backdrop + 1) % BACKDROPS.len();
                window.set_background_appearance(BACKDROPS[self.backdrop].0);
            }
            "t" => self.dark = !self.dark,
            "escape" => window.remove_window(),
            _ => return,
        }
        cx.notify();
    }

    fn pal(&self) -> Palette {
        if self.dark {
            DARK
        } else {
            LIGHT
        }
    }
}

/// CPU (% de todos os núcleos), GPU 3D (%) e memória (MB) do próprio processo.
struct Probe {
    #[cfg(windows)]
    last: Option<(Duration, Instant)>,
    #[cfg(windows)]
    gpu: Option<app_platform_win::foreground::GpuWatch>,
}

impl Probe {
    fn new() -> Self {
        #[cfg(windows)]
        {
            Self {
                last: app_platform_win::process::cpu_time().map(|c| (c, Instant::now())),
                gpu: app_platform_win::foreground::GpuWatch::new(std::process::id()),
            }
        }
        #[cfg(not(windows))]
        Self {}
    }

    fn read(&mut self) -> (Option<f64>, Option<f64>, Option<f64>) {
        #[cfg(windows)]
        {
            let now = Instant::now();
            let cores = std::thread::available_parallelism().map_or(1, |n| n.get()) as f64;
            let cpu = match (app_platform_win::process::cpu_time(), self.last) {
                (Some(c), Some((l, at))) => {
                    self.last = Some((c, now));
                    let wall = now.duration_since(at).as_secs_f64().max(1e-3);
                    Some(c.saturating_sub(l).as_secs_f64() / wall / cores * 100.0)
                }
                _ => None,
            };
            let gpu = self.gpu.as_mut().and_then(|g| g.utilization());
            let mem = app_platform_win::process::working_set().map(|b| b as f64 / 1048576.0);
            (cpu, gpu, mem)
        }
        #[cfg(not(windows))]
        (None, None, None)
    }
}

fn pct(v: Option<f64>) -> String {
    v.map_or("—".into(), |v| format!("{v:.1}%"))
}

fn nav_row(pal: Palette, label: &'static str, count: &'static str, selected: bool) -> Div {
    let row = div()
        .relative()
        .flex()
        .items_center()
        .gap(px(10.))
        .h(px(36.))
        .px(px(10.))
        .rounded(px(12.))
        .text_size(px(14.))
        .child(div().flex_1().child(label))
        .child(
            div()
                .font_family(FONT_NUM)
                .text_size(px(12.))
                .text_color(rgba(pal.text2))
                .child(count),
        );
    if selected {
        row.bg(rgba(pal.sel))
            .font_weight(FontWeight::SEMIBOLD)
            .shadow(vec![
                glass::shadow(4., 14., pal.sel_shadow),
                glass::hairline(if pal.dark { 0xffffff2e } else { 0x00000014 }),
            ])
            .child(glass::highlight(pal.sel_hi, px(12.), 1.))
    } else {
        row
    }
}

impl Render for GlassTest {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.usage.frames += 1;
        let pal = self.pal();
        let backdrop = BACKDROPS[self.backdrop].1;
        let secs = self.started.elapsed().as_secs();

        let sidebar = glass::panel(pal, px(22.))
            .flex()
            .flex_col()
            .flex_none()
            .w(px(232.))
            .h_full()
            .gap(px(2.))
            .pt(px(16.))
            .px(px(10.))
            .pb(px(12.))
            .child(
                drag_area(
                    "drag-brand",
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .px(px(8.))
                        .pb(px(16.)),
                )
                .child(
                    div()
                        .text_size(px(20.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("glint"),
                ),
            )
            .child(nav_row(pal, "Biblioteca", "128", true))
            .child(nav_row(pal, "Favoritos", "12", false))
            .child(nav_row(pal, "Publicados", "7", false))
            .child(div().flex_1())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .p(px(12.))
                    .rounded(px(16.))
                    .bg(rgba(pal.inset))
                    .border_1()
                    .border_color(rgba(pal.panel_border))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(div().size(px(8.)).rounded_full().bg(rgba(pal.red)).shadow(
                                vec![BoxShadow {
                                    color: rgba(pal.red_halo).into(),
                                    offset: point(px(0.), px(0.)),
                                    blur_radius: px(0.),
                                    spread_radius: px(4.),
                                    inset: false,
                                }],
                            ))
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("Gravando VALORANT"),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgba(pal.text2))
                            .child("F8 salva os últimos 60 s"),
                    ),
            );

        let toolbar = div()
            .flex()
            .items_center()
            .gap(px(10.))
            .pl(px(14.))
            .pr(px(6.))
            .py(px(6.))
            .child(
                drag_area(
                    "drag-title",
                    div().flex_1().h(px(38.)).flex().items_end().gap(px(10.)),
                )
                .child(
                    div()
                        .text_size(px(26.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Teste do vidro"),
                )
                .child(
                    div()
                        .pb(px(4.))
                        .text_size(px(14.))
                        .text_color(rgba(pal.text2))
                        .child(backdrop),
                ),
            )
            .child(
                glass::pill(pal)
                    .w(px(240.))
                    .text_size(px(14.))
                    .text_color(rgba(pal.text2))
                    .child("Buscar clipes"),
            )
            .child(glass::pill(pal).text_size(px(14.)).child("Mais recentes"))
            .child(div().ml(px(8.)).child(WindowControls::new(pal)));

        let line = |k: &'static str, v: String| {
            div()
                .flex()
                .justify_between()
                .text_size(px(14.))
                .child(div().text_color(rgba(pal.text2)).child(k))
                .child(div().font_family(FONT_NUM).child(v))
        };
        let metrics = glass::panel(pal, px(20.))
            .flex()
            .flex_col()
            .gap(px(10.))
            .p(px(20.))
            .w(px(420.))
            .child(
                div()
                    .text_size(px(15.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Uso do processo"),
            )
            .child(line("CPU agora", pct(self.usage.cpu)))
            .child(line("CPU média", pct(self.usage.cpu_avg)))
            .child(line("GPU 3D", pct(self.usage.gpu)))
            .child(line(
                "Memória",
                self.usage
                    .mem_mb
                    .map_or("—".into(), |m| format!("{m:.0} MB")),
            ))
            .child(line("Quadros desenhados", self.usage.frames.to_string()))
            .child(line("Aberto há", format!("{secs} s")))
            .child(
                div()
                    .pt(px(6.))
                    .text_size(px(12.))
                    .text_color(rgba(pal.text2))
                    .child(
                        "B troca o fundo · T claro/escuro · Esc fecha. Parado, só redesenha 1×/s.",
                    ),
            );

        let primary = div()
            .flex()
            .items_center()
            .justify_center()
            .h(px(46.))
            .px(px(22.))
            .rounded_full()
            .bg(linear_gradient(
                180.,
                linear_color_stop(rgba(pal.primary_top), 0.),
                linear_color_stop(rgba(pal.primary_bot), 1.),
            ))
            .text_color(rgba(pal.primary_fg))
            .text_size(px(15.))
            .font_weight(FontWeight::SEMIBOLD)
            .shadow(vec![glass::shadow(10., 26., 0x00000059)])
            .child("Publicar");

        let _ = window;
        div()
            .id("glass-test")
            .key_context("GlassTest")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .size_full()
            .flex()
            .gap(px(10.))
            .p(px(10.))
            .bg(rgba(pal.window))
            .font_family(FONT_UI)
            .text_color(rgba(pal.text))
            .child(sidebar)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(6.))
                    .child(toolbar)
                    .child(
                        div()
                            .flex()
                            .gap(px(16.))
                            .p(px(14.))
                            .items_start()
                            .child(metrics)
                            .child(primary),
                    ),
            )
    }
}
