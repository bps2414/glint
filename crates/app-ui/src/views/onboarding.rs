//! Onboarding em tela cheia (substitui as boas-vindas da 1.0): boas-vindas, teclas com teste
//! ao vivo, qualidade, servidor, sistema e resumo. Aparece enquanto `Config::onboarding` for
//! menor que [`ONBOARDING_ATUAL`]; concluir ou pular grava a versão. Cada passo grava na hora
//! (`apply`, que já manda o gravador reler a config). Plano em
//! `docs/PLANO-1.1-onboarding-launcher.md`.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use app_core::{Codec, Config, Resolution};
use app_ipc::RecorderStatus;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::app::Shell;
use crate::assets::icon;
use crate::components::window_controls::{drag_area, WindowControls};
use crate::components::{glass, input_box, Btn, BtnKind, KeyCap, Toggle};
use crate::recorder;
use crate::theme::size::Size;
use crate::theme::{self, tok, FONT_NUM, R_PANEL, R_SM};
use crate::views::server::{os_user_name, Ask};
use crate::views::settings::same_hotkey;
use crate::{tr, tr_args};

/// Versão do onboarding deste app. Subir refaz o onboarding de todo mundo uma vez.
pub const ONBOARDING_ATUAL: u32 = 2;

/// Intervalo do laço do onboarding (medidor do microfone); o status do gravador é lido a cada
/// dois ciclos (250 ms) no passo das teclas.
const TICK: Duration = Duration::from_millis(125);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Step {
    Welcome,
    Keys,
    Quality,
    Server,
    System,
    Done,
}

const STEPS: [Step; 6] = [
    Step::Welcome,
    Step::Keys,
    Step::Quality,
    Step::Server,
    Step::System,
    Step::Done,
];

impl Step {
    fn key(self) -> &'static str {
        match self {
            Step::Welcome => "onboarding.step_welcome",
            Step::Keys => "onboarding.step_keys",
            Step::Quality => "onboarding.step_quality",
            Step::Server => "onboarding.step_server",
            Step::System => "onboarding.step_system",
            Step::Done => "onboarding.step_done",
        }
    }
}

/// Qual hotkey (mesma ordem dos contadores do gravador).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Slot {
    Save = 0,
    Kill = 1,
}

/// Níveis de qualidade do passo 3.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    Light,
    Balanced,
    Max,
}

const PRESETS: [Preset; 3] = [Preset::Light, Preset::Balanced, Preset::Max];

impl Preset {
    /// (codec, resolução, fps, kbps). `hevc` = a GPU grava HEVC por hardware.
    fn values(self, hevc: bool) -> (Codec, Resolution, u32, u32) {
        let best = if hevc { Codec::Hevc } else { Codec::H264 };
        match self {
            Preset::Light => (Codec::H264, Resolution::P720, 60, 12_000),
            Preset::Balanced => (best, Resolution::P1080, 60, 25_000),
            Preset::Max => (best, Resolution::Native, 60, 40_000),
        }
    }

    fn name(self) -> SharedString {
        tr(match self {
            Preset::Light => "onboarding.preset_light",
            Preset::Balanced => "onboarding.preset_balanced",
            Preset::Max => "onboarding.preset_max",
        })
    }

    fn hint(self) -> SharedString {
        tr(match self {
            Preset::Light => "onboarding.preset_light_hint",
            Preset::Balanced => "onboarding.preset_balanced_hint",
            Preset::Max => "onboarding.preset_max_hint",
        })
    }

    fn apply(self, hevc: bool, c: &mut Config) {
        let (codec, res, fps, kbps) = self.values(hevc);
        c.video.codec = codec;
        c.video.resolution = res;
        c.video.fps = fps;
        c.video.bitrate_kbps = kbps;
    }

    /// Nível que bate com a config (com ou sem HEVC); `None` = personalizado.
    fn of(c: &Config) -> Option<Preset> {
        let v = &c.video;
        let now = (v.codec, v.resolution, v.fps, v.bitrate_kbps);
        PRESETS
            .into_iter()
            .find(|p| p.values(true) == now || p.values(false) == now)
    }
}

/// Nível recomendado pelos encoders que abriram: HEVC ou H.264 por hardware → Equilibrado;
/// nenhum → Leve. Máximo nunca é marcado sozinho.
fn recommended(encoders: &[&str]) -> Preset {
    if encoders.is_empty() {
        Preset::Light
    } else {
        Preset::Balanced
    }
}

