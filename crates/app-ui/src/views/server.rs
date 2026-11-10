//! Servidor pelo app: entrar com um convite (ou com o código de outro PC), convidar amigo,
//! membros, conectar outro PC e sair. Plano em `docs/PLANO-onboarding-servidor.md` (Fase 3).

use app_cloud::invite::{self, Invite};
use app_cloud::{Client, CloudError, LinkCode, Member, ServerInfo, API_VERSION};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::app::Shell;
use crate::components::{glass, text_field, Btn, BtnKind, Segmented};
use crate::format;
use crate::secret;
use crate::theme::size::Size;
use crate::theme::{tok, FONT_NUM, R_PANEL, R_SM};
use crate::{tr, tr_args};

/// Nome da conta: até 32 caracteres (mesma regra do servidor).
pub(super) const NAME_MAX: usize = 32;

#[derive(Clone, Copy, PartialEq, Eq)]
enum JoinMode {
    /// Link de convite: conta nova.
    Invite,
    /// Código de um PC já conectado: mesma conta.
    Code,
}

struct JoinFields {
    invite: Entity<InputState>,
    name: Entity<InputState>,
    server: Entity<InputState>,
    code: Entity<InputState>,
}

/// Janela "Entrar num servidor" (por cima de tudo). Os campos nascem no primeiro desenho.
pub struct JoinView {
    prefill: Option<Invite>,
    fields: Option<JoinFields>,
    mode: JoinMode,
    busy: bool,
    error: Option<SharedString>,
}

/// Janelas do servidor abertas a partir das Configurações.
pub enum ServerModal {
    /// Código para o outro PC.
    Link { server: String, code: LinkCode },
    /// Membros (admin). `None` = carregando.
    Members(Option<Vec<Member>>),
    /// "Sair do servidor?" (para voltar precisa de convite novo ou do código de outro PC).
    ConfirmLeave,
}

/// O que o botão Entrar pede ao servidor.
pub(crate) enum Ask {
    Join { code: String, name: String },
    Claim { code: String },
}

/// Nome do usuário do Windows (sugestão do nome da conta).
pub(crate) fn os_user_name() -> String {
    std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_default()
        .chars()
        .take(NAME_MAX)
        .collect()
}

impl Shell {
    /// Abre a janela "Entrar num servidor" (preenchida quando veio de um link).
    pub fn open_join(&mut self, prefill: Option<Invite>, cx: &mut Context<Self>) {
        if self.join.as_ref().is_some_and(|j| j.busy) {
            return;
        }
        self.join = Some(JoinView {
            prefill,
            fields: None,
            mode: JoinMode::Invite,
            busy: false,
            error: None,
        });
        cx.notify();
    }

    /// Link `glint://` recebido (abertura do app ou janela já aberta).
    pub fn open_link(&mut self, link: &str, cx: &mut Context<Self>) {
        // `glint://feedback[?kind=bug]` (item "Reportar bug" da bandeja).
        if link.to_ascii_lowercase().starts_with("glint://feedback") {
            let bug = link.contains("kind=bug");
            use app_cloud::feedback::Kind;
            return self.open_feedback(if bug { Kind::Bug } else { Kind::Feedback }, cx);
        }
        match invite::parse(link) {
            Some(i) => self.open_join(Some(i), cx),
            None => self.notify_user(tr("join.invalid_link"), cx),
        }
    }

    /// "Entrar num servidor" já no modo "Outro PC" (código de um PC conectado).
    pub fn open_join_code(&mut self, cx: &mut Context<Self>) {
        self.open_join(None, cx);
        if let Some(j) = self.join.as_mut() {
            j.mode = JoinMode::Code;
        }
    }

    /// Link de convite colado (ou só o código, para o servidor já configurado).
    pub(crate) fn parse_invite_text(&self, text: &str) -> Option<Invite> {
        invite::parse(text).or_else(|| {
            (self.has_server() && !text.is_empty() && !text.contains(['/', ' '])).then(|| Invite {
                server: self.config.server_url.clone(),
                code: text.to_string(),
            })
        })
    }

