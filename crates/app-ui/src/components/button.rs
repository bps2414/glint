//! Botões do app. Todo botão passa por aqui: [`Btn`] (pílula com texto) e [`IconBtn`]
//! (redondo, só ícone). Altura, folga, ícone e texto vêm de [`Size`]; nada de tamanho solto.
//!
//! Correspondência com o mockup: `Btn` Primary = PrimaryPill, Secondary = GlassPill,
//! `IconBtn` Glass = GlassCircle.

use std::rc::Rc;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::glass;
use super::OnClick;
use crate::theme::size::Size;
use crate::theme::{pal, tok, FONT_UI};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BtnKind {
    /// Ação principal (uma por tela): neutra (branca no escuro, grafite no claro).
    Primary,
    /// Ação comum: pílula de vidro.
    Secondary,
    /// Ação discreta: só texto, vidro no hover.
    Ghost,
    /// Confirmar algo destrutivo (apagar, despublicar). O único botão vermelho.
    Danger,
    /// Sobre vídeo (cápsula do player): texto branco, véu claro quando ligado.
    Media,
}

/// Botão com texto (pílula).
#[derive(IntoElement)]
pub struct Btn {
    id: ElementId,
    kind: BtnKind,
    size: Size,
    label: SharedString,
    icon: Option<SharedString>,
    disabled: bool,
    full_width: bool,
    /// Ligado (só `Media`: trilha de áudio ativa).
    active: bool,
    on_click: Option<OnClick>,
}

impl Btn {
    pub fn new(id: impl Into<ElementId>, kind: BtnKind, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            kind,
            size: Size::Md,
            label: label.into(),
            icon: None,
            disabled: false,
            full_width: false,
            active: false,
            on_click: None,
        }
    }

    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    pub fn icon(mut self, path: impl Into<SharedString>) -> Self {
        self.icon = Some(path.into());
        self
    }

    pub fn size(mut self, size: Size) -> Self {
        self.size = size;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn full_width(mut self) -> Self {
        self.full_width = true;
        self
    }

    pub fn on_click(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Rc::new(f));
        self
    }
}

impl RenderOnce for Btn {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let p = pal();
        let size = self.size;
        let enabled = !self.disabled;
        let fg = match self.kind {
            BtnKind::Primary => tok::primary_fg(),
            BtnKind::Secondary => tok::fg(),
            BtnKind::Ghost => tok::fg_2(),
            BtnKind::Danger | BtnKind::Media => tok::on_media(),
        };
        let strong = matches!(self.kind, BtnKind::Primary | BtnKind::Danger);

        let base = div()
            .id(self.id)
            .relative()
            .flex()
            .flex_row()
            .items_center()
            .justify_center()
            .gap(px(8.))
            .flex_none()
            .when(self.full_width, |d| d.w_full())
            .h(size.h())
            .px(size.pad())
            .rounded_full()
            .font_family(FONT_UI)
            .text_size(size.text())
            .font_weight(if strong {
                FontWeight::SEMIBOLD
            } else {
                FontWeight::MEDIUM
            })
            .whitespace_nowrap()
            .text_color(fg);

        let styled = match self.kind {
            BtnKind::Primary => base
                .bg(linear_gradient(
                    180.,
                    linear_color_stop(tok::primary_top(), 0.),
                    linear_color_stop(tok::primary_bot(), 1.),
                ))
                .shadow(vec![glass::shadow(4., 14., p.pill_shadow)])
                .when(enabled, |d| {
                    d.hover(move |s| s.shadow(vec![glass::shadow(6., 20., p.panel_shadow)]))
                        .active(|s| s.bg(tok::primary_bot()))
                }),
            BtnKind::Secondary => glass::pill_surface(base, p).when(enabled, |d| {
                d.hover(move |s| s.bg(rgba(p.sel)))
                    .active(move |s| s.bg(rgba(p.pill)))
            }),
            BtnKind::Ghost => base.when(enabled, |d| {
                d.hover(move |s| s.bg(rgba(p.pill)).text_color(tok::fg()))
                    .active(move |s| s.bg(rgba(p.sel)))
            }),
            BtnKind::Danger => base
                .bg(tok::red())
                .shadow(vec![glass::shadow(4., 14., p.red_halo)])
                .when(enabled, |d| {
                    d.hover(|s| s.opacity(0.92)).active(|s| s.opacity(0.84))
                }),
            BtnKind::Media => base
                .when(self.active, |d| d.bg(tok::capsule_on()))
                .when(!self.active, |d| d.opacity(0.72))
                .when(enabled, |d| {
                    d.hover(|s| s.bg(rgba(0xffffff26)).opacity(1.))
                        .active(|s| s.bg(tok::capsule_on()))
                }),
        };