fn has_hevc(encoders: Option<&[&'static str]>) -> bool {
    encoders.is_none_or(|e| e.iter().any(|n| n.starts_with("hevc")))
}

fn spec_text(codec: Codec, res: Resolution, fps: u32, kbps: u32) -> String {
    let res = match res {
        Resolution::P720 => "720p".to_string(),
        Resolution::P1080 => "1080p".to_string(),
        Resolution::Native => tr("settings.native").to_string(),
    };
    format!("{}, {res}, {fps} fps, {} Mbps", codec.label(), kbps / 1000)
}

/// Captura curta do microfone só para o medidor (solta = para).
struct MicMeter {
    stop: Arc<AtomicBool>,
    /// Pico recente (f32 em bits), já com decaimento.
    level: Arc<AtomicU32>,
}

impl MicMeter {
    fn level(&self) -> f32 {
        f32::from_bits(self.level.load(Ordering::Relaxed))
    }
}

impl Drop for MicMeter {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

pub struct Onboarding {
    step: usize,
    /// Veio de uma versão anterior (texto das boas-vindas muda, a qualidade não muda sozinha).
    upgraded: bool,
    /// Tecla esperando o próximo aperto.
    capture: Option<Slot>,
    key_error: Option<SharedString>,
    /// Apertos vistos no último status (por [`Slot`]).
    presses: [u64; 2],
    /// Apertos quando o passo das teclas abriu (`None` = ainda sem status).
    base: Option<[u64; 2]>,
    /// O gravador contou um aperto de cada tecla neste passo.
    saw: [bool; 2],
    /// Encoders que abriram nesta GPU (`None` = testando).
    encoders: Option<Vec<&'static str>>,
    gpu: Option<String>,
    /// A pessoa já escolheu um nível (não troca sozinho quando o teste termina).
    chose_preset: bool,
    mic: Option<MicMeter>,
    invite: Option<Entity<InputState>>,
    busy: bool,
    error: Option<SharedString>,
    /// "Trocar de servidor?" aberto.
    confirm_leave: bool,
    starting_recorder: bool,
    tick: u32,
}

impl Onboarding {
    fn step(&self) -> Step {
        STEPS[self.step]
    }
}

impl Shell {
    /// Abre o onboarding se a config ainda não concluiu esta versão.
    pub fn start_onboarding(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.config.onboarding >= ONBOARDING_ATUAL {
            return;
        }
        let upgraded =
            self.config.welcome_done || self.token.is_some() || !self.store.clips.is_empty();
        self.onboarding = Some(Onboarding {
            step: 0,
            upgraded,
            capture: None,
            key_error: None,
            presses: [0; 2],
            base: None,
            saw: [false; 2],
            encoders: None,
            gpu: None,
            chose_preset: false,
            mic: None,
            invite: None,
            busy: false,
            error: None,
            confirm_leave: false,
            starting_recorder: false,
            tick: 0,
        });
        // Teste dos encoders e nome da GPU em segundo plano (centenas de ms por encoder).
        cx.spawn(async move |this, cx| {
            let (enc, gpu) = cx
                .background_executor()
                .spawn(async move { (app_media::video::available_encoders(), gpu_name()) })
                .await;
            let _ = this.update(cx, |t, cx| t.on_encoders(enc, gpu, cx));
        })
        .detach();
        cx.spawn_in(window, async move |this, cx| loop {
            cx.background_executor().timer(TICK).await;
            let poll = this.update(cx, |t, cx| {
                let o = t.onboarding.as_mut()?;
                o.tick = o.tick.wrapping_add(1);
                if o.step() == Step::Quality && o.mic.is_some() {
                    cx.notify();
                }
                Some(o.step() == Step::Keys && o.tick % 2 == 0)
            });
            match poll {
                Ok(Some(true)) => {
                    let st = cx
                        .background_executor()
                        .spawn(async { recorder::status() })
                        .await;
                    if this.update(cx, |t, cx| t.on_status(st, cx)).is_err() {
                        break;
                    }
                }
                Ok(Some(false)) => {}
                _ => break,
            }
        })
        .detach();
    }

    fn on_encoders(&mut self, enc: Vec<&'static str>, gpu: String, cx: &mut Context<Self>) {
        tracing::info!(?enc, gpu = %gpu, "encoders desta GPU");
        let rec = recommended(&enc);
        let hevc = has_hevc(Some(&enc));
        let apply = match self.onboarding.as_mut() {
            Some(o) => {
                o.gpu = (!gpu.is_empty()).then_some(gpu);
                let apply = !o.upgraded && !o.chose_preset;
                o.encoders = Some(enc);
                apply
            }
            // Pulou antes do teste terminar (instalação nova): grava o recomendado agora.
            None => std::mem::take(&mut self.pending_preset),
        };
        if apply {
            self.apply(|c| rec.apply(hevc, c), cx);
        }
        cx.notify();
    }

    /// Status novo do gravador (o laço da Shell e o do passo das teclas).
    pub(crate) fn onboarding_status(&mut self, st: Option<&RecorderStatus>) {
        let Some(o) = self.onboarding.as_mut() else {
            return;
        };
        let Some(st) = st else {
            return;
        };
        o.starting_recorder = false;
        let now = [st.save_presses, st.kill_presses];
        let base = *o.base.get_or_insert(now);
        for i in 0..2 {
            // O gravador reiniciou (contador voltou): recomeça a contar daqui.
            if now[i] < o.presses[i] {
                o.base = Some(now);
            }
            if now[i] > o.presses[i] && now[i] > base[i] {
                o.saw[i] = true;
                // Apertou a própria hotkey enquanto a caixa escutava: o Windows entrega o
                // aperto ao gravador (não à janela), então a tecla continua a mesma.
                if o.capture.map(|s| s as usize) == Some(i) {
                    o.capture = None;
                }
            }
        }
        o.presses = now;
    }

    /// Conta nova pelo passo do servidor (convite, código ou servidor criado): segue em frente.
    pub(crate) fn onboarding_connected(&mut self, cx: &mut Context<Self>) {
        if let Some(o) = self.onboarding.as_mut() {
            o.busy = false;
            o.error = None;
            o.confirm_leave = false;
            if o.step() == Step::Server {
                self.onboarding_go(1, cx);
            }
        }
    }

    fn onboarding_go(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some(o) = self.onboarding.as_mut() else {
            return;
        };
        let next = (o.step as isize + delta).clamp(0, STEPS.len() as isize - 1) as usize;
        o.step = next;
        o.capture = None;
        o.key_error = None;
        o.error = None;
        o.confirm_leave = false;
        if o.step() == Step::Keys {
            // Conta só os apertos daqui para frente.
            o.base = self
                .status
                .as_ref()
                .map(|s| [s.save_presses, s.kill_presses]);
            o.presses = o.base.unwrap_or_default();
            o.saw = [false; 2];
        }
        self.sync_mic();
        cx.notify();
    }

    /// Concluir ("Começar") ou pular: grava a versão e mostra a biblioteca.
    fn finish_onboarding(&mut self, skipped: bool, cx: &mut Context<Self>) {
        let Some(o) = self.onboarding.take() else {
            return;
        };
        // Instalação nova pulando: grava os recomendados (os valores atuais já estão salvos).
        if skipped && !o.upgraded && !o.chose_preset {
            match &o.encoders {
                Some(enc) => {
                    let (rec, hevc) = (recommended(enc), has_hevc(Some(enc)));
                    self.apply(|c| rec.apply(hevc, c), cx);
                }
                None => self.pending_preset = true,
            }
        }
        self.apply(|c| c.onboarding = ONBOARDING_ATUAL, cx);
        self.join = None;
        cx.notify();
    }

    /// Medidor do microfone ligado só no passo da qualidade, com o microfone ligado.
    fn sync_mic(&mut self) {
        let want = self.config.audio.microphone;
        let Some(o) = self.onboarding.as_mut() else {
            return;
        };
        let on = o.step() == Step::Quality && want;
        if !on {
            o.mic = None;
            return;
        }
        if o.mic.is_some() {
            return;
        }
        let meter = MicMeter {
            stop: Arc::new(AtomicBool::new(false)),
            level: Arc::new(AtomicU32::new(0)),
        };
        #[cfg(windows)]
        {
            use app_platform_win::audio::{spawn_capture, AudioSource};
            let (stop, level) = (meter.stop.clone(), meter.level.clone());
            // A abertura do dispositivo espera a thread da captura: fora da thread da UI.
            std::thread::spawn(move || {
                let r = spawn_capture(AudioSource::Microphone, stop, move |samples, _| {
                    let peak = samples.iter().fold(0f32, |m, s| m.max(s.abs()));
                    let old = f32::from_bits(level.load(Ordering::Relaxed));
                    level.store(peak.max(old * 0.9).to_bits(), Ordering::Relaxed);
                });
                if let Err(e) = r {
                    tracing::info!("medidor do microfone: {e}");
                }
            });
        }
        o.mic = Some(meter);
    }

    /// Tecla apertada com a caixa escutando. `true` = consumiu o aperto.
    pub(crate) fn onboarding_key(&mut self, e: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        let Some(slot) = self.onboarding.as_ref().and_then(|o| o.capture) else {
            return false;
        };
        if e.keystroke.key == "escape" {
            if let Some(o) = self.onboarding.as_mut() {
                o.capture = None;
            }
            cx.notify();
            return true;
        }
        let spec = match hotkey_spec(&e.keystroke) {
            Ok(Some(s)) => s,
            // Só modificador: espera a tecla de verdade.
            Ok(None) => return true,
            Err(key) => {
                if let Some(o) = self.onboarding.as_mut() {
                    o.key_error = Some(tr(key));
                }
                cx.notify();
                return true;
            }
        };
        let other = match slot {
            Slot::Save => &self.config.kill.hotkey,
            Slot::Kill => &self.config.save_hotkey,
        };
        if same_hotkey(&spec, other) {
            if let Some(o) = self.onboarding.as_mut() {
                o.key_error = Some(tr("onboarding.keys_same"));
            }
            cx.notify();
            return true;
        }
        if let Some(o) = self.onboarding.as_mut() {
            o.capture = None;
            o.key_error = None;
            o.saw[slot as usize] = false;
        }
        match slot {
            Slot::Save => self.apply(|c| c.save_hotkey = spec, cx),
            Slot::Kill => self.apply(|c| c.kill.hotkey = spec, cx),
        }
        true
    }

    fn ensure_invite_field(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.onboarding.as_ref().is_none_or(|o| o.invite.is_some()) {
            return;
        }
        let s = cx.new(|cx| InputState::new(window, cx).placeholder(tr("join.invite_placeholder")));
        cx.subscribe(&s, |this, _, ev: &InputEvent, cx| {
            if matches!(ev, InputEvent::PressEnter { .. }) {
                this.onboarding_join(cx);
            }
        })
        .detach();
        if let Some(o) = self.onboarding.as_mut() {
            o.invite = Some(s);
        }
    }

    /// "Entrar" com o link colado no passo (nome da conta = usuário do Windows).
    fn onboarding_join(&mut self, cx: &mut Context<Self>) {
        let Some(o) = self.onboarding.as_ref().filter(|o| !o.busy) else {
            return;
        };
        let Some(field) = o.invite.as_ref() else {
            return;
        };
        let text = field.read(cx).value().trim().to_string();
        let set_error = |t: &mut Shell, e: SharedString, cx: &mut Context<Shell>| {
            if let Some(o) = t.onboarding.as_mut() {
                o.busy = false;
                o.error = Some(e);
            }
            cx.notify();
        };
        let Some(inv) = self.parse_invite_text(&text) else {
            return set_error(self, tr("join.invalid_link"), cx);
        };
        let mut name = os_user_name();
        if name.trim().is_empty() {
            name = app_core::brand::APP_NAME.to_string();
        }
        if let Some(o) = self.onboarding.as_mut() {
            o.busy = true;
            o.error = None;
        }
        cx.notify();
        self.join_with(
            inv.server,
            Ask::Join {
                code: inv.code,
                name,
            },
            move |t, err, cx| {
                if let Some(e) = err {
                    set_error(t, e, cx);
                }
            },
            cx,
        );
    }

    fn choose_preset(&mut self, p: Preset, cx: &mut Context<Self>) {
        let hevc = has_hevc(self.onboarding.as_ref().and_then(|o| o.encoders.as_deref()));
        if let Some(o) = self.onboarding.as_mut() {
            o.chose_preset = true;
        }
        self.apply(|c| p.apply(hevc, c), cx);
    }

    // ---------- desenho ----------

    pub fn onboarding_screen(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        self.ensure_invite_field(window, cx);
        self.sync_mic();
        let Some(o) = self.onboarding.as_ref() else {
            return div().into_any_element();
        };
        let step = o.step();
        let first = o.step == 0;
        let last = step == Step::Done;
        let body = match step {
            Step::Welcome => self.ob_welcome(),
            Step::Keys => self.ob_keys(window, cx),
            Step::Quality => self.ob_quality(cx),
            Step::Server => self.ob_server(window, cx),
            Step::System => self.ob_system(cx),
            Step::Done => self.ob_done(),
        };

        let footer = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.))
            .flex_none()
            .pt(px(20.))
            .when(!last, |d| {
                d.child(
                    Btn::new("ob-skip", BtnKind::Ghost, tr("onboarding.skip"))
                        .on_click(cx.listener(|this, _, _, cx| this.finish_onboarding(true, cx))),
                )
            })
            .child(div().flex_1())
            .when(!first, |d| {
                d.child(
                    Btn::new("ob-back", BtnKind::Secondary, tr("onboarding.back"))
                        .size(Size::Lg)
                        .on_click(cx.listener(|this, _, _, cx| this.onboarding_go(-1, cx))),
                )
            })
            .child(if last {
                Btn::new("ob-finish", BtnKind::Primary, tr("onboarding.finish"))
                    .size(Size::Lg)
                    .on_click(cx.listener(|this, _, _, cx| this.finish_onboarding(false, cx)))
            } else {
                Btn::new("ob-next", BtnKind::Primary, tr("onboarding.next"))
                    .size(Size::Lg)
                    .on_click(cx.listener(|this, _, _, cx| this.onboarding_go(1, cx)))
            });

        let stage = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .h_full()
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .flex_none()
                    .pr(px(6.))
                    .pt(px(6.))
                    .child(drag_area("ob-drag", div().flex_1().h(px(40.))))
                    .child(WindowControls::new(theme::pal())),
            )
            .child(
                div()
                    .id("ob-stage")
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(px(56.))
                    .pt(px(12.))
                    .child(div().max_w(px(680.)).child(body)),
            )
            .child(div().px(px(56.)).pb(px(28.)).child(footer));

        div()
            .flex()
            .flex_row()
            .flex_1()
            .h_full()
            .gap(px(6.))
            .child(self.ob_rail())
            .child(
                glass::glass_panel(R_PANEL)
                    .flex()
                    .flex_1()
                    .h_full()
                    .child(stage),
            )
            .into_any_element()
    }

    /// Trilha dos passos à esquerda (é uma sequência de verdade: numerada).
    fn ob_rail(&self) -> impl IntoElement {
        let cur = self.onboarding.as_ref().map_or(0, |o| o.step);
        glass::glass_panel(R_PANEL)
            .flex()
            .flex_col()
            .flex_none()
            .w(px(232.))
            .h_full()
            .pt(px(16.))
            .px(px(14.))
            .pb(px(20.))
            .child(drag_area(
                "ob-brand",
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(10.))
                    .px(px(4.))
                    .pb(px(36.))
                    .child(img("brand/glint.svg").size(px(28.)).flex_none())
                    .child(
                        div()
                            .text_size(px(20.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(app_core::brand::APP_NAME.to_lowercase()),
                    ),
            ))
            .children(STEPS.iter().enumerate().map(|(i, s)| {
                let (done, now) = (i < cur, i == cur);
                let dot = div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(26.))
                    .rounded_full()
                    .font_family(FONT_NUM)
                    .text_size(px(12.))
                    .map(|d| {
                        if now {
                            d.bg(tok::fg()).text_color(tok::bg())
                        } else if done {
                            d.bg(tok::surf_2())
                        } else {
                            d.border_1()
                                .border_color(tok::line_2())
                                .text_color(tok::muted())
                        }
                    })
                    .map(|d| {
                        if done {
                            d.child(svg().path(icon::CHECK).size(px(13.)).text_color(tok::fg()))
                        } else {
                            d.child((i + 1).to_string())
                        }
                    });
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(12.))
                    .px(px(4.))
                    .py(px(9.))
                    .child(dot)
                    .child(
                        div()
                            .text_size(px(14.))
                            .when(now, |d| d.font_weight(FontWeight::SEMIBOLD))
                            .text_color(if now || done { tok::fg() } else { tok::muted() })
                            .child(tr(s.key())),
                    )
            }))
            .child(div().flex_1())
            .child(hint(tr("onboarding.rail_hint")))
    }

    fn ob_welcome(&self) -> AnyElement {
        let upgraded = self.onboarding.as_ref().is_some_and(|o| o.upgraded);
        let name = app_core::brand::APP_NAME;
        let secs = self.config.buffer_secs.to_string();
        let mut col = div()
            .flex()
            .flex_col()
            .gap(px(20.))
            .pt(px(40.))
            .child(img("brand/glint.svg").size(px(64.)));
        if upgraded {
            col = col
                .child(headline(tr_args(
                    "onboarding.welcome_back_title",
                    &[("name", name)],
                )))
                .child(lead(tr("onboarding.welcome_back_body")))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(14.))
                        .pt(px(8.))
                        .child(news(
                            tr("onboarding.news_invite"),
                            tr("onboarding.news_invite_body"),
                        ))
                        .child(news(
                            tr("onboarding.news_create"),
                            tr("onboarding.news_create_body"),
                        ))
                        .child(news(
                            tr("onboarding.news_bug"),
                            tr("onboarding.news_bug_body"),
                        )),
                );
        } else {
            col = col
                .child(headline(tr("onboarding.welcome_title")))
                .child(lead(tr_args(
                    "onboarding.welcome_body",
                    &[("name", name), ("secs", &secs)],
                )));
        }
        col.into_any_element()
    }

    fn ob_keys(&self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let o = self.onboarding.as_ref().expect("onboarding aberto");
        let st = self.status.as_ref();
        let off = st.is_none();
        let keys = [
            self.config.save_hotkey.clone(),
            self.config.kill.hotkey.clone(),
        ];
        let busy = |i: usize| {
            st.is_some_and(|s| {
                let (k, ok) = if i == 0 {
                    (&s.hotkey, s.hotkey_ok)
                } else {
                    (&s.kill_hotkey, s.kill_hotkey_ok)
                };
                !ok && same_hotkey(k, &keys[i])
            })
        };
        let column = |i: usize, slot: Slot, cx: &mut Context<Self>| {
            let key = keys[i].clone();
            let listening = o.capture == Some(slot);
            let caption = tr(if i == 0 {
                "onboarding.key_save"
            } else {
                "onboarding.key_kill"
            });
            let detail = if i == 0 {
                tr_args(
                    "onboarding.key_save_hint",
                    &[("secs", &self.config.buffer_secs.to_string())],
                )
            } else {
                tr_args(
                    "onboarding.key_kill_hint",
                    &[
                        ("pre", &self.config.kill.pre_secs.to_string()),
                        ("post", &self.config.kill.post_secs.to_string()),
                    ],
                )
            };
            let status: Div = if listening {
                line(tr("onboarding.key_listening"), tok::blue())
            } else if busy(i) {
                line(tr_args("onboarding.key_busy", &[("key", &key)]), tok::red())
            } else if off {
                line(tr("onboarding.key_test_off"), tok::muted())
            } else if o.saw[i] {
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.))
                    .child(svg().path(icon::CHECK).size(px(14.)).text_color(tok::fg()))
                    .child(line(tr("onboarding.key_ok"), tok::fg()))
            } else {
                line(tr_args("onboarding.key_try", &[("key", &key)]), tok::fg_2())
            };
            div()
                .flex()
                .flex_col()
                .gap(px(14.))
                .w(px(240.))
                .child(
                    KeyCap::new(("ob-key", i), key)
                        .listening(listening)
                        .presses(
                            o.presses[i].saturating_sub(o.base.map_or(0, |b| b[i])),
                            o.saw[i],
                        )
                        .on_click(cx.listener(move |this, _, window, cx| {
                            window.focus(&this.focus, cx);
                            if let Some(o) = this.onboarding.as_mut() {
                                o.capture = if o.capture == Some(slot) {
                                    None
                                } else {
                                    Some(slot)
                                };
                                o.key_error = None;
                            }
                            cx.notify();
                        })),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(3.))
                        .child(
                            div()
                                .text_size(px(15.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(caption),
                        )
                        .child(hint(detail)),
                )
                .child(status)
        };
        div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .pt(px(28.))
            .child(headline(tr("onboarding.keys_title")))
            .child(lead(tr("onboarding.keys_body")))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(32.))
                    .pt(px(22.))
                    .child(column(0, Slot::Save, cx))
                    .child(column(1, Slot::Kill, cx)),
            )
            .when_some(o.key_error.clone(), |d, e| d.child(line(e, tok::red())))
            .when(off, |d| {
                d.child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(12.))
                        .pt(px(10.))
                        .child(
                            Btn::new(
                                "ob-start-rec",
                                BtnKind::Secondary,
                                tr(if o.starting_recorder {
                                    "recorder.starting"
                                } else {
                                    "recorder.start"
                                }),
                            )
                            .disabled(o.starting_recorder)
                            .on_click(cx.listener(|this, _, _, cx| {
                                match recorder::start() {
                                    Ok(()) => {
                                        if let Some(o) = this.onboarding.as_mut() {
                                            o.starting_recorder = true;
                                        }
                                    }
                                    Err(e) => this.notify_user(
                                        tr_args("recorder.start_error", &[("error", &e)]),
                                        cx,
                                    ),
                                }
                                cx.notify();
                            })),
                        )
                        .child(hint(tr("onboarding.keys_rec_off"))),
                )
            })
            .into_any_element()
    }

    fn ob_quality(&self, cx: &mut Context<Self>) -> AnyElement {
        let o = self.onboarding.as_ref().expect("onboarding aberto");
        let enc = o.encoders.as_deref();
        let hevc = has_hevc(enc);
        let current = Preset::of(&self.config);
        let rec = enc.map(recommended);
        let found: SharedString = match enc {
            None => tr("onboarding.gpu_testing"),
            Some(e) if e.iter().any(|n| n.starts_with("hevc")) => tr("onboarding.gpu_hevc"),
            Some(e) if !e.is_empty() => tr("onboarding.gpu_h264"),
            Some(_) => tr("onboarding.gpu_none"),
        };
        let gpu = o.gpu.clone().unwrap_or_default();
        let tier = |p: Preset, cx: &mut Context<Self>| {
            let (codec, res, fps, kbps) = p.values(hevc);
            let selected = current == Some(p);
            div()
                .id(("ob-preset", p as usize))
                .flex()
                .flex_col()
                .gap(px(8.))
                .flex_1()
                .min_w_0()
                .p(px(16.))
                .rounded(R_SM)
                .bg(if selected { tok::surf_2() } else { tok::surf() })
                .border_1()
                .border_color(if selected { tok::blue() } else { tok::edge() })
                .cursor_pointer()
                .hover(|s| s.bg(tok::surf_2()))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(8.))
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(16.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(p.name()),
                        )
                        .when(rec == Some(p), |d| {
                            d.child(
                                div()
                                    .text_size(px(11.))
                                    .px(px(8.))
                                    .py(px(2.))
                                    .rounded_full()
                                    .bg(tok::field())
                                    .text_color(tok::fg_2())
                                    .child(tr("onboarding.recommended")),
                            )
                        }),
                )
                .child(hint(p.hint()))
                .child(
                    div()
                        .pt(px(4.))
                        .font_family(FONT_NUM)
                        .text_size(px(12.))
                        .text_color(tok::fg_2())
                        .child(spec_text(codec, res, fps, kbps)),
                )
                .on_click(cx.listener(move |this, _, _, cx| this.choose_preset(p, cx)))
        };
        let v = &self.config.video;
        let mic_on = self.config.audio.microphone;
        let level = o.mic.as_ref().map_or(0.0, MicMeter::level);
        div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .pt(px(28.))
            .child(headline(tr("onboarding.quality_title")))
            .child(lead(if gpu.is_empty() {
                found.clone()
            } else {
                tr_args("onboarding.gpu", &[("gpu", &gpu), ("found", &found)])
            }))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(10.))
                    .pt(px(10.))
                    .child(tier(Preset::Light, cx))
                    .child(tier(Preset::Balanced, cx))
                    .child(tier(Preset::Max, cx)),
            )
            .when(current.is_none(), |d| {
                d.child(hint(tr_args(
                    "onboarding.custom",
                    &[(
                        "spec",
                        &spec_text(v.codec, v.resolution, v.fps, v.bitrate_kbps),
                    )],
                )))
            })
            .child(
                div()
                    .pt(px(18.))
                    .text_size(px(15.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(tr("settings.audio")),
            )
            .child(switch_row(
                "ob-sys",
                tr("onboarding.audio_game"),
                None,
                self.config.audio.system,
                cx.listener(|this, _, _, cx| this.apply(|c| c.audio.system = !c.audio.system, cx)),
            ))
            .child(switch_row(
                "ob-mic",
                tr("onboarding.audio_mic"),
                mic_on.then(|| tr("onboarding.audio_mic_hint")),
                mic_on,
                cx.listener(|this, _, _, cx| {
                    this.apply(|c| c.audio.microphone = !c.audio.microphone, cx)
                }),
            ))
            .when(mic_on, |d| d.child(vu_meter(level)))
            .into_any_element()
    }

    fn ob_server(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let o = self.onboarding.as_ref().expect("onboarding aberto");
        let url = self.config.server_url.clone();
        let host = url.split("://").nth(1).unwrap_or(&url).to_string();
        let server_name = self
            .server_info
            .as_ref()
            .map(|i| i.name.clone())
            .unwrap_or_else(|| host.clone());
        let col = div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .pt(px(28.))
            .child(headline(tr("onboarding.server_title")));
        if self.token.is_some() && self.has_server() {
            let who = match &self.account {
                Some(n) => tr_args(
                    "onboarding.server_connected_as",
                    &[("server", &server_name), ("name", n)],
                ),
                None => tr_args("onboarding.server_connected", &[("server", &server_name)]),
            };
            let card = glass::inset_box()
                .flex()
                .flex_col()
                .gap(px(12.))
                .p(px(18.))
                .child(
                    div()
                        .text_size(px(16.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(who),
                )
                .child(hint(SharedString::from(host)));
            let card = if o.confirm_leave {
                card.child(line(tr("onboarding.server_switch_confirm"), tok::fg_2()))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap(px(8.))
                            .child(
                                Btn::new("ob-leave-no", BtnKind::Secondary, tr("clip.cancel"))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if let Some(o) = this.onboarding.as_mut() {
                                            o.confirm_leave = false;
                                        }
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Btn::new("ob-leave-yes", BtnKind::Danger, tr("server.leave"))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if let Some(o) = this.onboarding.as_mut() {
                                            o.confirm_leave = false;
                                        }
                                        this.leave_server(cx);
                                    })),
                            ),
                    )
            } else {
                card.child(
                    div().flex().flex_row().child(
                        Btn::new(
                            "ob-switch",
                            BtnKind::Secondary,
                            tr("onboarding.server_switch"),
                        )
                        .size(Size::Sm)
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(o) = this.onboarding.as_mut() {
                                o.confirm_leave = true;
                            }
                            cx.notify();
                        })),
                    ),
                )
            };
            return col
                .child(lead(tr("onboarding.server_connected_body")))
                .child(card)
                .into_any_element();
        }
        let Some(field) = o.invite.as_ref() else {
            return col.into_any_element();
        };
        col.child(lead(tr("onboarding.server_body")))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.))
                    .pt(px(8.))
                    .child(div().flex_1().child(input_box(field, window, cx)))
                    .child(
                        Btn::new(
                            "ob-join",
                            BtnKind::Primary,
                            tr(if o.busy { "join.joining" } else { "join.ok" }),
                        )
                        .disabled(o.busy)
                        .on_click(cx.listener(|this, _, _, cx| this.onboarding_join(cx))),
                    ),
            )
            .child(hint(tr_args(
                "onboarding.server_name_hint",
                &[("name", &os_user_name())],
            )))
            .when_some(o.error.clone(), |d, e| d.child(line(e, tok::red())))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .gap(px(8.))
                    .pt(px(18.))
                    .child(
                        Btn::new("ob-code", BtnKind::Secondary, tr("onboarding.server_code"))
                            .disabled(o.busy)
                            .on_click(cx.listener(|this, _, _, cx| this.open_join_code(cx))),
                    )
                    .child(
                        Btn::new("ob-create", BtnKind::Secondary, tr("server.create_button"))
                            .disabled(o.busy)
                            .on_click(cx.listener(|this, _, _, cx| this.open_create(cx))),
                    )
                    .child(
                        Btn::new("ob-later", BtnKind::Ghost, tr("onboarding.server_later"))
                            .disabled(o.busy)
                            .on_click(cx.listener(|this, _, _, cx| this.onboarding_go(1, cx))),
                    ),
            )
            .into_any_element()
    }

    fn ob_system(&self, cx: &mut Context<Self>) -> AnyElement {
        let c = &self.config;
        let name = app_core::brand::APP_NAME;
        div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .pt(px(28.))
            .child(headline(tr("onboarding.system_title")))
            .child(lead(tr("onboarding.system_body")))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .pt(px(10.))
                    .child(switch_row(
                        "ob-autostart",
                        tr("settings.start_with_os"),
                        Some(tr("onboarding.autostart_hint")),
                        c.start_with_os,
                        cx.listener(|this, _, _, cx| {
                            this.apply(|c| c.start_with_os = !c.start_with_os, cx)
                        }),
                    ))
                    .child(switch_row(
                        "ob-presence",
                        tr("settings.discord_presence"),
                        Some(tr_args("settings.discord_presence_hint", &[("name", name)])),
                        c.discord_presence,
                        cx.listener(|this, _, _, cx| {
                            this.apply(|c| c.discord_presence = !c.discord_presence, cx)
                        }),
                    ))
                    .child(switch_row(
                        "ob-updates",
                        tr("onboarding.updates"),
                        Some(tr("onboarding.updates_hint")),
                        c.check_updates,
                        cx.listener(|this, _, _, cx| {
                            this.apply(|c| c.check_updates = !c.check_updates, cx)
                        }),
                    )),
            )
            .into_any_element()
    }

    fn ob_done(&self) -> AnyElement {
        let c = &self.config;
        let v = &c.video;
        let quality = match Preset::of(c) {
            Some(p) => format!(
                "{}: {}",
                p.name(),
                spec_text(v.codec, v.resolution, v.fps, v.bitrate_kbps)
            ),
            None => format!(
                "{}: {}",
                tr("onboarding.custom_name"),
                spec_text(v.codec, v.resolution, v.fps, v.bitrate_kbps)
            ),
        };
        let server: SharedString = if self.token.is_some() && self.has_server() {
            self.server_info
                .as_ref()
                .map(|i| i.name.clone())
                .unwrap_or_else(|| c.server_url.clone())
                .into()
        } else {
            tr("onboarding.summary_no_server")
        };
        let item = |label: SharedString, value: SharedString| {
            div()
                .flex()
                .flex_row()
                .gap(px(16.))
                .py(px(12.))
                .border_b_1()
                .border_color(tok::line())
                .child(
                    div()
                        .w(px(140.))
                        .flex_none()
                        .text_size(px(14.))
                        .text_color(tok::fg_2())
                        .child(label),
                )
                .child(div().flex_1().min_w_0().text_size(px(14.)).child(value))
        };
        div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .pt(px(28.))
            .child(headline(tr("onboarding.done_title")))
            .child(lead(tr_args(
                "onboarding.done_body",
                &[("key", &c.save_hotkey)],
            )))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .pt(px(10.))
                    .child(item(
                        tr("onboarding.summary_keys"),
                        tr_args(
                            "onboarding.summary_keys_value",
                            &[("save", &c.save_hotkey), ("kill", &c.kill.hotkey)],
                        ),
                    ))
                    .child(item(tr("onboarding.summary_quality"), quality.into()))
                    .child(item(tr("onboarding.summary_server"), server)),
            )
            .child(hint(tr("onboarding.done_hint")))
            .into_any_element()
    }
}

