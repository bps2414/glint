use std::rc::Rc;

use gpui_kit::component::input::Input;
use gpui_kit::component::input::InputState;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{glass, Handler, OnClick};
use crate::assets::icon;
use crate::theme::size::Size;
use crate::theme::{motion, pal, tok, FONT_UI, R_SM};

/// Pill de filtro da biblioteca.
#[derive(IntoElement)]
pub struct Pill {
    id: ElementId,
    label: SharedString,
    on: bool,
    on_click: Option<OnClick>,
}

impl Pill {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>, on: bool) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            on,
            on_click: None,
        }
    }

    pub fn on_click(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Rc::new(f));
        self
    }
}

impl RenderOnce for Pill {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let p = pal();
        let base = div()
            .id(self.id)
            .relative()
            .flex()
            .items_center()
            .flex_none()
            .h(Size::Sm.h())
            .px(Size::Sm.pad())
            .rounded_full()
            .font_family(FONT_UI)
            .text_size(Size::Sm.text())
            .font_weight(FontWeight::MEDIUM)
            .cursor_pointer();
        if self.on {
            glass::selected(base, p).text_color(tok::fg())
        } else {
            base.text_color(tok::fg_2())
                .hover(move |s| s.bg(rgba(p.pill)).text_color(tok::fg()))
        }
        .child(self.label)
        .when_some(self.on_click, |d, f| {
            d.on_click(move |e, w, cx| f(e, w, cx))
        })
    }
}

/// Controle segmentado (ex.: visibilidade Privado / Com link / Público).
#[derive(IntoElement)]
pub struct Segmented {
    id: ElementId,
    options: Vec<SharedString>,
    selected: usize,
    disabled: bool,
    on_select: Option<Handler<usize>>,
}

impl Segmented {
    pub fn new(id: impl Into<ElementId>, options: Vec<SharedString>, selected: usize) -> Self {
        Self {
            id: id.into(),
            options,
            selected,
            disabled: false,
            on_select: None,
        }
    }

    /// Esperando o servidor: aparece apagado e não aceita clique.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn on_select(mut self, f: impl Fn(&usize, &mut Window, &mut App) + 'static) -> Self {
        self.on_select = Some(Rc::new(f));
        self
    }
}

impl RenderOnce for Segmented {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let p = pal();
        let on_select = self.on_select.filter(|_| !self.disabled);
        let pad = (Size::Md.h() - Size::Sm.h()) / 2. - px(1.);
        div()
            .id(self.id)
            .flex()
            .flex_row()
            .items_center()
            .w_full()
            .h(Size::Md.h())
            .px(pad)
            .gap(px(2.))
            .rounded_full()
            .bg(rgba(p.field))
            .border_1()
            .border_color(rgba(p.panel_border))
            .when(self.disabled, |d| d.opacity(0.6))
            .children(self.options.into_iter().enumerate().map(|(i, label)| {
                let on = i == self.selected;
                let cb = on_select.clone();
                let item = div()
                    .id(i)
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .h(Size::Sm.h())
                    .rounded_full()
                    .font_family(FONT_UI)
                    .text_size(Size::Md.text())
                    .when(cb.is_some(), |d| d.cursor_pointer());
                if on {
                    glass::selected(item, p)
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(tok::fg())
                } else {
                    item.text_color(tok::fg_2())
                        .hover(|s| s.text_color(tok::fg()))
                }
                .child(label)
                .when_some(cb, |d, f| d.on_click(move |_, w, cx| f(&i, w, cx)))
            }))
    }
}

/// Interruptor liga/desliga.
#[derive(IntoElement)]
pub struct Toggle {
    id: ElementId,
    on: bool,
    on_click: Option<OnClick>,
}

impl Toggle {
    pub fn new(id: impl Into<ElementId>, on: bool) -> Self {
        Self {
            id: id.into(),
            on,
            on_click: None,
        }
    }

    pub fn on_click(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Rc::new(f));
        self
    }
}

impl RenderOnce for Toggle {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let on = self.on;
        let (from, to) = if on { (3., 19.) } else { (19., 3.) };
        // O id muda com o estado: a animação do botão recomeça a cada troca.
        let anim_id = SharedString::from(if on { "knob-on" } else { "knob-off" });
        div()
            .id(self.id)
            .relative()
            .flex_none()
            .w(px(36.))
            .h(px(20.))
            .rounded(px(10.))
            .cursor_pointer()
            .bg(if on { tok::blue() } else { tok::line_2() })
            .child(
                div()
                    .absolute()
                    .top(px(3.))
                    .size(px(14.))
                    .rounded_full()
                    .bg(rgb(0xffffff))
                    .with_animation(
                        anim_id,
                        Animation::new(motion::FAST).with_easing(ease_in_out),
                        move |d, t| d.left(px(from + (to - from) * t)),
                    ),
            )
            .when_some(self.on_click, |d, f| {
                d.on_click(move |e, w, cx| f(e, w, cx))
            })
    }
}

/// Campo de busca da barra superior.
#[derive(IntoElement)]
pub struct SearchField {
    state: Entity<InputState>,
}

impl SearchField {
    pub fn new(state: &Entity<InputState>) -> Self {
        Self {
            state: state.clone(),
        }
    }
}

impl RenderOnce for SearchField {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let focused = self.state.read(cx).focus_handle(cx).is_focused(window);
        glass::pill(pal())
            .flex_1()
            .max_w(px(280.))
            .min_w(px(160.))
            .pr(px(4.))
            .when(focused, |d| d.border_color(tok::blue()))
            .child(
                svg()
                    .path(icon::SEARCH)
                    .size(Size::Md.icon())
                    .flex_none()
                    .text_color(tok::fg_2()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(Size::Md.text())
                    .text_color(tok::fg())
                    .child(Input::new(&self.state).appearance(false)),
            )
    }
}

/// Caixa do campo de texto (fundo de campo, borda clara, azul com foco).
pub fn input_box(state: &Entity<InputState>, window: &Window, cx: &App) -> Div {
    let focused = state.read(cx).focus_handle(cx).is_focused(window);
    div()
        .w_full()
        .h(Size::Md.h())
        .flex()
        .items_center()
        // O Input do gpui-component já tem ~10 px de respiro interno.
        .px(px(4.))
        .bg(tok::field())
        .border_1()
        .border_color(if focused { tok::blue() } else { tok::edge() })
        .rounded(R_SM)
        .text_size(px(14.))
        .text_color(tok::fg())
        .child(Input::new(state).appearance(false))
}

/// Rótulo de campo (12 px, secundário).
pub fn field_label(label: impl Into<SharedString>) -> Div {
    div()
        .font_family(FONT_UI)
        .text_size(px(12.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(tok::fg_2())
        .child(label.into())
}

/// Campo de texto com rótulo (ex.: atalho, pasta).
pub fn text_field(
    label: impl Into<SharedString>,
    state: &Entity<InputState>,
    window: &Window,
    cx: &App,
) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(field_label(label))
        .child(input_box(state, window, cx))
}
