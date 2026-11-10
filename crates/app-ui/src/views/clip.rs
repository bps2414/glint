//! Tela do clipe (o "editor"): abre por cima da biblioteca com o player grande,
//! a linha do tempo com recorte, as trilhas de áudio e o painel de detalhes.
//!
//! Teclado (com o foco fora do campo de título): ← → trocam de clipe, Espaço
//! toca/pausa, Del apaga (com confirmação), F tela cheia, M mudo, Esc fecha.

use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use app_cloud::Visibility;
use app_media::{HwPreference, PcmTrack};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::app::Shell;
use crate::assets::icon;
use crate::components::window_controls::{drag_area, WindowControls};
use crate::components::{
    glass, modal, Btn, BtnKind, Grab, IconBtn, IconKind, Timeline, TimelineState, TIMELINE_PAD,
};
use crate::data;
use crate::format;
use crate::player::audio::MAX_TRACKS;
use crate::player::{AudioOut, VideoPlayer};
use crate::theme::size::{Size, INSET};
use crate::theme::{pal, tok, FONT_NUM, R_PANEL_SM};
use crate::thumbs;
use crate::trim::{Handle, Trim};
use crate::views::library::now_ms;
use crate::views::publish::Remote;
use crate::{tr, tr_args};

/// Limite do título (o do servidor).
pub const TITLE_MAX: usize = 100;
/// Miniaturas na tira da linha do tempo.
const STRIP_FRAMES: usize = 12;
/// Espera depois do último arrasto de alça para gravar o recorte no banco.
const TRIM_SAVE_DELAY: Duration = Duration::from_millis(400);
/// Em tela cheia, a cápsula do player some este tempo depois do último movimento do mouse.
const CAPSULE_HIDE: Duration = Duration::from_secs(2);
/// Parte de baixo do quadro desfocada para o vidro da cápsula.
const CAPSULE_BAND: f32 = 0.25;

pub struct ClipView {
    pub id: i64,
    pub player: Option<VideoPlayer>,
    pub error: Option<String>,
    pub trim: Trim,
    pub strip: Vec<Arc<RenderImage>>,
    pub timeline: Rc<TimelineState>,
    pub title: Entity<InputState>,
    /// Trilhas de áudio do arquivo (0 = jogo, 1 = microfone); `None` enquanto carrega.
    pub tracks: Option<usize>,
    pub confirm_delete: bool,
    pub confirm_unpublish: bool,
    /// Visibilidade no servidor (clipe publicado).
    pub remote: Remote,
    /// "Quem pode ver" escolhido antes de publicar.
    pub vis: Visibility,
    trim_gen: u64,
}

impl ClipView {
    fn ms(secs: f32) -> i64 {
        (secs * 1000.0).round() as i64
    }

    /// Recorte para o banco: `None` no lado que está no limite do clipe.
    pub fn trim_ms(&self) -> (Option<i64>, Option<i64>) {
        let t = &self.trim;
        let start = (t.start > 0.05).then(|| Self::ms(t.start));
        let end = (t.end < t.total - 0.05).then(|| Self::ms(t.end));
        (start, end)
    }
}

impl Shell {
    /// Saída de som (aberta uma vez, no primeiro clipe).
    fn audio_out(&mut self) -> Option<Rc<AudioOut>> {
        if !self.audio_tried {
            self.audio_tried = true;
            self.audio = AudioOut::open().map(Rc::new);
        }
        self.audio.clone()
    }

    /// Ids na ordem em que a biblioteca mostra (filtro, busca e ordem atuais).
    fn visible_ids(&self, cx: &App) -> Vec<i64> {
        let q = self.query(cx);
        data::visible(&self.store.clips, &self.filter, &q, self.sort)
            .into_iter()
            .map(|i| self.store.clips[i].id)
            .collect()
    }

    /// Clipe vizinho do aberto na ordem da biblioteca.
    fn neighbor(&self, delta: i32, cx: &App) -> Option<i64> {
        let id = self.clip.as_ref()?.id;
        let ids = self.visible_ids(cx);
        let i = ids.iter().position(|&x| x == id)? as i32 + delta;
        (i >= 0).then(|| ids.get(i as usize).copied()).flatten()
    }