/// Nome da placa de vídeo (adaptador DXGI do dispositivo D3D11).
fn gpu_name() -> String {
    #[cfg(windows)]
    {
        app_platform_win::gpu::Gpu::new()
            .map(|g| g.adapter_name)
            .unwrap_or_default()
    }
    #[cfg(not(windows))]
    {
        String::new()
    }
}

/// Texto da hotkey a partir do aperto ("Ctrl+Shift+F9"). `Ok(None)` = só modificador;
/// `Err(chave)` = tecla que não serve de hotkey.
fn hotkey_spec(ks: &Keystroke) -> Result<Option<String>, &'static str> {
    let k = ks.key.to_ascii_lowercase();
    let m = &ks.modifiers;
    let key: String = match k.as_str() {
        "shift" | "control" | "alt" | "platform" | "function" | "capslock" => return Ok(None),
        _ if k.len() >= 2
            && k.starts_with('f')
            && k[1..].parse::<u32>().is_ok_and(|n| (1..=24).contains(&n)) =>
        {
            k.to_uppercase()
        }
        _ if k.len() == 1 && k.chars().all(|c| c.is_ascii_alphanumeric()) => {
            // Letra ou número sozinho atrapalharia digitar e jogar.
            if !(m.control || m.alt || m.platform) {
                return Err("onboarding.keys_need_modifier");
            }
            k.to_uppercase()
        }
        "insert" => "Insert".into(),
        "home" => "Home".into(),
        "end" => "End".into(),
        "pageup" => "PageUp".into(),
        "pagedown" => "PageDown".into(),
        "pause" => "Pause".into(),
        "scrolllock" => "ScrollLock".into(),
        "printscreen" => "PrintScreen".into(),
        _ => return Err("onboarding.keys_invalid"),
    };
    let mut parts = Vec::new();
    if m.control {
        parts.push("Ctrl");
    }
    if m.alt {
        parts.push("Alt");
    }
    if m.shift {
        parts.push("Shift");
    }
    if m.platform {
        parts.push("Win");
    }
    let mut s = parts.join("+");
    if !s.is_empty() {
        s.push('+');
    }
    s.push_str(&key);
    Ok(Some(s))
}

