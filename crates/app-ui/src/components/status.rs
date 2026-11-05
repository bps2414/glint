use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::glass;
use crate::assets::icon;
use crate::theme::size::Size;
use crate::theme::{motion, pal, tok, FONT_NUM, FONT_UI, R, R_SM};

/// Tecla (ex.: `F8`).
#[derive(IntoElement)]
pub struct Kbd(pub SharedString);

impl RenderOnce for Kbd {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .flex_none()
            .h(px(20.))
            .px(px(6.))
            .bg(tok::surf())
            .border_1()
            .border_color(tok::line_2())
            .rounded(px(6.))
            .font_family(FONT_NUM)
            .text_size(px(11.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(tok::fg())
            .child(self.0)
    }
}

/// Card de status do gravador na sidebar: ponto (vermelho só gravando, com halo pulsando),
/// estado e a dica da hotkey (`meta` vazio esconde). `action` entra embaixo (ex.: "Iniciar").
#[derive(IntoElement)]
pub struct RecStatus {
    pub label: SharedString,
    pub meta: SharedString,
    pub recording: bool,
    pub action: Option<AnyElement>,
}

impl RenderOnce for RecStatus {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let color = if self.recording {
            tok::red()
        } else {
            tok::muted()
        };
        let dot = div()
            .relative()
            .size(px(8.))
            .flex_none()
            .when(self.recording, |d| {
                d.child(
                    div()
                        .absolute()
                        .rounded_full()
                        .bg(tok::red())
                        .with_animation(
                            "rec-pulse",
                            Animation::new(motion::REC_PULSE).repeat(),
                            |d, t| {
                                // 0..0,6: anel cresce de 8 para 20 px e some; 0,6..1: pausa.
                                let k = (t / 0.6).min(1.0);
                                let r = 4. + 6. * k;
                                d.left(px(4. - r))
                                    .top(px(4. - r))
                                    .size(px(2. * r))
                                    .opacity(0.55 * (1.0 - k))
                            },
                        ),
                )
            })
            .child(div().absolute().inset_0().rounded_full().bg(color));

        glass::inset_box()
            .flex()
            .flex_col()
            .gap(px(8.))
            .p(px(12.))
            .rounded(R)
            .font_family(FONT_UI)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.))
                    .text_size(px(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(tok::fg())
                    .child(dot)
                    .child(div().min_w_0().truncate().child(self.label)),
            )
            .when(!self.meta.is_empty(), |d| {
                d.child(
                    div()
                        .text_size(px(12.))
                        .text_color(tok::fg_2())
                        .child(self.meta),
                )
            })
            .children(self.action)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BadgeKind {
    New,
    Published,
    /// Na fila de publicação ou enviando.
    Upload,
    /// Clipe de kill ("Kill", "x2"...): só texto.
    Kill,
}

/// Selo sobre a miniatura.
#[derive(IntoElement)]
pub struct Badge {
    pub kind: BadgeKind,
    pub label: SharedString,
    /// Miniatura desfocada atrás (vidro fosco).
    pub backdrop: Option<glass::Backdrop>,
}

impl RenderOnce for Badge {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let new = self.kind == BadgeKind::New;
        glass::chip_on(if new { None } else { self.backdrop })
            .when(new, |d| {
                d.bg(tok::on_media())
                    .border_color(tok::on_media())
                    .text_color(rgba(0x111114ff))
            })
            .when(!new && self.kind != BadgeKind::Kill, |d| {
                d.child(
                    svg()
                        .path(if self.kind == BadgeKind::Upload {
                            icon::UPLOAD
                        } else {
                            icon::LINK
                        })
                        .size(Size::Xs.icon())
                        .text_color(tok::on_media()),
                )
            })
            .child(self.label)
    }
}

/// Duração no canto da miniatura.
#[derive(IntoElement)]
pub struct DurationTag(pub SharedString, pub Option<glass::Backdrop>);

impl RenderOnce for DurationTag {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        glass::chip_on(self.1)
            .font_family(FONT_NUM)
            .font_weight(FontWeight::MEDIUM)
            .child(self.0)
    }
}

/// Separador de dia na biblioteca ("Hoje ———").
#[derive(IntoElement)]
pub struct DayDivider(pub SharedString);

impl RenderOnce for DayDivider {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(10.))
            .mt(px(6.))
            .font_family(FONT_UI)
            .text_size(px(12.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(tok::muted())
            .child(self.0)
            .child(div().flex_1().h(px(1.)).bg(tok::line()))
    }
}

/// Avatar da conta (inicial sobre gradiente).
#[derive(IntoElement)]
pub struct Avatar(pub SharedString);

impl RenderOnce for Avatar {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        div()
            .flex_none()
            .size(px(28.))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(tok::surf())
            .border_1()
            .border_color(tok::edge())
            .font_family(FONT_UI)
            .text_size(px(12.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(tok::fg())
            .child(self.0)
    }
}

/// Caixa de "link copiado" depois de publicar.
#[derive(IntoElement)]
pub struct LinkDone {
    pub title: SharedString,
    pub url: SharedString,
    pub copy: SharedString,
    pub hint: SharedString,
}

impl RenderOnce for LinkDone {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let p = pal();
        glass::inset_box()
            .flex()
            .flex_col()
            .gap(px(8.))
            .p(px(12.))
            .font_family(FONT_UI)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(7.))
                    .text_size(px(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(tok::fg())
                    .child(svg().path(icon::CHECK).size(px(14.)).text_color(tok::fg()))
                    .child(self.title),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .flex_1()
                            .min_w_0()
                            .h(Size::Sm.h())
                            .px(px(10.))
                            .bg(tok::surf())
                            .border_1()
                            .border_color(tok::line())
                            .rounded(R_SM)
                            .font_family(FONT_NUM)
                            .text_size(px(12.))
                            .text_color(tok::fg_2())
                            .child(div().truncate().child(self.url)),
                    )
                    .child(
                        glass::pill_surface(
                            div()
                                .id("copy-link")
                                .relative()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(6.))
                                .h(Size::Sm.h())
                                .px(Size::Sm.pad())
                                .rounded_full(),
                            p,
                        )
                        .text_size(Size::Sm.text())
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(tok::fg())
                        .cursor_pointer()
                        .hover(move |s| s.bg(rgba(p.sel)))
                        .child(
                            svg()
                                .path(icon::COPY)
                                .size(Size::Sm.icon())
                                .text_color(tok::fg()),
                        )
                        .child(self.copy),
                    ),
            )
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(tok::fg_2())
                    .child(self.hint),
            )
    }
}
