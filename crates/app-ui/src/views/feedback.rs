//! Janela "Reportar bug / Enviar feedback": vai para o Worker do mantenedor
//! (`brand::FEEDBACK_URL`), não para o servidor de clipes. Plano em `docs/PLANO-feedback.md`.

use app_cloud::feedback::{self, Kind, Report, Specs, MAX_CONTACT, MAX_MESSAGE};
use app_core::brand;
use app_core::Config;
use gpui_kit::component::input::{InputState, Textarea, TextareaState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::server::{buttons, hint, overlay, title};
use crate::app::Shell;
use crate::components::{field_label, text_field, Btn, BtnKind, Segmented, Toggle};
use crate::theme::size::Size;
use crate::theme::{tok, R_SM};
use crate::tr;

struct Fields {
    message: Entity<TextareaState>,
    contact: Entity<InputState>,
}

pub struct FeedbackView {
    kind: Kind,
    fields: Option<Fields>,
    /// Incluir logs e informações do PC.
    include: bool,
    /// Specs e logs coletados em segundo plano ao abrir (`None` = coletando).
    prepared: Option<(Specs, String)>,
    busy: bool,
    error: Option<SharedString>,
    /// Id devolvido pelo Worker: mostra "Recebido".
    sent: Option<String>,
}

/// Coleta specs e logs (lento: cria um dispositivo D3D11 e lê arquivos).
fn prepare(capture_backend: String) -> (Specs, String) {
    #[cfg(windows)]
    let (os, gpu) = (
        app_platform_win::system::os_version(),
        app_platform_win::gpu::Gpu::new()
            .map(|g| g.adapter_name)
            .unwrap_or_default(),
    );
    #[cfg(not(windows))]
    let (os, gpu) = (std::env::consts::OS.to_string(), String::new());
    let specs = Specs {
        app_version: crate::updater::CURRENT.to_string(),
        os,
        gpu,
        capture_backend,
        locale: rust_i18n::locale().to_string(),
    };
    (
        specs,
        feedback::collect_logs(&Config::data_dir().join("logs")),
    )
}

impl Shell {
    pub fn open_feedback(&mut self, kind: Kind, cx: &mut Context<Self>) {
        if self.feedback.as_ref().is_some_and(|f| f.busy) {
            return;
        }
        self.feedback = Some(FeedbackView {
            kind,
            fields: None,
            include: kind == Kind::Bug,
            prepared: None,
            busy: false,
            error: None,
            sent: None,
        });
        let backend = format!("{:?}", self.config.video.capture_backend).to_lowercase();
        cx.spawn(async move |this, cx| {
            let p = cx
                .background_executor()
                .spawn(async move { prepare(backend) })
                .await;
            let _ = this.update(cx, |t, cx| {
                if let Some(f) = t.feedback.as_mut() {
                    f.prepared = Some(p);
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn close_feedback(&mut self, cx: &mut Context<Self>) {
        if !self.feedback.as_ref().is_some_and(|f| f.busy) {
            self.feedback = None;
            cx.notify();
        }
    }

    fn feedback_error(&mut self, msg: SharedString, cx: &mut Context<Self>) {
        if let Some(f) = self.feedback.as_mut() {
            f.error = Some(msg);
        }
        cx.notify();
    }

    fn ensure_feedback_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.feedback.as_ref().is_some_and(|f| f.fields.is_none()) {
            return;
        }
        let message = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(5, 10)
                .placeholder(tr("feedback.message_ph"))
        });
        let contact =
            cx.new(|cx| InputState::new(window, cx).placeholder(tr("feedback.contact_ph")));
        if let Some(f) = self.feedback.as_mut() {
            f.fields = Some(Fields { message, contact });
        }
    }

    /// O report como vai sair (`None` se a mensagem for inválida ou a coleta não terminou).
    fn build_report(&mut self, cx: &mut Context<Self>) -> Option<Report> {
        let f = self.feedback.as_ref()?;
        let fields = f.fields.as_ref()?;
        let message = fields.message.read(cx).value().trim().to_string();
        let contact = fields.contact.read(cx).value().trim().to_string();
        if message.is_empty() {
            self.feedback_error(tr("feedback.empty"), cx);
            return None;
        }
        if message.chars().count() > MAX_MESSAGE {
            self.feedback_error(tr("feedback.too_long"), cx);
            return None;
        }
        if contact.chars().count() > MAX_CONTACT {
            self.feedback_error(tr("feedback.contact_too_long"), cx);
            return None;
        }
        let (specs, logs) = match (f.include, f.prepared.as_ref()) {
            (true, Some((s, l))) => (Some(s.clone()), l.clone()),
            (true, None) => return None,
            (false, _) => (None, String::new()),
        };
        Some(Report {
            kind: f.kind,
            message,
            contact,
            specs,
            logs,
        })
    }

    /// "Ver o que será enviado": grava o texto final num arquivo e abre no editor padrão.
    fn preview_feedback(&mut self, cx: &mut Context<Self>) {
        let Some(r) = self.build_report(cx) else {
            return;
        };
        let path = Config::data_dir().join("logs").join("feedback-preview.txt");
        match std::fs::write(&path, r.preview()) {
            Ok(()) => cx.open_url(&path.to_string_lossy()),
            Err(e) => self.feedback_error(e.to_string().into(), cx),
        }
    }

    fn submit_feedback(&mut self, cx: &mut Context<Self>) {
        if self.feedback.as_ref().is_none_or(|f| f.busy) {
            return;
        }
        let Some(report) = self.build_report(cx) else {
            return;
        };
        if let Some(f) = self.feedback.as_mut() {
            f.busy = true;
            f.error = None;
        }
        cx.notify();
        cx.spawn(async move |this, cx| {
            let r = cx
                .background_executor()
                .spawn(async move {
                    feedback::send_report(brand::FEEDBACK_URL, crate::updater::CURRENT, &report)
                })
                .await;
            let _ = this.update(cx, |t, cx| {
                let Some(f) = t.feedback.as_mut() else {
                    return;
                };
                f.busy = false;
                match r {
                    Ok(id) => f.sent = Some(id),
                    Err(e) if e.to_string() == "rate_limited" => {
                        f.error = Some(tr("feedback.rate_limited"))
                    }
                    Err(e) => {
                        f.error = Some(crate::tr_args(
                            "feedback.error",
                            &[("error", &e.to_string())],
                        ))
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn feedback_modal(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        self.ensure_feedback_fields(window, cx);
        let f = self.feedback.as_ref()?;
        let fields = f.fields.as_ref()?;

        if let Some(id) = f.sent.clone() {
            return Some(overlay(
                "feedback",
                440.,
                div()
                    .flex()
                    .flex_col()
                    .gap(px(14.))
                    .child(title(tr("feedback.sent_title")))
                    .child(hint(crate::tr_args("feedback.sent_body", &[("id", &id)])))
                    .child(
                        div().flex().flex_row().justify_end().child(
                            Btn::new("feedback-close", BtnKind::Primary, tr("feedback.close"))
                                .size(Size::Md)
                                .on_click(cx.listener(|this, _, _, cx| this.close_feedback(cx))),
                        ),
                    ),
            ));
        }

        let busy = f.busy;
        let kind = f.kind;
        let include = f.include;
        let ready = f.prepared.is_some();
        let message_box = div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(field_label(tr(if kind == Kind::Bug {
                "feedback.message_bug"
            } else {
                "feedback.message_feedback"
            })))
            .child(
                div()
                    .w_full()
                    .px(px(4.))
                    .py(px(2.))
                    .bg(tok::field())
                    .border_1()
                    .border_color(tok::edge())
                    .rounded(R_SM)
                    .text_size(px(14.))
                    .text_color(tok::fg())
                    .child(Textarea::new(&fields.message).appearance(false)),
            );
        let include_row = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(10.))
            .child(
                Toggle::new("feedback-include", include).on_click(cx.listener(|this, _, _, cx| {
                    if let Some(f) = this.feedback.as_mut() {
                        f.include = !f.include;
                    }
                    cx.notify();
                })),
            )
            .child(
                div()
                    .flex_1()
                    .text_size(px(13.))
                    .text_color(tok::fg())
                    .child(tr("feedback.include")),
            )
            .child(
                Btn::new(
                    "feedback-preview",
                    BtnKind::Secondary,
                    tr(if include && !ready {
                        "feedback.collecting"
                    } else {
                        "feedback.preview"
                    }),
                )
                .size(Size::Sm)
                .disabled(busy || (include && !ready))
                .on_click(cx.listener(|this, _, _, cx| this.preview_feedback(cx))),
            );

        Some(overlay(
            "feedback",
            480.,
            div()
                .flex()
                .flex_col()
                .gap(px(14.))
                .child(title(tr("feedback.title")))
                .child(
                    Segmented::new(
                        "feedback-kind",
                        vec![tr("feedback.kind_bug"), tr("feedback.kind_feedback")],
                        (kind == Kind::Feedback) as usize,
                    )
                    .disabled(busy)
                    .on_select(cx.listener(|this, i: &usize, _, cx| {
                        if let Some(f) = this.feedback.as_mut() {
                            f.kind = if *i == 1 { Kind::Feedback } else { Kind::Bug };
                            f.include = f.kind == Kind::Bug;
                            f.error = None;
                        }
                        cx.notify();
                    })),
                )
                .child(message_box)
                .child(text_field(
                    tr("feedback.contact"),
                    &fields.contact,
                    window,
                    cx,
                ))
                .child(include_row)
                .child(hint(tr("feedback.public")))
                .when(kind == Kind::Bug, |d| {
                    d.child(hint(tr("feedback.security")))
                })
                .when_some(f.error.clone(), |d, e| {
                    d.child(div().text_size(px(13.)).text_color(tok::red()).child(e))
                })
                .child(buttons(
                    Btn::new("feedback-cancel", BtnKind::Secondary, tr("clip.cancel"))
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, _, cx| this.close_feedback(cx))),
                    Btn::new(
                        "feedback-send",
                        BtnKind::Primary,
                        tr(if busy {
                            "feedback.sending"
                        } else {
                            "feedback.send"
                        }),
                    )
                    .disabled(busy || (include && !ready))
                    .on_click(cx.listener(|this, _, _, cx| this.submit_feedback(cx))),
                )),
        ))
    }
}
