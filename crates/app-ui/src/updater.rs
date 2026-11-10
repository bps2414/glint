//! Atualização automática: lê no GitHub (releases do app) a versão mais nova ao abrir (depois de alguns
//! segundos) e a cada 6 h; com versão nova, a sidebar mostra "Atualizar". O instalador é
//! baixado em segundo plano (SHA-256 conferido), aberto em modo silencioso e o app fecha; o
//! instalador fecha o gravador, troca os arquivos e abre tudo de novo.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use app_cloud::update::{self, Release};
use app_core::brand::GITHUB_REPO;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::app::Shell;
use crate::assets::icon;
use crate::components::{glass, Btn, BtnKind, IconBtn, IconKind};
use crate::theme::size::Size;
use crate::theme::tok;
use crate::{tr, tr_args};

/// Versão deste build.
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");
const FIRST_CHECK: Duration = Duration::from_secs(8);
const EVERY: Duration = Duration::from_secs(6 * 3600);

#[derive(Clone, Debug, Default, PartialEq)]
pub enum UpdateState {
    #[default]
    None,
    Available(Release),
    /// Baixando (fração 0..1).
    Downloading(Release, f32),
    /// Falhou ao baixar/abrir (a mensagem aparece no card; dá para tentar de novo).
    Failed(Release, String),
}

impl UpdateState {
    pub fn release(&self) -> Option<&Release> {
        match self {
            UpdateState::None => None,
            UpdateState::Available(r)
            | UpdateState::Downloading(r, _)
            | UpdateState::Failed(r, _) => Some(r),
        }
    }
}

