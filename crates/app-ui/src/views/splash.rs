//! Launcher: splash pequeno na abertura fria (janela e gravador fechados, ou vindo do
//! instalador). Procura atualização (5 s no máximo), instala se tiver, sobe o gravador, carrega
//! a biblioteca e só então abre a janela principal já pronta. Fechar o splash cancela tudo.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use app_cloud::update;
use app_cloud::Client;
use app_core::brand::{self, GITHUB_REPO};
use app_core::Config;
use gpui_kit::*;

use crate::app::{Preloaded, Shell};
use crate::assets::icon;
use crate::components::{IconBtn, IconKind};
use crate::data::Store;
use crate::instance::Inbox;
use crate::theme::size::Size;
use crate::theme::{self, tok, FONT_NUM};
use crate::updater::{self, CURRENT};
use crate::{recorder, secret, thumbs, tr, tr_args};

/// Prazo da consulta de versão nova.
const CHECK_TIMEOUT: Duration = Duration::from_secs(5);
/// Espera pelo gravador recém-aberto.
const RECORDER_WAIT: Duration = Duration::from_secs(10);
/// Prazo das chamadas ao servidor de clipes (não seguram a abertura).
const NET_WAIT: Duration = Duration::from_secs(3);
/// Miniaturas carregadas antes de abrir (a primeira tela da grade).
const FIRST_THUMBS: usize = 24;
/// Quanto tempo "Não deu pra atualizar" fica na tela.
const FAIL_PAUSE: Duration = Duration::from_secs(2);

/// O splash aparece? Gravador sem resposta no IPC (abertura fria) ou vindo do instalador.
pub fn wanted(from_installer: bool) -> bool {
    from_installer || app_ipc::Client::connect().is_err()
}

pub struct Splash {
    status: SharedString,
    /// Fração baixada (`None` = sem barra determinada).
    progress: Option<f32>,
    cancel: Arc<AtomicBool>,
}

impl Splash {
    pub fn new(
        inbox: Inbox,
        link: Option<String>,
        from_installer: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = cancel.clone();
        cx.spawn_in(window, async move |this, cx| {
            let set =
                |this: &WeakEntity<Splash>, cx: &mut AsyncWindowContext, s: SharedString, p| {
                    this.update(cx, |t, cx| {
                        t.status = s;
                        t.progress = p;
                        cx.notify();
                    })
                    .is_ok()
                };
            let bg = cx.background_executor().clone();
            let config = bg
                .spawn(async {
                    Config::load_or_create(&Config::default_path()).unwrap_or_default()
                })
                .await;

            // 1-2. Versão nova: baixa, confere e abre o instalador (ele reabre o app).
            if config.check_updates && !from_installer && cfg!(windows) {
                if !set(&this, cx, tr("splash.checking"), None) {
                    return;
                }
                let found = bg
                    .spawn(async { update::latest_release_within(GITHUB_REPO, CHECK_TIMEOUT) })
                    .await;
                match found {
                    Ok(Some(rel)) if update::is_newer(&rel.version, CURRENT) => {
                        match install(&this, cx, &rel, stop.clone()).await {
                            Some(true) => return,
                            None => return,
                            Some(false) => {
                                if !set(&this, cx, tr("splash.update_failed"), None) {
                                    return;
                                }
                                bg.timer(FAIL_PAUSE).await;
                            }
                        }
                    }
                    Ok(_) => {}
                    Err(e) => tracing::info!("launcher: procurar atualização: {e}"),
                }
            }

            // 3. Gravador.
            if app_ipc::Client::connect().is_err() {
                if !set(&this, cx, tr("splash.recorder"), None) {
                    return;
                }
                match recorder::start() {
                    Ok(()) => {
                        let until = Instant::now() + RECORDER_WAIT;
                        while Instant::now() < until {
                            let up = bg
                                .spawn(async {
                                    matches!(
                                        app_ipc::Client::call(&app_ipc::Request::Hello {
                                            version: app_ipc::PROTOCOL_VERSION,
                                        }),
                                        Ok(app_ipc::Response::Hello { .. })
                                    )
                                })
                                .await;
                            if up || stop.load(Ordering::Relaxed) {
                                break;
                            }
                            bg.timer(Duration::from_millis(250)).await;
                        }
                    }
                    Err(e) => tracing::warn!("launcher: abrir o gravador: {e}"),
                }
            }

            // 4. Biblioteca, miniaturas da primeira tela e servidor (rede com prazo curto).
            if !set(&this, cx, tr("splash.library"), None) {
                return;
            }
            let pre = bg.spawn(async move { preload() }).await;
            if stop.load(Ordering::Relaxed) {
                return;
            }

            // 5. Janela principal pronta; o splash fecha.
            let _ = cx.update(|window, cx| {
                let options = crate::app_window_options(cx);
                let opened = gpui_kit::open_window(options, cx, move |window, cx| {
                    theme::refresh(window, cx);
                    cx.new(|cx| Shell::new(inbox, link, Some(pre), window, cx))
                });
                if let Err(e) = opened {
                    tracing::error!("abrir a janela principal: {e:#}");
                }
                window.remove_window();
            });
        })
        .detach();
        Self {
            status: tr("splash.starting"),
            progress: None,
            cancel,
        }
    }
}

