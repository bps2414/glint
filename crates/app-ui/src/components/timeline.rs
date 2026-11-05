//! Linha do tempo do editor: régua de tempos, tira de miniaturas, recorte
//! (início/fim) com alças e o cursor do player.
//!
//! Há um único arrasto no elemento: no clique decide-se o que foi agarrado
//! ([`pick`]: a alça mais perto, senão o cursor) e o arrasto segue com isso,
//! mesmo saindo da área. O estado que precisa sobreviver entre quadros
//! (o que está agarrado, onde a faixa foi desenhada) mora em [`TimelineState`],
//! guardado pela view.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::glass::shadow;
use super::Handler;
use crate::format;
use crate::theme::{tok, FONT_NUM, R_SM};

/// O que o arrasto está movendo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grab {
    Start,
    End,
    Playhead,
}

#[derive(Default)]
pub struct TimelineState {
    grab: Cell<Option<Grab>>,
    bounds: Cell<Option<Bounds<Pixels>>>,
}

#[derive(Clone)]
struct TimelineDrag(Rc<TimelineState>);

/// "Fantasma" vazio exigido pelo arrasto do GPUI (o que se move já é desenhado aqui).
struct DragGhost;

impl Render for DragGhost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

/// Régua (rótulos de 13 px + 10 de respiro até a tira).
const RULER_H: f32 = 23.;
const STRIP_H: f32 = 56.;
/// Moldura do recorte e alça.
const FRAME: f32 = 3.;
const HANDLE_W: f32 = 16.;
/// Folga lateral para os rótulos da régua e as alças não serem cortados.
pub const PAD_X: f32 = 14.;

/// Qual alça (ou o cursor) fica com um clique em `x` (px dentro da faixa).
pub fn pick(x: f32, width: f32, start: f32, end: f32) -> Grab {
    let (sx, ex) = (start * width, end * width);
    // As alças ficam centradas na borda; a zona de cada uma pende um pouco para fora.
    let ds = (x >= sx - 12. && x <= sx + 8.).then(|| (x - sx).abs());
    let de = (x >= ex - 8. && x <= ex + 12.).then(|| (x - ex).abs());
    match (ds, de) {
        (Some(a), Some(b)) => {
            if a <= b {
                Grab::Start
            } else {
                Grab::End
            }
        }
        (Some(_), None) => Grab::Start,
        (None, Some(_)) => Grab::End,
        _ => Grab::Playhead,
    }
}

/// Passo dos rótulos e das marcas menores da régua (segundos) para `total` s.
pub fn ruler_steps(total: f32) -> (f32, f32) {
    const STEPS: &[(f32, f32)] = &[
        (1., 0.5),
        (2., 0.5),
        (5., 1.),
        (10., 2.),
        (15., 5.),
        (30., 5.),
        (60., 10.),
        (120., 30.),
        (300., 60.),
        (600., 120.),
    ];
    STEPS
        .iter()
        .copied()
        .find(|(s, _)| total / s <= 7.)
        .unwrap_or((1200., 300.))
}

#[derive(IntoElement)]
pub struct Timeline {
    id: ElementId,
    state: Rc<TimelineState>,
    total_ms: i64,
    /// Frações (0..1) de início, fim e cursor.
    start: f32,
    end: f32,
    playhead: f32,
    frames: Vec<Arc<RenderImage>>,
    on_change: Option<Handler<(Grab, f32)>>,
}

impl Timeline {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<ElementId>,
        state: Rc<TimelineState>,
        total_ms: i64,
        (start, end, playhead): (f32, f32, f32),
        frames: Vec<Arc<RenderImage>>,
    ) -> Self {
        Self {
            id: id.into(),
            state,
            total_ms,
            start,
            end,
            playhead,
            frames,
            on_change: None,
        }
    }

    /// Chamado no clique e durante o arrasto com o que foi agarrado e a fração (0..1).
    pub fn on_change(mut self, f: impl Fn(&(Grab, f32), &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(f));
        self
    }
}

/// Alça do recorte: centrada na borda da seleção (16 × 62, como no mockup).
fn handle(left: bool) -> Div {
    div()
        .absolute()
        .top(px(-FRAME))
        .bottom(px(-FRAME))
        .w(px(HANDLE_W))
        .when(left, |d| d.left(px(-HANDLE_W / 2. - FRAME / 2.)))
        .when(!left, |d| d.right(px(-HANDLE_W / 2. - FRAME / 2.)))
        .rounded(px(8.))
        .bg(tok::handle())
        .shadow(vec![shadow(4., 12., 0x0000004d)])
        .cursor(CursorStyle::ResizeLeftRight)
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .w(px(2.))
                .h(px(18.))
                .rounded(px(2.))
                .bg(tok::handle_grip()),
        )
}

/// Elemento de largura zero na fração `f`: os filhos se posicionam em volta do ponto.
fn at(f: f32) -> Div {
    div()
        .absolute()
        .top_0()
        .bottom_0()
        .w(px(0.))
        .left(relative(f))
}

impl RenderOnce for Timeline {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let (a, b, p) = (self.start, self.end, self.playhead);
        let total = (self.total_ms as f32 / 1000.0).max(0.001);
        let (step, _) = ruler_steps(total);
        let state = self.state.clone();
        let on_change = self.on_change;

