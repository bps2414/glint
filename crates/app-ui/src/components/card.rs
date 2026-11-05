use std::{rc::Rc, sync::Arc};

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{game_mark, glass, Badge, BadgeKind, Btn, BtnKind, DurationTag, Handler, OnClick};
use crate::assets::icon;
use crate::theme::size::Size;
use crate::theme::{motion, pal, tok, FONT_UI, R};

/// Card de clipe da biblioteca: miniatura 16:9 com duração, título, jogo,
/// tamanho e data; favorito e barra de prévia no hover; "Copiar link" quando
/// o clipe já foi publicado.
#[derive(IntoElement)]
pub struct ClipCard {
    id: ElementId,
    image: Option<Arc<RenderImage>>,
    /// Cópia desfocada da miniatura (fundo dos chips).
    blur: Option<Arc<RenderImage>>,
    title: SharedString,
    game: SharedString,
    meta: SharedString,
    duration: SharedString,
    badge: Option<(BadgeKind, SharedString)>,
    /// Selo de clipe de kill ("Kill", "x2"...), no canto de baixo à esquerda.
    kill: Option<SharedString>,
    favorite: bool,
    hovered: bool,
    /// Selecionado (Ctrl+clique): anel em volta da miniatura.
    selected: bool,
    /// Clipe tocando sobre a miniatura (hover) e quanto já tocou (0..1).
    preview: Option<(AnyElement, f32)>,
    /// `Some(rótulo)` = publicado (rótulo do botão: "Copiar link" ou "Link copiado").
    link: Option<(SharedString, bool)>,
    on_click: Option<OnClick>,
    on_hover: Option<Handler<bool>>,
    on_favorite: Option<OnClick>,
    on_copy: Option<OnClick>,
}

impl ClipCard {
    pub fn new(
        id: impl Into<ElementId>,
        image: Option<Arc<RenderImage>>,
        title: impl Into<SharedString>,
        game: impl Into<SharedString>,
        meta: impl Into<SharedString>,
        duration: impl Into<SharedString>,
    ) -> Self {
        Self {
            id: id.into(),
            image,
            blur: None,
            title: title.into(),
            game: game.into(),
            meta: meta.into(),
            duration: duration.into(),
            badge: None,
            kill: None,
            favorite: false,
            hovered: false,
            selected: false,
            preview: None,
            link: None,
            on_click: None,
            on_hover: None,
            on_favorite: None,
            on_copy: None,
        }
    }

    /// Miniatura desfocada: os chips viram vidro fosco de verdade.
    pub fn blur(mut self, blur: Option<Arc<RenderImage>>) -> Self {
        self.blur = blur;
        self
    }

    pub fn badge(mut self, kind: BadgeKind, label: impl Into<SharedString>) -> Self {
        self.badge = Some((kind, label.into()));
        self
    }

    pub fn kill(mut self, label: impl Into<SharedString>) -> Self {
        self.kill = Some(label.into());
        self
    }

    pub fn favorite(mut self, favorite: bool) -> Self {
        self.favorite = favorite;
        self
    }

    /// Estado de hover vem da view (para a prévia recomeçar a cada entrada).
    pub fn hovered(mut self, hovered: bool) -> Self {
        self.hovered = hovered;
        self
    }

    /// Vídeo tocando por cima da miniatura e o progresso dele (barra embaixo).
    pub fn preview(mut self, video: impl IntoElement, progress: f32) -> Self {
        self.preview = Some((video.into_any_element(), progress));
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Clipe publicado: mostra o botão de copiar o link (`done` = acabou de copiar).
    pub fn link(mut self, label: impl Into<SharedString>, done: bool) -> Self {
        self.link = Some((label.into(), done));
        self
    }

    pub fn on_click(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Rc::new(f));
        self
    }

    pub fn on_hover(mut self, f: impl Fn(&bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_hover = Some(Rc::new(f));
        self
    }

    pub fn on_favorite(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_favorite = Some(Rc::new(f));
        self
    }

    pub fn on_copy(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_copy = Some(Rc::new(f));
        self
    }
}

impl RenderOnce for ClipCard {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let hovered = self.hovered;
        let fav = self.favorite;
        let (preview_video, progress) = match self.preview {
            Some((v, t)) => (Some(v), Some(t)),
            None => (None, None),
        };
        let area: glass::Area = Rc::default();
        let backdrop = self.blur.map(|image| glass::Backdrop {
            image,
            area: area.clone(),
        });

