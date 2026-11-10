//! Configurações (§7.2): cada mudança grava o `config.toml` na hora e pede ao
//! gravador para reler (`Request::ReloadConfig`). Textos gravam ao sair do campo.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use app_cloud::{login, Client};
use app_core::{Config, Corner, KillConfig, Resolution};
use app_ipc::Request;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::app::Shell;
use crate::components::{glass, input_box, Btn, BtnKind, Segmented, Toggle};
use crate::recorder;
use crate::secret;
use crate::theme::size::Size;
use crate::theme::{tok, R_PANEL_SM};
use crate::views::server::ServerModal;
use crate::{tr, tr_args};

/// Guia para subir o próprio servidor.
pub(crate) const DEPLOY_GUIDE_URL: &str =
    "https://github.com/bps2414/glint/blob/main/docs/DEPLOY.md";

const BUFFERS: [u32; 4] = [15, 30, 60, 120];
const RESOLUTIONS: [Resolution; 3] = [Resolution::P720, Resolution::P1080, Resolution::Native];
const FPS: [u32; 2] = [30, 60];
/// Rótulos do canto da marca d'água, na ordem de [`Corner::ALL`].
const CORNER_KEYS: [&str; 4] = [
    "settings.corner_top_left",
    "settings.corner_top_right",
    "settings.corner_bottom_left",
    "settings.corner_bottom_right",
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Hotkey,
    KillHotkey,
    KillPre,
    KillPost,
    KillCombo,
    Folder,
    Limit,
    Include,
    Exclude,
    Server,
}

pub struct SettingsView {
    hotkey: Entity<InputState>,
    kill_hotkey: Entity<InputState>,
    kill_pre: Entity<InputState>,
    kill_post: Entity<InputState>,
    kill_combo: Entity<InputState>,
    folder: Entity<InputState>,
    limit: Entity<InputState>,
    include: Entity<InputState>,
    exclude: Entity<InputState>,
    server: Entity<InputState>,
    /// Endereço do servidor recusado na última vez que o campo gravou.
    server_error: Option<SharedString>,
    login: Option<Arc<AtomicBool>>,
}

impl SettingsView {
    fn input(&self, field: Field) -> &Entity<InputState> {
        match field {
            Field::Hotkey => &self.hotkey,
            Field::KillHotkey => &self.kill_hotkey,
            Field::KillPre => &self.kill_pre,
            Field::KillPost => &self.kill_post,
            Field::KillCombo => &self.kill_combo,
            Field::Folder => &self.folder,
            Field::Limit => &self.limit,
            Field::Include => &self.include,
            Field::Exclude => &self.exclude,
            Field::Server => &self.server,
        }
    }
}