        // Régua: só os rótulos, cada um centrado no seu tempo.
        let mut marks: Vec<Div> = Vec::new();
        let n = (total / step).floor() as i32;
        for i in 0..=n {
            let t = i as f32 * step;
            marks.push(
                at(t / total).child(
                    div()
                        .absolute()
                        .top(px(0.))
                        .left(px(-22.))
                        .w(px(44.))
                        .flex()
                        .justify_center()
                        .font_family(FONT_NUM)
                        .text_size(px(11.))
                        .line_height(px(13.))
                        .text_color(tok::fg_2())
                        .child(format::short(t)),
                ),
            );
        }

        let strip = div()
            .relative()
            .h(px(STRIP_H))
            .w_full()
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .flex_row()
                    .gap(px(2.))
                    .rounded(R_SM)
                    .overflow_hidden()
                    .bg(tok::field())
                    .children(self.frames.into_iter().map(|f| {
                        div()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .child(img(f).size_full().object_fit(ObjectFit::Cover))
                    })),
            )
            .child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left_0()
                    .w(relative(a))
                    .rounded_l(R_SM)
                    .bg(tok::outside()),
            )
            .child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .right_0()
                    .w(relative(1. - b))
                    .rounded_r(R_SM)
                    .bg(tok::outside()),
            )
            .child(
                div()
                    .absolute()
                    .top(px(-FRAME))
                    .bottom(px(-FRAME))
                    .left(relative(a))
                    .right(relative(1. - b))
                    .border(px(FRAME))
                    .border_color(tok::handle())
                    .rounded(R_SM)
                    .shadow(vec![shadow(6., 18., 0x00000040)])
                    .child(handle(true))
                    .child(handle(false)),
            );

        // Cursor: linha branca um pouco acima e abaixo da tira, com a cabeça redonda em cima.
        let playhead = at(p)
            .child(
                div()
                    .absolute()
                    .top(px(RULER_H - 10.))
                    .bottom(px(-6.))
                    .left(px(-1.))
                    .w(px(2.))
                    .rounded(px(2.))
                    .bg(rgb(0xffffff))
                    .shadow(vec![BoxShadow {
                        color: hsla(0., 0., 0., 0.25),
                        offset: point(px(0.), px(0.)),
                        blur_radius: px(0.),
                        spread_radius: px(1.),
                        inset: false,
                    }]),
            )
            .child(
                div()
                    .absolute()
                    .top(px(RULER_H - 12.))
                    .left(px(-6.))
                    .size(px(12.))
                    .rounded_full()
                    .bg(rgb(0xffffff))
                    .shadow(vec![shadow(2., 6., 0x00000059)]),
            );

        let st_down = state.clone();
        let cb_down = on_change.clone();
        let track = div()
            .id("timeline-track")
            .relative()
            .flex()
            .flex_col()
            .w_full()
            .cursor_pointer()
            .child(
                canvas(
                    {
                        let st = state.clone();
                        move |bounds, _, _| st.bounds.set(Some(bounds))
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
            .child(div().relative().h(px(RULER_H)).w_full().children(marks))
            .child(strip)
            .child(playhead);

        // O clique e o arrasto valem na folga lateral também (as alças ficam
        // penduradas para fora da faixa); a posição é sempre relativa à faixa.
        let frac_at = |st: &TimelineState, x: Pixels| {
            let b = st.bounds.get()?;
            let width = f32::from(b.size.width);
            (width > 0.).then(|| (f32::from(x - b.left()) / width, width))
        };
        div()
            .id(self.id)
            .w_full()
            .px(px(PAD_X))
            .child(track)
            .on_mouse_down(MouseButton::Left, move |e: &MouseDownEvent, w, cx| {
                let Some((f, width)) = frac_at(&st_down, e.position.x) else {
                    return;
                };
                let grab = pick(f * width, width, a, b);
                st_down.grab.set(Some(grab));
                if let Some(cb) = &cb_down {
                    cb(&(grab, f.clamp(0., 1.)), w, cx);
                }
            })
            .on_drag(TimelineDrag(state), |_, _, _, cx| cx.new(|_| DragGhost))
            .when_some(on_change, |d, cb| {
                d.on_drag_move::<TimelineDrag>(move |e, w, cx| {
                    let st = e.drag(cx).0.clone();
                    let (Some(grab), Some((f, _))) =
                        (st.grab.get(), frac_at(&st, e.event.position.x))
                    else {
                        return;
                    };
                    cb(&(grab, f.clamp(0., 1.)), w, cx);
                })
            })
    }
}

#[cfg(test)]
mod tests {
    use super::{pick, ruler_steps, Grab};

    #[test]
    fn clique_escolhe_alca_ou_cursor() {
        // Faixa de 1000 px, recorte de 20% a 60%.
        assert_eq!(pick(195., 1000., 0.2, 0.6), Grab::Start);
        assert_eq!(pick(605., 1000., 0.2, 0.6), Grab::End);
        assert_eq!(pick(400., 1000., 0.2, 0.6), Grab::Playhead);
        // Por dentro da seleção, longe da borda: cursor.
        assert_eq!(pick(215., 1000., 0.2, 0.6), Grab::Playhead);
        // Alças coladas: vale a mais perto.
        assert_eq!(pick(498., 1000., 0.5, 0.505), Grab::Start);
        assert_eq!(pick(507., 1000., 0.5, 0.505), Grab::End);
    }

    #[test]
    fn passos_da_regua() {
        assert_eq!(ruler_steps(30.), (5., 1.));
        assert_eq!(ruler_steps(60.), (10., 2.));
        assert_eq!(ruler_steps(5.), (1., 0.5));
        assert_eq!(ruler_steps(300.), (60., 10.));
    }
}