/// Baixa e abre o instalador. `Some(true)` = instalador aberto (o app fecha); `Some(false)` =
/// falhou (segue na versão atual); `None` = splash fechado.
async fn install(
    this: &WeakEntity<Splash>,
    cx: &mut AsyncWindowContext,
    rel: &update::Release,
    stop: Arc<AtomicBool>,
) -> Option<bool> {
    let bg = cx.background_executor().clone();
    let progress = Arc::new(Mutex::new(0f32));
    let done = Arc::new(AtomicBool::new(false));
    let dest = std::env::temp_dir().join(&rel.file);
    let task = {
        let (p, d, s, r, dest) = (
            progress.clone(),
            done.clone(),
            stop.clone(),
            rel.clone(),
            dest.clone(),
        );
        bg.spawn(async move {
            let out = update::download(&r, &dest, &mut |got, total| {
                *p.lock().unwrap_or_else(|e| e.into_inner()) = got as f32 / total.max(1) as f32;
                !s.load(Ordering::Relaxed)
            });
            d.store(true, Ordering::Relaxed);
            out
        })
    };
    while !done.load(Ordering::Relaxed) {
        let f = *progress.lock().unwrap_or_else(|e| e.into_inner());
        let pct = format!("{:.0}", f * 100.0);
        let shown = this.update(cx, |t, cx| {
            t.status = tr_args(
                "splash.downloading",
                &[("version", &rel.version), ("percent", &pct)],
            );
            t.progress = Some(f);
            cx.notify();
        });
        if shown.is_err() {
            stop.store(true, Ordering::Relaxed);
            return None;
        }
        bg.timer(Duration::from_millis(150)).await;
    }
    match task.await {
        Ok(()) => {}
        Err(app_cloud::CloudError::Cancelled) => return None,
        Err(e) => {
            tracing::warn!("launcher: baixar a atualização: {e}");
            return Some(false);
        }
    }
    this.update(cx, |t, cx| {
        t.status = tr("splash.installing");
        t.progress = Some(1.0);
        cx.notify();
    })
    .ok()?;
    match updater::run_installer(&dest) {
        Ok(()) => {
            let _ = cx.update(|_, cx| cx.quit());
            Some(true)
        }
        Err(e) => {
            tracing::warn!("launcher: abrir o instalador: {e}");
            Some(false)
        }
    }
}

/// Tudo que a janela principal precisa ao abrir (roda fora da thread da UI).
fn preload() -> Preloaded {
    let config = Config::load_or_create(&Config::default_path()).unwrap_or_default();
    let store = Store::open();
    let thumbs = store
        .clips
        .iter()
        .filter_map(|c| c.thumb_path.clone())
        .take(FIRST_THUMBS)
        .filter_map(|p| thumbs::load(&p, thumbs::CARD_W).map(|t| (p, t)))
        .collect();

    let mut pre = Preloaded {
        config,
        store,
        thumbs,
        server_info: None,
        account: None,
        unauthorized: false,
    };
    let base = pre.config.server_url.trim().to_string();
    if base.is_empty() {
        return pre;
    }
    // A rede vai numa thread própria: passou do prazo, a janela abre sem e busca depois.
    let token = secret::load();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let info = Client::new(&base, None).info().ok();
        let me = token.map(|t| Client::new(&base, Some(t)).me());
        let _ = tx.send((info, me));
    });
    if let Ok((info, me)) = rx.recv_timeout(NET_WAIT) {
        pre.server_info = info;
        match me {
            Some(Ok(m)) => pre.account = Some((m.name().to_string(), m.user.is_admin())),
            Some(Err(app_cloud::CloudError::Unauthorized)) => pre.unauthorized = true,
            _ => {}
        }
    }
    pre
}

impl Render for Splash {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let cancel = self.cancel.clone();
        let bar = div()
            .h(px(3.))
            .w_full()
            .rounded_full()
            .bg(tok::track())
            .overflow_hidden()
            .child(match self.progress {
                Some(f) => div()
                    .h_full()
                    .rounded_full()
                    .bg(tok::blue())
                    .w(relative(f.clamp(0.0, 1.0)))
                    .into_any_element(),
                // Sem fração: um trecho que corre pela barra.
                None => div()
                    .h_full()
                    .w(relative(0.3))
                    .rounded_full()
                    .bg(tok::blue())
                    .with_animation(
                        "splash-run",
                        Animation::new(Duration::from_millis(1400))
                            .repeat()
                            .with_easing(ease_in_out),
                        |d, t| d.ml(relative(-0.3 + 1.3 * t)),
                    )
                    .into_any_element(),
            });
        div()
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .items_center()
            .bg(tok::bg())
            .font_family(theme::FONT_UI)
            .text_color(tok::fg())
            .child(crate::components::window_controls::drag_area(
                "splash-drag",
                div().absolute().top_0().left_0().right_0().h(px(48.)),
            ))
            .child(
                div().absolute().top(px(10.)).right(px(10.)).child(
                    IconBtn::new("splash-close", icon::CLOSE)
                        .kind(IconKind::Ghost)
                        .size(Size::Sm)
                        .on_click(move |_, window, _| {
                            cancel.store(true, Ordering::Relaxed);
                            window.remove_window();
                        }),
                ),
            )
            .child(div().flex_1())
            .child(img("brand/glint.svg").size(px(84.)))
            .child(
                div()
                    .pt(px(18.))
                    .text_size(px(26.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(brand::APP_NAME.to_lowercase()),
            )
            .child(div().flex_1())
            .child(
                div()
                    .w_full()
                    .px(px(36.))
                    .pb(px(30.))
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(tok::fg_2())
                            .truncate()
                            .child(self.status.clone()),
                    )
                    .child(bar)
                    .child(
                        div()
                            .font_family(FONT_NUM)
                            .text_size(px(11.))
                            .text_color(tok::muted())
                            .child(CURRENT),
                    ),
            )
    }
}