    /// Entra no servidor em segundo plano e guarda a conta; `done` recebe o erro (ou `None`).
    pub(crate) fn join_with(
        &mut self,
        server: String,
        ask: Ask,
        done: impl FnOnce(&mut Shell, Option<SharedString>, &mut Context<Shell>) + 'static,
        cx: &mut Context<Self>,
    ) {
        let incompatible = tr("join.incompatible").to_string();
        let base = server.clone();
        cx.spawn(async move |this, cx| {
            let r = cx
                .background_executor()
                .spawn(async move {
                    let c = Client::new(&base, None);
                    let info = match c.info() {
                        Ok(i) if i.api_version == API_VERSION => i,
                        // Servidor antigo (sem /info) ou de outra versão da API.
                        Ok(_) | Err(CloudError::Api { status: 404, .. }) => {
                            return Err(CloudError::Other(incompatible))
                        }
                        Err(e) => return Err(e),
                    };
                    let token = match ask {
                        Ask::Join { code, name } => c.join(&code, &name)?,
                        Ask::Claim { code } => c.claim_link(&code)?,
                    };
                    Ok((info, token))
                })
                .await;
            let _ = this.update(cx, |t, cx| {
                match r {
                    Ok((info, token)) => {
                        done(t, None, cx);
                        t.connect(server, info, token, cx);
                    }
                    Err(e) => done(t, Some(e.to_string().into()), cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn close_join(&mut self, cx: &mut Context<Self>) {
        if !self.join.as_ref().is_some_and(|j| j.busy) {
            self.join = None;
            cx.notify();
        }
    }

    fn ensure_join_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(j) = self.join.as_ref().filter(|j| j.fields.is_none()) else {
            return;
        };
        let invite_text = j
            .prefill
            .as_ref()
            .map(|i| format!("{}/i/{}", i.server, i.code))
            .unwrap_or_default();
        let server_text = j
            .prefill
            .as_ref()
            .map(|i| i.server.clone())
            .unwrap_or_else(|| self.config.server_url.clone());
        let mut mk = |value: String, ph: &str, submit: bool, cx: &mut Context<Self>| {
            let s = cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(value)
                    .placeholder(tr(ph))
            });
            if submit {
                cx.subscribe(&s, |this, _, ev: &InputEvent, cx| {
                    if matches!(ev, InputEvent::PressEnter { .. }) {
                        this.submit_join(cx);
                    }
                })
                .detach();
            }
            s
        };
        let fields = JoinFields {
            invite: mk(invite_text, "join.invite_placeholder", false, cx),
            name: mk(os_user_name(), "join.name_placeholder", true, cx),
            server: mk(server_text, "join.server_placeholder", false, cx),
            code: mk(String::new(), "join.code_placeholder", true, cx),
        };
        if let Some(j) = self.join.as_mut() {
            j.fields = Some(fields);
        }
    }

    fn join_error(&mut self, msg: SharedString, cx: &mut Context<Self>) {
        if let Some(j) = self.join.as_mut() {
            j.error = Some(msg);
        }
        cx.notify();
    }

    fn submit_join(&mut self, cx: &mut Context<Self>) {
        let Some(j) = self.join.as_ref().filter(|j| !j.busy) else {
            return;
        };
        let Some(f) = j.fields.as_ref() else {
            return;
        };
        let val = |e: &Entity<InputState>| e.read(cx).value().trim().to_string();
        let (server, ask) = match j.mode {
            JoinMode::Invite => {
                let text = val(&f.invite);
                let name = val(&f.name);
                // Só o código, sem link: vale para o servidor já configurado.
                let Some(inv) = self.parse_invite_text(&text) else {
                    return self.join_error(tr("join.invalid_link"), cx);
                };
                let n = name.chars().count();
                if n == 0 || n > NAME_MAX || name.chars().any(char::is_control) {
                    return self.join_error(tr("join.invalid_name"), cx);
                }
                (
                    inv.server,
                    Ask::Join {
                        code: inv.code,
                        name,
                    },
                )
            }
            JoinMode::Code => {
                let Some(server) = invite::origin(&val(&f.server)) else {
                    return self.join_error(tr("settings.server_invalid"), cx);
                };
                let code = val(&f.code);
                if code.is_empty() {
                    return self.join_error(tr("join.code_empty"), cx);
                }
                (server, Ask::Claim { code })
            }
        };
        if let Some(j) = self.join.as_mut() {
            j.busy = true;
            j.error = None;
        }
        cx.notify();
        self.join_with(
            server,
            ask,
            |t, err, cx| {
                if let Some(j) = t.join.as_mut() {
                    j.busy = false;
                }
                match err {
                    None => t.join = None,
                    Some(e) => t.join_error(e, cx),
                }
            },
            cx,
        );
    }

    /// Guarda a conta nova: troca de servidor se preciso (desconectando a conta antiga).
    pub(super) fn connect(
        &mut self,
        server: String,
        info: ServerInfo,
        token: String,
        cx: &mut Context<Self>,
    ) {
        if server != self.config.server_url {
            if let Some(j) = &self.job {
                j.cancel();
            }
            self.queue.clear();
        }
        self.sign_out(cx);
        if server != self.config.server_url {
            self.apply(|c| c.server_url = server, cx);
        }
        secret::save(&token);
        self.token = Some(token);
        self.notify_user(tr_args("join.connected", &[("name", &info.name)]), cx);
        self.server_info = Some(info);
        // O campo do endereço nas Configurações é recriado com o servidor novo.
        self.settings = None;
        self.fetch_account(cx);
        // Onboarding no passo do servidor: segue para o próximo.
        self.onboarding_connected(cx);
    }

    /// Sai do servidor: desconecta a conta e esquece o endereço.
    pub fn leave_server(&mut self, cx: &mut Context<Self>) {
        if let Some(j) = &self.job {
            j.cancel();
        }
        self.queue.clear();
        self.sign_out(cx);
        self.server_info = None;
        self.admin = false;
        self.server_modal = None;
        self.apply(|c| c.server_url = String::new(), cx);
        self.settings = None;
    }

    /// Chamada autenticada em segundo plano; `done` roda na UI com o resultado.
    fn server_call<T: Send + 'static>(
        &mut self,
        call: impl FnOnce(Client) -> Result<T, CloudError> + Send + 'static,
        done: impl FnOnce(&mut Shell, T, &mut Context<Shell>) + 'static,
        cx: &mut Context<Self>,
    ) {
        let Some(client) = self.client() else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let r = cx
                .background_executor()
                .spawn(async move { call(client) })
                .await;
            let _ = this.update(cx, |t, cx| {
                match r {
                    Ok(v) => done(t, v, cx),
                    Err(CloudError::Unauthorized) => {
                        secret::delete();
                        t.token = None;
                        t.account = None;
                        t.server_modal = None;
                        t.notify_user(CloudError::Unauthorized.to_string(), cx);
                    }
                    Err(e) => t.notify_user(e.to_string(), cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Cria um convite (uso único, 7 dias) e copia o link.
    pub fn invite_friend(&mut self, cx: &mut Context<Self>) {
        self.server_call(
            |c| c.create_invite(),
            |t, url, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(url));
                t.notify_user(tr("server.invite_copied"), cx);
            },
            cx,
        );
    }

    /// Gera o código para entrar com esta conta em outro PC.
    pub fn link_other_pc(&mut self, cx: &mut Context<Self>) {
        let server = self.config.server_url.clone();
        self.server_call(
            |c| c.link_code(),
            move |t, code, _| t.server_modal = Some(ServerModal::Link { server, code }),
            cx,
        );
    }

    pub fn open_members(&mut self, cx: &mut Context<Self>) {
        self.server_modal = Some(ServerModal::Members(None));
        self.reload_members(cx);
    }

    fn reload_members(&mut self, cx: &mut Context<Self>) {
        self.server_call(
            |c| c.members(),
            |t, list, _| {
                if let Some(ServerModal::Members(m)) = t.server_modal.as_mut() {
                    *m = Some(list);
                }
            },
            cx,
        );
    }

    fn set_member_enabled(&mut self, id: String, enabled: bool, cx: &mut Context<Self>) {
        self.server_call(
            move |c| c.set_member_enabled(&id, enabled),
            |t, (), cx| t.reload_members(cx),
            cx,
        );
    }

    fn revoke_device(&mut self, member: String, device: String, cx: &mut Context<Self>) {
        self.server_call(
            move |c| c.revoke_device(&member, &device),
            |t, (), cx| t.reload_members(cx),
            cx,
        );
    }

    /// Abre o painel do site já logado (código de uso único).
    fn open_admin_site(&mut self, cx: &mut Context<Self>) {
        self.server_call(|c| c.web_url("/admin"), |_, url, cx| cx.open_url(&url), cx);
    }

    // ---------- desenho ----------

    pub fn join_modal(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        self.ensure_join_fields(window, cx);
        let j = self.join.as_ref()?;
        let f = j.fields.as_ref()?;
        let mode = j.mode;
        let busy = j.busy;
        let body = match mode {
            JoinMode::Invite => div()
                .flex()
                .flex_col()
                .gap(px(12.))
                .child(text_field(tr("join.invite"), &f.invite, window, cx))
                .child(text_field(tr("join.name"), &f.name, window, cx))
                .child(hint(tr("join.invite_hint"))),
            JoinMode::Code => div()
                .flex()
                .flex_col()
                .gap(px(12.))
                .child(text_field(tr("join.server"), &f.server, window, cx))
                .child(text_field(tr("join.code"), &f.code, window, cx))
                .child(hint(tr("join.code_hint"))),
        };
        Some(overlay(
            "join",
            440.,
            div()
                .flex()
                .flex_col()
                .gap(px(16.))
                .child(title(tr("join.title")))
                .child(
                    Segmented::new(
                        "join-mode",
                        vec![tr("join.mode_invite"), tr("join.mode_code")],
                        (mode == JoinMode::Code) as usize,
                    )
                    .disabled(busy)
                    .on_select(cx.listener(|this, i: &usize, _, cx| {
                        if let Some(j) = this.join.as_mut() {
                            j.mode = if *i == 1 {
                                JoinMode::Code
                            } else {
                                JoinMode::Invite
                            };
                            j.error = None;
                        }
                        cx.notify();
                    })),
                )
                .child(body)
                .when_some(j.error.clone(), |d, e| {
                    d.child(div().text_size(px(13.)).text_color(tok::red()).child(e))
                })
                .child(buttons(
                    Btn::new("join-cancel", BtnKind::Secondary, tr("clip.cancel"))
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, _, cx| this.close_join(cx))),
                    Btn::new(
                        "join-ok",
                        BtnKind::Primary,
                        tr(if busy { "join.joining" } else { "join.ok" }),
                    )
                    .disabled(busy)
                    .on_click(cx.listener(|this, _, _, cx| this.submit_join(cx))),
                )),
        ))
    }

    pub fn server_modal_view(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let close = |cx: &mut Context<Self>| {
            Btn::new("srv-close", BtnKind::Primary, tr("server.close")).on_click(cx.listener(
                |this, _, _, cx| {
                    this.server_modal = None;
                    cx.notify();
                },
            ))
        };
        match self.server_modal.as_ref()? {
            ServerModal::ConfirmLeave => Some(
                crate::components::modal::confirm(
                    "srv-leave-confirm",
                    tr("server.leave_title"),
                    tr("server.leave_body"),
                    None,
                    Btn::new("srv-leave-no", BtnKind::Secondary, tr("clip.cancel")).on_click(
                        cx.listener(|this, _, _, cx| {
                            this.server_modal = None;
                            cx.notify();
                        }),
                    ),
                    Some(
                        Btn::new("srv-leave-yes", BtnKind::Danger, tr("server.leave"))
                            .on_click(cx.listener(|this, _, _, cx| this.leave_server(cx))),
                    ),
                )
                .into_any_element(),
            ),
            ServerModal::Link { server, code } => {
                let copy = code.code.clone();
                Some(overlay(
                    "srv-link",
                    420.,
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(14.))
                        .child(title(tr("server.link_title")))
                        .child(
                            div()
                                .py(px(14.))
                                .rounded(R_SM)
                                .bg(tok::field())
                                .border_1()
                                .border_color(tok::edge())
                                .flex()
                                .justify_center()
                                .font_family(FONT_NUM)
                                .text_size(px(30.))
                                .child(code.code.clone()),
                        )
                        .child(hint(tr_args("server.link_hint", &[("server", server)])))
                        .child(buttons(
                            Btn::new("srv-copy", BtnKind::Secondary, tr("server.copy_code"))
                                .on_click(move |_, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))
                                }),
                            close(cx),
                        )),
                ))
            }
            ServerModal::Members(list) => {
                let rows: AnyElement = match list {
                    None => hint(tr("server.loading")).into_any_element(),
                    Some(list) => div()
                        .flex()
                        .flex_col()
                        .gap(px(8.))
                        .children(
                            list.iter()
                                .enumerate()
                                .map(|(i, m)| self.member_row(i, m, cx)),
                        )
                        .into_any_element(),
                };
                Some(overlay(
                    "srv-members",
                    520.,
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(14.))
                        .child(title(tr("server.members")))
                        .child(
                            div()
                                .id("srv-members-list")
                                .max_h(px(420.))
                                .overflow_y_scrollbar()
                                .child(rows),
                        )
                        .child(buttons(
                            Btn::new("srv-admin", BtnKind::Secondary, tr("server.open_site"))
                                .on_click(cx.listener(|this, _, _, cx| this.open_admin_site(cx))),
                            close(cx),
                        )),
                ))
            }
        }
    }

    fn member_row(&self, i: usize, m: &Member, cx: &mut Context<Self>) -> Div {
        let now = chrono::Local::now().timestamp_millis();
        let me = m.devices.iter().any(|d| d.current);
        let name = m
            .user
            .name
            .clone()
            .unwrap_or_else(|| m.user.username.clone());
        let mut tags = Vec::new();
        if m.user.is_admin() {
            tags.push(tr("server.admin").to_string());
        }
        if m.disabled {
            tags.push(tr("server.disabled").to_string());
        }
        let meta = tr_args(
            "server.member_meta",
            &[
                ("clips", &m.clips.to_string()),
                ("size", &format::size(m.used_bytes)),
            ],
        );
        let (id, enabled) = (m.id.clone(), m.disabled);
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .p(px(12.))
            .rounded(R_SM)
            .bg(tok::surf())
            .border_1()
            .border_color(tok::edge())
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(10.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .gap(px(2.))
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(14.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(if tags.is_empty() {
                                        name
                                    } else {
                                        format!("{name} · {}", tags.join(" · "))
                                    }),
                            )
                            .child(hint(meta)),
                    )
                    .when(!me, |d| {
                        d.child(
                            Btn::new(
                                ElementId::NamedInteger("srv-member".into(), i as u64),
                                BtnKind::Secondary,
                                tr(if m.disabled {
                                    "server.enable"
                                } else {
                                    "server.disable"
                                }),
                            )
                            .size(Size::Sm)
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.set_member_enabled(id.clone(), enabled, cx)
                                },
                            )),
                        )
                    }),
            )
            .children(m.devices.iter().enumerate().map(|(k, d)| {
                let (member, device) = (m.id.clone(), d.id.clone());
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(10.))
                    .child(div().flex_1().min_w_0().child(hint(tr_args(
                        if d.current {
                            "server.device_current"
                        } else {
                            "server.device"
                        },
                        &[
                            (
                                "kind",
                                &tr(if d.kind == "web" {
                                    "server.device_web"
                                } else {
                                    "server.device_app"
                                }),
                            ),
                            ("when", &format::relative(d.last_used_at, now)),
                        ],
                    ))))
                    .when(!d.current, |row| {
                        row.child(
                            Btn::new(
                                ElementId::NamedInteger(format!("srv-dev-{i}").into(), k as u64),
                                BtnKind::Ghost,
                                tr("server.revoke"),
                            )
                            .size(Size::Sm)
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.revoke_device(member.clone(), device.clone(), cx)
                                },
                            )),
                        )
                    })
            }))
    }
}

/// Véu escuro + cartão de base sólida centralizado (mesmo visual do primeiro uso).
pub(super) fn overlay(id: &'static str, w: f32, content: Div) -> AnyElement {
    div()
        .id(id)
        .absolute()
        .inset_0()
        .occlude()
        .bg(rgba(0x0000008c))
        .flex()
        .items_center()
        .justify_center()
        .child(
            glass::glass_panel(R_PANEL)
                .bg(rgba(crate::theme::glass::solid(crate::theme::pal()).window))
                .w(px(w))
                .p(px(24.))
                .child(content),
        )
        .into_any_element()
}

pub(super) fn title(text: SharedString) -> Div {
    div()
        .text_size(px(19.))
        .font_weight(FontWeight::SEMIBOLD)
        .child(text)
}

pub(super) fn hint(text: SharedString) -> Div {
    div()
        .text_size(px(12.))
        .line_height(px(17.))
        .text_color(tok::fg_2())
        .child(text)
}

pub(super) fn buttons(left: Btn, right: Btn) -> Div {
    div()
        .flex()
        .flex_row()
        .justify_end()
        .gap(px(8.))
        .pt(px(4.))
        .child(left.size(Size::Md))
        .child(right.size(Size::Md))
}
