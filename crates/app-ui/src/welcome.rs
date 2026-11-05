//! Primeiro uso: três passos curtos por cima da biblioteca, uma vez só
//! (`Config::welcome_done`).

use gpui_kit::*;

use crate::app::Shell;
use crate::components::{glass, Btn, BtnKind};
use crate::theme::size::Size;
use crate::theme::{tok, FONT_NUM, R_PANEL};
use crate::{tr, tr_args};
use app_core::brand;

impl Shell {
    pub fn welcome_modal(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.config.welcome_done {
            return None;
        }
        let key = self.config.save_hotkey.clone();
        let secs = self.config.buffer_secs.to_string();
        let step = |n: &str, title: SharedString, body: SharedString| {
            div()
                .flex()
                .flex_row()
                .gap(px(14.))
                .child(
                    div()
                        .flex_none()
                        .size(Size::Sm.h())
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .bg(tok::field())
                        .border_1()
                        .border_color(tok::edge())
                        .font_family(FONT_NUM)
                        .text_size(px(13.))
                        .child(n.to_string()),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(3.))
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .text_size(px(14.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(title),
                        )
                        .child(
                            div()
                                .text_size(px(13.))
                                .line_height(px(19.))
                                .text_color(tok::fg_2())
                                .child(body),
                        ),
                )
        };
        Some(
            div()
                .id("welcome")
                .absolute()
                .inset_0()
                .occlude()
                .bg(rgba(0x0000008c))
                .flex()
                .items_center()
                .justify_center()
                .child(
                    glass::glass_panel(R_PANEL)
                        // Base sólida do tema: por cima da grade o vidro ficaria ilegível.
                        .bg(rgba(crate::theme::glass::solid(crate::theme::pal()).window))
                        .w(px(460.))
                        .flex()
                        .flex_col()
                        .gap(px(20.))
                        .p(px(26.))
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(12.))
                                .child(img("brand/glint.svg").size(px(40.)).flex_none())
                                .child(
                                    div()
                                        .text_size(px(22.))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(tr_args(
                                            "welcome.title",
                                            &[("name", brand::APP_NAME)],
                                        )),
                                ),
                        )
                        .child(step("1", tr("welcome.step1_title"), tr("welcome.step1")))
                        .child(step(
                            "2",
                            tr_args("welcome.step2_title", &[("key", &key)]),
                            tr_args("welcome.step2", &[("secs", &secs)]),
                        ))
                        .child(step("3", tr("welcome.step3_title"), tr("welcome.step3")))
                        .child(
                            Btn::new("welcome-ok", BtnKind::Primary, tr("welcome.start"))
                                .size(Size::Lg)
                                .full_width()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.apply(|c| c.welcome_done = true, cx)
                                })),
                        ),
                )
                .into_any_element(),
        )
    }
}