    pub fn open_clip(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        self.save_title(cx);
        let Some(c) = self.store.index_of(id).map(|i| self.store.clips[i].clone()) else {
            return;
        };
        // Libera o anterior (decoder da GPU, som) antes de abrir o próximo.
        if let Some(mut old) = self.clip.take() {
            if let Some(p) = old.player.as_mut() {
                p.set_audio(None);
            }
        }
        let audio = self.audio_out();
        if let Some(a) = &audio {
            a.set_tracks(Vec::new());
        }

        let title = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(tr("clip.untitled"))
                .default_value(c.title.clone().unwrap_or_default())
                .validate(|s, _| s.chars().count() <= TITLE_MAX)
        });
        cx.subscribe_in(
            &title,
            window,
            |this, _, ev: &InputEvent, window, cx| match ev {
                InputEvent::Change => cx.notify(),
                InputEvent::PressEnter { .. } => {
                    this.save_title(cx);
                    window.focus(&this.focus, cx);
                }
                InputEvent::Blur => this.save_title(cx),
                _ => {}
            },
        )
        .detach();

        let (player, error) = match VideoPlayer::open(&c.path, HwPreference::Auto) {
            Ok(mut p) => {
                p.looping = false;
                (Some(p), None)
            }
            Err(e) => (None, Some(format!("{e:#}"))),
        };
        let total_ms = player
            .as_ref()
            .map(|p| p.duration_ms())
            .filter(|&d| d > 0)
            .unwrap_or(c.duration_ms)
            .max(1);
        let mut trim = Trim::new(total_ms as f32 / 1000.0);
        if let Some(s) = c.trim_start_ms {
            trim.set(Handle::Start, s as f32 / 1000.0);
        }
        if let Some(e) = c.trim_end_ms {
            trim.set(Handle::End, e as f32 / 1000.0);
        }
        trim.playhead = trim.start;
        let mut player = player;
        if let Some(p) = player.as_mut().filter(|_| trim.start > 0.0) {
            p.seek(ClipView::ms(trim.start));
        }

        self.clip = Some(ClipView {
            id,
            player,
            error,
            trim,
            strip: Vec::new(),
            timeline: Rc::default(),
            title,
            tracks: None,
            confirm_delete: false,
            confirm_unpublish: false,
            remote: Remote::Unknown,
            vis: Visibility::Unlisted,
            trim_gen: 0,
        });
        self.hovered = None;
        window.focus(&self.focus, cx);
        self.fetch_remote(cx);
        self.load_clip_media(id, c.path, audio, cx);
        cx.notify();
    }

    /// Em segundo plano: trilhas de áudio (para tocar) e a tira de miniaturas.
    fn load_clip_media(
        &mut self,
        id: i64,
        path: std::path::PathBuf,
        audio: Option<Rc<AudioOut>>,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            let pa = path.clone();
            let tracks = cx
                .background_executor()
                .spawn(async move { app_media::decode_tracks(&pa) });
            let strip = cx.background_executor().spawn(async move {
                app_media::strip(&path, STRIP_FRAMES, 192, 108).map(|frames| {
                    frames
                        .into_iter()
                        .filter_map(|f| thumbs::image_from_bgra(f.width, f.height, f.data))
                        .collect::<Vec<_>>()
                })
            });
            let tracks = tracks.await;
            if this
                .update(cx, |t, cx| t.on_tracks(id, tracks, audio, cx))
                .is_err()
            {
                return;
            }
            let strip = strip.await;
            let _ = this.update(cx, |t, cx| {
                if let Some(v) = t.clip.as_mut().filter(|v| v.id == id) {
                    match strip {
                        Ok(images) => v.strip = images,
                        Err(e) => tracing::warn!("tira de miniaturas: {e}"),
                    }
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn on_tracks(
        &mut self,
        id: i64,
        tracks: app_media::Result<Vec<PcmTrack>>,
        audio: Option<Rc<AudioOut>>,
        cx: &mut Context<Self>,
    ) {
        let on = self.track_on(id);
        let gain = if self.muted { 0.0 } else { 1.0 };
        let Some(v) = self.clip.as_mut().filter(|v| v.id == id) else {
            return;
        };
        let tracks = match tracks {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!("áudio do clipe: {e}");
                Vec::new()
            }
        };
        v.tracks = Some(tracks.len().min(MAX_TRACKS));
        if let (Some(a), Some(p), false) = (audio, v.player.as_mut(), tracks.is_empty()) {
            a.set_tracks(tracks);
            for (k, &enabled) in on.iter().enumerate() {
                a.set_enabled(k, enabled);
            }
            a.set_gain(gain);
            p.set_audio(Some(a));
        }
        cx.notify();
    }

    /// Trilhas ligadas de um clipe (padrão: todas).
    pub fn track_on(&self, id: i64) -> [bool; MAX_TRACKS] {
        self.track_on
            .get(&id)
            .copied()
            .unwrap_or([true; MAX_TRACKS])
    }

    fn toggle_track(&mut self, k: usize, cx: &mut Context<Self>) {
        let Some(id) = self.clip.as_ref().map(|v| v.id) else {
            return;
        };
        let mut on = self.track_on(id);
        on[k] = !on[k];
        self.track_on.insert(id, on);
        if let Some(a) = &self.audio {
            a.set_enabled(k, on[k]);
        }
        cx.notify();
    }

    fn toggle_mute(&mut self, cx: &mut Context<Self>) {
        self.muted = !self.muted;
        if let Some(a) = &self.audio {
            a.set_gain(if self.muted { 0.0 } else { 1.0 });
        }
        cx.notify();
    }

    fn toggle_play(&mut self, cx: &mut Context<Self>) {
        if let Some(p) = self.clip.as_mut().and_then(|v| v.player.as_mut()) {
            p.toggle();
            cx.notify();
        }
    }

    fn toggle_fullscreen(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.fullscreen = !self.fullscreen;
        if window.is_fullscreen() != self.fullscreen {
            window.toggle_fullscreen();
        }
        cx.notify();
    }

    /// Fecha o clipe aberto (grava o título, solta o player e o som).
    pub fn drop_clip(&mut self, cx: &mut Context<Self>) {
        if self.clip.is_none() {
            return;
        }
        self.save_title(cx);
        if let Some(mut v) = self.clip.take() {
            if let Some(p) = v.player.as_mut() {
                p.set_audio(None);
            }
        }
        if let Some(a) = &self.audio {
            a.set_tracks(Vec::new());
        }
        cx.notify();
    }

    pub fn close_clip(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.drop_clip(cx);
        if self.fullscreen {
            self.toggle_fullscreen(window, cx);
        }
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn step_clip(&mut self, delta: i32, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.neighbor(delta, cx) {
            self.open_clip(id, window, cx);
        }
    }

    /// Grava o título se mudou (vazio = volta a "Sem título").
    pub(crate) fn save_title(&mut self, cx: &mut Context<Self>) {
        let Some(v) = self.clip.as_ref() else { return };
        let (id, new) = (v.id, v.title.read(cx).value().trim().to_string());
        let old = self
            .store
            .index_of(id)
            .and_then(|i| self.store.clips[i].title.clone())
            .unwrap_or_default();
        if new != old {
            self.store.set_title(id, &new);
            cx.notify();
        }
    }

    fn on_timeline(&mut self, grab: Grab, frac: f32, cx: &mut Context<Self>) {
        let Some(v) = self.clip.as_mut() else { return };
        match grab {
            Grab::Start | Grab::End => {
                let h = if grab == Grab::Start {
                    Handle::Start
                } else {
                    Handle::End
                };
                v.trim.set_fraction(h, frac);
                let t = if h == Handle::Start {
                    v.trim.start
                } else {
                    v.trim.end
                };
                if let Some(p) = v.player.as_mut() {
                    // Mexendo no recorte o player para e mostra o quadro da alça.
                    if p.playing() {
                        p.toggle();
                    }
                    let ms = ClipView::ms(t);
                    p.scrub(if h == Handle::End { ms - 1 } else { ms });
                }
                self.schedule_trim_save(cx);
            }
            Grab::Playhead => {
                let t = (frac * v.trim.total).clamp(v.trim.start, v.trim.end);
                if let Some(p) = v.player.as_mut() {
                    p.scrub(ClipView::ms(t));
                }
            }
        }
        cx.notify();
    }

    fn schedule_trim_save(&mut self, cx: &mut Context<Self>) {
        let Some(v) = self.clip.as_mut() else { return };
        v.trim_gen += 1;
        let (id, gen) = (v.id, v.trim_gen);
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(TRIM_SAVE_DELAY).await;
            let _ = this.update(cx, |t, _| {
                if t.clip
                    .as_ref()
                    .is_some_and(|v| v.id == id && v.trim_gen == gen)
                {
                    t.save_trim();
                }
            });
        })
        .detach();
    }

    pub(crate) fn save_trim(&mut self) {
        let Some(v) = self.clip.as_ref() else { return };
        let (id, (s, e)) = (v.id, v.trim_ms());
        self.store.set_trim(id, s, e);
    }

    fn reset_trim(&mut self, cx: &mut Context<Self>) {
        let Some(v) = self.clip.as_mut() else { return };
        let playhead = v.trim.playhead;
        v.trim = Trim::new(v.trim.total);
        v.trim.playhead = playhead;
        v.trim_gen += 1;
        self.save_trim();
        cx.notify();
    }

    fn toggle_clip_favorite(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.clip.as_ref().map(|v| v.id) {
            self.toggle_favorite(id, cx);
        }
    }

    pub(crate) fn ask_delete(&mut self, on: bool, cx: &mut Context<Self>) {
        // Enquanto publica ou exporta, o arquivo está sendo lido.
        if on
            && self
                .clip
                .as_ref()
                .is_some_and(|v| self.publish_busy(v.id) || self.exporting(v.id))
        {
            return;
        }
        if let Some(v) = self.clip.as_mut() {
            v.confirm_delete = on;
            if on {
                if let Some(p) = v.player.as_mut().filter(|p| p.playing()) {
                    p.toggle();
                }
            }
            cx.notify();
        }
    }

    fn delete_clip(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.clip.as_ref().map(|v| v.id) else {
            return;
        };
        let next = self.neighbor(1, cx).or_else(|| self.neighbor(-1, cx));
        let thumb = self
            .store
            .index_of(id)
            .and_then(|i| self.store.clips[i].thumb_path.clone());
        // Fecha o player antes: no Windows um arquivo aberto não pode ser apagado.
        if let Some(v) = self.clip.as_mut() {
            v.confirm_delete = false;
            if let Some(p) = v.player.as_mut() {
                p.set_audio(None);
            }
            v.player = None;
        }
        if let Some(a) = &self.audio {
            a.set_tracks(Vec::new());
        }
        match self.store.delete(id) {
            Ok(()) => {
                for img in thumb.map(|t| self.thumbs.remove(&t)).unwrap_or_default() {
                    let _ = window.drop_image(img);
                }
                self.track_on.remove(&id);
                // O título do apagado não deve ser gravado de volta.
                self.clip = None;
                match next {
                    Some(n) => self.open_clip(n, window, cx),
                    None => self.close_clip(window, cx),
                }
            }
            Err(e) => {
                self.notify_user(tr_args("clip.delete_error", &[("error", &e)]), cx);
                self.open_clip(id, window, cx);
            }
        }
    }

    /// Teclas da tela do clipe. `true` = tratou.
    pub fn clip_key(
        &mut self,
        e: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(v) = self.clip.as_ref() else {
            return false;
        };
        if !self.focus.is_focused(window) {
            return false;
        }
        let key = e.keystroke.key.as_str();
        // "Liberar espaço" aberto: só Esc (fecha); nada de apagar ou navegar por baixo dele.
        if self.make_room.is_some() {
            if key == "escape" {
                self.close_make_room(cx);
            }
            return true;
        }
        if v.confirm_delete {
            match key {
                "escape" => self.ask_delete(false, cx),
                "enter" => self.delete_clip(window, cx),
                _ => return false,
            }
            return true;
        }
        if v.confirm_unpublish {
            match key {
                "escape" => self.ask_unpublish(false, cx),
                "enter" => self.unpublish(cx),
                _ => return false,
            }
            return true;
        }
        match key {
            "escape" => {
                if self.fullscreen {
                    self.toggle_fullscreen(window, cx);
                } else {
                    self.close_clip(window, cx);
                }
            }
            "left" => self.step_clip(-1, window, cx),
            "right" => self.step_clip(1, window, cx),
            "space" => self.toggle_play(cx),
            "delete" => self.ask_delete(true, cx),
            "f" => self.toggle_fullscreen(window, cx),
            "m" => self.toggle_mute(cx),
            _ => return false,
        }
        true
    }

    /// Avança o player (e repete dentro do recorte). `true` = precisa de outro quadro.
    pub fn tick_clip(&mut self, window: &mut Window) -> bool {
        let Some(v) = self.clip.as_mut() else {
            return false;
        };
        let Some(p) = v.player.as_mut() else {
            return false;
        };
        let (start, end) = (ClipView::ms(v.trim.start), ClipView::ms(v.trim.end));
        if p.playing() {
            let pos = p.position_ms();
            if pos as f64 + p.info().frame_ms() >= end as f64 || pos + 40 < start {
                p.seek(start);
            }
        }
        let animate = p.tick(window);
        v.trim.playhead = (p.position_ms() as f32 / 1000.0).clamp(0.0, v.trim.total);
        animate
    }

    // ---------- desenho ----------

    /// A tela do clipe inteira (sem a sidebar e a barra da biblioteca): topo próprio,
    /// player + linha do tempo à esquerda e o painel Publicar à direita.
    pub fn clip_screen(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if self.clip.is_none() {
            return Empty.into_any_element();
        }
        div()
            .id("clip-screen")
            .size_full()
            .flex()
            .flex_col()
            .gap(INSET)
            .child(self.clip_header(cx))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h_0()
                    .gap(INSET)
                    .child(self.clip_main(window, cx))
                    .child(self.publish_panel(window, cx)),
            )
            .into_any_element()
    }

    /// Modal de confirmação aberto na tela do clipe, por cima da janela inteira.
    pub fn clip_modal(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let v = self.clip.as_ref()?;
        if v.confirm_delete {
            Some(self.delete_modal(cx).into_any_element())
        } else if v.confirm_unpublish {
            Some(self.unpublish_modal(cx).into_any_element())
        } else {
            None
        }
    }

    /// Só o player, ocupando a janela (tela cheia).
    pub fn clip_fullscreen(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id("clip-full")
            .size_full()
            .bg(tok::black())
            .child(self.player_box(true, window, cx))
            .into_any_element()
    }

    /// Topo: voltar, título e metadados (área de arrasto), ‹ › numa pílula, favoritar,
    /// apagar e os botões da janela.
    fn clip_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let prev = self.neighbor(-1, cx).is_some();
        let next = self.neighbor(1, cx).is_some();
        let v = self.clip.as_ref().expect("clipe aberto");
        let c = self.store.index_of(v.id).map(|i| &self.store.clips[i]);
        let favorite = c.is_some_and(|c| c.favorite);
        let typed = v.title.read(cx).value().trim().to_string();
        let title = if typed.is_empty() {
            tr("clip.untitled").to_string()
        } else {
            typed
        };
        let busy = self.publish_busy(v.id) || self.exporting(v.id);

        let mut meta = div()
            .flex()
            .flex_row()
            .items_baseline()
            .min_w_0()
            .overflow_hidden()
            .whitespace_nowrap()
            .text_size(px(12.))
            .text_color(tok::fg_2());
        if let Some(c) = c {
            let game: SharedString = c
                .game
                .clone()
                .map(Into::into)
                .unwrap_or_else(|| tr("nav.screen"));
            let (day, time) = format::when(c.created_ms, now_ms());
            let mono = |t: String| div().font_family(FONT_NUM).child(t);
            meta = meta
                .child(format!("{game} · {day}, "))
                .child(mono(time))
                .child(" · ")
                .child(mono(format::short(c.duration_ms as f32 / 1000.0)))
                .child(" · ")
                .child(mono(format::size(c.size_bytes)));
        }

        let p = pal();
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(12.))
            .flex_none()
            .px(px(4.))
            .py(px(4.))
            .child(
                IconBtn::new("clip-back", icon::BACK)
                    .on_click(cx.listener(|this, _, window, cx| this.close_clip(window, cx))),
            )
            .child(drag_area(
                "clip-drag",
                div()
                    .flex()
                    .flex_col()
                    .justify_center()
                    .gap(px(1.))
                    .flex_1()
                    .min_w_0()
                    .h(Size::Md.h())
                    .child(
                        div()
                            .truncate()
                            .text_size(px(18.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(tok::fg())
                            .child(title),
                    )
                    .child(meta),
            ))
            .child(
                glass::pill_surface(
                    div()
                        .relative()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(2.))
                        .flex_none()
                        .h(Size::Md.h())
                        .px((Size::Md.h() - Size::Sm.h()) / 2. - px(1.))
                        .rounded_full(),
                    p,
                )
                .child(self.nav_button("clip-prev", icon::CHEVRON_LEFT, prev, -1, cx))
                .child(self.nav_button(
                    "clip-next",
                    icon::CHEVRON_RIGHT,
                    next,
                    1,
                    cx,
                )),
            )
            .child(
                IconBtn::new(
                    "clip-fav",
                    if favorite {
                        icon::STAR_FILLED
                    } else {
                        icon::STAR
                    },
                )
                .active(favorite)
                .on_click(cx.listener(|this, _, _, cx| this.toggle_clip_favorite(cx))),
            )
            .child(
                IconBtn::new("clip-delete", icon::TRASH)
                    .disabled(busy)
                    .on_click(cx.listener(|this, _, _, cx| this.ask_delete(true, cx))),
            )
            .child(div().ml(px(6.)).child(WindowControls::new(p)))
    }

    fn clip_main(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .h_full()
            .gap(INSET)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| window.focus(&this.focus, cx)),
            )
            .child(self.player_box(false, window, cx))
            .child(self.timeline_panel(cx))
    }

    fn nav_button(
        &self,
        id: &'static str,
        path: &'static str,
        enabled: bool,
        delta: i32,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        IconBtn::new(id, path)
            .kind(IconKind::Ghost)
            .size(Size::Sm)
            .disabled(!enabled)
            .on_click(cx.listener(move |this, _, window, cx| this.step_clip(delta, window, cx)))
    }

    /// Player com a cápsula de controles por cima do vídeo. Em tela cheia a cápsula some
    /// [`CAPSULE_HIDE`] depois do último movimento do mouse (tocando).
    fn player_box(
        &mut self,
        full: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let playing = self
            .clip
            .as_ref()
            .and_then(|v| v.player.as_ref())
            .is_some_and(|p| p.playing());
        let show = !full || !playing || self.pointer_at.elapsed() < CAPSULE_HIDE;
        // Vidro da cápsula: só calcula com ela visível.
        let backdrop = if show && crate::theme::look().glass {
            self.clip
                .as_mut()
                .and_then(|v| v.player.as_mut())
                .and_then(|p| p.backdrop(window, CAPSULE_BAND))
        } else {
            None
        };
        let v = self.clip.as_ref().expect("clipe aberto");
        let player = v.player.as_ref();
        let radius = if full { px(0.) } else { R_PANEL_SM };
        let video: AnyElement = match (player, &v.error) {
            (Some(p), _) => p.surface_rounded(radius).into_any_element(),
            (None, err) => div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .p(px(24.))
                .text_size(px(13.))
                .text_color(tok::on_media())
                .child(tr_args(
                    "clip.open_error",
                    &[("error", err.as_deref().unwrap_or("?"))],
                ))
                .into_any_element(),
        };

        div()
            .id("clip-player")
            .relative()
            .flex_1()
            .min_h_0()
            .w_full()
            .bg(tok::black())
            .when(!full, |d| {
                d.rounded(radius)
                    .overflow_hidden()
                    .shadow(vec![glass::shadow(10., 30., pal().thumb_shadow)])
            })
            .on_mouse_move(cx.listener(|this, _, _, cx| {
                let hidden = this.pointer_at.elapsed() >= CAPSULE_HIDE;
                this.pointer_at = Instant::now();
                if hidden && this.fullscreen {
                    cx.notify();
                }
            }))
            .child(
                div()
                    .id("clip-video")
                    .absolute()
                    .inset_0()
                    .cursor_pointer()
                    .on_click(cx.listener(|this, e: &ClickEvent, window, cx| {
                        if e.click_count() >= 2 {
                            this.toggle_fullscreen(window, cx);
                        } else {
                            this.toggle_play(cx);
                        }
                    }))
                    .child(video),
            )
            .when(show && player.is_some(), |d| {
                d.child(
                    div()
                        .absolute()
                        .left_0()
                        .right_0()
                        .bottom(px(20.))
                        .flex()
                        .justify_center()
                        .child(self.capsule(full, backdrop, cx)),
                )
            })
    }

    /// Cápsula de vidro do player: tocar/pausar, tempo, trilhas, mudo e tela cheia.
    fn capsule(
        &self,
        full: bool,
        backdrop: Option<glass::Backdrop>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let v = self.clip.as_ref().expect("clipe aberto");
        let player = v.player.as_ref();
        let playing = player.is_some_and(|p| p.playing());
        let pos = player.map_or(0, |p| p.position_ms());
        let total = ClipView::ms(v.trim.total);
        let on = self.track_on(v.id);
        let divider = || {
            div()
                .flex_none()
                .w(px(1.))
                .h(Size::Sm.h() - px(8.))
                .bg(tok::capsule_on())
        };
        let gap = px(6.);
        let h = Size::Md.h() + px(16.);

        let mut caps = div()
            .id("clip-capsule")
            .relative()
            .flex()
            .flex_row()
            .items_center()
            .gap(gap)
            .flex_none()
            .h(h)
            .px(px(8.))
            .rounded_full()
            .map(|d| match backdrop {
                Some(b) => d.child(glass::frosted(b, h / 2., tok::capsule())),
                None => d.bg(tok::capsule()),
            })
            .border_1()
            .border_color(tok::capsule_edge())
            .shadow(vec![glass::shadow(16., 40., 0x00000059)])
            .text_color(tok::on_media())
            // O clique na cápsula não toca/pausa o vídeo de baixo.
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(glass::highlight(0xffffff33, px(999.), 0.45))
            .child(
                IconBtn::new("clip-play", if playing { icon::PAUSE } else { icon::PLAY })
                    .kind(IconKind::MediaSolid)
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_play(cx))),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_baseline()
                    .px(px(10.))
                    .font_family(FONT_NUM)
                    .text_size(px(13.))
                    .whitespace_nowrap()
                    .child(format::player_time(pos as f32 / 1000.0))
                    .child(
                        div()
                            .opacity(0.6)
                            .child(format!(" / {}", format::player_time(total as f32 / 1000.0))),
                    ),
            );

        if let Some(n) = v.tracks.filter(|&n| n > 0) {
            caps = caps.child(divider());
            for (k, &enabled) in on.iter().enumerate().take(n) {
                let (label, ic) = match (k, enabled) {
                    (0, true) => (tr("clip.track_game"), icon::VOLUME),
                    (0, false) => (tr("clip.track_game"), icon::VOLUME_OFF),
                    (_, true) => (tr("clip.track_mic"), icon::MIC),
                    (_, false) => (tr("clip.track_mic"), icon::MIC_OFF),
                };
                caps = caps.child(
                    Btn::new(("clip-track", k as u64), BtnKind::Media, label)
                        .icon(ic)
                        .active(enabled)
                        .on_click(cx.listener(move |this, _, _, cx| this.toggle_track(k, cx))),
                );
            }
        }
        caps.child(divider())
            .child(
                IconBtn::new(
                    "clip-mute",
                    if self.muted {
                        icon::VOLUME_OFF
                    } else {
                        icon::VOLUME
                    },
                )
                .kind(IconKind::Media)
                .active(self.muted)
                .on_click(cx.listener(|this, _, _, cx| this.toggle_mute(cx))),
            )
            .child(
                IconBtn::new(
                    "clip-full",
                    if full {
                        icon::FULLSCREEN_EXIT
                    } else {
                        icon::FULLSCREEN
                    },
                )
                .kind(IconKind::Media)
                .on_click(cx.listener(|this, _, window, cx| this.toggle_fullscreen(window, cx))),
            )
    }

    /// Painel de vidro da linha do tempo: régua, tira, alças, cursor e o resumo do trecho.
    fn timeline_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let v = self.clip.as_ref().expect("clipe aberto");
        let (a, b, p) = v.trim.fractions();
        let total_ms = ClipView::ms(v.trim.total);
        let t = &v.trim;
        let trimmed = v.trim_ms() != (None, None);

        glass::glass_panel(R_PANEL_SM)
            .flex()
            .flex_col()
            .gap(px(10.))
            .flex_none()
            .px(px(16. - TIMELINE_PAD))
            .pt(px(14.))
            .pb(px(16.))
            .child(
                Timeline::new(
                    "clip-timeline",
                    v.timeline.clone(),
                    total_ms,
                    (a, b, p),
                    v.strip.clone(),
                )
                .on_change(cx.listener(
                    |this, (grab, frac): &(Grab, f32), _, cx| this.on_timeline(*grab, *frac, cx),
                )),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.))
                    .px(px(TIMELINE_PAD))
                    .min_h(Size::Sm.h())
                    .text_size(px(12.))
                    .text_color(tok::fg_2())
                    .child(tr("clip.trim_label"))
                    .child(
                        div()
                            .font_family(FONT_NUM)
                            .text_color(tok::fg())
                            .child(format!(
                                "{} – {}",
                                format::short(t.start),
                                format::short(t.end)
                            )),
                    )
                    .child("·")
                    .child(div().font_family(FONT_NUM).child(format::seconds(t.len())))
                    .when(trimmed, |d| {
                        d.child(
                            Btn::new("clip-trim-reset", BtnKind::Ghost, tr("clip.trim_reset"))
                                .size(Size::Sm)
                                .on_click(cx.listener(|this, _, _, cx| this.reset_trim(cx))),
                        )
                    })
                    .child(div().flex_1())
                    .child(div().truncate().child(tr("clip.trim_hint"))),
            )
    }

    /// Miniatura desfocada do clipe aberto (fundo do modal).
    fn clip_blur(&self) -> Option<Arc<RenderImage>> {
        let id = self.clip.as_ref()?.id;
        let c = &self.store.clips[self.store.index_of(id)?];
        self.thumbs.blur(c.thumb_path.as_deref()?)
    }

    fn delete_modal(&self, cx: &mut Context<Self>) -> impl IntoElement {
        modal::confirm(
            "clip-delete-modal",
            tr("clip.delete_title"),
            tr("clip.delete_body"),
            self.clip_blur(),
            Btn::new("del-cancel", BtnKind::Secondary, tr("clip.cancel"))
                .on_click(cx.listener(|this, _, _, cx| this.ask_delete(false, cx))),
            Some(
                Btn::new("del-ok", BtnKind::Danger, tr("clip.delete"))
                    .icon(icon::TRASH)
                    .on_click(cx.listener(|this, _, window, cx| this.delete_clip(window, cx))),
            ),
        )
    }

    fn unpublish_modal(&self, cx: &mut Context<Self>) -> impl IntoElement {
        modal::confirm(
            "clip-unpublish-modal",
            tr("publish.unpublish_title"),
            tr("publish.unpublish_body"),
            self.clip_blur(),
            Btn::new("unpub-cancel", BtnKind::Secondary, tr("clip.cancel"))
                .on_click(cx.listener(|this, _, _, cx| this.ask_unpublish(false, cx))),
            Some(
                Btn::new("unpub-ok", BtnKind::Danger, tr("publish.unpublish"))
                    .icon(icon::TRASH)
                    .on_click(cx.listener(|this, _, _, cx| this.unpublish(cx))),
            ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::ClipView;

    #[test]
    fn ms_arredonda() {
        assert_eq!(ClipView::ms(12.3456), 12_346);
        assert_eq!(ClipView::ms(0.0), 0);
    }
}
