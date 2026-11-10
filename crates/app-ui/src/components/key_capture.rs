//! Tecla física grande do onboarding: mostra a hotkey, vira "escutando" ao clicar (a view
//! captura o próximo aperto) e afunda e acende quando o gravador conta um aperto dela.

use std::time::Duration;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::OnClick;
use crate::theme::{tok, FONT_NUM};

/// Duração do brilho depois de um aperto.
const FLASH: Duration = Duration::from_millis(900);

#[derive(IntoElement)]
pub struct KeyCap {
    id: ElementId,
    label: SharedString,
    listening: bool,
    /// Apertos contados até agora: cada valor novo dispara o brilho de novo.
    presses: u64,
    /// Já apertou pelo menos uma vez no teste (fica afundada e azul).
    lit: bool,
    on_click: Option<OnClick>,
}

impl KeyCap {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            listening: false,
            presses: 0,
            lit: false,
            on_click: None,
        }
    }

    pub fn listening(mut self, on: bool) -> Self {
        self.listening = on;
        self
    }

    pub fn presses(mut self, n: u64, lit: bool) -> Self {
        self.presses = n;
        self.lit = lit;
        self
    }

    pub fn on_click(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(std::rc::Rc::new(f));
        self
    }
}

impl RenderOnce for KeyCap {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let blue = tok::blue();
        let down = self.lit;
        // Combinações longas ("Ctrl+Shift+F10") encolhem para caber na tecla.
        let n = self.label.chars().count();
        let text = if self.listening {
            px(30.)
        } else if n <= 3 {
            px(40.)
        } else if n <= 8 {
            px(24.)
        } else {
            px(16.)
        };
        // Saia da tecla (a "altura"): 8 px solta, 3 px afundada.
        let skirt = if down { 3. } else { 8. };
        let face = div()
            .flex()
            .items_center()
            .justify_center()
            .size_full()
            .rounded(px(14.))
            .bg(tok::surf_2())
            .border_1()
            .border_color(if down || self.listening {
                blue
            } else {
                tok::line_2()
            })
            .font_family(FONT_NUM)
            .font_weight(FontWeight::MEDIUM)
            .text_size(text)
            .text_color(tok::fg())
            .child(if self.listening {
                SharedString::from("…")
            } else {
                self.label.clone()
            });
        let flash = (self.presses > 0).then(|| {
            div()
                .absolute()
                .inset_0()
                .rounded(px(14.))
                .bg(blue)
                .with_animation(
                    ElementId::NamedInteger("key-flash".into(), self.presses),
                    Animation::new(FLASH).with_easing(ease_out_quint()),
                    |d, t| d.opacity(0.55 * (1. - t)),
                )
        });
        div()
            .id(self.id)
            .relative()
            .flex_none()
            .w(px(168.))
            .h(px(132.))
            .pt(px(8. - skirt))
            .cursor_pointer()
            .child(
                // Base escura (lateral da tecla) e a face por cima, deslocada para cima.
                div()
                    .relative()
                    .size_full()
                    .rounded(px(16.))
                    .bg(tok::inset())
                    .border_1()
                    .border_color(if down { blue } else { tok::line() })
                    .pb(px(skirt))
                    .when(down, |d| {
                        let mut glow = blue;
                        glow.a = 0.35;
                        d.shadow(vec![BoxShadow {
                            color: glow.into(),
                            offset: point(px(0.), px(0.)),
                            blur_radius: px(28.),
                            spread_radius: px(0.),
                            inset: false,
                        }])
                    })
                    .child(div().relative().size_full().child(face).children(flash)),
            )
            .when_some(self.on_click, |d, f| {
                d.on_click(move |e, w, cx| f(e, w, cx))
            })
    }
}