impl Shell {
    pub fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings.is_some() {
            return;
        }
        let c = self.config.clone();
        let mut input = |field: Field, value: String, cx: &mut Context<Self>| {
            let s = cx.new(|cx| InputState::new(window, cx).default_value(value));
            cx.subscribe_in(&s, window, move |this, _, ev: &InputEvent, window, cx| {
                if matches!(ev, InputEvent::Blur | InputEvent::PressEnter { .. }) {
                    this.save_field(field, window, cx);
                }
            })
            .detach();
            s
        };
        let view = SettingsView {
            hotkey: input(Field::Hotkey, c.save_hotkey.clone(), cx),
            kill_hotkey: input(Field::KillHotkey, c.kill.hotkey.clone(), cx),
            kill_pre: input(Field::KillPre, c.kill.pre_secs.to_string(), cx),
            kill_post: input(Field::KillPost, c.kill.post_secs.to_string(), cx),
            kill_combo: input(Field::KillCombo, c.kill.combo_secs.to_string(), cx),
            folder: input(Field::Folder, c.clips_dir().display().to_string(), cx),
            limit: input(Field::Limit, c.disk_limit_gb.to_string(), cx),
            include: input(Field::Include, c.game.include.join(", "), cx),
            exclude: input(Field::Exclude, c.game.exclude.join(", "), cx),
            server: input(Field::Server, c.server_url.clone(), cx),
            server_error: None,
            login: None,
        };
        self.settings = Some(view);
        if self.account.is_none() {
            self.fetch_account(cx);
        }
    }

    /// Pergunta ao servidor o nome dele (`/info`) e, com conta, o nome e o papel
    /// (para "Você é X" e as ações de admin).
    pub fn fetch_account(&mut self, cx: &mut Context<Self>) {
        if !self.has_server() {
            return;
        }
        let token = self.token.clone();
        let base = self.config.server_url.clone();
        cx.spawn(async move |this, cx| {
            let (info, me) = cx
                .background_executor()
                .spawn(async move {
                    let info = Client::new(&base, None).info();
                    let me = token.map(|t| Client::new(&base, Some(t)).me());
                    (info, me)
                })
                .await;
            let _ = this.update(cx, |t, cx| {
                match info {
                    Ok(i) => t.server_info = Some(i),
                    Err(e) => tracing::warn!("servidor: {e}"),
                }
                match me {
                    Some(Ok(m)) => {
                        t.account = Some(m.name().to_string());
                        t.admin = m.user.is_admin();
                    }
                    Some(Err(app_cloud::CloudError::Unauthorized)) => {
                        secret::delete();
                        t.token = None;
                        t.admin = false;
                    }
                    Some(Err(e)) => tracing::warn!("conta: {e}"),
                    None => {}
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Grava a config e avisa o gravador.
    pub(crate) fn apply(&mut self, f: impl FnOnce(&mut Config), cx: &mut Context<Self>) {
        f(&mut self.config);
        if let Err(e) = self.config.save(&Config::default_path()) {
            self.notify_user(
                tr_args("settings.save_error", &[("error", &e.to_string())]),
                cx,
            );
            return;
        }
        cx.background_executor()
            .spawn(async { recorder::send(Request::ReloadConfig) })
            .detach();
        cx.notify();
    }

    fn save_field(&mut self, field: Field, window: &mut Window, cx: &mut Context<Self>) {
        let Some(s) = self.settings.as_ref() else {
            return;
        };
        let state = s.input(field);
        let text = state.read(cx).value().trim().to_string();
        let list = |t: &str| -> Vec<String> {
            t.split([',', ';'])
                .map(|x| x.trim().to_string())
                .filter(|x| !x.is_empty())
                .collect()
        };
        match field {
            Field::Hotkey if !text.is_empty() && text != self.config.save_hotkey => {
                if same_hotkey(&text, &self.config.kill.hotkey) {
                    self.reject_hotkey(field, window, cx);
                } else {
                    self.apply(|c| c.save_hotkey = text, cx);
                }
            }
            Field::KillHotkey if !text.is_empty() && text != self.config.kill.hotkey => {
                if same_hotkey(&text, &self.config.save_hotkey) {
                    self.reject_hotkey(field, window, cx);
                } else {
                    self.apply(|c| c.kill.hotkey = text, cx);
                }
            }
            Field::KillPre | Field::KillPost | Field::KillCombo => {
                self.save_kill_secs(field, &text, window, cx)
            }
            Field::Folder => {
                let default = Config {
                    output_dir: Default::default(),
                    ..self.config.clone()
                }
                .clips_dir();
                let p = if text.is_empty() || std::path::Path::new(&text) == default {
                    Default::default()
                } else {
                    std::path::PathBuf::from(&text)
                };
                if p != self.config.output_dir {
                    self.apply(|c| c.output_dir = p, cx);
                }
            }
            Field::Limit => match text.parse::<u32>() {
                Ok(gb) if gb != self.config.disk_limit_gb => {
                    self.apply(|c| c.disk_limit_gb = gb, cx)
                }
                Ok(_) => {}
                Err(_) => self.notify_user(tr("settings.limit_invalid"), cx),
            },
            Field::Include if list(&text) != self.config.game.include => {
                self.apply(|c| c.game.include = list(&text), cx)
            }
            Field::Exclude if list(&text) != self.config.game.exclude => {
                self.apply(|c| c.game.exclude = list(&text), cx)
            }
            Field::Server => self.save_server(&text, window, cx),
            _ => {}
        }
    }

    /// Atalho igual ao do outro clipe: avisa e devolve ao campo o valor gravado.
    fn reject_hotkey(&mut self, field: Field, window: &mut Window, cx: &mut Context<Self>) {
        self.notify_user(tr("settings.kill_hotkey_same"), cx);
        let current = match field {
            Field::KillHotkey => self.config.kill.hotkey.clone(),
            _ => self.config.save_hotkey.clone(),
        };
        if let Some(s) = self.settings.as_ref() {
            s.input(field)
                .update(cx, |i, cx| i.set_value(current, window, cx));
        }
    }

    /// Grava um tempo do clipe de kill (dentro da faixa válida) e mostra no campo o valor que vale.
    fn save_kill_secs(
        &mut self,
        field: Field,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let k = &self.config.kill;
        let (range, current) = match field {
            Field::KillPre => (KillConfig::PRE_RANGE, k.pre_secs),
            Field::KillPost => (KillConfig::POST_RANGE, k.post_secs),
            _ => (KillConfig::COMBO_RANGE, k.combo_secs),
        };
        let n = match text.parse::<u32>() {
            Ok(n) => n.clamp(range.0, range.1),
            Err(_) => {
                let (min, max) = (range.0.to_string(), range.1.to_string());
                self.notify_user(
                    tr_args(
                        "settings.kill_secs_invalid",
                        &[("min", &min), ("max", &max)],
                    ),
                    cx,
                );
                current
            }
        };
        if n != current {
            self.apply(|c| set_kill_secs(&mut c.kill, field, n), cx);
        }
        if let Some(s) = self.settings.as_ref() {
            let shown = n.to_string();
            let state = s.input(field);
            if state.read(cx).value() != shown.as_str() {
                state.update(cx, |i, cx| i.set_value(shown, window, cx));
            }
        }
    }

    /// Grava o endereço do servidor. Trocar de servidor desconecta a conta (o token era do outro).
    fn save_server(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(url) = normalize_server(text) else {
            if let Some(s) = self.settings.as_mut() {
                s.server_error = Some(tr("settings.server_invalid"));
            }
            cx.notify();
            return;
        };
        if let Some(s) = self.settings.as_mut() {
            s.server_error = None;
            if s.server.read(cx).value() != url.as_str() {
                s.server
                    .update(cx, |i, cx| i.set_value(url.clone(), window, cx));
            }
        }
        if url != self.config.server_url {
            if let Some(j) = &self.job {
                j.cancel();
            }
            self.queue.clear();
            self.sign_out(cx);
            self.apply(|c| c.server_url = url, cx);
        }
        cx.notify();
    }

    fn sign_in(&mut self, cx: &mut Context<Self>) {
        if !self.has_server() {
            return;
        }
        let base = self.config.server_url.clone();
        let pending = match login::start(&base) {
            Ok(p) => p,
            Err(e) => {
                self.notify_user(
                    tr_args("publish.login_error", &[("error", &e.to_string())]),
                    cx,
                );
                return;
            }
        };
        cx.open_url(&pending.url);
        let cancel = Arc::new(AtomicBool::new(false));
        if let Some(s) = self.settings.as_mut() {
            s.login = Some(cancel.clone());
        }
        cx.notify();
        cx.spawn(async move |this, cx| {
            let r = cx
                .background_executor()
                .spawn(async move {
                    let code = pending.wait(
                        Duration::from_secs(300),
                        &crate::views::publish::login_page(),
                        &|| cancel.load(Ordering::Relaxed),
                    )?;
                    Client::new(&base, None).exchange_code(&code, &pending.verifier)
                })
                .await;
            let _ = this.update(cx, |t, cx| {
                if let Some(s) = t.settings.as_mut() {
                    s.login = None;
                }
                match r {
                    Ok(token) => {
                        secret::save(&token);
                        t.token = Some(token);
                        t.fetch_account(cx);
                    }
                    Err(app_cloud::CloudError::Cancelled) => {}
                    Err(e) => t.notify_user(e.to_string(), cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn sign_out(&mut self, cx: &mut Context<Self>) {
        if let Some(token) = self.token.take() {
            let base = self.config.server_url.clone();
            if !base.trim().is_empty() {
                cx.background_executor()
                    .spawn(async move { Client::new(&base, Some(token)).logout() })
                    .detach();
            }
        }
        secret::delete();
        self.account = None;
        self.admin = false;
        cx.notify();
    }

    /// Linhas da seção Servidor: entrar/criar sem servidor; com conta, quem você é e as
    /// ações (convidar, membros, outro PC, sair).
    fn server_rows(&self, cx: &mut Context<Self>) -> Vec<Div> {
        let btn = |id: &'static str, kind: BtnKind, label: SharedString| {
            Btn::new(id, kind, label).size(Size::Sm)
        };
        let join = |kind: BtnKind, cx: &mut Context<Self>| {
            btn("srv-join", kind, tr("server.join_button"))
                .on_click(cx.listener(|this, _, _, cx| this.open_join(None, cx)))
        };
        let guide = |id: &'static str, cx: &mut Context<Self>| {
            btn(id, BtnKind::Secondary, tr("settings.open_guide"))
                .on_click(cx.listener(|_, _, _, cx| cx.open_url(DEPLOY_GUIDE_URL)))
        };
        if !self.has_server() {
            return vec![
                row(
                    tr("server.join"),
                    Some(tr("server.join_hint")),
                    join(BtnKind::Primary, cx),
                ),
                row(
                    tr("server.create"),
                    Some(tr("server.create_hint")),
                    btn("srv-create", BtnKind::Secondary, tr("server.create_button"))
                        .on_click(cx.listener(|this, _, _, cx| this.open_create(cx))),
                ),
            ];
        }
        let url = self.config.server_url.clone();
        let name: SharedString = self
            .server_info
            .as_ref()
            .map(|i| i.name.clone())
            .unwrap_or_else(|| url.split("://").nth(1).unwrap_or(&url).to_string())
            .into();
        let leave = |kind: BtnKind, label: &str, cx: &mut Context<Self>| {
            btn("srv-leave", kind, tr(label)).on_click(cx.listener(|this, _, _, cx| {
                this.server_modal = Some(ServerModal::ConfirmLeave);
                cx.notify();
            }))
        };
        let mut rows = Vec::new();
        if self.settings.as_ref().is_some_and(|s| s.login.is_some()) {
            rows.push(row(
                tr("publish.login"),
                Some(tr("publish.login_hint")),
                btn("acc-cancel", BtnKind::Secondary, tr("clip.cancel")).on_click(cx.listener(
                    |this, _, _, _| {
                        if let Some(c) = this.settings.as_ref().and_then(|s| s.login.clone()) {
                            c.store(true, Ordering::Relaxed);
                        }
                    },
                )),
            ));
        } else if self.token.is_some() {
            let who = self.account.as_ref().map(|n| {
                tr_args(
                    if self.admin {
                        "server.you_are_admin"
                    } else {
                        "server.you_are"
                    },
                    &[("name", n)],
                )
            });
            rows.push(row(
                name,
                Some(who.unwrap_or_else(|| url.clone().into())),
                leave(BtnKind::Secondary, "server.leave", cx),
            ));
            if self.admin {
                rows.push(row(
                    tr("server.invite"),
                    Some(tr("server.invite_hint")),
                    btn("srv-invite", BtnKind::Primary, tr("server.invite_button"))
                        .on_click(cx.listener(|this, _, _, cx| this.invite_friend(cx))),
                ));
                rows.push(row(
                    tr("server.members"),
                    Some(tr("server.members_hint")),
                    btn(
                        "srv-members",
                        BtnKind::Secondary,
                        tr("server.members_button"),
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.open_members(cx))),
                ));
            }
            rows.push(row(
                tr("server.link"),
                Some(tr("server.link_row_hint")),
                btn("srv-link", BtnKind::Secondary, tr("server.link_button"))
                    .on_click(cx.listener(|this, _, _, cx| this.link_other_pc(cx))),
            ));
        } else {
            rows.push(row(
                name,
                Some(tr("server.signed_out_hint")),
                leave(BtnKind::Ghost, "server.forget", cx),
            ));
            rows.push(row(
                tr("server.join"),
                Some(tr("server.join_again_hint")),
                join(BtnKind::Primary, cx),
            ));
            if self.server_info.as_ref().is_some_and(|i| i.discord_login) {
                rows.push(row(
                    tr("settings.signed_out"),
                    None,
                    btn("acc-in", BtnKind::Secondary, tr("settings.sign_in"))
                        .on_click(cx.listener(|this, _, _, cx| this.sign_in(cx))),
                ));
            }
        }
        if let Some(i) = self
            .server_info
            .as_ref()
            .filter(|i| app_cloud::update::is_newer(crate::updater::CURRENT, &i.server_version))
        {
            // O app atualiza o que está em <worker>.<conta>.workers.dev; domínio próprio, pelo guia.
            let host = url.split("://").nth(1).unwrap_or(&url).split('/').next();
            let on_workers_dev = host.is_some_and(|h| h.ends_with(".workers.dev"));
            rows.push(row(
                tr_args(
                    "server.outdated",
                    &[
                        ("server", &i.server_version),
                        ("app", crate::updater::CURRENT),
                    ],
                ),
                Some(tr(if on_workers_dev {
                    "server.outdated_hint"
                } else {
                    "server.outdated_hint_manual"
                })),
                if on_workers_dev {
                    btn("srv-update", BtnKind::Primary, tr("server.update_button"))
                        .on_click(cx.listener(|this, _, _, cx| this.open_update_server(cx)))
                } else {
                    guide("srv-update", cx)
                },
            ));
        }
        rows
    }

    // ---------- desenho ----------

    /// Configurações no estilo dos Ajustes do macOS: seções com título por fora e as
    /// linhas agrupadas num painel de vidro (rótulo à esquerda, controle à direita).
    pub fn settings_screen(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        self.open_settings(window, cx);
        let server_rows = self.server_rows(cx);
        let s = self.settings.as_ref().expect("configurações abertas");
        let c = &self.config;
        let buf = BUFFERS.iter().position(|&b| b == c.buffer_secs);
        let res = RESOLUTIONS
            .iter()
            .position(|&r| r == c.video.resolution)
            .unwrap_or(1);
        let fps = FPS.iter().position(|&f| f == c.video.fps).unwrap_or(1);
        let corner = Corner::ALL
            .iter()
            .position(|&k| k == c.watermark_corner)
            .unwrap_or(3);

        let toggle = |id: &'static str, label: SharedString, on: bool, f: fn(&mut Config)| {
            row(
                label,
                None,
                Toggle::new(id, on).on_click(cx.listener(move |this, _, _, cx| this.apply(f, cx))),
            )
        };

        let seg = |e: Segmented| div().flex_none().w(px(SEG_W)).child(e);
        let short_field = |state: &Entity<InputState>, w: f32| {
            div()
                .flex_none()
                .w(px(w))
                .child(input_box(state, window, cx))
        };

        div()
            .flex()
            .flex_col()
            .gap(px(22.))
            .max_w(px(760.))
            .px(px(14.))
            .pt(px(8.))
            .pb(px(24.))
            .child(section(
                tr("settings.recording"),
                vec![
                    row(
                        tr("settings.buffer"),
                        None,
                        seg(Segmented::new(
                            "set-buffer",
                            BUFFERS
                                .iter()
                                .map(|b| tr_args("settings.secs", &[("n", &b.to_string())]))
                                .collect(),
                            buf.unwrap_or(usize::MAX),
                        )
                        .on_select(cx.listener(
                            |this, i: &usize, _, cx| {
                                let b = BUFFERS[*i];
                                this.apply(|c| c.buffer_secs = b, cx)
                            },
                        ))),
                    ),
                    row(
                        tr("settings.quality"),
                        None,
                        seg(Segmented::new(
                            "set-res",
                            vec!["720p".into(), "1080p".into(), tr("settings.native")],
                            res,
                        )
                        .on_select(cx.listener(
                            |this, i: &usize, _, cx| {
                                let r = RESOLUTIONS[*i];
                                this.apply(|c| c.video.resolution = r, cx)
                            },
                        ))),
                    ),
                    row(
                        tr("settings.fps"),
                        None,
                        seg(
                            Segmented::new("set-fps", vec!["30 fps".into(), "60 fps".into()], fps)
                                .on_select(cx.listener(|this, i: &usize, _, cx| {
                                    let f = FPS[*i];
                                    this.apply(|c| c.video.fps = f, cx)
                                })),
                        ),
                    ),
                    row(
                        tr("settings.hotkey"),
                        Some(tr("settings.hotkey_hint")),
                        short_field(&s.hotkey, 180.),
                    ),
                    toggle("set-sound", tr("settings.save_sound"), c.save_sound, |c| {
                        c.save_sound = !c.save_sound
                    }),
                ],
            ))
            .child(section(
                tr("settings.kill"),
                vec![
                    row(
                        tr("settings.kill_hotkey"),
                        Some(tr("settings.kill_hotkey_hint")),
                        short_field(&s.kill_hotkey, 180.),
                    ),
                    row(
                        tr("settings.kill_pre"),
                        None,
                        short_field(&s.kill_pre, 120.),
                    ),
                    row(
                        tr("settings.kill_post"),
                        None,
                        short_field(&s.kill_post, 120.),
                    ),
                    row(
                        tr("settings.kill_combo"),
                        Some(tr("settings.kill_combo_hint")),
                        short_field(&s.kill_combo, 120.),
                    ),
                ],
            ))
            .child(section(
                tr("settings.audio"),
                vec![
                    toggle(
                        "set-sys",
                        tr("settings.audio_system"),
                        c.audio.system,
                        |c| c.audio.system = !c.audio.system,
                    ),
                    toggle(
                        "set-mic",
                        tr("settings.audio_mic"),
                        c.audio.microphone,
                        |c| c.audio.microphone = !c.audio.microphone,
                    ),
                ],
            ))
            .child(section(
                tr("settings.storage"),
                vec![
                    stacked(
                        tr("settings.folder"),
                        None,
                        input_box(&s.folder, window, cx),
                    ),
                    row(
                        tr("settings.limit"),
                        Some(tr("settings.limit_hint")),
                        short_field(&s.limit, 120.),
                    ),
                ],
            ))
            .child(section(
                tr("settings.games"),
                vec![
                    toggle(
                        "set-auto",
                        tr("settings.auto_detect"),
                        c.game.auto_detect,
                        |c| c.game.auto_detect = !c.game.auto_detect,
                    ),
                    stacked(
                        tr("settings.include"),
                        None,
                        input_box(&s.include, window, cx),
                    ),
                    stacked(
                        tr("settings.exclude"),
                        Some(tr("settings.games_hint")),
                        input_box(&s.exclude, window, cx),
                    ),
                ],
            ))
            .child(section(
                tr("settings.export"),
                vec![
                    row(
                        tr("settings.watermark"),
                        Some(tr_args(
                            "settings.watermark_hint",
                            &[("name", app_core::brand::APP_NAME)],
                        )),
                        Toggle::new("set-mark", c.watermark).on_click(cx.listener(
                            |this, _, _, cx| this.apply(|c| c.watermark = !c.watermark, cx),
                        )),
                    ),
                    row(
                        tr("settings.watermark_corner"),
                        None,
                        seg(Segmented::new(
                            "set-mark-corner",
                            CORNER_KEYS.iter().map(|k| tr(k)).collect(),
                            corner,
                        )
                        .disabled(!c.watermark)
                        .on_select(cx.listener(
                            |this, i: &usize, _, cx| {
                                let k = Corner::ALL[*i];
                                this.apply(|c| c.watermark_corner = k, cx)
                            },
                        ))),
                    ),
                ],
            ))
            .child(section(
                tr("settings.system"),
                vec![
                    toggle(
                        "set-autostart",
                        tr("settings.start_with_os"),
                        c.start_with_os,
                        |c| c.start_with_os = !c.start_with_os,
                    ),
                    toggle(
                        "set-updates",
                        tr("settings.check_updates"),
                        c.check_updates,
                        |c| c.check_updates = !c.check_updates,
                    ),
                    row(
                        tr("settings.discord_presence"),
                        Some(tr_args(
                            "settings.discord_presence_hint",
                            &[("name", app_core::brand::APP_NAME)],
                        )),
                        Toggle::new("set-presence", c.discord_presence).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.apply(|c| c.discord_presence = !c.discord_presence, cx)
                            },
                        )),
                    ),
                    row(
                        tr_args(
                            "settings.version",
                            &[
                                ("name", app_core::brand::APP_NAME),
                                ("version", crate::updater::CURRENT),
                            ],
                        ),
                        None,
                        Btn::new(
                            "set-check-now",
                            BtnKind::Secondary,
                            tr("settings.check_now"),
                        )
                        .size(Size::Sm)
                        .on_click(cx.listener(|this, _, _, cx| this.check_update(true, cx))),
                    ),
                ],
            ))
            .child(section(
                tr("settings.help"),
                vec![
                    row(
                        tr("settings.report_bug"),
                        Some(tr("settings.report_bug_hint")),
                        Btn::new(
                            "set-report-bug",
                            BtnKind::Secondary,
                            tr("settings.report_bug_btn"),
                        )
                        .size(Size::Sm)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.open_feedback(app_cloud::feedback::Kind::Bug, cx)
                        })),
                    ),
                    row(
                        tr("settings.send_feedback"),
                        Some(tr("settings.send_feedback_hint")),
                        Btn::new(
                            "set-feedback",
                            BtnKind::Secondary,
                            tr("settings.send_feedback_btn"),
                        )
                        .size(Size::Sm)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.open_feedback(app_cloud::feedback::Kind::Feedback, cx)
                        })),
                    ),
                    row(
                        tr("settings.star"),
                        Some(tr("settings.star_hint")),
                        Btn::new("set-star", BtnKind::Secondary, tr("settings.star_btn"))
                            .size(Size::Sm)
                            .on_click(
                                cx.listener(|_, _, _, cx| cx.open_url(app_core::brand::REPO_URL)),
                            ),
                    ),
                ],
            ))
            .child(section(tr("settings.server"), server_rows))
            .child(section(
                tr("settings.server_advanced"),
                vec![stacked(
                    tr("settings.server_url"),
                    Some(tr("settings.server_hint")),
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.))
                        .child(input_box(&s.server, window, cx))
                        .when_some(s.server_error.clone(), |d, e| {
                            d.child(div().text_size(px(12.)).text_color(tok::red()).child(e))
                        }),
                )],
            ))
            .into_any_element()
    }
}

