//! Assistente "Criar meu servidor" (e "Atualizar servidor"): o usuário cola um token da
//! Cloudflare uma vez (fica no cofre do Windows) e o app sobe o servidor pela API REST
//! (`app_cloud::cloudflare`), cria o admin e sai conectado. Plano em
//! `docs/PLANO-onboarding-servidor.md` (Fase 4).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use app_cloud::cloudflare::{self, CfError, NewServer, Step};
use app_cloud::{Client, ServerInfo};
use app_core::brand::GITHUB_REPO;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::app::Shell;
use crate::components::{text_field, Btn, BtnKind};
use crate::secret;
use crate::theme::tok;
use crate::updater::CURRENT;
use crate::views::server::{buttons, hint, os_user_name, overlay, title, NAME_MAX};
use crate::{tr, tr_args};

/// Nome do servidor: até 60 caracteres (o servidor corta o resto).
const SERVER_NAME_MAX: usize = 60;

#[derive(Clone, PartialEq, Eq)]
enum Mode {
    Create,
    /// Atualizar o servidor deste endereço.
    Update(String),
}

struct Fields {
    token: Entity<InputState>,
    name: Entity<InputState>,
    admin: Entity<InputState>,
}

/// Janela do assistente (por cima de tudo). Os campos nascem no primeiro desenho.
pub struct CreateView {
    mode: Mode,
    fields: Option<Fields>,
    /// Etapa em andamento (`None` = parado).
    step: Option<Step>,
    error: Option<SharedString>,
    /// R2 desligado nesta conta: botão para a página de ativação.
    r2_account: Option<String>,
}

enum Done {
    Created {
        url: String,
        info: ServerInfo,
        token: String,
    },
    Updated(ServerInfo),
}

/// Erro do assistente → texto da interface.
fn message(e: &CfError) -> SharedString {
    match e {
        CfError::BadToken => tr("create.err_token"),
        CfError::NoAccount => tr("create.err_account"),
        CfError::R2Disabled { .. } => tr("create.err_r2"),
        CfError::Exists => tr("create.err_exists"),
        CfError::NotWorkersDev => tr("create.err_not_workers_dev"),
        CfError::NotFound => tr("create.err_not_found"),
        CfError::NoBundle(v) => tr_args("create.err_bundle", &[("version", v)]),
        CfError::Timeout => tr("create.err_timeout"),
        CfError::Api(m) => tr_args("create.err_api", &[("error", m)]),
        CfError::Cloud(e) => e.to_string().into(),
    }
}

fn step_label(s: Step) -> SharedString {
    tr(match s {
        Step::Download => "create.step_download",
        Step::Account => "create.step_account",
        Step::Database => "create.step_database",
        Step::Storage => "create.step_storage",
        Step::Upload => "create.step_upload",
        Step::Publish => "create.step_publish",
        Step::Waiting => "create.step_waiting",
        Step::Admin => "create.step_admin",
    })
}

impl Shell {
    /// Abre o assistente para criar um servidor.
    pub fn open_create(&mut self, cx: &mut Context<Self>) {
        self.open_wizard(Mode::Create, cx);
    }

    /// Abre o assistente para atualizar o servidor configurado.
    pub fn open_update_server(&mut self, cx: &mut Context<Self>) {
        let url = self.config.server_url.clone();
        self.open_wizard(Mode::Update(url), cx);
    }

    fn open_wizard(&mut self, mode: Mode, cx: &mut Context<Self>) {
        if self.create.as_ref().is_some_and(|c| c.step.is_some()) {
            return;
        }
        self.create = Some(CreateView {
            mode,
            fields: None,
            step: None,
            error: None,
            r2_account: None,
        });
        cx.notify();
    }

    fn close_create(&mut self, cx: &mut Context<Self>) {
        if !self.create.as_ref().is_some_and(|c| c.step.is_some()) {
            self.create = None;
            cx.notify();
        }
    }

