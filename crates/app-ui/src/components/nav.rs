use std::rc::Rc;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{glass, OnClick};
use crate::theme::size::{NAV_ICON, NAV_ITEM};
use crate::theme::{pal, tok, FONT_NUM, FONT_UI, R_SM};

/// Marca de jogo: inicial do nome num quadrado neutro (não temos os ícones dos jogos).
pub fn game_mark(name: &str, size: f32) -> Div {
    let initial: String = name
        .chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_uppercase().collect())
        .unwrap_or_default();
    div()
        .flex_none()
        .size(px(size))
        .rounded(px((size * 0.27).round()))
        .bg(tok::surf_2())
        .border_1()
        .border_color(tok::line())
        .flex()
        .items_center()
        .justify_center()
        .font_family(FONT_UI)
        .text_size(px((size * 0.5).round()))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(tok::fg_2())
        .child(initial)
}

/// Item do menu lateral.
#[derive(IntoElement)]
pub struct NavItem {
    id: ElementId,
    icon: SharedString,
    label: SharedString,
    count: Option<u32>,
    active: bool,
    on_click: Option<OnClick>,
}

impl NavItem {
    pub fn new(
        id: impl Into<ElementId>,
        icon: impl Into<SharedString>,
        label: impl Into<SharedString>,
    ) -> Self {
        Self {
            id: id.into(),
            icon: icon.into(),
            label: label.into(),
            count: None,
            active: false,
            on_click: None,
        }
    }

    pub fn count(mut self, n: u32) -> Self {
        self.count = Some(n);
        self
    }

    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    pub fn on_click(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Rc::new(f));
        self
    }
}

impl RenderOnce for NavItem {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let fg = if self.active { tok::fg() } else { tok::fg_2() };
        div()
            .id(self.id)
            .flex()
            .flex_row()
            .items_center()
            .gap(px(10.))
            .h(NAV_ITEM)
            .px(px(10.))
            .rounded(R_SM)
            .font_family(FONT_UI)
            .text_size(px(14.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(fg)
            .cursor_pointer()
            .map(|d| {
                let p = pal();
                if self.active {
                    glass::selected(d, p)
                } else {
                    d.hover(move |s| s.bg(rgba(p.pill)).text_color(tok::fg()))
                }
            })
            .child(
                svg()
                    .path(self.icon)
                    .size(NAV_ICON)
                    .flex_none()
                    .text_color(fg),
            )
            .child(self.label)
            .when_some(self.count, |d, n| {
                d.child(
                    div()
                        .ml_auto()
                        .font_family(FONT_NUM)
                        .text_size(px(11.))
                        .text_color(tok::muted())
                        .child(n.to_string()),
                )
            })
            .when_some(self.on_click, |d, f| {
                d.on_click(move |e, w, cx| f(e, w, cx))
            })
    }
}

/// Atalho de jogo no menu lateral (filtra a biblioteca).
#[derive(IntoElement)]
pub struct GameItem {
    pub id: ElementId,
    pub name: SharedString,
    pub count: u32,
    pub active: bool,
}

impl GameItem {
    pub fn on_click(
        self,
        f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Stateful<Div> {
        self.into_div().on_click(f)
    }

    fn into_div(self) -> Stateful<Div> {
        let fg = if self.active { tok::fg() } else { tok::fg_2() };
        div()
            .id(self.id)
            .flex()
            .flex_row()
            .items_center()
            .gap(px(10.))
            .h(NAV_ITEM)
            .px(px(10.))
            .rounded(R_SM)
            .font_family(FONT_UI)
            .text_size(px(13.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(fg)
            .cursor_pointer()
            .map(|d| {
                let p = pal();
                if self.active {
                    glass::selected(d, p)
                } else {
                    d.hover(move |s| s.bg(rgba(p.pill)).text_color(tok::fg()))
                }
            })
            .child(game_mark(&self.name, 20.))
            .child(div().min_w_0().truncate().child(self.name))
            .child(
                div()
                    .ml_auto()
                    .font_family(FONT_NUM)
                    .text_size(px(11.))
                    .text_color(tok::muted())
                    .child(self.count.to_string()),
            )
    }
}

impl RenderOnce for GameItem {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        self.into_div()
    }
}

/// Uso de disco no rodapé do menu: "Disco  48,2 / 100 GB" e a barra.
#[derive(IntoElement)]
pub struct StorageMeter {
    pub label: SharedString,
    pub value: SharedString,
    pub fraction: f32,
}

impl RenderOnce for StorageMeter {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .px(px(12.))
            .pt(px(4.))
            .pb(px(10.))
            .font_family(FONT_UI)
            .text_size(px(12.))
            .text_color(tok::fg_2())
            .child(
                div()
                    .flex()
                    .flex_row()
                    .justify_between()
                    .child(self.label)
                    .child(div().font_family(FONT_NUM).child(self.value)),
            )
            .child(
                div()
                    .h(px(4.))
                    .w_full()
                    .rounded_full()
                    .bg(rgba(pal().track))
                    .child(
                        div()
                            .h_full()
                            .w(relative(self.fraction.clamp(0.0, 1.0)))
                            .rounded_full()
                            .bg(tok::fg_2()),
                    ),
            )
    }
}