fn set_kill_secs(k: &mut KillConfig, field: Field, n: u32) {
    match field {
        Field::KillPre => k.pre_secs = n,
        Field::KillPost => k.post_secs = n,
        _ => k.combo_secs = n,
    }
}

/// Mesma combinação de teclas, escrita de qualquer jeito ("ctrl+F9" = "F9+Ctrl").
fn same_hotkey(a: &str, b: &str) -> bool {
    let norm = |s: &str| {
        let mut parts: Vec<String> = s.split('+').map(|p| p.trim().to_uppercase()).collect();
        parts.sort();
        parts
    };
    norm(a) == norm(b)
}

/// Endereço do servidor pronto para gravar: sem espaços nem barra no fim, com http(s)://.
/// Vazio vale (sem servidor); `None` = endereço inválido.
fn normalize_server(text: &str) -> Option<String> {
    let t = text.trim().trim_end_matches('/');
    if t.is_empty() {
        return Some(String::new());
    }
    let lower = t.to_ascii_lowercase();
    let host = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"))?;
    (!host.is_empty() && !host.contains(char::is_whitespace)).then(|| t.to_string())
}

/// Largura dos segmentados nas linhas.
const SEG_W: f32 = 300.;

/// Seção: título por fora e as linhas num painel de vidro, separadas por um fio.
fn section(title: SharedString, rows: Vec<Div>) -> Div {
    let n = rows.len();
    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .child(
            div()
                .px(px(16.))
                .text_size(px(13.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(tok::fg_2())
                .child(title),
        )
        .child(glass::glass_panel(R_PANEL_SM).flex().flex_col().children(
            rows.into_iter().enumerate().map(move |(i, r)| {
                div().relative().child(r).when(i + 1 < n, |d| {
                    d.child(
                        div()
                            .absolute()
                            .bottom_0()
                            .left(px(16.))
                            .right(px(16.))
                            .h(px(1.))
                            .bg(tok::line()),
                    )
                })
            }),
        ))
}

/// Rótulo (+ dica) e o texto em coluna.
fn label_block(label: SharedString, hint_text: Option<SharedString>) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(3.))
        .flex_1()
        .min_w_0()
        .child(div().text_size(px(14.)).text_color(tok::fg()).child(label))
        .when_some(hint_text, |d, h| d.child(hint(h)))
}

/// Linha: rótulo à esquerda, controle à direita.
fn row(label: SharedString, hint_text: Option<SharedString>, control: impl IntoElement) -> Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(16.))
        .min_h(Size::Md.h() + px(16.))
        .px(px(16.))
        .py(px(8.))
        .child(label_block(label, hint_text))
        .child(control)
}

/// Linha com o campo embaixo do rótulo (caminhos e listas longas).
fn stacked(label: SharedString, hint_text: Option<SharedString>, control: impl IntoElement) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .px(px(16.))
        .py(px(12.))
        .child(div().text_size(px(14.)).text_color(tok::fg()).child(label))
        .child(control)
        .when_some(hint_text, |d, h| d.child(hint(h)))
}

fn hint(text: SharedString) -> Div {
    div()
        .text_size(px(12.))
        .line_height(px(17.))
        .text_color(tok::muted())
        .child(text)
}