        let thumb = div()
            .relative()
            .w_full()
            .aspect_ratio(16. / 9.)
            .rounded(R)
            .overflow_hidden()
            .bg(tok::surf())
            .shadow(vec![glass::shadow(8., 24., pal().thumb_shadow)])
            .child(glass::measure(area))
            .when_some(self.image, |d, image| {
                d.child(
                    img(image)
                        .absolute()
                        .inset_0()
                        .size_full()
                        .rounded(R)
                        .object_fit(ObjectFit::Cover),
                )
            })
            .when_some(preview_video, |d, video| {
                d.child(div().absolute().inset_0().size_full().child(video))
            })
            // Contorno interno sutil para a miniatura não "vazar" no fundo.
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .rounded(R)
                    .when(!self.selected, |d| {
                        d.border_1().border_color(hsla(
                            0.,
                            0.,
                            1.,
                            if hovered { 0.14 } else { 0.06 },
                        ))
                    })
                    // Seleção usa o azul de foco (único uso de cor além do vermelho de sinal).
                    .when(self.selected, |d| d.border_2().border_color(tok::blue())),
            )
            .when_some(self.badge, |d, (kind, label)| {
                d.child(div().absolute().left(px(8.)).top(px(8.)).child(Badge {
                    kind,
                    label,
                    backdrop: backdrop.clone(),
                }))
            })
            .when_some(self.kill, |d, label| {
                d.child(div().absolute().left(px(8.)).bottom(px(8.)).child(Badge {
                    kind: BadgeKind::Kill,
                    label,
                    backdrop: backdrop.clone(),
                }))
            })
            .when(fav || hovered, |d| {
                let star = svg()
                    .path(if fav { icon::STAR_FILLED } else { icon::STAR })
                    .size(Size::Xs.icon())
                    .text_color(tok::on_media());
                d.child(
                    glass::chip_on(backdrop.clone())
                        .id("fav")
                        .absolute()
                        .right(px(8.))
                        .top(px(8.))
                        .w(Size::Xs.h())
                        .px_0()
                        .justify_center()
                        .cursor_pointer()
                        .hover(|s| s.bg(rgba(0x141418a6)))
                        .child(star)
                        .when_some(self.on_favorite, |d, f| {
                            d.on_click(move |e, w, cx| {
                                cx.stop_propagation();
                                f(e, w, cx)
                            })
                        }),
                )
            })
            .child(
                div()
                    .absolute()
                    .right(px(8.))
                    .bottom(px(8.))
                    .child(DurationTag(self.duration, backdrop.clone())),
            )
            .when_some(progress, |d, t| {
                d.child(
                    div()
                        .absolute()
                        .left_0()
                        .right_0()
                        .bottom_0()
                        .h(px(3.))
                        .bg(hsla(0., 0., 1., 0.15))
                        .child(
                            div()
                                .h_full()
                                .bg(tok::on_media())
                                .w(relative(t.clamp(0.0, 1.0))),
                        ),
                )
            });

        let info = div()
            .flex()
            .flex_row()
            .gap(px(10.))
            .min_w_0()
            .child(game_mark(&self.game, 22.).mt(px(1.)))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .min_w_0()
                    .flex_1()
                    .child(
                        div()
                            .font_family(FONT_UI)
                            .text_size(px(14.))
                            .line_height(px(17.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(tok::fg())
                            .truncate()
                            .child(self.title),
                    )
                    .child(
                        div()
                            .font_family(FONT_UI)
                            .text_size(px(12.))
                            .line_height(px(14.))
                            .text_color(tok::muted())
                            .truncate()
                            .child(self.meta),
                    ),
            );

        let copy = self.link.map(|(label, done)| {
            let on_copy = self.on_copy;
            Btn::new("copy", BtnKind::Secondary, label)
                .size(Size::Sm)
                .icon(if done { icon::CHECK } else { icon::COPY })
                .on_click(move |e, w, cx| {
                    cx.stop_propagation();
                    if let Some(f) = &on_copy {
                        f(e, w, cx)
                    }
                })
        });

        div()
            .id(self.id)
            .flex()
            .flex_col()
            .gap(px(9.))
            .min_w_0()
            .cursor_pointer()
            .child(thumb)
            .child(info)
            .children(copy.map(|b| div().flex().child(b)))
            .when_some(self.on_hover, |d, f| {
                d.on_hover(move |h, w, cx| f(h, w, cx))
            })
            .when_some(self.on_click, |d, f| {
                d.on_click(move |e, w, cx| f(e, w, cx))
            })
    }
}