    fn ensure_create_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.create.as_ref().is_none_or(|c| c.fields.is_some()) {
            return;
        }
        let mut mk = |value: String, ph: &str, masked: bool, cx: &mut Context<Self>| {
            let s = cx.new(|cx| {
                InputState::new(window, cx)
                    .masked(masked)
                    .default_value(value)
                    .placeholder(tr(ph))
            });
            cx.subscribe(&s, |this, _, ev: &InputEvent, cx| {
                if matches!(ev, InputEvent::PressEnter { .. }) {
                    this.submit_create(cx);
                }
            })
            .detach();
            s
        };
        let fields = Fields {
            token: mk(
                secret::get(secret::CLOUDFLARE).unwrap_or_default(),
                "create.token_placeholder",
                true,
                cx,
            ),
            name: mk(String::new(), "create.name_placeholder", false, cx),
            admin: mk(os_user_name(), "join.name_placeholder", false, cx),
        };
        if let Some(c) = self.create.as_mut() {
            c.fields = Some(fields);
        }
    }

    fn create_error(&mut self, e: SharedString, cx: &mut Context<Self>) {
        if let Some(c) = self.create.as_mut() {
            c.error = Some(e);
        }
        cx.notify();
    }

    fn submit_create(&mut self, cx: &mut Context<Self>) {
        let Some(c) = self.create.as_ref().filter(|c| c.step.is_none()) else {
            return;
        };
        let Some(f) = c.fields.as_ref() else {
            return;
        };
        let val = |e: &Entity<InputState>| e.read(cx).value().trim().to_string();
        let (token, name, admin) = (val(&f.token), val(&f.name), val(&f.admin));
        let mode = c.mode.clone();
        if token.is_empty() {
            return self.create_error(tr("create.token_empty"), cx);
        }
        if mode == Mode::Create {
            let n = admin.chars().count();
            if n == 0 || n > NAME_MAX || admin.chars().any(char::is_control) {
                return self.create_error(tr("join.invalid_name"), cx);
            }
            if name.chars().count() > SERVER_NAME_MAX || name.chars().any(char::is_control) {
                return self.create_error(tr("create.invalid_server_name"), cx);
            }
        }
        // O token fica no cofre: atualizar o servidor depois não pede de novo.
        secret::put(secret::CLOUDFLARE, &token);
        // Criação interrompida (ex.: o endereço demorou a responder): retoma com o mesmo segredo.
        let (setup_token, resume) = match secret::get(secret::SETUP) {
            Some(t) => (t, true),
            None => {
                let t = cloudflare::random_secret();
                secret::put(secret::SETUP, &t);
                (t, false)
            }
        };
        let lang = if rust_i18n::locale().starts_with("pt") {
            "pt-BR"
        } else {
            "en"
        };
        let new = NewServer {
            name,
            lang: lang.into(),
            setup_token: setup_token.clone(),
            resume,
        };
        if let Some(c) = self.create.as_mut() {
            c.step = Some(Step::Download);
            c.error = None;
            c.r2_account = None;
        }
        cx.notify();

        let step = Arc::new(Mutex::new(Step::Download));
        let finished = Arc::new(AtomicBool::new(false));
        let (s2, f2) = (step.clone(), finished.clone());
        let task = cx.background_executor().spawn(async move {
            let mut on = |s: Step| *s2.lock().unwrap_or_else(|e| e.into_inner()) = s;
            let r = (|| {
                let bundle = cloudflare::fetch_bundle(GITHUB_REPO, CURRENT)?;
                match mode {
                    Mode::Create => {
                        let url = cloudflare::create(&token, &bundle, &new, &mut on)?;
                        on(Step::Admin);
                        let c = Client::new(&url, None);
                        let t = c.setup(&setup_token, &admin)?;
                        let info = c.info()?;
                        Ok(Done::Created {
                            url,
                            info,
                            token: t,
                        })
                    }
                    Mode::Update(url) => {
                        cloudflare::update(&token, &bundle, &url, &mut on)?;
                        Ok(Done::Updated(Client::new(&url, None).info()?))
                    }
                }
            })();
            f2.store(true, Ordering::Relaxed);
            r
        });
        cx.spawn(async move |this, cx| {
            while !finished.load(Ordering::Relaxed) {
                cx.background_executor()
                    .timer(Duration::from_millis(250))
                    .await;
                let s = *step.lock().unwrap_or_else(|e| e.into_inner());
                let _ = this.update(cx, |t, cx| {
                    if let Some(c) = t.create.as_mut().filter(|c| c.step.is_some()) {
                        c.step = Some(s);
                        cx.notify();
                    }
                });
            }
            let r: Result<Done, CfError> = task.await;
            let _ = this.update(cx, |t, cx| {
                if let Some(c) = t.create.as_mut() {
                    c.step = None;
                }
                match r {
                    Ok(Done::Created { url, info, token }) => {
                        secret::remove(secret::SETUP);
                        t.create = None;
                        t.connect(url, info, token, cx);
                    }
                    Ok(Done::Updated(info)) => {
                        t.create = None;
                        t.notify_user(
                            tr_args("create.updated", &[("version", &info.server_version)]),
                            cx,
                        );
                        t.server_info = Some(info);
                    }
                    Err(e) => {
                        if matches!(e, CfError::BadToken) {
                            secret::remove(secret::CLOUDFLARE);
                        }
                        let msg = message(&e);
                        if let Some(c) = t.create.as_mut() {
                            c.r2_account = match e {
                                CfError::R2Disabled { account } => Some(account),
                                _ => None,
                            };
                        }
                        t.create_error(msg, cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    // ---------- desenho ----------

    pub fn create_modal(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        self.ensure_create_fields(window, cx);
        let c = self.create.as_ref()?;
        let f = c.fields.as_ref()?;
        let busy = c.step.is_some();
        let creating = c.mode == Mode::Create;
        let token_row = div()
            .flex()
            .flex_row()
            .items_end()
            .gap(px(8.))
            .child(div().flex_1().min_w_0().child(text_field(
                tr("create.token"),
                &f.token,
                window,
                cx,
            )))
            .child(
                Btn::new(
                    "create-cf",
                    BtnKind::Secondary,
                    tr("create.open_cloudflare"),
                )
                .size(crate::theme::size::Size::Md)
                .on_click(|_, _, cx| cx.open_url(cloudflare::TOKEN_URL)),
            );
        let body = div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(hint(tr(if creating {
                "create.intro"
            } else {
                "create.update_intro"
            })))
            .child(hint(tr("create.token_hint")))
            .child(token_row)
            .when(creating, |d| {
                d.child(text_field(tr("create.name"), &f.name, window, cx))
                    .child(text_field(tr("join.name"), &f.admin, window, cx))
                    .child(hint(tr("create.r2_hint")))
            });
        let r2 = c.r2_account.clone();
        Some(overlay(
            "create",
            480.,
            div()
                .flex()
                .flex_col()
                .gap(px(16.))
                .child(title(tr(if creating {
                    "create.title"
                } else {
                    "create.update_title"
                })))
                .child(body)
                .when_some(c.step, |d, s| {
                    d.child(
                        div()
                            .text_size(px(13.))
                            .text_color(tok::fg_2())
                            .child(step_label(s)),
                    )
                })
                .when_some(c.error.clone(), |d, e| {
                    d.child(div().text_size(px(13.)).text_color(tok::red()).child(e))
                })
                .when_some(r2, |d, account| {
                    d.child(
                        div().flex().flex_row().child(
                            Btn::new("create-r2", BtnKind::Secondary, tr("create.open_r2"))
                                .size(crate::theme::size::Size::Sm)
                                .on_click(move |_, _, cx| {
                                    cx.open_url(&cloudflare::r2_url(&account))
                                }),
                        ),
                    )
                })
                .child(buttons(
                    Btn::new("create-cancel", BtnKind::Secondary, tr("clip.cancel"))
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, _, cx| this.close_create(cx))),
                    Btn::new(
                        "create-ok",
                        BtnKind::Primary,
                        tr(match (busy, creating) {
                            (true, _) => "create.working",
                            (false, true) => "create.ok",
                            (false, false) => "create.update_ok",
                        }),
                    )
                    .disabled(busy)
                    .on_click(cx.listener(|this, _, _, cx| this.submit_create(cx))),
                )),
        ))
    }
}