        styled
            .when(self.disabled, |d| d.opacity(0.45))
            .when(enabled, |d| d.cursor_pointer())
            .when_some(self.icon, |d, icon| {
                d.child(
                    svg()
                        .path(icon)
                        .size(size.icon())
                        .flex_none()
                        .text_color(fg),
                )
            })
            .child(self.label)
            .when_some(self.on_click.filter(|_| enabled), |d, f| {
                d.on_click(move |e, w, cx| f(e, w, cx))
            })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum IconKind {
    /// Círculo de vidro (barra do topo: voltar, favoritar, apagar).
    Glass,
    /// Sem fundo; vidro no hover (dentro de pílulas e painéis).
    Ghost,
    /// Sobre vídeo/imagem: ícone branco, véu claro no hover.
    Media,
    /// Sobre vídeo, a ação principal (tocar/pausar): círculo branco, ícone escuro.
    MediaSolid,
}

/// Botão redondo só com ícone.
#[derive(IntoElement)]
pub struct IconBtn {
    id: ElementId,
    icon: SharedString,
    kind: IconKind,
    size: Size,
    active: bool,
    disabled: bool,
    on_click: Option<OnClick>,
}

impl IconBtn {
    pub fn new(id: impl Into<ElementId>, icon: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            icon: icon.into(),
            kind: IconKind::Glass,
            size: Size::Md,
            active: false,
            disabled: false,
            on_click: None,
        }
    }

    pub fn kind(mut self, kind: IconKind) -> Self {
        self.kind = kind;
        self
    }

    pub fn size(mut self, size: Size) -> Self {
        self.size = size;
        self
    }

    /// Ligado (ex.: favorito): fundo de selecionado.
    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn on_click(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Rc::new(f));
        self
    }
}

impl RenderOnce for IconBtn {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let p = pal();
        let enabled = !self.disabled;
        let on = self.active;
        let fg = match self.kind {
            IconKind::Glass => tok::fg(),
            IconKind::Ghost if on => tok::fg(),
            IconKind::Ghost => tok::fg_2(),
            IconKind::Media => tok::on_media(),
            IconKind::MediaSolid => rgba(0x111114ff),
        };
        let base = div()
            .id(self.id)
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(self.size.h())
            .rounded_full();

        let styled = match self.kind {
            IconKind::Glass => glass::pill_surface(base, p)
                .when(on, move |d| d.bg(rgba(p.sel)))
                .when(enabled, |d| {
                    d.hover(move |s| s.bg(rgba(p.sel)))
                        .active(move |s| s.bg(rgba(p.pill)))
                }),
            IconKind::Ghost => base
                .when(on, move |d| d.bg(rgba(p.sel)))
                .when(enabled, |d| {
                    d.hover(move |s| s.bg(rgba(p.pill_hi)))
                        .active(move |s| s.bg(rgba(p.sel)))
                }),
            IconKind::Media => base
                .when(on, |d| d.bg(rgba(0xffffff38)))
                .when(enabled, |d| {
                    d.hover(|s| s.bg(rgba(0xffffff26)))
                        .active(|s| s.bg(rgba(0xffffff38)))
                }),
            IconKind::MediaSolid => base.bg(rgba(0xffffffff)).when(enabled, |d| {
                d.hover(|s| s.bg(rgba(0xf0f1f4ff)))
                    .active(|s| s.bg(rgba(0xe4e5e9ff)))
            }),
        };

        styled
            .when(self.disabled, |d| d.opacity(0.35))
            .when(enabled, |d| d.cursor_pointer())
            .child(svg().path(self.icon).size(self.size.icon()).text_color(fg))
            .when_some(self.on_click.filter(|_| enabled), |d, f| {
                d.on_click(move |e, w, cx| f(e, w, cx))
            })
    }
}
