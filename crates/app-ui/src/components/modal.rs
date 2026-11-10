//! Modal de confirmação (apagar, apagar do servidor): cartão de vidro sobre a janela
//! escurecida. O botão destrutivo (`BtnKind::Danger`) é o único vermelho da tela.
//!
//! O GPUI não tem `backdrop-filter`: o "desfoque" do fundo é um véu escuro forte sobre
//! o conteúdo e o cartão usa a base sólida do tema com alfa alto, para o texto ficar
//! legível em cima de qualquer coisa (vídeo, miniaturas).

use std::sync::Arc;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{glass, Btn};
use crate::theme::size::Size;
use crate::theme::{pal, tok, FONT_UI, R_PANEL};

/// Véu por cima de tudo + cartão centralizado com título, texto e dois botões (ou só o de
/// cancelar, com `ok = None`). Com `backdrop` (miniatura desfocada do clipe), o cartão é
/// vidro fosco sobre ela.
pub fn confirm(
    id: &'static str,
    title: SharedString,
    body: impl IntoElement,
    backdrop: Option<Arc<RenderImage>>,
    cancel: Btn,
    ok: Option<Btn>,
) -> impl IntoElement {
    let p = pal();
    // Base do cartão: a janela sólida do tema (o vidro aqui ficaria ilegível sobre o vídeo).
    let base = crate::theme::glass::over(p.window | 0xff, if p.dark { 0x141518 } else { 0xf2f3f6 });
    let glass_on = crate::theme::look().glass;
    div()
        .id(id)
        .absolute()
        .inset_0()
        .occlude()
        .bg(rgba(if p.dark { 0x0000008c } else { 0x14182059 }))
        .flex()
        .items_center()
        .justify_center()
        .font_family(FONT_UI)
        .child(
            glass::panel(
                crate::theme::glass::Palette {
                    panel: (base & 0xffffff00) | 0xf2,
                    ..p
                },
                R_PANEL,
            )
            .when_some(backdrop.filter(|_| glass_on), |d, img| {
                d.bg(transparent_black()).child(glass::frosted_cover(
                    img,
                    R_PANEL,
                    rgba((base & 0xffffff00) | 0xd9),
                ))
            })
            .w(px(400.))
            .flex()
            .flex_col()
            .gap(px(10.))
            .p(px(22.))
            .child(
                div()
                    .text_size(px(17.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(tok::fg())
                    .child(title),
            )
            .child(
                div()
                    .text_size(px(13.))
                    .line_height(px(19.))
                    .text_color(tok::fg_2())
                    .child(body),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .justify_end()
                    .gap(px(8.))
                    .pt(px(10.))
                    .child(cancel.size(Size::Md))
                    .when_some(ok, |d, ok| d.child(ok.size(Size::Md))),
            ),
        )
}