/// Título do passo: grande e fechado.
fn headline(text: SharedString) -> Div {
    div()
        .text_size(px(34.))
        .line_height(px(40.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(tok::fg())
        .child(text)
}

fn lead(text: SharedString) -> Div {
    div()
        .max_w(px(560.))
        .text_size(px(16.))
        .line_height(px(24.))
        .text_color(tok::fg_2())
        .child(text)
}

fn hint(text: SharedString) -> Div {
    div()
        .text_size(px(13.))
        .line_height(px(18.))
        .text_color(tok::muted())
        .child(text)
}

fn line(text: SharedString, color: Rgba) -> Div {
    div().text_size(px(14.)).text_color(color).child(text)
}

/// Novidade da 1.1 (boas-vindas de quem atualizou): marca curta e o texto.
fn news(title: SharedString, body: SharedString) -> Div {
    div()
        .flex()
        .flex_row()
        .gap(px(14.))
        .child(div().flex_none().w(px(3.)).rounded_full().bg(tok::line_2()))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(3.))
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .text_size(px(15.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title),
                )
                .child(hint(body)),
        )
}

fn switch_row(
    id: &'static str,
    label: SharedString,
    hint_text: Option<SharedString>,
    on: bool,
    click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(16.))
        .py(px(10.))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(3.))
                .flex_1()
                .min_w_0()
                .child(div().text_size(px(15.)).child(label))
                .when_some(hint_text, |d, h| d.child(hint(h))),
        )
        .child(Toggle::new(id, on).on_click(click))
}

/// Régua de VU do microfone: 24 segmentos de -60 a 0 dBFS.
fn vu_meter(level: f32) -> Div {
    const N: usize = 24;
    let db = if level > 0.0 {
        20.0 * level.log10()
    } else {
        -60.0
    };
    let lit = (((db + 60.0) / 60.0).clamp(0.0, 1.0) * N as f32).round() as usize;
    div()
        .flex()
        .flex_row()
        .gap(px(3.))
        .h(px(14.))
        .max_w(px(420.))
        .children((0..N).map(|i| {
            let on = i < lit;
            // Os últimos segmentos (perto de 0 dBFS) avisam que vai clipar.
            let color = if !on {
                tok::track()
            } else if i >= N - 3 {
                tok::red()
            } else {
                tok::blue()
            };
            div().flex_1().h_full().rounded(px(2.)).bg(color)
        }))
}