impl Shell {
    /// Procura atualização agora e depois a cada 6 h (se ligado nas Configurações).
    pub fn watch_updates(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(FIRST_CHECK).await;
            loop {
                if this.update(cx, |t, cx| t.check_update(false, cx)).is_err() {
                    break;
                }
                cx.background_executor().timer(EVERY).await;
            }
        })
        .detach();
    }

    /// Pergunta ao GitHub. `manual` = botão "Procurar agora" (avisa o resultado).
    pub fn check_update(&mut self, manual: bool, cx: &mut Context<Self>) {
        if !manual && !self.config.check_updates {
            return;
        }
        if matches!(self.update, UpdateState::Downloading(..)) {
            return;
        }
        cx.spawn(async move |this, cx| {
            let r = cx
                .background_executor()
                .spawn(async move { update::latest_release(GITHUB_REPO) })
                .await;
            let _ = this.update(cx, |t, cx| {
                match r {
                    Ok(Some(rel)) if update::is_newer(&rel.version, CURRENT) => {
                        if t.update.release() != Some(&rel) {
                            t.update = UpdateState::Available(rel);
                        }
                    }
                    Ok(_) if manual => {
                        t.notify_user(tr_args("update.latest", &[("version", CURRENT)]), cx)
                    }
                    Err(e) if manual => t.notify_user(
                        tr_args("update.check_error", &[("error", &e.to_string())]),
                        cx,
                    ),
                    Err(e) => tracing::info!("procurar atualização: {e}"),
                    Ok(_) => {}
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Baixa o instalador, abre em modo silencioso e fecha o app.
    pub fn install_update(&mut self, cx: &mut Context<Self>) {
        let Some(rel) = self.update.release().cloned() else {
            return;
        };
        if matches!(self.update, UpdateState::Downloading(..)) {
            return;
        }
        if !cfg!(windows) {
            // Fora do Windows não há instalador: abre a página de download.
            cx.open_url(&rel.url);
            return;
        }
        self.update = UpdateState::Downloading(rel.clone(), 0.0);
        cx.notify();
        let progress = Arc::new(Mutex::new(0f32));
        let stop = Arc::new(AtomicBool::new(false));
        let dest: PathBuf = std::env::temp_dir().join(&rel.file);
        let (p2, s2, rel2, dest2) = (progress.clone(), stop.clone(), rel.clone(), dest.clone());
        let task = cx.background_executor().spawn(async move {
            update::download(&rel2, &dest2, &mut |got, total| {
                *p2.lock().unwrap_or_else(|e| e.into_inner()) = got as f32 / total.max(1) as f32;
                !s2.load(Ordering::Relaxed)
            })
        });
        // Barra de progresso: lê a fração a cada 200 ms enquanto baixa.
        cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(200))
                .await;
            let f = *progress.lock().unwrap_or_else(|e| e.into_inner());
            let alive = this.update(cx, |t, cx| {
                if let UpdateState::Downloading(r, _) = &t.update {
                    t.update = UpdateState::Downloading(r.clone(), f);
                    cx.notify();
                    true
                } else {
                    false
                }
            });
            if !matches!(alive, Ok(true)) {
                stop.store(true, Ordering::Relaxed);
                break;
            }
            if f >= 1.0 {
                break;
            }
        })
        .detach();
        cx.spawn(async move |this, cx| {
            let r = task.await;
            let _ = this.update(cx, |t, cx| {
                match r
                    .map_err(|e| e.to_string())
                    .and_then(|()| run_installer(&dest).map_err(|e| e.to_string()))
                {
                    Ok(()) => {
                        t.save_title(cx);
                        t.save_trim();
                        cx.quit();
                    }
                    Err(e) => {
                        t.update = UpdateState::Failed(rel, e);
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    pub fn dismiss_update(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.update, UpdateState::Downloading(..)) {
            self.update = UpdateState::None;
            cx.notify();
        }
    }
}

/// Abre o instalador sem perguntas (Inno Setup: só a barra de progresso). Ele fecha o
/// gravador e este app, instala por cima e abre os dois de novo (o app com
/// `--from-installer`, que passa pelo splash sem procurar atualização).
pub(crate) fn run_installer(path: &std::path::Path) -> std::io::Result<()> {
    std::process::Command::new(path)
        .args([
            "/SILENT",
            "/SUPPRESSMSGBOXES",
            "/NORESTART",
            "/CLOSEAPPLICATIONS",
        ])
        .spawn()
        .map(|_| ())
}

impl Shell {
    /// Card da sidebar com a versão nova (acima do card do gravador).
    pub fn update_card(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let rel = self.update.release()?.clone();
        let mut card = glass::inset_box()
            .flex()
            .flex_col()
            .gap(px(8.))
            .p(px(12.))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(tr_args("update.available", &[("version", &rel.version)])),
                    )
                    .when(!matches!(self.update, UpdateState::Downloading(..)), |d| {
                        d.child(
                            IconBtn::new("update-close", icon::CLOSE)
                                .kind(IconKind::Ghost)
                                .size(Size::Xs)
                                .on_click(cx.listener(|this, _, _, cx| this.dismiss_update(cx))),
                        )
                    }),
            );
        if !rel.notes.is_empty() {
            card = card.child(
                div()
                    .text_size(px(12.))
                    .line_height(px(16.))
                    .text_color(tok::fg_2())
                    .child(rel.notes.clone()),
            );
        }
        card = match &self.update {
            UpdateState::Downloading(_, f) => card.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(tok::fg_2())
                            .child(tr_args(
                                "update.downloading",
                                &[("percent", &format!("{:.0}", f * 100.0))],
                            )),
                    )
                    .child(
                        div()
                            .h(px(4.))
                            .w_full()
                            .rounded_full()
                            .bg(tok::track())
                            .child(
                                div()
                                    .h_full()
                                    .rounded_full()
                                    .bg(tok::blue())
                                    .w(relative(f.clamp(0.0, 1.0))),
                            ),
                    ),
            ),
            state => card
                .when_some(
                    match state {
                        UpdateState::Failed(_, e) => Some(e.clone()),
                        _ => None,
                    },
                    |d, e| {
                        d.child(
                            div()
                                .text_size(px(12.))
                                .text_color(tok::fg_2())
                                .child(tr_args("update.failed", &[("error", &e)])),
                        )
                    },
                )
                .child(
                    Btn::new("update-install", BtnKind::Primary, tr("update.install"))
                        .size(Size::Sm)
                        .full_width()
                        .on_click(cx.listener(|this, _, _, cx| this.install_update(cx))),
                ),
        };
        Some(card.into_any_element())
    }
}
