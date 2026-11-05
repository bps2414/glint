//! Barra de título própria: botões minimizar/maximizar/fechar e área de arrasto.
//!
//! No Windows os botões são áreas de controle da janela (`WindowControlArea`): o sistema
//! trata o clique e mantém o Snap do Windows 11 (passar o mouse no maximizar). Nas outras
//! plataformas o clique chama a ação direto.

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::assets::icon;
use crate::theme::glass::Palette;

/// Os três botões da janela, redondos (32 px), na cor secundária do texto.
#[derive(IntoElement)]
pub struct WindowControls {
    pal: Palette,
}

impl WindowControls {
    pub fn new(pal: Palette) -> Self {
        Self { pal }
    }
}

#[derive(Clone, Copy)]
enum Ctl {
    Min,
    Max,
    Close,
}

fn button(pal: Palette, ctl: Ctl, maximized: bool) -> impl IntoElement {
    let (id, path, area) = match ctl {
        Ctl::Min => ("win-min", icon::WIN_MIN, WindowControlArea::Min),
        Ctl::Max if maximized => ("win-max", icon::WIN_RESTORE, WindowControlArea::Max),
        Ctl::Max => ("win-max", icon::WIN_MAX, WindowControlArea::Max),
        Ctl::Close => ("win-close", icon::WIN_CLOSE, WindowControlArea::Close),
    };
    div()
        .id(id)
        .size(px(32.))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded_full()
        .text_color(rgba(pal.text2))
        .hover(|s| s.bg(rgba(pal.pill_hi)).text_color(rgba(pal.text)))
        .active(|s| s.bg(rgba(pal.sel_hi)))
        .when(cfg!(windows), |d| d.window_control_area(area))
        .when(!cfg!(windows), |d| {
            d.on_click(move |_, window, cx| {
                cx.stop_propagation();
                match ctl {
                    Ctl::Min => window.minimize_window(),
                    Ctl::Max => window.zoom_window(),
                    Ctl::Close => window.remove_window(),
                }
            })
        })
        .child(svg().path(path).size(px(12.)).text_color(rgba(pal.text2)))
}

impl RenderOnce for WindowControls {
    fn render(self, window: &mut Window, _: &mut App) -> impl IntoElement {
        let max = window.is_maximized();
        div()
            .flex()
            .flex_none()
            .gap(px(4.))
            .child(button(self.pal, Ctl::Min, max))
            .child(button(self.pal, Ctl::Max, max))
            .child(button(self.pal, Ctl::Close, max))
    }
}

/// Faz do elemento uma área de arrasto da janela (duplo clique maximiza, como na barra do
/// sistema, no Windows). Usar só em áreas sem controles: no Windows o arrasto ganha de quem está por cima.
pub fn drag_area(id: &'static str, d: Div) -> Stateful<Div> {
    d.id(id)
        .when(cfg!(windows), |d| {
            d.window_control_area(WindowControlArea::Drag)
        })
        .when(!cfg!(windows), |d| {
            d.on_mouse_down(MouseButton::Left, |_, window, _| window.start_window_move())
        })
}
